//! Panel of Sibyl, laid out like Repro-5's synth view grown into a node, in
//! the board's flat, graphic style: line-art knobs with a coloured arc,
//! outlined buttons and fields, a colour per kind of section (modulation,
//! oscillators, mixer, filter, envelopes, voices, effects, arpeggiator).
//!
//! Along the top, the control bar: SYNTH and TWEAKS, the voices, the
//! programmer (the two-digit display, the program's name, PRESETS ▾, BANK
//! 1–6, PROGRAM 1–8 and RECORD, as on the original), HQ, A-440, TUNE and
//! OUTPUT.
//!
//! SYNTH, in three rows: VOICE MOD, OSCILLATOR A, MIXER, FILTER; LFO,
//! OSCILLATOR B, UNISON with GLIDE and VOICE DETUNE (and a light per voice),
//! FILTER ENVELOPE; WHEEL MOD, the MOD MATRIX (two slots a page, MM A and MM
//! B), AMPLIFIER. TWEAKS: the jumpers and selectors, the wheels' limits and
//! controllers, a pan per voice.
//!
//! Underneath, KEYS (the bend and mod wheels with the wheel-mod limits, the
//! arpeggiator, five octaves of keys in real proportions that light up with
//! the voices and play) or EFFECTS (the voices' distortion, the chain's
//! order — click a cell to switch it, drag to move it — and the five
//! effects); the EFFECTS tab shows which effects are on.

use egui::{Align2, Color32, CursorIcon, Id, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};

use super::nodes::{MIN_TEXT, NodeView};
use super::theme::{self, alpha};
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::sibyl::fx::{EFFECTS, SWITCHES};
use crate::audio::modules::sibyl::mods::{self, CURVES, DESTS, QUANTIZERS, RECTIFIERS, SH_SOURCES, SLEWS, SOURCES};
use crate::audio::modules::sibyl::{self as sb, *};
use crate::audio::params::note_label;
use crate::graph::{ModuleNode, ParamRef};

pub mod library;

use library::CATEGORIES;

pub const SIZE: Vec2 = vec2(1280.0, 724.0);
const W: f32 = SIZE.x;
const H: f32 = SIZE.y;

// A colour per kind of section.
const MOD_COL: Color32 = Color32::from_rgb(158, 138, 230);
const OSC_COL: Color32 = Color32::from_rgb(255, 196, 87);
const MIX_COL: Color32 = Color32::from_rgb(200, 204, 212);
const FILT_COL: Color32 = Color32::from_rgb(86, 200, 182);
const ENV_COL: Color32 = Color32::from_rgb(232, 116, 154);
const VOICE_COL: Color32 = Color32::from_rgb(96, 165, 238);
const FX_COL: Color32 = Color32::from_rgb(232, 146, 92);
const ARP_COL: Color32 = Color32::from_rgb(132, 204, 112);
/// Lights (RECORD, ALL OFF, the conditioner's load).
const LIGHT: Color32 = Color32::from_rgb(255, 92, 72);

// Columns and rows of the synth view.
const C1: [f32; 2] = [24.0, 292.0];
const C2: [f32; 2] = [304.0, 640.0];
const C3: [f32; 2] = [652.0, 820.0];
const C4: [f32; 2] = [832.0, 1256.0];
const R1: f32 = 64.0;
const R2: f32 = 230.0;
const R3: f32 = 396.0;
const LOWER: f32 = 566.0;
/// Where the lower panel's content starts (after its tabs).
const LOW_X: f32 = 92.0;
/// The keyboard: five octaves from C2, keys about six times as long as wide.
const LOW_KEY: u8 = 36;
const KEYS: usize = 61;
const KEYS_X: [f32; 2] = [430.0, 1256.0];

pub fn readout(p: &ModuleParams) -> (String, String) {
    let k = sb::program(p);
    let edited = if sb::edited(p) { " ·" } else { "" };
    let main = format!("{} {}{edited}", program_label(k), sound_name(p));
    let mode = if p.get(UNISON) > 0.5 { format!("unison ×{:.0}", p.get(UNI_VOICES)) } else { format!("{:.0} voices", p.get(VOICES)) };
    let arp = if p.get(ARP_ON) > 0.5 { " · arp" } else { "" };
    (main, format!("{mode}{arp}"))
}

/// The panel's state between frames (not saved).
#[derive(Clone, Default)]
struct View {
    record: bool,
    /// The chain cell being dragged.
    dragging: Option<usize>,
}

struct Panel<'a, 'b> {
    v: &'a NodeView<'b>,
    id: NodeId,
    p: &'a ModuleParams,
    accent: Color32,
    /// The colour of the section being drawn.
    col: std::cell::Cell<Color32>,
}

