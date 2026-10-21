//! Custom-painted controls. Every size is multiplied by the canvas zoom `z`.

use egui::epaint::{CubicBezierShape, Shadow};
use egui::{
    Align2, Color32, CursorIcon, Id, Painter, Pos2, Rect, Response, Sense, Shape, Stroke,
    Ui, emath, pos2, vec2,
};

use super::theme::{self, alpha};
use crate::audio::dsp::preview_sample;
use crate::audio::params::{FADER_UNITY, Waveform, gain_to_db};

// ---------------------------------------------------------------------------
// Meters
// ---------------------------------------------------------------------------

/// Display-side ballistics for a peak meter: fast attack, smooth release,
/// and a peak-hold marker.
#[derive(Default, Clone, Copy)]
pub struct MeterState {
    pub level: f32,
    pub peak: f32,
    peak_age: f32,
}

impl MeterState {
    pub fn update(&mut self, input: f32, dt: f32) {
        let fall = 10f32.powf(-26.0 * dt / 20.0); // 26 dB/s release
        self.level = input.max(self.level * fall);
        if input >= self.peak {
            self.peak = input;
            self.peak_age = 0.0;
        } else {
            self.peak_age += dt;
            if self.peak_age > 1.2 {
                self.peak *= 10f32.powf(-20.0 * dt / 20.0);
            }
        }
    }
}

const METER_MIN_DB: f32 = -54.0;
const METER_MAX_DB: f32 = 6.0;

fn meter_norm(lin: f32) -> f32 {
    ((gain_to_db(lin) - METER_MIN_DB) / (METER_MAX_DB - METER_MIN_DB)).clamp(0.0, 1.0)
}

fn meter_color(db: f32) -> Color32 {
    if db > -1.0 {
        theme::METER_RED
    } else if db > -9.0 {
        theme::METER_YELLOW
    } else {
        theme::METER_GREEN
    }
}

/// Horizontal peak meter: a continuous bar split into green / yellow / red
/// zones, plus a thin peak-hold marker.
pub fn hmeter(p: &Painter, rect: Rect, m: &MeterState, z: f32) {
    p.rect_filled(rect, 1.0 * z, theme::INSET);
    let x_of = |t: f32| emath::lerp(rect.left()..=rect.right(), t);
    let lvl = if m.level > 1e-4 { meter_norm(m.level) } else { 0.0 };
    let zones = [(-9.0, theme::METER_GREEN), (-1.0, theme::METER_YELLOW), (METER_MAX_DB, theme::METER_RED)];
    let mut t0 = 0.0;
    for (db, color) in zones {
        let t1 = (db - METER_MIN_DB) / (METER_MAX_DB - METER_MIN_DB);
        if lvl > t0 {
            let r = Rect::from_x_y_ranges(x_of(t0)..=x_of(lvl.min(t1)), rect.y_range());
            p.rect_filled(r, 0.0, alpha(color, 0.9));
        }
        t0 = t1;
    }
    if m.peak > 1e-4 {
        let pk = meter_norm(m.peak);
        let x = x_of(pk);
        let db = METER_MIN_DB + pk * (METER_MAX_DB - METER_MIN_DB);
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0 * z, meter_color(db)));
    }
}

// ---------------------------------------------------------------------------
// Knob
// ---------------------------------------------------------------------------

const KNOB_START: f32 = 0.75 * std::f32::consts::PI;
const KNOB_SWEEP: f32 = 1.5 * std::f32::consts::PI;

fn arc(c: Pos2, r: f32, a0: f32, a1: f32) -> Vec<Pos2> {
    // About one segment per 4 px of arc: smooth, with few vertices.
    let n = (((a1 - a0).abs() * r) / 4.0).ceil().max(2.0) as usize;
    (0..=n)
        .map(|i| {
            let a = emath::lerp(a0..=a1, i as f32 / n as f32);
            c + vec2(a.cos(), a.sin()) * r
        })
        .collect()
}

