use serde::{Deserialize, Serialize};

/// Text keys shown in the tray menu and tooltip. Each variant maps to one
/// user-visible string, so adding a label means adding a variant and its
/// translations side by side.
#[derive(Debug, Clone, Copy)]
pub enum TxtKey {
    Tooltip,
    Enabled,
    StartBoot,
    OpenCfg,
    Quit,
    /// Label appended to show the currently selected language.
    LangLabel,
    LangZh,
    LangEn,
}

/// Interface language, persisted in the config file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Zh,
    En,
}

impl Default for Lang {
    /// Falls back to the system UI language when the config has no entry.
    fn default() -> Self {
        if syslang() {
            Lang::Zh
        } else {
            Lang::En
        }
    }
}

impl Lang {
    /// Returns the text for `key` in this language. Strings are static and live
    /// in the read-only section, so switching costs no allocation.
    pub fn txt(self, key: TxtKey) -> &'static str {
        match (self, key) {
            (Lang::Zh, TxtKey::Tooltip) => "三指拖动",
            (Lang::Zh, TxtKey::Enabled) => "启用",
            (Lang::Zh, TxtKey::StartBoot) => "开机自启",
            (Lang::Zh, TxtKey::OpenCfg) => "打开配置文件夹",
            (Lang::Zh, TxtKey::Quit) => "退出",
            (Lang::Zh, TxtKey::LangLabel) => "语言",
            (Lang::Zh, TxtKey::LangZh) => "简体中文",
            (Lang::Zh, TxtKey::LangEn) => "English",

            (Lang::En, TxtKey::Tooltip) => "Three-Finger Drag",
            (Lang::En, TxtKey::Enabled) => "Enabled",
            (Lang::En, TxtKey::StartBoot) => "Start at boot",
            (Lang::En, TxtKey::OpenCfg) => "Open config folder",
            (Lang::En, TxtKey::Quit) => "Quit",
            (Lang::En, TxtKey::LangLabel) => "Language",
            (Lang::En, TxtKey::LangZh) => "简体中文",
            (Lang::En, TxtKey::LangEn) => "English",
        }
    }
}

/// Whether the Windows user interface language is Chinese. Reads the primary
/// language identifier from the user locale without loading any resource.
fn syslang() -> bool {
    use windows::Win32::Globalization::GetUserDefaultUILanguage;
    // Primary language id 0x04 is Chinese in any sublanguage.
    const LANG_CHINESE: u16 = 0x04;
    let id = unsafe { GetUserDefaultUILanguage() };
    (id & 0x3FF) as u16 == LANG_CHINESE
}