pub fn draw(v: &NodeView, id: NodeId, accent: Color32, m: &ModuleNode, (midi_in, out): (bool, bool)) {
    let p = &*m.params;
    let key = Id::unique(("sibyl-view", id));
    let mut view: View = v.ui.data_mut(|d| d.remove_temp(key)).unwrap_or_default();
    let s = Panel { v, id, p, accent, col: std::cell::Cell::new(accent) };

    // Ports.
    let col = |on: bool| if on { theme::TEXT_DIM } else { theme::TEXT_FAINT };
    v.text(10.0, 43.0, Align2::LEFT_CENTER, "MIDI", v.font(8.5), col(midi_in));
    v.text(W - 10.0, 43.0, Align2::RIGHT_CENTER, "OUT", v.font(8.5), col(out));

    s.control_bar(&mut view);
    if p.data[data::VIEW].get() < 0.5 {
        s.voice_mod();
        s.lfo();
        s.wheel_mod();
        s.osc_a();
        s.osc_b();
        s.mixer();
        s.unison();
        s.filter();
        s.filter_env();
        s.amplifier();
        s.matrix();
    } else {
        s.tweaks();
    }
    s.lower(&mut view);

    let busy = (0..MAX_VOICES).any(|i| p.viz[viz::LEVEL + i].get() > 1e-4) || p.get(ARP_ON) > 0.5;
    if busy {
        v.ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
    v.ui.data_mut(|d| d.insert_temp(key, view));
}

impl Panel<'_, '_> {
    fn z(&self) -> f32 {
        self.v.z
    }

    fn uid(&self, salt: impl std::hash::Hash + std::fmt::Debug) -> Id {
        Id::unique(("sibyl", self.id)).with(salt)
    }

    fn on(&self, k: usize) -> bool {
        self.p.get(k) > 0.5
    }

    fn idx(&self, k: usize) -> usize {
        self.p.get(k).round().max(0.0) as usize
    }

    fn set(&self, k: usize, value: f32) {
        let value = sb::snap(k, value);
        if value != self.p.get(k) {
            self.p.set(k, value);
        }
    }

    fn text(&self, x: f32, y: f32, align: Align2, s: impl ToString, size: f32, color: Color32) -> Rect {
        self.v.text(x, y, align, s, self.v.font(size), color)
    }

    fn strong(&self, x: f32, y: f32, align: Align2, s: impl ToString, size: f32, color: Color32) -> Rect {
        let font = theme::strong(size * self.z());
        if font.size < MIN_TEXT {
            return Rect::from_center_size(self.v.p(x, y), Vec2::ZERO);
        }
        self.v.painter.text(self.v.p(x, y), align, s, font, color)
    }

    fn stroke(&self, width: f32, color: Color32) -> Stroke {
        Stroke::new(width * self.z(), color)
    }

    /// A section: its name in its colour, then a thin line to its end. Sets
    /// the colour the controls after it use.
    fn section(&self, title: &str, x0: f32, x1: f32, y: f32, col: Color32) {
        self.col.set(col);
        let v = self.v;
        let r = self.strong(x0 + 2.0, y + 9.0, Align2::LEFT_CENTER, title, 8.0, col);
        let from = if r.width() > 0.0 { r.right() + 8.0 * self.z() } else { v.p(x0, 0.0).x };
        let cy = v.p(0.0, y + 9.0).y;
        v.painter.line_segment([pos2(from, cy), pos2(v.p(x1, 0.0).x, cy)], self.stroke(1.0, alpha(col, 0.3)));
    }

    /// A small caption over a group of controls.
    fn caption(&self, s: &str, x0: f32, x1: f32, y: f32) {
        let v = self.v;
        let r = self.text((x0 + x1) * 0.5, y, Align2::CENTER_CENTER, s, 6.2, theme::TEXT_FAINT);
        if r.width() > 0.0 {
            let stroke = self.stroke(0.8, alpha(theme::TEXT_FAINT, 0.5));
            v.painter.line_segment([v.p(x0, y), pos2(r.left() - 3.0 * self.z(), r.center().y)], stroke);
            v.painter.line_segment([pos2(r.right() + 3.0 * self.z(), r.center().y), v.p(x1, y)], stroke);
        }
    }

    // --- Controls ------------------------------------------------------

    /// A knob for parameter `k`: drag up or right (Shift: fine),
    /// double-click for its default. Line art: the travel as a faint arc,
    /// the value as an arc in the section's colour (from the middle on
    /// bipolar knobs), the pointer. `label` under it, or its value while
    /// touched (`value` overrides how it reads).
    fn knob_with(&self, k: usize, cx: f32, cy: f32, r: f32, label: &str, value: Option<String>) {
        let (v, z, p) = (self.v, self.z(), self.p);
        let col = self.col.get();
        let spec = &DEF.params[k];
        let c = v.p(cx, cy);
        let rect = Rect::from_center_size(c, vec2(r * 2.7, r * 2.7) * z);
        let id = self.uid(("knob", k));
        let resp = v.ui.interact(rect, id, Sense::click_and_drag()).on_hover_cursor(CursorIcon::ResizeVertical);
        v.hit(resp.rect, ParamRef::Module(k));
        let current = p.get(k);
        if resp.dragged() {
            let fine = v.ui.input(|i| i.modifiers.shift);
            let d = resp.drag_delta();
            let delta = (d.x - d.y) * if fine { 0.0009 } else { 0.0048 };
            // An unrounded position of its own, so stepped values step on.
            let key = id.with("pos");
            let norm = spec.to_norm(current);
            let pos = v.ui.data_mut(|m| {
                if resp.drag_started() {
                    m.remove_temp::<f32>(key);
                }
                let pos = m.get_temp_mut_or(key, norm);
                *pos = (*pos + delta).clamp(0.0, 1.0);
                *pos
            });
            self.set(k, spec.denorm(pos));
        }
        if resp.double_clicked() {
            self.set(k, spec.default);
        }
        let now = p.get(k);
        let norm = spec.to_norm(now);
        let hot = resp.hovered() || resp.dragged();
        const START: f32 = 0.75 * std::f32::consts::PI;
        const SWEEP: f32 = 1.5 * std::f32::consts::PI;
        let ring = r * 1.22;
        let arc = |a0: f32, a1: f32| -> Vec<Pos2> {
            let steps = ((a1 - a0).abs() / 0.12).ceil().max(1.0) as usize;
            (0..=steps)
                .map(|i| {
                    let a = a0 + (a1 - a0) * i as f32 / steps as f32;
                    c + vec2(a.cos(), a.sin()) * ring * z
                })
                .collect()
        };
        let w = (1.4 + r * 0.05) * z;
        v.painter.add(Shape::line(arc(START, START + SWEEP), Stroke::new(w, alpha(col, 0.16))));
        let a = START + SWEEP * norm;
        let (a0, a1) = if spec.bipolar {
            let mid = START + SWEEP * 0.5;
            (mid.min(a), mid.max(a))
        } else {
            (START, a)
        };
        if a1 - a0 > 0.01 {
            v.painter.add(Shape::line(arc(a0, a1), Stroke::new(w, col)));
        }
        v.painter.circle_filled(c, r * z, theme::INSET);
        v.painter.circle_stroke(c, r * z, self.stroke(1.0, if hot { col } else { theme::CONTROL_HI }));
        let dir = vec2(a.cos(), a.sin());
        v.painter.line_segment([c + dir * r * 0.25 * z, c + dir * r * 0.88 * z], self.stroke(1.3 + r * 0.03, theme::TEXT));
        let text = if hot { value.unwrap_or_else(|| spec.format(now)) } else { label.to_owned() };
        if !text.is_empty() {
            self.text(cx, cy + ring + 8.0, Align2::CENTER_CENTER, text, if r >= 12.0 { 6.8 } else { 6.0 }, if hot { theme::TEXT } else { theme::TEXT_DIM });
        }
        if hot && !label.is_empty() {
            resp.on_hover_text(spec.name);
        }
    }

    fn knob(&self, k: usize, cx: f32, cy: f32, r: f32, label: &str) {
        self.knob_with(k, cx, cy, r, label, None);
    }

    /// An outlined button, lit in the section's colour; returns its response.
    #[allow(clippy::too_many_arguments)]
    fn push(&self, salt: impl std::hash::Hash + std::fmt::Debug, cx: f32, cy: f32, w: f32, label: &str, lit: bool, tip: &str) -> egui::Response {
        let (v, z) = (self.v, self.z());
        let col = self.col.get();
        let rect = v.r(cx - w * 0.5, cy - 8.0, cx + w * 0.5, cy + 8.0);
        let resp = v.ui.interact(rect, self.uid(("push", salt)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        if lit {
            v.painter.rect_filled(rect, 3.0 * z, alpha(col, 0.22));
        }
        let edge = if lit {
            col
        } else if resp.hovered() {
            theme::TEXT_DIM
        } else {
            theme::CONTROL_HI
        };
        v.painter.rect_stroke(rect, 3.0 * z, self.stroke(1.0, edge), StrokeKind::Inside);
        if !label.is_empty() {
            let font = theme::font(5.8 * z);
            if font.size >= MIN_TEXT {
                v.painter.text(rect.center(), Align2::CENTER_CENTER, label, font, if lit { col } else { theme::TEXT_DIM });
            }
        }
        if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
    }

    /// A switch parameter as a button.
    fn button(&self, k: usize, cx: f32, cy: f32, w: f32, label: &str) {
        let resp = self.push(("param", k), cx, cy, w, label, self.on(k), DEF.params[k].name);
        self.v.hit(resp.rect, ParamRef::Module(k));
        if resp.clicked() {
            self.set(k, if self.on(k) { 0.0 } else { 1.0 });
        }
    }

    /// An outlined field showing parameter `k`'s choice, opening a menu of
    /// `names`.
    #[allow(clippy::too_many_arguments)]
    fn selector(&self, k: usize, x0: f32, y0: f32, x1: f32, y1: f32, names: &[&str], size: f32) {
        let (v, z) = (self.v, self.z());
        let col = self.col.get();
        let rect = v.r(x0, y0, x1, y1);
        let resp = v.ui.interact(rect, self.uid(("sel", k)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        v.hit(resp.rect, ParamRef::Module(k));
        let i = self.idx(k).min(names.len() - 1);
        v.painter.rect_filled(rect, 3.0 * z, theme::INSET);
        v.painter.rect_stroke(rect, 3.0 * z, self.stroke(1.0, if resp.hovered() { col } else { alpha(col, 0.45) }), StrokeKind::Inside);
        let clip = v.painter.with_clip_rect(rect.shrink(1.0).intersect(v.painter.clip_rect()));
        let font = theme::font(size * z);
        if font.size >= MIN_TEXT {
            clip.text(rect.center() - vec2(3.0 * z, 0.0), Align2::CENTER_CENTER, names[i], font, theme::TEXT);
            clip.text(pos2(rect.right() - 4.0 * z, rect.center().y), Align2::RIGHT_CENTER, "▾", theme::font(size * 0.8 * z), alpha(col, 0.8));
        }
        let resp = resp.on_hover_text(DEF.params[k].name);
        egui::Popup::menu(&resp).show(|ui| {
            ui.set_min_width(140.0);
            for (n, name) in names.iter().enumerate() {
                if ui.selectable_label(n == i, *name).clicked() {
                    self.set(k, n as f32);
                    ui.close();
                }
            }
        });
    }

    /// A row of buttons choosing one of `labels` for parameter `k`.
    #[allow(clippy::too_many_arguments)]
    fn choice(&self, k: usize, x0: f32, cy: f32, step: f32, w: f32, labels: &[&str]) {
        let cur = self.idx(k);
        for (n, label) in labels.iter().enumerate() {
            let resp = self.push(("choice", k, n), x0 + step * n as f32, cy, w, label, cur == n, DEF.params[k].name);
            if resp.clicked() {
                self.set(k, n as f32);
            }
        }
    }

    /// A drag box for a whole number (a controller number).
    fn drag_box(&self, k: usize, x0: f32, y0: f32, x1: f32, y1: f32) {
        let (v, z) = (self.v, self.z());
        let rect = v.r(x0, y0, x1, y1);
        let id = self.uid(("box", k));
        let resp = v.ui.interact(rect, id, Sense::click_and_drag()).on_hover_cursor(CursorIcon::ResizeVertical);
        if resp.dragged() {
            let d = resp.drag_delta();
            let steps = v.ui.data_mut(|m| {
                let acc = m.get_temp_mut_or(id.with("acc"), 0.0f32);
                *acc += (d.x - d.y) / z;
                let steps = (*acc / 5.0).trunc();
                *acc -= steps * 5.0;
                steps
            });
            if steps != 0.0 {
                self.set(k, self.p.get(k) + steps);
            }
        }
        if resp.double_clicked() {
            self.set(k, DEF.params[k].default);
        }
        let col = self.col.get();
        v.painter.rect_filled(rect, 3.0 * z, theme::INSET);
        v.painter.rect_stroke(rect, 3.0 * z, self.stroke(1.0, if resp.hovered() { col } else { alpha(col, 0.45) }), StrokeKind::Inside);
        self.text((x0 + x1) * 0.5, (y0 + y1) * 0.5, Align2::CENTER_CENTER, format!("CC {:.0}", self.p.get(k)), 7.0, col);
    }

    /// A light: an outline when off, filled as `level` (0..1) rises.
    fn light(&self, cx: f32, cy: f32, r: f32, level: f32, col: Color32) {
        let (v, z) = (self.v, self.z());
        let c = v.p(cx, cy);
        if level > 1e-3 {
            v.painter.circle_filled(c, r * z, alpha(col, 0.35 + 0.65 * level.clamp(0.0, 1.0)));
        } else {
            v.painter.circle_stroke(c, r * z, self.stroke(0.9, alpha(col, 0.35)));
        }
    }

    // --- Control bar ---------------------------------------------------

    fn control_bar(&self, view: &mut View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let cy = 43.0;
        self.col.set(self.accent);
        let tab = |salt: &str, x: f32, label: &str, on: bool| -> bool {
            let rect = v.r(x, cy - 9.0, x + 54.0, cy + 9.0);
            let resp = v.ui.interact(rect, self.uid(("tab", salt)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
            if on {
                v.painter.rect_filled(rect, 9.0 * z, alpha(self.accent, 0.2));
            }
            v.painter.rect_stroke(rect, 9.0 * z, self.stroke(1.0, if on { self.accent } else { theme::CONTROL_HI }), StrokeKind::Inside);
            self.strong(x + 27.0, cy, Align2::CENTER_CENTER, label, 7.0, if on { self.accent } else { theme::TEXT_DIM });
            resp.clicked()
        };
        let tweaks = p.data[data::VIEW].get() > 0.5;
        if tab("synth", 40.0, "SYNTH", !tweaks) {
            p.data[data::VIEW].set(0.0);
        }
        if tab("tweaks", 98.0, "TWEAKS", tweaks) {
            p.data[data::VIEW].set(1.0);
        }

        // Voices.
        let unison = self.on(UNISON);
        let k = if unison { UNI_VOICES } else { VOICES };
        self.text(164.0, cy, Align2::LEFT_CENTER, if unison { "UNISON" } else { "VOICES" }, 6.0, theme::TEXT_FAINT);
        let names: [&str; MAX_VOICES] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16"];
        let rect = v.r(200.0, cy - 8.0, 230.0, cy + 8.0);
        let resp = v.ui.interact(rect, self.uid("voices"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        v.painter.rect_stroke(rect, 3.0 * z, self.stroke(1.0, if resp.hovered() { VOICE_COL } else { alpha(VOICE_COL, 0.45) }), StrokeKind::Inside);
        self.text(215.0, cy, Align2::CENTER_CENTER, format!("{:.0}", p.get(k)), 7.5, theme::TEXT);
        let resp = resp.on_hover_text(if unison { "Voices stacked in UNISON" } else { "Voices played before one is stolen" });
        egui::Popup::menu(&resp).show(|ui| {
            for (n, name) in names.iter().enumerate() {
                if ui.selectable_label(self.idx(k) == n + 1, *name).clicked() {
                    self.set(k, (n + 1) as f32);
                    ui.close();
                }
            }
        });

        // The programmer: display, name, presets.
        let prog = sb::program(p);
        let disp = v.r(240.0, cy - 9.0, 276.0, cy + 9.0);
        v.painter.rect_filled(disp, 3.0 * z, theme::INSET);
        v.painter.rect_stroke(disp, 3.0 * z, self.stroke(1.0, alpha(self.accent, 0.45)), StrokeKind::Inside);
        let font = theme::mono(12.0 * z);
        if font.size >= MIN_TEXT {
            v.painter.text(disp.center(), Align2::CENTER_CENTER, program_label(prog), font, self.accent);
        }
        for (n, (label, delta)) in [("◀", PROGRAMS - 1), ("▶", 1)].into_iter().enumerate() {
            let x = 282.0 + 15.0 * n as f32;
            let rect = v.r(x, cy - 8.0, x + 13.0, cy + 8.0);
            let resp = v.ui.interact(rect, self.uid(("step", n)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
            v.painter.rect_stroke(rect, 3.0 * z, self.stroke(1.0, if resp.hovered() { theme::TEXT_DIM } else { theme::CONTROL_HI }), StrokeKind::Inside);
            self.text(x + 6.5, cy, Align2::CENTER_CENTER, label, 6.5, theme::TEXT_DIM);
            if resp.on_hover_text(if n == 0 { "Previous program" } else { "Next program" }).clicked() {
                load_program(p, (prog + delta) % PROGRAMS);
            }
        }
        let edited = sb::edited(p);
        let name = format!("{}{}", sound_name(p), if edited { " *" } else { "" });
        let clip = v.painter.with_clip_rect(v.r(318.0, cy - 10.0, 466.0, cy + 10.0).intersect(v.painter.clip_rect()));
        let font = theme::font(9.0 * z);
        if font.size >= MIN_TEXT {
            clip.text(v.p(320.0, cy), Align2::LEFT_CENTER, name, font, if edited { theme::TEXT_DIM } else { theme::TEXT });
        }
        let save = self.push("save", 488.0, cy, 36.0, "SAVE", false, "Save this sound: in your library (every project) or in a program");
        self.save_menu(&save);
        let browse = self.push("presets", 540.0, cy, 60.0, "PRESETS ▾", false, "Your saved sounds by category, the factory banks, import and export");
        self.browse_menu(&browse);

        // BANK, PROGRAM, RECORD.
        self.text(590.0, cy, Align2::RIGHT_CENTER, "BANK", 6.0, theme::TEXT_FAINT);
        for b in 0..BANKS {
            let tip = match b {
                0 => "Bank 1: arpeggios",
                1 => "Bank 2: pads",
                2 => "Bank 3: leads",
                3 => "Bank 4: keys and basses",
                4 => "Bank 5: textures",
                _ => "Bank 6: bouncing sequences",
            };
            let resp = self.push(("bank", b), 604.0 + 20.0 * b as f32, cy, 17.0, &format!("{}", b + 1), prog / 8 == b, tip);
            if resp.clicked() {
                load_program(p, b * 8 + prog % 8);
            }
        }
        self.text(758.0, cy, Align2::RIGHT_CENTER, "PROGRAM", 6.0, theme::TEXT_FAINT);
        for n in 0..8 {
            let k = (prog / 8) * 8 + n;
            let tip = if view.record { "Store the panel here" } else { bank::name(k) };
            let resp = self.push(("prog", n), 772.0 + 20.0 * n as f32, cy, 17.0, &format!("{}", n + 1), prog % 8 == n, tip);
            if resp.clicked() {
                if view.record {
                    store_program(p, k);
                    view.record = false;
                } else {
                    load_program(p, k);
                }
            }
            resp.context_menu(|ui| {
                if ui.button("Store the panel here").clicked() {
                    store_program(p, k);
                    ui.close();
                }
                if ui.button("Restore the factory sound").clicked() {
                    restore_program(p, k);
                    if prog == k {
                        load_program(p, k);
                    }
                    ui.close();
                }
            });
        }
        self.col.set(LIGHT);
        let resp = self.push("record", 950.0, cy, 40.0, "RECORD", view.record, "RECORD, then a program button: store the panel there (right-click a program to restore its factory sound)");
        if resp.clicked() {
            view.record = !view.record;
        }

        // HQ, A-440, TUNE, OUTPUT.
        self.col.set(self.accent);
        self.button(HQ, 992.0, cy, 26.0, "HQ");
        self.button(A440, 1026.0, cy, 34.0, "A-440");
        self.col.set(MIX_COL);
        self.knob_with(TUNE, 1066.0, cy, 8.0, "", None);
        self.text(1078.0, cy, Align2::LEFT_CENTER, "TUNE", 6.0, theme::TEXT_FAINT);
        self.knob_with(OUTPUT, 1118.0, cy, 8.0, "", None);
        self.text(1130.0, cy, Align2::LEFT_CENTER, "OUTPUT", 6.0, theme::TEXT_FAINT);
        self.col.set(LIGHT);
        let panic = self.push("panic", 1210.0, cy, 48.0, "ALL OFF", false, "Silence every voice and let go of every key");
        if panic.clicked() {
            p.viz[viz::PANIC].set(p.viz[viz::PANIC].get() + 1.0);
        }
        v.painter.line_segment([v.p(16.0, 58.0), v.p(W - 16.0, 58.0)], self.stroke(1.0, theme::NODE_BORDER));
    }

    // --- Presets -------------------------------------------------------

    /// SAVE: a name and a category, then into the library (replacing a
    /// preset of the same name) or into the current program.
    fn save_menu(&self, button: &egui::Response) {
        let p = self.p;
        let key = self.uid("save-state");
        if button.clicked() {
            // Opening: start from the sound as it is now.
            let state = SaveState { name: sound_name(p), category: library::factory_category(sb::program(p)), status: String::new() };
            self.v.ui.data_mut(|d| d.insert_temp(key, state));
        }
        egui::Popup::menu(button).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.set_min_width(260.0);
            let mut st: SaveState = ui.data(|d| d.get_temp(key)).unwrap_or_default();
            ui.strong("Save this sound");
            ui.label("Name");
            let field = ui.add(egui::TextEdit::singleline(&mut st.name).char_limit(NAME_LEN).hint_text("A name for it"));
            ui.label("Category");
            ui.horizontal_wrapped(|ui| {
                for (i, c) in CATEGORIES.iter().enumerate() {
                    if ui.selectable_label(st.category == i, *c).clicked() {
                        st.category = i;
                    }
                }
            });
            ui.separator();
            let lib = library_cache(ui);
            let name = st.name.trim().to_owned();
            let exists = lib.iter().any(|x| x.name == name);
            let label = if exists { "Replace in my library" } else { "Save in my library" };
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let clicked = ui.add_enabled(!name.is_empty(), egui::Button::new(label)).on_hover_text("Kept in the app's presets: there in every project").clicked();
            if (clicked || enter) && !name.is_empty() {
                let mut next = (*lib).clone();
                crate::presets::store(&mut next, library::capture(p, &name, st.category));
                st.status = match keep_library(ui, next) {
                    Ok(()) => {
                        sb::set_sound_name(p, &name);
                        format!("Saved \"{name}\" in {}", CATEGORIES[st.category])
                    }
                    Err(e) => format!("Could not save: {e}"),
                };
            }
            let k = sb::program(p);
            let store = egui::Button::new(format!("Store in program {} (now \"{}\")", program_label(k), program_name(p, k)));
            if ui.add_enabled(!name.is_empty(), store).on_hover_text("Kept with this node, in the project; BANK and PROGRAM recall it").clicked() {
                sb::set_sound_name(p, &name);
                store_program(p, k);
                st.status = format!("Stored in program {}", program_label(k));
            }
            if !st.status.is_empty() {
                ui.weak(&st.status);
            }
            ui.data_mut(|d| d.insert_temp(key, st));
        });
    }

    /// PRESETS ▾: the library by category (load, delete), the factory banks,
    /// export and import.
    fn browse_menu(&self, button: &egui::Response) {
        let p = self.p;
        let status_key = self.uid("browse-status");
        egui::Popup::menu(button).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.set_min_width(230.0);
            let lib = library_cache(ui);
            ui.weak("My sounds");
            if lib.is_empty() {
                ui.label("None yet: SAVE puts the panel here");
            }
            let mut remove = None;
            for (c, cat) in CATEGORIES.iter().enumerate() {
                let items: Vec<usize> = (0..lib.len()).filter(|&i| library::category(&lib[i]) == c).collect();
                if items.is_empty() {
                    continue;
                }
                ui.menu_button(format!("{cat}  ({})", items.len()), |ui| {
                    ui.set_min_width(200.0);
                    for i in items {
                        ui.horizontal(|ui| {
                            if ui.button(&lib[i].name).clicked() {
                                library::apply(p, &lib[i]);
                                ui.close();
                            }
                            if ui.small_button("×").on_hover_text("Delete this preset").clicked() {
                                remove = Some(i);
                            }
                        });
                    }
                });
            }
            if let Some(i) = remove {
                let mut next = (*lib).clone();
                let gone = next.remove(i).name;
                let status = match keep_library(ui, next) {
                    Ok(()) => format!("Deleted \"{gone}\""),
                    Err(e) => format!("Could not delete: {e}"),
                };
                ui.data_mut(|d| d.insert_temp(status_key, status));
            }
            ui.separator();
            ui.weak("Factory");
            const BANK_NAMES: [&str; BANKS] = ["1 · Arpeggios", "2 · Pads", "3 · Leads", "4 · Keys and basses", "5 · Textures", "6 · Bouncing"];
            for (b, title) in BANK_NAMES.iter().enumerate() {
                ui.menu_button(*title, |ui| {
                    for n in 0..8 {
                        let k = b * 8 + n;
                        if ui.button(format!("{}  {}", program_label(k), program_name(p, k))).clicked() {
                            load_program(p, k);
                            ui.close();
                        }
                    }
                });
            }
            ui.separator();
            let name = sound_name(p);
            let filter = ("Sibyl presets", &["json"][..]);
            if ui.button("Export this sound…").clicked() {
                let file = format!("{}.json", name.replace(['/', '\\', ':'], "-"));
                if let Some(path) = rfd::FileDialog::new().add_filter(filter.0, filter.1).set_file_name(file).save_file() {
                    let preset = library::capture(p, &name, library::factory_category(sb::program(p)));
                    let status = match library::export(&path, &[preset]) {
                        Ok(()) => format!("Exported \"{name}\""),
                        Err(e) => format!("Could not export: {e}"),
                    };
                    ui.data_mut(|d| d.insert_temp(status_key, status));
                }
            }
            if ui.add_enabled(!lib.is_empty(), egui::Button::new("Export my library…")).clicked()
                && let Some(path) = rfd::FileDialog::new().add_filter(filter.0, filter.1).set_file_name("Sibyl library.json").save_file()
            {
                let status = match library::export(&path, &lib) {
                    Ok(()) => format!("Exported {} presets", lib.len()),
                    Err(e) => format!("Could not export: {e}"),
                };
                ui.data_mut(|d| d.insert_temp(status_key, status));
            }
            if ui.button("Import…").on_hover_text("Adds the presets in a file to your library (a file with one sound loads it too)").clicked()
                && let Some(path) = rfd::FileDialog::new().add_filter(filter.0, filter.1).pick_file()
            {
                let incoming = library::import(&path);
                let status = if incoming.is_empty() {
                    "No Sibyl presets in that file".to_owned()
                } else {
                    if incoming.len() == 1 {
                        library::apply(p, &incoming[0]);
                    }
                    let count = incoming.len();
                    let mut next = (*lib).clone();
                    library::merge(&mut next, incoming);
                    match keep_library(ui, next) {
                        Ok(()) => format!("Imported {count} preset{}", if count == 1 { "" } else { "s" }),
                        Err(e) => format!("Could not import: {e}"),
                    }
                };
                ui.data_mut(|d| d.insert_temp(status_key, status));
            }
            if let Some(status) = ui.data(|d| d.get_temp::<String>(status_key)) {
                ui.weak(status);
            }
        });
    }

    // --- The synth view ------------------------------------------------

    fn voice_mod(&self) {
        let [x0, x1] = C1;
        let y = R1;
        self.section("VOICE MOD", x0, x1, y, MOD_COL);
        self.caption("SOURCE AMOUNT", x0 + 10.0, x0 + 146.0, y + 30.0);
        self.knob(PM_FENV, x0 + 45.0, y + 72.0, 16.0, "FILT ENV");
        self.knob(PM_OSCB, x0 + 112.0, y + 72.0, 16.0, "OSC B");
        self.caption("DESTINATION", x0 + 158.0, x1 - 6.0, y + 30.0);
        self.button(PM_FREQ_A, x0 + 176.0, y + 72.0, 30.0, "FREQ A");
        self.button(PM_PW_A, x0 + 212.0, y + 72.0, 30.0, "PW A");
        self.button(PM_FILTER, x0 + 248.0, y + 72.0, 30.0, "FILTER");
        self.text((x0 + x1) * 0.5, y + 126.0, Align2::CENTER_CENTER, "per voice, at audio rate", 5.8, theme::TEXT_FAINT);
    }

    fn lfo(&self) {
        let p = self.p;
        let [x0, x1] = C1;
        let y = R2;
        self.section("LFO", x0, x1, y, MOD_COL);
        let synced = self.on(LFO_SYNC);
        let div = LFO_DIVS[self.idx(LFO_DIV).min(LFO_DIVS.len() - 1)].0;
        if synced {
            self.knob_with(LFO_DIV, x0 + 45.0, y + 72.0, 16.0, "RATE", Some(div.to_owned()));
        } else {
            self.knob(LFO_RATE, x0 + 45.0, y + 72.0, 16.0, "RATE");
        }
        // A light that follows the LFO.
        self.light(x0 + 72.0, y + 50.0, 3.0, p.viz[viz::LFO].get().clamp(-1.0, 1.0) * 0.5 + 0.5, MOD_COL);
        self.button(LFO_SYNC, x0 + 110.0, y + 72.0, 30.0, "SYNC");
        self.caption("SHAPE", x0 + 158.0, x1 - 6.0, y + 30.0);
        self.button(LFO_SAW, x0 + 176.0, y + 72.0, 30.0, "SAW");
        self.button(LFO_TRI, x0 + 212.0, y + 72.0, 30.0, "TRI");
        self.button(LFO_SQR, x0 + 248.0, y + 72.0, 30.0, "SQUARE");
        let rate = if synced { format!("{div} of the board's tempo") } else { "shapes add up when several are on".to_owned() };
        self.text((x0 + x1) * 0.5, y + 126.0, Align2::CENTER_CENTER, rate, 5.8, theme::TEXT_FAINT);
    }

    fn wheel_mod(&self) {
        let [x0, x1] = C1;
        let y = R3;
        self.section("WHEEL MOD", x0, x1, y, MOD_COL);
        self.knob(WM_MIX, x0 + 45.0, y + 72.0, 16.0, "SOURCE MIX");
        self.text(x0 + 22.0, y + 92.0, Align2::RIGHT_CENTER, "LFO", 5.6, theme::TEXT_FAINT);
        let noise = if self.on(WM_SH) { "S&H" } else { "NOISE" };
        self.text(x0 + 68.0, y + 92.0, Align2::LEFT_CENTER, noise, 5.6, theme::TEXT_FAINT);
        self.caption("DESTINATION", x0 + 94.0, x1 - 6.0, y + 30.0);
        for (n, (k, label)) in [(WM_FREQ_A, "FREQ A"), (WM_FREQ_B, "FREQ B"), (WM_PW_A, "PW A"), (WM_PW_B, "PW B"), (WM_FILTER, "FILTER")].into_iter().enumerate() {
            self.button(k, x0 + 112.0 + 34.0 * n as f32, y + 72.0, 30.0, label);
        }
        let (low, high) = (self.p.get(WM_LOW), self.p.get(WM_HIGH));
        self.text((x0 + x1) * 0.5, y + 126.0, Align2::CENTER_CENTER, format!("wheel depth {:.0}–{:.0}%", low * 100.0, high * 100.0), 5.8, theme::TEXT_FAINT);
    }

    /// OCTAVE: a four-position switch, its positions around it.
    fn octave(&self, k: usize, cx: f32, cy: f32) {
        let o = self.idx(k);
        self.knob_with(k, cx, cy, 10.0, "OCTAVE", Some(format!("{:+} oct", o as i32 - 1)));
        for n in 0..4 {
            let a = 0.75 * std::f32::consts::PI + 1.5 * std::f32::consts::PI * n as f32 / 3.0;
            let p = self.v.p(cx, cy) + vec2(a.cos(), a.sin()) * 19.0 * self.z();
            let font = theme::font(5.4 * self.z());
            if font.size >= MIN_TEXT {
                self.v.painter.text(p, Align2::CENTER_CENTER, n, font, if n == o { self.col.get() } else { theme::TEXT_FAINT });
            }
        }
    }

    fn osc_a(&self) {
        let [x0, x1] = C2;
        let y = R1;
        self.section("OSCILLATOR A", x0, x1, y, OSC_COL);
        self.knob(A_FREQ, x0 + 40.0, y + 72.0, 16.0, "FREQUENCY");
        self.knob(A_FINE, x0 + 84.0, y + 56.0, 6.5, "");
        self.text(x0 + 84.0, y + 70.0, Align2::CENTER_CENTER, "FINE", 5.4, theme::TEXT_FAINT);
        self.octave(A_OCT, x0 + 126.0, y + 76.0);
        self.caption("SHAPE", x0 + 156.0, x0 + 222.0, y + 30.0);
        self.button(A_SAW, x0 + 172.0, y + 72.0, 30.0, "SAW");
        self.button(A_PULSE, x0 + 206.0, y + 72.0, 30.0, "PULSE");
        self.knob(A_PW, x0 + 258.0, y + 72.0, 16.0, "PULSE WIDTH");
        self.button(A_SYNC, x0 + 308.0, y + 72.0, 30.0, "SYNC");
        let model = A_MODELS[self.idx(A_MODEL).min(3)];
        self.text(x1 - 4.0, y + 126.0, Align2::RIGHT_CENTER, model, 5.8, theme::TEXT_FAINT);
    }

    fn osc_b(&self) {
        let [x0, x1] = C2;
        let y = R2;
        self.section("OSCILLATOR B", x0, x1, y, OSC_COL);
        self.knob(B_FREQ, x0 + 40.0, y + 72.0, 16.0, "FREQUENCY");
        self.knob(B_FINE, x0 + 86.0, y + 72.0, 11.0, "FINE");
        self.octave(B_OCT, x0 + 128.0, y + 76.0);
        self.caption("SHAPE", x0 + 152.0, x0 + 248.0, y + 30.0);
        self.button(B_SAW, x0 + 166.0, y + 72.0, 28.0, "SAW");
        self.button(B_TRI, x0 + 198.0, y + 72.0, 28.0, "TRI");
        self.button(B_PULSE, x0 + 230.0, y + 72.0, 28.0, "PULSE");
        self.knob(B_PW, x0 + 272.0, y + 72.0, 14.0, "PULSE WIDTH");
        self.button(B_LO, x0 + 318.0, y + 54.0, 32.0, "LO FREQ");
        self.button(B_KBD, x0 + 318.0, y + 92.0, 32.0, "KYBD");
        let model = B_MODELS[self.idx(B_MODEL).min(4)];
        self.text(x1 - 4.0, y + 126.0, Align2::RIGHT_CENTER, model, 5.8, theme::TEXT_FAINT);
    }

    fn mixer(&self) {
        let [x0, x1] = C3;
        let y = R1;
        self.section("MIXER", x0, x1, y, MIX_COL);
        self.knob(MIX_A, x0 + 30.0, y + 72.0, 14.0, "OSC A");
        self.knob(MIX_B, x0 + 84.0, y + 72.0, 14.0, "OSC B");
        let third = if self.on(MIX_FB) { "FEEDBACK" } else { "NOISE" };
        self.knob(MIX_NOISE, x0 + 138.0, y + 72.0, 14.0, third);
        let _ = x1;
    }

    fn unison(&self) {
        let p = self.p;
        let [x0, x1] = C3;
        let y = R2;
        self.section("UNISON", x0, x1, y, VOICE_COL);
        self.button(UNISON, x0 + 30.0, y + 52.0, 40.0, "UNISON");
        let stack = if self.on(UNISON) { format!("×{:.0}", p.get(UNI_VOICES)) } else { format!("{:.0} voices", p.get(VOICES)) };
        self.text(x0 + 30.0, y + 72.0, Align2::CENTER_CENTER, stack, 6.2, theme::TEXT_DIM);
        self.knob(GLIDE, x0 + 86.0, y + 58.0, 13.0, "GLIDE");
        self.knob(DETUNE, x0 + 140.0, y + 58.0, 13.0, "VOICE DETUNE");
        // A light per voice.
        let used = if self.on(UNISON) { self.idx(UNI_VOICES) } else { self.idx(VOICES) };
        for i in 0..MAX_VOICES {
            let (cx, cy) = (x0 + 16.0 + 19.5 * (i % 8) as f32, y + 110.0 + 15.0 * (i / 8) as f32);
            let level = p.viz[viz::LEVEL + i].get();
            if i < used || level > 1e-3 {
                self.light(cx, cy, 3.0, level.sqrt(), VOICE_COL);
            } else {
                self.v.painter.circle_filled(self.v.p(cx, cy), 1.2 * self.z(), theme::CONTROL_HI);
            }
        }
        let _ = x1;
    }

    fn filter(&self) {
        let [x0, x1] = C4;
        let y = R1;
        self.section("FILTER", x0, x1, y, FILT_COL);
        self.knob(CUTOFF, x0 + 64.0, y + 74.0, 21.0, "CUTOFF");
        self.knob(RESO, x0 + 160.0, y + 74.0, 16.0, "RESONANCE");
        let env = if self.on(FENV_INV) { "ENV AMOUNT −" } else { "ENV AMOUNT" };
        self.knob(ENV_AMT, x0 + 254.0, y + 74.0, 16.0, env);
        self.knob(KEY_AMT, x0 + 348.0, y + 74.0, 16.0, "KEYBOARD");
        let model = FILTER_MODELS[self.idx(FILTER_MODEL).min(3)];
        self.text(x1 - 4.0, y + 126.0, Align2::RIGHT_CENTER, format!("4-pole low pass · {model}"), 5.8, theme::TEXT_FAINT);
    }

    fn envelope(&self, title: &str, y: f32, vel: usize, adsr: [usize; 4]) {
        let [x0, x1] = C4;
        self.section(title, x0, x1, y, ENV_COL);
        self.knob(vel, x0 + 30.0, y + 60.0, 6.5, "");
        self.text(x0 + 30.0, y + 74.0, Align2::CENTER_CENTER, "VEL", 5.4, theme::TEXT_FAINT);
        for (n, (k, label)) in adsr.into_iter().zip(["ATTACK", "DECAY", "SUSTAIN", "RELEASE"]).enumerate() {
            self.knob(k, x0 + 100.0 + 84.0 * n as f32, y + 72.0, 16.0, label);
        }
        // The shape, as a line.
        let (v, p) = (self.v, self.p);
        let (a, d, s, r) = (p.get(adsr[0]), p.get(adsr[1]), p.get(adsr[2]), p.get(adsr[3]));
        let scale = |t: f32| (1.0 + t * 8.0).ln();
        let total = scale(a) + scale(d) + 0.6 + scale(r);
        let w = 110.0;
        let (gx, gy, gh) = (x1 - w - 4.0, y + 118.0, 16.0);
        let px = |t: f32| gx + w * t / total.max(1e-3);
        let pts = [
            pos2(gx, gy + gh),
            pos2(px(scale(a)), gy),
            pos2(px(scale(a) + scale(d)), gy + gh * (1.0 - s)),
            pos2(px(scale(a) + scale(d) + 0.6), gy + gh * (1.0 - s)),
            pos2(px(total), gy + gh),
        ];
        let pts: Vec<Pos2> = pts.iter().map(|q| v.p(q.x, q.y)).collect();
        v.painter.add(Shape::line(pts, self.stroke(1.2, alpha(ENV_COL, 0.8))));
    }

    fn filter_env(&self) {
        self.envelope("FILTER ENVELOPE", R2, FENV_VEL, [F_ATTACK, F_DECAY, F_SUSTAIN, F_RELEASE]);
    }

    fn amplifier(&self) {
        self.envelope("AMPLIFIER", R3, AENV_VEL, [A_ATTACK, A_DECAY, A_SUSTAIN, A_RELEASE]);
    }

    fn matrix(&self) {
        let p = self.p;
        let x0 = C2[0];
        let x1 = C3[1];
        let y = R3;
        self.section("MOD MATRIX", x0, x1, y, MOD_COL);
        let page = (p.data[data::MM_PAGE].get().round() as usize).min(1);
        for (n, label) in ["MM A", "MM B"].into_iter().enumerate() {
            let resp = self.push(("mm", n), x0 + 28.0 + 44.0 * n as f32, y + 29.0, 40.0, label, n == page, if n == 0 { "Slots 1 and 2" } else { "Slots 3 and 4" });
            if resp.clicked() {
                p.data[data::MM_PAGE].set(n as f32);
            }
        }
        for side in 0..2 {
            let k = page * 2 + side;
            let sx = x0 + 6.0 + 262.0 * side as f32;
            let live = self.idx(sp(k, slot::SOURCE)) != 0 && self.idx(sp(k, slot::DEST)) != 0;
            self.text(sx + 154.0, y + 29.0, Align2::RIGHT_CENTER, format!("SLOT {}", k + 1), 6.0, if live { MOD_COL } else { theme::TEXT_FAINT });
            self.text(sx, y + 47.0, Align2::LEFT_CENTER, "SOURCE", 5.4, theme::TEXT_FAINT);
            self.selector(sp(k, slot::SOURCE), sx, y + 52.0, sx + 154.0, y + 68.0, &SOURCES, 7.2);
            self.text(sx, y + 79.0, Align2::LEFT_CENTER, "DESTINATION", 5.4, theme::TEXT_FAINT);
            self.selector(sp(k, slot::DEST), sx, y + 84.0, sx + 154.0, y + 100.0, &DESTS, 7.2);
            self.knob(sp(k, slot::DEPTH), sx + 202.0, y + 74.0, 16.0, "DEPTH");
            let mods = [(slot::CURVE, &CURVES[..], "CURVE"), (slot::RECTIFY, &RECTIFIERS[..], "RECT"), (slot::QUANTIZE, &QUANTIZERS[..], "QUANT"), (slot::SH, &SH_SOURCES[..], "S+H"), (slot::SLEW, &SLEWS[..], "SLEW")];
            for (n, (f, names, label)) in mods.into_iter().enumerate() {
                let mx = sx + 50.0 * n as f32;
                self.text(mx + 23.0, y + 112.0, Align2::CENTER_CENTER, label, 5.2, theme::TEXT_FAINT);
                self.selector(sp(k, f), mx, y + 118.0, mx + 46.0, y + 132.0, names, 5.4);
            }
        }
        let _ = mods::SLOTS;
    }

    // --- Tweaks --------------------------------------------------------

    /// A group of tweaks: an outlined box with its name.
    fn board(&self, title: &str, x0: f32, y0: f32, x1: f32, y1: f32, col: Color32) {
        self.col.set(col);
        let (v, z) = (self.v, self.z());
        v.painter.rect_stroke(v.r(x0, y0, x1, y1), 6.0 * z, self.stroke(1.0, alpha(col, 0.35)), StrokeKind::Inside);
        self.strong(x0 + 12.0, y0 + 13.0, Align2::LEFT_CENTER, title, 7.5, col);
    }

    /// A jumper: pins in a row, the cap over two of them, a label at each
    /// end (or under each place, for three). Click to move it on.
    fn jumper(&self, k: usize, cx: f32, cy: f32, title: &str, labels: &[&str]) {
        let (v, z) = (self.v, self.z());
        let col = self.col.get();
        let count = labels.len().max(2);
        let pins = if count == 2 { 3 } else { count + 1 };
        let pitch = if count == 2 { 12.0 } else { 20.0 };
        let w = pitch * (pins - 1) as f32;
        let left = cx - w * 0.5;
        let rect = v.r(left - 28.0, cy - 16.0, left + w + 28.0, cy + 18.0);
        let resp = v.ui.interact(rect, self.uid(("jumper", k)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(DEF.params[k].name);
        v.hit(resp.rect, ParamRef::Module(k));
        let cur = self.idx(k).min(count - 1);
        if resp.clicked() {
            self.set(k, ((cur + 1) % count) as f32);
        }
        self.text(cx, cy - 13.0, Align2::CENTER_CENTER, title, 6.0, theme::TEXT_DIM);
        for i in 0..pins {
            let c = v.p(left + pitch * i as f32, cy);
            v.painter.rect_stroke(Rect::from_center_size(c, vec2(4.0, 4.0) * z), 0.5 * z, self.stroke(0.9, theme::TEXT_FAINT), StrokeKind::Inside);
        }
        let a = v.p(left + pitch * cur as f32 - 4.0, cy - 5.0);
        let b = v.p(left + pitch * (cur + 1) as f32 + 4.0, cy + 5.0);
        v.painter.rect_filled(Rect::from_two_pos(a, b), 2.0 * z, alpha(col, 0.2));
        v.painter.rect_stroke(Rect::from_two_pos(a, b), 2.0 * z, self.stroke(1.0, if resp.hovered() { theme::TEXT } else { col }), StrokeKind::Inside);
        if count == 2 {
            self.text(left - 8.0, cy, Align2::RIGHT_CENTER, labels[0], 5.8, if cur == 0 { col } else { theme::TEXT_FAINT });
            self.text(left + w + 8.0, cy, Align2::LEFT_CENTER, labels[1], 5.8, if cur == 1 { col } else { theme::TEXT_FAINT });
        } else {
            for (i, l) in labels.iter().enumerate() {
                self.text(left + pitch * (i as f32 + 0.5), cy + 12.0, Align2::CENTER_CENTER, l, 5.2, if cur == i { col } else { theme::TEXT_FAINT });
            }
        }
    }

    fn tweaks(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        // Jumpers.
        self.board("JUMPERS", 24.0, R1, 640.0, 300.0, MOD_COL);
        // Each: parameter, title, labels (the parameter's 0 first), colour.
        let jumpers: [(usize, &str, &[&str], Color32); 9] = [
            (LFO_DC, "LFO", &["NO DC", "DC"], MOD_COL),
            (LFO_INV, "LFO SAW", &["N", "INV"], MOD_COL),
            (WM_SH, "WHEEL MOD", &["NOISE", "S&H"], MOD_COL),
            (B_INV, "OSC B SAW", &["N", "INV"], OSC_COL),
            (NOTE_PRIO, "NOTE PRIORITY", &["LOW", "HIGH", "LAST"], VOICE_COL),
            (REALLOC, "REALLOCATE", &["OFF", "ON"], VOICE_COL),
            (MIX_FB, "MIXER", &["NOISE", "FB"], MIX_COL),
            (FENV_INV, "FILTER ENV", &["+", "−"], FILT_COL),
            (KEY_PB, "FILTER KEY", &["KEY", "KEY+PB"], FILT_COL),
        ];
        for (n, (k, title, labels, col)) in jumpers.into_iter().enumerate() {
            self.col.set(col);
            let (cx, cy) = (124.0 + 200.0 * (n % 3) as f32, R1 + 62.0 + 66.0 * (n / 3) as f32);
            self.jumper(k, cx, cy, title, labels);
        }
        // Selectors.
        self.board("SELECTORS", 652.0, R1, 1256.0, 300.0, OSC_COL);
        let sels: [(usize, &str, &[&str], Color32); 4] = [
            (A_MODEL, "OSC A", &A_MODELS, OSC_COL),
            (B_MODEL, "OSC B", &B_MODELS, OSC_COL),
            (FILTER_MODEL, "FILTER", &FILTER_MODELS, FILT_COL),
            (ENV_MODEL, "ENVELOPES", &ENV_MODELS, ENV_COL),
        ];
        for (n, (k, title, names, col)) in sels.into_iter().enumerate() {
            self.col.set(col);
            let x = 676.0 + 142.0 * n as f32;
            self.text(x + 55.0, R1 + 44.0, Align2::CENTER_CENTER, title, 6.2, theme::TEXT_DIM);
            self.selector(k, x, R1 + 52.0, x + 110.0, R1 + 70.0, names, 7.2);
        }
        self.col.set(MOD_COL);
        let ranges: Vec<String> = (0..BEND_RANGES).map(|i| format!("{:.0} st", bend_semitones(i as f32))).collect();
        let ranges: Vec<&str> = ranges.iter().map(|s| s.as_str()).collect();
        for (n, (k, title)) in [(PB_DOWN, "BEND DOWN"), (PB_UP, "BEND UP")].into_iter().enumerate() {
            let x = 676.0 + 142.0 * n as f32;
            self.text(x + 55.0, R1 + 104.0, Align2::CENTER_CENTER, title, 6.2, theme::TEXT_DIM);
            self.selector(k, x, R1 + 112.0, x + 110.0, R1 + 130.0, &ranges, 7.2);
        }
        let counts: [&str; MAX_VOICES] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16"];
        for (n, (k, title)) in [(VOICES, "POLY VOICES"), (UNI_VOICES, "UNISON VOICES")].into_iter().enumerate() {
            let x = 960.0 + 142.0 * n as f32;
            self.text(x + 55.0, R1 + 104.0, Align2::CENTER_CENTER, title, 6.2, theme::TEXT_DIM);
            let rect = v.r(x, R1 + 112.0, x + 110.0, R1 + 130.0);
            let resp = v.ui.interact(rect, self.uid(("count", k)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
            v.painter.rect_filled(rect, 3.0 * z, theme::INSET);
            v.painter.rect_stroke(rect, 3.0 * z, self.stroke(1.0, if resp.hovered() { VOICE_COL } else { alpha(VOICE_COL, 0.45) }), StrokeKind::Inside);
            self.text(x + 55.0, R1 + 121.0, Align2::CENTER_CENTER, format!("{:.0}", p.get(k)), 7.2, theme::TEXT);
            egui::Popup::menu(&resp).show(|ui| {
                for (i, name) in counts.iter().enumerate() {
                    if ui.selectable_label(self.idx(k) == i + 1, *name).clicked() {
                        self.set(k, (i + 1) as f32);
                        ui.close();
                    }
                }
            });
        }
        for (n, line) in [
            "OSC: P5 inverts the pulse against the saw · IDEAL is crisp and steady · P5 OLD drifts",
            "FILTER: POLY is darker and only opens fully when modulated · DRIVEN saturates",
            "ENVELOPES: ONE SHOT ignores the key · PIANO dampens when the key goes up",
        ]
        .into_iter()
        .enumerate()
        {
            self.text(954.0, R1 + 168.0 + 18.0 * n as f32, Align2::CENTER_CENTER, line, 5.8, theme::TEXT_FAINT);
        }

        // Wheels and controllers.
        self.board("WHEELS & CONTROLLERS", 24.0, 308.0, 640.0, 556.0, MOD_COL);
        self.knob(MOD_WHEEL, 100.0, 380.0, 16.0, "MOD WHEEL");
        self.knob(WM_LOW, 190.0, 380.0, 16.0, "LOW LIMIT");
        self.knob(WM_HIGH, 280.0, 380.0, 16.0, "HIGH LIMIT");
        self.text(190.0, 420.0, Align2::CENTER_CENTER, "the wheel moves wheel-mod between the limits", 5.8, theme::TEXT_FAINT);
        for (n, (k, title)) in [(CC_A, "CONTROL A"), (CC_B, "CONTROL B")].into_iter().enumerate() {
            let x = 380.0 + 120.0 * n as f32;
            self.text(x + 40.0, 360.0, Align2::CENTER_CENTER, title, 6.2, theme::TEXT_DIM);
            self.drag_box(k, x, 368.0, x + 80.0, 388.0);
        }
        self.text(480.0, 404.0, Align2::CENTER_CENTER, "controllers for the matrix (drag)", 5.8, theme::TEXT_FAINT);
        self.text(332.0, 476.0, Align2::CENTER_CENTER, "Mod wheel: CC 1 · Sustain: CC 64 · Aftertouch and bend are matrix sources too", 6.0, theme::TEXT_FAINT);
        self.text(332.0, 494.0, Align2::CENTER_CENTER, format!("Program Change 1–{PROGRAMS} picks a program"), 6.0, theme::TEXT_FAINT);

        // Voice panning.
        self.board("VOICE PANNING", 652.0, 308.0, 1256.0, 556.0, VOICE_COL);
        for i in 0..MAX_VOICES {
            let (cx, cy) = (706.0 + 70.0 * (i % 8) as f32, 380.0 + 84.0 * (i / 8) as f32);
            self.knob_with(PAN + i, cx, cy, 11.0, &format!("{}", i + 1), None);
            self.light(cx + 17.0, cy - 15.0, 2.5, p.viz[viz::LEVEL + i].get().sqrt(), VOICE_COL);
        }
        self.text(954.0, 540.0, Align2::CENTER_CENTER, "each voice's place: left to right · double-click for the default spread", 5.8, theme::TEXT_FAINT);
    }

    // --- The lower panel -----------------------------------------------

    fn lower(&self, view: &mut View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let y0 = LOWER;
        v.painter.line_segment([v.p(16.0, y0 - 6.0), v.p(W - 16.0, y0 - 6.0)], self.stroke(1.0, theme::NODE_BORDER));
        let effects = p.data[data::LOWER].get() > 0.5;
        // The distortion, then the chain's effects: which are on.
        let lit: Vec<bool> = std::iter::once(self.on(DIST_ON)).chain((0..5).map(|e| self.on(SWITCHES[e]))).collect();
        for (n, label) in ["KEYS", "EFFECTS"].into_iter().enumerate() {
            let rect = v.r(18.0, y0 + 6.0 + 72.0 * n as f32, 80.0, y0 + 70.0 + 72.0 * n as f32);
            let on = (n == 1) == effects;
            let resp = v.ui.interact(rect, self.uid(("lower", n)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
            let col = if n == 0 { ARP_COL } else { FX_COL };
            if on {
                v.painter.rect_filled(rect, 6.0 * z, alpha(col, 0.18));
            }
            let edge = if on {
                col
            } else if resp.hovered() {
                theme::TEXT_DIM
            } else {
                theme::CONTROL_HI
            };
            v.painter.rect_stroke(rect, 6.0 * z, self.stroke(1.0, edge), StrokeKind::Inside);
            let cy = y0 + 30.0 + 72.0 * n as f32;
            self.strong(49.0, cy, Align2::CENTER_CENTER, label, 7.0, if on { col } else { theme::TEXT_DIM });
            if n == 1 {
                for (i, &on) in lit.iter().enumerate() {
                    let c = v.p(34.0 + 6.0 * i as f32, cy + 14.0);
                    if on {
                        v.painter.circle_filled(c, 2.0 * z, FX_COL);
                    } else {
                        v.painter.circle_stroke(c, 2.0 * z, self.stroke(0.8, alpha(FX_COL, 0.35)));
                    }
                }
                let count = lit.iter().filter(|&&l| l).count();
                self.text(49.0, cy + 24.0, Align2::CENTER_CENTER, if count == 0 { "all off".to_owned() } else { format!("{count} on") }, 5.4, theme::TEXT_FAINT);
            }
            let resp = resp.on_hover_text(if n == 0 { "The keyboard, the wheels and the arpeggiator" } else { "Distortion, the effects chain and its five effects" });
            if resp.clicked() {
                p.data[data::LOWER].set(n as f32);
            }
        }
        if effects {
            self.effects(view);
        } else {
            self.wheels();
            self.arp();
            self.keyboard();
        }
    }

    fn wheels(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let (top, bottom) = (LOWER + 8.0, H - 22.0);
        let travel = bottom - top - 10.0;
        self.col.set(MOD_COL);
        // The bend wheel springs back; the mod wheel stays.
        for (n, label) in ["PB", "MOD"].into_iter().enumerate() {
            let x = LOW_X + 2.0 + 30.0 * n as f32;
            let rect = v.r(x, top, x + 20.0, bottom);
            v.painter.rect_filled(rect, 4.0 * z, theme::INSET);
            v.painter.rect_stroke(rect, 4.0 * z, self.stroke(1.0, theme::CONTROL_HI), StrokeKind::Inside);
            let value = if n == 0 { p.viz[viz::BEND].get() * 0.5 + 0.5 } else { p.viz[viz::WHEEL].get() };
            let y = bottom - 5.0 - travel * value.clamp(0.0, 1.0);
            if n == 0 {
                let mid = bottom - 5.0 - travel * 0.5;
                v.painter.line_segment([v.p(x + 4.0, mid), v.p(x + 16.0, mid)], self.stroke(0.8, theme::TEXT_FAINT));
            } else {
                v.painter.rect_filled(v.r(x + 3.0, y, x + 17.0, bottom - 3.0), 2.0 * z, alpha(MOD_COL, 0.18));
            }
            v.painter.rect_filled(v.r(x + 2.0, y - 2.5, x + 18.0, y + 2.5), 1.5 * z, MOD_COL);
            let resp = v.ui.interact(rect, self.uid(("wheel", n)), Sense::click_and_drag()).on_hover_cursor(CursorIcon::ResizeVertical);
            if n == 1 {
                v.hit(resp.rect, ParamRef::Module(MOD_WHEEL));
            }
            if let Some(pos) = resp.interact_pointer_pos().filter(|_| resp.is_pointer_button_down_on()) {
                let t = ((v.p(0.0, bottom - 5.0).y - pos.y) / (travel * z)).clamp(0.0, 1.0);
                if n == 0 {
                    p.viz[viz::PANEL_BEND].set((t * 2.0 - 1.0).clamp(-1.0, 1.0));
                } else {
                    self.set(MOD_WHEEL, t);
                }
            } else if n == 0 && p.viz[viz::PANEL_BEND].get() != 0.0 {
                p.viz[viz::PANEL_BEND].set(0.0);
            }
            self.text(x + 10.0, bottom + 9.0, Align2::CENTER_CENTER, label, 5.6, theme::TEXT_FAINT);
        }
        // The wheel-mod limits: triangles beside the mod wheel.
        let x = LOW_X + 56.0;
        for (k, filled) in [(WM_LOW, true), (WM_HIGH, false)] {
            let value = p.get(k);
            let y = bottom - 5.0 - travel * value;
            let pts = vec![v.p(x, y), v.p(x + 7.0, y - 4.5), v.p(x + 7.0, y + 4.5)];
            v.painter.add(Shape::convex_polygon(pts, if filled { MOD_COL } else { theme::INSET }, self.stroke(1.0, MOD_COL)));
            let rect = v.r(x - 2.0, y - 6.0, x + 11.0, y + 6.0);
            let resp = v.ui.interact(rect, self.uid(("limit", k)), Sense::drag()).on_hover_cursor(CursorIcon::ResizeVertical).on_hover_text(DEF.params[k].name);
            if resp.dragged() {
                let dy = resp.drag_delta().y / z;
                self.set(k, (value - dy / travel).clamp(0.0, 1.0));
            }
        }
    }

    fn arp(&self) {
        let (x0, x1) = (LOW_X + 82.0, KEYS_X[0] - 14.0);
        let y = LOWER;
        self.section("ARPEGGIATOR", x0, x1, y, ARP_COL);
        self.button(ARP_ON, x0 + 20.0, y + 38.0, 34.0, "ARP");
        self.button(ARP_LATCH, x0 + 20.0, y + 62.0, 34.0, "LATCH");
        self.text(x0 + 46.0, y + 25.0, Align2::LEFT_CENTER, "MODE", 5.4, theme::TEXT_FAINT);
        self.selector(ARP_MODE, x0 + 46.0, y + 30.0, x0 + 110.0, y + 46.0, &ARP_MODES, 6.8);
        let rates: Vec<&str> = ARP_RATES.iter().map(|r| r.0).collect();
        self.text(x0 + 116.0, y + 25.0, Align2::LEFT_CENTER, "RATE", 5.4, theme::TEXT_FAINT);
        self.selector(ARP_RATE, x0 + 116.0, y + 30.0, x0 + 162.0, y + 46.0, &rates, 6.8);
        self.text(x0 + 170.0, y + 25.0, Align2::LEFT_CENTER, "OCTAVES", 5.4, theme::TEXT_FAINT);
        let oct = self.idx(ARP_OCT).clamp(1, 4);
        for n in 1..=4 {
            let resp = self.push(("oct", n), x0 + 162.0 + 14.0 * n as f32, y + 38.0, 12.0, &format!("{n}"), oct == n, "Octaves");
            if resp.clicked() {
                self.set(ARP_OCT, n as f32);
            }
        }
        self.knob(ARP_GATE, x0 + 72.0, y + 82.0, 12.0, "GATE");
        self.knob(ARP_SWING, x0 + 120.0, y + 82.0, 12.0, "SWING");
        let p = self.p;
        let note = p.viz[viz::ARP_NOTE].get();
        let held = p.viz[viz::HELD].get() as usize;
        let state = if !self.on(ARP_ON) {
            "off: keys play the voices".to_owned()
        } else if note >= 0.0 {
            format!("{} · {held} {}", note_label(note as i32), if held == 1 { "key" } else { "keys" })
        } else if held > 0 {
            format!("{held} {}", if held == 1 { "key" } else { "keys" })
        } else {
            "play a chord".to_owned()
        };
        self.text(x0 + 196.0, y + 76.0, Align2::CENTER_CENTER, state, 5.8, theme::TEXT_DIM);
        if self.on(ARP_LATCH) && held > 0 {
            let resp = self.push("unlatch", x0 + 196.0, y + 96.0, 40.0, "CLEAR", false, "Let go of the latched chord");
            if resp.clicked() {
                p.viz[viz::UNLATCH].set(p.viz[viz::UNLATCH].get() + 1.0);
            }
        }
        let _ = x1;
    }

    fn keyboard(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let [x0, x1] = KEYS_X;
        let (top, bottom) = (LOWER + 8.0, H - 14.0);
        let whites: Vec<u8> = (LOW_KEY..LOW_KEY + KEYS as u8).filter(|n| !is_black(*n)).collect();
        let ww = (x1 - x0) / whites.len() as f32;
        // Black keys: about 60 % as long and wide as the white ones.
        let black_bottom = top + (bottom - top) * 0.62;
        let mut lit = [false; 128];
        for i in 0..MAX_VOICES {
            if p.viz[viz::GATE + i].get() > 0.5 {
                let n = p.viz[viz::NOTE + i].get();
                if (0.0..128.0).contains(&n) {
                    lit[n as usize] = true;
                }
            }
        }
        let panel_key = p.viz[viz::PANEL_KEY].get().round() as i32 - 1;
        if (0..128).contains(&panel_key) {
            lit[panel_key as usize] = true;
        }
        let white_x = |n: u8| whites.iter().position(|&w| w == n).map(|i| x0 + ww * i as f32);
        let black_span = |n: u8| {
            let left = white_x(n - 1).unwrap_or(x0);
            let bx = left + ww * 0.7;
            (bx, bx + ww * 0.6)
        };
        let full = v.r(x0, top, x1, bottom);
        let resp = v.ui.interact(full, self.uid("keys"), Sense::click_and_drag()).on_hover_cursor(CursorIcon::PointingHand);
        let mut pressed: Option<u8> = None;
        if let Some(pos) = resp.interact_pointer_pos().filter(|_| resp.is_pointer_button_down_on()) {
            let lx = (pos.x - v.min.x) / z;
            let ly = (pos.y - v.min.y) / z;
            if ly < black_bottom {
                pressed = (LOW_KEY..LOW_KEY + KEYS as u8).filter(|n| is_black(*n)).find(|&n| {
                    let (a, b) = black_span(n);
                    lx >= a && lx < b
                });
            }
            if pressed.is_none() {
                let i = ((lx - x0) / ww).floor().clamp(0.0, whites.len() as f32 - 1.0) as usize;
                pressed = Some(whites[i]);
            }
        }
        let want = pressed.map_or(0.0, |n| n as f32 + 1.0);
        if p.viz[viz::PANEL_KEY].get() != want {
            p.viz[viz::PANEL_KEY].set(want);
        }
        let key_col = self.accent;
        for (i, &n) in whites.iter().enumerate() {
            let r = v.r(x0 + ww * i as f32 + 0.6, top, x0 + ww * (i + 1) as f32 - 0.6, bottom);
            let on = lit[n as usize];
            v.painter.rect_filled(r, 2.5 * z, if on { alpha(key_col, 0.55) } else { theme::CONTROL });
            v.painter.rect_stroke(r, 2.5 * z, self.stroke(1.0, if on { key_col } else { theme::CONTROL_HI }), StrokeKind::Inside);
            if n % 12 == 0 {
                self.text(x0 + ww * (i as f32 + 0.5), bottom - 8.0, Align2::CENTER_CENTER, note_label(n as i32), 5.2, theme::TEXT_FAINT);
            }
        }
        for n in (LOW_KEY..LOW_KEY + KEYS as u8).filter(|n| is_black(*n)) {
            let (a, b) = black_span(n);
            let r = v.r(a, top, b, black_bottom);
            let on = lit[n as usize];
            v.painter.rect_filled(r, 2.0 * z, if on { theme::mix(key_col, theme::INSET, 0.35) } else { theme::INSET });
            v.painter.rect_stroke(r, 2.0 * z, self.stroke(1.0, if on { key_col } else { theme::CONTROL_HI }), StrokeKind::Inside);
        }
    }

    // --- Effects -------------------------------------------------------

    fn effects(&self, view: &mut View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let y = LOWER;
        let ky = y + 72.0;
        let ky2 = y + 116.0;

        // Distortion (each voice its own).
        let (x0, x1) = (LOW_X, LOW_X + 156.0);
        self.section("DISTORTION", x0, x1, y, FX_COL);
        self.button(DIST_ON, x0 + 16.0, y + 34.0, 28.0, "ON");
        self.selector(DIST_TYPE, x0 + 36.0, y + 26.0, x1 - 2.0, y + 42.0, &DIST_TYPES, 6.8);
        let corrode = self.idx(DIST_TYPE) == 3;
        self.knob(DIST_AMOUNT, x0 + 26.0, ky + 10.0, 14.0, if corrode { "RATE" } else { "AMOUNT" });
        self.knob(DIST_TONE, x0 + 78.0, ky + 10.0, 14.0, if corrode { "CRUSH" } else { "TONE" });
        self.knob(DIST_MIX, x0 + 130.0, ky + 10.0, 14.0, "MIX");
        self.text((x0 + x1) * 0.5, ky2 + 22.0, Align2::CENTER_CENTER, "one per voice, before the chain", 5.6, theme::TEXT_FAINT);

        // The chain: click to switch, drag to move.
        let (x0, x1) = (LOW_X + 168.0, LOW_X + 256.0);
        self.section("CHAIN", x0, x1, y, FX_COL);
        let order = sb::fx_order(p);
        let cell_h = 22.0;
        let top = y + 24.0;
        let pointer = v.ui.input(|i| i.pointer.interact_pos());
        let mut drop_at: Option<usize> = None;
        for (slot_i, &e) in order.iter().enumerate() {
            let cy = top + cell_h * slot_i as f32;
            let rect = v.r(x0, cy, x1, cy + cell_h - 4.0);
            let resp = v.ui.interact(rect, self.uid(("cell", e)), Sense::click_and_drag()).on_hover_cursor(CursorIcon::Grab);
            let on = self.on(SWITCHES[e]);
            let dragging = view.dragging == Some(e);
            if on || dragging {
                v.painter.rect_filled(rect, 3.0 * z, alpha(FX_COL, if dragging { 0.35 } else { 0.18 }));
            }
            let edge = if on {
                FX_COL
            } else if resp.hovered() {
                theme::TEXT_DIM
            } else {
                theme::CONTROL_HI
            };
            v.painter.rect_stroke(rect, 3.0 * z, self.stroke(1.0, edge), StrokeKind::Inside);
            self.text(x0 + 6.0, cy + (cell_h - 4.0) * 0.5, Align2::LEFT_CENTER, format!("{}", slot_i + 1), 5.4, theme::TEXT_FAINT);
            self.text(x0 + 16.0, cy + (cell_h - 4.0) * 0.5, Align2::LEFT_CENTER, EFFECTS[e], 6.0, if on { FX_COL } else { theme::TEXT_DIM });
            let resp = resp.on_hover_text("Click: switch in or out · drag: move it in the chain");
            if resp.clicked() {
                self.set(SWITCHES[e], if on { 0.0 } else { 1.0 });
            }
            if resp.drag_started() {
                view.dragging = Some(e);
            }
            if resp.drag_stopped()
                && let Some(pos) = pointer
            {
                let ly = (pos.y - v.min.y) / z;
                drop_at = Some((((ly - top) / cell_h).floor().max(0.0) as usize).min(4));
            }
        }
        if let (Some(e), Some(to)) = (view.dragging, drop_at) {
            let mut list: Vec<usize> = order.iter().copied().filter(|&x| x != e).collect();
            list.insert(to.min(list.len()), e);
            for (n, x) in list.into_iter().enumerate() {
                p.data[data::FX_ORDER + n].set(x as f32);
            }
            view.dragging = None;
        }
        if !v.ui.input(|i| i.pointer.any_down()) {
            view.dragging = None;
        }

        // Tape.
        let (x0, x1) = (LOW_X + 268.0, LOW_X + 374.0);
        self.section("TAPE", x0, x1, y, FX_COL);
        self.button(TAPE_ON, x0 + 14.0, y + 34.0, 24.0, "ON");
        self.selector(TAPE_FLAVOR, x0 + 30.0, y + 26.0, x1 - 2.0, y + 42.0, &TAPE_FLAVORS, 6.8);
        self.knob(TAPE_GAIN, x0 + 30.0, ky + 10.0, 15.0, "INPUT GAIN");
        self.button(TAPE_HUSH, x0 + 84.0, ky + 10.0, 36.0, "NO HISS");

        // Echo.
        let (x0, x1) = (LOW_X + 386.0, LOW_X + 660.0);
        self.section("ECHO", x0, x1, y, FX_COL);
        self.button(ECHO_ON, x0 + 14.0, y + 34.0, 24.0, "ON");
        for (k, (fx0, fx1), names, label) in [
            (ECHO_RANGE, (30.0, 108.0), &ECHO_RANGES[..], "RANGE"),
            (ECHO_FLAVOR, (114.0, 166.0), &ECHO_FLAVORS[..], "FLAVOR"),
            (ECHO_MODE, (172.0, 226.0), &ECHO_MODES[..], "MODE"),
            (ECHO_MOD, (232.0, 272.0), &ECHO_MODS[..], "MOD"),
        ] {
            self.text(x0 + fx0, y + 21.0, Align2::LEFT_CENTER, label, 5.2, theme::TEXT_FAINT);
            self.selector(k, x0 + fx0, y + 26.0, x0 + fx1, y + 42.0, names, 6.6);
        }
        let range = self.idx(ECHO_RANGE).min(3);
        let time = p.get(ECHO_TIME);
        let time_text = match range {
            0 => format!("{:.1} ms", (0.0006 + time * 0.03) * 1000.0),
            1 => crate::audio::params::format_seconds(0.01 * 200f32.powf(time)),
            2 => format!("{:.2} × 1/16", 1.0 + 7.0 * time),
            _ => format!("{:.2} × 1/4", 1.0 + 7.0 * time),
        };
        self.knob_with(ECHO_TIME, x0 + 54.0, ky + 10.0, 15.0, "TIME", Some(time_text));
        self.knob(ECHO_MIX, x0 + 140.0, ky + 10.0, 15.0, "MIX");
        self.knob(ECHO_REGEN, x0 + 226.0, ky + 10.0, 15.0, "REGEN");

        // Res·EQ.
        let (x0, x1) = (LOW_X + 672.0, LOW_X + 880.0);
        self.section("RES·EQ", x0, x1, y, FX_COL);
        self.button(RESEQ_ON, x0 + 14.0, y + 34.0, 24.0, "ON");
        let res = self.on(RESEQ_MODE);
        self.choice(RESEQ_MODE, x0 + 44.0, y + 34.0, 28.0, 26.0, &["EQ", "RES"]);
        self.knob(RESEQ_Q, x0 + 22.0, ky + 32.0, 12.0, "Q / RES");
        for (n, (f, g, label)) in [(RESEQ_LOW_F, RESEQ_LOW, "LOW"), (RESEQ_MID_F, RESEQ_MID, "MID"), (RESEQ_HIGH_F, RESEQ_HIGH, "HIGH")].into_iter().enumerate() {
            let cx = x0 + 90.0 + 46.0 * n as f32;
            self.knob(f, cx, ky - 4.0, 11.0, label);
            let gain = p.get(g);
            let shown = if res { format!("{:.0}%", gain * 100.0) } else { format!("{:+.1} dB", (gain - 0.5) * 36.0) };
            self.knob_with(g, cx, ky2 - 4.0, 11.0, if res { "VOL" } else { "GAIN" }, Some(shown));
        }
        let _ = x1;

        // Plate.
        let (x0, x1) = (LOW_X + 892.0, LOW_X + 1040.0);
        self.section("PLATE", x0, x1, y, FX_COL);
        self.button(PLATE_ON, x0 + 14.0, y + 34.0, 24.0, "ON");
        let rt = crate::audio::modules::sibyl::fx::plate_rt60(p.get(PLATE_DECAY));
        self.knob(PLATE_MIX, x0 + 60.0, y + 44.0, 13.0, "DRY/WET");
        self.knob_with(PLATE_DECAY, x0 + 112.0, y + 44.0, 13.0, "DECAY", Some(crate::audio::params::format_seconds(rt)));
        self.knob(PLATE_PRE, x0 + 60.0, ky2 - 4.0, 12.0, "PREDELAY");
        self.knob(PLATE_TONE, x0 + 112.0, ky2 - 4.0, 12.0, "TONE");
        let _ = x1;

        // Conditioner.
        let (x0, x1) = (LOW_X + 1052.0, W - 24.0);
        self.section("COND", x0, x1, y, FX_COL);
        self.button(COND_ON, x0 + 14.0, y + 34.0, 24.0, "ON");
        self.knob(COND_GAIN, x0 + 58.0, y + 44.0, 12.0, "GAIN");
        self.knob(COND_PUNCH, x0 + 22.0, ky2 - 4.0, 12.0, "PUNCH");
        self.knob(COND_WIDTH, x0 + 66.0, ky2 - 4.0, 12.0, "WIDTH");
        self.light(x0 + 92.0, y + 34.0, 3.0, p.viz[viz::COND_LOAD].get(), LIGHT);
        let _ = x1;
    }
}

/// SAVE's fields while its menu is open.
#[derive(Clone, Default)]
struct SaveState {
    name: String,
    category: usize,
    status: String,
}

/// The library, read once a session.
fn library_cache(ui: &egui::Ui) -> std::sync::Arc<Vec<crate::presets::ModulePreset>> {
    let id = Id::unique("sibyl-preset-library");
    if let Some(list) = ui.data(|d| d.get_temp::<std::sync::Arc<Vec<crate::presets::ModulePreset>>>(id)) {
        return list;
    }
    let list = std::sync::Arc::new(library::load());
    ui.data_mut(|d| d.insert_temp(id, list.clone()));
    list
}

/// Writes the library back, and keeps it for the session.
fn keep_library(ui: &egui::Ui, list: Vec<crate::presets::ModulePreset>) -> Result<(), String> {
    library::save(&list)?;
    ui.data_mut(|d| d.insert_temp(Id::unique("sibyl-preset-library"), std::sync::Arc::new(list)));
    Ok(())
}

fn is_black(n: u8) -> bool {
    matches!(n % 12, 1 | 3 | 6 | 8 | 10)
}
