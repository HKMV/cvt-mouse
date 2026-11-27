# 打 Windows 包，并把它压到最小：`nsh package --target windows` 出一条
# `.nsh\out\CvtMouse\`（exe + bundle）和一个同名 zip。
#
# 打完还会把整个 `nsh/` 目录**内嵌进 exe**（尾巴上挂 payload，见下面那段和各处注释），
# 所以最终产物是一个 exe：用户拿到的不再有 `nsh\` 目录。解包逻辑在 `src/infra/embed.rs`。
#
# # 为什么要设这几个环境变量
#
# 包里的 50MB 里 99% 是 **nsh 的运行时 runner**（`.text` 37MB + `.rdata` 13MB），
# 我们自己的东西只有 bundle 29KB + 图标 58KB。那个 runner 是 nsh 自己用 cargo
# 编的（`nsh.json` 的 `"rust": "."` 只是把本 crate 编进去），所以它在**别的
# workspace** 里编译，我们 `Cargo.toml` 的 `[profile.release]` 对它无效。
#
# 唯一能改的入口是 cargo 的 profile 环境变量——cargo 启动时读它们，对这次
# 调用里的所有 crate 生效（含本 crate）。实测（本机，同一份代码）：
#
#   | 配置                              | exe     | zip     |
#   |-----------------------------------|---------|---------|
#   | 默认（不设这些变量）              | 50.09MB | 19.06MB |
#   | opt-level=s + lto=thin + cgu=1    | 35.07MB | 13.83MB |
#   | opt-level=z + lto=fat + cgu=1     | 29.94MB | 12.65MB |
#   | 上面那组 + panic=abort            | 19.75MB |  9.81MB |
#
# 用最省的那组：`opt-level=z`（体积优先）、`lto=fat`（跨 crate 内联后能砍掉
# 没人调的泛型实例）、`codegen-units=1`（给 LTO 完整视野）、`strip=symbols`、
# `panic=abort`。代价是打包从 ~1 分钟变成 ~7 分钟（LTO 是全局的，改一行也要重编）。
#
# 为什么这里是环境变量而不是 `Cargo.toml` 的 `[profile.release]`：runner 在
# `.nsh/rust` 那个 workspace 里编，本 crate 只是它的一个成员，成员的 profile 会被
# cargo 忽略（还会打一条 "profiles for the non root package will be ignored"）。
# 同一个 `[profile.release]` 放在 `Cargo.toml` 里只有一层用处：`cargo test --release`
# 时给本 crate 自己的测试用。
#
# `panic=abort` 的代价：nsh 运行时里有 26 处 `catch_unwind`（`nsh-core/main_thread.rs`、
# `nsh-js/classes.rs`、`nsh-engine/host_channel.rs`），abort 之后这些兜底不再拦截 panic，
# Rust 侧一 panic 进程就退出。nsh 的 panic hook 仍会写日志（`nsh-engine/logging.rs`
# 里 `set_hook`），所以「连日志都不留」不成立，丢的是「拦住 panic 继续跑」。
param(
  # 改了 Rust 代码后正常跑；只想看体积、不想等 LTO 时用 -Fast（省一半时间，包大 5MB）。
  [switch]$Fast,
  # 打完直接打开输出目录。
  [switch]$Open
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:LOCALAPPDATA\nsh\bin;$env:PATH"

if ($Fast) {
  $env:CARGO_PROFILE_RELEASE_OPT_LEVEL = "s"
  $env:CARGO_PROFILE_RELEASE_LTO = "thin"
} else {
  $env:CARGO_PROFILE_RELEASE_OPT_LEVEL = "z"
  $env:CARGO_PROFILE_RELEASE_LTO = "fat"
}
$env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "1"
$env:CARGO_PROFILE_RELEASE_STRIP = "symbols"
$env:CARGO_PROFILE_RELEASE_PANIC = "abort"

$sw = [Diagnostics.Stopwatch]::StartNew()
nsh package --target windows
if ($LASTEXITCODE -ne 0) { throw "nsh package 失败（exit $LASTEXITCODE）" }

$exe = ".nsh\out\CvtMouse\CvtMouse.exe"
$zip = ".nsh\out\CvtMouse.zip"
foreach ($file in @($exe, $zip)) {
  if (-not (Test-Path $file)) { throw "没生成 $file" }
}

# ---- 把 nsh/ 整个目录内嵌进 exe -------------------------------------------
# nsh 的 runner 只认「exe 旁边的 nsh/」或者 NSH_APP_DIR，`nsh package` 没有内嵌开关
# （理由和源码依据写在 src/infra/embed.rs 的文件头）。这里把目录按
# `[u16 名长][名 UTF-8][u32 数据长][数据]` 拼成 payload 挂在 exe 尾巴上，末尾再跟
# `[u64 payload 长度][16 字节魔数]`；程序自己在 CRT 初始化段把 payload 解到
# `%LOCALAPPDATA%\CvtMouse\runtime\<摘要>\`，然后把 NSH_APP_DIR 指过去。
$appDir = ".nsh\out\CvtMouse"
$nshDir = Join-Path $appDir "nsh"
$trailer = [System.Text.Encoding]::ASCII.GetBytes("CVTMOUSE-EMBv1") + [byte[]](0, 0)
if ($trailer.Length -ne 16) { throw "魔数不是 16 字节（$($trailer.Length)）——和 src/infra/embed.rs 的 MAGIC 对不上" }

$payload = [System.IO.MemoryStream]::new()
$payloadWriter = [System.IO.BinaryWriter]::new($payload)
# 名字必须是**相对 nsh 目录**的：这里取绝对路径来截，别拿相对路径的长度去截绝对路径
# （截错了解出来的会是 `CvtMouse\.nsh\out\…\nsh\app.json` 这种嵌套残骸，runner 找不到
# app.json，只会弹一句 "Cannot read the app bundle"）。
$nshFull = (Resolve-Path $nshDir).Path
foreach ($file in Get-ChildItem $nshFull -Recurse -File | Sort-Object FullName) {
  # 包里用 `/`：解包侧按相对路径拼，Windows 也认。
  $name = $file.FullName.Substring($nshFull.Length + 1).Replace('\', '/')
  if ($name -match '[:\\]' -or $name -match '^\.\.') { throw "条目名不像相对路径：$name" }
  $nameBytes = [System.Text.Encoding]::UTF8.GetBytes($name)
  $data = [System.IO.File]::ReadAllBytes($file.FullName)
  $payloadWriter.Write([uint16]$nameBytes.Length)
  $payloadWriter.Write($nameBytes)
  $payloadWriter.Write([uint32]$data.Length)
  $payloadWriter.Write($data)
}
$payloadWriter.Flush()
$payloadBytes = $payload.ToArray()
$payloadWriter.Dispose()

# 打出来的包必须能自己跑起来：两样 runner 一进去就读的东西缺一个都是坏的。
foreach ($required in @("app.json", "bundle.nshc")) {
  if (-not (Test-Path (Join-Path $nshFull $required))) { throw "nsh 目录里缺 $required" }
  $escaped = [regex]::Escape($required)
  $found = [System.Text.Encoding]::UTF8.GetString($payloadBytes) -match $escaped
  if (-not $found) { throw "payload 里找不到 $required，条目名大概截错了" }
}

$exeStream = [System.IO.File]::Open($exe, 'Append', 'Write')
$exeStream.Write($payloadBytes, 0, $payloadBytes.Length)
$exeStream.Write([BitConverter]::GetBytes([uint64]$payloadBytes.Length), 0, 8)
$exeStream.Write($trailer, 0, $trailer.Length)
$exeStream.Dispose()

# 目录已经进 exe 了，删掉——留着的话 exe 会先用它（runner 默认先看 exe 旁边），
# 内嵌资源等于没生效，测出来的结果也不算数。
Remove-Item $nshDir -Recurse -Force
# `nsh package` 顺手写的 Inno Setup 脚本引用了 nsh\icon.ico，跟着 nsh/ 一起删；
# 本机没装 Inno Setup，从来没用过它。
Remove-Item ".nsh\out\CvtMouse.iss" -Force -ErrorAction SilentlyContinue
# nsh 打的 zip 里还是「exe + nsh/」的旧结构，按内嵌后的目录重打一份。
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path "$appDir/*" -DestinationPath $zip -Force

$exeBytes = (Get-Item $exe).Length
$zipBytes = (Get-Item $zip).Length
""
"用时 {0}s（{1}）" -f [int]$sw.Elapsed.TotalSeconds, $(if ($Fast) { "opt-level=s + lto=thin" } else { "opt-level=z + lto=fat" })
"  exe  {0:N2} MB  {1}（其中内嵌资源 {2:N0} B）" -f ($exeBytes / 1MB), $exe, $payloadBytes.Length
"  zip  {0:N2} MB  {1}（只有 exe，没有 nsh/ 目录）" -f ($zipBytes / 1MB), $zip
"  基线（不设 profile 变量）: exe 50.09 MB / zip 19.06 MB"

if ($Open) { Start-Process explorer.exe (Resolve-Path ".nsh\out\CvtMouse").Path }
