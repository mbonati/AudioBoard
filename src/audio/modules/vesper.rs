//! Vesper: a generative ambient instrument, for soundscapes, cinematic pads
//! and chord progressions. Pick a ROOT and one of sixteen moods; the engine
//! writes the chords ([`harmony`]) and five layers play them: Pad, Bass, Noise
//! and Ambience, forty sounds each, and Shimmer, small high pings over the
//! top. The sounds have no sample files behind them: each recipe ([`sounds`])
//! is rendered into a looped stereo sample by a thread of the node's own
//! ([`library`]), so that the audio thread does no more than play loops
//! ([`layer`]).
//!
//! LOOP repeats a progression of CHORDS chords; EVOLVE writes new chords for
//! as long as it plays. In both, VARIATION is how many of the scale's seven
//! degrees chords may stand on, COMPLEXITY how many notes each has, DURATION
//! how long each lasts, on the board's bars. GENERATE writes again with the
//! same sounds; SHUFFLE picks new sounds, but not on the layers LOCK keeps.
//!
//! Each layer has an envelope, two filters that move on their own, drive,
//! chorus, and sends to an echo and a long space that all share ([`fx`]); a
//! MixType then colours the whole.
//!
//! With PLAY on it plays with the transport, and sends its score on the MIDI
//! output (chord on channel 1, bass on 2, pings on 3). With PLAY off, the keys
//! on the MIDI input play it instead.
//!
//! An independent instrument in the spirit of generative ambient plugins such
//! as Rast Sound's Cinescapes PRO: same way of working, its own engine, sounds
//! and names.

pub mod fx;
pub mod harmony;
pub mod layer;
pub mod library;
pub mod sounds;

use std::sync::Arc;

use self::fx::{Buses, HIGH, LOW, MIX_TYPES, Master, Strip, StripSettings, Sweep};
use self::harmony::{MAX_CHORDS, MAX_TONES, MOODS, Rules, Step};
use self::layer::{Pings, Sampler, Shape};
use self::library::Library;
use self::sounds::{PINGS as PING_TIMBRES, recipes};
use super::aether::space::Space;
use super::larsen::echo::{self, Echo, Shape as EchoShape};
use super::steps::{Held, frame_of, note_event};
use super::{ModuleDef, ModuleParams, Quiet, Rng, silence, soft_limit};
use crate::audio::engine::{BLOCK, Buf, Inputs, MidiBuf, ProcessCtx, Processor};
use crate::audio::midi::MidiState;
use crate::audio::params::{ParamSpec, Unit, sync_division};

/// The layers.
pub const PAD: usize = 0;
pub const BASS: usize = 1;
pub const NOISE: usize = 2;
pub const AMBIENCE: usize = 3;
pub const SHIMMER: usize = 4;
pub const LAYERS: usize = 5;
/// The layers that play rendered loops (all but Shimmer).
pub const SAMPLED: usize = 4;
pub const LAYER_NAMES: [&str; LAYERS] = ["PAD", "BASS", "NOISE", "AMBIENCE", "SHIMMER"];

// The node's own knobs.
pub const ROOT: usize = 0;
pub const MOOD: usize = 1;
pub const VARIATION: usize = 2;
pub const COMPLEXITY: usize = 3;
pub const DURATION: usize = 4;
pub const CHORDS: usize = 5;
pub const MIXTYPE: usize = 6;
pub const SPACE: usize = 7;
pub const ECHO: usize = 8;
pub const REPEATS: usize = 9;
pub const RATE: usize = 10;
pub const LEVEL: usize = 11;
pub const PULSE: usize = 12;
/// Then a block of knobs per layer.
const FIRST: usize = 13;
const STRIDE: usize = 17;
const COUNT: usize = FIRST + LAYERS * STRIDE;

/// A layer's knobs: see [`knob`].
pub mod k {
    pub const LEVEL: usize = 0;
    pub const OCTAVE: usize = 1;
    pub const ATTACK: usize = 2;
    pub const RELEASE: usize = 3;
    pub const WIDTH: usize = 4;
    pub const FREQ1: usize = 5;
    pub const MOVE1: usize = 6;
    pub const RATE1: usize = 7;
    pub const FREQ2: usize = 8;
    pub const MOVE2: usize = 9;
    pub const RATE2: usize = 10;
    pub const DRIVE: usize = 11;
    pub const CHORUS: usize = 12;
    pub const ECHO: usize = 13;
    pub const REVERB: usize = 14;
    pub const HOLD: usize = 15;
    pub const BLOOM: usize = 16;
}

/// Where one of a layer's knobs is.
pub const fn knob(layer: usize, which: usize) -> usize {
    FIRST + layer * STRIDE + which
}

/// HOLD at its top: a note holds until it's over.
const ENDLESS: f32 = 60.0;

macro_rules! strip {
    ($n:literal, $level:expr, $oct:expr, $attack:expr, $release:expr, $width:expr, $f1:expr, $m1:expr, $r1:expr, $f2:expr, $m2:expr, $r2:expr, $drive:expr, $chorus:expr, $echo:expr, $verb:expr) => {
        [
            ParamSpec::lin(concat!($n, " LEVEL"), 0.0, 1.0, $level, Unit::Decibel),
            ParamSpec::lin(concat!($n, " OCTAVE"), -2.0, 2.0, $oct, Unit::Integer).bipolar(),
            ParamSpec::log(concat!($n, " ATTACK"), 0.005, 20.0, $attack, Unit::Seconds),
            ParamSpec::log(concat!($n, " RELEASE"), 0.02, 30.0, $release, Unit::Seconds),
            ParamSpec::lin(concat!($n, " WIDTH"), 0.0, 1.0, $width, Unit::Percent),
            ParamSpec::log(concat!($n, " FILTER 1"), 20.0, 20_000.0, $f1, Unit::Hz),
            ParamSpec::lin(concat!($n, " MOTION 1"), 0.0, 1.0, $m1, Unit::Percent),
            ParamSpec::log(concat!($n, " RATE 1"), 0.005, 4.0, $r1, Unit::Hz),
            ParamSpec::log(concat!($n, " FILTER 2"), 20.0, 20_000.0, $f2, Unit::Hz),
            ParamSpec::lin(concat!($n, " MOTION 2"), 0.0, 1.0, $m2, Unit::Percent),
            ParamSpec::log(concat!($n, " RATE 2"), 0.005, 4.0, $r2, Unit::Hz),
            ParamSpec::lin(concat!($n, " DRIVE"), 0.0, 1.0, $drive, Unit::Percent),
            ParamSpec::lin(concat!($n, " CHORUS"), 0.0, 1.0, $chorus, Unit::Percent),
            ParamSpec::lin(concat!($n, " ECHO"), 0.0, 1.0, $echo, Unit::Percent),
            ParamSpec::lin(concat!($n, " REVERB"), 0.0, 1.0, $verb, Unit::Percent),
            ParamSpec::log(concat!($n, " HOLD"), 0.05, ENDLESS, ENDLESS, Unit::Seconds),
            ParamSpec::lin(concat!($n, " BLOOM"), 0.0, 1.0, 0.0, Unit::Percent),
        ]
    };
}

