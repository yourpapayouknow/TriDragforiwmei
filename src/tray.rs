use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use log::{error, info};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos, InsertMenuW, LoadIconW,
    PostQuitMessage, SetForegroundWindow, TrackPopupMenu, HICON, HMENU, IDI_APPLICATION,
    MF_BYPOSITION, MF_CHECKED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, TPM_LEFTALIGN, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON,
};

use crate::config::Config;
use crate::scheduler;

pub struct Tray {
    hwnd: HWND,
    icon: HICON,
}

const ID_ENABLED: u32 = 1;
const ID_START_BOOT: u32 = 2;
const ID_OPEN_CONFIG: u32 = 3;
const ID_QUIT: u32 = 4;

impl Tray {
    pub fn new(hwnd: HWND, msg_id: u32) -> Result<Self> {
        let icon = unsafe { LoadIconW(None, IDI_APPLICATION).ok().context("load icon")? };
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: msg_id,
            hIcon: icon,
            szTip: [0; 128],
            ..Default::default()
        };
        copy_tooltip(&mut nid.szTip, w!("Three-Finger Drag"));

        unsafe {
            Shell_NotifyIconW(NIM_ADD, &nid).ok()?;
        }
        Ok(Self { hwnd, icon })
    }

    pub fn handle_event(
        &mut self,
        _wparam: WPARAM,
        lparam: LPARAM,
        config: &mut Config,
        config_path: &Path,
    ) {
        let event = lparam.0 as u32;
        match event {
            0x0204 | 0x0205 => {
                // WM_RBUTTONUP or WM_LBUTTONUP
                self.show_menu(config, config_path);
            }
            _ => {}
        }
    }

    fn show_menu(&mut self, config: &mut Config, config_path: &Path) {
        unsafe {
            // Render the menu at the DPI of the monitor the cursor is on. The
            // returned context is restored once the menu closes so the rest of
            // the process keeps its default awareness.
            let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

            let _ = SetForegroundWindow(self.hwnd).ok();
            let menu = CreatePopupMenu().unwrap();

            add_menu_item(menu, 0, ID_ENABLED, w!("Enabled"), config.enabled);
            add_menu_item(
                menu,
                1,
                ID_START_BOOT,
                w!("Start at boot"),
                config.start_at_boot,
            );
            add_separator(menu, 2);
            add_menu_item(menu, 3, ID_OPEN_CONFIG, w!("Open config folder"), false);
            add_menu_item(menu, 4, ID_QUIT, w!("Quit"), false);

            let mut pt = Default::default();
            let _ = GetCursorPos(&mut pt).ok();
            let cmd = TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY | TPM_LEFTALIGN,
                pt.x,
                pt.y,
                Some(0),
                self.hwnd,
                None,
            )
            .as_bool() as u32;
            let _ = DestroyMenu(menu).ok();
            SetThreadDpiAwarenessContext(previous);

            match cmd {
                ID_ENABLED => {
                    config.enabled = !config.enabled;
                    let _ = config.save(config_path);
                    info!("Enabled toggled to {}", config.enabled);
                }
                ID_START_BOOT => {
                    let target = !config.start_at_boot;
                    // Only persist the new value if the task was actually
                    // updated, so config never drifts from scheduler state.
                    match scheduler::sync_startup(target) {
                        Ok(()) => {
                            config.start_at_boot = target;
                            let _ = config.save(config_path);
                            info!("Start at boot toggled to {target}");
                        }
                        Err(e) => {
                            error!("Failed to update startup task: {e}");
                        }
                    }
                }
                ID_OPEN_CONFIG => {
                    if let Some(parent) = config_path.parent() {
                        let _ = Command::new("explorer.exe").arg(parent).spawn();
                    }
                }
                ID_QUIT => {
                    let _ = config.save(config_path);
                    remove_icon(self.hwnd);
                    PostQuitMessage(0);
                }
                _ => {}
            }
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            remove_icon(self.hwnd);
            let _ = DestroyIcon(self.icon).ok();
        }
    }
}

unsafe fn add_menu_item(
    menu: HMENU,
    pos: u32,
    id: u32,
    text: windows::core::PCWSTR,
    checked: bool,
) {
    let flags = MF_BYPOSITION | MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED };
    let _ = InsertMenuW(menu, pos, flags, id as usize, text).ok();
}

unsafe fn add_separator(menu: HMENU, pos: u32) {
    let _ = InsertMenuW(menu, pos, MF_BYPOSITION | MF_SEPARATOR, 0, w!("")).ok();
}

unsafe fn remove_icon(hwnd: HWND) {
    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    };
    let _ = Shell_NotifyIconW(NIM_DELETE, &nid).ok();
}

fn copy_tooltip(dst: &mut [u16], src: windows::core::PCWSTR) {
    unsafe {
        let len = wcslen(src.0);
        let slice = std::slice::from_raw_parts(src.0, len + 1);
        let copy_len = slice.len().min(dst.len());
        dst[..copy_len].copy_from_slice(&slice[..copy_len]);
    }
}

unsafe fn wcslen(ptr: *const u16) -> usize {
    let mut i = 0;
    while *ptr.add(i) != 0 {
        i += 1;
    }
    i
}
