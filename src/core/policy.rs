//! 滚轮决策：这一格要不要改、改成多少。
//!
//! 纯逻辑：不查键盘、不查窗口、不读时钟——环境由 [`Env`] 从外面送进来
//! （[`crate::runtime`] 那一层负责去 Win32 取）。所以这一层能在任何机器上单测，
//! 不用装钩子、不用真的滚鼠标。
//!
//! 统计（[`Stats`]）只是观测数据，单独放在 [`crate::core::stats`]。

use super::config::{Config, Mode};
use super::curve::{Accumulator, Curve, Meter};
use super::stats::Stats;

/// 一格滚轮的标准单位。
pub const NOTCH: i32 = 120;

/// `dwFlags` 里这一位表示事件是注入的（Win32 的 `LLMHF_INJECTED`）。
pub const LLMHF_INJECTED: u32 = 0x0000_0001;

/// 单个事件超过这么多单位就不像人滚出来的。
pub const MAX_SANE_UNITS: i32 = 240;

/// 速度超过这么多格/秒就不像人滚出来的。
pub const MAX_SANE_VELOCITY: f32 = 200.0;

/// 做判断需要的外部环境，由调用方填好。
#[derive(Clone, Debug, Default)]
pub struct Env {
    /// 光标下的程序（小写 exe 名）；认不出就是空串。
    pub exe: String,
    /// 目标窗口是否提权：我们的注入会被 UIPI 拦掉。
    pub target_elevated: bool,
    /// 我们自己是不是管理员。
    pub self_elevated: bool,
    /// 按住的修饰键（已经按配置过滤过：配置说不用管的就是 false）。
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// 现在的时间（毫秒），速度计要用。
    pub now_ms: u64,
}

/// 判断结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// 原样放行。
    Pass,
    /// 吃掉，什么都不发（倍率小于 1 时攒着）。
    Swallow,
    /// 吃掉，改发这么多格。
    Wheel(i32),
}

/// 决策状态：曲线、速度计、累加器、统计，以及这一轮要落的诊断行。
pub struct Policy {
    curve: Curve,
    meter: Meter,
    acc: Accumulator,
    pub stats: Stats,
    /// 这一轮该记的日志；由调用方取走再决定落到哪（`core` 不碰日志）。
    trace: Option<String>,
}

impl Policy {
    pub fn new(config: &Config) -> Policy {
        Policy {
            curve: config.curve(),
            meter: Meter::default(),
            acc: Accumulator::default(),
            stats: Stats::default(),
            trace: None,
        }
    }

    /// 换一份配置：曲线、速度窗口、累加器全部重来（换配置时旧的速度没有意义）。
    pub fn apply(&mut self, config: &Config) {
        self.curve = config.curve();
        self.reset();
    }

    /// 丢掉速度窗口和还没发完的零头（关变速、换配置时）。
    pub fn reset(&mut self) {
        self.meter = Meter::default();
        self.acc.reset();
    }

    /// 曲线上的断点数（日志用）。
    pub fn points(&self) -> usize {
        self.curve.points().len()
    }

    /// 取走这一轮要落的诊断行。
    pub fn take_trace(&mut self) -> Option<String> {
        self.trace.take()
    }

