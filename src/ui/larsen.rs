//! Panel of Larsen. Up top the presets, the keys' switches (PITCH, GATE)
//! and the strings' (POLY, LOCK). The display is the loop's field: the
//! output's spectrum on a log-frequency scale, over the band the two
//! filters let through, with the strings' harmonics rising from the
//! bottom and BODY's resonances hanging from the top: the drone settles
//! where they meet. Each string's note floats over its fundamental,
//! glowing with its loop. Then the knobs: what is inside the loop in the
//! node's colour (it changes what sustains), what comes after it in amber
//! (it only colours what comes out).

use std::f32::consts::TAU;

use egui::{Align2, Color32, Id, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};

use super::nodes::{NodeView, segmented};
use super::theme::{self, alpha, mix};
use super::{preset_menu, widgets};
use crate::audio::engine::NodeId;
use crate::audio::midi::note_hz;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::larsen::parts::{Biquad, Kind};
use crate::audio::modules::larsen::{
    self as la, BODY, COUPLING, DECAY, DRIVE, FACTORY, FEEDBACK, FREQ, GAIN, HIGHPASS, INPUT,
    INTERVAL2, INTERVAL3, LEVEL, LOWPASS, MIX, REPEATS, RING_LEN, SEND, TIME, TONE, WIDTH, data,
    viz,
};
use crate::audio::params::{format_seconds, note_label};
use crate::graph::{ModuleNode, ParamRef};

pub const SIZE: Vec2 = vec2(760.0, 400.0);
const W: f32 = SIZE.x;
const PAD: f32 = 12.0;
/// The field, and the two knob rows.
const TOP: f32 = 70.0;
const BOTTOM: f32 = 238.0;
const ROW_A: f32 = 254.0;
const ROW_B: f32 = 328.0;
/// The field's frequency range.
const F_LO: f32 = 30.0;
const F_HI: f32 = 16_000.0;
/// Its level range, in dB.
const DB_LO: f32 = -84.0;
const DB_HI: f32 = 0.0;
/// After the loop.
const AMBER: Color32 = Color32::from_rgb(232, 172, 88);
/// The three strings.
const STRING_COLORS: [Color32; 3] = [
    Color32::from_rgb(255, 122, 96),
    Color32::from_rgb(250, 204, 112),
    Color32::from_rgb(128, 200, 255),
];

/// A knob group: title, left and right edges, the row, knobs (index, label).
type Group = (
    &'static str,
    f32,
    f32,
    f32,
    &'static [(usize, &'static str)],
);
const GROUPS: [Group; 7] = [
    (
        "LOOP",
        PAD,
        246.0,
        ROW_A,
        &[(FREQ, "FREQ"), (FEEDBACK, "FEEDBACK"), (BODY, "BODY")],
    ),
    (
        "FILTER",
        258.0,
        398.0,
        ROW_A,
        &[(LOWPASS, "LOWPASS"), (HIGHPASS, "HIGHPASS")],
    ),
    (
        "REVERB",
        410.0,
        550.0,
        ROW_A,
        &[(MIX, "MIX"), (DECAY, "DECAY")],
    ),
    (
        "STRINGS",
        562.0,
        W - PAD,
        ROW_A,
        &[
            (COUPLING, "COUPLING"),
            (INTERVAL2, "INTERVAL 2"),
            (INTERVAL3, "INTERVAL 3"),
        ],
    ),
    (
        "ECHO",
        PAD,
        204.0,
        ROW_B,
        &[(SEND, "SEND"), (TIME, "TIME"), (REPEATS, "REPEATS")],
    ),
    (
        "PONG",
        312.0,
        560.0,
        ROW_B,
        &[
            (DRIVE, "DRIVE"),
            (TONE, "TONE"),
            (WIDTH, "WIDTH"),
            (GAIN, "GAIN"),
        ],
    ),
    (
        "LEVELS",
        572.0,
        W - PAD,
        ROW_B,
        &[(INPUT, "INPUT"), (LEVEL, "LEVEL")],
    ),
];

