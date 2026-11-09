//! Colloid: a generative MIDI node played by physics. A handpan's layout or
//! a chord (a root and a shape, or the keys played on its MIDI input)
//! becomes a set of notes, one per molecule, floating in a liquid: low notes are big and heavy and
//! sink, high ones small and light. While the transport plays, atoms are
//! let into the tank: every molecule pulls them (ATTRACT, RANGE), the
//! liquid slows them (VISCOSITY), shakes them (HEAT) and carries them on
//! its currents (FLOW). An atom striking a molecule plays its note: the
//! harder the strike (the atom's speed and MASS), the louder the note and
//! the longer the molecule rings; the note lasts as long as it rings, so a
//! thick liquid makes short notes and big molecules ring longer. After a
//! strike the atom is charged and pushed off for a while (CHARGE), then
//! falls back: orbits, bounces and captures make the rhythm. QUANT can pull
//! the notes onto the board's grid.
//!
//! Up to four CC lanes send controllers along with the notes: a sustain
//! pedal that catches each new chord, controllers the keys send passed
//! through, or what the tank is doing (how hard it rings, how many notes,
//! how fast the atoms go, how they swirl…) on any controller number.
//!
//! On the MIDI input (its cable or, with none, the MIDI keyboard) a single
//! key moves the chord's root, two or more keys played together become the
//! chord itself; it stays until the panel lets go of it. Program Change
//! 1–21 picks the shape (the handpans, then the chords).

pub mod chords;
pub mod physics;

use std::sync::Arc;

use self::chords::{CHORDS, ChordSet, FIRST_CHORD, SHAPES, mask_of, voicing};
use self::physics::{MAX_ATOMS, MAX_MOLECULES, MAX_STRIKES, SILENT, Settings, Strike, Tank};
use super::automa::hash01;
use super::steps::{Held, block_end, frame_of, note_event};
use super::{ModuleDef, ModuleParams, Rng};
use crate::audio::engine::{Buf, Inputs, MidiBuf, ProcessCtx, Processor};
use crate::audio::midi::MidiState;
use crate::audio::params::{DIVISIONS, ParamSpec, Unit};

pub const MOLECULES: usize = 0;
pub const ATOMS: usize = 1;
pub const MASS: usize = 2;
pub const ATTRACT: usize = 3;
pub const RANGE: usize = 4;
pub const VISCOSITY: usize = 5;
pub const HEAT: usize = 6;
pub const FLOW: usize = 7;
pub const BOUNCE: usize = 8;
pub const CHARGE: usize = 9;
pub const DRIFT: usize = 10;
pub const ROOT: usize = 11;
pub const OCTAVE: usize = 12;
pub const SPREAD: usize = 13;
pub const COLOR: usize = 14;
pub const VELOCITY: usize = 15;
pub const DYNAMICS: usize = 16;
pub const LENGTH: usize = 17;
pub const QUANT: usize = 18;
pub const CHANNEL: usize = 19;

const PARAMS: [ParamSpec; 20] = [
    ParamSpec::lin("MOLECULES", 2.0, MAX_MOLECULES as f32, 9.0, Unit::Integer),
    ParamSpec::lin("ATOMS", 1.0, MAX_ATOMS as f32, 6.0, Unit::Integer),
    ParamSpec::lin("MASS", 0.0, 1.0, 0.4, Unit::Percent),
    ParamSpec::lin("ATTRACT", 0.0, 1.0, 0.4, Unit::Percent),
    ParamSpec::lin("RANGE", 0.0, 1.0, 0.5, Unit::Percent),
    ParamSpec::lin("VISCOSITY", 0.0, 1.0, 0.3, Unit::Percent),
    ParamSpec::lin("HEAT", 0.0, 1.0, 0.35, Unit::Percent),
    ParamSpec::lin("FLOW", 0.0, 1.0, 0.3, Unit::Percent),
    ParamSpec::lin("BOUNCE", 0.0, 1.0, 0.6, Unit::Percent),
    ParamSpec::lin("CHARGE", 0.0, 1.0, 0.5, Unit::Percent),
    ParamSpec::lin("DRIFT", 0.0, 1.0, 0.4, Unit::Percent),
    ParamSpec::lin("ROOT", 0.0, 11.0, 2.0, Unit::NoteName),
    ParamSpec::lin("OCTAVE", 1.0, 6.0, 3.0, Unit::Integer),
    ParamSpec::lin("SPREAD", 1.0, 4.0, 3.0, Unit::Integer),
    ParamSpec::lin("COLOR", 0.0, 1.0, 0.3, Unit::Percent),
    ParamSpec::lin("VELOCITY", 1.0, 127.0, 112.0, Unit::Integer),
    ParamSpec::lin("DYNAMICS", 0.0, 1.0, 0.75, Unit::Percent),
    ParamSpec::log("LENGTH", 0.03, 4.0, 0.7, Unit::Seconds),
    ParamSpec::lin("QUANT", 0.0, DIVISIONS.len() as f32, 0.0, Unit::Integer),
    ParamSpec::lin("CHANNEL", 1.0, 16.0, 1.0, Unit::Channel),
];

