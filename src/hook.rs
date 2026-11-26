//! 全局滚轮钩子：拦下 `WM_MOUSEWHEEL`，按速度曲线重注入。
//!
//! # 为什么是这个方案
//!
//! 滚轮的一格是设备固件写死的 HID 报文（±120），「一格滚几行」是**主机侧**解释
//! 出来的。所以变速只有两条路：改系统那个全局固定值（`SPI_SETWHEELSCROLLLINES`，
//! 见 [`crate::platform::win32`]），或者在输入链路上把事件拦下来自己换算。要「按速度变」，
//! 只能是后者。
//!
//! # 四条不能违反的规矩
//!
//! 1. **回调里不做 IO、不注入、不阻塞**。低级钩子超过 `LowLevelHooksTimeout`
//!    （默认 300ms）不返回，系统会静默摘掉钩子——滚轮从此不再变速，而且没有
//!    任何报错。所以回调只算一个数字、丢进队列就返回，注入由
//!    [`injector_loop`] 这条独立线程按节拍做。
//! 2. **自己的注入要认出来**。`SendInput` 发出去的事件同样会走这个钩子，不认
//!    就会无限递归。认法：`dwExtraInfo` 打一个魔数（[`MAGIC`]）。
//! 3. **不该拦的一律放行**：修饰键按住（保留 Ctrl+滚轮缩放）、非 120 整数倍
//!    （精密触控板）、中键、倍率恰好是 1 的程序，以及**提权窗口**——我们的注入
//!    会被 UIPI 拦掉，拦下来就等于把滚轮弄死。
//! 4. **注入按节拍发**。一次把 5 格塞给应用是「跳」，分成 5 次每 8ms 一格才是
//!    「滚」。这也是快滚时不卡的原因：节拍泵天然给注入速率设了上限。

use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, Once, OnceLock};

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_MOUSE, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT,
    MOUSEEVENTF_WHEEL, MOUSEINPUT, RegisterHotKey, SendInput, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, MSLLHOOKSTRUCT, MSG, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_MOUSE_LL, WM_HOTKEY, WM_MOUSEWHEEL, WM_QUIT,
};

use crate::core::config::{Config, Mode};
use crate::core::curve::{self, Accumulator, Curve, Meter};
use crate::platform::win32::Target;
use crate::platform::{clock, log, store, win32};

/// 自己注入的事件的标记（`CVTM`）。
///
/// **必须是 32 位**。`MSLLHOOKSTRUCT.dwExtraInfo` 声明成 `ULONG_PTR`，但低级钩子里
/// 实际只保留低 32 位——用 64 位魔数会认不出自己的事件（实测：`0x4356544D0001`
/// 回来时变成 `0x544D0001`），于是自己注入的每一格都被当成新输入再放大一遍，
/// 回灌成环。所以这里用 32 位，并且比较时也只比低 32 位。
const MAGIC: u32 = 0x4356_544D;

/// `MSLLHOOKSTRUCT.flags` 里的「这是注入事件」位。
const LLMHF_INJECTED: u32 = 0x0000_0001;

/// 热键的 id。
const HOTKEY_ID: i32 = 0x4356;

/// 一格滚轮 = 120 个 wheel 单位（HID 与 Windows 的约定）。
const NOTCH: i32 = 120;

/// 单个事件里最多认这么多 wheel 单位（= 2 格）。
///
/// 真实硬件一次最多送一两格；超过这个数说明事件不是人手滚出来的——要么别的软件在
/// 注入（有些厂商驱动/滚轮工具会一次送 `0x7FFF`），要么是回灌成了环。这种事件一律
/// **原样放行**，绝不放大：放大它就是在帮倒忙，还可能把环放大成雪崩。
const MAX_SANE_UNITS: i32 = 240;

/// 速度超过这个值就当是机器发的，不再变速（人手动滚轮很难超过 100 格/秒）。
const MAX_SANE_VELOCITY: f32 = 200.0;

