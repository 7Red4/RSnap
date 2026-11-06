//! The annotation editor. Toolbar and floating panel are drawn with Direct2D; font pickers and
//! text entry are native controls because they need system font lists and IME input.

use crate::actions::{self, Session};
use crate::config;
use crate::gfx::{self, Align, Icon};
use crate::imaging::Image;
use crate::model::*;
use crate::{toast, util};
use std::cell::RefCell;
use std::rc::Rc;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::Dialogs::*;
use windows::Win32::UI::Controls::{EM_SETMARGINS, EM_SETSEL};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, Interface, PCWSTR, w};

const TOOLBAR_H: f32 = 90.0;
const PANEL_H: f32 = 46.0;
const MIN_THICKNESS: f32 = 1.0;
const MAX_THICKNESS: f32 = 40.0;
const HIT_TOLERANCE: f32 = 6.0;
const HANDLE_SIZE: f32 = 9.0;

const ID_FONT: i32 = 201;
const ID_WEIGHT: i32 = 202;
const ID_SIZE: i32 = 203;
const ID_TEXT: i32 = 210;
const WM_COMMIT_TEXT: u32 = WM_APP + 20;
const MK_SHIFT: usize = 0x4;

const WEIGHTS: [(u32, &str); 5] = [
    (300, "細"),
    (400, "標準"),
    (500, "中"),
    (700, "粗"),
    (900, "特粗"),
];
const SIZES: [u32; 15] = [12, 14, 16, 18, 20, 24, 28, 32, 36, 42, 48, 56, 64, 72, 96];

const BG_CANVAS: Rgb = Rgb::new(44, 45, 50);
const BG_BAR: Rgb = Rgb::new(30, 31, 36);
const BTN: Rgb = Rgb::new(58, 60, 67);
const BTN_HOVER: Rgb = Rgb::new(74, 76, 85);
const ACCENT: Rgb = Rgb::new(37, 99, 235);
const ACCENT_HOVER: Rgb = Rgb::new(59, 118, 245);
const WHITE: Rgb = Rgb::new(255, 255, 255);
const MUTED: Rgb = Rgb::new(170, 172, 180);
const DISABLED: Rgb = Rgb::new(120, 122, 130);

thread_local! {
    static CUSTOM_COLORS: RefCell<[COLORREF; 16]> = const { RefCell::new([COLORREF(0x00FF_FFFF); 16]) };
}

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Select,
    Pen,
    Rect,
    Circle,
    Ellipse,
    Text,
}

/// Whether a control changes the default for new objects or the selected object.
#[derive(Clone, Copy, PartialEq)]
enum Scope {
    Default,
    Selected,
}

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Tool(Tool),
    Swatch(Scope, Rgb),
    Custom(Scope),
    Eye(Scope),
    Thick(Scope, i32),
    Fill(Scope),
    Undo,
    Redo,
    Copy,
    Save,
}

