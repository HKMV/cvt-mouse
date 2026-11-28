# 打包体积与编译优化

这份文件只讲一件事：**怎么把 exe 压到最小，以及哪些路试过、为什么不走**。所有数字都是
本机同一份代码实测的字节数（`MB` 按 1 MiB = 1,048,576 B 折算），不是估算。

当前（stable 工具链，`.\package.ps1`）：

| 产物 | 字节 | 显示 |
| --- | --- | --- |
| `CvtMouse.exe` | 16,905,579 B | 16.12 MB |
| `CvtMouse.zip`（只有 exe） | 8,126,741 B | 7.75 MB |

起点是 0.1.3 的 32,572,284 B（31.06 MB），也就是 **exe 少了 48%**。

## 包里都是什么

99% 是 **nsh 的运行时 runner**：我们自己的东西只有 JS bundle 29 KB + 图标 58 KB +
内嵌资源 89,939 B。runner 是 nsh 用 cargo 在 `.nsh/rust` 那个 workspace 里编的，本仓库
`Cargo.toml` 的 `[profile.release]` 对它**无效**（cargo 会忽略成员 crate 的 profile，
还会打一条 `profiles for the non root package will be ignored`）。

唯一能改的入口是 **cargo 的 profile 环境变量**：cargo 启动时读它们，对这次调用里的所有
crate 生效。`package.ps1` 就是把这几个变量设好再调 `nsh package`：

```powershell
$env:CARGO_PROFILE_RELEASE_OPT_LEVEL     = "z"
$env:CARGO_PROFILE_RELEASE_LTO           = "fat"
$env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "1"
$env:CARGO_PROFILE_RELEASE_STRIP         = "symbols"
$env:CARGO_PROFILE_RELEASE_PANIC         = "abort"
```

历史各档（同一份代码，只改这些变量）：

| 配置 | exe | zip |
| --- | --- | --- |
| 不设（`nsh package` 直跑） | 50.09 MB | 19.06 MB |
| `-Fast`：`opt-level=s` + `lto=thin` | 35.07 MB | 13.83 MB |
| `opt-level=z` + `lto=fat`（0.1.3 及以前） | 30.97 MB | 13.03 MB |
| 上面那组 + `panic=abort` | 19.75 MB | 9.81 MB |
| 再去掉没用到的 `nsh-base` 依赖（= 现在这版） | **16.12 MB** | **7.75 MB** |
| ~~再加 nightly `build-std`~~（0.1.4 那版发布物用过，已弃用） | 15.16 MB | 7.40 MB |

## 四步压下来的账

| 动作 | runner exe | 省 |
| --- | --- | --- |
| 起点（0.1.3 发布版，CI 产物） | 32,572,284 B | — |
| `panic=abort` | 20,557,824 B | −11.8 MB |
| 去掉没人用的 `nsh-base` 依赖 | 16,756,224 B | −3.63 MB |
| ~~nightly `-Zbuild-std` + `-Zlocation-detail=none`~~（弃用，见下） | 15,751,168 B | −0.96 MB |

最后成品还要加内嵌资源 89,939 B，所以 stable 这版是 16,905,579 B。

### 1. `panic=abort`

runner 里有 26 处 `catch_unwind`（`nsh-core/main_thread.rs`、`nsh-js/classes.rs`、
`nsh-engine/host_channel.rs`）把 Rust panic 拦下来转成错误。开 abort 之后这些兜底不再
生效——**Rust 侧一 panic 进程就退出**，省掉的是所有展开表 / landing pad，一共 11.8 MB。

panic hook 仍然会跑（`nsh-engine/logging.rs` 里 `set_hook`），所以日志和位置还写得出，
丢的是「拦下来继续跑」。要去掉这条：删 `package.ps1` 里
`CARGO_PROFILE_RELEASE_PANIC` 那一行。

### 2. 去掉 `nsh-base` 依赖（这条是个真错误）

`Cargo.toml` 里曾经写着 `nsh-base = "0.7.2"`，默认特性 = `editor` / `highlight` /
`plot` / `rich-editor` / `term`，而 `src/` 里**一次都没用到 `nsh_base`**（只碰
`nsh_core::prelude`）。cargo 的 feature 统一让 nsh 自己的 `leave out highlight` 失效，
`syntect` + `oniguruma` + `ropey` + `vte` + `curve25519-dalek` 全被链进 exe（成品里
`onig` 有 137 处字符串）。

```text
$ cargo tree -i syntect
syntect v5.3.0
├── nsh-base v0.7.2
│   ├── cvtmouse v0.1.4 (D:\dev\project\i\rust\CvtMouse)   ← 就是这行
│   └── nsh-app-cvtmouse-windows-app v0.0.0
```

删掉那行（宿主 crate 自己声明的是 `default-features = false`）：

| | runner exe |
| --- | --- |
| 带 `nsh-base`（默认特性） | 20,557,824 B |
| 删掉 `nsh-base` | **16,756,224 B（−3.63 MB）** |

教训：**自己 crate 的依赖特性也会决定 runner 里有什么**。删无用依赖、关默认特性都要做，
nsh 打印的 `leave out` 不代表真的没编进去。

### 3. `opt-level` 用 `z` 不用 `s`

同一份代码只差这一项：`z` 的 runner 19.61 MB，`s` 是 22.99 MB（差 3.4 MB）。一切为了
体积就用 `z`；`-Fast` 那档（`s` + `thin` LTO）只用来临时看效果。

## runner 的 feature 怎么定的

哪些模块编进去是 **nsh 自己按「这个 app 实际 import / link 了什么」挑的**，
`nsh package -v` 会把决定逐条打出来（实测 0.7.2）：

