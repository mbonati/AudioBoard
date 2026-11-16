//! Iris: a video node. It draws visuals that react to sound, right on the
//! board: geometric, mathematical and generative scenes (see `ui::iris`).
//! It listens to its IN or to the board's main mix, and passes IN on to
//! THRU unchanged, so it can sit inside a chain.
//!
//! The audio thread does almost nothing here: it copies the sound it
//! listens to into a ring of samples in `viz`, and the panel analyses it
//! (spectrum, bands, onsets) and draws on the UI thread.

use std::sync::Arc;

use super::{ModuleDef, ModuleParams};
use crate::audio::engine::{Buf, Inputs, ProcessCtx, Processor};
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
pub const SCENES: [&str; 18] = [
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
];

/// Extra settings, saved with the node.
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
}

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
    pub const LEN: usize = RING + super::RING;
}

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
    midi_in: false,
    midi_out: false,
    data: &[DEFAULT_SIZE[0], DEFAULT_SIZE[1], 0.0, 0.0, 0.0, 0.0, 0.0],
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
    /// Samples written into the ring.
    written: u32,
    /// The main mix's samples already copied (see `Scope::written`).
    seen: Option<usize>,
}

impl Iris {
    pub fn new(params: Arc<ModuleParams>, shared: Arc<EngineShared>, sample_rate: f32) -> Self {
        params.viz[viz::SR].set(sample_rate);
        Self { params, shared, written: 0, seen: None }
    }

    #[inline]
    fn push(&mut self, s: f32) {
        self.params.viz[viz::RING + self.written as usize % RING].set(s);
        self.written = (self.written + 1) % COUNT;
    }
}

impl Processor for Iris {
    fn process(&mut self, _ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
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
            let Self { params, shared, written, seen } = self;
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
