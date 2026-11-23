//! Panel of Automa. On the left the generator's world: LIFE's grid (click
//! or drag to draw cells, the playhead sweeping it), the CASCADE waterfall,
//! the infinity series as a TREE (the path to the term playing lit), the
//! JULIA set with its orbit and, in a corner, the Mandelbrot set with c. On
//! the right the notes on a scrolling roll, over a lane for each tone of the
//! chord under them (the coming chords shown ahead, coloured by function:
//! tonic, subdominant, dominant). Under them the progression (drag a chord
//! up or down to change it), then the generator's knobs, RHYTHM, MELODY,
//! HARMONY and OUTPUT.

use std::collections::VecDeque;
use std::f32::consts::FRAC_PI_2;

use egui::{
    Align2, Color32, ColorImage, CursorIcon, Id, Painter, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, TextureHandle, TextureOptions, Vec2, pos2, vec2,
};

use super::nodes::{NodeView, segmented};
use super::theme::{self, alpha, mix};
use super::widgets;
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::automa::engines::{
    CASCADE_RULES, H, HISTORY, LIFE_RULES, W as COLS, escape, julia_c, series,
};
use crate::audio::modules::automa::harmony::{CHANGES, Chord, PRESETS, SCALES, SLOTS, family};
use crate::audio::modules::automa::{
    self as am, CASCADE, CASCADE_GEN, CASCADE_RULE, CHANGE, CHANNEL, CHAOS, DENSITY, DYNAMICS,
    ENGINES, GATE, INTERVAL, JULIA, KEY, LEAP, LIFE, LIFE_GEN, LIFE_RULE, MORPH, MUTATE, OCTAVE,
    PAD_VOICE, PHRASE, RATE, SCALE, SHAPE, STRIDE, SWING, TENSION, TREE, VELOCITY, VOICES, data,
    viz,
};
use crate::audio::modules::steps::steps_per_beat;
use crate::audio::params::{DIVISIONS, Unit, note_label};
use crate::graph::{ModuleNode, ParamRef};

pub const SIZE: Vec2 = vec2(760.0, 520.0);
const W: f32 = SIZE.x;
const PAD: f32 = 12.0;
/// The display band: the world (a square) and the roll.
const TOP: f32 = 58.0;
const WORLD: f32 = 272.0;
const BOTTOM: f32 = TOP + WORLD;
const ROLL_X: f32 = PAD + WORLD + 8.0;
const STRIP: [f32; 2] = [338.0, 380.0];
const ROWS_Y: [f32; 2] = [392.0, 456.0];

/// Steps the roll shows, and the part of it that is still to come.
const ROLL_STEPS: f64 = 48.0;
const AHEAD: f64 = 0.2;
/// How far JULIA's picture reaches from 0.
const EXTENT: f32 = 1.7;

/// Chord functions, for the lanes and the progression.
const TONIC: Color32 = Color32::from_rgb(96, 165, 238);
const SUBDOMINANT: Color32 = Color32::from_rgb(132, 194, 108);
const DOMINANT: Color32 = Color32::from_rgb(228, 142, 88);
/// TREE: terms below the middle, and above it.
const LOW: Color32 = Color32::from_rgb(128, 120, 250);
const HIGH: Color32 = Color32::from_rgb(250, 150, 96);

