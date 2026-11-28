//! Panel of Stormo, the swarm synth, drawn flat: dots and thin lines on a
//! dot matrix. On the left the field: the attractor (✕, drag it), the
//! orbit's ring, the boundary, every voice's nodes as dots (white, coral and
//! lilac by group, larger as they sound) linked to their nearest neighbour,
//! the clock trigger's sweep; click in the field to play the root. Above,
//! pills for the voices, the trigger, the root and scale, the preset. On the
//! right fourteen tiles, one per page, each a small dot figure of its
//! setting: click to edit the page, drag up or down to change its main
//! value. Under them the nodes' lanes (each column a node: its pitch, or
//! brightness, or pan), and the page's controls.

use std::f32::consts::TAU;

use egui::{Align2, Color32, CursorIcon, Id, Pos2, Sense, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};

use super::nodes::{NodeView, segmented};
use super::theme::{self, alpha};
use super::widgets;
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::quartz::VOICES as FM_VOICES;
use crate::audio::modules::stormo::swarm::{BOUNDS, FIELDS, MAX_NODES, SHAPES};
use crate::audio::modules::stormo::{
    self as st, ATTACK, BOUNCE, BOUND, BOUND_ON, BRIGHT, CHAOS, COHESION, CUTOFF, DAMPING, DECAY,
    DEPTH, DESTS, DETUNE, DOPPLER, F_ENV, F_MODE, F_MODES, FIELD, FOCUS, GLIDE, GRAVITY,
    MAX_VOICES, NODES, PRESETS, RADIUS, RANGE, RELEASE, RESO, ROOT, SCALE, SCALES, SCATTER, SHAPE,
    SIZE, SPEED, SPIN, STACK, STACKS, SUSTAIN, TILT, TILT_ANGLE, TIMBRE, TRIGGER, TRIGGERS,
    VOICES, VOLUME, WARP, WARP_TYPE, WARPS, WIDTH, X_AMT, X_DEST, Y_AMT, Y_DEST, data, viz,
};
use crate::audio::params::{NOTE_NAMES, Unit, format_hz, format_seconds};
use crate::graph::{ModuleNode, ParamRef};

pub const SIZE_V: Vec2 = vec2(940.0, 500.0);
const W: f32 = SIZE_V.x;
const PAD: f32 = 14.0;
/// The field: a square on the left.
const FX0: f32 = PAD;
const FY0: f32 = 76.0;
const FS: f32 = 400.0;
/// The tiles, the lanes, the page's controls.
const RX0: f32 = FX0 + FS + 22.0;
const RX1: f32 = W - PAD;
const TILE_Y: [f32; 2] = [76.0, 162.0];
const TILE_H: f32 = 82.0;
const LANE_Y: [f32; 2] = [256.0, 336.0];
const EDIT_Y: f32 = 350.0;

/// The nodes' colours, by group.
const GROUPS: [Color32; 3] = [
    Color32::from_rgb(236, 236, 240),
    Color32::from_rgb(255, 138, 128),
    Color32::from_rgb(170, 158, 255),
];
const CORAL: Color32 = GROUPS[1];
const LILAC: Color32 = GROUPS[2];
const DOT: Color32 = Color32::from_rgb(58, 60, 66);

/// The pages: a tile each, with the setting a drag on it changes.
const PAGES: [(&str, usize); 14] = [
    ("NODES", NODES),
    ("TIMBRE", TIMBRE),
    ("FILTER", CUTOFF),
    ("ENVELOPE", ATTACK),
    ("FIELD", GRAVITY),
    ("MOTION", SPEED),
    ("SPACE", WIDTH),
    ("X / Y", Y_AMT),
    ("BOUNDARY", BOUND),
    ("SHAPE", SHAPE),
    ("SIZE", SIZE),
    ("SPIN", SPIN),
    ("TILT", TILT),
    ("WARP", WARP),
];

pub fn readout(p: &ModuleParams) -> (String, String) {
    let k = p.data[data::PRESET].get().round().max(0.0) as usize % PRESETS.len();
    let scale = SCALES[st::int_of(p, SCALE) as usize].0;
    (
        format!("{} · {} / {scale}", PRESETS[k].name, NOTE_NAMES[st::int_of(p, ROOT) as usize]),
        format!("{} nodes · {} voices", st::nodes_per_voice(p), st::int_of(p, VOICES)),
    )
}

