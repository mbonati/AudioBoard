//! Panel of the Magma node, laid out like Volcano 3: the I/O bar, the
//! filter display (curves, dots, analyzer, piano), the filter controls with
//! the routing, and the modulation section — sources in tabs, their
//! editors, and the slots. Drag a source's handle onto any knob to
//! modulate it.

use std::cell::{Cell, RefCell};

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Align2, Color32, CursorIcon, Id, PointerButton, Pos2, Rect, Response, Sense, Shape, Stroke, Vec2, pos2, vec2};

use super::nodes::{NodeView, note_name, segmented};
use super::theme::{self, alpha};
use super::widgets::{self, MeterState};
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::magma::modulation::{self as md, SOURCES};
use crate::audio::modules::magma::{
    self as mg, FILTERS, PARAM_COUNT, ROUTING_MODES, ROUTINGS, SHAPES, SLOPES, SLOTS, STYLES, data, ef, eg, efp, egp, filt,
    fp, viz, xlfo, xlfo_data, xp,
};
use crate::graph::{ModuleNode, ParamRef};

pub const SIZE: Vec2 = vec2(820.0, 600.0);
const W: f32 = SIZE.x;
const PAD: f32 = 12.0;
const DISPLAY: [f32; 4] = [52.0, 58.0, 776.0, 262.0];
const CONTROLS_Y: f32 = 270.0;
const MOD_Y: f32 = 352.0;
const EDIT_Y: f32 = 380.0;
/// The display's scales.
const F_LO: f32 = 10.0;
const F_HI: f32 = 20_000.0;
const DB_SPAN: f32 = 30.0;
/// Volcano's yellow for the total curve.
const CURVE: Color32 = Color32::from_rgb(238, 196, 70);
/// Filter colours.
const FILTER_COLORS: [Color32; FILTERS] = [
    Color32::from_rgb(236, 132, 74),
    Color32::from_rgb(98, 196, 168),
    Color32::from_rgb(110, 156, 236),
    Color32::from_rgb(214, 108, 176),
];

/// The source tabs: name, colour, the sources they hold (X and Y for XY).
const TABS: [(&str, Color32, &[usize]); 10] = [
    ("XLFO 1", XLFO_COL, &[1]),
    ("XLFO 2", XLFO_COL, &[2]),
    ("EG 1", EG_COL, &[3]),
    ("EG 2", EG_COL, &[4]),
    ("EF 1", EF_COL, &[5]),
    ("EF 2", EF_COL, &[6]),
    ("MIDI 1", MIDI_COL, &[7]),
    ("MIDI 2", MIDI_COL, &[8]),
    ("XY 1", XY_COL, &[9, 10]),
    ("XY 2", XY_COL, &[11, 12]),
];
const XLFO_COL: Color32 = Color32::from_rgb(236, 168, 64);
const EG_COL: Color32 = Color32::from_rgb(124, 206, 104);
const EF_COL: Color32 = Color32::from_rgb(84, 196, 220);
const MIDI_COL: Color32 = Color32::from_rgb(170, 132, 236);
const XY_COL: Color32 = Color32::from_rgb(232, 112, 160);

/// A source's colour.
fn source_color(source: usize) -> Color32 {
    TABS.iter().find(|t| t.2.contains(&source)).map_or(theme::TEXT_DIM, |t| t.1)
}

struct Panel<'a, 'b> {
    v: &'a NodeView<'b>,
    id: NodeId,
    p: &'a ModuleParams,
    accent: Color32,
    /// Controls drawn this frame that a source can be dropped on.
    targets: RefCell<Vec<(Rect, usize)>>,
    /// A source handle let go this frame, and where.
    dropped: Cell<Option<(usize, Pos2)>>,
    /// A source being dragged (highlights the targets).
    dragging: Cell<Option<usize>>,
    /// Parameters some slot modulates.
    modulated: [bool; PARAM_COUNT],
}

pub fn draw(v: &NodeView, id: NodeId, accent: Color32, m: &ModuleNode, patched: [bool; 3], midi: bool) {
    let dragging = v.ui.data(|d| d.get_temp::<usize>(Id::unique(("magma-drag", id))));
    let mut modulated = [false; PARAM_COUNT];
    for s in 0..SLOTS {
        if let Some((_, target)) = mg::slot(&m.params, s) {
            modulated[target] = true;
        }
    }
    let s = Panel {
        v,
        id,
        p: &m.params,
        accent,
        targets: RefCell::new(Vec::new()),
        dropped: Cell::new(None),
        dragging: Cell::new(dragging),
        modulated,
    };
    let col = |on: bool| if on { theme::TEXT_DIM } else { theme::TEXT_FAINT };
    v.text(PAD, 42.0, Align2::LEFT_CENTER, "IN", v.font(9.0), col(patched[0]));
    v.text(PAD, 60.0, Align2::LEFT_CENTER, "SC", v.font(9.0), col(patched[1]));
    v.text(PAD, 78.0, Align2::LEFT_CENTER, "MIDI", v.font(9.0), col(midi));
    v.text(W - PAD, 42.0, Align2::RIGHT_CENTER, "OUT", v.font(9.0), col(patched[2]));
    s.top_bar();
    s.display();
    s.output_meter();
    s.filter_controls();
    s.modulation();
    s.finish_drop();
}

/// Collapsed nodes' readout.
pub fn readout(p: &ModuleParams) -> (String, String) {
    let filters: Vec<String> = (0..FILTERS)
        .filter(|&f| mg::on(p, data::FILTER_ON + f))
        .map(|f| format!("{} {}", SHAPES[mg::choice(p, data::SHAPE + f, SHAPES.len())].0, fmt_hz(p.get(fp(f, filt::FREQ)))))
        .collect();
    let main = if filters.is_empty() { "No filters".to_owned() } else { filters.join(" · ") };
    let slots = (0..SLOTS).filter(|&s| mg::slot(p, s).is_some()).count();
    let sub = format!("{} · {slots} mod", ROUTINGS[mg::choice(p, data::ROUTING, ROUTINGS.len())].0);
    (main, sub)
}

fn fmt_hz(f: f32) -> String {
    if f >= 1000.0 { format!("{:.1}k", f / 1000.0) } else { format!("{f:.0}") }
}

impl Panel<'_, '_> {
    fn z(&self) -> f32 {
        self.v.z
    }

    fn uid(&self, salt: impl std::hash::Hash + std::fmt::Debug) -> Id {
        Id::unique(("magma", self.id)).with(salt)
    }

    fn on(&self, i: usize) -> bool {
        mg::on(self.p, i)
    }

    fn flip(&self, i: usize) {
        self.p.data[i].set(if self.on(i) { 0.0 } else { 1.0 });
    }

    fn chip(&self, salt: &str, rect: Rect, label: &str, lit: bool, color: Color32, tip: &str) -> Response {
        widgets::toggle(self.v.ui, self.v.painter, self.uid(("chip", salt)), rect, label, lit, color, self.z()).on_hover_text(tip)
    }

    fn switch(&self, i: usize, rect: Rect, label: &str, tip: &str) {
        if self.chip(label, rect, label, self.on(i), self.accent, tip).clicked() {
            self.flip(i);
        }
    }

    fn seg(&self, salt: &str, rect: Rect, items: &[(&'static str, &'static str)], i: usize, color: Color32) {
        let v = self.v;
        let picked = segmented(v, self.uid(("seg", salt)), rect, items.len(), mg::choice(self.p, i, items.len()), color, |k, r, col| {
            v.painter.text(r.center(), Align2::CENTER_CENTER, items[k].0, v.font(8.0), col);
            items[k].1
        });
        if let Some(k) = picked {
            self.p.data[i].set(k as f32);
        }
    }

    /// A setting stepping through `labels`: click the right half for the
    /// next, the left half for the previous.
    fn cycler(&self, salt: &str, rect: Rect, i: usize, labels: &[&str], tip: &str) {
        let (v, z) = (self.v, self.z());
        let resp = v
            .ui
            .interact(rect, self.uid(("cycle", salt)), Sense::CLICK)
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(tip);
        let cur = mg::choice(self.p, i, labels.len());
        if resp.clicked() {
            let back = resp.interact_pointer_pos().is_some_and(|q| q.x < rect.center().x);
            let next = if back { (cur + labels.len() - 1) % labels.len() } else { (cur + 1) % labels.len() };
            self.p.data[i].set(next as f32);
        }
        v.painter.rect_filled(rect, 3.0 * z, if resp.hovered() { theme::CONTROL_HI } else { theme::CONTROL });
        v.painter.text(rect.center(), Align2::CENTER_CENTER, labels[cur], theme::font(8.0 * z), theme::TEXT);
        for (x, glyph) in [(rect.left() + 5.0 * z, "‹"), (rect.right() - 5.0 * z, "›")] {
            v.painter.text(pos2(x, rect.center().y), Align2::CENTER_CENTER, glyph, theme::font(8.5 * z), theme::TEXT_FAINT);
        }
    }

    fn inset(&self, rect: Rect) {
        self.v.painter.rect_filled(rect, 4.0 * self.z(), theme::INSET);
    }

    // -----------------------------------------------------------------------
    // Knobs: modulation targets
    // -----------------------------------------------------------------------

    /// A parameter's live knob position, if a slot modulates it (the
    /// processor's value alone may be stale, e.g. while it rests).
    fn live(&self, i: usize) -> Option<f32> {
        if !self.modulated[i] {
            return None;
        }
        let n = self.p.viz[viz::EFFECTIVE + i].get();
        (n >= 0.0).then_some(n)
    }