const fn build_params() -> [ParamSpec; COUNT] {
    let mut out = [ParamSpec::lin("", 0.0, 1.0, 0.0, Unit::Percent); COUNT];
    out[ROOT] = ParamSpec::lin("ROOT", 0.0, 11.0, 9.0, Unit::NoteName);
    out[MOOD] = ParamSpec::lin("MOOD", 0.0, 15.0, 4.0, Unit::Integer);
    out[VARIATION] = ParamSpec::lin("VARIATION", 1.0, 7.0, 5.0, Unit::Integer);
    out[COMPLEXITY] = ParamSpec::lin("COMPLEXITY", 2.0, 4.0, 3.0, Unit::Integer);
    out[DURATION] = ParamSpec::lin("DURATION", 0.0, 8.0, 4.0, Unit::Integer);
    out[CHORDS] = ParamSpec::lin("CHORDS", 2.0, 8.0, 4.0, Unit::Integer);
    out[MIXTYPE] = ParamSpec::lin("MIXTYPE", 0.0, 3.0, 0.0, Unit::Integer);
    out[SPACE] = ParamSpec::lin("SPACE", 0.0, 1.0, 0.7, Unit::Percent);
    out[ECHO] = ParamSpec::lin("ECHO TIME", 0.0, 13.0, 10.0, Unit::Sync);
    out[REPEATS] = ParamSpec::lin("REPEATS", 0.0, 0.95, 0.5, Unit::Percent);
    out[RATE] = ParamSpec::log("PINGS", 0.02, 6.0, 0.35, Unit::PerSecond);
    out[LEVEL] = ParamSpec::lin("LEVEL", 0.0, 1.0, 0.8, Unit::Decibel);
    out[PULSE] = ParamSpec::lin("PULSE", 0.0, 13.0, 5.0, Unit::Sync);
    let strips = [
        strip!(
            "PAD", 0.8, 0.0, 2.5, 5.0, 0.9, 3200.0, 0.3, 0.05, 90.0, 0.15, 0.03, 0.05, 0.3, 0.06,
            0.45
        ),
        strip!(
            "BASS", 0.55, 0.0, 2.0, 3.5, 0.15, 520.0, 0.2, 0.04, 21.0, 0.0, 0.03, 0.1, 0.0, 0.0,
            0.08
        ),
        strip!(
            "NOISE", 0.2, 0.0, 3.0, 5.0, 1.0, 6000.0, 0.45, 0.07, 240.0, 0.35, 0.04, 0.0, 0.0, 0.0,
            0.4
        ),
        strip!(
            "AMBIENCE", 0.4, 0.0, 4.0, 6.0, 1.0, 7000.0, 0.35, 0.03, 130.0, 0.25, 0.05, 0.0, 0.2,
            0.1, 0.6
        ),
        strip!(
            "SHIMMER", 0.4, 0.0, 0.01, 2.2, 1.0, 14_000.0, 0.0, 0.1, 600.0, 0.0, 0.1, 0.0, 0.25,
            0.5, 0.75
        ),
    ];
    let mut l = 0;
    while l < LAYERS {
        let mut k = 0;
        while k < STRIDE {
            out[FIRST + l * STRIDE + k] = strips[l][k];
            k += 1;
        }
        l += 1;
    }
    out
}

static PARAMS: [ParamSpec; COUNT] = build_params();

pub fn spec(i: usize) -> &'static ParamSpec {
    &PARAMS[i]
}

/// How long a chord can last: (label, beats).
pub const DURATIONS: [(&str, f64); 9] = [
    ("1/2 BAR", 2.0),
    ("1 BAR", 4.0),
    ("2 BARS", 8.0),
    ("3 BARS", 12.0),
    ("4 BARS", 16.0),
    ("6 BARS", 24.0),
    ("8 BARS", 32.0),
    ("12 BARS", 48.0),
    ("16 BARS", 64.0),
];

/// The modes.
pub const LOOP: usize = 0;
pub const EVOLVE: usize = 1;

/// Settings beyond the knobs.
pub mod data {
    use super::LAYERS;
    /// Each layer's sound, its lock (SHUFFLE leaves it alone) and its mute.
    pub const SOUND: usize = 0;
    pub const LOCK: usize = SOUND + LAYERS;
    pub const MUTE: usize = LOCK + LAYERS;
    /// What each layer's two motion filters are (`fx::FILTERS`).
    pub const KIND1: usize = MUTE + LAYERS;
    pub const KIND2: usize = KIND1 + LAYERS;
    /// What the chords come from: GENERATE picks another.
    pub const SEED: usize = KIND2 + LAYERS;
    /// The layer the panel is showing.
    pub const SELECTED: usize = SEED + 1;
    /// Shimmer plays an ostinato on PULSE's grid, not pings at random.
    pub const OSTINATO: usize = SELECTED + 1;
    pub const LEN: usize = OSTINATO + 1;
}

const fn build_data() -> [f32; data::LEN] {
    let mut d = [0.0; data::LEN];
    let mut l = 0;
    while l < LAYERS {
        d[data::KIND1 + l] = LOW as f32;
        d[data::KIND2 + l] = HIGH as f32;
        l += 1;
    }
    d[data::SOUND + PAD] = 1.0;
    d[data::SOUND + BASS] = 3.0;
    d[data::SOUND + NOISE] = 10.0;
    d[data::SOUND + AMBIENCE] = 0.0;
    d[data::SOUND + SHIMMER] = 0.0;
    d[data::SEED] = 1.0;
    d
}

static DATA: [f32; data::LEN] = build_data();

/// What the processor shows the panel.
pub mod viz {
    use super::{LAYERS, MAX_CHORDS, MAX_TONES};
    /// Each layer's peak (0…1), and where its filters are now (0…1).
    pub const PEAK: usize = 0;
    pub const FILTER1: usize = PEAK + LAYERS;
    pub const FILTER2: usize = FILTER1 + LAYERS;
    /// Each layer's sound that has arrived (−1: being made).
    pub const LOADED: usize = FILTER2 + LAYERS;
    /// The chord's notes low to high, how many, and the bass.
    pub const NOTE: usize = LOADED + LAYERS;
    pub const NOTES: usize = NOTE + MAX_TONES;
    pub const BASS: usize = NOTES + 1;
    /// 1 while it plays (generator or keys).
    pub const RUN: usize = BASS + 1;
    /// How far through the chord (0…1).
    pub const PROGRESS: usize = RUN + 1;
    /// The chords the panel lists, how many, and which one is playing.
    pub const CARD: usize = PROGRESS + 1;
    pub const CARDS: usize = CARD + MAX_CHORDS;
    pub const INDEX: usize = CARDS + 1;
    /// Pings so far, and the latest: a ring of [`EVENTS`] of (count, note,
    /// pan).
    pub const FIRED: usize = INDEX + 1;
    pub const EVENT: usize = FIRED + 1;
    pub const EVENTS: usize = 8;
    pub const END: usize = EVENT + 3 * EVENTS;
}

pub static DEF: ModuleDef = ModuleDef {
    name: "Vesper",
    prefix: "VESPER",
    hint: "generative ambient · pads · chords",
    inputs: &[],
    outputs: &["OUT"],
    params: &PARAMS,
    cv_depth: &[],
    modes: &["LOOP", "EVOLVE"],
    toggle: Some("PLAY"),
    toggle_default: true,
    map: false,
    midi_in: true,
    midi_out: true,
    data: &DATA,
    display_h: 0.0,
    controls_h: 0.0,
    viz: viz::END,
};

/// How many sounds a layer can pick from.
pub fn sound_count(layer: usize) -> usize {
    if layer == SHIMMER {
        PING_TIMBRES.len()
    } else {
        recipes(layer).len()
    }
}

pub fn sound_name(layer: usize, sound: usize) -> &'static str {
    let last = sound_count(layer) - 1;
    if layer == SHIMMER {
        PING_TIMBRES[sound.min(last)].name
    } else {
        recipes(layer)[sound.min(last)].name
    }
}

/// The sound a layer has picked.
pub fn sound(p: &ModuleParams, layer: usize) -> usize {
    (p.data[data::SOUND + layer].get().round().max(0.0) as usize).min(sound_count(layer) - 1)
}

pub fn set_sound(p: &ModuleParams, layer: usize, sound: usize) {
    p.data[data::SOUND + layer].set(sound.min(sound_count(layer) - 1) as f32);
}

pub fn flag(p: &ModuleParams, d: usize) -> bool {
    p.data[d].get() > 0.5
}

pub fn set_flag(p: &ModuleParams, d: usize, on: bool) {
    p.data[d].set(if on { 1.0 } else { 0.0 });
}

pub fn seed(p: &ModuleParams) -> u32 {
    p.data[data::SEED].get().max(1.0) as u32
}

/// GENERATE: new chords on the same sounds.
pub fn generate(p: &ModuleParams, seed: u32) {
    p.data[data::SEED].set((seed & 0xFF_FFFF).max(1) as f32);
}

