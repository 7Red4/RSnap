//! The two toast styles. Only one toast exists at a time.
//! - Card: top right, screenshot preview, a close button, Edit and Save icons on hover,
//!   5 seconds, paused by hover.
//! - Message: top center, text only, optionally tinted with a picked color.

use crate::actions::{self, Session};
use crate::gfx::{self, Align, Icon};
use crate::model::{RectF, Rgb, pt};
use crate::{editor, util};
use std::cell::Cell;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Interface, w};

const CARD_MS: u32 = 5000;
const MESSAGE_MS: u32 = 2500;
const TIMER: usize = 1;
const PANEL: Rgb = Rgb::new(32, 33, 38);
const WHITE: Rgb = Rgb::new(255, 255, 255);

thread_local! {
    static ACTIVE: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
}

enum Kind {
    Card {
        session: Session,
        status: String,
        hover: bool,
        pressed: Option<Button>,
    },
    Message {
        text: String,
        background: Option<Rgb>,
    },
}

#[derive(Clone, Copy, PartialEq)]
enum Button {
    Edit,
    Save,
    Close,
}

struct Toast {
    kind: Kind,
    scale: f32,
    rt: Option<ID2D1HwndRenderTarget>,
    bitmap: Option<ID2D1Bitmap>,
    mouse_at: (f32, f32),
}

pub fn register() {
    util::register_class(w!("RSnapToast"), Some(proc), WNDCLASS_STYLES(0));
}

