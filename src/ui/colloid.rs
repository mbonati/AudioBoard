//! Panel of Colloid, drawn flat and graphic, like a diagram that moves. The
//! tank fills the top: a liquid in flat bands of depth, a matrix of dots
//! drawn towards the molecules as far as they pull, dashed rings round each
//! molecule for its well. The molecules are line art, coloured outlines
//! only, bonded into a shape by their role (the root a tetrahedron, the third a triangle, the fifth a
//! line, the seventh a star, colour tones a benzene ring), tumbling, an
//! electron on its orbit, the note written in the core; nearby ones are
//! linked by dashes flowing between them. While a molecule rings, rings of
//! sound spread from it. Atoms are dots trailing a wake of smaller dots; a
//! strike draws a crisp ring and a burst of ticks, a note flashes its
//! molecule and floats its name up in a tag. A new chord sweeps a line
//! through the tank. Click or drag in the tank to stir it.
//!
//! Above the tank, HANDPAN or CHORD and the shape, then the root as a row
//! of keys; under it, the notes, one
//! cell per molecule, the knobs (PARTICLES, FIELD, LIQUID, HARMONY, NOTES)
//! and the CC EVENTS lanes.

use std::collections::VecDeque;
use std::f32::consts::{PI, TAU};

use egui::epaint::Mesh;
use egui::{Align2, Color32, CursorIcon, Id, Painter, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};

use super::nodes::{MIN_TEXT, NodeView, segmented};
use super::theme::{self, alpha, mix};
use super::widgets;
use crate::audio::engine::NodeId;
use crate::audio::modules::ModuleParams;
use crate::audio::modules::colloid::chords::{CHORDS, COLOUR, ChordSet, FIFTH, FIRST_CHORD, HANDPANS, ROLES, SEVENTH, THIRD, shape_name};
use crate::audio::modules::colloid::physics::{ASPECT, MAX_ATOMS, MAX_MOLECULES};
use crate::audio::modules::colloid::{
    self as co, ATOMS, ATTRACT, BOUNCE, CHANNEL, CHARGE, COLOR, DRIFT, DYNAMICS, FLOW, HEAT, LENGTH, MASS, MOLECULES, NOTE, OCTAVE, QUANT,
    RANGE, ROOT, SPREAD, STRIKE, VELOCITY, VISCOSITY, cc, viz,
};
use crate::audio::params::{DIVISIONS, NOTE_NAMES, Unit, note_label};
use crate::graph::{ModuleNode, ParamRef};

pub const SIZE: Vec2 = vec2(760.0, 585.0);
const W: f32 = SIZE.x;
const PAD: f32 = 12.0;
/// The tank (its shape is the physics' tank), the notes under it, the
/// knobs, the CC lanes.
/// The row of root keys, then the tank.
const KEYS_Y: [f32; 2] = [56.0, 73.0];
const TOP: f32 = 81.0;
const TANK_H: f32 = (W - 2.0 * PAD) / ASPECT;
const BOTTOM: f32 = TOP + TANK_H;
const STRIP: [f32; 2] = [BOTTOM + 7.0, BOTTOM + 29.0];
const ROWS_Y: [f32; 2] = [STRIP[1] + 14.0, STRIP[1] + 76.0];
const CC_Y: f32 = ROWS_Y[1] + 62.0;

/// How long an atom's wake lasts, and the effects.
const TRAIL: f64 = 0.45;
const GHOST: f64 = 0.7;
const WAVE: f64 = 0.5;
const LABEL: f64 = 1.0;
const SWEEP: f64 = 0.9;
const TRANSMUTE: f64 = 0.5;

/// The molecules' colours, by role: root, third, fifth, seventh, colour.
const ROLE_COLORS: [Color32; 5] = [
    Color32::from_rgb(255, 196, 87),
    Color32::from_rgb(255, 107, 157),
    Color32::from_rgb(72, 219, 251),
    Color32::from_rgb(162, 155, 254),
    Color32::from_rgb(126, 231, 135),
];
/// Atoms: slow, fast, charged.
const COLD: Color32 = Color32::from_rgb(214, 236, 255);
const HOT: Color32 = Color32::from_rgb(255, 150, 205);
const CHARGED: Color32 = Color32::from_rgb(255, 184, 76);
/// The liquid: its top band and its deepest.
const SHALLOW: Color32 = Color32::from_rgb(24, 33, 52);
const DEEP: Color32 = Color32::from_rgb(11, 14, 23);
const INK: Color32 = Color32::from_rgb(12, 14, 20);