/// SHUFFLE: another sound on every layer that isn't locked.
pub fn shuffle(p: &ModuleParams, seed: u32) {
    let mut rng = Rng::new(seed.max(1));
    for layer in 0..LAYERS {
        // The first draws follow the seed closely.
        let draw = rng.unit();
        if flag(p, data::LOCK + layer) {
            continue;
        }
        let count = sound_count(layer);
        let step = 1 + (draw * (count - 1) as f32) as usize;
        set_sound(p, layer, (sound(p, layer) + step) % count);
    }
}

/// The harmonic rules the knobs set.
pub fn rules(p: &ModuleParams) -> Rules {
    Rules {
        root: (p.get(ROOT).round() as i32).rem_euclid(12),
        mood: (p.get(MOOD).round().max(0.0) as usize).min(MOODS.len() - 1),
        variation: (p.get(VARIATION).round() as usize).clamp(1, 7),
        complexity: (p.get(COMPLEXITY).round() as usize).clamp(2, MAX_TONES),
    }
}

pub fn chords(p: &ModuleParams) -> usize {
    (p.get(CHORDS).round() as usize).clamp(2, MAX_CHORDS)
}

pub fn evolving(p: &ModuleParams) -> bool {
    p.mode() == EVOLVE
}

pub fn duration_index(p: &ModuleParams) -> usize {
    (p.get(DURATION).round().max(0.0) as usize).min(DURATIONS.len() - 1)
}

/// A motion filter's kind.
pub fn filter_kind(p: &ModuleParams, d: usize) -> usize {
    (p.data[d].get().round().max(0.0) as usize).min(fx::FILTERS.len() - 1)
}

/// A knob's value as the panel shows it.
pub fn format(p: &ModuleParams, i: usize) -> String {
    let v = p.get(i);
    match i {
        MOOD => MOODS[(v.round().max(0.0) as usize).min(MOODS.len() - 1)]
            .name
            .to_owned(),
        COMPLEXITY => format!("{} notes", v.round() as i32),
        VARIATION => format!("{} of 7", v.round() as i32),
        DURATION => DURATIONS[duration_index(p)].0.to_owned(),
        CHORDS => format!("{}", v.round() as i32),
        MIXTYPE => MIX_TYPES[(v.round().max(0.0) as usize).min(MIX_TYPES.len() - 1)]
            .0
            .to_owned(),
        ECHO | PULSE => sync_division(v).0.to_owned(),
        _ if i >= FIRST && (i - FIRST) % STRIDE == k::HOLD && v >= ENDLESS * 0.999 => {
            "sustain".to_owned()
        }
        _ if i >= FIRST && (i - FIRST) % STRIDE == k::OCTAVE => {
            format!("{:+} oct", v.round() as i32)
        }
        _ => PARAMS[i].format(v),
    }
}

/// How long a layer's notes hold before they let go by themselves.
pub fn hold_time(p: &ModuleParams, layer: usize) -> f32 {
    let v = p.get(knob(layer, k::HOLD));
    if v >= ENDLESS * 0.999 {
        f32::INFINITY
    } else {
        v
    }
}

