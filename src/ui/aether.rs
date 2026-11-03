//! Panel of Aether. Up top the chord shape (or FREE, none), LATCH or GATE
//! for the keys, RANDOM, the presets and where the chord comes from. The
//! display: on the left the
//! motion as a fractal flower — a branch per voice, forking four times into
//! its sixteen partials (two layers of eight), each fork bent by a faster
//! octave of the motion, so the trunks sway slowly and the twigs quiver;
//! every tip glows with its partial, and glitches flare on the tips they
//! land on. On the right the aurora: every partial's track over the last
//! seconds on a log-frequency scale, the voices in their colours (warm low,
//! ethereal high), glitches as sparks. Under them the timbre's motion over
//! one EVOLVE period round now — what has passed and what is coming (it is
//! the same every time) — with the bar lines when synced. Then the knobs:
//! VOICE (with GLIDE), MOTION (with SYNC and RESEED), WARMTH and GLITCH
//! (with its grid).

use std::collections::VecDeque;
use std::f32::consts::TAU;

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Align2, Color32, Id, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};

use super::nodes::{NodeView, segmented};
use super::theme::{self, alpha, mix};
use super::{preset_menu, widgets};
use crate::audio::engine::NodeId;
use crate::audio::midi::note_hz;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::aether::fractal::{self, PARTIALS, spectrum_name};
use crate::audio::modules::aether::{
    self as ae, AIR, CHORDS, CHORUS, COLOR, DECAY, DETUNE, DRIVE, EVOLVE, FACTORY, GLITCH, HARM, LANE_COLOR, LANE_LEVEL, LANE_PAN, LEVEL, MORPH,
    REPEAT, ROOT, ROUGH, SHIMMER, SKIP, SPACE, SPARK, TONE, VOICES, WIDTH, data, glitch, viz,
};
use crate::audio::params::{DIVISIONS, note_label};
use crate::graph::{ModuleNode, ParamRef};

pub const SIZE: Vec2 = vec2(760.0, 452.0);
const W: f32 = SIZE.x;
const PAD: f32 = 12.0;
/// The display band: the flower (a square) and the aurora.
const TOP: f32 = 58.0;
const BOTTOM: f32 = 262.0;
const FLOWER: f32 = BOTTOM - TOP;
const AURORA_X: f32 = PAD + FLOWER + 8.0;
/// The motion strip, the two knob rows, where the right-hand groups start.
const STRIP: [f32; 2] = [270.0, 302.0];
const ROW_A: f32 = 316.0;
const ROW_B: f32 = 386.0;
const SPLIT: f32 = 396.0;
/// GLIDE's switch, at the end of the VOICE title line.
const GLIDE_X: f32 = SPLIT - 12.0 - 54.0;

/// The voices' colours, low to high: warm amber up to an ethereal cyan.
const VOICE_COLORS: [Color32; VOICES] = [
    Color32::from_rgb(255, 176, 100),
    Color32::from_rgb(250, 126, 146),
    Color32::from_rgb(196, 136, 255),
    Color32::from_rgb(118, 168, 255),
    Color32::from_rgb(108, 226, 232),
];

/// Seconds of history in the aurora, and how often it is sampled.
const HISTORY: f64 = 18.0;
const SAMPLE: f64 = 0.1;
/// The aurora's frequency range.
const F_LO: f32 = 40.0;
const F_HI: f32 = 9000.0;
/// How long a glitch flares on the flower, and sparkles in the aurora.
const FLARE: f64 = 0.9;
const SPARKLE: f64 = 2.5;