/// Rotary knob editing a normalized value. Drag vertically (Shift = fine),
/// double-click to reset.
#[allow(clippy::too_many_arguments)]
pub fn knob(
    ui: &Ui,
    p: &Painter,
    id: Id,
    c: Pos2,
    r: f32,
    norm: &mut f32,
    default: f32,
    bipolar: bool,
    accent: Color32,
    z: f32,
) -> Response {
    let rect = Rect::from_center_size(c, vec2(r * 2.4, r * 2.4));
    let resp = ui
        .interact(rect, id, Sense::CLICK | Sense::DRAG)
        .on_hover_cursor(CursorIcon::ResizeVertical);
    if resp.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let d = resp.drag_delta();
        *norm = (*norm + (d.x - d.y) * if fine { 0.0012 } else { 0.0055 }).clamp(0.0, 1.0);
    }
    if resp.double_clicked() {
        *norm = default;
    }

    let active = resp.dragged() || resp.hovered();
    let a_val = KNOB_START + KNOB_SWEEP * *norm;
    let (a0, a1) = if bipolar {
        let mid = KNOB_START + KNOB_SWEEP * 0.5;
        (mid.min(a_val), mid.max(a_val))
    } else {
        (KNOB_START, a_val)
    };

    let w = 2.2 * z;
    p.add(Shape::line(arc(c, r, KNOB_START, KNOB_START + KNOB_SWEEP), Stroke::new(w, theme::CONTROL)));
    if a1 - a0 > 0.01 {
        p.add(Shape::line(arc(c, r, a0, a1), Stroke::new(w, accent)));
    }

    let body_r = r * 0.66;
    p.circle(
        c,
        body_r,
        if active { theme::CONTROL_HI } else { theme::CONTROL },
        Stroke::NONE,
    );
    let dir = vec2(a_val.cos(), a_val.sin());
    p.line_segment(
        [c + dir * body_r * 0.25, c + dir * body_r * 0.9],
        Stroke::new(1.6 * z, theme::TEXT),
    );
    resp
}

// ---------------------------------------------------------------------------
// Fader
// ---------------------------------------------------------------------------

/// Horizontal fader. `pos` is the 0..1 fader travel.
#[allow(clippy::too_many_arguments)]
pub fn hfader(
    ui: &Ui,
    p: &Painter,
    id: Id,
    rect: Rect,
    pos: &mut f32,
    accent: Color32,
    enabled: bool,
    z: f32,
) -> Response {
    let resp = ui
        .interact(rect.expand2(vec2(6.0 * z, 4.0 * z)), id, Sense::CLICK | Sense::DRAG)
        .on_hover_cursor(CursorIcon::ResizeHorizontal);
    if resp.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let k = if fine { 0.2 } else { 1.0 };
        *pos = (*pos + resp.drag_delta().x / rect.width() * k).clamp(0.0, 1.0);
    }
    if resp.double_clicked() {
        *pos = FADER_UNITY;
    }

    let cy = rect.center().y;
    let track = Rect::from_min_max(pos2(rect.left(), cy - 1.5 * z), pos2(rect.right(), cy + 1.5 * z));
    p.rect_filled(track, 1.5 * z, theme::CONTROL);
    let x = emath::lerp(rect.left()..=rect.right(), *pos);
    let fill = if enabled { accent } else { theme::TEXT_FAINT };
    p.rect_filled(Rect::from_min_max(track.min, pos2(x, track.max.y)), 1.5 * z, fill);

    // Unity-gain tick.
    let ux = emath::lerp(rect.left()..=rect.right(), FADER_UNITY);
    p.line_segment([pos2(ux, cy - 6.0 * z), pos2(ux, cy - 3.5 * z)], Stroke::new(1.0 * z, theme::TEXT_FAINT));

    let hot = resp.dragged() || resp.hovered();
    let thumb = Rect::from_center_size(pos2(x, cy), vec2(6.0 * z, 12.0 * z));
    p.rect_filled(
        thumb,
        1.5 * z,
        if hot { theme::TEXT } else { Color32::from_rgb(188, 192, 200) },
    );
    resp
}

// ---------------------------------------------------------------------------
// Buttons, ports, glyphs
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn toggle(
    ui: &Ui,
    p: &Painter,
    id: Id,
    rect: Rect,
    label: &str,
    on: bool,
    color: Color32,
    z: f32,
) -> Response {
    let resp = ui.interact(rect, id, Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand);
    let fill = if on {
        color
    } else if resp.hovered() {
        theme::CONTROL_HI
    } else {
        theme::CONTROL
    };
    p.rect_filled(rect, 3.0 * z, fill);
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        theme::font(9.0 * z),
        if on { theme::INSET } else { theme::TEXT_DIM },
    );
    resp
}

