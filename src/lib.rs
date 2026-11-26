//! CvtMouse 的后端 crate。
//!
//! 这个文件只有注册，没有实现：`#[native_module]` 和 `#[module]` 只能写在 crate
//! 根（nsh 的硬约束），接口本身在 [`api`]。
//!
//! # 分层
//!
//! 每个目录只有一个「为什么存在」，自检就是看谁碰了什么：
//!
//! * [`core`] —— 核心实现：曲线、决策、配置。纯逻辑，出现 `windows_sys` 或
//!   `std::fs` 就是错位；
//! * [`runtime`] —— 编排：钩子线程、注入泵、全局引擎状态。只有它碰线程和钩子；
//! * [`sys`] —— 系统调用：Win32、时钟。只有它能 `use windows_sys`；
//! * [`infra`] —— 落地：日志、配置存盘。只有它写文件；
//! * [`api`] —— nsh 边界：JSON 进出的 `Backend`。只有它碰 `nsh_core`。

mod api;
mod core;
mod infra;
mod runtime;
mod sys;

use nsh_core::prelude::*;

/// 本 crate 的 UI 集合（`nsh.json` 的 `"rust"`）。
#[native_module("cvtmouse/native")]
fn native(registry: &mut Registry) {
    registry.module::<api::Backend>();
}
