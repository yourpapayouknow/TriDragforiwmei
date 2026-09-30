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
use touchpad::TchpdEng;
use tray::Tray;

const WM_APP_TRAY: u32 = WM_APP + 1;

/// Application entry: dispatches privileged helper runs, then starts the tray.
fn main() -> Result<()> {
    if let Some(mode) = hlprmode() {
        utils::initlog()?;
        return runhlpr(mode);
    }

    let _single = utils::snglinst()?;
    utils::initlog()?;
    info!("Starting tridragforiwmei");

    let cfgpath = config::cfgpath()?;
    let config = Config::load(&cfgpath)?;

    if let Err(e) = scheduler::setstp(config.start_at_boot) {
        log::error!("Failed to sync startup task: {e}");
    }

    let hwnd = mkmsgwnd()?;
    TchpdEng::register(hwnd)?;
    let tray = Tray::new(hwnd, WM_APP_TRAY)?;

    let mut app = ApSt {
        tray,
        config,
        cfgpath,
        touchpad: TchpdEng::new(),
    };

    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr::addr_of_mut!(app) as isize);
        runmsgloop();
    }

    app.config.save(&app.cfgpath)?;
    Ok(())
}

/// Runtime state shared with the window procedure via GWLP_USERDATA.
struct ApSt {
    tray: Tray,
    config: Config,
    cfgpath: PathBuf,
    touchpad: TchpdEng,
}

/// Registers the window class and creates the hidden top-level window.
fn mkmsgwnd() -> Result<HWND> {
    let instance = unsafe { GetModuleHandleW(None)? };
    let clsname = w!("TriDragForIwmeiMessageWindow");

    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wndproc),
        hInstance: instance.into(),
        lpszClassName: clsname,
        ..Default::default()
    };

    if unsafe { RegisterClassExW(&wc) } == 0 {
        anyhow::bail!("RegisterClassExW failed");
    }

    // RIDEV_INPUTSINK needs a real top-level window; a message-only window
    // never receives WM_INPUT. Hide it from the taskbar and from Alt+Tab.
    unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            clsname,
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

/// Pumps the thread message queue until WM_QUIT.
unsafe fn runmsgloop() {
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).into() {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

/// Routes window messages to their handlers.
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_INPUT => hndlinp(hwnd, lparam),
        WM_INPUT_DEVICE_CHANGE => hndldevchg(hwnd, lparam),
        WM_APP_TRAY => hndltray(hwnd, wparam, lparam),
        WM_TIMER => hndltmr(hwnd, wparam),
        WM_DESTROY => hndldstr(hwnd),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Feeds touchpad contacts into the drag engine and emits mouse events.
unsafe fn hndlinp(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let app = getapst(hwnd);
    if app.config.enabled {
        if let Some(contacts) = app.touchpad.prsinp(lparam.0 as _) {
            let devid = app.touchpad.curdevid();

            if let Some(delta) = app.touchpad.engine.onctc(&contacts, &app.config, &devid) {
                mouse::sndmov(delta.x, delta.y);
            }
            match app.touchpad.engine.btnevnt() {
                Some(mouse::BtEv::Down) => mouse::sndbtndwn(app.config.button),
                Some(mouse::BtEv::Up) => mouse::sndbtnup(app.config.button),
                None => {}
            }
            if app.touchpad.engine.isdrgging() {
                let _ = SetTimer(Some(hwnd), touchpad::RLS_TMR_ID, app.config.rlsdly(), None);
            }
        }
    }
    DefWindowProcW(hwnd, WM_INPUT, WPARAM(lparam.0 as _), lparam)
}

/// Drops cached touchpad caps so the next report re-queries the device set.
unsafe fn hndldevchg(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let app = getapst(hwnd);
    app.touchpad.ondevchg(HANDLE(lparam.0 as _));
    DefWindowProcW(hwnd, WM_INPUT_DEVICE_CHANGE, WPARAM(0), lparam)
}

/// Forwards tray callback messages to the tray menu.
unsafe fn hndltray(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let app = getapst(hwnd);
    app.tray
        .hndlevnt(wparam, lparam, &mut app.config, &app.cfgpath);
    LRESULT(0)
}

/// Ends a drag once the release delay elapses without new input.
unsafe fn hndltmr(hwnd: HWND, wparam: WPARAM) -> LRESULT {
    let app = getapst(hwnd);
    if wparam.0 == touchpad::RLS_TMR_ID {
        app.touchpad.engine.onrltmr();
        if !app.touchpad.engine.isdrgging() {
            let _ = KillTimer(Some(hwnd), touchpad::RLS_TMR_ID);
        }
    }
    DefWindowProcW(hwnd, WM_TIMER, wparam, LPARAM(0))
}

/// Persists config and quits the message loop.
unsafe fn hndldstr(hwnd: HWND) -> LRESULT {
    let app = getapst(hwnd);
    let _ = app.config.save(&app.cfgpath);
    PostQuitMessage(0);
    LRESULT(0)
}

/// Reads the application state pointer stored in the window user data.
unsafe fn getapst(hwnd: HWND) -> &'static mut ApSt {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    &mut *(ptr as *mut ApSt)
}

/// Privileged setup modes run by an elevated copy of this binary.
#[derive(Clone, Copy)]
enum Hlpr {
    EnbStartup,
    DsbStartup,
}

/// Detects a helper invocation from the command line.
fn hlprmode() -> Option<Hlpr> {
    env::args().skip(1).find_map(|arg| match arg.as_str() {
        "--task-enable" => Some(Hlpr::EnbStartup),
        "--task-disable" => Some(Hlpr::DsbStartup),
        _ => None,
    })
}

/// Applies the requested startup task state, then exits.
fn runhlpr(mode: Hlpr) -> Result<()> {
    let enabled = matches!(mode, Hlpr::EnbStartup);
    let result = scheduler::setstp(enabled);
    match &result {
        Ok(()) => info!("Helper set startup task enabled={enabled}"),
        Err(e) => log::error!("Helper failed to set startup task: {e}"),
    }
    result
}
