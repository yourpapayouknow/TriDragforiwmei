use std::env;
use std::path::PathBuf;
use std::ptr;

use anyhow::{Context, Result};
use log::info;
use windows::core::w;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

mod config;
mod drag_engine;
mod mouse;
mod scheduler;
mod touchpad;
mod tray;
mod utils;

use config::Config;
use touchpad::TouchpadEngine;
use tray::Tray;

const WM_APP_TRAY: u32 = WM_APP + 1;

fn main() -> Result<()> {
    // Helpers run once, do their task and exit; they deliberately skip the
    // single-instance guard and the tray so the running instance is untouched.
    if let Some(mode) = helper_mode() {
        utils::init_logging()?;
        return run_helper(mode);
    }

    let _single = utils::single_instance()?;
    utils::init_logging()?;
    info!("Starting tridragforiwmei");

    let config_path = config::config_path()?;
    let config = Config::load(&config_path)?;

    // A failed startup-task sync must not stop the app from running; the user
    // may simply not be elevated, and the drag feature is independent of it.
    if let Err(e) = scheduler::set_startup(config.start_at_boot) {
        log::error!("Failed to sync startup task: {e}");
    }

    let hwnd = create_message_window()?;
    TouchpadEngine::register(hwnd)?;
    info!("Raw input registered for precision touchpad, hwnd={hwnd:?}");
    let tray = Tray::new(hwnd, WM_APP_TRAY)?;

    let mut app = AppState {
        hwnd,
        tray,
        config,
        config_path,
        touchpad: TouchpadEngine::new(),
    };

    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr::addr_of_mut!(app) as isize);
        run_message_loop();
    }

    app.config.save(&app.config_path)?;
    Ok(())
}

struct AppState {
    #[allow(dead_code)]
    hwnd: HWND,
    tray: Tray,
    config: Config,
    config_path: PathBuf,
    touchpad: TouchpadEngine,
}

fn create_message_window() -> Result<HWND> {
    let instance = unsafe { GetModuleHandleW(None)? };
    let class_name = w!("TriDragForIwmeiMessageWindow");

    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(window_proc),
        hInstance: instance.into(),
        lpszClassName: class_name,
        ..Default::default()
    };

    let atom = unsafe { RegisterClassExW(&wc) };
    if atom == 0 {
        anyhow::bail!("RegisterClassExW failed");
    }

    // Raw input with RIDEV_INPUTSINK requires a real top-level window; a
    // message-only window (HWND_MESSAGE) never receives WM_INPUT reports.
    // Use a hidden top-level window excluded from the taskbar and Alt+Tab.
    unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name,
            w!("TriDragForIwmei"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .context("CreateWindowExW failed")
    }
}

unsafe fn run_message_loop() {
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).into() {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_INPUT => handle_input(hwnd, lparam),
        WM_INPUT_DEVICE_CHANGE => handle_device_change(hwnd, lparam),
        WM_APP_TRAY => handle_tray(hwnd, wparam, lparam),
        WM_TIMER => handle_timer(hwnd, wparam),
        WM_DESTROY => handle_destroy(hwnd),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

unsafe fn handle_input(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let app = get_app_state(hwnd);
    if app.config.enabled {
        if let Some(contacts) = app.touchpad.parse_input(lparam.0 as _) {
            let device_id = app.touchpad.current_device_id();

            if let Some(delta) = app
                .touchpad
                .engine
                .on_contacts(&contacts, &app.config, &device_id)
            {
                mouse::send_move(delta.x, delta.y);
            }
            match app.touchpad.engine.button_event() {
                Some(mouse::ButtonEvent::Down) => mouse::send_button_down(app.config.button),
                Some(mouse::ButtonEvent::Up) => mouse::send_button_up(app.config.button),
                None => {}
            }
            if app.touchpad.engine.is_dragging() {
                let _ = SetTimer(
                    Some(hwnd),
                    touchpad::RELEASE_TIMER_ID,
                    app.config.release_delay(),
                    None,
                );
            }
        }
    }
    DefWindowProcW(hwnd, WM_INPUT, WPARAM(lparam.0 as _), lparam)
}

unsafe fn handle_device_change(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let app = get_app_state(hwnd);
    app.touchpad.on_device_change(HANDLE(lparam.0 as _));
    DefWindowProcW(hwnd, WM_INPUT_DEVICE_CHANGE, WPARAM(0), lparam)
}

unsafe fn handle_tray(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let app = get_app_state(hwnd);
    app.tray
        .handle_event(wparam, lparam, &mut app.config, &app.config_path);
    LRESULT(0)
}

unsafe fn handle_timer(hwnd: HWND, wparam: WPARAM) -> LRESULT {
    let app = get_app_state(hwnd);
    if wparam.0 == touchpad::RELEASE_TIMER_ID {
        app.touchpad.engine.on_release_timer();
        if !app.touchpad.engine.is_dragging() {
            let _ = KillTimer(Some(hwnd), touchpad::RELEASE_TIMER_ID);
        }
    }
    DefWindowProcW(hwnd, WM_TIMER, wparam, LPARAM(0))
}

unsafe fn handle_destroy(hwnd: HWND) -> LRESULT {
    let app = get_app_state(hwnd);
    let _ = app.config.save(&app.config_path);
    PostQuitMessage(0);
    LRESULT(0)
}

unsafe fn get_app_state(hwnd: HWND) -> &'static mut AppState {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    &mut *(ptr as *mut AppState)
}

/// Helper invocations used to run privileged setup from an unelevated process.
#[derive(Clone, Copy)]
enum Helper {
    EnableStartup,
    DisableStartup,
}

fn helper_mode() -> Option<Helper> {
    env::args().skip(1).find_map(|arg| match arg.as_str() {
        "--task-enable" => Some(Helper::EnableStartup),
        "--task-disable" => Some(Helper::DisableStartup),
        _ => None,
    })
}

fn run_helper(mode: Helper) -> Result<()> {
    let enabled = matches!(mode, Helper::EnableStartup);
    let result = scheduler::set_startup(enabled);
    match &result {
        Ok(()) => info!("Helper set startup task enabled={enabled}"),
        Err(e) => log::error!("Helper failed to set startup task: {e}"),
    }
    result
}
