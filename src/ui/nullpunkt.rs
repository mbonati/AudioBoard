//! Panel of Nullpunkt, kept bare: the grid's division, RANDOMIZE and the
//! presets up top, with the glitches lighting as they happen; the output as
//! a barcode of recent peaks; the four voices, each a lane of steps (click
//! to toggle, drag to paint, Shift-drag to rotate) with its mute and knobs;
//! then the glitch stage and the output.

use std::sync::Arc;

use egui::{Align2, Color32, CursorIcon, Id, Sense, Stroke, Vec2, vec2};

use super::nodes::{NodeView, segmented};
use super::theme::{self, alpha};
use super::widgets;
use crate::audio::engine::NodeId;
use crate::audio::modules::nullpunkt::{
    self as np, CRUSH, CUT, DIVS, FACTORY, GRAIN, JITTER, LEVEL, MAX_LEN, MUTATE, RATE, REVERSE, SCATTER, SKIP, STATIC, STUTTER,
    STUTTER_LEN, WARP, WIDTH, data, glitch, knob, voice, vp, viz,
};
use crate::audio::modules::{ModuleKind, ModuleParams};
use crate::graph::{ModuleNode, ParamRef};
use crate::presets::{self, ModulePreset};

pub const SIZE: Vec2 = vec2(760.0, 446.0);
const W: f32 = SIZE.x;
const PAD: f32 = 12.0;
/// The display band, the lanes, the knob groups.
const DISPLAY: [f32; 2] = [56.0, 150.0];
const LANES_Y: f32 = 168.0;
const LANE_H: f32 = 36.0;
const CELLS_X: f32 = 66.0;
const CELL: f32 = 10.5;
const VOICE_KNOBS_X: f32 = 440.0;
const VOICE_KNOB_DX: f32 = 52.0;

const VOICE_KNOBS: [(usize, &str); knob::COUNT] =
    [(knob::HITS, "HITS"), (knob::LEN, "LEN"), (knob::PROB, "PROB"), (knob::TONE, "TONE"), (knob::DECAY, "DECAY"), (knob::LEVEL, "LEVEL")];
const STUTTER_KNOBS: [(usize, &str); 4] = [(STUTTER, "CHANCE"), (STUTTER_LEN, "LENGTH"), (GRAIN, "GRAIN"), (SCATTER, "SCATTER")];
const PLAY_KNOBS: [(usize, &str); 4] = [(MUTATE, "MUTATE"), (JITTER, "JITTER"), (WIDTH, "WIDTH"), (LEVEL, "LEVEL")];
const GLITCH_KNOBS: [(usize, &str); 7] =
    [(SKIP, "SKIP"), (REVERSE, "REVERSE"), (WARP, "WARP"), (CUT, "CUT"), (STATIC, "STATIC"), (CRUSH, "CRUSH"), (RATE, "RATE")];
