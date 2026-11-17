//! Iris: a video node. It draws visuals that react to sound, right on the
//! board: geometric, mathematical, generative and 3D scenes (see
//! `ui::iris`), in up to four layers with LFOs, envelopes, audio and MIDI
//! modulators routed by a matrix, like a visual synthesizer. It listens to
//! its IN or to the board's main mix, and passes IN on to THRU unchanged,
//! so it can sit inside a chain; MIDI notes and CCs come in on its MIDI
//! input, or from the keyboards while that is free.
//!
//! The audio thread does almost nothing here: it copies the sound it
//! listens to into a ring of samples in `viz`, with the transport and the
//! MIDI it receives, and the panel analyses it (spectrum, bands, onsets),
//! runs the modulators and draws on the UI thread.

use std::sync::Arc;

use super::{ModuleDef, ModuleParams};
use crate::audio::engine::{Buf, Inputs, ProcessCtx, Processor};
use crate::audio::midi::MidiState;
use crate::audio::params::{EngineShared, ParamSpec, Unit};

/// How strongly the visuals follow the sound.
pub const REACT: usize = 0;
/// How fast they move.
pub const SPEED: usize = 1;
/// How much there is: lines, particles, cells, symmetry.
pub const DETAIL: usize = 2;
/// How slowly the analysis lets go (falls back after a peak).
pub const SMOOTH: usize = 3;
/// How long trails, echoes and histories are.
pub const TRAIL: usize = 4;

const PARAMS: [ParamSpec; 5] = [
    ParamSpec::lin("REACT", 0.0, 2.0, 1.0, Unit::Percent),
    ParamSpec::lin("SPEED", 0.0, 2.0, 1.0, Unit::Percent),
    ParamSpec::lin("DETAIL", 0.0, 1.0, 0.5, Unit::Percent),
    ParamSpec::lin("SMOOTH", 0.0, 1.0, 0.5, Unit::Percent),
    ParamSpec::lin("TRAIL", 0.0, 1.0, 0.5, Unit::Percent),
];

/// The scenes, in the order of the mode (saved by name; new ones go at the
/// end).
pub const SCENES: [&str; 19] = [
    "RIDGES",
    "MANDALA",
    "HARMONOGRAPH",
    "SPIRO",
    "TUNNEL",
    "FLOW",
    "LIFE",
    "AUTOMATON",
    "TRUCHET",
    "MOIRE",
    "ATTRACTOR",
    "PHASE",
    "DOTS",
    "ORBITS",
    "HALO",
    "GLOBE",
    "TERRAIN",
    "FLIGHT",
    "SCOPE",
];

/// Extra settings, saved with the node. Projects saved before a slot
/// existed load it at its default, so every default here leaves the
/// picture as it was: layers 2 to 4 off, layer 1 as it is, the matrix
/// empty.
pub mod data {
    /// The panel's size, in board units (the user resizes it).
    pub const W: usize = 0;
    pub const H: usize = 1;
    /// What it listens to: [`super::AUTO`], [`super::INPUT`] or
    /// [`super::MAIN`].
    pub const SOURCE: usize = 2;
    /// Index into the panel's palettes.
    pub const PALETTE: usize = 3;
    /// Moves on to the next scene every so many bars (0: never).
    pub const CYCLE: usize = 4;
    /// Varies the generative scenes.
    pub const SEED: usize = 5;
    /// 1: no frame, only the picture, as if stuck on the board.
    pub const FRAMELESS: usize = 6;

    /// The layers, drawn in order (layer 1 first, under the others); each
    /// is [`LAYER_LEN`] slots from `LAYER + k * LAYER_LEN` (see [`layer`]).
    /// Layer 1's scene is the node's mode.
    pub const LAYERS: usize = 4;
    pub const LAYER: usize = 7;
    pub const LAYER_LEN: usize = 15;
    pub mod layer {
        /// 1: drawn.
        pub const ON: usize = 0;
        /// Index into `SCENES` (layers 2 to 4).
        pub const SCENE: usize = 1;
        /// 0..1.
        pub const OPACITY: usize = 2;
        /// 0: normal, 1: add (light adds up, for dark palettes).
        pub const BLEND: usize = 3;
        /// Scale, 0.25..4.
        pub const ZOOM: usize = 4;
        /// Turns, -0.5..0.5.
        pub const ROTATE: usize = 5;
        /// Turns a second, -1..1.
        pub const SPIN: usize = 6;
        /// Offset, in half screens, -1..1.
        pub const X: usize = 7;
        pub const Y: usize = 8;
        /// Turns of the colour wheel, -0.5..0.5.
        pub const HUE: usize = 9;
        /// Index into `MIRRORS`.
        pub const MIRROR: usize = 10;
        /// Echoes of past frames behind it (feedback), 0..8.
        pub const ECHO: usize = 11;
        /// How much each echo grows, 0.8..1.25 (above 1: an endless zoom).
        pub const FEEDBACK: usize = 12;
        /// 1: only the soloed layers are drawn.
        pub const SOLO: usize = 13;
        /// Its own speed, times SPEED, 0..4.
        pub const SPEED: usize = 14;
    }

