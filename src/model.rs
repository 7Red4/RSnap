//! Annotation data and geometry. Independent of Windows APIs so it can be unit tested.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    /// Black or white, whichever stays readable on top of this color.
    pub fn contrast(self) -> Rgb {
        let luminance = 0.299 * self.r as f32 + 0.587 * self.g as f32 + 0.114 * self.b as f32;
        if luminance > 150.0 {
            Rgb::new(0, 0, 0)
        } else {
            Rgb::new(255, 255, 255)
        }
    }
}

/// Red, yellow, blue, green, black.
pub const PRESETS: [Rgb; 5] = [
    Rgb::new(229, 57, 53),
    Rgb::new(253, 216, 53),
    Rgb::new(30, 136, 229),
    Rgb::new(67, 160, 71),
    Rgb::new(0, 0, 0),
];

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
}

pub const fn pt(x: f32, y: f32) -> Pt {
    Pt { x, y }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct RectF {
    pub l: f32,
    pub t: f32,
    pub r: f32,
    pub b: f32,
}

impl RectF {
    pub const fn new(l: f32, t: f32, r: f32, b: f32) -> Self {
        Self { l, t, r, b }
    }

    pub fn xywh(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self::new(x, y, x + w, y + h)
    }

    pub fn from_points(a: Pt, b: Pt) -> Self {
        Self::new(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y))
    }

    pub fn w(&self) -> f32 {
        self.r - self.l
    }

    pub fn h(&self) -> f32 {
        self.b - self.t
    }

    pub fn inflate(&self, d: f32) -> Self {
        Self::new(self.l - d, self.t - d, self.r + d, self.b + d)
    }

    pub fn contains(&self, p: Pt) -> bool {
        p.x >= self.l && p.x <= self.r && p.y >= self.t && p.y <= self.b
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Pen(Vec<Pt>),
    Rect(RectF),
    Circle(RectF),
    Ellipse(RectF),
    Text(TextShape),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextShape {
    pub origin: Pt,
    pub text: String,
    pub font: String,
    pub weight: u32,
    pub size: f32,
    /// Laid-out width and height, refreshed by the editor whenever the text changes.
    pub measured: (f32, f32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub shape: Shape,
    pub color: Rgb,
    pub thickness: f32,
    /// Only meaningful for rectangles, circles and ellipses.
    pub filled: bool,
}

impl Object {
    pub fn is_text(&self) -> bool {
        matches!(self.shape, Shape::Text(_))
    }

    /// Whether this object can switch between solid and outlined.
    pub fn fillable(&self) -> bool {
        matches!(
            self.shape,
            Shape::Rect(_) | Shape::Circle(_) | Shape::Ellipse(_)
        )
    }

    /// The drawn stroke width. Text and solid shapes have no stroke, so thickness does not apply.
    pub fn stroke_width(&self) -> f32 {
        if self.is_text() || self.filled {
            0.0
        } else {
            self.thickness
        }
    }

    /// The geometric extent, excluding stroke width.
    pub fn geometry(&self) -> RectF {
        match &self.shape {
            Shape::Pen(points) => {
                let first = points.first().copied().unwrap_or_default();
                points
                    .iter()
                    .fold(RectF::new(first.x, first.y, first.x, first.y), |b, p| {
                        RectF::new(b.l.min(p.x), b.t.min(p.y), b.r.max(p.x), b.b.max(p.y))
                    })
            }
            Shape::Rect(r) | Shape::Circle(r) | Shape::Ellipse(r) => *r,
            Shape::Text(t) => RectF::xywh(t.origin.x, t.origin.y, t.measured.0, t.measured.1),
        }
    }

    /// The visible extent, including stroke width.
    pub fn bounds(&self) -> RectF {
        self.geometry().inflate(self.stroke_width() / 2.0)
    }

    /// Hits on what is visible: stroke, text or a solid shape, not the empty inside of an outline.
    pub fn hit(&self, p: Pt, tolerance: f32) -> bool {
        let reach = self.stroke_width() / 2.0 + tolerance;
        match &self.shape {
            Shape::Pen(points) => match points.as_slice() {
                [] => false,
                [only] => distance(*only, p) <= reach,
                many => many
                    .windows(2)
                    .any(|s| segment_distance(p, s[0], s[1]) <= reach),
            },
            Shape::Rect(r) => {
                let outer = r.inflate(reach);
                let inner = r.inflate(-reach);
                outer.contains(p)
                    && (self.filled || !(inner.w() > 0.0 && inner.h() > 0.0 && inner.contains(p)))
            }
            Shape::Circle(r) | Shape::Ellipse(r) => {
                (self.filled && ellipse_contains(*r, p)) || ellipse_distance(*r, p) <= reach
            }
            Shape::Text(_) => self.geometry().inflate(tolerance).contains(p),
        }
    }

    pub fn translate(&mut self, dx: f32, dy: f32) {
        let shift = |r: &mut RectF| *r = RectF::new(r.l + dx, r.t + dy, r.r + dx, r.b + dy);
        match &mut self.shape {
            Shape::Pen(points) => {
                for p in points {
                    p.x += dx;
                    p.y += dy;
                }
            }
            Shape::Rect(r) | Shape::Circle(r) | Shape::Ellipse(r) => shift(r),
            Shape::Text(t) => {
                t.origin.x += dx;
                t.origin.y += dy;
            }
        }
    }

    /// Fits the geometry into `to`, where `from` is its geometry before resizing.
    pub fn fit(&mut self, from: RectF, to: RectF) {
        match &mut self.shape {
            Shape::Pen(points) => {
                let sx = if from.w() > 0.0 {
                    to.w() / from.w()
                } else {
                    1.0
                };
                let sy = if from.h() > 0.0 {
                    to.h() / from.h()
                } else {
                    1.0
                };
                for p in points {
                    p.x = to.l + (p.x - from.l) * sx;
                    p.y = to.t + (p.y - from.t) * sy;
                }
            }
            Shape::Rect(r) | Shape::Circle(r) | Shape::Ellipse(r) => *r = to,
            Shape::Text(_) => {}
        }
    }
}

pub fn distance(a: Pt, b: Pt) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

fn segment_distance(p: Pt, a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length_sq = dx * dx + dy * dy;
    if length_sq == 0.0 {
        return distance(p, a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / length_sq).clamp(0.0, 1.0);
    distance(p, pt(a.x + t * dx, a.y + t * dy))
}

fn ellipse_contains(r: RectF, p: Pt) -> bool {
    let (rx, ry) = ((r.w() / 2.0).max(0.5), (r.h() / 2.0).max(0.5));
    let (x, y) = (p.x - (r.l + r.r) / 2.0, p.y - (r.t + r.b) / 2.0);
    x * x / (rx * rx) + y * y / (ry * ry) <= 1.0
}

/// First-order approximation of the distance to the ellipse outline inscribed in `r`.
fn ellipse_distance(r: RectF, p: Pt) -> f32 {
    let (rx, ry) = ((r.w() / 2.0).max(0.5), (r.h() / 2.0).max(0.5));
    let (x, y) = (p.x - (r.l + r.r) / 2.0, p.y - (r.t + r.b) / 2.0);
    let f = x * x / (rx * rx) + y * y / (ry * ry) - 1.0;
    let grad = 2.0 * (x * x / rx.powi(4) + y * y / ry.powi(4)).sqrt();
    if grad < 1e-6 {
        rx.min(ry)
    } else {
        f.abs() / grad
    }
}

/// The square with one corner at `anchor`, extending towards `p`.
pub fn square_from(anchor: Pt, p: Pt) -> RectF {
    let side = (p.x - anchor.x).abs().max((p.y - anchor.y).abs());
    let x = if p.x >= anchor.x {
        anchor.x + side
    } else {
        anchor.x - side
    };
    let y = if p.y >= anchor.y {
        anchor.y + side
    } else {
        anchor.y - side
    };
    RectF::from_points(anchor, pt(x, y))
}

/// Handle centers, clockwise from top-left: TL, T, TR, R, BR, B, BL, L.
pub fn handle_points(b: RectF) -> [Pt; 8] {
    let (cx, cy) = ((b.l + b.r) / 2.0, (b.t + b.b) / 2.0);
    [
        pt(b.l, b.t),
        pt(cx, b.t),
        pt(b.r, b.t),
        pt(b.r, cy),
        pt(b.r, b.b),
        pt(cx, b.b),
        pt(b.l, b.b),
        pt(b.l, cy),
    ]
}

/// The new bounds after dragging `handle` to `p`. Dragging past the opposite side mirrors the box.
pub fn resize_bounds(orig: RectF, handle: usize, p: Pt, lock_square: bool) -> RectF {
    let mut r = orig;
    match handle {
        0 => (r.l, r.t) = (p.x, p.y),
        1 => r.t = p.y,
        2 => (r.r, r.t) = (p.x, p.y),
        3 => r.r = p.x,
        4 => (r.r, r.b) = (p.x, p.y),
        5 => r.b = p.y,
        6 => (r.l, r.b) = (p.x, p.y),
        _ => r.l = p.x,
    }
    if !lock_square {
        return RectF::from_points(pt(r.l, r.t), pt(r.r, r.b));
    }
    let (cx, cy) = ((orig.l + orig.r) / 2.0, (orig.t + orig.b) / 2.0);
    match handle {
        0 => square_from(pt(orig.r, orig.b), p),
        2 => square_from(pt(orig.l, orig.b), p),
        4 => square_from(pt(orig.l, orig.t), p),
        6 => square_from(pt(orig.r, orig.t), p),
        1 | 5 => {
            let anchor = if handle == 1 { orig.b } else { orig.t };
            let side = (p.y - anchor).abs();
            RectF::new(
                cx - side / 2.0,
                anchor.min(p.y),
                cx + side / 2.0,
                anchor.max(p.y),
            )
        }
        _ => {
            let anchor = if handle == 3 { orig.l } else { orig.r };
            let side = (p.x - anchor).abs();
            RectF::new(
                anchor.min(p.x),
                cy - side / 2.0,
                anchor.max(p.x),
                cy + side / 2.0,
            )
        }
    }
}

#[derive(Default)]
pub struct History {
    past: Vec<Vec<Object>>,
    future: Vec<Vec<Object>>,
}

impl History {
    pub fn record(&mut self, before: Vec<Object>) {
        self.past.push(before);
        self.future.clear();
    }

    pub fn undo(&mut self, current: &mut Vec<Object>) -> bool {
        match self.past.pop() {
            Some(previous) => {
                self.future.push(std::mem::replace(current, previous));
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self, current: &mut Vec<Object>) -> bool {
        match self.future.pop() {
            Some(next) => {
                self.past.push(std::mem::replace(current, next));
                true
            }
            None => false,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(shape: Shape) -> Object {
        Object {
            shape,
            color: PRESETS[0],
            thickness: 4.0,
            filled: false,
        }
    }

    fn solid(shape: Shape) -> Object {
        Object {
            filled: true,
            ..object(shape)
        }
    }

    #[test]
    fn solid_shapes_hit_inside_but_not_outside() {
        let rect = solid(Shape::Rect(RectF::new(0.0, 0.0, 100.0, 100.0)));
        assert!(rect.hit(pt(50.0, 50.0), 2.0));
        assert!(!rect.hit(pt(120.0, 50.0), 2.0));
        let ellipse = solid(Shape::Ellipse(RectF::new(0.0, 0.0, 200.0, 100.0)));
        assert!(ellipse.hit(pt(100.0, 50.0), 2.0));
        assert!(!ellipse.hit(pt(5.0, 5.0), 2.0));
    }

    #[test]
    fn thickness_does_not_affect_solid_shapes() {
        let geometry = RectF::new(10.0, 10.0, 50.0, 50.0);
        let mut thin = solid(Shape::Rect(geometry));
        thin.thickness = 1.0;
        let mut thick = thin.clone();
        thick.thickness = 40.0;
        assert_eq!(thick.bounds(), geometry);
        assert_eq!(thin.bounds(), thick.bounds());
        assert_eq!(thin.hit(pt(5.0, 30.0), 2.0), thick.hit(pt(5.0, 30.0), 2.0));
    }

    #[test]
    fn only_rects_and_ellipses_are_fillable() {
        let r = RectF::new(0.0, 0.0, 1.0, 1.0);
        assert!(object(Shape::Rect(r)).fillable());
        assert!(object(Shape::Circle(r)).fillable());
        assert!(object(Shape::Ellipse(r)).fillable());
        assert!(!object(Shape::Pen(vec![pt(0.0, 0.0)])).fillable());
    }

    #[test]
    fn hex_is_uppercase_with_hash() {
        assert_eq!(Rgb::new(255, 10, 171).hex(), "#FF0AAB");
    }

    #[test]
    fn contrast_stays_readable_on_extremes() {
        assert_eq!(PRESETS[1].contrast(), Rgb::new(0, 0, 0));
        assert_eq!(Rgb::new(0, 0, 0).contrast(), Rgb::new(255, 255, 255));
        assert_eq!(Rgb::new(255, 255, 255).contrast(), Rgb::new(0, 0, 0));
    }

    #[test]
    fn rect_hits_outline_but_not_inside() {
        let o = object(Shape::Rect(RectF::new(0.0, 0.0, 100.0, 100.0)));
        assert!(o.hit(pt(0.0, 50.0), 2.0));
        assert!(o.hit(pt(50.0, 103.0), 2.0));
        assert!(!o.hit(pt(50.0, 50.0), 2.0));
        assert!(!o.hit(pt(120.0, 50.0), 2.0));
    }

    #[test]
    fn ellipse_hits_outline_but_not_center() {
        let o = object(Shape::Ellipse(RectF::new(0.0, 0.0, 200.0, 100.0)));
        assert!(o.hit(pt(0.0, 50.0), 2.0));
        assert!(o.hit(pt(100.0, 0.0), 2.0));
        assert!(!o.hit(pt(100.0, 50.0), 2.0));
        assert!(!o.hit(pt(5.0, 5.0), 2.0));
    }

    #[test]
    fn pen_hits_near_stroke_only() {
        let o = object(Shape::Pen(vec![pt(0.0, 0.0), pt(100.0, 0.0)]));
        assert!(o.hit(pt(50.0, 3.0), 2.0));
        assert!(!o.hit(pt(50.0, 10.0), 2.0));
        let dot = object(Shape::Pen(vec![pt(10.0, 10.0)]));
        assert!(dot.hit(pt(11.0, 11.0), 2.0));
    }

    #[test]
    fn square_follows_drag_direction() {
        assert_eq!(
            square_from(pt(10.0, 10.0), pt(30.0, 15.0)),
            RectF::new(10.0, 10.0, 30.0, 30.0)
        );
        assert_eq!(
            square_from(pt(10.0, 10.0), pt(0.0, -20.0)),
            RectF::new(-20.0, -20.0, 10.0, 10.0)
        );
    }

    #[test]
    fn resize_mirrors_past_opposite_side() {
        let r = RectF::new(10.0, 10.0, 20.0, 20.0);
        assert_eq!(
            resize_bounds(r, 3, pt(5.0, 0.0), false),
            RectF::new(5.0, 10.0, 10.0, 20.0)
        );
    }

    #[test]
    fn locked_resize_stays_square() {
        let r = RectF::new(0.0, 0.0, 10.0, 10.0);
        for handle in 0..8 {
            let out = resize_bounds(r, handle, pt(40.0, 25.0), true);
            assert!(
                (out.w() - out.h()).abs() < 0.001,
                "handle {handle}: {out:?}"
            );
        }
    }

    #[test]
    fn fit_scales_pen_points() {
        let mut o = object(Shape::Pen(vec![pt(0.0, 0.0), pt(10.0, 10.0)]));
        o.fit(
            RectF::new(0.0, 0.0, 10.0, 10.0),
            RectF::new(10.0, 10.0, 30.0, 50.0),
        );
        assert_eq!(o.shape, Shape::Pen(vec![pt(10.0, 10.0), pt(30.0, 50.0)]));
    }

    #[test]
    fn fit_survives_a_perfectly_straight_stroke() {
        let mut o = object(Shape::Pen(vec![pt(0.0, 5.0), pt(10.0, 5.0)]));
        o.fit(
            RectF::new(0.0, 5.0, 10.0, 5.0),
            RectF::new(0.0, 5.0, 20.0, 5.0),
        );
        assert_eq!(o.shape, Shape::Pen(vec![pt(0.0, 5.0), pt(20.0, 5.0)]));
    }

    #[test]
    fn undo_and_redo_round_trip() {
        let first = object(Shape::Rect(RectF::new(0.0, 0.0, 1.0, 1.0)));
        let mut current = vec![first.clone()];
        let mut history = History::default();
        history.record(Vec::new());
        assert!(history.undo(&mut current));
        assert!(current.is_empty());
        assert!(history.redo(&mut current));
        assert_eq!(current, vec![first]);
        assert!(!history.can_redo());
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut current = Vec::new();
        let mut history = History::default();
        history.record(Vec::new());
        history.undo(&mut current);
        history.record(Vec::new());
        assert!(!history.can_redo());
    }
}
