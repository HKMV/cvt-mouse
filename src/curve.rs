//! 速度 → 倍率，以及「攒够一格才发」的累加器。
//!
//! 这里一行 Win32 都没有：给定（格/秒，倍率）断点和一串滚轮格数，输出的就是
//! 要注入的格数。所以它能在 `cargo test` 里被完整验证，不需要真的去滚鼠标。

/// 一格滚轮 = 120 个 wheel 单位（HID 与 Windows 的约定）。
pub const NOTCH_UNITS: i32 = 120;

/// 速度窗口：看最近这么多毫秒内滚了几格。
pub const WINDOW_MS: u64 = 300;

/// 速度窗口里最多记这么多个事件。
///
/// 触控板/高分辨率滚轮一格能拆成几十个细事件，窗口内的条数会很多；太少会把这个
/// 设备的速度算低（被截断），所以给得宽一点。
pub const MAX_HITS: usize = 512;

/// 默认曲线：慢滚不动，中速 2.5 倍，快滚 5 倍。
///
/// 断点单位是「格/秒」。一格在 300ms 窗口里算 3.3 格/秒，所以单滚一格仍然是
/// 1.0 倍——不加速这件事必须对「一格一格点」的手感无影响。
pub const DEFAULT_CURVE: [(f32, f32); 4] = [(0.0, 1.0), (6.0, 1.0), (12.0, 2.5), (24.0, 5.0)];

/// 分段线性曲线：(速度, 倍率) 断点按速度升序。
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    points: Vec<(f32, f32)>,
}

impl Default for Curve {
    fn default() -> Self {
        Curve::new(DEFAULT_CURVE.to_vec())
    }
}

impl Curve {
    /// 用给定断点建一条曲线。
    ///
    /// 非法断点（负数、NaN、倍率 <= 0）直接丢掉；一个都不剩时退回默认曲线——
    /// 宁可给一条能用的曲线，也不要让钩子在半路上除以零。
    pub fn new(mut points: Vec<(f32, f32)>) -> Curve {
        points.retain(|(v, g)| v.is_finite() && g.is_finite() && *v >= 0.0 && *g > 0.0);
        points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        if points.is_empty() {
            points = DEFAULT_CURVE.to_vec();
        }
        Curve { points }
    }

    /// 这条曲线的断点。
    pub fn points(&self) -> &[(f32, f32)] {
        &self.points
    }

    /// 某个速度下的倍率：断点之间线性插值，两头取端点值。
    pub fn gain(&self, velocity: f32) -> f32 {
        let points = &self.points;
        let first = points[0];
        let last = points[points.len() - 1];
        if velocity <= first.0 {
            return first.1;
        }
        if velocity >= last.0 {
            return last.1;
        }
        for pair in points.windows(2) {
            let (x0, g0) = pair[0];
            let (x1, g1) = pair[1];
            if velocity <= x1 {
                let span = x1 - x0;
                let t = if span > 0.0 { (velocity - x0) / span } else { 1.0 };
                return g0 + (g1 - g0) * t;
            }
        }
        last.1
    }
}

/// 速度计：最近 [`WINDOW_MS`] 内滚过的格数换算成格/秒。
///
/// 反向滚动立刻清零：往上一格、往下一格交替时，速度不该被累加成「快滚」。
#[derive(Debug)]
pub struct Meter {
    hits: Vec<(u64, i32)>,
}

impl Default for Meter {
    fn default() -> Self {
        Meter { hits: Vec::with_capacity(32) }
    }
}

impl Meter {
    /// 记一格（可以是负数），返回此刻的速度（格/秒，取绝对值）。
    pub fn velocity(&mut self, now_ms: u64, detents: i32) -> f32 {
        self.velocity_units(now_ms, detents * NOTCH_UNITS)
    }

    /// 记一次滚轮事件（单位是 wheel 单位，120 = 一格），返回此刻的速度（格/秒）。
    ///
    /// 按 wheel 单位记而不是按「格」记：细粒度滚轮（触控板、高分辨率滚轮、以及
    /// 某些鼠标快甩时改报细值的情况）一格会拆成好几个事件，按格记会把它们当成
    /// 「速度 0」。
    pub fn velocity_units(&mut self, now_ms: u64, units: i32) -> f32 {
        if let Some(&(_, last)) = self.hits.last()
            && last.signum() != units.signum()
        {
            self.hits.clear();
        }
        self.hits.retain(|(at, _)| now_ms.saturating_sub(*at) <= WINDOW_MS);
        self.hits.push((now_ms, units));
        if self.hits.len() > MAX_HITS {
            self.hits.remove(0);
        }
        let total: i64 = self.hits.iter().map(|(_, u)| i64::from(u.abs())).sum();
        // 单位 → 格 → 格/秒。
        total as f32 / NOTCH_UNITS as f32 * 1000.0 / WINDOW_MS as f32
    }
}

