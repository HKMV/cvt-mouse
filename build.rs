//! 构建脚本：把 `app.rc` 里的版本占位符换成 `Cargo.toml` 的版本，再编进资源段。
//!
//! 为什么需要它：`nsh package` 会把自己的 runner 复制成 `<名字>.exe` 并把图标补进去，
//! 但**不写版本资源**（`nsh.exe` 里连 `VS_VERSION_INFO` 这个字符串都没有），于是
//! exe 属性页的「文件版本 / 产品版本」是空的。资源只能由我们自己的 crate 带进去。
//!
//! 版本号只有一处真相：`Cargo.toml` 的 `version`。CI 会把 tag 和它核一遍
//! （见 `.github/workflows/release.yml`），所以这里不再手工同步。

use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");

    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let template = fs::read_to_string("app.rc").expect("读不到 app.rc");
    let rc = template
        .replace("@VERSION_QUAD@", &version_quad(&version))
        .replace("@VERSION@", &version);

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("没有 OUT_DIR")).join("app.rc");
    fs::write(&out, rc).expect("写不进生成的 app.rc");

    if cfg!(windows) {
        // 第二个参数是「要不要顺带嵌 manifest」：不嵌，窗口/DPI 由 nsh 的宿主处理。
        let _ = embed_resource::compile(&out, embed_resource::NONE);
    }
}

/// `1.2.3` → `1,2,3,0`；预发布后缀（`1.2.3-beta.1`）只取前三段数字。
///
/// 四段式版本号是 `FILEVERSION` 的硬要求（每段 0..=65535），缺的按 0 补。
fn version_quad(version: &str) -> String {
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let mut parts: Vec<u16> = core
        .split('.')
        .map(|segment| segment.parse::<u16>().unwrap_or(0))
        .collect();
    parts.resize(4, 0);
    parts
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",")
}
