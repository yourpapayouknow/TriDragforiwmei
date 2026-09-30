use std::env;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use log::info;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::IsUserAnAdmin;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentProcessId;
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
    // Ensure single instance.
    let _single = utils::single_instance()?;

    // Initialize logging.
    utils::init_logging()?;
    info!("Starting tridragforiwmei");

    // Load config.
    let config_path = config::config_path()?;
    let mut config = Config::load(&config_path)?;

    // Handle elevation.
    if config.run_elevated && !is_admin() {
        if let Err(e) = restart_elevated() {
            log::error!("Failed to restart elevated: {}", e);
            config.run_elevated = false;
            config.save(&config_path)?;
        } else {
            return Ok(());
        }
    }

    // Apply startup task state.
    scheduler::sync_startup(config.start_at_boot)?;

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

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!("TriDragForIwmei"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        )?
    };

    // Register raw input for precision touchpad.
    TouchpadEngine::register(hwnd)?;

    // Create tray icon.
    let tray = Tray::new(hwnd, WM_APP_TRAY)?;

    // Store app state in window user data.
    let mut app = AppState {
        hwnd,
        tray,
        config,
        config_path,
        touchpad: TouchpadEngine::new(),
    };
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, &mut app as *mut _ as isize);
    }

    // Message loop.
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).into() {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    app.config.save(&app.config_path)?;
    Ok(())
}

struct AppState {
    hwnd: HWND,
    tray: Tray,
    config: Config,
    config_path: PathBuf,
    touchpad: TouchpadEngine,
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_INPUT => {
            let app = get_app_state(hwnd);
            let enabled = app.config.enabled;
            if let Some(contacts) = app.touchpad.parse_input(lparam.0 as _) {
                if enabled {
                    let device_id = app.touchpad.current_device_id();
                    let mut config = app.config.clone();
                    config.current_device_id = device_id;
                    let delta = app.touchpad.engine.on_contacts(&contacts, &config);
                    if let Some(d) = delta {
                        mouse::send_move(d.x, d.y);
                    }
                    match app.touchpad.engine.button_event() {
                        Some(mouse::ButtonEvent::Down) => mouse::send_button_down(&app.config.button),
                        Some(mouse::ButtonEvent::Up) => mouse::send_button_up(&app.config.button),
                        None => {}
                    }
                    // Restart release timer on drag activity.
                    if app.touchpad.engine.is_dragging() {
                        SetTimer(Some(hwnd), touchpad::RELEASE_TIMER_ID as usize, config.release_delay_ms, None);
                    }
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_INPUT_DEVICE_CHANGE => {
            let app = get_app_state(hwnd);
            app.touchpad.on_device_change(windows::Win32::Foundation::HANDLE(lparam.0 as _));
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_APP_TRAY => {
            let app = get_app_state(hwnd);
            app.tray.handle_event(wparam, lparam, &mut app.config, &app.config_path);
            LRESULT(0)
        }
        WM_TIMER => {
            let app = get_app_state(hwnd);
            if wparam.0 == touchpad::RELEASE_TIMER_ID as usize {
                app.touchpad.engine.on_release_timer();
                if !app.touchpad.engine.is_dragging() {
                    KillTimer(Some(hwnd), touchpad::RELEASE_TIMER_ID as usize);
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DESTROY => {
            let app = get_app_state(hwnd);
            let _ = app.config.save(&app.config_path);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn get_app_state(hwnd: HWND) -> &'static mut AppState {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        &mut *(ptr as *mut AppState)
    }
}

fn is_admin() -> bool {
    unsafe { IsUserAnAdmin().into() }
}

fn restart_elevated() -> Result<()> {
    let exe = env::current_exe().context("current_exe")?;
    let pid = unsafe { GetCurrentProcessId() };
    let mut cmd = Command::new("powershell.exe");
    cmd.arg("-NoProfile")
        .arg("-WindowStyle")
        .arg("Hidden")
        .arg("-Command")
        .arg(format!(
            "Start-Process -FilePath '{}' -Verb runas -ArgumentList '--elevated {}';",
            exe.display(),
            pid
        ));
    cmd.spawn().context("spawn elevator")?;
    Ok(())
}