const TIPS: [(usize, &str); 19] = [
    (
        FREQ,
        "The strings' pitch. The drone settles where their harmonics meet BODY's resonances, which is rarely exactly here",
    ),
    (
        FEEDBACK,
        "Gain round the loop: low, it dies away; high, it sustains and saturates",
    ),
    (
        BODY,
        "The loop's outer delay: moving it can jump the drone to another partial.\nTuned to a note, it snaps to a whole number of its periods",
    ),
    (
        LOWPASS,
        "A low-pass inside the loop: it picks which partials sustain, not only the tone",
    ),
    (HIGHPASS, "A high-pass inside the loop"),
    (
        MIX,
        "A reverb inside the loop, dry to wet (it pulls low notes a little out of tune)",
    ),
    (DECAY, "The reverb's feedback, up to endless"),
    (SEND, "How much goes into the echo"),
    (
        TIME,
        "The time between repeats (WARP makes it three to four times shorter)",
    ),
    (
        REPEATS,
        "The echo's feedback: past 100% it swells, held back by a soft clip",
    ),
    (
        DRIVE,
        "PONG: up to +12 dB into the repeats' soft clip; each bounce comes back louder",
    ),
    (TONE, "PONG: the centre of the repeats' band-pass"),
    (
        WIDTH,
        "PONG: from every repeat in the middle to fully left and right",
    ),
    (GAIN, "PONG: the repeats' level (not what feeds back)"),
    (
        INPUT,
        "The IN port into the strings (0 dB at the middle): sound to excite the loop",
    ),
    (LEVEL, "Output level"),
    (
        COUPLING,
        "With POLY: how much each loop hears the other two, like sympathetic strings",
    ),
    (
        INTERVAL2,
        "With POLY and no keys: the second string, in semitones from FREQ",
    ),
    (
        INTERVAL3,
        "With POLY and no keys: the third string, in semitones from FREQ",
    ),
];

/// Knobs after the loop.
fn after_loop(i: usize) -> bool {
    matches!(
        i,
        SEND | TIME | REPEATS | DRIVE | TONE | WIDTH | GAIN | LEVEL
    )
}

/// The node's two summary lines: the note and where it comes from, then
/// the feedback and BODY.
pub fn readout(p: &ModuleParams) -> (String, String) {
    let note = note_label(p.viz[viz::NOTE].get().round() as i32);
    let from = if p.viz[viz::MIDI].get() > 0.5 {
        "keys"
    } else {
        "drone"
    };
    let strings = if la::flag(p, data::POLY) {
        " · 3 strings"
    } else {
        ""
    };
    (
        format!("{note} · {from}{strings}"),
        format!(
            "FB {} · BODY {}",
            la::format(p, FEEDBACK),
            la::format(p, BODY)
        ),
    )
}

/// What the panel remembers between frames: the spectrum's working
/// buffers and each column's level.
#[derive(Clone, Default)]
struct View {
    window: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    columns: Vec<f32>,
    last: f64,
}

/// Columns across the field.
const COLUMNS: usize = 240;

