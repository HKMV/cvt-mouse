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
//! * [`infra`] —— 落地：日志、配置存盘、内嵌资源解包。只有它写文件；
//! * [`api`] —— nsh 边界：JSON 进出的 `Backend`。只有它碰 `nsh_core`。

mod api;
mod core;
mod infra;
mod runtime;
mod sys;

use nsh_core::prelude::*;

/// 把 exe 尾巴上的 `nsh/` 解出来，并把 `NSH_APP_DIR` 指过去（见 [`infra::embed`]）。
///
/// 必须是进程里**第一件**跑的事：nsh 生成的 `main` 一进去就读 `nsh/app.json`，
/// 中间没有钩子可挂。`.CRT$XCU` 是 CRT 的初始化段，比 `main` 早；没有内嵌资源的进程
/// （`nsh run` 的开发宿主、`nsh test` 的 headless runner）里，它只看一眼自己 exe 的
/// 尾巴就返回，什么都不改。
#[cfg(windows)]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static EMBED_BOOT: extern "C" fn() = infra::embed::boot;

/// 本 crate 的 UI 集合（`nsh.json` 的 `"rust"`）。
#[native_module("cvtmouse/native")]
fn native(registry: &mut Registry) {
    registry.module::<api::Backend>();
}
