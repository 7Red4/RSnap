//! Capture mode: a frozen copy of the desktop with selection, magnifier and color picking.

use crate::actions::{self, Session};
use crate::gfx::{self, Align};
use crate::imaging::{self, Image};
use crate::model::{RectF, Rgb};
use crate::{toast, util};
use std::cell::Cell;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Interface, w};

const MIN_SELECTION: i32 = 3;
const MAGNIFIER_CELLS: i32 = 15;
const RED: Rgb = Rgb::new(245, 59, 53);

thread_local! {
    static ACTIVE: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
}

struct Overlay {
    image: Image,
    origin: POINT,
    rt: Option<ID2D1HwndRenderTarget>,
    bitmap: Option<ID2D1Bitmap>,
    anchor: Option<(i32, i32)>,
    cursor: (i32, i32),
    red_cross: HCURSOR,
}

pub fn register() {
    util::register_class(w!("RSnapOverlay"), Some(proc), WNDCLASS_STYLES(0));
}

pub fn is_active() -> bool {
    !ACTIVE.with(Cell::get).is_invalid()
}

pub fn start() {
    if is_active() {
        return;
    }
    toast::close();
    let Ok((image, bounds)) = imaging::capture_virtual_screen() else {
        toast::show_message("無法擷取畫面".to_string(), None);
        return;
    };
    unsafe {
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            w!("RSnapOverlay"),
            w!("RSnap"),
            WS_POPUP,
            bounds.left,
            bounds.top,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            None,
            None,
            Some(util::hinstance()),
            None,
        ) else {
            toast::show_message("無法進入擷取模式".to_string(), None);
            return;
        };
        let p = util::cursor_pos();
        let (_, _, scale) = util::monitor_at(p);
        let origin = POINT {
            x: bounds.left,
            y: bounds.top,
        };
        util::set_state(
            hwnd,
            Box::new(Overlay {
                image,
                origin,
                rt: None,
                bitmap: None,
                anchor: None,
                cursor: (p.x - origin.x, p.y - origin.y),
                red_cross: make_red_cross(scale),
            }),
        );
        ACTIVE.with(|a| a.set(hwnd));

        util::set_cloaked(hwnd, true);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
        util::set_cloaked(hwnd, false);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
    }
}

/// A red cross with a thin white edge so it stays visible on a red background.
fn make_red_cross(scale: f32) -> HCURSOR {
    let size = (32.0 * scale).round() as i32;
    let center = size / 2;
    let arm = (12.0 * scale).round() as i32;
    let gap = (2.0 * scale).round() as i32;
    let half = ((scale).round() as i32).max(1);
    let mut pixels = vec![0u32; (size * size) as usize];
    let mut paint = |x: i32, y: i32, value: u32| {
        if x >= 0 && y >= 0 && x < size && y < size {
            pixels[(y * size + x) as usize] = value;
        }
    };
    for (extra, value) in [(1, 0xFFFF_FFFFu32), (0, 0xFFF5_3B35u32)] {
        for d in gap - extra..=arm + extra {
            for w in -(half - 1) - extra..=(half - 1) + extra {
                paint(center + d, center + w, value);
                paint(center - d, center + w, value);
                paint(center + w, center + d, value);
                paint(center + w, center - d, value);
            }
        }
    }
    util::cursor_from_pixels(&pixels, size, (center as u32, center as u32))
        .unwrap_or_else(|| unsafe { LoadCursorW(None, IDC_CROSS).unwrap_or_default() })
}

impl Overlay {
    fn selection(&self) -> Option<(i32, i32, i32, i32)> {
        let (ax, ay) = self.anchor?;
        let (cx, cy) = self.cursor;
        let (w, h) = (self.image.w as i32, self.image.h as i32);
        let (l, r) = (ax.min(cx).clamp(0, w), (ax.max(cx) + 1).clamp(0, w));
        let (t, b) = (ay.min(cy).clamp(0, h), (ay.max(cy) + 1).clamp(0, h));
        Some((l, t, r, b))
    }

