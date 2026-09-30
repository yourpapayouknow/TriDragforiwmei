use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const CONFIG_VERSION: i32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub version: i32,
    pub enabled: bool,
    pub button: MouseButton,
    pub allow_release_and_restart: bool,
    pub release_delay_ms: u32,
    pub cursor_averaging: u32,
    pub max_finger_move_distance: f32,
    pub start_threshold: f32,
    pub stop_threshold: f32,
    pub run_elevated: bool,
    pub start_at_boot: bool,
    pub device_configs: HashMap<String, DeviceConfig>,
    #[serde(skip)]
    pub current_device_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    pub cursor_move: bool,
    pub cursor_speed: f32,
    pub cursor_acceleration: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            enabled: true,
            button: MouseButton::Left,
            allow_release_and_restart: true,
            release_delay_ms: 500,
            cursor_averaging: 1,
            max_finger_move_distance: 0.0,
            start_threshold: 100.0,
            stop_threshold: 10.0,
            run_elevated: false,
            start_at_boot: false,
            device_configs: HashMap::new(),
            current_device_id: "default".to_string(),
        }
    }
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            cursor_move: true,
            cursor_speed: 30.0,
            cursor_acceleration: 10.0,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            let cfg = Config::default();
            cfg.save(path)?;
            return Ok(cfg);
        }
        let text = fs::read_to_string(path).context("read config")?;
        let mut cfg: Config = serde_json::from_str(&text).context("parse config")?;
        if cfg.version != CONFIG_VERSION {
            cfg.version = CONFIG_VERSION;
        }
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self).context("serialize config")?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("create config dir")?;
        }
        fs::write(path, text).context("write config")?;
        Ok(())
    }

    pub fn device_config(&self, device_id: &str) -> DeviceConfig {
        self.device_configs
            .get(device_id)
            .cloned()
            .unwrap_or_default()
    }
}

pub fn config_path() -> Result<PathBuf> {
    let dir = dirs::data_dir()
        .context("data_dir")?
        .join("TriDragForIwmei");
    Ok(dir.join("config.json"))
}
