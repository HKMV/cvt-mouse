//! 系统调用：只有这一层可以直接 `use windows_sys`。
//!
//! 权限、提权重启、光标下的窗口、系统滚轮行数在 [`win32`]，时间在 [`clock`]。
//! 换平台要重写的就是这两个文件。

pub mod clock;
pub mod win32;