```text
keep `js`: the app has a JS bundle
keep `net`: `fetch` and remote images depend on data, not imports
keep `svg`: an image's format depends on data, not imports
keep `plot`: the app links `nsh_part_plot`
leave out `shader` `editor` `rich-editor` `term` `emoji-picker` `highlight` `math`
          `zlib` `crypto` `websocket` `updater` `webview` `sound` `media`
```

改这个判断本身是改不动的，三条都试过：

* 宿主 crate `.nsh/rust/windows-app/Cargo.toml` 里有一条全量的 `default = [...]`，但每次
  `nsh package` 都会把它重写回去（改完再跑，4 秒后就被覆盖）；
* `nsh.json` 的 `"features"` 说的是 **app 自己 crate 的** Cargo feature，不是 runner 的
  （nsh 帮助原文：*The crate's Cargo features to turn on*）。拿一个不存在的 feature 名做
  探针塞进去，`nsh package -v` 的 keep/leave 列表一字未变，也不报错；
* `nsh package` 没有 `--features`，也没有相关的环境变量。

### `wgpu` 根本不在包里

它在 `nsh-engine` 里是 `optional = true`，由 `shader` feature 决定，而 runner 的判断是
`leave out shader`：

```text
$ cargo tree --no-default-features --features js,net,svg,plot -i wgpu
error: package ID specification `wgpu` did not match any packages
```

扫成品也印证（`.nsh/out/CvtMouse/CvtMouse.exe`）：`wgpu` 0 处、`naga::` 0 处、
`D3D12` / `DXIL` / `Vulkan` / `VK_KHR` / `SPIR-V` 全 0。16 处 `naga` 是 ICU 里的文字名
（`Devanagari`、`Nandinagari`、`Nag_Mundari`），2 处 `wgsl` 是 `nsh-core/src/shader.rs`
的文档/API 字符串——那个模块常驻编译，WGSL 前端在 `shader` feature 后面。

正向对照：故意带上 `shader` 编一次，runner 是 23,275,008 B（+2.59 MB），里面 `wgpu` 202 处、
`naga::` 210 处（`naga::front::wgsl::lower…`）、`D3D12` 35 处、`Vulkan` 19 处、`VK_KHR` 46 处
——所以它在不在，一眼就能看出来。0.1.3 的发布物（32,572,284 B）同样全是 0。

## 试过但没采用的

### nightly `-Zbuild-std` + `-Zlocation-detail=none`

省 0.96 MB（runner 16,756,224 → 15,751,168 B）的代价：

- 打包机要 `rustup toolchain install nightly --profile minimal --component rust-src`；
- `-Zlocation-detail=none` 把 panic 信息里的文件名和行号去掉（abort 之后本来也没有回溯）；
- `-Zbuild-std` 是 cargo 参数，塞不进 `RUSTFLAGS`（rustc 报 unknown unstable option），
  nsh 也不会替我们传，只能靠 cargo 的 unstable 配置环境变量
  `CARGO_UNSTABLE_BUILD_STD=std,panic_abort` + `RUSTUP_TOOLCHAIN=nightly` 拐进去。

**结论：不用。** 全项目统一 stable（`rustup show active-toolchain` 那个），少一层安装和
一个会让人看不懂的构建依赖。0.1.4 那一版发布物是用它打的，从下一版起回到 stable。

### `panic="immediate-abort"`

当前 nightly 上要整条 std 一起换 panic 策略才编得过（`error: the crate core was compiled
with a panic strategy which is incompatible with immediate-abort`），收益只有几百 KB，
而且 panic 完全不格式化、连日志都没有。不做。

### UPX

单文件能压到 7 MB 级，但杀软误报率高、每次启动要先解压，对一个常驻托盘程序不划算。不做。

## 还能压的（都被 nsh 挡着）

留下的 `net` / `svg` 值多少钱，直接按 feature 集合重编量（同一套 profile）：

| feature 集合 | runner exe | 差 |
| --- | --- | --- |
| `js,net,svg,plot`（实际用的） | 32,324,096 B | — |
| `js,svg,plot`（去掉 `net`） | 31,907,840 B | −416 KB |
| `js,plot`（再去掉 `svg`） | 29,782,016 B | −2.03 MB |

`net` 拖进来的是 `ureq` + `rustls-platform-verifier` 一套 TLS，`svg` 拖进来的是 `resvg` /
`usvg` / `tiny-skia` / `fontdb` / `harfrust` / `skrifa` 一整套光栅化，一共 34 个 crate。两个
都关不掉（nsh 判断不了数据驱动的用法），那 2.44 MB 得等上游给个声明入口。

## 分发包压缩：只用 zip

zip 只能 deflate。同一个 exe 用 7z 的 LZMA2（`-mx9 -ms=on`）能再小将近三分之一：

| | 字节 | 显示 |
| --- | --- | --- |
| deflate（`Compress-Archive`） | 8,126,741 B | 7.75 MB |
| LZMA2（`7z a -t7z -mx=9`） | 5,230,915 B | 4.99 MB |

0.1.4 那版两种都发过，**0.1.5 起只发 zip**：多一种包就多一套解压说明和校验，
省下来的那 2.7 MB 下载量不值得。要临时压一份看体积：

```powershell
7z a -t7z -mx=9 -ms=on .nsh\out\CvtMouse.7z .nsh\out\CvtMouse\CvtMouse.exe
```

## 编译时间

换档后 LTO 全量重编约 2~7 分钟（LTO 是全局的，改一行也要重编）；代码没动时再跑只要 2 秒。