    /// 这一格要不要改、改成多少。
    pub fn decide(&mut self, config: &Config, delta: i16, env: &Env, extra: usize, flags: u32) -> Act {
        self.stats.events += 1;
        self.stats.last_delta = delta as i32;
        self.stats.last_extra = extra;
        self.stats.last_flags = flags;

        if !config.enabled || config.mode != Mode::Hook {
            return Act::Pass;
        }
        if delta == 0 {
            return Act::Pass;
        }
        if (config.pass_ctrl && env.ctrl)
            || (config.pass_alt && env.alt)
            || (config.pass_shift && env.shift)
        {
            self.stats.passed += 1;
            return Act::Pass;
        }

        // 认不出光标下的程序（受保护进程，连 OpenProcess 都打不开）：原样放行。
        // 这种窗口我们的注入同样会被拦，拦下来只会把滚轮弄死。
        if env.exe.is_empty() {
            self.stats.passed += 1;
            return Act::Pass;
        }
        // 提权窗口：注入会被 UIPI 丢掉，拦下来等于把滚轮弄死，所以原样放行。
        let blocked = env.target_elevated && !env.self_elevated;
        if config.is_skipped(&env.exe) || blocked {
            self.stats.passed += 1;
            self.stats.exe = env.exe.clone();
            return Act::Pass;
        }

        let units = delta as i32;
        let velocity = self.meter.velocity_units(env.now_ms, units);
        let gain = self.curve.gain(velocity) * config.app_gain(&env.exe);

        self.stats.velocity = velocity;
        self.stats.gain = gain;
        self.stats.exe = env.exe.clone();
        self.stats.in_units += i64::from(units.unsigned_abs());

        // 不像人滚出来的事件：原样放行，绝不放大。
        // 这条是安全阀——没有它，别的软件注入的大增量（或回灌成的环）会被我们成倍放大。
        if units.abs() > MAX_SANE_UNITS || velocity > MAX_SANE_VELOCITY {
            self.stats.passed += 1;
            self.trace = Some(format!(
                "放行（异常事件）原始={units} 速度={velocity:.0}/s 倍率={gain:.2} extra={extra:x} flags={flags:x} 目标={}",
                env.exe
            ));
            return Act::Pass;
        }

        // 倍率恰好 1：原样放行。不注入就没有额外延迟，也不会跟别的滚轮工具打架。
        if (gain - 1.0).abs() < 0.01 {
            self.stats.out_units += i64::from(units.unsigned_abs());
            self.stats.passed += 1;
            return Act::Pass;
        }

        let out = self.acc.push_units(units, gain);

        // 留证据：快滚时（或者收到非标准值）把原始数据记下来，用完就停。
        if velocity >= 8.0 || units % NOTCH != 0 {
            self.trace = Some(format!(
                "滚轮 原始={units} 速度={velocity:.0}/s 倍率={gain:.2} 输出={out} extra={extra:x} 注入={} 目标={}",
                flags & LLMHF_INJECTED != 0,
                env.exe
            ));
        }

        if out == 0 {
            self.stats.swallowed += 1;
            return Act::Swallow;
        }
        self.stats.out_units += i64::from(out.unsigned_abs());
        Act::Wheel(out / NOTCH)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一条写死的曲线：6 格/秒以下 1.0x，12 格/秒 4.0x，24 格/秒以上 8.0x。
    fn config() -> Config {
        Config {
            curve: vec![(0.0, 1.0), (6.0, 1.0), (12.0, 4.0), (24.0, 8.0)],
            ..Config::default()
        }
    }

    fn env(now_ms: u64) -> Env {
        Env {
            exe: "notepad.exe".to_string(),
            self_elevated: true,
            now_ms,
            ..Env::default()
        }
    }

    #[test]
    fn a_single_detent_is_passed_through() {
        // 一格一格滚：速度低于起点，倍率 1.0x，原样放行。
        let config = config();
        let mut policy = Policy::new(&config);
        assert_eq!(policy.decide(&config, 120, &env(1000), 0, 0), Act::Pass);
        assert_eq!(policy.stats.passed, 1);
    }

    #[test]
    fn fast_scrolling_is_scaled_up() {
        // 连滚 8 格、每 10ms 一格：速度冲过 24 格/秒，倍率到 8.0x。
        let config = config();
        let mut policy = Policy::new(&config);
        let mut last = Act::Pass;
        for index in 0..8 {
            last = policy.decide(&config, 120, &env(1000 + index * 10), 0, 0);
        }
        match last {
            Act::Wheel(notches) => assert!(notches >= 2, "快滚至少顶两格，实际 {notches}"),
            other => panic!("快滚应当变速，实际 {other:?}"),
        }
        assert!(policy.stats.gain > 4.0, "倍率应当涨上去：{}", policy.stats.gain);
        assert!(policy.stats.out_units > policy.stats.in_units);
    }

    #[test]
    fn holding_ctrl_passes_through() {
        // 保留「Ctrl+滚轮缩放」：按住 Ctrl 时不改。
        let config = config();
        let mut policy = Policy::new(&config);
        let mut held = env(1000);
        held.ctrl = true;
        for index in 0..8 {
            held.now_ms = 1000 + index * 10;
            assert_eq!(policy.decide(&config, 120, &held, 0, 0), Act::Pass);
        }
    }

    #[test]
    fn elevated_window_passes_through_when_not_admin() {
        // 提权窗口 + 我们不是管理员：注入会被 UIPI 拦，只能放行。
        let config = config();
        let mut policy = Policy::new(&config);
        let mut elevated = env(1000);
        elevated.target_elevated = true;
        elevated.self_elevated = false;
        assert_eq!(policy.decide(&config, 120, &elevated, 0, 0), Act::Pass);

        // 自己是管理员就不一样了：照常变速。
        elevated.self_elevated = true;
        let mut last = Act::Pass;
        for index in 0..8 {
            elevated.now_ms = 1000 + index * 10;
            last = policy.decide(&config, 120, &elevated, 0, 0);
        }
        assert_ne!(last, Act::Pass, "管理员权限下提权窗口也该变速");
    }

    #[test]
    fn skipped_app_passes_through() {
        let config = Config {
            skip: vec!["game.exe".to_string()],
            ..config()
        };
        let mut policy = Policy::new(&config);
        let mut skipped = env(1000);
        skipped.exe = "game.exe".to_string();
        for index in 0..8 {
            skipped.now_ms = 1000 + index * 10;
            assert_eq!(policy.decide(&config, 120, &skipped, 0, 0), Act::Pass);
        }
    }

    #[test]
    fn disabled_or_system_mode_passes_through() {
        let mut config = config();
        config.enabled = false;
        let mut policy = Policy::new(&config);
        assert_eq!(policy.decide(&config, 120, &env(1000), 0, 0), Act::Pass);

        let mut config = config;
        config.enabled = true;
        config.mode = Mode::System;
        let mut policy = Policy::new(&config);
        assert_eq!(policy.decide(&config, 120, &env(1000), 0, 0), Act::Pass);
    }

    #[test]
    fn insane_events_are_passed_through() {
        // 一次 3 格以上、或者速度离谱：绝不放大的安全阀。
        let config = config();
        let mut policy = Policy::new(&config);
        assert_eq!(policy.decide(&config, 360, &env(1000), 0, 0), Act::Pass);
    }

    #[test]
    fn per_app_gain_multiplies_the_curve() {
        let mut config = config();
        config.apps.insert("notepad.exe".to_string(), 2.0);
        let mut policy = Policy::new(&config);
        let mut last = Act::Pass;
        for index in 0..8 {
            last = policy.decide(&config, 120, &env(1000 + index * 10), 0, 0);
        }
        assert_ne!(last, Act::Pass);
        // 同一时刻的速度，倍率应当是曲线的两倍（上限 20 之内）。
        assert!(policy.stats.gain > 8.0, "按应用倍率要叠上去：{}", policy.stats.gain);
    }
}