    /// Slots aimed at parameter `i`.
    fn slots_on(&self, i: usize) -> Vec<(usize, usize)> {
        (0..SLOTS).filter_map(|s| mg::slot(self.p, s).filter(|&(_, t)| t == i).map(|(src, _)| (s, src))).collect()
    }

    /// Records a control a source can be dropped on, and outlines it while
    /// a source is dragged.
    fn target(&self, rect: Rect, i: usize) {
        self.targets.borrow_mut().push((rect, i));
        if let Some(src) = self.dragging.get() {
            let hot = self.v.ui.input(|inp| inp.pointer.hover_pos()).is_some_and(|q| rect.contains(q));
            let col = source_color(src);
            self.v.painter.rect_stroke(
                rect.expand(2.0 * self.z()),
                4.0 * self.z(),
                Stroke::new(if hot { 2.0 } else { 1.0 } * self.z(), alpha(col, if hot { 1.0 } else { 0.5 })),
                egui::StrokeKind::Outside,
            );
        }
    }

    /// Label, knob and value. A modulated knob shows a ring from its
    /// position to where the modulation has it now, and a dot in the
    /// sources' colours; right-click it to modulate it or remove that.
    fn knob(&self, i: usize, cx: f32, row_y: f32, color: Color32) {
        let (v, p, z) = (self.v, self.p, self.z());
        let spec = mg::spec(i);
        v.text(cx, row_y, Align2::CENTER_CENTER, mg::label(i), v.font(8.0), theme::TEXT_FAINT);
        let mut norm = spec.to_norm(p.get(i));
        let c = v.p(cx, row_y + 18.0);
        let resp = widgets::knob(v.ui, v.painter, self.uid(("knob", i)), c, 11.0 * z, &mut norm, spec.to_norm(spec.default), spec.bipolar, color, z);
        v.hit(resp.rect, ParamRef::Module(i));
        self.target(resp.rect, i);
        if resp.dragged() || resp.double_clicked() {
            p.set(i, spec.denorm(norm));
        }
        let slots = self.slots_on(i);
        if let Some(live) = self.live(i) {
            let a0 = widgets::KNOB_START + widgets::KNOB_SWEEP * norm;
            let a1 = widgets::KNOB_START + widgets::KNOB_SWEEP * live;
            let col = slots.first().map_or(CURVE, |&(_, src)| source_color(src));
            if (a1 - a0).abs() > 0.01 {
                v.painter.add(Shape::line(widgets::arc(c, 14.5 * z, a0.min(a1), a0.max(a1)), Stroke::new(2.0 * z, col)));
            }
        }
        for (k, &(_, src)) in slots.iter().take(3).enumerate() {
            v.painter.circle_filled(c + vec2(14.0 + 5.0 * k as f32, -12.0) * z, 2.2 * z, source_color(src));
        }
        let shown = self.live(i).map_or(p.get(i), |n| spec.denorm(n));
        let hot = resp.dragged() || resp.hovered();
        v.text(cx, row_y + 36.0, Align2::CENTER_CENTER, mg::format(p, i, shown), v.mono(8.5), if hot { theme::TEXT } else { theme::TEXT_DIM });
        resp.context_menu(|ui| self.target_menu(ui, i));
    }

    fn target_menu(&self, ui: &mut egui::Ui, i: usize) {
        let p = self.p;
        ui.label(egui::RichText::new(mg::long_name(i)).size(10.5).color(theme::TEXT_DIM));
        ui.menu_button("Modulate with", |ui| {
            for (src, name) in md::SOURCE_NAMES.iter().enumerate().skip(1) {
                if ui.button(*name).clicked() {
                    mg::add_slot(p, src, i);
                    ui.close();
                }
            }
        });
        let slots = self.slots_on(i);
        if !slots.is_empty() && ui.button("Remove its modulation").clicked() {
            for (s, _) in slots {
                mg::set_slot(p, s, 0, 0);
            }
            ui.close();
        }
    }

    /// A compact value: drag it; double-click resets.
    fn number_box(&self, i: usize, rect: Rect, label: &str) {
        let (v, p, z) = (self.v, self.p, self.z());
        let spec = mg::spec(i);
        v.painter.text(rect.left_center() - vec2(4.0 * z, 0.0), Align2::RIGHT_CENTER, label, theme::font(8.0 * z), theme::TEXT_FAINT);
        let resp = v
            .ui
            .interact(rect, self.uid(("box", i)), Sense::click_and_drag())
            .on_hover_cursor(CursorIcon::ResizeHorizontal)
            .on_hover_text(mg::long_name(i));
        v.hit(rect, ParamRef::Module(i));
        self.target(rect, i);
        if resp.dragged() {
            let d = resp.drag_delta();
            let fine = v.ui.input(|inp| inp.modifiers.shift);
            p.set(i, spec.denorm(spec.to_norm(p.get(i)) + (d.x - d.y) * if fine { 0.001 } else { 0.004 }));
        }
        if resp.double_clicked() {
            p.set(i, spec.default);
        }
        v.painter.rect_filled(rect, 3.0 * z, if resp.hovered() || resp.dragged() { theme::CONTROL_HI } else { theme::INSET });
        let shown = self.live(i).map_or(p.get(i), |n| spec.denorm(n));
        v.painter.text(rect.center(), Align2::CENTER_CENTER, mg::format(p, i, shown), theme::mono(8.5 * z), theme::TEXT);
        resp.context_menu(|ui| self.target_menu(ui, i));
    }

    // -----------------------------------------------------------------------
    // Top bar
    // -----------------------------------------------------------------------

    fn top_bar(&self) {
        let v = self.v;
        let (y0, y1) = (34.0, 52.0);
        self.switch(data::HQ, v.r(52.0, y0, 84.0, y1), "HQ", "High Quality: 2× oversampling against aliasing (more CPU)");
        self.switch(data::AUTO_MUTE, v.r(88.0, y0, 160.0, y1), "AUTO MUTE", "Mute self-oscillation when the input goes silent");
        self.switch(data::ANALYZER, v.r(164.0, y0, 226.0, y1), "ANALYZER", "Spectrum of the output behind the curves");
        self.switch(data::PIANO, v.r(230.0, y0, 274.0, y1), "PIANO", "A piano along the frequency scale");
        let bypass = self.on(data::BYPASS);
        if self.chip("bypass", v.r(278.0, y0, 336.0, y1), "BYPASS", bypass, theme::MUTE, "Bypass, with a soft crossfade").clicked() {
            self.flip(data::BYPASS);
        }
        for (k, (i, label)) in [(mg::IN_GAIN, "IN"), (mg::IN_PAN, "PAN"), (mg::OUT_GAIN, "OUT"), (mg::OUT_PAN, "PAN"), (mg::MIX, "MIX")]
            .into_iter()
            .enumerate()
        {
            let x = 380.0 + k as f32 * 78.0;
            self.number_box(i, v.r(x, y0, x + 54.0, y1), label);
        }
    }

    // -----------------------------------------------------------------------
    // Filter display
    // -----------------------------------------------------------------------

