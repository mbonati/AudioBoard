//! Panel of the Frakture node, in four pages: HEXAGON (the macro sliders,
//! their mappings and the global controls), EFFECTS (grains, gate, repeat,
//! space and their routing), MUTATE (the grains pushed further, around a
//! live cloud of them) and MOTION (cursor, path and timeline).

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Align2, Color32, CursorIcon, Id, Pos2, Rect, Response, Sense, Shape, Stroke, Vec2, pos2, vec2};

use super::nodes::{NodeView, segmented};
use super::theme::{self, alpha};
use super::widgets;
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::frakture::motion::{self, PathPoint, SLIDERS};
use crate::audio::modules::frakture::{self as fx, data, grain_kind, layer, lp, viz};
use crate::audio::params::gain_to_db;
use crate::graph::{ModuleNode, ParamRef};

pub const SIZE: Vec2 = vec2(640.0, 556.0);
const W: f32 = SIZE.x;
const PAD: f32 = 12.0;
const TOP: f32 = 62.0;
/// Grain layer 2's colour (layer 1 uses the node's).
const SECOND: Color32 = Color32::from_rgb(96, 165, 238);

const PAGES: [(&str, &str); 4] = [
    ("HEXAGON", "The macro sliders, their mappings and the global controls"),
    ("EFFECTS", "Grains, gate, repeat, space and their order"),
    ("MUTATE", "Push the grains further: feedback, swarms, glides, resonance, blur"),
    ("MOTION", "Cursor, path and timeline"),
];

/// MUTATE's knobs, and what each does (the legend under them).
const MUTATE: [(usize, &str); 8] = [
    (fx::FEEDBACK, "grains back into the buffer, regenerating"),
    (fx::SWARM, "a cloud of grains per trigger"),
    (fx::GLIDE, "each grain bends up (or down) as it plays"),
    (fx::SHIMMER, "grains thrown up an octave or a fifth"),
    (fx::RESONATE, "grains ringing on a harmonic of KEY"),
    (fx::KEY, "the note RESONATE rings on"),
    (fx::BLUR, "grains melted into a wash"),
    (fx::ERODE, "grains worn down to a few bits"),
];

/// Path tools.
const LINE: usize = 0;
const CIRCLE: usize = 1;
const SELECT: usize = 2;
const ADD: usize = 3;
const REMOVE: usize = 4;
const SCALE: usize = 5;
const MOUSE: usize = 6;
const TOOLS: [(&str, &str); 7] = [
    ("LINE", "Line: click to add points joined by lines"),
    ("CIRCLE", "Circle: drag from the centre to draw a circular path"),
    ("SELECT", "Select: drag points; drag a line's handle to bend it"),
    ("ADD", "Add point: click on a line (or anywhere, to extend the path)"),
    ("REMOVE", "Remove point: click a point"),
    ("SCALE", "Scale: drag a corner to resize, anywhere else to move"),
    ("MOUSE", "Mouse: click a line to make it clicked or not clicked\n(also right-click, with any tool)"),
];

struct Panel<'a, 'b> {
    v: &'a NodeView<'b>,
    id: NodeId,
    p: &'a ModuleParams,
    accent: Color32,
}

/// Draws the node's panel. `patched` says whether IN and OUT are connected.
pub fn draw(v: &NodeView, id: NodeId, accent: Color32, m: &ModuleNode, patched: (bool, bool)) {
    let s = Panel { v, id, p: &m.params, accent };
    let label_col = |on: bool| if on { theme::TEXT_DIM } else { theme::TEXT_FAINT };
    v.text(PAD, 42.0, Align2::LEFT_CENTER, "IN", v.font(9.0), label_col(patched.0));
    v.text(W - PAD, 42.0, Align2::RIGHT_CENTER, "OUT", v.font(9.0), label_col(patched.1));

    let page = fx::choice(s.p, data::PAGE, PAGES.len());
    s.seg("page", v.r(150.0, 34.0, 480.0, 52.0), &PAGES, data::PAGE);
    let bypass = fx::on(s.p, data::BYPASS);
    if s.chip("bypass", v.r(500.0, 34.0, 570.0, 52.0), "BYPASS", bypass, "Bypass: the input passes through untouched").clicked() {
        s.flip(data::BYPASS);
    }
    if fx::on(s.p, data::PATH_PLAYING) && s.p.viz[viz::PATH_DONE].get() > 0.5 {
        s.p.data[data::PATH_PLAYING].set(0.0);
    }
    match page {
        0 => s.hexagon_page(m),
        1 => s.effects_page(),
        2 => s.mutate_page(),
        _ => s.motion_page(),
    }
}

/// Collapsed nodes' readout.
pub fn readout(p: &ModuleParams) -> (String, String) {
    let grains = p.viz[viz::GRAIN_COUNT].get() + p.viz[viz::GRAIN_COUNT + 1].get();
    let chain: Vec<&str> = fx::routing(p)
        .into_iter()
        .filter(|&s| fx::on(p, data::SECTION_ON + s))
        .map(|s| fx::SECTION_NAMES[s])
        .collect();
    let main = if fx::on(p, data::BYPASS) { "Bypassed".to_owned() } else { format!("{grains:.0} grains") };
    let sub = if chain.is_empty() { "No effects".to_owned() } else { chain.join(" → ") };
    (main, sub)
}

impl Panel<'_, '_> {
    fn z(&self) -> f32 {
        self.v.z
    }

    fn uid(&self, salt: impl std::hash::Hash + std::fmt::Debug) -> Id {
        Id::unique(("frakture", self.id)).with(salt)
    }

    fn on(&self, i: usize) -> bool {
        fx::on(self.p, i)
    }

    fn flip(&self, i: usize) {
        self.p.data[i].set(if self.on(i) { 0.0 } else { 1.0 });
    }

    fn chip(&self, salt: &str, rect: Rect, label: &str, lit: bool, tip: &str) -> Response {
        widgets::toggle(self.v.ui, self.v.painter, self.uid(("chip", salt)), rect, label, lit, self.accent, self.z())
            .on_hover_text(tip)
    }

    /// A chip switching a setting on and off.
    fn switch(&self, i: usize, rect: Rect, label: &str, tip: &str) {
        if self.chip(label, rect, label, self.on(i), tip).clicked() {
            self.flip(i);
        }
    }

    /// A segmented control choosing setting `i`.
    fn seg(&self, salt: &str, rect: Rect, items: &[(&'static str, &'static str)], i: usize) {
        let v = self.v;
        let picked = segmented(v, self.uid(("seg", salt)), rect, items.len(), fx::choice(self.p, i, items.len()), self.accent, |k, r, col| {
            v.painter.text(r.center(), Align2::CENTER_CENTER, items[k].0, v.font(8.0), col);
            items[k].1
        });
        if let Some(k) = picked {
            self.p.data[i].set(k as f32);
        }
    }

    fn title(&self, x: f32, y: f32, text: &str) {
        self.v.text(x, y, Align2::LEFT_CENTER, text, theme::strong(9.5 * self.z()), theme::TEXT_DIM);
    }

    /// A section's name doubling as its on/off switch.
    fn section_switch(&self, section: usize, x: f32, y: f32, w: f32) {
        let name = fx::SECTION_NAMES[section].to_uppercase();
        let i = data::SECTION_ON + section;
        let rect = self.v.r(x, y, x + w, y + 18.0);
        if self.chip(&name, rect, &format!("⏻ {name}"), self.on(i), "Switch this effect on or off").clicked() {
            self.flip(i);
        }
    }

    fn inset(&self, rect: Rect) {
        self.v.painter.rect_filled(rect, 4.0 * self.z(), theme::INSET);
    }

    // -----------------------------------------------------------------------
    // Knobs and other parameter controls
    // -----------------------------------------------------------------------

    /// The parameter's knob position, live if the hexagon moves it.
    fn norm(&self, i: usize) -> (f32, bool) {
        let live = (i < fx::MAPPABLE).then(|| self.p.viz[viz::EFFECTIVE + i].get()).filter(|n| *n >= 0.0);
        match live {
            Some(n) if fx::mapping(self.p, i).is_some() => (n, true),
            _ => (fx::spec(i).to_norm(self.p.get(i)), false),
        }
    }

    fn value(&self, i: usize) -> f32 {
        fx::spec(i).denorm(self.norm(i).0)
    }

    /// Label, knob and value, like other nodes' knob cells. A knob mapped
    /// to the hexagon follows it and shows the slider's number; right-click
    /// maps one.
    fn knob(&self, i: usize, cx: f32, row_y: f32) {
        let (v, p, z) = (self.v, self.p, self.z());
        let spec = fx::spec(i);
        v.text(cx, row_y, Align2::CENTER_CENTER, fx::label(i), v.font(8.5), theme::TEXT_FAINT);
        let (mut norm, mapped) = self.norm(i);
        let color = if mapped { theme::mix(self.accent, theme::TEXT, 0.45) } else { self.accent };
        let resp = widgets::knob(
            v.ui,
            v.painter,
            self.uid(("knob", i)),
            v.p(cx, row_y + 19.0),
            12.0 * z,
            &mut norm,
            spec.to_norm(spec.default),
            spec.bipolar,
            color,
            z,
        );
        v.hit(resp.rect, ParamRef::Module(i));
        let mut value = spec.denorm(norm);
        if !mapped && (resp.dragged() || resp.double_clicked()) {
            p.set(i, value);
        } else {
            value = self.value(i);
        }
        let hot = resp.dragged() || resp.hovered();
        v.text(
            cx,
            row_y + 38.0,
            Align2::CENTER_CENTER,
            fx::format(p, i, value),
            v.mono(9.0),
            if hot { theme::TEXT } else { theme::TEXT_DIM },
        );
        if i < fx::MAPPABLE {
            if let Some((slot, ..)) = fx::mapping(p, i) {
                self.badge(i, slot, v.p(cx - 21.0, row_y + 19.0));
            }
            resp.context_menu(|ui| self.map_menu(ui, i));
            if mapped {
                resp.on_hover_text("Moved by the hexagon: right-click to change the mapping,\nor set its range on the HEXAGON page");
            }
        }
    }

    /// The slider number on a mapped control; clicking it shows that
    /// slider's mappings.
    fn badge(&self, i: usize, slot: usize, c: Pos2) {
        let z = self.z();
        let rect = Rect::from_center_size(c, vec2(12.0, 12.0) * z);
        let resp = self
            .v
            .ui
            .interact(rect, self.uid(("badge", i)), Sense::CLICK)
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Show this slider's mappings");
        self.v.painter.circle_filled(c, 5.5 * z, self.accent);
        self.v.painter.text(c, Align2::CENTER_CENTER, slot_label(slot), theme::font(7.5 * z), theme::INSET);
        if resp.clicked() {
            self.p.data[data::PAGE].set(0.0);
            self.p.data[data::SELECTED].set((slot - 1) as f32);
        }
    }

