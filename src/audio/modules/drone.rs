//! Drone: eight detuned voices stacked into a chord, each drifting slowly in
//! pitch and level so the sound never sits still.

use std::f32::consts::TAU;
use std::sync::Arc;

use super::{ModuleDef, ModuleParams, Rng, Wander, approach, pan_gains, soft_limit};
use crate::audio::dsp::poly_blep;
use crate::audio::engine::{Buf, Inputs, ProcessCtx, Processor};
use crate::audio::params::{ParamSpec, Unit};

pub const PITCH: usize = 0;
pub const DETUNE: usize = 1;
pub const DRIFT: usize = 2;
pub const TONE: usize = 3;
pub const SPREAD: usize = 4;
pub const PITCH_MOD: usize = 5;
pub const TONE_MOD: usize = 6;
pub const LEVEL: usize = 7;

const PARAMS: [ParamSpec; 8] = [
    ParamSpec::log("PITCH", 20.0, 2000.0, 55.0, Unit::Hz),
    ParamSpec::lin("DETUNE", 0.0, 50.0, 12.0, Unit::Cents),
    ParamSpec::lin("DRIFT", 0.0, 1.0, 0.35, Unit::Percent),
    ParamSpec::lin("TONE", 0.0, 1.0, 0.35, Unit::Percent),
    ParamSpec::lin("SPREAD", 0.0, 1.0, 0.7, Unit::Percent),
    ParamSpec::lin("P.MOD", 0.0, 2.0, 0.0, Unit::Octaves),
    ParamSpec::lin("T.MOD", 0.0, 1.0, 0.0, Unit::Percent),
    ParamSpec::lin("LEVEL", 0.0, 1.0, 0.5, Unit::Decibel),
];

pub const VOICES: usize = 8;

/// Chord shapes: one interval (semitones) per voice.
pub const CHORDS: [(&str, [f32; VOICES]); 6] = [
    ("Unison", [0.0; VOICES]),
    ("Fifths", [0.0, 7.0, 0.0, 7.0, 12.0, 19.0, -12.0, 0.0]),
    ("Octaves", [0.0, 12.0, 0.0, -12.0, 12.0, 24.0, 0.0, 12.0]),
    ("Major", [0.0, 4.0, 7.0, 12.0, 16.0, 19.0, -12.0, 24.0]),
    ("Minor", [0.0, 3.0, 7.0, 12.0, 15.0, 19.0, -12.0, 24.0]),
    ("Suspended", [0.0, 5.0, 7.0, 12.0, 14.0, 19.0, -12.0, 24.0]),
];

pub static DEF: ModuleDef = ModuleDef {
    name: "Drone",
    prefix: "DRONE",
    hint: "detuned chord",
    inputs: &["PITCH", "TONE"],
    params: &PARAMS,
    cv_depth: &[(PITCH_MOD, 0), (TONE_MOD, 1)],
    modes: &["UNI", "5TH", "OCT", "MAJ", "MIN", "SUS"],
    toggle: None,
    viz: 0,
};

/// Where each voice sits within the detune spread and the stereo field.
const DETUNE_POS: [f32; VOICES] = [-1.0, 1.0, -0.55, 0.55, -0.2, 0.2, -0.8, 0.8];
const PAN_POS: [f32; VOICES] = [-1.0, 1.0, -0.6, 0.6, -0.25, 0.25, -0.8, 0.8];

pub struct Drone {
    params: Arc<ModuleParams>,
    sr: f32,
    rng: Rng,
    phase: [f32; VOICES],
    pitch_drift: [Wander; VOICES],
    level_drift: [Wander; VOICES],
    pitch: f32,
    tone: f32,
    level: f32,
    lp: [f32; 2],
}