pub fn close() {
    let hwnd = ACTIVE.with(Cell::get);
    if !hwnd.is_invalid() {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

pub fn show_card(session: Session, status: String) {
    close();
    let p = util::cursor_pos();
    let (_, work, scale) = util::monitor_at(p);
    let (w, h) = card_size(&session, scale);
    let margin = (16.0 * scale) as i32;
    let x = work.right - w - margin;
    let y = work.top + margin;
    let kind = Kind::Card {
        session,
        status,
        hover: false,
        pressed: None,
    };
    create(kind, scale, x, y, w, h, CARD_MS, false);
}

pub fn show_message(text: String, background: Option<Rgb>) {
    close();
    let p = util::cursor_pos();
    let (_, work, scale) = util::monitor_at(p);
    let (tw, _) = gfx::measure(&text, "Microsoft JhengHei UI", 15.0 * scale, 500);
    let w = (tw + 44.0 * scale).ceil() as i32;
    let h = (46.0 * scale) as i32;
    let x = work.left + (work.right - work.left - w) / 2;
    let y = work.top + (24.0 * scale) as i32;
    create(
        Kind::Message { text, background },
        scale,
        x,
        y,
        w,
        h,
        MESSAGE_MS,
        true,
    );
}

const BAR_HEIGHT: f32 = 34.0;

fn card_size(session: &Session, scale: f32) -> (i32, i32) {
    let (preview_w, preview_h) = preview_size(session, scale);
    (preview_w as i32, (preview_h + BAR_HEIGHT * scale) as i32)
}

fn preview_size(session: &Session, scale: f32) -> (f32, f32) {
    let (max_w, max_h) = (340.0 * scale, 220.0 * scale);
    let min_w = 280.0 * scale;
    let min_h = 110.0 * scale;
    let (iw, ih) = (session.composed.w as f32, session.composed.h as f32);
    let fit = (max_w / iw).min(max_h / ih).min(1.0);
    ((iw * fit).max(min_w).ceil(), (ih * fit).max(min_h).ceil())
}

#[allow(clippy::too_many_arguments)]
fn create(kind: Kind, scale: f32, x: i32, y: i32, w: i32, h: i32, ms: u32, click_through: bool) {
    unsafe {
        let mut ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        if click_through {
            ex |= WS_EX_TRANSPARENT | WS_EX_LAYERED;
        }
        let Ok(hwnd) = CreateWindowExW(
            ex,
            w!("RSnapToast"),
            w!("RSnap"),
            WS_POPUP,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(util::hinstance()),
            None,
        ) else {
            return;
        };
        if click_through {
            // Mouse pass-through requires a layered window; fully opaque keeps its look unchanged.
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
        }
        util::round_corners(hwnd);
        util::set_state(
            hwnd,
            Box::new(Toast {
                kind,
                scale,
                rt: None,
                bitmap: None,
                mouse_at: (-1.0, -1.0),
            }),
        );
        ACTIVE.with(|a| a.set(hwnd));
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = UpdateWindow(hwnd);
        SetTimer(Some(hwnd), TIMER, ms, None);
    }
}

impl Toast {
    fn buttons(&self, hwnd: HWND) -> (RectF, RectF) {
        let (cw, ch) = util::client_size(hwnd);
        let s = self.scale;
        let (bw, bh, gap) = (48.0 * s, 40.0 * s, 12.0 * s);
        let preview_h = ch as f32 - BAR_HEIGHT * s;
        let top = (preview_h - bh) / 2.0;
        let left = (cw as f32 - bw * 2.0 - gap) / 2.0;
        (
            RectF::xywh(left, top, bw, bh),
            RectF::xywh(left + bw + gap, top, bw, bh),
        )
    }

    fn close_button(&self, hwnd: HWND) -> RectF {
        let (cw, _) = util::client_size(hwnd);
        let s = self.scale;
        let side = 26.0 * s;
        RectF::xywh(cw as f32 - side - 8.0 * s, 8.0 * s, side, side)
    }

    fn button_at(&self, hwnd: HWND) -> Option<Button> {
        let (edit, save) = self.buttons(hwnd);
        let p = pt(self.mouse_at.0, self.mouse_at.1);
        if self.close_button(hwnd).contains(p) {
            Some(Button::Close)
        } else if edit.contains(p) {
            Some(Button::Edit)
        } else if save.contains(p) {
            Some(Button::Save)
        } else {
            None
        }
    }

    fn render(&mut self, hwnd: HWND) -> windows::core::Result<()> {
        if self.rt.is_none() {
            self.rt = Some(gfx::hwnd_target(hwnd)?);
            self.bitmap = None;
        }
        let Some(target) = self.rt.clone() else {
            return Ok(());
        };
        let rt: ID2D1RenderTarget = target.cast()?;
        if self.bitmap.is_none()
            && let Kind::Card { session, .. } = &self.kind
        {
            self.bitmap = Some(gfx::bitmap(&rt, &session.composed)?);
        }
        let (cw, ch) = util::client_size(hwnd);
        let (w, h) = (cw as f32, ch as f32);
        let s = self.scale;
        unsafe {
            rt.BeginDraw();
            match &self.kind {
                Kind::Message { text, background } => {
                    let bg = background.unwrap_or(PANEL);
                    rt.Clear(Some(&gfx::color(bg)));
                    gfx::label(
                        &rt,
                        text,
                        RectF::new(0.0, 0.0, w, h),
                        15.0 * s,
                        500,
                        bg.contrast(),
                        Align::Center,
                    );
                }
                Kind::Card {
                    session,
                    status,
                    hover,
                    ..
                } => {
                    rt.Clear(Some(&gfx::color(PANEL)));
                    let area_h = h - BAR_HEIGHT * s;
                    let (iw, ih) = (session.composed.w as f32, session.composed.h as f32);
                    let fit = (w / iw).min(area_h / ih).min(1.0);
                    let (dw, dh) = (iw * fit, ih * fit);
                    let dest = RectF::xywh((w - dw) / 2.0, (area_h - dh) / 2.0, dw, dh);
                    if let Some(bitmap) = &self.bitmap {
                        rt.DrawBitmap(
                            bitmap,
                            Some(&gfx::rect(dest)),
                            1.0,
                            D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                            None,
                        );
                    }
                    let bar = RectF::new(12.0 * s, area_h, w - 12.0 * s, h);
                    gfx::label(&rt, status, bar, 13.0 * s, 400, WHITE, Align::Leading);

                    let over = self.button_at(hwnd);
                    if *hover {
                        gfx::fill(
                            &rt,
                            RectF::new(0.0, 0.0, w, area_h),
                            gfx::color_a(Rgb::new(0, 0, 0), 0.42),
                        );
                        let (edit, save) = self.buttons(hwnd);
                        for (r, icon, kind) in [
                            (edit, Icon::Pen, Button::Edit),
                            (save, Icon::Save, Button::Save),
                        ] {
                            let fill = if over == Some(kind) {
                                Rgb::new(255, 255, 255)
                            } else {
                                Rgb::new(226, 226, 232)
                            };
                            gfx::fill_round(&rt, r, 8.0 * s, gfx::color(fill));
                            let glyph = 24.0 * s;
                            let area = RectF::xywh(
                                r.l + (r.w() - glyph) / 2.0,
                                r.t + (r.h() - glyph) / 2.0,
                                glyph,
                                glyph,
                            );
                            gfx::draw_icon(&rt, icon, area, Rgb::new(24, 24, 28));
                        }
                    }

                    let close = self.close_button(hwnd);
                    let alpha = if over == Some(Button::Close) {
                        0.9
                    } else {
                        0.6
                    };
                    gfx::fill_round(
                        &rt,
                        close,
                        close.w() / 2.0,
                        gfx::color_a(Rgb::new(0, 0, 0), alpha),
                    );
                    gfx::draw_icon(&rt, Icon::Close, close.inflate(-5.0 * s), WHITE);
                }
            }
            if rt.EndDraw(None, None).is_err() {
                self.rt = None;
                self.bitmap = None;
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
        Ok(())
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let Some(state) = util::state::<Toast>(hwnd) else {
            return DefWindowProcW(hwnd, msg, wp, lp);
        };
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut ps);
                let _ = state.render(hwnd);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_TIMER if wp.0 == TIMER => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = util::lparam_point(lp);
                state.mouse_at = (x as f32, y as f32);
                if let Kind::Card { hover, .. } = &mut state.kind {
                    if !*hover {
                        *hover = true;
                        // Hovering stops the countdown; leaving starts a full 5 seconds again.
                        let _ = KillTimer(Some(hwnd), TIMER);
                        let mut track = TRACKMOUSEEVENT {
                            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        let _ = TrackMouseEvent(&mut track);
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                state.mouse_at = (-1.0, -1.0);
                if let Kind::Card { hover, pressed, .. } = &mut state.kind {
                    *hover = false;
                    *pressed = None;
                    SetTimer(Some(hwnd), TIMER, CARD_MS, None);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = util::lparam_point(lp);
                state.mouse_at = (x as f32, y as f32);
                let over = state.button_at(hwnd);
                if let Kind::Card { pressed, .. } = &mut state.kind {
                    *pressed = over;
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = util::lparam_point(lp);
                state.mouse_at = (x as f32, y as f32);
                let over = state.button_at(hwnd);
                let Kind::Card {
                    session,
                    status,
                    pressed,
                    ..
                } = &mut state.kind
                else {
                    return LRESULT(0);
                };
                let clicked = pressed.take().filter(|p| Some(*p) == over);
                match clicked {
                    Some(Button::Edit) => {
                        let session = session.clone();
                        let _ = DestroyWindow(hwnd);
                        editor::open(session);
                    }
                    Some(Button::Save) => {
                        let image = session.composed.clone();
                        if let Some(path) = actions::save_with_recovery(&image, None) {
                            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                            *status = format!("已儲存 {}", name.unwrap_or_default());
                            let _ = InvalidateRect(Some(hwnd), None, false);
                        }
                    }
                    Some(Button::Close) => {
                        let _ = DestroyWindow(hwnd);
                    }
                    None => {}
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                if ACTIVE.with(Cell::get) == hwnd {
                    ACTIVE.with(|a| a.set(HWND(std::ptr::null_mut())));
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_NCDESTROY => {
                util::take_state::<Toast>(hwnd);
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
