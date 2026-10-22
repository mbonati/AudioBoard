//! Panel of Pops 66, in three sizes picked in its header: MACHINE (the
//! rhythm buttons on a vintage faceplate, tempo, volume, balance, the extra
//! percussion levels, the step lights), MIXER (+ a strip per instrument:
//! pad, level, pan, mute) and EDITOR (+ the pattern grid and the selected
//! instrument's sound).

use egui::{Align2, Color32, CursorIcon, Id, Rect, Response, Sense, Stroke, Vec2, pos2, vec2};

use super::nodes::{NodeView, segmented};
use super::theme::{self, alpha};
use super::widgets;
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::pops::rhythms::{MAX_LEN, RHYTHMS, ROW};
use crate::audio::modules::pops::voices::{self, COUNT};
use crate::audio::modules::pops::{self as pp, INSTRUMENTS, PATTERNS, data, ip, knob, viz};
use crate::graph::{ModuleNode, ParamRef};

const W: f32 = 744.0;
const PAD: f32 = 12.0;
/// The panel's height per detail level.
const HEIGHTS: [f32; 3] = [206.0, 338.0, 664.0];
pub const SIZE: Vec2 = vec2(W, HEIGHTS[0]);
const MIXER_Y: f32 = 206.0;
const EDITOR_Y: f32 = 340.0;
/// The faceplate: brushed aluminium, black and red lettering.
const PLATE: Color32 = Color32::from_rgb(206, 208, 210);
const INK: Color32 = Color32::from_rgb(34, 34, 36);
const RED_INK: Color32 = Color32::from_rgb(196, 52, 46);
const KEY: Color32 = Color32::from_rgb(244, 244, 240);

const EDIT_TIP: &str = "Edit (and play) this pattern";
const PATTERN_ITEMS: [(&str, &str); PATTERNS] =
    [("P1", EDIT_TIP), ("P2", EDIT_TIP), ("P3", EDIT_TIP), ("P4", EDIT_TIP), ("P5", EDIT_TIP), ("P6", EDIT_TIP), ("P7", EDIT_TIP), ("P8", EDIT_TIP)];

const DETAILS: [(&str, &str); 3] = [
    ("MACHINE", "The machine: rhythms, tempo, levels"),
    ("MIXER", "Plus a strip per instrument"),
    ("EDITOR", "Plus the pattern editor and each instrument's sound"),
];

pub fn detail(p: &ModuleParams) -> usize {
    (p.data[data::DETAIL].get().round().max(0.0) as usize).min(2)
}

pub fn size(p: &ModuleParams) -> Vec2 {
    vec2(W, HEIGHTS[detail(p)])
}

/// Collapsed nodes' readout.
pub fn readout(p: &ModuleParams) -> (String, String) {
    let main = if pp::on(p, data::SOURCE) {
        format!("USER {}", p.data[data::PATTERN].get().round() as usize + 1)
    } else {
        let names: Vec<&str> = (0..ROW).filter(|&i| pp::on(p, data::RHYTHM + i)).map(|i| RHYTHMS[pp::row_rhythm(p, i)].name).collect();
        if names.is_empty() { "No rhythm".to_owned() } else { names.join(" + ") }
    };
    let clock = if pp::on(p, data::SYNC) { "board tempo".to_owned() } else { format!("{:.0} BPM", p.get(pp::TEMPO)) };
    let state = if p.viz[viz::STEP].get() >= 0.0 { "playing" } else { "stopped" };
    (main, format!("{state} · {clock}"))
}

struct Panel<'a, 'b> {
    v: &'a NodeView<'b>,
    id: NodeId,
    p: &'a ModuleParams,
    accent: Color32,
}

