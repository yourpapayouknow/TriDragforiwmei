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
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos, InsertMenuW, LoadIconW,
    PostQuitMessage, SetForegroundWindow, TrackPopupMenu, HICON, HMENU, IDI_APPLICATION,
    MF_BYPOSITION, MF_CHECKED, MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, TPM_LEFTALIGN,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_LBUTTONUP, WM_RBUTTONUP,
};

use crate::config::Config;
use crate::lang::{Lang, TxtKey};
use crate::scheduler;
use crate::utils;

/// Owns the notification-area icon and its context menu.
pub struct Tray {
    hwnd: HWND,
    icon: HICON,
}

/// Command identifiers returned by the context menu.
const ID_ENABLED: u32 = 1;
const ID_START_BOOT: u32 = 2;
const ID_OPEN_CONFIG: u32 = 3;
const ID_QUIT: u32 = 4;
const ID_LANG_ZH: u32 = 5;
const ID_LANG_EN: u32 = 6;

impl Tray {
    /// Adds the notification icon and registers its callback message.
    pub fn new(hwnd: HWND, msgid: u32, lang: Lang) -> Result<Self> {
        let icon = unsafe { LoadIconW(None, IDI_APPLICATION).ok().context("load icon")? };
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: msgid,
            hIcon: icon,
            szTip: [0; 128],
            ..Default::default()
        };
        cpytip(&mut nid.szTip, lang.txt(TxtKey::Tooltip));

        unsafe {
            Shell_NotifyIconW(NIM_ADD, &nid).ok()?;
        }
        Ok(Self { hwnd, icon })
    }

    /// Opens the context menu on a mouse-up notification from the icon.
    pub fn hndlevnt(
        &mut self,
        _wparam: WPARAM,
        lparam: LPARAM,
        config: &mut Config,
        cfgpath: &Path,
    ) {
        // The low word carries the mouse message; the high word holds the icon
        // id, so mask it off before matching.
        match (lparam.0 as u32) & 0xFFFF {
            WM_LBUTTONUP | WM_RBUTTONUP => self.shwmenu(config, cfgpath),
            _ => {}
        }
    }

    /// Builds, displays and acts on the context menu.
    fn shwmenu(&mut self, config: &mut Config, cfgpath: &Path) {
        unsafe {
            // Render at the DPI of the monitor the cursor is on, then restore.
            let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

            let _ = SetForegroundWindow(self.hwnd).ok();
            let menu = CreatePopupMenu().unwrap();
            let lang = config.lang;

            addmni(
                menu,
                0,
                ID_ENABLED,
                lang.txt(TxtKey::Enabled),
                config.enabled,
            );
            addmni(
                menu,
                1,
                ID_START_BOOT,
                lang.txt(TxtKey::StartBoot),
                config.start_at_boot,
            );
            addsep(menu, 2);
            addmni(menu, 3, ID_OPEN_CONFIG, lang.txt(TxtKey::OpenCfg), false);

            // Language submenu, with a check mark on the active language.
            let submenu = CreatePopupMenu().unwrap();
            addmni(
                submenu,
                0,
                ID_LANG_ZH,
                lang.txt(TxtKey::LangZh),
                lang == Lang::Zh,
            );
            addmni(
                submenu,
                1,
                ID_LANG_EN,
                lang.txt(TxtKey::LangEn),
                lang == Lang::En,
            );
            let subtext = widen(lang.txt(TxtKey::LangLabel));
            let _ = InsertMenuW(
                menu,
                4,
                MF_BYPOSITION | MF_STRING | MF_POPUP,
                submenu.0 as usize,
                windows::core::PCWSTR(subtext.as_ptr()),
            )
            .ok();

            addmni(menu, 5, ID_QUIT, lang.txt(TxtKey::Quit), false);

            let mut pt = Default::default();
            let _ = GetCursorPos(&mut pt).ok();
            // With TPM_RETURNCMD the return value is the chosen command id, not
            // a boolean; the raw value must be read or every item collapses to 1.
            let cmd = TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY | TPM_LEFTALIGN,
                pt.x,
                pt.y,
                Some(0),
                self.hwnd,
                None,
            )
            .0 as u32;
            let _ = DestroyMenu(menu).ok();
            SetThreadDpiAwarenessContext(previous);

            match cmd {
                ID_ENABLED => {
                    config.enabled = !config.enabled;
                    let _ = config.save(cfgpath);
                    info!("Enabled toggled to {}", config.enabled);
                }
                ID_START_BOOT => Self::toogleboot(config, cfgpath),
                ID_OPEN_CONFIG => {
                    if let Some(parent) = cfgpath.parent() {
                        let _ = Command::new("explorer.exe").arg(parent).spawn();
                    }
                }
                ID_LANG_ZH | ID_LANG_EN => {
                    let picked = if cmd == ID_LANG_ZH {
                        Lang::Zh
                    } else {
                        Lang::En
                    };
                    if picked != config.lang {
                        config.lang = picked;
                        let _ = config.save(cfgpath);
                        // The tooltip is baked into the icon, so update it now.
                        self.rfreshtip(picked);
                        info!("Language switched to {:?}", picked);
                    }
                }
                ID_QUIT => {
                    let _ = config.save(cfgpath);
                    rmicon(self.hwnd);
                    PostQuitMessage(0);
                }
                _ => {}
            }
        }
    }

    /// Rewrites the notification icon tooltip after a language change.
    fn rfreshtip(&self, lang: Lang) {
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: NIF_TIP,
            szTip: [0; 128],
            ..Default::default()
        };
        cpytip(&mut nid.szTip, lang.txt(TxtKey::Tooltip));
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid).ok();
        }
    }

    /// Toggles the logon task, elevating when the process is unelevated. The
    /// config is only persisted when the task change actually succeeded.
    fn toogleboot(config: &mut Config, cfgpath: &Path) {
        let target = !config.start_at_boot;
        let result = if utils::isadmin() {
            scheduler::setstp(target)
        } else {
            let flag = if target {
                "--task-enable"
            } else {
                "--task-disable"
            };
            utils::runelev(flag)
        };

        match result {
            Ok(()) => {
                config.start_at_boot = target;
                let _ = config.save(cfgpath);
                info!("Start at boot toggled to {target}");
            }
            Err(e) => error!("Failed to update startup task: {e}"),
        }
    }
}