/// 平滑模式下每发一格的间隔（毫秒）。
const TICK_MS: u64 = 8;

/// 还没发出去的格数最多攒这么多（再多就丢，宁可少滚也不要越滚越滞后）。
const MAX_BACKLOG: i32 = 48;

/// 连续注入失败这么多次就自动关掉变速（注入被系统拦，说明拦下来只会把滚轮弄死）。
const MAX_FAILS: u64 = 3;

static ENGINE: OnceLock<Mutex<Engine>> = OnceLock::new();
static STARTED: Once = Once::new();
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// 钩子线程的 id：退出时要往里丢一个 `WM_QUIT`，否则它的消息循环永远不返回。
static HOOK_THREAD: AtomicU32 = AtomicU32::new(0);
/// 注入失败累计 / 连续次数。
static INJECT_FAILED: AtomicU64 = AtomicU64::new(0);
static FAILS_IN_A_ROW: AtomicU64 = AtomicU64::new(0);
/// 平滑开关的一份快照：注入线程每个节拍都要读，别去抢引擎的锁。
static SMOOTH: AtomicBool = AtomicBool::new(true);
/// 还能记多少条原始事件明细（给「快滚到底收到什么」留证，用完就停，免得刷爆日志）。
static TRACE_LEFT: AtomicU32 = AtomicU32::new(400);

/// 待注入的格数（有符号：正数向上、负数向下）+ 通知。
static OUTBOX: OnceLock<(Mutex<Pump>, Condvar)> = OnceLock::new();

/// 注入队列的状态。
#[derive(Debug, Default)]
struct Pump {
    pending: i32,
}

fn outbox() -> &'static (Mutex<Pump>, Condvar) {
    OUTBOX.get_or_init(|| (Mutex::new(Pump::default()), Condvar::new()))
}

/// 钩子有没有装上（装不上时界面要说明「为什么不生效」）。
pub fn installed() -> bool {
    INSTALLED.load(Ordering::SeqCst)
}

/// 注入失败的总次数。
pub fn inject_failed() -> u64 {
    INJECT_FAILED.load(Ordering::Relaxed)
}

/// 钩子回调要做的判断结果。
enum Act {
    /// 原样放行。
    Pass,
    /// 吃掉，什么都不发（倍率小于 1 时攒着）。
    Swallow,
    /// 吃掉，改发这么多格。
    Wheel(i32),
}

/// 运行统计（界面每 250ms 读一次）。
///
/// 计数用 **wheel 单位**（120 = 一格）而不是格：鼠标送来的值不一定是 120 的整数倍，
/// 按格记会把细粒度事件记成 0，看起来像「什么都没收到」。
#[derive(Clone, Debug, Default)]
pub struct Stats {
    /// 经过钩子的滚轮事件数。
    pub events: u64,
    /// 累计进来的 wheel 单位（绝对值）。
    pub in_units: i64,
    /// 累计发给应用的 wheel 单位（绝对值）。
    pub out_units: i64,
    /// 原样放行的次数。
    pub passed: u64,
    /// 吃掉但没发的次数。
    pub swallowed: u64,
    /// 最近一次的速度（格/秒）。
    pub velocity: f32,
    /// 最近一次的倍率。
    pub gain: f32,
    /// 最近一次收到的原始 `mouseData`（±120 才是标准一格）。
    pub last_delta: i32,
    /// 最近一次事件的 `dwExtraInfo`：用来认出「这条是谁注入的」。
    pub last_extra: usize,
    /// 最近一次事件的 `dwFlags`。
    pub last_flags: u32,
    /// 最近一次算倍率时认到的程序。
    pub exe: String,
}

/// 光标下目标窗口的缓存：`OpenProcess` + 令牌查询一次几十微秒，滚得快时每格都查没必要。
#[derive(Debug, Default)]
struct TargetCache {
    at: u64,
    target: Option<Target>,
}

