//! Display insets and the pictures drawn in them. A [`Screen`] is a clipped
//! area of a panel; the primitives draw plain numbers (levels, curves,
//! histories, positions) in flat line art: they are what a node plugin's
//! [`audioboard_node::Display`] is drawn with.

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Align2, Color32, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

use crate::theme::{self, alpha};
use crate::widgets::{MeterState, hmeter};

/// The dark inset a display sits in, `rect` on screen.
pub fn inset(p: &Painter, rect: Rect, z: f32) {
    p.rect_filled(rect, 4.0 * z, theme::INSET);
}

/// A display area: clipped, padded, in the node's colour.
pub struct Screen<'a> {
    pub p: &'a Painter,
    pub rect: Rect,
    pub accent: Color32,
    pub z: f32,
}

/// Draws the inset at `rect` (screen space) and returns a painter clipped
/// to it with the padded area inside: what a [`Screen`] is made of.
pub fn prepare(painter: &Painter, rect: Rect, z: f32) -> (Painter, Rect) {
    inset(painter, rect, z);
    let clipped = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    (clipped, rect.shrink2(vec2(6.0 * z, 5.0 * z)))
}

impl Screen<'_> {
    /// x for `t` (0…1) across the area.
    pub fn x(&self, t: f32) -> f32 {
        self.rect.left() + self.rect.width() * t
    }

    /// y for a bipolar value (−1 bottom … 1 top).
    pub fn y_bipolar(&self, v: f32) -> f32 {
        self.rect.center().y - v.clamp(-1.0, 1.0) * self.rect.height() * 0.5
    }

    /// y for a unipolar value (0 bottom … 1 top).
    pub fn y_unit(&self, v: f32) -> f32 {
        self.rect.bottom() - v.clamp(0.0, 1.0) * self.rect.height()
    }

    /// A faint horizontal guide at `y` (screen space).
    pub fn guide(&self, y: f32) {
        self.p.line_segment([pos2(self.rect.left(), y), pos2(self.rect.right(), y)], Stroke::new(1.0 * self.z, theme::CONTROL));
    }

    /// Vertical line from the baseline, `amp` (0…1) of the height.
    pub fn partial(&self, x: f32, amp: f32, color: Color32) {
        let r = self.rect;
        self.p.line_segment(
            [pos2(x, r.bottom()), pos2(x, r.bottom() - r.height() * amp.clamp(0.0, 1.0))],
            Stroke::new(1.3 * self.z, color),
        );
    }

    /// Polyline with a soft fill down to the baseline.
    pub fn area(&self, pts: Vec<Pos2>) {
        let base = self.rect.bottom();
        let mut mesh = Mesh::default();
        let fill = alpha(self.accent, 0.18);
        for (i, pt) in pts.iter().enumerate() {
            mesh.vertices.push(Vertex { pos: *pt, uv: WHITE_UV, color: fill });
            mesh.vertices.push(Vertex { pos: pos2(pt.x, base), uv: WHITE_UV, color: Color32::TRANSPARENT });
            if i > 0 {
                let k = (i * 2) as u32;
                mesh.add_triangle(k - 2, k - 1, k);
                mesh.add_triangle(k - 1, k + 1, k);
            }
        }
        self.p.add(Shape::mesh(mesh));
        self.p.add(Shape::line(pts, Stroke::new(1.5 * self.z, self.accent)));
    }

    /// Vertical bars for `values` (0…1 each), left to right.
    pub fn bars(&self, values: impl ExactSizeIterator<Item = f32>) {
        let n = values.len().max(1);
        let w = self.rect.width() / n as f32;
        let gap = (w * 0.25).min(2.0 * self.z);
        for (i, v) in values.enumerate() {
            let x0 = self.rect.left() + w * i as f32 + gap * 0.5;
            let top = self.y_unit(v);
            let bar = Rect::from_min_max(pos2(x0, top), pos2(x0 + w - gap, self.rect.bottom()));
            self.p.rect_filled(bar, 1.0 * self.z, alpha(self.accent, 0.25));
            self.p.line_segment([pos2(bar.left(), top), pos2(bar.right(), top)], Stroke::new(1.5 * self.z, self.accent));
        }
    }

    /// A line through `values` (−1…1 each), left to right, over a centre
    /// guide.
    pub fn curve(&self, values: impl ExactSizeIterator<Item = f32>) {
        let n = values.len();
        self.guide(self.rect.center().y);
        if n < 2 {
            return;
        }
        let pts: Vec<Pos2> = values.enumerate().map(|(i, v)| pos2(self.x(i as f32 / (n - 1) as f32), self.y_bipolar(v))).collect();
        self.p.add(Shape::line(pts, Stroke::new(1.5 * self.z, self.accent)));
    }

    /// A history (oldest first, −1…1), scrolling in from the right, with
    /// the latest value marked.
    pub fn trace(&self, history: impl ExactSizeIterator<Item = f32>, capacity: usize) {
        self.guide(self.rect.center().y);
        let n = history.len();
        if n < 2 {
            return;
        }
        let cap = capacity.max(n).max(2);
        let pts: Vec<Pos2> = history
            .enumerate()
            .map(|(i, v)| pos2(self.x((cap - n + i) as f32 / (cap - 1) as f32), self.y_bipolar(v)))
            .collect();
        let last = *pts.last().expect("two points or more");
        self.p.add(Shape::line(pts, Stroke::new(1.5 * self.z, self.accent)));
        self.p.circle_filled(last, 2.5 * self.z, self.accent);
    }

    /// A dot at (x, y), 0…1 each (y up), with cross-hairs.
    pub fn dot(&self, x: f32, y: f32) {
        let c = pos2(self.x(x.clamp(0.0, 1.0)), self.y_unit(y));
        let s = Stroke::new(1.0 * self.z, alpha(self.accent, 0.35));
        self.p.line_segment([pos2(self.rect.left(), c.y), pos2(self.rect.right(), c.y)], s);
        self.p.line_segment([pos2(c.x, self.rect.top()), pos2(c.x, self.rect.bottom())], s);
        self.p.circle_stroke(c, 4.0 * self.z, Stroke::new(1.5 * self.z, self.accent));
    }

    /// The output level, as a horizontal meter across the middle.
    pub fn level(&self, meter: &MeterState) {
        let h = (6.0 * self.z).min(self.rect.height());
        let c = self.rect.center().y;
        hmeter(self.p, Rect::from_x_y_ranges(self.rect.x_range(), c - h * 0.5..=c + h * 0.5), meter, self.z);
    }

    /// A short message in the middle (a node that stopped, an empty state).
    pub fn message(&self, text: &str, color: Color32) {
        let font = theme::font(9.5 * self.z);
        if font.size >= crate::view::MIN_TEXT {
            self.p.text(self.rect.center(), Align2::CENTER_CENTER, text, font, color);
        }
    }
}
