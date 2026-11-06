//! Direct2D, DirectWrite and WIC helpers. All render targets use 96 DPI, so one drawing unit is
//! one physical pixel and each window applies its own scale.

use crate::imaging::Image;
use crate::model::{Object, Pt, RectF, Rgb, Shape, TextShape};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::core::*;
use windows_numerics::{Matrix3x2, Vector2};

thread_local! {
    static D2D: ID2D1Factory = unsafe {
        D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).expect("Direct2D is unavailable")
    };
    static DWRITE: IDWriteFactory = unsafe {
        DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).expect("DirectWrite is unavailable")
    };
    static WIC: IWICImagingFactory = unsafe {
        CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).expect("WIC is unavailable")
    };
}

pub fn d2d() -> ID2D1Factory {
    D2D.with(Clone::clone)
}

pub fn dwrite() -> IDWriteFactory {
    DWRITE.with(Clone::clone)
}

pub fn color(c: Rgb) -> D2D1_COLOR_F {
    color_a(c, 1.0)
}

pub fn color_a(c: Rgb, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: c.r as f32 / 255.0,
        g: c.g as f32 / 255.0,
        b: c.b as f32 / 255.0,
        a,
    }
}

pub fn rect(r: RectF) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.l,
        top: r.t,
        right: r.r,
        bottom: r.b,
    }
}

pub fn point(p: Pt) -> Vector2 {
    Vector2 { X: p.x, Y: p.y }
}

pub fn identity() -> Matrix3x2 {
    transform(1.0, 0.0, 0.0)
}

pub fn transform(scale: f32, x: f32, y: f32) -> Matrix3x2 {
    Matrix3x2 {
        M11: scale,
        M12: 0.0,
        M21: 0.0,
        M22: scale,
        M31: x,
        M32: y,
    }
}

pub fn hwnd_target(hwnd: HWND) -> Result<ID2D1HwndRenderTarget> {
    let (w, h) = crate::util::client_size(hwnd);
    let props = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_IGNORE,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        usage: D2D1_RENDER_TARGET_USAGE_NONE,
        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
    };
    let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
        hwnd,
        pixelSize: D2D_SIZE_U {
            width: w.max(1) as u32,
            height: h.max(1) as u32,
        },
        presentOptions: D2D1_PRESENT_OPTIONS_NONE,
    };
    unsafe { d2d().CreateHwndRenderTarget(&props, &hwnd_props) }
}

pub fn bitmap(rt: &ID2D1RenderTarget, image: &Image) -> Result<ID2D1Bitmap> {
    let props = D2D1_BITMAP_PROPERTIES {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_IGNORE,
        },
        dpiX: 96.0,
        dpiY: 96.0,
    };
    unsafe {
        rt.CreateBitmap(
            D2D_SIZE_U {
                width: image.w,
                height: image.h,
            },
            Some(image.bgra.as_ptr().cast()),
            image.w * 4,
            &props,
        )
    }
}

pub fn brush(rt: &ID2D1RenderTarget, c: D2D1_COLOR_F) -> Result<ID2D1SolidColorBrush> {
    unsafe { rt.CreateSolidColorBrush(&c, None) }
}

pub fn fill(rt: &ID2D1RenderTarget, r: RectF, c: D2D1_COLOR_F) {
    if let Ok(b) = brush(rt, c) {
        unsafe { rt.FillRectangle(&rect(r), &b) };
    }
}

pub fn fill_round(rt: &ID2D1RenderTarget, r: RectF, radius: f32, c: D2D1_COLOR_F) {
    if let Ok(b) = brush(rt, c) {
        let rounded = D2D1_ROUNDED_RECT {
            rect: rect(r),
            radiusX: radius,
            radiusY: radius,
        };
        unsafe { rt.FillRoundedRectangle(&rounded, &b) };
    }
}

pub fn stroke_round(rt: &ID2D1RenderTarget, r: RectF, radius: f32, width: f32, c: D2D1_COLOR_F) {
    if let Ok(b) = brush(rt, c) {
        let rounded = D2D1_ROUNDED_RECT {
            rect: rect(r),
            radiusX: radius,
            radiusY: radius,
        };
        unsafe { rt.DrawRoundedRectangle(&rounded, &b, width, None) };
    }
}

