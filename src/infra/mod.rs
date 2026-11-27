//! 落地实现：日志、配置文件和内嵌资源。
//!
//! 跟 [`crate::sys`] 分开，是因为这里要的是 `std::fs` 而不是系统 API：
//! 换平台不用改，换存放位置（比如改成便携版）只改这几个文件。
//!
//! * [`log`] —— 内存环形缓冲 + 500ms 落盘线程（钩子回调里禁止 IO）；
//! * [`store`] —— `config.json` 的路径与读写；
//! * [`embed`] —— 进程起来第一件事：把 exe 尾巴上的 `nsh/` 解出来。

pub mod embed;
pub mod log;
pub mod store;
