use serde::{Deserialize, Serialize};

// 界面文本的键，每个键对应一条用户可见字符串
#[derive(Debug, Clone, Copy)]
pub enum TxtKey {
    Tooltip,
    Enabled,
    StartBoot,
    OpenCfg,
    Quit,
    LangLabel,
    LangZh,
    LangEn,
}

// 界面语言，随配置持久化
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Zh,
    En,
}

// 配置无语言项时跟随系统界面语言
impl Default for Lang {
    fn default() -> Self {
        if syslang() {
            Lang::Zh
        } else {
            Lang::En
        }
    }
}

impl Lang {
    // 取指定键在当前语言下的文本，字符串为静态常量不占堆
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

// 系统界面语言是否为中文，主语言标识 0x04 即中文任意子语言
fn syslang() -> bool {
    use windows::Win32::Globalization::GetUserDefaultUILanguage;
    // 中文主语言标识
    const LANG_CHINESE: u16 = 0x04;
    let id = unsafe { GetUserDefaultUILanguage() };
    (id & 0x3FF) as u16 == LANG_CHINESE
}
