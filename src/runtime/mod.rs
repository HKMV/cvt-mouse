//! 运行时（编排层）：把 [`crate::core::policy`] 的判断接到 Windows 的输入链路上。
//!
//! 这里只负责「装上钩子、收事件、把环境填好交出去、把结果交给注入线程」；
//! 要不要改、改多少是纯逻辑，在 `core/policy.rs` 里，能脱离 Windows 单测。
//! 三个文件各管一件事：[`hook`] 收事件，[`pump`] 发事件，本文件管状态和生命周期。
//!
//! # 为什么是这个方案
//!
//! 滚轮的一格是设备固件写死的 HID 报文（±120），「一格滚几行」是**主机侧**解释
//! 出来的。所以变速只有两条路：改系统那个全局固定值（`SPI_SETWHEELSCROLLLINES`，
//! 见 [`crate::sys::win32`]），或者在输入链路上把事件拦下来自己换算。要「按速度变」，
//! 只能是后者。
//!
//! # 四条不能违反的规矩
//!
//! 1. **回调里不做 IO、不注入、不阻塞**。低级钩子超过 `LowLevelHooksTimeout`
//!    （默认 300ms）不返回，系统会静默摘掉钩子——滚轮从此不再变速，而且没有
//!    任何报错。所以回调只算一个数字、丢进队列就返回，注入由
//!    [`pump`] 这条独立线程按节拍做。
//! 2. **自己的注入要认出来**。`SendInput` 发出去的事件同样会走这个钩子，不认
//!    就会无限递归。认法：`dwExtraInfo` 打一个魔数（[`MAGIC`]）。
//! 3. **不该拦的一律放行**：修饰键按住（保留 Ctrl+滚轮缩放）、非 120 整数倍
//!    （精密触控板）、中键、倍率恰好是 1 的程序，以及**提权窗口**——我们的注入
//!    会被 UIPI 拦掉，拦下来就等于把滚轮弄死。
//! 4. **注入按节拍发**。一次把 5 格塞给应用是「跳」，分成 5 次每 8ms 一格才是
//!    「滚」。这也是快滚时不卡的原因：节拍泵天然给注入速率设了上限。

mod hook;
mod pump;

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, Once, OnceLock};

use crate::core::config::Config;
use crate::core::policy::{Act, Env, Policy};
use crate::core::stats::Stats;
use crate::infra::{log, store};
use crate::sys::clock;
use crate::sys::win32::{self, Target};

/// 自己注入的事件的标记（`CVTM`）。
///
/// **必须是 32 位**。`MSLLHOOKSTRUCT.dwExtraInfo` 声明成 `ULONG_PTR`，但低级钩子里
/// 实际只保留低 32 位——用 64 位魔数会认不出自己的事件（实测：`0x4356544D0001`
/// 回来时变成 `0x544D0001`），于是自己注入的每一格都被当成新输入再放大一遍，
/// 回灌成环。所以这里用 32 位，并且比较时也只比低 32 位。
pub(crate) const MAGIC: u32 = 0x4356_544D;

/// 钩子线程的线程 id（退出时往它发 `WM_QUIT`）。
static HOOK_THREAD: AtomicU32 = AtomicU32::new(0);
/// 钩子有没有装上。
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// 注入失败的总次数。
static INJECT_FAILED: AtomicU64 = AtomicU64::new(0);
/// 平滑输出开关（配置里的 `smooth`），注入线程每一拍读一次。
static SMOOTH: AtomicBool = AtomicBool::new(true);
/// 还能记多少行诊断（每次运行最多这么多，用完就停，免得刷爆日志）。
static TRACE_LEFT: AtomicU32 = AtomicU32::new(400);

/// 全局唯一的引擎。
static ENGINE: OnceLock<Mutex<Engine>> = OnceLock::new();
/// 钩子线程和注入线程只起一次。
static STARTED: Once = Once::new();

/// 光标下目标窗口的缓存：`OpenProcess` + 令牌查询一次几十微秒，滚得快时每格都查没必要。
#[derive(Debug, Default)]
struct TargetCache {
    at: u64,
    target: Option<Target>,
}

/// 钩子线程持有的全部状态。
pub struct Engine {
    pub config: Config,
    policy: Policy,
    cache: TargetCache,
}

impl Engine {
    fn new(config: Config) -> Engine {
        Engine {
            policy: Policy::new(&config),
            config,
            cache: TargetCache::default(),
        }
    }