/// A built-in scene: the settings that make a place to start from.
pub struct Scene {
    pub name: &'static str,
    pub mood: usize,
    pub mode: usize,
    pub variation: f32,
    pub complexity: f32,
    pub duration: f32,
    pub chords: f32,
    pub mix: f32,
    pub space: f32,
    /// By name: Pad, Bass, Noise, Ambience, Shimmer.
    pub sounds: [&'static str; LAYERS],
    pub levels: [f32; LAYERS],
    pub pings: f32,
    pub seed: f32,
    /// Shimmer plays an ostinato.
    pub ostinato: bool,
    /// Any other knobs it sets.
    pub knobs: &'static [(usize, f32)],
}

macro_rules! scene {
    ($name:literal, $mood:expr, $mode:expr, $v:expr, $c:expr, $d:expr, $n:expr, $mix:expr, $space:expr, $sounds:expr, $levels:expr, $pings:expr, $seed:expr) => {
        scene!(
            $name,
            $mood,
            $mode,
            $v,
            $c,
            $d,
            $n,
            $mix,
            $space,
            $sounds,
            $levels,
            $pings,
            $seed,
            false,
            &[]
        )
    };
    ($name:literal, $mood:expr, $mode:expr, $v:expr, $c:expr, $d:expr, $n:expr, $mix:expr, $space:expr, $sounds:expr, $levels:expr, $pings:expr, $seed:expr, $ostinato:expr, $knobs:expr) => {
        Scene {
            name: $name,
            mood: $mood,
            mode: $mode,
            variation: $v,
            complexity: $c,
            duration: $d,
            chords: $n,
            mix: $mix,
            space: $space,
            sounds: $sounds,
            levels: $levels,
            pings: $pings,
            seed: $seed,
            ostinato: $ostinato,
            knobs: $knobs,
        }
    };
}

pub static FACTORY: [Scene; 28] = [
    scene!(
        "Default",
        4,
        LOOP,
        5.0,
        3.0,
        4.0,
        4.0,
        0.0,
        0.7,
        [
            "Slow Silk",
            "Round Triangle",
            "Soft Wind",
            "Cathedral Air",
            "Glass Tear"
        ],
        [0.8, 0.55, 0.2, 0.4, 0.4],
        0.35,
        1.0
    ),
    scene!(
        "First Light",
        3,
        EVOLVE,
        6.0,
        4.0,
        4.0,
        4.0,
        1.0,
        0.8,
        [
            "First Light",
            "Velvet Low",
            "Forest Hush",
            "Harmonic Mist",
            "Star Field"
        ],
        [0.8, 0.45, 0.16, 0.4, 0.45],
        0.5,
        11.0
    ),
    scene!(
        "Cathedral",
        5,
        LOOP,
        4.0,
        4.0,
        6.0,
        4.0,
        0.0,
        0.95,
        [
            "Ah Choir",
            "Cathedral Pedal",
            "Pink Air",
            "Ghost Choir",
            "Harmonic"
        ],
        [0.8, 0.5, 0.04, 0.4, 0.25],
        0.15,
        7.0
    ),
    scene!(
        "Deep Ocean",
        8,
        EVOLVE,
        3.0,
        2.0,
        6.0,
        4.0,
        1.0,
        0.85,
        [
            "Deep Ocean",
            "Sea Floor",
            "Far Shore",
            "Deep Sea",
            "Soft Rain"
        ],
        [0.75, 0.6, 0.3, 0.55, 0.2],
        0.12,
        23.0
    ),
    scene!(
        "Cold War",
        10,
        LOOP,
        5.0,
        3.0,
        4.0,
        4.0,
        2.0,
        0.6,
        [
            "Hollow Wind",
            "Dark Saw",
            "Radio Static",
            "Radio Ghosts",
            "Tine"
        ],
        [0.75, 0.55, 0.1, 0.4, 0.25],
        0.2,
        31.0
    ),
    scene!(
        "Desert Crossing",
        12,
        EVOLVE,
        4.0,
        3.0,
        4.0,
        4.0,
        0.0,
        0.7,
        [
            "Soft Reed",
            "Warm Triangle",
            "Desert Gusts",
            "Heat Haze",
            "Kalimba"
        ],
        [0.7, 0.5, 0.25, 0.3, 0.35],
        0.3,
        5.0
    ),
    scene!(
        "Nebula",
        14,
        EVOLVE,
        7.0,
        4.0,
        5.0,
        4.0,
        1.0,
        1.0,
        [
            "Nebula",
            "Noise Sub",
            "Cosmic Dust",
            "Solar Wind",
            "Thin Ice"
        ],
        [0.8, 0.4, 0.2, 0.4, 0.4],
        0.6,
        41.0
    ),
    scene!(
        "Last Train",
        4,
        LOOP,
        4.0,
        4.0,
        3.0,
        4.0,
        2.0,
        0.55,
        [
            "Tape Memory",
            "Tape Bass",
            "Subway",
            "Underpass",
            "Music Box"
        ],
        [0.8, 0.55, 0.2, 0.3, 0.3],
        0.25,
        17.0
    ),
    scene!(
        "Frozen Lake",
        7,
        LOOP,
        5.0,
        3.0,
        5.0,
        6.0,
        0.0,
        0.9,
        [
            "Bowed Glass",
            "Frost Sub",
            "Arctic Wind",
            "Frozen Lake",
            "Crystal"
        ],
        [0.75, 0.45, 0.22, 0.45, 0.4],
        0.4,
        29.0
    ),
    scene!(
        "Elegy",
        4,
        LOOP,
        5.0,
        4.0,
        4.0,
        8.0,
        0.0,
        0.75,
        [
            "Amber Strings",
            "Deep Cello",
            "Pink Air",
            "Phantom Strings",
            "Celesta"
        ],
        [0.85, 0.55, 0.06, 0.3, 0.25],
        0.18,
        3.0
    ),
    scene!(
        "Monolith",
        9,
        EVOLVE,
        3.0,
        2.0,
        7.0,
        4.0,
        3.0,
        0.8,
        [
            "Iron Drone",
            "Grit Drone",
            "Tunnel Rumble",
            "Machine Hall",
            "Spark"
        ],
        [0.7, 0.6, 0.3, 0.45, 0.15],
        0.1,
        13.0
    ),
    scene!(
        "Neon Rain",
        5,
        LOOP,
        5.0,
        4.0,
        2.0,
        4.0,
        1.0,
        0.65,
        [
            "Analog Horizon",
            "Pulse Bass",
            "Rain on Glass",
            "Space Station",
            "Bell Dust"
        ],
        [0.8, 0.5, 0.2, 0.25, 0.3],
        0.4,
        37.0
    ),
    scene!(
        "Temple Hours",
        13,
        EVOLVE,
        4.0,
        3.0,
        6.0,
        4.0,
        0.0,
        0.85,
        [
            "Distant Monks",
            "Monk Low",
            "Slow Breath",
            "Temple Bells",
            "Harmonic"
        ],
        [0.75, 0.55, 0.12, 0.5, 0.2],
        0.12,
        19.0
    ),
    scene!(
        "Iron Sky",
        11,
        LOOP,
        5.0,
        3.0,
        3.0,
        4.0,
        3.0,
        0.65,
        [
            "Rust Choir",
            "Rust Drone",
            "Machine Air",
            "Slow Machines",
            "Thin Ice"
        ],
        [0.75, 0.55, 0.22, 0.45, 0.2],
        0.18,
        43.0
    ),
    scene!(
        "Slow Tide",
        0,
        EVOLVE,
        5.0,
        3.0,
        6.0,
        4.0,
        1.0,
        0.85,
        [
            "Tidal Sweep",
            "Tidal Sub",
            "Ocean Surf",
            "Last Light",
            "Glass Tear"
        ],
        [0.75, 0.5, 0.3, 0.35, 0.3],
        0.2,
        47.0
    ),
    scene!(
        "Aurora",
        1,
        EVOLVE,
        6.0,
        4.0,
        5.0,
        4.0,
        1.0,
        0.95,
        [
            "Aurora",
            "Soft Sub",
            "Whisper",
            "Overtone Cloud",
            "Star Field"
        ],
        [0.8, 0.45, 0.1, 0.35, 0.45],
        0.7,
        53.0
    ),
    scene!(
        "Ghost Station",
        6,
        LOOP,
        4.0,
        3.0,
        5.0,
        4.0,
        2.0,
        0.75,
        [
            "Ghost Organ",
            "Organ Pedal",
            "Dead Channel",
            "Old Station",
            "Tine"
        ],
        [0.75, 0.5, 0.08, 0.5, 0.2],
        0.15,
        59.0
    ),
    scene!(
        "Lost Archive",
        15,
        LOOP,
        5.0,
        4.0,
        4.0,
        4.0,
        2.0,
        0.6,
        [
            "Lost Signal",
            "Worn Tape",
            "Worn Vinyl",
            "Radio Ghosts",
            "Music Box"
        ],
        [0.75, 0.5, 0.25, 0.35, 0.25],
        0.25,
        61.0
    ),
    scene!(
        "Golden Hour",
        2,
        LOOP,
        4.0,
        4.0,
        3.0,
        4.0,
        1.0,
        0.7,
        [
            "Saw Ensemble",
            "Amber Low",
            "Campfire",
            "Wind Chimes",
            "Celesta"
        ],
        [0.8, 0.5, 0.18, 0.3, 0.3],
        0.35,
        67.0
    ),
    scene!(
        "Event Horizon",
        9,
        EVOLVE,
        2.0,
        2.0,
        8.0,
        4.0,
        3.0,
        1.0,
        [
            "Bowed Metal",
            "Thunderhead",
            "Distant Storm",
            "Event Horizon",
            "Spark"
        ],
        [0.75, 0.6, 0.3, 0.55, 0.12],
        0.08,
        71.0
    ),
    // For the big screen: walls of brass, then the organ. In the manner of
    // large-scale film scoring; these scenes hold no one's themes.
    scene!(
        "Folded City",
        4,
        LOOP,
        4.0,
        2.0,
        2.0,
        4.0,
        3.0,
        0.8,
        [
            "Low Brass Wall",
            "Braam Floor",
            "Tunnel Rumble",
            "Falling City",
            "Spark"
        ],
        [1.0, 0.8, 0.08, 0.22, 0.0],
        0.05,
        SEED_EPIC,
        false,
        &[
            (knob(PAD, k::ATTACK), 0.14),
            (knob(PAD, k::HOLD), 1.3),
            (knob(PAD, k::RELEASE), 1.6),
            (knob(PAD, k::BLOOM), 0.65),
            (knob(PAD, k::DRIVE), 0.35),
            (knob(PAD, k::CHORUS), 0.12),
            (knob(PAD, k::REVERB), 0.35),
            (knob(PAD, k::FREQ1), 3400.0),
            (knob(PAD, k::MOVE1), 0.05),
            (knob(PAD, k::OCTAVE), -1.0),
            (knob(BASS, k::ATTACK), 0.08),
            (knob(BASS, k::HOLD), 1.3),
            (knob(BASS, k::RELEASE), 1.5),
            (knob(BASS, k::BLOOM), 0.35),
            (knob(BASS, k::DRIVE), 0.4),
            (knob(BASS, k::FREQ1), 1400.0),
        ]
    ),
    scene!(
        "Limbo",
        4,
        LOOP,
        4.0,
        3.0,
        4.0,
        4.0,
        2.0,
        0.85,
        [
            "Slowed Strings",
            "Pressure Sub",
            "Old Tape",
            "Time Ripple",
            "Felt Piano"
        ],
        [0.85, 0.5, 0.12, 0.35, 0.3],
        0.12,
        SEED_EPIC,
        false,
        &[
            (knob(PAD, k::ATTACK), 4.0),
            (knob(PAD, k::RELEASE), 7.0),
            (knob(PAD, k::OCTAVE), -1.0),
            (knob(SHIMMER, k::OCTAVE), -1.0),
            (knob(SHIMMER, k::RELEASE), 3.5),
        ]
    ),
    scene!(
        "Dream Stack",
        4,
        LOOP,
        4.0,
        3.0,
        2.0,
        4.0,
        0.0,
        0.7,
        [
            "Tense Strings",
            "Contra Tuba",
            "Pink Air",
            "Deep Space Hush",
            "Pizzicato"
        ],
        [0.8, 0.65, 0.05, 0.3, 0.7],
        0.3,
        SEED_EPIC,
        true,
        &[
            (PULSE, 5.0),
            (knob(PAD, k::ATTACK), 1.2),
            (knob(PAD, k::RELEASE), 2.5),
            (knob(BASS, k::ATTACK), 0.5),
            (knob(BASS, k::DRIVE), 0.25),
            (knob(BASS, k::FREQ1), 900.0),
            (knob(SHIMMER, k::OCTAVE), -2.0),
            (knob(SHIMMER, k::RELEASE), 2.0),
            (knob(SHIMMER, k::ECHO), 0.1),
            (knob(SHIMMER, k::REVERB), 0.3),
            (knob(SHIMMER, k::CHORUS), 0.0),
        ]
    ),
    scene!(
        "Last Thought",
        4,
        LOOP,
        4.0,
        3.0,
        2.0,
        4.0,
        1.0,
        0.85,
        [
            "Amber Strings",
            "Velvet Low",
            "Pink Air",
            "Time Ripple",
            "Felt Piano"
        ],
        [0.6, 0.45, 0.04, 0.25, 0.85],
        0.3,
        SEED_EPIC,
        true,
        &[
            (PULSE, 8.0),
            (knob(PAD, k::ATTACK), 3.0),
            (knob(PAD, k::RELEASE), 6.0),
            (knob(SHIMMER, k::OCTAVE), -1.0),
            (knob(SHIMMER, k::RELEASE), 3.0),
            (knob(SHIMMER, k::ECHO), 0.15),
            (knob(SHIMMER, k::REVERB), 0.45),
            (knob(SHIMMER, k::CHORUS), 0.0),
        ]
    ),
    scene!(
        "Gravity Organ",
        4,
        LOOP,
        4.0,
        4.0,
        4.0,
        4.0,
        0.0,
        1.0,
        [
            "Temple Organ",
            "Pedal 32",
            "Pink Air",
            "Deep Space Hush",
            "Organ Pipe"
        ],
        [0.9, 0.7, 0.03, 0.2, 0.55],
        0.3,
        SEED_EPIC,
        true,
        &[
            (PULSE, 5.0),
            (knob(PAD, k::ATTACK), 1.0),
            (knob(PAD, k::RELEASE), 5.0),
            (knob(PAD, k::REVERB), 0.75),
            (knob(PAD, k::CHORUS), 0.08),
            (knob(PAD, k::FREQ1), 9000.0),
            (knob(PAD, k::MOVE1), 0.08),
            (knob(PAD, k::FREQ2), 21.0),
            (knob(PAD, k::MOVE2), 0.0),
            (knob(BASS, k::ATTACK), 0.6),
            (knob(BASS, k::RELEASE), 5.0),
            (knob(BASS, k::REVERB), 0.4),
            (knob(BASS, k::FREQ1), 1400.0),
            (knob(SHIMMER, k::OCTAVE), -1.0),
            (knob(SHIMMER, k::ATTACK), 0.02),
            (knob(SHIMMER, k::RELEASE), 1.2),
            (knob(SHIMMER, k::REVERB), 0.8),
            (knob(SHIMMER, k::ECHO), 0.0),
            (knob(SHIMMER, k::CHORUS), 0.0),
        ]
    ),
    scene!(
        "Full Ascent",
        4,
        LOOP,
        4.0,
        4.0,
        2.0,
        4.0,
        0.0,
        0.95,
        [
            "Full Organ",
            "Pedal 32",
            "Pink Air",
            "Falling City",
            "Clock Tick"
        ],
        [0.9, 0.8, 0.03, 0.25, 0.8],
        0.3,
        SEED_EPIC,
        true,
        &[
            (PULSE, 8.0),
            (knob(PAD, k::ATTACK), 0.5),
            (knob(PAD, k::RELEASE), 4.0),
            (knob(PAD, k::REVERB), 0.7),
            (knob(PAD, k::CHORUS), 0.05),
            (knob(PAD, k::FREQ1), 14_000.0),
            (knob(PAD, k::MOVE1), 0.05),
            (knob(PAD, k::FREQ2), 21.0),
            (knob(PAD, k::MOVE2), 0.0),
            (knob(BASS, k::ATTACK), 0.3),
            (knob(BASS, k::FREQ1), 1800.0),
            (knob(BASS, k::REVERB), 0.3),
            (knob(SHIMMER, k::REVERB), 0.25),
            (knob(SHIMMER, k::ECHO), 0.0),
            (knob(SHIMMER, k::CHORUS), 0.0),
            (knob(SHIMMER, k::RELEASE), 1.0),
        ]
    ),
    scene!(
        "Dust Field",
        5,
        EVOLVE,
        4.0,
        3.0,
        4.0,
        4.0,
        1.0,
        0.95,
        [
            "Distant Organ",
            "Velvet Low",
            "Soft Wind",
            "Dust Field",
            "Clock Tick"
        ],
        [0.75, 0.45, 0.14, 0.35, 0.65],
        0.3,
        73.0,
        true,
        &[
            (PULSE, 11.0),
            (knob(PAD, k::ATTACK), 3.0),
            (knob(PAD, k::RELEASE), 7.0),
            (knob(PAD, k::REVERB), 0.7),
            (knob(PAD, k::FREQ2), 21.0),
            (knob(PAD, k::MOVE2), 0.0),
            (knob(SHIMMER, k::REVERB), 0.2),
            (knob(SHIMMER, k::ECHO), 0.25),
            (knob(SHIMMER, k::CHORUS), 0.0),
            (knob(SHIMMER, k::RELEASE), 1.0),
        ]
    ),
    scene!(
        "Tesseract",
        14,
        EVOLVE,
        5.0,
        4.0,
        5.0,
        4.0,
        1.0,
        1.0,
        [
            "Organ Reeds",
            "Pedal 32",
            "Whisper",
            "Time Ripple",
            "Organ Pipe"
        ],
        [0.85, 0.6, 0.06, 0.4, 0.5],
        0.25,
        79.0,
        false,
        &[
            (knob(PAD, k::ATTACK), 2.5),
            (knob(PAD, k::RELEASE), 8.0),
            (knob(PAD, k::REVERB), 0.75),
            (knob(SHIMMER, k::OCTAVE), -1.0),
            (knob(SHIMMER, k::RELEASE), 3.0),
        ]
    ),
];

/// A seed that writes i–VI–III–VII on a natural minor with four degrees: the
/// broad, slow turn the big-screen scenes want.
const SEED_EPIC: f32 = 3813.0;

/// Finds a sound by name (0 if the name is gone).
pub fn find_sound(layer: usize, name: &str) -> usize {
    (0..sound_count(layer))
        .find(|&s| sound_name(layer, s) == name)
        .unwrap_or(0)
}

/// Built-in scene `n`, from the defaults. It keeps ROOT and LEVEL (the
/// player's), and PLAY.
pub fn load_factory(p: &ModuleParams, n: usize) {
    let s = &FACTORY[n.min(FACTORY.len() - 1)];
    for (i, spec) in PARAMS
        .iter()
        .enumerate()
        .filter(|(i, _)| ![ROOT, LEVEL].contains(i))
    {
        p.set(i, spec.default);
    }
    p.set(MOOD, s.mood as f32);
    p.set(VARIATION, s.variation);
    p.set(COMPLEXITY, s.complexity);
    p.set(DURATION, s.duration);
    p.set(CHORDS, s.chords);
    p.set(MIXTYPE, s.mix);
    p.set(SPACE, s.space);
    p.set(RATE, s.pings);
    p.set_mode(s.mode);
    p.data[data::SEED].set(s.seed);
    set_flag(p, data::OSTINATO, s.ostinato);
    for layer in 0..LAYERS {
        set_sound(p, layer, find_sound(layer, s.sounds[layer]));
        p.set(knob(layer, k::LEVEL), s.levels[layer]);
        set_flag(p, data::MUTE + layer, false);
        p.data[data::KIND1 + layer].set(DATA[data::KIND1 + layer]);
        p.data[data::KIND2 + layer].set(DATA[data::KIND2 + layer]);
    }
    for &(i, v) in s.knobs {
        p.set(i, v);
    }
}

/// Where the Pad's chords sit.
const PAD_CENTER: i32 = 60;
/// The octaves the bass and a tonal Ambience stay in.
const BASS_LOW: i32 = 33;
const AMBIENCE_LOW: i32 = 45;
/// Shimmer's lowest note, before its OCTAVE.
const SHIMMER_LOW: i32 = 84;
/// How loud pings are against the loops.
const PING_LEVEL: f32 = 0.28;
/// The ostinato stands this far above the chord.
const OSTINATO_RISE: i32 = 12;
/// MIDI: how long a ping's note lasts, and how many can overlap.
const PING_NOTE: f32 = 0.12;
const PING_NOTES: usize = 8;
/// MIDI channels of the score, zero-based.
const CHORD_CHANNEL: u8 = 0;
const BASS_CHANNEL: u8 = 1;
const PING_CHANNEL: u8 = 2;

/// The ostinato's note at a step: up the chord's tones over two octaves and
/// back down, not playing the top and the bottom twice on the turn.
fn ostinato_note(tones: &[i32], step: i64) -> i32 {
    let mut sorted = [0i32; Held::CAP];
    let len = tones.len().min(Held::CAP);
    sorted[..len].copy_from_slice(&tones[..len]);
    sorted[..len].sort_unstable();
    let span = 2 * len as i64;
    let cycle = (2 * (span - 1)).max(1);
    let at = step.rem_euclid(cycle);
    let i = (if at < span { at } else { cycle - at }) as usize;
    sorted[i % len] + 12 * (i / len) as i32
}

/// What decides the chords, to notice when it changes.
#[derive(Clone, Copy, PartialEq)]
struct Writing {
    rules: Rules,
    seed: u32,
    count: usize,
    evolve: bool,
}

pub struct Vesper {
    params: Arc<ModuleParams>,
    midi: Arc<MidiState>,
    midi_cursor: u32,
    sr: f32,
    library: Library,
    samplers: Vec<Sampler>,
    pings: Pings,
    strips: Vec<Strip>,
    echo: Echo,
    space: Space,
    master: Master,
    limit: f32,
    level: f32,
    started: bool,
    quiet: Quiet,