enum Look {
    /// Icon only; the name appears as a tooltip on hover.
    Icon(Icon, &'static str),
    /// Icon and text, used for the two actions that finish editing.
    Primary(Icon, &'static str),
    Swatch(Rgb),
    Value(String),
}

struct Btn {
    r: RectF,
    action: Option<Action>,
    look: Look,
    active: bool,
    enabled: bool,
}

enum Drag {
    None,
    Draw {
        anchor: Pt,
        before: Vec<Object>,
    },
    Move {
        from: Pt,
        original: Object,
        before: Vec<Object>,
    },
    Resize {
        handle: usize,
        original: Object,
        bounds: RectF,
        before: Vec<Object>,
    },
}

struct TextEdit {
    hwnd: HWND,
    index: Option<usize>,
    origin: Pt,
    font_name: String,
    weight: u32,
    size: f32,
    color: Rgb,
    gdi_font: HFONT,
    brush: HBRUSH,
    before: Vec<Object>,
}

struct Editor {
    hwnd: HWND,
    rt: Option<ID2D1HwndRenderTarget>,
    bitmap: Option<ID2D1Bitmap>,
    base: Rc<Image>,
    objects: Vec<Object>,
    history: History,
    tool: Tool,
    selected: Option<usize>,
    drag: Drag,
    buttons: Vec<Btn>,
    panel: Option<RectF>,
    hover: Option<Action>,
    scale: f32,
    eyedrop: Option<(Scope, Image)>,
    edit: Option<TextEdit>,
    fonts: Vec<String>,
    font_combo: HWND,
    weight_combo: HWND,
    size_combo: HWND,
    control_font: HFONT,
    pen_cursor: HCURSOR,
}

pub fn register() {
    util::register_class(w!("RSnapEditor"), Some(proc), CS_DBLCLKS);
}

/// A pencil whose hotspot sits on the tip, so strokes start exactly where it points.
fn make_pen_cursor(scale: f32) -> HCURSOR {
    let size = (32.0 * scale).round() as u32;
    let tip = (size * 4 / 24, size * 20 / 24);
    gfx::outlined_icon_pixels(Icon::Pen, size)
        .ok()
        .and_then(|pixels| util::cursor_from_pixels(&pixels, size as i32, tip))
        .unwrap_or_else(|| unsafe { LoadCursorW(None, IDC_CROSS).unwrap_or_default() })
}

pub fn open(session: Session) {
    unsafe {
        let (_, work, s) = util::monitor_at(util::cursor_pos());
        let (work_w, work_h) = (work.right - work.left, work.bottom - work.top);
        let margin = (48.0 * s) as i32;
        let extra_h = (TOOLBAR_H * s) as i32 + margin + (40.0 * s) as i32;
        let w = (session.base.w as i32 + margin)
            .max((820.0 * s) as i32)
            .min((work_w as f32 * 0.94) as i32);
        let h = (session.base.h as i32 + extra_h)
            .max((520.0 * s) as i32)
            .min((work_h as f32 * 0.94) as i32);
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("RSnapEditor"),
            w!("RSnap 編輯"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            work.left + (work_w - w) / 2,
            work.top + (work_h - h) / 2,
            w,
            h,
            None,
            None,
            Some(util::hinstance()),
            None,
        ) else {
            toast::show_message("無法開啟編輯視窗".to_string(), None);
            return;
        };

        let scale = util::window_scale(hwnd);
        let control_font = util::ui_font(scale);
        let combo = |id: i32, width: f32| {
            let c = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("COMBOBOX"),
                None,
                WINDOW_STYLE(CBS_DROPDOWNLIST as u32) | WS_CHILD | WS_VSCROLL | WS_TABSTOP,
                0,
                0,
                (width * scale) as i32,
                (340.0 * scale) as i32,
                Some(hwnd),
                Some(HMENU(id as isize as *mut _)),
                Some(util::hinstance()),
                None,
            )
            .unwrap_or_default();
            util::set_font(c, control_font);
            c
        };
        let font_combo = combo(ID_FONT, 180.0);
        let weight_combo = combo(ID_WEIGHT, 76.0);
        let size_combo = combo(ID_SIZE, 66.0);

        let fonts = gfx::font_families();
        for name in &fonts {
            add_item(font_combo, name);
        }
        for (_, name) in WEIGHTS {
            add_item(weight_combo, name);
        }
        for size in SIZES {
            add_item(size_combo, &size.to_string());
        }

        let mut editor = Box::new(Editor {
            hwnd,
            rt: None,
            bitmap: None,
            base: session.base,
            objects: session.objects,
            history: History::default(),
            tool: Tool::Pen,
            selected: None,
            drag: Drag::None,
            buttons: Vec::new(),
            panel: None,
            hover: None,
            scale,
            eyedrop: None,
            edit: None,
            fonts,
            font_combo,
            weight_combo,
            size_combo,
            control_font,
            pen_cursor: make_pen_cursor(scale),
        });
        editor.rebuild();
        util::set_state(hwnd, editor);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

fn add_item(combo: HWND, text: &str) {
    let wide = HSTRING::from(text);
    unsafe {
        SendMessageW(
            combo,
            CB_ADDSTRING,
            None,
            Some(LPARAM(wide.as_ptr() as isize)),
        );
    }
}

fn select_item(combo: HWND, index: Option<usize>) {
    let value = index.unwrap_or(usize::MAX);
    unsafe {
        SendMessageW(combo, CB_SETCURSEL, Some(WPARAM(value)), None);
    }
}

fn combo_index(combo: HWND) -> Option<usize> {
    let index = unsafe { SendMessageW(combo, CB_GETCURSEL, None, None).0 };
    (index >= 0).then_some(index as usize)
}

impl Editor {
    /// Image scale and the client position of the image's top-left corner.
    fn view(&self) -> (f32, f32, f32) {
        let (cw, ch) = util::client_size(self.hwnd);
        let top = TOOLBAR_H * self.scale;
        let pad = 16.0 * self.scale;
        let (aw, ah) = (
            (cw as f32 - pad * 2.0).max(1.0),
            (ch as f32 - top - pad * 2.0).max(1.0),
        );
        let (iw, ih) = (self.base.w as f32, self.base.h as f32);
        let k = (aw / iw).min(ah / ih).min(1.0);
        (
            k,
            (cw as f32 - iw * k) / 2.0,
            top + (ch as f32 - top - ih * k) / 2.0,
        )
    }

    fn to_image(&self, x: i32, y: i32) -> Pt {
        let (k, ox, oy) = self.view();
        pt((x as f32 - ox) / k, (y as f32 - oy) / k)
    }

    fn to_screen(&self, p: Pt) -> Pt {
        let (k, ox, oy) = self.view();
        pt(p.x * k + ox, p.y * k + oy)
    }

    fn screen_rect(&self, r: RectF) -> RectF {
        let (a, b) = (self.to_screen(pt(r.l, r.t)), self.to_screen(pt(r.r, r.b)));
        RectF::new(a.x, a.y, b.x, b.y)
    }

    fn clamp_to_image(&self, p: Pt) -> Pt {
        pt(
            p.x.clamp(0.0, self.base.w as f32),
            p.y.clamp(0.0, self.base.h as f32),
        )
    }

    fn in_toolbar(&self, y: i32) -> bool {
        (y as f32) < TOOLBAR_H * self.scale
    }

    fn selected_object(&self) -> Option<&Object> {
        self.selected.and_then(|i| self.objects.get(i))
    }

    /// Lays out buttons for the current state and shows the font pickers where they belong.
    fn rebuild(&mut self) {
        let s = self.scale;
        let cfg = config::get();
        let (cw, _) = util::client_size(self.hwnd);
        let cw = cw as f32;
        let mut buttons = Vec::new();

        let mut push =
            |r: RectF, action: Option<Action>, look: Look, active: bool, enabled: bool| {
                buttons.push(Btn {
                    r,
                    action,
                    look,
                    active,
                    enabled,
                });
            };

        let (row1, row2, h) = (8.0 * s, 48.0 * s, 34.0 * s);
        let wide = 40.0 * s;
        let gap = 6.0 * s;

        let mut x = 10.0 * s;
        for (icon, name, action, enabled) in [
            (Icon::Undo, "復原", Action::Undo, self.history.can_undo()),
            (Icon::Redo, "重做", Action::Redo, self.history.can_redo()),
        ] {
            push(
                RectF::xywh(x, row1, wide, h),
                Some(action),
                Look::Icon(icon, name),
                false,
                enabled,
            );
            x += wide + gap;
        }
        x += 14.0 * s;
        let solid = cfg.fill_shapes;
        let pick = |filled: Icon, outlined: Icon| if solid { filled } else { outlined };
        for (icon, name, tool) in [
            (Icon::Select, "選取", Tool::Select),
            (Icon::Pen, "畫筆", Tool::Pen),
            (pick(Icon::RectFilled, Icon::Rect), "方形", Tool::Rect),
            (pick(Icon::CircleFilled, Icon::Circle), "圓形", Tool::Circle),
            (
                pick(Icon::EllipseFilled, Icon::Ellipse),
                "橢圓",
                Tool::Ellipse,
            ),
            (Icon::Text, "文字", Tool::Text),
        ] {
            push(
                RectF::xywh(x, row1, wide, h),
                Some(Action::Tool(tool)),
                Look::Icon(icon, name),
                self.tool == tool,
                true,
            );
            x += wide + gap;
        }
        x += 14.0 * s - gap;
        push(
            RectF::xywh(x, row1, wide, h),
            Some(Action::Fill(Scope::Default)),
            Look::Icon(Icon::Fill, fill_name(solid)),
            solid,
            true,
        );

        let action_w = 88.0 * s;
        push(
            RectF::xywh(cw - 10.0 * s - action_w, row1, action_w, h),
            Some(Action::Save),
            Look::Primary(Icon::Save, "儲存"),
            false,
            true,
        );
        push(
            RectF::xywh(cw - 10.0 * s - action_w * 2.0 - gap, row1, action_w, h),
            Some(Action::Copy),
            Look::Primary(Icon::Copy, "複製"),
            false,
            true,
        );

        let eye_scope = self.eyedrop.as_ref().map(|(scope, _)| *scope);
        x = 10.0 * s;
        x = color_controls(
            &mut push,
            x,
            row2,
            h,
            s,
            Scope::Default,
            cfg.color,
            eye_scope,
        );
        x += 14.0 * s;

        let mut combos_at = None;
        if self.tool == Tool::Text {
            combos_at = Some((x, row2 + 3.0 * s));
        } else {
            // The pen still uses thickness, so only a solid shape tool turns the control off.
            let shape_tool = matches!(self.tool, Tool::Rect | Tool::Circle | Tool::Ellipse);
            thickness_controls(
                &mut push,
                x,
                row2,
                h,
                s,
                Scope::Default,
                cfg.thickness,
                !(solid && shape_tool),
            );
        }

        // The floating panel for the selected object.
        self.panel = None;
        let idle = matches!(self.drag, Drag::None) && self.edit.is_none();
        if idle && let Some(object) = self.selected_object().cloned() {
            let is_text = object.is_text();
            let fill_w = if object.fillable() { 48.0 } else { 0.0 };
            let content_w =
                5.0 * 30.0 + 52.0 + 52.0 + 12.0 + if is_text { 334.0 } else { 118.0 + fill_w };
            let (pw, ph) = ((content_w + 20.0) * s, PANEL_H * s);
            let b = self.screen_rect(object.bounds());
            let (client_w, client_h) = util::client_size(self.hwnd);
            let top_limit = TOOLBAR_H * s + 4.0 * s;
            let mut py = b.t - ph - 12.0 * s;
            if py < top_limit {
                py = b.b + 12.0 * s;
                if py + ph > client_h as f32 - 4.0 * s {
                    py = top_limit;
                }
            }
            let px = ((b.l + b.r - pw) / 2.0)
                .clamp(4.0 * s, (client_w as f32 - pw - 4.0 * s).max(4.0 * s));
            self.panel = Some(RectF::xywh(px, py, pw, ph));
            let (ix, iy, ih) = (px + 10.0 * s, py + 6.0 * s, ph - 12.0 * s);
            let mut nx = color_controls(
                &mut push,
                ix,
                iy,
                ih,
                s,
                Scope::Selected,
                object.color,
                eye_scope,
            ) + 12.0 * s;
            if is_text {
                combos_at = Some((nx, iy + 1.0 * s));
            } else {
                if object.fillable() {
                    push(
                        RectF::xywh(nx, iy, wide, ih),
                        Some(Action::Fill(Scope::Selected)),
                        Look::Icon(Icon::Fill, fill_name(object.filled)),
                        object.filled,
                        true,
                    );
                    nx += fill_w * s;
                }
                thickness_controls(
                    &mut push,
                    nx,
                    iy,
                    ih,
                    s,
                    Scope::Selected,
                    object.thickness,
                    !object.filled,
                );
            }
        }

        self.buttons = buttons;
        self.place_combos(combos_at, &cfg);
    }

    fn place_combos(&self, at: Option<(f32, f32)>, cfg: &config::Config) {
        let combos = [self.font_combo, self.weight_combo, self.size_combo];
        unsafe {
            let Some((x, y)) = at else {
                for c in combos {
                    let _ = ShowWindow(c, SW_HIDE);
                }
                return;
            };
            let (font, weight, size) = match self.selected_object().map(|o| &o.shape) {
                Some(Shape::Text(t)) if self.panel.is_some() => (t.font.clone(), t.weight, t.size),
                _ => (cfg.font.clone(), cfg.font_weight, cfg.font_size),
            };
            select_item(self.font_combo, self.fonts.iter().position(|f| *f == font));
            select_item(
                self.weight_combo,
                WEIGHTS.iter().position(|(w, _)| *w == weight),
            );
            select_item(
                self.size_combo,
                SIZES.iter().position(|v| *v as f32 == size),
            );

            let s = self.scale;
            let mut cx = x;
            for (c, width) in [
                (self.font_combo, 180.0),
                (self.weight_combo, 76.0),
                (self.size_combo, 66.0),
            ] {
                let _ = SetWindowPos(
                    c,
                    Some(HWND_TOP),
                    cx as i32,
                    y as i32,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOACTIVATE,
                );
                let _ = ShowWindow(c, SW_SHOWNOACTIVATE);
                cx += (width + 6.0) * s;
            }
        }
    }

    fn refresh(&mut self) {
        self.rebuild();
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn paint(&mut self) -> windows::core::Result<()> {
        if self.rt.is_none() {
            self.rt = Some(gfx::hwnd_target(self.hwnd)?);
            self.bitmap = None;
        }
        let Some(target) = self.rt.clone() else {
            return Ok(());
        };
        let rt: ID2D1RenderTarget = target.cast()?;
        if self.bitmap.is_none() {
            self.bitmap = Some(gfx::bitmap(&rt, &self.base)?);
        }
        let (cw, _) = util::client_size(self.hwnd);
        let s = self.scale;
        let (k, ox, oy) = self.view();
        let (iw, ih) = (self.base.w as f32, self.base.h as f32);
        unsafe {
            rt.BeginDraw();
            rt.Clear(Some(&gfx::color(BG_CANVAS)));

            rt.SetTransform(&gfx::transform(k, ox, oy));
            rt.PushAxisAlignedClip(
                &gfx::rect(RectF::new(0.0, 0.0, iw, ih)),
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
            let mode = if k >= 1.0 {
                D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR
            } else {
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR
            };
            if let Some(bitmap) = &self.bitmap {
                rt.DrawBitmap(bitmap, None, 1.0, mode, None);
            }
            let hidden = self.edit.as_ref().and_then(|e| e.index);
            for (i, object) in self.objects.iter().enumerate() {
                if Some(i) != hidden {
                    gfx::draw_object(&rt, object);
                }
            }
            rt.PopAxisAlignedClip();
            rt.SetTransform(&gfx::identity());

            if self.edit.is_none()
                && let Some(object) = self.selected_object()
            {
                self.draw_selection(&rt, object);
            }

            gfx::fill(
                &rt,
                RectF::new(0.0, 0.0, cw as f32, TOOLBAR_H * s),
                gfx::color(BG_BAR),
            );
            if let Some(panel) = self.panel {
                gfx::fill_round(&rt, panel, 10.0 * s, gfx::color(BG_BAR));
                gfx::stroke_round(&rt, panel, 10.0 * s, 1.0, gfx::color_a(WHITE, 0.25));
            }
            for button in &self.buttons {
                self.draw_button(&rt, button);
            }
            self.draw_tooltip(&rt, cw as f32);
            if self.eyedrop.is_some() {
                let hint = RectF::new(
                    cw as f32 - 300.0 * s,
                    48.0 * s,
                    cw as f32 - 12.0 * s,
                    82.0 * s,
                );
                gfx::label(
                    &rt,
                    "點圖片吸取顏色，按 Esc 取消",
                    hint,
                    13.0 * s,
                    400,
                    MUTED,
                    Align::Leading,
                );
            }

            if rt.EndDraw(None, None).is_err() {
                self.rt = None;
                self.bitmap = None;
                let _ = InvalidateRect(Some(self.hwnd), None, false);
            }
        }
        Ok(())
    }

    fn draw_button(&self, rt: &ID2D1RenderTarget, b: &Btn) {
        let s = self.scale;
        let hovered = b.action.is_some() && b.action == self.hover && b.enabled;
        let radius = 6.0 * s;
        match &b.look {
            Look::Swatch(c) => {
                gfx::fill_round(rt, b.r, radius, gfx::color(*c));
                let width = if b.active { 2.5 * s } else { 1.0 };
                let edge = if b.active || hovered { WHITE } else { MUTED };
                gfx::stroke_round(rt, b.r, radius, width, gfx::color(edge));
            }
            Look::Value(text) => {
                let color = if b.enabled { WHITE } else { DISABLED };
                gfx::label(rt, text, b.r, 13.0 * s, 500, color, Align::Center);
            }
            Look::Primary(icon, text) => {
                gfx::fill_round(
                    rt,
                    b.r,
                    radius,
                    gfx::color(if hovered { ACCENT_HOVER } else { ACCENT }),
                );
                let glyph = 20.0 * s;
                let top = b.r.t + (b.r.h() - glyph) / 2.0;
                gfx::draw_icon(
                    rt,
                    *icon,
                    RectF::xywh(b.r.l + 12.0 * s, top, glyph, glyph),
                    WHITE,
                );
                let text_area = RectF::new(b.r.l + 36.0 * s, b.r.t, b.r.r - 6.0 * s, b.r.b);
                gfx::label(rt, text, text_area, 14.0 * s, 600, WHITE, Align::Leading);
            }
            Look::Icon(icon, _) => {
                let fill = if b.active {
                    ACCENT
                } else if hovered {
                    BTN_HOVER
                } else {
                    BTN
                };
                gfx::fill_round(rt, b.r, radius, gfx::color(fill));
                let color = if b.enabled { WHITE } else { DISABLED };
                let glyph = 20.0 * s;
                let area = RectF::xywh(
                    b.r.l + (b.r.w() - glyph) / 2.0,
                    b.r.t + (b.r.h() - glyph) / 2.0,
                    glyph,
                    glyph,
                );
                gfx::draw_icon(rt, *icon, area, color);
            }
        }
    }

    /// Names the hovered icon, since the buttons themselves have no text.
    fn draw_tooltip(&self, rt: &ID2D1RenderTarget, client_w: f32) {
        let s = self.scale;
        let hovered = self
            .buttons
            .iter()
            .find(|b| b.enabled && b.action.is_some() && b.action == self.hover);
        let Some(Btn {
            r,
            look: Look::Icon(_, name),
            ..
        }) = hovered
        else {
            return;
        };
        let (tw, th) = gfx::measure(name, "Microsoft JhengHei UI", 12.0 * s, 400);
        let (w, h) = (tw + 14.0 * s, th + 6.0 * s);
        let x = ((r.l + r.r - w) / 2.0).clamp(2.0, (client_w - w - 2.0).max(2.0));
        let tip = RectF::xywh(x, r.b + 4.0 * s, w, h);
        gfx::fill_round(rt, tip, 4.0 * s, gfx::color(Rgb::new(15, 15, 18)));
        gfx::stroke_round(rt, tip, 4.0 * s, 1.0, gfx::color_a(WHITE, 0.25));
        gfx::label(rt, name, tip, 12.0 * s, 400, WHITE, Align::Center);
    }

    fn draw_selection(&self, rt: &ID2D1RenderTarget, object: &Object) {
        let s = self.scale;
        let box_rect = self.screen_rect(object.bounds()).inflate(3.0 * s);
        let dashed = gfx::stroke_style(D2D1_CAP_STYLE_FLAT, D2D1_DASH_STYLE_DASH);
        unsafe {
            // White dashes over a dark line stay visible on any image.
            if let Ok(dark) = gfx::brush(rt, gfx::color_a(Rgb::new(0, 0, 0), 0.7)) {
                rt.DrawRectangle(&gfx::rect(box_rect), &dark, 1.5 * s, None);
            }
            if let Ok(light) = gfx::brush(rt, gfx::color(WHITE)) {
                rt.DrawRectangle(&gfx::rect(box_rect), &light, 1.5 * s, dashed.as_ref());
            }
        }
        if !object.is_text() {
            let half = HANDLE_SIZE * s / 2.0;
            for p in handle_points(box_rect) {
                let r = RectF::new(p.x - half, p.y - half, p.x + half, p.y + half);
                gfx::fill(rt, r, gfx::color(WHITE));
                if let Ok(edge) = gfx::brush(rt, gfx::color(Rgb::new(30, 30, 30))) {
                    unsafe { rt.DrawRectangle(&gfx::rect(r), &edge, 1.0, None) };
                }
            }
        }
    }

    fn button_at(&self, x: i32, y: i32) -> Option<&Btn> {
        let p = pt(x as f32, y as f32);
        self.buttons
            .iter()
            .rev()
            .find(|b| b.action.is_some() && b.r.contains(p))
    }

    fn handle_at(&self, x: i32, y: i32) -> Option<usize> {
        let object = self.selected_object()?;
        if object.is_text() {
            return None;
        }
        let box_rect = self.screen_rect(object.bounds()).inflate(3.0 * self.scale);
        let p = pt(x as f32, y as f32);
        let reach = HANDLE_SIZE * self.scale;
        handle_points(box_rect)
            .iter()
            .position(|h| distance(*h, p) <= reach)
    }

    fn object_at(&self, p: Pt) -> Option<usize> {
        let (k, _, _) = self.view();
        self.objects
            .iter()
            .rposition(|o| o.hit(p, HIT_TOLERANCE / k))
    }

    fn change<R>(&mut self, edit: impl FnOnce(&mut Self) -> R) -> R {
        let before = self.objects.clone();
        let result = edit(self);
        if self.objects != before {
            self.history.record(before);
        }
        self.refresh();
        result
    }

    fn set_color(&mut self, scope: Scope, color: Rgb) {
        match scope {
            Scope::Default => {
                config::update(|c| c.color = color);
                self.refresh();
            }
            Scope::Selected => self.change(|e| {
                if let Some(i) = e.selected
                    && let Some(o) = e.objects.get_mut(i)
                {
                    o.color = color;
                }
            }),
        }
    }

    fn adjust_thickness(&mut self, scope: Scope, delta: i32) {
        let clamp = |v: f32| (v + delta as f32).clamp(MIN_THICKNESS, MAX_THICKNESS);
        match scope {
            Scope::Default => {
                config::update(|c| c.thickness = clamp(c.thickness));
                self.refresh();
            }
            Scope::Selected => self.change(|e| {
                if let Some(i) = e.selected
                    && let Some(o) = e.objects.get_mut(i)
                {
                    o.thickness = clamp(o.thickness);
                }
            }),
        }
    }

    fn toggle_fill(&mut self, scope: Scope) {
        match scope {
            Scope::Default => {
                config::update(|c| c.fill_shapes = !c.fill_shapes);
                self.refresh();
            }
            Scope::Selected => self.change(|e| {
                if let Some(i) = e.selected
                    && let Some(o) = e.objects.get_mut(i)
                {
                    o.filled = !o.filled;
                }
            }),
        }
    }

    fn pick_custom_color(&mut self, scope: Scope) {
        let current = match scope {
            Scope::Default => config::get().color,
            Scope::Selected => self
                .selected_object()
                .map_or(config::get().color, |o| o.color),
        };
        let picked = CUSTOM_COLORS.with(|colors| {
            let mut colors = colors.borrow_mut();
            let mut dialog = CHOOSECOLORW {
                lStructSize: size_of::<CHOOSECOLORW>() as u32,
                hwndOwner: self.hwnd,
                rgbResult: util::colorref(current),
                lpCustColors: colors.as_mut_ptr(),
                Flags: CC_RGBINIT | CC_FULLOPEN,
                ..Default::default()
            };
            unsafe {
                ChooseColorW(&mut dialog)
                    .as_bool()
                    .then_some(util::rgb_from_colorref(dialog.rgbResult))
            }
        });
        if let Some(color) = picked {
            self.set_color(scope, color);
        }
    }

    fn run(&mut self, action: Action) {
        match action {
            Action::Tool(tool) => {
                self.tool = tool;
                self.selected = None;
                self.eyedrop = None;
                self.refresh();
            }
            Action::Swatch(scope, color) => self.set_color(scope, color),
            Action::Custom(scope) => self.pick_custom_color(scope),
            Action::Eye(scope) => {
                let already = self.eyedrop.as_ref().is_some_and(|(s, _)| *s == scope);
                self.eyedrop = if already {
                    None
                } else {
                    gfx::compose(&self.base, &self.objects)
                        .ok()
                        .map(|image| (scope, image))
                };
                self.refresh();
            }
            Action::Thick(scope, delta) => self.adjust_thickness(scope, delta),
            Action::Fill(scope) => self.toggle_fill(scope),
            Action::Undo => {
                if self.history.undo(&mut self.objects) {
                    self.selected = None;
                }
                self.refresh();
            }
            Action::Redo => {
                if self.history.redo(&mut self.objects) {
                    self.selected = None;
                }
                self.refresh();
            }
            Action::Copy => self.finish(false),
            Action::Save => self.finish(true),
        }
    }

    /// Delivers the result and closes the window. `self` is invalid once this destroys it.
    fn finish(&mut self, save: bool) {
        self.commit_text();
        let Ok(composed) = gfx::compose(&self.base, &self.objects) else {
            toast::show_message("無法輸出圖片".to_string(), None);
            return;
        };
        let session = Session {
            base: self.base.clone(),
            objects: self.objects.clone(),
            composed: Rc::new(composed),
        };
        let hwnd = self.hwnd;
        if save {
            let Some(path) = actions::save_with_recovery(&session.composed, Some(hwnd)) else {
                return;
            };
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            toast::show_card(session, format!("已儲存 {name}"));
        } else {
            actions::copy_and_notify(session);
        }
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }

    fn start_text(&mut self, origin: Pt, index: Option<usize>) {
        let cfg = config::get();
        let (text, font_name, weight, size, color) = match index
            .and_then(|i| self.objects.get(i))
            .map(|o| (&o.shape, o.color))
        {
            Some((Shape::Text(t), color)) => {
                (t.text.clone(), t.font.clone(), t.weight, t.size, color)
            }
            _ => (
                String::new(),
                cfg.font,
                cfg.font_weight,
                cfg.font_size,
                cfg.color,
            ),
        };
        let (k, _, _) = self.view();
        let at = self.to_screen(origin);
        unsafe {
            let face = HSTRING::from(font_name.as_str());
            let gdi_font = util::gdi_font(
                PCWSTR(face.as_ptr()),
                (size * k).round() as i32,
                weight as i32,
            );
            let shown = text.replace('\n', "\r\n");
            let Ok(edit) = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                &HSTRING::from(shown.as_str()),
                WS_CHILD
                    | WS_VISIBLE
                    | WS_BORDER
                    | WINDOW_STYLE(
                        (ES_MULTILINE | ES_AUTOHSCROLL | ES_AUTOVSCROLL | ES_WANTRETURN) as u32,
                    ),
                at.x as i32,
                at.y as i32,
                10,
                10,
                Some(self.hwnd),
                Some(HMENU(ID_TEXT as isize as *mut _)),
                Some(util::hinstance()),
                None,
            ) else {
                let _ = DeleteObject(gdi_font.into());
                return;
            };
            util::set_font(edit, gdi_font);
            SendMessageW(edit, EM_SETMARGINS, Some(WPARAM(3)), Some(LPARAM(0)));
            let _ = SetWindowSubclass(edit, Some(text_subclass), 1, self.hwnd.0 as usize);
            let brush = CreateSolidBrush(util::colorref(color.contrast()));
            self.edit = Some(TextEdit {
                hwnd: edit,
                index,
                origin,
                font_name,
                weight,
                size,
                color,
                gdi_font,
                brush,
                before: self.objects.clone(),
            });
            self.fit_text_box();
            let _ = SetFocus(Some(edit));
            SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
        }
        self.refresh();
    }

    fn fit_text_box(&self) {
        let Some(edit) = &self.edit else { return };
        let (k, _, _) = self.view();
        let s = self.scale;
        let text = read_text(edit.hwnd);
        let (w, h) = gfx::measure(&text, &edit.font_name, edit.size * k, edit.weight);
        unsafe {
            let _ = SetWindowPos(
                edit.hwnd,
                None,
                0,
                0,
                (w + 26.0 * s).max(110.0 * s) as i32,
                (h + 8.0 * s) as i32,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    /// Ends text entry if it is active. Empty text creates nothing and deletes an edited object.
    fn commit_text(&mut self) {
        let Some(edit) = self.edit.take() else { return };
        let text = read_text(edit.hwnd);
        unsafe {
            let _ = DestroyWindow(edit.hwnd);
            let _ = DeleteObject(edit.gdi_font.into());
            let _ = DeleteObject(edit.brush.into());
            let _ = SetFocus(Some(self.hwnd));
        }
        match edit.index {
            Some(i) if text.trim().is_empty() => {
                if i < self.objects.len() {
                    self.objects.remove(i);
                }
                self.selected = None;
            }
            Some(i) => {
                if let Some(Object {
                    shape: Shape::Text(t),
                    ..
                }) = self.objects.get_mut(i)
                {
                    t.text = text;
                    gfx::measure_text_shape(t);
                }
            }
            None if text.trim().is_empty() => {}
            None => {
                let mut shape = TextShape {
                    origin: edit.origin,
                    text,
                    font: edit.font_name,
                    weight: edit.weight,
                    size: edit.size,
                    measured: (0.0, 0.0),
                };
                gfx::measure_text_shape(&mut shape);
                self.objects.push(Object {
                    shape: Shape::Text(shape),
                    color: edit.color,
                    thickness: 1.0,
                    filled: false,
                });
            }
        }
        if self.objects != edit.before {
            self.history.record(edit.before);
        }
        self.refresh();
    }

    fn apply_font_change(&mut self, id: i32, combo: HWND) {
        let Some(index) = combo_index(combo) else {
            return;
        };
        let font = self.fonts.get(index).cloned();
        let weight = WEIGHTS.get(index).map(|(w, _)| *w);
        let size = SIZES.get(index).map(|v| *v as f32);
        let selected_text = self.selected_object().is_some_and(Object::is_text);
        if selected_text {
            self.change(|e| {
                if let Some(i) = e.selected
                    && let Some(Object {
                        shape: Shape::Text(t),
                        ..
                    }) = e.objects.get_mut(i)
                {
                    match id {
                        ID_FONT => t.font = font.unwrap_or_else(|| t.font.clone()),
                        ID_WEIGHT => t.weight = weight.unwrap_or(t.weight),
                        _ => t.size = size.unwrap_or(t.size),
                    }
                    gfx::measure_text_shape(t);
                }
            });
        } else {
            config::update(|c| match id {
                ID_FONT => c.font = font.unwrap_or_else(|| c.font.clone()),
                ID_WEIGHT => c.font_weight = weight.unwrap_or(c.font_weight),
                _ => c.font_size = size.unwrap_or(c.font_size),
            });
            self.refresh();
        }
        unsafe {
            let _ = SetFocus(Some(self.hwnd));
        }
    }

    fn mouse_down(&mut self, x: i32, y: i32) {
        unsafe {
            let _ = SetFocus(Some(self.hwnd));
        }
        if self.edit.is_some() {
            self.commit_text();
            return;
        }
        if let Some(button) = self.button_at(x, y) {
            if button.enabled
                && let Some(action) = button.action
            {
                self.run(action);
            }
            return;
        }
        if self.in_toolbar(y)
            || self
                .panel
                .is_some_and(|p| p.contains(pt(x as f32, y as f32)))
        {
            return;
        }

        let p = self.to_image(x, y);
        if let Some((scope, image)) = self.eyedrop.take() {
            let inside = RectF::new(0.0, 0.0, self.base.w as f32, self.base.h as f32).contains(p);
            if inside {
                self.set_color(scope, image.pixel(p.x as i32, p.y as i32));
            } else {
                self.refresh();
            }
            return;
        }

        let cfg = config::get();
        let before = self.objects.clone();
        let c = self.clamp_to_image(p);
        let empty = RectF::new(c.x, c.y, c.x, c.y);
        let new_shape = match self.tool {
            Tool::Select => None,
            Tool::Pen => Some(Shape::Pen(vec![c])),
            Tool::Rect => Some(Shape::Rect(empty)),
            Tool::Circle => Some(Shape::Circle(empty)),
            Tool::Ellipse => Some(Shape::Ellipse(empty)),
            Tool::Text => {
                self.start_text(c, None);
                return;
            }
        };

        if let Some(shape) = new_shape {
            self.selected = None;
            let mut object = Object {
                shape,
                color: cfg.color,
                thickness: cfg.thickness,
                filled: false,
            };
            object.filled = cfg.fill_shapes && object.fillable();
            self.objects.push(object);
            self.drag = Drag::Draw { anchor: c, before };
        } else if let Some(handle) = self.handle_at(x, y)
            && let Some(original) = self.selected_object().cloned()
        {
            let bounds = original.bounds();
            self.drag = Drag::Resize {
                handle,
                original,
                bounds,
                before,
            };
        } else if let Some(index) = self.object_at(p) {
            self.selected = Some(index);
            self.drag = Drag::Move {
                from: p,
                original: self.objects[index].clone(),
                before,
            };
        } else {
            self.selected = None;
        }
        unsafe {
            if !matches!(self.drag, Drag::None) {
                SetCapture(self.hwnd);
            }
        }
        self.refresh();
    }

    fn mouse_move(&mut self, x: i32, y: i32, shift: bool) {
        let p = self.to_image(x, y);
        let clamped = self.clamp_to_image(p);
        let lock_rect = shift;
        match &self.drag {
            Drag::None => {
                let over = self
                    .button_at(x, y)
                    .filter(|b| b.enabled)
                    .and_then(|b| b.action);
                if over != self.hover {
                    self.hover = over;
                    unsafe {
                        let _ = InvalidateRect(Some(self.hwnd), None, false);
                    }
                }
                return;
            }
            Drag::Draw { anchor, .. } => {
                let anchor = *anchor;
                if let Some(object) = self.objects.last_mut() {
                    match &mut object.shape {
                        Shape::Pen(points) => {
                            if points.last().is_none_or(|l| distance(*l, clamped) >= 0.5) {
                                points.push(clamped);
                            }
                        }
                        Shape::Rect(r) => {
                            *r = if lock_rect {
                                square_from(anchor, clamped)
                            } else {
                                RectF::from_points(anchor, clamped)
                            };
                        }
                        Shape::Circle(r) => *r = square_from(anchor, clamped),
                        Shape::Ellipse(r) => *r = RectF::from_points(anchor, clamped),
                        Shape::Text(_) => {}
                    }
                }
            }
            Drag::Move { from, original, .. } => {
                let (dx, dy) = (p.x - from.x, p.y - from.y);
                let mut moved = original.clone();
                moved.translate(dx, dy);
                if let Some(i) = self.selected {
                    self.objects[i] = moved;
                }
            }
            Drag::Resize {
                handle,
                original,
                bounds,
                ..
            } => {
                let lock = match original.shape {
                    Shape::Circle(_) => true,
                    Shape::Rect(_) => lock_rect,
                    _ => false,
                };
                let outer = resize_bounds(*bounds, *handle, p, lock);
                let inset = original.stroke_width() / 2.0;
                let inner = RectF::new(
                    outer.l + inset,
                    outer.t + inset,
                    (outer.r - inset).max(outer.l + inset + 1.0),
                    (outer.b - inset).max(outer.t + inset + 1.0),
                );
                let mut resized = original.clone();
                resized.fit(original.geometry(), inner);
                if let Some(i) = self.selected {
                    self.objects[i] = resized;
                }
            }
        }
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn mouse_up(&mut self) {
        let drag = std::mem::replace(&mut self.drag, Drag::None);
        unsafe {
            let _ = ReleaseCapture();
        }
        let before = match drag {
            Drag::None => return,
            Drag::Draw { before, .. } => {
                // A click without a drag makes a dot with the pen, but nothing with a shape tool.
                let degenerate = self.objects.last().is_some_and(|o| match &o.shape {
                    Shape::Rect(r) | Shape::Circle(r) | Shape::Ellipse(r) => {
                        r.w() < 2.0 && r.h() < 2.0
                    }
                    _ => false,
                });
                if degenerate {
                    self.objects.pop();
                }
                before
            }
            Drag::Move { before, .. } | Drag::Resize { before, .. } => before,
        };
        if self.objects != before {
            self.history.record(before);
        }
        self.refresh();
    }

    /// Returns false when a double-click has no meaning here. Windows reports the second of two
    /// quick clicks only as a double-click, so the caller must then treat it as a normal press;
    /// otherwise every other click on a button such as + would be lost.
    fn double_click(&mut self, x: i32, y: i32) -> bool {
        if self.tool != Tool::Select
            || self.edit.is_some()
            || self.eyedrop.is_some()
            || self.over_controls(x, y)
        {
            return false;
        }
        let p = self.to_image(x, y);
        if let Some(index) = self.object_at(p)
            && let Shape::Text(t) = &self.objects[index].shape
        {
            let origin = t.origin;
            self.selected = Some(index);
            self.start_text(origin, Some(index));
            return true;
        }
        false
    }

    fn delete_selected(&mut self) {
        let Some(index) = self.selected else { return };
        self.change(|e| {
            if index < e.objects.len() {
                e.objects.remove(index);
            }
            e.selected = None;
        });
    }

    fn over_controls(&self, x: i32, y: i32) -> bool {
        self.in_toolbar(y)
            || self.button_at(x, y).is_some()
            || self
                .panel
                .is_some_and(|p| p.contains(pt(x as f32, y as f32)))
    }

    fn cursor_for(&self, x: i32, y: i32) -> HCURSOR {
        if self.tool == Tool::Pen && self.eyedrop.is_none() && !self.over_controls(x, y) {
            return self.pen_cursor;
        }
        unsafe { LoadCursorW(None, self.stock_cursor_for(x, y)).unwrap_or_default() }
    }

    fn stock_cursor_for(&self, x: i32, y: i32) -> PCWSTR {
        if self.over_controls(x, y) {
            return IDC_ARROW;
        }
        if self.eyedrop.is_some() {
            return IDC_CROSS;
        }
        match self.tool {
            Tool::Text => IDC_IBEAM,
            Tool::Select => {
                if let Some(handle) = self.handle_at(x, y) {
                    return match handle {
                        0 | 4 => IDC_SIZENWSE,
                        2 | 6 => IDC_SIZENESW,
                        1 | 5 => IDC_SIZENS,
                        _ => IDC_SIZEWE,
                    };
                }
                if self.object_at(self.to_image(x, y)).is_some() {
                    IDC_SIZEALL
                } else {
                    IDC_ARROW
                }
            }
            _ => IDC_CROSS,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn color_controls(
    push: &mut impl FnMut(RectF, Option<Action>, Look, bool, bool),
    start: f32,
    y: f32,
    h: f32,
    s: f32,
    scope: Scope,
    current: Rgb,
    eye_scope: Option<Scope>,
) -> f32 {
    let mut x = start;
    let chip = 24.0 * s;
    let chip_y = y + (h - chip) / 2.0;
    for color in PRESETS {
        push(
            RectF::xywh(x, chip_y, chip, chip),
            Some(Action::Swatch(scope, color)),
            Look::Swatch(color),
            color == current,
            true,
        );
        x += 30.0 * s;
    }
    // Shows the current color when it is not one of the presets, and opens the full picker.
    push(
        RectF::xywh(x, y, 46.0 * s, h),
        Some(Action::Custom(scope)),
        Look::Icon(Icon::Palette, "自訂顏色"),
        !PRESETS.contains(&current),
        true,
    );
    x += 52.0 * s;
    push(
        RectF::xywh(x, y, 46.0 * s, h),
        Some(Action::Eye(scope)),
        Look::Icon(Icon::Eyedropper, "吸管"),
        eye_scope == Some(scope),
        true,
    );
    x + 52.0 * s
}

fn fill_name(solid: bool) -> &'static str {
    if solid {
        "圖形：實心"
    } else {
        "圖形：空心"
    }
}

#[allow(clippy::too_many_arguments)]
fn thickness_controls(
    push: &mut impl FnMut(RectF, Option<Action>, Look, bool, bool),
    start: f32,
    y: f32,
    h: f32,
    s: f32,
    scope: Scope,
    value: f32,
    enabled: bool,
) {
    let side = 32.0 * s;
    push(
        RectF::xywh(start, y, side, h),
        Some(Action::Thick(scope, -1)),
        Look::Icon(Icon::Minus, "減少粗細"),
        false,
        enabled && value > MIN_THICKNESS,
    );
    push(
        RectF::xywh(start + side, y, 50.0 * s, h),
        None,
        Look::Value(format!("{value:.0} px")),
        false,
        enabled,
    );
    push(
        RectF::xywh(start + side + 50.0 * s, y, side, h),
        Some(Action::Thick(scope, 1)),
        Look::Icon(Icon::Plus, "增加粗細"),
        false,
        enabled && value < MAX_THICKNESS,
    );
}

fn read_text(edit: HWND) -> String {
    unsafe {
        let length = GetWindowTextLengthW(edit);
        let mut buffer = vec![0u16; length as usize + 1];
        let copied = GetWindowTextW(edit, &mut buffer);
        String::from_utf16_lossy(&buffer[..copied as usize]).replace("\r\n", "\n")
    }
}

/// Lets the text box end editing on Esc, Ctrl+Enter or losing focus. The parent is told through
/// a posted message because the box cannot be destroyed from inside its own procedure.
unsafe extern "system" fn text_subclass(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    parent: usize,
) -> LRESULT {
    unsafe {
        let parent = HWND(parent as *mut _);
        match msg {
            WM_KEYDOWN => {
                let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                if wp.0 == VK_ESCAPE.0 as usize || (ctrl && wp.0 == VK_RETURN.0 as usize) {
                    let _ = PostMessageW(Some(parent), WM_COMMIT_TEXT, WPARAM(0), LPARAM(0));
                    return LRESULT(0);
                }
            }
            // Ctrl+Enter and Esc also arrive as characters, which would otherwise be inserted.
            WM_CHAR if wp.0 == 0x0A || wp.0 == 0x1B => return LRESULT(0),
            WM_KILLFOCUS => {
                let _ = PostMessageW(Some(parent), WM_COMMIT_TEXT, WPARAM(0), LPARAM(0));
            }
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(hwnd, Some(text_subclass), id);
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let Some(state) = util::state::<Editor>(hwnd) else {
            return DefWindowProcW(hwnd, msg, wp, lp);
        };
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut ps);
                let _ = state.paint();
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_SIZE => {
                if let Some(rt) = &state.rt {
                    let (w, h) = util::client_size(hwnd);
                    let _ = rt.Resize(&D2D_SIZE_U {
                        width: w.max(1) as u32,
                        height: h.max(1) as u32,
                    });
                }
                state.fit_text_box();
                state.refresh();
                LRESULT(0)
            }
            WM_GETMINMAXINFO => {
                let info = &mut *(lp.0 as *mut MINMAXINFO);
                info.ptMinTrackSize.x = (820.0 * state.scale) as i32;
                info.ptMinTrackSize.y = (480.0 * state.scale) as i32;
                LRESULT(0)
            }
            WM_DPICHANGED => {
                state.scale = (wp.0 & 0xFFFF) as f32 / 96.0;
                let _ = DestroyCursor(state.pen_cursor);
                state.pen_cursor = make_pen_cursor(state.scale);
                let suggested = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                state.refresh();
                LRESULT(0)
            }
            WM_SETCURSOR => {
                if (lp.0 & 0xFFFF) as u32 == HTCLIENT && wp.0 == hwnd.0 as usize {
                    let mut p = util::cursor_pos();
                    let _ = ScreenToClient(hwnd, &mut p);
                    SetCursor(Some(state.cursor_for(p.x, p.y)));
                    return LRESULT(1);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = util::lparam_point(lp);
                state.mouse_down(x, y);
                LRESULT(0)
            }
            WM_LBUTTONDBLCLK => {
                let (x, y) = util::lparam_point(lp);
                if !state.double_click(x, y) {
                    state.mouse_down(x, y);
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = util::lparam_point(lp);
                state.mouse_move(x, y, wp.0 & MK_SHIFT != 0);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                state.mouse_up();
                LRESULT(0)
            }
            WM_KEYDOWN => {
                let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                let key = wp.0 as u16;
                if ctrl && key == 0x5A {
                    state.run(Action::Undo);
                } else if ctrl && key == 0x59 {
                    state.run(Action::Redo);
                } else if key == VK_DELETE.0 {
                    state.delete_selected();
                } else if key == VK_ESCAPE.0 {
                    if state.eyedrop.is_some() {
                        state.eyedrop = None;
                    } else {
                        state.selected = None;
                    }
                    state.refresh();
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = (wp.0 & 0xFFFF) as i32;
                let code = ((wp.0 >> 16) & 0xFFFF) as u32;
                match id {
                    ID_FONT if code == CBN_SELCHANGE => {
                        state.apply_font_change(id, state.font_combo)
                    }
                    ID_WEIGHT if code == CBN_SELCHANGE => {
                        state.apply_font_change(id, state.weight_combo)
                    }
                    ID_SIZE if code == CBN_SELCHANGE => {
                        state.apply_font_change(id, state.size_combo)
                    }
                    ID_TEXT if code == EN_CHANGE => state.fit_text_box(),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_COMMIT_TEXT => {
                state.commit_text();
                LRESULT(0)
            }
            WM_CTLCOLOREDIT => {
                if let Some(edit) = &state.edit
                    && edit.hwnd.0 as isize == lp.0
                {
                    let dc = HDC(wp.0 as *mut _);
                    SetTextColor(dc, util::colorref(edit.color));
                    SetBkColor(dc, util::colorref(edit.color.contrast()));
                    return LRESULT(edit.brush.0 as isize);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_CLOSE => {
                state.commit_text();
                let _ = DestroyWindow(hwnd);
                actions::preview_clipboard();
                LRESULT(0)
            }
            WM_NCDESTROY => {
                let _ = DeleteObject(state.control_font.into());
                let _ = DestroyCursor(state.pen_cursor);
                util::take_state::<Editor>(hwnd);
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