pub fn spec(i: usize) -> &'static ParamSpec {
    &PARAMS[i]
}

/// What `ModuleParams::data` holds besides the knobs.
pub mod data {
    /// The tank's start: RESEED picks a new one.
    pub const SEED: usize = 0;
    /// The CC lanes, [`super::cc::LEN`] values each.
    pub const LANES: usize = 1;
    pub const LEN: usize = LANES + super::cc::LANES * super::cc::LEN;
}

/// CC lanes: controllers sent along with the notes.
pub mod cc {
    pub const LANES: usize = 4;
    /// A lane's values in `data`: on, the controller's number, its source,
    /// and the values it sends for the source's lowest and highest.
    pub const ON: usize = 0;
    pub const NUMBER: usize = 1;
    pub const SOURCE: usize = 2;
    pub const LOW: usize = 3;
    pub const HIGH: usize = 4;
    pub const LEN: usize = 5;

    pub const PEDAL: usize = 0;
    pub const KEYS: usize = 1;
    pub const ENERGY: usize = 2;
    pub const DENSITY: usize = 3;
    pub const SPEED: usize = 4;
    pub const STRIKE: usize = 5;
    pub const SWIRL: usize = 6;
    pub const CHARGE: usize = 7;
    pub const SOURCES: [(&str, &str); 8] = [
        ("PEDAL", "A sustain pedal: down while the atoms play, lifted for a moment at each new chord so the old one doesn't blur into it"),
        ("KEYS", "The same controller from the MIDI input, passed through (your sustain pedal, mod wheel…)"),
        ("ENERGY", "How hard the molecules ring"),
        ("DENSITY", "How many notes play (up to 16 a second)"),
        ("SPEED", "How fast the atoms go"),
        ("STRIKE", "Each note's force, falling away after it"),
        ("SWIRL", "Which way and how fast the atoms circle the tank (the middle: not at all)"),
        ("CHARGE", "How charged the atoms are (they charge as they strike)"),
    ];

    /// Controllers with a common use, for the menu.
    pub const NAMED: [(u8, &str); 13] = [
        (1, "Mod wheel"),
        (2, "Breath"),
        (4, "Foot"),
        (7, "Volume"),
        (10, "Pan"),
        (11, "Expression"),
        (64, "Sustain"),
        (66, "Sostenuto"),
        (67, "Soft pedal"),
        (71, "Resonance"),
        (74, "Cutoff"),
        (91, "Reverb"),
        (93, "Chorus"),
    ];

    pub fn name(number: u8) -> Option<&'static str> {
        NAMED.iter().find(|(n, _)| *n == number).map(|(_, name)| *name)
    }

    /// Where lane `k`'s value `i` is in `data`.
    pub fn at(k: usize, i: usize) -> usize {
        super::data::LANES + k * LEN + i
    }
}

const DATA: [f32; data::LEN] = [
    1.0, // seed
    // Off, ready: sustain from the pedal, mod wheel from the energy,
    // expression from the density, cutoff from the speed.
    0.0, 64.0, 0.0, 0.0, 127.0, //
    0.0, 1.0, 2.0, 0.0, 127.0, //
    0.0, 11.0, 3.0, 40.0, 127.0, //
    0.0, 74.0, 4.0, 30.0, 110.0,
];

/// What the processor publishes for the panel (and what the panel hands it).
pub mod viz {
    use super::{MAX_ATOMS, MAX_MOLECULES};

