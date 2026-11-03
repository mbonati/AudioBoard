//! MIDI In: the MIDI keyboard on a MIDI cable, so it can be patched into
//! an arpeggiator or a VST instrument. CHANNEL picks one channel (or all),
//! TRANSPOSE shifts the notes; controllers and pitch bend pass through.

use std::sync::Arc;

use super::steps::{Held, publish_notes, viz};
use super::{ModuleDef, ModuleParams};
use crate::audio::engine::{Buf, Inputs, MidiBuf, ProcessCtx, Processor};
use crate::audio::midi::MidiState;
use crate::audio::params::{ParamSpec, Unit};

pub const CHANNEL: usize = 0;
pub const TRANSPOSE: usize = 1;

const PARAMS: [ParamSpec; 2] = [
    ParamSpec::lin("CHANNEL", 0.0, 16.0, 0.0, Unit::Channel),
    ParamSpec::lin("TRANSPOSE", -24.0, 24.0, 0.0, Unit::Semitones).bipolar(),
];

pub static DEF: ModuleDef = ModuleDef {
    name: "MIDI In",
    prefix: "MIDI IN",
    hint: "keyboard → MIDI cable",
    inputs: &[],
    outputs: &[],
    params: &PARAMS,
    cv_depth: &[],
    modes: &[],
    toggle: None,
    toggle_default: false,
    map: false,
    midi_in: false,
    midi_out: true,
    data: &[],
    display_h: 42.0,
    controls_h: 0.0,
    viz: viz::LEN,
};

pub struct MidiIn {
    params: Arc<ModuleParams>,
    midi: Arc<MidiState>,
    cursor: u32,
    out: MidiBuf,
    /// The note sent for each key held, so its note-off matches even if
    /// TRANSPOSE moved meanwhile.
    sent: [Option<u8>; 128],
    held: Held,
}

impl MidiIn {
    pub fn new(params: Arc<ModuleParams>, midi: Arc<MidiState>) -> Self {
        Self {
            params,
            cursor: midi.stream_head(),
            midi,
            out: MidiBuf::default(),
            sent: [None; 128],
            held: Held::new(),
        }
    }
}

impl Processor for MidiIn {
    fn process(&mut self, _ctx: &ProcessCtx, _inputs: &Inputs, _outputs: &mut [Buf], _n: usize) {
        self.out.clear();
        let p = &self.params;
        let channel = p.get(CHANNEL).round() as u8;
        let transpose = p.get(TRANSPOSE).round() as i32;
        let Self { midi, cursor, out, sent, held, .. } = self;
        midi.read_stream(cursor, |[status, d1, d2]| {
            // The clock and transport are on no channel: they always pass.
            if channel > 0 && status < 0xF0 && status & 0x0F != channel - 1 {
                return;
            }
            let key = d1 as usize & 0x7F;
            match status & 0xF0 {
                0x90 if d2 > 0 => {
                    let note = (d1 as i32 + transpose).clamp(0, 127) as u8;
                    sent[key] = Some(note);
                    held.press(note);
                    out.push(0, [status, note, d2]);
                }
                0x80 | 0x90 => {
                    if let Some(note) = sent[key].take() {
                        held.release(note);
                        out.push(0, [status, note, d2]);
                    }
                }
                0xA0 => {
                    if let Some(note) = sent[key] {
                        out.push(0, [status, note, d2]);
                    }
                }
                _ => out.push(0, [status, d1, d2]),
            }
        });
        publish_notes(p, held.notes().last().copied(), held.notes());
    }

    fn midi_out(&self) -> Option<&MidiBuf> {
        Some(&self.out)
    }
}
