use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use log::info;

// 登录任务在计划程序中的名称与目录
const TSKNAME: &str = "TriDragForIwmeiStartup";
const TSKFOLDER: &str = "\\TriDragForIwmei";

// schtasks 使用的完整任务路径
fn tskpath() -> String {
    format!("{TSKFOLDER}\\{TSKNAME}")
}

// 登录任务当前是否已注册
pub fn isstpenb() -> bool {
    Command::new("schtasks.exe")
        .args(["/Query", "/TN", &tskpath()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// 使登录任务与 enabled 一致，状态相符时不动作以免波及无关任务
pub fn setstp(enabled: bool) -> Result<()> {
    if enabled == isstpenb() {
        return Ok(());
    }
    if enabled {
        enbstartup()
    } else {
        dsbstartup()
    }
}

// 注册登录后重新拉起本程序的任务
fn enbstartup() -> Result<()> {
    let exe = std::env::current_exe().context("获取当前程序路径失败")?;
    let exestr = exe.to_string_lossy();
    // 使用 SID 而非用户名，含空格或非 ASCII 的账户名无法通过名称查找
    let user = crate::utils::curusrsid().context("获取当前用户 SID 失败")?;

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Start Three-Finger Drag at logon</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
    </Exec>
  </Actions>
</Task>"#,
        user = xmlescp(&user),
        command = xmlescp(&exestr)
    );

    // schtasks 要求 UTF-16 文件，磁盘字节需与声明编码一致
    let temp = std::env::temp_dir().join("tridragforiwmei_task.xml");
    wrutf16(&temp, &xml).context("写入任务 XML 失败")?;

    let output = Command::new("schtasks.exe")
        .args([
            "/Create",
            "/TN",
            &tskpath(),
            "/XML",
            &temp.to_string_lossy(),
            "/F",
        ])
        .output()
        .context("执行 schtasks /Create 失败")?;

    std::fs::remove_file(temp).ok();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("schtasks /Create 失败: {stderr}");
    }

    info!("Startup task enabled");
    Ok(())
}

// 删除登录任务
fn dsbstartup() -> Result<()> {
    let output = Command::new("schtasks.exe")
        .args(["/Delete", "/TN", &tskpath(), "/F"])
        .output()
        .context("执行 schtasks /Delete 失败")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("schtasks /Delete 失败: {stderr}");
    }

    info!("Startup task disabled");
    Ok(())
}

// 以 schtasks 可读的 UTF-16LE 带 BOM 格式写出文本
fn wrutf16(path: &Path, text: &str) -> std::io::Result<()> {
    let mut bytes = Vec::with_capacity(text.len() * 2 + 2);
    bytes.extend_from_slice(&[0xFF, 0xFE]);
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(path, bytes)
}

// 转义 XML 元字符
fn xmlescp(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
