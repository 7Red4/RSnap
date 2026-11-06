//! The settings window, built from native controls. Changes apply immediately.

use crate::config::{self, Hotkey, ImageFormat};
use crate::{actions, autostart, hotkey, util};
use std::cell::Cell;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

const ID_BROWSE: i32 = 101;
const ID_FORMAT: i32 = 102;
const ID_AUTOSTART: i32 = 103;
const ID_APPLY_HOTKEY: i32 = 104;
const ID_CLOSE: i32 = 105;
const ID_EXIT: i32 = 106;

// The hotkey control's flag values differ from the ones RegisterHotKey uses.
const HOTKEYF_SHIFT: u32 = 1;
const HOTKEYF_CONTROL: u32 = 2;
const HOTKEYF_ALT: u32 = 4;

const WIDTH: f32 = 500.0;
const HEIGHT: f32 = 330.0;

thread_local! {
    static ACTIVE: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
}

struct Settings {
    font: HFONT,
    background: HBRUSH,
    path: HWND,
    format: HWND,
    autostart: HWND,
    hotkey: HWND,
    status: HWND,
}

pub fn register() {
    util::register_class(w!("RSnapSettings"), Some(proc), WNDCLASS_STYLES(0));
}

/// The open settings window, so the message loop can route Tab navigation to it.
pub fn dialog_handle() -> Option<HWND> {
    let hwnd = ACTIVE.with(Cell::get);
    (!hwnd.is_invalid()).then_some(hwnd)
}

pub fn open() {
    if let Some(hwnd) = dialog_handle() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
        return;
    }
    unsafe {
        let (_, work, scale) = util::monitor_at(util::cursor_pos());
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: (WIDTH * scale) as i32,
            bottom: (HEIGHT * scale) as i32,
        };
        let _ = windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi(
            &mut frame,
            style,
            false,
            WINDOW_EX_STYLE(0),
            (scale * 96.0) as u32,
        );
        let (w, h) = (frame.right - frame.left, frame.bottom - frame.top);
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("RSnapSettings"),
            w!("RSnap 設定"),
            style,
            work.left + (work.right - work.left - w) / 2,
            work.top + (work.bottom - work.top - h) / 2,
            w,
            h,
            None,
            None,
            Some(util::hinstance()),
            None,
        ) else {
            return;
        };
        ACTIVE.with(|a| a.set(hwnd));
        let state = build(hwnd, scale);
        util::set_state(hwnd, Box::new(state));
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

#[allow(clippy::too_many_arguments)]
fn control(
    parent: HWND,
    class: PCWSTR,
    text: &str,
    style: u32,
    id: i32,
    rect: (f32, f32, f32, f32),
    scale: f32,
    font: HFONT,
) -> HWND {
    unsafe {
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            &HSTRING::from(text),
            WINDOW_STYLE(style) | WS_CHILD | WS_VISIBLE,
            (rect.0 * scale) as i32,
            (rect.1 * scale) as i32,
            (rect.2 * scale) as i32,
            (rect.3 * scale) as i32,
            Some(parent),
            Some(HMENU(id as isize as *mut _)),
            Some(util::hinstance()),
            None,
        )
        .unwrap_or_default();
        util::set_font(hwnd, font);
        hwnd
    }
}

fn format_label(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "PNG",
        ImageFormat::Jpg => "JPG",
        ImageFormat::Webp => "WebP",
    }
}

fn build(hwnd: HWND, s: f32) -> Settings {
    let cfg = config::get();
    let font = util::ui_font(s);
    let tab = WS_TABSTOP.0;
    let button = w!("BUTTON");
    let edit = w!("EDIT");
    let label = w!("STATIC");
    let combo = w!("COMBOBOX");

    control(
        hwnd,
        label,
        "儲存路徑",
        0,
        0,
        (20.0, 22.0, 90.0, 22.0),
        s,
        font,
    );
    let path = control(
        hwnd,
        edit,
        &actions::save_dir().to_string_lossy(),
        WS_BORDER.0 | ES_AUTOHSCROLL as u32 | ES_READONLY as u32 | tab,
        0,
        (115.0, 20.0, 275.0, 26.0),
        s,
        font,
    );
    control(
        hwnd,
        button,
        "瀏覽…",
        tab,
        ID_BROWSE,
        (400.0, 19.0, 80.0, 28.0),
        s,
        font,
    );

    control(
        hwnd,
        label,
        "儲存格式",
        0,
        0,
        (20.0, 68.0, 90.0, 22.0),
        s,
        font,
    );
    let format = control(
        hwnd,
        combo,
        "",
        CBS_DROPDOWNLIST as u32 | WS_VSCROLL.0 | tab,
        ID_FORMAT,
        (115.0, 65.0, 120.0, 200.0),
        s,
        font,
    );
    unsafe {
        for (i, f) in ImageFormat::ALL.iter().enumerate() {
            let text = HSTRING::from(format_label(*f));
            SendMessageW(
                format,
                CB_ADDSTRING,
                None,
                Some(LPARAM(text.as_ptr() as isize)),
            );
            if *f == cfg.format {
                SendMessageW(format, CB_SETCURSEL, Some(WPARAM(i)), None);
            }
        }
    }

    control(
        hwnd,
        label,
        "截圖快速鍵",
        0,
        0,
        (20.0, 112.0, 90.0, 22.0),
        s,
        font,
    );
    let hotkey_box = control(
        hwnd,
        w!("msctls_hotkey32"),
        "",
        tab,
        0,
        (115.0, 110.0, 165.0, 26.0),
        s,
        font,
    );
    set_hotkey_control(hotkey_box, cfg.hotkey);
    control(
        hwnd,
        button,
        "套用",
        tab,
        ID_APPLY_HOTKEY,
        (290.0, 109.0, 80.0, 28.0),
        s,
        font,
    );
    let status = control(hwnd, label, "", 0, 0, (115.0, 142.0, 365.0, 22.0), s, font);
    set_text(status, &hotkey::status_text());

    let autostart_box = control(
        hwnd,
        button,
        "開機時自動啟動",
        BS_AUTOCHECKBOX as u32 | tab,
        ID_AUTOSTART,
        (115.0, 178.0, 300.0, 26.0),
        s,
        font,
    );
    unsafe {
        SendMessageW(
            autostart_box,
            BM_SETCHECK,
            Some(WPARAM(if cfg.autostart {
                BST_CHECKED.0 as usize
            } else {
                BST_UNCHECKED.0 as usize
            })),
            None,
        );
    }

    control(
        hwnd,
        label,
        "關閉這個視窗後，RSnap 會繼續在系統匣待命。",
        0,
        0,
        (20.0, 222.0, 460.0, 22.0),
        s,
        font,
    );
    control(
        hwnd,
        button,
        "結束 RSnap",
        tab,
        ID_EXIT,
        (20.0, 278.0, 110.0, 32.0),
        s,
        font,
    );
    control(
        hwnd,
        button,
        "關閉視窗",
        BS_DEFPUSHBUTTON as u32 | tab,
        ID_CLOSE,
        (370.0, 278.0, 110.0, 32.0),
        s,
        font,
    );

    Settings {
        font,
        background: unsafe { GetSysColorBrush(COLOR_WINDOW) },
        path,
        format,
        autostart: autostart_box,
        hotkey: hotkey_box,
        status,
    }
}

