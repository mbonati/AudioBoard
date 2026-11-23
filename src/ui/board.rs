//! The board's look on the canvas (see `crate::board` for what is saved):
//! - the **background**: a preset (colour and grid: lines, dots or none) or
//!   an image filling the view, veiled by the preset's colour;
//! - **stickers**: drawn behind modules, cables and nodes; dragged to move,
//!   with handles to resize and rotate the selected one; a menu to restack,
//!   rotate, duplicate, lock and delete; images dropped from the Finder;
//! - the **built-in designs**, flat line art on a dark badge;
//! - the canvas menu's **Stickers** and **Background** submenus.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use egui::{
    Align2, Color32, CursorIcon, FontId, Id, Mesh, Painter, Pos2, Rect, Response, RichText, Sense,
    Shape, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2,
};

use super::theme::{self, alpha};
use crate::audio::engine::NodeId;
use crate::board::{Art, BACKGROUND_MAX_PX, Board, STICKER_MAX_PX, STICKER_WIDTH, WIDTH_RANGE};

/// The canvas grid's step, in canvas units.
const GRID: f32 = 24.0;

/// How the canvas maps canvas units to the screen.
#[derive(Clone, Copy, Debug)]
pub struct View {
    /// Where canvas (0, 0) is on screen.
    pub origin: Pos2,
    pub zoom: f32,
}

impl View {
    pub fn to_screen(&self, w: Pos2) -> Pos2 {
        self.origin + w.to_vec2() * self.zoom
    }

    pub fn to_world(&self, s: Pos2) -> Pos2 {
        ((s - self.origin) / self.zoom).to_pos2()
    }
}

// ---------------------------------------------------------------------------
// Backgrounds
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grid {
    Lines,
    Dots,
    None,
}

/// A background preset: its colour and grid.
pub struct Preset {
    pub id: &'static str,
    pub label: &'static str,
    pub fill: Color32,
    pub grid: Grid,
    pub minor: Color32,
    pub major: Color32,
}

pub const PRESETS: &[Preset] = &[
    Preset {
        id: "graphite",
        label: "Graphite",
        fill: theme::bg(),
        grid: Grid::Lines,
        minor: theme::grid_minor(),
        major: theme::grid_major(),
    },
    Preset {
        id: "dots",
        label: "Dots",
        fill: theme::bg(),
        grid: Grid::Dots,
        minor: Color32::from_rgb(40, 42, 48),
        major: Color32::from_rgb(58, 61, 69),
    },
    Preset {
        id: "blueprint",
        label: "Blueprint",
        fill: Color32::from_rgb(14, 27, 46),
        grid: Grid::Lines,
        minor: Color32::from_rgb(21, 37, 61),
        major: Color32::from_rgb(31, 52, 84),
    },
    Preset {
        id: "night",
        label: "Night",
        fill: Color32::from_rgb(9, 10, 13),
        grid: Grid::Dots,
        minor: Color32::from_rgb(24, 26, 31),
        major: Color32::from_rgb(38, 41, 49),
    },
    Preset {
        id: "moss",
        label: "Moss",
        fill: Color32::from_rgb(17, 25, 21),
        grid: Grid::Lines,
        minor: Color32::from_rgb(23, 33, 28),
        major: Color32::from_rgb(31, 44, 37),
    },
    Preset {
        id: "ember",
        label: "Ember",
        fill: Color32::from_rgb(27, 19, 18),
        grid: Grid::Dots,
        minor: Color32::from_rgb(45, 31, 29),
        major: Color32::from_rgb(64, 43, 39),
    },
    Preset {
        id: "plum",
        label: "Plum",
        fill: Color32::from_rgb(23, 18, 30),
        grid: Grid::Lines,
        minor: Color32::from_rgb(30, 24, 39),
        major: Color32::from_rgb(40, 32, 52),
    },
    Preset {
        id: "plain",
        label: "Plain",
        fill: theme::bg(),
        grid: Grid::None,
        minor: theme::bg(),
        major: theme::bg(),
    },
];

/// A preset by id (the default for one that isn't known).
pub fn preset(id: &str) -> &'static Preset {
    PRESETS.iter().find(|p| p.id == id).unwrap_or(&PRESETS[0])
}

/// The grid, in screen space: lines or dots every `GRID` units, every
/// fifth one stronger; zoomed far out, only the strong ones.
fn draw_grid(p: &Painter, rect: Rect, view: View, style: Grid, minor: Color32, major: Color32) {
    if style == Grid::None {
        return;
    }
    let mut step = GRID * view.zoom;
    let mut major_every = 5;
    if step < 10.0 {
        step *= 5.0;
        major_every = 1;
    }
    let ppp = p.ctx().pixels_per_point();
    let snap = |v: f32| (v * ppp).round() / ppp;
    let first = |o: f32, lo: f32| lo + (o - lo).rem_euclid(step);
    let strong = |v: f32, o: f32| ((v - o) / step).round() as i64 % major_every == 0;
    let mut mesh = Mesh::default();
    match style {
        Grid::Lines => {
            let mut x = first(view.origin.x, rect.left());
            while x < rect.right() {
                let x0 = snap(x);
                let c = if strong(x, view.origin.x) {
                    major
                } else {
                    minor
                };
                mesh.add_colored_rect(Rect::from_x_y_ranges(x0..=x0 + 1.0, rect.y_range()), c);
                x += step;
            }
            let mut y = first(view.origin.y, rect.top());
            while y < rect.bottom() {
                let y0 = snap(y);
                let c = if strong(y, view.origin.y) {
                    major
                } else {
                    minor
                };
                mesh.add_colored_rect(Rect::from_x_y_ranges(rect.x_range(), y0..=y0 + 1.0), c);
                y += step;
            }
        }
        Grid::Dots => {
            // Dots on every crossing: a small square, bigger on the strong ones.
            let mut x = first(view.origin.x, rect.left());
            while x < rect.right() {
                let mut y = first(view.origin.y, rect.top());
                let sx = strong(x, view.origin.x);
                while y < rect.bottom() {
                    let on = sx && strong(y, view.origin.y);
                    let side = if on { 3.0 } else { 2.0 };
                    mesh.add_colored_rect(
                        Rect::from_center_size(pos2(snap(x), snap(y)), vec2(side, side)),
                        if on { major } else { minor },
                    );
                    y += step;
                }
                x += step;
            }
        }
        Grid::None => {}
    }
    p.add(Shape::mesh(mesh));
}

// ---------------------------------------------------------------------------
// Built-in stickers
// ---------------------------------------------------------------------------

/// The built-in designs: (id, label). Text badges come last.
pub const BUILTINS: &[(&str, &str)] = &[
    ("wave", "Wave"),
    ("moon", "Moon"),
    ("cassette", "Cassette"),
    ("knob", "Knob"),
    ("bolt", "Bolt"),
    ("mushroom", "Mushroom"),
    ("planet", "Planet"),
    ("heart", "Heart"),
    ("sparkle", "Sparkle"),
    ("headphones", "Headphones"),
    ("speaker", "Speaker"),
    ("leaf", "Leaf"),
    ("sun", "Sun"),
    ("eye", "Eye"),
    ("tag-ambient", "AMBIENT"),
    ("tag-drone", "DRONE"),
    ("tag-loop", "LOOP"),
    ("tag-live", "LIVE"),
    ("tag-wip", "WIP"),
    ("tag-noinput", "NO INPUT"),
];

/// Example sticker files (SVG, in `assets/stickers`), added as images.
pub const EXAMPLES: &[(&str, &[u8])] = &[
    ("Vinyl", include_bytes!("../../assets/stickers/vinyl.svg")),
    ("Synth", include_bytes!("../../assets/stickers/synth.svg")),
    (
        "Rain cloud",
        include_bytes!("../../assets/stickers/cloud.svg"),
    ),
    (
        "Patch cable",
        include_bytes!("../../assets/stickers/cable.svg"),
    ),
];

/// Height over width of a design.
pub fn builtin_aspect(name: &str) -> f32 {
    match name {
        n if n.starts_with("tag-") => 0.34,
        "cassette" => 0.66,
        _ => 1.0,
    }
}

fn builtin_width(name: &str) -> f32 {
    if name.starts_with("tag-") {
        190.0
    } else {
        STICKER_WIDTH
    }
}

