//! Bench victim: a visible, titled, unowned top-level window — exactly what
//! auto-idle-close tracks. `--ignore-close` makes it swallow WM_CLOSE (an app
//! with an unsaved-changes dialog); `--title <t>` names the window. Shown
//! without activation so it never steals foreground from the bench operator.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    MSG, PostQuitMessage, RegisterClassW, SW_SHOWNOACTIVATE, ShowWindow, TranslateMessage,
    WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};
use windows::core::{PCWSTR, w};

static IGNORE_CLOSE: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CLOSE => {
                if !IGNORE_CLOSE.load(Ordering::Relaxed) {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    IGNORE_CLOSE.store(args.iter().any(|a| a == "--ignore-close"), Ordering::Relaxed);
    let title: Vec<u16> = args
        .iter()
        .position(|a| a == "--title")
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| "aic victim".into())
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let hinstance = GetModuleHandleW(None).expect("GetModuleHandleW");
        let class = w!("aic-victim-window");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            320,
            180,
            None,
            None,
            Some(hinstance.into()),
            None,
        )
        .expect("CreateWindowExW");
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