/// `out`: the audio output is patched; `midi`: the MIDI input and output.
pub fn draw(v: &NodeView, id: NodeId, accent: Color32, m: &ModuleNode, out: bool, midi: (bool, bool)) {
    let s = Panel { v, id, p: &m.params, accent };
    let col = |on: bool| if on { theme::TEXT_DIM } else { theme::TEXT_FAINT };
    v.text(PAD, 42.0, Align2::LEFT_CENTER, "MIDI", v.font(9.0), col(midi.0));
    v.text(W - PAD, 42.0, Align2::RIGHT_CENTER, "OUT", v.font(9.0), col(out));
    v.text(W - PAD, 60.0, Align2::RIGHT_CENTER, "MIDI", v.font(9.0), col(midi.1));
    s.machine();
    let level = detail(s.p);
    if level >= 1 {
        s.mixer();
    }
    if level >= 2 {
        s.editor();
    }
    if s.p.viz[viz::STEP].get() >= 0.0 || level >= 1 {
        v.ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
}

impl Panel<'_, '_> {
    fn z(&self) -> f32 {
        self.v.z
    }

    fn uid(&self, salt: impl std::hash::Hash + std::fmt::Debug) -> Id {
        Id::unique(("pops", self.id)).with(salt)
    }

    fn on(&self, i: usize) -> bool {
        pp::on(self.p, i)
    }

    fn flip(&self, i: usize) {
        self.p.data[i].set(if self.on(i) { 0.0 } else { 1.0 });
    }

    fn chip(&self, salt: &str, rect: Rect, label: &str, lit: bool, color: Color32, tip: &str) -> Response {
        widgets::toggle(self.v.ui, self.v.painter, self.uid(("chip", salt)), rect, label, lit, color, self.z()).on_hover_text(tip)
    }

    fn seg(&self, salt: &str, rect: Rect, items: &[(&'static str, &'static str)], current: usize) -> Option<usize> {
        let v = self.v;
        segmented(v, self.uid(("seg", salt)), rect, items.len(), current, self.accent, |k, r, col| {
            v.painter.text(r.center(), Align2::CENTER_CENTER, items[k].0, v.font(8.0), col);
            items[k].1
        })
    }

    /// Label, knob and value (`r`: the knob's radius).
    fn knob(&self, i: usize, cx: f32, row_y: f32, label: &str, r: f32, dim: bool) {
        let (v, p, z) = (self.v, self.p, self.z());
        let spec = pp::spec(i);
        if !label.is_empty() {
            v.text(cx, row_y, Align2::CENTER_CENTER, label, v.font(7.5), if dim { alpha(theme::TEXT_FAINT, 0.5) } else { theme::TEXT_FAINT });
        }
        let mut norm = spec.to_norm(p.get(i));
        let resp = widgets::knob(
            v.ui,
            v.painter,
            self.uid(("knob", i)),
            v.p(cx, row_y + 6.0 + r),
            r * z,
            &mut norm,
            spec.to_norm(spec.default),
            spec.bipolar,
            if dim { theme::TEXT_FAINT } else { self.accent },
            z,
        );
        v.hit(resp.rect, ParamRef::Module(i));
        if resp.dragged() || resp.double_clicked() {
            p.set(i, spec.denorm(norm));
        }
        let hot = resp.dragged() || resp.hovered();
        v.text(cx, row_y + 13.0 + 2.0 * r, Align2::CENTER_CENTER, pp::format(i, p.get(i)), v.mono(7.5), if hot { theme::TEXT } else { theme::TEXT_DIM });
    }

    // -----------------------------------------------------------------------
    // The machine
    // -----------------------------------------------------------------------

    fn machine(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let (y0, y1) = (34.0, 52.0);
        let power = self.on(data::POWER);
        if self.chip("power", v.r(52.0, y0, 104.0, y1), "POWER", power, Color32::from_rgb(220, 70, 60), "Power: off, the rhythm stops (MIDI notes still play)").clicked() {
            self.flip(data::POWER);
        }
        let sync = self.on(data::SYNC);
        if self.chip("sync", v.r(108.0, y0, 150.0, y1), "SYNC", sync, self.accent, "Follow the board's tempo and Start/Stop").clicked() {
            self.flip(data::SYNC);
        }
        let playing = p.viz[viz::STEP].get() >= 0.0;
        let (label, tip) = if sync {
            ("▶ BOARD", "Synced: the board's Start/Stop runs the rhythm (turn SYNC off to use PLAY)")
        } else if self.on(data::RUNNING) {
            ("■ STOP", "Stop the rhythm")
        } else {
            ("▶ PLAY", "Play on the machine's own TEMPO")
        };
        if self.chip("play", v.r(154.0, y0, 226.0, y1), label, playing, Color32::from_rgb(110, 200, 120), tip).clicked() && !sync {
            self.flip(data::RUNNING);
        }
        let source = usize::from(self.on(data::SOURCE));
        if let Some(k) = self.seg("source", v.r(236.0, y0, 352.0, y1), &[("PRESETS", "The twenty rhythms"), ("USER", "Your eight patterns")], source) {
            p.data[data::SOURCE].set(k as f32);
        }
        if let Some(k) = self.seg("detail", v.r(470.0, y0, 690.0, y1), &DETAILS, detail(p)) {
            p.data[data::DETAIL].set(k as f32);
        }

        // The faceplate with the rhythm buttons, or the user patterns; it
        // stops short of the right edge to leave the MIDI out port visible.
        let plate = v.r(PAD, 58.0, W - 40.0, 120.0);
        v.painter.rect_filled(plate, 5.0 * z, PLATE);
        if source == 0 {
            self.rhythm_buttons(plate);
        } else {
            self.pattern_buttons(plate);
        }

        // Knobs.
        let sync_dim = sync;
        let knobs: [(usize, &str, bool); 7] = [
            (pp::TEMPO, "TEMPO", sync_dim),
            (pp::VOLUME, "VOLUME", false),
            (pp::BALANCE, "BALANCE", false),
            (pp::SWING, "SWING", false),
            (ip(voices::QUIJADA, knob::LEVEL), "QUIJADA", false),
            (ip(voices::GUIRO, knob::LEVEL), "GUIRO", false),
            (ip(voices::TAMBOURINE, knob::LEVEL), "TAMBOURINE", false),
        ];
        for (k, (i, label, dim)) in knobs.into_iter().enumerate() {
            self.knob(i, 44.0 + 62.0 * k as f32, 128.0, label, 11.0, dim);
        }
        if sync {
            v.text(44.0, 192.0, Align2::CENTER_CENTER, format!("board {:.0}", v.tempo), v.font(7.0), theme::TEXT_FAINT);
        }

        // Step lights and what is playing.
        let len = (p.viz[viz::LEN].get().round() as usize).clamp(1, MAX_LEN);
        let step = p.viz[viz::STEP].get();
        let lights = v.r(486.0, 134.0, W - PAD, 146.0);
        let w = lights.width() / len as f32;
        for s in 0..len {
            let r = Rect::from_min_size(pos2(lights.left() + w * s as f32 + 1.0, lights.top()), vec2(w - 2.0, lights.height()));
            let lit = step >= 0.0 && step as usize == s;
            v.painter.rect_filled(r, 1.5 * z, if lit { Color32::from_rgb(240, 80, 60) } else { theme::INSET });
        }
        let (main, sub) = readout(p);
        v.text(486.0, 160.0, Align2::LEFT_CENTER, main, theme::strong(9.5 * z), theme::TEXT);
        let feel = pp::current(p).map_or(String::new(), |r| {
            let bars = r.len as f32 / (r.per_beat as f32 * 4.0);
            format!("{} steps · {}", r.len, if r.per_beat == 3 { format!("triplets · {bars:.1} bars") } else { format!("16ths · {bars:.1} bars") })
        });
        v.text(486.0, 176.0, Align2::LEFT_CENTER, format!("{sub} · {feel}"), v.font(8.0), theme::TEXT_DIM);
        let hint = if source == 0 { "click: one rhythm · shift/right-click: combine" } else { "click: play a pattern · edit it in the EDITOR" };
        v.text(486.0, 192.0, Align2::LEFT_CENTER, hint, v.font(7.5), theme::TEXT_FAINT);
    }

    fn rhythm_buttons(&self, plate: Rect) {
        let (v, p, z) = (self.v, self.p, self.z());
        let bottom = self.on(data::ROW);
        // The row switch.
        for (k, label) in ["ROW 1", "ROW 2"].into_iter().enumerate() {
            let r = Rect::from_min_size(plate.min + vec2(8.0, 8.0 + 24.0 * k as f32) * z, vec2(64.0, 20.0) * z);
            let lit = usize::from(bottom) == k;
            let resp = v.ui.interact(r, self.uid(("row", k)), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Which row of rhythms plays");
            v.painter.rect_filled(r, 3.0 * z, if lit { INK } else { alpha(INK, 0.15) });
            v.painter.text(r.center(), Align2::CENTER_CENTER, label, theme::font(8.0 * z), if lit { PLATE } else { INK });
            if resp.clicked() {
                p.data[data::ROW].set(k as f32);
            }
        }
        let x0 = plate.left() + 84.0 * z;
        let cw = (plate.right() - 8.0 * z - x0) / ROW as f32;
        for i in 0..ROW {
            let cx = x0 + cw * (i as f32 + 0.5);
            for (row, ink) in [(0usize, INK), (1, RED_INK)] {
                let name = RHYTHMS[row * ROW + i].name;
                let active = usize::from(bottom) == row;
                let y = plate.top() + (9.0 + 12.0 * row as f32) * z;
                v.painter.text(pos2(cx, y), Align2::CENTER_CENTER, name, theme::strong(7.0 * z), if active { ink } else { alpha(ink, 0.35) });
            }
            let on = self.on(data::RHYTHM + i);
            let key = Rect::from_center_size(pos2(cx, plate.bottom() - 14.0 * z), vec2(cw - 6.0 * z, (if on { 20.0 } else { 24.0 }) * z));
            let resp = v
                .ui
                .interact(key, self.uid(("rhythm", i)), Sense::CLICK)
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(format!("{}: click to play it alone, shift/right-click to add it", RHYTHMS[pp::row_rhythm(p, i)].name));
            v.painter.rect_filled(key.translate(vec2(0.0, 2.0) * z), 2.0 * z, alpha(INK, 0.35));
            v.painter.rect_filled(key, 2.0 * z, if on { Color32::from_rgb(214, 214, 206) } else { KEY });
            if on {
                v.painter.rect_filled(Rect::from_center_size(key.center_top() + vec2(0.0, 4.0 * z), vec2(10.0, 3.0) * z), 1.0 * z, RED_INK);
            }
            let add = v.ui.input(|inp| inp.modifiers.shift) || resp.secondary_clicked();
            if resp.clicked() || resp.secondary_clicked() {
                if add {
                    self.flip(data::RHYTHM + i);
                } else {
                    for k in 0..ROW {
                        p.data[data::RHYTHM + k].set(if k == i { 1.0 } else { 0.0 });
                    }
                }
            }
        }
    }

    fn pattern_buttons(&self, plate: Rect) {
        let (v, p, z) = (self.v, self.p, self.z());
        let current = p.data[data::PATTERN].get().round() as usize;
        v.painter.text(plate.left_top() + vec2(10.0, 10.0) * z, Align2::LEFT_TOP, "USER\nPATTERNS", theme::strong(8.0 * z), INK);
        let x0 = plate.left() + 84.0 * z;
        let cw = (plate.right() - 8.0 * z - x0) / PATTERNS as f32;
        for k in 0..PATTERNS {
            let r = Rect::from_min_size(pos2(x0 + cw * k as f32 + 3.0 * z, plate.top() + 8.0 * z), vec2(cw - 6.0 * z, plate.height() - 16.0 * z));
            let resp = v.ui.interact(r, self.uid(("pattern", k)), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Play this pattern (and edit it in the EDITOR)");
            let lit = k == current;
            v.painter.rect_filled(r, 3.0 * z, if lit { KEY } else { alpha(KEY, 0.55) });
            v.painter.text(r.left_top() + vec2(5.0, 4.0) * z, Align2::LEFT_TOP, format!("P{}", k + 1), theme::strong(8.0 * z), if lit { RED_INK } else { INK });
            // A tiny picture of the pattern.
            let pat = pp::user(p, k);
            let area = Rect::from_min_max(r.left_top() + vec2(5.0, 18.0) * z, r.right_bottom() - vec2(5.0, 4.0) * z);
            let (dw, dh) = (area.width() / pat.len as f32, area.height() / COUNT as f32);
            for (inst, bits) in pat.rows.iter().enumerate() {
                for s in 0..pat.len {
                    if bits >> s & 1 == 1 {
                        let c = Rect::from_min_size(pos2(area.left() + dw * s as f32, area.top() + dh * inst as f32), vec2(dw.max(1.0), dh.max(1.0)));
                        v.painter.rect_filled(c, 0.0, alpha(INK, 0.75));
                    }
                }
            }
            if resp.clicked() {
                p.data[data::PATTERN].set(k as f32);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Mixer
    // -----------------------------------------------------------------------

    fn mixer(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        v.painter.hline(v.r(PAD, MIXER_Y, W - PAD, MIXER_Y).x_range(), v.p(0.0, MIXER_Y).y, Stroke::new(1.0 * z, theme::GRID_MAJOR));
        let cw = (W - 2.0 * PAD) / COUNT as f32;
        let now = v.ui.input(|i| i.time);
        let flash_id = self.uid("flash");
        let mut flashes: [(f32, f64); COUNT] = v.ui.data(|d| d.get_temp(flash_id)).unwrap_or([(0.0, 0.0); COUNT]);
        for (inst, info) in INSTRUMENTS.iter().enumerate() {
            let x = PAD + cw * inst as f32;
            let cx = x + cw * 0.5;
            let selected = (p.data[data::SELECTED].get().round() as usize) == inst;
            v.text(cx, MIXER_Y + 10.0, Align2::CENTER_CENTER, info.short, v.font(7.5), if selected { self.accent } else { theme::TEXT_DIM });
            // The pad: flashes on each hit; click to play it.
            let hits = p.viz[viz::HITS + inst].get();
            if hits != flashes[inst].0 {
                flashes[inst] = (hits, now);
            }
            let glow = (1.0 - ((now - flashes[inst].1) / 0.15) as f32).clamp(0.0, 1.0);
            let pad = v.r(x + 4.0, MIXER_Y + 18.0, x + cw - 4.0, MIXER_Y + 40.0);
            let resp = v.ui.interact(pad, self.uid(("pad", inst)), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(format!("{}: click to play (selects it in the EDITOR)", info.name));
            let base = if self.on(data::MUTE + inst) { alpha(theme::CONTROL, 0.4) } else { theme::CONTROL };
            v.painter.rect_filled(pad, 3.0 * z, theme::mix(base, self.accent, glow));
            if resp.clicked() {
                let at = viz::AUDITION + inst;
                p.viz[at].set(p.viz[at].get() + 1.0);
                p.data[data::SELECTED].set(inst as f32);
            }
            self.knob(ip(inst, knob::LEVEL), cx, MIXER_Y + 46.0, "", 9.0, false);
            self.knob(ip(inst, knob::PAN), cx, MIXER_Y + 80.0, "", 7.0, false);
            let mute = self.on(data::MUTE + inst);
            if self.chip(&format!("mute{inst}"), v.r(x + 6.0, MIXER_Y + 110.0, x + cw - 6.0, MIXER_Y + 124.0), "M", mute, theme::MUTE, "Mute in the rhythm").clicked() {
                self.flip(data::MUTE + inst);
            }
        }
        v.ui.data_mut(|d| d.insert_temp(flash_id, flashes));
    }

    // -----------------------------------------------------------------------
    // Editor
    // -----------------------------------------------------------------------

    fn editor(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        v.painter.hline(v.r(PAD, EDITOR_Y, W - PAD, EDITOR_Y).x_range(), v.p(0.0, EDITOR_Y).y, Stroke::new(1.0 * z, theme::GRID_MAJOR));
        let k = (p.data[data::PATTERN].get().round().max(0.0) as usize).min(PATTERNS - 1);
        let pat = pp::user(p, k);
        let y = EDITOR_Y + 6.0;
        v.text(PAD, y + 9.0, Align2::LEFT_CENTER, "PATTERN", theme::strong(8.5 * z), theme::TEXT_DIM);
        if let Some(n) = self.seg("edit", v.r(70.0, y, 330.0, y + 18.0), &PATTERN_ITEMS, k) {
            p.data[data::PATTERN].set(n as f32);
        }
        // Length: click − / +, or drag.
        let len_rect = v.r(340.0, y, 420.0, y + 18.0);
        let resp = v.ui.interact(len_rect, self.uid("len"), Sense::click_and_drag()).on_hover_cursor(CursorIcon::ResizeVertical).on_hover_text("Steps in the pattern: click left/right half, or drag");
        let mut len = pat.len as i32;
        if resp.clicked() {
            len += if resp.interact_pointer_pos().is_some_and(|q| q.x < len_rect.center().x) { -1 } else { 1 };
        }
        if resp.dragged() {
            let acc_id = self.uid("len-acc");
            let dy = resp.drag_delta().y;
            len += v.ui.data_mut(|d| {
                let acc = d.get_temp_mut_or(acc_id, 0.0f32);
                *acc -= dy;
                let steps = (*acc / 8.0).trunc();
                *acc -= steps * 8.0;
                steps as i32
            });
        }
        let len = len.clamp(1, MAX_LEN as i32) as usize;
        if len != pat.len {
            pp::set_shape(p, k, len, pat.per_beat == 3);
        }
        v.painter.rect_filled(len_rect, 3.0 * z, if resp.hovered() { theme::CONTROL_HI } else { theme::CONTROL });
        v.painter.text(len_rect.center(), Align2::CENTER_CENTER, format!("‹ {len} STEPS ›"), theme::font(8.0 * z), theme::TEXT);
        let feel = usize::from(pat.per_beat == 3);
        if let Some(f) = self.seg("feel", v.r(426.0, y, 546.0, y + 18.0), &[("16THS", "Sixteenth notes"), ("TRIPLETS", "Eighth-note triplets (swing, 6/8, slow rock)")], feel) {
            pp::set_shape(p, k, pat.len, f == 1);
        }
        let copy = self.chip("copy", v.r(552.0, y, 640.0, y + 18.0), "COPY FROM…", false, self.accent, "Start this pattern from a preset rhythm");
        egui::Popup::menu(&copy).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            if ui.button("What's playing").clicked() {
                pp::copy_current_to_user(p, k);
                ui.close();
            }
            ui.separator();
            for r in &RHYTHMS {
                if ui.button(r.name).clicked() {
                    pp::set_user(p, k, r);
                    ui.close();
                }
            }
        });
        if self.chip("clear", v.r(644.0, y, W - PAD, y + 18.0), "CLEAR", false, theme::MUTE, "Clear every step").clicked() {
            let mut empty = pat;
            empty.rows = [0; COUNT];
            pp::set_user(p, k, &empty);
        }

        // The grid.
        let top = y + 24.0;
        let row_h = 15.0;
        let x0 = 90.0;
        let cell = (W - PAD - x0) / MAX_LEN as f32;
        let selected = p.data[data::SELECTED].get().round() as usize;
        let step = p.viz[viz::STEP].get();
        let shown = self.on(data::SOURCE) && step >= 0.0;
        for (inst, info) in INSTRUMENTS.iter().enumerate() {
            let ry = top + row_h * inst as f32;
            let label = v.r(PAD, ry, x0 - 4.0, ry + row_h - 1.0);
            let lr = v.ui.interact(label, self.uid(("inst", inst)), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(format!("{}: show its sound below", info.name));
            if inst == selected {
                v.painter.rect_filled(label, 2.0 * z, alpha(self.accent, 0.25));
            }
            v.painter.text(label.left_center() + vec2(4.0 * z, 0.0), Align2::LEFT_CENTER, info.name, theme::font(8.0 * z), if inst == selected { theme::TEXT } else { theme::TEXT_DIM });
            if lr.clicked() {
                p.data[data::SELECTED].set(inst as f32);
            }
        }
        let grid = v.r(x0, top, x0 + cell * MAX_LEN as f32, top + row_h * COUNT as f32);
        for s in 0..MAX_LEN {
            let x = x0 + cell * s as f32;
            let used = s < pat.len;
            if s % pat.per_beat == 0 {
                v.painter.vline(v.p(x, 0.0).x, grid.y_range(), Stroke::new(1.0 * z, theme::GRID_MAJOR));
            }
            if shown && used && step as usize == s {
                v.painter.rect_filled(v.r(x, top, x + cell, top + row_h * COUNT as f32), 0.0, alpha(theme::TEXT, 0.08));
            }
            for (inst, bits) in pat.rows.iter().enumerate() {
                let ry = top + row_h * inst as f32;
                let r = v.r(x + 1.5, ry + 1.5, x + cell - 1.5, ry + row_h - 1.5);
                let hit = bits >> s & 1 == 1;
                let fill = match (used, hit) {
                    (true, true) => self.accent,
                    (true, false) => theme::CONTROL,
                    (false, true) => alpha(self.accent, 0.25),
                    (false, false) => alpha(theme::CONTROL, 0.3),
                };
                v.painter.rect_filled(r, 2.0 * z, fill);
            }
        }
        // Click a step to flip it; drag to paint.
        let resp = v.ui.interact(grid, self.uid("grid"), Sense::click_and_drag()).on_hover_cursor(CursorIcon::PointingHand);
        let paint_id = self.uid("paint");
        let cell_at = |q: egui::Pos2| {
            let s = ((q.x - grid.left()) / (cell * z)) as usize;
            let i = ((q.y - grid.top()) / (row_h * z)) as usize;
            (s < MAX_LEN && i < COUNT).then_some((i, s))
        };
        if let Some(q) = resp.interact_pointer_pos()
            && let Some((inst, s)) = cell_at(q)
        {
            let hit = pat.rows[inst] >> s & 1 == 1;
            if resp.clicked() {
                pp::set_hit(p, k, inst, s, !hit);
            }
            if resp.drag_started() {
                v.ui.data_mut(|d| d.insert_temp(paint_id, !hit));
            }
            if resp.dragged()
                && let Some(state) = v.ui.data(|d| d.get_temp::<bool>(paint_id))
            {
                pp::set_hit(p, k, inst, s, state);
            }
        }

        // The selected instrument's sound.
        self.sound(top + row_h * COUNT as f32 + 8.0, selected.min(COUNT - 1));
    }

    fn sound(&self, y: f32, inst: usize) {
        let (v, p, z) = (self.v, self.p, self.z());
        let info = &INSTRUMENTS[inst];
        v.text(PAD, y + 8.0, Align2::LEFT_CENTER, "SOUND", theme::strong(8.5 * z), theme::TEXT_DIM);
        v.text(PAD, y + 22.0, Align2::LEFT_CENTER, info.name, theme::font(9.0 * z), self.accent);
        let mut knobs: Vec<(usize, &str)> = vec![(knob::TUNE, "TUNE")];
        if info.enveloped {
            knobs.push((knob::ATTACK, "ATTACK"));
            knobs.push((knob::DECAY, "DECAY"));
        }
        for (k, label) in info.extras.iter().enumerate() {
            if !label.is_empty() {
                knobs.push((knob::X1 + k, label));
            }
        }
        knobs.push((knob::LEVEL, "LEVEL"));
        knobs.push((knob::PAN, "PAN"));
        for (col, (k, label)) in knobs.into_iter().enumerate() {
            self.knob(ip(inst, k), 120.0 + 64.0 * col as f32, y, label, 10.0, false);
        }
        if self.chip("play-sound", v.r(W - PAD - 150.0, y + 6.0, W - PAD - 80.0, y + 24.0), "▶ PLAY", false, self.accent, "Hear it").clicked() {
            let at = viz::AUDITION + inst;
            p.viz[at].set(p.viz[at].get() + 1.0);
        }
        if self.chip("reset", v.r(W - PAD - 74.0, y + 6.0, W - PAD, y + 24.0), "RESET", false, self.accent, "Back to its original sound").clicked() {
            for k in 0..knob::COUNT {
                let i = ip(inst, k);
                p.set(i, pp::spec(i).default);
            }
        }
    }
}
