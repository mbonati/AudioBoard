//! Modules for ambient, drone and granular sound: generators, texture
//! processors and effects. Each one is described by a [`ModuleDef`] (knobs,
//! ports, optional mode selector and toggle), stores its settings in the
//! generic [`ModuleParams`] and has a single stereo output.

pub mod aether;
pub mod arpeggiator;
pub mod audio_in;
pub mod automa;
pub mod colloid;
pub mod delay;
pub mod drone;
pub mod equalizer;
pub mod filter;
pub mod fjord;
pub mod frakture;
pub mod granular;
pub mod iris;
pub mod larsen;
pub mod looper;
pub mod magma;
pub mod midi_cc;
pub mod midi_in;
pub mod midicer;
pub mod pingpong;
pub mod pops;
pub mod prism;
pub mod midi_note;
pub mod nullpunkt;
pub mod orrery;
pub mod oversample;
pub mod random;
pub mod resonator;
pub mod reverb;
pub mod sequencer;
pub mod splitter;
pub mod steps;
pub mod tape;
pub mod vesper;

use std::sync::Arc;

use super::AudioHandle;
use super::engine::Processor;

/// A module's static description: the node SDK's `NodeDef` (see
/// `audioboard_node::def`).
pub use audioboard_node::NodeDef as ModuleDef;
/// The DSP helpers every node shares live in the node SDK.
pub use audioboard_node::dsp::{
    DelayLine, Quiet, Rng, Wander, approach, mix_gains, pan_gains, silence, soft_ceiling, soft_limit, time_coeff,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ModuleKind {
    Drone,
    Aether,
    Larsen,
    Vesper,
    Random,
    Pops,
    Fjord,
    Splitter,
    Nullpunkt,
    AudioIn,
    Granular,
    Frakture,
    Resonator,
    Filter,
    Magma,
    Equalizer,
    Prism,
    Tape,
    Delay,
    PingPong,
    Reverb,
    MidiCc,
    MidiNote,
    Sequencer,
    Arpeggiator,
    MidiIn,
    Midicer,
    Automa,
    Colloid,
    Orrery,
    Looper,
    Iris,
    /// A node from a node plugin (see `audio::node_plugin`).
    Ext(super::node_plugin::ExtId),
}

impl ModuleKind {
    pub const ALL: [ModuleKind; 32] = [
        ModuleKind::Drone,
        ModuleKind::Aether,
        ModuleKind::Larsen,
        ModuleKind::Vesper,
        ModuleKind::Random,
        ModuleKind::Pops,
        ModuleKind::Fjord,
        ModuleKind::Splitter,
        ModuleKind::Nullpunkt,
        ModuleKind::AudioIn,
        ModuleKind::Granular,
        ModuleKind::Frakture,
        ModuleKind::Resonator,
        ModuleKind::Filter,
        ModuleKind::Magma,
        ModuleKind::Equalizer,
        ModuleKind::Prism,
        ModuleKind::Tape,
        ModuleKind::Delay,
        ModuleKind::PingPong,
        ModuleKind::Reverb,
        ModuleKind::MidiCc,
        ModuleKind::MidiNote,
        ModuleKind::Sequencer,
        ModuleKind::Arpeggiator,
        ModuleKind::MidiIn,
        ModuleKind::Midicer,
        ModuleKind::Automa,
        ModuleKind::Colloid,
        ModuleKind::Orrery,
        ModuleKind::Looper,
        ModuleKind::Iris,
    ];

    pub fn def(self) -> &'static ModuleDef {
        match self {
            ModuleKind::Drone => &drone::DEF,
            ModuleKind::Aether => &aether::DEF,
            ModuleKind::Larsen => &larsen::DEF,
            ModuleKind::Vesper => &vesper::DEF,
            ModuleKind::Random => &random::DEF,
            ModuleKind::Pops => &pops::DEF,
            ModuleKind::Fjord => &fjord::DEF,
            ModuleKind::Splitter => &splitter::DEF,
            ModuleKind::Nullpunkt => &nullpunkt::DEF,
            ModuleKind::AudioIn => &audio_in::DEF,
            ModuleKind::Granular => &granular::DEF,
            ModuleKind::Frakture => &frakture::DEF,
            ModuleKind::Resonator => &resonator::DEF,
            ModuleKind::Filter => &filter::DEF,
            ModuleKind::Equalizer => &equalizer::DEF,
            ModuleKind::Prism => &prism::DEF,
            ModuleKind::Magma => &magma::DEF,
            ModuleKind::Tape => &tape::DEF,
            ModuleKind::Delay => &delay::DEF,
            ModuleKind::PingPong => &pingpong::DEF,
            ModuleKind::Reverb => &reverb::DEF,
            ModuleKind::MidiCc => &midi_cc::DEF,
            ModuleKind::MidiNote => &midi_note::DEF,
            ModuleKind::Sequencer => &sequencer::DEF,
            ModuleKind::Arpeggiator => &arpeggiator::DEF,
            ModuleKind::MidiIn => &midi_in::DEF,
            ModuleKind::Midicer => &midicer::DEF,
            ModuleKind::Automa => &automa::DEF,
            ModuleKind::Colloid => &colloid::DEF,
            ModuleKind::Orrery => &orrery::DEF,
            ModuleKind::Looper => &looper::DEF,
            ModuleKind::Iris => &iris::DEF,
            ModuleKind::Ext(e) => super::node_plugin::ext(e).def,
        }
    }

    /// The name a project saves the module by: built-in modules by name,
    /// plugin nodes by their stable id.
    pub fn key(self) -> &'static str {
        match self {
            ModuleKind::Ext(e) => super::node_plugin::ext(e).id,
            kind => kind.def().name,
        }
    }

    /// The module saved as `key` (see [`Self::key`]), if it is built in or
    /// its plugin is loaded.
    pub fn from_key(key: &str) -> Option<ModuleKind> {
        Self::ALL.iter().copied().find(|k| k.def().name == key).or_else(|| super::node_plugin::find(key).map(ModuleKind::Ext))
    }

    /// Builds the audio-thread processor. Called on the UI thread, so
    /// buffers can be allocated here.
    pub fn processor(self, params: Arc<ModuleParams>, audio: &AudioHandle) -> Box<dyn Processor> {
        let sample_rate = audio.info.sample_rate as f32;
        let midi = audio.midi.clone();
        match self {
            ModuleKind::Drone => Box::new(drone::Drone::new(params, sample_rate)),
            ModuleKind::Aether => Box::new(aether::Aether::new(params, midi.clone(), sample_rate)),
            ModuleKind::Larsen => Box::new(larsen::Larsen::new(params, midi.clone(), sample_rate)),
            ModuleKind::Vesper => Box::new(vesper::Vesper::new(params, midi.clone(), sample_rate)),
            ModuleKind::Random => Box::new(random::Random::new(params, sample_rate)),
            ModuleKind::Pops => Box::new(pops::Pops::new(params, sample_rate)),
            ModuleKind::Fjord => Box::new(fjord::Fjord::new(params, sample_rate)),
            ModuleKind::Splitter => Box::new(splitter::Splitter::new(params, sample_rate)),
            ModuleKind::Nullpunkt => Box::new(nullpunkt::Nullpunkt::new(params, sample_rate)),
            ModuleKind::AudioIn => Box::new(audio_in::AudioIn::new(params, audio.shared.clone(), sample_rate)),
            ModuleKind::Granular => Box::new(granular::Granular::new(params, sample_rate)),
            ModuleKind::Frakture => Box::new(frakture::Frakture::new(params, sample_rate)),
            ModuleKind::Resonator => Box::new(resonator::Resonator::new(params, sample_rate)),
            ModuleKind::Filter => Box::new(filter::Filter::new(params, sample_rate)),
            ModuleKind::Equalizer => Box::new(equalizer::Equalizer::new(params, sample_rate)),
            ModuleKind::Prism => Box::new(prism::Prism::new(params, sample_rate)),
            ModuleKind::Magma => Box::new(magma::Magma::new(params, midi.clone(), sample_rate)),
            ModuleKind::Tape => Box::new(tape::Tape::new(params, sample_rate)),
            ModuleKind::Delay => Box::new(delay::Delay::new(params, sample_rate)),
            ModuleKind::PingPong => Box::new(pingpong::PingPong::new(params, sample_rate)),
            ModuleKind::Reverb => Box::new(reverb::Reverb::new(params, sample_rate)),
            ModuleKind::MidiCc => Box::new(midi_cc::MidiCc::new(params, midi, sample_rate)),
            ModuleKind::MidiNote => Box::new(midi_note::MidiNote::new(params, midi, sample_rate)),
            ModuleKind::Sequencer => Box::new(sequencer::Sequencer::new(params)),
            ModuleKind::Arpeggiator => Box::new(arpeggiator::Arpeggiator::new(params, midi)),
            ModuleKind::MidiIn => Box::new(midi_in::MidiIn::new(params, midi)),
            ModuleKind::Midicer => Box::new(midicer::Midicer::new(params, midi)),
            ModuleKind::Automa => Box::new(automa::Automa::new(params)),
            ModuleKind::Colloid => Box::new(colloid::Colloid::new(params, midi.clone(), sample_rate)),
            ModuleKind::Orrery => Box::new(orrery::Orrery::new(params)),
            ModuleKind::Looper => Box::new(looper::Looper::new(params, midi, sample_rate)),
            ModuleKind::Iris => Box::new(iris::Iris::new(params, audio.shared.clone(), midi.clone(), sample_rate)),
            ModuleKind::Ext(e) => super::node_plugin::processor(e, params, sample_rate),
        }
    }
}

/// Settings shared lock-free between the UI and a module's processor: the
/// node SDK's `Params` (knobs, mode, switch, data, meter, display values),
/// tagged with the module's kind, and the sample it holds, if any (Splitter).
/// Derefs to the `Params`.
pub struct ModuleParams {
    pub kind: ModuleKind,
    params: audioboard_node::Params,
    /// An audio file the module plays (empty for most modules).
    pub sample: crate::audio::sample::SampleSlot,
}

impl ModuleParams {
    pub fn new(kind: ModuleKind) -> Self {
        Self { kind, params: audioboard_node::Params::new(kind.def()), sample: Default::default() }
    }

    /// A fresh copy of the settings (not the meters), sharing the sample.
    pub fn duplicate(&self) -> Self {
        let p = Self::new(self.kind);
        p.params.copy_from(&self.params);
        if let Some(s) = self.sample.get() {
            p.sample.set(Some(s));
        }
        p
    }
}

impl std::ops::Deref for ModuleParams {
    type Target = audioboard_node::Params;

    fn deref(&self) -> &Self::Target {
        &self.params
    }
}
