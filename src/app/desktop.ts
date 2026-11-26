//! 桌面外壳：托盘图标、关窗收进托盘。
//!
//! 这个程序是常驻的：窗口一关，钩子跟着没了，滚轮就回到原生手感。所以关窗
//! 不是退出，是收进托盘；真退出走托盘菜单。

import { effect } from "nsh";
import { Application, currentWindow, Tray } from "nsh/desktop";
import type { WindowRef } from "nsh/desktop";
import * as api from "./api";

/** 窗口句柄。`currentWindow()` 只能在渲染期间调，所以渲染时取一次存这儿。 */
let handle: WindowRef | null = null;

/** 在根组件里调一次。 */
export function bindWindow(): void {
  if (!handle) handle = currentWindow();
}

export function hideToTray(): void {
  handle?.hide();
}

export function showWindow(): void {
  handle?.show();
  handle?.focus();
}

export function quit(): void {
  // 先把钩子线程收掉，再关应用：不然它会一直卡在消息循环里，进程退不干净。
  api.shutdown();
  Application.quit();
}

/** 装托盘：一句状态 + 三项菜单，点图标把窗口叫回来。 */
export function installTray(options: { enabled: () => boolean; onToggle: () => void }): void {
  let last = "";
  effect(() => {
    const enabled = options.enabled();
    // 只在状态真的变了时重设：快照每秒好几份，每份都 Tray.set 等于不停重建原生菜单。
    if (String(enabled) === last) return;
    last = String(enabled);
    Tray.set({
      tooltip: `CvtMouse · ${enabled ? "变速已开启" : "变速已关闭"}`,
      menu: [
        { label: enabled ? "关闭变速" : "开启变速", onSelect: options.onToggle },
        { type: "separator" },
        { label: "显示窗口", onSelect: showWindow },
        { label: "退出", onSelect: quit },
      ],
      onClick: showWindow,
    });
  });
}

/** 拦下关闭按钮：收进托盘，别退出。 */
export function interceptClose(): void {
  const win = handle ?? currentWindow();
  handle = win;
  win.set({ closeOnPress: false });
  win.on("closePress", () => hideToTray());
}