pub fn stroke_style(cap: D2D1_CAP_STYLE, dash: D2D1_DASH_STYLE) -> Option<ID2D1StrokeStyle> {
    let props = D2D1_STROKE_STYLE_PROPERTIES {
        startCap: cap,
        endCap: cap,
        dashCap: D2D1_CAP_STYLE_FLAT,
        lineJoin: D2D1_LINE_JOIN_ROUND,
        miterLimit: 10.0,
        dashStyle: dash,
        dashOffset: 0.0,
    };
    unsafe { d2d().CreateStrokeStyle(&props, None).ok() }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Align {
    Leading,
    Center,
}

fn text_format(font: &str, size: f32, weight: u32) -> Result<IDWriteTextFormat> {
    unsafe {
        dwrite().CreateTextFormat(
            &HSTRING::from(font),
            None,
            DWRITE_FONT_WEIGHT(weight as i32),
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size.max(1.0),
            w!("zh-tw"),
        )
    }
}

fn text_layout(text: &str, font: &str, size: f32, weight: u32) -> Result<IDWriteTextLayout> {
    let format = text_format(font, size, weight)?;
    let wide: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        dwrite().CreateTextLayout(&wide, &format, 100_000.0, 100_000.0)
    }
}

/// Width and height of laid-out text. An empty line still has height, so a new label has a box.
pub fn measure(text: &str, font: &str, size: f32, weight: u32) -> (f32, f32) {
    let sample = if text.is_empty() { " " } else { text };
    let Ok(layout) = text_layout(sample, font, size, weight) else {
        return (size, size * 1.3);
    };
    let mut metrics = DWRITE_TEXT_METRICS::default();
    match unsafe { layout.GetMetrics(&mut metrics) } {
        Ok(()) => (metrics.widthIncludingTrailingWhitespace, metrics.height),
        Err(_) => (size, size * 1.3),
    }
}

/// Single-line interface text, vertically centered in `r`.
pub fn label(
    rt: &ID2D1RenderTarget,
    text: &str,
    r: RectF,
    size: f32,
    weight: u32,
    c: Rgb,
    align: Align,
) {
    label_with_font(rt, text, "Microsoft JhengHei UI", r, size, weight, c, align);
}

#[allow(clippy::too_many_arguments)]
pub fn label_with_font(
    rt: &ID2D1RenderTarget,
    text: &str,
    font: &str,
    r: RectF,
    size: f32,
    weight: u32,
    c: Rgb,
    align: Align,
) {
    let Ok(format) = text_format(font, size, weight) else {
        return;
    };
    let Ok(b) = brush(rt, color(c)) else { return };
    let wide: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        let _ = format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
        let _ = format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
        let _ = format.SetTextAlignment(match align {
            Align::Leading => DWRITE_TEXT_ALIGNMENT_LEADING,
            Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
        });
        rt.DrawText(
            &wide,
            &format,
            &rect(r),
            &b,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }
}

