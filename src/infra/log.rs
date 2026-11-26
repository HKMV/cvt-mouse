//! 日志：内存里留最近几百行给界面，磁盘上按天滚动一份。
//!
//! 两条硬约束决定了这里长这样：
//!
//! 1. **钩子回调里不能做 IO**。低级鼠标钩子超过 `LowLevelHooksTimeout`（默认
//!    300ms）不返回，系统会静默把钩子摘掉——滚轮从此不再变速，而且没有任何
//!    报错。所以这里的每个函数只往内存队列里塞一行，落盘交给独立的写入线程。
//! 2. 时间戳必须是完整的 `YYYY-MM-DD HH:MM:SS`：事后对时间线全靠它。
//!
//! # 两个级别阈值
//!
//! 一条记录分别过两道闸，两边互不影响：
//!
//! * **界面**（[`crate::core::config::Config::log_level`]）决定它进不进内存环形
//!   缓冲——也就是界面「日志」卡里看得见什么；
//! * **文件**（[`crate::core::config::Config::log_file_level`]）决定它写不写进磁盘那份。
//!
//! 默认界面 `Info`、文件 `Debug`：逐格决策行是 `Debug`，平时不刷界面，但事后
//! 翻文件还在。级别本身在配置里，改配置走 [`set_levels`]。

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::core::config::Level;

/// 界面最多看这么多行。
const RING: usize = 300;
/// 还没落盘的缓冲上限：写入线程跟不上时宁可丢日志，也不让钩子阻塞。
const PENDING: usize = 4096;

struct Inner {
    /// 级别和整行一起存：换门槛时可以按级别把不合格的行剔掉，不用回头解析字符串。
    ring: VecDeque<(Level, String)>,
    pending: Vec<String>,
    /// 进界面的门槛。
    view: Level,
    /// 写文件的门槛。
    file: Level,
}

static LOG: OnceLock<Mutex<Inner>> = OnceLock::new();

fn inner() -> &'static Mutex<Inner> {
    LOG.get_or_init(|| {
        Mutex::new(Inner {
            ring: VecDeque::with_capacity(RING),
            pending: Vec::new(),
            // 配置还没读到时的临时门槛；`boot` 里会按配置覆盖。
            view: Level::Info,
            file: Level::Info,
        })
    })
}

fn lock() -> std::sync::MutexGuard<'static, Inner> {
    match inner().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl Level {
    /// 行首那个词。定宽 5 字符（`ERROR` / `WARN ` …），按列看的时候对得齐。
    fn tag(self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warn => "WARN",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
        }
    }
}

/// 一行带时间戳的日志：按两个阈值分别决定去处。
fn push(level: Level, message: String) {
    let line = format!("{} {:<5} {message}", crate::sys::clock::stamp(), level.tag());
    let mut guard = lock();
    if level <= guard.view {
        if guard.ring.len() == RING {
            guard.ring.pop_front();
        }
        guard.ring.push_back((level, line.clone()));
    }
    if level <= guard.file && guard.pending.len() < PENDING {
        guard.pending.push(line);
    }
}

/// 功能坏了：用户得知道，而且多半得动手（重启、改配置、装权限）。
pub fn error(message: impl Into<String>) {
    push(Level::Error, message.into());
}

/// 一条需要留意的记录：功能降级但还能用。
pub fn warn(message: impl Into<String>) {
    push(Level::Warn, message.into());
}

/// 一条普通记录：启动、配置生效、开关切换这类「发生过什么」。
pub fn info(message: impl Into<String>) {
    push(Level::Info, message.into());
}

/// 排查用的细节：逐格决策、启动过程中的每一步。
pub fn debug(message: impl Into<String>) {
    push(Level::Debug, message.into());
}

/// 换两个门槛（配置生效时调）。变了才记一行，免得每次保存都多一条。
pub fn set_levels(view: Level, file: Level) {
    let changed = {
        let mut guard = lock();
        let changed = guard.view != view || guard.file != file;
        // 换了界面门槛就立刻生效：缓冲区里比新门槛啰嗦的行直接剔掉，不然
        // 界面上写着「只看警告」却还挂着几十行「信息」。
        if guard.view != view {
            guard.ring.retain(|(level, _)| *level <= view);
        }
        guard.view = view;
        guard.file = file;
        changed
    };
    if changed {
        // 这一行本身也过新门槛：界面调到 Warn 就只在文件里留个痕。
        info(format!("日志级别：界面 {}，文件 {}", view.tag(), file.tag()));
    }
}

/// 最近这些行（旧 → 新），已经按界面的门槛过滤过。
pub fn recent() -> Vec<String> {
    lock().ring.iter().map(|(_, line)| line.clone()).collect()
}

/// 清空内存里的日志（磁盘文件保留）。
pub fn clear() {
    lock().ring.clear();
}

/// 日志文件所在目录。
pub fn dir() -> std::path::PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("CvtMouse").join("logs")
}

/// 起写入线程：每 500ms 把攒下的行追加到当天的文件。
///
/// 只起一次；重复调用是空操作。
pub fn start() {
    static START: std::sync::Once = std::sync::Once::new();
    START.call_once(|| {
        let spawned = std::thread::Builder::new()
            .name("cvtmouse-log".to_string())
            .spawn(|| {
                loop {
                    std::thread::sleep(Duration::from_millis(500));
                    flush();
                }
            });
        if spawned.is_err() {
            // 写线程起不来不影响功能：内存里的日志照样能看。
        }
    });
}

fn flush() {
    let lines = {
        let mut guard = lock();
        if guard.pending.is_empty() {
            return;
        }
        std::mem::take(&mut guard.pending)
    };
    let directory = dir();
    if std::fs::create_dir_all(&directory).is_err() {
        return;
    }
    let name = format!("cvtmouse-{}.log", crate::sys::clock::date());
    let path = directory.join(name);
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        for line in lines {
            let _ = writeln!(file, "{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 门槛的语义：`记录级别 <= 阈值` 才输出；界面调严了，缓冲里旧的行一起剔掉。
    ///
    /// 只有这个用例动全局门槛（其他用例不读日志缓冲），所以不怕并行。
    #[test]
    fn view_threshold_filters_and_prunes() {
        set_levels(Level::Debug, Level::Debug);
        clear();
        debug("逐格决策");
        info("启动完成");
        assert_eq!(recent().len(), 2, "Debug 门槛下两行都在：{:?}", recent());

        // 界面调到「警告」：缓冲区里那两行当场消失。
        set_levels(Level::Warn, Level::Debug);
        assert!(recent().is_empty(), "调严后旧行要一起清掉：{:?}", recent());

        info("不该进界面");
        warn("该进界面");
        let lines = recent();
        assert_eq!(lines.len(), 1, "只有 Warn 能进界面：{lines:?}");
        assert!(lines[0].contains("WARN"), "行首要有级别：{}", lines[0]);
        assert!(lines[0].contains("该进界面"), "留下来的应当是那条 warn：{}", lines[0]);
    }
}
