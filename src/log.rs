//! 日志：内存里留最近几百行给界面，磁盘上按天滚动一份。
//!
//! 两条硬约束决定了这里长这样：
//!
//! 1. **钩子回调里不能做 IO**。低级鼠标钩子超过 `LowLevelHooksTimeout`（默认
//!    300ms）不返回，系统会静默把钩子摘掉——滚轮从此不再变速，而且没有任何
//!    报错。所以 [`info`] / [`warn`] 只往内存队列里塞一行，落盘交给独立的
//!    写入线程。
//! 2. 时间戳必须是完整的 `YYYY-MM-DD HH:MM:SS`：事后对时间线全靠它。

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// 界面最多看这么多行。
const RING: usize = 300;
/// 还没落盘的缓冲上限：写入线程跟不上时宁可丢日志，也不让钩子阻塞。
const PENDING: usize = 4096;

struct Inner {
    ring: VecDeque<String>,
    pending: Vec<String>,
}

static LOG: OnceLock<Mutex<Inner>> = OnceLock::new();

fn inner() -> &'static Mutex<Inner> {
    LOG.get_or_init(|| {
        Mutex::new(Inner { ring: VecDeque::with_capacity(RING), pending: Vec::new() })
    })
}

/// 一行带时间戳的日志。
fn push(level: &str, message: String) {
    let line = format!("{} {level} {message}", crate::clock::stamp());
    let mut guard = match inner().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if guard.ring.len() == RING {
        guard.ring.pop_front();
    }
    guard.ring.push_back(line.clone());
    if guard.pending.len() < PENDING {
        guard.pending.push(line);
    }
}

/// 一条普通记录。
pub fn info(message: impl Into<String>) {
    push("INFO ", message.into());
}

/// 一条需要留意的记录。
pub fn warn(message: impl Into<String>) {
    push("WARN ", message.into());
}

/// 最近这些行（旧 → 新）。
pub fn recent() -> Vec<String> {
    let guard = match inner().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.ring.iter().cloned().collect()
}

/// 清空内存里的日志（磁盘文件保留）。
pub fn clear() {
    let mut guard = match inner().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.ring.clear();
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
        let mut guard = match inner().lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if guard.pending.is_empty() {
            return;
        }
        std::mem::take(&mut guard.pending)
    };
    let directory = dir();
    if std::fs::create_dir_all(&directory).is_err() {
        return;
    }
    let name = format!("cvtmouse-{}.log", crate::clock::date());
    let path = directory.join(name);
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        for line in lines {
            let _ = writeln!(file, "{line}");
        }
    }
}