struct Panel<'a, 'b> {
    v: &'a NodeView<'b>,
    id: NodeId,
    p: &'a ModuleParams,
    accent: Color32,
}

pub fn draw(v: &NodeView, id: NodeId, accent: Color32, m: &ModuleNode, (midi_in, out): (bool, bool)) {
    let s = Panel {
        v,
        id,
        p: &m.params,
        accent,
    };
    let col = |on: bool| if on { theme::TEXT_DIM } else { theme::TEXT_FAINT };
    v.text(PAD, 42.0, Align2::LEFT_CENTER, "MIDI", v.font(9.0), col(midi_in));
    v.text(W - PAD, 42.0, Align2::RIGHT_CENTER, "OUT", v.font(9.0), col(out));
    s.pills();
    s.field();
    s.tiles();
    s.lanes();
    s.editor();
    v.ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
}

impl Panel<'_, '_> {
    fn z(&self) -> f32 {
        self.v.z
    }

    fn uid(&self, salt: impl std::hash::Hash + std::fmt::Debug) -> Id {
        Id::unique(("stormo", self.id)).with(salt)
    }

    fn idx(&self, i: usize) -> usize {
        st::int_of(self.p, i).max(0) as usize
    }

    fn page(&self) -> usize {
        (self.p.data[data::PAGE].get().round().max(0.0) as usize).min(PAGES.len() - 1)
    }

    fn stroke(&self, w: f32, c: Color32) -> Stroke {
        Stroke::new(w * self.z(), c)
    }

    fn norm(&self, i: usize) -> f32 {
        st::spec(i).to_norm(self.p.get(i))
    }

    fn format(&self, i: usize) -> String {
        let p = self.p;
        let v = p.get(i);
        let n = st::int_of(p, i);
        match i {
            NODES | VOICES => format!("{n}"),
            TIMBRE => FM_VOICES[n as usize % FM_VOICES.len()].name.to_owned(),
            CUTOFF => format_hz(v),
            ATTACK | DECAY | RELEASE => format_seconds(v),
            DETUNE => format!("{v:.0}c"),
            RANGE => format!("±{v:.0} st"),
            SPEED => format!("×{v:.2}"),
            TILT_ANGLE => format!("{v:.0}°"),
            X_AMT | Y_AMT | F_ENV | FOCUS | SPIN | WARP => format!("{:+.0}%", v * 100.0),
            BRIGHT => format!("{:.0}%", v * 100.0),
            _ => match st::spec(i).unit {
                Unit::Integer => format!("{n}"),
                _ => format!("{:.0}%", v * 100.0),
            },
        }
    }

    /// A knob at `cx`: its label at `y`, the knob, the value.
    fn knob(&self, i: usize, cx: f32, y: f32, label: &str) {
        let (v, p, z) = (self.v, self.p, self.z());
        const R: f32 = 11.0;
        let spec = st::spec(i);
        v.text(cx, y, Align2::CENTER_CENTER, label, v.font(7.0), theme::TEXT_FAINT);
        let top = y + 8.0;
        let mut norm = spec.to_norm(p.get(i));
        let resp = widgets::knob(
            v.ui,
            v.painter,
            self.uid(("knob", i)),
            v.p(cx, top + R),
            R * z,
            &mut norm,
            spec.to_norm(spec.default),
            spec.bipolar,
            self.accent,
            z,
        )
        .on_hover_text(spec.name);
        v.hit(resp.rect, ParamRef::Module(i));
        if resp.dragged() || resp.double_clicked() {
            let mut value = spec.denorm(norm);
            if matches!(spec.unit, Unit::Integer | Unit::NoteName) {
                value = value.round();
            }
            if value != p.get(i) {
                p.set(i, value);
            }
        }
        let hot = resp.dragged() || resp.hovered();
        v.text(cx, top + 2.0 * R + 8.0, Align2::CENTER_CENTER, self.format(i), v.mono(7.0), if hot { theme::TEXT } else { theme::TEXT_DIM });
    }

