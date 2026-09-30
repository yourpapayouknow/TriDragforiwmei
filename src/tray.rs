use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use log::{error, info};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetCursorPos, InsertMenuW, LoadImageW, PostQuitMessage,
    SetForegroundWindow, TrackPopupMenu, HICON, HMENU, IMAGE_ICON, LR_DEFAULTSIZE, LR_SHARED,
    MF_BYPOSITION, MF_CHECKED, MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, TPM_LEFTALIGN,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_LBUTTONUP, WM_RBUTTONUP,
};

use crate::config::Config;
use crate::lang::{Lang, TxtKey};
use crate::scheduler;
use crate::utils;

// 持有通知区图标及其右键菜单
pub struct Tray {
    hwnd: HWND,
}

// 右键菜单返回的命令标识
const ID_ENABLED: u32 = 1;
const ID_START_BOOT: u32 = 2;
const ID_OPEN_CONFIG: u32 = 3;
const ID_QUIT: u32 = 4;
const ID_LANG_ZH: u32 = 5;
const ID_LANG_EN: u32 = 6;

impl Tray {
    // 添加通知图标并注册其回调消息
    pub fn new(hwnd: HWND, msgid: u32, lang: Lang) -> Result<Self> {
        let icon = ldpicon()?;
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
        Ok(Self { hwnd })
    }

    // 收到图标抬起消息时弹出右键菜单
    pub fn hndlevnt(
        &mut self,
        _wparam: WPARAM,
        lparam: LPARAM,
        config: &mut Config,
        cfgpath: &Path,
    ) {
        // 低字节为鼠标消息、高字节为图标标识，匹配前先掩去高字节
        match (lparam.0 as u32) & 0xFFFF {
            WM_LBUTTONUP | WM_RBUTTONUP => self.shwmenu(config, cfgpath),
            _ => {}
        }
    }

    // 构建、显示并响应右键菜单
    fn shwmenu(&mut self, config: &mut Config, cfgpath: &Path) {
        unsafe {
            // 按光标所在显示器的 DPI 渲染菜单，结束后恢复原上下文
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

            // 语言子菜单，当前语言带勾选标记
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
            // 使用 TPM_RETURNCMD 时返回值是命令标识而非布尔，须取原值否则各项都会塌缩为 1
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
                        // 提示文字绑定在图标上，需立即刷新
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

    // 切换语言后重写通知图标提示文字
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

    // 切换登录任务，进程未提权时先提权
    // 仅在任务改动真正成功后写入配置
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
    // 从任务栏移除通知图标，图标句柄为系统共享资源

    fn drop(&mut self) {
        unsafe {
            rmicon(self.hwnd);
        }
    }
}

// 加载以资源号 1 嵌入的程序图标，该图标为共享资源

fn ldpicon() -> Result<HICON> {
    unsafe {
        let module = GetModuleHandleW(None).context("GetModuleHandleW")?;
        let icon = LoadImageW(
            Some(module.into()),
            windows::core::PCWSTR(1 as _),
            IMAGE_ICON,
            0,
            0,
            LR_DEFAULTSIZE | LR_SHARED,
        )
        .context("LoadImageW(icon)")?;
        Ok(HICON(icon.0))
    }
}

// 在指定位置插入带勾选状态的菜单项
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

// 在指定位置插入分隔线
unsafe fn addsep(menu: HMENU, pos: u32) {
    let _ = InsertMenuW(menu, pos, MF_BYPOSITION | MF_SEPARATOR, 0, w!("")).ok();
}

// 从任务栏删除通知图标
unsafe fn rmicon(hwnd: HWND) {
    let nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    };
    let _ = Shell_NotifyIconW(NIM_DELETE, &nid).ok();
}

// 将文本复制进固定长度的提示缓冲区，超出则截断
fn cpytip(dst: &mut [u16], src: &str) {
    let wide = widen(src);
    let n = wide.len().min(dst.len());
    dst[..n].copy_from_slice(&wide[..n]);
}

// 将文本展开为以 NUL 结尾的 UTF-16 缓冲区
fn widen(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
