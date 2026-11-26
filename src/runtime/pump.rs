//! 注入线程：按节拍把攒下的格数一格一格发出去。
//!
//! 为什么不在钩子回调里直接 `SendInput`：回调里做的每件事都在输入链路上，慢一点
//! 就是整机滚轮发涩；而且注入的事件还会再走一遍钩子，等于在最不该递归的地方递归。
//!
//! 为什么按节拍而不是一次发完：一次发 5 格是「跳」，5 次每 8ms 一格是「滚」。
//! 节拍同时给注入速率设了上限（125 格/秒），快滚时不会把应用的输入队列灌满。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_WHEEL, MOUSEINPUT,
};

use super::{INJECT_FAILED, MAGIC, SMOOTH};
use crate::core::pacing;
use crate::core::policy::NOTCH;
use crate::infra::log;

/// 一拍多少毫秒。
const TICK_MS: u64 = 8;
/// 队列里最多攒多少格（再多也没意义，只会让「松开手还在滚」更明显）。
const MAX_BACKLOG: i32 = 48;
/// 连续失败几次就认定注入被系统拦了，自动关掉变速。
const MAX_FAILS: u64 = 3;

/// 还没发出去的格数。
struct Pump {
    pending: i32,
}

/// 队列 + 唤醒用的条件变量（钩子回调和注入线程之间唯一的交接点）。
static OUTBOX: OnceLock<(Mutex<Pump>, Condvar)> = OnceLock::new();
/// 连续失败计数（成功一次就清零）。
static FAILS_IN_A_ROW: AtomicU64 = AtomicU64::new(0);

fn outbox() -> &'static (Mutex<Pump>, Condvar) {
    OUTBOX.get_or_init(|| (Mutex::new(Pump { pending: 0 }), Condvar::new()))
}

/// 起注入线程。
pub fn spawn() {
    let spawned = std::thread::Builder::new()
        .name("cvtmouse-inject".to_string())
        .spawn(injector_loop);
    if let Err(error) = spawned {
        log::warn(format!("注入线程起不来：{error}"));
    }
}

fn injector_loop() {
    let (state, wake) = outbox();
    loop {
        let mut pump = match state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if pump.pending == 0 {
            let waited = match wake.wait_timeout(pump, std::time::Duration::from_millis(500)) {
                Ok(pair) => pair,
                Err(poisoned) => poisoned.into_inner(),
            };
            pump = waited.0;
            if pump.pending == 0 {
                continue;
            }
        }

        // 平滑：按积压量分档发（见 `pacing::pump_step`）。不平滑：一次发完。
        let smooth = SMOOTH.load(Ordering::Relaxed);
        let step = pacing::pump_step(pump.pending, smooth);
        pump.pending -= step;
        drop(pump);

        if unsafe { inject(step) } {
            FAILS_IN_A_ROW.store(0, Ordering::Relaxed);
        } else {
            INJECT_FAILED.fetch_add(1, Ordering::Relaxed);
            // 没发出去就放回队列：输入队列满是一过性的，下一拍补发就能自愈。
            // 不放回去的话这一格就永久丢了——表现正是「滚得快时忽然不动」。
            if let Ok(mut pump) = state.lock() {
                pump.pending = (pump.pending + step).clamp(-MAX_BACKLOG, MAX_BACKLOG);
            }
            let fails = FAILS_IN_A_ROW.fetch_add(1, Ordering::Relaxed) + 1;
            if fails == MAX_FAILS {
                log::warn(
                    "注入连续失败：多半是目标窗口提权了（UIPI 拦掉注入）。已自动关闭变速，滚轮恢复原生。",
                );
                super::set_enabled(false);
            }
        }

        if smooth {
            std::thread::sleep(std::time::Duration::from_millis(TICK_MS));
        }
    }
}

/// 清空还没发出去的格数（关变速、换配置时）。
pub fn clear_pending() {
    let (state, _) = outbox();
    if let Ok(mut pump) = state.lock() {
        pump.pending = 0;
    }
}

/// 交给注入线程。
pub fn queue(notches: i32) {
    let (state, wake) = outbox();
    {
        let mut pump = match state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        // 反向：还没发完的立刻丢掉，否则往上滚的第一下会先把上面积压的往下发完。
        if pump.pending.signum() != 0 && pump.pending.signum() != notches.signum() {
            pump.pending = 0;
        }
        pump.pending = (pump.pending + notches).clamp(-MAX_BACKLOG, MAX_BACKLOG);
    }
    wake.notify_one();
}

/// 把「几格」换算成 `MOUSEINPUT.mouseData` 该填的值。
///
/// **填的是原始有符号增量**（一格 = 120），由系统自己把它放进
/// `MSLLHOOKSTRUCT.mouseData` 的高 16 位、以及 `WM_MOUSEWHEEL` 的 `wParam` 里。
///
/// 这里有两个正好相反、都踩过的坑：
///
/// * 发送方**不要**自己左移 16 位。填 `120 << 16` 不会报错，`SendInput` 也返回成功，
///   但系统会把整个 32 位值当成一个数按 i16 饱和——正数变 `32767`、负数变 `-32768`
///   （实测：发 `0xFF880000`，钩子收到 `-32768`）。
/// * 接收方（[`super::hook`] 的 `hook_proc`）才要取高 16 位。
fn wheel_mouse_data(notches: i32) -> u32 {
    // 高 16 位装得下 ±273 格；再多也没意义（平滑泵会把积压分批发完）。
    const LIMIT: i32 = 273;
    let clamped = notches.clamp(-LIMIT, LIMIT);
    (clamped * NOTCH) as u32
}

/// 注入 `notches` 格滚轮（正数向上、负数向下）。返回是否成功。
unsafe fn inject(notches: i32) -> bool {
    if notches == 0 {
        return true;
    }
    let mut input: INPUT = unsafe { std::mem::zeroed() };
    input.r#type = INPUT_MOUSE;
    input.Anonymous = INPUT_0 {
        mi: MOUSEINPUT {
            dx: 0,
            dy: 0,
            mouseData: wheel_mouse_data(notches),
            dwFlags: MOUSEEVENTF_WHEEL,
            time: 0,
            dwExtraInfo: MAGIC as usize,
        },
    };
    unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) == 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_data_holds_the_raw_signed_delta() {
        // 发送方填原始增量，系统负责放进高 16 位。
        assert_eq!(wheel_mouse_data(1) as i32, 120);
        assert_eq!(wheel_mouse_data(-1) as i32, -120);
        assert_eq!(wheel_mouse_data(12) as i32, 1440);
        assert_eq!(wheel_mouse_data(-12) as i32, -1440);
        // 自己左移 16 位是错的：系统会把它当 i16 饱和成 ±1 格。
        assert_ne!(wheel_mouse_data(12), 1440u32 << 16);
    }

    #[test]
    fn mouse_data_stays_inside_the_signed_16_bit_range() {
        // 系统按 i16 饱和，超出就是「一格」，所以这里先截断。
        assert_eq!(wheel_mouse_data(1000) as i32, 273 * 120);
        assert_eq!(wheel_mouse_data(-1000) as i32, -273 * 120);
        assert!((wheel_mouse_data(1000) as i32) <= i16::MAX as i32);
    }
}