    fn choices(&self, i: usize, x0: f32, y0: f32, x1: f32, y1: f32, names: &[&'static str]) {
        let (v, p) = (self.v, self.p);
        let sel = self.idx(i).min(names.len() - 1);
        let picked = segmented(v, self.uid(("seg", i)), v.r(x0, y0, x1, y1), names.len(), sel, self.accent, |k, r, col| {
            v.painter.text(r.center(), Align2::CENTER_CENTER, names[k], v.font(7.0), col);
            st::spec(i).name
        });
        if let Some(k) = picked {
            p.set(i, k as f32);
        }
    }

    // --- Pills --------------------------------------------------------------

    /// A pill: click steps on (right half) or back (left half).
    fn pill(&self, salt: &str, x1: f32, text: &str) -> (f32, Option<i32>) {
        let (v, z) = (self.v, self.z());
        let w = text.chars().count() as f32 * 6.4 + 18.0;
        let x0 = x1 - w;
        let r = v.r(x0, 50.0, x1, 66.0);
        let resp = v.ui.interact(r, self.uid(("pill", salt)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        v.painter.rect_filled(r, 2.0 * z, if resp.hovered() { theme::CONTROL } else { alpha(theme::CONTROL, 0.6) });
        v.painter.text(r.center(), Align2::CENTER_CENTER, text, v.font(8.5), if resp.hovered() { theme::TEXT } else { theme::TEXT_DIM });
        let step = resp.clicked().then(|| {
            let x = resp.interact_pointer_pos().map_or(r.right(), |p| p.x);
            if x < r.center().x { -1 } else { 1 }
        });
        let step = step.or_else(|| resp.secondary_clicked().then_some(-1));
        (x0 - 6.0, step)
    }

    fn pills(&self) {
        let p = self.p;
        let wrap = |i: usize, d: i32| {
            let s = st::spec(i);
            let span = (s.max - s.min) as i32 + 1;
            let v = (st::int_of(p, i) - s.min as i32 + d).rem_euclid(span) + s.min as i32;
            p.set(i, v as f32);
        };
        let k = p.data[data::PRESET].get().round().max(0.0) as usize % PRESETS.len();
        let (x, step) = self.pill("preset", RX1, &format!("{}  ← →", PRESETS[k].name));
        if let Some(d) = step {
            st::load_preset(p, (k as i32 + d).rem_euclid(PRESETS.len() as i32) as usize);
        }
        let scale = SCALES[self.idx(SCALE)].0;
        let (x, step) = self.pill("scale", x, &format!("{} / {scale}", NOTE_NAMES[self.idx(ROOT)]));
        if let Some(d) = step {
            if d < 0 { wrap(ROOT, 1) } else { wrap(SCALE, 1) }
        }
        let (x, step) = self.pill("trigger", x, &format!("Trigger → {}", TRIGGERS[self.idx(TRIGGER)]));
        if let Some(d) = step {
            wrap(TRIGGER, d);
        }
        let (_, step) = self.pill("voices", x, &format!("Voices → {}", self.idx(VOICES)));
        if let Some(d) = step {
            wrap(VOICES, d);
        }
        let v = self.v;
        v.text(PAD + 44.0, 58.0, Align2::LEFT_CENTER, "STORMO", theme::strong(10.0 * self.z()), theme::TEXT);
        v.text(PAD + 98.0, 58.0, Align2::LEFT_CENTER, "swarm", v.font(9.0), theme::TEXT_FAINT);
        v.painter.hline(v.p(PAD, 0.0).x..=v.p(RX1, 0.0).x, v.p(0.0, 70.0).y, self.stroke(1.0, alpha(theme::TEXT_FAINT, 0.25)));
    }

    // --- The field ------------------------------------------------------------

    /// A field position (−1..1) on screen.
    fn fp(&self, x: f32, y: f32) -> Pos2 {
        let h = FS * 0.5;
        self.v.p(FX0 + h + x * h * 0.94, FY0 + h + y * h * 0.94)
    }

    fn field(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let rect = v.r(FX0, FY0, FX0 + FS, FY0 + FS);
        // The dot matrix.
        let step = FS / 40.0;
        for i in 0..=40 {
            for j in 0..=40 {
                v.painter.circle_filled(v.p(FX0 + step * i as f32, FY0 + step * j as f32), 0.7 * z, DOT);
            }
        }
        // The boundary.
        if self.idx(BOUND_ON) > 0 {
            let ink = alpha(theme::TEXT_FAINT, 0.5);
            match self.idx(BOUND) {
                0 => {
                    for (cx, cy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                        let c = self.fp(cx, cy);
                        let l = 12.0 * z;
                        v.painter.line_segment([c, c + vec2(-cx * l, 0.0)], self.stroke(1.0, ink));
                        v.painter.line_segment([c, c + vec2(0.0, -cy * l)], self.stroke(1.0, ink));
                    }
                }
                1 => dashed_circle(v.painter, self.fp(0.0, 0.0), FS * 0.47 * z, 48, self.stroke(1.0, ink)),
                _ => {
                    let r = egui::Rect::from_two_pos(self.fp(-1.0, -1.0), self.fp(1.0, 1.0));
                    for (a, b) in [(r.left_top(), r.right_top()), (r.right_top(), r.right_bottom()), (r.right_bottom(), r.left_bottom()), (r.left_bottom(), r.left_top())] {
                        dashed_line(v.painter, a, b, 24, self.stroke(1.0, ink));
                    }
                }
            }
        }
        let (ax, ay) = (p.viz[viz::AX].get(), p.viz[viz::AY].get());
        // The orbit's ring.
        let radius = (0.08 + 0.8 * p.get(RADIUS)) * FS * 0.47 * z;
        v.painter.circle_stroke(self.fp(ax, ay), radius, self.stroke(1.0, alpha(LILAC, 0.75)));

        // The nodes: every voice, else the idle swarm.
        let count = (p.viz[viz::COUNT].get().max(1.0) as usize).min(MAX_NODES);
        let mut any = false;
        for vi in 0..MAX_VOICES {
            let base = viz::VOICE + vi * viz::VOICE_LEN;
            if p.viz[base].get() < 0.5 {
                continue;
            }
            any = true;
            let nodes: Vec<(Pos2, f32)> = (0..count)
                .map(|i| {
                    let at = base + 3 + i * 3;
                    (self.fp(p.viz[at].get(), p.viz[at + 1].get()), p.viz[at + 2].get())
                })
                .collect();
            self.swarm(&nodes, 1.0);
        }
        if !any {
            let nodes: Vec<(Pos2, f32)> = (0..count)
                .map(|i| (self.fp(p.viz[viz::GHOST + i * 2].get(), p.viz[viz::GHOST + i * 2 + 1].get()), 0.0))
                .collect();
            self.swarm(&nodes, 0.75);
        }

        // The sweep.
        let sweep = p.viz[viz::SWEEP].get();
        if sweep >= 0.0 {
            let a = (sweep + 0.5 / 16.0) * TAU - TAU * 0.25;
            let c = self.fp(ax, ay);
            let tip = c + vec2(a.cos(), a.sin()) * FS * 0.47 * z;
            dashed_line(v.painter, c, tip, 16, self.stroke(1.0, alpha(CORAL, 0.6)));
        }

        // The attractor: drag it; elsewhere, a click plays the root.
        let c = self.fp(ax, ay);
        let grab = egui::Rect::from_center_size(c, vec2(18.0, 18.0) * z);
        let gresp = v.ui.interact(grab, self.uid("attractor"), Sense::drag()).on_hover_cursor(CursorIcon::Grab).on_hover_text("The attractor: drag it");
        if gresp.dragged() {
            let d = gresp.drag_delta() / (FS * 0.5 * 0.94 * z);
            let nx = (p.data[data::ATTR_X].get() + d.x).clamp(-0.9, 0.9);
            let ny = (p.data[data::ATTR_Y].get() + d.y).clamp(-0.9, 0.9);
            p.data[data::ATTR_X].set(nx);
            p.data[data::ATTR_Y].set(ny);
        }
        if gresp.double_clicked() {
            p.data[data::ATTR_X].set(0.0);
            p.data[data::ATTR_Y].set(0.0);
        }
        let s = 6.0 * z;
        let ink = if gresp.hovered() || gresp.dragged() { theme::TEXT } else { theme::TEXT_DIM };
        v.painter.line_segment([c + vec2(-s, -s), c + vec2(s, s)], self.stroke(1.4, ink));
        v.painter.line_segment([c + vec2(-s, s), c + vec2(s, -s)], self.stroke(1.4, ink));

        let resp = v.ui.interact(rect, self.uid("field"), Sense::click_and_drag());
        let held = resp.is_pointer_button_down_on() && !gresp.dragged() && !gresp.is_pointer_button_down_on();
        let key = 60 + self.idx(ROOT) as i32;
        let want = if held { key as f32 + 1.0 } else { 0.0 };
        if p.viz[viz::PANEL_KEY].get() != want {
            p.viz[viz::PANEL_KEY].set(want);
        }
        if !any {
            v.text(FX0 + FS * 0.5, FY0 + FS - 10.0, Align2::CENTER_CENTER, "play, or hold the field", v.font(7.5), theme::TEXT_FAINT);
        }
    }

    /// A swarm's dots, each linked to its nearest neighbour.
    fn swarm(&self, nodes: &[(Pos2, f32)], fade: f32) {
        let (v, z) = (self.v, self.z());
        let reach = FS * 0.3 * z;
        for (i, &(a, la)) in nodes.iter().enumerate() {
            let near = nodes
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .map(|(j, &(b, _))| (j, a.distance(b)))
                .min_by(|x, y| x.1.total_cmp(&y.1));
            if let Some((_, d)) = near.filter(|n| n.1 < reach)
                && let Some(&(b, _)) = near.map(|n| &nodes[n.0])
            {
                let strength = (1.0 - d / reach) * (0.35 + 0.65 * la.min(1.0));
                v.painter.line_segment([a, b], self.stroke(1.0, alpha(CORAL, strength * 0.8 * fade)));
            }
        }
        for (i, &(c, l)) in nodes.iter().enumerate() {
            let col = GROUPS[i % 3];
            let r = (2.4 + 2.6 * l.clamp(0.0, 1.0)) * z;
            v.painter.circle_filled(c, r, alpha(col, (0.45 + 0.55 * l.clamp(0.0, 1.0)) * fade.max(0.3)));
        }
    }

    // --- Tiles ------------------------------------------------------------------

    fn tiles(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let cw = (RX1 - RX0) / 7.0;
        let page = self.page();
        for (k, &(name, macro_)) in PAGES.iter().enumerate() {
            let (c, r) = (k % 7, k / 7);
            let x0 = RX0 + cw * c as f32;
            let y0 = TILE_Y[r];
            let rect = v.r(x0, y0, x0 + cw, y0 + TILE_H);
            let resp = v.ui.interact(rect, self.uid(("tile", k)), Sense::click_and_drag()).on_hover_cursor(CursorIcon::ResizeVertical);
            let on = k == page;
            if on {
                v.painter.rect_filled(rect, 2.0 * z, alpha(theme::CONTROL, 0.8));
            } else if resp.hovered() {
                v.painter.rect_filled(rect, 2.0 * z, alpha(theme::CONTROL, 0.35));
            }
            if resp.clicked() || resp.drag_started() {
                p.data[data::PAGE].set(k as f32);
            }
            if resp.dragged() {
                let spec = st::spec(macro_);
                let n = (spec.to_norm(p.get(macro_)) - resp.drag_delta().y / (160.0 * z)).clamp(0.0, 1.0);
                let mut value = spec.denorm(n);
                if matches!(spec.unit, Unit::Integer | Unit::NoteName) {
                    value = value.round();
                }
                p.set(macro_, value);
            }
            if resp.double_clicked() {
                p.set(macro_, st::spec(macro_).default);
            }
            v.hit(rect, ParamRef::Module(macro_));
            let cx = x0 + cw * 0.5;
            self.icon(k, cx, y0 + 30.0, on || resp.hovered());
            let label = match k {
                0 => format!("1 → {}", st::nodes_per_voice(p)),
                _ => name.to_owned(),
            };
            let lr = v.text(cx, y0 + 64.0, Align2::CENTER_CENTER, &label, v.font(9.0), if on { theme::TEXT } else { theme::TEXT_DIM });
            if on {
                v.painter.hline(lr.x_range(), lr.bottom() + 2.0 * z, self.stroke(1.0, theme::TEXT));
            }
            let _ = resp.on_hover_text(format!("{name}: click to edit, drag to change {}", st::spec(macro_).name));
        }
    }

    /// Tile `k`'s little figure of dots, centred on (cx, cy).
    fn icon(&self, k: usize, cx: f32, cy: f32, hot: bool) {
        let (v, p, z) = (self.v, self.p, self.z());
        let dim = if hot { theme::TEXT_DIM } else { alpha(theme::TEXT_FAINT, 0.8) };
        let dot = |x: f32, y: f32, c: Color32| v.painter.circle_filled(v.p(cx + x, cy + y), 2.6 * z, c);
        let n = |i: usize| self.norm(i);
        match k {
            0 => {
                let count = st::nodes_per_voice(p).min(9);
                for i in 0..count {
                    let a = i as f32 / count as f32 * TAU;
                    let r = if count == 1 { 0.0 } else { 9.0 };
                    dot(a.cos() * r, a.sin() * r, GROUPS[i % 3]);
                }
            }
            1 => {
                for i in 0..7 {
                    let t = i as f32 / 6.0;
                    let y = (t * TAU).sin() * 8.0 * (0.3 + n(BRIGHT));
                    dot(-15.0 + 30.0 * t, -y, if i % 3 == 0 { CORAL } else { dim });
                }
            }
            2 => {
                let c = n(CUTOFF);
                for i in 0..7 {
                    let t = i as f32 / 6.0;
                    let y = if t < c { -4.0 } else { -4.0 + (t - c) * 40.0 };
                    dot(-15.0 + 30.0 * t, y.min(10.0), dim);
                }
                dot(-26.0, 0.0, CORAL);
            }
            3 => {
                let pts = [(-15.0, 9.0), (-15.0 + 10.0 * n(ATTACK), -9.0), (0.0, -9.0 + 16.0 * (1.0 - p.get(SUSTAIN))), (10.0, -9.0 + 16.0 * (1.0 - p.get(SUSTAIN))), (15.0, 9.0)];
                for (x, y) in pts {
                    dot(x, y, dim);
                }
            }
            4 => {
                let r = 4.0 + 10.0 * (1.0 - n(GRAVITY));
                for i in 0..5 {
                    let a = i as f32 / 5.0 * TAU;
                    dot(a.cos() * r, a.sin() * r, if i == 0 { LILAC } else { dim });
                }
            }
            5 => {
                let s = 3.0 + 6.0 * n(SPEED);
                for i in 0..4 {
                    dot(-12.0 + s * i as f32, (i as f32 * 1.7).sin() * 6.0 * n(CHAOS), dim);
                }
            }
            6 => {
                let w = 4.0 + 12.0 * n(WIDTH);
                dot(-w, 0.0, dim);
                dot(w, 0.0, dim);
                dot(0.0, -5.0, CORAL);
            }
            7 => {
                dot(-8.0 * p.get(X_AMT), 0.0, dim);
                dot(8.0 * p.get(X_AMT), 0.0, dim);
                dot(0.0, -8.0 * p.get(Y_AMT), LILAC);
                dot(0.0, 8.0 * p.get(Y_AMT), dim);
            }
            8 => {
                let on = self.idx(BOUND_ON) > 0;
                v.text(cx, cy - 9.0, Align2::CENTER_CENTER, if on { "On" } else { "Off" }, v.font(9.0), if on { theme::TEXT } else { theme::TEXT_FAINT });
                v.painter.hline(v.p(cx - 16.0, 0.0).x..=v.p(cx + 16.0, 0.0).x, v.p(0.0, cy + 1.0).y, self.stroke(1.0, dim));
                v.text(cx, cy + 11.0, Align2::CENTER_CENTER, BOUNDS[self.idx(BOUND).min(2)], v.font(8.0), theme::TEXT_DIM);
            }
            9 => {
                let shape = self.idx(SHAPE);
                for i in 0..6 {
                    let t = i as f32 / 6.0;
                    let (x, y) = match shape {
                        0 => ((i as f32 * 2.4).cos() * 5.0, (i as f32 * 2.4).sin() * 5.0),
                        1 => ((t * TAU).cos() * 10.0, (t * TAU).sin() * 10.0),
                        2 => (-14.0 + 28.0 * t, 0.0),
                        3 => (-6.0 + 12.0 * (i % 3) as f32 / 2.0 * 1.0, if i < 3 { -5.0 } else { 5.0 }),
                        4 => ((t * TAU * 1.5).cos() * (3.0 + 10.0 * t), (t * TAU * 1.5).sin() * (3.0 + 10.0 * t)),
                        _ => ((i as f32 * 7.3).sin() * 12.0, (i as f32 * 3.1).cos() * 9.0),
                    };
                    dot(x, y, if i == 0 { CORAL } else { dim });
                }
            }
            10 => {
                let s = 3.0 + 9.0 * n(SIZE);
                for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                    dot(x * s, y * s, dim);
                }
            }
            11 => {
                let spin = p.get(SPIN);
                for i in 0..4 {
                    let a = -TAU * 0.25 + spin * i as f32 * 0.8;
                    dot(a.cos() * 10.0, a.sin() * 10.0, if i == 3 { LILAC } else { dim });
                }
            }
            12 => {
                let a = p.get(TILT_ANGLE).to_radians() - TAU * 0.25;
                let t = p.get(TILT);
                for i in 0..3 {
                    let r = -10.0 + 10.0 * i as f32;
                    dot(a.cos() * r * (0.3 + t), a.sin() * r * (0.3 + t), dim);
                }
            }
            _ => {
                let (amt, kind) = (p.get(WARP), self.idx(WARP_TYPE));
                for i in 0..5 {
                    let x = -1.0 + 0.5 * i as f32;
                    let y = st::warp(x, amt, kind);
                    dot(x * 14.0, -y * 10.0, if i == 4 { LILAC } else { dim });
                }
            }
        }
    }

    // --- Lanes ------------------------------------------------------------------

    fn lanes(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let (y0, y1) = (LANE_Y[0], LANE_Y[1]);
        let count = (p.viz[viz::COUNT].get().max(1.0) as usize).min(MAX_NODES);
        let page = self.page();
        // What the lanes show: brightness for the tone pages, pan for space.
        let (field, label, lo, hi) = match page {
            1 | 2 => (1, "BRIGHT", 0.0, 2.0),
            6 => (2, "PAN", -1.0, 1.0),
            _ => {
                let r = p.get(RANGE).max(1.0) + 12.0;
                (0, "PITCH", -r, r)
            }
        };
        // The matrix behind.
        let cols = 48;
        let w = RX1 - RX0;
        for i in 0..cols {
            for j in 0..6 {
                let x = RX0 + w * (i as f32 + 0.5) / cols as f32;
                v.painter.circle_filled(v.p(x, y0 + (y1 - y0) * j as f32 / 5.0), 0.6 * z, DOT);
            }
        }
        let mid = y0 + (y1 - y0) * (hi / (hi - lo));
        let playing = (0..MAX_VOICES).any(|vi| p.viz[viz::VOICE + vi * viz::VOICE_LEN].get() > 0.5);
        for i in 0..cols {
            let x = RX0 + w * (i as f32 + 0.5) / cols as f32;
            v.painter.rect_filled(egui::Rect::from_center_size(v.p(x, mid), vec2(2.2, 2.2) * z), 0.0, alpha(theme::TEXT, 0.65));
        }
        if playing {
            for i in 0..count {
                let val = p.viz[viz::LANES + i * viz::LANE_LEN + field].get();
                let t = ((val - lo) / (hi - lo)).clamp(0.0, 1.0);
                let col = (i as f32 + 0.5) / count as f32;
                let x = RX0 + w * col;
                let y = y1 - (y1 - y0) * t;
                v.painter.rect_filled(egui::Rect::from_center_size(v.p(x, y), vec2(3.6, 3.6) * z), 0.0, GROUPS[i % 3]);
            }
        }
        v.text(RX0, y0 - 6.0, Align2::LEFT_CENTER, label, v.font(7.0), theme::TEXT_FAINT);
    }

    // --- The page's controls -----------------------------------------------------

    fn editor(&self) {
        let p = self.p;
        let y = EDIT_Y;
        let x0 = RX0;
        let row = |items: &[(usize, &str)], from: f32| {
            let cw = 74.0;
            for (k, &(i, label)) in items.iter().enumerate() {
                self.knob(i, from + cw * (k as f32 + 0.5), y + 50.0, label);
            }
        };
        let v = self.v;
        v.text(x0, y + 6.0, Align2::LEFT_CENTER, PAGES[self.page()].0, theme::strong(8.5 * self.z()), theme::TEXT_DIM);
        match self.page() {
            0 => {
                row(&[(NODES, "NODES"), (VOICES, "VOICES"), (VOLUME, "VOLUME")], x0);
                let n = st::nodes_per_voice(p);
                let msg = if n < st::int_of(p, NODES) as usize {
                    format!("{n} nodes a voice: {} in all at most", st::NODE_BUDGET)
                } else {
                    format!("{n} FM nodes a voice")
                };
                v.text(x0 + 240.0, y + 70.0, Align2::LEFT_CENTER, &msg, v.font(7.5), theme::TEXT_FAINT);
            }
            1 => {
                self.choices(STACK, x0 + 60.0, y, x0 + 380.0, y + 16.0, &STACKS);
                row(&[(TIMBRE, "FM VOICE"), (BRIGHT, "BRIGHT"), (DETUNE, "DETUNE")], x0);
            }
            2 => {
                self.choices(F_MODE, x0 + 60.0, y, x0 + 200.0, y + 16.0, &F_MODES);
                row(&[(CUTOFF, "CUTOFF"), (RESO, "RESO"), (F_ENV, "ENV"), (FOCUS, "FOCUS")], x0);
            }
            3 => row(&[(ATTACK, "ATTACK"), (DECAY, "DECAY"), (SUSTAIN, "SUSTAIN"), (RELEASE, "RELEASE"), (SCATTER, "SCATTER")], x0),
            4 => {
                self.choices(FIELD, x0 + 60.0, y, x0 + 340.0, y + 16.0, &FIELDS);
                row(&[(GRAVITY, "GRAVITY"), (RADIUS, "RADIUS"), (COHESION, "COHESION")], x0);
            }
            5 => row(&[(SPEED, "SPEED"), (CHAOS, "CHAOS"), (DAMPING, "DAMPING")], x0),
            6 => row(&[(WIDTH, "WIDTH"), (DEPTH, "DEPTH"), (DOPPLER, "DOPPLER")], x0),
            7 => {
                v.text(x0 + 60.0, y + 8.0, Align2::LEFT_CENTER, "X", v.font(8.0), theme::TEXT_DIM);
                self.choices(X_DEST, x0 + 72.0, y, x0 + 260.0, y + 16.0, &DESTS);
                v.text(x0 + 272.0, y + 8.0, Align2::LEFT_CENTER, "Y", v.font(8.0), theme::TEXT_DIM);
                self.choices(Y_DEST, x0 + 284.0, y, x0 + 472.0, y + 16.0, &DESTS);
                row(&[(X_AMT, "X AMOUNT"), (Y_AMT, "Y AMOUNT"), (RANGE, "RANGE"), (GLIDE, "GLIDE")], x0);
            }
            8 => {
                self.choices(BOUND_ON, x0 + 60.0, y, x0 + 140.0, y + 16.0, &["OFF", "ON"]);
                self.choices(BOUND, x0 + 150.0, y, x0 + 330.0, y + 16.0, &BOUNDS);
                row(&[(BOUNCE, "BOUNCE")], x0);
            }
            9 => {
                self.choices(SHAPE, x0 + 60.0, y, x0 + 440.0, y + 16.0, &SHAPES);
                v.text(x0 + 10.0, y + 70.0, Align2::LEFT_CENTER, "how a new note lays its nodes out round the ✕", v.font(7.5), theme::TEXT_FAINT);
            }
            10 => row(&[(SIZE, "SIZE")], x0),
            11 => row(&[(SPIN, "SPIN")], x0),
            12 => row(&[(TILT, "TILT"), (TILT_ANGLE, "ANGLE")], x0),
            _ => {
                self.choices(WARP_TYPE, x0 + 60.0, y, x0 + 240.0, y + 16.0, &WARPS);
                row(&[(WARP, "WARP")], x0);
            }
        }
        let _ = (pos2, Shape::Noop, StrokeKind::Inside);
    }
}

fn dashed_circle(painter: &egui::Painter, c: Pos2, r: f32, dashes: usize, stroke: Stroke) {
    for i in 0..dashes {
        let a0 = i as f32 / dashes as f32 * TAU;
        let a1 = a0 + TAU / dashes as f32 * 0.5;
        painter.line_segment([c + vec2(a0.cos(), a0.sin()) * r, c + vec2(a1.cos(), a1.sin()) * r], stroke);
    }
}

fn dashed_line(painter: &egui::Painter, a: Pos2, b: Pos2, dashes: usize, stroke: Stroke) {
    for i in 0..dashes {
        let t0 = i as f32 / dashes as f32;
        let t1 = t0 + 0.5 / dashes as f32;
        painter.line_segment([a + (b - a) * t0, a + (b - a) * t1], stroke);
    }
}