/// 钩子线程持有的全部状态。
pub struct Engine {
    pub config: Config,
    curve: Curve,
    meter: Meter,
    acc: Accumulator,
    cache: TargetCache,
    pub stats: Stats,
}

impl Engine {
    fn new(config: Config) -> Engine {
        let curve = config.curve();
        Engine {
            config,
            curve,
            meter: Meter::default(),
            acc: Accumulator::default(),
            cache: TargetCache::default(),
            stats: Stats::default(),
        }
    }

    /// 换一份配置：曲线、速度窗口、累加器全部重来（换配置时旧的速度没有意义）。
    fn apply(&mut self, config: Config) {
        self.curve = config.curve();
        self.meter = Meter::default();
        self.acc.reset();
        SMOOTH.store(config.smooth, Ordering::Relaxed);
        self.config = config;
        log::info(format!(
            "配置已生效：{}，曲线 {} 个断点，按应用规则 {} 条，跳过 {} 个进程，平滑{}",
            if self.config.enabled { "已开启" } else { "已关闭" },
            self.curve.points().len(),
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
    STARTED.call_once(spawn);
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
    lock().stats.clone()
}

/// 开关一次（界面按钮和全局热键都走这里），并落盘。
pub fn set_enabled(on: bool) {
    let (enabled, saved) = {
        let mut state = lock();
        state.config.enabled = on;
        state.meter = Meter::default();
        state.acc.reset();
        let saved = store::save_config(&state.config);
        (state.config.enabled, saved)
    };
    clear_pending();
    log::info(format!("总开关：{}", if enabled { "开启" } else { "关闭" }));
    if let Err(error) = saved {
        log::warn(format!("配置存盘失败：{error}"));
    }
}

fn spawn() {
    let spawned = std::thread::Builder::new()
        .name("cvtmouse-hook".to_string())
        .spawn(run);
    if let Err(error) = spawned {
        log::warn(format!("钩子线程起不来：{error}"));
    }
    let injector = std::thread::Builder::new()
        .name("cvtmouse-inject".to_string())
        .spawn(injector_loop);
    if let Err(error) = injector {
        log::warn(format!("注入线程起不来：{error}"));
    }
}

/// 注入线程：按节拍把攒下的格数一格一格发出去。
///
/// 为什么不在钩子回调里直接 `SendInput`：回调里做的每件事都在输入链路上，慢一点
/// 就是整机滚轮发涩；而且注入的事件还会再走一遍钩子，等于在最不该递归的地方递归。
///
/// 为什么按节拍而不是一次发完：一次发 5 格是「跳」，5 次每 8ms 一格是「滚」。
/// 节拍同时给注入速率设了上限（125 格/秒），快滚时不会把应用的输入队列灌满。
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

        // 平滑：按积压量分档发（见 `curve::pump_step`）。不平滑：一次发完。
        let smooth = SMOOTH.load(Ordering::Relaxed);
        let step = curve::pump_step(pump.pending, smooth);
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
                set_enabled(false);
            }
        }

        if smooth {
            std::thread::sleep(std::time::Duration::from_millis(TICK_MS));
        }
    }
}

/// 清空还没发出去的格数（关变速、换配置、反向时）。
fn clear_pending() {
    let (state, _) = outbox();
    if let Ok(mut pump) = state.lock() {
        pump.pending = 0;
    }
}

/// 让钩子线程退出（它的消息循环收到 `WM_QUIT` 就返回）。退出程序前调用。
pub fn shutdown() {
    let id = HOOK_THREAD.swap(0, Ordering::SeqCst);
    if id != 0 {
        unsafe { PostThreadMessageW(id, WM_QUIT, 0, 0) };
    }
}