/// 平滑输出的「这一拍发几格」。
///
/// 积压 6 格以内一拍一格：8ms 一格，视觉上是连续的滚，而不是「跳一行」。积压更多
/// 就一拍多发几格追上——否则快滚时输出会一直落后于输入，越滚越滞后。
///
/// 不平滑（`smooth = false`）时一拍把积压全部发完：跟手，但每一拍都是一次跳。
pub fn pump_step(pending: i32, smooth: bool) -> i32 {
    if pending == 0 {
        return 0;
    }
    if !smooth {
        return pending;
    }
    ((pending.abs() + 5) / 6).max(1) * pending.signum()
}

/// 「攒够一格才发」的累加器。///
/// 倍率小于 1 时一格不够一行：这一格先记着，下次凑够再一起发。反过来倍率大于
/// 1 时一格可能顶好几行。小数部分留着，长时间滚下去不会越滚越偏。
#[derive(Debug, Default)]
pub struct Accumulator {
    acc: f32,
    last_dir: i32,
}

impl Accumulator {
    /// 进来 `detents` 格，按 `gain` 换算，返回这次要注入的格数（0 表示继续攒着）。
    pub fn push(&mut self, detents: i32, gain: f32) -> i32 {
        self.push_units(detents * NOTCH_UNITS, gain) / NOTCH_UNITS
    }

    /// 进来 `units` 个 wheel 单位，返回这次要注入的 wheel 单位数（120 的整数倍）。
    ///
    /// 为什么按单位攒而不是按格攒：鼠标送来的 `mouseData` 不一定是 120 的整数倍
    /// ——触控板、高分辨率滚轮、以及某些鼠标快甩时改报细值，都会送 ±1…±119。
    /// 按格算会把这些事件当成 0 直接丢掉（表现就是「滚得快反而不动」）；按单位
    /// 攒够 120 再发一格，细事件也能被正常放大。
    ///
    /// 发出去的永远是 120 的整数倍：`wParam` 里不足一格的分量，很多程序（EDIT
    /// 控件就是）直接按整除算，等于白送。
    pub fn push_units(&mut self, units: i32, gain: f32) -> i32 {
        if units == 0 {
            return 0;
        }
        let dir = units.signum();
        if dir != self.last_dir {
            // 换方向：上一方向的余数不能带到这一方向，否则反向的第一格会突然
            // 蹦出两行。
            self.acc = 0.0;
            self.last_dir = dir;
        }
        self.acc += units as f32 * gain;
        let notches = (self.acc / NOTCH_UNITS as f32).trunc();
        if notches == 0.0 {
            return 0;
        }
        self.acc -= notches * NOTCH_UNITS as f32;
        // 浮点余数抹一下，别在 0 附近抖动。
        if self.acc.abs() < 1e-3 {
            self.acc = 0.0;
        }
        (notches as i32) * NOTCH_UNITS
    }