impl View {
    fn update(&mut self, p: &ModuleParams, now: f64) {
        let n = RING_LEN;
        if self.window.len() != n {
            self.window = (0..n)
                .map(|i| 0.5 - 0.5 * (TAU * i as f32 / (n - 1) as f32).cos())
                .collect();
            self.re = vec![0.0; n];
            self.im = vec![0.0; n];
            self.columns = vec![DB_LO; COLUMNS];
        }
        if now == self.last {
            return;
        }
        let dt = (now - self.last).clamp(0.0, 0.5) as f32;
        self.last = now;
        let write = p.viz[viz::WRITE].get() as usize;
        for k in 0..n {
            self.re[k] = p.viz[viz::RING + (write + k) % n].get() * self.window[k];
            self.im[k] = 0.0;
        }
        super::monitor::fft(&mut self.re, &mut self.im);
        let sr = p.viz[viz::SR].get().max(8000.0);
        let bin = sr / n as f32;
        // Hann's coherent gain is 0.5: a full-scale sine reads 0 dB.
        let norm = 2.0 / (n as f32 * 0.5);
        let k_up = 1.0 - (-dt / 0.05).exp();
        let k_down = 1.0 - (-dt / 0.35).exp();
        for c in 0..COLUMNS {
            let (lo, hi) = (column_hz(c as f32), column_hz(c as f32 + 1.0));
            let (b0, b1) = (
                (lo / bin).floor() as usize,
                ((hi / bin).ceil() as usize).min(n / 2 - 1),
            );
            let power = (b0.max(1)..=b1.max(b0.max(1)))
                .map(|k| self.re[k] * self.re[k] + self.im[k] * self.im[k])
                .fold(0.0, f32::max);
            let db = (10.0 * (power * norm * norm + 1e-20).log10()).max(DB_LO);
            let k = if db > self.columns[c] { k_up } else { k_down };
            self.columns[c] += (db - self.columns[c]) * k;
        }
    }
}

/// A small caption on the field, on a patch of its background so that
/// lines pass behind it.
fn caption(painter: &egui::Painter, pos: Pos2, align: Align2, text: &str, z: f32) {
    let galley = painter.layout_no_wrap(text.to_owned(), theme::font(6.5 * z), theme::text()_FAINT);
    let rect = align.anchor_size(pos, galley.size());
    painter.rect_filled(rect.expand(2.0 * z), 2.0 * z, alpha(theme::inset(), 0.85));
    painter.galley(rect.min, galley, theme::text()_FAINT);
}

/// Column `c`'s lower edge, in Hz.
fn column_hz(c: f32) -> f32 {
    F_LO * (F_HI / F_LO).powf(c / COLUMNS as f32)
}

struct Panel<'a, 'b> {
    v: &'a NodeView<'b>,
    id: NodeId,
    p: &'a ModuleParams,
    accent: Color32,
    now: f64,
}

/// `(input, midi, out)`: what is patched.
pub fn draw(
    v: &NodeView,
    id: NodeId,
    accent: Color32,
    m: &ModuleNode,
    (input, midi, out): (bool, bool, bool),
) {
    let p = &m.params;
    let key = Id::unique(("larsen-view", id));
    let mut view: View = v.ui.data_mut(|d| d.remove_temp(key)).unwrap_or_default();
    let now = v.ui.input(|i| i.time);
    view.update(p, now);
    let s = Panel {
        v,
        id,
        p,
        accent,
        now,
    };
    let col = |on: bool| {
        if on {
            theme::text()_DIM
        } else {
            theme::text()_FAINT
        }
    };
    v.text(
        PAD,
        42.0,
        Align2::LEFT_CENTER,
        "IN",
        v.font(9.0),
        col(input),
    );
    v.text(
        PAD,
        60.0,
        Align2::LEFT_CENTER,
        "MIDI",
        v.font(9.0),
        col(midi),
    );
    v.text(
        W - PAD,
        42.0,
        Align2::RIGHT_CENTER,
        "OUT",
        v.font(9.0),
        col(out),
    );
    s.header();
    s.field(&view);
    s.groups();
    s.echo_switches();
    let loud = (0..3).any(|k| p.viz[viz::LEVEL + k].get() > 1e-4)
        || view.columns.iter().any(|&c| c > DB_LO + 3.0);
    if loud {
        v.ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
    }
    v.ui.data_mut(|d| d.insert_temp(key, view));
}

impl Panel<'_, '_> {
    fn z(&self) -> f32 {
        self.v.z
    }

    fn uid(&self, salt: impl std::hash::Hash + std::fmt::Debug) -> Id {
        Id::unique(("larsen", self.id)).with(salt)
    }

    fn switch(&self, salt: &str, rect: Rect, label: &str, d: usize, tip: &str) {
        let (v, p) = (self.v, self.p);
        let on = la::flag(p, d);
        if widgets::toggle(
            v.ui,
            v.painter,
            self.uid(salt),
            rect,
            label,
            on,
            self.accent,
            self.z(),
        )
        .on_hover_text(tip)
        .clicked()
        {
            la::set_flag(p, d, !on);
        }
    }

