//! # audioboard-node: the foundation AudioBoard nodes are written against
//!
//! Everything a node needs and nothing it doesn't (no UI toolkit, no other
//! dependency):
//!
//! - **What it is**: [`NodeDef`] (name, ports, knobs, modes, switch, saved
//!   data, display values), [`Descriptor`] (plus a stable id, its maker, and
//!   the [`Panel`] the host draws for it), [`ParamSpec`] with its
//!   [`Taper`] and [`Unit`].
//! - **How it runs**: the [`Node`] trait, processing blocks of [`BLOCK`]
//!   stereo frames ([`Buf`]) with MIDI events at frame offsets
//!   ([`MidiMsg`]), under a [`ProcessCtx`] (sample rate, smoothing, beat,
//!   tempo, playing, restart), through [`Io`].
//! - **Settings shared lock-free**: [`Params`] (the host's side, in
//!   atomics) and [`Shared`] (the node's view of it).
//! - **DSP helpers**: [`dsp`] (randomness, smoothing, delay line, rest
//!   detection, pan and mix laws, limiters) and [`fastmath`].
//! - **Plugins**: [`abi`], the versioned C ABI that lets AudioBoard load
//!   precompiled nodes from a dynamic library, and [`export_nodes!`], which
//!   generates it.
//!
//! A minimal effect:
//!
//! ```
//! use audioboard_node::prelude::*;
//!
//! const PARAMS: &[ParamSpec] = &[ParamSpec::lin("GAIN", 0.0, 2.0, 1.0, Unit::Percent)];
//!
//! pub struct Gain {
//!     params: Shared,
//!     gain: Smoothed,
//! }
//!
//! impl Node for Gain {
//!     const DESCRIPTOR: Descriptor = Descriptor {
//!         id: "com.example.gain",
//!         maker: "Example",
//!         def: NodeDef {
//!             name: "Gain",
//!             prefix: "GAIN",
//!             hint: "louder · softer",
//!             inputs: &["IN"],
//!             outputs: &["OUT"],
//!             params: PARAMS,
//!             ..NodeDef::EMPTY
//!         },
//!         panel: Panel { display: Display::Level, readout: Readout::Param(0) },
//!     };
//!
//!     fn new(init: &Init) -> Self {
//!         Gain { params: init.params, gain: Smoothed::new(1.0) }
//!     }
//!
//!     fn process(&mut self, ctx: &ProcessCtx, io: &mut Io<'_>) {
//!         let n = io.frames();
//!         let target = self.params.get(0);
//!         let (input, out) = io.through(0, 0);
//!         for i in 0..n {
//!             let g = self.gain.next(target, ctx.smooth);
//!             for c in 0..2 {
//!                 out[c][i] = input.map_or(0.0, |b| b[c][i]) * g;
//!             }
//!         }
//!     }
//! }
//!
//! // In a `cdylib` crate, this makes it a plugin AudioBoard can load:
//! // audioboard_node::export_nodes! { name: "Gain", maker: "Example", version: "1.0", nodes: [Gain] }
//! ```

pub mod abi;
pub mod buffer;
pub mod def;
pub mod dsp;
pub mod fastmath;
pub mod node;
pub mod param;

pub use buffer::{BLOCK, Buf, MIDI_CAP, MidiBuf, MidiMsg, ProcessCtx};
pub use def::{Descriptor, Display, NodeDef, Panel, Params, Readout};
pub use node::{Init, Io, Node, Shared};
pub use param::{AtomicF32, ParamSpec, Taper, Unit};

/// This crate's version, recorded in every plugin built with it.
pub const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Everything a node usually needs, in one `use`.
pub mod prelude {
    pub use crate::buffer::{BLOCK, Buf, MidiMsg, ProcessCtx};
    pub use crate::def::{Descriptor, Display, NodeDef, Panel, Readout};
    pub use crate::dsp::{self, DelayLine, OnePole, Quiet, Rng, Smoothed, Wander};
    pub use crate::fastmath;
    pub use crate::node::{Init, Io, Node, Shared};
    pub use crate::param::{ParamSpec, Taper, Unit};
}
