use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::lang::Lang;

// 配置文件当前版本号
const CFG_VER: i32 = 1;

// 释放拖拽按键前的最小延迟毫秒数
pub const RLS_FNG_THR_MS: u32 = 40;

// 持久化配置，字段名即 JSON 文件契约
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
    // 界面语言，旧文件中缺省时跟随系统
    #[serde(default)]
    pub lang: Lang,
    pub device_configs: HashMap<String, DevCfg>,
}

// 三指拖拽期间按住的鼠标键
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Btn {
    Left,
    Right,
    Middle,
}

// 单个触摸板的光标移动参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevCfg {
    pub cursor_move: bool,
    pub cursor_speed: f32,
    pub cursor_acceleration: f32,
}

// 配置文件默认值
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

// 单设备配置默认值
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
    // 无新输入时释放按键前的等待时长
    pub fn rlsdly(&self) -> u32 {
        if self.allow_release_and_restart {
            self.release_delay_ms.max(RLS_FNG_THR_MS)
        } else {
            RLS_FNG_THR_MS
        }
    }

    // 读入配置，首次运行时创建默认文件
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            let cfg = Config::default();
            cfg.save(path)?;
            return Ok(cfg);
        }
        let text = fs::read_to_string(path).context("读取配置失败")?;
        let mut cfg: Config = serde_json::from_str(&text).context("解析配置失败")?;
        if cfg.version != CFG_VER {
            cfg.version = CFG_VER;
        }
        Ok(cfg)
    }

    // 写出配置，必要时创建父目录
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self).context("序列化配置失败")?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("创建配置目录失败")?;
        }
        fs::write(path, text).context("写入配置失败")?;
        Ok(())
    }

    // 取指定触摸板的配置，缺省时返回默认值
    pub fn devcfg(&self, devid: &str) -> DevCfg {
        self.device_configs.get(devid).cloned().unwrap_or_default()
    }
}

// 用户数据目录下的配置文件路径
pub fn cfgpath() -> Result<PathBuf> {
    let dir = dirs::data_dir()
        .context("无法获取数据目录")?
        .join("TriDragForIwmei");
    Ok(dir.join("config.json"))
}
