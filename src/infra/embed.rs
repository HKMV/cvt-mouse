//! 把 `nsh/` 目录从 exe 自己身上取出来（内嵌资源的解包）。
//!
//! # 为什么要「挂尾巴 + 启动时解包」
//!
//! nsh 0.7.1 的 runner（`nsh-windows-app`）只会从两个地方读 `app.json` /
//! `bundle.nshc` / `assets/` / `icon.ico`：`current_exe().parent()/nsh`，或者
//! `NSH_APP_DIR` 指的目录。`nsh package` 没有内嵌开关，`AssetBase` 也只有
//! 「目录」和「HTTP」两种（没有内存态）。所以「把整个 `nsh/` 塞进 exe」只能这么做：
//! 打包脚本把目录拼成 payload 挂在 exe 尾巴上，进程起来的第一件事把它解到缓存目录，
//! 再把 `NSH_APP_DIR` 指过去——runner 就当那是它的 `nsh/`。
//!
//! # 为什么是 `.CRT$XCU` 而不是 `main`
//!
//! nsh 生成的 `main` 一进去就 `read_to_string(nsh/app.json)`，中间插不进手。
//! `.CRT$XCU` 是 CRT 的初始化段，比 `main` 早。这时候还没有日志线程、没有钩子，所以
//! 这里只做两件确定的事（读 exe 尾巴、写 90 KB），出错就弹一个框然后**什么都不改**
//! ——退回「exe 旁边得有 `nsh/`」的老样子，至少报错看得见、不是静默不启动。
//!
//! # 解到哪
//!
//! `%LOCALAPPDATA%\CvtMouse\runtime\<payload 的 64 位摘要>\`：同一个 exe 反复启动只解
//! 一次，换版本自然落到新目录（旧目录不动，用户随时可删）。先写临时目录再改名，两个
//! 实例同时启动也不会把对方写坏。
//!
//! # payload 布局（打包侧写在 `package.ps1`）
//!
//! ```text
//! payload = 条目*
//! 条目    = [u16 LE 名字长度][名字 UTF-8][u32 LE 数据长度][数据]
//! exe 尾  = [payload][u64 LE payload 总长][16 字节魔数]
//! ```

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// exe 尾巴上的魔数，正好 16 字节。
const MAGIC: &[u8; 16] = b"CVTMOUSE-EMBv1\0\0";

/// 尾巴长度：payload 总长（u64）+ 魔数。
const TRAILER: u64 = 8 + MAGIC.len() as u64;

/// 解包后必须存在的两个文件——runner 一启动就要读它们。
const REQUIRED: [&str; 2] = ["app.json", "bundle.nshc"];

/// 进程起来的第一件事：解出内嵌的 `nsh/`，把 `NSH_APP_DIR` 指过去。
///
/// `nsh run` 的开发宿主、`nsh test` 的 headless runner、`cargo test` 跑的也是这份
/// 代码，它们的 exe 尾巴上没有魔数，于是这里看一眼就直接返回，行为一个字都不变。
pub extern "C" fn boot() {
    match extract() {
        Ok(Some(dir)) => {
            // SAFETY: 这里在 CRT 的初始化段里，进程还是单线程，没有谁会并发读环境块。
            unsafe { std::env::set_var("NSH_APP_DIR", dir) };
        }
        Ok(None) => {}
        Err(error) => crate::sys::win32::alert(&format!("CvtMouse 的内置资源解不出来：\n{error}")),
    }
}

