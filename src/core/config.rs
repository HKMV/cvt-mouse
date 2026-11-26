//! 配置的结构、默认值与校验。
//!
//! 字段名用 camelCase：这份 JSON 既是磁盘格式，也是界面提交草稿的格式，
//! 两边同一套名字，省掉一次翻译。
//!
//! 这里**不碰文件系统**：读盘和落盘在 [`crate::infra::store`]，所以本模块的
//! 校验逻辑可以直接单测。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::curve::{Curve, DEFAULT_CURVE};

/// 变速的实现方式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// 全局钩子拦下来重注入（默认，能智能变速）。
    #[default]
    Hook,
    /// 直接改 Windows 的「每格滚几行」（兼容模式，不装钩子）。
    System,
}

/// 一份配置。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    /// 总开关。
    pub enabled: bool,
    /// 实现方式。
    pub mode: Mode,
    /// 变速曲线断点 `[[格/秒, 倍率], …]`。
    pub curve: Vec<(f32, f32)>,
    /// 按应用覆盖的倍率，键是小写的 exe 名。
    pub apps: BTreeMap<String, f32>,
    /// 完全不接管的进程（游戏、远程桌面这类）。
    pub skip: Vec<String>,
    /// 按住这些键时不改（保留原生行为：Ctrl+滚轮缩放）。
    pub pass_ctrl: bool,
    pub pass_alt: bool,
    pub pass_shift: bool,
    /// 平滑输出：一次大跳拆成若干小格按节拍发（无级感）。关掉更跟手，但会跳。
    pub smooth: bool,
    /// 兼容模式用的「每格行数」。
    pub system_lines: u32,
    /// 进兼容模式之前系统的原始行数，退出时还原。
    pub original_lines: Option<u32>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            enabled: true,
            mode: Mode::Hook,
            curve: DEFAULT_CURVE.to_vec(),
            apps: BTreeMap::new(),
            skip: Vec::new(),
            pass_ctrl: true,
            pass_alt: true,
            pass_shift: true,
            smooth: true,
            system_lines: 3,
            original_lines: None,
        }
    }
}

impl Config {
    /// 校验一份（可能是界面送来的）配置。
    ///
    /// 错误文案是给用户直接看的：说清哪个值不对、该在什么范围。
    pub fn validate(&self) -> Result<(), String> {
        if self.curve.len() < 2 {
            return Err("曲线至少要有两个断点".to_string());
        }
        for (velocity, gain) in &self.curve {
            if !velocity.is_finite() || *velocity < 0.0 || *velocity > 200.0 {
                return Err(format!("速度断点要在 0 到 200 格/秒之间，现在是 {velocity}"));
            }
            if !gain.is_finite() || *gain < 0.1 || *gain > 20.0 {
                return Err(format!("倍率要在 0.1 到 20 之间，现在是 {gain}"));
            }
        }
        for (exe, gain) in &self.apps {
            if exe.trim().is_empty() {
                return Err("按应用规则里的程序名不能为空".to_string());
            }
            if !gain.is_finite() || *gain < 0.1 || *gain > 20.0 {
                return Err(format!("{exe} 的倍率要在 0.1 到 20 之间，现在是 {gain}"));
            }
        }
        if self.mode == Mode::System && !(1..=25).contains(&self.system_lines) {
            return Err("兼容模式的每格行数要在 1 到 25 之间".to_string());
        }
        Ok(())
    }

    /// 这份配置对应的曲线。
    pub fn curve(&self) -> Curve {
        Curve::new(self.curve.clone())
    }

    /// 某个进程的倍率（没单独配置就是 1.0）。
    pub fn app_gain(&self, exe: &str) -> f32 {
        self.apps.get(&exe.to_ascii_lowercase()).copied().unwrap_or(1.0)
    }

    /// 这个进程是不是完全不接管。
    pub fn is_skipped(&self, exe: &str) -> bool {
        self.skip.iter().any(|item| item.eq_ignore_ascii_case(exe))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn gain_out_of_range_is_rejected() {
        let mut config = Config::default();
        config.curve = vec![(0.0, 1.0), (12.0, 50.0)];
        let error = config.validate().unwrap_err();
        assert!(error.contains("倍率"), "错误文案要说清是倍率：{error}");
    }

    #[test]
    fn system_mode_lines_must_be_in_range() {
        let mut config = Config::default();
        config.mode = Mode::System;
        config.system_lines = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn app_gain_lookup_ignores_case() {
        let mut config = Config::default();
        config.apps.insert("code.exe".to_string(), 2.0);
        assert_eq!(config.app_gain("CODE.EXE"), 2.0);
        assert_eq!(config.app_gain("other.exe"), 1.0);
    }
}
