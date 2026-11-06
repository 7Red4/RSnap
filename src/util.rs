//! Small Win32 helpers shared by every window.

use crate::model::Rgb;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, PCWSTR, w};

pub fn hinstance() -> HINSTANCE {
    unsafe { GetModuleHandleW(None).unwrap_or_default().into() }
}

pub fn register_class(name: PCWSTR, proc: WNDPROC, style: WNDCLASS_STYLES) {
    unsafe {
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style,
            lpfnWndProc: proc,
            hInstance: hinstance(),
            lpszClassName: name,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: app_icon(32),
            hIconSm: app_icon(16),
            ..Default::default()
        };
        RegisterClassExW(&class);
    }
}

pub fn app_icon(size: i32) -> HICON {
    unsafe {
        LoadImageW(
            Some(hinstance()),
            PCWSTR(1 as _),
            IMAGE_ICON,
            size,
            size,
            LR_DEFAULTCOLOR,
        )
        .map(|h| HICON(h.0))
        .unwrap_or_default()
    }
}

/// Stores per-window state. Ownership returns through `take_state` on WM_NCDESTROY.
pub fn set_state<T>(hwnd: HWND, state: Box<T>) {
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
    }
}

/// A window's state, or None before `set_state` has run.
///
/// Win32 re-enters window procedures on the same thread, so a RefCell would abort on valid
/// nesting. Callers must not touch the reference again after destroying the window.
pub fn state<'a, T>(hwnd: HWND) -> Option<&'a mut T> {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut T;
        ptr.as_mut()
    }
}

pub fn take_state<T>(hwnd: HWND) {
    unsafe {
        let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut T;
        if !ptr.is_null() {
            drop(Box::from_raw(ptr));
        }
    }
}

pub fn lparam_point(lp: LPARAM) -> (i32, i32) {
    (
        (lp.0 & 0xFFFF) as i16 as i32,
        ((lp.0 >> 16) & 0xFFFF) as i16 as i32,
    )
}

pub fn cursor_pos() -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    p
}

/// The monitor under a screen point: its bounds, work area and scale (1.0 = 100%).
pub fn monitor_at(p: POINT) -> (RECT, RECT, f32) {
    unsafe {
        let monitor = MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(monitor, &mut info);
        let (mut dpi_x, mut dpi_y) = (96, 96);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        (info.rcMonitor, info.rcWork, dpi_x as f32 / 96.0)
    }
}

pub fn window_scale(hwnd: HWND) -> f32 {
    unsafe { GetDpiForWindow(hwnd) as f32 / 96.0 }
}

pub fn client_size(hwnd: HWND) -> (i32, i32) {
    let mut r = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut r);
    }
    (r.right - r.left, r.bottom - r.top)
}

/// Builds a cursor from `size`x`size` top-down straight-alpha BGRA pixels.
pub fn cursor_from_pixels(pixels: &[u32], size: i32, hotspot: (u32, u32)) -> Option<HCURSOR> {
    unsafe {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let screen = GetDC(None);
        let mut bits = std::ptr::null_mut();
        let color = CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0);
        ReleaseDC(None, screen);
        let color = color.ok()?;
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u32>(), pixels.len());
        let mask = CreateBitmap(size, size, 1, 1, None);
        let icon = ICONINFO {
            fIcon: FALSE,
            xHotspot: hotspot.0,
            yHotspot: hotspot.1,
            hbmMask: mask,
            hbmColor: color,
        };
        let cursor = CreateIconIndirect(&icon);
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        cursor.ok().map(|c| HCURSOR(c.0))
    }
}

pub fn colorref(c: Rgb) -> COLORREF {
    COLORREF(c.r as u32 | (c.g as u32) << 8 | (c.b as u32) << 16)
}

pub fn rgb_from_colorref(c: COLORREF) -> Rgb {
    Rgb::new(
        (c.0 & 0xFF) as u8,
        ((c.0 >> 8) & 0xFF) as u8,
        ((c.0 >> 16) & 0xFF) as u8,
    )
}

pub fn ui_font(scale: f32) -> HFONT {
    gdi_font(
        w!("Microsoft JhengHei UI"),
        (14.0 * scale).round() as i32,
        400,
    )
}

pub fn gdi_font(face: PCWSTR, height: i32, weight: i32) -> HFONT {
    unsafe {
        CreateFontW(
            -height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            face,
        )
    }
}

pub fn set_font(hwnd: HWND, font: HFONT) {
    unsafe {
        SendMessageW(
            hwnd,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );
    }
}

/// Windows 11 rounds and shadows the corners of a plain popup when asked.
pub fn round_corners(hwnd: HWND) {
    unsafe {
        let preference = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const preference).cast(),
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
    }
}

/// While cloaked a window can render its first frame unseen, which avoids a blank flash.
pub fn set_cloaked(hwnd: HWND, cloaked: bool) {
    unsafe {
        let value = BOOL::from(cloaked);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CLOAK,
            (&raw const value).cast(),
            size_of::<BOOL>() as u32,
        );
    }
}