/// 解包并返回 runner 该用的 `nsh/` 目录；`Ok(None)` 表示这个 exe 没内嵌资源。
fn extract() -> Result<Option<PathBuf>, String> {
    let Some(payload) = read_payload()? else {
        return Ok(None);
    };
    let dir = cache_dir(&payload);
    if is_complete(&dir) {
        return Ok(Some(dir));
    }
    // 先写临时目录：写到一半被杀掉，也不会留下一个「看起来能用」的坏目录。
    let staging = dir.with_extension(format!("{}.tmp", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    write_entries(&payload, &staging)?;
    match std::fs::rename(&staging, &dir) {
        Ok(()) => Ok(Some(dir)),
        // 改名失败但目录已经完整：另一个实例抢先解好了，用它的（Windows 上改名到已存在
        // 的目录本来就会失败，这条是常态，不是错误）。
        Err(_) if is_complete(&dir) => {
            let _ = std::fs::remove_dir_all(&staging);
            Ok(Some(dir))
        }
        Err(error) => Err(format!("改名到 {} 失败：{error}", dir.display())),
    }
}

/// 读 exe 尾巴上的 payload。没有魔数就是普通构建，返回 `Ok(None)`。
///
/// 只读尾巴那一段，不把 30 MB 的 exe 整个读进内存。
fn read_payload() -> Result<Option<Vec<u8>>, String> {
    let exe = std::env::current_exe().map_err(|error| format!("取不到自己的路径：{error}"))?;
    let mut file =
        File::open(&exe).map_err(|error| format!("打不开自己（{}）：{error}", exe.display()))?;
    let size = file
        .metadata()
        .map_err(|error| format!("读不到自己的大小：{error}"))?
        .len();
    if size < TRAILER {
        return Ok(None);
    }
    let mut trailer = [0u8; TRAILER as usize];
    file.seek(SeekFrom::End(-(TRAILER as i64)))
        .and_then(|_| file.read_exact(&mut trailer))
        .map_err(|error| format!("读尾巴失败：{error}"))?;
    if &trailer[8..] != MAGIC {
        return Ok(None);
    }
    let length = u64::from_le_bytes(trailer[..8].try_into().expect("8 字节"));
    if length == 0 || length + TRAILER > size {
        return Err(format!("内嵌资源长度不对：{length} 字节（exe 只有 {size}）"));
    }
    let mut payload = vec![0u8; length as usize];
    file.seek(SeekFrom::End(-((TRAILER + length) as i64)))
        .and_then(|_| file.read_exact(&mut payload))
        .map_err(|error| format!("读内嵌资源失败：{error}"))?;
    Ok(Some(payload))
}

/// 解包目录：按 payload 的摘要分，换版本就换目录。
fn cache_dir(payload: &[u8]) -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("CvtMouse")
        .join("runtime")
        .join(format!("{:016x}", digest(payload)))
}

/// payload 的 64 位摘要（FNV-1a）：**只用来当目录名**，不是完整性校验（要防篡改得
/// 签名，这个程序的 exe 本来也没签名）。
fn digest(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 目录里该有的东西都在？解到一半留下的残骸要能认出来。
fn is_complete(dir: &Path) -> bool {
    REQUIRED
        .iter()
        .all(|name| dir.join(name).metadata().is_ok_and(|meta| meta.len() > 0))
}

/// 按 payload 布局把条目写进目录，子目录按需创建。
fn write_entries(payload: &[u8], dir: &Path) -> Result<(), String> {
    let entries = entries(payload)?;
    if entries.is_empty() {
        return Err("内嵌资源是空的".to_owned());
    }
    for (name, data) in entries {
        let path = dir.join(name);
        // 名字来自我们自己打的包，这里仍然挡一道：路径跑出解包目录就拒绝。
        if !path.starts_with(dir) {
            return Err(format!("条目跑出解包目录了：{name}"));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("建目录失败（{}）：{error}", parent.display()))?;
        }
        std::fs::write(&path, data)
            .map_err(|error| format!("写文件失败（{}）：{error}", path.display()))?;
    }
    Ok(())
}

/// 解析 payload 的条目表，返回 (相对路径, 内容)。
fn entries(payload: &[u8]) -> Result<Vec<(&str, &[u8])>, String> {
    let mut rest = payload;
    let mut out = Vec::new();
    while !rest.is_empty() {
        if rest.len() < 2 {
            return Err("条目头不全（缺名字长度）".to_owned());
        }
        let name_length = u16::from_le_bytes([rest[0], rest[1]]) as usize;
        rest = &rest[2..];
        if rest.len() < name_length + 4 {
            return Err("条目头不全（缺名字或数据长度）".to_owned());
        }
        let name = std::str::from_utf8(&rest[..name_length])
            .map_err(|error| format!("条目名不是 UTF-8：{error}"))?;
        rest = &rest[name_length..];
        let data_length = u32::from_le_bytes(rest[..4].try_into().expect("4 字节")) as usize;
        rest = &rest[4..];
        if rest.len() < data_length {
            return Err(format!("条目 {name} 的数据不全：要 {data_length}，剩 {}", rest.len()));
        }
        out.push((name, &rest[..data_length]));
        rest = &rest[data_length..];
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 按 package.ps1 的写法拼一份 payload。
    fn payload(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, data) in entries {
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(data);
        }
        out
    }

    #[test]
    fn entries_reads_what_package_writes() {
        let bytes = payload(&[
            ("app.json", b"{}"),
            ("bundle.nshc", b"\x00\x01\x02"),
            ("assets/ui/logo.svg", b"<svg/>"),
        ]);
        let parsed = entries(&bytes).expect("能解出来");
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0], ("app.json", b"{}".as_slice()));
        assert_eq!(parsed[1], ("bundle.nshc", b"\x00\x01\x02".as_slice()));
        assert_eq!(parsed[2], ("assets/ui/logo.svg", b"<svg/>".as_slice()));
    }

    #[test]
    fn truncated_entries_are_an_error() {
        let mut bytes = payload(&[("app.json", b"{}")]);
        bytes.truncate(bytes.len() - 1);
        assert!(entries(&bytes).is_err());
    }

    #[test]
    fn digest_depends_on_the_content() {
        assert_ne!(digest(b"a"), digest(b"b"));
        assert_eq!(digest(b"same"), digest(b"same"));
    }

    #[test]
    fn incomplete_directory_is_recognized() {
        let dir = std::env::temp_dir().join(format!("cvtmouse-embed-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        assert!(!is_complete(&dir), "空目录不算完整");
        std::fs::write(dir.join("app.json"), b"{}").expect("写 app.json");
        assert!(!is_complete(&dir), "缺 bundle.nshc 不算完整");
        std::fs::write(dir.join("bundle.nshc"), b"x").expect("写 bundle.nshc");
        assert!(is_complete(&dir), "两个都在才算完整");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
