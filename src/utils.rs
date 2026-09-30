use std::fs::{self, File};

use anyhow::{Context, Result};
use log::info;
use simplelog::{Config as LogConfig, LevelFilter, WriteLogger};
use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, WIN32_ERROR};
use windows::Win32::System::Threading::CreateMutexW;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn pcwstr(s: &str) -> Vec<u16> {
    wide(s)
}

pub fn init_logging() -> Result<()> {
    let log_dir = dirs::data_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join("TriDragForIwmei");
    fs::create_dir_all(&log_dir)?;
    let log_path = log_dir.join("tridragforiwmei.log");
    let file = File::create(&log_path).context("open log file")?;
    WriteLogger::init(LevelFilter::Debug, LogConfig::default(), file)
        .context("init logger")?;
    info!("Log file: {:?}", log_path);
    Ok(())
}

pub fn single_instance() -> Result<SingleInstanceGuard> {
    let name = w!("Global\\TriDragForIwmeiSingleInstance");
    unsafe {
        let handle = CreateMutexW(None, true, name)?;
        if GetLastError() == WIN32_ERROR(183) {
            // ERROR_ALREADY_EXISTS
            CloseHandle(handle)?;
            anyhow::bail!("Another instance is already running");
        }
        Ok(SingleInstanceGuard(handle))
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

#[allow(dead_code)]
pub fn data_dir() -> std::path::PathBuf {
    dirs::data_dir().unwrap_or_else(|| std::env::temp_dir())
}
