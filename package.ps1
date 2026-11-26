# 打 Windows 包，并把它压到最小：`nsh package --target windows` 出一条
# `.nsh\out\CvtMouse\`（exe + bundle）和一个同名 zip。
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
#
# 用最省的那组：`opt-level=z`（体积优先）、`lto=fat`（跨 crate 内联后能砍掉
# 没人调的泛型实例）、`codegen-units=1`（给 LTO 完整视野）、`strip=symbols`。
# 代价是打包从 ~1 分钟变成 ~7 分钟（LTO 是全局的，改一行也要重编）。
#
# 没开 `panic=abort`：还能再省 2~3MB，但任何 panic 会变成进程直接消失、
# 连日志都不留；这个程序的价值就在「出问题能查」，不划算。
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

$sw = [Diagnostics.Stopwatch]::StartNew()
nsh package --target windows
if ($LASTEXITCODE -ne 0) { throw "nsh package 失败（exit $LASTEXITCODE）" }

$exe = ".nsh\out\CvtMouse\CvtMouse.exe"
$zip = ".nsh\out\CvtMouse.zip"
foreach ($file in @($exe, $zip)) {
  if (-not (Test-Path $file)) { throw "没生成 $file" }
}
$exeBytes = (Get-Item $exe).Length
$zipBytes = (Get-Item $zip).Length
""
"用时 {0}s（{1}）" -f [int]$sw.Elapsed.TotalSeconds, $(if ($Fast) { "opt-level=s + lto=thin" } else { "opt-level=z + lto=fat" })
"  exe  {0:N2} MB  {1}" -f ($exeBytes / 1MB), $exe
"  zip  {0:N2} MB  {1}" -f ($zipBytes / 1MB), $zip
"  基线（不设 profile 变量）: exe 50.09 MB / zip 19.06 MB"

if ($Open) { Start-Process explorer.exe (Resolve-Path ".nsh\out\CvtMouse").Path }
