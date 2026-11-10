//! All unsafe Win32 access is confined to this module.

use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, Ordering};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM,
    STILL_ACTIVE, WPARAM,
};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAK, DWMWA_CLOAKED, DwmGetWindowAttribute, DwmSetWindowAttribute,
};
use windows::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcessId, GetExitCodeProcess, OpenProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, QueryFullProcessImageNameW,
    TerminateProcess,
};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GetForegroundWindow, GetSystemMetrics, GetWindow,
    GetWindowLongW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW,
    SM_REMOTESESSION, WM_CLOSE, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, PWSTR, w};

/// A visible, unowned, titled top-level window. `hwnd` is stored as `isize`
/// so the type stays plain data (Send) — converted back to HWND only here.
#[derive(Clone, Debug)]
pub struct TopWindow {
    pub hwnd: isize,
    pub pid: u32,
    pub title: String,
}

pub fn enum_top_windows() -> Vec<TopWindow> {
    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        unsafe {
            let out = &mut *(lparam.0 as *mut Vec<TopWindow>);
            if !IsWindowVisible(hwnd).as_bool() {
                return BOOL::from(true);
            }
            // Skip owned windows (dialogs) and tool windows.
            if GetWindow(hwnd, GW_OWNER).is_ok() {
                return BOOL::from(true);
            }
            let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
                return BOOL::from(true);
            }
            // Skip cloaked windows (suspended UWP ghosts, other virtual desktops' shells).
            let mut cloaked: u32 = 0;
            let _ = DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                &mut cloaked as *mut u32 as *mut core::ffi::c_void,
                size_of::<u32>() as u32,
            );
            if cloaked != 0 {
                return BOOL::from(true);
            }
            let mut title = [0u16; 512];
            let len = GetWindowTextW(hwnd, &mut title);
            if len <= 0 {
                return BOOL::from(true);
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == 0 {
                return BOOL::from(true);
            }
            out.push(TopWindow {
                hwnd: hwnd.0 as isize,
                pid,
                title: String::from_utf16_lossy(&title[..len as usize]),
            });
            BOOL::from(true)
        }
    }

    let mut windows: Vec<TopWindow> = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(callback),
            LPARAM(&mut windows as *mut Vec<TopWindow> as isize),
        );
    }
    windows
}

/// True when this process runs inside a remote (RDP) session, where WGL
/// exposes only OpenGL 1.1 and the glow backend cannot start.
pub fn is_remote_session() -> bool {
    unsafe { GetSystemMetrics(SM_REMOTESESSION) != 0 }
}

/// Full image path of a process. `None` for processes we cannot query
/// (typically elevated ones) — callers treat those as unmanageable.
pub fn exe_for_pid(pid: u32) -> Option<PathBuf> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let result =
            QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(handle);
        result.ok()?;
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
    }
}

/// Lowercase executable file name for a pid, e.g. "notepad.exe".
pub fn exe_name_for_pid(pid: u32) -> Option<String> {
    exe_for_pid(pid)
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
}

pub fn foreground_pid() -> Option<u32> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        (pid != 0).then_some(pid)
    }
}