fn run() {
    unsafe {
        HOOK_THREAD.store(GetCurrentThreadId(), Ordering::SeqCst);
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(hook_proc), ptr::null_mut(), 0);
        if hook.is_null() {
            INSTALLED.store(false, Ordering::SeqCst);
            log::warn(
                "装不上全局鼠标钩子：滚轮不会变速。常见原因是安全软件拦了低级键盘/鼠标钩子。",
            );
            return;
        }
        INSTALLED.store(true, Ordering::SeqCst);
        log::info("全局滚轮钩子已装上（WH_MOUSE_LL）");

        // 热键是「一键停用」的保险：系统里抢这个组合的程序不少，所以按顺序试几个。
        let candidates: [(u32, u32, &str); 3] = [
            (MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, 'S' as u32, "Ctrl+Alt+S"),
            (
                MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_NOREPEAT,
                'S' as u32,
                "Ctrl+Alt+Shift+S",
            ),
            (MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, 0x7B, "Ctrl+Alt+F12"),
        ];
        let mut bound = "";
        for (modifiers, key, name) in candidates {
            if RegisterHotKey(ptr::null_mut(), HOTKEY_ID, modifiers, key) != 0 {
                bound = name;
                break;
            }
        }
        if bound.is_empty() {
            log::warn("热键都没注册上（Ctrl+Alt+S 等被占用），开关请用界面或托盘菜单");
        } else {
            log::info(format!("热键 {bound}：一键开关变速"));
        }

        let mut message: MSG = std::mem::zeroed();
        // GetMessageW 返回 0 是 WM_QUIT，-1 是出错；两种都退出。
        while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
            if message.message == WM_HOTKEY {
                let next = !lock().config.enabled;
                set_enabled(next);
            }
        }
        UnhookWindowsHookEx(hook);
        INSTALLED.store(false, Ordering::SeqCst);
    }
}

/// 钩子回调。**必须快**：这里只做判断和一次入队。
unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && wparam as u32 == WM_MOUSEWHEEL {
        let info = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        // 自己注入的事件直接放行，否则会无限递归。魔数只比低 32 位（见 MAGIC 的说明）。
        //
        // 别的软件注入的事件**不跳过**：有些鼠标驱动/触控板就是用注入的方式送滚轮的，
        // 跳过它们等于让那类设备完全失去变速。防回灌靠另外三道闸：32 位魔数、
        // 「单事件超过 2 格就放行」、以及「速度超过 200 格/秒就放行」。
        if info.dwExtraInfo as u32 != MAGIC {
            let delta = ((info.mouseData >> 16) & 0xFFFF) as u16 as i16;
            match decide(delta, info.dwExtraInfo, info.flags) {
                Act::Pass => {}
                Act::Swallow => return 1,
                Act::Wheel(notches) => {
                    queue(notches);
                    return 1;
                }
            }
        }
    }
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