    fn display(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let rect = v.r(DISPLAY[0], DISPLAY[1], DISPLAY[2], DISPLAY[3]);
        let painter = v.painter.with_clip_rect(rect.intersect(v.painter.clip_rect()));
        painter.rect_filled(rect, 4.0 * z, theme::INSET);
        let axis = Axis { rect };
        let sr = p.viz[viz::SR].get().max(8_000.0);
        let font = theme::font(7.5 * z);
        for (f, label) in [(20.0, "20"), (50.0, "50"), (100.0, "100"), (200.0, "200"), (500.0, "500"), (1e3, "1k"), (2e3, "2k"), (5e3, "5k"), (1e4, "10k")] {
            let x = axis.x(f);
            painter.vline(x, rect.y_range(), Stroke::new(1.0 * z, theme::GRID_MAJOR));
            painter.text(pos2(x + 2.0 * z, rect.top() + 2.0 * z), Align2::LEFT_TOP, label, font.clone(), theme::TEXT_FAINT);
        }
        for db in [-24.0, -12.0, 0.0, 12.0, 24.0f32] {
            let y = axis.y(db);
            painter.hline(rect.x_range(), y, Stroke::new(1.0 * z, if db == 0.0 { alpha(theme::TEXT_FAINT, 0.5) } else { theme::GRID_MAJOR }));
            let label = if db == 0.0 { "0".to_owned() } else { format!("{db:+.0}") };
            painter.text(pos2(rect.left() + 3.0 * z, y - 1.0 * z), Align2::LEFT_BOTTOM, label, font.clone(), theme::TEXT_FAINT);
        }
        if self.on(data::ANALYZER) {
            self.analyzer(&painter, &axis, sr);
        }

        // Curves: each filter's, the total (and the live one when modulated).
        let base = mg::base_values(p);
        let live: [f32; PARAM_COUNT] =
            std::array::from_fn(|i| self.live(i).map_or(base[i], |n| mg::spec(i).denorm(n)));
        let modulated = (0..PARAM_COUNT).any(|i| self.live(i).is_some());
        let xs: Vec<f32> = (0..=220).map(|k| rect.left() + rect.width() * k as f32 / 220.0).collect();
        let freqs: Vec<f32> = xs.iter().map(|&x| axis.f(x)).collect();
        let selected = mg::choice(p, data::SELECTED, FILTERS);
        for (f, &color) in FILTER_COLORS.iter().enumerate() {
            if !self.on(data::FILTER_ON + f) {
                continue;
            }
            let pts: Vec<Pos2> = xs
                .iter()
                .zip(&freqs)
                .map(|(&x, &fr)| pos2(x, axis.y(mg::db(mg::filter_response(p, &base, f, 0, fr, sr)))))
                .collect();
            if f == selected {
                fill_to(&painter, &pts, axis.y(0.0), alpha(color, 0.12));
            }
            painter.add(Shape::line(pts, Stroke::new(1.0 * z, alpha(color, if f == selected { 0.9 } else { 0.5 }))));
        }
        let mode = mg::choice(p, data::ROUTING_MODE, 3);
        let split = mode != mg::STEREO || (0..FILTERS).any(|f| self.on(data::FILTER_ON + f) && base[fp(f, filt::FPAN)] != 0.0);
        let total = |vals: &[f32; PARAM_COUNT], ch: usize| -> Vec<Pos2> {
            xs.iter().zip(&freqs).map(|(&x, &fr)| pos2(x, axis.y(mg::db(mg::total_response(p, vals, ch, fr, sr))))).collect()
        };
        if split {
            let cols = if mode == mg::MID_SIDE {
                [Color32::from_rgb(240, 160, 140), Color32::from_rgb(120, 170, 240)]
            } else {
                [theme::TEXT, Color32::from_rgb(232, 88, 88)]
            };
            for (ch, col) in cols.into_iter().enumerate() {
                painter.add(Shape::line(total(&base, ch), Stroke::new(1.6 * z, alpha(col, 0.85))));
            }
        } else {
            painter.add(Shape::line(total(&base, 0), Stroke::new(2.4 * z, CURVE)));
        }
        if modulated {
            painter.add(Shape::line(total(&live, 0), Stroke::new(1.0 * z, alpha(Color32::from_rgb(255, 236, 150), 0.9))));
            v.ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        }
        if self.on(data::PIANO) {
            self.piano(&painter, &axis);
        }

        // Dots.
        let dots = self.dots(&axis, &live);
        for &(f, c) in &dots {
            let color = FILTER_COLORS[f];
            let r = if f == selected { 7.0 } else { 5.5 } * z;
            painter.circle_filled(c, r, if f == selected { color } else { theme::INSET });
            painter.circle_stroke(c, r, Stroke::new(1.6 * z, color));
            painter.text(c, Align2::CENTER_CENTER, (f + 1).to_string(), theme::font(7.5 * z), if f == selected { theme::INSET } else { color });
        }
        let resp = v.ui.interact(rect, self.uid("display"), Sense::click_and_drag());
        self.display_input(&resp, &axis, &dots);
        if let Some(q) = resp.hover_pos() {
            let f = axis.f(q.x);
            let text = format!("{}  {}", fmt_freq(f), note_name(f));
            painter.text(rect.right_top() + vec2(-6.0, 4.0) * z, Align2::RIGHT_TOP, text, theme::mono(8.0 * z), theme::TEXT_DIM);
        }
        if p.viz[viz::MUTED].get() > 0.5 {
            painter.text(rect.left_bottom() + vec2(6.0, -4.0) * z, Align2::LEFT_BOTTOM, "auto-muted", theme::font(8.0 * z), theme::TEXT_FAINT);
        }
    }

    /// Dots of the enabled filters: at their frequency, and their peak as
    /// the height (gain for bells and shelves).
    fn dots(&self, axis: &Axis, live: &[f32; PARAM_COUNT]) -> Vec<(usize, Pos2)> {
        (0..FILTERS)
            .filter(|&f| self.on(data::FILTER_ON + f))
            .map(|f| (f, pos2(axis.x(live[fp(f, filt::FREQ)]), axis.y(live[fp(f, filt::PEAK)] * mg::BELL_DB))))
            .collect()
    }

