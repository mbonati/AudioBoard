//! Panel of Vesper. Up top the transport of ideas: PLAY, GENERATE, SHUFFLE,
//! the presets, LOOP or EVOLVE, the MixType. The stage is a landscape that
//! plays: far off the Ambience, then the Pad, a bank of Noise as fog, the Bass
//! in front, each ridge rising with its layer and rolling with its filters;
//! the chord's notes hang in the sky as beams, Shimmer's pings come out as
//! stars, and the moon crosses once for every chord. Beside it the moods, the
//! root and the rules of the chords; under it the chords themselves. Then a
//! strip per layer (its sound, lock, mute and level), and the knobs of the one
//! selected, and those they all share.

use std::collections::VecDeque;
use std::f32::consts::{PI, TAU};

use egui::epaint::Mesh;
use egui::{Align2, Color32, Id, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};

use super::nodes::{NodeView, segmented};
use super::theme::{self, alpha, mix};
use super::{preset_menu, widgets};
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::vesper::fx::{FILTERS, MIX_TYPES};
use crate::audio::modules::vesper::harmony::{self, MOODS, Step};
use crate::audio::modules::vesper::{
    self as ve, AMBIENCE, BASS, CHORDS, COMPLEXITY, DURATION, ECHO, EVOLVE, FACTORY, LAYER_NAMES,
    LAYERS, LEVEL, LOOP, MIXTYPE, MOOD, NOISE, PAD, PULSE, RATE, REPEATS, ROOT, SHIMMER, SPACE,
    VARIATION, data, k, knob, viz,
};
use crate::audio::params::{NOTE_NAMES, note_label};
use crate::graph::{ModuleNode, ParamRef};

pub const SIZE: Vec2 = vec2(860.0, 508.0);
const W: f32 = SIZE.x;
const PAD_X: f32 = 12.0;
/// The stage, and what stands beside it.
const TOP: f32 = 70.0;
const STAGE_R: f32 = 548.0;
const STAGE_B: f32 = 226.0;
const SIDE_L: f32 = 558.0;
/// The chords, the strips, the knobs of the layer, the knobs they share.
const CARDS: [f32; 2] = [232.0, 272.0];
const STRIPS: [f32; 2] = [278.0, 352.0];
const ROW_A: f32 = 362.0;
const ROW_B: f32 = 444.0;

/// Each layer's colour.
const LAYER_COLORS: [Color32; LAYERS] = [
    Color32::from_rgb(138, 146, 255),
    Color32::from_rgb(236, 112, 132),
    Color32::from_rgb(178, 182, 196),
    Color32::from_rgb(88, 204, 190),
    Color32::from_rgb(248, 216, 132),
];

/// The sky for each mood: high up, at the horizon.
const SKIES: [(Color32, Color32); 16] = [
    (
        Color32::from_rgb(40, 70, 120),
        Color32::from_rgb(244, 186, 120),
    ),
    (
        Color32::from_rgb(52, 48, 120),
        Color32::from_rgb(238, 150, 180),
    ),
    (
        Color32::from_rgb(38, 66, 96),
        Color32::from_rgb(226, 170, 110),
    ),
    (
        Color32::from_rgb(58, 62, 118),
        Color32::from_rgb(250, 170, 130),
    ),
    (
        Color32::from_rgb(20, 26, 50),
        Color32::from_rgb(96, 88, 130),
    ),
    (
        Color32::from_rgb(20, 36, 60),
        Color32::from_rgb(90, 120, 130),
    ),
    (
        Color32::from_rgb(28, 16, 36),
        Color32::from_rgb(150, 66, 76),
    ),
    (
        Color32::from_rgb(18, 22, 54),
        Color32::from_rgb(92, 72, 150),
    ),
    (Color32::from_rgb(16, 16, 30), Color32::from_rgb(82, 62, 80)),
    (Color32::from_rgb(8, 8, 14), Color32::from_rgb(48, 44, 56)),
    (Color32::from_rgb(12, 16, 26), Color32::from_rgb(60, 76, 92)),
    (
        Color32::from_rgb(26, 10, 24),
        Color32::from_rgb(120, 50, 60),
    ),
    (
        Color32::from_rgb(52, 36, 42),
        Color32::from_rgb(226, 138, 76),
    ),
    (
        Color32::from_rgb(40, 24, 50),
        Color32::from_rgb(214, 122, 90),
    ),
    (
        Color32::from_rgb(10, 14, 44),
        Color32::from_rgb(70, 110, 180),
    ),
    (
        Color32::from_rgb(10, 26, 50),
        Color32::from_rgb(60, 170, 170),
    ),
];

/// A star lives this long, after its ping.
const STAR_LIFE: f64 = 3.5;

