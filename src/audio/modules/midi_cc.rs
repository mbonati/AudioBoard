//! MIDI CC: an external controller (knob, fader, mod wheel…) as a control
//! signal. LEARN picks up the next controller moved; the node can also drive
//! any knob directly (see the MAP button in the UI).

use std::sync::Arc;

use super::{ModuleDef, ModuleParams, approach, time_coeff};
use crate::audio::engine::{Buf, Inputs, ProcessCtx, Processor};
use crate::audio::midi::MidiState;
use crate::audio::params::{ParamSpec, Unit};

pub const CC: usize = 0;
pub const CHANNEL: usize = 1;
pub const SMOOTH: usize = 2;

const PARAMS: [ParamSpec; 3] = [
    ParamSpec::lin("CC", 0.0, 127.0, 1.0, Unit::Integer),
    ParamSpec::lin("CHANNEL", 0.0, 16.0, 0.0, Unit::Channel),
    ParamSpec::lin("SMOOTH", 0.0, 1.0, 0.2, Unit::Percent),
];

pub const UNIPOLAR: usize = 0;

/// `viz[VALUE]`: the current output.
pub const VALUE: usize = 0;

pub static DEF: ModuleDef = ModuleDef {
    name: "MIDI CC",
    prefix: "CC",
    hint: "controller → signal",
    inputs: &[],
    outputs: &["OUT"],
    params: &PARAMS,
    cv_depth: &[],
    modes: &["0…1", "±1"],
    toggle: Some("LEARN"),
    toggle_default: false,
    map: true,
    midi_in: false,
    midi_out: false,
    data: &[],
    display_h: 42.0,
    controls_h: 0.0,
    viz: 1,
};

/// The (channel, controller) a node listens to.
pub fn address(p: &ModuleParams) -> (usize, usize) {
    (p.get(CHANNEL).round() as usize, p.get(CC).round() as usize)
}

pub struct MidiCc {
    params: Arc<ModuleParams>,
    midi: Arc<MidiState>,
    sr: f32,
    value: f32,
}

impl MidiCc {
    pub fn new(params: Arc<ModuleParams>, midi: Arc<MidiState>, sample_rate: f32) -> Self {
        Self {
            params,
            midi,
            sr: sample_rate,
            value: 0.0,
        }
    }
}

impl Processor for MidiCc {
    fn process(&mut self, _ctx: &ProcessCtx, _inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = &self.params;
        let (channel, number) = address(p);
        let (target, _) = self.midi.cc(channel, number);
        // SMOOTH: none up to half a second of glide.
        let smooth = p.get(SMOOTH);
        let k = if smooth < 1e-3 {
            1.0
        } else {
            time_coeff(smooth * smooth * 0.5, self.sr)
        };
        let bipolar = p.mode() != UNIPOLAR;

        let mut y = 0.0;
        let [out_l, out_r] = &mut outputs[0];
        for (l, r) in out_l[..n].iter_mut().zip(&mut out_r[..n]) {
            approach(&mut self.value, target, k);
            y = if bipolar {
                self.value * 2.0 - 1.0
            } else {
                self.value
            };
            *l = y;
            *r = y;
        }
        p.viz[VALUE].set(y);
        p.meter.fetch_max(y.abs());
    }
}
