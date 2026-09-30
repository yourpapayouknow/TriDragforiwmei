use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use log::info;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos, InsertMenuW, LoadIconW,
    SetForegroundWindow, TrackPopupMenu, HICON, IDI_APPLICATION, MF_BYPOSITION, MF_CHECKED,
    MF_SEPARATOR, MF_STRING, MF_UNCHECKED, TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD,
    TPM_RIGHTBUTTON,
};

use crate::config::Config;
use crate::scheduler;

pub struct Tray {
    hwnd: HWND,
    icon: HICON,
    msg_id: u32,
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
        let tip = w!("Three-Finger Drag");
        unsafe {
            let tip_slice: &[u16] = std::slice::from_raw_parts(tip.0, wcslen(tip.0) + 1);
            let len = tip_slice.len().min(nid.szTip.len());
            nid.szTip[..len].copy_from_slice(&tip_slice[..len]);
            Shell_NotifyIconW(NIM_ADD, &mut nid).ok()?;
        }
        Ok(Self { hwnd, icon, msg_id })
    }

    pub fn handle_event(&mut self, _wparam: WPARAM, lparam: LPARAM, config: &mut Config, config_path: &Path) {
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
            SetForegroundWindow(self.hwnd).ok();
            let menu = CreatePopupMenu().unwrap();

            let enabled_text = w!("Enabled");
            InsertMenuW(
                menu,
                0,
                MF_BYPOSITION | MF_STRING | if config.enabled { MF_CHECKED } else { MF_UNCHECKED },
                ID_ENABLED as usize,
                enabled_text,
            )
            .ok();

            let boot_text = w!("Start at boot");
            InsertMenuW(
                menu,
                1,
                MF_BYPOSITION | MF_STRING | if config.start_at_boot { MF_CHECKED } else { MF_UNCHECKED },
                ID_START_BOOT as usize,
                boot_text,
            )
            .ok();

            InsertMenuW(
                menu,
                2,
                MF_BYPOSITION | MF_SEPARATOR,
                0,
                w!(""),
            )
            .ok();

            let open_text = w!("Open config folder");
            InsertMenuW(
                menu,
                3,
                MF_BYPOSITION | MF_STRING,
                ID_OPEN_CONFIG as usize,
                open_text,
            )
            .ok();

            let quit_text = w!("Quit");
            InsertMenuW(
                menu,
                4,
                MF_BYPOSITION | MF_STRING,
                ID_QUIT as usize,
                quit_text,
            )
            .ok();

            let mut pt = Default::default();
            GetCursorPos(&mut pt).ok();
            let cmd = TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY | TPM_LEFTALIGN,
                pt.x,
                pt.y,
                Some(0),
                self.hwnd,
                None,
            ).as_bool() as u32;
            DestroyMenu(menu).ok();

            let cmd_id = cmd;
            match cmd_id {
                ID_ENABLED => {
                    config.enabled = !config.enabled;
                    let _ = config.save(config_path);
                    info!("Enabled toggled to {}", config.enabled);
                }
                ID_START_BOOT => {
                    config.start_at_boot = !config.start_at_boot;
                    let _ = scheduler::sync_startup(config.start_at_boot);
                    let _ = config.save(config_path);
                    info!("Start at boot toggled to {}", config.start_at_boot);
                }
                ID_OPEN_CONFIG => {
                    let _ = config_path.parent().map(|p| {
                        Command::new("explorer.exe").arg(p).spawn()
                    });
                }
                ID_QUIT => {
                    let _ = config.save(config_path);
                    let mut nid = NOTIFYICONDATAW {
                        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                        hWnd: self.hwnd,
                        uID: 1,
                        ..Default::default()
                    };
                    Shell_NotifyIconW(NIM_DELETE, &mut nid).ok();
                    windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
                }
                _ => {}
            }
        }
    }
}

fn wcslen(ptr: *const u16) -> usize {
    unsafe {
        let mut i = 0;
        while *ptr.add(i) != 0 {
            i += 1;
        }
        i
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            let mut nid = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: self.hwnd,
                uID: 1,
                ..Default::default()
            };
            Shell_NotifyIconW(NIM_DELETE, &mut nid).ok();
            DestroyIcon(self.icon).ok();
        }
    }
}