    /// Drag a dot (frequency across, peak up; Alt: one axis, Shift: fine,
    /// Cmd: frequency pan across and slope up/down). Alt-click: on/off;
    /// Cmd+Alt-click: shape; Alt+Shift-click: slope. Double-click the
    /// background for a new filter shaped by where you click.
    fn display_input(&self, resp: &Response, axis: &Axis, dots: &[(usize, Pos2)]) {
        let (v, p, z) = (self.v, self.p, self.z());
        let hit = |q: Pos2| {
            dots.iter()
                .map(|&(f, c)| (f, c.distance(q)))
                .filter(|&(_, d)| d < 10.0 * z)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(f, _)| f)
        };
        let m = v.ui.input(|i| i.modifiers);
        let drag_id = self.uid("dot-drag");
        if resp.drag_started_by(PointerButton::Primary)
            && let Some(origin) = v.ui.input(|i| i.pointer.press_origin())
        {
            let grab = hit(origin).map(|f| {
                p.data[data::SELECTED].set(f as f32);
                (f, p.get(fp(f, filt::FREQ)), p.get(fp(f, filt::PEAK)), p.get(fp(f, filt::FPAN)), p.data[data::SLOPE + f].get(), origin)
            });
            v.ui.data_mut(|d| d.insert_temp(drag_id, grab));
        }
        if resp.dragged_by(PointerButton::Primary)
            && let Some(Some((f, f0, peak0, pan0, slope0, origin))) = v.ui.data(|d| d.get_temp::<Option<(usize, f32, f32, f32, f32, Pos2)>>(drag_id))
            && let Some(q) = resp.interact_pointer_pos()
        {
            let mut d = q - origin;
            if m.shift {
                d *= 0.25;
            }
            if m.command {
                p.set(fp(f, filt::FPAN), (pan0 + d.x / (200.0 * z)).clamp(-1.0, 1.0));
                let steps = (-d.y / (24.0 * z)).round();
                p.data[data::SLOPE + f].set((slope0 + steps).clamp(0.0, 3.0));
            } else {
                if m.alt {
                    if d.x.abs() > d.y.abs() {
                        d.y = 0.0;
                    } else {
                        d.x = 0.0;
                    }
                }
                p.set(fp(f, filt::FREQ), axis.f(axis.x(f0) + d.x).clamp(10.0, 20_000.0));
                let db = axis.db(axis.y(peak0 * mg::BELL_DB) + d.y);
                p.set(fp(f, filt::PEAK), (db / mg::BELL_DB).clamp(-1.0, 1.0));
            }
        }
        if resp.clicked()
            && let Some(f) = resp.interact_pointer_pos().and_then(hit)
        {
            p.data[data::SELECTED].set(f as f32);
            if m.alt && m.command {
                let s = data::SHAPE + f;
                p.data[s].set(((mg::choice(p, s, SHAPES.len()) + 1) % SHAPES.len()) as f32);
            } else if m.alt && m.shift {
                let s = data::SLOPE + f;
                p.data[s].set(((mg::choice(p, s, SLOPES.len()) + 1) % SLOPES.len()) as f32);
            } else if m.alt {
                self.flip(data::FILTER_ON + f);
            }
        }
        if resp.double_clicked()
            && let Some(q) = resp.interact_pointer_pos()
            && hit(q).is_none()
        {
            self.new_filter(axis, q);
        }
        if let Some(f) = resp.hover_pos().and_then(hit) {
            v.ui.ctx().set_cursor_icon(CursorIcon::Grab);
            resp.clone().on_hover_text(format!(
                "Filter {}: {} · {} · {}\nDrag: frequency / peak · Cmd+drag: pan / slope\nAlt-click: on/off · Cmd+Alt-click: shape · Alt+Shift-click: slope",
                f + 1,
                SHAPES[mg::choice(p, data::SHAPE + f, SHAPES.len())].1,
                SLOPES[mg::choice(p, data::SLOPE + f, SLOPES.len())].1,
                STYLES[mg::choice(p, data::STYLE + f, STYLES.len())].0,
            ));
        }
    }

    /// A new filter where the display was double-clicked: high pass on the
    /// far left, low pass on the far right, a notch along the bottom, a
    /// bell elsewhere.
    fn new_filter(&self, axis: &Axis, q: Pos2) {
        let p = self.p;
        let Some(f) = (0..FILTERS).find(|&f| !self.on(data::FILTER_ON + f)) else {
            return;
        };
        let rel_x = (q.x - axis.rect.left()) / axis.rect.width();
        let rel_y = (q.y - axis.rect.top()) / axis.rect.height();
        let (shape, peak) = if rel_x < 0.12 {
            (mg::HIGH_PASS, 0.0)
        } else if rel_x > 0.88 {
            (mg::LOW_PASS, 0.0)
        } else if rel_y > 0.82 {
            (mg::NOTCH, 0.0)
        } else {
            (mg::BELL, (axis.db(q.y) / mg::BELL_DB).clamp(-1.0, 1.0))
        };
        p.data[data::SHAPE + f].set(shape as f32);
        p.data[data::SLOPE + f].set(if shape == mg::BELL { 1.0 } else { 2.0 });
        p.set(fp(f, filt::FREQ), axis.f(q.x));
        p.set(fp(f, filt::PEAK), peak);
        p.data[data::FILTER_ON + f].set(1.0);
        p.data[data::SELECTED].set(f as f32);
    }

    fn piano(&self, painter: &egui::Painter, axis: &Axis) {
        let z = self.z();
        let r = axis.rect;
        let top = r.bottom() - 14.0 * z;
        painter.rect_filled(Rect::from_min_max(pos2(r.left(), top), r.max), 0.0, Color32::from_rgb(214, 216, 220));
        for note in 15..136 {
            let f0 = 440.0 * 2f32.powf((note as f32 - 69.5) / 12.0);
            let f1 = f0 * 2f32.powf(1.0 / 12.0);
            let (x0, x1) = (axis.x(f0), axis.x(f1));
            if x1 < r.left() || x0 > r.right() {
                continue;
            }
            if matches!(note % 12, 1 | 3 | 6 | 8 | 10) {
                painter.rect_filled(Rect::from_min_max(pos2(x0, top), pos2(x1, top + 8.5 * z)), 0.0, theme::INSET);
            } else if note % 12 == 0 {
                painter.vline(x0, top..=r.bottom(), Stroke::new(1.0 * z, theme::TEXT_FAINT));
            }
        }
    }

    /// The output's spectrum, faint behind the curves.
    fn analyzer(&self, painter: &egui::Painter, axis: &Axis, sr: f32) {
        let (v, p) = (self.v, self.p);
        let (now, dt) = v.ui.input(|i| (i.time, i.stable_dt.min(0.1)));
        let id = self.uid("rta");
        let levels: Vec<(f32, f32)> = v.ui.data_mut(|d| {
            let st = d.get_temp_mut_or_insert_with(id, Rta::default);
            st.update(p, sr, dt, now);
            st.bands.iter().zip(&st.levels).map(|(&(lo, hi), &l)| ((lo * hi).sqrt(), l)).collect()
        });
        let y_of = |db: f32| axis.rect.top() + axis.rect.height() * (db / -96.0).clamp(0.0, 1.0);
        let pts: Vec<Pos2> = levels.iter().map(|&(f, l)| pos2(axis.x(f), y_of(l))).collect();
        fill_to(painter, &pts, axis.rect.bottom(), alpha(Color32::from_rgb(120, 130, 150), 0.22));
    }

    fn output_meter(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let id = self.uid("meters");
        let dt = v.ui.input(|i| i.stable_dt.min(0.1));
        let states = v.ui.data_mut(|d| {
            let m = d.get_temp_mut_or_default::<[MeterState; 2]>(id);
            m[0].update(p.viz[viz::OUT_L].take(), dt);
            m[1].update(p.viz[viz::OUT_R].take(), dt);
            *m
        });
        let norm = |lin: f32| ((crate::audio::params::gain_to_db(lin) + 48.0) / 48.0).clamp(0.0, 1.0);
        for (ch, state) in states.iter().enumerate() {
            let x = DISPLAY[2] + 8.0 + ch as f32 * 14.0;
            let r = v.r(x, DISPLAY[1], x + 11.0, DISPLAY[3]);
            v.painter.rect_filled(r, 2.0 * z, theme::INSET);
            let h = norm(state.level);
            if h > 0.0 {
                let db = crate::audio::params::gain_to_db(state.level);
                let col = if db > -1.0 { theme::METER_RED } else if db > -9.0 { theme::METER_YELLOW } else { theme::METER_GREEN };
                v.painter.rect_filled(Rect::from_min_max(pos2(r.left(), r.bottom() - r.height() * h), r.max), 2.0 * z, col);
            }
            v.painter.hline(r.x_range(), r.bottom() - r.height() * norm(state.peak), Stroke::new(1.0 * z, theme::TEXT_DIM));
        }
    }

    // -----------------------------------------------------------------------
    // Filter controls
    // -----------------------------------------------------------------------

    fn filter_controls(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let y = CONTROLS_Y;
        // Routing.
        let routing = mg::choice(p, data::ROUTING, ROUTINGS.len());
        let rrect = v.r(PAD, y, 112.0, y + 36.0);
        let resp = v
            .ui
            .interact(rrect, self.uid("routing"), Sense::CLICK)
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(format!("Routing: {}\nClick for the next (left half: previous)", ROUTINGS[routing].1));
        if resp.clicked() {
            let back = resp.interact_pointer_pos().is_some_and(|q| q.x < rrect.center().x);
            let n = ROUTINGS.len();
            p.data[data::ROUTING].set(if back { (routing + n - 1) % n } else { (routing + 1) % n } as f32);
        }
        v.painter.rect_filled(rrect, 4.0 * z, if resp.hovered() { theme::CONTROL_HI } else { theme::CONTROL });
        let mode = mg::choice(p, data::ROUTING_MODE, 3);
        let diagram = if mode == mg::STEREO {
            ROUTINGS[routing].0
        } else if routing == 0 {
            "1→2 / 3→4"
        } else {
            "1|2 / 3|4"
        };
        v.painter.text(rrect.center() - vec2(0.0, 5.0 * z), Align2::CENTER_CENTER, diagram, theme::mono(9.0 * z), CURVE);
        v.painter.text(rrect.center() + vec2(0.0, 10.0 * z), Align2::CENTER_CENTER, "ROUTING", theme::font(7.0 * z), theme::TEXT_FAINT);
        self.seg("mode", v.r(PAD, y + 40.0, 112.0, y + 58.0), &ROUTING_MODES, data::ROUTING_MODE, self.accent);

        // Which filter, and its switches.
        let selected = mg::choice(p, data::SELECTED, FILTERS);
        for (f, &color) in FILTER_COLORS.iter().enumerate() {
            let x = 120.0 + f as f32 * 46.0;
            let rect = v.r(x, y, x + 42.0, y + 18.0);
            let on = self.on(data::FILTER_ON + f);
            let r = v
                .ui
                .interact(rect, self.uid(("filter", f)), Sense::CLICK)
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text("Click to select; double-click to switch on or off");
            let fill = if f == selected { if on { color } else { theme::TEXT_FAINT } } else if r.hovered() { theme::CONTROL_HI } else { theme::CONTROL };
            v.painter.rect_filled(rect, 3.0 * z, fill);
            let text = if f == selected { theme::INSET } else if on { color } else { theme::TEXT_FAINT };
            v.painter.text(rect.center(), Align2::CENTER_CENTER, (f + 1).to_string(), theme::font(9.0 * z), text);
            if r.double_clicked() {
                self.flip(data::FILTER_ON + f);
            } else if r.clicked() {
                p.data[data::SELECTED].set(f as f32);
            }
        }
        let f = selected;
        let color = FILTER_COLORS[f];
        let shapes: Vec<&str> = SHAPES.iter().map(|s| s.0).collect();
        self.cycler("shape", v.r(120.0, y + 22.0, 190.0, y + 40.0), data::SHAPE + f, &shapes, "Shape");
        self.seg("slope", v.r(194.0, y + 22.0, 300.0, y + 40.0), &SLOPES, data::SLOPE + f, color);
        let styles: Vec<&str> = STYLES.iter().map(|s| s.0).collect();
        let style = mg::choice(p, data::STYLE + f, STYLES.len());
        self.cycler("style", v.r(120.0, y + 44.0, 226.0, y + 62.0), data::STYLE + f, &styles, STYLES[style].1);
        let on = self.on(data::FILTER_ON + f);
        if self.chip("on", v.r(230.0, y + 44.0, 264.0, y + 62.0), if on { "ON" } else { "OFF" }, on, color, "Enable or bypass this filter").clicked() {
            self.flip(data::FILTER_ON + f);
        }
        if self.chip("delete", v.r(268.0, y + 44.0, 300.0, y + 62.0), "×", false, color, "Delete this filter (and its modulation)").clicked() {
            p.data[data::FILTER_ON + f].set(0.0);
            for k in 0..filt::COUNT {
                for (s, _) in self.slots_on(fp(f, k)) {
                    mg::set_slot(p, s, 0, 0);
                }
            }
        }
        for k in 0..filt::COUNT {
            self.knob(fp(f, k), 334.0 + 64.0 * k as f32, y + 2.0, color);
        }
    }

    // -----------------------------------------------------------------------
    // Modulation
    // -----------------------------------------------------------------------

    fn modulation(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let shown = mg::choice(p, data::SOURCE_SHOWN, TABS.len());
        let used: Vec<usize> = (0..SLOTS).filter_map(|s| mg::slot(p, s).map(|(src, _)| src)).collect();
        let drag_id = Id::unique(("magma-drag", self.id));
        for (t, &(name, color, sources)) in TABS.iter().enumerate() {
            let x = PAD + t as f32 * 72.0;
            let rect = v.r(x, MOD_Y, x + 68.0, MOD_Y + 20.0);
            let handle_w = 15.0 * z;
            let body = Rect::from_min_max(pos2(rect.left() + handle_w * sources.len() as f32, rect.top()), rect.max);
            let r = v.ui.interact(body, self.uid(("tab", t)), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand);
            let lit = t == shown;
            v.painter.rect_filled(rect, 3.0 * z, if lit { alpha(color, 0.35) } else if r.hovered() { theme::CONTROL_HI } else { theme::CONTROL });
            let in_use = sources.iter().any(|s| used.contains(s));
            v.painter.text(body.center(), Align2::CENTER_CENTER, name, theme::font(8.5 * z), if in_use || lit { theme::TEXT } else { theme::TEXT_DIM });
            if r.clicked() {
                p.data[data::SOURCE_SHOWN].set(t as f32);
            }
            // Drag handles: drop one on a knob to modulate it.
            for (k, &src) in sources.iter().enumerate() {
                let h = Rect::from_min_size(rect.min + vec2(handle_w * k as f32, 0.0), vec2(handle_w, rect.height()));
                let hr = v
                    .ui
                    .interact(h, self.uid(("handle", src)), Sense::drag())
                    .on_hover_cursor(CursorIcon::Grab)
                    .on_hover_text(format!("Drag {} onto a knob to modulate it", md::SOURCE_NAMES[src]));
                let c = h.center();
                let active = used.contains(&src);
                v.painter.circle_stroke(c, 4.5 * z, Stroke::new(1.4 * z, color));
                if active {
                    v.painter.circle_filled(c, 2.2 * z, color);
                }
                if sources.len() > 1 {
                    v.painter.text(c + vec2(0.0, 7.5 * z), Align2::CENTER_CENTER, ["X", "Y"][k], theme::font(6.0 * z), color);
                }
                if hr.drag_started() {
                    v.ui.data_mut(|d| d.insert_temp(drag_id, src));
                }
                if hr.dragged()
                    && let Some(q) = hr.interact_pointer_pos()
                {
                    v.painter.line_segment([c, q], Stroke::new(1.5 * z, color));
                    v.painter.circle_filled(q, 3.0 * z, color);
                    v.ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                }
                if hr.drag_stopped() {
                    v.ui.data_mut(|d| d.remove::<usize>(drag_id));
                    if let Some(q) = hr.interact_pointer_pos().or_else(|| v.ui.input(|i| i.pointer.latest_pos())) {
                        self.dropped.set(Some((src, q)));
                    }
                }
            }
        }
        let editor = v.r(PAD, EDIT_Y, 520.0, SIZE.y - PAD);
        self.inset(editor.expand(2.0 * z));
        match shown {
            0 | 1 => self.xlfo_editor(shown),
            2 | 3 => self.eg_editor(shown - 2),
            4 | 5 => self.ef_editor(shown - 4),
            6 | 7 => self.midi_editor(shown - 6),
            _ => self.xy_editor(shown - 8),
        }
        if !matches!(shown, 2..=5) {
            p.viz[viz::AUDITION].set(-1.0);
        }
        self.slots_panel(530.0, W - PAD);
    }

    /// A source dropped on a control makes a slot.
    fn finish_drop(&self) {
        let Some((src, q)) = self.dropped.get() else {
            return;
        };
        let target = self.targets.borrow().iter().rev().find(|(r, _)| r.contains(q)).map(|&(_, i)| i);
        if let Some(i) = target {
            mg::add_slot(self.p, src, i);
        }
    }

    fn xlfo_editor(&self, x: usize) {
        let (v, p, z) = (self.v, self.p, self.z());
        let d = xlfo_data(x);
        let n = md::steps(p, x);
        let area = v.r(PAD + 4.0, EDIT_Y + 4.0, 340.0, EDIT_Y + 92.0);
        v.painter.rect_filled(area, 3.0 * z, alpha(theme::CONTROL, 0.4));
        let col_w = area.width() / n as f32;
        let mid = area.center().y;
        let half = area.height() * 0.45;
        let selected = (p.data[d + xlfo::SELECTED].get().round().max(0.0) as usize).min(n - 1);
        let phase = p.viz[viz::XLFO_PHASE + x].get();
        let playing = ((phase * n as f32) as usize).min(n - 1);
        v.painter.hline(area.x_range(), mid, Stroke::new(1.0 * z, theme::GRID_MAJOR));
        for i in 0..n {
            let (value, random, _, _) = md::step(p, x, i);
            let x0 = area.left() + col_w * i as f32;
            let col = Rect::from_min_max(pos2(x0 + 1.5 * z, area.top()), pos2(x0 + col_w - 1.5 * z, area.bottom()));
            if i == selected {
                v.painter.rect_filled(col, 2.0 * z, alpha(XLFO_COL, 0.12));
            }
            let y = mid - value * half;
            let bar = Rect::from_min_max(pos2(col.left(), y.min(mid)), pos2(col.right(), y.max(mid)));
            let c = if random { alpha(theme::TEXT_FAINT, 0.6) } else { alpha(XLFO_COL, if i == playing { 0.9 } else { 0.55 }) };
            v.painter.rect_filled(bar, 1.0 * z, c);
            if random {
                v.painter.text(col.center(), Align2::CENTER_CENTER, "?", theme::font(10.0 * z), theme::TEXT_DIM);
            }
        }
        // The waveform the steps make.
        let mut pts = Vec::new();
        for k in 0..=160 {
            let psi = k as f32 / 160.0;
            pts.push(pos2(area.left() + area.width() * psi, mid - xlfo_shape(p, x, psi) * half));
        }
        v.painter.add(Shape::line(pts, Stroke::new(1.3 * z, XLFO_COL)));
        v.painter.vline(area.left() + area.width() * phase, area.y_range(), Stroke::new(1.0 * z, alpha(theme::TEXT, 0.7)));
        let resp = v.ui.interact(area, self.uid(("steps", x)), Sense::click_and_drag());
        if let Some(q) = resp.interact_pointer_pos()
            && (resp.clicked() || resp.dragged())
        {
            let i = (((q.x - area.left()) / col_w) as usize).min(n - 1);
            p.data[d + xlfo::SELECTED].set(i as f32);
            if resp.dragged() || resp.clicked() {
                let value = ((mid - q.y) / half).clamp(-1.0, 1.0);
                p.data[d + xlfo::STEP0 + i * 4].set(value);
            }
        }
        // Step count.
        for (k, (glyph, delta)) in [("+", 1i32), ("−", -1)].into_iter().enumerate() {
            let y = EDIT_Y + 4.0 + k as f32 * 22.0;
            if self.chip(&format!("steps{glyph}"), v.r(346.0, y, 366.0, y + 18.0), glyph, false, XLFO_COL, "Add or remove a step").clicked() {
                let new = (n as i32 + delta).clamp(1, mg::MAX_STEPS as i32) as usize;
                if new > n {
                    // A new step continues the last one.
                    let (value, ..) = md::step(p, x, n - 1);
                    let at = d + xlfo::STEP0 + n * 4;
                    p.data[at].set(value);
                    p.data[at + 1].set(0.0);
                    p.data[at + 2].set(0.0);
                    p.data[at + 3].set(0.0);
                }
                p.data[d + xlfo::STEPS].set(new as f32);
            }
        }
        v.text(356.0, EDIT_Y + 56.0, Align2::CENTER_CENTER, format!("{n}"), v.mono(9.0), theme::TEXT_DIM);

        // Global settings.
        let synced = mg::choice(p, d + xlfo::SYNC, mg::SYNC_LENGTHS.len() + 1) != 0;
        let first = if synced { xp(x, xlfo::OFFSET) } else { xp(x, xlfo::RATE) };
        for (k, i) in [first, xp(x, xlfo::BALANCE), xp(x, xlfo::GLIDE), xp(x, xlfo::PHASE)].into_iter().enumerate() {
            self.knob(i, PAD + 32.0 + 64.0 * k as f32, EDIT_Y + 102.0, XLFO_COL);
        }
        let mut sync_labels = vec!["FREE"];
        sync_labels.extend(mg::SYNC_LENGTHS.iter().map(|s| s.0));
        let col = 376.0;
        v.text(col, EDIT_Y + 8.0, Align2::LEFT_CENTER, "SYNC (BARS)", v.font(7.5), theme::TEXT_FAINT);
        self.cycler("sync", v.r(col, EDIT_Y + 14.0, 516.0, EDIT_Y + 30.0), d + xlfo::SYNC, &sync_labels, "Free running, or a cycle of this many bars");
        v.text(col, EDIT_Y + 40.0, Align2::LEFT_CENTER, "MIDI", v.font(7.5), theme::TEXT_FAINT);
        self.cycler("midi", v.r(col, EDIT_Y + 46.0, 466.0, EDIT_Y + 62.0), d + xlfo::MIDI_TRIG, &md::MIDI_TRIGGERS, "Restart on MIDI notes (LEGATO: only the first of overlapping notes)");
        self.switch(d + xlfo::SNAP, v.r(470.0, EDIT_Y + 46.0, 516.0, EDIT_Y + 62.0), "SNAP", "Steps snap to semitones over two octaves (an arpeggiator on frequencies)");

        // The selected step.
        let at = d + xlfo::STEP0 + selected * 4;
        v.text(col, EDIT_Y + 76.0, Align2::LEFT_CENTER, format!("STEP {}", selected + 1), v.font(8.0), XLFO_COL);
        let random = p.data[at + 1].get() > 0.5;
        if self.chip("random", v.r(col, EDIT_Y + 84.0, 440.0, EDIT_Y + 100.0), "RANDOM", random, XLFO_COL, "A new random value each time this step comes").clicked() {
            p.data[at + 1].set(if random { 0.0 } else { 1.0 });
        }
        self.cycler("curve", v.r(444.0, EDIT_Y + 84.0, 516.0, EDIT_Y + 100.0), at + 2, &md::STEP_CURVES, "The curve gliding to the next step");
        self.data_box(v.r(col + 40.0, EDIT_Y + 106.0, 516.0, EDIT_Y + 122.0), at, "VALUE", (-1.0, 1.0), |v| format!("{:+.0}%", v * 100.0));
        self.data_box(v.r(col + 40.0, EDIT_Y + 128.0, 516.0, EDIT_Y + 144.0), at + 3, "GLIDE", (0.0, 1.0), |v| format!("{:.0}%", v * 100.0));
    }

    /// A value kept in `data` (not a knob): drag it.
    fn data_box(&self, rect: Rect, i: usize, label: &str, (lo, hi): (f32, f32), fmt: impl Fn(f32) -> String) {
        let (v, p, z) = (self.v, self.p, self.z());
        v.painter.text(rect.left_center() - vec2(4.0 * z, 0.0), Align2::RIGHT_CENTER, label, theme::font(7.5 * z), theme::TEXT_FAINT);
        let resp = v.ui.interact(rect, self.uid(("data", i)), Sense::click_and_drag()).on_hover_cursor(CursorIcon::ResizeHorizontal);
        if resp.dragged() {
            let d = resp.drag_delta();
            p.data[i].set((p.data[i].get() + (d.x - d.y) * (hi - lo) * 0.004).clamp(lo, hi));
        }
        v.painter.rect_filled(rect, 3.0 * z, if resp.hovered() || resp.dragged() { theme::CONTROL_HI } else { theme::CONTROL });
        v.painter.text(rect.center(), Align2::CENTER_CENTER, fmt(p.data[i].get()), theme::mono(8.0 * z), theme::TEXT);
    }

    fn trigger_meter(&self, rect: Rect, level: f32, threshold: Option<f32>, color: Color32) {
        let (v, z) = (self.v, self.z());
        v.painter.rect_filled(rect, 2.0 * z, theme::INSET);
        let x_of = |db: f32| rect.left() + rect.width() * ((db + 60.0) / 60.0).clamp(0.0, 1.0);
        v.painter.rect_filled(Rect::from_min_max(rect.min, pos2(x_of(level), rect.bottom())), 2.0 * z, alpha(color, 0.6));
        if let Some(t) = threshold {
            v.painter.vline(x_of(t), rect.y_range(), Stroke::new(1.5 * z, theme::TEXT));
        }
    }

    /// The audition button: while held, the output is the SC input.
    fn audition(&self, rect: Rect, which: usize, color: Color32) {
        let (v, p, z) = (self.v, self.p, self.z());
        let resp = v
            .ui
            .interact(rect, self.uid(("audition", which)), Sense::click_and_drag())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Hold to hear the side chain this source triggers on");
        let held = resp.is_pointer_button_down_on();
        p.viz[viz::AUDITION].set(if held { which as f32 } else { -1.0 });
        v.painter.rect_filled(rect, 3.0 * z, if held { color } else { theme::CONTROL });
        v.painter.text(rect.center(), Align2::CENTER_CENTER, "AUDITION", theme::font(8.0 * z), if held { theme::INSET } else { theme::TEXT_DIM });
    }

    fn eg_editor(&self, e: usize) {
        let (v, p, z) = (self.v, self.p, self.z());
        let area = v.r(PAD + 4.0, EDIT_Y + 4.0, 366.0, EDIT_Y + 92.0);
        v.painter.rect_filled(area, 3.0 * z, alpha(theme::CONTROL, 0.4));
        let val = |k: usize| self.live(egp(e, k)).map_or(p.get(egp(e, k)), |n| mg::spec(egp(e, k)).denorm(n));
        let sustain = val(eg::SUSTAIN);
        // Segment widths: log-ish times, so short and long both show.
        let w = |t: f32| (1.0 + t * 20.0).ln();
        let segs = [w(val(eg::DELAY)), w(val(eg::ATTACK)), w(val(eg::DECAY)), 1.2, w(val(eg::HOLD)), w(val(eg::RELEASE))];
        let total: f32 = segs.iter().sum();
        let mut x = area.left();
        let y_of = |l: f32| area.bottom() - area.height() * 0.9 * l;
        let mut pts = vec![pos2(x, y_of(0.0))];
        let curve = |pts: &mut Vec<Pos2>, x0: f32, x1: f32, from: f32, to: f32, slope: f32| {
            for k in 1..=16 {
                let u = k as f32 / 16.0;
                pts.push(pos2(x0 + (x1 - x0) * u, y_of(from + (to - from) * md::segment(u, slope))));
            }
        };
        let stage = p.viz[viz::EG_STAGE + e].get() as usize;
        let mut spans = Vec::new();
        for (k, &s) in segs.iter().enumerate() {
            let x1 = x + area.width() * s / total;
            match k {
                0 => pts.push(pos2(x1, y_of(0.0))),
                1 => curve(&mut pts, x, x1, 0.0, 1.0, val(eg::A_SLOPE)),
                2 => curve(&mut pts, x, x1, 1.0, sustain, val(eg::D_SLOPE)),
                3 | 4 => pts.push(pos2(x1, y_of(sustain))),
                _ => curve(&mut pts, x, x1, sustain, 0.0, val(eg::R_SLOPE)),
            }
            spans.push((x, x1));
            x = x1;
        }
        if (1..=6).contains(&stage) {
            let (a, b) = spans[stage - 1];
            v.painter.rect_filled(Rect::from_x_y_ranges(a..=b, area.y_range()), 0.0, alpha(EG_COL, 0.12));
        }
        v.painter.add(Shape::line(pts, Stroke::new(1.5 * z, EG_COL)));
        let out = p.viz[viz::SOURCES_AT + md::SRC_EG - 1 + e].get();
        v.text(370.0 - 8.0, EDIT_Y + 10.0, Align2::RIGHT_CENTER, format!("{:+.2}", out), v.mono(8.0), theme::TEXT_DIM);
        for (k, i) in [eg::DELAY, eg::ATTACK, eg::DECAY, eg::SUSTAIN, eg::HOLD, eg::RELEASE].into_iter().enumerate() {
            self.knob(egp(e, i), PAD + 30.0 + 58.0 * k as f32, EDIT_Y + 100.0, EG_COL);
        }
        for (k, i) in [eg::THRESHOLD, eg::A_SLOPE, eg::D_SLOPE, eg::R_SLOPE].into_iter().enumerate() {
            self.knob(egp(e, i), PAD + 30.0 + 58.0 * k as f32, EDIT_Y + 152.0, EG_COL);
        }
        let col = 376.0;
        let d = data::EG + e * 2;
        v.text(col, EDIT_Y + 8.0, Align2::LEFT_CENTER, "TRIGGER", v.font(7.5), theme::TEXT_FAINT);
        self.cycler("egtrig", v.r(col, EDIT_Y + 14.0, 516.0, EDIT_Y + 30.0), d, &md::EG_TRIGGERS, "What triggers it: the input, the SC input, or MIDI notes");
        v.text(col, EDIT_Y + 40.0, Align2::LEFT_CENTER, "RANGE", v.font(7.5), theme::TEXT_FAINT);
        self.cycler("egrange", v.r(col, EDIT_Y + 46.0, 516.0, EDIT_Y + 62.0), d + 1, &md::EG_RANGES, "NORMAL: 0…1; NEUTRAL: 0 while sustaining");
        let trigger = mg::choice(p, d, 3);
        if trigger != 2 {
            v.text(col, EDIT_Y + 72.0, Align2::LEFT_CENTER, "LEVEL vs THRESHOLD", v.font(7.5), theme::TEXT_FAINT);
            self.trigger_meter(v.r(col, EDIT_Y + 78.0, 516.0, EDIT_Y + 86.0), p.viz[viz::TRIGGER + e].get(), Some(val(eg::THRESHOLD)), EG_COL);
        }
        if trigger == 1 {
            self.audition(v.r(col, EDIT_Y + 94.0, 516.0, EDIT_Y + 112.0), e, EG_COL);
        } else {
            p.viz[viz::AUDITION].set(-1.0);
        }
    }

    fn ef_editor(&self, f: usize) {
        let (v, p, z) = (self.v, self.p, self.z());
        let area = v.r(PAD + 4.0, EDIT_Y + 4.0, 366.0, EDIT_Y + 92.0);
        v.painter.rect_filled(area, 3.0 * z, alpha(theme::CONTROL, 0.4));
        // Recent output, newest on the right.
        let id = self.uid(("ef-trace", f));
        let out = p.viz[viz::SOURCES_AT + md::SRC_EF - 1 + f].get();
        let trace: Vec<f32> = v.ui.data_mut(|d| {
            let t = d.get_temp_mut_or_default::<std::collections::VecDeque<f32>>(id);
            t.push_back(out);
            while t.len() > 120 {
                t.pop_front();
            }
            t.iter().copied().collect()
        });
        let pts: Vec<Pos2> = trace
            .iter()
            .enumerate()
            .map(|(k, &o)| pos2(area.right() - area.width() * (trace.len() - 1 - k) as f32 / 119.0, area.bottom() - area.height() * 0.9 * o))
            .collect();
        if pts.len() > 1 {
            fill_to(&v.painter.with_clip_rect(area), &pts, area.bottom(), alpha(EF_COL, 0.2));
            v.painter.add(Shape::line(pts, Stroke::new(1.4 * z, EF_COL)));
        }
        v.ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        for (k, i) in [ef::ATTACK, ef::RELEASE].into_iter().enumerate() {
            self.knob(efp(f, i), PAD + 30.0 + 58.0 * k as f32, EDIT_Y + 100.0, EF_COL);
        }
        let col = 376.0;
        let d = data::EF + f * 2;
        v.text(col, EDIT_Y + 8.0, Align2::LEFT_CENTER, "TRIGGER", v.font(7.5), theme::TEXT_FAINT);
        self.cycler("eftrig", v.r(col, EDIT_Y + 14.0, 516.0, EDIT_Y + 30.0), d, &md::EF_TRIGGERS, "What it follows: the input or the SC input");
        v.text(col, EDIT_Y + 40.0, Align2::LEFT_CENTER, "MODE", v.font(7.5), theme::TEXT_FAINT);
        self.cycler("efmode", v.r(col, EDIT_Y + 46.0, 516.0, EDIT_Y + 62.0), d + 1, &md::EF_MODES, "ENVELOPE: the level; TRANSIENT: only short bursts (drum hits)");
        v.text(col, EDIT_Y + 72.0, Align2::LEFT_CENTER, "LEVEL", v.font(7.5), theme::TEXT_FAINT);
        self.trigger_meter(v.r(col, EDIT_Y + 78.0, 516.0, EDIT_Y + 86.0), p.viz[viz::TRIGGER + 2 + f].get(), None, EF_COL);
        if mg::choice(p, d, 2) == 1 {
            self.audition(v.r(col, EDIT_Y + 94.0, 516.0, EDIT_Y + 112.0), 2 + f, EF_COL);
        } else {
            p.viz[viz::AUDITION].set(-1.0);
        }
    }

    fn midi_editor(&self, m: usize) {
        let (v, p, z) = (self.v, self.p, self.z());
        let area = v.r(PAD + 4.0, EDIT_Y + 4.0, 200.0, EDIT_Y + 150.0);
        v.painter.rect_filled(area, 3.0 * z, alpha(theme::CONTROL, 0.4));
        let d = data::MIDI + m * 3;
        let curve = mg::choice(p, d + 2, md::MIDI_CURVES.len());
        let pts: Vec<Pos2> = (0..=48)
            .map(|k| {
                let u = k as f32 / 48.0;
                pos2(area.left() + area.width() * u, area.bottom() - area.height() * md::midi_curve(curve, u))
            })
            .collect();
        v.painter.add(Shape::line(pts, Stroke::new(1.5 * z, MIDI_COL)));
        let out = p.viz[viz::SOURCES_AT + md::SRC_MIDI - 1 + m].get();
        v.painter.hline(area.x_range(), area.bottom() - area.height() * out.abs(), Stroke::new(1.0 * z, alpha(theme::TEXT, 0.5)));
        v.text(206.0, EDIT_Y + 10.0, Align2::LEFT_CENTER, format!("{out:+.2}"), v.mono(9.0), theme::TEXT_DIM);
        let col = 376.0;
        v.text(col, EDIT_Y + 8.0, Align2::LEFT_CENTER, "INPUT", v.font(7.5), theme::TEXT_FAINT);
        self.cycler("midiin", v.r(col, EDIT_Y + 14.0, 516.0, EDIT_Y + 30.0), d, &md::MIDI_INPUTS, "Which MIDI data drives it");
        if mg::choice(p, d, md::MIDI_INPUTS.len()) == 5 {
            self.data_box(v.r(col + 40.0, EDIT_Y + 36.0, 516.0, EDIT_Y + 52.0), d + 1, "CC", (0.0, 127.0), |v| format!("{:.0}", v.round()));
        }
        v.text(col, EDIT_Y + 62.0, Align2::LEFT_CENTER, "CURVE", v.font(7.5), theme::TEXT_FAINT);
        self.cycler("midicurve", v.r(col, EDIT_Y + 68.0, 516.0, EDIT_Y + 84.0), d + 2, &md::MIDI_CURVES, "Response curve");
        v.text(206.0, EDIT_Y + 28.0, Align2::LEFT_TOP, "MIDI from the MIDI input,\nor the keyboards when\nnothing is patched there", v.font(8.0), theme::TEXT_FAINT);
    }

    fn xy_editor(&self, c: usize) {
        let (v, p, z) = (self.v, self.p, self.z());
        let d = data::XY + c * 2;
        let slider = mg::choice(p, d, 2) == 1;
        let pad = if slider { v.r(PAD + 4.0, EDIT_Y + 4.0, PAD + 40.0, SIZE.y - PAD - 4.0) } else { v.r(PAD + 4.0, EDIT_Y + 4.0, PAD + 200.0, SIZE.y - PAD - 4.0) };
        v.painter.rect_filled(pad, 4.0 * z, alpha(XY_COL, 0.08));
        v.painter.rect_stroke(pad, 4.0 * z, Stroke::new(1.0 * z, alpha(XY_COL, 0.4)), egui::StrokeKind::Inside);
        let (ix, iy) = (mg::XY0 + c * 2, mg::XY0 + c * 2 + 1);
        let resp = v.ui.interact(pad, self.uid(("xy", c)), Sense::click_and_drag()).on_hover_cursor(CursorIcon::Crosshair);
        v.hit(pad, ParamRef::Module(iy));
        self.target(pad, iy);
        if let Some(q) = resp.interact_pointer_pos()
            && resp.dragged()
        {
            let x = ((q.x - pad.left()) / pad.width() * 2.0 - 1.0).clamp(-1.0, 1.0);
            let y = ((pad.bottom() - q.y) / pad.height() * 2.0 - 1.0).clamp(-1.0, 1.0);
            let shift = v.ui.input(|i| i.modifiers.shift);
            if !slider && !shift {
                p.set(ix, x);
            }
            p.set(iy, y);
        }
        if resp.double_clicked() {
            p.set(ix, 0.0);
            p.set(iy, 0.0);
        }
        let pos = pos2(
            if slider { pad.center().x } else { pad.left() + pad.width() * (p.get(ix) + 1.0) * 0.5 },
            pad.bottom() - pad.height() * (p.get(iy) + 1.0) * 0.5,
        );
        if !slider {
            v.painter.vline(pos.x, pad.y_range(), Stroke::new(1.0 * z, alpha(XY_COL, 0.4)));
        }
        v.painter.hline(pad.x_range(), pos.y, Stroke::new(1.0 * z, alpha(XY_COL, 0.4)));
        v.painter.circle_filled(pos, 6.0 * z, XY_COL);
        let col = 376.0;
        v.text(col, EDIT_Y + 8.0, Align2::LEFT_CENTER, "MODE", v.font(7.5), theme::TEXT_FAINT);
        self.cycler("xymode", v.r(col, EDIT_Y + 14.0, 516.0, EDIT_Y + 30.0), d, &md::XY_MODES, "Two dimensions, or one slider");
        v.text(col, EDIT_Y + 40.0, Align2::LEFT_CENTER, "RANGE", v.font(7.5), theme::TEXT_FAINT);
        let bipolar = self.on(d + 1);
        if self.chip("range", v.r(col, EDIT_Y + 46.0, 516.0, EDIT_Y + 62.0), if bipolar { "BIPOLAR −1…1" } else { "UNIPOLAR 0…1" }, bipolar, XY_COL, "Output range").clicked() {
            self.flip(d + 1);
        }
        let text = if slider { format!("Y {:+.2}", p.get(iy)) } else { format!("X {:+.2}   Y {:+.2}", p.get(ix), p.get(iy)) };
        v.text(col, EDIT_Y + 76.0, Align2::LEFT_CENTER, text, v.mono(8.5), theme::TEXT_DIM);
        v.text(col, EDIT_Y + 96.0, Align2::LEFT_TOP, "Drag the dot (Shift: Y only);\ndouble-click centres it.\nDrag X / Y from the tab\nonto knobs.", v.font(8.0), theme::TEXT_FAINT);
    }

    /// The slots: source, target, level (a knob, so modulatable), invert,
    /// remove.
    fn slots_panel(&self, x0: f32, x1: f32) {
        let (v, p, z) = (self.v, self.p, self.z());
        v.text(x0, EDIT_Y + 6.0, Align2::LEFT_CENTER, "MODULATION SLOTS", theme::strong(8.5 * z), theme::TEXT_DIM);
        let rows: Vec<usize> = (0..SLOTS).filter(|&s| mg::slot_raw(p, s) != (0, 0)).collect();
        let per_page = 8;
        let pages = rows.len().div_ceil(per_page).max(1);
        let page = (p.data[data::SLOT_PAGE].get().max(0.0) as usize).min(pages - 1);
        if pages > 1 && self.chip("page", v.r(x1 - 40.0, EDIT_Y, x1, EDIT_Y + 14.0), &format!("{}/{pages}", page + 1), false, self.accent, "More slots").clicked() {
            p.data[data::SLOT_PAGE].set(((page + 1) % pages) as f32);
        }
        if rows.is_empty() {
            v.text((x0 + x1) * 0.5, EDIT_Y + 60.0, Align2::CENTER_CENTER, "Drag a source's ◯ onto a knob,\nor right-click a knob", v.font(8.5), theme::TEXT_FAINT);
        }
        for (k, &s) in rows.iter().skip(page * per_page).take(per_page).enumerate() {
            let y = EDIT_Y + 16.0 + k as f32 * 22.0;
            let (src, target) = mg::slot_raw(p, s);
            let color = source_color(src);
            v.painter.circle_filled(v.p(x0 + 4.0, y + 9.0), 3.0 * z, color);
            // Source.
            let names: Vec<&str> = md::SOURCE_NAMES.to_vec();
            let src_rect = v.r(x0 + 10.0, y, x0 + 70.0, y + 18.0);
            let r = v.ui.interact(src_rect, self.uid(("slot-src", s)), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Click for the next source");
            if r.clicked() {
                p.data[data::SLOT_SOURCE + s].set((src % SOURCES + 1) as f32);
            }
            v.painter.rect_filled(src_rect, 3.0 * z, if r.hovered() { theme::CONTROL_HI } else { theme::CONTROL });
            v.painter.text(src_rect.center(), Align2::CENTER_CENTER, names[src], theme::font(7.5 * z), color);
            // Target.
            let tgt_rect = v.r(x0 + 74.0, y, x0 + 168.0, y + 18.0);
            let label = if target == 0 { "choose…".to_owned() } else { short_target(target - 1) };
            let t = v.ui.interact(tgt_rect, self.uid(("slot-tgt", s)), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand);
            v.painter.rect_filled(tgt_rect, 3.0 * z, if t.hovered() { theme::CONTROL_HI } else { theme::CONTROL });
            v.painter.text(tgt_rect.center(), Align2::CENTER_CENTER, label, theme::font(7.5 * z), theme::TEXT);
            let t = if target > 0 { t.on_hover_text(mg::long_name(target - 1)) } else { t };
            egui::Popup::menu(&t).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| self.target_picker(ui, s));
            // Level: drag sideways.
            let lvl_rect = v.r(x0 + 172.0, y + 2.0, x1 - 40.0, y + 16.0);
            let li = mg::SLOT0 + s;
            let lr = v.ui.interact(lvl_rect, self.uid(("slot-lvl", s)), Sense::click_and_drag()).on_hover_cursor(CursorIcon::ResizeHorizontal);
            v.hit(lvl_rect, ParamRef::Module(li));
            self.target(lvl_rect, li);
            if lr.dragged() {
                let fine = v.ui.input(|i| i.modifiers.shift);
                p.set(li, (p.get(li) + lr.drag_delta().x / lvl_rect.width() * if fine { 0.5 } else { 2.0 }).clamp(-1.0, 1.0));
            }
            if lr.double_clicked() {
                p.set(li, 0.5);
            }
            let level = self.live(li).map_or(p.get(li), |n| n * 2.0 - 1.0);
            v.painter.rect_filled(lvl_rect, 2.0 * z, theme::INSET);
            let cx = lvl_rect.center().x;
            let xl = cx + lvl_rect.width() * 0.5 * level;
            v.painter.rect_filled(Rect::from_min_max(pos2(cx.min(xl), lvl_rect.top()), pos2(cx.max(xl), lvl_rect.bottom())), 2.0 * z, alpha(color, 0.7));
            v.painter.text(lvl_rect.center(), Align2::CENTER_CENTER, format!("{:+.0}%", level * 100.0), theme::mono(7.5 * z), theme::TEXT);
            // Invert, remove.
            if self.chip(&format!("inv{s}"), v.r(x1 - 36.0, y, x1 - 20.0, y + 18.0), "±", false, color, "Invert").clicked() {
                p.set(li, -p.get(li));
            }
            if self.chip(&format!("del{s}"), v.r(x1 - 16.0, y, x1, y + 18.0), "×", false, theme::MUTE, "Remove this slot").clicked() {
                mg::set_slot(p, s, 0, 0);
            }
        }
        let add = self.chip("add-slot", v.r(x0, SIZE.y - PAD - 18.0, x0 + 90.0, SIZE.y - PAD), "+ SLOT", false, self.accent, "A new slot from the shown source");
        if add.clicked() {
            let shown = mg::choice(p, data::SOURCE_SHOWN, TABS.len());
            if let Some(s) = (0..SLOTS).find(|&s| mg::slot_raw(p, s) == (0, 0)) {
                mg::set_slot(p, s, TABS[shown].2[0], 0);
                p.set(mg::SLOT0 + s, 0.5);
            }
        }
    }

    fn target_picker(&self, ui: &mut egui::Ui, s: usize) {
        let p = self.p;
        let (src, _) = mg::slot_raw(p, s);
        let pick = |ui: &mut egui::Ui, i: usize| {
            if ui.button(mg::label(i)).clicked() {
                mg::set_slot(p, s, src, i + 1);
                ui.close();
            }
        };
        for f in 0..FILTERS {
            ui.menu_button(format!("Filter {}", f + 1), |ui| {
                for k in 0..filt::COUNT {
                    pick(ui, fp(f, k));
                }
            });
        }
        ui.menu_button("Global", |ui| {
            for i in [mg::IN_GAIN, mg::IN_PAN, mg::OUT_GAIN, mg::OUT_PAN, mg::MIX] {
                pick(ui, i);
            }
        });
        for x in 0..2 {
            ui.menu_button(format!("XLFO {}", x + 1), |ui| {
                for k in 0..xlfo::COUNT {
                    pick(ui, xp(x, k));
                }
            });
        }
        for e in 0..2 {
            ui.menu_button(format!("EG {}", e + 1), |ui| {
                for k in 0..eg::COUNT {
                    pick(ui, egp(e, k));
                }
            });
        }
        for f in 0..2 {
            ui.menu_button(format!("EF {}", f + 1), |ui| {
                for k in 0..ef::COUNT {
                    pick(ui, efp(f, k));
                }
            });
        }
        ui.menu_button("Slot levels", |ui| {
            for k in 0..SLOTS {
                if k != s && ui.button(format!("Slot {}", k + 1)).clicked() {
                    mg::set_slot(p, s, src, mg::SLOT0 + k + 1);
                    ui.close();
                }
            }
        });
    }
}

