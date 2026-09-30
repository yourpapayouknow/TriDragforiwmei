use std::process::Command;

use anyhow::{bail, Context, Result};
use log::info;

const TASK_NAME: &str = "TriDragForIwmeiStartup";
const TASK_FOLDER: &str = "\\TriDragForIwmei";

pub fn sync_startup(enabled: bool) -> Result<()> {
    if enabled {
        enable_startup()
    } else {
        disable_startup()
    }
}

fn enable_startup() -> Result<()> {
    let exe = std::env::current_exe().context("current_exe")?;
    let exe_str = exe.to_string_lossy();

    // Ensure any old task is removed first.
    let _ = disable_startup();

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Start Three-Finger Drag at logon</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
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
      <Command>{}</Command>
    </Exec>
  </Actions>
</Task>"#,
        xml_escape(&exe_str)
    );

    let temp = std::env::temp_dir().join("tridragforiwmei_task.xml");
    std::fs::write(&temp, xml).context("write task xml")?;

    let output = Command::new("schtasks.exe")
        .args([
            "/Create",
            "/TN",
            &format!("{}\\{}", TASK_FOLDER, TASK_NAME),
            "/XML",
            &temp.to_string_lossy(),
            "/F",
        ])
        .output()
        .context("run schtasks /Create")?;

    std::fs::remove_file(temp).ok();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("schtasks /Create failed: {}", stderr);
    }

    info!("Startup task enabled");
    Ok(())
}

fn disable_startup() -> Result<()> {
    let output = Command::new("schtasks.exe")
        .args(["/Delete", "/TN", &format!("{}\\{}", TASK_FOLDER, TASK_NAME), "/F"])
        .output()
        .context("run schtasks /Delete")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Treat "does not exist" as success.
        if !stderr.contains("cannot find") && !stderr.contains("0x80070002") {
            bail!("schtasks /Delete failed: {}", stderr);
        }
    }

    info!("Startup task disabled");
    Ok(())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