    // The score.
    writing: Writing,
    degrees: ([u8; 7], usize),
    progression: [Step; MAX_CHORDS],
    /// EVOLVE: the chords so far, the last being the next to come.
    window: [Step; MAX_CHORDS],
    filled: usize,
    stream: Rng,
    /// The chord now counted on the bars, while it runs.
    step: Option<i64>,
    /// The chord's notes low to high, and the bass, while it plays.
    notes: [i32; MAX_TONES],
    note_count: usize,
    bass: i32,
    keys: Held,

    // MIDI.
    out: MidiBuf,
    midi_chord: [u8; MAX_TONES],
    midi_chord_len: usize,
    midi_bass: Option<u8>,
    ping_offs: [(u8, f32); PING_NOTES],

    rng: Rng,
    /// Shimmer's ostinato: the step it last struck, and the beat when the
    /// transport stands still (keys keep it going).
    pulse_step: Option<i64>,
    clock: f64,
    /// A chord has just begun: notes strike again.
    new_chord: bool,
    /// What a chord's voices share, gliding as their number changes.
    chord_norm: f32,
}

impl Vesper {
    pub fn new(params: Arc<ModuleParams>, midi: Arc<MidiState>, sample_rate: f32) -> Self {
        let sr = sample_rate;
        let rules_now = rules(&params);
        let seed_now = seed(&params);
        let count = chords(&params);
        let evolve = evolving(&params);
        let degrees = harmony::allowed(seed_now, &rules_now);
        let progression = harmony::progression(seed_now, count, &rules_now);
        let mut stream = harmony::stream(seed_now);
        let mut window = [progression[0]; MAX_CHORDS];
        window[1] = harmony::next_step(
            &mut stream,
            &rules_now,
            &degrees.0[..degrees.1],
            window[0],
            None,
            None,
        );
        Self {
            midi_cursor: midi.stream_head(),
            midi,
            sr,
            library: Library::start(params.clone(), sr),
            samplers: (0..SAMPLED)
                .map(|l| Sampler::new(0x5A40_1000 + 77 * l as u32))
                .collect(),
            pings: Pings::new(),
            strips: (0..LAYERS)
                .map(|l| Strip::new(0xF117_0000 + 991 * l as u32, sr))
                .collect(),
            echo: Echo::new(sr),
            space: Space::new(sr),
            master: Master::new(sr),
            limit: 0.0,
            level: params.get(LEVEL),
            started: false,
            quiet: Quiet::new((sr * (echo::LONGEST + 1.0)) as usize),
            writing: Writing {
                rules: rules_now,
                seed: seed_now,
                count,
                evolve,
            },
            degrees,
            progression,
            window,
            filled: 2,
            stream,
            step: None,
            notes: [0; MAX_TONES],
            note_count: 0,
            bass: BASS_LOW,
            keys: Held::new(),
            out: MidiBuf::default(),
            midi_chord: [0; MAX_TONES],
            midi_chord_len: 0,
            midi_bass: None,
            ping_offs: [(0, 0.0); PING_NOTES],
            rng: Rng::new(0x51A7_71E5),
            pulse_step: None,
            clock: 0.0,
            new_chord: false,
            chord_norm: 1.0,
            params,
        }
    }