    /// 1 while atoms are in the tank (playing).
    pub const LIVE: usize = 0;
    /// The tank's clock, in seconds (the currents follow it).
    pub const TIME: usize = 1;
    pub const MOLECULES: usize = 2;
    pub const ATOMS: usize = 3;
    /// The chord: root, pitch classes and added colour tones (as bits), and
    /// where it comes from (0 the panel, 1 a key's root, 2 keys played).
    pub const ROOT: usize = 4;
    pub const MASK: usize = 5;
    pub const COLOURS: usize = 6;
    pub const SOURCE: usize = 7;
    /// Notes played since the node was made.
    pub const NOTES: usize = 8;
    /// Per molecule: x, y, radius, angle, ring, note, role, sounding, spin.
    pub const MOLECULE: usize = 9;
    pub const MOLECULE_LEN: usize = 9;
    /// Per atom: x, y, vx, vy, charge, age.
    pub const ATOM: usize = MOLECULE + MAX_MOLECULES * MOLECULE_LEN;
    pub const ATOM_LEN: usize = 6;
    /// Strikes and notes so far, and the latest: per event its number (from
    /// 1), kind, molecule, x, y, value (a strike's force, a note's
    /// velocity / 127) and the atom (strikes) or note (notes).
    pub const EVENT_SEQ: usize = ATOM + MAX_ATOMS * ATOM_LEN;
    pub const EVENTS: usize = EVENT_SEQ + 1;
    pub const EVENT_CAP: usize = 64;
    pub const EVENT_LEN: usize = 7;
    /// The panel bumps this to let go of the chord from the keys.
    pub const UNLATCH: usize = EVENTS + EVENT_CAP * EVENT_LEN;
    /// Written by the panel: the liquid stirred at x, y with a velocity
    /// (0, 0: a push outwards), and a count it bumps each time.
    pub const STIR: usize = UNLATCH + 1;
    pub const STIR_SEQ: usize = STIR + 4;
    /// The handpan playing (its index), or −1 for a chord.
    pub const LAYOUT: usize = STIR_SEQ + 1;
    /// Each CC lane's last value sent (−1: none).
    pub const LANES: usize = LAYOUT + 1;
    pub const LEN: usize = LANES + super::cc::LANES;
}

/// Event kinds.
pub const STRIKE: u8 = 0;
pub const NOTE: u8 = 1;

pub static DEF: ModuleDef = ModuleDef {
    name: "Colloid",
    prefix: "COLLOID",
    hint: "atoms strike chord molecules → notes",
    inputs: &[],
    outputs: &[],
    params: &PARAMS,
    cv_depth: &[],
    modes: &[
        "KURD", "CELTIC", "AMARA", "INTEGRAL", "HIJAZ", "PYGMY", "AKEBONO", "SABYE", "AEGEAN", "MAJ", "MIN", "SUS2", "SUS4", "MAJ7", "MIN7", "DOM7", "MAJ9",
        "MIN9", "6/9", "QUART", "PENTA",
    ],
    toggle: Some("PLAY"),
    toggle_default: true,
    map: false,
    midi_in: true,
    midi_out: true,
    data: &DATA,
    display_h: 0.0,
    controls_h: 0.0,
    viz: viz::LEN,
};

/// Shortest time between two notes of one molecule, in seconds.
const COOL: f32 = 0.045;
/// Shortest note, in seconds.
const MIN_HELD: f32 = 0.03;
/// Seconds after the last key comes down before the chord is taken.
const SETTLE: f32 = 0.03;
/// Colour tones offered round a chord played on the keys.
const KEY_COLOURS: [u8; 5] = [2, 9, 5, 6, 11];

/// A knob's value as a whole number within `lo..=hi`.
pub fn int(p: &ModuleParams, i: usize, lo: i64, hi: i64) -> i64 {
    (p.get(i).round() as i64).clamp(lo, hi)
}

pub fn seed(p: &ModuleParams) -> u32 {
    p.data[data::SEED].get().max(0.0) as u32
}

/// A new start: new places for the molecules, new atoms.
pub fn reseed(p: &ModuleParams, seed: u32) {
    p.data[data::SEED].set(((seed % (1 << 24)).max(1)) as f32);
}

/// The shape picked: a handpan (below [`FIRST_CHORD`]) or a chord.
pub fn shape(p: &ModuleParams) -> usize {
    p.mode().min(SHAPES - 1)
}

pub fn channel(p: &ModuleParams) -> u8 {
    int(p, CHANNEL, 1, 16) as u8 - 1
}

