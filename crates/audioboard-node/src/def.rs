//! What a node is: its static description ([`NodeDef`]), the settings the
//! UI and the audio thread share ([`Params`]), and, for nodes made with the
//! SDK, how the host should draw them ([`Descriptor`], [`Panel`]).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::param::{AtomicF32, ParamSpec, Unit};

/// Static description of a node's panel and ports.
#[derive(Clone, Copy, Debug)]
pub struct NodeDef {
    /// Name in menus ("Reverb").
    pub name: &'static str,
    /// Node title prefix ("REVERB" → "REVERB 1").
    pub prefix: &'static str,
    /// Short description shown next to the name in menus.
    pub hint: &'static str,
    pub inputs: &'static [&'static str],
    pub outputs: &'static [&'static str],
    pub params: &'static [ParamSpec],
    /// `(param, input)`: knobs that set how much a CV input modulates.
    pub cv_depth: &'static [(usize, usize)],
    /// Labels of the segmented mode selector (empty: none).
    pub modes: &'static [&'static str],
    /// Label of the on/off switch, if any.
    pub toggle: Option<&'static str>,
    /// The switch starts on.
    pub toggle_default: bool,
    /// Can drive any knob directly (MIDI nodes): shows a MAP button.
    pub map: bool,
    /// Takes MIDI on a MIDI cable.
    pub midi_in: bool,
    /// Sends MIDI on a MIDI cable.
    pub midi_out: bool,
    /// Extra values kept with the settings (a sequencer's steps), with
    /// their defaults.
    pub data: &'static [f32],
    /// Height of the display, in node units.
    pub display_h: f32,
    /// Height of extra controls under the display (buttons), if any.
    pub controls_h: f32,
    /// Number of values the processor publishes for the node's display.
    pub viz: usize,
}

impl NodeDef {
    /// An empty description to start from with `..NodeDef::EMPTY`.
    pub const EMPTY: NodeDef = NodeDef {
        name: "",
        prefix: "",
        hint: "",
        inputs: &[],
        outputs: &[],
        params: &[],
        cv_depth: &[],
        modes: &[],
        toggle: None,
        toggle_default: false,
        map: false,
        midi_in: false,
        midi_out: false,
        data: &[],
        display_h: 0.0,
        controls_h: 0.0,
        viz: 0,
    };
}

/// Settings shared lock-free between the UI and a node's processor: knob
/// values, the mode, the switch, extra saved data, and what flows back (the
/// output meter, display values).
pub struct Params {
    values: Box<[AtomicF32]>,
    mode: AtomicU32,
    toggle: AtomicBool,
    /// Output peak since the UI last looked.
    pub meter: AtomicF32,
    /// Values published by the processor for the node's display.
    pub viz: Box<[AtomicF32]>,
    /// Extra settings beyond the knobs ([`NodeDef::data`]), saved too.
    pub data: Box<[AtomicF32]>,
}

impl Params {
    /// Every setting at its default.
    pub fn new(def: &NodeDef) -> Self {
        Self {
            values: def
                .params
                .iter()
                .map(|p| AtomicF32::new(p.default))
                .collect(),
            mode: AtomicU32::new(0),
            toggle: AtomicBool::new(def.toggle_default),
            meter: AtomicF32::new(0.0),
            viz: (0..def.viz).map(|_| AtomicF32::new(0.0)).collect(),
            data: def.data.iter().map(|&v| AtomicF32::new(v)).collect(),
        }
    }

    pub fn get(&self, i: usize) -> f32 {
        self.values[i].get()
    }

    pub fn set(&self, i: usize, v: f32) {
        self.values[i].set(v);
    }

    /// Number of knobs.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn mode(&self) -> usize {
        self.mode.load(Ordering::Relaxed) as usize
    }

    pub fn set_mode(&self, m: usize) {
        self.mode.store(m as u32, Ordering::Relaxed);
    }

    pub fn toggled(&self) -> bool {
        self.toggle.load(Ordering::Relaxed)
    }

    pub fn set_toggle(&self, on: bool) {
        self.toggle.store(on, Ordering::Relaxed);
    }

    /// Takes `other`'s settings (not its meters or display values).
    pub fn copy_from(&self, other: &Params) {
        for (dst, src) in self.values.iter().zip(&other.values) {
            dst.set(src.get());
        }
        self.set_mode(other.mode());
        self.set_toggle(other.toggled());
        for (dst, src) in self.data.iter().zip(&other.data) {
            dst.set(src.get());
        }
    }

    /// Pointers to the atomics, for a node plugin (see [`crate::abi`]). They
    /// stay valid as long as `self` does not move or drop: keep it in an
    /// `Arc` that outlives the plugin's node.
    pub fn raw(&self) -> crate::abi::RawParams {
        crate::abi::RawParams {
            values: self.values.as_ptr(),
            n_values: self.values.len() as u32,
            mode: &self.mode,
            toggle: &self.toggle,
            data: self.data.as_ptr(),
            n_data: self.data.len() as u32,
            viz: self.viz.as_ptr(),
            n_viz: self.viz.len() as u32,
        }
    }
}