    /// The presets, the keys' and the strings' switches, and what is
    /// playing.
    fn header(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let menu = widgets::toggle(
            v.ui,
            v.painter,
            self.uid("presets"),
            v.r(50.0, 34.0, 124.0, 50.0),
            "PRESETS ▾",
            false,
            self.accent,
            z,
        )
        .on_hover_text(
            "Save these settings as a preset, or load one (built-in ones keep your LEVEL)",
        );
        let names = FACTORY.map(|(name, ..)| name);
        preset_menu::show(&menu, p, self.uid("presets"), "sound", &names, |k| {
            la::load_factory(p, k)
        });
        v.text(
            140.0,
            42.0,
            Align2::LEFT_CENTER,
            "KEYS",
            v.font(7.0),
            theme::text()_FAINT,
        );
        self.switch(
            "pitch",
            v.r(166.0, 34.0, 210.0, 50.0),
            "PITCH",
            data::PITCH,
            "Keys on the MIDI input set the pitch (bend ±2). BODY snaps to whole periods of the note and a band-pass keeps the loop on it.\nMoving FREQ with no key down takes the pitch back",
        );
        self.switch(
            "gate",
            v.r(214.0, 34.0, 258.0, 50.0),
            "GATE",
            data::GATE,
            "The loop opens only while a key is held: it swells on the key and dies away after",
        );
        v.text(
            274.0,
            42.0,
            Align2::LEFT_CENTER,
            "STRINGS",
            v.font(7.0),
            theme::text()_FAINT,
        );
        self.switch(
            "poly",
            v.r(316.0, 34.0, 360.0, 50.0),
            "POLY",
            data::POLY,
            "Three strings, three loops: on FREQ and two intervals, or on three keys",
        );
        self.switch(
            "lock",
            v.r(364.0, 34.0, 408.0, 50.0),
            "LOCK",
            data::LOCK,
            "With POLY and no keys: on, the strings sound the notes they show (BODY snaps to them);\noff, each loop roams to its own resonance",
        );
        let midi = p.viz[viz::MIDI].get() > 0.5;
        let poly = la::flag(p, data::POLY);
        let text = match (midi, poly, la::flag(p, data::GATE)) {
            (true, true, _) => "three keys set the strings".to_owned(),
            (true, false, _) => "a key sets the pitch".to_owned(),
            (false, _, true) => "silent until a key is held".to_owned(),
            (false, true, _) if la::flag(p, data::LOCK) => "three strings, tuned".to_owned(),
            (false, true, _) => "three strings, roaming".to_owned(),
            (false, false, _) => "a drone on its own".to_owned(),
        };
        v.text(
            424.0,
            42.0,
            Align2::LEFT_CENTER,
            text,
            v.font(7.5),
            if midi { self.accent } else { theme::text()_FAINT },
        );
        // The knobs' two colours.
        let r = v.text(
            W - PAD - 36.0,
            42.0,
            Align2::RIGHT_CENTER,
            "after it",
            v.font(7.0),
            AMBER,
        );
        let x = (r.left() - v.min.x) / z - 8.0;
        v.text(
            x,
            42.0,
            Align2::RIGHT_CENTER,
            "in the loop",
            v.font(7.0),
            self.accent,
        );
    }

    fn x_of(&self, rect: Rect, hz: f32) -> f32 {
        rect.left() + rect.width() * ((hz / F_LO).ln() / (F_HI / F_LO).ln())
    }

    fn y_of(&self, rect: Rect, db: f32) -> f32 {
        rect.bottom() - rect.height() * ((db - DB_LO) / (DB_HI - DB_LO)).clamp(0.0, 1.0)
    }