/// A port: round for audio and CV, square for MIDI.
pub fn port(p: &Painter, c: Pos2, connected: bool, hot: bool, color: Color32, midi: bool, z: f32) {
    let r = if hot { 5.5 } else { 4.5 } * z;
    let ring = if connected || hot { color } else { theme::TEXT_FAINT };
    if midi {
        let rect = Rect::from_center_size(c, vec2(r, r) * 1.8);
        p.rect(rect, 1.5 * z, theme::NODE_BODY, Stroke::new(1.4 * z, ring), egui::StrokeKind::Middle);
        if connected {
            p.rect_filled(Rect::from_center_size(c, vec2(4.0, 4.0) * z), 0.5 * z, color);
        }
        return;
    }
    p.circle(c, r, theme::NODE_BODY, Stroke::new(1.4 * z, ring));
    if connected {
        p.circle_filled(c, 2.2 * z, color);
    }
}

/// Polyline of `cycles` periods of a waveform fitted inside `rect`.
pub fn wave_points(rect: Rect, wave: Waveform, shape: f32, cycles: f32, amp: f32, phase: f32, seed: u32) -> Vec<Pos2> {
    let n = (rect.width() / 2.0).clamp(16.0, 240.0) as usize;
    (0..=n)
        .map(|i| {
            let x = i as f32 / n as f32;
            let y = preview_sample(wave, shape, x * cycles + phase, seed) * amp;
            pos2(
                emath::lerp(rect.left()..=rect.right(), x),
                rect.center().y - y * rect.height() * 0.5,
            )
        })
        .collect()
}

pub fn shadow(p: &Painter, rect: Rect, radius: f32, z: f32) {
    let s = Shadow {
        offset: [0, (3.0 * z).clamp(0.0, 127.0) as i8],
        blur: (12.0 * z).clamp(0.0, 255.0) as u8,
        spread: 0,
        color: Color32::from_black_alpha(70),
    };
    p.add(s.as_shape(rect, radius));
}

/// Patch cable between two screen points: a horizontal S-curve.
pub fn cable_shape(from: Pos2, to: Pos2, z: f32) -> CubicBezierShape {
    let dx = ((to.x - from.x).abs() * 0.5).max(40.0 * z);
    CubicBezierShape::from_points_stroke(
        [from, from + vec2(dx, 0.0), to - vec2(dx, 0.0), to],
        false,
        Color32::TRANSPARENT,
        Stroke::NONE,
    )
}

/// `level` (0..1) brightens the cable so active signal paths stand out.
/// MIDI cables are dashed.
#[allow(clippy::too_many_arguments)]
pub fn draw_cable(p: &Painter, from: Pos2, to: Pos2, color: Color32, level: f32, hot: bool, midi: bool, z: f32) {
    let mut s = cable_shape(from, to, z);
    let (width, color) = if hot {
        (2.6, theme::mix(color, Color32::WHITE, 0.3))
    } else {
        (1.8, alpha(color, 0.55 + 0.45 * level.clamp(0.0, 1.0).sqrt()))
    };
    let stroke = Stroke::new(width * z, color);
    if midi {
        let n = ((to - from).length() / (6.0 * z)).clamp(16.0, 160.0) as usize;
        let pts: Vec<Pos2> = (0..=n).map(|i| s.sample(i as f32 / n as f32)).collect();
        p.extend(Shape::dashed_line(&pts, stroke, 6.0 * z, 4.0 * z));
        return;
    }
    s.stroke = stroke.into();
    p.add(s);
}

/// Minimum distance from `pt` to the cable curve.
pub fn cable_distance(from: Pos2, to: Pos2, pt: Pos2, z: f32) -> f32 {
    let shape = cable_shape(from, to, z);
    let mut prev = shape.sample(0.0);
    let mut best = f32::INFINITY;
    for i in 1..=32 {
        let next = shape.sample(i as f32 / 32.0);
        let seg = next - prev;
        let t = ((pt - prev).dot(seg) / seg.length_sq().max(1e-6)).clamp(0.0, 1.0);
        best = best.min((prev + seg * t - pt).length());
        prev = next;
    }
    best
}