    /// EVOLVE, on a chord's end: what was next is now, and a new next.
    fn advance(&mut self) {
        let rules = self.writing.rules;
        if self.filled == MAX_CHORDS {
            self.window.copy_within(1.., 0);
            self.filled -= 1;
        }
        let (prev, before) = (self.window[self.filled - 1], self.window[self.filled - 2]);
        self.window[self.filled] = harmony::next_step(
            &mut self.stream,
            &rules,
            &self.degrees.0[..self.degrees.1],
            prev,
            Some(before.degree),
            None,
        );
        self.filled += 1;
    }

    /// EVOLVE, from the seed again: a first chord and what follows it.
    fn restart_stream(&mut self) {
        let w = self.writing;
        self.stream = harmony::stream(w.seed);
        self.window[0] = self.progression[0];
        self.window[1] = harmony::next_step(
            &mut self.stream,
            &w.rules,
            &self.degrees.0[..self.degrees.1],
            self.window[0],
            None,
            None,
        );
        self.filled = 2;
    }

    /// The chord playing.
    fn current(&self) -> Step {
        if self.writing.evolve {
            self.window[self.filled - 2]
        } else {
            let index = self.step.unwrap_or(0).rem_euclid(self.writing.count as i64) as usize;
            self.progression[index]
        }
    }

    /// Lays the chord out and tells the MIDI output what changed: notes that
    /// stay are held over.
    fn write_chord(&mut self, frame: usize) {
        let rules = self.writing.rules;
        let step = self.current();
        let prev = self.notes;
        let mut notes = [0; MAX_TONES];
        let len = harmony::voice(
            &rules,
            step,
            &prev[..self.note_count],
            PAD_CENTER,
            &mut notes,
        );
        self.notes = notes;
        self.note_count = len;
        self.bass = harmony::fold(rules.chord_root(step), BASS_LOW);

        let keys: [u8; MAX_TONES] = notes.map(|n| n.clamp(0, 127) as u8);
        for k in 0..self.midi_chord_len {
            let key = self.midi_chord[k];
            if !keys[..len].contains(&key) {
                self.out.push(frame, [0x80 | CHORD_CHANNEL, key, 0]);
            }
        }
        for &key in &keys[..len] {
            if !self.midi_chord[..self.midi_chord_len].contains(&key) {
                self.out.push(frame, [0x90 | CHORD_CHANNEL, key, 84]);
            }
        }
        self.midi_chord = keys;
        self.midi_chord_len = len;
        let bass = self.bass.clamp(0, 127) as u8;
        if self.midi_bass != Some(bass) {
            if let Some(old) = self.midi_bass {
                self.out.push(frame, [0x80 | BASS_CHANNEL, old, 0]);
            }
            self.out.push(frame, [0x90 | BASS_CHANNEL, bass, 90]);
            self.midi_bass = Some(bass);
        }
    }

    /// The score has stopped: every note it holds ends.
    fn end_score(&mut self, frame: usize) {
        for k in 0..self.midi_chord_len {
            self.out
                .push(frame, [0x80 | CHORD_CHANNEL, self.midi_chord[k], 0]);
        }
        self.midi_chord_len = 0;
        if let Some(old) = self.midi_bass.take() {
            self.out.push(frame, [0x80 | BASS_CHANNEL, old, 0]);
        }
        for off in &mut self.ping_offs {
            if off.1 > 0.0 {
                self.out.push(frame, [0x80 | PING_CHANNEL, off.0, 0]);
                off.1 = 0.0;
            }
        }
        self.note_count = 0;
    }

