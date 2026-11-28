//! Panel of Quartz, the four-operator FM synth: flat and graphic. On top a
//! backlit LCD with the voice (and arrows through the factory voices), the
//! eight algorithms drawn as small diagrams to click, feedback and the
//! master section. Under it a row per operator: its switch and live level,
//! the eight waveforms as icons, RATIO or FIX and the frequency it gives,
//! the knobs (frequency, level, the five envelope rates and levels drawn as
//! a little envelope, EG SHIFT, the scalings and sensitivities). Then the
//! LFO and the FUNCTION page, the performance effect and the microtonal
//! scale, the wheels, and a keyboard to play it from the panel.

use egui::{Align2, Color32, CursorIcon, Id, Pos2, Sense, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};

use super::nodes::{NodeView, segmented};
use super::theme::{self, alpha, mix};
use super::widgets;
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::fm::{self, ALGORITHMS, LFO_WAVES, MICRO_NAMES, OPS, SHIFTS};
use crate::audio::modules::quartz::{
    self as qz, ALG, AMD, AMS, BC_AMP, BC_EBIAS, BC_PBIAS, BC_PITCH, BREATH, CHORD, CHORDS,
    DLY_FB, DLY_LEVEL, DLY_SHIFT, DLY_TIME, EFFECT, EFFECTS, FEEDBACK, LFO_DELAY, LFO_SPEED,
    LFO_SYNC, LFO_WAVE, MAX_VOICES, MICRO, MICRO_KEY, MOD_WHEEL, MONO, MW_AMP, MW_PITCH, NOTES,
    PAN_DIR, PAN_DIRS, PAN_RANGE, PAN_SOURCES, PAN_SRC, PB_RANGE, PMD, PMS, PORTA_MODE,
    PORTA_TIME, REVERB, TRANSPOSE, VOICES, VOLUME, data, op, viz,
};
use crate::audio::params::{NOTE_NAMES, Unit, note_label};
use crate::graph::{ModuleNode, ParamRef};

pub const SIZE: Vec2 = vec2(1180.0, 700.0);
const W: f32 = SIZE.x;
const H: f32 = SIZE.y;
const PAD: f32 = 12.0;
const TOP: f32 = 52.0;
const OPS_Y: f32 = 138.0;
const OP_H: f32 = 76.0;
const LOWER: f32 = OPS_Y + OP_H * 4.0 + 8.0;
const LOWER2: f32 = LOWER + 82.0;
const KEYS_Y: f32 = LOWER2 + 82.0;

/// The LCD's backlight and ink.
const LCD: Color32 = Color32::from_rgb(28, 44, 36);
const LCD_INK: Color32 = Color32::from_rgb(150, 236, 170);
/// Carriers and modulators in the diagrams.
const CARRIER: Color32 = Color32::from_rgb(255, 170, 120);
const MODULATOR: Color32 = Color32::from_rgb(150, 190, 255);

/// The keyboard's range.
const LOW_KEY: u8 = 36;
const KEYS: u8 = 49;