impl Drop for Tray {
    /// Removes the notification icon and releases the icon handle.
    fn drop(&mut self) {
        unsafe {
            rmicon(self.hwnd);
            let _ = DestroyIcon(self.icon).ok();
        }
    }
}

/// Inserts a checked or unchecked menu item at a fixed position.
unsafe fn addmni(menu: HMENU, pos: u32, id: u32, text: &str, checked: bool) {
    let flags = MF_BYPOSITION | MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED };
    let wide = widen(text);
    let _ = InsertMenuW(
        menu,
        pos,
        flags,
        id as usize,
        windows::core::PCWSTR(wide.as_ptr()),
    )
    .ok();
}

/// Inserts a separator at a fixed position.
unsafe fn addsep(menu: HMENU, pos: u32) {
    let _ = InsertMenuW(menu, pos, MF_BYPOSITION | MF_SEPARATOR, 0, w!("")).ok();
}

/// Deletes the notification icon from the taskbar.
unsafe fn rmicon(hwnd: HWND) {
    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    };
    let _ = Shell_NotifyIconW(NIM_DELETE, &nid).ok();
}

/// Copies a text into a fixed-size tooltip buffer, truncating if needed.
fn cpytip(dst: &mut [u16], src: &str) {
    let wide = widen(src);
    let n = wide.len().min(dst.len());
    dst[..n].copy_from_slice(&wide[..n]);
}

/// Expands a text into a NUL-terminated UTF-16 buffer.
fn widen(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