/// Draws in a sticker's own space: (−0.5, −0.5) is its top-left corner,
/// (0.5, 0.5) its bottom-right, whatever its size and rotation. Lengths
/// that aren't positions (radii, strokes) are in units of its shorter side.
struct Pen<'a> {
    p: &'a Painter,
    c: Pos2,
    x: Vec2,
    y: Vec2,
    s: f32,
    angle: f32,
}

impl<'a> Pen<'a> {
    fn new(p: &'a Painter, c: Pos2, size: Vec2, degrees: f32) -> Self {
        let angle = degrees.to_radians();
        let (sin, cos) = angle.sin_cos();
        Self {
            p,
            c,
            x: vec2(cos, sin) * size.x,
            y: vec2(-sin, cos) * size.y,
            s: size.x.min(size.y),
            angle,
        }
    }

    fn at(&self, u: f32, v: f32) -> Pos2 {
        self.c + self.x * u + self.y * v
    }

    /// A point at `r` (in short-side units) from (u, v), `deg` clockwise
    /// from straight up.
    fn polar(&self, u: f32, v: f32, r: f32, deg: f32) -> (f32, f32) {
        let a = deg.to_radians();
        let k = self.s / self.x.length().max(1e-3);
        let l = self.s / self.y.length().max(1e-3);
        (u + r * a.sin() * k, v - r * a.cos() * l)
    }

    fn stroke(&self, w: f32, col: Color32) -> Stroke {
        Stroke::new((w * self.s).max(0.8), col)
    }

    fn line(&self, pts: &[(f32, f32)], w: f32, col: Color32) {
        let stroke = self.stroke(w, col);
        self.p.add(Shape::line(
            blunt(
                pts.iter().map(|&(u, v)| self.at(u, v)).collect(),
                false,
                stroke.width,
            ),
            stroke,
        ));
    }

    fn closed(&self, pts: &[(f32, f32)], w: f32, col: Color32) {
        let stroke = self.stroke(w, col);
        self.p.add(Shape::closed_line(
            blunt(
                pts.iter().map(|&(u, v)| self.at(u, v)).collect(),
                true,
                stroke.width,
            ),
            stroke,
        ));
    }

    /// A convex shape, filled and outlined.
    fn convex(&self, pts: &[(f32, f32)], fill: Color32, w: f32, col: Color32) {
        self.p.add(Shape::convex_polygon(
            pts.iter().map(|&(u, v)| self.at(u, v)).collect(),
            fill,
            self.stroke(w, col),
        ));
    }

    fn circle(&self, u: f32, v: f32, r: f32, fill: Color32, w: f32, col: Color32) {
        let stroke = if w > 0.0 {
            self.stroke(w, col)
        } else {
            Stroke::NONE
        };
        self.p.circle(self.at(u, v), r * self.s, fill, stroke);
    }

    /// Points of an arc around (u, v), radius `r`, from `a0` to `a1` degrees
    /// (clockwise from up).
    fn arc(&self, u: f32, v: f32, r: f32, a0: f32, a1: f32) -> Vec<(f32, f32)> {
        let n = ((a1 - a0).abs() / 6.0).ceil().max(4.0) as usize;
        (0..=n)
            .map(|k| self.polar(u, v, r, a0 + (a1 - a0) * k as f32 / n as f32))
            .collect()
    }

    /// A rectangle with rounded corners, (u0, v0)–(u1, v1), corner radius
    /// `r` in short-side units.
    fn rounded(&self, u0: f32, v0: f32, u1: f32, v1: f32, r: f32) -> Vec<(f32, f32)> {
        let k = self.s / self.x.length().max(1e-3);
        let l = self.s / self.y.length().max(1e-3);
        let (ru, rv) = ((r * k).min((u1 - u0) / 2.0), (r * l).min((v1 - v0) / 2.0));
        let mut pts = Vec::new();
        for (cu, cv, a0) in [
            (u1 - ru, v0 + rv, 0.0f32),
            (u1 - ru, v1 - rv, 90.0),
            (u0 + ru, v1 - rv, 180.0),
            (u0 + ru, v0 + rv, 270.0),
        ] {
            for j in 0..=6 {
                let a = (a0 + 90.0 * j as f32 / 6.0).to_radians();
                pts.push((cu + ru * a.sin(), cv - rv * a.cos()));
            }
        }
        pts
    }

    /// Text centred on (u, v), `size` tall at most and no wider than
    /// `max_w` (in width units).
    fn text(&self, u: f32, v: f32, text: &str, size: f32, max_w: f32, col: Color32) {
        let mut galley = self
            .p
            .layout_no_wrap(text.to_owned(), theme::strong(size * self.s), col);
        let room = max_w * self.x.length();
        if galley.size().x > room && room > 0.0 {
            galley = self.p.layout_no_wrap(
                text.to_owned(),
                theme::strong(size * self.s * room / galley.size().x),
                col,
            );
        }
        let pos = self.at(u, v) - galley.size() * 0.5;
        self.p.add(
            egui::epaint::TextShape::new(pos, galley, col)
                .with_angle_and_anchor(self.angle, Align2::CENTER_CENTER),
        );
    }
}

/// Fill behind the line art: dark, so the grid doesn't show through.
const BADGE: Color32 = Color32::from_rgb(29, 30, 35);

fn accent(i: usize) -> Color32 {
    theme::osc_accents()[i % theme::osc_accents().len()]
}

