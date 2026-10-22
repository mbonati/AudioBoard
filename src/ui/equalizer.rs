//! Panel of the Equalizer node, laid out like Waves F6: the analyzer's
//! toolbar, the graph (band markers, each band's curve, the total curve and
//! the analyzer) with the output meter, the band selectors with MIX and
//! OUT, the selected band's controls, and the global release and arrows.

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Align2, Color32, CursorIcon, Id, PointerButton, Pos2, Rect, Response, Sense, Shape, Stroke, Vec2, pos2, vec2};

use super::nodes::{KnobCell, NodeView, knob_cell, note_name, segmented};
use super::theme::{self, alpha};
use super::widgets::{self, MeterState};
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::equalizer::{self as eq, BANDS, HP, LP, band, bp, data, viz};
use crate::audio::params::gain_to_db;
use crate::graph::{ModuleNode, ParamRef};

pub const SIZE: Vec2 = vec2(760.0, 434.0);
const W: f32 = SIZE.x;
const PAD: f32 = 12.0;
/// The graph, node-local: left, top, right, bottom.
const GRAPH: [f32; 4] = [40.0, 60.0, 704.0, 290.0];
/// The graph's frequency span and its EQ scale (± dB).
const F_LO: f32 = 16.0;
const F_HI: f32 = 21_000.0;
const DB_SPAN: f32 = 18.0;
/// The analyzer's scale: 0 dBFS at the top down to this.
const RTA_FLOOR: f32 = -96.0;
const SELECTOR_Y: f32 = 298.0;
const CONTROLS_Y: f32 = 330.0;
const GLOBAL_Y: f32 = 404.0;
/// Knob columns of the band controls.
const KNOB_X0: f32 = 136.0;
const KNOB_DX: f32 = 72.0;

/// Band colours, then HP and LP.
const COLORS: [Color32; 8] = [
    Color32::from_rgb(232, 112, 82),
    Color32::from_rgb(178, 204, 78),
    Color32::from_rgb(72, 198, 178),
    Color32::from_rgb(78, 164, 236),
    Color32::from_rgb(132, 124, 236),
    Color32::from_rgb(226, 96, 162),
    Color32::from_rgb(196, 200, 208),
    Color32::from_rgb(196, 200, 208),
];

const KNOBS: [(usize, &str); band::COUNT] = [
    (band::FREQ, "FREQ"),
    (band::Q, "Q"),
    (band::GAIN, "GAIN"),
    (band::RANGE, "RANGE"),
    (band::THRESHOLD, "THRESHOLD"),
    (band::ATTACK, "ATTACK"),
    (band::RELEASE, "RELEASE"),
];

struct Panel<'a, 'b> {
    v: &'a NodeView<'b>,
    id: NodeId,
    p: &'a ModuleParams,
    accent: Color32,
}

/// Draws the node's panel. `patched`: IN, SC and OUT connected.
pub fn draw(v: &NodeView, id: NodeId, accent: Color32, m: &ModuleNode, patched: [bool; 3]) {
    let s = Panel { v, id, p: &m.params, accent };
    let col = |on: bool| if on { theme::TEXT_DIM } else { theme::TEXT_FAINT };
    v.text(PAD, 42.0, Align2::LEFT_CENTER, "IN", v.font(9.0), col(patched[0]));
    v.text(PAD, 60.0, Align2::LEFT_CENTER, "SC", v.font(9.0), col(patched[1]));
    v.text(W - PAD, 42.0, Align2::RIGHT_CENTER, "OUT", v.font(9.0), col(patched[2]));
    s.toolbar();
    s.graph();
    s.output_meter();
    s.selectors();
    let selected = eq::choice(s.p, data::SELECTED, LP + 1);
    if selected < BANDS {
        s.band_controls(selected);
    } else {
        s.pass_controls(selected);
    }
    s.global_row();
}

/// Collapsed nodes' readout.
pub fn readout(p: &ModuleParams) -> (String, String) {
    let working = |b: usize| {
        eq::on(p, data::BAND_ON + b) && (p.get(bp(b, band::GAIN)) != 0.0 || p.get(bp(b, band::RANGE)) != 0.0)
    };
    let active = (0..BANDS).filter(|&b| working(b)).count();
    let dynamic = (0..BANDS).filter(|&b| eq::on(p, data::BAND_ON + b) && p.get(bp(b, band::RANGE)) != 0.0).count();
    let mut sub = vec![format!("{dynamic} dynamic")];
    if eq::on(p, data::HP_ON) {
        sub.push(format!("HP {}", fmt_hz(p.get(eq::HP_FREQ))));
    }
    if eq::on(p, data::LP_ON) {
        sub.push(format!("LP {}", fmt_hz(p.get(eq::LP_FREQ))));
    }
    (format!("{active} bands"), sub.join(" · "))
}

fn fmt_hz(f: f32) -> String {
    if f >= 1000.0 { format!("{:.1}k", f / 1000.0) } else { format!("{f:.0}") }
}

/// "BELL" etc. and the long names.
const TYPES: [(&str, &str); 3] = [("LS", "Low shelf"), ("BELL", "Bell"), ("HS", "High shelf")];
const MODES: [(&str, &str); 3] =
    [("ST", "Stereo: both channels"), ("MID", "Mid: the centre (L+R) only"), ("SIDE", "Side: the edges (L−R) only")];

impl Panel<'_, '_> {
    fn z(&self) -> f32 {
        self.v.z
    }

