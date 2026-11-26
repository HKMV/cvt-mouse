//! 钩子线程与回调：装 `WH_MOUSE_LL`、注册热键、跑消息循环。
//!
//! 回调里**只做判断和一次入队**，注入交给 [`super::pump`]（见模块文档的四条规矩）。

use std::ptr;
use std::sync::atomic::Ordering;

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, RegisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, MSLLHOOKSTRUCT, MSG, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_MOUSE_LL, WM_HOTKEY, WM_MOUSEWHEEL, WM_QUIT,
};

use super::{decide, lock, pump, HOOK_THREAD, INSTALLED, MAGIC};
use crate::core::policy::Act;
use crate::infra::log;

/// 修饰键：`decide` 里要按配置判断「按住就不改」，键码从这里拿。
pub(crate) use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_CONTROL, VK_MENU, VK_SHIFT};

/// 热键的 id（`RegisterHotKey`/`WM_HOTKEY` 用，随便一个不冲突的值）。
const HOTKEY_ID: i32 = 0x4356;

/// 起钩子线程。
pub fn spawn() {
    let spawned = std::thread::Builder::new()
        .name("cvtmouse-hook".to_string())
        .spawn(run);
    if let Err(error) = spawned {
        log::warn(format!("钩子线程起不来：{error}"));
    }
}

/// 让钩子线程的消息循环收到 `WM_QUIT` 并返回。
pub fn quit_thread(id: u32) {
    unsafe { PostThreadMessageW(id, WM_QUIT, 0, 0) };
}

fn run() {
    unsafe {
        HOOK_THREAD.store(GetCurrentThreadId(), Ordering::SeqCst);
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(hook_proc), ptr::null_mut(), 0);
        if hook.is_null() {
            INSTALLED.store(false, Ordering::SeqCst);
            log::warn(
                "装不上全局鼠标钩子：滚轮不会变速。常见原因是安全软件拦了低级键盘/鼠标钩子。",
            );
            return;
        }
        INSTALLED.store(true, Ordering::SeqCst);
        log::info("全局滚轮钩子已装上（WH_MOUSE_LL）");

        // 热键是「一键停用」的保险：系统里抢这个组合的程序不少，所以按顺序试几个。
        let candidates: [(u32, u32, &str); 3] = [
            (MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, 'S' as u32, "Ctrl+Alt+S"),
            (
                MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_NOREPEAT,
                'S' as u32,
                "Ctrl+Alt+Shift+S",
            ),
            (MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, 0x7B, "Ctrl+Alt+F12"),
        ];
        let mut bound = "";
        for (modifiers, key, name) in candidates {
            if RegisterHotKey(ptr::null_mut(), HOTKEY_ID, modifiers, key) != 0 {
                bound = name;
                break;
            }
        }
        if bound.is_empty() {
            log::warn("热键都没注册上（Ctrl+Alt+S 等被占用），开关请用界面或托盘菜单");
        } else {
            log::info(format!("热键 {bound}：一键开关变速"));
        }

        let mut message: MSG = std::mem::zeroed();
        // GetMessageW 返回 0 是 WM_QUIT，-1 是出错；两种都退出。
        while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
            if message.message == WM_HOTKEY {
                let next = !lock().config.enabled;
                super::set_enabled(next);
            }
        }
        UnhookWindowsHookEx(hook);
        INSTALLED.store(false, Ordering::SeqCst);
    }
}

/// 钩子回调。**必须快**：这里只做判断和一次入队。
unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && wparam as u32 == WM_MOUSEWHEEL {
        let info = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        // 自己注入的事件直接放行，否则会无限递归。魔数只比低 32 位（见 MAGIC 的说明）。
        //
        // 别的软件注入的事件**不跳过**：有些鼠标驱动/触控板就是用注入的方式送滚轮的，
        // 跳过它们等于让那类设备完全失去变速。防回灌靠另外三道闸：32 位魔数、
        // 「单事件超过 2 格就放行」、以及「速度超过 200 格/秒就放行」。
        if info.dwExtraInfo as u32 != MAGIC {
            let delta = ((info.mouseData >> 16) & 0xFFFF) as u16 as i16;
            match decide(delta, info.dwExtraInfo, info.flags) {
                Act::Pass => {}
                Act::Swallow => return 1,
                Act::Wheel(notches) => {
                    pump::queue(notches);
                    return 1;
                }
            }
        }
    }
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

/// 某个键现在按着吗。
pub(crate) fn key_down(key: u16) -> bool {
    unsafe { GetAsyncKeyState(key as i32) < 0 }
}

#[cfg(test)]
mod tests {
    /// 按 Windows 的读法解出滚轮增量（和 `hook_proc` 里的解码同一套）。
    fn decode(mouse_data: u32) -> i16 {
        ((mouse_data >> 16) & 0xFFFF) as u16 as i16
    }

    #[test]
    fn high_word_decode_reads_what_the_system_builds() {
        // 系统把增量放进高 16 位后，钩子这一侧读出来要还原成原来的增量。
        assert_eq!(decode((120u32) << 16), 120);
        assert_eq!(decode((-1440i32 as u32) << 16), -1440);
    }
}
