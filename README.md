# CvtMouse 鼠标无级变速

鼠标的无级变速：按操作速度自动放大，慢的还是原生手感，越快越跟得上。

| 部分 | 状态 |
| --- | --- |
| **滚轮** | 已实现：慢滚一格一行，越滚越快，倍率跟着曲线上去 |
| **指针**（光标移动） | 计划中：同样按曲线无级变速 |

不绑定品牌和型号——只要是一只标准 HID 鼠标（有线、2.4G、蓝牙都算），装上去就生效。

用 Rust 写核心（Win32 低级鼠标钩子 + 注入），界面用 [nsh](https://github.com/longbridge/nsh)（TSX）。

## 界面

![设置页：变速曲线与实时读数](assets/screenshots/settings-zh.png)

![实现方式、修饰键放行与按应用配置](assets/screenshots/mode-zh.png)

日志级别分「界面」和「文件」两档、各四档（错误 / 警告 / 信息 / 调试）：低于选定级别的
记录左边不进界面、右边不写文件。平时界面 `信息`、文件 `调试`；要排查就把
`%APPDATA%\CvtMouse\logs` 下当天的文件发出来。

![日志级别：界面 INFO、文件 DEBUG](assets/screenshots/log-level-zh.png)

## 为什么滚轮必须走钩子

滚轮一格是鼠标固件写死的 HID 报文（`±120` 个 wheel 单位），而「一格滚几行」是**主机侧**
解释出来的。所以想变速只有两条路：

1. 改系统那个全局固定值 `SPI_SETWHEELSCROLLLINES`——所有程序同一个值，不可能「按速度变」。
2. 在输入链路上把事件拦下来，自己换算再发回去——本项目的做法（`WH_MOUSE_LL` +
   `SendInput` 重注入）。

设备协议那条路（找厂商自定义 HID 集合改固件参数）对绝大多数鼠标不存在：它们只暴露
标准 mouse/consumer 集合，没有可写的厂商集合。所以变速只能做在主机侧，好处是**换鼠标、
换接口都不用改代码**。

## 两种模式

| 模式 | 做什么 | 适合 |
| --- | --- | --- |
| **钩子 + 重注入**（默认） | 按速度曲线改每一格，能按应用分别设倍率 | 想要智能变速 |
| **兼容模式** | 只改系统的「每格滚几行」，不装钩子 | 只想整体快一点，或钩子被安全软件拦了 |

兼容模式会记住原值，切回钩子模式时自动还原。

## 界面能调什么

- **速度曲线**：三个滑块对应曲线上的三个点（起点、中速、高速），断点是「格/秒 → 倍率」，
  中间线性插值，两头取端点值。默认 `[(0,1) (6,1) (12,2.5) (24,5)]`：单滚一格不加速，
  快滚到 24 格/秒时 5 倍。
- **平滑滚动**：一次大跳拆成小格、每 8ms 发一格（积压多了就一拍多发几格追上）。这是
  「无级」手感的来源；关掉更跟手，但会一跳一跳。
- **Ctrl / Alt / Shift 按住时不改**：保留 `Ctrl+滚轮` 缩放、`Shift+滚轮` 横向滚。
- **按应用倍率**、**跳过名单**：某些程序（游戏、远程桌面）完全不接管。
- **热键**：默认 `Ctrl+Alt+S` 一键开关；被占用时依次退到 `Ctrl + Alt + Shift + S`、`Ctrl+Alt+F12`。

## 再点一次图标，不会再开一个

`nsh.json` 里 `"singleInstance": true`：第二次启动直接退出（连窗口都不建），把这次启动
交给已经在跑的那个实例；那个实例听到 `reopen` / `secondInstance` 就把窗口叫回来并置顶。
所以「以为程序关了、又点一次图标」的结果是托盘里的窗口回来——不会变成两个进程、两个
钩子抢同一只鼠标。

## 配置

`%APPDATA%\CvtMouse\config.json`，字段名 camelCase（磁盘格式 = 界面草稿格式）：

| 字段 | 含义 |
| --- | --- |
| `enabled` | 总开关 |
| `mode` | `hook` / `system` |
| `curve` | 断点 `[[格/秒, 倍率], …]` |
| `apps` | 按进程覆盖倍率，键是小写 exe 名 |
| `skip` | 完全不接管的进程 |
| `passCtrl` `passAlt` `passShift` | 按住这些键时原样放行 |
| `smooth` | 平滑输出（按节拍发，见上） |
| `systemLines` / `originalLines` | 兼容模式的每格行数 / 进兼容模式前的原值 |

日志：`%APPDATA%\CvtMouse\logs\cvtmouse-YYYY-MM-DD.log`，每行都带完整时间戳。

## 怎么读「统计」卡

| 指标 | 说明 |
| --- | --- |
| 进来 / 发给应用 | 都是 **wheel 单位**（120 = 一格），显示成格时保留一位小数 |
| 加速比 | 发给应用 ÷ 进来 |
| 原样放行 | 倍率恰好 1、按了修饰键、在跳过名单、提权窗口、认不出程序——这些都直接交给系统 |
| 注入失败 | `SendInput` 被系统拒掉的次数，正常应为 0；连续 3 次会自动关掉变速 |
| 最近一格原始值 | 标准一格是 `±120`；不是 `±120` 说明鼠标在送细粒度值（触控板、高分辨率滚轮） |

快滚时（速度 ≥ 8 格/秒）或收到非标准值时，日志里会留一行
`滚轮 原始=120 速度=17/s 倍率=7.48 输出=960 目标=xxx.exe`，每次运行最多 400 行。
**这是排查「为什么没动」的第一手证据**：有这行说明事件进来了、也算出了要发多少；
没有这行说明事件根本没到钩子（鼠标/BLE 丢报文，或者钩子没装上）。

## 已知限制

- **提权窗口**：普通权限运行时，注入会被 UIPI 丢掉。所以这类窗口一律原样放行（滚轮正常，
  但不加速）。要加速就「以管理员身份重启」。
- **认不出的程序**（受保护进程）同样放行，理由同上。
- **蓝牙鼠标快甩可能丢报文**：这是链路层的限制，主机侧收不到就无从放大。
- 安全软件（如某些国产 HIPS）可能拦低级鼠标钩子或注入。钩子装不上时界面会说明。

## 开发

```sh
nsh run          # 起开发宿主（TSX 改动热更新，Rust 改动重建并重启）
nsh check        # cargo check + 入口与 ui/ 的类型检查
nsh test         # cargo test + ui/ 下的界面测试（无窗口）
nsh build        # 只出 bundle（.nsh/out），不打包
.\package.ps1    # 打 Windows 包（exe + zip），并把体积压到最小
```

工具链是 nsh 0.7.1：`Cargo.toml` 的 `nsh-core` / `nsh-base` 必须和它对齐，CI 里也钉着
同一个版本（见下面「发布」一节）。

### 打包体积

包里的 99% 是 **nsh 的运行时 runner**（我们自己的东西只有 bundle 29KB + 图标 58KB），
而 runner 是 nsh 用 cargo 在它自己的 workspace 里编的，所以本仓库 `Cargo.toml` 的
`[profile.release]` 对它无效——只能用 cargo 的 profile 环境变量，`package.ps1` 就是
把这几个变量设好再调 `nsh package`：

| 配置 | exe | zip |
| --- | --- | --- |
| 不设（`nsh package` 直跑） | 50.09 MB | 19.06 MB |
| `-Fast`：`opt-level=s` + `lto=thin` | 35.07 MB | 13.83 MB |
| 默认：`opt-level=z` + `lto=fat` | **29.94 MB** | **12.65 MB** |

两者都带 `codegen-units=1` 和 `strip=symbols`。代价是换档后要重编 2~4 分钟
（LTO 是全局的，改一行也要重编）；代码没动时再跑只要 2 秒。没开 `panic=abort`
（还能再省 2~3MB），因为那样任何 panic 都是进程直接消失、连日志都不留。

### exe 属性页里的版本

`nsh package` 会把它的 runner 复制成 `CvtMouse.exe`、把图标补进去，但**不写版本资源**
（`nsh.exe` 里连 `VS_VERSION_INFO` 这个字符串都没有），所以属性页的「文件版本」本来是空的。
版本信息只能由我们自己的 crate 带进去：`app.rc` 写一份 VERSIONINFO，`build.rs` 把里面的
`@VERSION@` / `@VERSION_QUAD@` 换成 `Cargo.toml` 的 `version`（版本号只有一处真相，发版
流程里不用像别的项目那样 sed 改 app.rc），`embed-resource` 编成资源段随 crate 链进 exe。

实测（`package.ps1` 那档产物）：`FileVersion 0.1.0` / `ProductVersion 0.1.0` /
`ProductName CvtMouse` / `FileDescription CvtMouse 鼠标无级变速`。

目录按「谁碰什么」分开：改算法不用碰系统接口，改界面不用碰 Win32：

```text
App.tsx                 入口，只 mount（必须在仓库根：nsh 认 `nsh.json` 里的 `"rust": "."`，
                        而 `nsh.json` 必须和入口同级——`nsh package` 的说明原文是
                        「Metadata comes from `nsh.json` next to the entry」）
app.rc                  Windows 资源脚本：exe 属性页里的产品名 / 说明 / 版本（版本是占位符）
build.rs                把 app.rc 的占位符换成 Cargo.toml 的 version，再编进资源段
src/                    Rust 后端
├─ lib.rs               只有注册（nsh 要求 `#[native_module]` 在 crate 根）
├─ api/                 nsh 边界：Backend 的每个方法，JSON 文本进出
├─ runtime/             编排：把 core 的判断接到 Windows 输入链路上
│  ├─ mod.rs            全局引擎、线程生命周期、把系统信息填成 Env 交给 Policy
│  ├─ hook.rs           钩子线程：WH_MOUSE_LL 回调、热键、消息循环
│  └─ pump.rs           注入线程：按 8ms 节拍把积压的格数一格一格发出去
├─ core/                核心实现：纯逻辑，不碰 Win32、不碰文件系统
│  ├─ policy.rs         要不要改、改多少（Env/Act/Policy，有单测）
│  ├─ stats.rs          界面读的那些计数（观测数据，不参与决策）
│  ├─ curve.rs          曲线插值、速度计、累加器（有单测）
│  ├─ pacing.rs         每拍发几格（输出侧节流，有单测）
│  └─ config.rs         配置结构、默认值、校验
├─ sys/                 系统调用：只有这里能 use windows_sys
│  ├─ win32.rs          权限、提权重启、光标下窗口、系统行数
│  └─ clock.rs          时间戳（SYSTEMTIME）与单调毫秒
└─ infra/               落地：只有这里写文件
   ├─ log.rs            内存环形缓冲 + 500ms 落盘（钩子回调里禁止 IO）
   └─ store.rs          config.json 的路径与读写
ui/                     TSX 界面
├─ Settings.tsx         整页设置（顶栏兼任标题栏和拖动区，四边四角拖边也在里面）
├─ format.ts            倍率/速度的文字
├─ components/ui.tsx    卡片、一行设置、读数（只有布局，没有控件）
└─ lib/                外部依赖
   ├─ api.ts            唯一 import cvtmouse/native 的地方
   └─ desktop.ts        托盘图标 + 关窗收进托盘
assets/                 图标与截图
├─ icon.svg             设计源文件，改这个
├─ icon.png             1024×1024，`nsh.json` 的 `"icon"` 指它（`nsh package` 用）
└─ screenshots/         README 里的界面截图（中英文各一套时按 `-zh` / `-en` 分开）
.github/workflows/      推 `v*` tag 自动打包发版（见下面「发布」）
tools/notes-reflow.py   发布说明的软换行合并（CI 兜底 + 本地自查）
```

窗口是**无边框**的（`nsh.json` 里 `"frame": "none"`）：系统连标题栏、边框和三个
窗口按钮都不画，所以这些得自己做——顶栏（状态点 + 标题 + 自检 + 总开关 + 窗口按钮）
兼任标题栏和拖动区，四边四角的拖边热区补在根视图上，用的是 nsh 的
`desktop={{ windowDrag }}` / `windowControl` / `windowResize`。关闭按钮走的仍是
`closeOnPress`，所以还是收进托盘而不是退出。

图标改了 svg 之后要重新导出 png（无头 Edge 渲染，圆角外面是透明的）：

```powershell
& "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe" --headless=new --disable-gpu --hide-scrollbars `
  --default-background-color=00000000 --window-size=1024,1024 `
  --screenshot=assets\icon.png "file:///$(($PWD.Path -replace '\\','/'))/assets/icon.svg"
```

自检就一句话：**每个目录只有一个「为什么存在」，看它碰了什么就知道有没有错位**。
`core` 里出现 `windows_sys` 或 `std::fs` 是错位；`sys` 里出现 `serde_json` 是错位；
`api` 里出现 `SetWindowsHookEx` 是错位。`runtime/` 之所以留在顶层而不是塞进 `core/`，
是因为它整个职责就是碰 Win32（`SetWindowsHookEx`、`SendInput`、消息循环）；
`core/policy.rs` 相反，它只是策略，所以进 `core/`——这样「要不要变速」能脱离 Windows 单测。

外层骨架和 `nsh init --template mixed` 生成的一模一样（`Cargo.toml` 和 `src/lib.rs` 与
`App.tsx` 同级，`nsh.json` 里 `"rust": "."`）：**在 `src/` 里的是 Rust，不是界面**。
我们只多做两件事——`src/` 内部按「核心 / 系统 / 落地 / 编排 / 边界」分了层，把官方模板里
那一个 `App.tsx` 拆成了 `ui/` 几个组件（官方模板里没有 `ui/` 这个位置，界面全在 `App.tsx`）。

### 三个踩过的坑（都验证过，别再踩）

1. **`MOUSEINPUT.mouseData` 填的是原始有符号增量**（一格 = `120`），由系统自己放进
   `MSLLHOOKSTRUCT.mouseData` 的高 16 位。发送方**不要**自己 `<< 16`：那样不报错、
   `SendInput` 也返回成功，但系统会把整个 32 位值按 i16 饱和——正数变 `32767`、
   负数变 `-32768`（实测发 `0xFF880000`，钩子收到 `-32768`），等于每格只滚一行。
   接收方（钩子）才要取高 16 位。见 `runtime::pump::wheel_mouse_data` 及其单测。

2. **注入事件的标记（魔数）必须是 32 位**。`MSLLHOOKSTRUCT.dwExtraInfo` 声明成
   `ULONG_PTR`，但低级钩子只保留低 32 位：用 64 位魔数会认不出自己注入的事件
   （`0x4356544D0001` 回来变成 `0x544D0001`），于是每一格都被当成新输入再放大一遍，
   **回灌成环**——滚轮会瞬间冲到页面底部然后「不动」，日志里能看到
   `原始=32767 速度=4551/s` 这种机器级数字。比较时只比低 32 位。

3. **钩子回调里不能做 IO、不能注入、不能阻塞**。超过 `LowLevelHooksTimeout`
   （默认 300ms）系统会静默摘掉钩子，而且不报错。所以回调只算数、入队，注入交给
   独立线程按节拍做。

另外两道保险也留着：单个事件超过 2 格（`MAX_SANE_UNITS`）或速度超过 200 格/秒
（`MAX_SANE_VELOCITY`）一律原样放行——那不是人手滚出来的，放大它只会帮倒忙。

## 发布

版本号写在**两处**，发版前都要改（CI 会核对，不一致直接失败）：`Cargo.toml` 的
`version` 和 `nsh.json` 的 `version`——后者才是 `nsh package` 用的那个，也是界面和
exe 属性页显示的那个。

```sh
# 1. 写发布说明。一段一行、一项一行，别按 80 列手工折行：
#    Release 页面把单个换行当成真的换行，折过的正文每行只走半截就断。
#    本地可以先自查一次（有折行会返回 1 并告诉你能合到几行）：
python3 tools/notes-reflow.py --check notes.txt

# 2. 打 tag。--cleanup=verbatim 不能省：不加时 git 把 `## 标题` 这种以 `#`
#    开头的行当注释剥掉，Release 页面只剩一串没有分段的 bullet。
git tag -a v0.1.0 -F notes.txt --cleanup=verbatim
git push origin v0.1.0
```

推上去之后 `.github/workflows/release.yml` 在 `windows-latest` 上：

1. 检出这个 tag，装 Rust stable 和 nsh；
2. 核对 tag 与 `Cargo.toml`、`nsh.json` 的版本；
3. 跑 `package.ps1`（`opt-level=z` + `lto=fat`，约 7 分钟），产物改名成
   `CvtMouse-<tag>-windows-x64.zip`（程序目录 + README + LICENSE）；
4. 生成 `SHA256SUMS.txt`，用 tag 说明当发布正文，建 Release（已存在则只覆盖产物和正文）。

想重发同一个 tag：Actions 页面手动 Run workflow、填 tag 名即可，不用重建 tag。
发布说明里如果混进了手工折行，CI 会用 `tools/notes-reflow.py` 自动合并并留一条 warning。

**nsh 在 CI 里是钉版本的**（`env.NSH_VERSION`，现在 `v0.7.1`，和 `Cargo.toml` 的
`nsh-core` / `nsh-base` 对齐）。安装脚本默认装 `latest`，而上游 2026-11-25 把 latest
从 0.6.1 换成了 0.7.1——0.7.1 直接拒绝 0.6 时代的依赖（`cvtmouse 0.1.0 requires
nsh-core ^0.6.1`），v0.1.0 第一次发版就是这么挂的。升 nsh 是四件事一起做：改
`Cargo.toml` 那两个依赖、改这里的 `NSH_VERSION`、跑 `nsh check` / `nsh test`、
再实机看一眼界面（0.7 起窗口更晚才进合成器，`DwmSetWindowAttribute` 要重试才生效，
见 `runtime::tune_window_frame`）。

## 许可证

[MIT](LICENSE)。
