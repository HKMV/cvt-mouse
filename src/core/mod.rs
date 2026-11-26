//! 核心实现：纯逻辑，不碰 Win32，也不碰文件系统。
//!
//! 这一层能在任何机器上跑单测（`cargo test`），改算法不用碰系统接口。
//! 自检只有一条：这里出现 `windows_sys` 或 `std::fs` 就是错位了——
//! 系统调用在 [`crate::sys`]，落地实现在 [`crate::infra`]。

pub mod config;
pub mod curve;
pub mod pacing;
pub mod policy;
pub mod stats;