/// "F1 Freq", "XLFO1 Rate", "Slot 3".
fn short_target(i: usize) -> String {
    let long = mg::long_name(i);
    long.replace("Filter ", "F").replace(" · ", " ").replace("XLFO ", "XLFO").replace("Global ", "")
}

/// The XLFO's shape at `psi` (0…1), as the processor plays it (random
/// steps at their stored value).
fn xlfo_shape(p: &ModuleParams, x: usize, psi: f32) -> f32 {
    let n = md::steps(p, x);
    let pos = psi * n as f32;
    let i = (pos as usize).min(n - 1);
    let t = pos - i as f32;
    let (here, _, curve, glide) = md::step(p, x, i);
    let glide = (glide + p.get(xp(x, xlfo::GLIDE))).clamp(0.0, 1.0);
    if glide > 0.0 && t > 1.0 - glide {
        let u = (t - (1.0 - glide)) / glide;
        let next = md::step(p, x, (i + 1) % n).0;
        let c = match curve {
            1 => u * u,
            2 => u.sqrt(),
            3 => 0.5 - 0.5 * (u * std::f32::consts::PI).cos(),
            _ => u,
        };
        here + (next - here) * c
    } else {
        here
    }
}

struct Axis {
    rect: Rect,
}

impl Axis {
    fn x(&self, f: f32) -> f32 {
        self.rect.left() + self.rect.width() * (f.max(1.0) / F_LO).ln() / (F_HI / F_LO).ln()
    }

