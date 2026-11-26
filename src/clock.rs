//! 时间：日志用的完整时间戳、按天滚动用的日期、钩子用的单调毫秒。
//!
//! 不用 `time` 之类的 crate：这里只需要「本地时间格式化」和「单调毫秒」两件事，
//! Win32 各给了一个函数，省一个依赖也就省一次编译。

/// `YYYY-MM-DD HH:MM:SS`（本地时间）。
#[cfg(windows)]
pub fn stamp() -> String {
    let local = local_time();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute, local.wSecond
    )
}

/// `YYYY-MM-DD`（本地时间），日志文件名用它。
#[cfg(windows)]
pub fn date() -> String {
    let local = local_time();
    format!("{:04}-{:02}-{:02}", local.wYear, local.wMonth, local.wDay)
}

#[cfg(windows)]
fn local_time() -> windows_sys::Win32::Foundation::SYSTEMTIME {
    let mut local: windows_sys::Win32::Foundation::SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut local) };
    local
}

/// 单调毫秒（`GetTickCount64`）：速度窗口用它，不受系统时间调整影响。
#[cfg(windows)]
pub fn now_ms() -> u64 {
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() }
}

#[cfg(not(windows))]
pub fn stamp() -> String {
    format!("{}", unix_seconds())
}

#[cfg(not(windows))]
pub fn date() -> String {
    format!("{}", unix_seconds())
}

#[cfg(not(windows))]
fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(not(windows))]
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
