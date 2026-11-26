//! `config.json` 的路径与读写。
//!
//! 落在 `%APPDATA%\CvtMouse\config.json`（取不到 `APPDATA` 就退到临时目录）。
//! 结构、默认值和校验在 [`crate::core::config`]，这里只管文件。

use std::path::PathBuf;

use crate::core::config::Config;

/// 配置文件路径。
pub fn config_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("CvtMouse").join("config.json")
}

/// 读盘；文件不在、或者读坏了都退回默认值（不覆盖磁盘上那份，交给用户改）。
pub fn load_config() -> Config {
    match std::fs::read_to_string(config_path()) {
        Ok(text) => match serde_json::from_str::<Config>(&text) {
            Ok(config) => config,
            Err(error) => {
                super::log::warn(format!("配置解析失败，先用默认值：{error}"));
                Config::default()
            }
        },
        Err(_) => Config::default(),
    }
}

/// 落盘。目录不存在就建。
pub fn save_config(config: &Config) -> Result<(), String> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| format!("建目录失败：{error}"))?;
    }
    let text = serde_json::to_string_pretty(config).map_err(|error| format!("序列化失败：{error}"))?;
    std::fs::write(&path, text).map_err(|error| format!("写文件失败：{error}"))
}
