//! # audioboard-ui: AudioBoard's UI Kit
//!
//! The parts every node panel is drawn from, on egui, in AudioBoard's flat
//! line-art style:
//!
//! - [`theme`]: colours, fonts, the global egui style.
//! - [`view::NodeView`]: draws in node units at any canvas zoom, and
//!   records controls for MIDI mapping.
//! - [`widgets`]: knobs, horizontal and vertical faders, toggles, peak
//!   meters, ports, cables.
//! - [`controls`]: a labelled knob with its value ([`controls::knob_cell`]),
//!   segmented selectors, pills, port labels, captions.
//! - [`card`]: the node card and header.
//! - [`display`]: display insets and the primitives drawn in them (bars,
//!   curves, traces, dots, levels).
//! - [`panel`]: the generic panel's layout, computed from a node's
//!   description.
//!
//! Plugin nodes don't link this crate: egui's types can't cross the plugin
//! ABI. They describe their panel instead ([`audioboard_node::Panel`]) and
//! the app draws it from these parts. Nodes compiled into the app use the
//! parts directly.

pub mod card;
pub mod controls;
pub mod display;
pub mod panel;
pub mod theme;
pub mod view;
pub mod widgets;

pub use controls::{KnobCell, knob_cell, segmented};
pub use view::{Hit, MIN_TEXT, NodeView};