    fn uid(&self, salt: impl std::hash::Hash + std::fmt::Debug) -> Id {
        Id::unique(("equalizer", self.id)).with(salt)
    }

    fn on(&self, i: usize) -> bool {
        eq::on(self.p, i)
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
        let picked = segmented(v, self.uid(("seg", salt)), rect, items.len(), eq::choice(self.p, i, items.len()), color, |k, r, col| {
            v.painter.text(r.center(), Align2::CENTER_CENTER, items[k].0, v.font(8.0), col);
            items[k].1
        });
        if let Some(k) = picked {
            self.p.data[i].set(k as f32);
        }
    }

    /// A setting stepping through `labels`: click the right half for the
    /// next one, the left half for the previous.
    fn cycler(&self, salt: &str, rect: Rect, i: usize, labels: &[&str], tip: &str) {
        let (v, z) = (self.v, self.z());
        let resp = v.ui.interact(rect, self.uid(("cycle", salt)), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip);
        let cur = eq::choice(self.p, i, labels.len());
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

    // -----------------------------------------------------------------------
    // Analyzer toolbar
    // -----------------------------------------------------------------------

    fn toolbar(&self) {
        let v = self.v;
        let (y0, y1) = (34.0, 52.0);
        let neutral = theme::TEXT_DIM;
        self.seg("view", v.r(44.0, y0, 112.0, y1), &[("FILL", "Filled: the analysis as a solid area"), ("BARS", "Bars: one per band (BANDING)")], data::RTA_BARS, neutral);
        self.seg("response", v.r(116.0, y0, 180.0, y1), &[("PEAK", "Peak: each band shows its loudest frequency"), ("RMS", "RMS: each band shows its average power")], data::RTA_RMS, neutral);
        self.seg("slope", v.r(184.0, y0, 264.0, y1), &[("PINK", "Pink slope: equal energy per octave (pink noise reads flat)"), ("WHITE", "White slope: equal energy per frequency")], data::RTA_WHITE, neutral);
        let bandings: Vec<&str> = eq::BANDINGS.iter().map(|b| b.0).collect();
        self.cycler("banding", v.r(268.0, y0, 324.0, y1), data::RTA_BANDING, &bandings, "Banding: the analyzer's resolution, in bands per octave");
        let averaging: Vec<&str> = eq::AVERAGING.iter().map(|a| a.0).collect();
        self.cycler("averaging", v.r(328.0, y0, 396.0, y1), data::RTA_AVERAGING, &averaging, "Averaging: how fast the analyzer follows the sound");
        self.switch(data::RTA_MA, v.r(400.0, y0, 430.0, y1), "MA", "A line for the moving average");
        self.switch(data::RTA_MAX, v.r(434.0, y0, 468.0, y1), "MAX", "A line for the peaks");
        self.cycler("source", v.r(472.0, y0, 528.0, y1), data::RTA_SOURCE, &eq::RTA_SOURCES, "Source: left, right, L+R (mid) or sides (L−R)");
        self.cycler("point", v.r(532.0, y0, 584.0, y1), data::RTA_POINT, &eq::RTA_POINTS, "What the analyzer listens to: before the EQ, after it, the SC input, or nothing");
        self.switch(data::RTA_ON, v.r(588.0, y0, 624.0, y1), "RTA", "Analyzer on or off");
    }

    // -----------------------------------------------------------------------
    // Graph
    // -----------------------------------------------------------------------

    fn graph(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let rect = v.r(GRAPH[0], GRAPH[1], GRAPH[2], GRAPH[3]);
        let painter = v.painter.with_clip_rect(rect.intersect(v.painter.clip_rect()));
        painter.rect_filled(rect, 4.0 * z, theme::INSET);
        let axis = Axis { rect };
        let sr = p.viz[viz::SR].get().max(8_000.0);

        // Grid.
        let font = theme::font(7.5 * z);
        for (f, label) in [(20.0, "20"), (50.0, "50"), (100.0, "100"), (200.0, "200"), (500.0, "500"), (1e3, "1k"), (2e3, "2k"), (5e3, "5k"), (1e4, "10k"), (2e4, "20k")] {
            let x = axis.x(f);
            painter.vline(x, rect.y_range(), Stroke::new(1.0 * z, theme::GRID_MAJOR));
            painter.text(pos2(x + 2.0 * z, rect.bottom() - 2.0 * z), Align2::LEFT_BOTTOM, label, font.clone(), theme::TEXT_FAINT);
        }
        for db in [-18.0, -12.0, -6.0, 0.0, 6.0, 12.0, 18.0f32] {
            let y = axis.y(db);
            let col = if db == 0.0 { alpha(theme::TEXT_FAINT, 0.6) } else { theme::GRID_MAJOR };
            painter.hline(rect.x_range(), y, Stroke::new(1.0 * z, col));
            if db != 18.0 && db != -18.0 {
                let label = if db == 0.0 { "0".to_owned() } else { format!("{db:+.0}") };
                painter.text(pos2(rect.left() + 3.0 * z, y - 1.0 * z), Align2::LEFT_BOTTOM, label, font.clone(), theme::TEXT_FAINT);
            }
        }

        let rta = self.analyzer(&painter, &axis);

        // Curves: each band's (live: gain plus dynamics), and the total.
        let xs: Vec<f32> = (0..=200).map(|k| rect.left() + rect.width() * k as f32 / 200.0).collect();
        let freqs: Vec<f32> = xs.iter().map(|&x| axis.f(x)).collect();
        let mut total = vec![0.0f32; xs.len()];
        for (b, &color) in COLORS.iter().enumerate().take(BANDS) {
            if !self.on(data::BAND_ON + b) {
                continue;
            }
            let (kind, f0, q) = (eq::choice(p, data::BAND_TYPE + b, 3), p.get(bp(b, band::FREQ)), p.get(bp(b, band::Q)));
            let gain = p.get(bp(b, band::GAIN));
            let live = gain + p.viz[viz::DYN + b].get();
            let range = p.get(bp(b, band::RANGE));
            if gain == 0.0 && live.abs() < 0.01 && range == 0.0 {
                continue;
            }
            let curve: Vec<f32> = freqs.iter().map(|&f| eq::band_db(kind, f0, q, live, f, sr)).collect();
            for (t, c) in total.iter_mut().zip(&curve) {
                *t += c;
            }
            let pts: Vec<Pos2> = xs.iter().zip(&curve).map(|(&x, &db)| pos2(x, axis.y(db))).collect();
            fill_to(&painter, &pts, axis.y(0.0), alpha(color, 0.16));
            painter.add(Shape::line(pts, Stroke::new(1.0 * z, alpha(color, 0.7))));
            if range != 0.0 {
                // How far the dynamics can take it.
                let reach: Vec<Pos2> =
                    xs.iter().zip(&freqs).map(|(&x, &f)| pos2(x, axis.y(eq::band_db(kind, f0, q, gain + range, f, sr)))).collect();
                dashed(&painter, &reach, Stroke::new(1.0 * z, alpha(color, 0.45)));
            }
        }
        for (high, on, freq, slope) in [
            (true, data::HP_ON, eq::HP_FREQ, data::HP_SLOPE),
            (false, data::LP_ON, eq::LP_FREQ, data::LP_SLOPE),
        ] {
            if self.on(on) {
                let (fc, slope) = (p.get(freq), eq::SLOPES[eq::choice(p, slope, 4)]);
                for (t, &f) in total.iter_mut().zip(&freqs) {
                    *t += eq::pass_db(high, fc, slope, f, sr);
                }
            }
        }
        let total_pts: Vec<Pos2> = xs.iter().zip(&total).map(|(&x, &db)| pos2(x, axis.y(db))).collect();
        painter.add(Shape::line(total_pts, Stroke::new(2.0 * z, alpha(theme::TEXT, 0.9))));

        // Markers.
        let markers = self.markers(&axis);
        let selected = eq::choice(p, data::SELECTED, LP + 1);
        for &(k, c) in &markers {
            let on = match k {
                HP => self.on(data::HP_ON),
                LP => self.on(data::LP_ON),
                _ => self.on(data::BAND_ON + k),
            };
            let color = if on { COLORS[k] } else { theme::TEXT_FAINT };
            if k < BANDS && on {
                // The dynamics, live: from the set gain to where they take it.
                let d = p.viz[viz::DYN + k].get();
                if d.abs() > 0.05 {
                    let to = pos2(c.x, axis.y(p.get(bp(k, band::GAIN)) + d));
                    painter.line_segment([c, to], Stroke::new(3.0 * z, alpha(color, 0.8)));
                }
            }
            let r = 8.0 * z;
            painter.circle_filled(c, r, if k == selected { color } else { theme::INSET });
            painter.circle_stroke(c, r, Stroke::new(1.6 * z, color));
            let label = match k {
                HP => "HP".to_owned(),
                LP => "LP".to_owned(),
                _ => (k + 1).to_string(),
            };
            let text_col = if k == selected { theme::INSET } else { color };
            painter.text(c, Align2::CENTER_CENTER, label, theme::font(if k < BANDS { 9.0 } else { 7.0 } * z), text_col);
            let mode = match k {
                HP => eq::choice(p, data::HP_MODE, 3),
                LP => eq::choice(p, data::LP_MODE, 3),
                _ => eq::choice(p, data::BAND_MODE + k, 3),
            };
            if mode != eq::STEREO {
                let letter = if mode == eq::MID { "M" } else { "S" };
                painter.text(c + vec2(r, r * 0.6), Align2::LEFT_CENTER, letter, theme::font(8.0 * z), color);
            }
        }

        let resp = v.ui.interact(rect, self.uid("graph"), Sense::click_and_drag());
        self.graph_input(&resp, &axis, &markers);

        // Where the mouse is: frequency, note, and the analyzer's level.
        if let Some(q) = resp.hover_pos() {
            let f = axis.f(q.x);
            let mut text = format!("{} ({})", fmt_freq(f), note_name(f));
            if let Some(levels) = &rta {
                let near = levels.iter().min_by(|a, b| (a.0 / f).ln().abs().total_cmp(&(b.0 / f).ln().abs()));
                if let Some(&(_, db)) = near {
                    text.push_str(&format!("   RTA {db:.1} dB"));
                }
            }
            painter.text(rect.right_top() + vec2(-6.0, 4.0) * z, Align2::RIGHT_TOP, text, theme::mono(8.0 * z), theme::TEXT_DIM);
        }
    }

    /// Marker positions: bands at (frequency, gain), HP and LP on 0 dB.
    fn markers(&self, axis: &Axis) -> Vec<(usize, Pos2)> {
        let p = self.p;
        let mut out: Vec<(usize, Pos2)> = (0..BANDS)
            .map(|b| (b, pos2(axis.x(p.get(bp(b, band::FREQ))), axis.y(p.get(bp(b, band::GAIN))))))
            .collect();
        out.push((HP, pos2(axis.x(p.get(eq::HP_FREQ)), axis.y(0.0))));
        out.push((LP, pos2(axis.x(p.get(eq::LP_FREQ)), axis.y(0.0))));
        out
    }

    /// Dragging markers (Alt: Q; Ctrl: one axis only), Cmd-click for the
    /// band type, double-click on/off, right-click and hold to solo (moving
    /// it solos other frequencies).
    fn graph_input(&self, resp: &Response, axis: &Axis, markers: &[(usize, Pos2)]) {
        let (v, p, z) = (self.v, self.p, self.z());
        let hit = |q: Pos2| {
            markers
                .iter()
                .rev()
                .map(|&(k, c)| (k, c.distance(q)))
                .filter(|&(_, d)| d < 11.0 * z)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(k, _)| k)
        };
        let freq_param = |k: usize| match k {
            HP => eq::HP_FREQ,
            LP => eq::LP_FREQ,
            _ => bp(k, band::FREQ),
        };
        let modifiers = v.ui.input(|i| i.modifiers);
        let drag_id = self.uid("marker-drag");

        if resp.drag_started_by(PointerButton::Primary)
            && let Some(origin) = v.ui.input(|i| i.pointer.press_origin())
        {
            let grab = hit(origin).map(|k| {
                let gain = if k < BANDS { p.get(bp(k, band::GAIN)) } else { 0.0 };
                let q = if k < BANDS { p.get(bp(k, band::Q)) } else { 1.0 };
                p.data[data::SELECTED].set(k as f32);
                (k, p.get(freq_param(k)), gain, q, origin)
            });
            v.ui.data_mut(|d| d.insert_temp(drag_id, grab));
        }
        if resp.dragged_by(PointerButton::Primary)
            && let Some(Some((k, f0, g0, q0, origin))) = v.ui.data(|d| d.get_temp::<Option<(usize, f32, f32, f32, Pos2)>>(drag_id))
            && let Some(q) = resp.interact_pointer_pos()
        {
            let mut d = q - origin;
            if modifiers.alt && k < BANDS {
                // Alt: sideways sets the width.
                p.set(bp(k, band::Q), (q0 * (d.x / (60.0 * z)).exp()).clamp(0.4, 60.0));
            } else {
                if modifiers.ctrl {
                    if d.x.abs() > d.y.abs() {
                        d.y = 0.0;
                    } else {
                        d.x = 0.0;
                    }
                }
                let f = axis.f(axis.x(f0) + d.x).clamp(eq::FREQ_RANGE.0, eq::FREQ_RANGE.1);
                p.set(freq_param(k), f);
                if k < BANDS {
                    p.set(bp(k, band::GAIN), axis.db(axis.y(g0) + d.y).clamp(-DB_SPAN, DB_SPAN));
                }
            }
        }

        if resp.clicked()
            && let Some(k) = resp.interact_pointer_pos().and_then(hit)
        {
            p.data[data::SELECTED].set(k as f32);
            if modifiers.command && k < BANDS {
                let t = data::BAND_TYPE + k;
                p.data[t].set(((eq::choice(p, t, 3) + 1) % 3) as f32);
            }
        }
        if resp.double_clicked()
            && let Some(k) = resp.interact_pointer_pos().and_then(hit)
        {
            self.flip(match k {
                HP => data::HP_ON,
                LP => data::LP_ON,
                _ => data::BAND_ON + k,
            });
        }

        // Right-click and hold: solo.
        let solo_id = self.uid("solo-hold");
        let (pressed, down) = v.ui.input(|i| (i.pointer.button_pressed(PointerButton::Secondary), i.pointer.button_down(PointerButton::Secondary)));
        if pressed
            && let Some(k) = resp.hover_pos().and_then(hit)
        {
            p.data[data::SOLO].set(k as f32);
            p.data[data::SELECTED].set(k as f32);
            v.ui.data_mut(|d| d.insert_temp(solo_id, k));
        }
        if let Some(k) = v.ui.data(|d| d.get_temp::<usize>(solo_id)) {
            if down {
                if resp.dragged_by(PointerButton::Secondary)
                    && let Some(q) = resp.interact_pointer_pos()
                {
                    p.set(freq_param(k), axis.f(q.x).clamp(eq::FREQ_RANGE.0, eq::FREQ_RANGE.1));
                }
            } else {
                p.data[data::SOLO].set(-1.0);
                v.ui.data_mut(|d| d.remove::<usize>(solo_id));
            }
        }
        if resp.hover_pos().and_then(hit).is_some() {
            v.ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }
    }

