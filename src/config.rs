use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::lang::Lang;

/// Current on-disk settings schema version.
const CFG_VER: i32 = 1;

/// Minimum delay before releasing the drag button, in milliseconds. Precision
/// Touchpads report roughly every 10 ms, so a shorter window cannot tell a
/// finger release apart from a dropped report.
pub const RLS_FNG_THR_MS: u32 = 40;

/// Persisted settings. Field names are part of the JSON file contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub version: i32,
    pub enabled: bool,
    pub button: Btn,
    pub allow_release_and_restart: bool,
    pub release_delay_ms: u32,
    pub cursor_averaging: u32,
    pub max_finger_move_distance: f32,
    pub start_threshold: f32,
    pub stop_threshold: f32,
    pub run_elevated: bool,
    pub start_at_boot: bool,
    /// Interface language; absent files default to the system UI language.
    #[serde(default)]
    pub lang: Lang,
    pub device_configs: HashMap<String, DevCfg>,
}

/// Mouse button held during a three-finger drag.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Btn {
    Left,
    Right,
    Middle,
}

/// Per-touchpad cursor movement tuning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevCfg {
    pub cursor_move: bool,
    pub cursor_speed: f32,
    pub cursor_acceleration: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CFG_VER,
            enabled: true,
            button: Btn::Left,
            allow_release_and_restart: true,
            release_delay_ms: 500,
            cursor_averaging: 1,
            max_finger_move_distance: 0.0,
            start_threshold: 100.0,
            stop_threshold: 10.0,
            run_elevated: false,
            start_at_boot: false,
            lang: Lang::default(),
            device_configs: HashMap::new(),
        }
    }
}

impl Default for DevCfg {
    fn default() -> Self {
        Self {
            cursor_move: true,
            cursor_speed: 30.0,
            cursor_acceleration: 10.0,
        }
    }
}

impl Config {
    /// Delay before the held button releases when no input arrives. Honours the
    /// configured delay only when release-and-restart is on, otherwise falls
    /// back to the raw finger-release threshold.
    pub fn rlsdly(&self) -> u32 {
        if self.allow_release_and_restart {
            self.release_delay_ms.max(RLS_FNG_THR_MS)
        } else {
            RLS_FNG_THR_MS
        }
    }

    /// Reads settings from disk, creating a default file on first run.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            let cfg = Config::default();
            cfg.save(path)?;
            return Ok(cfg);
        }
        let text = fs::read_to_string(path).context("read config")?;
        let mut cfg: Config = serde_json::from_str(&text).context("parse config")?;
        if cfg.version != CFG_VER {
            cfg.version = CFG_VER;
        }
        Ok(cfg)
    }

    /// Writes settings to disk, creating the parent directory if needed.
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self).context("serialize config")?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("create config dir")?;
        }
        fs::write(path, text).context("write config")?;
        Ok(())
    }

    /// Returns the stored config for a touchpad, or the defaults.
    pub fn devcfg(&self, devid: &str) -> DevCfg {
        self.device_configs.get(devid).cloned().unwrap_or_default()
    }
}

/// Location of the settings file under the user's roaming data directory.
pub fn cfgpath() -> Result<PathBuf> {
    let dir = dirs::data_dir()
        .context("data_dir")?
        .join("TriDragForIwmei");
    Ok(dir.join("config.json"))
}