pub fn readout(p: &ModuleParams) -> (String, String) {
    let k = p.data[data::VOICE].get().round().max(0.0) as usize % VOICES.len();
    let mono = if qz::int_of(p, MONO) > 0 { "mono" } else { "poly" };
    (
        format!("I{:02} {}", k + 1, VOICES[k].name),
        format!(
            "ALG {} · FB {} · {mono}",
            qz::int_of(p, ALG) + 1,
            qz::int_of(p, FEEDBACK)
        ),
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
    s.header();
    for k in 0..OPS {
        s.operator(k, OPS_Y + OP_H * k as f32);
    }
    s.lfo_function();
    s.effect_tuning();
    s.keyboard();
    let busy = (0..MAX_VOICES).any(|i| m.params.viz[viz::LEVEL + i].get() > 1e-4);
    if busy {
        v.ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
}

impl Panel<'_, '_> {
    fn z(&self) -> f32 {
        self.v.z
    }

    fn uid(&self, salt: impl std::hash::Hash + std::fmt::Debug) -> Id {
        Id::unique(("quartz", self.id)).with(salt)
    }

    fn idx(&self, i: usize) -> usize {
        qz::int_of(self.p, i).max(0) as usize
    }

    fn stroke(&self, w: f32, c: Color32) -> Stroke {
        Stroke::new(w * self.z(), c)
    }

    fn line(&self, a: (f32, f32), b: (f32, f32), w: f32, c: Color32) {
        let v = self.v;
        v.painter.line_segment([v.p(a.0, a.1), v.p(b.0, b.1)], self.stroke(w, c));
    }

    fn title(&self, title: &str, x0: f32, x1: f32, y: f32) {
        let (v, z) = (self.v, self.z());
        let r = v.text(x0, y, Align2::LEFT_CENTER, title, theme::strong(8.0 * z), theme::TEXT_DIM);
        let from = r.right() + 6.0 * z;
        let to = v.p(x1, 0.0).x;
        if to > from {
            v.painter.hline(from..=to, v.p(0.0, y).y, self.stroke(1.0, alpha(theme::TEXT_FAINT, 0.3)));
        }
    }

    /// A parameter's value as the panel shows it.
    fn format(&self, i: usize) -> String {
        let p = self.p;
        let n = qz::int_of(p, i);
        let field = i % qz::OP_LEN;
        if i < qz::OP_LEN * OPS {
            return match field {
                op::DET => format!("{n:+}"),
                op::SHIFT => SHIFTS[n.clamp(0, 3) as usize].to_owned(),
                _ => format!("{n}"),
            };
        }
        match i {
            ALG => format!("{}", n + 1),
            TRANSPOSE => note_label(60 + n),
            BC_PBIAS | DLY_SHIFT => format!("{n:+}"),
            DLY_TIME => format!("{:.2}s", (n as f32 + 1.0) * 0.01),
            MICRO_KEY => NOTE_NAMES[n.clamp(0, 11) as usize].to_owned(),
            MOD_WHEEL | BREATH => format!("{:.0}%", p.get(i) * 100.0),
            _ => format!("{n}"),
        }
    }

    /// A knob at `cx`: its label at `y`, the knob, the value.
    fn knob(&self, i: usize, cx: f32, y: f32, label: &str, color: Color32) {
        let (v, p, z) = (self.v, self.p, self.z());
        const R: f32 = 9.0;
        let spec = qz::spec(i);
        v.text(cx, y, Align2::CENTER_CENTER, label, v.font(6.6), theme::TEXT_FAINT);
        let top = y + 7.0;
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
            color,
            z,
        );
        let resp = resp.on_hover_text(spec.name);
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
        v.text(
            cx,
            top + 2.0 * R + 7.0,
            Align2::CENTER_CENTER,
            self.format(i),
            v.mono(6.6),
            if hot { theme::TEXT } else { theme::TEXT_DIM },
        );
    }

    /// A row of named choices for parameter `i`.
    #[allow(clippy::too_many_arguments)]
    fn choices(&self, i: usize, x0: f32, y0: f32, x1: f32, y1: f32, names: &[&'static str], size: f32) {
        let (v, p) = (self.v, self.p);
        let sel = self.idx(i).min(names.len() - 1);
        let picked = segmented(v, self.uid(("seg", i)), v.r(x0, y0, x1, y1), names.len(), sel, self.accent, |k, r, col| {
            v.painter.text(r.center(), Align2::CENTER_CENTER, names[k], v.font(size), col);
            qz::spec(i).name
        });
        if let Some(k) = picked {
            p.set(i, k as f32);
        }
    }

    /// An on/off switch for parameter `i`.
    fn switch(&self, i: usize, x0: f32, y0: f32, x1: f32, y1: f32, label: &str) -> bool {
        let (v, p, z) = (self.v, self.p, self.z());
        let on = self.idx(i) > 0;
        let resp = widgets::toggle(v.ui, v.painter, self.uid(("sw", i)), v.r(x0, y0, x1, y1), label, on, self.accent, z)
            .on_hover_text(qz::spec(i).name);
        v.hit(resp.rect, ParamRef::Module(i));
        if resp.clicked() {
            p.set(i, if on { 0.0 } else { 1.0 });
        }
        on
    }

    /// A small text button; true when clicked.
    fn button(&self, salt: &str, x0: f32, y0: f32, x1: f32, y1: f32, label: &str) -> bool {
        let (v, z) = (self.v, self.z());
        let r = v.r(x0, y0, x1, y1);
        let resp = v.ui.interact(r, self.uid(("btn", salt)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        let fill = if resp.hovered() { theme::CONTROL_HI } else { theme::CONTROL };
        v.painter.rect_filled(r, 3.0 * z, fill);
        v.painter.text(r.center(), Align2::CENTER_CENTER, label, v.font(8.0), theme::TEXT);
        resp.clicked()
    }

    // --- Header ----------------------------------------------------------

    fn header(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        // The LCD.
        let lcd = v.r(PAD, TOP, PAD + 250.0, TOP + 52.0);
        v.painter.rect_filled(lcd, 3.0 * z, LCD);
        v.painter.rect_stroke(lcd, 3.0 * z, self.stroke(1.0, alpha(LCD_INK, 0.25)), StrokeKind::Inside);
        let k = p.data[data::VOICE].get().round().max(0.0) as usize % VOICES.len();
        let mono = qz::int_of(p, MONO) > 0;
        let line1 = format!("PLAY SINGLE  {}", if mono { "MONO" } else { "POLY" });
        let line2 = format!("I{:02} {}", k + 1, VOICES[k].name);
        v.text(PAD + 10.0, TOP + 16.0, Align2::LEFT_CENTER, &line1, v.mono(10.0), LCD_INK);
        v.text(PAD + 10.0, TOP + 36.0, Align2::LEFT_CENTER, &line2, v.mono(11.0), LCD_INK);
        // Voices playing: eight dots.
        for i in 0..MAX_VOICES {
            let l = p.viz[viz::LEVEL + i].get();
            let c = v.p(PAD + 196.0 + 6.0 * i as f32, TOP + 16.0);
            v.painter.circle_filled(c, 2.0 * z, if l > 1e-3 { LCD_INK } else { alpha(LCD_INK, 0.2) });
        }
        if self.button("prev", PAD + 256.0, TOP, PAD + 280.0, TOP + 24.0, "‹") {
            qz::load_voice(p, (k + VOICES.len() - 1) % VOICES.len());
        }
        if self.button("next", PAD + 256.0, TOP + 28.0, PAD + 280.0, TOP + 52.0, "›") {
            qz::load_voice(p, (k + 1) % VOICES.len());
        }

        // The algorithms.
        let (x0, x1) = (PAD + 294.0, PAD + 750.0);
        self.title("ALGORITHM", x0, x1, TOP - 4.0);
        let cw = (x1 - x0) / 8.0;
        let sel = self.idx(ALG).min(7);
        for a in 0..8 {
            let cx0 = x0 + cw * a as f32;
            let r = v.r(cx0 + 2.0, TOP + 4.0, cx0 + cw - 2.0, TOP + 72.0);
            let resp = v.ui.interact(r, self.uid(("alg", a)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
            let on = a == sel;
            v.painter.rect_filled(r, 4.0 * z, if on { theme::CONTROL } else { theme::INSET });
            if on {
                v.painter.rect_stroke(r, 4.0 * z, self.stroke(1.0, self.accent), StrokeKind::Inside);
            }
            self.algorithm(a, cx0 + cw * 0.5, TOP + 12.0, on || resp.hovered());
            v.text(cx0 + 8.0, TOP + 64.0, Align2::LEFT_CENTER, format!("{}", a + 1), v.mono(7.0), if on { theme::TEXT } else { theme::TEXT_FAINT });
            if resp.clicked() {
                p.set(ALG, a as f32);
            }
        }

        // Feedback and the master section.
        let x = PAD + 770.0;
        self.title("VOICE", x, W - PAD, TOP - 4.0);
        self.knob(FEEDBACK, x + 22.0, TOP + 10.0, "FEEDBACK", MODULATOR);
        self.knob(NOTES, x + 72.0, TOP + 10.0, "NOTES", self.accent);
        self.knob(VOLUME, x + 122.0, TOP + 10.0, "VOLUME", self.accent);
        self.choices(MONO, x + 152.0, TOP + 14.0, x + 252.0, TOP + 32.0, &["POLY", "MONO"], 7.0);
        if self.button("panic", x + 270.0, TOP + 14.0, W - PAD, TOP + 32.0, "ALL OFF") {
            p.viz[viz::PANIC].set(p.viz[viz::PANIC].get() + 1.0);
        }
        self.choices(PORTA_MODE, x + 152.0, TOP + 40.0, W - PAD, TOP + 58.0, &["FULL PORTA", "FINGERED"], 6.5);
    }

    /// Algorithm `a`'s diagram, centred on `cx`, its top at `y`.
    #[allow(clippy::needless_range_loop)]
    fn algorithm(&self, a: usize, cx: f32, y: f32, hot: bool) {
        let (v, z) = (self.v, self.z());
        // Where each operator sits: (column, row up from the carriers).
        const AT: [[(f32, f32); 4]; 8] = [
            [(0.0, 0.0), (0.0, 1.0), (0.0, 2.0), (0.0, 3.0)],
            [(0.0, 0.0), (0.0, 1.0), (-0.5, 2.0), (0.5, 2.0)],
            [(-0.25, 0.0), (-0.25, 1.0), (-0.25, 2.0), (0.6, 1.0)],
            [(-0.25, 0.0), (0.6, 1.0), (-0.25, 1.0), (-0.25, 2.0)],
            [(-0.5, 0.0), (-0.5, 1.0), (0.5, 0.0), (0.5, 1.0)],
            [(-0.8, 0.0), (0.0, 0.0), (0.8, 0.0), (0.0, 1.0)],
            [(-0.8, 0.0), (0.0, 0.0), (0.8, 0.0), (0.8, 1.0)],
            [(-1.2, 0.0), (-0.4, 0.0), (0.4, 0.0), (1.2, 0.0)],
        ];
        let (mods, car) = ALGORITHMS[a];
        let box_s = 9.0;
        let bottom = y + 40.0;
        let pos = |k: usize| {
            let (c, r) = AT[a][k];
            (cx + c * 15.0, bottom - r * 11.0)
        };
        let ink = if hot { theme::TEXT_DIM } else { theme::TEXT_FAINT };
        // Wires: modulator above, into the target's top.
        for t in 0..OPS {
            for j in 0..OPS {
                if mods[t] & (1 << j) != 0 {
                    let (a0, b0) = pos(j);
                    let (a1, b1) = pos(t);
                    self.line((a0, b0 + box_s * 0.5), (a1, b1 - box_s * 0.5), 1.0, ink);
                }
            }
        }
        // The output bus under the carriers.
        let xs: Vec<f32> = (0..OPS).filter(|k| car & (1 << k) != 0).map(|k| pos(k).0).collect();
        let lo = xs.iter().cloned().fold(f32::MAX, f32::min);
        let hi = xs.iter().cloned().fold(f32::MIN, f32::max);
        let bus = bottom + box_s * 0.5 + 4.0;
        for &x in &xs {
            self.line((x, bottom + box_s * 0.5), (x, bus), 1.0, ink);
        }
        self.line((lo, bus), (hi, bus), 1.0, ink);
        // Feedback round operator 4.
        let (fx, fy) = pos(3);
        let h = box_s * 0.5;
        let loop_pts = [
            v.p(fx + h, fy),
            v.p(fx + h + 4.0, fy),
            v.p(fx + h + 4.0, fy - h - 3.0),
            v.p(fx, fy - h - 3.0),
            v.p(fx, fy - h),
        ];
        v.painter.add(Shape::line(loop_pts.to_vec(), self.stroke(1.0, ink)));
        for k in 0..OPS {
            let (x, y) = pos(k);
            let r = v.r(x - h, y - h, x + h, y + h);
            let c = if car & (1 << k) != 0 { CARRIER } else { MODULATOR };
            v.painter.rect_filled(r, 1.5 * z, theme::INSET);
            v.painter.rect_stroke(r, 1.5 * z, self.stroke(1.0, if hot { c } else { alpha(c, 0.6) }), StrokeKind::Inside);
            v.painter.text(r.center(), Align2::CENTER_CENTER, format!("{}", k + 1), v.mono(6.0), if hot { c } else { alpha(c, 0.7) });
        }
    }

    // --- Operators -------------------------------------------------------

    fn operator(&self, k: usize, y: f32) {
        let (v, p, z) = (self.v, self.p, self.z());
        let car = ALGORITHMS[self.idx(ALG).min(7)].1 & (1 << k) != 0;
        let color = if car { CARRIER } else { MODULATOR };
        let card = v.r(PAD, y, W - PAD, y + OP_H - 6.0);
        v.painter.rect_filled(card, 4.0 * z, alpha(theme::INSET, 0.6));
        let on = self.idx(qz::op(k, op::ON)) > 0;

        // Name, switch, level.
        v.text(PAD + 8.0, y + 12.0, Align2::LEFT_CENTER, format!("OP{}", k + 1), theme::strong(10.0 * z), color);
        v.text(PAD + 8.0, y + 25.0, Align2::LEFT_CENTER, if car { "carrier" } else { "modulator" }, v.font(6.5), theme::TEXT_FAINT);
        self.switch(qz::op(k, op::ON), PAD + 8.0, y + 34.0, PAD + 40.0, y + 48.0, if on { "ON" } else { "OFF" });
        let lvl = p.viz[viz::OP_LEVEL + k].get().clamp(0.0, 1.0);
        let bar = v.r(PAD + 46.0, y + 34.0, PAD + 52.0, y + 62.0);
        v.painter.rect_filled(bar, 1.0 * z, theme::CONTROL);
        let fill = v.r(PAD + 46.0, y + 62.0 - 28.0 * lvl, PAD + 52.0, y + 62.0);
        v.painter.rect_filled(fill, 1.0 * z, color);

        // Waveforms.
        let (wx0, wy0) = (PAD + 62.0, y + 6.0);
        let cell = 23.0;
        let wsel = self.idx(qz::op(k, op::WAVE)).min(7);
        for w in 0..8 {
            let (c, r) = (w % 4, w / 4);
            let x0 = wx0 + c as f32 * cell;
            let y0 = wy0 + r as f32 * 30.0;
            let rect = v.r(x0, y0, x0 + cell - 2.0, y0 + 27.0);
            let resp = v.ui.interact(rect, self.uid(("wave", k, w)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(fm::WAVE_NAMES[w]);
            let sel = w == wsel;
            v.painter.rect_filled(rect, 2.5 * z, if sel { theme::CONTROL } else { theme::INSET });
            if sel {
                v.painter.rect_stroke(rect, 2.5 * z, self.stroke(1.0, color), StrokeKind::Inside);
            }
            let pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let t = i as f32 / 24.0;
                    v.p(x0 + 2.0 + t * (cell - 6.0), y0 + 11.0 - fm::wave(w, t) * 7.0)
                })
                .collect();
            let ink = if sel { color } else if resp.hovered() { theme::TEXT_DIM } else { theme::TEXT_FAINT };
            v.painter.add(Shape::line(pts, self.stroke(1.0, ink)));
            v.text(x0 + (cell - 2.0) * 0.5, y0 + 23.0, Align2::CENTER_CENTER, fm::WAVE_NAMES[w], v.mono(5.5), ink);
            if resp.clicked() {
                p.set(qz::op(k, op::WAVE), w as f32);
            }
        }

        // Frequency.
        let fx = wx0 + cell * 4.0 + 8.0;
        let fixed = self.idx(qz::op(k, op::FIXED)) > 0;
        self.choices(qz::op(k, op::FIXED), fx, y + 6.0, fx + 74.0, y + 20.0, &["RATIO", "FIX"], 6.5);
        let crs = self.idx(qz::op(k, op::CRS));
        let fine = self.idx(qz::op(k, op::FINE));
        let freq = if fixed {
            let hz = fm::fixed_hz(crs, fine, self.idx(qz::op(k, op::RANGE)));
            if hz >= 1000.0 { format!("{:.2}kHz", hz / 1000.0) } else { format!("{hz:.0}Hz") }
        } else {
            format!("×{:.2}", fm::ratio(crs, fine))
        };
        v.text(fx + 37.0, y + 38.0, Align2::CENTER_CENTER, &freq, v.mono(11.0), if on { theme::TEXT } else { theme::TEXT_FAINT });
        let det = qz::int_of(p, qz::op(k, op::DET));
        v.text(fx + 37.0, y + 54.0, Align2::CENTER_CENTER, format!("det {det:+}"), v.mono(6.5), theme::TEXT_FAINT);

        // Knobs.
        let mut x = fx + 98.0;
        let ky = y + 8.0;
        let step = 38.0;
        let knobs: [(usize, &str); 4] = [
            (op::CRS, "CRS"),
            (op::FINE, "FINE"),
            (if fixed { op::RANGE } else { op::DET }, if fixed { "RANGE" } else { "DET" }),
            (op::OUT, "OUT"),
        ];
        for (f, label) in knobs {
            self.knob(qz::op(k, f), x, ky, label, if f == op::OUT { color } else { self.accent });
            x += step;
        }
        if fixed {
            // DET still counts in FIX: keep it reachable.
            self.knob(qz::op(k, op::DET), x, ky, "DET", self.accent);
        }
        x += step + 4.0;

        // Envelope.
        let ex0 = x - 18.0;
        for (f, label) in [(op::AR, "AR"), (op::D1R, "D1R"), (op::D1L, "D1L"), (op::D2R, "D2R"), (op::RR, "RR")] {
            self.knob(qz::op(k, f), x, ky, label, color);
            x += step;
        }
        let gx = x - 14.0;
        self.envelope(k, gx, y + 8.0, gx + 110.0, y + 46.0, color);
        self.choices(qz::op(k, op::SHIFT), gx, y + 52.0, gx + 110.0, y + 64.0, &SHIFTS, 5.5);
        let _ = ex0;
        x = gx + 110.0 + 26.0;

        // Scaling and sensitivity.
        for (f, label) in [(op::RS, "RS"), (op::LS, "LS"), (op::KVS, "KVS"), (op::EBS, "EBS")] {
            self.knob(qz::op(k, f), x, ky, label, self.accent);
            x += step;
        }
        self.switch(qz::op(k, op::AME), x - 14.0, y + 22.0, x + 22.0, y + 38.0, "AME");
    }

    /// The operator's envelope drawn from its rates and levels.
    fn envelope(&self, k: usize, x0: f32, y0: f32, x1: f32, y1: f32, color: Color32) {
        let (v, p, z) = (self.v, self.p, self.z());
        let r = v.r(x0, y0, x1, y1);
        v.painter.rect_filled(r, 3.0 * z, theme::INSET);
        let g = |f: usize| p.get(qz::op(k, f)).round();
        let shift = [1.0, 0.5, 0.25, 0.125][self.idx(qz::op(k, op::SHIFT)).min(3)];
        // Time on a square-root scale, so short and long both show.
        let tw = |s: f32| s.min(8.0).sqrt() * 12.0 + 2.0;
        let lvl = |att: f32| y1 - 3.0 - (1.0 - (att * shift / 96.0).min(1.0)) * (y1 - y0 - 6.0);
        let mut pts = vec![(x0 + 3.0, lvl(96.0))];
        let mut x = x0 + 3.0;
        let ar = g(op::AR);
        x += if ar <= 0.0 { 30.0 } else { tw(fm::attack_seconds(ar)) };
        pts.push((x, lvl(0.0)));
        let d1 = fm::d1l_db(g(op::D1L));
        let d1r = g(op::D1R);
        if g(op::D1L) < 15.0 {
            x += if d1r <= 0.0 { 30.0 } else { tw(fm::decay_seconds(d1r) * d1 / 96.0) };
        }
        pts.push((x, lvl(d1)));
        // Second decay over a held key.
        let hold = 26.0;
        let d2r = g(op::D2R);
        let held = if d2r <= 0.0 { d1 } else { (d1 + 96.0 * 0.6 / fm::decay_seconds(d2r)).min(96.0) };
        x = (x + hold).min(x1 - 20.0);
        pts.push((x, lvl(held)));
        let rr = g(op::RR).max(1.0) * 2.0 + 1.0;
        x = (x + tw(fm::decay_seconds(rr) * (96.0 - held) / 96.0)).min(x1 - 3.0);
        pts.push((x, lvl(96.0)));
        let line: Vec<Pos2> = pts.iter().map(|&(x, y)| v.p(x, y)).collect();
        v.painter.add(Shape::line(line, self.stroke(1.2, color)));
        for &(x, y) in &pts[1..pts.len() - 1] {
            v.painter.circle_filled(v.p(x, y), 1.8 * z, color);
        }
    }

    // --- LFO and functions -----------------------------------------------

    fn lfo_function(&self) {
        let p = self.p;
        let y = LOWER;
        let (x0, x1) = (PAD, PAD + 470.0);
        self.title("LFO", x0, x1, y + 6.0);
        self.choices(LFO_WAVE, x0, y + 16.0, x0 + 150.0, y + 32.0, &LFO_WAVES, 6.0);
        self.switch(LFO_SYNC, x0, y + 40.0, x0 + 46.0, y + 56.0, "SYNC");
        // The LFO now.
        let lfo = p.viz[viz::LFO].get();
        let (v, z) = (self.v, self.z());
        let r = v.r(x0 + 54.0, y + 40.0, x0 + 150.0, y + 56.0);
        v.painter.rect_filled(r, 2.0 * z, theme::INSET);
        let cx = x0 + 102.0 + lfo * 44.0;
        v.painter.line_segment([v.p(cx, y + 42.0), v.p(cx, y + 54.0)], self.stroke(2.0, self.accent));
        let mut x = x0 + 180.0;
        for (i, label) in [(LFO_SPEED, "SPEED"), (LFO_DELAY, "DELAY"), (PMD, "PMD"), (AMD, "AMD"), (PMS, "PMS"), (AMS, "AMS")] {
            self.knob(i, x, y + 18.0, label, self.accent);
            x += 48.0;
        }

        let (x0, x1) = (PAD + 486.0, W - PAD);
        self.title("FUNCTION", x0, x1, y + 6.0);
        let mut x = x0 + 22.0;
        for (i, label) in [
            (PB_RANGE, "PB RANGE"),
            (PORTA_TIME, "PORTA"),
            (MW_PITCH, "MW PITCH"),
            (MW_AMP, "MW AMP"),
            (BC_PITCH, "BC PITCH"),
            (BC_AMP, "BC AMP"),
            (BC_PBIAS, "BC P BIAS"),
            (BC_EBIAS, "BC EG BIAS"),
            (TRANSPOSE, "MIDDLE C"),
            (REVERB, "REVERB"),
        ] {
            self.knob(i, x, y + 18.0, label, self.accent);
            x += 64.0;
        }
    }

    fn effect_tuning(&self) {
        let y = LOWER2;
        let (x0, x1) = (PAD, PAD + 560.0);
        self.title("EFFECT", x0, x1, y + 6.0);
        self.choices(EFFECT, x0, y + 16.0, x0 + 200.0, y + 32.0, &EFFECTS, 6.5);
        let effect = self.idx(EFFECT);
        let kx = x0 + 240.0;
        match effect {
            1 => {
                for (j, (i, label)) in [(DLY_TIME, "TIME"), (DLY_SHIFT, "SHIFT"), (DLY_FB, "FEEDBACK"), (DLY_LEVEL, "LEVEL")].into_iter().enumerate() {
                    self.knob(i, kx + 70.0 * j as f32, y + 18.0, label, self.accent);
                }
            }
            2 => {
                self.choices(PAN_SRC, kx - 30.0, y + 18.0, kx + 120.0, y + 34.0, &PAN_SOURCES, 6.0);
                self.choices(PAN_DIR, kx - 30.0, y + 40.0, kx + 120.0, y + 56.0, &PAN_DIRS, 6.0);
                self.knob(PAN_RANGE, kx + 170.0, y + 18.0, "RANGE", self.accent);
            }
            3 => {
                let names: Vec<&'static str> = CHORDS.iter().map(|c| c.0).collect();
                self.choices(CHORD, kx - 30.0, y + 22.0, x1, y + 40.0, &names, 5.8);
            }
            _ => {
                let (v, _) = (self.v, self.z());
                v.text(kx, y + 32.0, Align2::LEFT_CENTER, "transposed delay · pan · single-key chord", v.font(7.0), theme::TEXT_FAINT);
            }
        }

        let (x0, x1) = (PAD + 576.0, PAD + 960.0);
        self.title("MICROTUNE", x0, x1, y + 6.0);
        let sel = self.idx(MICRO).min(MICRO_NAMES.len() - 1);
        let v = self.v;
        v.text(x0 + 70.0, y + 32.0, Align2::CENTER_CENTER, MICRO_NAMES[sel], v.mono(10.0), if sel > 0 { theme::TEXT } else { theme::TEXT_DIM });
        if self.button("micro-", x0, y + 22.0, x0 + 18.0, y + 42.0, "‹") {
            self.p.set(MICRO, ((sel + MICRO_NAMES.len() - 1) % MICRO_NAMES.len()) as f32);
        }
        if self.button("micro+", x0 + 122.0, y + 22.0, x0 + 140.0, y + 42.0, "›") {
            self.p.set(MICRO, ((sel + 1) % MICRO_NAMES.len()) as f32);
        }
        self.knob(MICRO_KEY, x0 + 175.0, y + 18.0, "KEY", self.accent);

        let x0 = PAD + 976.0;
        self.title("WHEELS", x0, W - PAD, y + 6.0);
        self.knob(MOD_WHEEL, x0 + 30.0, y + 18.0, "MOD WHEEL", self.accent);
        self.knob(BREATH, x0 + 110.0, y + 18.0, "BREATH", self.accent);
    }

    // --- Keyboard ---------------------------------------------------------

    fn keyboard(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let (x0, x1) = (PAD, W - PAD);
        let (top, bottom) = (KEYS_Y, H - 12.0);
        let black = |n: u8| matches!(n % 12, 1 | 3 | 6 | 8 | 10);
        let whites: Vec<u8> = (LOW_KEY..LOW_KEY + KEYS).filter(|&n| !black(n)).collect();
        let ww = (x1 - x0) / whites.len() as f32;
        let black_bottom = top + (bottom - top) * 0.6;
        let mut lit = [false; 128];
        for i in 0..MAX_VOICES {
            if p.viz[viz::LEVEL + i].get() > 1e-3 {
                let n = p.viz[viz::NOTE + i].get();
                if (0.0..128.0).contains(&n) {
                    lit[n as usize] = true;
                }
            }
        }
        let white_x = |n: u8| whites.iter().position(|&w| w == n).map(|i| x0 + ww * i as f32);
        let black_span = |n: u8| {
            let bx = white_x(n - 1).unwrap_or(x0) + ww * 0.68;
            (bx, bx + ww * 0.64)
        };
        let full = v.r(x0, top, x1, bottom);
        let resp = v.ui.interact(full, self.uid("keys"), Sense::click_and_drag()).on_hover_cursor(CursorIcon::PointingHand);
        let mut pressed = None;
        if let Some(pos) = resp.interact_pointer_pos().filter(|_| resp.is_pointer_button_down_on()) {
            let lx = (pos.x - v.min.x) / z;
            let ly = (pos.y - v.min.y) / z;
            if ly < black_bottom {
                pressed = (LOW_KEY..LOW_KEY + KEYS).filter(|&n| black(n)).find(|&n| {
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
        if let Some(n) = pressed {
            lit[n as usize] = true;
        }
        for (i, &n) in whites.iter().enumerate() {
            let r = v.r(x0 + ww * i as f32 + 0.6, top, x0 + ww * (i + 1) as f32 - 0.6, bottom);
            let on = lit[n as usize];
            v.painter.rect_filled(r, 2.0 * z, if on { alpha(self.accent, 0.5) } else { theme::CONTROL });
            if n % 12 == 0 {
                v.text(x0 + ww * (i as f32 + 0.5), bottom - 7.0, Align2::CENTER_CENTER, note_label(n as i32), v.font(5.5), theme::TEXT_FAINT);
            }
        }
        for n in (LOW_KEY..LOW_KEY + KEYS).filter(|&n| black(n)) {
            let (a, b) = black_span(n);
            let r = v.r(a, top, b, black_bottom);
            let on = lit[n as usize];
            v.painter.rect_filled(r, 2.0 * z, if on { mix(self.accent, theme::INSET, 0.35) } else { theme::INSET });
            v.painter.rect_stroke(r, 2.0 * z, self.stroke(1.0, if on { self.accent } else { theme::CONTROL_HI }), StrokeKind::Inside);
        }
        let _ = pos2;
    }
}