/// Simulate clicking the close button: post WM_CLOSE to each window.
pub fn post_close(hwnds: &[isize]) {
    for &raw in hwnds {
        unsafe {
            let _ = PostMessageW(
                Some(HWND(raw as *mut core::ffi::c_void)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

pub fn is_alive(pid: u32) -> bool {
    unsafe {
        match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(handle) => {
                let mut code = 0u32;
                let ok = GetExitCodeProcess(handle, &mut code).is_ok();
                let _ = CloseHandle(handle);
                ok && code == STILL_ACTIVE.0 as u32
            }
            // Access denied means the process exists but is elevated.
            Err(e) => e.code() == ERROR_ACCESS_DENIED.to_hresult(),
        }
    }
}

pub fn terminate(pid: u32) -> bool {
    unsafe {
        match OpenProcess(PROCESS_TERMINATE, false, pid) {
            Ok(handle) => {
                let ok = TerminateProcess(handle, 1).is_ok();
                let _ = CloseHandle(handle);
                ok
            }
            Err(_) => false,
        }
    }
}

/// Native "open file" dialog filtered to executables. Blocks until dismissed.
pub fn pick_exe_file() -> Option<std::path::PathBuf> {
    // Filter format: pairs of display\0pattern\0, double-\0 terminated.
    let filter: Vec<u16> = "Programs (*.exe)\0*.exe\0All files (*.*)\0*.*\0\0"
        .encode_utf16()
        .collect();
    let mut file_buf = [0u16; 1024];
    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        lpstrFilter: windows::core::PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(file_buf.as_mut_ptr()),
        nMaxFile: file_buf.len() as u32,
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST,
        ..Default::default()
    };
    unsafe {
        if !GetOpenFileNameW(&mut ofn).as_bool() {
            return None;
        }
    }
    let len = file_buf.iter().position(|&c| c == 0).unwrap_or(0);
    (len > 0).then(|| std::path::PathBuf::from(String::from_utf16_lossy(&file_buf[..len])))
}

/// Milliseconds since the last keyboard/mouse input anywhere in the session.
pub fn ms_since_input() -> Option<u64> {
    unsafe {
        let mut info = LASTINPUTINFO {
            cbSize: size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if !GetLastInputInfo(&mut info).as_bool() {
            return None;
        }
        // Tick counts wrap every ~49 days; wrapping_sub handles it.
        Some(u64::from(GetTickCount().wrapping_sub(info.dwTime)))
    }
}

pub fn my_pid() -> u32 {
    unsafe { GetCurrentProcessId() }
}

/// True when the Windows display language is Polish (for Language::Auto).
pub fn system_language_is_polish() -> bool {
    const LANG_POLISH: u16 = 0x15;
    let langid = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    (langid & 0x3FF) == LANG_POLISH
}

// ---------------------------------------------------------------------------
// Start hidden in the tray (--tray)
// ---------------------------------------------------------------------------
//
// eframe 0.35 creates every root window hidden and then unconditionally
// calls `set_visible(true)` after the first painted frame
// (EpiIntegration::post_rendering), ignoring `ViewportBuilder::with_visible
// (false)`. Hidden windows are painted too, so a `--tray` start popped the
// main window up about a second after launch. That forced ShowWindow cannot
// be vetoed (stripping SWP_SHOWWINDOW in WM_WINDOWPOSCHANGING does not stop
// it), so instead the window is DWM-cloaked until the app has hidden it
// again: a cloaked window is never composited and gets no taskbar button.

/// HWND of the main window while it is cloaked; 0 = not cloaked.
static CLOAKED_MAIN: AtomicIsize = AtomicIsize::new(0);

fn set_cloak(hwnd: isize, cloak: bool) {
    let value = BOOL::from(cloak);
    unsafe {
        let _ = DwmSetWindowAttribute(
            HWND(hwnd as *mut core::ffi::c_void),
            DWMWA_CLOAK,
            &value as *const BOOL as *const core::ffi::c_void,
            std::mem::size_of::<BOOL>() as u32,
        );
    }
}

/// Keep the main window off the screen even while it is "shown", until
/// [`allow_show`].
pub fn suppress_show(hwnd: isize) {
    CLOAKED_MAIN.store(hwnd, Ordering::Relaxed);
    set_cloak(hwnd, true);
}

/// Undo [`suppress_show`]: from now on showing the window really shows it.
pub fn allow_show() {
    let hwnd = CLOAKED_MAIN.swap(0, Ordering::Relaxed);
    if hwnd != 0 {
        set_cloak(hwnd, false);
    }
}

/// Returns false if another instance already holds the mutex.
/// The handle is intentionally leaked for the process lifetime.
pub fn acquire_single_instance() -> bool {
    unsafe {
        let _ = CreateMutexW(None, true, w!("Local\\auto-idle-close-single-instance"));
        GetLastError() != ERROR_ALREADY_EXISTS
    }
}