/// Draws a built-in design filling `size` around `c`, turned `degrees`.
pub fn draw_builtin(p: &Painter, c: Pos2, size: Vec2, degrees: f32, name: &str) {
    let pen = Pen::new(p, c, size, degrees);
    let (blue, rose, green, violet, orange, teal) = (
        accent(0),
        accent(1),
        accent(2),
        accent(3),
        accent(4),
        accent(5),
    );
    let yellow = theme::solo();
    let ink = theme::text();
    let badge = |col: Color32| pen.circle(0.0, 0.0, 0.47, BADGE, 0.035, col);
    let w = 0.045;
    match name {
        "wave" => {
            badge(blue);
            let wave = |phase: f32, amp: f32| -> Vec<(f32, f32)> {
                (0..=48)
                    .map(|k| {
                        let u = -0.3 + 0.6 * k as f32 / 48.0;
                        (u, amp * (u / 0.3 * std::f32::consts::TAU + phase).sin())
                    })
                    .collect()
            };
            pen.line(&wave(0.9, 0.09), 0.03, alpha(blue, 0.45));
            pen.line(&wave(0.0, 0.15), w, blue);
        }
        "moon" => {
            badge(violet);
            // A crescent: the outer circle where the bite doesn't reach,
            // then the bite's edge inside it, back to the start.
            let (r1, (bu, bv), r2) = (0.27f32, (0.12f32, -0.08f32), 0.22f32);
            let in_bite = |q: (f32, f32)| {
                let k = pen.s / pen.x.length().max(1e-3);
                let l = pen.s / pen.y.length().max(1e-3);
                ((q.0 - bu) / k).powi(2) + ((q.1 - bv) / l).powi(2) < r2 * r2
            };
            let in_moon = |q: (f32, f32)| {
                let k = pen.s / pen.x.length().max(1e-3);
                let l = pen.s / pen.y.length().max(1e-3);
                (q.0 / k).powi(2) + (q.1 / l).powi(2) < r1 * r1
            };
            let deg = |u: f32, v: f32| u.atan2(-v).to_degrees();
            // Start each circle opposite the part that is cut away, so what
            // is kept is one run.
            let bite = deg(bu, bv);
            let outer: Vec<(f32, f32)> = pen
                .arc(0.0, 0.0, r1, bite, bite + 360.0)
                .into_iter()
                .filter(|q| !in_bite(*q))
                .collect();
            let toward = deg(-bu, -bv);
            let mut inner: Vec<(f32, f32)> = pen
                .arc(bu, bv, r2, toward + 180.0, toward + 540.0)
                .into_iter()
                .filter(|q| in_moon(*q))
                .collect();
            if let (Some(last), Some(first), Some(end)) =
                (outer.last(), inner.first(), inner.last())
            {
                let d = |a: &(f32, f32), b: &(f32, f32)| (a.0 - b.0).powi(2) + (a.1 - b.1).powi(2);
                if d(last, end) < d(last, first) {
                    inner.reverse();
                }
            }
            let mut crescent = outer;
            crescent.extend(inner);
            pen.closed(&crescent, w, violet);
            for (u, v, r) in [(0.22, -0.2, 0.07), (0.3, 0.06, 0.045), (0.12, 0.27, 0.04)] {
                sparkle(&pen, u, v, r, 0.025, yellow);
            }
        }
        "cassette" => {
            let body = pen.rounded(-0.48, -0.47, 0.48, 0.47, 0.06);
            pen.convex(&body, BADGE, 0.035, orange);
            pen.closed(
                &pen.rounded(-0.38, -0.36, 0.38, 0.16, 0.03),
                0.025,
                alpha(orange, 0.7),
            );
            pen.line(&[(-0.3, -0.24), (0.3, -0.24)], 0.02, alpha(ink, 0.5));
            for u in [-0.19, 0.19] {
                pen.circle(u, -0.02, 0.13, BADGE, 0.035, orange);
                for k in 0..3 {
                    let a = 30.0 + 120.0 * k as f32;
                    let (pu, pv) = pen.polar(u, -0.02, 0.08, a);
                    pen.line(&[(u, -0.02), (pu, pv)], 0.025, orange);
                }
            }
            pen.line(&[(-0.06, -0.02), (0.06, -0.02)], 0.02, alpha(orange, 0.6));
            pen.closed(
                &[(-0.28, 0.47), (-0.2, 0.27), (0.2, 0.27), (0.28, 0.47)],
                0.03,
                orange,
            );
            for u in [-0.1, 0.1] {
                pen.circle(u, 0.37, 0.025, BADGE, 0.02, orange);
            }
        }
        "knob" => {
            badge(teal);
            for k in 0..11 {
                let a = -135.0 + 27.0 * k as f32;
                let (o, i) = (
                    pen.polar(0.0, 0.0, 0.41, a),
                    pen.polar(0.0, 0.0, if k % 5 == 0 { 0.33 } else { 0.36 }, a),
                );
                pen.line(&[i, o], 0.025, alpha(ink, 0.6));
            }
            pen.line(&pen.arc(0.0, 0.0, 0.3, -135.0, 45.0), 0.04, orange);
            pen.circle(0.0, 0.0, 0.22, BADGE, w, teal);
            let (pu, pv) = pen.polar(0.0, 0.0, 0.17, 45.0);
            let (qu, qv) = pen.polar(0.0, 0.0, 0.05, 45.0);
            pen.line(&[(qu, qv), (pu, pv)], w, teal);
        }
        "bolt" => {
            badge(yellow);
            pen.closed(
                &[
                    (0.07, -0.36),
                    (-0.17, 0.05),
                    (-0.01, 0.05),
                    (-0.07, 0.36),
                    (0.18, -0.07),
                    (0.02, -0.07),
                ],
                w,
                yellow,
            );
        }
        "mushroom" => {
            badge(rose);
            let mut cap = pen.arc(0.0, 0.04, 0.3, -90.0, 90.0);
            cap.push((0.3, 0.04));
            pen.convex(&cap, BADGE, w, rose);
            for (u, v, r) in [
                (-0.14, -0.08, 0.045),
                (0.06, -0.16, 0.05),
                (0.17, -0.03, 0.035),
                (-0.02, -0.03, 0.03),
            ] {
                pen.circle(u, v, r, Color32::TRANSPARENT, 0.025, ink);
            }
            pen.closed(
                &[(-0.08, 0.04), (0.08, 0.04), (0.1, 0.27), (-0.1, 0.27)],
                w,
                rose,
            );
            pen.line(&[(-0.24, 0.31), (0.24, 0.31)], 0.03, green);
        }
        "planet" => {
            badge(green);
            let ring = |a0: f32, a1: f32| -> Vec<(f32, f32)> {
                let tilt = (-18.0f32).to_radians();
                (0..=40)
                    .map(|k| {
                        let t = (a0 + (a1 - a0) * k as f32 / 40.0).to_radians();
                        let (x, y) = (0.4 * t.cos(), 0.11 * t.sin());
                        (
                            x * tilt.cos() - y * tilt.sin(),
                            x * tilt.sin() + y * tilt.cos(),
                        )
                    })
                    .collect()
            };
            pen.line(&ring(180.0, 360.0), 0.035, alpha(orange, 0.7));
            pen.circle(0.0, 0.0, 0.21, BADGE, w, green);
            pen.line(
                &pen.arc(0.0, 0.0, 0.14, 200.0, 280.0),
                0.025,
                alpha(green, 0.6),
            );
            pen.line(&ring(0.0, 180.0), 0.035, orange);
            pen.circle(0.29, -0.29, 0.035, Color32::TRANSPARENT, 0.025, ink);
        }
        "heart" => {
            badge(rose);
            let pts: Vec<(f32, f32)> = (0..64)
                .map(|k| {
                    let t = std::f32::consts::TAU * k as f32 / 64.0;
                    let x = 16.0 * t.sin().powi(3);
                    let y = 13.0 * t.cos()
                        - 5.0 * (2.0 * t).cos()
                        - 2.0 * (3.0 * t).cos()
                        - (4.0 * t).cos();
                    (x / 16.0 * 0.28, -y / 16.0 * 0.28 + 0.02)
                })
                .collect();
            pen.closed(&pts, w, rose);
            pen.line(
                &pen.arc(-0.12, -0.1, 0.07, 280.0, 340.0),
                0.03,
                alpha(ink, 0.6),
            );
        }
        "sparkle" => {
            badge(yellow);
            sparkle(&pen, 0.0, 0.02, 0.3, w, yellow);
            sparkle(&pen, 0.24, -0.24, 0.09, 0.03, teal);
            sparkle(&pen, -0.25, 0.22, 0.07, 0.03, rose);
        }
        "headphones" => {
            badge(blue);
            pen.line(&pen.arc(0.0, 0.06, 0.27, -100.0, 100.0), w, blue);
            for (u0, u1) in [(-0.34, -0.19), (0.19, 0.34)] {
                pen.convex(&pen.rounded(u0, 0.02, u1, 0.3, 0.05), BADGE, w, blue);
            }
            pen.line(
                &pen.arc(0.0, 0.16, 0.08, -60.0, 60.0),
                0.025,
                alpha(teal, 0.8),
            );
            pen.line(
                &pen.arc(0.0, 0.16, 0.13, -50.0, 50.0),
                0.025,
                alpha(teal, 0.5),
            );
        }
        "speaker" => {
            pen.convex(
                &pen.rounded(-0.47, -0.47, 0.47, 0.47, 0.12),
                BADGE,
                0.035,
                orange,
            );
            pen.closed(&pen.rounded(-0.27, -0.37, 0.27, 0.37, 0.05), w, orange);
            pen.circle(0.0, -0.19, 0.07, Color32::TRANSPARENT, 0.035, orange);
            pen.circle(0.0, 0.12, 0.17, Color32::TRANSPARENT, w, orange);
            pen.circle(0.0, 0.12, 0.06, alpha(orange, 0.35), 0.03, orange);
        }
        "leaf" => {
            badge(green);
            let (b, t) = ((-0.2f32, 0.22f32), (0.24f32, -0.24f32));
            let axis = (t.0 - b.0, t.1 - b.1);
            let len = (axis.0 * axis.0 + axis.1 * axis.1).sqrt();
            let n = (-axis.1 / len, axis.0 / len);
            let side = |sign: f32| -> Vec<(f32, f32)> {
                (0..=24)
                    .map(|k| {
                        let s = k as f32 / 24.0;
                        let wdt = 0.17 * (std::f32::consts::PI * s).sin() * (1.0 - 0.3 * s);
                        (
                            b.0 + axis.0 * s + n.0 * wdt * sign,
                            b.1 + axis.1 * s + n.1 * wdt * sign,
                        )
                    })
                    .collect()
            };
            let mut outline = side(1.0);
            outline.extend(side(-1.0).into_iter().rev());
            pen.closed(&outline, w, green);
            pen.line(&[(b.0 - 0.09, b.1 + 0.09), t], 0.03, green);
            for s in [0.3f32, 0.5, 0.7] {
                let m = (b.0 + axis.0 * s, b.1 + axis.1 * s);
                for sign in [1.0, -1.0] {
                    let e = (
                        m.0 + axis.0 * 0.14 + n.0 * 0.1 * sign,
                        m.1 + axis.1 * 0.14 + n.1 * 0.1 * sign,
                    );
                    pen.line(&[m, e], 0.022, alpha(green, 0.7));
                }
            }
        }
        "sun" => {
            badge(orange);
            pen.circle(0.0, 0.0, 0.16, alpha(yellow, 0.18), w, yellow);
            for k in 0..12 {
                let a = 30.0 * k as f32;
                let outer = if k % 2 == 0 { 0.37 } else { 0.31 };
                pen.line(
                    &[pen.polar(0.0, 0.0, 0.23, a), pen.polar(0.0, 0.0, outer, a)],
                    0.035,
                    orange,
                );
            }
        }
        "eye" => {
            badge(teal);
            let lens = |sign: f32| -> Vec<(f32, f32)> {
                (0..=32)
                    .map(|k| {
                        let u = -0.34 + 0.68 * k as f32 / 32.0;
                        (u, sign * 0.2 * (1.0 - (u / 0.34).powi(2)))
                    })
                    .collect()
            };
            let mut outline = lens(-1.0);
            outline.extend(lens(1.0).into_iter().rev());
            pen.closed(&outline, w, teal);
            pen.circle(0.0, 0.0, 0.12, Color32::TRANSPARENT, 0.04, violet);
            pen.circle(0.0, 0.0, 0.05, teal, 0.0, teal);
            pen.circle(0.04, -0.04, 0.02, ink, 0.0, ink);
        }
        tag if tag.starts_with("tag-") => {
            let (text, col) = match tag {
                "tag-ambient" => ("AMBIENT", teal),
                "tag-drone" => ("DRONE", violet),
                "tag-loop" => ("LOOP", green),
                "tag-live" => ("LIVE", theme::meter_red()),
                "tag-wip" => ("WIP", yellow),
                "tag-noinput" => ("NO INPUT", rose),
                _ => ("TAG", blue),
            };
            pen.convex(
                &pen.rounded(-0.49, -0.46, 0.49, 0.46, 0.5),
                BADGE,
                0.06,
                col,
            );
            pen.circle(-0.36, 0.0, 0.12, col, 0.0, col);
            pen.text(0.07, 0.02, text, 0.5, 0.66, col);
        }
        _ => {
            // A design this version doesn't know: a plain badge with a "?".
            badge(theme::text()_DIM);
            pen.text(0.0, 0.0, "?", 0.5, 0.8, theme::text()_DIM);
        }
    }
}