    /// 换一份配置：曲线、速度窗口、累加器全部重来（换配置时旧的速度没有意义）。
    fn apply(&mut self, config: Config) {
        self.policy.apply(&config);
        SMOOTH.store(config.smooth, Ordering::Relaxed);
        self.config = config;
        log::info(format!(
            "配置已生效：{}，曲线 {} 个断点，按应用规则 {} 条，跳过 {} 个进程，平滑{}",
            if self.config.enabled { "已开启" } else { "已关闭" },
            self.policy.points(),
            self.config.apps.len(),
            self.config.skip.len(),
            if self.config.smooth { "开" } else { "关" }
        ));
    }

    /// 光标下那个窗口的程序名与是否提权（250ms 缓存）。
    fn target(&mut self) -> Target {
        let now = clock::now_ms();
        if now.saturating_sub(self.cache.at) < 250 {
            if let Some(target) = &self.cache.target {
                return target.clone();
            }
        }
        let target = win32::cursor_target();
        self.cache.at = now;
        self.cache.target = Some(target.clone());
        target
    }
}

/// 全局唯一的引擎。
pub fn engine() -> &'static Mutex<Engine> {
    ENGINE.get_or_init(|| Mutex::new(Engine::new(store::load_config())))
}

fn lock() -> std::sync::MutexGuard<'static, Engine> {
    match engine().lock() {
        Ok(guard) => guard,
        // 钩子里 panic 过一次的话锁是毒的，但状态本身还能用：继续用。
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// 装上钩子（只起一次线程），并让这份配置立刻生效。
pub fn install(config: Config) {
    lock().apply(config);
    STARTED.call_once(|| {
        hook::spawn();
        pump::spawn();
    });
}

/// 换配置，不重装钩子。
pub fn reload(config: Config) {
    lock().apply(config);
}

/// 当前配置的一份拷贝。
pub fn config() -> Config {
    lock().config.clone()
}

/// 当前统计的一份拷贝。
pub fn stats() -> Stats {
    lock().policy.stats.clone()
}

/// 钩子装上了吗。
pub fn installed() -> bool {
    INSTALLED.load(Ordering::SeqCst)
}

/// 注入失败的总次数。
pub fn inject_failed() -> u64 {
    INJECT_FAILED.load(Ordering::Relaxed)
}

/// 开关一次（界面按钮和全局热键都走这里），并落盘。
pub fn set_enabled(on: bool) {
    let (enabled, saved) = {
        let mut state = lock();
        state.config.enabled = on;
        state.policy.reset();
        let saved = store::save_config(&state.config);
        (state.config.enabled, saved)
    };
    pump::clear_pending();
    log::info(format!("总开关：{}", if enabled { "开启" } else { "关闭" }));
    if let Err(error) = saved {
        log::warn(format!("配置存盘失败：{error}"));
    }
}

/// 让钩子线程退出（它的消息循环收到 `WM_QUIT` 就返回）。退出程序前调用。
pub fn shutdown() {
    let id = HOOK_THREAD.swap(0, Ordering::SeqCst);
    if id != 0 {
        hook::quit_thread(id);
    }
}

/// 一次判断：把平台信息填成 [`Env`]，交给 [`Policy`]，再把这轮该记的日志落掉。
///
/// 调用方是钩子回调，所以这里只做查询和一次加锁，**不能**有 IO。
pub(crate) fn decide(delta: i16, extra: usize, flags: u32) -> Act {
    let mut guard = lock();
    // 先摘成 `&mut Engine`：这样 `policy` 和 `config` 才是两个不相交的字段借用。
    let state = &mut *guard;
    let target = state.target();
    let env = Env {
        exe: target.exe,
        target_elevated: target.elevated,
        self_elevated: win32::is_elevated(),
        // 配置说不用管的修饰键就不查（少两次 GetAsyncKeyState）。
        ctrl: state.config.pass_ctrl && hook::key_down(hook::VK_CONTROL),
        alt: state.config.pass_alt && hook::key_down(hook::VK_MENU),
        shift: state.config.pass_shift && hook::key_down(hook::VK_SHIFT),
        now_ms: clock::now_ms(),
    };
    let act = state.policy.decide(&state.config, delta, &env, extra, flags);
    if let Some(line) = state.policy.take_trace() {
        trace(line);
    }
    act
}

/// 记一行诊断（每次运行最多 [`TRACE_LEFT`] 行，用完就停，免得刷爆日志）。
pub(crate) fn trace(message: String) {
    let left = TRACE_LEFT.load(Ordering::Relaxed);
    if left > 0 {
        TRACE_LEFT.store(left - 1, Ordering::Relaxed);
        log::info(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_fits_in_32_bits() {
        // 低级钩子只保留 dwExtraInfo 的低 32 位；魔数一旦超过 32 位，就认不出
        // 自己注入的事件，会回灌成环（这就是上一版的 bug）。
        assert!(u64::from(MAGIC) <= u64::from(u32::MAX));
        assert_eq!(MAGIC as usize as u32, MAGIC);
    }
}
