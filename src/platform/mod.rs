//! 外部依赖：Win32 调用、时钟、日志落盘、配置存盘。
//!
//! 和操作系统打交道的东西全在这里，[`crate::core`] 只认这一层给出的结果。
//! 换平台、或者想在测试里换成假实现，改这个模块就够。

pub mod clock;
pub mod log;
pub mod store;
pub mod win32;
