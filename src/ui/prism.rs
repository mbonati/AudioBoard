//! Panel of the Prism node, laid out after Pro-Q 4 and drawn flat: the
//! analyzer's bar, the interactive display (spectra, each band's curve as a
//! coloured outline, the total curve in yellow, outlined band dots, the
//! dynamics' reach and live position), the selected band's strip, and the
//! bottom bar (processing mode, resolution, character, EQ Match, Sketch,
//! auto gain, gain scale, pan, output).

use egui::ecolor::Hsva;
use egui::{Align2, Color32, CursorIcon, Id, PointerButton, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};

use super::nodes::{KnobCell, NodeView, knob_cell, note_name, segmented};
use super::theme::{self, alpha};
use super::widgets::{self, MeterState};
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::prism::design::{
    self, ALL_PASS, Analog, BAND_PASS, BELL, BRICKWALL, FLAT_TILT, HIGH_CUT, HIGH_SHELF, LOW_CUT, LOW_SHELF, NOTCH, SHAPES, SLOPES, Shape as Band, TILT_SHELF,
};
use crate::audio::modules::prism::{self as pq, BANDS, band, bp, data, fit, viz};
use crate::audio::params::gain_to_db;
use crate::graph::{ModuleNode, ParamRef};

pub const SIZE: Vec2 = vec2(920.0, 556.0);
const W: f32 = SIZE.x;
const PAD: f32 = 12.0;
/// The display, node-local: left, top, right, bottom.
const GRAPH: [f32; 4] = [40.0, 56.0, 868.0, 380.0];
const F_LO: f32 = 10.0;
const F_HI: f32 = 30_000.0;
const STRIP_Y: f32 = 388.0;
const KNOBS_Y: f32 = 418.0;
const BAR_Y: f32 = 494.0;
const BAR2_Y: f32 = 522.0;
/// Points across the display for curves and spectra.
const COLS: usize = 240;
/// Pro-Q's yellow for the total curve.
const YELLOW: Color32 = Color32::from_rgb(236, 206, 84);
/// Colours of the per-channel totals: left, right, mid, side.
const PLACE_COLORS: [Color32; 5] = [
    YELLOW,
    Color32::from_rgb(232, 232, 236),
    Color32::from_rgb(230, 96, 96),
    Color32::from_rgb(110, 210, 120),
    Color32::from_rgb(96, 150, 236),
];
const SHAPE_NAMES: [&str; SHAPES] =
    ["Bell", "Low Shelf", "Low Cut", "High Shelf", "High Cut", "Notch", "Band Pass", "Tilt Shelf", "Flat Tilt", "All Pass"];
const PLACE_NAMES: [(&str, &str); 5] = [
    ("ST", "Stereo: both channels"),
    ("L", "Left channel only"),
    ("R", "Right channel only"),
    ("M", "Mid: the centre (L+R)"),
    ("S", "Side: the edges (L−R)"),
];

/// Band `b`'s colour: hues a golden turn apart, so neighbours differ.
pub fn band_color(b: usize) -> Color32 {
    let h = (0.02 + b as f32 * 0.381_966).fract();
    Color32::from(Hsva::new(h, 0.62, 0.95, 1.0))
}

/// Collapsed nodes' readout.
pub fn readout(p: &ModuleParams) -> (String, String) {
    let count = (0..BANDS).filter(|&b| pq::used(p, b)).count();
    let dynamic = (0..BANDS).filter(|&b| pq::active(p, b) && pq::dynamic(p, b) && !pq::spectral(p, b)).count();
    let spectral = (0..BANDS).filter(|&b| pq::active(p, b) && pq::spectral(p, b)).count();
    let mode = ["ZERO LATENCY", "NATURAL PHASE", "LINEAR PHASE"][pq::choice(p, data::MODE, 3)];
    let mut sub = vec![mode.to_owned()];
    if dynamic > 0 {
        sub.push(format!("{dynamic} dynamic"));
    }
    if spectral > 0 {
        sub.push(format!("{spectral} spectral"));
    }
    (format!("{count} band{}", if count == 1 { "" } else { "s" }), sub.join(" · "))
}

fn fmt_hz(f: f32) -> String {
    if f >= 1000.0 { format!("{:.2} kHz", f / 1000.0) } else { format!("{f:.1} Hz") }
}

fn fmt_short_hz(f: f32) -> String {
    if f >= 1000.0 { format!("{:.1}k", f / 1000.0) } else { format!("{f:.0}") }
}

/// The display's scales.
#[derive(Clone, Copy)]
struct Axis {
    rect: Rect,
    /// The EQ scale: ± this many dB.
    range: f32,
    /// The analyzer scale: 0 dBFS at the top down to −this.
    floor: f32,
}

impl Axis {
    fn x(&self, f: f32) -> f32 {
        self.rect.left() + self.rect.width() * (f.max(1.0) / F_LO).ln() / (F_HI / F_LO).ln()
    }
    fn f(&self, x: f32) -> f32 {
        F_LO * (F_HI / F_LO).powf(((x - self.rect.left()) / self.rect.width()).clamp(0.0, 1.0))
    }
    fn half(&self) -> f32 {
        self.rect.height() * 0.42
    }
    fn y(&self, db: f32) -> f32 {
        self.rect.center().y - (db / self.range).clamp(-1.15, 1.15) * self.half()
    }
    fn db(&self, y: f32) -> f32 {
        (self.rect.center().y - y) / self.half() * self.range
    }
    /// Analyzer level to y.
    fn ay(&self, db: f32) -> f32 {
        self.rect.top() + self.rect.height() * (-db / self.floor).clamp(0.0, 1.0)
    }
    /// Gainless shapes keep their dot at a height for Q (Q 1 on 0 dB).
    fn q_y(&self, q: f32) -> f32 {
        self.rect.center().y - (q.ln() / pq::Q_RANGE.1.ln()).clamp(-1.0, 1.0) * self.half()
    }
    fn y_q(&self, y: f32) -> f32 {
        (((self.rect.center().y - y) / self.half()).clamp(-1.0, 1.0) * pq::Q_RANGE.1.ln()).exp()
    }
}

/// Which band shape a click at `q` creates: cuts at the far ends, shelves
/// near them, a notch along the bottom, a bell elsewhere.
fn shape_at(axis: &Axis, q: Pos2) -> usize {
    let t = (q.x - axis.rect.left()) / axis.rect.width();
    if t < 0.035 {
        LOW_CUT
    } else if t > 0.965 {
        HIGH_CUT
    } else if q.y > axis.rect.bottom() - axis.rect.height() * 0.08 {
        NOTCH
    } else if t < 0.09 {
        LOW_SHELF
    } else if t > 0.91 {
        HIGH_SHELF
    } else {
        BELL
    }
}

/// What a drag on the display is doing.
#[derive(Clone, Default)]
enum Gesture {
    #[default]
    None,
    /// Moving the selected bands: each one's (band, freq, gain, Q) at the
    /// start, the band grabbed, where the drag began.
    Bands(Vec<(usize, f32, f32, f32)>, usize, Pos2),
    /// Dragging a dynamic band's range handle: band, range at the start.
    Range(usize, f32, Pos2),
    Lasso(Pos2),
    Sketch(Vec<Pos2>),
}

/// The analyzer's spectra and EQ Match's averages, kept between frames.
#[derive(Clone, Default)]
struct Spectra {
    n: usize,
    window: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    /// Shown levels per column (dB): pre, post, SC.
    shown: [Vec<f32>; 3],
    /// This frame's power per column, untilted (for EQ Match).
    power: [Vec<f32>; 3],
    last: f64,
}

#[derive(Clone, Default)]
struct Matching {
    open: bool,
    input: Vec<f64>,
    reference: Vec<f64>,
    frames_in: u32,
    frames_ref: u32,
    /// The reference was captured from the input (else it is the SC input).
    captured: bool,
    detail: f32,
    /// Every band's knobs and data before matching, to cancel.
    snapshot: Option<(Vec<f32>, Vec<f32>)>,
}

/// Spectrum Grab: the pointer resting on the spectrum freezes it and marks
/// its peaks.
#[derive(Clone, Default)]
struct Grab {
    rest: Option<(Pos2, f64)>,
    frozen: Option<Vec<f32>>,
}

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
    s.top_bar();
    s.display();
    s.output_meter();
    s.strip();
    s.bottom_bar();
}

impl Panel<'_, '_> {
    fn z(&self) -> f32 {
        self.v.z
    }

    fn uid(&self, salt: impl std::hash::Hash + std::fmt::Debug) -> Id {
        Id::unique(("prism", self.id)).with(salt)
    }

    fn on(&self, i: usize) -> bool {
        pq::on(self.p, i)
    }

    fn flip(&self, i: usize) {
        self.p.data[i].set(if self.on(i) { 0.0 } else { 1.0 });
    }

    fn sel(&self) -> u32 {
        self.p.data[data::SELECTED].get().max(0.0) as u32
    }

    fn set_sel(&self, mask: u32) {
        self.p.data[data::SELECTED].set((mask & 0xFF_FFFF) as f32);
    }

    fn is_sel(&self, b: usize) -> bool {
        self.sel() & (1 << b) != 0
    }

    /// The band the strip shows: the last one picked, if still selected.
    fn primary(&self) -> Option<usize> {
        let mask = self.sel();
        let last = self.v.ui.data(|d| d.get_temp::<usize>(self.uid("primary")));
        last.filter(|&b| b < BANDS && mask & (1 << b) != 0 && pq::used(self.p, b))
            .or_else(|| (0..BANDS).find(|&b| mask & (1 << b) != 0 && pq::used(self.p, b)))
    }

    fn pick(&self, b: usize, add: bool) {
        let mask = if add { self.sel() | (1 << b) } else { 1 << b };
        self.set_sel(mask);
        self.v.ui.data_mut(|d| d.insert_temp(self.uid("primary"), b));
    }

    fn selected(&self) -> Vec<usize> {
        (0..BANDS).filter(|&b| self.is_sel(b) && pq::used(self.p, b)).collect()
    }

    fn chip(&self, salt: impl std::hash::Hash + std::fmt::Debug, rect: Rect, label: &str, lit: bool, color: Color32, tip: &str) -> Response {
        widgets::toggle(self.v.ui, self.v.painter, self.uid(("chip", salt)), rect, label, lit, color, self.z()).on_hover_text(tip)
    }

    fn switch(&self, i: usize, rect: Rect, label: &str, tip: &str) {
        if self.chip(("switch", i), rect, label, self.on(i), self.accent, tip).clicked() {
            self.flip(i);
        }
    }