/// The bottom groups: title, left and right edges, the row's top, knobs.
type KnobGroup = (&'static str, f32, f32, f32, &'static [(usize, &'static str)]);
const GROUPS: [KnobGroup; 3] = [
    ("STUTTER", PAD, 370.0, 322.0, &STUTTER_KNOBS),
    ("PLAY", 390.0, W - PAD, 322.0, &PLAY_KNOBS),
    ("GLITCH", PAD, W - PAD, 384.0, &GLITCH_KNOBS),
];
/// The glitch lights in the header: what lights them, and their names.
const LIGHTS: [(usize, &str); 6] = [
    (glitch::STUTTER, "STUTTER"),
    (glitch::SKIP, "SKIP"),
    (glitch::REVERSE, "REV"),
    (glitch::WARP, "WARP"),
    (glitch::CUT, "CUT"),
    (usize::MAX, "STATIC"),
];

const TIPS: [&str; 4] = [
    "Clicks: impulses through a resonance (COLOR), ringing for DECAY",
    "High sine blips",
    "Sub-bass sine pulses",
    "1-bit digital noise, held at the GRAIN rate",
];
const KNOB_TIPS: [(usize, &str); 14] = [
    (STUTTER, "The chance a step starts a stutter"),
    (STUTTER_LEN, "Up to how many steps a stutter lasts"),
    (GRAIN, "How fine its grains can go: subdivisions of the step, triplets too"),
    (SCATTER, "Grains an octave up or down, backwards or from earlier; stutters start afresh"),
    (JITTER, "Each hit a little different: level, tone, decay"),
    (SKIP, "A step sticks on a slice of what just played"),
    (REVERSE, "A step plays the one before it backwards"),
    (WARP, "A step loops a slice an octave up or down"),
    (CUT, "A step drops to silence"),
    (STATIC, "Bursts of digital errors"),
    (CRUSH, "Fewer bits"),
    (RATE, "Fewer samples: each held for longer"),
    (MUTATE, "Steps flip now and then; hits ratchet"),
    (WIDTH, "Each hit lands somewhere in the stereo field"),
];

pub fn readout(p: &ModuleParams) -> (String, String) {
    let lens: Vec<String> = (0..voice::COUNT).map(|v| np::pattern(p, v).1.to_string()).collect();
    let hits: Vec<String> =
        (0..voice::COUNT).map(|v| if np::muted(p, v) { "–".to_owned() } else { np::steps(p, v).count_ones().to_string() }).collect();
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

/// Where the session keeps the saved presets.
fn saved_id() -> Id {
    Id::unique("nullpunkt-presets")
}

/// The saved presets, read once and kept for the session.
fn saved(ui: &egui::Ui) -> Arc<Vec<ModulePreset>> {
    let id = saved_id();
    if let Some(list) = ui.data(|d| d.get_temp::<Arc<Vec<ModulePreset>>>(id)) {
        return list;
    }
    let list = Arc::new(presets::dir().map(|dir| presets::load(&dir, ModuleKind::Nullpunkt)).unwrap_or_default());
    ui.data_mut(|d| d.insert_temp(id, list.clone()));
    list
}

/// Writes the saved presets back, and keeps them for the session.
fn keep(ui: &egui::Ui, list: Vec<ModulePreset>) -> Result<(), String> {
    let dir = presets::dir().ok_or("no folder for presets")?;
    presets::save(&dir, ModuleKind::Nullpunkt, &list)?;
    ui.data_mut(|d| d.insert_temp(saved_id(), Arc::new(list)));
    Ok(())
}

impl Panel<'_, '_> {
    fn z(&self) -> f32 {
        self.v.z
    }

    fn uid(&self, salt: impl std::hash::Hash + std::fmt::Debug) -> Id {
        Id::unique(("nullpunkt", self.id)).with(salt)
    }

    /// The grid's division, RANDOMIZE, the presets, and the glitches as
    /// they happen.
    fn header(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        const TIPS: [&str; 8] = [
            "Quarter notes",
            "Eighth notes",
            "Eighth-note triplets",
            "Sixteenth notes",
            "Sixteenth-note triplets",
            "Thirty-second notes",
            "Thirty-second-note triplets",
            "Sixty-fourth notes",
        ];
        let picked = segmented(v, self.uid("div"), v.r(PAD, 34.0, PAD + 216.0, 50.0), DIVS.len(), np::division(p), self.accent, |k, r, col| {
            v.painter.text(r.center(), Align2::CENTER_CENTER, DIVS[k].0, v.font(7.0), col);
            TIPS[k]
        });
        if let Some(k) = picked {
            p.data[data::DIV].set(k as f32);
        }

        let dice = widgets::toggle(v.ui, v.painter, self.uid("random"), v.r(236.0, 34.0, 316.0, 50.0), "RANDOMIZE", false, self.accent, z)
            .on_hover_text("New patterns and sounds for the voices not muted · Shift: patterns only");
        if dice.clicked() {
            let now = v.ui.input(|i| i.time);
            let only_patterns = v.ui.input(|i| i.modifiers.shift);
            np::randomize(p, (now * 1000.0) as u32 ^ self.id.wrapping_mul(0x9e37_79b9), !only_patterns);
        }
        let menu = widgets::toggle(v.ui, v.painter, self.uid("presets"), v.r(322.0, 34.0, 402.0, 50.0), "PRESETS ▾", false, self.accent, z)
            .on_hover_text("Save this scene as a preset, or load one");
        self.presets_menu(&menu);

        let playing = p.viz[viz::STEP].get() >= 0.0;
        let state = if playing { "in time" } else { "waiting for Start" };
        v.text(410.0, 42.0, Align2::LEFT_CENTER, state, v.font(7.5), theme::TEXT_FAINT);
        let now = p.viz[viz::GLITCH].get().round() as usize;
        let crackle = p.viz[viz::STATIC].get() > 0.5;
        for (k, (at, label)) in LIGHTS.into_iter().enumerate() {
            let on = if at == usize::MAX { crackle } else { at == now };
            let x = W - 252.0 + 38.0 * k as f32;
            v.text(x, 42.0, Align2::CENTER_CENTER, label, theme::strong(7.5 * z), if on { theme::TEXT } else { alpha(theme::TEXT_FAINT, 0.5) });
        }
    }

    /// Save the scene under a name; load a built-in or a saved preset;
    /// delete a saved one.
    fn presets_menu(&self, button: &egui::Response) {
        let p = self.p;
        let name_id = self.uid("preset-name");
        let error_id = self.uid("preset-error");
        egui::Popup::menu(button).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.set_min_width(200.0);
            ui.label("Save this scene as");
            let mut name: String = ui.data(|d| d.get_temp(name_id)).unwrap_or_else(|| "My scene".to_owned());
            let mut save = false;
            ui.horizontal(|ui| {
                let field = ui.text_edit_singleline(&mut name);
                save = ui.button("Save").clicked() || (field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            });
            ui.data_mut(|d| d.insert_temp(name_id, name.clone()));
            let list = saved(ui);
            if save && !name.trim().is_empty() {
                let mut next = (*list).clone();
                presets::store(&mut next, ModulePreset::capture(p, name.trim()));
                let result = keep(ui, next);
                ui.data_mut(|d| d.insert_temp(error_id, result.err().unwrap_or_default()));
            }
            if let Some(error) = ui.data(|d| d.get_temp::<String>(error_id)).filter(|e| !e.is_empty()) {
                ui.colored_label(theme::MUTE, format!("Could not save: {error}"));
            }
            ui.separator();
            ui.weak("Built in");
            for (k, (name, _)) in FACTORY.iter().enumerate() {
                if ui.button(*name).clicked() {
                    np::load_factory(p, k);
                    ui.close();
                }
            }
            let list = saved(ui);
            if !list.is_empty() {
                ui.separator();
                ui.weak("Saved");
                let mut remove = None;
                for (k, preset) in list.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.button(&preset.name).clicked() {
                            preset.apply(p);
                            ui.close();
                        }
                        if ui.small_button("×").on_hover_text("Delete this preset").clicked() {
                            remove = Some(k);
                        }
                    });
                }
                if let Some(k) = remove {
                    let mut next = (*list).clone();
                    next.remove(k);
                    let result = keep(ui, next);
                    ui.data_mut(|d| d.insert_temp(error_id, result.err().unwrap_or_default()));
                }
            }
        });
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

    /// The four voices: name, mute, lane of steps, knobs.
    fn lanes(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let painter = v.painter;
        for (k, (_, label)) in VOICE_KNOBS.iter().enumerate() {
            v.text(VOICE_KNOBS_X + VOICE_KNOB_DX * k as f32, LANES_Y - 6.0, Align2::CENTER_CENTER, label, v.font(7.0), theme::TEXT_FAINT);
        }
        let step = p.viz[viz::STEP].get();
        let now = v.ui.input(|i| i.time);
        let flash_id = self.uid("flash");
        let mut flashes: [(f32, f64); voice::COUNT] = v.ui.data(|d| d.get_temp(flash_id)).unwrap_or([(0.0, 0.0); voice::COUNT]);
        for vc in 0..voice::COUNT {
            let top = LANES_Y + LANE_H * vc as f32;
            let cy = top + LANE_H * 0.5;
            let muted = np::muted(p, vc);
            let hits = p.viz[viz::HITS + vc].get();
            if hits != flashes[vc].0 {
                flashes[vc] = (hits, now);
            }
            let glow = (1.0 - ((now - flashes[vc].1) / 0.12) as f32).clamp(0.0, 1.0);
            let name_col = if muted { theme::TEXT_FAINT } else { theme::mix(theme::TEXT_DIM, theme::TEXT, glow) };
            let name_rect = v.text(PAD, cy, Align2::LEFT_CENTER, voice::NAMES[vc], theme::strong(8.5 * z), name_col);
            v.ui.interact(name_rect, self.uid(("name", vc)), Sense::hover()).on_hover_text(TIPS[vc]);
            let mute = widgets::toggle(v.ui, painter, self.uid(("mute", vc)), v.r(46.0, cy - 7.0, 60.0, cy + 7.0), "M", muted, theme::MUTE, z)
                .on_hover_text("Mute this voice");
            if mute.clicked() {
                p.data[data::MUTE + vc].set(if muted { 0.0 } else { 1.0 });
            }
            self.lane(vc, top, step, muted);

            for (k, (kn, _)) in VOICE_KNOBS.iter().enumerate() {
                self.knob(vp(vc, *kn), VOICE_KNOBS_X + VOICE_KNOB_DX * k as f32, top + 2.0, None, 8.5);
            }
        }
        v.ui.data_mut(|d| d.insert_temp(flash_id, flashes));
    }

    /// One voice's steps: hits filled, the playhead ringed. Click a step to
    /// toggle it, drag to paint, Shift-drag sideways to rotate.
    fn lane(&self, vc: usize, top: f32, step: f32, muted: bool) {
        let (v, p, z) = (self.v, self.p, self.z());
        let painter = v.painter;
        let cy = top + LANE_H * 0.5;
        let (_, len, _) = np::pattern(p, vc);
        let rect = v.r(CELLS_X, top + 6.0, CELLS_X + CELL * MAX_LEN as f32, top + LANE_H - 6.0);
        let resp = v
            .ui
            .interact(rect, self.uid(("lane", vc)), Sense::click_and_drag())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Click a step to switch it, drag to paint · Shift-drag sideways to rotate");
        let shift = v.ui.input(|i| i.modifiers.shift);
        let cell_at = |x: f32| ((x - rect.left()) / (CELL * z)).floor().clamp(0.0, (len - 1) as f32) as usize;
        let paint_id = self.uid(("paint", vc));
        if let Some(q) = resp.interact_pointer_pos() {
            let s = cell_at(q.x);
            let on = np::steps(p, vc) >> s & 1 == 1;
            if resp.clicked() {
                np::set_step(p, vc, s, !on);
            }
            if resp.drag_started() {
                v.ui.data_mut(|d| d.insert_temp(paint_id, !on));
            }
            if resp.dragged() && shift {
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
                    np::rotate(p, vc, moved);
                }
            } else if resp.dragged()
                && let Some(state) = v.ui.data(|d| d.get_temp::<bool>(paint_id))
            {
                np::set_step(p, vc, s, state);
            }
        }

        let bits = np::steps(p, vc);
        let at = if step >= 0.0 { Some(np::position(p, vc, step as i64)) } else { None };
        let ink = if muted { 0.3 } else { 1.0 };
        for s in 0..MAX_LEN {
            let c = v.p(CELLS_X + CELL * (s as f32 + 0.5), cy);
            if s >= len {
                painter.circle_filled(c, 0.8 * z, alpha(theme::TEXT_FAINT, 0.25));
                continue;
            }
            let hit = bits >> s & 1 == 1;
            let here = at == Some(s) && !muted;
            if hit {
                let col = if here { self.accent } else { alpha(theme::TEXT, 0.75 * ink) };
                painter.rect_filled(egui::Rect::from_center_size(c, vec2(6.0, 6.0) * z), 1.0 * z, col);
            } else {
                painter.rect_stroke(egui::Rect::from_center_size(c, vec2(5.0, 5.0) * z), 1.0 * z, Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.6 * ink)), egui::StrokeKind::Inside);
            }
            if here {
                painter.rect_stroke(egui::Rect::from_center_size(c, vec2(9.0, 9.0) * z), 1.5 * z, Stroke::new(1.0 * z, self.accent), egui::StrokeKind::Outside);
            }
        }
    }

    /// STUTTER and PLAY, then GLITCH.
    fn groups(&self) {
        let (v, z) = (self.v, self.z());
        for (title, x0, x1, y, knobs) in GROUPS {
            v.text(x0, y, Align2::LEFT_CENTER, title, theme::strong(8.5 * z), theme::TEXT_DIM);
            v.painter.hline(v.p(x0 + 60.0, 0.0).x..=v.p(x1, 0.0).x, v.p(0.0, y).y, Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.3)));
            let cw = (x1 - x0) / knobs.len() as f32;
            for (k, (i, label)) in knobs.iter().enumerate() {
                self.knob(*i, x0 + cw * (k as f32 + 0.5), y + 14.0, Some(label), 10.0);
            }
        }
    }

    /// A knob at `cx`; its label (if any) at `y`, the knob under it, the
    /// value under that. HITS and LEN lay their voice's pattern down again.
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
        let mut resp = widgets::knob(v.ui, v.painter, self.uid(("knob", i)), v.p(cx, top + r), r * z, &mut norm, spec.to_norm(spec.default), spec.bipolar, self.accent, z);
        if let Some((_, tip)) = KNOB_TIPS.iter().find(|(k, _)| *k == i) {
            resp = resp.on_hover_text(*tip);
        }
        let generator = i < voice::COUNT * knob::COUNT && matches!(i % knob::COUNT, knob::HITS | knob::LEN);
        if generator {
            resp = resp.on_hover_text("Lays the Euclidean pattern down again (over your edits)");
        } else {
            v.hit(resp.rect, ParamRef::Module(i));
        }
        if resp.dragged() || resp.double_clicked() {
            let value = spec.denorm(norm);
            let value = if matches!(spec.unit, crate::audio::params::Unit::Integer) { value.round() } else { value };
            if value != p.get(i) {
                p.set(i, value);
                if generator {
                    np::regenerate(p, i / knob::COUNT);
                }
            }
        }
        let hot = resp.dragged() || resp.hovered();
        v.text(cx, top + 2.0 * r + 8.0, Align2::CENTER_CENTER, np::format(p, i), v.mono(7.0), if hot { theme::TEXT } else { theme::TEXT_DIM });
    }
}