/// Cuts the sharpest corners of a stroked path a little: thick strokes
/// otherwise end in long spikes there.
fn blunt(pts: Vec<Pos2>, closed: bool, width: f32) -> Vec<Pos2> {
    let n = pts.len();
    if n < 3 {
        return pts;
    }
    let mut out = Vec::with_capacity(n + 8);
    for i in 0..n {
        let interior = closed || (i > 0 && i + 1 < n);
        let (prev, next) = (pts[(i + n - 1) % n], pts[(i + 1) % n]);
        let (a, b) = (prev - pts[i], next - pts[i]);
        let (la, lb) = (a.length(), b.length());
        // Sharper than about 70 degrees.
        if interior && la > 1e-3 && lb > 1e-3 && (a / la).dot(b / lb) > 0.34 {
            let cut = (width * 0.8).min(la / 3.0).min(lb / 3.0);
            out.push(pts[i] + a / la * cut);
            out.push(pts[i] + b / lb * cut);
        } else {
            out.push(pts[i]);
        }
    }
    out
}

/// A four-pointed star with curved sides around (u, v), `r` its points.
fn sparkle(pen: &Pen, u: f32, v: f32, r: f32, w: f32, col: Color32) {
    let mut pts = Vec::new();
    for k in 0..4 {
        let a = 90.0 * k as f32;
        let tip = pen.polar(u, v, r, a);
        let next = pen.polar(u, v, r, a + 90.0);
        let ctrl = pen.polar(u, v, r * 0.12, a + 45.0);
        for j in 0..8 {
            let t = j as f32 / 8.0;
            let q = |p0: f32, p1: f32, p2: f32| {
                (1.0 - t).powi(2) * p0 + 2.0 * (1.0 - t) * t * p1 + t * t * p2
            };
            pts.push((q(tip.0, ctrl.0, next.0), q(tip.1, ctrl.1, next.1)));
        }
    }
    pen.closed(&pts, w, col);
}

// ---------------------------------------------------------------------------
// Stickers on the canvas
// ---------------------------------------------------------------------------