    /// A segmented choice stored in data `i`; `enabled` dims and locks it.
    fn seg(&self, i: usize, rect: Rect, items: &[(&str, &'static str)], color: Color32, enabled: bool) -> Option<usize> {
        let v = self.v;
        let cur = pq::choice(self.p, i, items.len());
        let color = if enabled { color } else { theme::TEXT_FAINT };
        let picked = segmented(v, self.uid(("seg", i)), rect, items.len(), cur, color, |k, r, col| {
            let col = if enabled { col } else { alpha(col, 0.5) };
            v.painter.text(r.center(), Align2::CENTER_CENTER, items[k].0, v.font(7.5), col);
            items[k].1
        });
        let picked = picked.filter(|_| enabled);
        if let Some(k) = picked {
            self.p.data[i].set(k as f32);
        }
        picked
    }

    /// Steps through `labels`: right half next, left half previous.
    fn cycler(&self, i: usize, rect: Rect, labels: &[String], tip: &str) {
        let (v, z) = (self.v, self.z());
        let resp = v.ui.interact(rect, self.uid(("cycle", i)), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip);
        let cur = pq::choice(self.p, i, labels.len());
        if resp.clicked() {
            let back = resp.interact_pointer_pos().is_some_and(|q| q.x < rect.center().x);
            let next = if back { (cur + labels.len() - 1) % labels.len() } else { (cur + 1) % labels.len() };
            self.p.data[i].set(next as f32);
        }
        v.painter.rect_filled(rect, 3.0 * z, if resp.hovered() { theme::CONTROL_HI } else { theme::CONTROL });
        v.painter.text(rect.center(), Align2::CENTER_CENTER, &labels[cur], theme::font(7.5 * z), theme::TEXT);
        for (x, glyph) in [(rect.left() + 5.0 * z, "‹"), (rect.right() - 5.0 * z, "›")] {
            v.painter.text(pos2(x, rect.center().y), Align2::CENTER_CENTER, glyph, theme::font(8.0 * z), theme::TEXT_FAINT);
        }
    }

    fn label(&self, x: f32, y: f32, text: &str) {
        self.v.text(x, y, Align2::LEFT_CENTER, text, self.v.font(7.5), theme::TEXT_FAINT);
    }

    // -----------------------------------------------------------------------
    // Analyzer bar
    // -----------------------------------------------------------------------

    fn top_bar(&self) {
        let v = self.v;
        let (y0, y1) = (32.0, 50.0);
        let grey = theme::TEXT_DIM;
        self.switch(data::AN_PRE, v.r(40.0, y0, 74.0, y1), "PRE", "The spectrum before the EQ");
        self.switch(data::AN_POST, v.r(77.0, y0, 115.0, y1), "POST", "The spectrum after the EQ");
        self.switch(data::AN_SC, v.r(118.0, y0, 146.0, y1), "SC", "The side chain's spectrum (red outline)");
        self.label(154.0, 41.0, "RANGE");
        self.seg(data::AN_RANGE, v.r(184.0, y0, 262.0, y1), &[("60", "60 dB"), ("90", "90 dB"), ("120", "120 dB")], grey, true);
        self.label(268.0, 41.0, "RES");
        let res: Vec<(&str, &'static str)> =
            vec![("LOW", "1024 points"), ("MED", "2048 points"), ("HIGH", "4096 points"), ("MAX", "8192 points: finest in the lows, slowest")];
        self.seg(data::AN_RES, v.r(288.0, y0, 404.0, y1), &res, grey, true);
        let speeds: Vec<String> = pq::AN_SPEEDS.iter().map(|s| s.0.to_owned()).collect();
        self.cycler(data::AN_SPEED, v.r(410.0, y0, 470.0, y1), &speeds, "Speed: how fast the spectrum falls back");
        let tilts: Vec<String> = pq::AN_TILTS.iter().map(|t| format!("{t:.1} dB/o")).collect();
        self.cycler(data::AN_TILT, v.r(474.0, y0, 548.0, y1), &tilts, "Tilt: dB per octave around 1 kHz (4.5 is close to how loud we hear)");
        self.switch(data::AN_FREEZE, v.r(552.0, y0, 600.0, y1), "FREEZE", "Hold the peaks: the spectrum stops falling");
        self.switch(data::AN_GRAB, v.r(604.0, y0, 646.0, y1), "GRAB", "Spectrum Grab: rest the pointer on the spectrum to mark its peaks, then drag one");
        self.switch(data::PIANO, v.r(700.0, y0, 744.0, y1), "PIANO", "Piano along the display; dragging snaps to semitones (Shift: free)");
        let ranges = [("±3", "±3 dB (mastering)"), ("±6", "±6 dB"), ("±12", "±12 dB"), ("±30", "±30 dB")];
        self.seg(data::DISPLAY_RANGE, v.r(750.0, y0, GRAPH[2], y1), &ranges, YELLOW, true);
    }

    fn axis(&self) -> Axis {
        let p = self.p;
        Axis {
            rect: self.v.r(GRAPH[0], GRAPH[1], GRAPH[2], GRAPH[3]),
            range: pq::DISPLAY_RANGES[pq::choice(p, data::DISPLAY_RANGE, 4)],
            floor: pq::AN_RANGES[pq::choice(p, data::AN_RANGE, 3)],
        }
    }

    // -----------------------------------------------------------------------
    // Display
    // -----------------------------------------------------------------------

    fn display(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let axis = self.axis();
        let rect = axis.rect;
        let painter = v.painter.with_clip_rect(rect.intersect(v.painter.clip_rect()));
        painter.rect_filled(rect, 4.0 * z, theme::INSET);
        let sr = p.viz[viz::SR].get().max(8_000.0);

        // Grid.
        let font = theme::font(7.0 * z);
        for (f, label) in [(20.0, "20"), (50.0, "50"), (100.0, "100"), (200.0, "200"), (500.0, "500"), (1e3, "1k"), (2e3, "2k"), (5e3, "5k"), (1e4, "10k"), (2e4, "20k")] {
            let x = axis.x(f);
            painter.vline(x, rect.y_range(), Stroke::new(1.0 * z, theme::GRID_MAJOR));
            painter.text(pos2(x + 2.0 * z, rect.bottom() - 2.0 * z), Align2::LEFT_BOTTOM, label, font.clone(), theme::TEXT_FAINT);
        }
        let step = match axis.range as i32 {
            3 => 1.0,
            6 => 2.0,
            12 => 3.0,
            _ => 6.0,
        };
        let mut db = -axis.range;
        while db <= axis.range + 0.01 {
            let y = axis.y(db);
            let col = if db == 0.0 { alpha(YELLOW, 0.25) } else { theme::GRID_MAJOR };
            painter.hline(rect.x_range(), y, Stroke::new(1.0 * z, col));
            if db.abs() < axis.range - 0.01 {
                let label = if db == 0.0 { "0".to_owned() } else { format!("{db:+.0}") };
                painter.text(pos2(rect.right() - 3.0 * z, y - 1.0 * z), Align2::RIGHT_BOTTOM, label, font.clone(), alpha(YELLOW, 0.6));
            }
            db += step;
        }
        // The analyzer's scale on the left.
        let mut a = 0.0;
        while a > -axis.floor + 1.0 {
            let y = axis.ay(a);
            if a < 0.0 {
                painter.text(pos2(rect.left() + 3.0 * z, y), Align2::LEFT_CENTER, format!("{a:.0}"), font.clone(), alpha(theme::TEXT_FAINT, 0.8));
            }
            a -= 30.0;
        }

        // Spectra.
        let xs: Vec<f32> = (0..COLS).map(|k| rect.left() + rect.width() * k as f32 / (COLS - 1) as f32).collect();
        let freqs: Vec<f32> = xs.iter().map(|&x| axis.f(x)).collect();
        let shown = self.spectra(&freqs, sr);
        let grab = self.grab_state();
        let nyq_x = axis.x(sr * 0.5);
        let line = |levels: &[f32], color: Color32, width: f32| {
            let pts: Vec<Pos2> = xs.iter().zip(levels).filter(|(x, _)| **x <= nyq_x).map(|(&x, &l)| pos2(x, axis.ay(l))).collect();
            painter.add(Shape::line(pts, Stroke::new(width * z, color)));
        };
        if let Some(sh) = &shown {
            if self.on(data::AN_PRE) {
                line(&sh[0], alpha(Color32::from_rgb(120, 150, 190), 0.55), 1.0);
            }
            if self.on(data::AN_SC) {
                line(&sh[2], alpha(Color32::from_rgb(232, 96, 96), 0.75), 1.0);
            }
            if self.on(data::AN_POST) {
                let post = grab.frozen.as_deref().unwrap_or(&sh[1]);
                line(post, alpha(theme::TEXT, 0.7), 1.2);
            }
        }

        // Curves.
        let used: Vec<usize> = (0..BANDS).filter(|&b| pq::used(p, b)).collect();
        let selected = self.sel();
        let mut totals = [vec![0.0f32; COLS], vec![0.0; COLS], vec![0.0; COLS], vec![0.0; COLS], vec![0.0; COLS]];
        let mut places_used = [true, false, false, false, false];
        let mut reach_any = false;
        for &b in &used {
            let color = band_color(b);
            let act = pq::active(p, b);
            let place = pq::place(p, b);
            let a = pq::analog(p, b, 0.0);
            let curve: Vec<f32> = freqs.iter().map(|&f| a.db(f as f64) as f32).collect();
            if act {
                places_used[place] = true;
                for (t, c) in totals[place].iter_mut().zip(&curve) {
                    *t += c;
                }
            }
            let pts: Vec<Pos2> = xs.iter().zip(&curve).map(|(&x, &db)| pos2(x, axis.y(db))).collect();
            let is_sel = selected & (1 << b) != 0;
            let col = if act { color } else { theme::TEXT_FAINT };
            if is_sel && act {
                tint(&painter, &pts, axis.y(0.0), alpha(color, 0.08));
            }
            painter.add(Shape::line(pts, Stroke::new(if is_sel { 1.8 } else { 1.0 } * z, alpha(col, if is_sel { 0.95 } else { 0.6 }))));
            if act && pq::dynamic(p, b) {
                reach_any = true;
                // How far the dynamics can take it (dashed), and where they are.
                let range = p.get(bp(b, band::RANGE)) * p.get(pq::SCALE);
                let ra = pq::analog(p, b, range);
                let reach: Vec<Pos2> = xs.iter().zip(&freqs).map(|(&x, &f)| pos2(x, axis.y(ra.db(f as f64) as f32))).collect();
                dashed(&painter, &reach, Stroke::new(1.0 * z, alpha(color, 0.5)));
                let d = p.viz[viz::DYN + b].get();
                if d.abs() > 0.05 && !pq::spectral(p, b) {
                    let la = pq::analog(p, b, d);
                    let live: Vec<Pos2> = xs.iter().zip(&freqs).map(|(&x, &f)| pos2(x, axis.y(la.db(f as f64) as f32))).collect();
                    painter.add(Shape::line(live, Stroke::new(1.6 * z, color)));
                }
            }
        }
        // Spectral bands' live changes along the spectrum.
        if used.iter().any(|&b| pq::active(p, b) && pq::spectral(p, b)) {
            let curve: Vec<Pos2> = (0..crate::audio::modules::prism::spectral::CURVE)
                .map(|k| {
                    let t = k as f32 / (crate::audio::modules::prism::spectral::CURVE - 1) as f32;
                    let f = crate::audio::modules::prism::spectral::GRID_LO * (sr * 0.5 / crate::audio::modules::prism::spectral::GRID_LO).powf(t);
                    pos2(axis.x(f), axis.y(p.viz[viz::CURVE_AT + k].get()))
                })
                .collect();
            // Dotted, as it flickers with the music.
            for pair in curve.windows(2).step_by(2) {
                painter.line_segment([pair[0], pair[1]], Stroke::new(1.2 * z, alpha(theme::TEXT, 0.75)));
            }
        }
        // Totals: yellow for what every channel gets, then one per placement.
        for place in (0..5).rev() {
            if !places_used[place] || (place > 0 && used.is_empty()) {
                continue;
            }
            let total: Vec<f32> = if place == 0 { totals[0].clone() } else { totals[0].iter().zip(&totals[place]).map(|(a, b)| a + b).collect() };
            let pts: Vec<Pos2> = xs.iter().zip(&total).map(|(&x, &db)| pos2(x, axis.y(db))).collect();
            painter.add(Shape::line(pts, Stroke::new(2.0 * z, alpha(PLACE_COLORS[place], 0.95))));
        }
        let _ = reach_any;

        // Dots.
        let dots = self.dots(&axis);
        let hovered = v.ui.input(|i| i.pointer.hover_pos()).filter(|q| rect.contains(*q));
        let hit = |q: Pos2| dots.iter().rev().map(|&(b, c)| (b, c.distance(q))).filter(|&(_, d)| d < 10.0 * z).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(b, _)| b);
        let hover_band = hovered.and_then(hit);
        for &(b, c) in &dots {
            let act = pq::active(p, b);
            let color = if act { band_color(b) } else { theme::TEXT_FAINT };
            let is_sel = selected & (1 << b) != 0;
            if act && pq::dynamic(p, b) {
                // The range handle and the live position along it.
                let range = p.get(bp(b, band::RANGE)) * p.get(pq::SCALE);
                let base = p.get(bp(b, band::GAIN)) * p.get(pq::SCALE);
                let end = pos2(c.x, axis.y(base + range));
                painter.line_segment([c, end], Stroke::new(1.0 * z, alpha(color, 0.6)));
                diamond(&painter, end, 4.0 * z, Stroke::new(1.2 * z, color));
                let d = p.viz[viz::DYN + b].get();
                if d.abs() > 0.05 {
                    let at = pos2(c.x, axis.y(base + d));
                    painter.line_segment([c, at], Stroke::new(3.0 * z, alpha(YELLOW, 0.9)));
                }
            }
            let r = 6.5 * z;
            painter.circle_filled(c, r, theme::INSET);
            painter.circle_stroke(c, r, Stroke::new(if is_sel { 2.2 } else { 1.5 } * z, color));
            if is_sel {
                painter.circle_stroke(c, r + 3.0 * z, Stroke::new(1.0 * z, alpha(color, 0.7)));
            }
            let tag = match pq::place(p, b) {
                pq::LEFT => "L",
                pq::RIGHT => "R",
                pq::MID => "M",
                pq::SIDE => "S",
                _ if pq::spectral(p, b) => "≈",
                _ => "",
            };
            if !tag.is_empty() {
                painter.text(c, Align2::CENTER_CENTER, tag, theme::font(7.5 * z), color);
            } else if hover_band == Some(b) {
                painter.circle_filled(c, 2.0 * z, color);
            }
        }

        let resp = v.ui.interact(rect, self.uid("display"), Sense::click_and_drag());
        self.display_input(&resp, &axis, &dots, &freqs, &xs, &shown, &painter);

        // Spectrum Grab's peaks.
        if let Some(frozen) = &grab.frozen {
            for (k, _) in peaks(frozen, &freqs, sr) {
                let c = pos2(xs[k], axis.ay(frozen[k]));
                painter.circle_stroke(c, 4.0 * z, Stroke::new(1.2 * z, theme::TEXT));
                painter.text(c - vec2(0.0, 8.0 * z), Align2::CENTER_BOTTOM, fmt_short_hz(freqs[k]), theme::font(7.5 * z), theme::TEXT);
            }
        }

        // The band under the pointer (or being dragged): its values.
        let tag_band = hover_band.or_else(|| match v.ui.data(|d| d.get_temp::<Gesture>(self.uid("gesture"))) {
            Some(Gesture::Bands(_, b, _)) => Some(b),
            _ => None,
        });
        if let Some(b) = tag_band
            && let Some(&(_, c)) = dots.iter().find(|(k, _)| *k == b)
        {
            let sh = pq::shape(p, b);
            let mut text = format!("{}  {}", SHAPE_NAMES[sh], fmt_hz(p.get(bp(b, band::FREQ))));
            if design::has_gain(sh) {
                text.push_str(&format!("  {:+.1} dB", p.get(bp(b, band::GAIN))));
            }
            if design::has_q(sh, pq::slope(p, b)) {
                text.push_str(&format!("  Q {:.2}", p.get(bp(b, band::Q))));
            }
            if pq::choice(p, data::PIANO, 2) == 1 {
                text.push_str(&format!("  {}", note_name(p.get(bp(b, band::FREQ)))));
            }
            let font = theme::mono(8.0 * z);
            let galley = painter.layout_no_wrap(text, font, theme::TEXT);
            let size = galley.size() + vec2(10.0, 6.0) * z;
            let mut at = c + vec2(12.0, -12.0 - size.y) * z;
            at.x = at.x.min(rect.right() - size.x - 2.0 * z);
            at.y = at.y.max(rect.top() + 2.0 * z);
            let r = Rect::from_min_size(at, size);
            painter.rect_filled(r, 3.0 * z, alpha(theme::INSET, 0.92));
            painter.rect_stroke(r, 3.0 * z, Stroke::new(1.0 * z, band_color(b)), StrokeKind::Inside);
            painter.galley(r.min + vec2(5.0, 3.0) * z, galley, theme::TEXT);
        } else if let Some(q) = hovered {
            let text = format!("{} ({})", fmt_hz(axis.f(q.x)), note_name(axis.f(q.x)));
            painter.text(rect.right_top() + vec2(-6.0, 4.0) * z, Align2::RIGHT_TOP, text, theme::mono(8.0 * z), theme::TEXT_DIM);
        }

        if self.on(data::PIANO) {
            self.piano(&painter, &axis);
        }
        if p.viz[viz::WARN].get() > 0.5 {
            painter.text(rect.left_top() + vec2(8.0, 6.0) * z, Align2::LEFT_TOP, "Dynamics are off at VERY HIGH and MAX resolution", theme::font(8.0 * z), theme::METER_YELLOW);
        }
        if used.is_empty() && hovered.is_none() {
            painter.text(rect.center(), Align2::CENTER_CENTER, "Click to add a band · Alt-click: dynamic · Alt+Shift-click: spectral", theme::font(9.0 * z), theme::TEXT_FAINT);
        }
        self.match_panel(&painter, &axis, &freqs, &xs, sr);
        if self.on(data::AN_PRE) || self.on(data::AN_POST) || self.on(data::AN_SC) || self.on(data::MATCH) {
            v.ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        }
    }

    /// Dot positions: at (frequency, gain) for shapes with a gain, at a
    /// height for Q for the others.
    fn dots(&self, axis: &Axis) -> Vec<(usize, Pos2)> {
        let p = self.p;
        (0..BANDS)
            .filter(|&b| pq::used(p, b))
            .map(|b| {
                let sh = pq::shape(p, b);
                let x = axis.x(p.get(bp(b, band::FREQ)));
                let y = if design::has_gain(sh) { axis.y(p.get(bp(b, band::GAIN)) * p.get(pq::SCALE)) } else { axis.q_y(p.get(bp(b, band::Q))) };
                (b, pos2(x, y))
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn display_input(&self, resp: &Response, axis: &Axis, dots: &[(usize, Pos2)], freqs: &[f32], xs: &[f32], shown: &Option<[Vec<f32>; 3]>, painter: &egui::Painter) {
        let (v, p, z) = (self.v, self.p, self.z());
        let hit = |q: Pos2| dots.iter().rev().map(|&(b, c)| (b, c.distance(q))).filter(|&(_, d)| d < 10.0 * z).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(b, _)| b);
        let range_hit = |q: Pos2| {
            dots.iter().find(|&&(b, c)| {
                pq::active(p, b) && pq::dynamic(p, b) && {
                    let end = pos2(c.x, axis.y((p.get(bp(b, band::GAIN)) + p.get(bp(b, band::RANGE))) * p.get(pq::SCALE)));
                    end.distance(q) < 6.0 * z && c.distance(q) > 6.0 * z
                }
            })
            .map(|&(b, _)| b)
        };
        let mods = v.ui.input(|i| i.modifiers);
        let gid = self.uid("gesture");
        let sketching = v.ui.data(|d| d.get_temp::<bool>(self.uid("sketch-on"))).unwrap_or(false);
        let piano = self.on(data::PIANO);

        // Total curve under the pointer (for creating a band by dragging it).
        let total_at = |x: f32| -> f32 {
            let f = axis.f(x);
            (0..BANDS).filter(|&b| pq::active(p, b) && pq::place(p, b) == pq::STEREO).map(|b| pq::analog(p, b, 0.0).db(f as f64) as f32).sum()
        };

        if resp.drag_started_by(PointerButton::Primary)
            && let Some(origin) = v.ui.input(|i| i.pointer.press_origin())
        {
            let grab = self.grab_state();
            let gesture = if let Some(b) = range_hit(origin) {
                Gesture::Range(b, p.get(bp(b, band::RANGE)), origin)
            } else if let Some(b) = hit(origin) {
                if !self.is_sel(b) {
                    self.pick(b, mods.command || mods.shift);
                } else {
                    v.ui.data_mut(|d| d.insert_temp(self.uid("primary"), b));
                }
                Gesture::Bands(self.drag_start(), b, origin)
            } else if sketching {
                Gesture::Sketch(vec![origin])
            } else if let Some((k, f)) = grab.frozen.as_ref().and_then(|fr| peaks(fr, freqs, p.viz[viz::SR].get()).into_iter().map(|(k, _)| (k, freqs[k])).find(|&(k, _)| (xs[k] - origin.x).abs() < 10.0 * z)) {
                // Spectrum Grab: a bell on the peak, its width from the peak's.
                let q = grab.frozen.as_ref().map_or(4.0, |fr| peak_q(fr, freqs, k));
                match self.create(BELL, f, 0.0, q, false, false) {
                    Some(b) => Gesture::Bands(self.drag_start(), b, origin),
                    None => Gesture::None,
                }
            } else if (axis.y(total_at(origin.x)) - origin.y).abs() < 7.0 * z {
                // Dragging the yellow curve makes a band (shelves at the ends).
                let mut sh = shape_at(axis, origin);
                sh = match sh {
                    LOW_CUT => LOW_SHELF,
                    HIGH_CUT => HIGH_SHELF,
                    NOTCH => BELL,
                    s => s,
                };
                match self.create(sh, axis.f(origin.x), 0.0, 1.0, mods.alt, mods.alt && mods.shift) {
                    Some(b) => Gesture::Bands(self.drag_start(), b, origin),
                    None => Gesture::None,
                }
            } else {
                Gesture::Lasso(origin)
            };
            v.ui.data_mut(|d| d.insert_temp(gid, gesture));
        }

        let gesture = v.ui.data(|d| d.get_temp::<Gesture>(gid)).unwrap_or_default();
        if resp.dragged_by(PointerButton::Primary)
            && let Some(q) = resp.interact_pointer_pos()
        {
            match gesture.clone() {
                Gesture::Bands(start, grabbed, origin) => {
                    let fine = if mods.shift { 0.25 } else { 1.0 };
                    let mut d = (q - origin) * fine;
                    if mods.alt {
                        if d.x.abs() > d.y.abs() {
                            d.y = 0.0;
                        } else {
                            d.x = 0.0;
                        }
                    }
                    let Some(&(_, f0, g0, _)) = start.iter().find(|s| s.0 == grabbed) else { return };
                    let scale = p.get(pq::SCALE).max(0.01);
                    let grabbed_shape = pq::shape(p, grabbed);
                    // Frequency: the grabbed band's move, as a ratio for all.
                    let mut f_new = axis.f(axis.x(f0) + d.x).clamp(pq::FREQ_RANGE.0, pq::FREQ_RANGE.1);
                    if piano && !mods.shift {
                        f_new = snap_note(f_new);
                    }
                    let ratio = f_new / f0;
                    for &(b, f, g, qq) in &start {
                        let sh = pq::shape(p, b);
                        if mods.command {
                            // Cmd: vertical drag is Q, for every selected band.
                            let new_q = (qq * (-d.y / (60.0 * z)).exp()).clamp(pq::Q_RANGE.0, pq::Q_RANGE.1);
                            p.set(bp(b, band::Q), new_q);
                            continue;
                        }
                        p.set(bp(b, band::FREQ), (f * ratio).clamp(pq::FREQ_RANGE.0, pq::FREQ_RANGE.1));
                        if design::has_gain(sh) {
                            let g_new = if design::has_gain(grabbed_shape) {
                                let gd = axis.db(axis.y(g0 * scale) + d.y) / scale;
                                // Gains move together, scaled when they can be.
                                if g0.abs() > 0.5 && b != grabbed { g * gd / g0 } else if b == grabbed { gd } else { g + (gd - g0) }
                            } else {
                                g
                            };
                            p.set(bp(b, band::GAIN), g_new.clamp(-pq::GAIN_RANGE, pq::GAIN_RANGE));
                        } else if b == grabbed || !design::has_gain(grabbed_shape) {
                            // Gainless shapes: up and down is Q.
                            let qn = axis.y_q(axis.q_y(qq) + d.y);
                            if design::has_q(sh, pq::slope(p, b)) {
                                p.set(bp(b, band::Q), qn.clamp(pq::Q_RANGE.0, pq::Q_RANGE.1));
                            }
                        }
                    }
                    // The scale follows a band dragged past it.
                    if design::has_gain(grabbed_shape) {
                        let g = p.get(bp(grabbed, band::GAIN)).abs() * scale;
                        let cur = pq::choice(p, data::DISPLAY_RANGE, 4);
                        if g > pq::DISPLAY_RANGES[cur] + 0.5
                            && let Some(next) = (cur + 1..4).find(|&i| pq::DISPLAY_RANGES[i] >= g)
                        {
                            p.data[data::DISPLAY_RANGE].set(next as f32);
                        }
                    }
                }
                Gesture::Range(b, r0, origin) => {
                    let scale = p.get(pq::SCALE).max(0.01);
                    let dr = -(q.y - origin.y) / axis.half() * axis.range / scale;
                    let r = (r0 + dr).clamp(-pq::GAIN_RANGE, pq::GAIN_RANGE);
                    p.set(bp(b, band::RANGE), if r.abs() < 0.1 { if r0 < 0.0 { -0.1 } else { 0.1 } } else { r });
                }
                Gesture::Lasso(origin) => {
                    let r = Rect::from_two_pos(origin, q);
                    for s in Shape::dashed_line(&[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()], Stroke::new(1.0 * z, theme::TEXT_DIM), 4.0 * z, 3.0 * z) {
                        painter.add(s);
                    }
                    let mask = dots.iter().filter(|(_, c)| r.contains(*c)).fold(if mods.command || mods.shift { self.sel() } else { 0 }, |m, &(b, _)| m | (1 << b));
                    self.set_sel(mask);
                }
                Gesture::Sketch(mut path) => {
                    if path.last().is_none_or(|l| l.distance(q) > 2.0 * z) {
                        // Moving back redraws: drop what lies to the right.
                        path.retain(|pt| pt.x < q.x);
                        path.push(q);
                    }
                    painter.add(Shape::line(path.clone(), Stroke::new(2.0 * z, alpha(YELLOW, 0.8))));
                    v.ui.data_mut(|d| d.insert_temp(gid, Gesture::Sketch(path)));
                }
                Gesture::None => {}
            }
        }
        if resp.drag_stopped() {
            if let Gesture::Sketch(path) = &gesture {
                self.finish_sketch(axis, path, freqs);
            }
            v.ui.data_mut(|d| d.insert_temp(gid, Gesture::None));
            self.end_grab();
        }

        // Clicks.
        if resp.clicked()
            && let Some(q) = resp.interact_pointer_pos()
        {
            if let Some(b) = hit(q) {
                if mods.alt && mods.command {
                    let sh = (pq::shape(p, b) + 1) % SHAPES;
                    self.set_shape(b, sh);
                } else if mods.alt && mods.shift {
                    let s = design::next_slope(pq::shape(p, b), pq::slope(p, b));
                    p.data[data::SLOPE + b].set(s as f32);
                } else if mods.alt {
                    self.flip(data::ON + b);
                } else if mods.command {
                    self.set_sel(self.sel() ^ (1 << b));
                    v.ui.data_mut(|d| d.insert_temp(self.uid("primary"), b));
                } else if mods.shift {
                    self.select_range(b);
                } else {
                    self.pick(b, false);
                }
            } else if !sketching {
                let nothing_selected = self.selected().is_empty();
                if nothing_selected || mods.command || mods.alt {
                    let sh = shape_at(axis, q);
                    let gain = if design::has_gain(sh) { axis.db(q.y).clamp(-axis.range, axis.range) } else { 0.0 };
                    self.create(sh, axis.f(q.x), gain, 1.0, mods.alt, mods.alt && mods.shift);
                } else {
                    self.set_sel(0);
                }
            }
        }
        if resp.double_clicked()
            && let Some(q) = resp.interact_pointer_pos()
            && hit(q).is_none()
            && !sketching
            && !self.selected().is_empty()
        {
            let sh = shape_at(axis, q);
            let gain = if design::has_gain(sh) { axis.db(q.y).clamp(-axis.range, axis.range) } else { 0.0 };
            self.create(sh, axis.f(q.x), gain, 1.0, mods.alt, mods.alt && mods.shift);
        }

        // Hover: the band a click would make, and Spectrum Grab's timer.
        let hover = resp.hover_pos();
        if let Some(q) = hover {
            if hit(q).is_some() || range_hit(q).is_some() {
                v.ui.ctx().set_cursor_icon(CursorIcon::Grab);
            } else if matches!(gesture, Gesture::None) && !sketching {
                let sh = shape_at(axis, q);
                let gain = if design::has_gain(sh) { axis.db(q.y).clamp(-axis.range, axis.range) } else { 0.0 };
                let a = Analog::design(&Band { shape: sh, slope: if matches!(sh, LOW_CUT | HIGH_CUT) { 3 } else { 1 }, freq: axis.f(q.x) as f64, gain: gain as f64, q: 1.0 });
                let pts: Vec<Pos2> = xs.iter().zip(freqs).map(|(&x, &f)| pos2(x, axis.y(a.db(f as f64) as f32))).collect();
                let col = alpha(theme::TEXT_DIM, 0.35);
                if self.selected().is_empty() {
                    painter.add(Shape::line(pts, Stroke::new(1.0 * z, col)));
                } else {
                    dashed(painter, &pts, Stroke::new(1.0 * z, col));
                }
            }
            self.grab_timer(q, hit(q).is_none() && shown.is_some() && matches!(gesture, Gesture::None) && !sketching, shown);
        } else if !resp.dragged() {
            self.end_grab();
        }
    }

    fn drag_start(&self) -> Vec<(usize, f32, f32, f32)> {
        let p = self.p;
        self.selected().into_iter().map(|b| (b, p.get(bp(b, band::FREQ)), p.get(bp(b, band::GAIN)), p.get(bp(b, band::Q)))).collect()
    }

    /// Shift-click: every band between the primary and `b`, by frequency.
    fn select_range(&self, b: usize) {
        let p = self.p;
        let Some(a) = self.primary() else {
            self.pick(b, false);
            return;
        };
        let (f0, f1) = (p.get(bp(a, band::FREQ)), p.get(bp(b, band::FREQ)));
        let (lo, hi) = (f0.min(f1), f0.max(f1));
        let mask = (0..BANDS).filter(|&k| pq::used(p, k) && (lo..=hi).contains(&p.get(bp(k, band::FREQ)))).fold(self.sel(), |m, k| m | (1 << k));
        self.set_sel(mask);
    }

    /// A new band in the first free slot, selected.
    fn create(&self, sh: usize, freq: f32, gain: f32, q: f32, dynamic: bool, spectral: bool) -> Option<usize> {
        let p = self.p;
        let b = pq::free_band(p)?;
        pq::create_band(p, b, sh, freq, gain, q);
        if (dynamic || spectral) && design::can_be_dynamic(sh) {
            p.set(bp(b, band::RANGE), -6.0);
            p.data[data::SPECTRAL + b].set(spectral as u8 as f32);
        }
        self.pick(b, false);
        Some(b)
    }

    fn set_shape(&self, b: usize, sh: usize) {
        let p = self.p;
        p.data[data::SHAPE + b].set(sh as f32);
        p.data[data::SLOPE + b].set(design::fix_slope(sh, pq::choice(p, data::SLOPE + b, BRICKWALL + 1)) as f32);
    }

    /// EQ Sketch: the drawn line becomes a target curve, and bands fit it.
    fn finish_sketch(&self, axis: &Axis, path: &[Pos2], freqs: &[f32]) {
        let p = self.p;
        self.v.ui.data_mut(|d| d.insert_temp(self.uid("sketch-on"), false));
        if path.len() < 3 {
            return;
        }
        let mut pts = path.to_vec();
        pts.sort_by(|a, b| a.x.total_cmp(&b.x));
        let target: Vec<f32> = freqs
            .iter()
            .map(|&f| {
                let x = axis.x(f);
                if x < pts[0].x || x > pts[pts.len() - 1].x {
                    return 0.0;
                }
                let i = pts.partition_point(|q| q.x < x).clamp(1, pts.len() - 1);
                let (a, b) = (pts[i - 1], pts[i]);
                let t = if b.x > a.x { (x - a.x) / (b.x - a.x) } else { 0.0 };
                axis.db(a.y + (b.y - a.y) * t)
            })
            .collect();
        let free = (0..BANDS).filter(|&b| !pq::used(p, b)).count();
        let bands = fit::fit(freqs, &target, free.min(12), 0.4);
        self.add_fitted(&bands);
    }

    fn add_fitted(&self, bands: &[fit::Fitted]) {
        let p = self.p;
        let mut mask = 0;
        for f in bands {
            let Some(b) = pq::free_band(p) else { break };
            pq::create_band(p, b, f.shape, f.freq, f.gain / p.get(pq::SCALE).max(0.01), f.q);
            mask |= 1 << b;
        }
        self.set_sel(mask);
    }

    // -----------------------------------------------------------------------
    // Analyzer
    // -----------------------------------------------------------------------

    /// The spectra on `freqs` (pre, post, SC, dB), updated once a frame.
    fn spectra(&self, freqs: &[f32], sr: f32) -> Option<[Vec<f32>; 3]> {
        let (v, p) = (self.v, self.p);
        let show = [self.on(data::AN_PRE), self.on(data::AN_POST), self.on(data::AN_SC)];
        let matching = self.on(data::MATCH);
        if !show.iter().any(|&s| s) && !matching {
            return None;
        }
        let n = pq::AN_RESOLUTIONS[pq::choice(p, data::AN_RES, 4)].1;
        let fall = 20.0 / pq::AN_SPEEDS[pq::choice(p, data::AN_SPEED, 5)].1;
        let tilt = pq::AN_TILTS[pq::choice(p, data::AN_TILT, 5)];
        let freeze = self.on(data::AN_FREEZE);
        let (now, dt) = v.ui.input(|i| (i.time, i.stable_dt.min(0.1)));
        let floor = -150.0;
        let id = self.uid("spectra");
        let out = v.ui.data_mut(|d| {
            let st = d.get_temp_mut_or_default::<Spectra>(id);
            if st.n != n {
                st.n = n;
                st.window = (0..n).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos()).collect();
                st.re = vec![0.0; n];
                st.im = vec![0.0; n];
            }
            for k in 0..3 {
                if st.shown[k].len() != freqs.len() {
                    st.shown[k] = vec![floor; freqs.len()];
                    st.power[k] = vec![0.0; freqs.len()];
                }
            }
            if now != st.last {
                st.last = now;
                let bin_hz = sr / n as f32;
                // A full-scale sine reads 0 dB (Hann's coherent gain is ½).
                let norm = 2.0 / (n as f32 * 0.5);
                for (ring, &shown) in show.iter().enumerate() {
                    if !shown && !(matching && ring != pq::RING_POST) {
                        continue;
                    }
                    let write = p.viz[viz::RING_WRITE + ring].get() as usize;
                    let base = viz::RINGS + ring * pq::RTA_LEN;
                    for k in 0..n {
                        let at = (write + pq::RTA_LEN - n + k) % pq::RTA_LEN;
                        st.re[k] = p.viz[base + at].get() * st.window[k];
                        st.im[k] = 0.0;
                    }
                    super::monitor::fft(&mut st.re, &mut st.im);
                    let power = |k: usize| (st.re[k] * st.re[k] + st.im[k] * st.im[k]) * norm * norm;
                    for (c, &f) in freqs.iter().enumerate() {
                        // The column's span: halfway to its neighbours.
                        let (fa, fb) = (f / 1.016, f * 1.016);
                        let (b0, b1) = ((fa / bin_hz).ceil() as usize, ((fb / bin_hz).floor() as usize).min(n / 2 - 1));
                        let pw = if f >= sr * 0.5 {
                            0.0
                        } else if b1 < b0 {
                            // Narrower than a bin: between the two nearest.
                            let x = f / bin_hz;
                            let i = (x as usize).clamp(1, n / 2 - 2);
                            let t = x - i as f32;
                            power(i) * (1.0 - t) + power(i + 1) * t
                        } else {
                            (b0..=b1).map(power).fold(0.0, f32::max)
                        };
                        st.power[ring][c] = pw;
                    }
                    // A touch of smoothing across columns, as Pro-Q's line is.
                    for (c, &f) in freqs.iter().enumerate() {
                        let (a, b) = (c.saturating_sub(1), (c + 1).min(freqs.len() - 1));
                        let pw = 0.25 * st.power[ring][a] + 0.5 * st.power[ring][c] + 0.25 * st.power[ring][b];
                        let db = (10.0 * (pw + 1e-20).log10() + tilt * (f / 1000.0).log2()).max(floor);
                        let shown = &mut st.shown[ring][c];
                        *shown = if db > *shown {
                            db
                        } else if freeze {
                            *shown
                        } else {
                            (*shown - fall * dt).max(db)
                        };
                    }
                }
            }
            [st.shown[0].clone(), st.shown[1].clone(), st.shown[2].clone()]
        });
        Some(out)
    }

    fn grab_state(&self) -> Grab {
        self.v.ui.data(|d| d.get_temp::<Grab>(self.uid("grab"))).unwrap_or_default()
    }

    /// Resting the pointer on the spectrum for a second freezes it.
    fn grab_timer(&self, q: Pos2, eligible: bool, shown: &Option<[Vec<f32>; 3]>) {
        let v = self.v;
        if !self.on(data::AN_GRAB) || !(self.on(data::AN_POST) || self.on(data::AN_PRE)) {
            return;
        }
        let now = v.ui.input(|i| i.time);
        let id = self.uid("grab");
        let mut g = self.grab_state();
        if !eligible {
            if g.frozen.is_none() {
                g.rest = None;
            }
        } else if g.frozen.is_none() {
            match g.rest {
                Some((at, since)) if at.distance(q) < 4.0 * v.z => {
                    if now - since > 1.0
                        && let Some(sh) = shown
                    {
                        g.frozen = Some(if self.on(data::AN_POST) { sh[1].clone() } else { sh[0].clone() });
                    } else {
                        v.ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                    }
                }
                _ => g.rest = Some((q, now)),
            }
        } else if let Some((at, _)) = g.rest
            && at.distance(q) > 60.0 * v.z
        {
            g = Grab::default();
        }
        v.ui.data_mut(|d| d.insert_temp(id, g));
    }

    fn end_grab(&self) {
        self.v.ui.data_mut(|d| d.insert_temp(self.uid("grab"), Grab::default()));
    }

    fn piano(&self, painter: &egui::Painter, axis: &Axis) {
        let z = self.z();
        let rect = axis.rect;
        let (y0, y1) = (rect.bottom() - 26.0 * z, rect.bottom() - 13.0 * z);
        for note in 12..=135 {
            let f = 440.0 * 2f32.powf((note as f32 - 69.0) / 12.0);
            let (x0, x1) = (axis.x(f * 2f32.powf(-1.0 / 24.0)), axis.x(f * 2f32.powf(1.0 / 24.0)));
            if x1 < rect.left() || x0 > rect.right() {
                continue;
            }
            let black = matches!(note % 12, 1 | 3 | 6 | 8 | 10);
            let r = Rect::from_min_max(pos2(x0, y0), pos2(x1, if black { y0 + (y1 - y0) * 0.6 } else { y1 }));
            if black {
                painter.rect_filled(r, 0.0, theme::TEXT_FAINT);
            } else {
                painter.rect_stroke(r, 0.0, Stroke::new(0.6 * z, alpha(theme::TEXT_FAINT, 0.7)), StrokeKind::Inside);
            }
            if note % 12 == 0 && x1 - x0 > 2.0 * z {
                painter.text(pos2((x0 + x1) * 0.5, y0 - 1.0 * z), Align2::CENTER_BOTTOM, format!("C{}", note / 12 - 1), theme::font(6.5 * z), theme::TEXT_FAINT);
            }
        }
    }

    // -----------------------------------------------------------------------
    // EQ Match
    // -----------------------------------------------------------------------

    fn matching(&self) -> Matching {
        self.v.ui.data(|d| d.get_temp::<Matching>(self.uid("match"))).unwrap_or_default()
    }

    fn set_matching(&self, m: Matching) {
        self.p.data[data::MATCH].set(m.open as u8 as f32);
        self.v.ui.data_mut(|d| d.insert_temp(self.uid("match"), m));
    }

    fn snapshot(&self) -> (Vec<f32>, Vec<f32>) {
        let p = self.p;
        ((0..pq::PARAM_COUNT).map(|i| p.get(i)).collect(), p.data.iter().map(|d| d.get()).collect())
    }

    fn restore(&self, snap: &(Vec<f32>, Vec<f32>)) {
        let p = self.p;
        for (i, &v) in snap.0.iter().enumerate() {
            p.set(i, v);
        }
        for (d, &v) in p.data.iter().zip(&snap.1).take(data::MODE) {
            d.set(v);
        }
    }

    /// The reference against the input, smoothed, level-neutral, ±12 dB.
    fn match_difference(m: &Matching, freqs: &[f32], sr: f32) -> Option<Vec<f32>> {
        if m.frames_in < 10 || m.frames_ref < 10 || m.input.len() != freqs.len() || m.reference.len() != freqs.len() {
            return None;
        }
        let top = (sr * 0.5).min(20_000.0);
        let raw: Vec<f32> = (0..freqs.len())
            .map(|c| {
                let (a, b) = (m.reference[c] / m.frames_ref as f64, m.input[c] / m.frames_in as f64);
                if a < 1e-14 || b < 1e-14 || freqs[c] < 20.0 || freqs[c] > top { f32::NAN } else { (10.0 * (a / b).log10()) as f32 }
            })
            .collect();
        // A sixth of an octave each way.
        let half = (COLS as f32 / (F_HI / F_LO).log2() / 6.0).round() as usize;
        let smooth: Vec<f32> = (0..raw.len())
            .map(|c| {
                let vals: Vec<f32> = raw[c.saturating_sub(half)..(c + half + 1).min(raw.len())].iter().copied().filter(|v| v.is_finite()).collect();
                if vals.is_empty() || !raw[c].is_finite() { f32::NAN } else { vals.iter().sum::<f32>() / vals.len() as f32 }
            })
            .collect();
        let mid: Vec<f32> = smooth.iter().zip(freqs).filter(|(v, f)| v.is_finite() && (100.0..10_000.0).contains(*f)).map(|(v, _)| *v).collect();
        if mid.is_empty() {
            return None;
        }
        let mean = mid.iter().sum::<f32>() / mid.len() as f32;
        Some(smooth.iter().map(|v| if v.is_finite() { (v - mean).clamp(-12.0, 12.0) } else { 0.0 }).collect())
    }

    fn match_panel(&self, painter: &egui::Painter, axis: &Axis, freqs: &[f32], xs: &[f32], sr: f32) {
        let (v, z) = (self.v, self.z());
        let mut m = self.matching();
        if !m.open {
            return;
        }
        // Gather this frame's spectra.
        let id = self.uid("spectra");
        let powers = v.ui.data(|d| d.get_temp::<Spectra>(id)).map(|s| s.power);
        if let Some(pw) = powers {
            let frame = v.ui.input(|i| i.time);
            let last = v.ui.data(|d| d.get_temp::<f64>(self.uid("match-frame"))).unwrap_or(-1.0);
            if frame != last && pw[pq::RING_PRE].len() == freqs.len() {
                v.ui.data_mut(|d| d.insert_temp(self.uid("match-frame"), frame));
                let loud = |x: &[f32]| x.iter().any(|&v| v > 1e-9);
                if loud(&pw[pq::RING_PRE]) {
                    if m.input.len() != freqs.len() {
                        m.input = vec![0.0; freqs.len()];
                    }
                    for (a, &b) in m.input.iter_mut().zip(&pw[pq::RING_PRE]) {
                        *a += b as f64;
                    }
                    m.frames_in += 1;
                }
                if !m.captured && loud(&pw[pq::RING_SC]) {
                    if m.reference.len() != freqs.len() {
                        m.reference = vec![0.0; freqs.len()];
                    }
                    for (a, &b) in m.reference.iter_mut().zip(&pw[pq::RING_SC]) {
                        *a += b as f64;
                    }
                    m.frames_ref += 1;
                }
            }
        }
        let diff = Self::match_difference(&m, freqs, sr);
        if let Some(d) = &diff {
            let pts: Vec<Pos2> = xs.iter().zip(d).map(|(&x, &db)| pos2(x, axis.y(db))).collect();
            dashed(painter, &pts, Stroke::new(1.5 * z, alpha(Color32::from_rgb(232, 96, 96), 0.9)));
        }

        // The panel.
        let r = Rect::from_min_size(axis.rect.left_top() + vec2(10.0, 10.0) * z, vec2(250.0, 104.0) * z);
        painter.rect_filled(r, 4.0 * z, alpha(theme::NODE_BODY, 0.96));
        painter.rect_stroke(r, 4.0 * z, Stroke::new(1.0 * z, theme::PANEL_BORDER), StrokeKind::Inside);
        let o = (r.min - v.min) / z;
        let at = |x: f32, y: f32| (o.x + x, o.y + y);
        let (tx, ty) = at(10.0, 12.0);
        v.text(tx, ty, Align2::LEFT_CENTER, "EQ MATCH", v.font(8.5), theme::TEXT);
        let status = format!("input {:.0}s · reference {:.0}s", m.frames_in as f32 / 30.0, m.frames_ref as f32 / 30.0);
        v.text(o.x + 240.0, ty, Align2::RIGHT_CENTER, status, v.font(7.5), theme::TEXT_FAINT);
        let (x0, y0) = at(10.0, 24.0);
        let mut changed = false;
        if self.chip("m-sc", v.r(x0, y0, x0 + 70.0, y0 + 16.0), "SC INPUT", !m.captured, self.accent, "Reference: what plays into the SC input").clicked() && m.captured {
            m.captured = false;
            m.reference.clear();
            m.frames_ref = 0;
            changed = true;
        }
        if self.chip("m-cap", v.r(x0 + 74.0, y0, x0 + 150.0, y0 + 16.0), "CAPTURE", m.captured, self.accent, "Keep what the input has played so far as the reference, then listen again for the part to match").clicked() && m.frames_in > 0 {
            m.reference = m.input.clone();
            m.frames_ref = m.frames_in;
            m.input = vec![0.0; freqs.len()];
            m.frames_in = 0;
            m.captured = true;
            changed = true;
        }
        if self.chip("m-reset", v.r(x0 + 154.0, y0, x0 + 230.0, y0 + 16.0), "RESET", false, self.accent, "Listen to the input afresh").clicked() {
            m.input.clear();
            m.frames_in = 0;
            changed = true;
        }
        let (x1, y1) = at(10.0, 50.0);
        v.text(x1, y1 + 8.0, Align2::LEFT_CENTER, "DETAIL", v.font(7.5), theme::TEXT_FAINT);
        let slider = v.r(x1 + 44.0, y1 + 2.0, x1 + 200.0, y1 + 14.0);
        let sresp = v.ui.interact(slider, self.uid("m-detail"), Sense::click_and_drag()).on_hover_text("How many bands the match may use");
        if (sresp.dragged() || sresp.clicked())
            && let Some(q) = sresp.interact_pointer_pos()
        {
            m.detail = ((q.x - slider.left()) / slider.width()).clamp(0.0, 1.0);
            changed = true;
        }
        let max_bands = 2 + (m.detail * 14.0).round() as usize;
        painter.rect_filled(slider, 3.0 * z, theme::INSET);
        painter.rect_filled(Rect::from_min_max(slider.min, pos2(slider.left() + slider.width() * m.detail, slider.bottom())), 3.0 * z, alpha(self.accent, 0.6));
        v.text(x1 + 206.0, y1 + 8.0, Align2::LEFT_CENTER, format!("{max_bands}"), v.mono(8.0), theme::TEXT_DIM);
        let (x2, y2) = at(10.0, 76.0);
        let ready = diff.is_some();
        if self.chip("m-go", v.r(x2, y2, x2 + 70.0, y2 + 18.0), "MATCH", false, self.accent, "Fit bands to the difference (the red line)").clicked()
            && ready
            && let Some(d) = &diff
        {
            if let Some(snap) = &m.snapshot {
                self.restore(snap);
            } else {
                m.snapshot = Some(self.snapshot());
            }
            let free = (0..BANDS).filter(|&b| !pq::used(self.p, b)).count();
            let range: Vec<usize> = (0..freqs.len()).filter(|&c| freqs[c] >= 20.0 && freqs[c] <= (sr * 0.5).min(20_000.0)).collect();
            if let (Some(&a), Some(&b)) = (range.first(), range.last()) {
                let bands = fit::fit(&freqs[a..=b], &d[a..=b], max_bands.min(free), 0.5);
                self.add_fitted(&bands);
            }
            changed = true;
        }
        if self.chip("m-ok", v.r(x2 + 74.0, y2, x2 + 150.0, y2 + 18.0), "FINISH", false, self.accent, "Keep the matched bands").clicked() {
            m = Matching::default();
            changed = true;
        }
        if self.chip("m-no", v.r(x2 + 154.0, y2, x2 + 230.0, y2 + 18.0), "CANCEL", false, self.accent, "Back to the bands as they were").clicked() {
            if let Some(snap) = &m.snapshot {
                self.restore(snap);
            }
            m = Matching::default();
            changed = true;
        }
        if !ready {
            v.text(o.x + 240.0, y2 + 9.0, Align2::RIGHT_CENTER, "", v.font(7.0), theme::TEXT_FAINT);
        }
        let _ = changed;
        self.set_matching(m);
    }

    /// The output meter right of the display, −60…+6 dBFS, with the peak.
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
        let norm = |lin: f32| ((gain_to_db(lin) + 60.0) / 66.0).clamp(0.0, 1.0);
        let (x0, x1) = (GRAPH[2] + 8.0, W - PAD);
        let half = (x1 - x0 - 3.0) / 2.0;
        for (ch, state) in states.iter().enumerate() {
            let left = x0 + ch as f32 * (half + 3.0);
            let r = v.r(left, GRAPH[1] + 14.0, left + half, GRAPH[3]);
            v.painter.rect_stroke(r, 2.0 * z, Stroke::new(1.0 * z, theme::CONTROL), StrokeKind::Inside);
            let level = norm(state.level);
            let db = gain_to_db(state.level);
            let col = if db > -0.1 {
                theme::METER_RED
            } else if db > -9.0 {
                theme::METER_YELLOW
            } else {
                theme::METER_GREEN
            };
            if level > 0.0 {
                v.painter.rect_filled(Rect::from_min_max(pos2(r.left() + 1.5 * z, r.bottom() - r.height() * level), r.max - vec2(1.5, 0.0) * z), 1.0 * z, col);
            }
            let py = r.bottom() - r.height() * norm(state.peak);
            v.painter.hline(r.x_range(), py, Stroke::new(1.0 * z, theme::TEXT_DIM));
        }
        let peak = gain_to_db(states[0].peak.max(states[1].peak));
        let text = if peak > -99.0 { format!("{peak:+.1}") } else { "—".to_owned() };
        v.text((x0 + x1) * 0.5, GRAPH[1] + 6.0, Align2::CENTER_CENTER, text, v.mono(7.5), if peak > 0.0 { theme::METER_RED } else { theme::TEXT_DIM });
    }

    // -----------------------------------------------------------------------
    // The selected band's strip
    // -----------------------------------------------------------------------

    fn strip(&self) {
        let (v, p) = (self.v, self.p);
        let Some(b) = self.primary() else {
            let hint = "Click the display for a band (its place picks the shape: cuts at the ends, shelves near them, a notch along the bottom) · drag the yellow curve · drag on the background to select";
            v.text(GRAPH[0], STRIP_Y + 12.0, Align2::LEFT_CENTER, hint, v.font(8.0), theme::TEXT_FAINT);
            let hint2 = "Alt-click: dynamic band · Alt+Shift-click: spectral · Alt-click a dot: bypass · Cmd+Alt-click: shape · Alt+Shift-click: slope · Cmd-drag: Q";
            v.text(GRAPH[0], STRIP_Y + 30.0, Align2::LEFT_CENTER, hint2, v.font(8.0), theme::TEXT_FAINT);
            return;
        };
        let color = band_color(b);
        let sh = pq::shape(p, b);
        let slope = pq::slope(p, b);
        let y0 = STRIP_Y;
        let y1 = STRIP_Y + 20.0;
        let selected = self.selected();

        // Bypass.
        let on = pq::on(p, data::ON + b);
        if self.chip(("bypass", b), v.r(GRAPH[0], y0, GRAPH[0] + 26.0, y1), "⏻", on, color, "Band on or bypassed (Alt-click its dot)").clicked() {
            for &k in &selected {
                p.data[data::ON + k].set(if on { 0.0 } else { 1.0 });
            }
        }
        // Shapes, as their own curves.
        for (s, name) in SHAPE_NAMES.iter().enumerate() {
            let x = 70.0 + s as f32 * 27.0;
            let r = v.r(x, y0, x + 25.0, y1);
            let resp = v.ui.interact(r, self.uid(("shape", s)), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(*name);
            let lit = s == sh;
            v.painter.rect_filled(r, 3.0 * v.z, if lit { theme::CONTROL_HI } else if resp.hovered() { theme::CONTROL } else { theme::INSET });
            shape_icon(v.painter, r.shrink2(vec2(3.0, 4.0) * v.z), s, if lit { color } else { theme::TEXT_DIM }, v.z);
            if resp.clicked() {
                for &k in &selected {
                    self.set_shape(k, s);
                }
            }
        }
        // Slopes the shape allows.
        let slopes: Vec<usize> = (0..=BRICKWALL).filter(|&i| design::slope_ok(sh, i)).collect();
        if design::has_slope(sh) {
            let r = v.r(344.0, y0, 344.0 + 25.0 * slopes.len() as f32, y1);
            let cur = slopes.iter().position(|&i| i == slope).unwrap_or(0);
            let picked = segmented(v, self.uid(("slope", b)), r, slopes.len(), cur, color, |k, rr, col| {
                let label = if slopes[k] == BRICKWALL { "BW".to_owned() } else { SLOPES[slopes[k]].to_string() };
                v.painter.text(rr.center(), Align2::CENTER_CENTER, label, v.font(7.5), col);
                "Slope, dB/octave (BW: brickwall)"
            });
            if let Some(k) = picked {
                for &kb in &selected {
                    p.data[data::SLOPE + kb].set(design::fix_slope(pq::shape(p, kb), slopes[k]) as f32);
                }
            }
        }
        // Placement.
        let pr = v.r(600.0, y0, 700.0, y1);
        let cur = pq::place(p, b);
        let picked = segmented(v, self.uid(("place", b)), pr, 5, cur, color, |k, rr, col| {
            v.painter.text(rr.center(), Align2::CENTER_CENTER, PLACE_NAMES[k].0, v.font(7.5), col);
            PLACE_NAMES[k].1
        });
        if let Some(k) = picked {
            for &kb in &selected {
                p.data[data::PLACE + kb].set(k as f32);
            }
        }
        // Split: a copy on the other side (L → R, M → S).
        if self.chip(("split", b), v.r(704.0, y0, 736.0, y1), "SPLIT", false, color, "Split into left and right (or mid and side) bands").clicked() {
            self.split(b);
        }
        // Band navigation.
        let order = self.band_order();
        let idx = order.iter().position(|&k| k == b).unwrap_or(0);
        for (rect, step, glyph) in [(v.r(740.0, y0, 756.0, y1), order.len() - 1, "‹"), (v.r(806.0, y0, 822.0, y1), 1, "›")] {
            if self.chip(("nav", glyph), rect, glyph, false, color, "Previous / next band, by frequency").clicked() && !order.is_empty() {
                self.pick(order[(idx + step) % order.len()], false);
            }
        }
        v.text(781.0, y0 + 10.0, Align2::CENTER_CENTER, format!("BAND {}", b + 1), v.font(8.0), color);
        // Solo: held.
        let solo_rect = v.r(826.0, y0, 848.0, y1);
        let sresp = v.ui.interact(solo_rect, self.uid("solo"), Sense::click_and_drag()).on_hover_text("Hold to hear what the band works on; drag sideways for frequency (Cmd: Q)");
        let held = sresp.is_pointer_button_down_on();
        p.data[data::SOLO].set(if held { b as f32 } else if pq::band_setting(p, data::SOLO) == Some(b) { -1.0 } else { p.data[data::SOLO].get() });
        if held && sresp.dragged() {
            let d = sresp.drag_delta();
            if v.ui.input(|i| i.modifiers.command) {
                p.set(bp(b, band::Q), (p.get(bp(b, band::Q)) * (-d.y * 0.01).exp()).clamp(pq::Q_RANGE.0, pq::Q_RANGE.1));
            } else {
                p.set(bp(b, band::FREQ), (p.get(bp(b, band::FREQ)) * (d.x * 0.006).exp()).clamp(pq::FREQ_RANGE.0, pq::FREQ_RANGE.1));
            }
        }
        v.painter.rect_filled(solo_rect, 3.0 * v.z, if held { theme::SOLO } else if sresp.hovered() { theme::CONTROL_HI } else { theme::CONTROL });
        v.painter.text(solo_rect.center(), Align2::CENTER_CENTER, "S", v.font(9.0), if held { theme::INSET } else { theme::TEXT_DIM });
        if self.chip(("delete", b), v.r(852.0, y0, GRAPH[2], y1), "×", false, color, "Delete the selected bands").clicked() {
            for &k in &selected {
                pq::delete_band(p, k);
            }
            return;
        }

        // Knobs.
        let has_gain = design::has_gain(sh);
        let knob = |i: usize, cx: f32, label: &str, dimmed: bool| {
            let cell = KnobCell { cx, row_y: KNOBS_Y, label, dimmed, accent: color, param: ParamRef::Module(i) };
            let old = p.get(i);
            if let Some(value) = knob_cell(v, self.uid(("knob", i)), pq::spec(i), old, cell) {
                p.set(i, value);
                self.apply_to_selected(b, i, old, value);
            }
        };
        knob(bp(b, band::FREQ), 64.0, "FREQ", false);
        knob(bp(b, band::GAIN), 122.0, "GAIN", !has_gain);
        knob(bp(b, band::Q), 180.0, "Q", !design::has_q(sh, slope));
        // The gain knob's ring: the range and the live change.
        if has_gain && pq::dynamic(p, b) {
            let spec = pq::spec(bp(b, band::GAIN));
            let c = v.p(122.0, KNOBS_Y + 19.0);
            let r = 16.0 * v.z;
            let g = p.get(bp(b, band::GAIN));
            let to_a = |db: f32| widgets::KNOB_START + widgets::KNOB_SWEEP * spec.to_norm(db);
            let (a0, a1) = (to_a(g), to_a(g + p.get(bp(b, band::RANGE))));
            v.painter.add(Shape::line(widgets::arc(c, r, a0.min(a1), a0.max(a1)), Stroke::new(2.0 * v.z, alpha(theme::METER_RED, 0.8))));
            let d = p.viz[viz::DYN + b].get();
            if d.abs() > 0.05 {
                let (b0, b1) = (to_a(g), to_a(g + d));
                v.painter.add(Shape::line(widgets::arc(c, r - 3.0 * v.z, b0.min(b1), b0.max(b1)), Stroke::new(2.5 * v.z, YELLOW)));
            }
        }

        // Dynamics.
        let x = 214.0;
        if !design::can_be_dynamic(sh) {
            v.text(x + 10.0, KNOBS_Y + 20.0, Align2::LEFT_CENTER, "Dynamics need a bell, a shelf or a tilt", v.font(8.0), theme::TEXT_FAINT);
            return;
        }
        let dynamic = pq::dynamic(p, b);
        knob(bp(b, band::RANGE), x + 24.0, "RANGE", false);
        if !dynamic {
            v.text(x + 58.0, KNOBS_Y + 12.0, Align2::LEFT_CENTER, "Turn RANGE (or Alt-drag the dot's handle) to make the band dynamic:", v.font(8.0), theme::TEXT_FAINT);
            v.text(x + 58.0, KNOBS_Y + 28.0, Align2::LEFT_CENTER, "below 0 it turns loud moments down, above 0 it lifts them.", v.font(8.0), theme::TEXT_FAINT);
            if self.chip(("make-dyn", b), v.r(x + 58.0, KNOBS_Y + 38.0, x + 138.0, KNOBS_Y + 54.0), "DYNAMIC", false, color, "Make the band dynamic (range −6 dB)").clicked() {
                for &k in &selected {
                    if design::can_be_dynamic(pq::shape(p, k)) {
                        p.set(bp(k, band::RANGE), -6.0);
                    }
                }
            }
            if self.chip(("make-spec", b), v.r(x + 142.0, KNOBS_Y + 38.0, x + 222.0, KNOBS_Y + 54.0), "SPECTRAL", false, color, "Make the band spectral (range −6 dB)").clicked() {
                for &k in &selected {
                    if design::can_be_dynamic(pq::shape(p, k)) {
                        p.set(bp(k, band::RANGE), -6.0);
                        p.data[data::SPECTRAL + k].set(1.0);
                    }
                }
            }
            return;
        }
        let auto = pq::on(p, data::AUTO + b);
        knob(bp(b, band::THRESHOLD), x + 82.0, "THRESH", auto);
        // The trigger's level round the threshold, and the threshold in use.
        let spectral = pq::spectral(p, b);
        if !spectral {
            let level = p.viz[viz::LEVEL + b].get();
            let spec = pq::spec(bp(b, band::THRESHOLD));
            let c = v.p(x + 82.0, KNOBS_Y + 19.0);
            let n = spec.to_norm(level);
            if level > -100.0 && n > 0.0 {
                let a0 = widgets::KNOB_START;
                v.painter.add(Shape::line(widgets::arc(c, 16.0 * v.z, a0, a0 + widgets::KNOB_SWEEP * n), Stroke::new(2.0 * v.z, alpha(color, 0.75))));
            }
            if auto {
                let a = widgets::KNOB_START + widgets::KNOB_SWEEP * spec.to_norm(p.viz[viz::THRESHOLD + b].get());
                let dir = vec2(a.cos(), a.sin());
                v.painter.line_segment([c + dir * 13.0 * v.z, c + dir * 19.0 * v.z], Stroke::new(1.6 * v.z, YELLOW));
            }
        }
        if self.chip(("auto", b), v.r(x + 101.0, KNOBS_Y + 8.0, x + 119.0, KNOBS_Y + 22.0), "A", auto, color, "Automatic threshold: rides over the trigger's average level").clicked() {
            for &k in &selected {
                p.data[data::AUTO + k].set(if auto { 0.0 } else { 1.0 });
            }
        }
        knob(bp(b, band::ATTACK), x + 150.0, "ATTACK", false);
        knob(bp(b, band::RELEASE), x + 206.0, "RELEASE", false);
        // Trigger.
        let tx = x + 240.0;
        let ext = pq::on(p, data::SC_EXT + b);
        let free = pq::on(p, data::SC_FREE + b);
        let trig = |i: usize, rect: Rect, items: [(&str, &'static str); 2]| {
            let cur = pq::on(p, i) as usize;
            let picked = segmented(v, self.uid(("trig", i)), rect, 2, cur, color, |k, rr, col| {
                v.painter.text(rr.center(), Align2::CENTER_CENTER, items[k].0, v.font(7.5), col);
                items[k].1
            });
            if let Some(k) = picked {
                for &kb in &selected {
                    p.data[i - b + kb].set(k as f32);
                }
            }
        };
        trig(data::SC_EXT + b, v.r(tx, KNOBS_Y - 6.0, tx + 70.0, KNOBS_Y + 10.0), [("INT", "Triggered by the node's input"), ("EXT", "Triggered by the SC input")]);
        trig(data::SC_FREE + b, v.r(tx, KNOBS_Y + 14.0, tx + 70.0, KNOBS_Y + 30.0), [("BAND", "Triggered by the band's own range"), ("FREE", "Triggered by the range between SC LO and SC HI")]);
        let aud = v.r(tx, KNOBS_Y + 34.0, tx + 70.0, KNOBS_Y + 50.0);
        let aresp = v.ui.interact(aud, self.uid("audition"), Sense::click_and_drag()).on_hover_text("Hold to hear the trigger");
        let held = aresp.is_pointer_button_down_on() && !spectral;
        p.data[data::AUDITION].set(if held { b as f32 } else if pq::band_setting(p, data::AUDITION) == Some(b) { -1.0 } else { p.data[data::AUDITION].get() });
        v.painter.rect_filled(aud, 3.0 * v.z, if held { color } else if aresp.hovered() { theme::CONTROL_HI } else { theme::CONTROL });
        v.painter.text(aud.center(), Align2::CENTER_CENTER, "AUDITION", v.font(7.5), if held { theme::INSET } else { theme::TEXT_DIM });
        knob(bp(b, band::SC_LO), tx + 100.0, "SC LO", !free);
        knob(bp(b, band::SC_HI), tx + 156.0, "SC HI", !free);
        let _ = ext;
        // Spectral.
        let sx = tx + 190.0;
        if self.chip(("spectral", b), v.r(sx, KNOBS_Y - 6.0, sx + 70.0, KNOBS_Y + 10.0), "SPECTRAL", spectral, color, "Spectral dynamics: only the frequencies that cross the threshold move").clicked() {
            for &k in &selected {
                p.data[data::SPECTRAL + k].set(if spectral { 0.0 } else { 1.0 });
            }
        }
        if self.chip(("tilt", b), v.r(sx, KNOBS_Y + 14.0, sx + 70.0, KNOBS_Y + 30.0), "TILT", pq::on(p, data::SPEC_TILT + b), color, "Spectral tilt: +3 dB/octave on the trigger, so highs trigger a little sooner").clicked() {
            self.flip(data::SPEC_TILT + b);
        }
        knob(bp(b, band::DENSITY), sx + 100.0, "DENSITY", !spectral);
        if spectral {
            let act = p.viz[viz::DYN + b].get();
            v.text(sx, KNOBS_Y + 42.0, Align2::LEFT_CENTER, format!("{act:+.1} dB"), v.mono(8.0), theme::TEXT_DIM);
        }
    }

    /// A knob moved on the primary band moves the other selected bands as
    /// much: frequency and Q as a ratio, dB as a difference.
    fn apply_to_selected(&self, primary: usize, i: usize, old: f32, new: f32) {
        let p = self.p;
        let k = i - primary * band::COUNT;
        if k >= band::COUNT {
            return;
        }
        for b in self.selected() {
            if b == primary {
                continue;
            }
            let j = bp(b, k);
            let spec = pq::spec(j);
            let v = match k {
                band::FREQ | band::Q | band::SC_LO | band::SC_HI if old > 0.0 => p.get(j) * new / old,
                _ => p.get(j) + (new - old),
            };
            p.set(j, v.clamp(spec.min, spec.max));
        }
    }

    /// Bands in display order (by frequency).
    fn band_order(&self) -> Vec<usize> {
        let p = self.p;
        let mut order: Vec<usize> = (0..BANDS).filter(|&b| pq::used(p, b)).collect();
        order.sort_by(|a, b| p.get(bp(*a, band::FREQ)).total_cmp(&p.get(bp(*b, band::FREQ))));
        order
    }

    /// Split: the band takes one side, a copy the other.
    fn split(&self, b: usize) {
        let p = self.p;
        let Some(c) = pq::free_band(p) else { return };
        let (mine, theirs) = match pq::place(p, b) {
            pq::MID | pq::SIDE => (pq::MID, pq::SIDE),
            _ => (pq::LEFT, pq::RIGHT),
        };
        for k in 0..band::COUNT {
            p.set(bp(c, k), p.get(bp(b, k)));
        }
        for base in [data::USED, data::ON, data::SHAPE, data::SLOPE, data::AUTO, data::SC_EXT, data::SC_FREE, data::SPECTRAL, data::SPEC_TILT] {
            p.data[base + c].set(p.data[base + b].get());
        }
        p.data[data::PLACE + b].set(mine as f32);
        p.data[data::PLACE + c].set(theirs as f32);
        self.set_sel((1 << b) | (1 << c));
    }

    // -----------------------------------------------------------------------
    // Bottom bar
    // -----------------------------------------------------------------------

    fn bottom_bar(&self) {
        let (v, p) = (self.v, self.p);
        let (y0, y1) = (BAR_Y, BAR_Y + 20.0);
        let accent = self.accent;
        self.label(GRAPH[0], y0 + 10.0, "MODE");
        let modes = [
            ("ZERO LATENCY", "Analog-matched filters, no latency"),
            ("NATURAL PHASE", "Analog magnitude and phase, at twice the rate (about 0.6 ms of latency)"),
            ("LINEAR PHASE", "Only the magnitude changes, phase untouched (latency: see RESOLUTION)"),
        ];
        self.seg(data::MODE, v.r(70.0, y0, 340.0, y1), &modes, accent, true);
        let mode = pq::choice(p, data::MODE, 3);
        let spectral = (0..BANDS).any(|b| pq::active(p, b) && pq::spectral(p, b));
        let res_on = mode == pq::LINEAR_PHASE || spectral;
        self.label(350.0, y0 + 10.0, "RESOLUTION");
        let res = [
            ("LOW", "Small frames: least latency, for wide bands or the mid and high range"),
            ("MED", "For most uses"),
            ("HIGH", "Better in the lows, for narrow low bands"),
            ("V.HIGH", "Finer still (linear phase only, no dynamics)"),
            ("MAX", "The finest, with the most latency and possible pre-echo (no dynamics)"),
        ];
        let cur = pq::choice(p, data::RESOLUTION, 5);
        if let Some(k) = self.seg(data::RESOLUTION, v.r(406.0, y0, 600.0, y1), &res, accent, res_on)
            && mode != pq::LINEAR_PHASE
            && k >= pq::DYNAMIC_RESOLUTIONS
        {
            // Spectral bands stop at HIGH.
            p.data[data::RESOLUTION].set(cur.min(pq::DYNAMIC_RESOLUTIONS - 1).max(k.min(pq::DYNAMIC_RESOLUTIONS - 1)) as f32);
        }
        let sr = p.viz[viz::SR].get().max(1.0);
        let latency = p.viz[viz::LATENCY].get();
        let text = if latency > 0.0 {
            format!("latency {:.1} ms", latency / sr * 1000.0)
        } else if mode == pq::ZERO_LATENCY {
            "zero latency".to_owned()
        } else {
            "no latency".to_owned()
        };
        v.text(606.0, y0 + 10.0, Align2::LEFT_CENTER, text, v.font(7.5), theme::TEXT_DIM);
        self.label(694.0, y0 + 10.0, "CHAR");
        let chars = [
            (pq::CHARACTERS[0], "Transparent"),
            (pq::CHARACTERS[1], "A mild saturation, more as the level rises"),
            (pq::CHARACTERS[2], "A warmer, tube-like saturation with even harmonics"),
        ];
        self.seg(data::CHARACTER, v.r(720.0, y0, W - PAD, y1), &chars, accent, true);

        let (y2, y3) = (BAR2_Y, BAR2_Y + 20.0);
        let mut m = self.matching();
        if self.chip("match", v.r(GRAPH[0], y2, 96.0, y3), "MATCH", m.open, accent, "EQ Match: bands that make the input sound like a reference (the SC input, or a captured part)").clicked() {
            if m.open {
                if let Some(snap) = &m.snapshot {
                    self.restore(snap);
                }
                m = Matching::default();
            } else {
                m = Matching { open: true, detail: 0.5, ..Default::default() };
            }
            self.set_matching(m);
        }
        let sketch_id = self.uid("sketch-on");
        let sketching = v.ui.data(|d| d.get_temp::<bool>(sketch_id)).unwrap_or(false);
        if self.chip("sketch", v.r(100.0, y2, 160.0, y3), "SKETCH", sketching, accent, "EQ Sketch: draw a curve across the display, and bands follow it").clicked() {
            v.ui.data_mut(|d| d.insert_temp(sketch_id, !sketching));
        }
        self.switch(data::GAIN_Q, v.r(164.0, y2, 222.0, y3), "GAIN-Q", "Gain-Q interaction: bells narrow as they gain, like analog ones");
        self.switch(data::AUTO_GAIN, v.r(290.0, y2, 362.0, y3), "AUTO GAIN", "Make up the level the curve adds or takes (an estimate from the curve)");
        self.switch(data::INVERT, v.r(366.0, y2, 390.0, y3), "Ø", "Invert the output's polarity");
        if self.on(data::AUTO_GAIN) {
            v.text(394.0, y2 + 10.0, Align2::LEFT_CENTER, format!("{:+.1}", p.viz[viz::AUTO_GAIN].get()), v.mono(7.5), theme::TEXT_DIM);
        }
        self.number_box(pq::SCALE, v.r(486.0, y2, 586.0, y3), "SCALE", "Gain scale: every band's gain and range, ×0 to ×2");
        self.number_box(pq::PAN, v.r(618.0, y2, 700.0, y3), "PAN", "Balance: left/right, or mid/side");
        self.seg(data::PAN_MS, v.r(704.0, y2, 760.0, y3), &[("L/R", "Pan between left and right"), ("M/S", "Pan between mid and side")], accent, true);
        self.number_box(pq::OUTPUT, v.r(800.0, y2, W - PAD, y3), "OUT", "Output gain");
    }

    /// "LABEL [−] value [+]": drag the value, click − and +; double-click
    /// resets.
    fn number_box(&self, i: usize, rect: Rect, label: &str, tip: &str) {
        let (v, p, z) = (self.v, self.p, self.z());
        let spec = pq::spec(i);
        v.painter.text(rect.left_center() - vec2(4.0 * z, 0.0), Align2::RIGHT_CENTER, label, theme::font(7.5 * z), theme::TEXT_FAINT);
        let bw = 14.0 * z;
        let minus = Rect::from_min_size(rect.min, vec2(bw, rect.height()));
        let plus = Rect::from_min_max(pos2(rect.right() - bw, rect.top()), rect.max);
        let mid = Rect::from_min_max(pos2(minus.right() + 2.0 * z, rect.top()), pos2(plus.left() - 2.0 * z, rect.bottom()));
        let step = (spec.max - spec.min) / 120.0;
        let mut value = p.get(i);
        for (r, sign, glyph) in [(minus, -1.0, "−"), (plus, 1.0, "+")] {
            let resp = v.ui.interact(r, self.uid(("step", i, glyph)), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand);
            v.painter.rect_filled(r, 3.0 * z, if resp.hovered() { theme::CONTROL_HI } else { theme::CONTROL });
            v.painter.text(r.center(), Align2::CENTER_CENTER, glyph, theme::font(9.0 * z), theme::TEXT_DIM);
            if resp.clicked() {
                value += sign * step;
            }
        }
        let resp = v.ui.interact(mid, self.uid(("value", i)), Sense::click_and_drag()).on_hover_cursor(CursorIcon::ResizeHorizontal).on_hover_text(tip);
        v.hit(mid, ParamRef::Module(i));
        if resp.dragged() {
            let d = resp.drag_delta();
            let fine = if v.ui.input(|i| i.modifiers.shift) { 0.2 } else { 1.0 };
            value = spec.denorm(spec.to_norm(value) + (d.x - d.y) * 0.004 * fine);
        }
        if resp.double_clicked() {
            value = spec.default;
        }
        let value = value.clamp(spec.min, spec.max);
        if value != p.get(i) {
            p.set(i, value);
        }
        v.painter.rect_filled(mid, 3.0 * z, theme::INSET);
        v.painter.text(mid.center(), Align2::CENTER_CENTER, spec.format(value), theme::mono(8.0 * z), theme::TEXT);
    }
}

/// A shape's icon: its own curve (the all pass: its phase).
fn shape_icon(painter: &egui::Painter, r: Rect, s: usize, color: Color32, z: f32) {
    let (gain, slope) = match s {
        LOW_SHELF | HIGH_SHELF | TILT_SHELF => (-12.0, 1),
        BELL => (12.0, 1),
        FLAT_TILT => (14.0, 1),
        LOW_CUT | HIGH_CUT => (0.0, 3),
        BAND_PASS => (0.0, 2),
        _ => (0.0, 1),
    };
    let a = Analog::design(&Band { shape: s, slope, freq: 600.0, gain, q: if s == NOTCH { 3.0 } else if s == BAND_PASS { 1.5 } else { 1.0 } });
    let pts: Vec<Pos2> = (0..=20)
        .map(|k| {
            let t = k as f32 / 20.0;
            let f = 30.0 * (12_000.0f32 / 30.0).powf(t);
            let v = if s == ALL_PASS { (a.phase(f as f64) as f32).sin() * 0.8 } else { (a.db(f as f64) as f32 / 16.0).clamp(-1.0, 1.0) };
            pos2(r.left() + r.width() * t, r.center().y - v * r.height() * 0.5)
        })
        .collect();
    painter.add(Shape::line(pts, Stroke::new(1.3 * z, color)));
}

/// The analyzer's most prominent peaks: (column, level).
fn peaks(levels: &[f32], freqs: &[f32], sr: f32) -> Vec<(usize, f32)> {
    let n = levels.len();
    let mut out: Vec<(usize, f32)> = (3..n.saturating_sub(3))
        .filter(|&k| freqs[k] > 20.0 && freqs[k] < sr * 0.45)
        .filter(|&k| (k - 3..=k + 3).all(|j| levels[j] <= levels[k]))
        .filter(|&k| {
            let around = levels[k.saturating_sub(10)..(k + 11).min(n)].iter().copied().fold(f32::MAX, f32::min);
            levels[k] - around >= 6.0 && levels[k] > -100.0
        })
        .map(|k| (k, levels[k]))
        .collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1));
    out.truncate(6);
    out
}

/// A peak's Q, from where it falls 3 dB.
fn peak_q(levels: &[f32], freqs: &[f32], k: usize) -> f32 {
    let top = levels[k];
    let mut lo = k;
    while lo > 0 && levels[lo - 1] > top - 3.0 {
        lo -= 1;
    }
    let mut hi = k;
    while hi + 1 < levels.len() && levels[hi + 1] > top - 3.0 {
        hi += 1;
    }
    (freqs[k] / (freqs[(hi + 1).min(levels.len() - 1)] - freqs[lo.saturating_sub(1)]).max(1e-3)).clamp(1.0, 12.0)
}

/// The nearest semitone (A 440 Hz).
fn snap_note(f: f32) -> f32 {
    let note = (69.0 + 12.0 * (f / 440.0).log2()).round();
    440.0 * 2f32.powf((note - 69.0) / 12.0)
}

/// A faint tint between a curve and a horizontal line.
fn tint(painter: &egui::Painter, pts: &[Pos2], base: f32, color: Color32) {
    use egui::epaint::{Mesh, Vertex, WHITE_UV};
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

fn diamond(painter: &egui::Painter, c: Pos2, r: f32, stroke: Stroke) {
    let pts = vec![c + vec2(0.0, -r), c + vec2(r, 0.0), c + vec2(0.0, r), c + vec2(-r, 0.0)];
    painter.add(Shape::closed_line(pts, stroke));
}