    fn paint(&mut self, hwnd: HWND) {
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            BeginPaint(hwnd, &mut ps);
            let _ = self.render(hwnd);
            let _ = EndPaint(hwnd, &ps);
        }
    }

    fn render(&mut self, hwnd: HWND) -> windows::core::Result<()> {
        if self.rt.is_none() {
            let target = gfx::hwnd_target(hwnd)?;
            let base: ID2D1RenderTarget = target.cast()?;
            self.bitmap = Some(gfx::bitmap(&base, &self.image)?);
            self.rt = Some(target);
        }
        let (Some(target), Some(bitmap)) = (self.rt.clone(), self.bitmap.clone()) else {
            return Ok(());
        };
        let rt: ID2D1RenderTarget = target.cast()?;
        unsafe {
            rt.BeginDraw();
            rt.DrawBitmap(
                &bitmap,
                None,
                1.0,
                D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                None,
            );
            self.draw_selection(&rt);
            self.draw_magnifier(&rt, &bitmap);
            if rt.EndDraw(None, None).is_err() {
                // The device was lost, for example by a driver reset. Rebuild on the next paint.
                self.rt = None;
                self.bitmap = None;
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
        Ok(())
    }

    fn scale(&self) -> f32 {
        util::monitor_at(self.screen_cursor()).2
    }

    fn screen_cursor(&self) -> POINT {
        POINT {
            x: self.cursor.0 + self.origin.x,
            y: self.cursor.1 + self.origin.y,
        }
    }

    fn draw_selection(&self, rt: &ID2D1RenderTarget) {
        let Some((l, t, r, b)) = self.selection() else {
            return;
        };
        let (w, h) = (self.image.w as f32, self.image.h as f32);
        let (lf, tf, rf, bf) = (l as f32, t as f32, r as f32, b as f32);
        let dim = gfx::color_a(Rgb::new(0, 0, 0), 0.45);
        for area in [
            RectF::new(0.0, 0.0, w, tf),
            RectF::new(0.0, bf, w, h),
            RectF::new(0.0, tf, lf, bf),
            RectF::new(rf, tf, w, bf),
        ] {
            gfx::fill(rt, area, dim);
        }

        let s = self.scale();
        if let Ok(brush) = gfx::brush(rt, gfx::color(RED)) {
            let border = RectF::new(lf - 0.5, tf - 0.5, rf + 0.5, bf + 0.5);
            unsafe { rt.DrawRectangle(&gfx::rect(border), &brush, (1.5 * s).max(1.0), None) };
        }

        let text = format!("{} × {}", r - l, b - t);
        let (tw, th) = gfx::measure(&text, "Microsoft JhengHei UI", 13.0 * s, 500);
        let (bw, bh) = (tw + 14.0 * s, th + 6.0 * s);
        let above = tf - bh - 6.0 * s;
        let y = if above >= 0.0 { above } else { tf + 6.0 * s };
        let tag = RectF::xywh(lf.min(w - bw).max(0.0), y, bw, bh);
        gfx::fill_round(rt, tag, 4.0 * s, gfx::color_a(Rgb::new(24, 24, 28), 0.92));
        gfx::label(
            rt,
            &text,
            tag,
            13.0 * s,
            500,
            Rgb::new(255, 255, 255),
            Align::Center,
        );
    }

    fn draw_magnifier(&self, rt: &ID2D1RenderTarget, bitmap: &ID2D1Bitmap) {
        let s = self.scale();
        let (mx, my) = self.cursor;
        let cell = (10.0 * s).round();
        let side = cell * MAGNIFIER_CELLS as f32;
        let strip = 30.0 * s;
        let gap = 22.0 * s;
        let (monitor, _, _) = util::monitor_at(self.screen_cursor());
        let left = (monitor.left - self.origin.x) as f32;
        let top = (monitor.top - self.origin.y) as f32;
        let right = (monitor.right - self.origin.x) as f32;
        let bottom = (monitor.bottom - self.origin.y) as f32;

        // Sits to the lower right of the cursor and flips when it would leave the monitor.
        let mut x = mx as f32 + gap;
        let mut y = my as f32 + gap;
        if x + side > right {
            x = mx as f32 - gap - side;
        }
        if y + side + strip > bottom {
            y = my as f32 - gap - side - strip;
        }
        x = x.max(left);
        y = y.max(top);

        let panel = RectF::xywh(x, y, side, side + strip);
        gfx::fill(rt, panel, gfx::color(Rgb::new(18, 18, 22)));

        // Only the on-image part of the 15x15 cell window is drawn, so edges show as dark cells.
        let half = MAGNIFIER_CELLS / 2;
        let (x0, y0) = (mx - half, my - half);
        let sl = x0.max(0);
        let st = y0.max(0);
        let sr = (x0 + MAGNIFIER_CELLS).min(self.image.w as i32);
        let sb = (y0 + MAGNIFIER_CELLS).min(self.image.h as i32);
        if sr > sl && sb > st {
            let dest = RectF::new(
                x + (sl - x0) as f32 * cell,
                y + (st - y0) as f32 * cell,
                x + (sr - x0) as f32 * cell,
                y + (sb - y0) as f32 * cell,
            );
            let source = RectF::new(sl as f32, st as f32, sr as f32, sb as f32);
            unsafe {
                rt.DrawBitmap(
                    bitmap,
                    Some(&gfx::rect(dest)),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                    Some(&gfx::rect(source)),
                );
            }
        }

        let center = RectF::xywh(x + half as f32 * cell, y + half as f32 * cell, cell, cell);
        if let Ok(brush) = gfx::brush(rt, gfx::color(RED)) {
            unsafe { rt.DrawRectangle(&gfx::rect(center), &brush, (1.5 * s).max(1.0), None) };
        }
        if let Ok(brush) = gfx::brush(rt, gfx::color(Rgb::new(255, 255, 255))) {
            unsafe { rt.DrawRectangle(&gfx::rect(panel), &brush, 1.0, None) };
        }

        let color = self.image.pixel(mx, my);
        let chip = 16.0 * s;
        let chip_rect = RectF::xywh(x + 8.0 * s, y + side + (strip - chip) / 2.0, chip, chip);
        gfx::fill(rt, chip_rect, gfx::color(color));
        if let Ok(brush) = gfx::brush(rt, gfx::color(Rgb::new(255, 255, 255))) {
            unsafe { rt.DrawRectangle(&gfx::rect(chip_rect), &brush, 1.0, None) };
        }
        let text_area = RectF::new(chip_rect.r + 8.0 * s, y + side, x + side, y + side + strip);
        gfx::label_with_font(
            rt,
            &color.hex(),
            "Consolas",
            text_area,
            15.0 * s,
            600,
            Rgb::new(255, 255, 255),
            Align::Leading,
        );
    }
}

/// Destroys the overlay window and frees its state.
fn close(hwnd: HWND) {
    unsafe {
        let _ = ReleaseCapture();
        let _ = DestroyWindow(hwnd);
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let Some(state) = util::state::<Overlay>(hwnd) else {
            return DefWindowProcW(hwnd, msg, wp, lp);
        };
        match msg {
            WM_PAINT => {
                state.paint(hwnd);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_SETCURSOR => {
                SetCursor(Some(state.red_cross));
                LRESULT(1)
            }
            WM_MOUSEMOVE => {
                state.cursor = util::lparam_point(lp);
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let p = util::lparam_point(lp);
                state.cursor = p;
                state.anchor = Some(p);
                SetCapture(hwnd);
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                state.cursor = util::lparam_point(lp);
                let selection = state.selection();
                state.anchor = None;
                let _ = ReleaseCapture();
                let _ = InvalidateRect(Some(hwnd), None, false);
                if let Some((l, t, r, b)) = selection
                    && r - l >= MIN_SELECTION
                    && b - t >= MIN_SELECTION
                {
                    let image =
                        state
                            .image
                            .crop(l as u32, t as u32, (r - l) as u32, (b - t) as u32);
                    close(hwnd);
                    actions::copy_and_notify(Session::new(image));
                }
                LRESULT(0)
            }
            // These act on release. Closing on press would send the release to the window
            // underneath, and a right-button release there opens its context menu.
            WM_MBUTTONDOWN | WM_RBUTTONDOWN => LRESULT(0),
            WM_MBUTTONUP => {
                let (x, y) = util::lparam_point(lp);
                let color = state.image.pixel(x, y);
                close(hwnd);
                match imaging::copy_text(&color.hex()) {
                    Ok(()) => {
                        toast::show_message(format!("已複製顏色 {}", color.hex()), Some(color))
                    }
                    Err(_) => toast::show_message("無法寫入剪貼簿".to_string(), None),
                }
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                close(hwnd);
                LRESULT(0)
            }
            WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => {
                close(hwnd);
                LRESULT(0)
            }
            // The desktop was captured at start; changing DPI must not resize this window.
            WM_DPICHANGED => LRESULT(0),
            WM_NCDESTROY => {
                let _ = DestroyCursor(state.red_cross);
                ACTIVE.with(|a| a.set(HWND(std::ptr::null_mut())));
                util::take_state::<Overlay>(hwnd);
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