/// What the board's menus ask for, done at the start of the next frame
/// (file dialogs block, and menus only see the board to read it).
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    AddBuiltin(&'static str, Pos2),
    /// One of `EXAMPLES`.
    AddExample(usize, Pos2),
    /// Pick image files (a dialog) and add them at this point.
    AddFiles(Pos2),
    AddPath(PathBuf, Pos2),
    Preset(&'static str),
    BackgroundImage,
    ClearBackgroundImage,
    Dim(f32),
    Grid(bool),
    UnlockAll,
    RevealFolder,
    Front(u32),
    Back(u32),
    RotateBy(u32, f32),
    ResetRotation(u32),
    Duplicate(u32),
    Lock(u32),
    Delete(u32),
}

/// Dragging a sticker, or one of the selected sticker's handles.
#[derive(Clone, Copy, Debug)]
enum Drag {
    Move(u32),
    /// Width at the start, and the pointer's distance from the centre then.
    Resize(u32, f32, f32),
    /// Rotation at the start, and the pointer's angle then (degrees).
    Rotate(u32, f32, f32),
}

/// What the canvas needs to know after the stickers had their turn.
#[derive(Default)]
pub struct Outcome {
    /// The pointer is on a sticker: the canvas's own menu stays shut.
    pub hovered: bool,
    /// A right or middle drag that started on a sticker pans the canvas.
    pub pan: Vec2,
}

#[derive(Default)]
pub struct BoardUi {
    /// The sticker selected: its outline and handles show.
    pub selected: Option<u32>,
    drag: Option<Drag>,
    /// Textures by image id and the size they were drawn at (SVGs are drawn
    /// again for each size they are shown at, raster images once).
    textures: HashMap<(String, u32), TextureHandle>,
    pub commands: Vec<Command>,
    /// Images in the stickers folder, and when it was last read.
    folder: Vec<PathBuf>,
    folder_read: Option<f64>,
}

/// The folder whose images the Stickers menu offers.
pub fn stickers_dir() -> Option<PathBuf> {
    crate::project::data_dir().map(|d| d.join("Stickers"))
}

/// A sticker's corners on screen, clockwise from top-left.
fn corners(c: Pos2, size: Vec2, degrees: f32) -> [Pos2; 4] {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let x = vec2(cos, sin) * size.x * 0.5;
    let y = vec2(-sin, cos) * size.y * 0.5;
    [c - x - y, c + x - y, c + x + y, c - x + y]
}

fn bounds(pts: &[Pos2]) -> Rect {
    pts.iter()
        .fold(Rect::NOTHING, |r, p| r.union(Rect::from_min_max(*p, *p)))
}

/// Whether `p` is inside the quad.
fn inside(quad: &[Pos2; 4], p: Pos2) -> bool {
    let side = |a: Pos2, b: Pos2| (b - a).x * (p - a).y - (b - a).y * (p - a).x;
    let s: Vec<f32> = (0..4).map(|i| side(quad[i], quad[(i + 1) % 4])).collect();
    s.iter().all(|&v| v >= 0.0) || s.iter().all(|&v| v <= 0.0)
}

impl BoardUi {
    /// A texture for `id` drawn about `px` pixels on its longest side
    /// (raster images: their own size, whatever `px`).
    fn texture(
        &mut self,
        ctx: &egui::Context,
        board: &Board,
        id: &str,
        px: u32,
    ) -> Option<TextureHandle> {
        let blob = board.images.get(id)?;
        let level = if blob.svg {
            px.clamp(64, 2048).next_power_of_two().min(2048)
        } else {
            0
        };
        let key = (id.to_owned(), level);
        if let Some(t) = self.textures.get(&key) {
            return Some(t.clone());
        }
        let (w, h, premultiplied, rgba) = crate::board::decode_rgba(blob, level).ok()?;
        let size = [w as usize, h as usize];
        let image = if premultiplied {
            egui::ColorImage::from_rgba_premultiplied(size, &rgba)
        } else {
            egui::ColorImage::from_rgba_unmultiplied(size, &rgba)
        };
        let options = TextureOptions {
            mipmap_mode: Some(egui::TextureFilter::Linear),
            ..TextureOptions::LINEAR
        };
        let texture = ctx.load_texture(format!("board-{id}-{level}"), image, options);
        self.textures.insert(key, texture.clone());
        Some(texture)
    }

    /// Lets go of textures for images the board no longer has.
    fn forget_textures(&mut self, board: &Board) {
        self.textures
            .retain(|(id, _), _| board.images.contains_key(id));
    }

    /// The background: the preset's colour, the image (filling the view,
    /// veiled), the grid.
    pub fn draw_background(&mut self, ui: &Ui, p: &Painter, rect: Rect, view: View, board: &Board) {
        let bg = &board.background;
        let preset = preset(&bg.preset);
        p.rect_filled(rect, 0.0, preset.fill);
        let image = bg.image.as_ref().and_then(|id| {
            let px = (rect.width().max(rect.height()) * ui.ctx().pixels_per_point()) as u32;
            self.texture(ui.ctx(), board, id, px)
                .map(|t| (t, board.images[id].clone()))
        });
        if let Some((texture, blob)) = &image {
            // Cover: the whole view, cropped to keep the image's shape.
            let img_aspect = blob.w.max(1) as f32 / blob.h.max(1) as f32;
            let view_aspect = rect.width() / rect.height().max(1.0);
            let uv = if img_aspect > view_aspect {
                let w = view_aspect / img_aspect;
                Rect::from_min_max(pos2(0.5 - w / 2.0, 0.0), pos2(0.5 + w / 2.0, 1.0))
            } else {
                let h = img_aspect / view_aspect;
                Rect::from_min_max(pos2(0.0, 0.5 - h / 2.0), pos2(1.0, 0.5 + h / 2.0))
            };
            p.image(texture.id(), rect, uv, Color32::WHITE);
            p.rect_filled(rect, 0.0, alpha(preset.fill, bg.dim.clamp(0.0, 0.95)));
        }
        if bg.grid {
            let (minor, major) = if image.is_some() {
                (alpha(theme::text(), 0.05), alpha(theme::text(), 0.10))
            } else {
                (preset.minor, preset.major)
            };
            let style = if preset.grid == Grid::None && image.is_some() {
                Grid::None
            } else {
                preset.grid
            };
            draw_grid(p, rect, view, style, minor, major);
        }
    }

    /// The stickers, back to front, and what the pointer does with them.
    /// Call before the canvas's nodes are laid out, so nodes stay on top
    /// for the pointer as on screen. `nodes` is the node selection: picking
    /// a sticker clears it, selecting nodes drops the sticker's.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &Ui,
        p: &Painter,
        rect: Rect,
        view: View,
        board: &mut Board,
        bg: &Response,
        nodes: &mut HashSet<NodeId>,
        notices: &mut Vec<String>,
    ) -> Outcome {
        let mut out = Outcome::default();
        self.run_commands(board, notices);
        self.take_drops(ui, board, view, rect, notices);
        if !nodes.is_empty() || bg.clicked() || board.stickers.is_empty() {
            self.selected = None;
        }
        if self.selected.is_some_and(|id| board.sticker(id).is_none()) {
            self.selected = None;
        }
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let ctrl_click = cfg!(target_os = "macos") && ui.input(|i| i.modifiers.ctrl);

        for k in 0..board.stickers.len() {
            let s = board.stickers[k].clone();
            let aspect = board.aspect(&s.art, builtin_aspect);
            let size = vec2(s.width, s.width * aspect) * view.zoom;
            let c = view.to_screen(pos2(s.pos[0], s.pos[1]));
            let quad = corners(c, size, s.rotation);
            let area = bounds(&quad);
            if !area.intersects(rect) {
                continue;
            }
            match &s.art {
                Art::Builtin(name) if size.x.min(size.y) >= 6.0 => {
                    draw_builtin(p, c, size, s.rotation, name)
                }
                Art::Builtin(_) => {}
                Art::Image(id) => {
                    let px = (size.x.max(size.y) * ui.ctx().pixels_per_point()) as u32;
                    match self.texture(ui.ctx(), board, id, px) {
                        Some(t) => {
                            let mut mesh = Mesh::with_texture(t.id());
                            let uvs = [
                                pos2(0.0, 0.0),
                                pos2(1.0, 0.0),
                                pos2(1.0, 1.0),
                                pos2(0.0, 1.0),
                            ];
                            for (q, uv) in quad.iter().zip(uvs) {
                                mesh.vertices.push(egui::epaint::Vertex {
                                    pos: *q,
                                    uv,
                                    color: Color32::WHITE,
                                });
                            }
                            mesh.indices.extend([0, 1, 2, 0, 2, 3]);
                            p.add(Shape::mesh(mesh));
                        }
                        None => {
                            p.add(Shape::closed_line(
                                quad.to_vec(),
                                Stroke::new(1.0, theme::text()_FAINT),
                            ));
                        }
                    }
                }
            }
            if s.locked {
                continue;
            }
            let resp = ui.interact(
                area,
                Id::unique(("board_sticker", s.id)),
                Sense::CLICK | Sense::DRAG,
            );
            let on_art = pointer.is_some_and(|q| inside(&quad, q));
            if resp.hovered() && on_art {
                out.hovered = true;
                ui.ctx().set_cursor_icon(CursorIcon::Grab);
                if self.selected != Some(s.id) {
                    p.add(Shape::closed_line(
                        quad.to_vec(),
                        Stroke::new(1.0, alpha(theme::accent(), 0.45)),
                    ));
                }
            }
            if (resp.clicked() && !ctrl_click) || resp.drag_started_by(egui::PointerButton::Primary)
            {
                self.selected = Some(s.id);
                nodes.clear();
            }
            if resp.drag_started_by(egui::PointerButton::Primary) {
                self.drag = Some(Drag::Move(s.id));
            }
            if resp.dragged_by(egui::PointerButton::Primary)
                && let Some(Drag::Move(id)) = self.drag
                && id == s.id
            {
                let d = resp.drag_delta() / view.zoom;
                if let Some(st) = board.sticker_mut(id) {
                    st.pos = [st.pos[0] + d.x, st.pos[1] + d.y];
                }
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            }
            if resp.dragged_by(egui::PointerButton::Secondary)
                || resp.dragged_by(egui::PointerButton::Middle)
            {
                out.pan += resp.drag_delta();
            }
            let open = resp.secondary_clicked() || (ctrl_click && resp.clicked());
            if open {
                self.selected = Some(s.id);
                nodes.clear();
            }
            let commands = &mut self.commands;
            egui::Popup::menu(&resp)
                .open_memory(open.then_some(egui::SetOpenCommand::Bool(true)))
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .at_pointer_fixed()
                .show(|ui| sticker_menu(ui, s.id, commands));
        }

        // The selected sticker's outline and handles, over the stickers.
        if let Some(id) = self.selected
            && let Some(s) = board.sticker(id).cloned()
        {
            let aspect = board.aspect(&s.art, builtin_aspect);
            let size = vec2(s.width, s.width * aspect) * view.zoom;
            let c = view.to_screen(pos2(s.pos[0], s.pos[1]));
            let quad = corners(c, size, s.rotation);
            p.add(Shape::closed_line(
                quad.to_vec(),
                Stroke::new(1.0, theme::accent()),
            ));
            let resize_at = quad[2];
            let out_dir = (quad[1] - c).normalized();
            let rotate_at = quad[1] + out_dir * 14.0;
            p.line_segment(
                [quad[1], rotate_at],
                Stroke::new(1.0, alpha(theme::accent(), 0.6)),
            );
            let resize = ui.interact(
                Rect::from_center_size(resize_at, vec2(16.0, 16.0)),
                Id::unique(("board_resize", id)),
                Sense::DRAG,
            );
            let rotate = ui.interact(
                Rect::from_center_size(rotate_at, vec2(16.0, 16.0)),
                Id::unique(("board_rotate", id)),
                Sense::DRAG,
            );
            let hot = |r: &Response| r.hovered() || r.dragged();
            p.rect(
                Rect::from_center_size(resize_at, vec2(8.0, 8.0)),
                1.5,
                if hot(&resize) { theme::accent() } else { BADGE },
                Stroke::new(1.2, theme::accent()),
                StrokeKind::Middle,
            );
            p.circle(
                rotate_at,
                4.5,
                if hot(&rotate) { theme::accent() } else { BADGE },
                Stroke::new(1.2, theme::accent()),
            );
            if hot(&resize) || hot(&rotate) {
                out.hovered = true;
            }
            let resize = resize
                .on_hover_cursor(CursorIcon::ResizeNwSe)
                .on_hover_text("Drag to resize");
            let rotate = rotate
                .on_hover_cursor(CursorIcon::Alias)
                .on_hover_text("Drag to rotate (Shift: 15° steps)");
            if let Some(q) = pointer {
                if resize.drag_started() {
                    self.drag = Some(Drag::Resize(id, s.width, (q - c).length().max(1.0)));
                }
                if rotate.drag_started() {
                    self.drag = Some(Drag::Rotate(id, s.rotation, (q - c).angle().to_degrees()));
                }
                match self.drag {
                    Some(Drag::Resize(did, w0, d0)) if did == id && resize.dragged() => {
                        let w = (w0 * (q - c).length() / d0).clamp(WIDTH_RANGE.0, WIDTH_RANGE.1);
                        if let Some(st) = board.sticker_mut(id) {
                            st.width = w;
                        }
                    }
                    Some(Drag::Rotate(did, r0, a0)) if did == id && rotate.dragged() => {
                        let mut r = r0 + (q - c).angle().to_degrees() - a0;
                        if ui.input(|i| i.modifiers.shift) {
                            r = (r / 15.0).round() * 15.0;
                        }
                        if let Some(st) = board.sticker_mut(id) {
                            st.rotation = (r + 180.0).rem_euclid(360.0) - 180.0;
                        }
                    }
                    _ => {}
                }
            }
            // Delete and Esc act on the sticker while it is selected.
            if ui.memory(|m| m.focused().is_none()) {
                let (delete, escape) = ui.input_mut(|i| {
                    let d = i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                        || i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace);
                    (d, i.key_pressed(egui::Key::Escape))
                });
                if delete {
                    board.remove(id);
                    self.selected = None;
                } else if escape {
                    self.selected = None;
                }
            }
        }
        if !ui.input(|i| i.pointer.any_down()) {
            self.drag = None;
        }
        out
    }

    /// Does what the menus asked for.
    fn run_commands(&mut self, board: &mut Board, notices: &mut Vec<String>) {
        for command in std::mem::take(&mut self.commands) {
            match command {
                Command::AddBuiltin(name, at) => {
                    self.selected = Some(board.add(
                        Art::Builtin(name.to_owned()),
                        [at.x, at.y],
                        builtin_width(name),
                    ));
                }
                Command::AddFiles(at) => {
                    let mut dialog = rfd::FileDialog::new()
                        .set_title("Add stickers")
                        .add_filter("Images", &["png", "jpg", "jpeg", "svg", "svgz", "webp"]);
                    if let Some(dir) = stickers_dir().filter(|d| d.exists()) {
                        dialog = dialog.set_directory(dir);
                    }
                    for (k, path) in dialog
                        .pick_files()
                        .unwrap_or_default()
                        .into_iter()
                        .enumerate()
                    {
                        let at = at + vec2(28.0, 28.0) * k as f32;
                        self.add_image_file(board, &path, at, notices);
                    }
                }
                Command::AddPath(path, at) => self.add_image_file(board, &path, at, notices),
                Command::AddExample(i, at) => {
                    if let Some((name, bytes)) = EXAMPLES.get(i) {
                        match board.import_image(bytes, STICKER_MAX_PX, false) {
                            Ok(id) => {
                                self.selected = Some(board.add(Art::Image(id), [at.x, at.y], 170.0))
                            }
                            Err(e) => notices.push(format!("Could not add {name}: {e}")),
                        }
                    }
                }
                Command::Preset(id) => board.background.preset = id.to_owned(),
                Command::BackgroundImage => {
                    let picked = rfd::FileDialog::new()
                        .set_title("Background image")
                        .add_filter("Images", &["png", "jpg", "jpeg", "svg", "svgz", "webp"])
                        .pick_file();
                    if let Some(path) = picked {
                        match std::fs::read(&path)
                            .map_err(|e| e.to_string())
                            .and_then(|b| board.import_image(&b, BACKGROUND_MAX_PX, true))
                        {
                            Ok(id) => {
                                board.background.image = Some(id);
                                board.prune();
                            }
                            Err(e) => {
                                notices.push(format!("Could not use {}: {e}", file_name(&path)))
                            }
                        }
                    }
                }
                Command::ClearBackgroundImage => {
                    board.background.image = None;
                    board.prune();
                }
                Command::Dim(d) => board.background.dim = d.clamp(0.0, 0.95),
                Command::Grid(on) => board.background.grid = on,
                Command::UnlockAll => board.stickers.iter_mut().for_each(|s| s.locked = false),
                Command::RevealFolder => {
                    if let Some(dir) = stickers_dir() {
                        let opened = std::fs::create_dir_all(&dir).and_then(|()| {
                            let opener = if cfg!(target_os = "macos") {
                                "open"
                            } else if cfg!(target_os = "windows") {
                                "explorer"
                            } else {
                                "xdg-open"
                            };
                            std::process::Command::new(opener)
                                .arg(&dir)
                                .spawn()
                                .map(|_| ())
                        });
                        if let Err(e) = opened {
                            notices.push(format!("Could not open {}: {e}", dir.display()));
                        }
                        self.folder_read = None;
                    }
                }
                Command::Front(id) => board.restack(id, true),
                Command::Back(id) => board.restack(id, false),
                Command::RotateBy(id, d) => {
                    if let Some(s) = board.sticker_mut(id) {
                        s.rotation = (s.rotation + d + 180.0).rem_euclid(360.0) - 180.0;
                    }
                }
                Command::ResetRotation(id) => {
                    if let Some(s) = board.sticker_mut(id) {
                        s.rotation = 0.0;
                    }
                }
                Command::Duplicate(id) => self.selected = board.duplicate(id),
                Command::Lock(id) => {
                    if let Some(s) = board.sticker_mut(id) {
                        s.locked = true;
                    }
                    self.selected = None;
                }
                Command::Delete(id) => {
                    board.remove(id);
                    self.selected = None;
                }
            }
        }
        self.forget_textures(board);
    }

    fn add_image_file(
        &mut self,
        board: &mut Board,
        path: &std::path::Path,
        at: Pos2,
        notices: &mut Vec<String>,
    ) {
        match std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|b| board.import_image(&b, STICKER_MAX_PX, false))
        {
            Ok(id) => {
                let blob = &board.images[&id];
                // Its own size, within reason; an SVG a little larger.
                let width = if blob.svg {
                    180.0
                } else {
                    (blob.w as f32).clamp(80.0, 280.0)
                };
                self.selected = Some(board.add(Art::Image(id), [at.x, at.y], width));
            }
            Err(e) => notices.push(format!("Could not add {}: {e}", file_name(path))),
        }
    }

    /// Image files dropped on the canvas become stickers where they land;
    /// while they hover, the canvas says so.
    fn take_drops(
        &mut self,
        ui: &Ui,
        board: &mut Board,
        view: View,
        rect: Rect,
        notices: &mut Vec<String>,
    ) {
        let (hovering, dropped, pointer) = ui.input(|i| {
            (
                !i.raw.hovered_files.is_empty(),
                i.raw.dropped_files.clone(),
                i.pointer.hover_pos(),
            )
        });
        if hovering {
            let p = ui.painter_at(rect);
            p.rect(
                rect.shrink(6.0),
                10.0,
                alpha(theme::accent(), 0.05),
                Stroke::new(1.5, alpha(theme::accent(), 0.7)),
                StrokeKind::Inside,
            );
            p.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Drop images to add them as stickers (PNG, JPG, SVG)",
                FontId::proportional(13.0),
                theme::text(),
            );
        }
        let at = view.to_world(
            pointer
                .filter(|q| rect.contains(*q))
                .unwrap_or(rect.center()),
        );
        for (k, file) in dropped.iter().enumerate() {
            let at = at + vec2(28.0, 28.0) * k as f32;
            let path = file.path();
            let name = file_name(path);
            let bytes = if crate::board::is_image_path(path) {
                file.bytes()
            } else {
                Err("not a PNG, JPG, SVG or WebP image".to_owned())
            };
            match bytes.and_then(|b| board.import_image(&b, STICKER_MAX_PX, false)) {
                Ok(id) => {
                    let blob = &board.images[&id];
                    let width = if blob.svg {
                        180.0
                    } else {
                        (blob.w as f32).clamp(80.0, 280.0)
                    };
                    self.selected = Some(board.add(Art::Image(id), [at.x, at.y], width));
                }
                Err(e) => notices.push(format!("Could not add {name}: {e}")),
            }
        }
    }

    /// The canvas menu's Stickers and Background submenus. `at` is where
    /// the menu was opened, in canvas units.
    pub fn canvas_menu(&mut self, ui: &mut Ui, at: Pos2, board: &Board) {
        let now = ui.input(|i| i.time);
        if self.folder_read.is_none_or(|t| now - t > 2.0) {
            self.folder_read = Some(now);
            self.folder = stickers_dir()
                .and_then(|d| std::fs::read_dir(d).ok())
                .map(|entries| {
                    let mut files: Vec<PathBuf> = entries
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| crate::board::is_image_path(p))
                        .collect();
                    files.sort();
                    files
                })
                .unwrap_or_default();
        }
        ui.separator();
        let commands = &mut self.commands;
        let folder = &self.folder;
        ui.menu_button(
            RichText::new("Stickers").color(theme::text()).size(12.5),
            |ui| {
                sticker_picker(ui, at, commands);
                ui.separator();
                if super::canvas::menu_item(ui, "Image file…", "PNG · JPG · SVG") {
                    commands.push(Command::AddFiles(at));
                    ui.close();
                }
                ui.label(
                    RichText::new("EXAMPLE FILES")
                        .size(9.5)
                        .color(theme::text()_FAINT),
                );
                for (i, (name, _)) in EXAMPLES.iter().enumerate() {
                    if super::canvas::menu_item(ui, name, "SVG") {
                        commands.push(Command::AddExample(i, at));
                        ui.close();
                    }
                }
                if !folder.is_empty() {
                    ui.label(
                        RichText::new("YOUR STICKERS")
                            .size(9.5)
                            .color(theme::text()_FAINT),
                    );
                    for path in folder.iter().take(24) {
                        if super::canvas::menu_item(ui, &file_name(path), "") {
                            commands.push(Command::AddPath(path.clone(), at));
                            ui.close();
                        }
                    }
                }
                if super::canvas::menu_item(ui, "Open stickers folder", "") {
                    commands.push(Command::RevealFolder);
                    ui.close();
                }
                if board.stickers.iter().any(|s| s.locked)
                    && super::canvas::menu_item(ui, "Unlock all stickers", "")
                {
                    commands.push(Command::UnlockAll);
                    ui.close();
                }
            },
        );
        ui.menu_button(
            RichText::new("Background").color(theme::text()).size(12.5),
            |ui| background_picker(ui, board, commands),
        );
    }
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// A sticker's own menu.
fn sticker_menu(ui: &mut Ui, id: u32, commands: &mut Vec<Command>) {
    let mut item = |ui: &mut Ui, label: &str, hint: &str, c: Command| {
        if super::canvas::menu_item(ui, label, hint) {
            commands.push(c);
            ui.close();
        }
    };
    item(ui, "Bring to front", "", Command::Front(id));
    item(ui, "Send to back", "", Command::Back(id));
    item(ui, "Rotate left", "15°", Command::RotateBy(id, -15.0));
    item(ui, "Rotate right", "15°", Command::RotateBy(id, 15.0));
    item(ui, "Straighten", "", Command::ResetRotation(id));
    item(ui, "Duplicate", "", Command::Duplicate(id));
    item(ui, "Lock", "clicks pass through", Command::Lock(id));
    item(ui, "Delete", "Del", Command::Delete(id));
}