    /// Follows the rules and the bars. Returns whether the generator runs.
    fn run_score(&mut self, ctx: &ProcessCtx, n: usize) -> bool {
        let p = self.params.clone();
        let writing = Writing {
            rules: rules(&p),
            seed: seed(&p),
            count: chords(&p),
            evolve: evolving(&p),
        };
        let changed = writing != self.writing;
        if changed {
            let (old, seed_moved) = (self.writing, writing.seed != self.writing.seed);
            self.writing = writing;
            self.degrees = harmony::allowed(writing.seed, &writing.rules);
            self.progression = harmony::progression(writing.seed, writing.count, &writing.rules);
            if seed_moved || old.evolve != writing.evolve {
                self.restart_stream();
            }
        }
        if ctx.restart {
            self.restart_stream();
            self.step = None;
        }
        let running = p.toggled() && ctx.playing;
        if !running {
            if self.step.is_some() || self.midi_bass.is_some() {
                self.end_score(0);
            }
            self.step = None;
            return false;
        }
        // The chord on this block's last frame.
        let beats = DURATIONS[duration_index(&p)].1;
        let last = ctx.beat + ctx.beats_per_sample * (n as f64 - 0.5);
        let index = (last / beats + 1e-9).floor() as i64;
        if self.step != Some(index) {
            let boundary = self.step.is_some();
            self.step = Some(index);
            if boundary && writing.evolve {
                self.advance();
            }
            self.write_chord(frame_of(ctx, n, index as f64 * beats));
            self.new_chord = true;
        } else if changed {
            self.write_chord(0);
        }
        p.viz[viz::PROGRESS].set(((ctx.beat / beats).fract()) as f32);
        true
    }

    fn publish_cards(&self) {
        let p = &self.params;
        if self.writing.evolve {
            for (k, s) in self.window.iter().enumerate() {
                p.viz[viz::CARD + k].set(s.pack());
            }
            p.viz[viz::CARDS].set(self.filled as f32);
            p.viz[viz::INDEX].set((self.filled - 2) as f32);
        } else {
            for (k, s) in self.progression.iter().enumerate() {
                p.viz[viz::CARD + k].set(s.pack());
            }
            p.viz[viz::CARDS].set(self.writing.count as f32);
            let index = self
                .step
                .map_or(-1, |s| s.rem_euclid(self.writing.count as i64));
            p.viz[viz::INDEX].set(index as f32);
        }
    }

    /// A ping, perhaps, this block: at random, or the ostinato's next note.
    fn ring(&mut self, ctx: &ProcessCtx, tones: &[i32], n: usize) {
        let p = self.params.clone();
        let secs = n as f32 / self.sr;
        for off in &mut self.ping_offs {
            if off.1 > 0.0 {
                off.1 -= secs;
                if off.1 <= 0.0 {
                    self.out.push(0, [0x80 | PING_CHANNEL, off.0, 0]);
                }
            }
        }
        let beat = if ctx.playing { ctx.beat } else { self.clock };
        self.clock = beat + ctx.beats_per_sample * n as f64;
        if tones.is_empty() {
            self.pulse_step = None;
            return;
        }
        let timbre = &PING_TIMBRES[sound(&p, SHIMMER)];
        let octave = 12 * p.get(knob(SHIMMER, k::OCTAVE)).round() as i32;
        let (note, pan, level, frame) = if flag(&p, data::OSTINATO) {
            let grid = sync_division(p.get(PULSE)).1;
            let last = beat + ctx.beats_per_sample * (n as f64 - 0.5);
            let step = (last / grid + 1e-9).floor() as i64;
            if self.pulse_step == Some(step) {
                return;
            }
            self.pulse_step = Some(step);
            let note = ostinato_note(tones, step) + OSTINATO_RISE + octave;
            // Hardest on the bar, a little on the beat, and a slow sway from
            // side to side.
            let at = step as f64 * grid;
            let level = if (at / 4.0).fract().abs() < 1e-6 {
                1.0
            } else if at.fract().abs() < 1e-6 {
                0.8
            } else {
                0.62
            };
            let pan = 0.35 * (step as f32 * 0.37).sin();
            (
                note.clamp(0, 127),
                pan,
                level,
                if ctx.playing { frame_of(ctx, n, at) } else { 0 },
            )
        } else {
            if self.rng.unit() >= p.get(RATE) * secs {
                return;
            }
            let tone = tones[(self.rng.unit() * tones.len() as f32) as usize % tones.len()];
            let lift = if self.rng.unit() < timbre.lift { 12 } else { 0 };
            let pan = self.rng.bipolar() * 0.8;
            (
                (harmony::fold(tone, SHIMMER_LOW) + lift + octave).clamp(0, 127),
                pan,
                1.0,
                0,
            )
        };
        let (attack, release) = (
            p.get(knob(SHIMMER, k::ATTACK)),
            p.get(knob(SHIMMER, k::RELEASE)),
        );
        self.pings
            .strike(timbre, note as f32, attack, release, pan, level, self.sr);
        let count = p.viz[viz::FIRED].get() + 1.0;
        p.viz[viz::FIRED].set(count);
        let at = viz::EVENT + 3 * (count as usize % viz::EVENTS);
        p.viz[at].set(count);
        p.viz[at + 1].set(note as f32);
        p.viz[at + 2].set(pan);
        // Only the generator writes a score.
        if p.toggled()
            && let Some(off) = self.ping_offs.iter_mut().find(|o| o.1 <= 0.0)
        {
            self.out.push(
                frame,
                [0x90 | PING_CHANNEL, note as u8, (100.0 * level) as u8],
            );
            *off = (note as u8, PING_NOTE);
        }
    }

    /// What each sampler should be playing.
    fn assign_notes(&mut self, running: bool) {
        let p = self.params.clone();
        let play = p.toggled();
        let octave = |l: usize| 12.0 * p.get(knob(l, k::OCTAVE)).round();
        let held = self.keys.notes();
        let mut scratch = [0.0f32; Held::CAP];

        // Pad: the chord, or the keys.
        let count = if running {
            for (s, &m) in scratch.iter_mut().zip(&self.notes[..self.note_count]) {
                *s = m as f32 + octave(PAD);
            }
            self.note_count
        } else if !play {
            for (s, &m) in scratch.iter_mut().zip(held) {
                *s = m as f32 + octave(PAD);
            }
            held.len()
        } else {
            0
        };
        self.samplers[PAD].set_notes(&scratch[..count]);

        // Bass and a tonal Ambience: the chord's root, or the lowest key.
        let low_key = held.iter().copied().min().map(|m| m as i32);
        let root = if running {
            Some(self.bass)
        } else if !play {
            low_key
        } else {
            None
        };
        match root {
            Some(r) => {
                self.samplers[BASS].set_notes(&[harmony::fold(r, BASS_LOW) as f32 + octave(BASS)]);
                for l in [NOISE, AMBIENCE] {
                    let note = if self.samplers[l].tonal() {
                        harmony::fold(r, AMBIENCE_LOW) as f32 + octave(l)
                    } else {
                        self.samplers[l].home() + octave(l)
                    };
                    self.samplers[l].set_notes(&[note]);
                }
            }
            None => {
                for sampler in &mut self.samplers[BASS..] {
                    sampler.set_notes(&[]);
                }
            }
        }
    }
}

impl Processor for Vesper {
    #[allow(clippy::needless_range_loop)] // the buses and the outputs at once
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        self.out.clear();
        let p = self.params.clone();
        let sr = self.sr;
        let secs = n as f32 / sr;

        // Keys: they only play with PLAY off.
        let play = p.toggled();
        let keys = &mut self.keys;
        let mut handle = |msg: [u8; 3]| match note_event(msg) {
            Some(true) if !play => keys.press(msg[1]),
            Some(false) => keys.release(msg[1]),
            None if msg[0] & 0xF0 == 0xB0 && matches!(msg[1], 120 | 123) => keys.clear(),
            _ => {}
        };
        if inputs.midi_connected {
            for e in inputs.midi.events() {
                handle(e.data);
            }
            self.midi_cursor = self.midi.stream_head();
        } else {
            self.midi.read_stream(&mut self.midi_cursor, handle);
        }
        if play {
            self.keys.clear();
        }

