use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use log::info;

/// Registered name and folder of the logon task.
const TSKNAME: &str = "TriDragForIwmeiStartup";
const TSKFOLDER: &str = "\\TriDragForIwmei";

/// Full task path used by schtasks.
fn tskpath() -> String {
    format!("{TSKFOLDER}\\{TSKNAME}")
}

/// Whether the logon task is currently registered.
pub fn isstpenb() -> bool {
    Command::new("schtasks.exe")
        .args(["/Query", "/TN", &tskpath()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Brings the logon task in line with `enabled`, doing nothing when it already
/// matches so unrelated tasks are never touched.
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

/// Registers the logon task that relaunches this executable after sign-in.
fn enbstartup() -> Result<()> {
    let exe = std::env::current_exe().context("current_exe")?;
    let exestr = exe.to_string_lossy();
    // A SID is used rather than a user name because names containing spaces or
    // non-ASCII characters fail the account lookup.
    let user = crate::utils::curusrsid().context("resolve current user SID")?;

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

    // schtasks expects a UTF-16 file, so the declared encoding must match the
    // bytes actually written.
    let temp = std::env::temp_dir().join("tridragforiwmei_task.xml");
    wrutf16(&temp, &xml).context("write task xml")?;

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
        .context("run schtasks /Create")?;

    std::fs::remove_file(temp).ok();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("schtasks /Create failed: {stderr}");
    }

    info!("Startup task enabled");
    Ok(())
}

/// Removes the logon task.
fn dsbstartup() -> Result<()> {
    let output = Command::new("schtasks.exe")
        .args(["/Delete", "/TN", &tskpath(), "/F"])
        .output()
        .context("run schtasks /Delete")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("schtasks /Delete failed: {stderr}");
    }

    info!("Startup task disabled");
    Ok(())
}

/// Writes `text` as UTF-16LE with a BOM, the form schtasks reads.
fn wrutf16(path: &Path, text: &str) -> std::io::Result<()> {
    let mut bytes = Vec::with_capacity(text.len() * 2 + 2);
    bytes.extend_from_slice(&[0xFF, 0xFE]);
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(path, bytes)
}

/// Escapes the XML metacharacters in `s`.
fn xmlescp(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
