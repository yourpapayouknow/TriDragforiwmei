use std::fs::{self, File};

use anyhow::{Context, Result};
use log::info;
use simplelog::{Config as LogConfig, LevelFilter, WriteLogger};
use windows::core::{w, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, HANDLE, HLOCAL, WIN32_ERROR,
};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::{
    IsUserAnAdmin, ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Redirects logging to a file under the user's roaming data directory.
pub fn initlog() -> Result<()> {
    let dir = dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("TriDragForIwmei");
    fs::create_dir_all(&dir)?;
    let path = dir.join("tridragforiwmei.log");
    let file = File::create(&path).context("open log file")?;
    WriteLogger::init(LevelFilter::Debug, LogConfig::default(), file).context("init logger")?;
    info!("Log file: {:?}", path);
    Ok(())
}

/// Whether this process currently holds administrator rights.
pub fn isadmin() -> bool {
    unsafe { IsUserAnAdmin().into() }
}

/// Relaunches this executable elevated with the shell "runas" verb, passing
/// `args`. Returns an error when the user cancels the UAC prompt so the caller
/// can leave its state unchanged.
pub fn runelev(args: &str) -> Result<()> {
    let exe = std::env::current_exe().context("current_exe")?;
    let exewide = widen(&exe.to_string_lossy());
    let argswide = widen(args);

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: windows::core::PCWSTR(exewide.as_ptr()),
        lpParameters: windows::core::PCWSTR(argswide.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };

    unsafe {
        ShellExecuteExW(&mut info).context("ShellExecuteExW(runas)")?;
        if !info.hProcess.is_invalid() {
            let _ = CloseHandle(info.hProcess);
        }
    }
    Ok(())
}

/// Returns the current user's SID string, e.g. "S-1-5-21-...".
pub fn curusrsid() -> Result<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .context("OpenProcessToken")?;

        let result = (|| -> Result<String> {
            let mut size = 0u32;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut size);
            if size == 0 {
                anyhow::bail!("GetTokenInformation returned no size");
            }

            let mut buf = vec![0u8; size as usize];
            GetTokenInformation(
                token,
                TokenUser,
                Some(buf.as_mut_ptr() as _),
                size,
                &mut size,
            )
            .context("GetTokenInformation")?;

            let tuser = &*(buf.as_ptr() as *const TOKEN_USER);
            let mut sidstr = PWSTR::null();
            ConvertSidToStringSidW(tuser.User.Sid, &mut sidstr)
                .context("ConvertSidToStringSidW")?;

            let sid = sidstr.to_string().context("SID is not valid UTF-8")?;
            let _ = LocalFree(Some(HLOCAL(sidstr.0 as _)));
            Ok(sid)
        })();

        let _ = CloseHandle(token);
        result
    }
}

/// Takes ownership of a named mutex so only one instance can run.
pub fn snglinst() -> Result<SnglInst> {
    let name = w!("Global\\TriDragForIwmeiSingleInstance");
    unsafe {
        let handle = CreateMutexW(None, true, name)?;
        if GetLastError() == WIN32_ERROR(183) {
            CloseHandle(handle)?;
            anyhow::bail!("Another instance is already running");
        }
        Ok(SnglInst(handle))
    }
}

/// Releases the single-instance mutex when dropped.
pub struct SnglInst(HANDLE);

impl Drop for SnglInst {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Expands a string into a NUL-terminated UTF-16 buffer.
fn widen(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
