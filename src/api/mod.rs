//! nsh 边界：界面（`ui/`）能调的每一个方法。
//!
//! 约定和 tun2proxy-gui 那套一致：
//!
//! * 读数据返回**一份 JSON 文本**；
//! * 动作返回 `{"ok":true}` 或 `{"ok":false,"error":"…"}`，错误文案已经能直接
//!   显示给用户。
//!
//! 界面只认这些方法，不碰 Win32。注册（`#[native_module]`）留在 crate 根的
//! `lib.rs`，因为那是 nsh 的硬约束；实现全在这里，`lib.rs` 才能只剩几行。

use nsh_core::prelude::*;
use serde_json::json;

use crate::core::config::{Config, Mode};
use crate::core::curve::{Accumulator, Meter};
use crate::infra::{log, store};
use crate::runtime;
use crate::sys::win32;

/// 程序版本。
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 后端接口（JS 侧的名字是 `Backend`）。
#[module]
pub struct Backend;

fn ok() -> String {
    json!({ "ok": true }).to_string()
}

fn fail(error: impl Into<String>) -> String {
    json!({ "ok": false, "error": error.into() }).to_string()
}

/// 权限不足时给界面的一句话；权限够时是空串。
fn hint() -> String {
    if win32::is_elevated() {
        return String::new();
    }
    "当前不是管理员权限：提权窗口（任务管理器这类）里的滚轮保持原生，不会变速。"
        .to_string()
}

/// 把一份配置落到系统上并生效。
fn apply_config(mut config: Config) -> Result<(), String> {
    config.validate()?;
    match config.mode {
        Mode::System => {
            if config.original_lines.is_none() {
                config.original_lines = Some(win32::wheel_lines());
            }
            if !win32::set_wheel_lines(config.system_lines) {
                return Err("改系统滚轮行数失败（SystemParametersInfo 返回 0）".to_string());
            }
            log::info(format!(
                "兼容模式：系统每格 {} 行（原值 {:?}）",
                config.system_lines, config.original_lines
            ));
        }
        Mode::Hook => {
            // 从兼容模式切回来：把系统那个值还原，否则会和钩子叠加。
            if let Some(original) = config.original_lines.take() {
                win32::set_wheel_lines(original);
                log::info(format!("已还原系统每格 {original} 行"));
            }
        }
    }
    store::save_config(&config)?;
    runtime::reload(config);
    Ok(())
}

#[module]
impl Backend {
    /// 界面启动时调一次：配置、权限、路径、系统当前的每格行数。
    pub fn boot() -> String {
        log::start();
        let config = store::load_config();
        // 门槛要在下面这几行之前落地，否则启动时的记录会按默认级别走。
        log::set_levels(config.log_level, config.log_file_level);
        log::info(format!(
            "CvtMouse {VERSION} 启动（{}）",
            if win32::is_elevated() { "管理员权限" } else { "普通权限" }
        ));
        if config.mode == Mode::System {
            if !win32::set_wheel_lines(config.system_lines) {
                log::warn("兼容模式：改系统滚轮行数失败");
            }
        }
        runtime::install(config.clone());
        json!({
            "config": config,
            "meta": {
                "version": VERSION,
                "elevated": win32::is_elevated(),
                "hint": hint(),
                "configPath": store::config_path().to_string_lossy(),
                "logDir": log::dir().to_string_lossy(),
                "systemLines": win32::wheel_lines(),
            }
        })
        .to_string()
    }

    /// 这一刻的实时状态：开关、钩子、统计、最近日志。
    pub fn snapshot() -> String {
        let config = runtime::config();
        let stats = runtime::stats();
        json!({
            "enabled": config.enabled,
            "mode": config.mode,
            "installed": runtime::installed(),
            "exe": stats.exe,
            "appGain": config.app_gain(&stats.exe),
            "stats": {
                "events": stats.events,
                "inUnits": stats.in_units,
                "outUnits": stats.out_units,
                "passed": stats.passed,
                "swallowed": stats.swallowed,
                "velocity": stats.velocity,
                "gain": stats.gain,
                "lastDelta": stats.last_delta,
                "injectFailed": runtime::inject_failed(),
            },
            "logs": log::recent(),
        })
        .to_string()
    }