type Knobs = [(usize, &'static str)];
const ENGINE_KNOBS: [[(usize, &str); 4]; 4] = [
    [
        (LIFE_RULE, "RULE"),
        (LIFE_GEN, "GEN"),
        (MUTATE, "MUTATE"),
        (PHRASE, "PHRASE"),
    ],
    [
        (CASCADE_RULE, "RULE"),
        (CASCADE_GEN, "GEN"),
        (MUTATE, "MUTATE"),
        (PHRASE, "PHRASE"),
    ],
    [
        (STRIDE, "STRIDE"),
        (INTERVAL, "INTERVAL"),
        (MUTATE, "MUTATE"),
        (PHRASE, "PHRASE"),
    ],
    [
        (SHAPE, "SHAPE"),
        (CHAOS, "CHAOS"),
        (MORPH, "MORPH"),
        (PHRASE, "PHRASE"),
    ],
];
const RHYTHM: [(usize, &str); 4] = [
    (RATE, "RATE"),
    (DENSITY, "DENSITY"),
    (SWING, "SWING"),
    (GATE, "GATE"),
];
const MELODY: [(usize, &str); 4] = [
    (LEAP, "LEAP"),
    (TENSION, "TENSION"),
    (VOICES, "VOICES"),
    (OCTAVE, "OCTAVE"),
];
const HARMONY: [(usize, &str); 3] = [(KEY, "KEY"), (SCALE, "SCALE"), (CHANGE, "CHANGE")];
const OUTPUT: [(usize, &str); 3] = [
    (VELOCITY, "VELOCITY"),
    (DYNAMICS, "DYNAMICS"),
    (CHANNEL, "CHANNEL"),
];

const TIPS: [(usize, &str); 22] = [
    (RATE, "Step length on the global grid"),
    (DENSITY, "How many steps play (the strong ones more often)"),
    (SWING, "Delays the off-beat steps"),
    (GATE, "Note length, as a part of the step"),
    (
        LEAP,
        "How far the line may jump: low, to the nearest offered note; high, anywhere\n(TREE and JULIA: how wide a range they play in)",
    ),
    (
        TENSION,
        "Colour notes (9ths, 11ths, 13ths) on the weaker steps; at 0 only chord tones.\nThe first step of each chord always lands on it",
    ),
    (
        VOICES,
        "Parts: the melody, then harmony from the other cells that offered (as chord tones)",
    ),
    (OCTAVE, "Octave of the bottom row"),
    (KEY, "The key's tonic"),
    (CHANGE, "How long each chord lasts"),
    (VELOCITY, "Velocity of the strongest notes"),
    (
        DYNAMICS,
        "How much accents (the beat, the generator's energy) shape the velocity",
    ),
    (CHANNEL, "MIDI channel of the notes"),
    (
        LIFE_GEN,
        "Steps per generation: at 16 the world evolves once a sweep, at 1 on every step",
    ),
    (CASCADE_GEN, "Steps per generation"),
    (
        MUTATE,
        "Keeps it alive: cells born now and then, a new shape when the world dies or loops\n(TREE: jumps to another branch; JULIA: noise on the orbit)",
    ),
    (
        PHRASE,
        "The generator starts again every so many steps, on the grid (∞: never)",
    ),
    (
        STRIDE,
        "Terms per step: 2 plays the series upside down, 4 as it is, odd strides make new melodies",
    ),
    (INTERVAL, "Scale degrees per step of the series"),
    (
        SHAPE,
        "Where c sits round the main cardioid: the angle the orbit turns by, so the figure the melody traces",
    ),
    (
        CHAOS,
        "Inside the cardioid the orbit settles, on its edge it circles, past it it escapes (rests)",
    ),
    (
        MORPH,
        "c drifts round the cardioid with the beat (at 100%, once every 16 bars)",
    ),
];

fn function_color(degree: u8) -> Color32 {
    match family(degree)[0] {
        0 => TONIC,
        1 => SUBDOMINANT,
        _ => DOMINANT,
    }
}

/// A knob's value as the panel shows it.
pub fn format(p: &ModuleParams, i: usize) -> String {
    let v = p.get(i);
    let pick = |n: usize| (v.round().max(0.0) as usize).min(n - 1);
    match i {
        RATE => DIVISIONS[pick(DIVISIONS.len())].0.to_owned(),
        KEY => am::harmony(p).name(pick(12) as u8).to_owned(),
        SCALE => SCALES[pick(SCALES.len())].0.to_owned(),
        CHANGE => CHANGES[pick(CHANGES.len())].0.to_owned(),
        LIFE_RULE => LIFE_RULES[pick(LIFE_RULES.len())].0.to_owned(),
        CASCADE_RULE => format!("R{}", CASCADE_RULES[pick(CASCADE_RULES.len())].0),
        LIFE_GEN | CASCADE_GEN => format!("{} st", v.round()),
        PHRASE if v.round() < 1.0 => "∞".to_owned(),
        PHRASE => format!("{} st", v.round()),
        INTERVAL => format!("×{}", v.round()),
        CHANNEL => format!("Ch {}", v.round()),
        VOICES | OCTAVE | STRIDE | VELOCITY => format!("{}", v.round()),
        SHAPE => format!("{:.0}°", v * 360.0),
        _ => format!("{:.0}%", v * 100.0),
    }
}

/// The node's two summary lines: the generator and key, the chords.
pub fn readout(p: &ModuleParams) -> (String, String) {
    let h = am::harmony(p);
    let prog = am::progression(p);
    let chords: Vec<String> = prog.degrees[..prog.len]
        .iter()
        .map(|&d| h.chord(d).name(&h))
        .collect();
    (
        format!("{} · {}", ENGINES[p.mode().min(JULIA)].0, h.label()),
        chords.join(" "),
    )
}

/// A note on the roll.
#[derive(Clone, Copy)]
struct Mark {
    pitch: u8,
    velocity: u8,
    step: f64,
    length: f64,
    voice: u8,
    at: f64,
}

/// What the panel remembers between frames.
#[derive(Clone, Default)]
struct View {
    time: f64,
    /// The transport's beat as last published, and when that was seen.
    beat: (f32, f64),
    step: f32,
    /// Each cell's presence (fading in and out), and when it last sounded.
    glow: Vec<f32>,
    flash: Vec<f64>,
    marks: VecDeque<Mark>,
    seq: f32,
    /// The generation counter, and when it last moved (CASCADE scrolls).
    generation: (f32, f64),
    /// TREE's latest terms.
    leaves: VecDeque<(u64, f64)>,
    julia: Option<(TextureHandle, (f32, f32), Color32)>,
    mandelbrot: Option<(TextureHandle, Color32)>,
}

impl View {
    /// Takes in what the processor published since the last frame.
    fn update(&mut self, p: &ModuleParams, now: f64, dt: f32) {
        let cells = COLS * H;
        if self.glow.len() != cells {
            self.glow = vec![0.0; cells];
            self.flash = vec![f64::NEG_INFINITY; cells];
        }
        let beat = p.viz[viz::BEAT].get();
        if beat != self.beat.0 {
            if beat + 1.0 < self.beat.0 {
                // Rewound: the old notes are gone.
                self.marks.clear();
                self.leaves.clear();
            }
            self.beat = (beat, now);
        }

        let seq = p.viz[viz::EVENT_SEQ].get();
        if seq < self.seq {
            // A new processor (the node was rebuilt).
            self.seq = 0.0;
            self.marks.clear();
        }
        let (from, to) = (self.seq as u32 + 1, seq as u32);
        for n in from.max(to.saturating_sub(viz::EVENT_CAP as u32 - 1))..=to {
            let at = viz::EVENTS + (n as usize - 1) % viz::EVENT_CAP * viz::EVENT_LEN;
            let field = |k: usize| p.viz[at + k].get();
            if field(0) as u32 != n {
                continue;
            }
            let mark = Mark {
                pitch: field(1) as u8,
                velocity: field(2) as u8,
                step: field(3) as f64,
                length: field(4) as f64,
                voice: field(5) as u8,
                at: now,
            };
            if field(0) as u32 == n {
                self.marks.push_back(mark);
            }
        }
        self.seq = seq;
        let oldest = self.beat.0 as f64 * steps_per_beat(p.get(RATE)) - ROLL_STEPS * 2.0;
        while self
            .marks
            .front()
            .is_some_and(|m| m.step + m.length < oldest)
            || self.marks.len() > 400
        {
            self.marks.pop_front();
        }

        let step = p.viz[viz::STEP].get();
        if step >= 0.0 && step != self.step {
            let played = p.viz[viz::PLAYED].get() as u32;
            for bit in (0..16).filter(|b| played >> b & 1 == 1) {
                match p.mode() {
                    LIFE => self.flash[bit * COLS + (step as usize % COLS)] = now,
                    CASCADE => self.flash[bit] = now,
                    _ => {}
                }
            }
        }
        self.step = step;
        let generation = p.viz[viz::GENERATION].get();
        if generation != self.generation.0 {
            self.generation = (generation, now);
            if p.mode() == TREE && step >= 0.0 {
                self.leaves.push_back((generation as u64, now));
                if self.leaves.len() > 12 {
                    self.leaves.pop_front();
                }
            }
        }

        let k = 1.0 - (-dt / 0.09).exp();
        for (i, g) in self.glow.iter_mut().enumerate() {
            let on = p.viz[viz::CELLS + i].get() > 0.0;
            *g += (if on { 1.0 } else { 0.0 } - *g) * k;
        }
    }

    /// The beat now, carried on from the last one published at the tempo.
    fn beat_now(&self, playing: bool, tempo: f32, now: f64) -> f64 {
        let (beat, seen) = self.beat;
        if playing {
            beat as f64 + ((now - seen) * tempo as f64 / 60.0).clamp(0.0, 0.5)
        } else {
            beat as f64
        }
    }

    fn settled(&self, p: &ModuleParams, now: f64) -> bool {
        let on = |i: usize| {
            if p.viz[viz::CELLS + i].get() > 0.0 {
                1.0
            } else {
                0.0
            }
        };
        self.glow
            .iter()
            .enumerate()
            .all(|(i, g)| (g - on(i)).abs() < 0.01)
            && self.flash.iter().all(|&t| now - t > 0.5)
    }
}

struct Panel<'a, 'b> {
    v: &'a NodeView<'b>,
    id: NodeId,
    p: &'a ModuleParams,
    accent: Color32,
    now: f64,
    beat: f64,
    playing: bool,
}

/// `out`: the MIDI output is patched.
pub fn draw(v: &NodeView, id: NodeId, accent: Color32, m: &ModuleNode, out: bool) {
    let p = &m.params;
    let key = Id::unique(("automa-view", id));
    let mut view: View = v.ui.data_mut(|d| d.remove_temp(key)).unwrap_or_default();
    let now = v.ui.input(|i| i.time);
    let dt = (now - view.time).clamp(0.0, 0.1) as f32;
    view.time = now;
    view.update(p, now, dt);
    let playing = p.viz[viz::STEP].get() >= 0.0;
    let s = Panel {
        v,
        id,
        p,
        accent,
        now,
        beat: view.beat_now(playing, v.tempo, now),
        playing,
    };

    v.text(
        W - PAD,
        42.0,
        Align2::RIGHT_CENTER,
        "MIDI",
        v.font(9.0),
        if out {
            theme::text()_DIM
        } else {
            theme::text()_FAINT
        },
    );
    s.header();
    s.world(&mut view);
    s.roll(&view);
    s.strip();
    s.knobs();
    if playing || !view.settled(p, now) {
        v.ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
    }
    v.ui.data_mut(|d| d.insert_temp(key, view));
}

impl Panel<'_, '_> {
    fn z(&self) -> f32 {
        self.v.z
    }

    fn uid(&self, salt: impl std::hash::Hash + std::fmt::Debug) -> Id {
        Id::unique(("automa", self.id)).with(salt)
    }

    fn engine(&self) -> usize {
        self.p.mode().min(JULIA)
    }

    fn on(&self, i: usize) -> bool {
        self.p.data[i].get() > 0.5
    }

    fn flip(&self, i: usize) {
        self.p.data[i].set(if self.on(i) { 0.0 } else { 1.0 });
    }

    fn button(
        &self,
        salt: &str,
        x0: f32,
        x1: f32,
        label: &str,
        lit: bool,
        tip: &str,
    ) -> egui::Response {
        let (v, z) = (self.v, self.z());
        widgets::toggle(
            v.ui,
            v.painter,
            self.uid(salt),
            v.r(x0, 34.0, x1, 50.0),
            label,
            lit,
            self.accent,
            z,
        )
        .on_hover_text(tip)
    }

    /// The chord under `beat` (and the degree it stands on).
    fn chord_at(&self, beat: f64) -> (u8, Chord) {
        let prog = am::progression(self.p);
        let degree = prog.degree_at(beat.max(0.0));
        (degree, am::harmony(self.p).chord(degree))
    }

    /// The generator, RESEED, FREEZE, PLAY (KEEP and CLEAR for LIFE) and
    /// what the generator is doing.
    fn header(&self) {
        let (v, p) = (self.v, self.p);
        let engine = self.engine();
        let picked = segmented(
            v,
            self.uid("engine"),
            v.r(PAD, 34.0, PAD + 246.0, 50.0),
            ENGINES.len(),
            engine,
            self.accent,
            |k, r, col| {
                v.painter.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    ENGINES[k].0,
                    v.font(8.0),
                    col,
                );
                ENGINES[k].1
            },
        );
        if let Some(k) = picked {
            p.set_mode(k);
        }
        let reseed = self.button(
            "reseed",
            266.0,
            326.0,
            "RESEED",
            false,
            "A new start: a new seed for every generator and a random pattern for LIFE",
        );
        if reseed.clicked() {
            am::reseed(
                p,
                (self.now * 1000.0) as u32 ^ self.id.wrapping_mul(0x9E37_79B9),
            );
        }
        let frozen = self.on(data::FREEZE);
        let freeze = self.button("freeze", 332.0, 386.0, "FREEZE", frozen, "Loop what is playing: LIFE stops evolving, the others repeat their last bar (or PHRASE)");
        if freeze.clicked() {
            self.flip(data::FREEZE);
        }
        let play = self.button(
            "play",
            392.0,
            434.0,
            "PLAY",
            p.toggled(),
            "Play or pause the generator (the transport runs on)",
        );
        if play.clicked() {
            p.set_toggle(!p.toggled());
        }
        let mut x = 442.0;
        if engine == LIFE {
            let keep = self.button(
                "keep",
                x,
                x + 40.0,
                "KEEP",
                false,
                "Make the grid as it is now LIFE's start (Start plays from it)",
            );
            if keep.clicked() {
                let rows: [u16; H] = std::array::from_fn(|y| {
                    (0..COLS)
                        .filter(|&x| p.viz[viz::CELLS + y * COLS + x].get() > 0.0)
                        .fold(0, |r, x| r | 1 << x)
                });
                am::set_pattern(p, &rows);
            }
            let clear = self.button(
                "clear",
                x + 46.0,
                x + 90.0,
                "CLEAR",
                false,
                "Kill every cell (MUTATE sows new ones)",
            );
            if clear.clicked() {
                for i in 0..COLS * H {
                    self.edit(i % COLS, i / COLS, false);
                }
            }
            x += 98.0;
        }

        let state = if !p.toggled() {
            "paused"
        } else if !self.playing {
            "waiting for Start"
        } else if frozen {
            "frozen"
        } else {
            ""
        };
        let generation = p.viz[viz::GENERATION].get();
        let detail = match engine {
            LIFE => {
                let population = (0..COLS * H)
                    .filter(|&i| p.viz[viz::CELLS + i].get() > 0.0)
                    .count();
                format!("gen {generation} · {population} cells")
            }
            CASCADE => format!(
                "rule {} · gen {generation}",
                CASCADE_RULES[am::int(p, CASCADE_RULE, 0, 7) as usize].0
            ),
            TREE => {
                let n = p.viz[viz::TERM].get() as u64;
                format!("n {n} · a(n) {:+}", series(n))
            }
            _ => format!("phrase {}", generation + 1.0),
        };
        let text = if state.is_empty() {
            detail
        } else {
            format!("{state} · {detail}")
        };
        v.text(
            x,
            42.0,
            Align2::LEFT_CENTER,
            text,
            v.font(7.5),
            theme::text()_FAINT,
        );
    }

    /// A cell drawn (or rubbed out) on LIFE's grid. Stopped, the drawing is
    /// LIFE's start too.
    fn edit(&self, x: usize, y: usize, on: bool) {
        let p = self.p;
        let i = y * COLS + x;
        p.viz[viz::EDITS + i].set(if on { 1.0 } else { -1.0 });
        p.viz[viz::EDITED].set(1.0);
        p.viz[viz::CELLS + i].set(if on { 1.0 } else { 0.0 });
        if !self.playing {
            let mut rows = am::pattern(p);
            if on {
                rows[y] |= 1 << x;
            } else {
                rows[y] &= !(1 << x);
            }
            am::set_pattern(p, &rows);
        }
    }

    fn world(&self, view: &mut View) {
        let v = self.v;
        let rect = v.r(PAD, TOP, PAD + WORLD, BOTTOM);
        v.painter.rect_filled(rect, 4.0 * self.z(), theme::inset());
        let painter = v
            .painter
            .with_clip_rect(rect.intersect(v.painter.clip_rect()));
        match self.engine() {
            LIFE => self.life(&painter, rect, view),
            CASCADE => self.cascade(&painter, rect, view),
            TREE => self.tree(&painter, rect, view),
            _ => self.julia(&painter, rect, view),
        }
    }

    /// The colour of a living cell by its age: white-hot when newborn, the
    /// node's colour, then cooler as it settles.
    fn cell_color(&self, age: f32) -> Color32 {
        if age <= 1.0 {
            mix(self.accent, Color32::WHITE, 0.55)
        } else if age <= 3.0 {
            self.accent
        } else {
            mix(
                self.accent,
                theme::text()_FAINT,
                ((age - 3.0) / 14.0).min(0.75),
            )
        }
    }

    /// A cell's ring as it sounds.
    fn ripple(&self, painter: &Painter, c: Pos2, size: f32, since: f64) {
        let t = (since / 0.45) as f32;
        if (0.0..1.0).contains(&t) {
            let r = size * (0.5 + 0.9 * t);
            painter.rect_stroke(
                Rect::from_center_size(c, vec2(r, r) * 2.0),
                3.0 * self.z(),
                Stroke::new(1.5 * self.z(), alpha(Color32::WHITE, (1.0 - t) * 0.8)),
                StrokeKind::Middle,
            );
            painter.circle_filled(c, size * 0.9, alpha(self.accent, (1.0 - t) * 0.25));
        }
    }

    fn life(&self, painter: &Painter, rect: Rect, view: &View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let cs = rect.width() / COLS as f32;
        let h = am::harmony(p);
        let (degree, chord) = self.chord_at(self.beat);
        let cell = |x: usize, y: usize| {
            Rect::from_min_size(
                pos2(
                    rect.left() + x as f32 * cs,
                    rect.bottom() - (y + 1) as f32 * cs,
                ),
                vec2(cs, cs),
            )
        };
        // Lanes for the chord's tones, the root's brighter.
        for y in 0..H {
            let pc = h.pc(y as i32);
            if chord.contains(pc) {
                let row = Rect::from_x_y_ranges(rect.x_range(), cell(0, y).y_range());
                let a = if pc == chord.pcs[0] { 0.12 } else { 0.06 };
                painter.rect_filled(row, 0.0, alpha(function_color(degree), a));
            }
        }
        // The playhead, gliding.
        if self.playing {
            let at = (self.beat * steps_per_beat(p.get(RATE))).rem_euclid(COLS as f64) as f32;
            let x = rect.left() + at * cs;
            painter.rect_filled(
                Rect::from_min_max(pos2(x, rect.top()), pos2(x + cs, rect.bottom())),
                2.0 * z,
                alpha(self.accent, 0.1),
            );
            painter.vline(
                x + cs,
                rect.y_range(),
                Stroke::new(1.0 * z, alpha(self.accent, 0.4)),
            );
        }
        for y in 0..H {
            for x in 0..COLS {
                let i = y * COLS + x;
                let r = cell(x, y);
                let g = view.glow[i];
                if g < 0.03 {
                    painter.circle_filled(r.center(), 0.8 * z, alpha(theme::text()_FAINT, 0.18));
                    continue;
                }
                let age = p.viz[viz::CELLS + i].get();
                let color = self.cell_color(if age > 0.0 { age } else { 8.0 });
                let size = cs * (0.3 + 0.55 * g);
                if age <= 1.5 && age > 0.0 {
                    painter.rect_filled(
                        Rect::from_center_size(r.center(), vec2(size, size) * 1.5),
                        4.0 * z,
                        alpha(color, 0.12 * g),
                    );
                }
                painter.rect_filled(
                    Rect::from_center_size(r.center(), vec2(size, size)),
                    2.5 * z,
                    alpha(color, g),
                );
                self.ripple(painter, r.center(), size, self.now - view.flash[i]);
            }
        }
        // Drawing cells.
        let resp = v
            .ui
            .interact(rect, self.uid("grid"), Sense::click_and_drag())
            .on_hover_cursor(CursorIcon::Crosshair)
            .on_hover_text("Click a cell to bring it to life (or kill it), drag to paint · KEEP makes the grid the start");
        if let Some(q) = resp.interact_pointer_pos() {
            let x = (((q.x - rect.left()) / cs) as usize).min(COLS - 1);
            let y = (((rect.bottom() - q.y) / cs) as usize).min(H - 1);
            let alive = p.viz[viz::CELLS + y * COLS + x].get() > 0.0;
            let paint = self.uid("paint");
            if resp.clicked() {
                self.edit(x, y, !alive);
            } else if resp.drag_started() {
                v.ui.data_mut(|d| d.insert_temp(paint, !alive));
                self.edit(x, y, !alive);
            } else if resp.dragged()
                && let Some(state) = v.ui.data(|d| d.get_temp::<bool>(paint))
                && alive != state
            {
                self.edit(x, y, state);
            }
        }
    }

    fn cascade(&self, painter: &Painter, rect: Rect, view: &View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let cs = rect.width() / COLS as f32;
        let h = am::harmony(p);
        let (degree, chord) = self.chord_at(self.beat);
        for x in 0..COLS {
            let pc = h.pc(x as i32);
            if chord.contains(pc) {
                let col = Rect::from_min_max(
                    pos2(rect.left() + x as f32 * cs, rect.top()),
                    pos2(rect.left() + (x + 1) as f32 * cs, rect.bottom()),
                );
                painter.rect_filled(
                    col,
                    0.0,
                    alpha(
                        function_color(degree),
                        if pc == chord.pcs[0] { 0.1 } else { 0.05 },
                    ),
                );
            }
        }
        // The generations slide down one row as a new one arrives.
        let step_seconds = 60.0 / (v.tempo.max(1.0) as f64 * steps_per_beat(p.get(RATE)));
        let scroll = if self.playing {
            (1.0 - (self.now - view.generation.1) / (step_seconds * 0.6).min(0.15)).clamp(0.0, 1.0)
                as f32
        } else {
            0.0
        };
        for age in 0..HISTORY {
            let fade = (1.0 - age as f32 / HISTORY as f32).powf(1.3);
            let top = rect.top() + (age as f32 - scroll) * cs;
            for x in 0..COLS {
                if p.viz[viz::CELLS + age * COLS + x].get() < 0.5 {
                    continue;
                }
                let r = Rect::from_min_size(pos2(rect.left() + x as f32 * cs, top), vec2(cs, cs))
                    .shrink(1.5 * z);
                let color = if age == 0 {
                    mix(self.accent, Color32::WHITE, 0.5)
                } else {
                    self.accent
                };
                if age == 0 {
                    painter.rect_filled(r.expand(2.5 * z), 4.0 * z, alpha(color, 0.15));
                }
                painter.rect_filled(r, 2.0 * z, alpha(color, 0.15 + 0.85 * fade));
                if age == 0 {
                    self.ripple(painter, r.center(), r.width(), self.now - view.flash[x]);
                }
            }
        }
        let line = rect.top() + (1.0 - scroll) * cs;
        painter.hline(
            rect.x_range(),
            line,
            Stroke::new(1.0 * z, alpha(self.accent, 0.3)),
        );
        let (rule, what) = CASCADE_RULES[am::int(p, CASCADE_RULE, 0, 7) as usize];
        v.text(
            PAD + 6.0,
            BOTTOM - 8.0,
            Align2::LEFT_CENTER,
            format!("rule {rule} · {what}"),
            v.font(6.5),
            alpha(theme::text()_FAINT, 0.8),
        );
    }

    /// The infinity series as a binary tree, six levels down from the
    /// block of 64 terms playing: a branch to the left turns the melody
    /// over, to the right steps it up. Each branch is coloured by its term,
    /// the path to the term playing is lit.
    fn tree(&self, painter: &Painter, rect: Rect, view: &View) {
        let (v, z) = (self.v, self.z());
        let n = self.p.viz[viz::TERM].get() as u64;
        let origin = pos2(rect.center().x, rect.bottom() - 12.0 * z);
        let leaves = self.branch(painter, n, 0, 0, origin, -FRAC_PI_2, 60.0 * z);
        for &(term, at) in &view.leaves {
            if term >> 6 != n >> 6 {
                continue;
            }
            let c = leaves[(term & 63) as usize];
            let t = ((self.now - at) / 1.6) as f32;
            if t < 1.0 {
                painter.circle_filled(
                    c,
                    (2.5 + 3.5 * (1.0 - t)) * z,
                    alpha(Color32::WHITE, 0.6 * (1.0 - t)),
                );
            }
        }
        if self.playing {
            let c = leaves[(n & 63) as usize];
            let since = view
                .leaves
                .back()
                .map_or(1.0, |&(_, at)| (self.now - at) as f32);
            let pulse = (1.0 - since / 0.4).clamp(0.0, 1.0);
            painter.circle_filled(
                c,
                (4.0 + 4.0 * pulse) * z,
                alpha(self.accent, 0.35 + 0.4 * pulse),
            );
            painter.circle_stroke(
                c,
                (7.0 + 10.0 * (1.0 - pulse)) * z,
                Stroke::new(1.2 * z, alpha(Color32::WHITE, pulse)),
            );
        }
        // The term's address: its last six bits, the turns taken.
        let bits: String = (0..6)
            .rev()
            .map(|k| if n >> k & 1 == 1 { '1' } else { '0' })
            .collect();
        v.text(
            PAD + 8.0,
            TOP + 12.0,
            Align2::LEFT_CENTER,
            format!("n = {n}"),
            v.mono(8.0),
            theme::text()_DIM,
        );
        v.text(
            PAD + 8.0,
            TOP + 24.0,
            Align2::LEFT_CENTER,
            format!("…{bits}"),
            v.mono(8.0),
            alpha(self.accent, 0.9),
        );
        v.text(
            PAD + WORLD - 8.0,
            TOP + 12.0,
            Align2::RIGHT_CENTER,
            format!("a(n) = {:+}", series(n)),
            v.mono(8.0),
            theme::text()_DIM,
        );
        v.text(
            PAD + 6.0,
            BOTTOM - 8.0,
            Align2::LEFT_CENTER,
            "0 ← turn over · step up → 1",
            v.font(6.5),
            alpha(theme::text()_FAINT, 0.8),
        );
    }

    /// One branch and those above it; returns where the 64 leaves are.
    #[allow(clippy::too_many_arguments)]
    fn branch(
        &self,
        painter: &Painter,
        n: u64,
        depth: u32,
        prefix: u64,
        from: Pos2,
        angle: f32,
        length: f32,
    ) -> [Pos2; 64] {
        let z = self.z();
        let to = from + vec2(angle.cos(), angle.sin()) * length;
        let term = series((n >> 6) << depth | prefix);
        let t = (term as f32 / 5.0).clamp(-1.0, 1.0);
        let base = if t < 0.0 {
            mix(theme::text()_DIM, LOW, -t)
        } else {
            mix(theme::text()_DIM, HIGH, t)
        };
        let lit = (n & 63) >> (6 - depth) == prefix;
        let width = (3.2 - 0.42 * depth as f32) * z;
        let (color, width) = if lit && self.playing {
            (mix(base, Color32::WHITE, 0.6), width + 1.2 * z)
        } else {
            (alpha(base, 0.75), width)
        };
        if lit && self.playing {
            painter.line_segment(
                [from, to],
                Stroke::new(width + 4.0 * z, alpha(self.accent, 0.18)),
            );
        }
        painter.line_segment([from, to], Stroke::new(width, color));
        let mut leaves = [to; 64];
        if depth == 6 {
            painter.circle_filled(to, 1.8 * z, color);
            return leaves;
        }
        let spread = 0.5 - 0.035 * depth as f32;
        let shrink = if depth == 0 { 0.72 } else { 0.7 };
        let left = self.branch(
            painter,
            n,
            depth + 1,
            prefix << 1,
            to,
            angle - spread,
            length * shrink,
        );
        let right = self.branch(
            painter,
            n,
            depth + 1,
            prefix << 1 | 1,
            to,
            angle + spread,
            length * shrink,
        );
        // The leaves under this branch: left's first, then right's.
        let below = 1usize << (5 - depth);
        let first = ((prefix as usize) << (6 - depth)) & 63;
        for k in 0..below {
            leaves[first + k] = left[first + k];
            leaves[first + below + k] = right[first + below + k];
        }
        leaves
    }

    /// Maps the plane onto the world square.
    fn plane(rect: Rect, (x, y): (f32, f32)) -> Pos2 {
        let s = rect.width() / (2.0 * EXTENT);
        pos2(rect.center().x + x * s, rect.center().y - y * s)
    }

    fn julia(&self, painter: &Painter, rect: Rect, view: &mut View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let c = (p.viz[viz::C].get(), p.viz[viz::C + 1].get());
        let stale = view.julia.as_ref().is_none_or(|(_, at, color)| {
            (at.0 - c.0).abs() + (at.1 - c.1).abs() > 2e-4 || *color != self.accent
        });
        if stale {
            let image = julia_image(c, self.accent);
            match &mut view.julia {
                Some((texture, at, color)) => {
                    texture.set(image, TextureOptions::LINEAR);
                    (*at, *color) = (c, self.accent);
                }
                None => {
                    view.julia = Some((
                        v.ui.ctx()
                            .load_texture("automa-julia", image, TextureOptions::LINEAR),
                        c,
                        self.accent,
                    ))
                }
            }
        }
        if let Some((texture, _, _)) = &view.julia {
            painter.image(
                texture.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }

        // The fixed point the orbit turns round.
        let fix = Self::plane(rect, (p.viz[viz::FIX].get(), p.viz[viz::FIX + 1].get()));
        let arm = 4.0 * z;
        let cross = Stroke::new(1.0 * z, alpha(theme::text(), 0.6));
        painter.line_segment([fix - vec2(arm, 0.0), fix + vec2(arm, 0.0)], cross);
        painter.line_segment([fix - vec2(0.0, arm), fix + vec2(0.0, arm)], cross);

        // The orbit, oldest to newest.
        let pos = p.viz[viz::ORBIT_POS].get() as usize;
        let points: Vec<Pos2> = (0..viz::ORBIT_LEN)
            .map(|k| {
                let i = (pos + k) % viz::ORBIT_LEN;
                Self::plane(
                    rect,
                    (
                        p.viz[viz::ORBIT + 2 * i].get(),
                        p.viz[viz::ORBIT + 2 * i + 1].get(),
                    ),
                )
            })
            .collect();
        for (k, pair) in points.windows(2).enumerate() {
            let a = (k + 1) as f32 / viz::ORBIT_LEN as f32;
            painter.line_segment(
                [pair[0], pair[1]],
                Stroke::new(
                    (0.6 + 1.4 * a) * z,
                    alpha(mix(self.accent, Color32::WHITE, 0.5), a * 0.9),
                ),
            );
            painter.circle_filled(pair[1], (1.0 + 1.5 * a) * z, alpha(Color32::WHITE, a));
        }
        if let Some(&last) = points.last() {
            let since = (self.now
                - view
                    .generation
                    .1
                    .max(view.marks.back().map_or(f64::NEG_INFINITY, |m| m.at)))
                as f32;
            let pulse = if self.playing {
                (1.0 - since / 0.35).clamp(0.0, 1.0)
            } else {
                0.0
            };
            painter.circle_filled(last, (3.5 + 3.0 * pulse) * z, alpha(Color32::WHITE, 0.9));
            painter.circle_stroke(
                last,
                (6.0 + 8.0 * (1.0 - pulse)) * z,
                Stroke::new(1.2 * z, alpha(self.accent, 0.3 + 0.7 * pulse)),
            );
        }

        self.mandelbrot(painter, rect, view, c);
        v.text(
            PAD + 6.0,
            BOTTOM - 8.0,
            Align2::LEFT_CENTER,
            format!("c = {:.3} {:+.3}i", c.0, c.1),
            v.mono(7.0),
            alpha(theme::text()_DIM, 0.9),
        );
    }

    /// The Mandelbrot set in a corner, the path c takes round the cardioid
    /// at this CHAOS, and c on it.
    fn mandelbrot(&self, painter: &Painter, rect: Rect, view: &mut View, c: (f32, f32)) {
        let z = self.z();
        const RE: (f32, f32) = (-2.1, 0.7);
        const IM: f32 = 1.2;
        if view
            .mandelbrot
            .as_ref()
            .is_none_or(|(_, color)| *color != self.accent)
        {
            let image = mandelbrot_image(RE, IM, self.accent);
            view.mandelbrot = Some((
                self.v
                    .ui
                    .ctx()
                    .load_texture("automa-mandelbrot", image, TextureOptions::LINEAR),
                self.accent,
            ));
        }
        let inset = Rect::from_min_size(
            rect.right_top() + vec2(-82.0 * z, 6.0 * z),
            vec2(76.0, 65.0) * z,
        );
        painter.rect_filled(inset.expand(1.0 * z), 3.0 * z, theme::inset());
        if let Some((texture, _)) = &view.mandelbrot {
            painter.image(
                texture.id(),
                inset,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        let at = |(x, y): (f32, f32)| {
            pos2(
                inset.left() + (x - RE.0) / (RE.1 - RE.0) * inset.width(),
                inset.center().y - y / IM * inset.height() * 0.5,
            )
        };
        let chaos = self.p.get(CHAOS);
        let path: Vec<Pos2> = (0..=48)
            .map(|k| at(julia_c(k as f32 / 48.0, chaos, 0.0, 0.0)))
            .collect();
        painter.add(Shape::line(
            path,
            Stroke::new(0.8 * z, alpha(theme::text(), 0.35)),
        ));
        painter.circle_filled(at(c), 2.5 * z, Color32::WHITE);
        painter.circle_stroke(at(c), 4.5 * z, Stroke::new(1.0 * z, self.accent));
        painter.rect_stroke(
            inset,
            3.0 * z,
            Stroke::new(1.0 * z, alpha(theme::text()_FAINT, 0.5)),
            StrokeKind::Outside,
        );
    }

    /// The notes as they play, over the lanes of each chord's tones; the
    /// chords to come on the right, behind a veil.
    fn roll(&self, view: &View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let rect = v.r(ROLL_X, TOP, W - PAD, BOTTOM);
        v.painter.rect_filled(rect, 4.0 * z, theme::inset());
        let painter = v
            .painter
            .with_clip_rect(rect.intersect(v.painter.clip_rect()));
        let per_beat = steps_per_beat(p.get(RATE));
        let now = self.beat * per_beat;
        let px = rect.width() as f64 / ROLL_STEPS;
        let now_x = rect.left() + rect.width() * (1.0 - AHEAD) as f32;
        let x_of = |step: f64| now_x + ((step - now) * px) as f32;
        let h = am::harmony(p);
        let base = am::base(p);
        let (low, high) = (base - 12, base + h.semis(H as i32 - 1));
        let rh = rect.height() / (high - low + 1) as f32;
        let y_of = |pitch: i32| rect.bottom() - ((pitch - low) as f32 + 0.5) * rh;

        // The chords: a lane per tone, the root's brighter, and the name.
        let prog = am::progression(p);
        let first = (now - ROLL_STEPS * (1.0 - AHEAD)) / per_beat;
        let last = (now + ROLL_STEPS * AHEAD) / per_beat;
        let mut beat = (first / prog.beats).floor() * prog.beats;
        while beat < last {
            let degree = prog.degree_at(beat.max(0.0));
            let chord = h.chord(degree);
            let color = function_color(degree);
            let (x0, x1) = (x_of(beat * per_beat), x_of((beat + prog.beats) * per_beat));
            if beat >= 0.0 {
                for pitch in low..=high {
                    let pc = pitch.rem_euclid(12) as u8;
                    if chord.contains(pc) {
                        let a = if pc == chord.pcs[0] { 0.2 } else { 0.09 };
                        let y = y_of(pitch);
                        painter.rect_filled(
                            Rect::from_min_max(pos2(x0, y - rh * 0.45), pos2(x1, y + rh * 0.45)),
                            0.0,
                            alpha(color, a),
                        );
                    }
                }
                painter.vline(x0, rect.y_range(), Stroke::new(1.0 * z, alpha(color, 0.35)));
                if theme::font(8.0 * z).size >= 4.0 {
                    painter.text(
                        pos2(x0 + 4.0 * z, rect.top() + 8.0 * z),
                        Align2::LEFT_CENTER,
                        chord.name(&h),
                        v.mono(8.5),
                        alpha(color, 0.95),
                    );
                    painter.text(
                        pos2(x0 + 4.0 * z, rect.top() + 19.0 * z),
                        Align2::LEFT_CENTER,
                        chord.numeral(),
                        v.font(7.0),
                        alpha(color, 0.6),
                    );
                }
            }
            beat += prog.beats;
        }
        // Bars.
        let mut bar = (first / 4.0).floor() * 4.0;
        while bar < last {
            if bar >= 0.0 && (bar / prog.beats).fract().abs() > 1e-6 {
                painter.vline(
                    x_of(bar * per_beat),
                    rect.y_range(),
                    Stroke::new(1.0 * z, alpha(theme::text()_FAINT, 0.12)),
                );
            }
            bar += 4.0;
        }
        // The tonic of each octave.
        for pitch in (low..=high).filter(|&n| n.rem_euclid(12) == h.key as i32) {
            painter.text(
                pos2(rect.left() + 4.0 * z, y_of(pitch)),
                Align2::LEFT_CENTER,
                note_label(pitch),
                v.font(6.5),
                alpha(theme::text()_FAINT, 0.9),
            );
        }

        // The melody's line: its notes joined, broken by rests over a beat.
        let mut line: Vec<Pos2> = Vec::new();
        let mut previous: Option<f64> = None;
        let contour = Stroke::new(1.2 * z, alpha(mix(self.accent, Color32::WHITE, 0.4), 0.45));
        for m in view.marks.iter().filter(|m| m.voice == 0) {
            if previous.is_some_and(|s| m.step - s > per_beat) {
                if line.len() > 1 {
                    painter.add(Shape::line(std::mem::take(&mut line), contour));
                }
                line.clear();
            }
            line.push(pos2(x_of(m.step + m.length * 0.5), y_of(m.pitch as i32)));
            previous = Some(m.step);
        }
        if line.len() > 1 {
            painter.add(Shape::line(line, contour));
        }

        for m in &view.marks {
            let (x0, x1) = (x_of(m.step), x_of(m.step + m.length));
            if x1 < rect.left() || x0 > rect.right() {
                continue;
            }
            let y = y_of(m.pitch as i32);
            let fresh = (1.0 - (self.now - m.at) / 0.35).clamp(0.0, 1.0) as f32;
            let level = 0.45 + 0.55 * m.velocity as f32 / 127.0;
            let r = Rect::from_min_max(
                pos2(x0, y - rh * 0.42),
                pos2(x1.max(x0 + 2.0 * z), y + rh * 0.42),
            );
            if m.voice as usize == PAD_VOICE {
                // The pad: the chord held, framed in its function's colour.
                let color = function_color(self.chord_at(m.step / per_beat).0);
                let r = r.shrink2(vec2(1.0 * z, 0.0));
                painter.rect_filled(r, 2.0 * z, alpha(color, 0.38 * level));
                painter.rect_stroke(
                    r,
                    2.0 * z,
                    Stroke::new(1.0 * z, alpha(color, 0.85)),
                    StrokeKind::Inside,
                );
                continue;
            }
            let color = if m.voice == 0 {
                mix(self.accent, Color32::WHITE, 0.35)
            } else {
                mix(self.accent, theme::text()_DIM, 0.35)
            };
            if fresh > 0.0 {
                painter.rect_filled(
                    r.expand(3.0 * z * fresh),
                    3.0 * z,
                    alpha(color, 0.3 * fresh),
                );
            }
            painter.rect_filled(
                r,
                1.5 * z,
                alpha(mix(color, Color32::WHITE, fresh * 0.6), level),
            );
        }
        // What is to come, behind a veil; now.
        painter.rect_filled(
            Rect::from_min_max(pos2(now_x, rect.top()), rect.right_bottom()),
            0.0,
            alpha(Color32::BLACK, 0.28),
        );
        painter.vline(
            now_x,
            rect.y_range(),
            Stroke::new(
                1.0 * z,
                alpha(theme::text(), if self.playing { 0.45 } else { 0.2 }),
            ),
        );
    }

    /// The progression: drag a chord up or down to change it, click one
    /// past the end to lengthen it; CHORDS ▾ holds the presets.
    fn strip(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let h = am::harmony(p);
        let prog = am::progression(p);
        let [y0, y1] = STRIP;
        let (x0, x1) = (PAD, W - PAD - 92.0);
        let sw = (x1 - x0) / SLOTS as f32;
        let (playing_slot, cycle) = prog.slot_at(self.beat.max(0.0));
        let phase = (self.beat.max(0.0).rem_euclid(prog.beats) / prog.beats) as f32;
        for i in 0..SLOTS {
            let rect = v.r(
                x0 + sw * i as f32 + 2.0,
                y0,
                x0 + sw * (i + 1) as f32 - 2.0,
                y1,
            );
            let active = i < prog.len;
            let written = prog.degrees[i];
            let degree = if active {
                prog.degree(i, if self.playing { cycle } else { 0 })
            } else {
                written
            };
            let chord = h.chord(degree);
            let color = function_color(degree);
            let current = active && i == playing_slot;
            v.painter.rect_filled(
                rect,
                3.0 * z,
                if current {
                    theme::control()
                } else {
                    theme::inset()
                },
            );
            let ink = if active { 1.0 } else { 0.3 };
            v.painter.rect_filled(
                Rect::from_min_size(rect.min, vec2(rect.width(), 3.0 * z)),
                1.5 * z,
                alpha(color, 0.9 * ink),
            );
            let cx = rect.center().x;
            let numeral_color = if current {
                theme::text()
            } else {
                alpha(theme::text()_DIM, ink)
            };
            v.painter.text(
                pos2(cx, rect.top() + 15.0 * z),
                Align2::CENTER_CENTER,
                chord.numeral(),
                theme::strong(11.0 * z),
                numeral_color,
            );
            v.painter.text(
                pos2(cx, rect.top() + 29.0 * z),
                Align2::CENTER_CENTER,
                chord.name(&h),
                v.mono(8.0),
                alpha(color, 0.85 * ink),
            );
            if degree != written {
                // WANDER put another chord of the same function here.
                v.painter.text(
                    rect.right_top() + vec2(-4.0, 9.0) * z,
                    Align2::RIGHT_CENTER,
                    "~",
                    v.mono(8.0),
                    alpha(color, 0.9),
                );
            }
            if current && self.playing {
                let bar = Rect::from_min_size(
                    pos2(rect.left(), rect.bottom() - 2.0 * z),
                    vec2(rect.width() * phase, 2.0 * z),
                );
                v.painter.rect_filled(bar, 1.0 * z, self.accent);
            }
            let resp =
                v.ui.interact(rect, self.uid(("slot", i)), Sense::click_and_drag())
                    .on_hover_cursor(CursorIcon::ResizeVertical)
                    .on_hover_text(if active {
                        "Drag up or down to change the chord"
                    } else {
                        "Click to make the progression this long"
                    });
            if resp.clicked() && !active {
                p.data[data::CHORDS].set((i + 1) as f32);
            }
            if resp.dragged() {
                let acc = self.uid(("slot-drag", i));
                let dy = -resp.drag_delta().y / z;
                let moved = v.ui.data_mut(|d| {
                    let a = d.get_temp_mut_or(acc, 0.0f32);
                    *a += dy;
                    let steps = (*a / 14.0).trunc();
                    *a -= steps * 14.0;
                    steps as i32
                });
                if moved != 0 {
                    p.data[data::PROG + i].set((written as i32 + moved).rem_euclid(7) as f32);
                    if !active {
                        p.data[data::CHORDS].set((i + 1) as f32);
                    }
                }
            }
        }

        let menu = widgets::toggle(
            v.ui,
            v.painter,
            self.uid("chords"),
            v.r(W - PAD - 84.0, y0 + 2.0, W - PAD, y0 + 18.0),
            "CHORDS ▾",
            false,
            self.accent,
            z,
        )
        .on_hover_text("Progressions, and how many chords");
        v.text(
            W - PAD - 42.0,
            y0 + 31.0,
            Align2::CENTER_CENTER,
            format!(
                "{} × {}",
                prog.len,
                CHANGES[am::int(p, CHANGE, 0, 3) as usize].0.to_lowercase()
            ),
            v.font(7.5),
            theme::text()_FAINT,
        );
        egui::Popup::menu(&menu)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width(240.0);
                ui.weak("Progressions (in this key and scale)");
                for (name, degrees) in PRESETS {
                    let numerals: Vec<String> =
                        degrees.iter().map(|&d| h.chord(d).numeral()).collect();
                    if ui
                        .button(format!("{name}   {}", numerals.join("–")))
                        .clicked()
                    {
                        am::set_progression(p, degrees);
                        ui.close();
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.weak("Chords");
                    for len in 1..=SLOTS {
                        if ui
                            .selectable_label(prog.len == len, len.to_string())
                            .clicked()
                        {
                            p.data[data::CHORDS].set(len as f32);
                        }
                    }
                });
            });
    }

    /// The knob groups: the generator's, RHYTHM and MELODY, then HARMONY
    /// (with 7THS, PAD and WANDER) and OUTPUT.
    fn knobs(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let engine = self.engine();
        let [top, bottom] = ROWS_Y;
        self.group(ENGINES[engine].0, PAD, 280.0, top, &ENGINE_KNOBS[engine]);
        self.group("RHYTHM", 292.0, 520.0, top, &RHYTHM);
        self.group("MELODY", 532.0, W - PAD, top, &MELODY);
        self.group_title("HARMONY", PAD, 410.0, bottom);
        for (k, &(i, label)) in HARMONY.iter().enumerate() {
            self.knob(i, PAD + 38.0 + 76.0 * k as f32, bottom + 14.0, label);
        }
        const CHIPS: [(usize, &str, &str); 3] = [
            (
                data::SEVENTHS,
                "7THS",
                "Chords with their seventh (the melody's chord tones too)",
            ),
            (
                data::PAD,
                "PAD",
                "Play the chords as well, under the melody's range, by inversion",
            ),
            (
                data::WANDER,
                "WANDER",
                "After the first time round, a chord may give way to another of the same function",
            ),
        ];
        for (k, (i, label, tip)) in CHIPS.into_iter().enumerate() {
            let x = 250.0 + 54.0 * k as f32;
            let resp = widgets::toggle(
                v.ui,
                v.painter,
                self.uid(label),
                v.r(x, bottom + 16.0, x + 50.0, bottom + 32.0),
                label,
                self.on(i),
                self.accent,
                z,
            )
            .on_hover_text(tip);
            if resp.clicked() {
                self.flip(i);
            }
        }
        let h = am::harmony(p);
        let (_, chord) = self.chord_at(self.beat);
        let tones: Vec<&str> = chord.tones().iter().map(|&pc| h.name(pc)).collect();
        v.text(
            330.0,
            bottom + 46.0,
            Align2::CENTER_CENTER,
            format!("{} · {}", chord.name(&h), tones.join(" ")),
            v.mono(8.0),
            theme::text()_DIM,
        );
        self.group("OUTPUT", 422.0, W - PAD, bottom, &OUTPUT);
    }

    fn group_title(&self, title: &str, x0: f32, x1: f32, y: f32) {
        let (v, z) = (self.v, self.z());
        let r = v.text(
            x0,
            y,
            Align2::LEFT_CENTER,
            title,
            theme::strong(8.5 * z),
            theme::text()_DIM,
        );
        let from = (r.right() + 8.0 * z).max(v.p(x0 + 50.0, 0.0).x);
        v.painter.hline(
            from..=v.p(x1, 0.0).x,
            v.p(0.0, y).y,
            Stroke::new(1.0 * z, alpha(theme::text()_FAINT, 0.3)),
        );
    }

    fn group(&self, title: &str, x0: f32, x1: f32, y: f32, knobs: &Knobs) {
        self.group_title(title, x0, x1, y);
        let cw = (x1 - x0) / knobs.len() as f32;
        for (k, &(i, label)) in knobs.iter().enumerate() {
            self.knob(i, x0 + cw * (k as f32 + 0.5), y + 14.0, label);
        }
    }

    /// A knob at `cx`: its label at `y`, the knob under it, the value under
    /// that.
    fn knob(&self, i: usize, cx: f32, y: f32, label: &str) {
        let (v, p, z) = (self.v, self.p, self.z());
        const R: f32 = 10.0;
        let spec = am::spec(i);
        v.text(
            cx,
            y,
            Align2::CENTER_CENTER,
            label,
            v.font(7.5),
            theme::text()_FAINT,
        );
        let top = y + 8.0;
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
            self.accent,
            z,
        );
        let tip = match i {
            SCALE => SCALES[am::int(p, SCALE, 0, 7) as usize].1.to_owned(),
            LIFE_RULE => LIFE_RULES[am::int(p, LIFE_RULE, 0, 5) as usize]
                .1
                .to_owned(),
            CASCADE_RULE => {
                let (rule, what) = CASCADE_RULES[am::int(p, CASCADE_RULE, 0, 7) as usize];
                format!("Rule {rule}: {what}")
            }
            _ => TIPS
                .iter()
                .find(|(k, _)| *k == i)
                .map_or("", |(_, t)| *t)
                .to_owned(),
        };
        let resp = resp.on_hover_text(tip);
        v.hit(resp.rect, ParamRef::Module(i));
        if resp.dragged() || resp.double_clicked() {
            let value = spec.denorm(norm);
            let whole = matches!(
                spec.unit,
                Unit::Integer | Unit::Division | Unit::NoteName | Unit::Channel
            );
            let value = if whole { value.round() } else { value };
            if value != p.get(i) {
                p.set(i, value);
            }
        }
        let hot = resp.dragged() || resp.hovered();
        v.text(
            cx,
            top + 2.0 * R + 8.0,
            Align2::CENTER_CENTER,
            format(p, i),
            v.mono(7.0),
            if hot { theme::text() } else { theme::text()_DIM },
        );
    }
}

/// The filled Julia set of `c`: the outside dark, glowing towards the set's
/// edge, the inside faintly tinted.
fn julia_image(c: (f32, f32), accent: Color32) -> ColorImage {
    const N: usize = 112;
    let mut pixels = Vec::with_capacity(N * N);
    for py in 0..N {
        for px in 0..N {
            let x = (px as f32 + 0.5) / N as f32 * 2.0 * EXTENT - EXTENT;
            let y = EXTENT - (py as f32 + 0.5) / N as f32 * 2.0 * EXTENT;
            pixels.push(shade(escape((x, y), c, 48), accent));
        }
    }
    ColorImage::new([N, N], pixels)
}

/// The Mandelbrot set over `re` and ±`im`.
fn mandelbrot_image(re: (f32, f32), im: f32, accent: Color32) -> ColorImage {
    const N: [usize; 2] = [88, 76];
    let mut pixels = Vec::with_capacity(N[0] * N[1]);
    for py in 0..N[1] {
        for px in 0..N[0] {
            let x = re.0 + (px as f32 + 0.5) / N[0] as f32 * (re.1 - re.0);
            let y = im - (py as f32 + 0.5) / N[1] as f32 * 2.0 * im;
            pixels.push(shade(escape((0.0, 0.0), (x, y), 40), accent));
        }
    }
    ColorImage::new(N, pixels)
}

fn shade(t: f32, accent: Color32) -> Color32 {
    if t >= 1.0 {
        return mix(theme::inset(), accent, 0.16);
    }
    let g = t.powf(0.5);
    let edge = ((t - 0.5) / 0.5).clamp(0.0, 1.0);
    mix(
        mix(theme::inset(), accent, g),
        Color32::WHITE,
        edge * edge * 0.7,
    )
}
