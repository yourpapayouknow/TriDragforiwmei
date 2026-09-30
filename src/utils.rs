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

// 将日志重定向到用户数据目录下的文件
pub fn initlog() -> Result<()> {
    let dir = dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("TriDragForIwmei");
    fs::create_dir_all(&dir)?;
    let path = dir.join("tridragforiwmei.log");
    let file = File::create(&path).context("打开日志文件失败")?;
    WriteLogger::init(LevelFilter::Debug, LogConfig::default(), file).context("初始化日志失败")?;
    info!("Log file: {:?}", path);
    Ok(())
}

// 当前进程是否具备管理员权限
pub fn isadmin() -> bool {
    unsafe { IsUserAnAdmin().into() }
}

// 以 runas 动词重新拉起本程序并附加参数，用户取消 UAC 时返回错误
pub fn runelev(args: &str) -> Result<()> {
    let exe = std::env::current_exe().context("获取当前程序路径失败")?;
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
        ShellExecuteExW(&mut info).context("提权启动失败")?;
        if !info.hProcess.is_invalid() {
            let _ = CloseHandle(info.hProcess);
        }
    }
    Ok(())
}

// 取当前用户的安全标识符字符串，形如 S-1-5-21-...
pub fn curusrsid() -> Result<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .context("打开进程令牌失败")?;

        let result = (|| -> Result<String> {
            let mut size = 0u32;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut size);
            if size == 0 {
                anyhow::bail!("获取令牌信息长度失败");
            }

            let mut buf = vec![0u8; size as usize];
            GetTokenInformation(
                token,
                TokenUser,
                Some(buf.as_mut_ptr() as _),
                size,
                &mut size,
            )
            .context("获取令牌信息失败")?;

            let tuser = &*(buf.as_ptr() as *const TOKEN_USER);
            let mut sidstr = PWSTR::null();
            ConvertSidToStringSidW(tuser.User.Sid, &mut sidstr).context("转换 SID 失败")?;

            let sid = sidstr.to_string().context("SID 不是合法 UTF-8")?;
            let _ = LocalFree(Some(HLOCAL(sidstr.0 as _)));
            Ok(sid)
        })();

        let _ = CloseHandle(token);
        result
    }
}

// 占用具名互斥体，确保只有一个实例在运行
pub fn snglinst() -> Result<SnglInst> {
    let name = w!("Global\\TriDragForIwmeiSingleInstance");
    unsafe {
        let handle = CreateMutexW(None, true, name)?;
        if GetLastError() == WIN32_ERROR(183) {
            CloseHandle(handle)?;
            anyhow::bail!("已有实例在运行");
        }
        Ok(SnglInst(handle))
    }
}

// 实例守卫，释放时归还互斥体
pub struct SnglInst(HANDLE);

impl Drop for SnglInst {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

// 将字符串展开为以 NUL 结尾的 UTF-16 缓冲区
fn widen(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