    /// LFOs: [`LFO_LEN`] slots each from `LFO + k * LFO_LEN` (see [`lfo`]).
    pub const LFOS: usize = 4;
    pub const LFO: usize = LAYER + LAYERS * LAYER_LEN;
    pub const LFO_LEN: usize = 4;
    pub mod lfo {
        /// Index into `LFO_SHAPES`.
        pub const SHAPE: usize = 0;
        /// Free: Hz (0.02..10). Synced: index into `SYNC_BEATS`.
        pub const RATE: usize = 1;
        /// 1: synced to the transport.
        pub const SYNC: usize = 2;
        /// 0..1 of a cycle.
        pub const PHASE: usize = 3;
    }

    /// Envelopes: [`ENV_LEN`] slots each from `ENV + k * ENV_LEN`.
    pub const ENVS: usize = 2;
    pub const ENV: usize = LFO + LFOS * LFO_LEN;
    pub const ENV_LEN: usize = 5;
    pub mod env {
        /// Seconds.
        pub const ATTACK: usize = 0;
        pub const DECAY: usize = 1;
        /// 0..1.
        pub const SUSTAIN: usize = 2;
        pub const RELEASE: usize = 3;
        /// Index into `ENV_TRIGGERS`.
        pub const TRIGGER: usize = 4;
    }

    /// Audio modulators: a range of the 32 bands and a gain.
    pub const AUDIO_MODS: usize = 4;
    pub const AUDIO: usize = ENV + ENVS * ENV_LEN;
    pub const AUDIO_LEN: usize = 3;
    pub mod audio {
        /// Bands 0..31.
        pub const FROM: usize = 0;
        pub const TO: usize = 1;
        /// 0..4.
        pub const GAIN: usize = 2;
    }

    /// The controller numbers read by the CC A and CC B sources.
    pub const CC: usize = AUDIO + AUDIO_MODS * AUDIO_LEN;
    pub const CCS: usize = 2;

    /// The matrix: a source, a destination and a bipolar amount a slot.
    pub const SLOTS: usize = 12;
    pub const MATRIX: usize = CC + CCS;
    pub const SLOT_LEN: usize = 3;
    pub mod slot {
        /// Index into the panel's sources (0: none).
        pub const SOURCE: usize = 0;
        /// Index into the panel's destinations (0: none).
        pub const DEST: usize = 1;
        /// -1..1.
        pub const AMOUNT: usize = 2;
    }

    pub const LEN: usize = MATRIX + SLOTS * SLOT_LEN;
}

/// Layer mirrors: off, mirrored left to right, in four quadrants, three
/// and six turned copies.
pub const MIRRORS: [&str; 5] = ["Off", "Mirror", "Quad", "×3", "×6"];
pub const LFO_SHAPES: [&str; 5] = ["Sine", "Triangle", "Saw", "Square", "Random"];
/// Synced LFO periods, in beats, and their names.
pub const SYNC_BEATS: [f32; 8] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
pub const SYNC_NAMES: [&str; 8] = ["1/16", "1/8", "1/4", "1/2", "1 bar", "2 bars", "4 bars", "8 bars"];
pub const ENV_TRIGGERS: [&str; 3] = ["MIDI notes", "Onsets", "Both"];