/// One knob: index, label.
type Knobs = [(usize, &'static str)];

/// The rules of the chords, beside the stage.
const RULE_KNOBS: [(usize, &str); 4] = [
    (VARIATION, "VARIATION"),
    (COMPLEXITY, "NOTES"),
    (DURATION, "DURATION"),
    (CHORDS, "CHORDS"),
];
/// What they all share.
const SHARED: [(&str, f32, f32, &Knobs); 3] = [
    (
        "SPACE AND ECHO",
        PAD_X,
        330.0,
        &[(SPACE, "SPACE"), (ECHO, "ECHO TIME"), (REPEATS, "REPEATS")],
    ),
    (
        "SHIMMER",
        342.0,
        560.0,
        &[(RATE, "PINGS"), (PULSE, "PULSE")],
    ),
    ("OUTPUT", 572.0, 660.0, &[(LEVEL, "LEVEL")]),
];

const TIPS: [(usize, &str); 9] = [
    (
        VARIATION,
        "How many of the scale's seven degrees chords may stand on.\n1 stays on the tonic; 7 goes everywhere",
    ),
    (
        COMPLEXITY,
        "How many notes each chord has: two (open), three, or four",
    ),
    (DURATION, "How long each chord lasts, on the board's bars"),
    (CHORDS, "LOOP: how many chords before it comes round again"),
    (
        SPACE,
        "The long space the layers send to: how large, and how much of it",
    ),
    (
        ECHO,
        "The echo the layers send to, bouncing from side to side: its time, on the board's tempo",
    ),
    (REPEATS, "How long the echo goes on"),
    (
        RATE,
        "How often Shimmer strikes, on average, when it isn't an ostinato",
    ),
    (
        PULSE,
        "OSTINATO: the note value it plays in, on the board's tempo",
    ),
];

/// A layer's knobs: title, knobs (which, label, tip).
type LayerKnob = (usize, &'static str, &'static str);
const ENVELOPE: [LayerKnob; 6] = [
    (
        k::OCTAVE,
        "OCTAVE",
        "Up or down, in whole octaves. On a layer with no pitch, how fast the sound plays",
    ),
    (k::ATTACK, "ATTACK", "How long a note takes to come in"),
    (
        k::HOLD,
        "HOLD",
        "How long a note holds before it lets go by itself: short for a brass stab; at the top it sustains",
    ),
    (
        k::RELEASE,
        "RELEASE",
        "How long a note takes to go: chords overlap for as long as this",
    ),
    (
        k::BLOOM,
        "BLOOM",
        "On each new chord the first filter starts this far below itself and opens as the note comes in: the swell of brass",
    ),
    (k::WIDTH, "WIDTH", "From mono to as wide as it goes"),
];
const MOTION1: [LayerKnob; 3] = [
    (k::FREQ1, "FREQ", "The first filter's frequency"),
    (k::MOVE1, "MOTION", "How far it moves on its own"),
    (k::RATE1, "RATE", "How fast it moves"),
];
const MOTION2: [LayerKnob; 3] = [
    (k::FREQ2, "FREQ", "The second filter's frequency"),
    (k::MOVE2, "MOTION", "How far it moves on its own"),
    (k::RATE2, "RATE", "How fast it moves"),
];
const EFFECTS: [LayerKnob; 4] = [
    (k::DRIVE, "DRIVE", "Soft saturation"),
    (k::CHORUS, "CHORUS", "Three swaying copies a side"),
    (k::ECHO, "ECHO", "How much goes to the echo"),
    (k::REVERB, "REVERB", "How much goes to the space"),
];

/// The node's two summary lines: the mood and key, what is playing.
pub fn readout(p: &ModuleParams) -> (String, String) {
    let rules = ve::rules(p);
    let key = harmony::note_name(&rules, rules.root);
    let main = format!("{key} · {}", MOODS[rules.mood].name);
    let second = if !p.toggled() {
        "keys play".to_owned()
    } else if p.viz[viz::RUN].get() > 0.5 {
        let index = p.viz[viz::INDEX].get().max(0.0) as usize;
        harmony::label(
            &rules,
            Step::unpack(p.viz[viz::CARD + index.min(harmony::MAX_CHORDS - 1)].get()),
        )
    } else if ve::evolving(p) {
        "evolve".to_owned()
    } else {
        format!("loop of {}", ve::chords(p))
    };
    (main, second)
}

/// A ping, on its way out.
#[derive(Clone, Copy)]
struct Star {
    at: f64,
    note: f32,
    pan: f32,
}

/// What the panel remembers between frames.
#[derive(Clone, Default)]
struct View {
    stars: VecDeque<Star>,
    fired: f32,
    /// Each ridge's height, gliding.
    height: [f32; LAYERS],
}

impl View {
    fn update(&mut self, p: &ModuleParams, now: f64) {
        let fired = p.viz[viz::FIRED].get();
        if fired < self.fired {
            // A new processor (the node was rebuilt).
            self.fired = 0.0;
        }
        let (from, to) = (self.fired as u32 + 1, fired as u32);
        for n in from.max(to.saturating_sub(viz::EVENTS as u32 - 1))..=to {
            let at = viz::EVENT + 3 * (n as usize % viz::EVENTS);
            if p.viz[at].get() as u32 == n {
                self.stars.push_back(Star {
                    at: now,
                    note: p.viz[at + 1].get(),
                    pan: p.viz[at + 2].get(),
                });
            }
        }
        self.fired = fired;
        while self.stars.front().is_some_and(|s| now - s.at > STAR_LIFE) || self.stars.len() > 48 {
            self.stars.pop_front();
        }
        for (l, h) in self.height.iter_mut().enumerate() {
            let level = p.viz[viz::PEAK + l].get().clamp(0.0, 1.0);
            *h += (level.sqrt() - *h) * 0.04;
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

/// `(midi in, midi out, out)`: what is patched.
pub fn draw(
    v: &NodeView,
    id: NodeId,
    accent: Color32,
    m: &ModuleNode,
    (midi_in, midi_out, out): (bool, bool, bool),
) {
    let p = &m.params;
    let key = Id::unique(("vesper-view", id));
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
            theme::TEXT_DIM
        } else {
            theme::TEXT_FAINT
        }
    };
    v.text(
        PAD_X,
        42.0,
        Align2::LEFT_CENTER,
        "MIDI",
        v.font(9.0),
        col(midi_in),
    );
    v.text(
        W - PAD_X,
        42.0,
        Align2::RIGHT_CENTER,
        "OUT",
        v.font(9.0),
        col(out),
    );
    v.text(
        W - PAD_X,
        60.0,
        Align2::RIGHT_CENTER,
        "MIDI",
        v.font(9.0),
        col(midi_out),
    );
    s.header();
    s.stage(&view);
    s.moods();
    s.cards();
    let selected = s.selected();
    for layer in 0..LAYERS {
        s.strip(layer, selected);
    }
    s.layer_knobs(selected);
    s.shared();
    let busy = p.viz[viz::RUN].get() > 0.5
        || view.height.iter().any(|&h| h > 0.01)
        || !view.stars.is_empty();
    if busy {
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
        Id::unique(("vesper", self.id)).with(salt)
    }

    /// A fresh seed, for GENERATE and SHUFFLE.
    fn seed(&self) -> u32 {
        (self.now * 1000.0) as u32 ^ self.id.wrapping_mul(0x9e37_79b9)
    }

    fn selected(&self) -> usize {
        (self.p.data[data::SELECTED].get().round().max(0.0) as usize).min(LAYERS - 1)
    }

    /// PLAY, GENERATE, SHUFFLE, the presets, the mode, the MixType.
    fn header(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let playing = p.toggled();
        let play = widgets::toggle(v.ui, v.painter, self.uid("play"), v.r(46.0, 34.0, 88.0, 50.0), "PLAY", playing, self.accent, z)
            .on_hover_text("On: it plays by itself with the transport, and sends its score on the MIDI output.\nOff: the keys on the MIDI input play it");
        if play.clicked() {
            p.set_toggle(!playing);
        }
        let generate = widgets::toggle(
            v.ui,
            v.painter,
            self.uid("generate"),
            v.r(94.0, 34.0, 158.0, 50.0),
            "GENERATE",
            false,
            self.accent,
            z,
        )
        .on_hover_text("New chords, on the same sounds and within the same rules");
        if generate.clicked() {
            ve::generate(p, self.seed());
        }
        let shuffle = widgets::toggle(
            v.ui,
            v.painter,
            self.uid("shuffle"),
            v.r(164.0, 34.0, 220.0, 50.0),
            "SHUFFLE",
            false,
            self.accent,
            z,
        )
        .on_hover_text("New sounds on every layer that isn't locked. The chords stay");
        if shuffle.clicked() {
            ve::shuffle(p, self.seed());
        }
        let menu = widgets::toggle(
            v.ui,
            v.painter,
            self.uid("presets"),
            v.r(226.0, 34.0, 300.0, 50.0),
            "PRESETS ▾",
            false,
            self.accent,
            z,
        )
        .on_hover_text(
            "Save this scene as a preset, or load one (built-in ones keep your ROOT and LEVEL)",
        );
        let names: Vec<&str> = FACTORY.iter().map(|s| s.name).collect();
        preset_menu::show(&menu, p, self.uid("presets"), "scene", &names, |n| {
            ve::load_factory(p, n)
        });

        const MODES: [(&str, &str); 2] = [
            ("LOOP", "Repeats a progression of CHORDS chords"),
            (
                "EVOLVE",
                "Keeps writing new chords, with no end, within the same rules",
            ),
        ];
        let picked = segmented(
            v,
            self.uid("mode"),
            v.r(310.0, 34.0, 416.0, 50.0),
            2,
            p.mode().min(EVOLVE),
            self.accent,
            |n, r, col| {
                v.painter.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    MODES[n].0,
                    v.font(7.0),
                    col,
                );
                MODES[n].1
            },
        );
        if let Some(n) = picked {
            p.set_mode(if n == 1 { EVOLVE } else { LOOP });
        }
        let which = (p.get(MIXTYPE).round().max(0.0) as usize).min(MIX_TYPES.len() - 1);
        v.text(
            426.0,
            42.0,
            Align2::LEFT_CENTER,
            "MIX",
            v.font(7.0),
            theme::TEXT_FAINT,
        );
        let picked = segmented(
            v,
            self.uid("mix"),
            v.r(446.0, 34.0, 660.0, 50.0),
            MIX_TYPES.len(),
            which,
            self.accent,
            |n, r, col| {
                v.painter.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    MIX_TYPES[n].0,
                    v.font(7.0),
                    col,
                );
                MIX_TYPES[n].1
            },
        );
        if let Some(n) = picked {
            p.set(MIXTYPE, n as f32);
        }
        let status = if !playing {
            "keys play it"
        } else if p.viz[viz::RUN].get() > 0.5 {
            ""
        } else {
            "start the transport"
        };
        v.text(
            W - PAD_X - 34.0,
            42.0,
            Align2::RIGHT_CENTER,
            status,
            v.font(7.5),
            if playing {
                theme::TEXT_FAINT
            } else {
                self.accent
            },
        );
    }

    /// The landscape.
    fn stage(&self, view: &View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let rect = v.r(PAD_X, TOP, STAGE_R, STAGE_B);
        v.painter.rect_filled(rect, 4.0 * z, theme::INSET);
        let painter = v
            .painter
            .with_clip_rect(rect.intersect(v.painter.clip_rect()));
        let t = self.now as f32;
        let (high, low) = SKIES[ve::rules(p).mood];
        let running = p.viz[viz::RUN].get() > 0.5;
        let lit = if running { 1.0 } else { 0.55 };
        let horizon = rect.top() + rect.height() * 0.72;

        // The sky.
        let mut sky = Mesh::default();
        let dim = |c: Color32| mix(theme::INSET, c, lit);
        sky.colored_vertex(rect.left_top(), dim(high));
        sky.colored_vertex(rect.right_top(), dim(high));
        sky.colored_vertex(pos2(rect.right(), horizon), dim(low));
        sky.colored_vertex(pos2(rect.left(), horizon), dim(low));
        sky.colored_vertex(rect.right_bottom(), dim(low));
        sky.colored_vertex(rect.left_bottom(), dim(low));
        sky.add_triangle(0, 1, 2);
        sky.add_triangle(0, 2, 3);
        sky.add_triangle(3, 2, 4);
        sky.add_triangle(3, 4, 5);
        painter.add(Shape::mesh(sky));

        // The chord's notes hang as beams.
        let note_y = |note: f32| {
            rect.top()
                + 12.0 * z
                + ((84.0 - note) / 40.0).clamp(0.0, 1.0) * (horizon - rect.top() - 24.0 * z)
        };
        let count = (p.viz[viz::NOTES].get().max(0.0) as usize).min(harmony::MAX_TONES);
        for n in 0..count {
            let note = p.viz[viz::NOTE + n].get();
            let y = note_y(note);
            let sway = 5.0 * z * (t * 0.3 + n as f32).sin();
            let (from, to) = (
                pos2(rect.left(), y + sway),
                pos2(rect.right() - 20.0 * z, y),
            );
            painter.line_segment(
                [from, to],
                Stroke::new(3.0 * z, alpha(Color32::WHITE, 0.04)),
            );
            painter.line_segment([from, to], Stroke::new(0.8 * z, alpha(Color32::WHITE, 0.2)));
            // The label stays put at the beam's still end, so that close notes stay legible.
            painter.text(
                pos2(rect.right() - 4.0 * z, y),
                Align2::RIGHT_CENTER,
                note_label(note.round() as i32),
                theme::font(6.5 * z),
                alpha(Color32::WHITE, 0.6),
            );
        }

        // The moon crosses once for every chord.
        let progress = p.viz[viz::PROGRESS].get().clamp(0.0, 1.0);
        let moon = pos2(
            rect.left() + rect.width() * (0.04 + 0.92 * progress),
            horizon - (horizon - rect.top() - 18.0 * z) * (PI * progress).sin().max(0.12),
        );
        let glow = if running { 1.0 } else { 0.4 };
        for r in [20.0, 13.0, 8.0] {
            painter.circle_filled(moon, r * z, alpha(Color32::WHITE, 0.035 * glow));
        }
        painter.circle_filled(
            moon,
            5.0 * z,
            alpha(Color32::from_rgb(255, 246, 226), 0.9 * glow),
        );

        // Stars, where the pings land.
        let gold = LAYER_COLORS[SHIMMER];
        for star in &view.stars {
            let age = ((self.now - star.at) / STAR_LIFE) as f32;
            let fade = (1.0 - age).clamp(0.0, 1.0).powi(2);
            let x = rect.center().x + star.pan * rect.width() * 0.48;
            let y = rect.top() + (6.0 + ((108.0 - star.note) / 36.0).clamp(0.0, 1.0) * 56.0) * z;
            let size = (1.2 + 3.0 * fade) * z;
            painter.circle_filled(pos2(x, y), size * 2.6, alpha(gold, 0.16 * fade));
            painter.circle_filled(pos2(x, y), size * 0.6, alpha(Color32::WHITE, fade));
            painter.line_segment(
                [pos2(x - size * 2.2, y), pos2(x + size * 2.2, y)],
                Stroke::new(0.6 * z, alpha(gold, 0.6 * fade)),
            );
            painter.line_segment(
                [pos2(x, y - size * 2.2), pos2(x, y + size * 2.2)],
                Stroke::new(0.6 * z, alpha(gold, 0.6 * fade)),
            );
        }

        // The ridges, far to near.
        self.ridge(&painter, rect, view, AMBIENCE, 0.62, 16.0, 1.9, 0.03, 0.55);
        self.ridge(&painter, rect, view, PAD, 0.70, 20.0, 2.8, 0.05, 0.8);
        self.ridge(&painter, rect, view, NOISE, 0.80, 7.0, 8.0, 0.14, 0.32);
        self.ridge(&painter, rect, view, BASS, 0.90, 11.0, 1.4, 0.02, 1.0);

        // Sounds still being made.
        let making =
            (0..ve::SAMPLED).any(|l| p.viz[viz::LOADED + l].get() != ve::sound(p, l) as f32);
        if making {
            painter.text(
                rect.left_top() + vec2(8.0, 8.0) * z,
                Align2::LEFT_TOP,
                "making the sound…",
                theme::font(7.5 * z),
                alpha(Color32::WHITE, 0.7),
            );
        }
    }

    /// A ridge, drawn in columns: it rises with the layer, rolls with its
    /// filters, and turns to fog if `solid` is low.
    #[allow(clippy::too_many_arguments)]
    fn ridge(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        view: &View,
        layer: usize,
        base: f32,
        amp: f32,
        folds: f32,
        speed: f32,
        solid: f32,
    ) {
        let (p, z) = (self.p, self.z());
        let level = view.height[layer].clamp(0.0, 1.0);
        let shown = !ve::flag(p, data::MUTE + layer);
        let f1 = p.viz[viz::FILTER1 + layer].get().clamp(0.0, 1.0);
        let f2 = p.viz[viz::FILTER2 + layer].get().clamp(0.0, 1.0);
        let t = self.now as f32 * speed;
        let rise = if shown { level } else { 0.0 };
        let top = rect.top() + rect.height() * (base + 0.1 * (1.0 - rise));
        let height = amp * z * (0.35 + 0.65 * f1 + 0.5 * rise);
        const COLUMNS: usize = 110;
        let seed = layer as f32 * 2.3;
        let points: Vec<Pos2> = (0..=COLUMNS)
            .map(|c| {
                let x = c as f32 / COLUMNS as f32;
                let w = TAU * folds * x;
                let y = 0.55 * (w + seed + t).sin()
                    + 0.3 * (2.3 * w - 1.4 * t + seed * 1.7).sin()
                    + 0.15 * ((3.0 + 6.0 * f2) * w + 2.2 * t).sin();
                pos2(rect.left() + rect.width() * x, top - height * y)
            })
            .collect();
        let color = LAYER_COLORS[layer];
        let body = mix(
            theme::INSET,
            color,
            0.12 + 0.5 * rise * if shown { 1.0 } else { 0.0 },
        );
        let fill = alpha(body, solid);
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let strip = vec![a, b, pos2(b.x, rect.bottom()), pos2(a.x, rect.bottom())];
            painter.add(Shape::convex_polygon(strip, fill, Stroke::NONE));
        }
        painter.add(Shape::line(
            points,
            Stroke::new(1.0 * z, alpha(color, (0.2 + 0.7 * rise) * solid)),
        ));
    }

    /// The sixteen moods, the root and the rules of the chords.
    fn moods(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let right = W - PAD_X;
        v.text(
            SIDE_L,
            68.0,
            Align2::LEFT_CENTER,
            "MOOD",
            theme::strong(8.0 * z),
            theme::TEXT_DIM,
        );
        let mood = (p.get(MOOD).round().max(0.0) as usize).min(MOODS.len() - 1);
        v.text(
            SIDE_L + 36.0,
            68.0,
            Align2::LEFT_CENTER,
            MOODS[mood].scale,
            v.font(7.5),
            theme::TEXT_FAINT,
        );
        let cell = (right - SIDE_L) / 4.0;
        for (n, m) in MOODS.iter().enumerate() {
            let x = SIDE_L + cell * (n % 4) as f32;
            let y = 76.0 + 16.0 * (n / 4) as f32;
            let rect = v.r(x + 1.0, y, x + cell - 1.0, y + 14.0);
            let resp =
                v.ui.interact(rect, self.uid(("mood", n)), egui::Sense::CLICK)
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(m.scale);
            let on = n == mood;
            let fill = if on {
                self.accent
            } else if resp.hovered() {
                theme::CONTROL_HI
            } else {
                theme::CONTROL
            };
            v.painter.rect_filled(rect, 2.0 * z, fill);
            v.painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                m.name,
                v.font(7.5),
                if on { theme::INSET } else { theme::TEXT_DIM },
            );
            if resp.clicked() {
                p.set(MOOD, n as f32);
            }
        }
        // The root, as a row of keys.
        v.text(
            SIDE_L,
            150.0,
            Align2::LEFT_CENTER,
            "ROOT",
            theme::strong(8.0 * z),
            theme::TEXT_DIM,
        );
        let root = (p.get(ROOT).round() as i32).rem_euclid(12) as usize;
        let key_w = (right - SIDE_L - 32.0) / 12.0;
        for (n, name) in NOTE_NAMES.iter().enumerate() {
            let x = SIDE_L + 32.0 + key_w * n as f32;
            let rect = v.r(x + 0.5, 142.0, x + key_w - 0.5, 158.0);
            let resp =
                v.ui.interact(rect, self.uid(("root", n)), egui::Sense::CLICK)
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
            let on = n == root;
            let sharp = name.len() > 1;
            let fill = if on {
                self.accent
            } else if resp.hovered() {
                theme::CONTROL_HI
            } else if sharp {
                theme::INSET
            } else {
                theme::CONTROL
            };
            v.painter.rect_filled(rect, 1.5 * z, fill);
            v.painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                *name,
                v.font(6.5),
                if on { theme::INSET } else { theme::TEXT_DIM },
            );
            // A dot on the notes of the scale.
            if !on && ve::rules(p).in_scale(n as i32) {
                v.painter.circle_filled(
                    rect.center_bottom() - vec2(0.0, 2.5) * z,
                    1.2 * z,
                    alpha(self.accent, 0.9),
                );
            }
            if resp.clicked() {
                p.set(ROOT, n as f32);
            }
        }
        // The rules.
        let cw = (right - SIDE_L) / RULE_KNOBS.len() as f32;
        let evolve = ve::evolving(p);
        for (n, &(i, label)) in RULE_KNOBS.iter().enumerate() {
            self.knob(
                i,
                SIDE_L + cw * (n as f32 + 0.5),
                170.0,
                label,
                i == CHORDS && evolve,
                self.accent,
                None,
            );
        }
    }

    /// The chords: the loop, or what EVOLVE has written and what comes next.
    fn cards(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let rules = ve::rules(p);
        let evolve = ve::evolving(p);
        let count = (p.viz[viz::CARDS].get().max(0.0) as usize).clamp(1, harmony::MAX_CHORDS);
        let index = p.viz[viz::INDEX].get().round() as i32;
        let keys = !p.toggled();
        let progress = p.viz[viz::PROGRESS].get().clamp(0.0, 1.0);
        let running = p.viz[viz::RUN].get() > 0.5;
        let width = (W - 2.0 * PAD_X) / harmony::MAX_CHORDS as f32;
        for n in 0..count {
            let x = PAD_X + width * n as f32;
            let rect = v.r(x + 1.5, CARDS[0], x + width - 1.5, CARDS[1]);
            let step = Step::unpack(p.viz[viz::CARD + n].get());
            let now = n as i32 == index && running && !keys;
            let next = evolve && n + 1 == count;
            let fill = if now {
                mix(theme::INSET, self.accent, 0.3)
            } else {
                theme::INSET
            };
            v.painter.rect_filled(rect, 3.0 * z, fill);
            if now {
                v.painter.rect_stroke(
                    rect,
                    3.0 * z,
                    Stroke::new(1.0 * z, self.accent),
                    egui::StrokeKind::Inside,
                );
                let bar = Rect::from_min_max(
                    pos2(rect.left() + 3.0 * z, rect.bottom() - 3.5 * z),
                    pos2(
                        rect.left() + 3.0 * z + (rect.width() - 6.0 * z) * progress,
                        rect.bottom() - 2.0 * z,
                    ),
                );
                v.painter.rect_filled(bar, 0.0, self.accent);
            }
            let ink = if keys {
                theme::TEXT_FAINT
            } else if now {
                theme::TEXT
            } else if next {
                theme::TEXT_FAINT
            } else {
                theme::TEXT_DIM
            };
            v.painter.text(
                rect.center() - vec2(0.0, 5.0) * z,
                Align2::CENTER_CENTER,
                harmony::label(&rules, step),
                theme::strong(11.5 * z),
                ink,
            );
            let below = if next {
                "next".to_owned()
            } else {
                harmony::numeral(&rules, step)
            };
            v.painter.text(
                rect.center() + vec2(0.0, 10.0) * z,
                Align2::CENTER_CENTER,
                below,
                v.font(7.5),
                theme::TEXT_FAINT,
            );
        }
    }

    /// A layer's strip: name, lock, mute, sound, level.
    fn strip(&self, layer: usize, selected: usize) {
        let (v, p, z) = (self.v, self.p, self.z());
        let color = LAYER_COLORS[layer];
        let gap = 6.0;
        let width = (W - 2.0 * PAD_X - gap * (LAYERS - 1) as f32) / LAYERS as f32;
        let x0 = PAD_X + (width + gap) * layer as f32;
        let x1 = x0 + width;
        let rect = v.r(x0, STRIPS[0], x1, STRIPS[1]);
        let chosen = layer == selected;
        v.painter.rect_filled(
            rect,
            3.0 * z,
            if chosen {
                mix(theme::INSET, color, 0.1)
            } else {
                theme::INSET
            },
        );
        v.painter.rect_filled(
            v.r(x0, STRIPS[0], x1, STRIPS[0] + 2.0),
            1.0 * z,
            if chosen { color } else { alpha(color, 0.45) },
        );

        // The name picks it for the knobs below.
        let name_rect = v.r(x0, STRIPS[0] + 2.0, x1 - 74.0, STRIPS[0] + 22.0);
        let name =
            v.ui.interact(name_rect, self.uid(("name", layer)), egui::Sense::CLICK)
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Show this layer's knobs below");
        if name.clicked() {
            p.data[data::SELECTED].set(layer as f32);
        }
        v.text(
            x0 + 6.0,
            STRIPS[0] + 12.0,
            Align2::LEFT_CENTER,
            LAYER_NAMES[layer],
            theme::strong(8.5 * z),
            if chosen { color } else { theme::TEXT_DIM },
        );

        let locked = ve::flag(p, data::LOCK + layer);
        let lock = widgets::toggle(
            v.ui,
            v.painter,
            self.uid(("lock", layer)),
            v.r(x1 - 72.0, STRIPS[0] + 5.0, x1 - 38.0, STRIPS[0] + 19.0),
            "LOCK",
            locked,
            color,
            z,
        )
        .on_hover_text("SHUFFLE leaves this layer's sound alone");
        if lock.clicked() {
            ve::set_flag(p, data::LOCK + layer, !locked);
        }
        let muted = ve::flag(p, data::MUTE + layer);
        let mute = widgets::toggle(
            v.ui,
            v.painter,
            self.uid(("mute", layer)),
            v.r(x1 - 36.0, STRIPS[0] + 5.0, x1 - 4.0, STRIPS[0] + 19.0),
            "MUTE",
            muted,
            theme::MUTE,
            z,
        )
        .on_hover_text("Silence this layer");
        if mute.clicked() {
            ve::set_flag(p, data::MUTE + layer, !muted);
        }

        // The sound: back, its name (a list), on.
        let count = ve::sound_count(layer);
        let sound = ve::sound(p, layer);
        let y = STRIPS[0] + 24.0;
        let prev = widgets::toggle(
            v.ui,
            v.painter,
            self.uid(("prev", layer)),
            v.r(x0 + 4.0, y, x0 + 18.0, y + 16.0),
            "<",
            false,
            color,
            z,
        );
        if prev.clicked() {
            ve::set_sound(p, layer, (sound + count - 1) % count);
        }
        let next = widgets::toggle(
            v.ui,
            v.painter,
            self.uid(("next", layer)),
            v.r(x1 - 18.0, y, x1 - 4.0, y + 16.0),
            ">",
            false,
            color,
            z,
        );
        if next.clicked() {
            ve::set_sound(p, layer, (sound + 1) % count);
        }
        let list_rect = v.r(x0 + 20.0, y, x1 - 20.0, y + 16.0);
        let list =
            v.ui.interact(list_rect, self.uid(("list", layer)), egui::Sense::CLICK)
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Pick a sound");
        v.painter.rect_filled(
            list_rect,
            2.0 * z,
            if list.hovered() {
                theme::CONTROL_HI
            } else {
                theme::CONTROL
            },
        );
        v.painter.text(
            list_rect.center(),
            Align2::CENTER_CENTER,
            ve::sound_name(layer, sound),
            v.font(8.0),
            theme::TEXT,
        );
        egui::Popup::menu(&list).show(|ui| {
            ui.set_min_width(170.0);
            egui::ScrollArea::vertical()
                .max_height(340.0)
                .show(ui, |ui| {
                    for s in 0..count {
                        if ui
                            .selectable_label(s == sound, ve::sound_name(layer, s))
                            .clicked()
                        {
                            ve::set_sound(p, layer, s);
                            ui.close();
                        }
                    }
                });
        });

        // The level, over its meter.
        let i = knob(layer, k::LEVEL);
        let spec = ve::spec(i);
        let fader_rect = v.r(x0 + 8.0, y + 22.0, x1 - 8.0, y + 34.0);
        let mut pos = spec.to_norm(p.get(i));
        let resp = widgets::hfader(
            v.ui,
            v.painter,
            self.uid(("level", layer)),
            fader_rect,
            &mut pos,
            color,
            !muted,
            z,
        );
        v.hit(resp.rect, ParamRef::Module(i));
        let resp = resp.on_hover_text(format!(
            "{} level: {}",
            LAYER_NAMES[layer],
            ve::format(p, i)
        ));
        if resp.double_clicked() {
            p.set(i, spec.default);
        } else if resp.dragged() {
            p.set(i, spec.denorm(pos));
        }
        let peak = p.viz[viz::PEAK + layer].get().clamp(0.0, 1.0).sqrt();
        let (mx0, mx1) = (x0 + 8.0, x1 - 8.0);
        v.painter
            .rect_filled(v.r(mx0, y + 38.0, mx1, y + 40.0), 1.0 * z, theme::CONTROL);
        v.painter.rect_filled(
            v.r(mx0, y + 38.0, mx0 + (mx1 - mx0) * peak, y + 40.0),
            1.0 * z,
            color,
        );
    }

    /// The knobs of the layer selected.
    fn layer_knobs(&self, layer: usize) {
        let (v, z) = (self.v, self.z());
        let color = LAYER_COLORS[layer];
        let groups: [(&str, f32, f32, &[LayerKnob]); 4] = [
            ("SHAPE", PAD_X, 296.0, &ENVELOPE),
            ("MOTION 1", 306.0, 450.0, &MOTION1),
            ("MOTION 2", 460.0, 604.0, &MOTION2),
            ("EFFECTS", 614.0, W - PAD_X, &EFFECTS),
        ];
        for (title, x0, x1, knobs) in groups {
            let shown = format!("{} · {title}", LAYER_NAMES[layer]);
            let label = if title == "SHAPE" {
                shown.as_str()
            } else {
                title
            };
            v.text(
                x0,
                ROW_A,
                Align2::LEFT_CENTER,
                label,
                theme::strong(8.5 * z),
                theme::TEXT_DIM,
            );
            let line_from = if title == "SHAPE" {
                x0 + 92.0
            } else {
                x0 + 100.0
            };
            v.painter.hline(
                v.p(line_from, 0.0).x..=v.p(x1, 0.0).x,
                v.p(0.0, ROW_A).y,
                Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.3)),
            );
            if title == "MOTION 1" {
                self.filter_kind(data::KIND1 + layer, x0 + 54.0, color);
            } else if title == "MOTION 2" {
                self.filter_kind(data::KIND2 + layer, x0 + 54.0, color);
            }
            let cw = (x1 - x0) / knobs.len() as f32;
            for (n, &(which, name, tip)) in knobs.iter().enumerate() {
                // Pings have no note to hold.
                let dim = layer == SHIMMER && which == k::HOLD;
                self.knob(
                    knob(layer, which),
                    x0 + cw * (n as f32 + 0.5),
                    ROW_A + 14.0,
                    name,
                    dim,
                    color,
                    Some(tip),
                );
            }
        }
    }

    /// What a motion filter is: a click takes the next.
    fn filter_kind(&self, d: usize, x: f32, color: Color32) {
        let (v, p, z) = (self.v, self.p, self.z());
        let kind = ve::filter_kind(p, d);
        let resp = widgets::toggle(
            v.ui,
            v.painter,
            self.uid(("kind", d)),
            v.r(x, ROW_A - 7.0, x + 42.0, ROW_A + 7.0),
            FILTERS[kind],
            false,
            color,
            z,
        )
        .on_hover_text(
            "What this filter is: low-pass, high-pass, band or notch. Click for the next",
        );
        if resp.clicked() {
            p.data[d].set(((kind + 1) % FILTERS.len()) as f32);
        }
    }

    /// The knobs they all share.
    fn shared(&self) {
        let (v, z) = (self.v, self.z());
        for (title, x0, x1, knobs) in SHARED {
            v.text(
                x0,
                ROW_B,
                Align2::LEFT_CENTER,
                title,
                theme::strong(8.5 * z),
                theme::TEXT_DIM,
            );
            let from = x0 + 8.5 * title.len() as f32;
            v.painter.hline(
                v.p(from.min(x1), 0.0).x..=v.p(x1, 0.0).x,
                v.p(0.0, ROW_B).y,
                Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.3)),
            );
            let cw = (x1 - x0) / knobs.len() as f32;
            for (n, &(i, label)) in knobs.iter().enumerate() {
                let tip = TIPS.iter().find(|(t, _)| *t == i).map(|(_, tip)| *tip);
                self.knob(
                    i,
                    x0 + cw * (n as f32 + 0.5),
                    ROW_B + 14.0,
                    label,
                    false,
                    self.accent,
                    tip,
                );
            }
        }
        let on = ve::flag(self.p, data::OSTINATO);
        let toggle = widgets::toggle(v.ui, v.painter, self.uid("ostinato"), v.r(440.0, ROW_B - 7.0, 510.0, ROW_B + 7.0), "OSTINATO", on, LAYER_COLORS[SHIMMER], z)
            .on_hover_text("Shimmer plays the chord's notes in time, up two octaves and back, on PULSE's note value, rather than pings at random.\nWith a low OCTAVE and a short RELEASE it's a pulse under everything; with Clock Tick, a clock");
        if toggle.clicked() {
            ve::set_flag(self.p, data::OSTINATO, !on);
        }
        let hint = "Score on the MIDI output:\nchord ch 1, bass ch 2, Shimmer ch 3";
        v.text(
            W - PAD_X,
            ROW_B + 30.0,
            Align2::RIGHT_CENTER,
            hint,
            v.font(7.0),
            theme::TEXT_FAINT,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn knob(
        &self,
        i: usize,
        cx: f32,
        y: f32,
        label: &str,
        dim: bool,
        color: Color32,
        tip: Option<&str>,
    ) {
        let (v, p, z) = (self.v, self.p, self.z());
        let spec = ve::spec(i);
        let r = 10.0;
        v.text(
            cx,
            y,
            Align2::CENTER_CENTER,
            label,
            v.font(7.5),
            theme::TEXT_FAINT,
        );
        let top = y + 8.0;
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
        let found = tip.or_else(|| TIPS.iter().find(|(t, _)| *t == i).map(|(_, text)| *text));
        if let Some(text) = found {
            resp = resp.on_hover_text(text);
        }
        v.hit(resp.rect, ParamRef::Module(i));
        if resp.dragged() || resp.double_clicked() {
            let value = spec.denorm(norm);
            let whole = matches!(i, VARIATION | COMPLEXITY | DURATION | CHORDS | ECHO | PULSE)
                || is_octave(i);
            let value = if whole { value.round() } else { value };
            if value != p.get(i) {
                p.set(i, value);
            }
        }
        let hot = resp.dragged() || resp.hovered();
        let col = if hot {
            theme::TEXT
        } else if dim {
            theme::TEXT_FAINT
        } else {
            theme::TEXT_DIM
        };
        v.text(
            cx,
            top + 2.0 * r + 8.0,
            Align2::CENTER_CENTER,
            ve::format(p, i),
            v.mono(7.0),
            col,
        );
    }
}

fn is_octave(i: usize) -> bool {
    (0..LAYERS).any(|l| i == knob(l, k::OCTAVE))
}