type Knobs = [(usize, &'static str)];
const PARTICLES: [(usize, &str); 3] = [(MOLECULES, "MOLECULES"), (ATOMS, "ATOMS"), (MASS, "MASS")];
const FIELD: [(usize, &str); 4] = [(ATTRACT, "ATTRACT"), (RANGE, "RANGE"), (CHARGE, "CHARGE"), (BOUNCE, "BOUNCE")];
const LIQUID: [(usize, &str); 4] = [(VISCOSITY, "VISCOSITY"), (HEAT, "HEAT"), (FLOW, "FLOW"), (DRIFT, "DRIFT")];
const HARMONY: [(usize, &str); 3] = [(OCTAVE, "OCTAVE"), (SPREAD, "SPREAD"), (COLOR, "COLOR")];
const NOTES: [(usize, &str); 5] = [(VELOCITY, "VELOCITY"), (DYNAMICS, "DYNAMICS"), (LENGTH, "LENGTH"), (QUANT, "QUANT"), (CHANNEL, "CHANNEL")];

const TIPS: [(usize, &str); 20] = [
    (MOLECULES, "How many molecules: notes of the handpan or the chord, spread from the bottom up"),
    (ATOMS, "How many atoms are let into the tank while it plays"),
    (MASS, "The atoms' weight: heavier atoms strike harder (louder, longer notes)"),
    (ATTRACT, "How strongly the molecules pull the atoms: low, they drift by; high, they orbit and fall in"),
    (RANGE, "How far the pull reaches: short, only atoms passing close are caught"),
    (CHARGE, "After a strike the atom is pushed off that molecule for a while, then falls back: bouncing rhythms.\nAt 0 atoms cling to a molecule and rattle it with quick, soft notes"),
    (BOUNCE, "How much speed a strike gives back (past 85% strikes add energy: wild)"),
    (VISCOSITY, "How thick the liquid is: it slows the atoms and damps the rings (shorter notes)"),
    (HEAT, "Brownian motion: random kicks that keep the atoms moving"),
    (FLOW, "The liquid's currents, swirling the atoms (and gently the molecules)"),
    (DRIFT, "How much the molecules wander and gather into clusters"),
    (ROOT, "The root: the handpan's ding or the chord's root.\nA key on the MIDI input moves it too; a click here lets go of the notes from the keys"),
    (OCTAVE, "The octave of the lowest molecule (the ding)"),
    (SPREAD, "How many octaves a chord's molecules spread over (a handpan keeps its own layout)"),
    (COLOR, "Colour tones (9ths, 11ths, 13ths) added to a chord (a handpan has its own notes)"),
    (VELOCITY, "Velocity of the hardest strikes"),
    (DYNAMICS, "How much the strike's force shapes the velocity (0: all at VELOCITY)"),
    (LENGTH, "How long the hardest strike rings, so how long its note lasts (thick liquid shortens it, big molecules ring longer)"),
    (QUANT, "FREE: notes when the atoms strike; else held back to the next step of the board's grid"),
    (CHANNEL, "MIDI channel of the notes and the CC lanes"),
];

/// A knob's value as the panel shows it.
pub fn format(p: &ModuleParams, i: usize) -> String {
    let v = p.get(i);
    match i {
        MOLECULES | ATOMS | OCTAVE | VELOCITY => format!("{}", v.round()),
        SPREAD => format!("{} oct", v.round()),
        ROOT => NOTE_NAMES[(v.round().max(0.0) as usize).min(11)].to_owned(),
        LENGTH if v < 1.0 => format!("{:.0} ms", v * 1000.0),
        LENGTH => format!("{v:.2} s"),
        QUANT => match v.round().max(0.0) as usize {
            0 => "FREE".to_owned(),
            q => DIVISIONS[(q - 1).min(DIVISIONS.len() - 1)].0.to_owned(),
        },
        CHANNEL => format!("Ch {}", v.round()),
        _ => format!("{:.0}%", v * 100.0),
    }
}

/// The notes the processor plays (or, before it has run, the knobs').
fn chord(p: &ModuleParams) -> ChordSet {
    let mask = p.viz[viz::MASK].get() as u16;
    if mask == 0 {
        return ChordSet::new(co::int(p, ROOT, 0, 11) as u8, co::shape(p), |_| false);
    }
    let layout = p.viz[viz::LAYOUT].get();
    let layout = (layout >= 0.0).then_some(layout as u8);
    ChordSet { root: p.viz[viz::ROOT].get() as u8 % 12, mask, colours: p.viz[viz::COLOURS].get() as u16, layout }
}

/// The node's two summary lines: the notes, the particles.
pub fn readout(p: &ModuleParams) -> (String, String) {
    let source = if p.viz[viz::SOURCE].get() > 0.5 { " · keys" } else { "" };
    (format!("{}{source}", chord(p).name()), format!("{} molecules · {} atoms", co::int(p, MOLECULES, 2, 16), co::int(p, ATOMS, 1, 48)))
}

/// A molecule as published.
#[derive(Clone, Copy, Default)]
struct Mol {
    x: f32,
    y: f32,
    r: f32,
    angle: f32,
    ring: f32,
    note: u8,
    role: u8,
    sounding: bool,
    spin: f32,
}

/// An atom as published.
#[derive(Clone, Copy, Default)]
struct At {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    charge: f32,
    age: f32,
}

fn molecules(p: &ModuleParams) -> ([Mol; MAX_MOLECULES], usize) {
    let n = (p.viz[viz::MOLECULES].get().max(0.0) as usize).min(MAX_MOLECULES);
    let mols = std::array::from_fn(|k| {
        let f = |i: usize| p.viz[viz::MOLECULE + k * viz::MOLECULE_LEN + i].get();
        Mol { x: f(0), y: f(1), r: f(2), angle: f(3), ring: f(4).clamp(0.0, 1.0), note: f(5) as u8, role: (f(6) as u8).min(4), sounding: f(7) > 0.5, spin: f(8) }
    });
    (mols, n)
}

fn atoms(p: &ModuleParams) -> ([At; MAX_ATOMS], usize) {
    let n = (p.viz[viz::ATOMS].get().max(0.0) as usize).min(MAX_ATOMS);
    let atoms = std::array::from_fn(|k| {
        let f = |i: usize| p.viz[viz::ATOM + k * viz::ATOM_LEN + i].get();
        At { x: f(0), y: f(1), vx: f(2), vy: f(3), charge: f(4).clamp(0.0, 1.0), age: f(5) }
    });
    (atoms, n)
}

/// An atom's colour: pale when slow, pink when fast, amber when charged.
fn atom_color(a: &At) -> Color32 {
    let speed = (a.vx * a.vx + a.vy * a.vy).sqrt();
    mix(mix(COLD, HOT, (speed / 1.8).clamp(0.0, 1.0)), CHARGED, a.charge * 0.85)
}

#[derive(Clone)]
struct Ghost {
    trail: Vec<(Pos2, f64)>,
    at: f64,
    color: Color32,
}

#[derive(Clone, Copy)]
struct Wave {
    at: Pos2,
    t0: f64,
    size: f32,
    strength: f32,
    color: Color32,
}

/// A tick of a strike's burst, flying out.
#[derive(Clone, Copy)]
struct Tick {
    pos: Pos2,
    vel: Vec2,
    t0: f64,
    life: f32,
    color: Color32,
}

#[derive(Clone)]
struct Label {
    molecule: usize,
    text: String,
    t0: f64,
    color: Color32,
    level: f32,
}

/// What the panel remembers between frames.
#[derive(Clone, Default)]
struct View {
    started: bool,
    time: f64,
    seq: f32,
    rng: u32,
    trails: Vec<VecDeque<(Pos2, f64)>>,
    atom_count: usize,
    ghosts: Vec<Ghost>,
    waves: Vec<Wave>,
    ticks: Vec<Tick>,
    labels: Vec<Label>,
    /// Per molecule: its last note (when, how hard), and what it showed
    /// (note, colour from, colour to, when it changed).
    flash: [(f64, f32); MAX_MOLECULES],
    shown: [(u8, Color32, Color32, f64); MAX_MOLECULES],
    /// The notes seen, and when they changed.
    chord: (u16, u16, u8, Option<u8>),
    chord_at: f64,
    /// Times of the latest notes, for the rate.
    notes: VecDeque<f64>,
    /// The last stir's ring.
    stirred: f64,
}

impl View {
    fn random(&mut self) -> f32 {
        let mut x = self.rng.max(1);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Takes in what the processor published since the last frame.
    fn update(&mut self, p: &ModuleParams, now: f64, dt: f32, mols: &[Mol], ats: &[At]) {
        let set = chord(p);
        if !self.started {
            self.started = true;
            self.rng = 0x2545_F491;
            self.flash = [(f64::NEG_INFINITY, 0.0); MAX_MOLECULES];
            self.shown = std::array::from_fn(|k| {
                let c = mols.get(k).map_or(ROLE_COLORS[0], |m| ROLE_COLORS[m.role as usize]);
                (mols.get(k).map_or(0, |m| m.note), c, c, f64::NEG_INFINITY)
            });
            self.chord = (set.mask, set.colours, set.root, set.layout);
            self.chord_at = f64::NEG_INFINITY;
            self.stirred = f64::NEG_INFINITY;
            self.seq = p.viz[viz::EVENT_SEQ].get();
        }
        if self.trails.len() != MAX_ATOMS {
            self.trails = vec![VecDeque::new(); MAX_ATOMS];
        }

        // Strikes and notes.
        let seq = p.viz[viz::EVENT_SEQ].get();
        if seq < self.seq {
            self.seq = 0.0;
        }
        let (from, to) = (self.seq as u32 + 1, seq as u32);
        for n in from.max(to.saturating_sub(viz::EVENT_CAP as u32 - 1))..=to {
            let at = viz::EVENTS + (n as usize - 1) % viz::EVENT_CAP * viz::EVENT_LEN;
            let field = |k: usize| p.viz[at + k].get();
            if field(0) as u32 != n {
                continue;
            }
            let k = (field(2) as usize).min(MAX_MOLECULES - 1);
            let Some(m) = mols.get(k).copied() else { continue };
            let color = ROLE_COLORS[m.role as usize];
            match field(1) as u8 {
                STRIKE => self.strike(m, pos2(field(3), field(4)), field(5), color, now),
                NOTE => {
                    let level = field(5);
                    self.flash[k] = (now, level);
                    self.notes.push_back(now);
                    // One name floats up at a time from each molecule.
                    self.labels.retain(|l| l.molecule != k);
                    self.labels.push(Label { molecule: k, text: note_label(field(6) as i32), t0: now, color, level });
                }
                _ => {}
            }
        }
        self.seq = seq;

        // Wakes: each atom's path; atoms that left fade as ghosts.
        for (k, a) in ats.iter().enumerate() {
            let trail = &mut self.trails[k];
            let here = pos2(a.x, a.y);
            if trail.back().is_some_and(|(q, _)| (*q - here).length() > 0.35) || a.age < 0.02 {
                trail.clear();
            }
            trail.push_back((here, now));
            while trail.front().is_some_and(|(_, t)| now - t > TRAIL) || trail.len() > 64 {
                trail.pop_front();
            }
        }
        for k in ats.len()..self.atom_count.min(MAX_ATOMS) {
            let trail: Vec<(Pos2, f64)> = self.trails[k].drain(..).collect();
            if !trail.is_empty() {
                self.ghosts.push(Ghost { trail, at: now, color: COLD });
            }
        }
        for trail in &mut self.trails[ats.len()..] {
            trail.clear();
        }
        self.atom_count = ats.len();

        // Molecules taking new notes change colour.
        for (k, m) in mols.iter().enumerate() {
            let color = ROLE_COLORS[m.role as usize];
            let (note, _, to, _) = self.shown[k];
            if note != m.note {
                let was = self.molecule_color(k, now);
                self.shown[k] = (m.note, was, color, now);
                if m.r > 0.01 {
                    self.waves.push(Wave { at: pos2(m.x, m.y), t0: now, size: m.r * 2.2, strength: 0.8, color });
                }
            } else if to != color {
                self.shown[k] = (note, self.molecule_color(k, now), color, now);
            }
        }
        // New notes: a line sweeps the tank.
        let seen = (set.mask, set.colours, set.root, set.layout);
        if seen != self.chord {
            if (seen.0, seen.2, seen.3) != (self.chord.0, self.chord.2, self.chord.3) {
                self.chord_at = now;
            }
            self.chord = seen;
        }

        // Effects move on and fade.
        for t in &mut self.ticks {
            t.pos += t.vel * dt;
            t.vel *= (-dt * 4.0).exp();
        }
        self.ticks.retain(|t| now - t.t0 < t.life as f64);
        if self.ticks.len() > 400 {
            self.ticks.drain(..self.ticks.len() - 400);
        }
        self.waves.retain(|w| now - w.t0 < WAVE);
        self.labels.retain(|l| now - l.t0 < LABEL);
        if self.labels.len() > 24 {
            self.labels.drain(..self.labels.len() - 24);
        }
        self.ghosts.retain(|g| now - g.at < GHOST);
        while self.notes.front().is_some_and(|&t| now - t > 4.0) {
            self.notes.pop_front();
        }
    }

    /// A strike: a crisp ring, and ticks bursting off the surface.
    fn strike(&mut self, m: Mol, at: Pos2, force: f32, color: Color32, now: f64) {
        let force = force.clamp(0.0, 1.0);
        let light = mix(color, Color32::WHITE, 0.55);
        self.waves.push(Wave { at, t0: now, size: 0.03 + 0.07 * force, strength: 0.4 + 0.6 * force, color: light });
        let normal = (at - pos2(m.x, m.y)).normalized();
        let count = 3 + (force * 5.0) as usize;
        for i in 0..count {
            let spread = (i as f32 + 0.5) / count as f32 - 0.5;
            let angle = normal.angle() + spread * 2.2 + (self.random() - 0.5) * 0.3;
            let speed = 0.35 + 0.6 * force;
            self.ticks.push(Tick { pos: at, vel: Vec2::angled(angle) * speed, t0: now, life: 0.28, color: light });
        }
    }

    /// The colour molecule `k` shows now (crossfading to a new role's).
    fn molecule_color(&self, k: usize, now: f64) -> Color32 {
        let (_, from, to, at) = self.shown[k];
        let t = ((now - at) / TRANSMUTE).clamp(0.0, 1.0) as f32;
        mix(from, to, t * t * (3.0 - 2.0 * t))
    }

    fn busy(&self, now: f64) -> bool {
        !self.ticks.is_empty() || !self.waves.is_empty() || !self.labels.is_empty() || !self.ghosts.is_empty() || now - self.chord_at < SWEEP
    }
}

struct Panel<'a, 'b> {
    v: &'a NodeView<'b>,
    id: NodeId,
    p: &'a ModuleParams,
    accent: Color32,
    now: f64,
    live: bool,
}

/// From tank coordinates to the screen.
#[derive(Clone, Copy)]
struct Tank {
    rect: Rect,
    scale: f32,
}

impl Tank {
    fn at(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.rect.left() + x * self.scale, self.rect.top() + y * self.scale)
    }

    fn pos(&self, q: Pos2) -> Pos2 {
        self.at(q.x, q.y)
    }

    fn back(&self, s: Pos2) -> Pos2 {
        pos2((s.x - self.rect.left()) / self.scale, (s.y - self.rect.top()) / self.scale)
    }
}

/// `(midi_in, midi_out)`: the MIDI input and output are patched.
pub fn draw(v: &NodeView, id: NodeId, accent: Color32, m: &ModuleNode, (midi_in, midi_out): (bool, bool)) {
    let p = &m.params;
    let key = Id::unique(("colloid-view", id));
    let mut view: View = v.ui.data_mut(|d| d.remove_temp(key)).unwrap_or_default();
    let now = v.ui.input(|i| i.time);
    let dt = (now - view.time).clamp(0.0, 0.1) as f32;
    view.time = now;
    let (mols, n_mols) = molecules(p);
    let (ats, n_atoms) = atoms(p);
    view.update(p, now, dt, &mols[..n_mols], &ats[..n_atoms]);
    let live = p.viz[viz::LIVE].get() > 0.5;
    let s = Panel { v, id, p, accent, now, live };

    let col = |on: bool| if on { theme::TEXT_DIM } else { theme::TEXT_FAINT };
    v.text(PAD, 42.0, Align2::LEFT_CENTER, "MIDI", v.font(9.0), col(midi_in));
    v.text(W - PAD, 42.0, Align2::RIGHT_CENTER, "MIDI", v.font(9.0), col(midi_out));
    s.header(&view);
    s.roots();
    s.tank(&mut view, &mols[..n_mols], &ats[..n_atoms]);
    s.strip(&view, &mols[..n_mols]);
    s.knobs();
    s.lanes();
    if live || view.busy(now) {
        v.ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }
    v.ui.data_mut(|d| d.insert_temp(key, view));
}

impl Panel<'_, '_> {
    fn z(&self) -> f32 {
        self.v.z
    }

    fn uid(&self, salt: impl std::hash::Hash + std::fmt::Debug) -> Id {
        Id::unique(("colloid", self.id)).with(salt)
    }

    /// A handpan is playing (not a chord).
    fn handpan(&self) -> bool {
        chord(self.p).layout.is_some()
    }

    /// HANDPAN or CHORD, the shape, RESEED, PLAY and what the tank is doing.
    fn header(&self, view: &View) {
        let (v, p, z) = (self.v, self.p, self.z());
        let shape = co::shape(p);
        let keyed = p.viz[viz::SOURCE].get() > 1.5;
        let family = (shape >= FIRST_CHORD) as usize;
        const FAMILIES: [(&str, &str); 2] = [("HANDPAN", "Handpan and Hang layouts: a ding and its tone fields, every note in tune with every other"), ("CHORD", "Chords, with colour tones (COLOR)")];
        let picked = segmented(v, self.uid("family"), v.r(46.0, 34.0, 150.0, 50.0), 2, family, self.accent, |k, r, col| {
            v.painter.text(r.center(), Align2::CENTER_CENTER, FAMILIES[k].0, v.font(7.0), col);
            FAMILIES[k].1
        });
        if let Some(k) = picked
            && k != family
        {
            p.set_mode(if k == 0 { 0 } else { FIRST_CHORD });
        }
        let (first, count) = if family == 0 { (0, HANDPANS.len()) } else { (FIRST_CHORD, CHORDS.len()) };
        let picked = segmented(v, self.uid(("shape", family)), v.r(156.0, 34.0, 522.0, 50.0), count, shape - first, self.accent, |k, r, col| {
            let col = if keyed { alpha(col, 0.5) } else { col };
            let (label, long) = shape_name(first + k);
            v.painter.text(r.center(), Align2::CENTER_CENTER, label, v.font(if family == 0 { 6.5 } else { 7.0 }), col);
            long
        });
        if let Some(k) = picked {
            p.set_mode(first + k);
        }
        let reseed = widgets::toggle(v.ui, v.painter, self.uid("reseed"), v.r(530.0, 34.0, 588.0, 50.0), "RESEED", false, self.accent, z)
            .on_hover_text("A new start: the molecules scattered anew, new atoms, other colour tones");
        if reseed.clicked() {
            co::reseed(p, (self.now * 1000.0) as u32 ^ self.id.wrapping_mul(0x9E37_79B9));
        }
        let play = widgets::toggle(v.ui, v.painter, self.uid("play"), v.r(594.0, 34.0, 636.0, 50.0), "PLAY", p.toggled(), self.accent, z)
            .on_hover_text("Let the atoms in while the transport plays (off: they leave, the molecules keep floating)");
        if play.clicked() {
            p.set_toggle(!p.toggled());
        }
        let rate = view.notes.len() as f32 / 4.0;
        let text = if !p.toggled() {
            "paused".to_owned()
        } else if !self.live {
            "waiting".to_owned()
        } else {
            format!("{rate:.1} n/s")
        };
        v.text(644.0, 42.0, Align2::LEFT_CENTER, text, v.font(7.5), if self.live { theme::TEXT_DIM } else { theme::TEXT_FAINT });
    }

    /// The root as a row of keys, C to B: click one (it also lets go of
    /// notes held from the keys). When the keys set the root, the one they
    /// play is ringed.
    fn roots(&self) {
        let (v, p, z) = (self.v, self.p, self.z());
        let [y0, y1] = KEYS_Y;
        let mid = (y0 + y1) * 0.5;
        v.text(PAD, mid, Align2::LEFT_CENTER, "ROOT", theme::strong(8.0 * z), theme::TEXT_DIM);
        let (x0, x1) = (48.0, 522.0);
        let kw = (x1 - x0) / 12.0;
        let picked = co::int(p, ROOT, 0, 11) as usize;
        let keyed = p.viz[viz::SOURCE].get() > 0.5;
        let playing = chord(p).root as usize;
        let tip = TIPS.iter().find(|(k, _)| *k == ROOT).map_or("", |(_, t)| *t);
        for (k, name) in NOTE_NAMES.iter().enumerate() {
            let rect = v.r(x0 + kw * k as f32 + 1.0, y0, x0 + kw * (k + 1) as f32 - 1.0, y1);
            let sharp = matches!(k, 1 | 3 | 6 | 8 | 10);
            let resp = v.ui.interact(rect, self.uid(("root", k)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip);
            let selected = k == picked;
            let fill = if selected {
                self.accent
            } else if resp.hovered() {
                theme::CONTROL_HI
            } else if sharp {
                theme::INSET
            } else {
                theme::CONTROL
            };
            v.painter.rect_filled(rect, 3.0 * z, fill);
            let ink = if selected {
                theme::INSET
            } else if sharp {
                theme::TEXT_DIM
            } else {
                theme::TEXT
            };
            label(v.painter, rect.center(), Align2::CENTER_CENTER, name, v.font(7.5), ink);
            if keyed && k == playing {
                v.painter.rect_stroke(rect.expand(1.0 * z), 3.5 * z, Stroke::new(1.5 * z, theme::TEXT), StrokeKind::Outside);
            }
            if resp.clicked() {
                p.set(ROOT, k as f32);
                if keyed {
                    p.viz[viz::UNLATCH].set(p.viz[viz::UNLATCH].get() + 1.0);
                }
            }
        }
        v.hit(v.r(x0, y0, x1, y1), ParamRef::Module(ROOT));
        let note = if keyed { format!("keys: {}", NOTE_NAMES[playing]) } else { format!("{}{}", NOTE_NAMES[picked], co::int(p, OCTAVE, 1, 6)) };
        v.text(530.0, mid, Align2::LEFT_CENTER, note, v.font(7.5), if keyed { theme::TEXT } else { theme::TEXT_FAINT });
    }

    fn tank(&self, view: &mut View, mols: &[Mol], ats: &[At]) {
        let (v, z) = (self.v, self.z());
        let rect = v.r(PAD, TOP, W - PAD, BOTTOM);
        let t = Tank { rect, scale: rect.height() };
        let painter = v.painter.with_clip_rect(rect.intersect(v.painter.clip_rect()));
        let now = self.now;

        self.backdrop(&painter, t);
        self.dots(&painter, t, mols);
        self.wells(&painter, t, mols, view);
        self.bonds(&painter, t, mols, view);
        self.wakes(&painter, t, view, ats);
        for (k, m) in mols.iter().enumerate() {
            self.molecule(&painter, t, k, m, view);
        }
        self.effects(&painter, t, view, mols);
        self.atoms(&painter, t, ats);
        self.sweep(&painter, t, view);
        self.overlay(&painter, t, mols, ats.len());
        painter.rect_stroke(rect, 4.0 * z, Stroke::new(1.0 * z, alpha(theme::NODE_BORDER, 0.9)), StrokeKind::Inside);

        // Stirring: a click pushes outwards, a drag drags the liquid along.
        let resp = v
            .ui
            .interact(rect, self.uid("tank"), Sense::click_and_drag())
            .on_hover_cursor(CursorIcon::Crosshair)
            .on_hover_text("Click to push the liquid outwards, drag to stir it");
        if let Some(q) = resp.interact_pointer_pos() {
            let at = t.back(q);
            let dt = v.ui.input(|i| i.stable_dt).max(1e-3);
            let stir = if resp.clicked() || resp.drag_started() {
                Some((0.0, 0.0))
            } else if resp.dragged() {
                let d = resp.drag_delta() / t.scale / dt;
                (d.length() > 0.05).then_some((d.x, d.y))
            } else {
                None
            };
            if let Some((dx, dy)) = stir {
                let p = self.p;
                for (i, value) in [at.x, at.y, dx, dy].into_iter().enumerate() {
                    p.viz[viz::STIR + i].set(value);
                }
                p.viz[viz::STIR_SEQ].set(p.viz[viz::STIR_SEQ].get() + 1.0);
                let push = dx == 0.0 && dy == 0.0;
                if push || now - view.stirred > 0.08 {
                    view.stirred = now;
                    view.waves.push(Wave { at, t0: now, size: if push { 0.28 } else { 0.1 }, strength: 0.7, color: COLD });
                }
            }
        }
        // Letting go of the keys' chord.
        if self.p.viz[viz::SOURCE].get() > 0.5 {
            let r = v.r(PAD + 8.0, TOP + 40.0, PAD + 70.0, TOP + 54.0);
            let resp = widgets::toggle(v.ui, v.painter, self.uid("unlatch"), r, "× KEYS", false, self.accent, z)
                .on_hover_text("Let go of the notes from the keys: back to the panel's ROOT and shape");
            if resp.clicked() {
                self.p.viz[viz::UNLATCH].set(self.p.viz[viz::UNLATCH].get() + 1.0);
            }
        }
    }

    /// The liquid in flat bands, lighter towards the surface, and the
    /// surface itself, a moving line.
    fn backdrop(&self, painter: &Painter, t: Tank) {
        let (z, now) = (self.z(), self.now as f32);
        const BANDS: usize = 6;
        for b in 0..BANDS {
            let (y0, y1) = (b as f32 / BANDS as f32, (b + 1) as f32 / BANDS as f32);
            let shade = mix(SHALLOW, DEEP, (b as f32 / (BANDS - 1) as f32).powf(0.8));
            painter.rect_filled(Rect::from_min_max(t.at(0.0, y0), t.at(ASPECT, y1 + 0.002)), 0.0, shade);
        }
        let y = |x: f32| 0.014 + 0.006 * (x * 7.0 + now * 1.1).sin();
        let pts: Vec<Pos2> = (0..=60).map(|i| i as f32 / 60.0 * ASPECT).map(|x| t.at(x, y(x))).collect();
        painter.add(Shape::line(pts, Stroke::new(1.2 * z, alpha(Color32::from_rgb(170, 210, 255), 0.4))));
    }

    /// A matrix of dots, drawn towards the molecules as far as they pull,
    /// bigger and brighter in the wells.
    fn dots(&self, painter: &Painter, t: Tank, mols: &[Mol]) {
        let (p, z) = (self.p, self.z());
        let pull = 0.25 + 1.3 * p.get(ATTRACT).clamp(0.0, 1.0);
        let reach = 0.08 + 1.5 * p.get(RANGE).clamp(0.0, 1.0).powi(2);
        let warp = |x: f32, y: f32| -> (Pos2, f32) {
            let (mut dx, mut dy, mut s) = (0.0, 0.0, 0.0);
            for m in mols {
                let (ex, ey) = (m.x - x, m.y - y);
                let d2 = ex * ex + ey * ey;
                let d = d2.sqrt().max(1e-4);
                let q = d2 / (reach * reach);
                let g = pull * (m.r / 0.07).powi(2) * (1.0 + 0.6 * m.ring);
                let mag = (1.4 * g * m.r * m.r / (d + m.r) / (1.0 + q * q)).min(d * 0.55);
                dx += ex / d * mag;
                dy += ey / d * mag;
                s += mag / (m.r * 0.6);
            }
            (t.at(x + dx, y + dy), s.min(1.0))
        };
        const STEP: f32 = 1.0 / 15.0;
        let cols = (ASPECT / STEP).round() as usize;
        let ink = Color32::from_rgb(120, 160, 210);
        let mut mesh = Mesh::default();
        for row in 1..15 {
            for col in 1..cols {
                let (q, s) = warp(col as f32 * ASPECT / cols as f32, row as f32 * STEP);
                disc(&mut mesh, q, (0.9 + 1.5 * s) * z, alpha(ink, 0.16 + 0.55 * s), 6);
            }
        }
        painter.add(Shape::mesh(mesh));
    }

    /// Each molecule's well: a dashed ring turning round it, wider as the
    /// pull reaches further, plainer as it pulls harder.
    fn wells(&self, painter: &Painter, t: Tank, mols: &[Mol], view: &View) {
        let (p, z, now) = (self.p, self.z(), self.now as f32);
        let reach = p.get(RANGE).clamp(0.0, 1.0);
        let strength = 0.07 + 0.16 * p.get(ATTRACT).clamp(0.0, 1.0);
        for (k, m) in mols.iter().enumerate() {
            let color = view.molecule_color(k, self.now);
            let r = m.r * t.scale * (2.1 + 1.5 * reach);
            let turn = if k % 2 == 0 { 0.15 } else { -0.15 };
            dashed_circle(painter, t.at(m.x, m.y), r, 18, now * turn + k as f32, Stroke::new(1.0 * z, alpha(color, strength)));
        }
    }

    /// Molecules near each other linked by dashes flowing between them.
    fn bonds(&self, painter: &Painter, t: Tank, mols: &[Mol], view: &View) {
        let (z, now) = (self.z(), self.now as f32);
        for i in 0..mols.len() {
            for j in i + 1..mols.len() {
                let (a, b) = (&mols[i], &mols[j]);
                let (pa, pb) = (pos2(a.x, a.y), pos2(b.x, b.y));
                let d = (pb - pa).length();
                let near = (a.r + b.r) * 3.4;
                if d >= near || d < 1e-3 {
                    continue;
                }
                let close = 1.0 - d / near;
                let dir = (pb - pa) / d;
                let (sa, sb) = (t.pos(pa + dir * a.r * 1.1), t.pos(pb - dir * b.r * 1.1));
                let color = mix(view.molecule_color(i, self.now), view.molecule_color(j, self.now), 0.5);
                dashed_line(painter, sa, sb, 4.0 * z, 4.0 * z, now * 14.0 * z, Stroke::new((0.8 + 1.0 * close) * z, alpha(color, 0.25 + 0.5 * close)));
            }
        }
    }

    /// How bright molecule `k`'s last note still flashes.
    fn flash(&self, view: &View, k: usize) -> f32 {
        let (at, level) = view.flash[k];
        (-(self.now - at) as f32 / 0.18).exp() * (0.4 + 0.6 * level)
    }

    /// Atoms' wakes: dots along their path, smaller and fainter behind;
    /// those of atoms that left fade out.
    fn wakes(&self, painter: &Painter, t: Tank, view: &View, ats: &[At]) {
        let z = self.z();
        let ra = (co::settings(self.p).atom_r * t.scale).max(1.8 * z);
        let gap = 6.0 * z;
        let mut mesh = Mesh::default();
        let mut wake = |trail: &mut dyn DoubleEndedIterator<Item = &(Pos2, f64)>, color: Color32, fade: f32| {
            let mut prev: Option<Pos2> = None;
            let mut walked = 0.0;
            let mut next = gap * 0.6;
            for (q, at) in trail.rev() {
                let s = t.pos(*q);
                let age = ((self.now - at) / TRAIL) as f32;
                if let Some(p0) = prev {
                    let seg = s - p0;
                    let len = seg.length();
                    while next <= walked + len {
                        let f = (next - walked) / len.max(1e-6);
                        let left = (1.0 - age).clamp(0.0, 1.0);
                        disc(&mut mesh, p0 + seg * f, ra * 0.8 * left, alpha(color, 0.85 * left * fade), 8);
                        next += gap;
                    }
                    walked += len;
                }
                prev = Some(s);
            }
        };
        for (k, a) in ats.iter().enumerate() {
            wake(&mut view.trails[k].iter(), atom_color(a), 1.0);
        }
        for g in &view.ghosts {
            let fade = 1.0 - ((self.now - g.at) / GHOST) as f32;
            wake(&mut g.trail.iter(), g.color, fade.max(0.0));
        }
        painter.add(Shape::mesh(mesh));
    }

    /// A molecule in line art: rings bonded into its role's shape, drawn
    /// in coloured outlines only, tumbling, an electron on its orbit, its
    /// note written in the core; rings of sound spreading while it rings.
    /// A note brightens and thickens its lines.
    fn molecule(&self, painter: &Painter, t: Tank, k: usize, m: &Mol, view: &View) {
        let z = self.z();
        let now = self.now as f32;
        let flash = self.flash(view, k);
        let r = m.r * t.scale * (1.0 + 0.1 * flash);
        if r < 0.5 {
            return;
        }
        let c = t.at(m.x, m.y);
        let color = view.molecule_color(k, self.now);
        let ring = m.ring;
        // Rings of sound while it rings.
        if ring > 0.03 {
            for i in 0..2 {
                let phase = (now * 1.5 + i as f32 / 2.0 + k as f32 * 0.37).fract();
                painter.circle_stroke(c, r * (1.1 + 1.2 * phase), Stroke::new(1.1 * z, alpha(color, ring * (1.0 - phase) * 0.7)));
            }
        }
        // Its nuclei: (count, distance, radius, core) by role.
        let (count, dist, size, core) = match m.role {
            THIRD => (3, 0.68, 0.3, 0.52),
            FIFTH => (2, 0.7, 0.34, 0.5),
            SEVENTH => (5, 0.7, 0.26, 0.46),
            COLOUR => (6, 0.7, 0.24, 0.0),
            _ => (4, 0.68, 0.3, 0.54),
        };
        let tilt = 0.75 + 0.35 * (now * 0.27 + k as f32 * 1.3).sin();
        let shake = |i: usize| -> Vec2 {
            let fi = i as f32 * 1.7 + k as f32;
            vec2((now * 41.0 + fi).sin(), (now * 37.0 + fi * 1.3).cos()) * ring * 0.09 * r
        };
        let place = |i: usize| -> (Pos2, f32) {
            let a = m.angle + i as f32 * TAU / count as f32;
            (c + vec2(a.cos(), a.sin() * tilt.cos()) * dist * r + shake(i + 1), a.sin() * tilt.sin())
        };
        let mut nuclei: Vec<(Pos2, f32, f32)> = (0..count).map(|i| (place(i).0, size * r, place(i).1)).collect();
        if m.role == 0 {
            // A tetrahedron: one nucleus up front, the others round it.
            nuclei[0] = (c + vec2(0.0, -0.08 * r) + shake(9), size * r * 1.1, 1.0);
        }
        let line = mix(color, Color32::WHITE, 0.7 * flash);
        let width = (r * 0.055).max(1.1 * z) * (1.0 + 0.9 * flash);
        // Hollow: what is behind a ring (its bonds) stays hidden.
        let hollow = alpha(INK, 0.88);
        let core_at = c + shake(0);
        let core_r = core * r;
        let nucleus = |q: Pos2, s: f32, depth: f32| {
            let a = if depth < 0.0 { 1.0 + depth * 0.45 } else { 1.0 };
            painter.circle_filled(q, s, hollow);
            painter.circle_stroke(q, s, Stroke::new(width, alpha(line, a)));
        };
        let bond = |a: Pos2, b: Pos2, depth: f32| {
            let fade = if depth < 0.0 { 0.55 } else { 0.85 };
            painter.line_segment([a, b], Stroke::new(width * 0.85, alpha(line, fade)));
        };
        if core > 0.0 {
            let mut order: Vec<usize> = (0..nuclei.len()).collect();
            order.sort_by(|&a, &b| nuclei[a].2.total_cmp(&nuclei[b].2));
            for &i in order.iter().filter(|&&i| nuclei[i].2 < 0.0) {
                bond(core_at, nuclei[i].0, nuclei[i].2);
                nucleus(nuclei[i].0, nuclei[i].1, nuclei[i].2);
            }
            nucleus(core_at, core_r, 0.0);
            painter.circle_stroke(core_at, core_r * 0.7, Stroke::new(width * 0.5, alpha(line, 0.35)));
            for &i in order.iter().filter(|&&i| nuclei[i].2 >= 0.0) {
                bond(core_at, nuclei[i].0, nuclei[i].2);
                nucleus(nuclei[i].0, nuclei[i].1, nuclei[i].2);
            }
        } else {
            // A benzene ring: bonds round it, every other one double.
            for i in 0..count {
                let ((a, da), (b, db)) = ((nuclei[i].0, nuclei[i].2), (nuclei[(i + 1) % count].0, nuclei[(i + 1) % count].2));
                bond(a, b, (da + db) * 0.5);
                if i % 2 == 0 {
                    let inward = ((c - a) + (c - b)).normalized() * r * 0.17;
                    painter.line_segment([a + inward, b + inward], Stroke::new(width * 0.5, alpha(line, 0.6)));
                }
            }
            for &(q, s, depth) in &nuclei {
                nucleus(q, s, depth);
            }
        }
        // An electron (two for the root) on a tilted orbit.
        if r > 5.0 * z {
            for e in 0..if m.role == 0 { 2 } else { 1 } {
                let turn = k as f32 * 1.3 + e as f32 * 1.9 + now * 0.11;
                let (ax, ay) = (1.5 * r, 0.42 * r);
                let orbit = |a: f32| -> Pos2 {
                    let (x, y) = (a.cos() * ax, a.sin() * ay);
                    c + vec2(x * turn.cos() - y * turn.sin(), x * turn.sin() + y * turn.cos())
                };
                let pts: Vec<Pos2> = (0..=40).map(|i| orbit(i as f32 / 40.0 * TAU)).collect();
                painter.add(Shape::line(pts, Stroke::new(0.7 * z, alpha(color, 0.2 + 0.3 * ring))));
                let phase = now * (1.4 + 5.5 * ring + 0.8 * m.spin.abs().min(4.0)) + k as f32 + e as f32 * PI;
                let at = orbit(phase);
                painter.circle_filled(at, 2.2 * z, hollow);
                painter.circle_stroke(at, 2.2 * z, Stroke::new(1.0 * z, line));
            }
        }
        // Its note: in the core, or under the molecule if the core is small.
        let name = note_label(m.note as i32);
        let inside = if core > 0.0 { core_r } else { r * 0.5 };
        if inside >= 10.5 * z {
            label(painter, if core > 0.0 { core_at } else { c }, Align2::CENTER_CENTER, &name, theme::strong(7.0 * z), line);
        } else {
            let ink = alpha(mix(color, Color32::WHITE, 0.3), if m.sounding { 0.95 } else { 0.5 });
            label(painter, c + vec2(0.0, r + 7.0 * z), Align2::CENTER_CENTER, &name, theme::mono(6.5 * z), ink);
        }
    }

    /// Rings, bursts of ticks and the notes' tags floating up.
    fn effects(&self, painter: &Painter, t: Tank, view: &View, mols: &[Mol]) {
        let z = self.z();
        for w in &view.waves {
            let age = ((self.now - w.t0) / WAVE) as f32;
            let ease = 1.0 - (1.0 - age).powi(3);
            let rad = (0.008 + w.size * ease) * t.scale;
            painter.circle_stroke(t.pos(w.at), rad, Stroke::new((2.0 * (1.0 - age) + 0.5) * z, alpha(w.color, (1.0 - age) * w.strength)));
        }
        for tick in &view.ticks {
            let age = ((self.now - tick.t0) / tick.life as f64) as f32;
            let fade = (1.0 - age).clamp(0.0, 1.0);
            let head = t.pos(tick.pos);
            let tail = t.pos(tick.pos - tick.vel * 0.05);
            painter.line_segment([tail, head], Stroke::new(1.4 * z, alpha(tick.color, fade)));
        }
        for l in &view.labels {
            let Some(m) = mols.get(l.molecule) else { continue };
            let age = ((self.now - l.t0) / LABEL) as f32;
            let rise = (1.0 - (1.0 - age).powi(2)) * 0.08;
            let fade = (1.0 - age).clamp(0.0, 1.0).powf(0.6);
            let at = t.at(m.x, m.y - m.r - 0.04 - rise);
            let font = theme::strong((8.0 + 2.0 * l.level) * z);
            if font.size < MIN_TEXT {
                continue;
            }
            let galley = painter.layout_no_wrap(l.text.clone(), font, l.color);
            let tag = Rect::from_center_size(at, galley.size() + vec2(8.0, 3.0) * z);
            painter.rect_filled(tag, tag.height() * 0.5, alpha(INK, 0.8 * fade));
            painter.rect_stroke(tag, tag.height() * 0.5, Stroke::new(1.2 * z, alpha(l.color, fade)), StrokeKind::Inside);
            painter.galley(tag.center() - galley.size() * 0.5, galley, alpha(l.color, fade));
        }
    }

    /// The atoms: flat dots, ringed in amber while charged, a ring opening
    /// as they enter.
    fn atoms(&self, painter: &Painter, t: Tank, ats: &[At]) {
        let z = self.z();
        let ra = (co::settings(self.p).atom_r * t.scale).max(1.8 * z);
        for a in ats {
            let c = t.at(a.x, a.y);
            let color = atom_color(a);
            if a.charge > 0.05 {
                painter.circle_stroke(c, ra * 2.2, Stroke::new(1.0 * z, alpha(CHARGED, 0.8 * a.charge)));
            }
            painter.circle_filled(c, ra, color);
            painter.circle_stroke(c, ra, Stroke::new(0.8 * z, alpha(INK, 0.6)));
            if a.age < 0.5 {
                let f = a.age / 0.5;
                painter.circle_stroke(c, ra * (2.0 + 9.0 * f), Stroke::new(1.2 * z, alpha(Color32::WHITE, 0.8 * (1.0 - f))));
            }
        }
    }

    /// New notes: a line sweeping across, the name over it.
    fn sweep(&self, painter: &Painter, t: Tank, view: &View) {
        let age = ((self.now - view.chord_at) / SWEEP) as f32;
        if !(0.0..1.0).contains(&age) {
            return;
        }
        let z = self.z();
        let x = age * ASPECT;
        painter.rect_filled(Rect::from_min_max(t.at(x - 0.12, 0.0), t.at(x, 1.0)), 0.0, alpha(Color32::WHITE, 0.05 * (1.0 - age)));
        painter.vline(t.at(x, 0.0).x, t.rect.y_range(), Stroke::new(1.5 * z, alpha(Color32::WHITE, 0.7 * (1.0 - age))));
        let name = chord(self.p).name();
        let fade = (age * 6.0).min(1.0) * (1.0 - age).powf(1.3);
        let font = theme::strong(22.0 * z);
        if font.size < MIN_TEXT {
            return;
        }
        let galley = painter.layout_no_wrap(name, font, Color32::WHITE);
        let tag = Rect::from_center_size(t.at(ASPECT * 0.5, 0.5), galley.size() + vec2(18.0, 6.0) * z);
        painter.rect_filled(tag, 6.0 * z, alpha(INK, 0.8 * fade));
        painter.rect_stroke(tag, 6.0 * z, Stroke::new(1.5 * z, alpha(Color32::WHITE, 0.9 * fade)), StrokeKind::Inside);
        painter.galley(tag.center() - galley.size() * 0.5, galley, alpha(Color32::WHITE, fade));
    }

    /// The notes and what plays, over the tank.
    fn overlay(&self, painter: &Painter, t: Tank, mols: &[Mol], atoms: usize) {
        let z = self.z();
        let set = chord(self.p);
        let source = self.p.viz[viz::SOURCE].get().round() as i32;
        let at = t.rect.left_top() + vec2(10.0, 9.0) * z;
        let what = match source {
            1 => "root from the keys".to_owned(),
            2 => "chord from the keys".to_owned(),
            _ => shape_name(co::shape(self.p)).1.split(" (").next().unwrap_or("").to_owned(),
        };
        let shadow = alpha(Color32::BLACK, 0.7);
        for d in [vec2(1.0, 1.0), vec2(-1.0, 1.0), vec2(0.0, 1.5)] {
            label(painter, at + d * z, Align2::LEFT_TOP, set.name(), theme::strong(13.0 * z), shadow);
            label(painter, at + vec2(0.0, 18.0 * z) + d * z, Align2::LEFT_TOP, &what, theme::font(7.5 * z), shadow);
        }
        label(painter, at, Align2::LEFT_TOP, set.name(), theme::strong(13.0 * z), alpha(theme::TEXT, 0.95));
        label(painter, at + vec2(0.0, 18.0 * z), Align2::LEFT_TOP, &what, theme::font(7.5 * z), alpha(theme::TEXT_DIM, 0.9));
        // The notes' pitch classes, a chip each, coloured by role.
        let mut x = t.rect.right() - 10.0 * z;
        let tones: Vec<u8> = (0..12u8).map(|i| (set.root + i) % 12).filter(|&pc| set.contains(pc)).collect();
        for &pc in tones.iter().rev() {
            let font = theme::mono(8.5 * z);
            if font.size < MIN_TEXT {
                break;
            }
            let color = ROLE_COLORS[set.role(pc) as usize];
            let galley = painter.layout_no_wrap(NOTE_NAMES[pc as usize].to_owned(), font, color);
            let chip = Rect::from_min_size(pos2(x - galley.size().x - 8.0 * z, at.y), galley.size() + vec2(8.0, 2.0) * z);
            painter.rect_filled(chip, 3.0 * z, alpha(INK, 0.7));
            painter.rect_stroke(chip, 3.0 * z, Stroke::new(1.0 * z, color), StrokeKind::Inside);
            painter.galley(chip.center() - galley.size() * 0.5, galley, color);
            x = chip.left() - 4.0 * z;
        }
        let sounding = mols.iter().filter(|m| m.sounding).count();
        let stats = format!("{} molecules · {atoms} atoms · {sounding} ringing", mols.len());
        let at = t.rect.right_top() + vec2(-10.0, 25.0) * z;
        label(painter, at + vec2(1.0, 1.0) * z, Align2::RIGHT_TOP, &stats, theme::font(7.0 * z), shadow);
        label(painter, at, Align2::RIGHT_TOP, stats, theme::font(7.0 * z), alpha(theme::TEXT_DIM, 0.8));
        label(painter, t.rect.left_bottom() + vec2(10.0, -8.0) * z, Align2::LEFT_BOTTOM, "COLLOID", theme::strong(7.0 * z), alpha(theme::TEXT_FAINT, 0.6));
    }

    /// The notes: a cell per molecule, low to high, filling as it rings.
    fn strip(&self, view: &View, mols: &[Mol]) {
        let (v, z) = (self.v, self.z());
        let [y0, y1] = STRIP;
        let mut order: Vec<usize> = (0..mols.len()).collect();
        order.sort_by_key(|&k| mols[k].note);
        let (x0, x1) = (PAD, W - PAD);
        let cw = ((x1 - x0) / MAX_MOLECULES as f32).min((x1 - x0) / order.len().max(1) as f32);
        let start = x0 + ((x1 - x0) - cw * order.len() as f32) * 0.5;
        for (i, &k) in order.iter().enumerate() {
            let m = &mols[k];
            let rect = v.r(start + cw * i as f32 + 1.5, y0, start + cw * (i + 1) as f32 - 1.5, y1);
            let color = view.molecule_color(k, self.now);
            let flash = self.flash(view, k);
            v.painter.rect_filled(rect, 3.0 * z, theme::INSET);
            let level = m.ring.max(flash).clamp(0.0, 1.0);
            if level > 0.01 {
                let fill = Rect::from_min_max(pos2(rect.left(), rect.bottom() - rect.height() * level), rect.max);
                v.painter.rect_filled(fill, 3.0 * z, alpha(color, 0.3 + 0.4 * level));
            }
            v.painter.rect_filled(Rect::from_min_size(rect.min, vec2(rect.width(), 2.5 * z)), 1.0 * z, color);
            let ink = if m.sounding { theme::TEXT } else { theme::TEXT_DIM };
            label(v.painter, rect.center() + vec2(0.0, 1.0 * z), Align2::CENTER_CENTER, note_label(m.note as i32), v.mono(8.0), ink);
            let resp = v.ui.interact(rect, self.uid(("cell", k)), Sense::hover());
            resp.on_hover_text(format!("{} · {}", note_label(m.note as i32), ROLES[m.role as usize]));
        }
    }

    /// The knob groups. SPREAD and COLOR are for chords: dimmed while a
    /// handpan plays.
    fn knobs(&self) {
        let [top, bottom] = ROWS_Y;
        self.group("PARTICLES", PAD, 214.0, top, &PARTICLES);
        self.group("FIELD", 226.0, 486.0, top, &FIELD);
        self.group("LIQUID", 498.0, W - PAD, top, &LIQUID);
        self.group("HARMONY", PAD, 300.0, bottom, &HARMONY);
        self.group("NOTES", 312.0, W - PAD, bottom, &NOTES);
    }

    fn group_title(&self, title: &str, x0: f32, x1: f32, y: f32) {
        let (v, z) = (self.v, self.z());
        let r = v.text(x0, y, Align2::LEFT_CENTER, title, theme::strong(8.5 * z), theme::TEXT_DIM);
        let from = (r.right() + 8.0 * z).max(v.p(x0 + 50.0, 0.0).x);
        v.painter.hline(from..=v.p(x1, 0.0).x, v.p(0.0, y).y, Stroke::new(1.0 * z, alpha(theme::TEXT_FAINT, 0.3)));
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
        let spec = co::spec(i);
        let idle = matches!(i, SPREAD | COLOR) && self.handpan();
        let faint = |c: Color32| if idle { alpha(c, 0.45) } else { c };
        v.text(cx, y, Align2::CENTER_CENTER, label, v.font(7.5), faint(theme::TEXT_FAINT));
        let top = y + 8.0;
        let mut norm = spec.to_norm(p.get(i));
        let color = if idle { theme::TEXT_FAINT } else { self.accent };
        let resp = widgets::knob(v.ui, v.painter, self.uid(("knob", i)), v.p(cx, top + R), R * z, &mut norm, spec.to_norm(spec.default), spec.bipolar, color, z);
        let tip = TIPS.iter().find(|(k, _)| *k == i).map_or("", |(_, t)| *t);
        let resp = resp.on_hover_text(tip);
        v.hit(resp.rect, ParamRef::Module(i));
        if resp.dragged() || resp.double_clicked() {
            let value = spec.denorm(norm);
            let whole = matches!(spec.unit, Unit::Integer | Unit::NoteName | Unit::Channel);
            let value = if whole { value.round() } else { value };
            if value != p.get(i) {
                p.set(i, value);
            }
        }
        let hot = resp.dragged() || resp.hovered();
        v.text(cx, top + 2.0 * R + 8.0, Align2::CENTER_CENTER, format(p, i), v.mono(7.0), faint(if hot { theme::TEXT } else { theme::TEXT_DIM }));
    }

    /// The CC lanes: a card each, with its switch, controller, source,
    /// the value going out and its range.
    fn lanes(&self) {
        self.group_title("CC EVENTS", PAD, W - PAD, CC_Y);
        let gap = 8.0;
        let cw = (W - 2.0 * PAD - gap * (cc::LANES - 1) as f32) / cc::LANES as f32;
        for k in 0..cc::LANES {
            let x0 = PAD + k as f32 * (cw + gap);
            self.lane(k, x0, x0 + cw, CC_Y + 10.0);
        }
    }

    fn lane(&self, k: usize, x0: f32, x1: f32, y0: f32) {
        let (v, p, z) = (self.v, self.p, self.z());
        let at = |i: usize| p.data[cc::at(k, i)].get();
        let set = |i: usize, value: f32| p.data[cc::at(k, i)].set(value);
        let on = at(cc::ON) > 0.5;
        let number = at(cc::NUMBER).round().clamp(0.0, 127.0) as u8;
        let source = (at(cc::SOURCE).round().max(0.0) as usize).min(cc::SOURCES.len() - 1);
        let card = v.r(x0, y0, x1, y0 + 42.0);
        v.painter.rect_filled(card, 4.0 * z, if on { theme::CONTROL } else { theme::INSET });
        let ink = |c: Color32| if on { c } else { alpha(c, 0.55) };

        // The switch, and the controller (a menu).
        let switch = widgets::toggle(v.ui, v.painter, self.uid(("cc-on", k)), v.r(x0 + 4.0, y0 + 4.0, x0 + 32.0, y0 + 19.0), if on { "ON" } else { "OFF" }, on, self.accent, z)
            .on_hover_text("Send this lane's controller with the notes");
        if switch.clicked() {
            set(cc::ON, if on { 0.0 } else { 1.0 });
        }
        let title = match cc::name(number) {
            Some(name) => format!("CC {number} · {name} ▾"),
            None => format!("CC {number} ▾"),
        };
        let menu_rect = v.r(x0 + 36.0, y0 + 4.0, x1 - 4.0, y0 + 19.0);
        let menu = v.ui.interact(menu_rect, self.uid(("cc-number", k)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text("The controller this lane sends");
        v.painter.rect_filled(menu_rect, 3.0 * z, if menu.hovered() { theme::CONTROL_HI } else { theme::NODE_BODY });
        label(v.painter, menu_rect.left_center() + vec2(5.0 * z, 0.0), Align2::LEFT_CENTER, title, v.mono(7.5), ink(theme::TEXT));
        egui::Popup::menu(&menu).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.set_min_width(170.0);
            ui.weak("Controller");
            for (n, name) in cc::NAMED {
                if ui.selectable_label(n == number, format!("{n:>3}  {name}")).clicked() {
                    set(cc::NUMBER, n as f32);
                    ui.close();
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.weak("Any");
                let mut n = number as i32;
                if ui.add(egui::DragValue::new(&mut n).range(0..=127)).changed() {
                    set(cc::NUMBER, n as f32);
                }
            });
        });

        // The source (a menu), the value going out, its range.
        let src_rect = v.r(x0 + 4.0, y0 + 23.0, x0 + 70.0, y0 + 38.0);
        let src = v.ui.interact(src_rect, self.uid(("cc-source", k)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(cc::SOURCES[source].1);
        v.painter.rect_filled(src_rect, 3.0 * z, if src.hovered() { theme::CONTROL_HI } else { theme::NODE_BODY });
        label(v.painter, src_rect.center(), Align2::CENTER_CENTER, format!("{} ▾", cc::SOURCES[source].0), v.font(7.0), ink(self.accent));
        egui::Popup::menu(&src).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.set_min_width(260.0);
            ui.weak("What drives it");
            for (i, (name, what)) in cc::SOURCES.iter().enumerate() {
                if ui.selectable_label(i == source, *name).on_hover_text(*what).clicked() {
                    set(cc::SOURCE, i as f32);
                    ui.close();
                }
            }
        });
        let (low, high) = (at(cc::LOW), at(cc::HIGH));
        let meter = v.r(x0 + 76.0, y0 + 27.0, x1 - 62.0, y0 + 34.0);
        v.painter.rect_filled(meter, 2.0 * z, theme::INSET);
        let x_of = |value: f32| meter.left() + meter.width() * value / 127.0;
        let (a, b) = (x_of(low.min(high)), x_of(low.max(high)));
        v.painter.rect_filled(Rect::from_x_y_ranges(a..=b, meter.y_range()), 2.0 * z, alpha(self.accent, if on { 0.25 } else { 0.1 }));
        let sent = p.viz[viz::LANES + k].get();
        if on && sent >= 0.0 {
            let x = x_of(sent);
            v.painter.rect_filled(Rect::from_min_max(meter.min, pos2(x, meter.max.y)), 2.0 * z, self.accent);
            v.painter.vline(x, meter.y_range().expand(2.0 * z), Stroke::new(1.5 * z, theme::TEXT));
        }
        let shown = if on && sent >= 0.0 { format!("{}", sent as i32) } else { "—".to_owned() };
        label(v.painter, meter.center_top() - vec2(0.0, 1.0 * z), Align2::CENTER_BOTTOM, shown, v.mono(6.5), ink(theme::TEXT_DIM));
        self.number_box(k, cc::LOW, v.r(x1 - 58.0, y0 + 23.0, x1 - 32.0, y0 + 38.0), "Value sent at the source's lowest (drag; double-click: 0)", 0.0, on);
        self.number_box(k, cc::HIGH, v.r(x1 - 30.0, y0 + 23.0, x1 - 4.0, y0 + 38.0), "Value sent at the source's highest (drag; double-click: 127)", 127.0, on);
    }

    /// A value 0…127 of a lane: drag up or right to raise it, a step every
    /// few pixels.
    fn number_box(&self, k: usize, i: usize, rect: Rect, tip: &str, reset: f32, on: bool) {
        let (v, p, z) = (self.v, self.p, self.z());
        let slot = &p.data[cc::at(k, i)];
        let id = self.uid(("cc-box", k, i));
        let resp = v.ui.interact(rect, id, Sense::click_and_drag()).on_hover_cursor(CursorIcon::ResizeVertical).on_hover_text(tip);
        if resp.dragged() {
            let d = resp.drag_delta();
            let steps = v.ui.data_mut(|m| {
                let acc = m.get_temp_mut_or(id.with("acc"), 0.0f32);
                *acc += (d.x - d.y) / z;
                let steps = (*acc / 3.0).trunc();
                *acc -= steps * 3.0;
                steps
            });
            slot.set((slot.get() + steps).clamp(0.0, 127.0));
        }
        if resp.double_clicked() {
            slot.set(reset);
        }
        let hot = resp.hovered() || resp.dragged();
        v.painter.rect_filled(rect, 3.0 * z, if hot { theme::CONTROL_HI } else { theme::NODE_BODY });
        let ink = if on { theme::TEXT } else { theme::TEXT_FAINT };
        label(v.painter, rect.center(), Align2::CENTER_CENTER, format!("{}", slot.get().round() as i32), v.mono(7.5), ink);
    }
}

/// Text, unless it is too small to read (zoomed far out).
fn label(painter: &Painter, at: Pos2, align: Align2, text: impl ToString, font: egui::FontId, color: Color32) -> Rect {
    if font.size < MIN_TEXT {
        return Rect::from_center_size(at, Vec2::ZERO);
    }
    painter.text(at, align, text.to_string(), font, color)
}

/// A flat disc of `segments` sides.
fn disc(mesh: &mut Mesh, c: Pos2, r: f32, color: Color32, segments: u32) {
    if r < 0.3 || color.a() == 0 {
        return;
    }
    let base = mesh.vertices.len() as u32;
    mesh.colored_vertex(c, color);
    for i in 0..segments {
        mesh.colored_vertex(c + Vec2::angled(i as f32 / segments as f32 * TAU) * r, color);
    }
    for i in 0..segments {
        mesh.add_triangle(base, base + 1 + i, base + 1 + (i + 1) % segments);
    }
}

/// A circle of `dashes` dashes, turned by `turn` radians.
fn dashed_circle(painter: &Painter, c: Pos2, r: f32, dashes: usize, turn: f32, stroke: Stroke) {
    if r < 2.0 {
        return;
    }
    for i in 0..dashes {
        let a0 = turn + i as f32 / dashes as f32 * TAU;
        let a1 = a0 + TAU / dashes as f32 * 0.55;
        let pts: Vec<Pos2> = (0..=3).map(|s| c + Vec2::angled(a0 + (a1 - a0) * s as f32 / 3.0) * r).collect();
        painter.add(Shape::line(pts, stroke));
    }
}

/// A dashed line from `a` to `b`, the dashes moving along by `offset`.
fn dashed_line(painter: &Painter, a: Pos2, b: Pos2, dash: f32, gap: f32, offset: f32, stroke: Stroke) {
    let len = (b - a).length();
    if len < 1.0 {
        return;
    }
    let dir = (b - a) / len;
    let period = dash + gap;
    let mut s = offset.rem_euclid(period) - period;
    while s < len {
        let (s0, s1) = (s.max(0.0), (s + dash).min(len));
        if s1 > s0 {
            painter.line_segment([a + dir * s0, a + dir * s1], stroke);
        }
        s += period;
    }
}
