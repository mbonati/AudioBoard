//! Panel of Nullpunkt, kept bare: the output as a barcode of recent peaks
//! up top; then the four voices, each a lane of its Euclidean pattern with
//! a playhead (drag a lane sideways to rotate it) and its knobs; then the
//! glitch stage and the output.

use egui::{Align2, Color32, CursorIcon, Id, Sense, Stroke, Vec2, vec2};

use super::nodes::{NodeView, segmented};
use super::theme::{self, alpha};
use super::widgets;
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::nullpunkt::{
    self as np, CRUSH, CUT, DIVS, LEVEL, MAX_LEN, MUTATE, RATE, SKIP, WIDTH, data, knob, voice, vp, viz,
};
use crate::graph::{ModuleNode, ParamRef};

pub const SIZE: Vec2 = vec2(760.0, 396.0);
const W: f32 = SIZE.x;
const PAD: f32 = 12.0;
/// The display band, the lanes, the knob groups.
const DISPLAY: [f32; 2] = [56.0, 150.0];
const LANES_Y: f32 = 168.0;
const LANE_H: f32 = 36.0;
const CELLS_X: f32 = 66.0;
const CELL: f32 = 10.5;
const VOICE_KNOBS_X: f32 = 440.0;
const GROUPS_Y: f32 = 322.0;