    /// The loop's field: the spectrum, the band, harmonics and resonances.
    fn field(&self, view: &View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let outer = v.r(PAD, TOP, W - PAD, BOTTOM);
        v.painter.rect_filled(outer, 4.0 * z, theme::inset());
        let painter = v
            .painter
            .with_clip_rect(outer.intersect(v.painter.clip_rect()));
        let rect = outer.shrink2(vec2(8.0 * z, 6.0 * z));

        // Frequency lines.
        for (hz, label) in [
            (50.0, ""),
            (100.0, "100"),
            (200.0, "200"),
            (500.0, "500"),
            (1000.0, "1k"),
            (2000.0, "2k"),
            (5000.0, "5k"),
            (10_000.0, "10k"),
        ] {
            let x = self.x_of(rect, hz);
            painter.line_segment(
                [pos2(x, rect.top()), pos2(x, rect.bottom())],
                Stroke::new(0.6 * z, alpha(theme::text()_FAINT, 0.1)),
            );
            painter.text(
                pos2(x + 2.0 * z, rect.bottom() - 1.0 * z),
                Align2::LEFT_BOTTOM,
                label,
                theme::font(6.5 * z),
                alpha(theme::text()_FAINT, 0.7),
            );
        }

        // The band the two filters let through, as a veil.
        let sr = p.viz[viz::SR].get().max(8000.0);
        let (mut lp, mut hp) = (Biquad::default(), Biquad::default());
        lp.set(Kind::Low, p.get(LOWPASS), 0.9, sr);
        hp.set(Kind::High, p.get(HIGHPASS), 0.9, sr);
        let steps = 120;
        let mut band = Vec::with_capacity(steps + 3);
        band.push(pos2(rect.left(), rect.bottom()));
        for k in 0..=steps {
            let hz = F_LO * (F_HI / F_LO).powf(k as f32 / steps as f32);
            let w = TAU * hz / sr;
            let gain = if hz < sr * 0.5 {
                lp.response(w).abs() * hp.response(w).abs()
            } else {
                0.0
            };
            let db = 20.0 * gain.max(1e-6).log10();
            band.push(pos2(self.x_of(rect, hz), self.y_of(rect, db * 0.6 - 8.0)));
        }
        band.push(pos2(rect.right(), rect.bottom()));
        let edge: Vec<Pos2> = band[1..band.len() - 1].to_vec();
        for pair in band.windows(2) {
            // A strip at a time: the outline need not be convex.
            let (a, b) = (pair[0], pair[1]);
            let strip = vec![a, b, pos2(b.x, rect.bottom()), pos2(a.x, rect.bottom())];
            painter.add(Shape::convex_polygon(
                strip,
                alpha(self.accent, 0.035),
                Stroke::NONE,
            ));
        }
        painter.add(Shape::line(
            edge,
            Stroke::new(0.8 * z, alpha(self.accent, 0.25)),
        ));

        // Each running string: BODY's resonances from the top, its
        // harmonics from the bottom, its note over its fundamental.
        let tuned = p.viz[viz::TUNED].get() > 0.5;
        for (s, color) in STRING_COLORS.into_iter().enumerate() {
            let runs = p.viz[viz::RUNS + s].get();
            if runs < 0.02 {
                continue;
            }
            let note = p.viz[viz::NOTE + s].get();
            let f = note_hz(note);
            let body = p.viz[viz::BODY + s].get().max(1e-4);
            let open = p.viz[viz::OPEN + s].get() > 0.5;
            let ink = runs * if open { 1.0 } else { 0.4 };
            let mut k = 1.0;
            while k / body < F_HI && k < 400.0 {
                let x = self.x_of(rect, k / body);
                // Where they crowd (high up) they fade, rather than smear.
                let gap = self.x_of(rect, (k + 1.0) / body) - x;
                if x >= rect.left() {
                    let len = (if tuned { 9.0 } else { 6.0 }) * z;
                    let a = 0.3 * ink * (gap / (5.0 * z)).min(1.0);
                    painter.line_segment(
                        [pos2(x, rect.top()), pos2(x, rect.top() + len)],
                        Stroke::new(0.8 * z, alpha(color, a)),
                    );
                }
                k += 1.0;
            }
            for h in 1..=32 {
                let hz = f * h as f32;
                if hz > F_HI {
                    break;
                }
                let x = self.x_of(rect, hz);
                let len = (14.0 / (h as f32).sqrt()) * z;
                painter.line_segment(
                    [pos2(x, rect.bottom()), pos2(x, rect.bottom() - len)],
                    Stroke::new(1.2 * z, alpha(color, (0.65 / (h as f32).sqrt()) * ink)),
                );
            }
            let level = p.viz[viz::LEVEL + s].get().clamp(0.0, 1.0);
            let x = self
                .x_of(rect, f)
                .clamp(rect.left() + 14.0 * z, rect.right() - 14.0 * z);
            let y = rect.top() + (20.0 + 13.0 * s as f32) * z;
            painter.circle_filled(
                pos2(x, y),
                (3.0 + 7.0 * level.sqrt()) * z,
                alpha(color, 0.18 * ink),
            );
            painter.circle_filled(pos2(x, y), 2.2 * z, alpha(color, ink));
            painter.text(
                pos2(x + 7.0 * z, y),
                Align2::LEFT_CENTER,
                note_label(note.round() as i32),
                theme::font(7.5 * z),
                alpha(color, ink),
            );
        }

        // The output's spectrum, glowing.
        let points: Vec<Pos2> = (0..COLUMNS)
            .map(|c| {
                let hz = column_hz(c as f32 + 0.5);
                pos2(
                    self.x_of(rect, hz),
                    self.y_of(rect, view.columns.get(c).copied().unwrap_or(DB_LO)),
                )
            })
            .collect();
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let strip = vec![a, b, pos2(b.x, rect.bottom()), pos2(a.x, rect.bottom())];
            painter.add(Shape::convex_polygon(
                strip,
                alpha(self.accent, 0.12),
                Stroke::NONE,
            ));
        }
        painter.add(Shape::line(
            points.clone(),
            Stroke::new(3.0 * z, alpha(self.accent, 0.15)),
        ));
        painter.add(Shape::line(
            points,
            Stroke::new(1.1 * z, mix(self.accent, theme::text(), 0.35)),
        ));

