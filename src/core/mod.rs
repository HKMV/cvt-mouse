//! 核心实现：纯逻辑，不碰 Win32，也不碰文件系统。
//!
//! 这一层能在任何机器上跑单测（`cargo test`），改算法不用碰系统接口；
//! 和操作系统打交道的东西都在 [`crate::platform`] 里。

pub mod config;
pub mod curve;