/// The built-in designs as a grid of tiles; a click adds one at `at`.
fn sticker_picker(ui: &mut Ui, at: Pos2, commands: &mut Vec<Command>) {
    ui.label(RichText::new("STICKERS").size(9.5).color(theme::text()_FAINT));
    let icons: Vec<&(&str, &str)> = BUILTINS
        .iter()
        .filter(|(id, _)| !id.starts_with("tag-"))
        .collect();
    let tags: Vec<&(&str, &str)> = BUILTINS
        .iter()
        .filter(|(id, _)| id.starts_with("tag-"))
        .collect();
    let mut tile = |ui: &mut Ui, id: &'static str, label: &str, size: Vec2| {
        let (rect, resp) = ui.allocate_exact_size(size, Sense::CLICK);
        let p = ui.painter();
        if resp.hovered() {
            p.rect_filled(rect, 6.0, theme::control());
        }
        let inner = rect.shrink(5.0);
        let aspect = builtin_aspect(id);
        let art = if inner.height() / inner.width() > aspect {
            vec2(inner.width(), inner.width() * aspect)
        } else {
            vec2(inner.height() / aspect, inner.height())
        };
        draw_builtin(p, rect.center(), art, 0.0, id);
        if resp
            .on_hover_text(label)
            .on_hover_cursor(CursorIcon::PointingHand)
            .clicked()
        {
            commands.push(Command::AddBuiltin(id, at));
            ui.close();
        }
    };
    for row in icons.chunks(5) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for &&(id, label) in row {
                tile(ui, id, label, vec2(50.0, 50.0));
            }
        });
    }
    for row in tags.chunks(3) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for &&(id, label) in row {
                tile(ui, id, label, vec2(84.0, 32.0));
            }
        });
    }
}