/// 这一格要不要改、改成多少。
fn decide(delta: i16, extra: usize, flags: u32) -> Act {
    let mut state = lock();
    state.stats.events += 1;
    state.stats.last_delta = delta as i32;
    state.stats.last_extra = extra;
    state.stats.last_flags = flags;

    if !state.config.enabled || state.config.mode != Mode::Hook {
        return Act::Pass;
    }
    if delta == 0 {
        return Act::Pass;
    }
    if (state.config.pass_ctrl && key_down(VK_CONTROL))
        || (state.config.pass_alt && key_down(VK_MENU))
        || (state.config.pass_shift && key_down(VK_SHIFT))
    {
        state.stats.passed += 1;
        return Act::Pass;
    }

    let target = state.target();
    // 认不出光标下的程序（受保护进程，连 OpenProcess 都打不开）：原样放行。
    // 这种窗口我们的注入同样会被拦，拦下来只会把滚轮弄死。
    if target.exe.is_empty() {
        state.stats.passed += 1;
        return Act::Pass;
    }
    let skipped = state.config.is_skipped(&target.exe);
    let app_gain = state.config.app_gain(&target.exe);
    // 提权窗口：我们的注入会被 UIPI 丢掉，拦下来等于把滚轮弄死，所以原样放行。
    let blocked = target.elevated && !win32::is_elevated();
    if skipped || blocked {
        state.stats.passed += 1;
        state.stats.exe = target.exe;
        return Act::Pass;
    }

    let units = delta as i32;
    let velocity = state.meter.velocity_units(clock::now_ms(), units);
    let gain = state.curve.gain(velocity) * app_gain;

    state.stats.velocity = velocity;
    state.stats.gain = gain;
    state.stats.exe = target.exe;
    state.stats.in_units += i64::from(units.unsigned_abs());

    // 不像人滚出来的事件：原样放行，绝不放大。
    // 这条是安全阀——没有它，别的软件注入的大增量（或回灌成的环）会被我们成倍放大。
    if units.abs() > MAX_SANE_UNITS || velocity > MAX_SANE_VELOCITY {
        state.stats.passed += 1;
        let exe = state.stats.exe.clone();
        trace(
            &mut state,
            format!(
                "放行（异常事件）原始={units} 速度={velocity:.0}/s 倍率={gain:.2} extra={extra:x} flags={flags:x} 目标={exe}"
            ),
        );
        return Act::Pass;
    }

    // 倍率恰好 1：原样放行。不注入就没有额外延迟，也不会跟别的滚轮工具打架。
    if (gain - 1.0).abs() < 0.01 {
        state.stats.out_units += i64::from(units.unsigned_abs());
        state.stats.passed += 1;
        return Act::Pass;
    }

    let out = state.acc.push_units(units, gain);

    // 留证据：快滚时（或者收到非标准值）把原始数据记下来，用完就停。
    if velocity >= 8.0 || units % NOTCH != 0 {
        let exe = state.stats.exe.clone();
        trace(
            &mut state,
            format!(
                "滚轮 原始={units} 速度={velocity:.0}/s 倍率={gain:.2} 输出={out} extra={extra:x} 注入={} 目标={exe}",
                flags & LLMHF_INJECTED != 0
            ),
        );
    }

    if out == 0 {
        state.stats.swallowed += 1;
        return Act::Swallow;
    }
    state.stats.out_units += i64::from(out.unsigned_abs());
    Act::Wheel(out / NOTCH)
}

/// 记一行诊断（每次运行最多 [`TRACE_CAP`] 行，用完就停，免得刷爆日志）。
fn trace(state: &mut Engine, message: String) {
    let left = TRACE_LEFT.load(Ordering::Relaxed);
    if left > 0 {
        TRACE_LEFT.store(left - 1, Ordering::Relaxed);
        log::info(message);
    }
    let _ = state;
}

/// 交给注入线程。
fn queue(notches: i32) {
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

fn key_down(key: u16) -> bool {
    unsafe { GetAsyncKeyState(key as i32) < 0 }
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
/// * 接收方（本文件的 [`hook_proc`]）才要取高 16 位。
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

    /// 按 Windows 的读法解出滚轮增量（和 `hook_proc` 里的解码同一套）。
    fn decode(mouse_data: u32) -> i16 {
        ((mouse_data >> 16) & 0xFFFF) as u16 as i16
    }

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

    #[test]
    fn high_word_decode_reads_what_the_system_builds() {
        // 系统把增量放进高 16 位后，钩子这一侧读出来要还原成原来的增量。
        assert_eq!(decode((120u32) << 16), 120);
        assert_eq!(decode((-1440i32 as u32) << 16), -1440);
    }

    #[test]
    fn magic_fits_in_32_bits() {
        // 低级钩子只保留 dwExtraInfo 的低 32 位；魔数一旦超过 32 位，就认不出
        // 自己注入的事件，会回灌成环（这就是上一版的 bug）。
        assert!(u64::from(MAGIC) <= u64::from(u32::MAX));
        assert_eq!(MAGIC as usize as u32, MAGIC);
    }
}