type Knobs = [(usize, &'static str)];
const VOICE_KNOBS: [(usize, &str); 6] =
    [(ROOT, "ROOT"), (DETUNE, "DETUNE"), (COLOR, "COLOR"), (TONE, "TONE"), (MORPH, "MORPH"), (SHIMMER, "SHIMMER")];
const MOTION_KNOBS: [(usize, &str); 2] = [(EVOLVE, "EVOLVE"), (ROUGH, "ROUGH")];
const WARMTH_KNOBS: [(usize, &str); 6] = [(DRIVE, "DRIVE"), (CHORUS, "CHORUS"), (SPACE, "SPACE"), (AIR, "AIR"), (WIDTH, "WIDTH"), (LEVEL, "LEVEL")];
const GLITCH_KNOBS: [(usize, &str); 6] = [(GLITCH, "CHANCE"), (HARM, "HARM"), (REPEAT, "REPEAT"), (SKIP, "SKIP"), (DECAY, "DECAY"), (SPARK, "LEVEL")];
/// The knob groups: title, left and right edges, the row's top, knobs.
type KnobGroup = (&'static str, f32, f32, f32, &'static Knobs);
const GROUPS: [KnobGroup; 4] = [
    ("VOICE", PAD, SPLIT - 12.0, ROW_A, &VOICE_KNOBS),
    ("MOTION", SPLIT, SPLIT + 132.0, ROW_A, &MOTION_KNOBS),
    ("WARMTH", PAD, SPLIT - 12.0, ROW_B, &WARMTH_KNOBS),
    ("GLITCH", SPLIT, W - PAD, ROW_B, &GLITCH_KNOBS),
];

const TIPS: [(usize, &str); 20] = [
    (ROOT, "The chord's root, or in FREE the note alone (keys on the MIDI input take over)"),
    (DETUNE, "How far each voice's two layers drift apart: the beating, breathing with the motion"),
    (COLOR, "The timbre: WARM (a soft flute), SILK (strings), HOLLOW (a reed), CHOIR (a vowel)"),
    (TONE, "Dark to bright: the partials' tilt and the warm low-pass"),
    (MORPH, "How far the motion moves the timbre, the tone and the voices' pitch"),
    (SHIMMER, "Each partial's own fractal ripple, in level and tuning: the higher, the faster"),
    (EVOLVE, "The motion's slowest period: in bars when SYNC is on, else in seconds.\nIts six octaves run from the whole period down to 1/32 of it"),
    (ROUGH, "Smooth (the slow swells rule) to rough (the faster octaves show): the fractal's gain"),
    (DRIVE, "Soft saturation: warmth, then a rounded glow"),
    (CHORUS, "Three swaying copies a side, crossed for width"),
    (SPACE, "A wide, diffuse space: how large, and how much of it"),
    (AIR, "The space's highs lifted, and a breath tuned to the chord's upper harmonics"),
    (WIDTH, "How far the voices and their layers spread"),
    (LEVEL, "Output level"),
    (GLITCH, "The chance a step of the grid glitches"),
    (HARM, "The highest harmonic a glitch may land on (sour ones are skipped)"),
    (REPEAT, "Ratchets: a glitch repeats in subdivisions of the step, sometimes climbing"),
    (SKIP, "The share of glitches that skip (loop a slice in tune) rather than ping"),
    (DECAY, "How long a ping rings and a skip holds"),
    (SPARK, "The glitches' level"),
];

/// The node's two summary lines: the chord, the motion and glitches.
pub fn readout(p: &ModuleParams) -> (String, String) {
    let chord = CHORDS[ae::chord(p)].0;
    let main = match p.viz[viz::SOURCE].get().round() as i32 {
        1 => format!("keys · {chord}"),
        2 => format!("latched · {chord}"),
        _ => format!("{} · {chord}", note_label(p.get(ROOT).round() as i32)),
    };
    (main, format!("{} · glitch {}", ae::format(p, EVOLVE), ae::format(p, GLITCH)))
}

/// Light that adds up (premultiplied with no alpha): glows.
fn glow(c: Color32, a: f32) -> Color32 {
    let c = alpha(c, a);
    Color32::from_rgba_premultiplied(c.r(), c.g(), c.b(), 0)
}

/// What the aurora keeps of each moment.
#[derive(Clone, Copy)]
struct Moment {
    at: f64,
    notes: [f32; VOICES],
    amps: [f32; VOICES * PARTIALS],
    run: f32,
}

#[derive(Clone, Copy)]
struct Spark {
    at: f64,
    hz: f32,
    kind: u8,
}

/// What the panel remembers between frames.
#[derive(Clone, Default)]
struct View {
    moments: VecDeque<Moment>,
    sparks: VecDeque<Spark>,
    fired: f32,
}

impl View {
    fn update(&mut self, p: &ModuleParams, now: f64) {
        if self.moments.back().is_none_or(|m| now - m.at >= SAMPLE) {
            self.moments.push_back(Moment {
                at: now,
                notes: std::array::from_fn(|v| p.viz[viz::NOTE + v].get()),
                amps: std::array::from_fn(|k| p.viz[viz::AMP + k].get()),
                run: p.viz[viz::RUN].get(),
            });
        }
        while self.moments.front().is_some_and(|m| now - m.at > HISTORY + SAMPLE) {
            self.moments.pop_front();
        }
        let fired = p.viz[viz::FIRED].get();
        if fired < self.fired {
            // A new processor (the node was rebuilt).
            self.fired = 0.0;
        }
        let (from, to) = (self.fired as u32 + 1, fired as u32);
        for n in from.max(to.saturating_sub(viz::EVENTS as u32 - 1))..=to {
            let at = viz::EVENT + 3 * (n as usize % viz::EVENTS);
            if p.viz[at].get() as u32 == n {
                self.sparks.push_back(Spark { at: now, hz: p.viz[at + 1].get(), kind: p.viz[at + 2].get() as u8 });
            }
        }
        self.fired = fired;
        while self.sparks.front().is_some_and(|s| now - s.at > SPARKLE) || self.sparks.len() > 64 {
            self.sparks.pop_front();
        }
    }
}

struct Panel<'a, 'b> {
    v: &'a NodeView<'b>,
    id: NodeId,
    p: &'a ModuleParams,
    accent: Color32,
    now: f64,
}

/// `(midi, out)`: the MIDI input and the output are patched.
pub fn draw(v: &NodeView, id: NodeId, accent: Color32, m: &ModuleNode, (midi, out): (bool, bool)) {
    let p = &m.params;
    let key = Id::unique(("aether-view", id));
    let mut view: View = v.ui.data_mut(|d| d.remove_temp(key)).unwrap_or_default();
    let now = v.ui.input(|i| i.time);
    view.update(p, now);
    let s = Panel { v, id, p, accent, now };
    let col = |on: bool| if on { theme::TEXT_DIM } else { theme::TEXT_FAINT };
    v.text(PAD, 42.0, Align2::LEFT_CENTER, "MIDI", v.font(9.0), col(midi));
    v.text(W - PAD, 42.0, Align2::RIGHT_CENTER, "OUT", v.font(9.0), col(out));
    s.header();
    s.flower(&view);
    s.aurora(&view);
    s.strip();
    s.groups();
    s.glide();
    s.motion_extras();
    s.glitch_grid(&view);
    // Animate while it sounds, or while glitches still flare.
    let busy = p.viz[viz::RUN].get() > 1e-3 || view.sparks.back().is_some_and(|sp| now - sp.at < SPARKLE);
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
        Id::unique(("aether", self.id)).with(salt)
    }

    fn run(&self) -> f32 {
        self.p.viz[viz::RUN].get().clamp(0.0, 1.0)
    }

    /// How far a voice has faded in.
    fn live(&self, voice: usize) -> f32 {
        self.p.viz[viz::LIVE + voice].get().clamp(0.0, 1.0)
    }

    /// A fresh seed, for RANDOM and RESEED.
    fn seed(&self) -> u32 {
        (self.now * 1000.0) as u32 ^ self.id.wrapping_mul(0x9e37_79b9)
    }

    /// The chord shape, the keys' mode, RANDOM, the presets and the chord's
    /// source.
    fn header(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let picked = segmented(v, self.uid("chord"), v.r(50.0, 34.0, 362.0, 50.0), CHORDS.len(), ae::chord(p), self.accent, |k, r, col| {
            v.painter.text(r.center(), Align2::CENTER_CENTER, CHORDS[k].0, v.font(7.0), col);
            CHORDS[k].1
        });
        if let Some(k) = picked {
            p.set_mode(k);
        }
        const KEYS: [(&str, &str); 2] =
            [("LATCH", "Keys set the chord, which stays when you let go"), ("GATE", "It sounds only while keys are held")];
        let gated = ae::gated(p) as usize;
        let picked = segmented(v, self.uid("keys"), v.r(370.0, 34.0, 462.0, 50.0), 2, gated, self.accent, |k, r, col| {
            v.painter.text(r.center(), Align2::CENTER_CENTER, KEYS[k].0, v.font(7.0), col);
            KEYS[k].1
        });
        if let Some(k) = picked {
            p.data[data::KEYS].set(k as f32);
        }
        let dice = widgets::toggle(v.ui, v.painter, self.uid("random"), v.r(470.0, 34.0, 532.0, 50.0), "RANDOM", false, self.accent, z)
            .on_hover_text("A new sound: chord, timbre, motion, warmth and glitches (ROOT, LEVEL, SYNC, GLIDE and the keys stay; so does FREE)\nShift: the timbre and warmth only");
        if dice.clicked() {
            let timbre_only = v.ui.input(|i| i.modifiers.shift);
            ae::randomize(p, self.seed(), timbre_only);
        }
        let menu = widgets::toggle(v.ui, v.painter, self.uid("presets"), v.r(538.0, 34.0, 612.0, 50.0), "PRESETS ▾", false, self.accent, z)
            .on_hover_text("Save this sound as a preset, or load one (built-in ones keep your ROOT, LEVEL, GLIDE and keys)");
        let names = FACTORY.map(|(name, ..)| name);
        preset_menu::show(&menu, p, self.uid("presets"), "sound", &names, |k| ae::load_factory(p, k));
        let source = p.viz[viz::SOURCE].get().round() as i32;
        let (text, lit) = match source {
            1 => ("keys held", true),
            2 => ("keys latched", true),
            _ if ae::free(p) => ("ROOT in unison", false),
            _ => ("chord on ROOT", false),
        };
        let r = v.text(620.0, 42.0, Align2::LEFT_CENTER, text, v.font(7.5), if lit { self.accent } else { theme::TEXT_FAINT });
        if source == 2 {
            let rect = Rect::from_min_size(pos2(r.right() + 4.0 * z, r.center().y - 6.0 * z), vec2(12.0, 12.0) * z);
            let resp = widgets::toggle(v.ui, v.painter, self.uid("unlatch"), rect, "×", false, self.accent, z)
                .on_hover_text("Let go of the latched keys: back to ROOT");
            if resp.clicked() {
                p.viz[viz::UNLATCH].set(p.viz[viz::UNLATCH].get() + 1.0);
            }
        }
    }

    /// The fractal flower: five branches, four forks each, bent by the
    /// motion's octaves; tips glowing with their partials.
    fn flower(&self, view: &View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let rect = v.r(PAD, TOP, PAD + FLOWER, BOTTOM);
        v.painter.rect_filled(rect, 4.0 * z, theme::INSET);
        let painter = v.painter.with_clip_rect(rect.intersect(v.painter.clip_rect()));
        let c = rect.center();
        let radius = rect.width() * 0.5;
        let run = self.run();
        let seed = ae::seed(p);
        let pos = p.viz[viz::POS].get() as f64;

        // A halo that breathes with the sound.
        for k in 0..6 {
            let r = radius * (0.2 + 0.16 * k as f32);
            painter.circle_filled(c, r, glow(self.accent, 0.025 * (0.3 + run)));
        }
        for k in 1..4 {
            painter.circle_stroke(c, radius * 0.3 * k as f32, Stroke::new(0.6 * z, alpha(theme::TEXT_FAINT, 0.12)));
        }

        let spin = TAU * (pos * 0.125).fract() as f32;
        let mut tips = [[Pos2::ZERO; 2 * PARTIALS]; VOICES];
        for voice in 0..VOICES {
            // Voices without a key (in FREE) stay as faint outlines.
            let ink = (0.35 + 0.65 * run) * (0.15 + 0.85 * self.live(voice));
            // The voice's own strands of motion, an octave per fork.
            let bend: [f32; fractal::OCTAVES] = std::array::from_fn(|o| fractal::octave(seed, LANE_LEVEL + voice as u32, o, pos));
            let lean: [f32; fractal::OCTAVES] = std::array::from_fn(|o| fractal::octave(seed, LANE_PAN + voice as u32, o, pos));
            let angle = spin + TAU * voice as f32 / VOICES as f32 + 0.3 * lean[0] - TAU * 0.25;
            let len = radius * 0.3 * (0.88 + 0.12 * bend[0]);
            let amps: [f32; PARTIALS] = std::array::from_fn(|k| p.viz[viz::AMP + voice * PARTIALS + k].get());
            let color = VOICE_COLORS[voice];
            let mut stack = vec![(c, angle, len, 0usize, 0usize)];
            while let Some((from, a, l, depth, index)) = stack.pop() {
                let to = from + vec2(a.cos(), a.sin()) * l;
                let width = (2.6 - 0.5 * depth as f32) * z;
                let col = mix(color, theme::TEXT, 0.12 * depth as f32);
                painter.line_segment([from, to], Stroke::new(width * 2.2, glow(col, 0.12 * ink)));
                painter.line_segment([from, to], Stroke::new(width, alpha(col, 0.55 * ink)));
                if depth == 4 {
                    tips[voice][index] = to;
                    let amp = amps[index % PARTIALS].max(0.0).sqrt();
                    painter.circle_filled(to, (1.2 + 5.0 * amp) * z, glow(col, (0.15 + 0.85 * amp) * ink));
                    painter.circle_filled(to, (0.8 + 1.6 * amp) * z, alpha(mix(col, Color32::WHITE, 0.5), (0.3 + 0.7 * amp) * ink));
                    continue;
                }
                let spread = 0.3 + 0.17 * bend[depth + 1];
                let asym = 0.35 * lean[depth + 1];
                let next = l * (0.74 + 0.08 * bend[depth.min(fractal::OCTAVES - 1)]);
                // The first fork splits the layers, the rest the partials.
                stack.push((to, a - spread * (1.0 + asym), next, depth + 1, index));
                stack.push((to, a + spread * (1.0 - asym), next, depth + 1, index | 1 << (3 - depth)));
            }
        }

        // The core, and the glitches flaring on their tips.
        let ink = 0.35 + 0.65 * run;
        painter.circle_filled(c, (3.0 + 6.0 * run) * z, glow(self.accent, 0.5 * ink));
        painter.circle_filled(c, 2.2 * z, alpha(Color32::WHITE, 0.4 + 0.6 * run));
        let notes: [f32; VOICES] = std::array::from_fn(|k| p.viz[viz::NOTE + k].get());
        for spark in view.sparks.iter().filter(|s| self.now - s.at < FLARE) {
            let age = ((self.now - spark.at) / FLARE) as f32;
            let fade = 1.0 - age;
            if spark.kind == glitch::SKIP {
                painter.circle_stroke(c, (6.0 + radius * 0.8 * age) * 1.0, Stroke::new(2.0 * z * fade, glow(Color32::WHITE, 0.6 * fade)));
                continue;
            }
            let live: [f32; VOICES] = std::array::from_fn(|k| self.live(k));
            let (voice, partial) = nearest_partial(&notes, &live, spark.hz);
            let at = tips[voice][partial];
            let col = mix(VOICE_COLORS[voice], Color32::WHITE, 0.4);
            painter.circle_filled(at, (3.0 + 10.0 * age) * z, glow(col, 0.5 * fade));
            painter.circle_stroke(at, (4.0 + 16.0 * age) * z, Stroke::new(1.2 * z, glow(col, 0.8 * fade)));
        }
        v.text(PAD + 8.0, TOP + 10.0, Align2::LEFT_CENTER, "FRACTAL", v.font(6.5), theme::TEXT_FAINT);
    }

    /// The partials' tracks over the last seconds, and the glitches.
    fn aurora(&self, view: &View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let rect = v.r(AURORA_X, TOP, W - PAD, BOTTOM);
        let painter = v.painter.with_clip_rect(rect.intersect(v.painter.clip_rect()));
        // Night sky: deep violet at the top to ink at the bottom.
        let mut sky = Mesh::default();
        for (k, y) in [rect.top(), rect.bottom()].into_iter().enumerate() {
            let col = if k == 0 { Color32::from_rgb(22, 18, 38) } else { theme::INSET };
            sky.vertices.push(Vertex { pos: pos2(rect.left(), y), uv: WHITE_UV, color: col });
            sky.vertices.push(Vertex { pos: pos2(rect.right(), y), uv: WHITE_UV, color: col });
        }
        sky.add_triangle(0, 1, 2);
        sky.add_triangle(1, 3, 2);
        painter.add(Shape::mesh(sky));

        let y_of = |hz: f32| rect.bottom() - (hz.max(F_LO) / F_LO).ln() / (F_HI / F_LO).ln() * rect.height();
        let x_of = |at: f64| rect.right() - ((self.now - at) / HISTORY) as f32 * rect.width();
        // The A of every octave.
        for octave in 1..8 {
            let hz = 27.5 * (1 << octave) as f32;
            let y = y_of(hz);
            painter.hline(rect.x_range(), y, Stroke::new(0.6 * z, alpha(theme::TEXT_FAINT, 0.14)));
            if octave < 7 {
                v.text(AURORA_X + 4.0, (y - rect.top()) / z + TOP - 5.0, Align2::LEFT_CENTER, format!("A{}", octave - 1), v.font(6.0), alpha(theme::TEXT_FAINT, 0.6));
            }
        }

        // Each partial a ribbon of light: bright in the middle, fading to
        // its edges, older moments dimmer.
        let mut mesh = Mesh::default();
        let moments: Vec<&Moment> = view.moments.iter().collect();
        for (voice, &color) in VOICE_COLORS.iter().enumerate() {
            for k in 0..PARTIALS {
                let mut prev: Option<(f64, u32)> = None;
                for m in &moments {
                    let amp = m.amps[voice * PARTIALS + k].max(0.0) * m.run;
                    let x = x_of(m.at);
                    let y = y_of(note_hz(m.notes[voice]) * (k + 1) as f32);
                    let age = ((self.now - m.at) / HISTORY) as f32;
                    let fade = (1.0 - age).clamp(0.0, 1.0).powf(0.6);
                    let light = amp.sqrt() * fade;
                    let half = (0.8 + 7.0 * amp.sqrt()) * z * if k == 0 { 1.3 } else { 1.0 };
                    let col = glow(mix(color, Color32::WHITE, 0.08 * k as f32), 0.9 * light);
                    let base = mesh.vertices.len() as u32;
                    mesh.vertices.push(Vertex { pos: pos2(x, y - half), uv: WHITE_UV, color: Color32::TRANSPARENT });
                    mesh.vertices.push(Vertex { pos: pos2(x, y), uv: WHITE_UV, color: col });
                    mesh.vertices.push(Vertex { pos: pos2(x, y + half), uv: WHITE_UV, color: Color32::TRANSPARENT });
                    if let Some((at, a)) = prev
                        && m.at - at < SAMPLE * 4.0
                    {
                        let b = base;
                        mesh.add_triangle(a, a + 1, b);
                        mesh.add_triangle(a + 1, b + 1, b);
                        mesh.add_triangle(a + 1, a + 2, b + 1);
                        mesh.add_triangle(a + 2, b + 2, b + 1);
                    }
                    prev = Some((m.at, base));
                }
            }
        }
        painter.add(Shape::mesh(mesh));

        // Glitches: sparks where (and when) they landed.
        for spark in &view.sparks {
            let age = ((self.now - spark.at) / SPARKLE) as f32;
            let fade = (1.0 - age).clamp(0.0, 1.0);
            let at = pos2(x_of(spark.at), y_of(spark.hz));
            if spark.kind == glitch::SKIP {
                // A skip: the slice's pitch, stuttering.
                for k in 0..4 {
                    let x = at.x - (k as f32 * 4.0 - 6.0) * z;
                    painter.line_segment([pos2(x, at.y - 4.0 * z), pos2(x, at.y + 4.0 * z)], Stroke::new(1.4 * z, glow(Color32::WHITE, 0.7 * fade)));
                }
                continue;
            }
            let r = (2.0 + 4.0 * fade) * z;
            painter.circle_filled(at, r * 1.8, glow(Color32::WHITE, 0.25 * fade));
            painter.line_segment([at - vec2(r * 2.2, 0.0), at + vec2(r * 2.2, 0.0)], Stroke::new(0.8 * z, glow(Color32::WHITE, 0.8 * fade)));
            painter.line_segment([at - vec2(0.0, r * 2.2), at + vec2(0.0, r * 2.2)], Stroke::new(0.8 * z, glow(Color32::WHITE, 0.8 * fade)));
            painter.circle_filled(at, 1.4 * z, alpha(Color32::WHITE, fade));
        }

        // The chord now, on the right edge, each name on a dark tag and
        // pushed apart where the notes are close (once a note, sounding
        // voices only).
        let run = self.run();
        let mut tags: Vec<(f32, usize)> = Vec::with_capacity(VOICES);
        let note_of = |voice: usize| p.viz[viz::NOTE + voice].get().round() as i32;
        for voice in (0..VOICES).filter(|&k| self.live(k) > 0.05) {
            if tags.iter().all(|&(_, u)| note_of(u) != note_of(voice)) {
                tags.push(((y_of(note_hz(p.viz[viz::NOTE + voice].get())) - rect.top()) / z + TOP, voice));
            }
        }
        tags.sort_by(|a, b| b.0.total_cmp(&a.0));
        for k in 1..tags.len() {
            tags[k].0 = tags[k].0.min(tags[k - 1].0 - 10.0);
        }
        for (y, voice) in tags {
            let note = note_of(voice);
            let tag = v.r(W - PAD - 30.0, y - 4.5, W - PAD - 4.0, y + 4.5);
            v.painter.rect_filled(tag, 2.0 * z, alpha(theme::INSET, 0.8));
            let col = alpha(VOICE_COLORS[voice], 0.5 + 0.5 * run);
            v.text(W - PAD - 17.0, y, Align2::CENTER_CENTER, note_label(note), v.mono(6.5), col);
        }
        let state = if run > 1e-3 { "" } else { "waiting for Start (or a key)" };
        v.text(AURORA_X + 8.0, TOP + 10.0, Align2::LEFT_CENTER, "AURORA", v.font(6.5), theme::TEXT_FAINT);
        v.text(AURORA_X + 48.0, TOP + 10.0, Align2::LEFT_CENTER, state, v.font(6.5), theme::TEXT_FAINT);
    }

    /// The timbre's motion over one EVOLVE period round now, scaled to
    /// fill the strip (its shape is what matters): the past solid, the
    /// future (it is known) faint, the slowest octave alone as a glow.
    fn strip(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let [y0, y1] = STRIP;
        let rect = v.r(PAD, y0, W - PAD, y1);
        v.painter.rect_filled(rect, 4.0 * z, theme::INSET);
        let painter = v.painter.with_clip_rect(rect.intersect(v.painter.clip_rect()));
        let seed = ae::seed(p);
        let pos = p.viz[viz::POS].get() as f64;
        let gain = fractal::gain(p.get(ROUGH));
        let inner = rect.shrink2(vec2(70.0 * z, 4.0 * z));
        const N: usize = 240;
        let at = |i: usize| pos - 0.5 + i as f64 / N as f64;
        let full: Vec<f32> = (0..=N).map(|i| fractal::fbm(seed, LANE_COLOR, at(i), gain)).collect();
        let slow: Vec<f32> = (0..=N).map(|i| fractal::octave(seed, LANE_COLOR, 0, at(i)) * 0.6).collect();
        let reach = full.iter().chain(&slow).fold(0.12f32, |m, v| m.max(v.abs()));
        let y_of = |m: f32| inner.center().y - (m / reach * 0.9).clamp(-1.0, 1.0) * inner.height() * 0.5;
        let curve = |values: &[f32]| -> Vec<Pos2> {
            values.iter().enumerate().map(|(i, &m)| pos2(inner.left() + i as f32 / N as f32 * inner.width(), y_of(m))).collect()
        };
        painter.hline(inner.x_range(), inner.center().y, Stroke::new(0.6 * z, alpha(theme::TEXT_FAINT, 0.15)));
        // Bar lines (synced) or a line every tenth of the period.
        let (ticks, every) = if p.toggled() {
            let bars = ae::evolve_bars(p.get(EVOLVE)) as f64;
            (bars, if bars > 16.0 { 4.0 } else { 1.0 })
        } else {
            (10.0, 1.0)
        };
        let first = ((pos - 0.5) * ticks / every).ceil() as i64;
        let last = ((pos + 0.5) * ticks / every).floor() as i64;
        for k in first..=last {
            let t = (k as f64 * every / ticks - (pos - 0.5)) as f32;
            let x = inner.left() + t * inner.width();
            painter.vline(x, inner.y_range(), Stroke::new(0.6 * z, alpha(theme::TEXT_FAINT, 0.2)));
        }
        painter.add(Shape::line(curve(&slow), Stroke::new(3.0 * z, glow(self.accent, 0.12))));
        let full = curve(&full);
        let (past, future) = full.split_at(N / 2 + 1);
        painter.add(Shape::line(past.to_vec(), Stroke::new(1.4 * z, self.accent)));
        let mut ahead = vec![past[past.len() - 1]];
        ahead.extend_from_slice(future);
        painter.add(Shape::line(ahead, Stroke::new(1.0 * z, alpha(self.accent, 0.35))));
        let now_x = inner.center().x;
        painter.vline(now_x, rect.y_range(), Stroke::new(1.0 * z, alpha(theme::TEXT, 0.4)));
        let color_now = p.viz[viz::COLOR].get();
        painter.circle_filled(past[past.len() - 1], 3.0 * z, glow(self.accent, 0.9));
        let mid = (y0 + y1) * 0.5;
        v.text(PAD + 8.0, mid - 5.0, Align2::LEFT_CENTER, "TIMBRE", v.font(6.5), theme::TEXT_FAINT);
        v.text(PAD + 8.0, mid + 5.0, Align2::LEFT_CENTER, spectrum_name(color_now), theme::strong(7.0 * z), theme::TEXT_DIM);
        v.text(W - PAD - 8.0, mid - 5.0, Align2::RIGHT_CENTER, "PERIOD", v.font(6.5), theme::TEXT_FAINT);
        v.text(W - PAD - 8.0, mid + 5.0, Align2::RIGHT_CENTER, ae::format(p, EVOLVE), v.mono(7.0), theme::TEXT_DIM);
    }

    /// The knob groups.
    fn groups(&self) {
        let (v, z) = (self.v, self.z());
        for (title, x0, x1, y, knobs) in GROUPS {
            v.text(x0, y, Align2::LEFT_CENTER, title, theme::strong(8.5 * z), theme::TEXT_DIM);
            // GLITCH has its step lights and grid on the title line, VOICE
            // its GLIDE.
            if title != "GLITCH" {
                let end = if title == "VOICE" { GLIDE_X - 8.0 } else { x1 };
                v.painter.hline(v.p(x0 + 62.0, 0.0).x..=v.p(end, 0.0).x, v.p(0.0, y).y, Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.3)));
            }
            let cw = (x1 - x0) / knobs.len() as f32;
            for (k, (i, label)) in knobs.iter().enumerate() {
                self.knob(*i, x0 + cw * (k as f32 + 0.5), y + 14.0, label, 10.0);
            }
        }
    }

    /// GLIDE, on the VOICE title line.
    fn glide(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let on = ae::glides(p);
        let rect = v.r(GLIDE_X, ROW_A - 8.0, GLIDE_X + 54.0, ROW_A + 8.0);
        let resp = widgets::toggle(v.ui, v.painter, self.uid("glide"), rect, "GLIDE", on, self.accent, z)
            .on_hover_text("On, the voices glide to their new notes (portamento); off, they step there");
        if resp.clicked() {
            p.data[data::GLIDE].set(if on { 0.0 } else { 1.0 });
        }
    }

    /// SYNC and RESEED beside the motion's knobs, and what the period
    /// comes to.
    fn motion_extras(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let x = SPLIT + 146.0;
        let synced = p.toggled();
        let resp = widgets::toggle(v.ui, v.painter, self.uid("sync"), v.r(x, ROW_A + 18.0, x + 54.0, ROW_A + 34.0), "SYNC", synced, self.accent, z)
            .on_hover_text("EVOLVE in bars of the board's tempo (the motion turns on the bar lines), or free in seconds");
        if resp.clicked() {
            p.set_toggle(!synced);
        }
        let reseed = widgets::toggle(v.ui, v.painter, self.uid("reseed"), v.r(x, ROW_A + 38.0, x + 54.0, ROW_A + 54.0), "RESEED", false, self.accent, z)
            .on_hover_text("Another evolution: the same settings, moving another way");
        if reseed.clicked() {
            ae::reseed(p, self.seed());
        }
        let seconds = if synced {
            ae::evolve_bars(p.get(EVOLVE)) * 4.0 * 60.0 / v.tempo.max(1.0)
        } else {
            ae::evolve_seconds(p.get(EVOLVE))
        };
        // Octaves faster than 0.12 s are left out: they would tremble.
        let fastest = (seconds / 32.0).max(0.12).min(seconds);
        let line = if synced { format!("{:.1} s at {:.0} BPM", seconds, v.tempo) } else { format!("{seconds:.0} s, free") };
        v.text(x + 64.0, ROW_A + 22.0, Align2::LEFT_CENTER, line, v.font(7.5), theme::TEXT_DIM);
        v.text(x + 64.0, ROW_A + 34.0, Align2::LEFT_CENTER, format!("octaves from {seconds:.1} s to {fastest:.2} s"), v.font(7.0), theme::TEXT_FAINT);
        v.text(x + 64.0, ROW_A + 46.0, Align2::LEFT_CENTER, format!("seed {}", ae::seed(p)), v.mono(6.5), theme::TEXT_FAINT);
    }

    /// The glitch grid's step lights (white as a glitch lands) and its
    /// division, on the GLITCH title line.
    fn glitch_grid(&self, view: &View) {
        let (v, p) = (self.v, self.p);
        const TIPS: [&str; 6] = ["Quarter notes", "Eighth notes", "Eighth-note triplets", "Sixteenth notes", "Sixteenth-note triplets", "Thirty-second notes"];
        let rect = v.r(W - PAD - 220.0, ROW_B - 8.0, W - PAD, ROW_B + 8.0);
        let picked = segmented(v, self.uid("div"), rect, DIVISIONS.len(), ae::division(p), self.accent, |k, r, col| {
            v.painter.text(r.center(), Align2::CENTER_CENTER, DIVISIONS[k].0, v.font(7.0), col);
            TIPS[k]
        });
        if let Some(k) = picked {
            p.data[data::DIV].set(k as f32);
        }
        let step = p.viz[viz::STEP].get();
        let now = if step >= 0.0 { Some((step as i64).rem_euclid(8) as usize) } else { None };
        let landed = view.sparks.back().is_some_and(|s| self.now - s.at < 0.2);
        for k in 0..8 {
            let c = v.p(SPLIT + 68.0 + k as f32 * 7.0, ROW_B);
            let here = now == Some(k);
            let col = match (here, landed) {
                (true, true) => Color32::WHITE,
                (true, false) => self.accent,
                _ => alpha(theme::TEXT_FAINT, 0.4),
            };
            if here && landed {
                v.painter.circle_filled(c, 4.0 * v.z, glow(Color32::WHITE, 0.5));
            }
            v.painter.circle_filled(c, if here { 2.0 } else { 1.4 } * v.z, col);
        }
    }

    /// A knob at `cx`: its label at `y`, the knob under it, the value under
    /// that.
    fn knob(&self, i: usize, cx: f32, y: f32, label: &str, r: f32) {
        let (v, p, z) = (self.v, self.p, self.z());
        let spec = ae::spec(i);
        v.text(cx, y, Align2::CENTER_CENTER, label, v.font(7.5), theme::TEXT_FAINT);
        let top = y + 8.0;
        let mut norm = spec.to_norm(p.get(i));
        let mut resp = widgets::knob(v.ui, v.painter, self.uid(("knob", i)), v.p(cx, top + r), r * z, &mut norm, spec.to_norm(spec.default), spec.bipolar, self.accent, z);
        if let Some((_, tip)) = TIPS.iter().find(|(k, _)| *k == i) {
            resp = resp.on_hover_text(*tip);
        }
        v.hit(resp.rect, ParamRef::Module(i));
        if resp.dragged() || resp.double_clicked() {
            let value = spec.denorm(norm);
            let value = if matches!(spec.unit, crate::audio::params::Unit::Integer | crate::audio::params::Unit::Note) { value.round() } else { value };
            if value != p.get(i) {
                p.set(i, value);
            }
        }
        let hot = resp.dragged() || resp.hovered();
        v.text(cx, top + 2.0 * r + 8.0, Align2::CENTER_CENTER, ae::format(p, i), v.mono(7.0), if hot { theme::TEXT } else { theme::TEXT_DIM });
    }
}

/// The sounding voice and partial nearest a glitch's frequency (partials
/// past the eighth go to the outermost tip).
fn nearest_partial(notes: &[f32; VOICES], live: &[f32; VOICES], hz: f32) -> (usize, usize) {
    let mut best = (0, 0, f32::MAX);
    for (voice, &note) in notes.iter().enumerate().filter(|&(k, _)| live[k] > 0.05) {
        let f = note_hz(note);
        let h = (hz / f).round().max(1.0);
        let miss = (hz / (f * h)).log2().abs();
        if miss < best.2 {
            best = (voice, (h as usize).min(PARTIALS) - 1, miss);
        }
    }
    (best.0, best.1)
}