impl Drone {
    pub fn new(params: Arc<ModuleParams>, sample_rate: f32) -> Self {
        let mut rng = Rng::new(0x5EED_D201);
        let phase = std::array::from_fn(|_| rng.unit());
        let pitch_drift = std::array::from_fn(|_| Wander::new(&mut rng));
        let level_drift = std::array::from_fn(|_| Wander::new(&mut rng));
        Self {
            pitch: params.get(PITCH),
            tone: params.get(TONE),
            level: 0.0,
            params,
            sr: sample_rate,
            rng,
            phase,
            pitch_drift,
            level_drift,
            lp: [0.0; 2],
        }
    }
}

impl Processor for Drone {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = &self.params;
        let chord = CHORDS[p.mode().min(CHORDS.len() - 1)].1;
        let detune = p.get(DETUNE);
        let drift = p.get(DRIFT);
        let spread = p.get(SPREAD);
        let (pitch_t, tone_t, level_t) = (p.get(PITCH), p.get(TONE), p.get(LEVEL));
        let (pitch_mod, tone_mod) = (p.get(PITCH_MOD), p.get(TONE_MOD));
        let pitch_cv = inputs.cv(0);
        let tone_cv = inputs.cv(1);

        // Per-block voice tuning, drift and placement.
        let mut ratio = [0.0f32; VOICES];
        let mut gl = [0.0f32; VOICES];
        let mut gr = [0.0f32; VOICES];
        let block_secs = n as f32 / self.sr;
        for v in 0..VOICES {
            let rate = 0.07 + 0.025 * v as f32; // segments per second
            let pd = self.pitch_drift[v].tick(&mut self.rng, rate * block_secs);
            let ld = self.level_drift[v].tick(&mut self.rng, rate * 0.7 * block_secs);
            let cents = chord[v] * 100.0 + detune * DETUNE_POS[v] + drift * 30.0 * pd;
            ratio[v] = (cents / 1200.0).exp2();
            let amp = 1.0 + drift * 0.4 * ld;
            let (l, r) = pan_gains(PAN_POS[v] * spread);
            gl[v] = l * amp;
            gr[v] = r * amp;
        }

        // Eight incoherent voices: about the loudness of one oscillator.
        let norm = 1.5 / (VOICES as f32).sqrt();
        let mut peak = 0.0f32;
        for i in 0..n {
            approach(&mut self.pitch, pitch_t, ctx.smooth);
            approach(&mut self.tone, tone_t, ctx.smooth);
            approach(&mut self.level, level_t, ctx.smooth);

            let mut f = self.pitch;
            if let Some(cv) = pitch_cv {
                f *= (cv[i].clamp(-1.0, 1.0) * pitch_mod).exp2();
            }
            let tone = match tone_cv {
                Some(cv) => (self.tone + cv[i] * tone_mod).clamp(0.0, 1.0),
                None => self.tone,
            };
            let saw_mix = tone * tone;

            let (mut l, mut r) = (0.0, 0.0);
            for v in 0..VOICES {
                let dt = (f * ratio[v] / self.sr).min(0.45);
                let t = self.phase[v];
                let sine = (t * TAU).sin();
                let saw = 2.0 * t - 1.0 - poly_blep(t, dt);
                let s = sine + (saw - sine) * saw_mix;
                l += s * gl[v];
                r += s * gr[v];
                self.phase[v] += dt;
                if self.phase[v] >= 1.0 {
                    self.phase[v] -= 1.0;
                }
            }

            // Warmth: a gentle low-pass that opens with TONE.
            let fc = 150.0 * (tone * 7.0).exp2();
            let a = (TAU * fc / self.sr).min(1.0);
            self.lp[0] += a * (l - self.lp[0]);
            self.lp[1] += a * (r - self.lp[1]);

            let g = self.level * norm;
            let (yl, yr) = (soft_limit(self.lp[0] * g), soft_limit(self.lp[1] * g));
            outputs[0][0][i] = yl;
            outputs[0][1][i] = yr;
            peak = peak.max(yl.abs()).max(yr.abs());
        }
        self.params.meter.fetch_max(peak);
    }
}
