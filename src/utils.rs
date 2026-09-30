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
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Logging level. Kept at Debug for diagnostics; the hot per-report path uses
/// trace!, so this still produces no steady disk writes during normal use.
pub fn init_logging() -> Result<()> {
    let log_dir = dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("TriDragForIwmei");
    fs::create_dir_all(&log_dir)?;
    let log_path = log_dir.join("tridragforiwmei.log");
    let file = File::create(&log_path).context("open log file")?;
    WriteLogger::init(LevelFilter::Debug, LogConfig::default(), file).context("init logger")?;
    info!("Log file: {:?}", log_path);
    Ok(())
}

pub fn single_instance() -> Result<SingleInstanceGuard> {
    let name = w!("Global\\TriDragForIwmeiSingleInstance");
    unsafe {
        let handle = CreateMutexW(None, true, name)?;
        if GetLastError() == WIN32_ERROR(183) {
            CloseHandle(handle)?;
            anyhow::bail!("Another instance is already running");
        }
        Ok(SingleInstanceGuard(handle))
    }
}

/// Whether this process currently holds administrator rights.
pub fn is_admin() -> bool {
    unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().into() }
}

/// Relaunches this executable elevated via the shell "runas" verb, passing
/// `args` and waiting for the child to exit. Returns an error if the user
/// cancels the UAC prompt so the caller can leave its state unchanged.
///
/// Using ShellExecuteExW directly avoids spawning a PowerShell host, which
/// matters because this is the only elevation path the app needs.
pub fn run_elevated(args: &str) -> Result<()> {
    let exe = std::env::current_exe().context("current_exe")?;
    let exe_wide = wide(&exe.to_string_lossy());
    let args_wide = wide(args);
    let verb = w!("runas");

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: verb,
        lpFile: windows::core::PCWSTR(exe_wide.as_ptr()),
        lpParameters: windows::core::PCWSTR(args_wide.as_ptr()),
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

/// Expands `line` to a NUL-terminated UTF-16 buffer for wide-char Win32 calls.
fn wide(line: &str) -> Vec<u16> {
    line.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Returns the current user's SID as a string, e.g. "S-1-5-21-...".
/// Task Scheduler accepts a SID for <UserId>, which avoids ambiguity around
/// spaced or localised account names.
pub fn current_user_sid() -> Result<String> {
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

            let token_user = &*(buf.as_ptr() as *const TOKEN_USER);
            let mut sid_str = PWSTR::null();
            ConvertSidToStringSidW(token_user.User.Sid, &mut sid_str)
                .context("ConvertSidToStringSidW")?;

            let sid = sid_str.to_string().context("SID is not valid UTF-8")?;
            let _ = LocalFree(Some(HLOCAL(sid_str.0 as _)));
            Ok(sid)
        })();

        let _ = CloseHandle(token);
        result
    }
}

pub struct SingleInstanceGuard(HANDLE);

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
