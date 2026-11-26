//! 后端接口的唯一入口。
//!
//! 界面这一侧只认这个文件：它把 Rust 那边 `Backend` 模块的 JSON 文本解析成
//! 类型化的对象。页面里不出现 `JSON.parse`，也不直接 import `cvtmouse/native`。
//!
//! 类型是手写的，和 `src/lib.rs` / `src/config.rs` 一一对应；字段改名时两边一起改。

import { Backend } from "cvtmouse/native";

/** 变速的实现方式。 */
export type Mode = "hook" | "system";

/** 曲线上的一个断点：`[格/秒, 倍率]`。 */
export type CurvePoint = [number, number];

/** 一份配置（磁盘格式 = 界面草稿格式）。 */
export interface Config {
  enabled: boolean;
  mode: Mode;
  curve: CurvePoint[];
  /** 按应用覆盖的倍率，键是小写 exe 名。 */
  apps: Record<string, number>;
  /** 完全不接管的进程。 */
  skip: string[];
  passCtrl: boolean;
  passAlt: boolean;
  passShift: boolean;
  /** 平滑输出：一次大跳拆成若干小格按节拍发。 */
  smooth: boolean;
  systemLines: number;
  originalLines: number | null;
}

/** 启动时那点环境信息。 */
export interface Meta {
  version: string;
  elevated: boolean;
  /** 权限不足时的一句话，够用时是空串。 */
  hint: string;
  configPath: string;
  logDir: string;
  /** 系统当前的「每格滚几行」。 */
  systemLines: number;
}

/** `boot()` 的返回。 */
export interface Boot {
  config: Config;
  meta: Meta;
}

/** 运行统计。格数一律用 wheel 单位（120 = 一格），因为鼠标不一定送 120 的整数倍。 */
export interface Stats {
  events: number;
  inUnits: number;
  outUnits: number;
  passed: number;
  swallowed: number;
  velocity: number;
  gain: number;
  /** 最近一次收到的原始 mouseData（±120 才是标准一格）。 */
  lastDelta: number;
  /** 注入被系统拦掉的累计次数（正常应为 0）。 */
  injectFailed: number;
}

/** 这一刻的状态。 */
export interface Snapshot {
  enabled: boolean;
  mode: Mode;
  /** 钩子有没有装上。 */
  installed: boolean;
  /** 最近一次算倍率时认到的程序。 */
  exe: string;
  appGain: number;
  stats: Stats;
  logs: string[];
}

/** 动作的返回。 */
export interface Result {
  ok: boolean;
  error?: string;
}

/** 自检结果。 */
export interface SelfTest {
  curve: { velocity: number; gain: number }[];
  burst: { detents: number; intervalMs: number; out: number };
  steps: { velocity: number; gain: number; out: number }[];
}

export function boot(): Boot {
  return JSON.parse(Backend.boot()) as Boot;
}

export function snapshot(): Snapshot {
  return JSON.parse(Backend.snapshot()) as Snapshot;
}

/** 保存整份配置（校验不过时 `ok` 为 false，`error` 可以直接显示）。 */
export function save(config: Config): Result {
  return JSON.parse(Backend.save(JSON.stringify(config))) as Result;
}

export function setEnabled(on: boolean): Result {
  return JSON.parse(Backend.setEnabled(on)) as Result;
}

/** 退出前调用：让钩子线程退出，否则进程关不干净。 */
export function shutdown(): Result {
  return JSON.parse(Backend.shutdown()) as Result;
}

/** 光标下那个窗口的程序名。 */
export function foreground(): { exe: string } {
  return JSON.parse(Backend.foreground()) as { exe: string };
}

export function clearLogs(): Result {
  return JSON.parse(Backend.clearLogs()) as Result;
}

export function relaunchElevated(): Result {
  return JSON.parse(Backend.relaunchElevated()) as Result;
}

export function systemLines(): { lines: number } {
  return JSON.parse(Backend.systemLines()) as { lines: number };
}

export function setSystemLines(lines: number): Result {
  return JSON.parse(Backend.setSystemLines(lines)) as Result;
}

export function selfTest(): SelfTest {
  return JSON.parse(Backend.selfTest()) as SelfTest;
}