/// The background presets as swatches, then the image and its veil, and
/// the grid.
fn background_picker(ui: &mut Ui, board: &Board, commands: &mut Vec<Command>) {
    let bg = &board.background;
    ui.label(
        RichText::new("BACKGROUND")
            .size(9.5)
            .color(theme::text()_FAINT),
    );
    for row in PRESETS.chunks(4) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for preset in row {
                let (rect, resp) = ui.allocate_exact_size(vec2(64.0, 58.0), Sense::CLICK);
                let swatch = Rect::from_min_size(rect.min + vec2(2.0, 2.0), vec2(60.0, 38.0));
                let p = ui.painter();
                p.rect_filled(swatch, 5.0, preset.fill);
                let view = View {
                    origin: swatch.min,
                    zoom: 0.5,
                };
                draw_grid(
                    &p.with_clip_rect(swatch.shrink(1.0)),
                    swatch,
                    view,
                    preset.grid,
                    preset.major,
                    theme::mix(preset.major, theme::text(), 0.2),
                );
                let on = bg.preset == preset.id;
                let border = if on {
                    theme::accent()
                } else if resp.hovered() {
                    theme::control()_HI
                } else {
                    theme::node_border()
                };
                p.rect_stroke(
                    swatch,
                    5.0,
                    Stroke::new(if on { 1.5 } else { 1.0 }, border),
                    StrokeKind::Inside,
                );
                p.text(
                    pos2(rect.center().x, rect.bottom() - 8.0),
                    Align2::CENTER_CENTER,
                    preset.label,
                    FontId::proportional(10.0),
                    if on { theme::text() } else { theme::text()_DIM },
                );
                if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    commands.push(Command::Preset(preset.id));
                }
            }
        });
    }
    ui.separator();
    if super::canvas::menu_item(
        ui,
        if bg.image.is_some() {
            "Change image…"
        } else {
            "Image…"
        },
        "PNG · JPG · SVG",
    ) {
        commands.push(Command::BackgroundImage);
        ui.close();
    }
    if bg.image.is_some() {
        if super::canvas::menu_item(ui, "Remove image", "") {
            commands.push(Command::ClearBackgroundImage);
            ui.close();
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("Veil").size(11.5).color(theme::text()_DIM));
            let mut dim = bg.dim;
            if ui
                .add(egui::Slider::new(&mut dim, 0.0..=0.95).show_value(false))
                .on_hover_text("How much the colour veils the image, so nodes stay readable")
                .changed()
            {
                commands.push(Command::Dim(dim));
            }
        });
    }
    let mut grid = bg.grid;
    if ui
        .checkbox(&mut grid, RichText::new("Grid").size(12.0))
        .changed()
    {
        commands.push(Command::Grid(grid));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_and_designs_are_known() {
        assert_eq!(preset("blueprint").label, "Blueprint");
        assert_eq!(preset("nonsense").id, "graphite", "unknown: the default");
        let mut ids: Vec<&str> = BUILTINS.iter().map(|(id, _)| *id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), BUILTINS.len(), "ids are unique");
        assert_eq!(builtin_aspect("tag-live"), 0.34);
        assert_eq!(builtin_aspect("moon"), 1.0);
    }

    #[test]
    fn example_files_are_readable_svgs() {
        let mut b = Board::default();
        for (name, bytes) in EXAMPLES {
            let id = b
                .import_image(bytes, STICKER_MAX_PX, false)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let blob = &b.images[&id];
            assert!(blob.svg && blob.w > 0 && blob.h > 0, "{name}");
            let (_, _, _, px) = crate::board::decode_rgba(blob, 256).unwrap();
            assert!(
                px.chunks(4).any(|p| p[3] == 0),
                "{name}: has a transparent background"
            );
            assert!(px.chunks(4).any(|p| p[3] == 255), "{name}: draws something");
        }
    }

    /// Frames of the stickers alone over a 600×400 canvas, the pointer
    /// driven by `events`.
    struct Rig {
        ctx: egui::Context,
        ui: BoardUi,
        board: Board,
        nodes: HashSet<NodeId>,
        time: f64,
    }

    impl Rig {
        fn frame(&mut self, events: Vec<egui::Event>) -> Outcome {
            self.time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut outcome = Outcome::default();
            let (st, board, nodes) = (&mut self.ui, &mut self.board, &mut self.nodes);
            let mut out = self.ctx.run_ui(input, |ui| {
                let rect = ui.max_rect();
                let bg = ui.interact(rect, Id::unique("bg"), Sense::CLICK | Sense::DRAG);
                let p = ui.painter_at(rect);
                let view = View {
                    origin: rect.min,
                    zoom: 1.0,
                };
                st.draw_background(ui, &p, rect, view, board);
                let mut notices = Vec::new();
                outcome = st.show(ui, &p, rect, view, board, &bg, nodes, &mut notices);
            });
            out.textures_delta.clear();
            outcome
        }
    }

    fn button(pos: Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn stickers_are_picked_moved_resized_and_deleted() {
        use egui::PointerButton::{Primary, Secondary};
        let mut rig = Rig {
            ctx: egui::Context::default(),
            ui: BoardUi::default(),
            board: Board::default(),
            nodes: HashSet::new(),
            time: 0.0,
        };
        let id = rig
            .board
            .add(Art::Builtin("moon".into()), [200.0, 200.0], 100.0);
        rig.nodes.insert(7);
        rig.frame(vec![]);
        // Over it: the canvas menu must stay shut.
        assert!(
            rig.frame(vec![egui::Event::PointerMoved(pos2(200.0, 200.0))])
                .hovered
        );
        assert!(
            !rig.frame(vec![egui::Event::PointerMoved(pos2(500.0, 350.0))])
                .hovered
        );

        // A click picks it, and drops the node selection.
        rig.frame(vec![
            egui::Event::PointerMoved(pos2(200.0, 200.0)),
            button(pos2(200.0, 200.0), Primary, true),
        ]);
        rig.frame(vec![button(pos2(200.0, 200.0), Primary, false)]);
        rig.frame(vec![]);
        assert_eq!(rig.ui.selected, Some(id));
        assert!(rig.nodes.is_empty());

        // Dragged 60 to the right and 30 down.
        rig.frame(vec![button(pos2(200.0, 200.0), Primary, true)]);
        for k in 1..=6 {
            rig.frame(vec![egui::Event::PointerMoved(pos2(
                200.0 + 10.0 * k as f32,
                200.0 + 5.0 * k as f32,
            ))]);
        }
        rig.frame(vec![button(pos2(260.0, 230.0), Primary, false)]);
        let s = rig.board.sticker(id).unwrap();
        assert!(
            (s.pos[0] - 260.0).abs() < 12.0 && (s.pos[1] - 230.0).abs() < 8.0,
            "{:?}",
            s.pos
        );

        // The resize handle, at its bottom-right corner, doubles it.
        let s = rig.board.sticker(id).unwrap().clone();
        let corner = pos2(s.pos[0] + 50.0, s.pos[1] + 50.0);
        rig.frame(vec![egui::Event::PointerMoved(corner)]);
        rig.frame(vec![button(corner, Primary, true)]);
        for k in 1..=10 {
            rig.frame(vec![egui::Event::PointerMoved(
                corner + vec2(5.0, 5.0) * k as f32,
            )]);
        }
        rig.frame(vec![button(corner + vec2(50.0, 50.0), Primary, false)]);
        let w = rig.board.sticker(id).unwrap().width;
        assert!((w - 200.0).abs() < 25.0, "width {w}");

        // A right-drag on it pans the canvas instead.
        let at = pos2(
            rig.board.sticker(id).unwrap().pos[0],
            rig.board.sticker(id).unwrap().pos[1],
        );
        rig.frame(vec![
            egui::Event::PointerMoved(at),
            button(at, Secondary, true),
        ]);
        let mut pan = Vec2::ZERO;
        for k in 1..=5 {
            pan += rig
                .frame(vec![egui::Event::PointerMoved(
                    at + vec2(0.0, 8.0 * k as f32),
                )])
                .pan;
        }
        rig.frame(vec![button(at + vec2(0.0, 40.0), Secondary, false)]);
        assert!(pan.y > 20.0, "panned {pan:?}");

        // Delete removes the selected sticker.
        rig.frame(vec![egui::Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert!(rig.board.stickers.is_empty());
        assert_eq!(rig.ui.selected, None);
    }

    #[test]
    fn locked_stickers_let_the_pointer_through() {
        let mut rig = Rig {
            ctx: egui::Context::default(),
            ui: BoardUi::default(),
            board: Board::default(),
            nodes: HashSet::new(),
            time: 0.0,
        };
        let id = rig
            .board
            .add(Art::Builtin("sun".into()), [200.0, 200.0], 100.0);
        rig.ui.commands.push(Command::Lock(id));
        rig.frame(vec![]);
        assert!(
            !rig.frame(vec![egui::Event::PointerMoved(pos2(200.0, 200.0))])
                .hovered
        );
        rig.ui.commands.push(Command::UnlockAll);
        rig.frame(vec![]);
        assert!(
            rig.frame(vec![egui::Event::PointerMoved(pos2(201.0, 200.0))])
                .hovered
        );
        // Restacking and turning, from its menu.
        let other = rig
            .board
            .add(Art::Builtin("eye".into()), [400.0, 200.0], 100.0);
        rig.ui.commands.extend([
            Command::Back(other),
            Command::RotateBy(id, 15.0),
            Command::RotateBy(id, 180.0),
        ]);
        rig.frame(vec![]);
        assert_eq!(rig.board.stickers[0].id, other);
        assert_eq!(rig.board.sticker(id).unwrap().rotation, -165.0);
    }

    #[test]
    fn quads_and_hits() {
        let q = corners(pos2(100.0, 100.0), vec2(40.0, 20.0), 0.0);
        assert_eq!(q[0], pos2(80.0, 90.0));
        assert!(inside(&q, pos2(100.0, 100.0)));
        assert!(!inside(&q, pos2(100.0, 115.0)));
        // Turned a quarter: now tall.
        let q = corners(pos2(100.0, 100.0), vec2(40.0, 20.0), 90.0);
        assert!(inside(&q, pos2(100.0, 115.0)));
        assert!(!inside(&q, pos2(115.0, 100.0)));
        let v = View {
            origin: pos2(10.0, 20.0),
            zoom: 2.0,
        };
        assert_eq!(v.to_world(v.to_screen(pos2(3.0, 4.0))), pos2(3.0, 4.0));
    }

    /// Every design draws without panicking, at any size and turn.
    #[test]
    fn designs_draw() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let p = ui.painter();
            for (id, _) in BUILTINS.iter().chain(&[("unknown", "?")]) {
                for (size, deg) in [
                    (vec2(150.0, 150.0), 0.0),
                    (vec2(12.0, 4.0), 33.0),
                    (vec2(400.0, 136.0), -170.0),
                ] {
                    draw_builtin(p, pos2(200.0, 200.0), size, deg, id);
                }
            }
        });
        out.textures_delta.clear();
    }
}