    /// 保存一份草稿配置（界面把整份 JSON 送回来）。
    pub fn save(draft: String) -> String {
        match serde_json::from_str::<Config>(&draft) {
            Ok(config) => match apply_config(config) {
                Ok(()) => ok(),
                Err(error) => fail(error),
            },
            Err(error) => fail(format!("配置格式不对：{error}")),
        }
    }

    /// 总开关。
    pub fn set_enabled(on: bool) -> String {
        runtime::set_enabled(on);
        ok()
    }

    /// 退出前调用：让钩子线程的消息循环收到 `WM_QUIT` 并返回。
    ///
    /// 不调用的话，那个线程会一直卡在 `GetMessageW` 里，进程关不干净。
    pub fn shutdown() -> String {
        runtime::shutdown();
        log::info("钩子已卸载");
        ok()
    }

    /// 光标下那个窗口的程序名（「把当前程序加进列表」用）。
    pub fn foreground() -> String {
        json!({ "exe": win32::exe_under_cursor() }).to_string()
    }

    /// 清空内存里的日志（磁盘文件保留）。
    pub fn clear_logs() -> String {
        log::clear();
        ok()
    }

    /// 以管理员身份重新拉起自己。成功后界面应当退出，否则会有两个窗口。
    pub fn relaunch_elevated() -> String {
        if win32::is_elevated() {
            return fail("已经是管理员权限了");
        }
        if win32::relaunch_elevated() {
            log::info("已请求以管理员身份重启");
            ok()
        } else {
            fail("提权被取消，或者启动失败")
        }
    }

    /// 系统当前的「每格滚几行」。
    pub fn system_lines() -> String {
        json!({ "lines": win32::wheel_lines() }).to_string()
    }

    /// 直接把系统行数改成 `lines`（不走钩子的兼容手段）。
    pub fn set_system_lines(lines: u32) -> String {
        if !(1..=25).contains(&lines) {
            return fail("每格行数要在 1 到 25 之间");
        }
        if win32::set_wheel_lines(lines) {
            log::info(format!("系统每格行数改成 {lines}"));
            ok()
        } else {
            fail("改系统滚轮行数失败")
        }
    }

    /// 自检：拿当前曲线跑一遍确定的输入，把每一步的倍率与输出格数摊开。
    ///
    /// 不碰真实鼠标，所以可以在界面里随时点，也是「曲线到底怎么算的」的证据。
    pub fn self_test() -> String {
        let config = runtime::config();
        let curve = config.curve();

        let probes: Vec<f32> = vec![3.3, 6.0, 9.0, 12.0, 18.0, 24.0, 33.0];
        let curve_rows: Vec<_> = probes
            .iter()
            .map(|velocity| json!({ "velocity": velocity, "gain": curve.gain(*velocity) }))
            .collect();

        // 模拟：每 40ms 滚一格，连滚 12 格。
        let mut meter = Meter::default();
        let mut acc = Accumulator::default();
        let mut steps = Vec::new();
        let mut out_total = 0;
        let mut at = 1_000u64;
        for _ in 0..12 {
            at += 40;
            let velocity = meter.velocity(at, 1);
            let gain = curve.gain(velocity);
            let out = acc.push(1, gain);
            out_total += out;
            steps.push(json!({ "velocity": velocity, "gain": gain, "out": out }));
        }

        log::info(format!(
            "自检：连滚 12 格（40ms 一格）→ 输出 {out_total} 格"
        ));
        json!({
            "curve": curve_rows,
            "burst": { "detents": 12, "intervalMs": 40, "out": out_total },
            "steps": steps,
        })
        .to_string()
    }
}