    /// The real-time analyzer behind the curves; returns its bands
    /// (centre frequency, dB) for the readout.
    fn analyzer(&self, painter: &egui::Painter, axis: &Axis) -> Option<Vec<(f32, f32)>> {
        let (v, p, z) = (self.v, self.p, self.z());
        if !self.on(data::RTA_ON) || eq::choice(p, data::RTA_POINT, 4) == 3 {
            return None;
        }
        let sr = p.viz[viz::SR].get().max(8_000.0);
        let per_octave = eq::BANDINGS[eq::choice(p, data::RTA_BANDING, eq::BANDINGS.len())].1;
        let tau = eq::AVERAGING[eq::choice(p, data::RTA_AVERAGING, eq::AVERAGING.len())].1;
        let (rms, white) = (self.on(data::RTA_RMS), self.on(data::RTA_WHITE));
        let (now, dt) = v.ui.input(|i| (i.time, i.stable_dt.min(0.1)));
        let state_id = self.uid("rta");
        let bands: Vec<(f32, f32, f32, f32, f32)> = v.ui.data_mut(|d| {
            let st = d.get_temp_mut_or_insert_with(state_id, Rta::default);
            st.update(p, sr, per_octave, rms, white, tau, dt, now);
            st.bands.iter().zip(&st.levels).zip(st.ma.iter().zip(&st.max)).map(|((b, l), (m, x))| (b.0, b.1, *l, *m, *x)).collect()
        });
        if bands.is_empty() {
            return None;
        }
        let y_of = |db: f32| {
            let r = axis.rect;
            r.top() + r.height() * (db / RTA_FLOOR).clamp(0.0, 1.0)
        };
        let fill = alpha(Color32::from_rgb(104, 150, 164), 0.35);
        if self.on(data::RTA_BARS) {
            for &(lo, hi, level, ..) in &bands {
                let (x0, x1) = (axis.x(lo), axis.x(hi));
                let r = Rect::from_min_max(pos2(x0 + 0.5 * z, y_of(level)), pos2((x1 - 0.5 * z).max(x0 + 0.5 * z), axis.rect.bottom()));
                painter.rect_filled(r, 0.0, fill);
            }
        } else {
            let pts: Vec<Pos2> = bands.iter().map(|&(lo, hi, level, ..)| pos2(axis.x((lo * hi).sqrt()), y_of(level))).collect();
            fill_to(painter, &pts, axis.rect.bottom(), fill);
        }
        let line = |pick: fn(&(f32, f32, f32, f32, f32)) -> f32, color: Color32| {
            let pts: Vec<Pos2> = bands.iter().map(|b| pos2(axis.x((b.0 * b.1).sqrt()), y_of(pick(b)))).collect();
            painter.add(Shape::line(pts, Stroke::new(1.0 * z, color)));
        };
        if self.on(data::RTA_MA) {
            line(|b| b.3, alpha(theme::TEXT_DIM, 0.7));
        }
        if self.on(data::RTA_MAX) {
            line(|b| b.4, alpha(theme::METER_YELLOW, 0.7));
        }
        if eq::on(p, data::RTA_ON) {
            v.ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        }
        Some(bands.iter().map(|b| ((b.0 * b.1).sqrt(), b.2)).collect())
    }