/// A node made with the SDK: its description, a stable id to save it by, who
/// made it, and how the host draws its panel.
#[derive(Clone, Copy, Debug)]
pub struct Descriptor {
    /// Unique and stable forever: projects find the node by it. Reverse-DNS
    /// style: `"com.example.shimmer"`.
    pub id: &'static str,
    /// Who made it: the palette lists plugin nodes by maker.
    pub maker: &'static str,
    pub def: NodeDef,
    pub panel: Panel,
}

/// How the host draws a node it only knows by its description: the generic
/// panel (ports, readout, switch, display, mode selector, knobs) with one
/// of a few display primitives fed by the node's display values
/// ([`Params::viz`], published with [`crate::Shared::set_viz`]).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Panel {
    pub display: Display,
    /// The big line at the top of the panel.
    pub readout: Readout,
}

/// The display area's picture (its height is [`NodeDef::display_h`]).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Display {
    /// No picture (keep `display_h` small, or 0).
    #[default]
    None,
    /// The output level, as a horizontal meter.
    Level,
    /// `count` display values from `from`, 0…1, as vertical bars (a
    /// spectrum, step levels, voices).
    Bars { from: usize, count: usize },
    /// `count` display values from `from`, −1…1, as a line left to right
    /// (a waveform, a transfer curve, an envelope).
    Curve { from: usize, count: usize },
    /// The recent history of one display value, −1…1, scrolling (an LFO).
    Trace { index: usize },
    /// A dot at two display values, 0…1 each (an XY position).
    Dot { x: usize, y: usize },
}

/// What the readout line says.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Readout {
    /// The node's name.
    #[default]
    Name,
    /// A knob's value, as it formats itself.
    Param(usize),
    /// A display value, formatted as `unit`.
    Viz { index: usize, unit: Unit },
    /// The selected mode's label.
    Mode,
}

impl Descriptor {
    /// Why the host can't use this description, if it can't. The host runs
    /// the same check on what a plugin sends.
    pub fn problem(&self) -> Option<String> {
        let d = &self.def;
        let viz = |i: usize| i < d.viz;
        if self.id.trim().is_empty() || self.id.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Some(format!("the id {:?} is empty or has spaces", self.id));
        }
        if d.name.trim().is_empty() {
            return Some(format!("{}: no name", self.id));
        }
        if d.inputs.len() > MAX_PORTS || d.outputs.len() > MAX_PORTS {
            return Some(format!(
                "{}: more than {MAX_PORTS} inputs or outputs",
                self.id
            ));
        }
        if d.params.len() > MAX_PARAMS {
            return Some(format!("{}: more than {MAX_PARAMS} parameters", self.id));
        }
        if let Some(p) = d.params.iter().find_map(|p| p.problem()) {
            return Some(format!("{}: {p}", self.id));
        }
        if d.modes.len() > MAX_MODES || d.data.len() > MAX_DATA || d.viz > MAX_VIZ {
            return Some(format!(
                "{}: too many modes, data or display values",
                self.id
            ));
        }
        if d.cv_depth
            .iter()
            .any(|&(p, i)| p >= d.params.len() || i >= d.inputs.len())
        {
            return Some(format!(
                "{}: a CV depth names a missing knob or input",
                self.id
            ));
        }
        if !(0.0..=400.0).contains(&d.display_h) || !(0.0..=400.0).contains(&d.controls_h) {
            return Some(format!("{}: display height out of range", self.id));
        }
        let shown = match self.panel.display {
            Display::None | Display::Level => true,
            Display::Bars { from, count } | Display::Curve { from, count } => {
                count > 0 && from + count <= d.viz
            }
            Display::Trace { index } => viz(index),
            Display::Dot { x, y } => viz(x) && viz(y),
        };
        let read = match self.panel.readout {
            Readout::Name | Readout::Mode => true,
            Readout::Param(i) => i < d.params.len(),
            Readout::Viz { index, .. } => viz(index),
        };
        if !shown || !read {
            return Some(format!(
                "{}: the panel shows a display value or knob it doesn't have",
                self.id
            ));
        }
        None
    }
}

/// Most audio inputs or outputs a node can have.
pub const MAX_PORTS: usize = 24;
/// Most knobs.
pub const MAX_PARAMS: usize = 64;
/// Most modes.
pub const MAX_MODES: usize = 16;
/// Most extra saved values.
pub const MAX_DATA: usize = 4096;
/// Most display values.
pub const MAX_VIZ: usize = 1024;
