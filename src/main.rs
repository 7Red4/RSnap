#![cfg_attr(not(test), windows_subsystem = "windows")]

mod actions;
mod autostart;
mod config;
mod editor;
mod gfx;
mod hotkey;
mod imaging;
mod model;
mod overlay;
mod settings;
mod toast;
mod util;

use std::cell::Cell;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

const WM_TRAY: u32 = WM_APP + 1;
const WM_OPEN_SETTINGS: u32 = WM_APP + 2;
const MENU_CAPTURE: usize = 1;
const MENU_SETTINGS: usize = 2;
const MENU_EXIT: usize = 3;

thread_local! {
    static MAIN: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
}

fn main() {
    let background = std::env::args().any(|a| a == "--background");
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        // The handle stays open for the life of the process; that is what marks it as running.
        let _mutex = CreateMutexW(None, false, w!("Local\\RSnapSingleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if !background && let Ok(running) = FindWindowW(w!("RSnapMain"), None) {
                let mut pid = 0;
                GetWindowThreadProcessId(running, Some(&mut pid));
                let _ = AllowSetForegroundWindow(pid);
                let _ = PostMessageW(Some(running), WM_OPEN_SETTINGS, WPARAM(0), LPARAM(0));
            }
            return;
        }

        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let controls = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_HOTKEY_CLASS | ICC_STANDARD_CLASSES,
        };
        let _ = InitCommonControlsEx(&controls);

        util::register_class(w!("RSnapMain"), Some(main_proc), WNDCLASS_STYLES(0));
        overlay::register();
        toast::register();
        editor::register();
        settings::register();

        TASKBAR_CREATED.with(|t| t.set(RegisterWindowMessageW(w!("TaskbarCreated"))));

        let Ok(main_window) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("RSnapMain"),
            w!("RSnap"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(util::hinstance()),
            None,
        ) else {
            return;
        };
        MAIN.with(|m| m.set(main_window));
        add_tray_icon(main_window);

        let config = config::get();
        autostart::sync(config.autostart);
        let hotkey_ok = hotkey::init(main_window);

        // A taken hotkey would otherwise fail silently, so the settings window opens to explain it.
        if !background || !hotkey_ok {
            settings::open();
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if let Some(dialog) = settings::dialog_handle()
                && IsDialogMessageW(dialog, &msg).as_bool()
            {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        CoUninitialize();
    }
}

pub fn exit_app() {
    unsafe {
        let hwnd = MAIN.with(Cell::get);
        remove_tray_icon(hwnd);
        PostQuitMessage(0);
    }
}

fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    }
}

fn add_tray_icon(hwnd: HWND) {
    let mut data = tray_data(hwnd);
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = WM_TRAY;
    data.hIcon = util::app_icon(unsafe { GetSystemMetrics(SM_CXSMICON) });
    for (slot, unit) in data.szTip.iter_mut().zip("RSnap".encode_utf16()) {
        *slot = unit;
    }
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &data);
    }
}

fn remove_tray_icon(hwnd: HWND) {
    let data = tray_data(hwnd);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
    }
}

fn show_tray_menu(hwnd: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(menu, MF_STRING, MENU_CAPTURE, w!("開始截圖"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_SETTINGS, w!("設定"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, MENU_EXIT, w!("結束"));
        let p = util::cursor_pos();
        // Without this the menu would stay open after clicking elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let choice = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            p.x,
            p.y,
            None,
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);
        match choice.0 as usize {
            MENU_CAPTURE => overlay::start(),
            MENU_SETTINGS => settings::open(),
            MENU_EXIT => exit_app(),
            _ => {}
        }
    }
}

unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == TASKBAR_CREATED.with(Cell::get) && msg != 0 {
            // Explorer restarted and lost the tray, so the icon has to be added again.
            add_tray_icon(hwnd);
            return LRESULT(0);
        }
        match msg {
            WM_HOTKEY if wp.0 as i32 == hotkey::ID => {
                overlay::start();
                LRESULT(0)
            }
            WM_OPEN_SETTINGS => {
                settings::open();
                LRESULT(0)
            }
            WM_TRAY => {
                match (lp.0 & 0xFFFF) as u32 {
                    WM_LBUTTONUP => settings::open(),
                    WM_RBUTTONUP => show_tray_menu(hwnd),
                    _ => {}
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