/// Every extra setting at its default.
const DATA: [f32; data::LEN] = {
    use data::*;
    let mut d = [0.0; LEN];
    d[W] = DEFAULT_SIZE[0];
    d[H] = DEFAULT_SIZE[1];
    // Layers 2 to 4 wait, off, with a scene, adding light.
    const SCENES_2_4: [f32; 3] = [14.0, 1.0, 4.0];
    let mut k = 0;
    while k < LAYERS {
        let b = LAYER + k * LAYER_LEN;
        d[b + layer::ON] = if k == 0 { 1.0 } else { 0.0 };
        d[b + layer::SCENE] = if k == 0 { 0.0 } else { SCENES_2_4[k - 1] };
        d[b + layer::OPACITY] = if k == 0 { 1.0 } else { 0.8 };
        d[b + layer::BLEND] = if k == 0 { 0.0 } else { 1.0 };
        d[b + layer::ZOOM] = 1.0;
        d[b + layer::FEEDBACK] = 1.06;
        d[b + layer::SPEED] = 1.0;
        k += 1;
    }
    let mut k = 0;
    while k < LFOS {
        let b = LFO + k * LFO_LEN;
        d[b + lfo::SHAPE] = k as f32;
        d[b + lfo::RATE] = if k % 2 == 0 { 0.25 } else { 4.0 };
        d[b + lfo::SYNC] = (k % 2) as f32;
        k += 1;
    }
    let mut k = 0;
    while k < ENVS {
        let b = ENV + k * ENV_LEN;
        d[b + env::ATTACK] = if k == 0 { 0.01 } else { 0.4 };
        d[b + env::DECAY] = if k == 0 { 0.3 } else { 1.0 };
        d[b + env::SUSTAIN] = if k == 0 { 0.0 } else { 0.6 };
        d[b + env::RELEASE] = if k == 0 { 0.4 } else { 1.5 };
        d[b + env::TRIGGER] = if k == 0 { 2.0 } else { 0.0 };
        k += 1;
    }
    // Bass, mids, highs and everything.
    const RANGES: [[f32; 2]; 4] = [[0.0, 7.0], [8.0, 19.0], [20.0, 31.0], [0.0, 31.0]];
    let mut k = 0;
    while k < AUDIO_MODS {
        let b = AUDIO + k * AUDIO_LEN;
        d[b + audio::FROM] = RANGES[k][0];
        d[b + audio::TO] = RANGES[k][1];
        d[b + audio::GAIN] = 1.0;
        k += 1;
    }
    d[CC] = 2.0;
    d[CC + 1] = 74.0;
    d
};

/// Size of a new Iris, and the smallest and largest it can be made.
pub const DEFAULT_SIZE: [f32; 2] = [480.0, 330.0];
pub const MIN_SIZE: [f32; 2] = [240.0, 170.0];
pub const MAX_SIZE: [f32; 2] = [2400.0, 1600.0];

/// Samples of sound handed to the panel.
pub const RING: usize = 4096;
/// `viz[WRITTEN]` counts samples modulo this (exact in an f32).
pub const COUNT: u32 = 1 << 20;

pub mod viz {
    /// Samples written so far, modulo [`super::COUNT`].
    pub const WRITTEN: usize = 0;
    pub const SR: usize = 1;
    /// 1 while sound arrives from the chosen source (0: IN unpatched).
    pub const LIVE: usize = 2;
    /// What it listens to now: 1 its IN, 0 the main mix.
    pub const FROM_INPUT: usize = 3;
    /// The ring: sample `n` sits at `RING + n % super::RING`.
    pub const RING: usize = 4;
    /// The transport: its position in beats (modulo [`super::BEATS`]),
    /// and 1 while it plays.
    pub const BEAT: usize = RING + super::RING;
    pub const PLAYING: usize = BEAT + 1;
    /// Notes played so far (modulo [`super::COUNT`]): a change is a new
    /// note, however short.
    pub const NOTES: usize = BEAT + 2;
    /// The latest note (0..127) and its velocity (0..1).
    pub const NOTE: usize = BEAT + 3;
    pub const VELOCITY: usize = BEAT + 4;
    /// How many notes are held.
    pub const HELD: usize = BEAT + 5;
    /// The 128 controllers, 0..1.
    pub const CC: usize = BEAT + 6;
    pub const LEN: usize = CC + 128;
}

/// `viz[BEAT]` counts beats modulo this (a whole number of every synced
/// LFO period, and exact enough in an f32).
pub const BEATS: f64 = 1024.0;

/// Listens to IN when a cable is plugged into it, to the main mix otherwise.
pub const AUTO: f32 = 0.0;
/// Always IN (nothing while unpatched).
pub const INPUT: f32 = 1.0;
/// Always the main mix, even with IN patched (IN still goes on to THRU).
pub const MAIN: f32 = 2.0;

pub static DEF: ModuleDef = ModuleDef {
    name: "Iris",
    prefix: "IRIS",
    hint: "visuals that react to sound",
    inputs: &["IN"],
    outputs: &["THRU"],
    params: &PARAMS,
    cv_depth: &[],
    modes: &SCENES,
    toggle: None,
    toggle_default: false,
    map: false,
    midi_in: true,
    midi_out: false,
    data: &DATA,
    display_h: 0.0,
    controls_h: 0.0,
    viz: viz::LEN,
};

/// Listens to IN (true) or to the main mix, with IN `patched` or not.
pub fn listens_to_input(p: &ModuleParams, patched: bool) -> bool {
    let source = p.data[data::SOURCE].get().round();
    if source == INPUT {
        true
    } else if source == MAIN {
        false
    } else {
        patched
    }
}