/// The physics, from the knobs.
pub fn settings(p: &ModuleParams) -> Settings {
    Settings::new(
        p.get(ATTRACT),
        p.get(RANGE),
        p.get(VISCOSITY),
        p.get(HEAT),
        p.get(FLOW),
        p.get(BOUNCE),
        p.get(CHARGE),
        p.get(DRIFT),
        p.get(MASS),
        p.get(LENGTH),
    )
}

/// The lowest note (the root in OCTAVE) and the register's top.
pub fn register(p: &ModuleParams, root: u8) -> (i32, i32) {
    let low = 12 * (int(p, OCTAVE, 1, 6) as i32 + 1) + root as i32;
    (low, low + 12 * int(p, SPREAD, 1, 4) as i32)
}

/// The velocity of a strike of `force` (0…1).
pub fn velocity(p: &ModuleParams, force: f32) -> u8 {
    let dynamics = p.get(DYNAMICS).clamp(0.0, 1.0);
    let level = 1.0 - dynamics + dynamics * force.clamp(0.0, 1.0).powf(0.8);
    (p.get(VELOCITY) * level).round().clamp(1.0, 127.0) as u8
}

/// Colour tones from `offered` (semitones above `root`) that COLOR lets in
/// (in an order the seed picks), leaving out any a half step above a chord
/// tone: they would clash.
fn colours(root: u8, mask: u16, offered: &[u8], color: f32, seed: u32) -> u16 {
    let mut bits = 0;
    for (k, &i) in offered.iter().enumerate() {
        let pc = (root + i) % 12;
        let below = (pc + 11) % 12;
        if mask >> pc & 1 == 0 && mask >> below & 1 == 0 && hash01(seed, k as u64, 0xC0_1010) < color {
            bits |= 1 << pc;
        }
    }
    bits
}

/// A CC lane's state.
#[derive(Clone, Copy, Default)]
struct Lane {
    /// The controller and value last sent.
    number: u8,
    sent: Option<u8>,
    /// The source, smoothed, and seconds since the last value went out.
    level: f32,
    since: f32,
    /// PEDAL: down, and seconds until it goes down again after a lift.
    down: bool,
    relift: Option<f32>,
    /// KEYS: the latest value from the input.
    keys: Option<u8>,
}

/// What the tank is doing, for the CC lanes, each 0…1.
fn measures(tank: &Tank) -> [f32; cc::SOURCES.len()] {
    let mols = &tank.molecules[..tank.molecule_count];
    let atoms = &tank.atoms[..tank.atom_count];
    let n = atoms.len().max(1) as f32;
    let energy = (mols.iter().map(|m| m.ring).sum::<f32>() / (0.3 * mols.len().max(1) as f32)).min(1.0);
    let speed = (atoms.iter().map(|a| (a.vx * a.vx + a.vy * a.vy).sqrt()).sum::<f32>() / n / 1.6).min(1.0);
    let (cx, cy) = (physics::ASPECT * 0.5, 0.5);
    let swirl = atoms.iter().map(|a| (a.x - cx) * a.vy - (a.y - cy) * a.vx).sum::<f32>() / n / 0.5;
    let charge = atoms.iter().map(|a| a.charge).sum::<f32>() / n;
    let mut out = [0.0; cc::SOURCES.len()];
    out[cc::ENERGY] = energy;
    out[cc::SPEED] = speed;
    out[cc::SWIRL] = (0.5 + 0.5 * swirl).clamp(0.0, 1.0);
    out[cc::CHARGE] = charge.clamp(0.0, 1.0);
    out
}

/// A molecule's note: sounding or not, and what the grid holds back.
#[derive(Clone, Copy, Default)]
struct Voice {
    /// The note sounding and its channel.
    note: Option<(u8, u8)>,
    /// Seconds it has sounded.
    held: f32,
    /// Seconds until the molecule may play again.
    cool: f32,
    /// A strike waiting for the grid: its beat and force.
    pending: Option<(f64, f32)>,
}