    /// The output meter, right of the graph: −40…0 dBFS.
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
        let norm = |lin: f32| ((gain_to_db(lin) + 40.0) / 40.0).clamp(0.0, 1.0);
        let (x0, x1) = (GRAPH[2] + 8.0, W - PAD);
        let half = (x1 - x0 - 2.0) / 2.0;
        for (ch, state) in states.iter().enumerate() {
            let left = x0 + ch as f32 * (half + 2.0);
            let r = v.r(left, GRAPH[1], left + half, GRAPH[3]);
            v.painter.rect_filled(r, 2.0 * z, theme::INSET);
            let level = norm(state.level);
            let db = gain_to_db(state.level);
            let col = if db > -1.0 {
                theme::METER_RED
            } else if db > -9.0 {
                theme::METER_YELLOW
            } else {
                theme::METER_GREEN
            };
            if level > 0.0 {
                v.painter.rect_filled(Rect::from_min_max(pos2(r.left(), r.bottom() - r.height() * level), r.max), 2.0 * z, col);
            }
            let py = r.bottom() - r.height() * norm(state.peak);
            v.painter.hline(r.x_range(), py, Stroke::new(1.0 * z, theme::TEXT_DIM));
        }
    }

    // -----------------------------------------------------------------------
    // Band selectors, MIX and OUT
    // -----------------------------------------------------------------------

    fn selectors(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let selected = eq::choice(p, data::SELECTED, LP + 1);
        let order = [HP, 0, 1, 2, 3, 4, 5, LP];
        for (slot, &k) in order.iter().enumerate() {
            let x = GRAPH[0] + slot as f32 * 46.0;
            let rect = v.r(x, SELECTOR_Y, x + 42.0, SELECTOR_Y + 20.0);
            let on = match k {
                HP => self.on(data::HP_ON),
                LP => self.on(data::LP_ON),
                _ => self.on(data::BAND_ON + k),
            };
            let resp = v
                .ui
                .interact(rect, self.uid(("select", k)), Sense::CLICK)
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text("Click to select; double-click to switch on or off");
            let color = COLORS[k];
            if k == selected {
                v.painter.rect_filled(rect, 3.0 * z, if on { color } else { theme::TEXT_FAINT });
            } else {
                v.painter.rect_filled(rect, 3.0 * z, if resp.hovered() { theme::CONTROL_HI } else { theme::CONTROL });
                v.painter.hline(rect.shrink(4.0 * z).x_range(), rect.bottom() - 2.0 * z, Stroke::new(2.0 * z, alpha(color, if on { 0.9 } else { 0.25 })));
            }
            let label = match k {
                HP => "HP".to_owned(),
                LP => "LP".to_owned(),
                _ => (k + 1).to_string(),
            };
            let text = if k == selected { theme::INSET } else if on { theme::TEXT } else { theme::TEXT_FAINT };
            v.painter.text(rect.center(), Align2::CENTER_CENTER, label, theme::font(9.0 * z), text);
            if resp.double_clicked() {
                self.flip(match k {
                    HP => data::HP_ON,
                    LP => data::LP_ON,
                    _ => data::BAND_ON + k,
                });
            } else if resp.clicked() {
                p.data[data::SELECTED].set(k as f32);
            }
        }
        self.number_box(eq::MIX, v.r(470.0, SELECTOR_Y, 590.0, SELECTOR_Y + 20.0), "MIX", 0.01, "Mix: dry (0) to fully processed (100), for parallel processing");
        self.number_box(eq::OUT, v.r(628.0, SELECTOR_Y, W - PAD, SELECTOR_Y + 20.0), "OUT", 0.1, "Output gain");
    }

    /// "MIX [−] 100 [+]": drag the value, or click − and +; double-click
    /// resets.
    fn number_box(&self, i: usize, rect: Rect, label: &str, step: f32, tip: &str) {
        let (v, p, z) = (self.v, self.p, self.z());
        let spec = eq::spec(i);
        v.painter.text(rect.left_center() - vec2(6.0 * z, 0.0), Align2::RIGHT_CENTER, label, theme::font(8.5 * z), theme::TEXT_FAINT);
        let bw = 18.0 * z;
        let minus = Rect::from_min_size(rect.min, vec2(bw, rect.height()));
        let plus = Rect::from_min_max(pos2(rect.right() - bw, rect.top()), rect.max);
        let mid = Rect::from_min_max(pos2(minus.right() + 2.0 * z, rect.top()), pos2(plus.left() - 2.0 * z, rect.bottom()));
        let mut value = p.get(i);
        for (r, sign, glyph) in [(minus, -1.0, "−"), (plus, 1.0, "+")] {
            let resp = v.ui.interact(r, self.uid(("step", i, glyph)), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand);
            v.painter.rect_filled(r, 3.0 * z, if resp.hovered() { theme::CONTROL_HI } else { theme::CONTROL });
            v.painter.text(r.center(), Align2::CENTER_CENTER, glyph, theme::font(10.0 * z), theme::TEXT_DIM);
            if resp.clicked() {
                value += sign * step;
            }
        }
        let resp = v.ui.interact(mid, self.uid(("value", i)), Sense::click_and_drag()).on_hover_cursor(CursorIcon::ResizeHorizontal).on_hover_text(tip);
        v.hit(mid, ParamRef::Module(i));
        if resp.dragged() {
            let d = resp.drag_delta();
            value = spec.denorm(spec.to_norm(value) + (d.x - d.y) * 0.004);
        }
        if resp.double_clicked() {
            value = spec.default;
        }
        let value = value.clamp(spec.min, spec.max);
        if value != p.get(i) {
            p.set(i, value);
        }
        v.painter.rect_filled(mid, 3.0 * z, theme::INSET);
        let text = if i == eq::MIX { format!("{:.0}", value * 100.0) } else { format!("{value:+.1}") };
        v.painter.text(mid.center(), Align2::CENTER_CENTER, text, theme::mono(9.0 * z), theme::TEXT);
    }

    // -----------------------------------------------------------------------
    // Band controls
    // -----------------------------------------------------------------------

    /// Power, then stereo/mid/side, for band or filter `k`.
    fn power_and_mode(&self, k: usize, on_i: usize, mode_i: usize) {
        let v = self.v;
        let color = COLORS[k];
        let on = self.on(on_i);
        if self.chip("power", v.r(PAD, CONTROLS_Y - 2.0, 92.0, CONTROLS_Y + 16.0), if on { "⏻ ON" } else { "⏻ OFF" }, on, color, "Switch on or off").clicked() {
            self.flip(on_i);
        }
        let mode = eq::choice(self.p, mode_i, 3);
        for (m, (label, tip)) in MODES.iter().enumerate() {
            let y = CONTROLS_Y + 22.0 + m as f32 * 18.0;
            if self.chip(&format!("mode{m}"), v.r(PAD, y, 48.0, y + 16.0), label, mode == m, color, tip).clicked() {
                self.p.data[mode_i].set(m as f32);
            }
        }
    }

    fn band_controls(&self, b: usize) {
        let (v, p, z) = (self.v, self.p, self.z());
        let color = COLORS[b];
        self.power_and_mode(b, data::BAND_ON + b, data::BAND_MODE + b);
        let kind = eq::choice(p, data::BAND_TYPE + b, 3);
        for (t, (label, tip)) in TYPES.iter().enumerate() {
            let y = CONTROLS_Y + 22.0 + t as f32 * 18.0;
            if self.chip(&format!("type{t}"), v.r(52.0, y, 92.0, y + 16.0), label, kind == t, color, tip).clicked() {
                p.data[data::BAND_TYPE + b].set(t as f32);
            }
        }
        let dimmed = !self.on(data::BAND_ON + b);
        for (col, &(k, label)) in KNOBS.iter().enumerate() {
            let i = bp(b, k);
            let cx = KNOB_X0 + KNOB_DX * col as f32;
            let cell = KnobCell { cx, row_y: CONTROLS_Y, label, dimmed, accent: color, param: ParamRef::Module(i) };
            if let Some(value) = knob_cell(v, self.uid(("knob", i)), eq::spec(i), p.get(i), cell) {
                p.set(i, value);
            }
            if k == band::THRESHOLD {
                // The side chain's level around the threshold knob.
                let level = p.viz[viz::SC_LEVEL + b].get();
                let n = ((level + 60.0) / 60.0).clamp(0.0, 1.0);
                let c = v.p(cx, CONTROLS_Y + 19.0);
                let r = 16.5 * z;
                if n > 0.0 {
                    let a0 = widgets::KNOB_START;
                    let pts = widgets::arc(c, r, a0, a0 + widgets::KNOB_SWEEP * n);
                    v.painter.add(Shape::line(pts, Stroke::new(2.0 * z, alpha(color, 0.75))));
                }
            }
        }

        // Solo and side chain.
        let x0 = 612.0;
        let soloed = eq::solo(p) == Some(b);
        if self.chip("solo", v.r(x0, CONTROLS_Y - 2.0, W - PAD, CONTROLS_Y + 16.0), "SOLO", soloed, color, "Hear this band's side chain (also: right-click and hold its marker)").clicked() {
            p.data[data::SOLO].set(if soloed { -1.0 } else { b as f32 });
        }
        v.text(x0, CONTROLS_Y + 27.0, Align2::LEFT_CENTER, "SC SOURCE", v.font(7.5), theme::TEXT_FAINT);
        self.seg("source", v.r(x0 + 52.0, CONTROLS_Y + 19.0, W - PAD, CONTROLS_Y + 35.0), &[("INT", "Internal: the node's own input"), ("EXT", "External: the SC input")], data::SC_EXT + b, color);
        v.text(x0, CONTROLS_Y + 49.0, Align2::LEFT_CENTER, "SC MODE", v.font(7.5), theme::TEXT_FAINT);
        self.seg("scmode", v.r(x0 + 52.0, CONTROLS_Y + 41.0, W - PAD, CONTROLS_Y + 57.0), &[("SPLIT", "Split: only the band's frequencies drive its dynamics"), ("WIDE", "Wide: the full range drives its dynamics")], data::SC_WIDE + b, color);
        let level = p.viz[viz::SC_LEVEL + b].get();
        let text = if level > -100.0 { format!("SC {level:.1} dB") } else { "SC —".to_owned() };
        v.text(x0, CONTROLS_Y + 66.0, Align2::LEFT_CENTER, text, v.mono(8.0), theme::TEXT_FAINT);
    }

    fn pass_controls(&self, k: usize) {
        let (v, p) = (self.v, self.p);
        let color = COLORS[k];
        let (on_i, mode_i, slope_i, freq_i) = if k == HP {
            (data::HP_ON, data::HP_MODE, data::HP_SLOPE, eq::HP_FREQ)
        } else {
            (data::LP_ON, data::LP_MODE, data::LP_SLOPE, eq::LP_FREQ)
        };
        self.power_and_mode(k, on_i, mode_i);
        let cell = KnobCell { cx: KNOB_X0, row_y: CONTROLS_Y, label: "FREQ", dimmed: !self.on(on_i), accent: color, param: ParamRef::Module(freq_i) };
        if let Some(value) = knob_cell(v, self.uid(("knob", freq_i)), eq::spec(freq_i), p.get(freq_i), cell) {
            p.set(freq_i, value);
        }
        v.text(KNOB_X0 + 60.0, CONTROLS_Y, Align2::LEFT_CENTER, "SLOPE (dB/OCT)", v.font(8.5), theme::TEXT_FAINT);
        let slopes = [("6", "6 dB/octave"), ("12", "12 dB/octave"), ("18", "18 dB/octave"), ("24", "24 dB/octave")];
        self.seg("slope", v.r(KNOB_X0 + 60.0, CONTROLS_Y + 10.0, KNOB_X0 + 220.0, CONTROLS_Y + 28.0), &slopes, slope_i, color);
        let name = if k == HP { "High-pass" } else { "Low-pass" };
        v.text(KNOB_X0 + 60.0, CONTROLS_Y + 44.0, Align2::LEFT_CENTER, format!("{name}: only frequency, slope and channels"), v.font(8.5), theme::TEXT_FAINT);
        let soloed = eq::solo(p) == Some(k);
        if self.chip("solo", v.r(612.0, CONTROLS_Y - 2.0, W - PAD, CONTROLS_Y + 16.0), "SOLO", soloed, color, "Hear what this filter lets through").clicked() {
            p.data[data::SOLO].set(if soloed { -1.0 } else { k as f32 });
        }
    }

    /// Global release, and the arrows moving a control on every band.
    fn global_row(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        v.text(PAD, GLOBAL_Y + 9.0, Align2::LEFT_CENTER, "GLOBAL RELEASE", v.font(8.0), theme::TEXT_FAINT);
        self.seg("arc", v.r(PAD + 76.0, GLOBAL_Y, PAD + 156.0, GLOBAL_Y + 18.0), &[("MNL", "Manual: each band's RELEASE"), ("ARC", "Automatic release control: shorter on transients, never longer than RELEASE")], data::ARC, self.accent);
        for (col, &(k, label)) in KNOBS.iter().enumerate() {
            if !matches!(k, band::GAIN | band::RANGE | band::THRESHOLD | band::ATTACK | band::RELEASE) {
                continue;
            }
            let cx = KNOB_X0 + KNOB_DX * col as f32;
            let rect = v.r(cx - 20.0, GLOBAL_Y, cx + 20.0, GLOBAL_Y + 18.0);
            let id = self.uid(("arrows", k));
            let resp = v
                .ui
                .interact(rect, id, Sense::click_and_drag())
                .on_hover_cursor(CursorIcon::ResizeVertical)
                .on_hover_text(format!("{label} of every band: drag up or down (or click ▲ ▼)"));
            let mut delta = 0.0;
            if resp.dragged() {
                delta = -resp.drag_delta().y * 0.003;
            }
            if resp.clicked() {
                let up = resp.interact_pointer_pos().is_some_and(|q| q.y < rect.center().y);
                delta = if up { 0.01 } else { -0.01 };
            }
            if delta != 0.0 {
                for b in 0..BANDS {
                    let i = bp(b, k);
                    let spec = eq::spec(i);
                    p.set(i, spec.denorm(spec.to_norm(p.get(i)) + delta));
                }
            }
            let hot = resp.hovered() || resp.dragged();
            v.painter.rect_filled(rect, 3.0 * z, if hot { theme::CONTROL_HI } else { theme::CONTROL });
            let font = theme::font(7.5 * z);
            v.painter.text(rect.center() - vec2(0.0, 4.0 * z), Align2::CENTER_CENTER, "▲", font.clone(), theme::TEXT_DIM);
            v.painter.text(rect.center() + vec2(0.0, 4.0 * z), Align2::CENTER_CENTER, "▼", font, theme::TEXT_DIM);
        }
    }
}