    fn map_menu(&self, ui: &mut egui::Ui, i: usize) {
        let p = self.p;
        ui.label(egui::RichText::new(fx::long_name(i)).size(10.5).color(theme::TEXT_DIM));
        let current = fx::mapping(p, i);
        let base = fx::spec(i).to_norm(p.get(i));
        let range = current.map(|(_, lo, hi)| (lo, hi)).unwrap_or(if base < 0.9 { (base, 1.0) } else { (0.0, base) });
        for slot in 1..=fx::AUDIO_SLOT {
            let name = if slot == fx::AUDIO_SLOT { "Input level (audio-reactive)".to_owned() } else { format!("Hexagon slider {slot}") };
            if ui.radio(current.is_some_and(|c| c.0 == slot), name).clicked() {
                fx::set_mapping(p, i, Some((slot, range.0, range.1)));
                ui.close();
            }
        }
        if current.is_some() && ui.button("Remove mapping").clicked() {
            fx::set_mapping(p, i, None);
            ui.close();
        }
    }

    /// A compact stepper for whole-number settings (BUFFER, STEPS,
    /// AMOUNT): click its left or right half, or drag up and down.
    fn spinner(&self, i: usize, rect: Rect, prefix: &str, tip: &str) {
        let (v, p, z) = (self.v, self.p, self.z());
        let spec = fx::spec(i);
        let id = self.uid(("spin", i));
        let resp = v
            .ui
            .interact(rect, id, Sense::click_and_drag())
            .on_hover_cursor(CursorIcon::ResizeVertical)
            .on_hover_text(tip);
        v.hit(rect, ParamRef::Module(i));
        let (_, mapped) = self.norm(i);
        let cur = p.get(i).round();
        let mut next = cur;
        if !mapped {
            if resp.dragged() {
                let dy = resp.drag_delta().y;
                next += v.ui.data_mut(|d| {
                    let acc = d.get_temp_mut_or(id, 0.0f32);
                    *acc -= dy;
                    let steps = (*acc / 8.0).trunc();
                    *acc -= steps * 8.0;
                    steps
                });
            }
            if resp.clicked() {
                let left = resp.interact_pointer_pos().is_some_and(|q| q.x < rect.center().x);
                next += if left { -1.0 } else { 1.0 };
            }
            next = next.clamp(spec.min, spec.max);
            if next != cur {
                p.set(i, next);
            }
        }
        let hot = resp.hovered() || resp.dragged();
        v.painter.rect_filled(rect, 3.0 * z, if hot { theme::CONTROL_HI } else { theme::CONTROL });
        let text = format!("{prefix} {}", fx::format(p, i, self.value(i)));
        v.painter.text(rect.center(), Align2::CENTER_CENTER, text, theme::font(8.5 * z), if mapped { self.accent } else { theme::TEXT });
        for (x, glyph) in [(rect.left() + 6.0 * z, "‹"), (rect.right() - 6.0 * z, "›")] {
            v.painter.text(pos2(x, rect.center().y), Align2::CENTER_CENTER, glyph, theme::font(9.0 * z), theme::TEXT_FAINT);
        }
        if let Some((slot, ..)) = fx::mapping(p, i) {
            self.badge(i, slot, rect.right_top() + vec2(-2.0, 1.0) * z);
        }
        resp.context_menu(|ui| self.map_menu(ui, i));
    }

    /// LO–HI band: two handles on a log frequency axis.
    fn band(&self, rect: Rect, lo_i: usize, hi_i: usize) {
        let (v, p, z) = (self.v, self.p, self.z());
        let text_w = 58.0 * z;
        let track = Rect::from_min_max(pos2(rect.left() + text_w, rect.top()), pos2(rect.right() - text_w, rect.bottom()));
        let x_of = |f: f32| track.left() + track.width() * (f / 20.0).ln() / 1000f32.ln();
        let f_of = |x: f32| 20.0 * 1000f32.powf(((x - track.left()) / track.width()).clamp(0.0, 1.0));
        let (lo, lo_mapped) = self.norm(lo_i);
        let (hi, hi_mapped) = self.norm(hi_i);
        let (lo_hz, hi_hz) = (fx::spec(lo_i).denorm(lo), fx::spec(hi_i).denorm(hi));
        let id = self.uid(("band", lo_i));
        let resp = v.ui.interact(track.expand2(vec2(6.0, 3.0) * z), id, Sense::click_and_drag()).on_hover_cursor(CursorIcon::ResizeHorizontal);
        if let Some(q) = resp.interact_pointer_pos() {
            if resp.drag_started() || resp.clicked() {
                let pick_hi = (q.x - x_of(hi_hz)).abs() < (q.x - x_of(lo_hz)).abs();
                v.ui.data_mut(|d| d.insert_temp(id, pick_hi));
            }
            if resp.dragged() || resp.clicked() {
                let pick_hi = v.ui.data(|d| d.get_temp::<bool>(id).unwrap_or(false));
                let f = f_of(q.x);
                if pick_hi && !hi_mapped {
                    p.set(hi_i, f.max(lo_hz));
                } else if !pick_hi && !lo_mapped {
                    p.set(lo_i, f.min(hi_hz));
                }
            }
        }
        if resp.double_clicked() {
            p.set(lo_i, 20.0);
            p.set(hi_i, 20_000.0);
        }
        let cy = rect.center().y;
        v.painter.rect_filled(Rect::from_min_max(pos2(track.left(), cy - 1.5 * z), pos2(track.right(), cy + 1.5 * z)), 1.5 * z, theme::CONTROL);
        v.painter.rect_filled(Rect::from_min_max(pos2(x_of(lo_hz), cy - 1.5 * z), pos2(x_of(hi_hz), cy + 1.5 * z)), 1.5 * z, self.accent);
        for (f, i) in [(lo_hz, lo_i), (hi_hz, hi_i)] {
            let thumb = Rect::from_center_size(pos2(x_of(f), cy), vec2(5.0, 11.0) * z);
            v.painter.rect_filled(thumb, 1.5 * z, theme::TEXT);
            v.hit(thumb, ParamRef::Module(i));
        }
        let font = theme::mono(8.0 * z);
        v.painter.text(pos2(rect.left(), cy), Align2::LEFT_CENTER, fx::format(p, lo_i, lo_hz), font.clone(), theme::TEXT_DIM);
        v.painter.text(pos2(rect.right(), cy), Align2::RIGHT_CENTER, fx::format(p, hi_i, hi_hz), font, theme::TEXT_DIM);
        resp.on_hover_text("Lo–Hi: drag the handles to cut lows and highs (double-click: open)");
    }

    // -----------------------------------------------------------------------
    // HEXAGON page
    // -----------------------------------------------------------------------

    fn hexagon_page(&self, m: &ModuleNode) {
        let v = self.v;
        let area = v.r(PAD, TOP, 402.0, 452.0);
        self.hexagon(area, false);
        self.mappings(414.0, 628.0);

        // Path and magnet.
        let playing = self.on(data::PATH_PLAYING);
        let play = self.chip("play", v.r(PAD, 466.0, 92.0, 484.0), if playing { "■ PATH" } else { "▶ PATH" }, playing, "Play the path: the cursor follows it");
        if play.clicked() {
            self.flip(data::PATH_PLAYING);
        }
        self.seg("magnet", v.r(100.0, 466.0, 300.0, 484.0), &motion::MAGNET_MODES, data::MAGNET);
        v.text(PAD, 499.0, Align2::LEFT_CENTER, "MAGNET", v.font(8.5), theme::TEXT_FAINT);
        self.seg("activation", v.r(100.0, 490.0, 300.0, 508.0), &motion::ACTIVATIONS, data::ACTIVATION);
        v.text(PAD, 526.0, Align2::LEFT_CENTER, self.path_status(), v.font(8.5), theme::TEXT_FAINT);
        v.text(
            PAD,
            542.0,
            Align2::LEFT_CENTER,
            "Move over the hexagon · right-click an effect knob to map it",
            v.font(8.5),
            theme::TEXT_FAINT,
        );

        // Global controls.
        for (k, i) in [fx::IN, fx::OUT, fx::WET].into_iter().enumerate() {
            self.knob(i, 340.0 + 66.0 * k as f32, 470.0);
        }
        widgets::hmeter(v.painter, v.r(536.0, 472.0, 628.0, 478.0), &m.meter, self.z());
        self.switch(data::LIMITER, v.r(536.0, 488.0, 580.0, 506.0), "LIMIT", "Safety limiter: the output never goes above 0 dB");
        self.switch(data::THRU, v.r(584.0, 488.0, 628.0, 506.0), "THRU", "Input thru: off, nothing new comes in and the grains play out");
    }

    /// "Path ▶ · circle · 1/1 ×4 · synced".
    fn path_status(&self) -> String {
        let p = self.p;
        let kind = if fx::choice(p, data::PATH_KIND, 2) == motion::RECORDED {
            format!("recording of {:.1} s", motion::rec_count(p) as f32 / fx::REC_HZ)
        } else if motion::drawn_closed(p) {
            format!("closed path of {} points", motion::drawn_count(p))
        } else {
            format!("path of {} points", motion::drawn_count(p))
        };
        let seconds = motion::duration(p, 60.0 / self.v.tempo.max(1.0) as f64);
        let state = if self.on(data::PATH_PLAYING) { "playing" } else { "stopped" };
        format!("Path {state} · {kind} · {seconds:.1} s per pass")
    }