        caption(
            &painter,
            rect.left_top() + vec2(0.0, 1.0) * z,
            Align2::LEFT_TOP,
            "▾ BODY's resonances",
            z,
        );
        caption(
            &painter,
            rect.left_bottom() + vec2(0.0, -12.0) * z,
            Align2::LEFT_BOTTOM,
            "▴ the strings' harmonics",
            z,
        );
    }

    fn groups(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let poly = la::flag(p, data::POLY);
        let midi = p.viz[viz::MIDI].get() > 0.5;
        let pong = la::flag(p, data::PONG);
        for (title, x0, x1, y, knobs) in GROUPS {
            v.text(
                x0,
                y,
                Align2::LEFT_CENTER,
                title,
                theme::strong(8.5 * z),
                theme::text()_DIM,
            );
            let end = if title == "ECHO" { 300.0 } else { x1 };
            v.painter.hline(
                v.p(x0 + 62.0, 0.0).x..=v.p(end, 0.0).x,
                v.p(0.0, y).y,
                Stroke::new(1.0 * z, alpha(theme::text()_FAINT, 0.3)),
            );
            let cw = (x1 - x0) / knobs.len() as f32;
            for (k, &(i, label)) in knobs.iter().enumerate() {
                let dim = match i {
                    FREQ => midi,
                    COUPLING => !poly,
                    INTERVAL2 | INTERVAL3 => !poly || midi,
                    DRIVE | TONE | WIDTH | GAIN => !pong,
                    _ => false,
                };
                self.knob(i, x0 + cw * (k as f32 + 0.5), y + 14.0, label, dim);
            }
        }
    }

    /// WARP, CLASSIC or PONG, and the repeats' pulse beside the echo's
    /// knobs.
    fn echo_switches(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let x = 212.0;
        self.switch(
            "warp",
            v.r(x, ROW_B + 18.0, x + 44.0, ROW_B + 34.0),
            "WARP",
            data::WARP,
            "The echo three to four times shorter: a doppler warp more than a halving",
        );
        const MODES: [(&str, &str); 2] = [
            ("CLASSIC", "Each side echoes itself"),
            (
                "PONG",
                "The repeats bounce from left to right, shaped by the PONG knobs",
            ),
        ];
        let pong = la::flag(p, data::PONG) as usize;
        let picked = segmented(
            v,
            self.uid("pong"),
            v.r(x, ROW_B + 38.0, 300.0, ROW_B + 54.0),
            2,
            pong,
            self.accent,
            |k, r, col| {
                v.painter.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    MODES[k].0,
                    v.font(7.0),
                    col,
                );
                MODES[k].1
            },
        );
        if let Some(k) = picked {
            la::set_flag(p, data::PONG, k == 1);
        }
        // A dot a side, lit at each repeat: with PONG they take turns.
        let time = p.viz[viz::ECHO].get().max(0.01) as f64;
        let repeats = self.now / time;
        let lit = repeats.fract() < 0.18 && p.get(SEND) > 0.0;
        let bounce = p.viz[viz::PONG].get() > 0.5;
        for side in 0..2 {
            let on = lit && (!bounce || (repeats.floor() as i64 % 2) as usize == side);
            let c = v.p(266.0 + 12.0 * side as f32, ROW_B + 26.0);
            v.painter
                .circle_filled(c, 3.0 * z, if on { AMBER } else { alpha(AMBER, 0.15) });
        }
        v.text(
            266.0 + 6.0,
            ROW_B + 12.0,
            Align2::CENTER_CENTER,
            format_seconds(time as f32),
            v.mono(6.5),
            theme::text()_FAINT,
        );
    }

    fn knob(&self, i: usize, cx: f32, y: f32, label: &str, dim: bool) {
        let (v, p, z) = (self.v, self.p, self.z());
        let spec = la::spec(i);
        let r = 10.0;
        v.text(
            cx,
            y,
            Align2::CENTER_CENTER,
            label,
            v.font(7.5),
            theme::text()_FAINT,
        );
        let top = y + 8.0;
        let color = if after_loop(i) { AMBER } else { self.accent };
        let color = if dim { alpha(color, 0.35) } else { color };
        let mut norm = spec.to_norm(p.get(i));
        let mut resp = widgets::knob(
            v.ui,
            v.painter,
            self.uid(("knob", i)),
            v.p(cx, top + r),
            r * z,
            &mut norm,
            spec.to_norm(spec.default),
            spec.bipolar,
            color,
            z,
        );
        if let Some((_, tip)) = TIPS.iter().find(|(k, _)| *k == i) {
            resp = resp.on_hover_text(*tip);
        }
        v.hit(resp.rect, ParamRef::Module(i));
        if resp.dragged() || resp.double_clicked() {
            let value = spec.denorm(norm);
            let value = if matches!(i, INTERVAL2 | INTERVAL3) {
                value.round()
            } else {
                value
            };
            if value != p.get(i) {
                p.set(i, value);
            }
        }
        let hot = resp.dragged() || resp.hovered();
        // What really plays, where it is not the knob.
        let text = match i {
            FREQ if p.viz[viz::MIDI].get() > 0.5 => format!(
                "{} · MIDI",
                note_label(p.viz[viz::NOTE].get().round() as i32)
            ),
            BODY if p.viz[viz::TUNED].get() > 0.5 => {
                format!("{} · SNAP", format_seconds(p.viz[viz::BODY].get()))
            }
            _ => la::format(p, i),
        };
        let col = if hot {
            theme::text()
        } else if dim {
            theme::text()_FAINT
        } else {
            theme::text()_DIM
        };
        v.text(
            cx,
            top + 2.0 * r + 8.0,
            Align2::CENTER_CENTER,
            text,
            v.mono(7.0),
            col,
        );
    }
}