        // Sounds that have finished being made, and those to free.
        for l in 0..SAMPLED {
            let sampler = &mut self.samplers[l];
            if sampler.can_install()
                && let Some(bank) = self.library.take(l)
            {
                sampler.install(bank);
            }
            if let Some(old) = sampler.finished()
                && let Err(old) = self.library.retire(l, old)
            {
                sampler.keep(old);
            }
            p.viz[viz::LOADED + l].set(sampler.loaded().map_or(-1.0, |s| s as f32));
        }
        p.viz[viz::LOADED + SHIMMER].set(sound(&p, SHIMMER) as f32);

        let running = self.run_score(ctx, n);
        self.publish_cards();
        self.assign_notes(running);
        if std::mem::take(&mut self.new_chord) {
            for sampler in &mut self.samplers {
                sampler.strike_again();
            }
            for strip in &mut self.strips {
                strip.bloom_again();
            }
        }
        let keyed = !play && !self.keys.is_empty();
        p.viz[viz::RUN].set(if running || keyed { 1.0 } else { 0.0 });
        let pad_octave = 12.0 * p.get(knob(PAD, k::OCTAVE)).round();
        for (n, &m) in self.notes.iter().enumerate() {
            p.viz[viz::NOTE + n].set(m as f32 + pad_octave);
        }
        p.viz[viz::NOTES].set(if running { self.note_count as f32 } else { 0.0 });
        p.viz[viz::BASS].set(self.bass as f32);

        // What the pings can land on.
        let mut tones = [0i32; Held::CAP];
        let tone_count = if running {
            tones[..self.note_count].copy_from_slice(&self.notes[..self.note_count]);
            self.note_count
        } else if keyed {
            let held = self.keys.notes();
            for (t, &m) in tones.iter_mut().zip(held) {
                *t = m as i32;
            }
            held.len()
        } else {
            0
        };
        self.ring(ctx, &tones[..tone_count], n);

        let idle = !running && !keyed && self.all_silent();
        if idle && self.quiet.resting() {
            silence(outputs, n);
            for l in 0..LAYERS {
                p.viz[viz::PEAK + l].set(0.0);
            }
            return;
        }

        // The layers.
        let mut buses = Buses {
            dry: [[0.0; BLOCK]; 2],
            echo: [[0.0; BLOCK]; 2],
            verb: [[0.0; BLOCK]; 2],
        };
        // The voices of a chord share a loudness (and tails keep the last one).
        let voices = if running {
            self.note_count
        } else if keyed {
            self.keys.notes().len()
        } else {
            0
        };
        if voices > 0 {
            self.chord_norm +=
                (1.0 / (voices as f32).sqrt() - self.chord_norm) * (1.0 - (-secs / 0.15).exp());
        }
        let chord_norm = self.chord_norm;
        for l in 0..LAYERS {
            let mut x = [[0.0f32; BLOCK]; 2];
            let knob_of = |w: usize| p.get(knob(l, w));
            let sounding = if l == SHIMMER {
                let any = self.pings.any();
                if any {
                    self.pings.render(&mut x, n);
                }
                any
            } else {
                let shape = Shape {
                    attack: knob_of(k::ATTACK),
                    release: knob_of(k::RELEASE),
                    hold: hold_time(&p, l),
                    drift: if l == BASS { 0.012 } else { 0.05 },
                    sway: if l == BASS { 0.04 } else { 0.16 },
                    spread: if l == PAD {
                        0.45 * knob_of(k::WIDTH)
                    } else {
                        0.0
                    },
                };
                self.samplers[l].render(&shape, &mut x, n, secs)
            };
            let strip = &mut self.strips[l];
            strip.note_activity(sounding);
            if strip.resting() {
                p.viz[viz::PEAK + l].set(0.0);
                continue;
            }
            let trim = match l {
                PAD => chord_norm,
                SHIMMER => PING_LEVEL,
                _ => 1.0,
            };
            let settings = StripSettings {
                first: Sweep {
                    kind: filter_kind(&p, data::KIND1 + l),
                    hz: knob_of(k::FREQ1),
                    depth: knob_of(k::MOVE1),
                    rate: knob_of(k::RATE1),
                    width: knob_of(k::WIDTH),
                },
                second: Sweep {
                    kind: filter_kind(&p, data::KIND2 + l),
                    hz: knob_of(k::FREQ2),
                    depth: knob_of(k::MOVE2),
                    rate: knob_of(k::RATE2),
                    width: knob_of(k::WIDTH),
                },
                bloom: knob_of(k::BLOOM),
                bloom_time: knob_of(k::ATTACK),
                drive: knob_of(k::DRIVE),
                chorus: knob_of(k::CHORUS),
                width: knob_of(k::WIDTH),
                level: if flag(&p, data::MUTE + l) {
                    0.0
                } else {
                    knob_of(k::LEVEL) * trim
                },
                echo: knob_of(k::ECHO),
                verb: knob_of(k::REVERB),
            };
            let (peak, f1, f2) = strip.process(&mut x, &settings, &mut buses, n, sr);
            let shown = p.viz[viz::PEAK + l].get();
            let fall = (-secs / 0.2).exp();
            p.viz[viz::PEAK + l].set((peak * settings.level).min(1.0).max(shown * fall));
            p.viz[viz::FILTER1 + l].set(f1);
            p.viz[viz::FILTER2 + l].set(f2);
        }

        // What they share.
        let mix_type = (p.get(MIXTYPE).round().max(0.0) as usize).min(MIX_TYPES.len() - 1);
        self.master.begin(mix_type, n);
        let shape = self.master.shape();
        let spb = 1.0 / (ctx.beats_per_sample.max(1e-9) * sr as f64);
        let echo_time = ((sync_division(p.get(ECHO)).1 * spb) as f32).min(echo::LONGEST);
        let first = !self.started;
        self.started = true;
        self.echo.set(
            echo_time,
            p.get(REPEATS),
            true,
            EchoShape {
                drive: 0.0,
                tone: 1800.0,
                width: 0.9,
                level: 1.0,
            },
            first,
        );
        let space = self.space.settings(
            (p.get(SPACE) + shape.space).clamp(0.0, 1.0),
            0.55,
            shape.air,
            [1000.0, 1500.0],
            0.0,
        );
        let dry_share = space.dry();

        let attack = 1.0 - (-1.0 / (0.0004 * sr)).exp();
        let release = 1.0 / sr;
        let level_t = p.get(LEVEL);
        if first {
            self.level = level_t;
        }
        let mut peak = 0.0f32;
        let mut tail = 0.0f32;
        for i in 0..n {
            self.level += (level_t - self.level) * ctx.smooth;
            let (el, er) = self.echo.process(buses.echo[0][i], buses.echo[1][i]);
            let send = [buses.verb[0][i], buses.verb[1][i]];
            let (y, t) = self.space.process(send, &space);
            tail = tail.max(t).max(el.abs()).max(er.abs());
            let l = buses.dry[0][i] + el + y[0] - send[0] * dry_share;
            let r = buses.dry[1][i] + er + y[1] - send[1] * dry_share;
            let [l, r] = self.master.process([l, r]);
            let (l, r) = (l * self.level, r * self.level);
            let a = l.abs().max(r.abs());
            self.limit += (a - self.limit) * if a > self.limit { attack } else { release };
            let g = if self.limit > 1.0 {
                1.0 / self.limit
            } else {
                1.0
            };
            let (l, r) = (
                soft_limit(l * g).clamp(-1.0, 1.0),
                soft_limit(r * g).clamp(-1.0, 1.0),
            );
            outputs[0][0][i] = l;
            outputs[0][1][i] = r;
            peak = peak.max(l.abs()).max(r.abs());
        }
        let layers_rest = self.strips.iter().all(Strip::resting);
        self.quiet.update(idle && layers_rest, tail.max(peak), n);
        p.meter.fetch_max(peak);
    }

    fn midi_out(&self) -> Option<&MidiBuf> {
        Some(&self.out)
    }
}

impl Vesper {
    /// No voice sounds on any layer.
    fn all_silent(&self) -> bool {
        !self.pings.any() && !self.samplers.iter().any(Sampler::any)
    }
}