    /// The mapping list of the selected slider (or the input level).
    fn mappings(&self, x0: f32, x1: f32) {
        let (v, p, z) = (self.v, self.p, self.z());
        let selected = fx::choice(p, data::SELECTED, SLIDERS + 1);
        let slot = selected + 1;
        let cw = (x1 - x0 + 2.0) / 7.0;
        for k in 0..=SLIDERS {
            let rect = v.r(x0 + cw * k as f32, TOP, x0 + cw * (k + 1) as f32 - 2.0, TOP + 18.0);
            let tip = if k == SLIDERS { "Mappings that follow the input level" } else { "This slider's mappings" };
            if self.chip(&format!("sel{k}"), rect, slot_label(k + 1), k == selected, tip).clicked() {
                p.data[data::SELECTED].set(k as f32);
            }
        }
        let mut y = TOP + 28.0;
        if slot == fx::AUDIO_SLOT {
            let cw = (x1 - x0) / 4.0;
            for (k, i) in [fx::AR_GAIN, fx::AR_ATTACK, fx::AR_RELEASE, fx::AR_THRESHOLD].into_iter().enumerate() {
                self.knob(i, x0 + cw * (k as f32 + 0.5), y);
            }
            y += 58.0;
        }
        let what = if slot == fx::AUDIO_SLOT { "INPUT LEVEL".to_owned() } else { format!("SLIDER {slot}") };
        v.text(x0, y, Align2::LEFT_CENTER, what, v.font(8.5), theme::TEXT_FAINT);
        v.text(x1 - 62.0, y, Align2::CENTER_CENTER, "MIN", v.font(8.0), theme::TEXT_FAINT);
        v.text(x1 - 26.0, y, Align2::CENTER_CENTER, "MAX", v.font(8.0), theme::TEXT_FAINT);
        y += 8.0;
        let rows: Vec<(usize, f32, f32)> = (0..fx::MAPPABLE)
            .filter_map(|i| fx::mapping(p, i).filter(|m| m.0 == slot).map(|(_, lo, hi)| (i, lo, hi)))
            .collect();
        let room = ((428.0 - y) / 20.0) as usize;
        if rows.is_empty() {
            v.text(
                (x0 + x1) * 0.5,
                y + 30.0,
                Align2::CENTER_CENTER,
                "Nothing mapped yet:\n+ ADD, or right-click an effect knob",
                v.font(8.5),
                theme::TEXT_FAINT,
            );
        }
        for (row, &(i, lo, hi)) in rows.iter().take(room).enumerate() {
            let top = y + row as f32 * 20.0;
            let line = v.r(x0, top, x1, top + 18.0);
            if row % 2 == 0 {
                v.painter.rect_filled(line, 3.0 * z, alpha(theme::INSET, 0.6));
            }
            let name = format!("{} {}", short_section(i), fx::label(i).to_lowercase());
            v.text(x0 + 4.0, top + 9.0, Align2::LEFT_CENTER, name, v.font(8.5), theme::TEXT_DIM);
            for (k, (bound, x)) in [(lo, x1 - 84.0), (hi, x1 - 48.0)].into_iter().enumerate() {
                if let Some(n) = self.norm_drag(i, k, v.r(x, top + 1.0, x + 44.0, top + 17.0), bound) {
                    let (lo, hi) = if k == 0 { (n, hi) } else { (lo, n) };
                    fx::set_mapping(p, i, Some((slot, lo, hi)));
                }
            }
            let x_rect = v.r(x1 - 2.0 - 10.0, top + 3.0, x1 - 2.0, top + 15.0);
            let x_resp = v.ui.interact(x_rect, self.uid(("unmap", i)), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Remove");
            let col = if x_resp.hovered() { theme::MUTE } else { theme::TEXT_FAINT };
            v.painter.text(x_rect.center(), Align2::CENTER_CENTER, "×", theme::font(10.0 * z), col);
            if x_resp.clicked() {
                fx::set_mapping(p, i, None);
            }
        }
        if rows.len() > room {
            v.text(x0, 424.0, Align2::LEFT_CENTER, format!("+{} more", rows.len() - room), v.font(8.5), theme::TEXT_FAINT);
        }

        let add = self.chip("add", v.r(x0, 434.0, (x0 + x1) * 0.5 - 2.0, 452.0), "+ ADD", false, "Map an effect parameter to this slider");
        egui::Popup::menu(&add)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| self.add_menu(ui, slot));
        if self.chip("clear", v.r((x0 + x1) * 0.5 + 2.0, 434.0, x1, 452.0), "CLEAR", false, "Remove all of this slider's mappings").clicked() {
            for &(i, ..) in &rows {
                fx::set_mapping(p, i, None);
            }
        }
    }

    /// A mapping's MIN or MAX: drag sideways; double-click for the end.
    fn norm_drag(&self, i: usize, k: usize, rect: Rect, n: f32) -> Option<f32> {
        let (v, z) = (self.v, self.z());
        let resp = v
            .ui
            .interact(rect, self.uid(("bound", i, k)), Sense::click_and_drag())
            .on_hover_cursor(CursorIcon::ResizeHorizontal)
            .on_hover_text("Drag to set where the slider's end puts the parameter");
        let hot = resp.hovered() || resp.dragged();
        v.painter.rect_filled(rect, 2.5 * z, if hot { theme::CONTROL_HI } else { theme::CONTROL });
        let mut out = None;
        if resp.dragged() {
            let fine = v.ui.input(|i| i.modifiers.shift);
            let d = resp.drag_delta();
            out = Some((n + (d.x - d.y) * if fine { 0.0008 } else { 0.004 }).clamp(0.0, 1.0));
        }
        if resp.double_clicked() {
            out = Some(if k == 0 { 0.0 } else { 1.0 });
        }
        let shown = fx::format(self.p, i, fx::spec(i).denorm(out.unwrap_or(n)));
        v.painter.text(rect.center(), Align2::CENTER_CENTER, shown, theme::mono(8.0 * z), theme::TEXT);
        out
    }

    fn add_menu(&self, ui: &mut egui::Ui, slot: usize) {
        let p = self.p;
        ui.set_width(200.0);
        let mut last = "";
        for i in 0..fx::MAPPABLE {
            let section = fx::section(i);
            if section != last {
                last = section;
                ui.add_space(3.0);
                ui.label(egui::RichText::new(section.to_uppercase()).size(9.5).color(theme::TEXT_FAINT));
            }
            let current = fx::mapping(p, i);
            let mut text = fx::label(i).to_owned();
            if let Some((s, ..)) = current {
                text = format!("{text}  ({})", slot_label(s));
            }
            if ui.selectable_label(current.is_some_and(|c| c.0 == slot), text).clicked() {
                let base = fx::spec(i).to_norm(p.get(i));
                let (lo, hi) = current.map(|c| (c.1, c.2)).unwrap_or(if base < 0.9 { (base, 1.0) } else { (0.0, base) });
                fx::set_mapping(p, i, Some((slot, lo, hi)));
            }
        }
    }

    // -----------------------------------------------------------------------
    // The hexagon
    // -----------------------------------------------------------------------