    fn f(&self, x: f32) -> f32 {
        F_LO * (F_HI / F_LO).powf(((x - self.rect.left()) / self.rect.width()).clamp(0.0, 1.0))
    }

    fn y(&self, db: f32) -> f32 {
        self.rect.center().y - (db / DB_SPAN).clamp(-1.2, 1.2) * self.rect.height() * 0.45
    }

    fn db(&self, y: f32) -> f32 {
        (self.rect.center().y - y) / (self.rect.height() * 0.45) * DB_SPAN
    }
}

fn fmt_freq(f: f32) -> String {
    if f >= 1000.0 { format!("{:.2} kHz", f / 1000.0) } else { format!("{f:.0} Hz") }
}

fn fill_to(painter: &egui::Painter, pts: &[Pos2], base: f32, color: Color32) {
    let mut mesh = Mesh::default();
    for (i, pt) in pts.iter().enumerate() {
        mesh.vertices.push(Vertex { pos: *pt, uv: WHITE_UV, color });
        mesh.vertices.push(Vertex { pos: pos2(pt.x, base), uv: WHITE_UV, color });
        if i > 0 {
            let k = (i * 2) as u32;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// The analyzer's state between frames: 1/6-octave bands, peak, slow.
#[derive(Clone, Default)]
struct Rta {
    re: Vec<f32>,
    im: Vec<f32>,
    window: Vec<f32>,
    bands: Vec<(f32, f32)>,
    levels: Vec<f32>,
    last: f64,
}

impl Rta {
    fn update(&mut self, p: &ModuleParams, sr: f32, dt: f32, now: f64) {
        let n = mg::RTA_LEN;
        if self.window.len() != n {
            self.window = (0..n).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (n - 1) as f32).cos()).collect();
            self.re = vec![0.0; n];
            self.im = vec![0.0; n];
            self.bands = (0..60)
                .map(|k| {
                    let c = 20.0 * 2f32.powf(k as f32 / 6.0);
                    let h = 2f32.powf(1.0 / 12.0);
                    (c / h, c * h)
                })
                .filter(|b| b.0 < sr * 0.5)
                .collect();
            self.levels = vec![-96.0; self.bands.len()];
        }
        if now == self.last {
            return;
        }
        self.last = now;
        let write = p.viz[viz::RTA_WRITE].get() as usize;
        for k in 0..n {
            self.re[k] = p.viz[viz::RTA + (write + k) % n].get() * self.window[k];
            self.im[k] = 0.0;
        }
        super::monitor::fft(&mut self.re, &mut self.im);
        let norm = 2.0 / (n as f32 * 0.5);
        let bin_hz = sr / n as f32;
        let k_avg = 1.0 - (-dt / 0.25).exp();
        for (j, &(lo, hi)) in self.bands.iter().enumerate() {
            let (b0, b1) = ((lo / bin_hz).ceil() as usize, ((hi / bin_hz).floor() as usize).min(n / 2 - 1));
            let power = |k: usize| (self.re[k] * self.re[k] + self.im[k] * self.im[k]) * norm * norm;
            let pw = if b1 < b0 { power((((lo * hi).sqrt() / bin_hz).round() as usize).clamp(1, n / 2 - 1)) } else { (b0..=b1).map(power).fold(0.0, f32::max) };
            let db = (10.0 * (pw + 1e-20).log10()).max(-96.0);
            self.levels[j] += (db - self.levels[j]) * k_avg;
        }
    }
}