const VOICE_KNOBS: [(usize, &str); knob::COUNT] = [(knob::HITS, "HITS"), (knob::LEN, "LEN"), (knob::TONE, "TONE"), (knob::DECAY, "DECAY"), (knob::LEVEL, "LEVEL")];
const GLITCH_KNOBS: [(usize, &str); 4] = [(CRUSH, "CRUSH"), (RATE, "RATE"), (SKIP, "SKIP"), (CUT, "CUT")];
const OUT_KNOBS: [(usize, &str); 3] = [(MUTATE, "MUTATE"), (WIDTH, "WIDTH"), (LEVEL, "LEVEL")];
/// The bottom groups: title, left and right edges, knobs.
type KnobGroup = (&'static str, f32, f32, &'static [(usize, &'static str)]);
const GROUPS: [KnobGroup; 2] = [("GLITCH", PAD, 420.0, &GLITCH_KNOBS), ("OUT", 440.0, W - PAD, &OUT_KNOBS)];

const TIPS: [&str; 4] = [
    "Clicks: impulses through a resonance (COLOR), ringing for DECAY",
    "High sine blips",
    "Sub-bass sine pulses",
    "1-bit digital noise, held at the GRAIN rate",
];

pub fn readout(p: &ModuleParams) -> (String, String) {
    let lens: Vec<String> = (0..voice::COUNT).map(|v| np::pattern(p, v).1.to_string()).collect();
    let hits: Vec<String> = (0..voice::COUNT).map(|v| np::pattern(p, v).0.to_string()).collect();
    (format!("{} · {}", DIVS[np::division(p)].0, lens.join("·")), format!("{} hits", hits.join("·")))
}

struct Panel<'a, 'b> {
    v: &'a NodeView<'b>,
    id: NodeId,
    p: &'a ModuleParams,
    accent: Color32,
}

/// `out`: the output is patched.
pub fn draw(v: &NodeView, id: NodeId, accent: Color32, m: &ModuleNode, out: bool) {
    let s = Panel { v, id, p: &m.params, accent };
    v.text(W - PAD, 42.0, Align2::RIGHT_CENTER, "OUT", v.font(9.0), if out { theme::TEXT_DIM } else { theme::TEXT_FAINT });
    s.header();
    s.display();
    s.lanes();
    s.groups();
    // Animate while anything moves: the grid, the barcode.
    let p = &m.params;
    let busy = p.viz[viz::STEP].get() >= 0.0 || (0..viz::STRIP_LEN).any(|k| p.viz[viz::STRIP + k].get() > 1e-4);
    if busy {
        v.ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
}

impl Panel<'_, '_> {
    fn z(&self) -> f32 {
        self.v.z
    }

    fn uid(&self, salt: impl std::hash::Hash + std::fmt::Debug) -> Id {
        Id::unique(("nullpunkt", self.id)).with(salt)
    }

    /// The grid's division, and the glitches as they happen.
    fn header(&self) {
        let (v, p) = (self.v, self.p);
        let items = [("1/8", "Eighth notes"), ("1/16", "Sixteenth notes"), ("1/32", "Thirty-second notes")];
        let picked = segmented(v, self.uid("div"), v.r(PAD, 34.0, PAD + 144.0, 50.0), items.len(), np::division(p), self.accent, |k, r, col| {
            v.painter.text(r.center(), Align2::CENTER_CENTER, items[k].0, v.font(8.0), col);
            items[k].1
        });
        if let Some(k) = picked {
            p.data[data::DIV].set(k as f32);
        }
        let playing = p.viz[viz::STEP].get() >= 0.0;
        let state = if playing { "on the board's clock" } else { "waiting for the board's Start" };
        v.text(PAD + 154.0, 42.0, Align2::LEFT_CENTER, state, v.font(8.0), theme::TEXT_FAINT);
        for (k, (at, label)) in [(viz::SKIP, "SKIP"), (viz::CUT, "CUT")].into_iter().enumerate() {
            let on = p.viz[at].get() > 0.5;
            let x = W - 120.0 + 40.0 * k as f32;
            v.text(x, 42.0, Align2::CENTER_CENTER, label, theme::strong(8.0 * self.z()), if on { theme::TEXT } else { alpha(theme::TEXT_FAINT, 0.5) });
        }
    }

    /// The output as a barcode: one bar per recent slice, newest on the
    /// right.
    fn display(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let painter = v.painter;
        let [y0, y1] = DISPLAY;
        painter.rect_filled(v.r(PAD, y0, W - PAD, y1), 4.0 * z, theme::INSET);
        let pos = p.viz[viz::STRIP_POS].get() as usize;
        let n = viz::STRIP_LEN;
        let (x0, x1) = (PAD + 10.0, W - PAD - 10.0);
        let bw = (x1 - x0) / n as f32;
        let mid = (y0 + y1) * 0.5;
        for k in 0..n {
            let at = (pos + 1 + k) % n;
            let peak = p.viz[viz::STRIP + at].get().clamp(0.0, 1.0);
            if peak < 1e-4 {
                continue;
            }
            let h = (peak.sqrt() * (y1 - y0 - 16.0) * 0.5).max(0.5);
            let x = x0 + bw * (k as f32 + 0.5);
            let width = (bw * (0.25 + 0.6 * peak)).max(1.0);
            painter.line_segment([v.p(x, mid - h), v.p(x, mid + h)], Stroke::new(width * z, alpha(theme::TEXT, 0.35 + 0.65 * peak)));
        }
        v.text(x0, y1 - 8.0, Align2::LEFT_CENTER, "OUT", v.font(6.5), theme::TEXT_FAINT);
    }

    /// The four voices: name, pattern lane, knobs.
    fn lanes(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let painter = v.painter;
        for (k, (_, label)) in VOICE_KNOBS.iter().enumerate() {
            v.text(VOICE_KNOBS_X + 62.0 * k as f32, LANES_Y - 6.0, Align2::CENTER_CENTER, label, v.font(7.0), theme::TEXT_FAINT);
        }
        let step = p.viz[viz::STEP].get();
        let now = v.ui.input(|i| i.time);
        let flash_id = self.uid("flash");
        let mut flashes: [(f32, f64); voice::COUNT] = v.ui.data(|d| d.get_temp(flash_id)).unwrap_or([(0.0, 0.0); voice::COUNT]);
        for vc in 0..voice::COUNT {
            let top = LANES_Y + LANE_H * vc as f32;
            let cy = top + LANE_H * 0.5;
            let hits = p.viz[viz::HITS + vc].get();
            if hits != flashes[vc].0 {
                flashes[vc] = (hits, now);
            }
            let glow = (1.0 - ((now - flashes[vc].1) / 0.12) as f32).clamp(0.0, 1.0);
            let name_rect = v.text(PAD, cy, Align2::LEFT_CENTER, voice::NAMES[vc], theme::strong(8.5 * z), theme::mix(theme::TEXT_DIM, theme::TEXT, glow));
            let name = v.ui.interact(name_rect, self.uid(("name", vc)), Sense::hover());
            name.on_hover_text(TIPS[vc]);

            // The lane: one cell per step of its length; hits filled, the
            // playhead ringed. Drag sideways to rotate.
            let (hits_n, len, rot) = np::pattern(p, vc);
            let lane = v.r(CELLS_X, top + 6.0, CELLS_X + CELL * MAX_LEN as f32, top + LANE_H - 6.0);
            let resp = v.ui.interact(lane, self.uid(("lane", vc)), Sense::drag()).on_hover_cursor(CursorIcon::ResizeHorizontal).on_hover_text("Drag sideways to rotate the pattern");
            if resp.dragged() {
                let acc_id = self.uid(("rot", vc));
                let dx = resp.drag_delta().x / z;
                let moved = v.ui.data_mut(|d| {
                    let acc = d.get_temp_mut_or(acc_id, 0.0f32);
                    *acc += dx;
                    let cells = (*acc / CELL).trunc();
                    *acc -= cells * CELL;
                    cells as i32
                });
                if moved != 0 {
                    let r = (rot as i32 + moved).rem_euclid(len as i32);
                    p.data[data::ROT + vc].set(r as f32);
                }
            }
            let at = if step >= 0.0 { Some(np::position(p, vc, step as i64)) } else { None };
            for s in 0..MAX_LEN {
                let c = v.p(CELLS_X + CELL * (s as f32 + 0.5), cy);
                if s >= len {
                    painter.circle_filled(c, 0.8 * z, alpha(theme::TEXT_FAINT, 0.25));
                    continue;
                }
                let hit = np::euclid(hits_n, len, s);
                let here = at == Some(s);
                if hit {
                    let col = if here { self.accent } else { alpha(theme::TEXT, 0.75) };
                    painter.rect_filled(egui::Rect::from_center_size(c, vec2(6.0, 6.0) * z), 1.0 * z, col);
                } else {
                    painter.rect_stroke(egui::Rect::from_center_size(c, vec2(5.0, 5.0) * z), 1.0 * z, Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.6)), egui::StrokeKind::Inside);
                }
                if here {
                    painter.rect_stroke(egui::Rect::from_center_size(c, vec2(9.0, 9.0) * z), 1.5 * z, Stroke::new(1.0 * z, self.accent), egui::StrokeKind::Outside);
                }
            }

            for (k, (kn, _)) in VOICE_KNOBS.iter().enumerate() {
                self.knob(vp(vc, *kn), VOICE_KNOBS_X + 62.0 * k as f32, top + 2.0, None, 8.5);
            }
        }
        v.ui.data_mut(|d| d.insert_temp(flash_id, flashes));
    }

    /// GLITCH and OUT.
    fn groups(&self) {
        let (v, z) = (self.v, self.z());
        for (title, x0, x1, knobs) in GROUPS {
            v.text(x0, GROUPS_Y, Align2::LEFT_CENTER, title, theme::strong(8.5 * z), theme::TEXT_DIM);
            v.painter.hline(v.p(x0 + 48.0, 0.0).x..=v.p(x1, 0.0).x, v.p(0.0, GROUPS_Y).y, Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.3)));
            let cw = (x1 - x0) / knobs.len() as f32;
            for (k, (i, label)) in knobs.iter().enumerate() {
                self.knob(*i, x0 + cw * (k as f32 + 0.5), GROUPS_Y + 16.0, Some(label), 11.0);
            }
        }
    }

    /// A knob at `cx`; its label (if any) at `y`, the knob under it, the
    /// value under that.
    fn knob(&self, i: usize, cx: f32, y: f32, label: Option<&str>, r: f32) {
        let (v, p, z) = (self.v, self.p, self.z());
        let spec = np::spec(i);
        let top = if let Some(label) = label {
            v.text(cx, y, Align2::CENTER_CENTER, label, v.font(7.5), theme::TEXT_FAINT);
            y + 8.0
        } else {
            y
        };
        let mut norm = spec.to_norm(p.get(i));
        let resp = widgets::knob(v.ui, v.painter, self.uid(("knob", i)), v.p(cx, top + r), r * z, &mut norm, spec.to_norm(spec.default), spec.bipolar, self.accent, z);
        v.hit(resp.rect, ParamRef::Module(i));
        if resp.dragged() || resp.double_clicked() {
            let value = spec.denorm(norm);
            p.set(i, if matches!(spec.unit, crate::audio::params::Unit::Integer) { value.round() } else { value });
        }
        let hot = resp.dragged() || resp.hovered();
        v.text(cx, top + 2.0 * r + 8.0, Align2::CENTER_CENTER, np::format(p, i), v.mono(7.0), if hot { theme::TEXT } else { theme::TEXT_DIM });
    }
}