fn set_text(hwnd: HWND, text: &str) {
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowTextW(hwnd, &HSTRING::from(text));
    }
}

fn set_hotkey_control(hwnd: HWND, key: Hotkey) {
    let mut flags = 0;
    if key.modifiers & 2 != 0 {
        flags |= HOTKEYF_CONTROL;
    }
    if key.modifiers & 4 != 0 {
        flags |= HOTKEYF_SHIFT;
    }
    if key.modifiers & 1 != 0 {
        flags |= HOTKEYF_ALT;
    }
    unsafe {
        SendMessageW(
            hwnd,
            HKM_SETHOTKEY,
            Some(WPARAM(((flags << 8) | key.vk) as usize)),
            None,
        );
    }
}

fn read_hotkey_control(hwnd: HWND) -> Option<Hotkey> {
    let value = unsafe { SendMessageW(hwnd, HKM_GETHOTKEY, None, None).0 } as u32;
    let vk = value & 0xFF;
    if vk == 0 {
        return None;
    }
    let flags = (value >> 8) & 0xFF;
    let mut modifiers = 0;
    if flags & HOTKEYF_CONTROL != 0 {
        modifiers |= 2;
    }
    if flags & HOTKEYF_SHIFT != 0 {
        modifiers |= 4;
    }
    if flags & HOTKEYF_ALT != 0 {
        modifiers |= 1;
    }
    Some(Hotkey { modifiers, vk })
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let Some(state) = util::state::<Settings>(hwnd) else {
            return DefWindowProcW(hwnd, msg, wp, lp);
        };
        match msg {
            WM_CTLCOLORSTATIC => {
                let dc = HDC(wp.0 as *mut _);
                SetBkMode(dc, TRANSPARENT);
                LRESULT(state.background.0 as isize)
            }
            WM_ERASEBKGND => {
                let dc = HDC(wp.0 as *mut _);
                let mut r = RECT::default();
                let _ = GetClientRect(hwnd, &mut r);
                FillRect(dc, &r, state.background);
                LRESULT(1)
            }
            WM_COMMAND => {
                let id = (wp.0 & 0xFFFF) as i32;
                let code = ((wp.0 >> 16) & 0xFFFF) as u32;
                match id {
                    ID_BROWSE => {
                        if let Some(dir) =
                            actions::pick_folder(Some(hwnd), Some(&actions::save_dir()))
                        {
                            set_text(state.path, &dir.to_string_lossy());
                            config::update(|c| c.save_dir = Some(dir));
                        }
                    }
                    ID_FORMAT if code == CBN_SELCHANGE => {
                        let index = SendMessageW(state.format, CB_GETCURSEL, None, None).0;
                        if let Some(format) = ImageFormat::ALL.get(index as usize) {
                            config::update(|c| c.format = *format);
                        }
                    }
                    ID_AUTOSTART => {
                        let checked = SendMessageW(state.autostart, BM_GETCHECK, None, None).0
                            == BST_CHECKED.0 as isize;
                        config::update(|c| c.autostart = checked);
                        autostart::sync(checked);
                    }
                    ID_APPLY_HOTKEY => {
                        match read_hotkey_control(state.hotkey) {
                            Some(key) => {
                                hotkey::apply(key);
                                // A rejected key stays unsaved, so the control returns to the live one.
                                set_hotkey_control(state.hotkey, config::get().hotkey);
                            }
                            None => set_hotkey_control(state.hotkey, config::get().hotkey),
                        }
                        set_text(state.status, &hotkey::status_text());
                    }
                    ID_CLOSE => {
                        let _ = DestroyWindow(hwnd);
                    }
                    ID_EXIT => crate::exit_app(),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                ACTIVE.with(|a| a.set(HWND(std::ptr::null_mut())));
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_NCDESTROY => {
                let _ = DeleteObject(state.font.into());
                util::take_state::<Settings>(hwnd);
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