pub struct Colloid {
    params: Arc<ModuleParams>,
    midi: Arc<MidiState>,
    cursor: u32,
    cable: bool,
    out: MidiBuf,
    tank: Tank,
    rng: Rng,
    seed: u32,
    sample_rate: f32,
    voices: [Voice; MAX_MOLECULES],
    /// Atoms are in the tank (playing).
    live: bool,
    /// Keys held, and every key pressed since the hand came down.
    held: Held,
    gesture: Held,
    /// Seconds since the last key came down, until the chord is taken.
    settle: Option<f32>,
    /// From the keys: a root for the panel's shape, or a whole chord.
    key_root: Option<u8>,
    key_set: Option<(u8, u16)>,
    unlatch_seen: f32,
    stir_seen: f32,
    /// What the notes were made from, to make them again only on a change.
    tuned: Option<(ChordSet, i32, i32, usize)>,
    events: u32,
    notes: u32,
    lanes: [Lane; cc::LANES],
    /// Notes a second (smoothed), the last note's force (falling away), and
    /// notes this block.
    density: f32,
    strike: f32,
    played: u32,
    /// The notes the pedal last caught.
    pedalled: Option<ChordSet>,
}

impl Colloid {
    pub fn new(params: Arc<ModuleParams>, midi: Arc<MidiState>, sample_rate: f32) -> Self {
        let seed = seed(&params);
        let mut colloid = Self {
            cursor: midi.stream_head(),
            midi,
            cable: false,
            out: MidiBuf::default(),
            tank: Tank::new(),
            rng: Rng::new(1),
            seed,
            sample_rate,
            voices: [Voice::default(); MAX_MOLECULES],
            live: false,
            held: Held::new(),
            gesture: Held::new(),
            settle: None,
            key_root: None,
            key_set: None,
            unlatch_seen: params.viz[viz::UNLATCH].get(),
            stir_seen: params.viz[viz::STIR_SEQ].get(),
            tuned: None,
            events: 0,
            notes: 0,
            lanes: [Lane::default(); cc::LANES],
            density: 0.0,
            strike: 0.0,
            played: 0,
            pedalled: None,
            params,
        };
        let p = colloid.params.clone();
        colloid.reset_rng();
        let set = colloid.chord(&p);
        colloid.retune(&p, set, 0);
        colloid.tank.scatter(&mut colloid.rng);
        for m in &mut colloid.tank.molecules {
            m.r = m.size;
        }
        colloid.publish(&p, set);
        colloid
    }

    fn reset_rng(&mut self) {
        self.rng = Rng::new(self.seed.wrapping_mul(0x9E37_79B9) ^ 0xC011_0D00);
    }

    /// The chord now, and where it comes from.
    fn chord(&self, p: &ModuleParams) -> ChordSet {
        let color = p.get(COLOR).clamp(0.0, 1.0);
        let shape = shape(p);
        let (root, mask, offered) = match (self.key_set, self.key_root) {
            (Some((root, mask)), _) => (root, mask, &KEY_COLOURS[..]),
            (None, root) => {
                let root = root.unwrap_or(int(p, ROOT, 0, 11) as u8);
                if shape < FIRST_CHORD {
                    return ChordSet::handpan(root, shape);
                }
                let (_, _, tones, offered) = CHORDS[shape - FIRST_CHORD];
                (root, mask_of(root, tones), offered)
            }
        };
        ChordSet { root, mask, colours: colours(root, mask, offered, color, self.seed), layout: None }
    }

    fn source(&self) -> u8 {
        if self.key_set.is_some() {
            2
        } else {
            self.key_root.is_some() as u8
        }
    }

    /// Ends molecule `k`'s note.
    fn end(&mut self, k: usize, frame: usize) {
        if let Some((note, channel)) = self.voices[k].note.take() {
            self.out.push(frame, [0x80 | channel, note, 0]);
        }
    }

    /// Ends every note and forgets what the grid held back.
    fn silence(&mut self, frame: usize) {
        for k in 0..MAX_MOLECULES {
            self.end(k, frame);
            self.voices[k].pending = None;
        }
    }

    /// Gives the molecules the chord's notes (when they or the chord
    /// changed): those there were take them in order of their old notes, so
    /// each moves as little as it can; new ones condense, gone ones end.
    fn retune(&mut self, p: &ModuleParams, set: ChordSet, frame: usize) {
        let count = int(p, MOLECULES, 2, MAX_MOLECULES as i64) as usize;
        let (low, top) = register(p, set.root);
        let octaves = (top - low) / 12;
        let key = (set, low, octaves, count);
        if self.tuned == Some(key) {
            return;
        }
        self.tuned = Some(key);
        let mut notes = [0u8; MAX_MOLECULES];
        let len = voicing(&set, low, octaves, count, &mut notes);
        let old = self.tank.molecule_count;
        for k in len..old {
            self.end(k, frame);
            self.voices[k].pending = None;
        }
        let mut order: [usize; MAX_MOLECULES] = std::array::from_fn(|k| k);
        let rank = |k: usize| if k < old { self.tank.molecules[k].note as usize } else { 256 + k };
        order[..len].sort_by_key(|&k| rank(k));
        let highest = notes[..len].last().copied().unwrap_or(0) as i32;
        let top = if set.layout.is_some() { highest } else { highest.max(top) };
        let span = (top - low).max(1) as f32;
        for (rank, &k) in order[..len].iter().enumerate() {
            let note = notes[rank];
            let t = (note as i32 - low) as f32 / span;
            self.tank.tune(k, note, set.role(note % 12), t, len);
            if k >= old {
                self.tank.condense(k, &mut self.rng);
            }
        }
        self.tank.molecule_count = len;
    }