pub struct Iris {
    params: Arc<ModuleParams>,
    shared: Arc<EngineShared>,
    midi: Arc<MidiState>,
    midi_cursor: u32,
    /// Samples written into the ring.
    written: u32,
    /// The main mix's samples already copied (see `Scope::written`).
    seen: Option<usize>,
    notes: u32,
    held: [bool; 128],
}

impl Iris {
    pub fn new(params: Arc<ModuleParams>, shared: Arc<EngineShared>, midi: Arc<MidiState>, sample_rate: f32) -> Self {
        params.viz[viz::SR].set(sample_rate);
        let midi_cursor = midi.stream_head();
        Self { params, shared, midi, midi_cursor, written: 0, seen: None, notes: 0, held: [false; 128] }
    }

    /// Notes and controllers, for the panel's MIDI sources.
    fn midi_message(&mut self, m: [u8; 3]) {
        let v = &self.params.viz;
        let (note, value) = (m[1] as usize & 127, m[2]);
        match m[0] & 0xF0 {
            0x90 if value > 0 => {
                self.notes = (self.notes + 1) % COUNT;
                self.held[note] = true;
                v[viz::NOTES].set(self.notes as f32);
                v[viz::NOTE].set(note as f32);
                v[viz::VELOCITY].set(value as f32 / 127.0);
            }
            0x80 | 0x90 => self.held[note] = false,
            0xB0 => {
                v[viz::CC + note].set(value as f32 / 127.0);
                // All notes off, all sound off.
                if note == 123 || note == 120 {
                    self.held = [false; 128];
                }
            }
            _ => {}
        }
    }

    #[inline]
    fn push(&mut self, s: f32) {
        self.params.viz[viz::RING + self.written as usize % RING].set(s);
        self.written = (self.written + 1) % COUNT;
    }
}

impl Processor for Iris {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let input = inputs.get(0);
        // THRU: the input as it is.
        let out = &mut outputs[0];
        match input {
            Some(b) => {
                out[0][..n].copy_from_slice(&b[0][..n]);
                out[1][..n].copy_from_slice(&b[1][..n]);
            }
            None => {
                out[0][..n].fill(0.0);
                out[1][..n].fill(0.0);
            }
        }
        // MIDI: the cable if patched, else the keyboards.
        if inputs.midi_connected {
            for e in inputs.midi.events() {
                self.midi_message(e.data);
            }
            self.midi_cursor = self.midi.stream_head();
        } else {
            let (midi, mut cursor) = (self.midi.clone(), self.midi_cursor);
            midi.read_stream(&mut cursor, |m| self.midi_message(m));
            self.midi_cursor = cursor;
        }
        let held = self.held.iter().filter(|&&h| h).count();
        self.params.viz[viz::HELD].set(held as f32);
        self.params.viz[viz::BEAT].set(ctx.beat.rem_euclid(BEATS) as f32);
        self.params.viz[viz::PLAYING].set(if ctx.playing { 1.0 } else { 0.0 });

        let from_input = listens_to_input(&self.params, input.is_some());
        self.params.viz[viz::FROM_INPUT].set(if from_input { 1.0 } else { 0.0 });
        let live = if from_input {
            self.seen = None;
            match input {
                Some(b) => {
                    let mut peak = 0.0f32;
                    for i in 0..n {
                        let s = 0.5 * (b[0][i] + b[1][i]);
                        peak = peak.max(s.abs());
                        self.push(s);
                    }
                    self.params.meter.fetch_max(peak);
                    true
                }
                None => false,
            }
        } else {
            // The main mix of the passes before this one: the scope is
            // written once a pass, after every node has run.
            let Self { params, shared, written, seen, .. } = self;
            let scope = &shared.scope;
            let end = scope.written();
            let from = match *seen {
                Some(s) if end.wrapping_sub(s) <= RING => s,
                _ => end.wrapping_sub(RING.min(crate::audio::params::Scope::LEN)),
            };
            let mut i = from;
            let mut peak = 0.0f32;
            while i != end {
                let s = scope.sample(i);
                peak = peak.max(s.abs());
                params.viz[viz::RING + *written as usize % RING].set(s);
                *written = (*written + 1) % COUNT;
                i = i.wrapping_add(1);
            }
            *seen = Some(end);
            params.meter.fetch_max(peak);
            true
        };
        self.params.viz[viz::WRITTEN].set(self.written as f32);
        self.params.viz[viz::LIVE].set(if live { 1.0 } else { 0.0 });
    }
}
