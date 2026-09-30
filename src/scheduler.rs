use std::process::Command;

use anyhow::{bail, Context, Result};
use log::info;

const TASK_NAME: &str = "TriDragForIwmeiStartup";
const TASK_FOLDER: &str = "\\TriDragForIwmei";

fn task_path() -> String {
    format!("{TASK_FOLDER}\\{TASK_NAME}")
}

/// Current state of the startup task.
pub fn is_startup_enabled() -> bool {
    Command::new("schtasks.exe")
        .args(["/Query", "/TN", &task_path()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Brings the startup task in line with `enabled`, doing nothing when it is
/// already in the desired state so unrelated tasks are never touched.
pub fn set_startup(enabled: bool) -> Result<()> {
    if enabled == is_startup_enabled() {
        return Ok(());
    }
    if enabled {
        enable_startup()
    } else {
        disable_startup()
    }
}

fn enable_startup() -> Result<()> {
    let exe = std::env::current_exe().context("current_exe")?;
    let exe_str = exe.to_string_lossy();
    let user = crate::utils::current_user_sid().context("resolve current user SID")?;

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
        user = xml_escape(&user),
        command = xml_escape(&exe_str)
    );

    // schtasks expects a UTF-16 file for /XML; write it accordingly so the
    // declared encoding matches the bytes on disk.
    let temp = std::env::temp_dir().join("tridragforiwmei_task.xml");
    write_utf16(&temp, &xml).context("write task xml")?;

    let output = Command::new("schtasks.exe")
        .args([
            "/Create",
            "/TN",
            &task_path(),
            "/XML",
            &temp.to_string_lossy(),
            "/F",
        ])
        .output()
        .context("run schtasks /Create")?;

    std::fs::remove_file(temp).ok();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("schtasks /Create failed: {stderr}");
    }

    info!("Startup task enabled");
    Ok(())
}

fn disable_startup() -> Result<()> {
    let output = Command::new("schtasks.exe")
        .args(["/Delete", "/TN", &task_path(), "/F"])
        .output()
        .context("run schtasks /Delete")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("schtasks /Delete failed: {stderr}");
    }

    info!("Startup task disabled");
    Ok(())
}

/// Encodes `text` as UTF-16LE with a BOM, which is what schtasks /XML reads.
fn write_utf16(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let mut bytes = Vec::with_capacity(text.len() * 2 + 2);
    bytes.extend_from_slice(&[0xFF, 0xFE]); // little-endian BOM
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(path, bytes)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