    /// A key on the MIDI input.
    fn key(&mut self, p: &ModuleParams, msg: [u8; 3]) {
        match note_event(msg) {
            Some(true) => {
                if self.held.is_empty() {
                    self.gesture.clear();
                }
                self.held.press(msg[1]);
                self.gesture.press(msg[1]);
                self.settle = Some(0.0);
            }
            Some(false) => self.held.release(msg[1]),
            None if msg[0] & 0xF0 == 0xB0 => {
                for (k, lane) in self.lanes.iter_mut().enumerate() {
                    let at = |i: usize| p.data[cc::at(k, i)].get();
                    if at(cc::ON) > 0.5 && at(cc::SOURCE).round() as usize == cc::KEYS && at(cc::NUMBER).round() as u8 == msg[1] {
                        lane.keys = Some(msg[2]);
                    }
                }
            }
            None if msg[0] & 0xF0 == 0xC0 => p.set_mode(msg[1] as usize % SHAPES),
            None => {}
        }
    }

    /// The keys played together become the chord (one pitch class: its root).
    fn take_chord(&mut self) {
        let keys = self.gesture.notes();
        let Some(&lowest) = keys.iter().min() else { return };
        let mask = keys.iter().fold(0u16, |m, &n| m | 1 << (n % 12));
        if mask.count_ones() == 1 {
            self.key_root = Some(lowest % 12);
            self.key_set = None;
        } else {
            self.key_set = Some((lowest % 12, mask));
            self.key_root = None;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn event(&mut self, p: &ModuleParams, kind: u8, molecule: usize, x: f32, y: f32, value: f32, extra: f32) {
        let at = viz::EVENTS + (self.events as usize % viz::EVENT_CAP) * viz::EVENT_LEN;
        self.events += 1;
        p.viz[at].set(-1.0);
        for (k, v) in [kind as f32, molecule as f32, x, y, value, extra].into_iter().enumerate() {
            p.viz[at + 1 + k].set(v);
        }
        p.viz[at].set(self.events as f32);
        p.viz[viz::EVENT_SEQ].set(self.events as f32);
    }

    /// Molecule `k` plays its note at `frame`, as hard as `force`.
    fn play(&mut self, p: &ModuleParams, k: usize, force: f32, frame: usize) {
        let m = self.tank.molecules[k];
        let channel = channel(p);
        self.end(k, frame);
        // The same note on another molecule (it was retuned) gives way.
        for other in 0..MAX_MOLECULES {
            if self.voices[other].note.is_some_and(|(n, c)| n == m.note && c == channel) {
                self.end(other, frame);
            }
        }
        let vel = velocity(p, force);
        self.out.push(frame, [0x90 | channel, m.note, vel]);
        let voice = &mut self.voices[k];
        voice.note = Some((m.note, channel));
        voice.held = 0.0;
        voice.cool = COOL;
        self.tank.molecules[k].ring = self.tank.molecules[k].ring.max(force);
        self.notes += 1;
        self.played += 1;
        self.strike = self.strike.max(force);
        self.event(p, NOTE, k, m.x, m.y, vel as f32 / 127.0, m.note as f32);
    }

    fn strike(&mut self, p: &ModuleParams, ctx: &ProcessCtx, strike: &Strike) {
        self.event(p, STRIKE, strike.molecule, strike.x, strike.y, strike.force, strike.atom as f32);
        if !self.live {
            return;
        }
        let k = strike.molecule;
        match int(p, QUANT, 0, DIVISIONS.len() as i64) as usize {
            0 => {
                if self.voices[k].cool <= 0.0 {
                    self.play(p, k, strike.force, 0);
                }
            }
            q => {
                let per_beat = DIVISIONS[q - 1].1;
                let beat = ((ctx.beat * per_beat) - 1e-9).ceil() / per_beat;
                let voice = &mut self.voices[k];
                voice.pending = Some(match voice.pending {
                    Some((at, force)) => (at, force.max(strike.force)),
                    None => (beat, strike.force),
                });
            }
        }
    }

    /// The CC lanes, after this block's notes.
    fn send_lanes(&mut self, p: &ModuleParams, set: &ChordSet, dt: f32) {
        // What the tank is doing.
        let rate = self.played as f32 / dt.max(1e-6);
        self.played = 0;
        self.density += (rate - self.density) * (1.0 - (-dt / 1.0).exp());
        self.strike *= (-dt / 0.25).exp();
        let mut measure = measures(&self.tank);
        measure[cc::DENSITY] = (self.density / 16.0).clamp(0.0, 1.0);
        measure[cc::STRIKE] = self.strike.clamp(0.0, 1.0);
        // A new chord lifts the pedal for a moment.
        let changed = self.pedalled.is_some_and(|c| (c.root, c.mask, c.layout) != (set.root, set.mask, set.layout));
        self.pedalled = Some(*set);
        let channel = channel(p);
        for k in 0..cc::LANES {
            let at = |i: usize| p.data[cc::at(k, i)].get();
            let on = at(cc::ON) > 0.5;
            let number = at(cc::NUMBER).round().clamp(0.0, 127.0) as u8;
            let source = (at(cc::SOURCE).round().max(0.0) as usize).min(cc::SOURCES.len() - 1);
            let (low, high) = (at(cc::LOW).clamp(0.0, 127.0), at(cc::HIGH).clamp(0.0, 127.0));
            let value = |v: f32| (low + (high - low) * v.clamp(0.0, 1.0)).round() as u8;
            let lane = &mut self.lanes[k];
            lane.since += dt;
            // A pedal let go when its lane goes off, moves to another
            // controller or stops being a pedal.
            if lane.down && (!on || number != lane.number || source != cc::PEDAL) {
                self.out.push(0, [0xB0 | channel, lane.number, value(0.0)]);
                lane.down = false;
                lane.sent = Some(value(0.0));
            }
            if lane.number != number {
                lane.number = number;
                lane.sent = None;
            }
            if !on {
                lane.sent = None;
                lane.keys = None;
                lane.relift = None;
                continue;
            }
            let out = match source {
                cc::PEDAL => {
                    if changed && lane.down {
                        lane.relift = Some(0.09);
                    }
                    if let Some(t) = lane.relift {
                        lane.relift = (t - dt > 0.0).then_some(t - dt);
                    }
                    let down = self.live && lane.relift.is_none();
                    (down != lane.down || lane.sent.is_none() && down).then(|| {
                        lane.down = down;
                        value(if down { 1.0 } else { 0.0 })
                    })
                }
                cc::KEYS => lane.keys.take().map(|v| value(v as f32 / 127.0)),
                _ => {
                    lane.level += (measure[source] - lane.level) * (1.0 - (-dt / 0.06).exp());
                    let v = value(lane.level);
                    (self.live && lane.sent != Some(v) && lane.since >= 0.01).then_some(v)
                }
            };
            if let Some(v) = out {
                self.out.push(0, [0xB0 | channel, number, v]);
                lane.sent = Some(v);
                lane.since = 0.0;
            }
        }
    }

    fn publish(&self, p: &ModuleParams, set: ChordSet) {
        for (k, lane) in self.lanes.iter().enumerate() {
            p.viz[viz::LANES + k].set(lane.sent.map_or(-1.0, f32::from));
        }
        let t = &self.tank;
        p.viz[viz::LIVE].set(if self.live { 1.0 } else { 0.0 });
        p.viz[viz::TIME].set(t.time);
        p.viz[viz::MOLECULES].set(t.molecule_count as f32);
        p.viz[viz::ATOMS].set(t.atom_count as f32);
        p.viz[viz::ROOT].set(set.root as f32);
        p.viz[viz::MASK].set(set.mask as f32);
        p.viz[viz::COLOURS].set(set.colours as f32);
        p.viz[viz::SOURCE].set(self.source() as f32);
        p.viz[viz::LAYOUT].set(set.layout.map_or(-1.0, f32::from));
        p.viz[viz::NOTES].set(self.notes as f32);
        for (k, m) in t.molecules[..t.molecule_count].iter().enumerate() {
            let at = viz::MOLECULE + k * viz::MOLECULE_LEN;
            let sounding = if self.voices[k].note.is_some() { 1.0 } else { 0.0 };
            for (i, v) in [m.x, m.y, m.r, m.angle, m.ring, m.note as f32, m.role as f32, sounding, m.spin].into_iter().enumerate() {
                p.viz[at + i].set(v);
            }
        }
        for (k, a) in t.atoms[..t.atom_count].iter().enumerate() {
            let at = viz::ATOM + k * viz::ATOM_LEN;
            for (i, v) in [a.x, a.y, a.vx, a.vy, a.charge, a.age].into_iter().enumerate() {
                p.viz[at + i].set(v);
            }
        }
    }
}

impl Processor for Colloid {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, _outputs: &mut [Buf], n: usize) {
        self.out.clear();
        let p = self.params.clone();
        let dt = n as f32 / self.sample_rate;

        // The MIDI input: its cable, or else the keyboard.
        if inputs.midi_connected != self.cable {
            self.cable = inputs.midi_connected;
            self.held.clear();
        }
        if self.cable {
            self.cursor = self.midi.stream_head();
            for e in inputs.midi.events() {
                self.key(&p, e.data);
            }
        } else {
            let midi = self.midi.clone();
            let mut cursor = self.cursor;
            midi.read_stream(&mut cursor, |msg| self.key(&p, msg));
            self.cursor = cursor;
        }
        if let Some(t) = self.settle {
            if t + dt >= SETTLE {
                self.settle = None;
                self.take_chord();
            } else {
                self.settle = Some(t + dt);
            }
        }
        let unlatch = p.viz[viz::UNLATCH].get();
        if unlatch != self.unlatch_seen {
            self.unlatch_seen = unlatch;
            self.key_root = None;
            self.key_set = None;
        }
        let stir = p.viz[viz::STIR_SEQ].get();
        if stir != self.stir_seen {
            self.stir_seen = stir;
            let at = |i: usize| p.viz[viz::STIR + i].get();
            self.tank.stir(at(0), at(1), at(2), at(3));
        }

        // RESEED: the molecules scattered anew, new atoms.
        let seed = seed(&p);
        if seed != self.seed {
            self.seed = seed;
            self.reset_rng();
            self.tank.scatter(&mut self.rng);
            self.tank.atom_count = 0;
        }
        // Rewound: the atoms start again from the seed.
        if ctx.restart {
            self.silence(0);
            self.reset_rng();
            self.tank.atom_count = 0;
            self.live = false;
        }

        let set = self.chord(&p);
        self.retune(&p, set, 0);

        let live = p.toggled() && ctx.playing;
        if self.live && !live {
            self.silence(0);
            self.tank.atom_count = 0;
        }
        self.live = live;
        if live {
            let want = int(&p, ATOMS, 1, MAX_ATOMS as i64) as usize;
            while self.tank.atom_count < want {
                self.tank.spawn(&mut self.rng);
            }
            self.tank.atom_count = self.tank.atom_count.min(want);
        }

        // Rings that died away end their notes.
        for k in 0..MAX_MOLECULES {
            let voice = &mut self.voices[k];
            voice.cool = (voice.cool - dt).max(0.0);
            if voice.note.is_some() {
                voice.held += dt;
                let ring = if k < self.tank.molecule_count { self.tank.molecules[k].ring } else { 0.0 };
                if ring < SILENT && voice.held >= MIN_HELD && voice.pending.is_none() {
                    self.end(k, 0);
                }
            }
        }

        let s = settings(&p);
        let mut strikes = [Strike::default(); MAX_STRIKES];
        let count = self.tank.step(&s, dt, &mut self.rng, &mut strikes);
        for strike in &strikes[..count] {
            self.strike(&p, ctx, strike);
        }
        // Strikes the grid held back, on their step.
        if live {
            let end = block_end(ctx, n);
            for k in 0..self.tank.molecule_count {
                if let Some((beat, force)) = self.voices[k].pending
                    && beat < end
                {
                    self.voices[k].pending = None;
                    self.play(&p, k, force, frame_of(ctx, n, beat));
                }
            }
        }
        self.send_lanes(&p, &set, dt);
        self.publish(&p, set);
    }

    fn midi_out(&self) -> Option<&MidiBuf> {
        Some(&self.out)
    }
}