/// Installed font families, using the Traditional Chinese name when a font has one.
pub fn font_families() -> Vec<String> {
    let mut names = Vec::new();
    unsafe {
        let mut collection = None;
        if dwrite()
            .GetSystemFontCollection(&mut collection, false)
            .is_err()
        {
            return names;
        }
        let Some(collection) = collection else {
            return names;
        };
        for i in 0..collection.GetFontFamilyCount() {
            let Ok(family) = collection.GetFontFamily(i) else {
                continue;
            };
            let Ok(localized) = family.GetFamilyNames() else {
                continue;
            };
            let (mut index, mut exists) = (0, BOOL(0));
            if localized
                .FindLocaleName(w!("zh-tw"), &mut index, &mut exists)
                .is_err()
                || !exists.as_bool()
            {
                let _ = localized.FindLocaleName(w!("en-us"), &mut index, &mut exists);
                if !exists.as_bool() {
                    index = 0;
                }
            }
            let Ok(length) = localized.GetStringLength(index) else {
                continue;
            };
            let mut buffer = vec![0u16; length as usize + 1];
            if localized.GetString(index, &mut buffer).is_ok() {
                names.push(String::from_utf16_lossy(&buffer[..length as usize]));
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

pub fn measure_text_shape(t: &mut TextShape) {
    t.measured = measure(&t.text, &t.font, t.size, t.weight);
}

pub fn draw_object(rt: &ID2D1RenderTarget, object: &Object) {
    let Ok(b) = brush(rt, color(object.color)) else {
        return;
    };
    let width = object.thickness;
    let round = stroke_style(D2D1_CAP_STYLE_ROUND, D2D1_DASH_STYLE_SOLID);
    unsafe {
        match &object.shape {
            Shape::Pen(points) => match points.as_slice() {
                [] => {}
                [only] => rt.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: point(*only),
                        radiusX: width / 2.0,
                        radiusY: width / 2.0,
                    },
                    &b,
                ),
                many => {
                    if let Ok(path) = polyline(many) {
                        rt.DrawGeometry(&path, &b, width, round.as_ref());
                    }
                }
            },
            Shape::Rect(r) if object.filled => rt.FillRectangle(&rect(*r), &b),
            Shape::Rect(r) => rt.DrawRectangle(&rect(*r), &b, width, round.as_ref()),
            Shape::Circle(r) | Shape::Ellipse(r) => {
                let ellipse = D2D1_ELLIPSE {
                    point: Vector2 {
                        X: (r.l + r.r) / 2.0,
                        Y: (r.t + r.b) / 2.0,
                    },
                    radiusX: r.w() / 2.0,
                    radiusY: r.h() / 2.0,
                };
                if object.filled {
                    rt.FillEllipse(&ellipse, &b);
                } else {
                    rt.DrawEllipse(&ellipse, &b, width, round.as_ref());
                }
            }
            Shape::Text(t) => {
                if let Ok(layout) = text_layout(&t.text, &t.font, t.size, t.weight) {
                    rt.DrawTextLayout(
                        Vector2 {
                            X: t.origin.x,
                            Y: t.origin.y,
                        },
                        &layout,
                        &b,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                }
            }
        }
    }
}

fn polyline(points: &[Pt]) -> Result<ID2D1PathGeometry> {
    unsafe {
        let path = d2d().CreatePathGeometry()?;
        let sink = path.Open()?;
        sink.BeginFigure(point(points[0]), D2D1_FIGURE_BEGIN_HOLLOW);
        let rest: Vec<Vector2> = points[1..].iter().copied().map(point).collect();
        sink.AddLines(&rest);
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        sink.Close()?;
        Ok(path)
    }
}

/// A software render target whose pixels can be read back from the returned bitmap.
fn offscreen_target(w: u32, h: u32) -> Result<(IWICBitmap, ID2D1RenderTarget)> {
    unsafe {
        let wic = WIC.with(Clone::clone);
        let target =
            wic.CreateBitmap(w, h, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)?;
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let rt = d2d().CreateWicBitmapRenderTarget(&target, &props)?;
        Ok((target, rt))
    }
}

/// Straight-alpha BGRA pixels of a white icon with a dark outline, for use as a mouse cursor.
pub fn outlined_icon_pixels(icon: Icon, size: u32) -> Result<Vec<u32>> {
    unsafe {
        let (target, rt) = offscreen_target(size, size)?;
        let area = RectF::xywh(0.0, 0.0, size as f32, size as f32);
        rt.BeginDraw();
        draw_icon_stroked(&rt, icon, area, Rgb::new(0, 0, 0), 4.2);
        draw_icon_stroked(&rt, icon, area, Rgb::new(255, 255, 255), 1.8);
        rt.EndDraw(None, None)?;

        let mut premultiplied = vec![0u8; size as usize * size as usize * 4];
        target.CopyPixels(std::ptr::null(), size * 4, &mut premultiplied)?;
        Ok(premultiplied
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&[b, g, r, a]| {
                let straight = |c: u8| {
                    if a == 0 {
                        0
                    } else {
                        (c as u32 * 255 / a as u32).min(255)
                    }
                };
                straight(b) | straight(g) << 8 | straight(r) << 16 | (a as u32) << 24
            })
            .collect())
    }
}

/// Flattens annotations onto the base image at full resolution.
pub fn compose(base: &Image, objects: &[Object]) -> Result<Image> {
    if objects.is_empty() {
        return Ok(base.clone());
    }
    unsafe {
        let (target, rt) = offscreen_target(base.w, base.h)?;
        rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let image = bitmap(&rt, base)?;
        rt.BeginDraw();
        rt.DrawBitmap(
            &image,
            None,
            1.0,
            D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
            None,
        );
        for object in objects {
            draw_object(&rt, object);
        }
        rt.EndDraw(None, None)?;

        let mut bgra = vec![0u8; base.bgra.len()];
        target.CopyPixels(std::ptr::null(), base.w * 4, &mut bgra)?;
        for px in bgra.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        Ok(Image {
            w: base.w,
            h: base.h,
            bgra,
        })
    }
}

/// Toolbar glyphs, drawn from strokes on a 24x24 grid so they stay sharp at any display scale.
#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Undo,
    Redo,
    Select,
    Pen,
    Rect,
    RectFilled,
    Circle,
    CircleFilled,
    Ellipse,
    EllipseFilled,
    /// The solid / outlined toggle.
    Fill,
    Text,
    Palette,
    Eyedropper,
    Minus,
    Plus,
    Copy,
    Save,
    Close,
}

pub fn draw_icon(rt: &ID2D1RenderTarget, icon: Icon, area: RectF, c: Rgb) {
    draw_icon_stroked(rt, icon, area, c, 1.8);
}

/// `stroke` is measured in units of the icon's 24x24 grid.
fn draw_icon_stroked(rt: &ID2D1RenderTarget, icon: Icon, area: RectF, c: Rgb, stroke: f32) {
    let Ok(b) = brush(rt, color(c)) else { return };
    let unit = area.w() / 24.0;
    let width = (stroke * unit).max(1.0);
    let round = stroke_style(D2D1_CAP_STYLE_ROUND, D2D1_DASH_STYLE_SOLID);
    let at = |x: f32, y: f32| Pt {
        x: area.l + x * unit,
        y: area.t + y * unit,
    };
    let line = |points: &[(f32, f32)]| {
        let mapped: Vec<Pt> = points.iter().map(|(x, y)| at(*x, *y)).collect();
        if let Ok(path) = polyline(&mapped) {
            unsafe { rt.DrawGeometry(&path, &b, width, round.as_ref()) };
        }
    };
    let oval = |x: f32, y: f32, rx: f32, ry: f32| {
        let ellipse = D2D1_ELLIPSE {
            point: point(at(x, y)),
            radiusX: rx * unit,
            radiusY: ry * unit,
        };
        unsafe { rt.DrawEllipse(&ellipse, &b, width, round.as_ref()) };
    };
    let solid_oval = |x: f32, y: f32, rx: f32, ry: f32| {
        let ellipse = D2D1_ELLIPSE {
            point: point(at(x, y)),
            radiusX: rx * unit,
            radiusY: ry * unit,
        };
        unsafe { rt.FillEllipse(&ellipse, &b) };
        oval(x, y, rx, ry);
    };
    let solid_box = |l: f32, t: f32, r: f32, bottom: f32| {
        let (a, z) = (at(l, t), at(r, bottom));
        unsafe { rt.FillRectangle(&rect(RectF::new(a.x, a.y, z.x, z.y)), &b) };
    };
    match icon {
        Icon::Undo => {
            line(&[(9.0, 5.0), (4.0, 10.0), (9.0, 15.0)]);
            line(&[
                (4.0, 10.0),
                (15.0, 10.0),
                (18.0, 11.0),
                (20.0, 13.5),
                (20.0, 16.5),
                (18.0, 19.0),
                (15.0, 20.0),
                (8.0, 20.0),
            ]);
        }
        Icon::Redo => {
            line(&[(15.0, 5.0), (20.0, 10.0), (15.0, 15.0)]);
            line(&[
                (20.0, 10.0),
                (9.0, 10.0),
                (6.0, 11.0),
                (4.0, 13.5),
                (4.0, 16.5),
                (6.0, 19.0),
                (9.0, 20.0),
                (16.0, 20.0),
            ]);
        }
        Icon::Select => line(&[
            (6.0, 3.0),
            (6.0, 19.0),
            (10.5, 15.0),
            (13.5, 21.0),
            (16.0, 20.0),
            (13.0, 14.0),
            (19.0, 14.0),
            (6.0, 3.0),
        ]),
        Icon::Pen => {
            line(&[
                (4.0, 20.0),
                (5.0, 15.0),
                (16.0, 4.0),
                (20.0, 8.0),
                (9.0, 19.0),
                (4.0, 20.0),
            ]);
            line(&[(13.5, 6.5), (17.5, 10.5)]);
        }
        Icon::Rect => line(&[
            (4.0, 6.0),
            (20.0, 6.0),
            (20.0, 18.0),
            (4.0, 18.0),
            (4.0, 6.0),
        ]),
        Icon::RectFilled => {
            solid_box(4.0, 6.0, 20.0, 18.0);
            line(&[
                (4.0, 6.0),
                (20.0, 6.0),
                (20.0, 18.0),
                (4.0, 18.0),
                (4.0, 6.0),
            ]);
        }
        Icon::Circle => oval(12.0, 12.0, 8.0, 8.0),
        Icon::CircleFilled => solid_oval(12.0, 12.0, 8.0, 8.0),
        Icon::Ellipse => oval(12.0, 12.0, 9.5, 5.5),
        Icon::EllipseFilled => solid_oval(12.0, 12.0, 9.5, 5.5),
        Icon::Fill => {
            solid_box(4.0, 4.0, 12.0, 20.0);
            line(&[
                (4.0, 4.0),
                (20.0, 4.0),
                (20.0, 20.0),
                (4.0, 20.0),
                (4.0, 4.0),
            ]);
        }
        Icon::Text => {
            line(&[(5.0, 5.0), (19.0, 5.0)]);
            line(&[(12.0, 5.0), (12.0, 20.0)]);
            line(&[(9.0, 20.0), (15.0, 20.0)]);
        }
        Icon::Palette => {
            for (x, y, tint) in [
                (8.0, 8.0, Rgb::new(229, 57, 53)),
                (16.0, 8.0, Rgb::new(253, 216, 53)),
                (8.0, 16.0, Rgb::new(30, 136, 229)),
                (16.0, 16.0, Rgb::new(67, 160, 71)),
            ] {
                if let Ok(dot) = brush(rt, color(tint)) {
                    let ellipse = D2D1_ELLIPSE {
                        point: point(at(x, y)),
                        radiusX: 3.6 * unit,
                        radiusY: 3.6 * unit,
                    };
                    unsafe { rt.FillEllipse(&ellipse, &dot) };
                }
            }
        }
        Icon::Eyedropper => {
            line(&[
                (4.0, 20.0),
                (5.0, 16.0),
                (13.0, 8.0),
                (16.0, 11.0),
                (8.0, 19.0),
                (4.0, 20.0),
            ]);
            line(&[(11.0, 6.0), (18.0, 13.0)]);
            line(&[
                (14.5, 7.5),
                (17.5, 4.5),
                (19.5, 4.5),
                (19.5, 6.5),
                (16.5, 9.5),
            ]);
        }
        Icon::Minus => line(&[(6.0, 12.0), (18.0, 12.0)]),
        Icon::Plus => {
            line(&[(6.0, 12.0), (18.0, 12.0)]);
            line(&[(12.0, 6.0), (12.0, 18.0)]);
        }
        Icon::Copy => {
            line(&[
                (9.0, 9.0),
                (20.0, 9.0),
                (20.0, 20.0),
                (9.0, 20.0),
                (9.0, 9.0),
            ]);
            line(&[
                (15.0, 9.0),
                (15.0, 4.0),
                (4.0, 4.0),
                (4.0, 15.0),
                (9.0, 15.0),
            ]);
        }
        Icon::Save => {
            line(&[(12.0, 4.0), (12.0, 15.0)]);
            line(&[(7.0, 10.0), (12.0, 15.0), (17.0, 10.0)]);
            line(&[(4.0, 20.0), (20.0, 20.0)]);
        }
        Icon::Close => {
            line(&[(6.0, 6.0), (18.0, 18.0)]);
            line(&[(18.0, 6.0), (6.0, 18.0)]);
        }
    }
}
