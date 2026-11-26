//! 注入节拍：每一拍发几格。
//!
//! 这是「输出侧」的策略，消费者是 [`crate::runtime::pump`] 的注入线程；跟曲线
//! （输入 → 倍率）没有关系，所以单独放一个文件。

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

#[cfg(test)]
mod tests {
    use super::*;

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
