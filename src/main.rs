// 编译为 GUI 程序以免 Windows 分配控制台窗口，调试构建保留控制台便于查看输出
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
mod lang;
mod mouse;
mod scheduler;
mod touchpad;
mod tray;
mod utils;

use config::Config;
use touchpad::TchpdEng;
use tray::Tray;

// 托盘图标的回调消息号
const WM_APP_TRAY: u32 = WM_APP + 1;

// 程序入口：先分派提权辅助模式，否则启动托盘
fn main() -> Result<()> {
    // 日志最先初始化，后续任一步骤失败才有据可查
    utils::initlog()?;

    let result = if let Some(mode) = hlprmode() {
        runhlpr(mode)
    } else {
        runapp()
    };

    if let Err(e) = &result {
        log::error!("Fatal: {e:#}");
    }
    result
}

// 建立单实例、窗口、托盘并进入消息循环
fn runapp() -> Result<()> {
    let _single = utils::snglinst()?;
    info!("Starting tridragforiwmei");

    let cfgpath = config::cfgpath()?;
    let config = Config::load(&cfgpath)?;

    if let Err(e) = scheduler::setstp(config.start_at_boot) {
        log::error!("Failed to sync startup task: {e}");
    }

    let hwnd = mkmsgwnd()?;
    TchpdEng::register(hwnd)?;
    let tray = Tray::new(hwnd, WM_APP_TRAY, config.lang)?;

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

// 经 GWLP_USERDATA 与窗口过程共享的运行时状态
struct ApSt {
    tray: Tray,
    config: Config,
    cfgpath: PathBuf,
    touchpad: TchpdEng,
}

// 注册窗口类并创建隐藏的顶层窗口
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

    // RIDEV_INPUTSINK 需要真实顶层窗口，仅消息窗口收不到 WM_INPUT
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

// 抽送线程消息队列直至收到 WM_QUIT
unsafe fn runmsgloop() {
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).into() {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

// 将窗口消息分派到各处理函数
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

// 将触摸板触点送入拖拽引擎并发出鼠标事件
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

// 设备增删后清除触摸板缓存
unsafe fn hndldevchg(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let app = getapst(hwnd);
    app.touchpad.ondevchg(HANDLE(lparam.0 as _));
    DefWindowProcW(hwnd, WM_INPUT_DEVICE_CHANGE, WPARAM(0), lparam)
}

// 将托盘回调消息转交托盘菜单处理
unsafe fn hndltray(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let app = getapst(hwnd);
    app.tray
        .hndlevnt(wparam, lparam, &mut app.config, &app.cfgpath);
    LRESULT(0)
}

// 释放延迟内无新输入时结束拖拽
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

// 保存配置并退出消息循环
unsafe fn hndldstr(hwnd: HWND) -> LRESULT {
    let app = getapst(hwnd);
    let _ = app.config.save(&app.cfgpath);
    PostQuitMessage(0);
    LRESULT(0)
}

// 读取窗口用户数据中保存的应用状态指针
unsafe fn getapst(hwnd: HWND) -> &'static mut ApSt {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    &mut *(ptr as *mut ApSt)
}

// 由本程序的提权副本执行的辅助模式
#[derive(Clone, Copy)]
enum Hlpr {
    EnbStartup,
    DsbStartup,
}

// 从命令行识别辅助模式调用
fn hlprmode() -> Option<Hlpr> {
    env::args().skip(1).find_map(|arg| match arg.as_str() {
        "--task-enable" => Some(Hlpr::EnbStartup),
        "--task-disable" => Some(Hlpr::DsbStartup),
        _ => None,
    })
}

// 应用所请求的登录任务状态后退出
fn runhlpr(mode: Hlpr) -> Result<()> {
    let enabled = matches!(mode, Hlpr::EnbStartup);
    let result = scheduler::setstp(enabled);
    match &result {
        Ok(()) => info!("Helper set startup task enabled={enabled}"),
        Err(e) => log::error!("Helper failed to set startup task: {e}"),
    }
    result
}
