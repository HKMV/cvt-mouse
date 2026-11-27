//! Win32 的几件杂事：权限、提权重启、前台程序名、系统滚轮行数。
//!
//! 和 [`crate::runtime`] 分开，是因为这些都在**钩子回调之外**跑：可以慢、可以
//! 分配内存、可以弹 UAC，不受 `LowLevelHooksTimeout` 约束。

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr;

use windows_sys::Win32::Foundation::{BOOL, CloseHandle, HWND, LPARAM, POINT};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GA_ROOT, GW_OWNER, GetAncestor, GetCursorPos, GetWindow, GetWindowThreadProcessId,
    IsWindowVisible, MB_ICONERROR, MB_SETFOREGROUND, MessageBoxW, SPI_GETWHEELSCROLLLINES,
    SPI_SETWHEELSCROLLLINES, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SW_SHOWNORMAL,
    SystemParametersInfoW, WindowFromPoint,
};

/// UTF-16 + NUL。
fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect()
}

/// 弹一个错误框，用在进程刚起来、还没有日志也没有界面的地方。
///
/// 唯一的调用点是内嵌资源解包失败（[`crate::infra::embed`]）：那种情况下 nsh 的
/// runner 只会说「Cannot read the app bundle in …\nsh」，用户看不出所以然。
/// `MB_SETFOREGROUND` 不能省——不然框可能开在别的窗口后面，看起来就是「点了没反应」。
pub fn alert(text: &str) {
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            wide(text).as_ptr(),
            wide("CvtMouse").as_ptr(),
            MB_ICONERROR | MB_SETFOREGROUND,
        );
    }
}

/// 光标下的目标窗口：程序名 + 这个程序是不是提权的。
#[derive(Clone, Debug, Default)]
pub struct Target {
    /// 进程的 exe 名（小写，带扩展名）。
    pub exe: String,
    /// 这个进程是不是以管理员身份在跑。
    pub elevated: bool,
}

/// 当前进程是不是以管理员身份在跑。
///
/// 结果不会变，所以只查一次：这个函数在滚轮回调里也要用（判断目标窗口能不能注入），
/// 每格都开一次令牌是没必要的开销。
pub fn is_elevated() -> bool {
    static CACHE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CACHE.get_or_init(probe_elevated)
}

fn probe_elevated() -> bool {
    unsafe {
        let mut token = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut returned = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

/// 以管理员身份重新拉起自己。`true` 表示新进程已经起来了，调用方应当退出。
pub fn relaunch_elevated() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let file = wide(&exe.to_string_lossy());
    let verb = wide("runas");
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW 的返回值 > 32 才算成功。
    result as isize > 32
}

/// 光标下面那个窗口所属进程的 exe 名（小写，带扩展名）。
///
/// 用「光标下」而不是「前台窗口」：Windows 10 起滚轮默认发给鼠标指着的那个窗口，
/// 哪怕它不是前台——按前台判会认错程序。
pub fn exe_under_cursor() -> String {
    cursor_target().exe
}

/// 光标下面那个窗口的程序名，以及它是不是提权的。
///
/// 提权与否决定了我们的注入能不能落地：UIPI 会丢掉低完整性进程发给高完整性窗口的
/// `SendInput`。所以这种窗口必须**原样放行**——拦下来又发不出去，滚轮就死了。
pub fn cursor_target() -> Target {
    unsafe {
        let mut point = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut point) == 0 {
            return Target::default();
        }
        let mut hwnd = WindowFromPoint(point);
        if hwnd.is_null() {
            return Target::default();
        }
        let root = GetAncestor(hwnd, GA_ROOT);
        if !root.is_null() {
            hwnd = root;
        }
        Target {
            exe: exe_of_window(hwnd),
            elevated: window_elevated(hwnd),
        }
    }
}

/// 某个窗口所属的进程是不是提权的。
fn window_elevated(hwnd: HWND) -> bool {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return false;
        }
        // 权限不够的进程打不开，`OpenProcess` 直接返回空——那就当它不是提权的。
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut token = ptr::null_mut();
        let opened = OpenProcessToken(process, TOKEN_QUERY, &mut token);
        CloseHandle(process);
        if opened == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut returned = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

/// 某个窗口所属进程的 exe 名。
pub fn exe_of_window(hwnd: HWND) -> String {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return String::new();
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return String::new();
        }
        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut size);
        CloseHandle(process);
        if ok == 0 {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buffer[..size as usize]);
        path.rsplit(['\\', '/'])
            .next()
            .unwrap_or(&path)
            .to_ascii_lowercase()
    }
}

/// 去掉系统沿窗口外圈画的那条 1px 描边，并显式要圆角。
///
/// Windows 11 会给每个顶层窗口描一条 1px 边框（颜色取系统强调色，没开强调色就是
/// 深灰），白底窗口上看起来就是「突兀的一圈黑线」。这条线是 DWM 画的，TSX 侧改不了，
/// 只能进程内用 `DwmSetWindowAttribute` 改：`DWMWA_BORDER_COLOR` 设成
/// `DWMWA_COLOR_NONE` 让它不画；圆角偏好再显式要一次（默认本来就是圆角，写上是为了
/// 不受窗口样式影响）。
///
/// 两项都设成功才返回 `true`。Win10 上这两个属性不存在，返回 `false`，不影响别的功能。
pub fn soften_window_frame() -> bool {
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWA_BORDER_COLOR: u32 = 34;
    const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFE;
    const DWMWCP_ROUND: u32 = 2;

    let Some(hwnd) = own_window() else {
        return false;
    };
    unsafe {
        let border = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &DWMWA_COLOR_NONE as *const _ as *const _,
            4,
        );
        let corner = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &DWMWCP_ROUND as *const _ as *const _,
            4,
        );
        border == 0 && corner == 0
    }
}

/// 本进程的那个主窗口：可见、没有 owner、属于当前进程的第一个顶层窗口。
fn own_window() -> Option<HWND> {
    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        unsafe {
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == GetCurrentProcessId() && IsWindowVisible(hwnd) != 0 {
                if GetWindow(hwnd, GW_OWNER).is_null() {
                    *(lparam as *mut Option<HWND>) = Some(hwnd);
                    return 0; // 找到就停
                }
            }
            1
        }
    }
    let mut found: Option<HWND> = None;
    unsafe {
        EnumWindows(Some(callback), &mut found as *mut _ as LPARAM);
    }
    found
}

/// 系统当前的「每格滚几行」（`WHEEL_PAGESCROLL` 会读成 `u32::MAX`）。
pub fn wheel_lines() -> u32 {
    let mut value = 0u32;
    unsafe {
        SystemParametersInfoW(
            SPI_GETWHEELSCROLLLINES,
            0,
            &mut value as *mut _ as *mut _,
            0,
        );
    }
    value
}

/// 改系统的「每格滚几行」并写进用户配置。
pub fn set_wheel_lines(lines: u32) -> bool {
    unsafe {
        SystemParametersInfoW(
            SPI_SETWHEELSCROLLLINES,
            lines,
            ptr::null_mut(),
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        ) != 0
    }
}