    /// The hexagon in `area`: sliders, nodes, the blob, the magnet and the
    /// path. On the HEXAGON page the mouse is the magnet (and in SELECT
    /// mode drags the nodes' start positions); on the MOTION page it edits
    /// or records the path.
    fn hexagon(&self, area: Rect, motion_page: bool) {
        let (v, p, z) = (self.v, self.p, self.z());
        let hex = Hex { c: area.center(), r: area.width().min(area.height()) * 0.5 * 0.86 };
        let painter = v.painter.with_clip_rect(area.intersect(v.painter.clip_rect()));
        let now = v.ui.input(|i| i.time);

        // Outline and sliders.
        let corners: Vec<Pos2> = (0..6).map(|i| hex.screen(motion::spoke(i))).collect();
        painter.add(Shape::convex_polygon(corners.clone(), alpha(theme::INSET, 0.55), Stroke::NONE));
        painter.add(Shape::closed_line(corners, Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.35))));
        self.blob(&painter, hex, now as f32);
        let selected = fx::choice(p, data::SELECTED, SLIDERS + 1);
        for i in 0..SLIDERS {
            let a = hex.screen(motion::node_pos(i, 0.0));
            let b = hex.screen(motion::node_pos(i, 1.0));
            let lit = i == selected && !motion_page;
            painter.line_segment([a, b], Stroke::new(1.2 * z, alpha(theme::TEXT_FAINT, if lit { 0.9 } else { 0.5 })));
            let (dx, dy) = motion::spoke(i);
            let tip = hex.screen((dx * 1.04, dy * 1.04));
            painter.circle_stroke(b, 3.0 * z, Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.8)));
            painter.text(tip, Align2::CENTER_CENTER, slot_label(i + 1), theme::font(9.0 * z), if lit { self.accent } else { theme::TEXT_FAINT });
        }

        // The path.
        let playing = self.on(data::PATH_PLAYING);
        if motion_page || playing {
            self.draw_path(&painter, hex, motion_page);
        }

        // Nodes: where each slider rests (ring) and where it is (dot).
        for i in 0..SLIDERS {
            let start = p.data[data::SLIDER_START + i].get();
            let live = p.viz[viz::SLIDERS + i].get();
            painter.circle_stroke(hex.screen(motion::node_pos(i, start)), 4.0 * z, Stroke::new(1.0 * z, alpha(theme::TEXT_DIM, 0.6)));
            let c = hex.screen(motion::node_pos(i, live));
            painter.circle_filled(c, 6.5 * z, theme::TEXT);
            painter.circle_stroke(c, 6.5 * z, Stroke::new(2.0 * z, self.accent));
        }

        // The input-level centre.
        let env = p.viz[viz::AUDIO_ENV].get();
        let centre_r = 0.09 * hex.r;
        painter.circle_filled(hex.c, centre_r, alpha(self.accent, 0.15 + 0.6 * env));
        let lit = selected == SLIDERS && !motion_page;
        painter.circle_stroke(hex.c, centre_r, Stroke::new(1.0 * z, if lit { self.accent } else { alpha(theme::TEXT_FAINT, 0.6) }));
        painter.text(hex.c, Align2::CENTER_CENTER, "A", theme::font(8.5 * z), if lit { theme::TEXT } else { theme::TEXT_DIM });

        // The magnet.
        let state = p.viz[viz::CURSOR_STATE].get();
        if state > 0.5 {
            let c = hex.screen((p.viz[viz::CURSOR_X].get(), p.viz[viz::CURSOR_Y].get()));
            let range = p.get(fx::CUR_RANGE) * hex.r;
            if state > 1.5 {
                painter.circle_filled(c, range, alpha(self.accent, 0.08));
                painter.circle_stroke(c, range, Stroke::new(1.0 * z, alpha(self.accent, 0.6)));
            } else {
                painter.circle_stroke(c, range, Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.4)));
            }
            painter.circle_filled(c, 3.0 * z, if state > 1.5 { self.accent } else { theme::TEXT_DIM });
        }

        let resp = v.ui.interact(area, self.uid("hex"), Sense::click_and_drag());
        let recording = motion_page && fx::choice(p, data::PATH_KIND, 2) == motion::RECORDED;
        if !motion_page || recording {
            self.mouse_magnet(&resp, hex);
        } else {
            p.viz[viz::UI_IN].set(0.0);
        }
        if motion_page {
            self.edit_path(&resp, hex);
        } else {
            self.pick_nodes(&resp, hex);
        }
        // Keep redrawing while anything on it moves.
        let values: [f32; SLIDERS] = std::array::from_fn(|i| p.viz[viz::SLIDERS + i].get());
        let seen = self.uid("seen");
        let moving = v.ui.data_mut(|d| std::mem::replace(d.get_temp_mut_or(seen, values), values)) != values;
        if playing || resp.hovered() || resp.dragged() || moving || state > 0.5 {
            v.ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        }
    }

    /// The pixel blob in the middle: it breathes with the output level and
    /// leans towards the sliders that are out.
    fn blob(&self, painter: &egui::Painter, hex: Hex, t: f32) {
        let p = self.p;
        let level = ((gain_to_db(p.viz[viz::LEVEL].get()) + 48.0) / 48.0).clamp(0.0, 1.0);
        let values: [f32; SLIDERS] = std::array::from_fn(|i| p.viz[viz::SLIDERS + i].get());
        let radius = |a: f32| {
            let mut r = 0.26 + 0.12 * level + 0.025 * (3.0 * a + t * 0.8).sin() + 0.02 * (5.0 * a - t * 1.1).sin();
            for (i, v) in values.iter().enumerate() {
                let toward = (a - (-90.0 + 60.0 * i as f32).to_radians()).cos().max(0.0);
                r += 0.17 * v * toward.powi(3);
            }
            r
        };
        let cell = 0.045;
        let half = (0.75 / cell) as i32;
        let size = cell * hex.r * 0.72;
        let dark = theme::mix(theme::INSET, self.accent, 0.35);
        let mid = theme::mix(theme::INSET, self.accent, 0.6);
        let mut mesh = Mesh::default();
        for gy in -half..=half {
            for gx in -half..=half {
                let (x, y) = (gx as f32 * cell, gy as f32 * cell);
                let d = x.hypot(y);
                let edge = radius(y.atan2(x));
                if d > edge {
                    continue;
                }
                let n = (x * 7.0 + t * 0.9).sin() + (y * 9.0 - t * 0.7).sin() + ((x + y) * 5.0 + t * 1.3).sin();
                let col = if n > 1.0 {
                    self.accent
                } else if n > -0.6 {
                    mid
                } else {
                    dark
                };
                let col = alpha(col, 0.35 + 0.65 * (1.0 - d / edge).sqrt());
                let c = hex.screen((x, y));
                let k = mesh.vertices.len() as u32;
                for (ox, oy) in [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
                    mesh.vertices.push(Vertex { pos: c + vec2(ox, oy) * size, uv: WHITE_UV, color: col });
                }
                mesh.add_triangle(k, k + 1, k + 2);
                mesh.add_triangle(k, k + 2, k + 3);
            }
        }
        painter.add(Shape::mesh(mesh));
    }

    /// Hands the mouse over the hexagon to the processor's magnet.
    fn mouse_magnet(&self, resp: &Response, hex: Hex) {
        let (v, p) = (self.v, self.p);
        let stamp = p.viz[viz::UI_STAMP].get();
        p.viz[viz::UI_STAMP].set(if stamp > 1e6 { 0.0 } else { stamp + 1.0 });
        let pointer = resp.hover_pos().or_else(|| resp.dragged().then(|| resp.interact_pointer_pos()).flatten());
        match pointer.map(|q| hex.space(q)).filter(|&(x, y)| x.hypot(y) <= 1.1) {
            Some((x, y)) => {
                p.viz[viz::UI_X].set(x);
                p.viz[viz::UI_Y].set(y);
                p.viz[viz::UI_IN].set(1.0);
                let down = v.ui.input(|i| i.pointer.primary_down()) && (resp.is_pointer_button_down_on() || resp.dragged());
                p.viz[viz::UI_DOWN].set(if down { 1.0 } else { 0.0 });
            }
            None => {
                p.viz[viz::UI_IN].set(0.0);
                p.viz[viz::UI_DOWN].set(0.0);
            }
        }
    }

    /// HEXAGON page: click a slider's end (or the centre) for its mappings;
    /// in SELECT mode, drag the nodes to set where the sliders rest.
    fn pick_nodes(&self, resp: &Response, hex: Hex) {
        let p = self.p;
        let z = self.z();
        let select = fx::choice(p, data::MAGNET, 3) == motion::SELECT;
        let id = self.uid("node-drag");
        if select && resp.drag_started()
            && let Some(q) = resp.interact_pointer_pos()
        {
            let near = (0..SLIDERS)
                .map(|i| (i, hex.screen(motion::node_pos(i, p.data[data::SLIDER_START + i].get())).distance(q)))
                .filter(|&(_, d)| d < 16.0 * z)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i);
            self.v.ui.data_mut(|d| d.insert_temp(id, near));
        }
        if select && resp.dragged()
            && let (Some(Some(i)), Some(q)) = (self.v.ui.data(|d| d.get_temp::<Option<usize>>(id)), resp.interact_pointer_pos())
        {
            p.data[data::SLIDER_START + i].set(motion::project(i, hex.space(q)));
            p.data[data::SELECTED].set(i as f32);
        }
        if resp.clicked()
            && let Some(q) = resp.interact_pointer_pos()
        {
            if q.distance(hex.c) < 0.1 * hex.r {
                p.data[data::SELECTED].set(SLIDERS as f32);
            }
            for i in 0..SLIDERS {
                let (dx, dy) = motion::spoke(i);
                let end = hex.screen((dx * 1.0, dy * 1.0));
                let node = hex.screen(motion::node_pos(i, p.viz[viz::SLIDERS + i].get()));
                if end.distance(q) < 14.0 * z || (select && node.distance(q) < 10.0 * z) {
                    p.data[data::SELECTED].set(i as f32);
                }
            }
        }
    }

    /// The drawn path (with its points and handles while editing) or the
    /// recording.
    fn draw_path(&self, painter: &egui::Painter, hex: Hex, editing: bool) {
        let (p, z) = (self.p, self.z());
        let strong = Stroke::new(1.6 * z, alpha(self.accent, if editing { 0.9 } else { 0.5 }));
        let weak = Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, if editing { 0.8 } else { 0.4 }));
        if fx::choice(p, data::PATH_KIND, 2) == motion::RECORDED {
            let n = motion::rec_count(p);
            let (a, b) = motion::rec_span(p);
            for k in 1..n {
                let (q0, q1) = (motion::rec_point(p, k - 1), motion::rec_point(p, k));
                let inside = (k - 1) as f32 >= a.floor() && k as f32 <= b.ceil();
                let stroke = if !inside {
                    Stroke::new(0.8 * z, alpha(theme::TEXT_FAINT, 0.25))
                } else if q0.clicked {
                    strong
                } else {
                    weak
                };
                painter.line_segment([hex.screen((q0.x, q0.y)), hex.screen((q1.x, q1.y))], stroke);
            }
            return;
        }
        let n = motion::drawn_count(p);
        let closed = motion::drawn_closed(p);
        let segments = if closed { n } else { n.saturating_sub(1) };
        let tool = fx::choice(p, data::TOOL, TOOLS.len());
        for s in 0..segments {
            let (a, b) = (motion::drawn_point(p, s), motion::drawn_point(p, (s + 1) % n));
            let pts: Vec<Pos2> = (0..=16).map(|k| hex.screen(motion::segment(a, b, k as f32 / 16.0))).collect();
            if a.clicked {
                painter.add(Shape::line(pts, strong));
            } else {
                // Dashed: not clicked along this line.
                for pair in pts.chunks(2) {
                    if let [q0, q1] = pair {
                        painter.line_segment([*q0, *q1], weak);
                    }
                }
            }
            if editing && tool == SELECT {
                let h = hex.screen(motion::segment(a, b, 0.5));
                let d = 3.5 * z;
                painter.add(Shape::convex_polygon(
                    vec![h + vec2(0.0, -d), h + vec2(d, 0.0), h + vec2(0.0, d), h + vec2(-d, 0.0)],
                    alpha(theme::TEXT_DIM, 0.8),
                    Stroke::NONE,
                ));
            }
        }
        if editing {
            for i in 0..n {
                let pt = motion::drawn_point(p, i);
                let c = hex.screen((pt.x, pt.y));
                let col = if i == 0 { self.accent } else { theme::TEXT };
                painter.rect_filled(Rect::from_center_size(c, vec2(5.0, 5.0) * z), 1.0 * z, col);
            }
            if tool == SCALE && n > 0 {
                let (lo, hi) = path_bounds(p);
                let r = Rect::from_min_max(hex.screen(lo), hex.screen(hi)).expand(6.0 * z);
                painter.rect_stroke(r, 0.0, Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.6)), egui::StrokeKind::Middle);
                for c in [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()] {
                    painter.rect_filled(Rect::from_center_size(c, vec2(6.0, 6.0) * z), 1.0 * z, theme::TEXT_DIM);
                }
            }
        }
    }

    /// MOTION page: the path tools, and recording.
    fn edit_path(&self, resp: &Response, hex: Hex) {
        let (v, p, z) = (self.v, self.p, self.z());
        if fx::choice(p, data::PATH_KIND, 2) == motion::RECORDED {
            self.record(resp, hex);
            return;
        }
        let tool = fx::choice(p, data::TOOL, TOOLS.len());
        let n = motion::drawn_count(p);
        let closed = motion::drawn_closed(p);
        let points: Vec<PathPoint> = (0..n).map(|i| motion::drawn_point(p, i)).collect();
        let segments = if closed { n } else { n.saturating_sub(1) };
        let near_point = |q: Pos2| {
            (0..n)
                .map(|i| (i, hex.screen((points[i].x, points[i].y)).distance(q)))
                .filter(|&(_, d)| d < 9.0 * z)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
        };
        // The line under the pointer, and where on it.
        let near_segment = |q: Pos2| {
            let mut best: Option<(usize, f32, f32)> = None;
            for s in 0..segments {
                for k in 0..=24 {
                    let t = k as f32 / 24.0;
                    let d = hex.screen(motion::segment(points[s], points[(s + 1) % n], t)).distance(q);
                    if d < 8.0 * z && best.is_none_or(|b| d < b.2) {
                        best = Some((s, t, d));
                    }
                }
            }
            best
        };
        let drag_id = self.uid("path-drag");

        // Right-click on a line: clicked / not clicked, with any tool.
        if resp.secondary_clicked()
            && let Some(q) = resp.interact_pointer_pos()
            && let Some((s, ..)) = near_segment(q)
        {
            let mut pt = points[s];
            pt.clicked = !pt.clicked;
            motion::set_drawn_point(p, s, pt);
        }
        let Some(q) = resp.interact_pointer_pos() else {
            return;
        };
        let at = hex.space(q);
        match tool {
            LINE if resp.clicked() => {
                let new = PathPoint { x: at.0, y: at.1, bend: 0.0, clicked: true };
                if closed || n >= fx::MAX_POINTS {
                    motion::set_drawn(p, &[new], false);
                } else {
                    let mut all = points.clone();
                    all.push(new);
                    motion::set_drawn(p, &all, false);
                }
            }
            CIRCLE => {
                if resp.drag_started() {
                    v.ui.data_mut(|d| d.insert_temp(drag_id, at));
                }
                if let Some(centre) = v.ui.data(|d| d.get_temp::<(f32, f32)>(drag_id)) {
                    let r = (at.0 - centre.0).hypot(at.1 - centre.1);
                    if resp.dragged() {
                        v.painter.circle_stroke(hex.screen(centre), r * hex.r, Stroke::new(1.0 * z, alpha(self.accent, 0.7)));
                    }
                    if resp.drag_stopped() {
                        v.ui.data_mut(|d| d.remove::<(f32, f32)>(drag_id));
                        let r = if r < 0.05 { fx::CIRCLE_R } else { r };
                        let ring: Vec<PathPoint> = (0..12)
                            .map(|k| {
                                let a = (k as f32 * 30.0).to_radians();
                                PathPoint { x: centre.0 + r * a.cos(), y: centre.1 + r * a.sin(), bend: 0.0, clicked: true }
                            })
                            .collect();
                        motion::set_drawn(p, &ring, true);
                    }
                } else if resp.clicked() {
                    let ring: Vec<PathPoint> = (0..12)
                        .map(|k| {
                            let a = (k as f32 * 30.0).to_radians();
                            PathPoint { x: at.0 + fx::CIRCLE_R * a.cos(), y: at.1 + fx::CIRCLE_R * a.sin(), bend: 0.0, clicked: true }
                        })
                        .collect();
                    motion::set_drawn(p, &ring, true);
                }
            }
            SELECT => {
                if resp.drag_started() {
                    // A point, else a line's bend handle.
                    let grab = near_point(q).map(|i| (i, false)).or_else(|| {
                        (0..segments)
                            .find(|&s| hex.screen(motion::segment(points[s], points[(s + 1) % n], 0.5)).distance(q) < 8.0 * z)
                            .map(|s| (s, true))
                    });
                    v.ui.data_mut(|d| d.insert_temp(drag_id, grab));
                }
                if resp.dragged()
                    && let Some(Some((i, bend))) = v.ui.data(|d| d.get_temp::<Option<(usize, bool)>>(drag_id))
                    && i < n
                {
                    let mut pt = points[i];
                    if bend {
                        let b = points[(i + 1) % n];
                        let (dx, dy) = (b.x - pt.x, b.y - pt.y);
                        let len2 = (dx * dx + dy * dy).max(1e-6);
                        let (mx, my) = ((pt.x + b.x) * 0.5, (pt.y + b.y) * 0.5);
                        // The curve's middle sits half-way to its control point.
                        pt.bend = (((at.0 - mx) * -dy + (at.1 - my) * dx) / len2 * 2.0).clamp(-1.0, 1.0);
                    } else {
                        pt.x = at.0;
                        pt.y = at.1;
                    }
                    motion::set_drawn_point(p, i, pt);
                }
            }
            ADD if resp.clicked() && n < fx::MAX_POINTS => {
                let new = PathPoint { x: at.0, y: at.1, bend: 0.0, clicked: true };
                let mut all = points.clone();
                match near_segment(q) {
                    Some((s, ..)) => {
                        let mut new = new;
                        new.clicked = points[s].clicked;
                        all.insert(s + 1, new);
                    }
                    None => all.push(new),
                }
                motion::set_drawn(p, &all, closed);
            }
            REMOVE if resp.clicked() => {
                if let Some(i) = near_point(q) {
                    let mut all = points.clone();
                    all.remove(i);
                    let keep_closed = closed && all.len() >= 3;
                    motion::set_drawn(p, &all, keep_closed);
                }
            }
            SCALE if n > 0 => {
                let (lo, hi) = path_bounds(p);
                let centre = ((lo.0 + hi.0) * 0.5, (lo.1 + hi.1) * 0.5);
                if resp.drag_started() {
                    let r = Rect::from_min_max(hex.screen(lo), hex.screen(hi)).expand(6.0 * z);
                    let corner = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].iter().any(|c| c.distance(q) < 9.0 * z);
                    v.ui.data_mut(|d| d.insert_temp(drag_id, (corner, at, points.clone())));
                }
                if resp.dragged()
                    && let Some((corner, from, original)) = v.ui.data(|d| d.get_temp::<(bool, (f32, f32), Vec<PathPoint>)>(drag_id))
                {
                    let moved: Vec<PathPoint> = if corner {
                        let d0 = (from.0 - centre.0).hypot(from.1 - centre.1).max(1e-3);
                        let k = (at.0 - centre.0).hypot(at.1 - centre.1) / d0;
                        let oc = bounds_centre(&original);
                        original
                            .iter()
                            .map(|pt| PathPoint { x: oc.0 + (pt.x - oc.0) * k, y: oc.1 + (pt.y - oc.1) * k, ..*pt })
                            .collect()
                    } else {
                        let (dx, dy) = (at.0 - from.0, at.1 - from.1);
                        original.iter().map(|pt| PathPoint { x: pt.x + dx, y: pt.y + dy, ..*pt }).collect()
                    };
                    for (i, pt) in moved.into_iter().enumerate() {
                        motion::set_drawn_point(p, i, pt);
                    }
                }
            }
            MOUSE if resp.clicked() => {
                if let Some((s, ..)) = near_segment(q) {
                    let mut pt = points[s];
                    pt.clicked = !pt.clicked;
                    motion::set_drawn_point(p, s, pt);
                }
            }
            _ => {}
        }
    }

    /// RECORD: while on, the mouse over the hexagon is sampled 25 times a
    /// second (with its clicks), up to 30 s. R over the hexagon toggles it.
    fn record(&self, resp: &Response, hex: Hex) {
        let (v, p) = (self.v, self.p);
        let id = self.uid("rec");
        let now = v.ui.input(|i| i.time);
        if resp.hovered() && v.ui.input(|i| i.key_pressed(egui::Key::R)) {
            self.toggle_recording(now);
        }
        let Some(started) = v.ui.data(|d| d.get_temp::<f64>(id)) else {
            return;
        };
        let due = (((now - started) * fx::REC_HZ as f64) as usize + 1).min(fx::MAX_REC);
        let mut count = motion::rec_count(p);
        let pointer = resp.hover_pos().or_else(|| resp.interact_pointer_pos()).map(|q| hex.space(q));
        let clicked = v.ui.input(|i| i.pointer.primary_down()) && resp.is_pointer_button_down_on();
        while count < due {
            let last = if count > 0 { motion::rec_point(p, count - 1) } else { PathPoint::default() };
            let (x, y) = pointer.unwrap_or((last.x, last.y));
            motion::set_rec_point(p, count, PathPoint { x, y, bend: 0.0, clicked });
            count += 1;
        }
        p.data[data::REC_COUNT].set(count as f32);
        if count >= fx::MAX_REC {
            v.ui.data_mut(|d| d.remove::<f64>(id));
        }
        v.ui.ctx().request_repaint();
    }

    fn recording(&self) -> bool {
        self.v.ui.data(|d| d.get_temp::<f64>(self.uid("rec")).is_some())
    }

    fn toggle_recording(&self, now: f64) {
        let (v, p) = (self.v, self.p);
        let id = self.uid("rec");
        if self.recording() {
            v.ui.data_mut(|d| d.remove::<f64>(id));
        } else {
            p.data[data::PATH_PLAYING].set(0.0);
            p.data[data::REC_COUNT].set(0.0);
            p.data[data::REC_START].set(0.0);
            p.data[data::REC_END].set(1.0);
            p.data[data::PATH_KIND].set(motion::RECORDED as f32);
            v.ui.data_mut(|d| d.insert_temp(id, now));
        }
    }

    // -----------------------------------------------------------------------
    // EFFECTS page
    // -----------------------------------------------------------------------

    fn effects_page(&self) {
        self.grains_column(PAD, 352.0);
        self.rhythm_column(364.0, W - PAD);
        self.routing(518.0);
    }

    fn grains_column(&self, x0: f32, x1: f32) {
        let (v, p) = (self.v, self.p);
        let displays = self.on(data::DISPLAYS);
        self.section_switch(fx::GRAINS, x0, TOP, 70.0);
        self.spinner(fx::BUFFER, v.r(x0 + 76.0, TOP, x0 + 170.0, TOP + 18.0), "BUFFER", "How much of the input the grains come from");
        self.switch(data::RETRIG, v.r(x0 + 176.0, TOP, x0 + 232.0, TOP + 18.0), "RETRIG", "Retrigger: the buffer and the playheads start over when the transport starts");
        self.switch(data::FREEZE, v.r(x0 + 238.0, TOP, x0 + 294.0, TOP + 18.0), "FREEZE", "Freeze: stop recording and keep the captured audio");
        self.switch(data::DISPLAYS, v.r(x0 + 300.0, TOP, x1, TOP + 18.0), "VIEW", "Show the effects' displays");
        self.band(v.r(x0, TOP + 24.0, x1, TOP + 38.0), fx::LO, fx::HI);

        let mut y = TOP + 44.0;
        if displays {
            self.grains_display(v.r(x0, y, x1, y + 96.0));
            y += 100.0;
            let mid = (x0 + x1) * 0.5;
            self.shape_display(v.r(x0, y, mid - 4.0, y + 30.0), 0);
            self.shape_display(v.r(mid + 4.0, y, x1, y + 30.0), 1);
            y += 36.0;
        }

        // Layer: which, on/off, playhead and pitch modes.
        let shown = fx::choice(p, data::LAYER_SHOWN, 2);
        let colors = [self.accent, SECOND];
        let bar = v.r(x0, y, x0 + 56.0, y + 18.0);
        let picked = segmented(v, self.uid("layer"), bar, 2, shown, colors[shown], |k, r, col| {
            v.painter.text(r.center(), Align2::CENTER_CENTER, ["1", "2"][k], v.font(9.0), col);
            "Grain layer shown below"
        });
        if let Some(k) = picked {
            p.data[data::LAYER_SHOWN].set(k as f32);
        }
        self.switch(data::LAYER_ON + shown, v.r(x0 + 62.0, y, x0 + 100.0, y + 18.0), "ON", "Switch this grain layer on or off");
        self.seg(&format!("playhead{shown}"), v.r(x0 + 106.0, y, x1, y + 18.0), &fx::PLAYHEAD_MODES, data::PLAYHEAD_MODE + shown);
        y += 24.0;
        v.text(x0, y + 9.0, Align2::LEFT_CENTER, "PITCH", v.font(8.5), theme::TEXT_FAINT);
        self.seg(&format!("pitch{shown}"), v.r(x0 + 106.0, y, x1, y + 18.0), &fx::PITCH_MODES, data::PITCH_MODE + shown);
        y += 28.0;

        let cols = 4;
        let cw = (x1 - x0) / cols as f32;
        for k in 0..layer::COUNT {
            let i = lp(shown, k);
            let cx = x0 + cw * ((k % cols) as f32 + 0.5);
            let row_y = y + 56.0 * (k / cols) as f32;
            self.knob(i, cx, row_y);
            let scanning = fx::choice(p, data::PLAYHEAD_MODE + shown, fx::PLAYHEAD_MODES.len()) == fx::SCAN;
            let sync = match k {
                layer::DENSITY => Some((data::DENSITY_SYNC + shown, "Sync to the tempo (note values)")),
                layer::SIZE => Some((data::SIZE_SYNC + shown, "Sync to the tempo (note values)")),
                layer::PLAYHEAD if scanning => Some((data::SCAN_SYNC + shown, "Scan in time: whole passes of the buffer, locked to the transport")),
                _ => None,
            };
            if let Some((d, tip)) = sync {
                self.switch(d, v.r(cx + 15.0, row_y + 12.0, cx + 41.0, row_y + 26.0), "SYNC", tip);
            }
        }
    }

    /// Both layers' grains over the buffer: position across, pitch up.
    fn grains_display(&self, rect: Rect) {
        let (p, z) = (self.p, self.z());
        self.inset(rect);
        let painter = self.v.painter.with_clip_rect(rect.intersect(self.v.painter.clip_rect()));
        let r = rect.shrink2(vec2(4.0, 6.0) * z);
        let half = r.height() * 0.5;
        let bar = (r.width() / fx::OVERVIEW as f32 * 0.8).max(1.0);
        for k in 0..fx::OVERVIEW {
            let h = p.viz[viz::OVERVIEW_AT + k].get().min(1.0).sqrt() * half * 0.8;
            if h < 0.5 {
                continue;
            }
            let x = r.left() + r.width() * (k as f32 + 0.5) / fx::OVERVIEW as f32;
            painter.line_segment([pos2(x, r.center().y - h), pos2(x, r.center().y + h)], Stroke::new(bar, alpha(theme::TEXT_FAINT, 0.5)));
        }
        painter.hline(r.x_range(), r.center().y, Stroke::new(1.0 * z, theme::GRID_MAJOR));
        let x_of = |f: f32| r.left() + r.width() * f.clamp(0.0, 1.0);
        painter.vline(x_of(p.viz[viz::WRITE_HEAD].get()), r.y_range(), Stroke::new(1.0 * z, alpha(theme::TEXT, 0.5)));
        let colors = [self.accent, SECOND];
        for (k, &color) in colors.iter().enumerate() {
            if !self.on(data::LAYER_ON + k) {
                continue;
            }
            painter.vline(x_of(p.viz[viz::PLAYHEADS + k].get()), r.y_range(), Stroke::new(1.4 * z, alpha(color, 0.9)));
            for g in 0..fx::VIZ_GRAINS {
                let at = viz::GRAINS + (k * fx::VIZ_GRAINS + g) * fx::GRAIN_VIZ;
                let amp = p.viz[at + 2].get();
                if amp < 0.0 {
                    continue;
                }
                let y = r.center().y - (p.viz[at + 1].get() / 36.0).clamp(-1.0, 1.0) * half;
                painter.circle_filled(pos2(x_of(p.viz[at].get()), y), 2.6 * z, alpha(color, 0.25 + 0.75 * amp));
            }
        }
        let font = theme::font(7.5 * z);
        painter.text(rect.right_top() + vec2(-4.0, 3.0) * z, Align2::RIGHT_TOP, "+36", font.clone(), theme::TEXT_FAINT);
        painter.text(rect.right_bottom() + vec2(-4.0, -3.0) * z, Align2::RIGHT_BOTTOM, "−36", font.clone(), theme::TEXT_FAINT);
        let seconds = p.viz[viz::BUF_SECONDS].get();
        painter.text(rect.left_top() + vec2(4.0, 3.0) * z, Align2::LEFT_TOP, format!("Grains 1 & 2 · {seconds:.1} s"), font, theme::TEXT_FAINT);
    }

    /// A layer's grain envelope (SHAPE).
    fn shape_display(&self, rect: Rect, k: usize) {
        let z = self.z();
        self.inset(rect);
        let r = rect.shrink2(vec2(6.0, 5.0) * z);
        let (a, rel) = fx::shape_ramps(self.value(lp(k, layer::SHAPE)));
        let (a, rel) = (a.max(0.02), rel.max(0.02));
        let pts = vec![
            r.left_bottom(),
            pos2(r.left() + r.width() * a, r.top()),
            pos2(r.right() - r.width() * rel, r.top()),
            r.right_bottom(),
        ];
        let col = [self.accent, SECOND][k];
        let on = self.on(data::LAYER_ON + k);
        self.v.painter.add(Shape::convex_polygon(pts.clone(), alpha(col, if on { 0.15 } else { 0.05 }), Stroke::NONE));
        self.v.painter.add(Shape::line(pts, Stroke::new(1.3 * z, alpha(col, if on { 1.0 } else { 0.35 }))));
        self.v.painter.text(rect.left_top() + vec2(4.0, 2.0) * z, Align2::LEFT_TOP, format!("Grain {}", k + 1), theme::font(7.5 * z), theme::TEXT_FAINT);
    }

    fn rhythm_column(&self, x0: f32, x1: f32) {
        let (v, p, z) = (self.v, self.p, self.z());
        let displays = self.on(data::DISPLAYS);

        // Gate.
        self.section_switch(fx::GATE, x0, TOP, 64.0);
        self.spinner(fx::GATE_STEPS, v.r(x0 + 70.0, TOP, x0 + 150.0, TOP + 18.0), "STEPS", "Steps in the gate pattern");
        let steps = (self.value(fx::GATE_STEPS).round() as usize).clamp(1, fx::GATE_MAX);
        let current = (p.viz[viz::GATE_POS].get() * steps as f32) as usize;
        let cw = (x1 - x0) / 16.0;
        for s in 0..fx::GATE_MAX {
            let (row, col) = (s / 16, s % 16);
            let top = TOP + 24.0 + row as f32 * 16.0;
            let rect = v.r(x0 + cw * col as f32 + 1.0, top, x0 + cw * (col + 1) as f32 - 1.0, top + 14.0);
            let used = s < steps;
            let lit = self.on(data::GATE_PATTERN + s);
            let resp = v.ui.interact(rect, self.uid(("step", s)), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand);
            let col_fill = match (used, lit) {
                (false, _) => alpha(theme::CONTROL, 0.35),
                (true, true) => self.accent,
                (true, false) => theme::CONTROL,
            };
            v.painter.rect_filled(rect, 2.0 * z, col_fill);
            if used && s == current && self.on(data::SECTION_ON + fx::GATE) {
                v.painter.rect_stroke(rect, 2.0 * z, Stroke::new(1.5 * z, theme::TEXT), egui::StrokeKind::Inside);
            }
            if resp.clicked() {
                self.flip(data::GATE_PATTERN + s);
            }
        }
        let mut y = TOP + 58.0;
        if displays {
            self.gate_display(v.r(x0, y, x1, y + 24.0));
            y += 30.0;
        }
        let cw4 = (x1 - x0) / 4.0;
        for (k, i) in [fx::GATE_RATE, fx::GATE_ATTACK, fx::GATE_RELEASE, fx::GATE_MIX].into_iter().enumerate() {
            self.knob(i, x0 + cw4 * (k as f32 + 0.5), y);
        }
        y += 60.0;

        // Repeat.
        self.section_switch(fx::REPEAT, x0, y, 76.0);
        self.spinner(fx::REP_AMOUNT, v.r(x0 + 82.0, y, x0 + 170.0, y + 18.0), "AMOUNT", "How many times the slice repeats");
        self.switch(data::REPEAT_SYNC, v.r(x0 + 176.0, y, x0 + 220.0, y + 18.0), "SYNC", "RATE in note values (else milliseconds)");
        y += 24.0;
        self.band(v.r(x0, y, x1, y + 14.0), fx::REP_LO, fx::REP_HI);
        y += 20.0;
        if displays {
            self.repeat_display(v.r(x0, y, x1, y + 24.0));
            y += 30.0;
        }
        let cw3 = (x1 - x0) / 3.0;
        for (k, i) in [fx::REP_TRIGGER, fx::REP_OFFSET, fx::REP_DUCK, fx::REP_RATE, fx::REP_VOLUME].into_iter().enumerate() {
            self.knob(i, x0 + cw3 * ((k % 3) as f32 + 0.5), y + 56.0 * (k / 3) as f32);
        }
        y += 118.0;

        // Space.
        self.section_switch(fx::SPACE, x0, y, 70.0);
        self.switch(data::SPACE_SYNC, v.r(x0 + 76.0, y, x0 + 120.0, y + 18.0), "SYNC", "LENGTH in note values at the tempo");
        y += 26.0;
        self.knob(fx::SPACE_LENGTH, x0 + cw3 * 0.5, y);
        self.knob(fx::SPACE_MIX, x0 + cw3 * 1.5, y);
    }

    /// The level before (faint) and after the gate along the pattern.
    fn gate_display(&self, rect: Rect) {
        let (p, z) = (self.p, self.z());
        self.inset(rect);
        let r = rect.shrink2(vec2(3.0, 3.0) * z);
        let w = r.width() / fx::GATE_BINS as f32;
        for k in 0..fx::GATE_BINS {
            let x = r.left() + w * (k as f32 + 0.5);
            for (at, col) in [(viz::GATE_IN, alpha(theme::TEXT_FAINT, 0.5)), (viz::GATE_OUT, self.accent)] {
                let h = p.viz[at + k].get().min(1.0).sqrt() * r.height();
                if h > 0.5 {
                    self.v.painter.line_segment([pos2(x, r.bottom()), pos2(x, r.bottom() - h)], Stroke::new((w * 0.7).max(1.0), col));
                }
            }
        }
        let x = r.left() + r.width() * p.viz[viz::GATE_POS].get();
        self.v.painter.vline(x, r.y_range(), Stroke::new(1.0 * z, alpha(theme::TEXT, 0.6)));
    }

    /// The slice being repeated, and how far along.
    fn repeat_display(&self, rect: Rect) {
        let (p, z) = (self.p, self.z());
        self.inset(rect);
        let r = rect.shrink2(vec2(3.0, 3.0) * z);
        let state = p.viz[viz::REP_STATE].get();
        let w = r.width() / fx::SLICE_BINS as f32;
        let col = if state > 1.5 { self.accent } else { alpha(theme::TEXT_FAINT, 0.6) };
        for k in 0..fx::SLICE_BINS {
            let h = p.viz[viz::REP_SLICE + k].get().min(1.0).sqrt() * r.height() * 0.5;
            let x = r.left() + w * (k as f32 + 0.5);
            if h > 0.5 {
                self.v.painter.line_segment([pos2(x, r.center().y - h), pos2(x, r.center().y + h)], Stroke::new((w * 0.7).max(1.0), col));
            }
        }
        if state > 0.5 {
            let x = r.left() + r.width() * p.viz[viz::REP_PROGRESS].get();
            self.v.painter.vline(x, r.y_range(), Stroke::new(1.0 * z, alpha(theme::TEXT, 0.7)));
        }
        let text = match state {
            s if s > 1.5 => format!("repeat {:.0}/{:.0}", p.viz[viz::REP_COUNT].get(), self.value(fx::REP_AMOUNT).round()),
            s if s > 0.5 => "taking a slice".to_owned(),
            _ => String::new(),
        };
        self.v.painter.text(rect.right_top() + vec2(-4.0, 2.0) * z, Align2::RIGHT_TOP, text, theme::font(7.5 * z), theme::TEXT_DIM);
    }

    /// The effects' order: drag a name along the row.
    fn routing(&self, y: f32) {
        let (v, p, z) = (self.v, self.p, self.z());
        v.text(PAD, y + 9.0, Align2::LEFT_CENTER, "ROUTING", theme::strong(9.5 * z), theme::TEXT_DIM);
        let order = fx::routing(p);
        let (x0, cw) = (90.0, 132.0);
        let id = self.uid("route");
        for (k, &section) in order.iter().enumerate() {
            let x = x0 + cw * k as f32;
            let rect = v.r(x, y, x + cw - 26.0, y + 18.0);
            let resp = v
                .ui
                .interact(rect, id.with(section), Sense::click_and_drag())
                .on_hover_cursor(CursorIcon::Grab)
                .on_hover_text("Drag to change the order of the effects");
            let lit = self.on(data::SECTION_ON + section);
            let fill = if resp.dragged() { theme::CONTROL_HI } else { theme::CONTROL };
            v.painter.rect_filled(rect, 3.0 * z, fill);
            v.painter.text(rect.center(), Align2::CENTER_CENTER, fx::SECTION_NAMES[section], theme::font(9.0 * z), if lit { theme::TEXT } else { theme::TEXT_FAINT });
            v.text(x + cw - 13.0, y + 9.0, Align2::CENTER_CENTER, "→", v.font(10.0), theme::TEXT_FAINT);
            if resp.dragged()
                && let Some(q) = resp.interact_pointer_pos()
            {
                let slot = (((q.x - v.p(x0, 0.0).x) / (cw * z)).floor().max(0.0) as usize).min(3);
                if slot != k {
                    let mut new: Vec<usize> = order.to_vec();
                    new.remove(k);
                    new.insert(slot, section);
                    for (j, s) in new.into_iter().enumerate() {
                        p.data[data::ROUTING + j].set(s as f32);
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // MUTATE page
    // -----------------------------------------------------------------------

    fn mutate_page(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        self.title(PAD, TOP + 9.0, "MUTATE");
        v.text(PAD + 66.0, TOP + 9.0, Align2::LEFT_CENTER, "push the grains further", v.font(8.5), theme::TEXT_FAINT);
        for (k, label) in ["LAYER 1", "LAYER 2"].into_iter().enumerate() {
            let x = 330.0 + 66.0 * k as f32;
            self.switch(data::MUTATE_LAYERS + k, v.r(x, TOP, x + 60.0, TOP + 18.0), label, "MUTATE changes this grain layer");
        }
        let dice = self.chip("dice", v.r(W - PAD - 70.0, TOP, W - PAD, TOP + 18.0), "DICE", false, "Roll new mutations");
        if dice.clicked() {
            self.roll_mutations();
        }

        self.cloud(v.r(PAD, TOP + 26.0, W - PAD, TOP + 282.0));

        let cw = (W - 2.0 * PAD) / MUTATE.len() as f32;
        for (k, (i, _)) in MUTATE.iter().enumerate() {
            let cx = PAD + cw * (k as f32 + 0.5);
            self.knob(*i, cx, TOP + 290.0);
            // SWARM, GLIDE and FEEDBACK can follow the tempo: SYNC, and the
            // note value under it.
            let sync = match *i {
                fx::SWARM => Some((data::SWARM_SYNC, fx::SWARM_RATE, "A cloud's grains one after another, a note value apart: rolls in time")),
                fx::GLIDE => Some((data::GLIDE_SYNC, fx::GLIDE_TIME, "The bend takes a note value, whatever the grain's length")),
                fx::FEEDBACK => Some((data::FEEDBACK_SYNC, fx::FEEDBACK_TIME, "The grains come back a note value later: echoes in time")),
                _ => None,
            };
            if let Some((d, rate, tip)) = sync {
                let y = TOP + 336.0;
                self.switch(d, v.r(cx - 30.0, y, cx + 30.0, y + 16.0), "SYNC", tip);
                if self.on(d) {
                    self.spinner(rate, v.r(cx - 34.0, y + 20.0, cx + 34.0, y + 36.0), "", tip);
                }
            }
        }
        // The legend: what each knob does.
        let y0 = TOP + 384.0;
        for (k, (i, what)) in MUTATE.iter().enumerate() {
            let (col, row) = (k % 2, k / 2);
            let x = PAD + 6.0 + (W - 2.0 * PAD) * 0.5 * col as f32;
            let y = y0 + 16.0 * row as f32;
            v.text(x, y, Align2::LEFT_CENTER, fx::label(*i), theme::strong(8.0 * z), self.accent);
            v.text(x + 66.0, y, Align2::LEFT_CENTER, *what, v.font(8.0), theme::TEXT_DIM);
        }
        let marks = "●  a grain     ◯  resonating     ■  eroded     +  shimmer     │  glide     ⋯  feedback echoes";
        v.text(W * 0.5, y0 + 70.0, Align2::CENTER_CENTER, marks, v.font(7.5), theme::TEXT_FAINT);
        if p.viz[viz::GRAIN_COUNT].get() + p.viz[viz::GRAIN_COUNT + 1].get() > 0.0 {
            v.ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        }
    }

    /// DICE: new mutations, each within a range that stays musical.
    fn roll_mutations(&self) {
        let p = self.p;
        let seed = (self.v.ui.input(|i| i.time) * 1000.0) as u32 ^ self.id.wrapping_mul(0x9e37_79b9);
        let mut rng = crate::audio::modules::Rng::new(seed);
        let mut set = |i: usize, lo: f32, hi: f32| p.set(i, lo + (hi - lo) * rng.unit());
        set(fx::FEEDBACK, 0.0, 0.7);
        set(fx::SWARM, 0.0, 1.0);
        set(fx::GLIDE, -0.6, 0.6);
        set(fx::SHIMMER, 0.0, 0.6);
        set(fx::RESONATE, 0.0, 0.8);
        set(fx::BLUR, 0.0, 0.8);
        set(fx::ERODE, 0.0, 0.4);
        // KEY on a semitone between A1 and A4.
        let semis = (rng.unit() * 36.0).round();
        p.set(fx::KEY, 55.0 * (semis / 12.0).exp2());
    }

    /// The grains as a living cloud: across, where they are in the buffer;
    /// up, their pitch now. A tail shows how far GLIDE has bent one, a halo
    /// how BLUR smears them, a ring a resonating grain, a square an eroded
    /// one, a spark a shimmering one; FEEDBACK echoes the buffer outward.
    fn cloud(&self, rect: Rect) {
        let (p, z) = (self.p, self.z());
        self.inset(rect);
        let painter = self.v.painter.with_clip_rect(rect.intersect(self.v.painter.clip_rect()));
        let r = rect.shrink2(vec2(12.0, 12.0) * z);
        let (mid, half) = (r.center().y, r.height() * 0.5);
        let y_of = |semis: f32| mid - (semis / 36.0).clamp(-1.0, 1.0) * half;
        let x_of = |f: f32| r.left() + r.width() * f.clamp(0.0, 1.0);
        let font = theme::font(7.0 * z);
        for semis in [-36.0f32, -24.0, -12.0, 0.0, 12.0, 24.0, 36.0] {
            let y = y_of(semis);
            let a = if semis == 0.0 { 0.18 } else { 0.07 };
            painter.hline(r.x_range(), y, Stroke::new(1.0 * z, alpha(theme::TEXT, a)));
            if semis != 0.0 {
                painter.text(pos2(r.left() + 2.0 * z, y - 1.0 * z), Align2::LEFT_BOTTOM, format!("{semis:+.0}"), font.clone(), theme::TEXT_FAINT);
            }
        }

        // The buffer as thin bars, and FEEDBACK's echoes of it as ghostly
        // outlines, each fainter and further out.
        let feedback = self.value(fx::FEEDBACK);
        let level = |k: usize| p.viz[viz::OVERVIEW_AT + k].get().min(1.0).sqrt();
        let x_at = |k: usize| r.left() + r.width() * (k as f32 + 0.5) / fx::OVERVIEW as f32;
        for k in 0..fx::OVERVIEW {
            let h = level(k) * half * 0.3;
            if h >= 0.5 {
                painter.line_segment([pos2(x_at(k), mid - h), pos2(x_at(k), mid + h)], Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.3)));
            }
        }
        for echo in 1..4 {
            let a = feedback * 0.45 / echo as f32;
            if a < 0.02 {
                continue;
            }
            let reach = half * (0.3 + 0.2 * echo as f32);
            for side in [-1.0f32, 1.0] {
                let outline: Vec<Pos2> = (0..fx::OVERVIEW).map(|k| pos2(x_at(k), mid + side * level(k) * reach)).collect();
                painter.add(Shape::line(outline, Stroke::new(1.0 * z, alpha(self.accent, a))));
            }
        }
        painter.vline(x_of(p.viz[viz::WRITE_HEAD].get()), r.y_range(), Stroke::new(1.0 * z, alpha(theme::TEXT, 0.35)));

        let blur = self.value(fx::BLUR);
        let resonate = self.value(fx::RESONATE);
        let colors = [self.accent, SECOND];
        let mut count = 0;
        for (k, &color) in colors.iter().enumerate() {
            if !self.on(data::LAYER_ON + k) {
                continue;
            }
            for g in 0..fx::VIZ_GRAINS {
                let at = viz::GRAINS + (k * fx::VIZ_GRAINS + g) * fx::GRAIN_VIZ;
                let amp = p.viz[at + 2].get();
                if amp < 0.0 {
                    continue;
                }
                count += 1;
                let kind = p.viz[at + 3].get() as u32;
                let (x, y, y0) = (x_of(p.viz[at].get()), y_of(p.viz[at + 1].get()), y_of(p.viz[at + 4].get()));
                let c = pos2(x, y);
                let rad = (2.2 + 4.5 * amp) * z;
                if blur > 0.02 {
                    painter.circle_filled(c, rad * (2.0 + 6.0 * blur), alpha(color, 0.07 * amp * blur));
                }
                if (y - y0).abs() > 1.5 * z {
                    painter.line_segment([pos2(x, y0), c], Stroke::new(1.2 * z, alpha(color, 0.25 + 0.35 * amp)));
                    painter.circle_filled(pos2(x, y0), 1.2 * z, alpha(color, 0.4));
                }
                let fill = alpha(color, 0.35 + 0.65 * amp);
                if kind & grain_kind::ERODED != 0 {
                    painter.rect_filled(Rect::from_center_size(c, Vec2::splat(rad * 1.7)), 0.0, fill);
                } else {
                    painter.circle_filled(c, rad, fill);
                }
                if kind & grain_kind::RESONANT != 0 {
                    painter.circle_stroke(c, rad + (2.5 + 5.0 * resonate) * z, Stroke::new(1.0 * z, alpha(color, 0.15 + 0.55 * amp * resonate)));
                }
                if kind & grain_kind::SHIMMER != 0 {
                    let s = c - vec2(0.0, rad + 6.0 * z);
                    let d = 3.5 * z;
                    let spark = Stroke::new(1.0 * z, alpha(Color32::WHITE, 0.3 + 0.7 * amp));
                    painter.line_segment([s - vec2(d, 0.0), s + vec2(d, 0.0)], spark);
                    painter.line_segment([s - vec2(0.0, d), s + vec2(0.0, d)], spark);
                }
            }
        }
        let seconds = p.viz[viz::BUF_SECONDS].get();
        let key = self.value(fx::KEY);
        painter.text(
            rect.right_top() + vec2(-8.0, 6.0) * z,
            Align2::RIGHT_TOP,
            format!("{count} grains · {seconds:.1} s of buffer · KEY {}", fx::format(p, fx::KEY, key)),
            theme::font(8.0 * z),
            theme::TEXT_DIM,
        );
    }

    // -----------------------------------------------------------------------
    // MOTION page
    // -----------------------------------------------------------------------

    fn motion_page(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let area = v.r(PAD, TOP, 352.0, 402.0);
        self.hexagon(area, true);

        // Path: draw or record.
        let kinds = [("DRAW", "Draw a path with points and lines"), ("RECORD", "Record the mouse's movements over the hexagon")];
        self.seg("kind", v.r(PAD, 410.0, 150.0, 428.0), &kinds, data::PATH_KIND);
        if self.chip("delete", v.r(156.0, 410.0, 252.0, 428.0), "DELETE PATH", false, "Delete the path").clicked() {
            if fx::choice(p, data::PATH_KIND, 2) == motion::RECORDED {
                p.data[data::REC_COUNT].set(0.0);
            } else {
                motion::set_drawn(p, &[], false);
            }
            p.data[data::PATH_PLAYING].set(0.0);
        }
        let playing = self.on(data::PATH_PLAYING);
        if self.chip("play2", v.r(258.0, 410.0, 352.0, 428.0), if playing { "■ STOP" } else { "▶ PLAY" }, playing, "Play the path").clicked() {
            self.flip(data::PATH_PLAYING);
        }
        if fx::choice(p, data::PATH_KIND, 2) == motion::DRAWN {
            self.seg("tool", v.r(PAD, 434.0, 352.0, 452.0), &TOOLS, data::TOOL);
            let tool = fx::choice(p, data::TOOL, TOOLS.len());
            v.text(PAD, 466.0, Align2::LEFT_CENTER, TOOLS[tool].1.lines().next().unwrap_or(""), v.font(8.5), theme::TEXT_FAINT);
        } else {
            let recording = self.recording();
            let now = v.ui.input(|i| i.time);
            if self.chip("rec", v.r(PAD, 434.0, 72.0, 452.0), "● REC", recording, "Record the mouse over the hexagon (or press R over it)").clicked() {
                self.toggle_recording(now);
            }
            self.trim(v.r(80.0, 434.0, 300.0, 452.0));
            if self.chip("trim", v.r(306.0, 434.0, 352.0, 452.0), "✂ TRIM", false, "Keep only the part between START and END").clicked() {
                self.cut_recording();
            }
            let what = if recording {
                format!("Recording… {:.1} s", motion::rec_count(p) as f32 / fx::REC_HZ)
            } else {
                format!("{:.1} s recorded", motion::rec_count(p) as f32 / fx::REC_HZ)
            };
            v.text(PAD, 466.0, Align2::LEFT_CENTER, what, v.font(8.5), theme::TEXT_FAINT);
        }

        // Cursor.
        let x0 = 364.0;
        let x1 = W - PAD;
        self.title(x0, TOP + 7.0, "CURSOR");
        self.seg("magnet2", v.r(x0, TOP + 18.0, x1, TOP + 36.0), &motion::MAGNET_MODES, data::MAGNET);
        self.seg("activation2", v.r(x0, TOP + 40.0, x1, TOP + 58.0), &motion::ACTIVATIONS, data::ACTIVATION);
        let cw4 = (x1 - x0) / 4.0;
        for (k, i) in [fx::CUR_RANGE, fx::CUR_STRENGTH, fx::CUR_HOLD, fx::CUR_RELEASE].into_iter().enumerate() {
            self.knob(i, x0 + cw4 * (k as f32 + 0.5), TOP + 66.0);
        }

        // Timeline.
        let ty = TOP + 136.0;
        self.title(x0, ty + 9.0, "TIMELINE");
        let directions = [("→", "Forwards"), ("←", "Backwards"), ("⇄", "Pendulum: forwards, then backwards")];
        self.seg("direction", v.r(x0 + 120.0, ty, x0 + 210.0, ty + 18.0), &directions, data::DIRECTION);
        self.switch(data::LOOP, v.r(x0 + 216.0, ty, x1, ty + 18.0), "LOOP", "Loop the path (else it plays once)");
        let play_rect = v.r(x0, ty + 26.0, x0 + 40.0, ty + 66.0);
        let play = v.ui.interact(play_rect, self.uid("tl-play"), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Play / stop the path");
        let c = play_rect.center();
        v.painter.circle_stroke(c, 17.0 * z, Stroke::new(1.2 * z, if play.hovered() { theme::TEXT_DIM } else { theme::TEXT_FAINT }));
        if playing {
            v.painter.rect_filled(Rect::from_center_size(c, vec2(10.0, 10.0) * z), 1.5 * z, self.accent);
        } else {
            v.painter.add(Shape::convex_polygon(
                vec![c + vec2(-4.0, -7.0) * z, c + vec2(8.0, 0.0) * z, c + vec2(-4.0, 7.0) * z],
                self.accent,
                Stroke::NONE,
            ));
        }
        if play.clicked() {
            self.flip(data::PATH_PLAYING);
        }
        self.speed_graph(v.r(x0 + 48.0, ty + 26.0, x1, ty + 66.0));
        let row = ty + 74.0;
        let half = (x0 + x1) * 0.5;
        self.seg("duration", v.r(x0, row, half - 3.0, row + 18.0), &[("FIXED", "Fixed: TIME × MULTIPLY long"), ("FREE", "Free: as long as the recording")], data::DURATION);
        self.seg("speed", v.r(half + 3.0, row, x1, row + 18.0), &[("TIMELINE", "The cursor moves evenly"), ("RECORDED", "The cursor moves as it was recorded")], data::SPEED);
        let row = row + 24.0;
        let retrig = [
            ("OFF", "Retrigger off: the path runs on its own"),
            ("ON", "Retrigger: the path starts over when the transport starts"),
            ("SYNC", "Sync: the path is locked to the transport"),
        ];
        self.seg("retrigger", v.r(x0, row, half - 3.0, row + 18.0), &retrig, data::RETRIGGER);
        self.switch(data::TIME_SYNC, v.r(half + 3.0, row, x1, row + 18.0), "TIME SYNC", "TIME in note values (else seconds)");
        let cw3 = (x1 - x0) / 3.0;
        for (k, i) in [fx::TL_TIME, fx::TL_MULT, fx::TL_OFFSET].into_iter().enumerate() {
            self.knob(i, x0 + cw3 * (k as f32 + 0.5), row + 28.0);
        }
        v.text(x0, row + 84.0, Align2::LEFT_CENTER, self.path_status(), v.font(8.5), theme::TEXT_FAINT);
    }

    /// The cursor's speed over one pass: even, or as recorded.
    fn speed_graph(&self, rect: Rect) {
        let (p, z) = (self.p, self.z());
        self.inset(rect);
        let r = rect.shrink2(vec2(6.0, 6.0) * z);
        let recorded = fx::choice(p, data::PATH_KIND, 2) == motion::RECORDED
            && fx::choice(p, data::SPEED, 2) == motion::AS_RECORDED
            && motion::rec_count(p) > 2;
        let mut pts = Vec::new();
        if recorded {
            let (a, b) = motion::rec_span(p);
            let (a, b) = (a as usize, (b as usize).max(a as usize + 1));
            let bins = 48;
            let speeds: Vec<f32> = (0..bins)
                .map(|k| {
                    let i = a + (b - a) * k / bins;
                    let j = (i + 1).min(motion::rec_count(p) - 1);
                    let (q0, q1) = (motion::rec_point(p, i), motion::rec_point(p, j));
                    (q1.x - q0.x).hypot(q1.y - q0.y)
                })
                .collect();
            let top = speeds.iter().copied().fold(1e-6f32, f32::max);
            for (k, s) in speeds.iter().enumerate() {
                pts.push(pos2(r.left() + r.width() * k as f32 / (bins - 1) as f32, r.bottom() - r.height() * s / top));
            }
        } else {
            pts.push(pos2(r.left(), r.center().y));
            pts.push(pos2(r.right(), r.center().y));
        }
        self.v.painter.add(Shape::line(pts, Stroke::new(1.3 * z, alpha(theme::TEXT_DIM, 0.9))));
        let u = p.viz[viz::PATH_POS].get();
        if u >= 0.0 {
            let x = r.left() + r.width() * u;
            self.v.painter.vline(x, rect.y_range(), Stroke::new(1.2 * z, self.accent));
        }
    }

    /// START and END of the played part of the recording.
    fn trim(&self, rect: Rect) {
        let (v, p, z) = (self.v, self.p, self.z());
        let id = self.uid("trim-range");
        let resp = v.ui.interact(rect, id, Sense::click_and_drag()).on_hover_cursor(CursorIcon::ResizeHorizontal).on_hover_text("Start and end of the recording that plays");
        let (a, b) = (p.data[data::REC_START].get(), p.data[data::REC_END].get());
        let x_of = |f: f32| rect.left() + rect.width() * f;
        if let Some(q) = resp.interact_pointer_pos() {
            let f = ((q.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            if resp.drag_started() || resp.clicked() {
                v.ui.data_mut(|d| d.insert_temp(id, (q.x - x_of(b)).abs() < (q.x - x_of(a)).abs()));
            }
            if resp.dragged() || resp.clicked() {
                if v.ui.data(|d| d.get_temp::<bool>(id).unwrap_or(false)) {
                    p.data[data::REC_END].set(f.max(a + 0.01).min(1.0));
                } else {
                    p.data[data::REC_START].set(f.min(b - 0.01).max(0.0));
                }
            }
        }
        let cy = rect.center().y;
        v.painter.rect_filled(Rect::from_min_max(pos2(rect.left(), cy - 1.5 * z), pos2(rect.right(), cy + 1.5 * z)), 1.5 * z, theme::CONTROL);
        v.painter.rect_filled(Rect::from_min_max(pos2(x_of(a), cy - 1.5 * z), pos2(x_of(b), cy + 1.5 * z)), 1.5 * z, self.accent);
        for f in [a, b] {
            v.painter.rect_filled(Rect::from_center_size(pos2(x_of(f), cy), vec2(5.0, 12.0) * z), 1.5 * z, theme::TEXT);
        }
    }

    /// Keeps only START…END of the recording.
    fn cut_recording(&self) {
        let p = self.p;
        let (a, b) = motion::rec_span(p);
        let (a, b) = (a.round() as usize, b.round() as usize);
        if b <= a {
            return;
        }
        for (k, i) in (a..=b).enumerate() {
            let pt = motion::rec_point(p, i);
            motion::set_rec_point(p, k, pt);
        }
        p.data[data::REC_COUNT].set((b - a + 1) as f32);
        p.data[data::REC_START].set(0.0);
        p.data[data::REC_END].set(1.0);
    }
}

/// Hexagon geometry on screen.
#[derive(Clone, Copy)]
struct Hex {
    c: Pos2,
    r: f32,
}

impl Hex {
    fn screen(&self, (x, y): (f32, f32)) -> Pos2 {
        self.c + vec2(x, y) * self.r
    }

    fn space(&self, q: Pos2) -> (f32, f32) {
        let d = (q - self.c) / self.r;
        (d.x, d.y)
    }
}

/// "1"…"6", "A" for the input level.
fn slot_label(slot: usize) -> &'static str {
    ["", "1", "2", "3", "4", "5", "6", "A"].get(slot).copied().unwrap_or("")
}

/// "G1", "Gate", … for the mapping list.
fn short_section(i: usize) -> &'static str {
    match fx::section(i) {
        "Grains 1" => "G1",
        "Grains 2" => "G2",
        "Grains" => "Grains",
        "Global" => "",
        s => s,
    }
}

/// The drawn path's bounding box.
fn path_bounds(p: &ModuleParams) -> ((f32, f32), (f32, f32)) {
    let n = motion::drawn_count(p);
    let mut lo = (f32::MAX, f32::MAX);
    let mut hi = (f32::MIN, f32::MIN);
    for i in 0..n {
        let pt = motion::drawn_point(p, i);
        lo = (lo.0.min(pt.x), lo.1.min(pt.y));
        hi = (hi.0.max(pt.x), hi.1.max(pt.y));
    }
    if n == 0 { ((0.0, 0.0), (0.0, 0.0)) } else { (lo, hi) }
}

/// Centre of a set of points' bounding box.
fn bounds_centre(points: &[PathPoint]) -> (f32, f32) {
    let lo = points.iter().fold((f32::MAX, f32::MAX), |a, q| (a.0.min(q.x), a.1.min(q.y)));
    let hi = points.iter().fold((f32::MIN, f32::MIN), |a, q| (a.0.max(q.x), a.1.max(q.y)));
    ((lo.0 + hi.0) * 0.5, (lo.1 + hi.1) * 0.5)
}
