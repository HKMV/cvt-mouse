//! TSX 入口。
//!
//! 只有这一个文件放在仓库根：nsh 从 `nsh.json` 同级的入口解析 `cvtmouse/native`，
//! 入口挪到子目录里就找不到 Rust 那一套了。界面本身全在 `ui/` 里：
//!
//! ```text
//! ui/Settings.tsx          整页设置
//! ui/lib/api.ts            后端接口（唯一 import cvtmouse/native 的地方）
//! ui/lib/desktop.ts        托盘图标 + 关窗收进托盘
//! ui/components/ui.tsx     卡片、一行设置、读数（只有布局，没有控件）
//! ui/format.ts             倍率/速度的文字
//! ```
//!
//! 启动：`nsh run App.tsx`（QuickJS，TSX 热更新）。

import { mount } from "nsh";
import { Settings } from "./ui/Settings";

mount(Settings);