/// The graph's scales.
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
        self.rect.center().y - (db / DB_SPAN).clamp(-1.1, 1.1) * self.rect.height() * 0.45
    }

    fn db(&self, y: f32) -> f32 {
        (self.rect.center().y - y) / (self.rect.height() * 0.45) * DB_SPAN
    }
}

fn fmt_freq(f: f32) -> String {
    if f >= 1000.0 { format!("{:.2} kHz", f / 1000.0) } else { format!("{f:.0} Hz") }
}

/// A soft fill between a curve and a horizontal line.
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

fn dashed(painter: &egui::Painter, pts: &[Pos2], stroke: Stroke) {
    for pair in pts.chunks(2) {
        if let [a, b] = pair {
            painter.line_segment([*a, *b], stroke);
        }
    }
}

/// The analyzer's state, kept by the panel between frames.
#[derive(Clone, Default)]
struct Rta {
    re: Vec<f32>,
    im: Vec<f32>,
    window: Vec<f32>,
    /// Bands: (low, high) edges, and their smoothed level, moving average
    /// and held peak, in dB.
    bands: Vec<(f32, f32)>,
    levels: Vec<f32>,
    ma: Vec<f32>,
    max: Vec<f32>,
    per_octave: f32,
    last: f64,
}

impl Rta {
    #[allow(clippy::too_many_arguments)]
    fn update(&mut self, p: &ModuleParams, sr: f32, per_octave: f32, rms: bool, white: bool, tau: f32, dt: f32, now: f64) {
        let n = eq::RTA_LEN;
        if self.window.len() != n {
            self.window = (0..n).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (n - 1) as f32).cos()).collect();
            self.re = vec![0.0; n];
            self.im = vec![0.0; n];
        }
        if self.per_octave != per_octave || self.bands.is_empty() {
            self.per_octave = per_octave;
            let count = (1000f32.log2() * per_octave).ceil() as usize + 1;
            self.bands = (0..count)
                .map(|k| {
                    let c = 20.0 * 2f32.powf(k as f32 / per_octave);
                    let h = 2f32.powf(0.5 / per_octave);
                    (c / h, c * h)
                })
                .filter(|b| b.0 < sr * 0.5)
                .collect();
            self.levels = vec![RTA_FLOOR; self.bands.len()];
            self.ma = self.levels.clone();
            self.max = self.levels.clone();
        }
        // Once per frame is enough.
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
        // Hann's coherent gain is 0.5: a full-scale sine reads 0 dB.
        let norm = 2.0 / (n as f32 * 0.5);
        let bin_hz = sr / n as f32;
        let power = |k: usize| (self.re[k] * self.re[k] + self.im[k] * self.im[k]) * norm * norm;
        let k_avg = 1.0 - (-dt / tau).exp();
        let k_ma = 1.0 - (-dt / 3.0).exp();
        for (j, &(lo, hi)) in self.bands.iter().enumerate() {
            let (b0, b1) = ((lo / bin_hz).ceil() as usize, ((hi / bin_hz).floor() as usize).min(n / 2 - 1));
            let pw = if b1 < b0 {
                // Narrower than a bin: the nearest one.
                power((((lo * hi).sqrt() / bin_hz).round() as usize).clamp(1, n / 2 - 1))
            } else if rms {
                (b0..=b1).map(power).sum::<f32>() / (b1 + 1 - b0) as f32
            } else {
                (b0..=b1).map(power).fold(0.0, f32::max)
            };
            let mut db = 10.0 * (pw + 1e-20).log10();
            if !white {
                // Pink: +3 dB per octave around 1 kHz, so pink noise reads flat.
                db += 10.0 * ((lo * hi).sqrt() / 1000.0).log10();
            }
            let db = db.max(RTA_FLOOR);
            self.levels[j] += (db - self.levels[j]) * k_avg;
            self.ma[j] += (db - self.ma[j]) * k_ma;
            self.max[j] = if db > self.max[j] { db } else { self.max[j] - 3.0 * dt };
        }
    }
}