    /// 清空（关掉加速、换配置时）。
    pub fn reset(&mut self) {
        self.acc = 0.0;
        self.last_dir = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn gain_holds_at_both_ends() {
        let curve = Curve::default();
        assert!(close(curve.gain(0.0), 1.0));
        assert!(close(curve.gain(3.3), 1.0));
        assert!(close(curve.gain(6.0), 1.0));
        assert!(close(curve.gain(24.0), 5.0));
        assert!(close(curve.gain(999.0), 5.0));
    }

    #[test]
    fn gain_interpolates_between_points() {
        let curve = Curve::default();
        // (6,1.0) 到 (12,2.5) 的中点。
        assert!(close(curve.gain(9.0), 1.75));
        // (12,2.5) 到 (24,5.0) 的中点。
        assert!(close(curve.gain(18.0), 3.75));
    }

    #[test]
    fn bad_points_fall_back_to_default() {
        let curve = Curve::new(vec![(0.0, 0.0), (f32::NAN, 2.0), (-1.0, 3.0)]);
        assert_eq!(curve.points(), DEFAULT_CURVE.as_slice());
    }

    #[test]
    fn single_slow_detent_is_not_accelerated() {
        let curve = Curve::default();
        let mut meter = Meter::default();
        let mut acc = Accumulator::default();
        let v = meter.velocity(1_000, 1);
        assert!(close(curve.gain(v), 1.0), "单滚一格的速度 {v} 不该加速");
        assert_eq!(acc.push(1, curve.gain(v)), 1);
    }

    #[test]
    fn fast_burst_accelerates() {
        let curve = Curve::default();
        let mut meter = Meter::default();
        let mut acc = Accumulator::default();
        // 300ms 内滚 6 格 = 20 格/秒 → 1.0 + (2.5-1.0)*(20-12)/12 之外还有一段，
        // 具体值由曲线决定，这里只断言「明显大于 1」且注入量对得上。
        let mut out = 0;
        for i in 0..6u64 {
            let v = meter.velocity(1_000 + i * 40, 1);
            out += acc.push(1, curve.gain(v));
        }
        assert!(out >= 8, "6 格快滚只输出了 {out} 格，加速没生效");
    }

    #[test]
    fn slow_mode_holds_until_whole_line() {
        let mut acc = Accumulator::default();
        assert_eq!(acc.push(1, 0.5), 0);
        assert_eq!(acc.push(1, 0.5), 1);
        assert_eq!(acc.push(1, 0.5), 0);
        assert_eq!(acc.push(1, 0.5), 1);
    }

    #[test]
    fn reversal_drops_the_remainder() {
        let mut acc = Accumulator::default();
        assert_eq!(acc.push(1, 0.5), 0);
        assert_eq!(acc.push(-1, 1.0), -1);
    }

    #[test]
    fn meter_resets_on_reversal() {
        let mut meter = Meter::default();
        for i in 0..6u64 {
            meter.velocity(1_000 + i * 40, 1);
        }
        let fast = meter.velocity(1_200, 1);
        let slow = meter.velocity(1_210, -1);
        assert!(fast > slow, "反向之后速度该掉回一格，{fast} vs {slow}");
    }

    #[test]
    fn fine_grained_units_are_accelerated_not_dropped() {
        // 鼠标快甩时可能送 ±40 这种「不足一格」的值。旧实现按格算（40/120 = 0）
        // 会把这些事件全丢掉，表现就是「滚得快反而不动」。
        let mut acc = Accumulator::default();
        let mut out = 0;
        for _ in 0..3 {
            out += acc.push_units(40, 1.0);
        }
        assert_eq!(out, 120, "3 个 40 单位应该攒成 1 格");

        // 放大 12 倍：同样 3 个事件应该出 12 格。
        let mut acc = Accumulator::default();
        let mut out = 0;
        for _ in 0..3 {
            out += acc.push_units(40, 12.0);
        }
        assert_eq!(out, 12 * 120);
    }

    #[test]
    fn meter_counts_fine_grained_units_as_notches() {
        // 同样是一格的运动，拆成 3 个 40 单位和一次 120 单位，速度要算成一样。
        let mut whole = Meter::default();
        let mut fine = Meter::default();
        let v_whole = whole.velocity(1_000, 1);
        fine.velocity_units(1_000, 40);
        fine.velocity_units(1_000, 40);
        let v_fine = fine.velocity_units(1_000, 40);
        assert!(
            (v_whole - v_fine).abs() < 0.01,
            "整格 {v_whole} 与细粒度 {v_fine} 的速度该一致"
        );
    }

    #[test]
    fn pump_spreads_a_burst_into_single_notches() {
        // 5 格积压 → 每拍 1 格，5 拍（40ms）发完，中途每一拍都是「滚」而不是「跳」。
        let mut pending = 5;
        let mut ticks = 0;
        let mut steps = Vec::new();
        while pending != 0 && ticks < 100 {
            let step = pump_step(pending, true);
            steps.push(step);
            pending -= step;
            ticks += 1;
        }
        assert_eq!(pending, 0);
        assert_eq!(steps, vec![1, 1, 1, 1, 1]);
    }

    #[test]
    fn pump_keeps_direction_and_catches_up() {
        // 反向的积压只发负格，不会把上面没发完的正格先发出去。
        assert_eq!(pump_step(-5, true), -1);
        // 积压很多时一拍多发几格追上，不会越滚越滞后。
        assert_eq!(pump_step(48, true), 8);
        assert_eq!(pump_step(12, true), 2);
        // 不平滑：一拍发完。
        assert_eq!(pump_step(5, false), 5);
        assert_eq!(pump_step(-5, false), -5);
        assert_eq!(pump_step(0, true), 0);
    }
}
