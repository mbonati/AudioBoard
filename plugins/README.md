# Writing an AudioBoard node plugin

A **node plugin** is a dynamic library (`.dylib` on macOS, `.so` on Linux, `.dll` on Windows) holding one or
more nodes written in Rust against the node SDK, [`audioboard-node`](../crates/audioboard-node). AudioBoard
loads it at launch (or on **Rescan**) from its Nodes folder, and its nodes then work like the built-in ones:
in the palette, patched with audio and MIDI cables, mapped to MIDI controllers, saved in projects.

This guide builds a small effect, **Tremolo**, from an empty folder to a node on the board, then shows the
other kinds of nodes, how to test and distribute a plugin, and what to do when AudioBoard refuses one. The
main [README](../README.md#writing-nodes-sdk-plugins-and-ui-kit) describes the SDK, the plugin format and the
UI Kit; [`example-nodes`](example-nodes) is a complete library with an instrument, an effect and a MIDI
generator.

- [What you need](#what-you-need)
- [1. Create the crate](#1-create-the-crate)
- [2. Write the node](#2-write-the-node)
- [3. Build it](#3-build-it)
- [4. Install it](#4-install-it)
- [5. Test it without the app](#5-test-it-without-the-app)
- [Other kinds of nodes](#other-kinds-of-nodes)
- [The panel you get](#the-panel-you-get)
- [Rules for `process`](#rules-for-process)
- [Keeping projects working](#keeping-projects-working)
- [Distributing a plugin](#distributing-a-plugin)
- [When AudioBoard refuses a plugin](#when-audioboard-refuses-a-plugin)
- [Reference](#reference)

## What you need

- Rust (stable, 1.88 or newer: the SDK and the examples use edition 2024) and `cargo`.
- A checkout of AudioBoard (for `crates/audioboard-node`), or access to its repository as a git dependency.
- To build for the computer AudioBoard runs on: same operating system, same processor architecture (see
  [Distributing a plugin](#distributing-a-plugin) for macOS universal libraries).

A plugin doesn't need egui, or anything of the app: only the SDK, which has no dependencies.

## 1. Create the crate

**Inside the AudioBoard checkout** (simplest while learning): every folder in `plugins/` is a member of the
workspace, so a new crate there shares its `target/` and `Cargo.lock`.

```bash
cargo new --lib plugins/tremolo
```

**Or anywhere else**, as a crate of its own:

```bash
cargo new --lib tremolo
```

Then make it a dynamic library and add the SDK. `plugins/tremolo/Cargo.toml` (or `tremolo/Cargo.toml`):

```toml
[package]
name = "tremolo"
version = "0.1.0"
edition = "2024"

# A C-compatible dynamic library: libtremolo.dylib, libtremolo.so or tremolo.dll.
[lib]
crate-type = ["cdylib"]

[dependencies]
# Inside the checkout:
audioboard-node = { path = "../../crates/audioboard-node" }
# Elsewhere, a path to the checkout instead:
#   audioboard-node = { path = "/path/to/AudioBoard/crates/audioboard-node" }
# or the repository:
#   audioboard-node = { git = "https://github.com/mbonati/AudioBoard" }
```

The package name gives the file name (`tremolo` → `libtremolo.dylib`; a name with dashes, `my-nodes`, gives
`libmy_nodes.dylib`). A crate outside the workspace may add a `[profile.release]` section (`lto = true` makes
the library smaller and a little faster); inside the workspace, profiles come from the root `Cargo.toml`.
Never set `panic = "abort"`: AudioBoard relies on catching a node's panic (see [Rules](#rules-for-process)).

## 2. Write the node

Replace `src/lib.rs` with this. Tremolo turns its input's volume up and down with an LFO: two knobs (RATE,
DEPTH), a mode selector for the LFO's shape, a STEREO switch that moves the two channels in opposition, and a
display tracing the LFO.

```rust
//! Tremolo: an AudioBoard node plugin (the walkthrough in plugins/README.md).

use audioboard_node::prelude::*;

// Knob indexes, in the order of PARAMS.
const RATE: usize = 0;
const DEPTH: usize = 1;

const PARAMS: &[ParamSpec] = &[
    ParamSpec::log("RATE", 0.1, 20.0, 4.0, Unit::Hz),
    ParamSpec::lin("DEPTH", 0.0, 1.0, 0.6, Unit::Percent),
];

/// The LFO's shapes: the mode selector.
const SHAPES: &[&str] = &["SINE", "TRIANGLE"];

pub struct Tremolo {
    params: Shared,
    sample_rate: f32,
    /// The LFO's phase, in turns (0…1).
    phase: f32,
    depth: Smoothed,
}

impl Node for Tremolo {
    const DESCRIPTOR: Descriptor = Descriptor {
        id: "com.example.tremolo",
        maker: "Example",
        def: NodeDef {
            name: "Tremolo",
            prefix: "TREM",
            hint: "volume wobble",
            inputs: &["IN"],
            outputs: &["OUT"],
            params: PARAMS,
            modes: SHAPES,
            toggle: Some("STEREO"),
            display_h: 40.0,
            viz: 1,
            ..NodeDef::EMPTY
        },
        panel: Panel { display: Display::Trace { index: 0 }, readout: Readout::Param(RATE) },
    };

    fn new(init: &Init) -> Self {
        Tremolo {
            params: init.params,
            sample_rate: init.sample_rate,
            phase: 0.0,
            depth: Smoothed::new(init.params.get(DEPTH)),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io<'_>) {
        let n = io.frames();
        let p = self.params;
        let step = p.get(RATE) / self.sample_rate;
        let shape = p.mode();
        // STEREO: the right channel's LFO half a turn behind the left's.
        let offset = if p.toggled() { 0.5 } else { 0.0 };
        let depth = p.get(DEPTH);
        let (input, out) = io.through(0, 0);
        for i in 0..n {
            let d = self.depth.next(depth, ctx.smooth);
            for c in 0..2 {
                let lfo = wave(shape, self.phase + offset * c as f32);
                let gain = 1.0 - d * (0.5 - 0.5 * lfo);
                out[c][i] = input.map_or(0.0, |b| b[c][i]) * gain;
            }
            self.phase = (self.phase + step).fract();
        }
        // For the display: the LFO now.
        p.set_viz(0, wave(shape, self.phase));
    }
}

/// The LFO, −1…1, at a phase in turns.
fn wave(shape: usize, t: f32) -> f32 {
    match shape {
        1 => 1.0 - 4.0 * (t.fract() - 0.5).abs(),
        _ => fastmath::sin_turns(t),
    }
}

audioboard_node::export_nodes! {
    name: "Tremolo",
    maker: "Example",
    version: env!("CARGO_PKG_VERSION"),
    nodes: [Tremolo],
}
```

What each part does:

- **`DESCRIPTOR`** is everything AudioBoard knows about the node before it runs:
  - `id`: unique and **stable forever**. Projects find the node by it, so pick a reverse-DNS name you own
    and never change it (no spaces).
  - `maker`: the palette and the right-click menu list plugin nodes under it.
  - `def`, a `NodeDef`: `name` (in menus), `prefix` (node titles: "TREM 1", "TREM 2"; empty: the name in
    capitals), `hint` (next to the name in menus), the audio `inputs` and `outputs` (each a stereo cable),
    `params` (the knobs), `modes` (the segmented selector; empty: none), `toggle` (a switch with this label),
    `display_h` (the display's height in node units; 0 hides it) and `viz` (how many display values the node
    publishes). `..NodeDef::EMPTY` leaves the rest empty: no MIDI, no extra data.
  - `panel`: what the display shows and what the big readout line says (see [The panel](#the-panel-you-get)).
- **`ParamSpec`** describes a knob: `lin` or `log` (log for frequencies and times: equal turns are equal
  ratios), the range, the default (also where a double-click resets it) and the `Unit` its value reads in
  ("4.00 Hz", "60%"). `.bipolar()` draws the knob's arc from the centre.
- **`new`** runs on the UI thread when the node is added (or the sample rate changes): read what you need
  from `init` and allocate everything here.
- **`process`** runs on the audio thread for every block of at most 64 frames (`BLOCK`):
  - `io.frames()` is how many frames to make (usually 64, sometimes fewer);
  - `io.through(0, 0)` hands over input 0 (`None` when nothing is patched) and output 0 together; every
    output must be written for all `n` frames;
  - `self.params` (a `Shared`) reads the knobs (`get`), the mode (`mode`), the switch (`toggled`), without
    locks: the UI changes them while the audio plays;
  - `Smoothed::next(target, ctx.smooth)` glides the depth over about 10 ms, so turning the knob never clicks;
  - `set_viz(0, …)` publishes a number for the display; the panel traces it.
- **`export_nodes!`** makes the library a plugin: it exports the `audioboard_node_entry` symbol AudioBoard
  looks for. List every node type of the library in `nodes`.

## 3. Build it

Inside the checkout:

```bash
cargo build --release -p tremolo
```

The library is `target/release/libtremolo.dylib` (`libtremolo.so` on Linux, `tremolo.dll` on Windows). For a
crate of its own, run `cargo build --release` in its folder; the library is in its own `target/release/`.

To check it is a plugin (macOS; `nm -D` on Linux), look for the entry point:

```bash
nm -gU target/release/libtremolo.dylib | grep audioboard_node_entry
```

## 4. Install it

Copy the library into the Nodes folder:

| System | Nodes folder |
|---|---|
| macOS | `~/Library/Application Support/AudioBoard/Nodes` |
| Linux | `$XDG_DATA_HOME/audioboard/Nodes` (usually `~/.local/share/audioboard/Nodes`) |
| Windows | `%APPDATA%\AudioBoard\Nodes` |

On macOS:

```bash
mkdir -p ~/Library/Application\ Support/AudioBoard/Nodes && cp target/release/libtremolo.dylib ~/Library/Application\ Support/AudioBoard/Nodes/
```

In AudioBoard, press **Rescan** under **Plugin nodes** in the palette (or **Rescan plugins** in Settings →
Plugins), or restart it. The palette's **Open Nodes folder** link opens the folder, creating it if needed.
Tremolo is now under **Plugin nodes → Example**: drag it onto the board, patch a source into IN and OUT into
the mixer.

**While developing**, skip the copying: AudioBoard also searches the folders listed in
`AUDIOBOARD_NODE_PATH` (separated like `PATH`). From the AudioBoard checkout, with the crate in `plugins/`:

```bash
cargo build --release -p tremolo && AUDIOBOARD_NODE_PATH=target/release cargo run --release
```

Run from a terminal like this, AudioBoard prints what it loaded or refused ("Node plugins: 1 node plugin(s)
loaded (1 nodes)"), and a panicking node's message. A library is never unloaded while AudioBoard runs (its
nodes run its code): a **new** library loads on Rescan, a **rebuilt** one after a restart.

## 5. Test it without the app

A node is plain Rust: give it settings and buffers and call `process`. Add to `src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use audioboard_node::Params;

    #[test]
    fn the_description_holds_together() {
        assert_eq!(Tremolo::DESCRIPTOR.problem(), None);
    }

    #[test]
    fn full_depth_takes_the_level_down_to_silence_and_back() {
        // The settings the app would own, and the node's view of them.
        let params = Params::new(&Tremolo::DESCRIPTOR.def);
        params.set(DEPTH, 1.0);
        // SAFETY: `params` outlives the node.
        let shared = unsafe { Shared::from_raw(params.raw()) };
        let mut node = Tremolo::new(&Init { sample_rate: 48_000.0, params: shared });

        let ctx = ProcessCtx { sample_rate: 48_000.0, smooth: 0.01, beat: 0.0, beats_per_sample: 0.0, playing: true, restart: false };
        let input = [[[1.0; BLOCK]; 2]];
        let mut output = [[[0.0; BLOCK]; 2]];
        let (mut low, mut high) = (f32::MAX, 0.0f32);
        for _ in 0..750 {
            // One second, block by block.
            let mut io = Io::new(BLOCK, &input, &[true], &mut output, &[], &mut []);
            node.process(&ctx, &mut io);
            for s in output[0][0] {
                low = low.min(s);
                high = high.max(s);
            }
        }
        assert!(low < 0.05 && high > 0.95, "{low} … {high}");
    }
}
```

```bash
cargo test -p tremolo
```

`Descriptor::problem()` runs the same checks AudioBoard runs when it loads the library: a test with it catches
a broken description before you install anything. `Io::new(frames, inputs, connected, outputs, midi_in,
midi_out)` takes MIDI too (`MidiMsg { frame, data }`), and `io.midi_sent()` returns what the node sent: see
the tests in [`example-nodes/src/lib.rs`](example-nodes/src/lib.rs).

## Other kinds of nodes

**An instrument (MIDI in).** Set `midi_in: true` and no `inputs`. `io.midi()` holds the block's events in
frame order; apply each at its frame. The events borrow `io`, so copy them out before taking the output:

```rust
let n = io.frames();
let mut events = [MidiMsg::default(); audioboard_node::MIDI_CAP];
let count = io.midi().len();
events[..count].copy_from_slice(io.midi());
let mut next = 0;
let out = io.output(0);
for i in 0..n {
    while next < count && events[next].frame as usize <= i {
        if let Some((note, velocity)) = events[next].note_on() {
            // start a voice
        } else if let Some(note) = events[next].note_off() {
            // release it
        }
        next += 1;
    }
    // out[0][i] = …; out[1][i] = …;
}
```

[`example-nodes/src/glass.rs`](example-nodes/src/glass.rs) is a complete eight-voice instrument.

**A MIDI generator (MIDI out).** Set `midi_out: true`; it may have no audio outputs at all. Send with
`io.send_midi(frame, [status, data1, data2])` (at most 128 events a block). Follow the transport through
`ctx`: `ctx.beat_at(i)` is the beat at frame `i`, `ctx.playing` says whether it runs, and on `ctx.restart`
(or when it stops) end the notes you are holding. [`example-nodes/src/pulse.rs`](example-nodes/src/pulse.rs)
plays a Euclidean rhythm on the beat.

**Control (CV) inputs.** Any input can carry a control signal: `io.cv(i)` is its left channel, `None` when
unpatched. A knob that sets how much an input modulates is listed in `cv_depth` as `(knob, input)`; the
panel dims it while that input is unpatched:

```rust
inputs: &["IN", "RATE CV"],
cv_depth: &[(DEPTH, 1)],
// in process: let cv = io.cv(1).map_or(0.0, |c| c[i]);
```

**Several outputs.** List them in `outputs` and write each (`io.output(j)`, or `io.outputs()` for all). An
input and several outputs are borrowed from `io` together by copying the input first:

```rust
let input = io.input(0).copied();
let outs = io.outputs();
let (main, rest) = outs.split_at_mut(1);
// main[0] is OUT, rest[0] the second output; input is an Option<Buf>
```

**Saved state of its own.** `data: &[0.0; 16]` gives the node 16 extra values (with their defaults) saved in
the project and restored with it: `params.data(i)` and `params.set_data(i, v)`, from `process` too. The
generic panel doesn't show them: they are for what the node keeps by itself (a captured pattern, a learned
value).

**Tempo.** `ctx.tempo()` is the tempo in BPM and `ctx.beats_per_sample` the beat's speed: an LFO synced to the
beat reads its phase from `ctx.beat_at(i)` instead of counting samples.

## The panel you get

A plugin can't draw (egui's types can't cross the plugin boundary), so AudioBoard draws a generic panel from
the description:

```text
 ┌──────────────────────────────┐
 │ ▾ ● TREM 1              ○  × │  header: title from `prefix`; ○ bypass on effects
 │ IN       4.00 Hz         OUT │  ports down the sides, the readout in the middle
 │          SINE      [STEREO]  │  the mode (or the hint), the switch
 │ ┌──────────────────────────┐ │
 │ │     display (Trace)      │ │  `display_h` tall
 │ └──────────────────────────┘ │
 │ [   SINE   |   TRIANGLE   ]  │  the mode selector
 │      RATE        DEPTH       │  knobs, three to a row (four for 4 or 7+ knobs)
 └──────────────────────────────┘
```

The **display** shows one of these pictures, fed by the values the node publishes with `set_viz`:

| `Display` | Shows | Values |
|---|---|---|
| `None` | nothing | |
| `Level` | the output level, as a meter | (measured by AudioBoard) |
| `Bars { from, count }` | vertical bars | `count` values from `from`, 0…1 |
| `Curve { from, count }` | a line, left to right | `count` values from `from`, −1…1 |
| `Trace { index }` | the value's recent history, scrolling | one value, −1…1 |
| `Dot { x, y }` | a dot with cross-hairs | two values, 0…1 (y up) |

The **readout** (the big line) is one of `Readout::Name` (the node's name), `Param(i)` (knob `i`'s value),
`Viz { index, unit }` (a display value read in a unit, for example `Unit::Note` for "C4") or `Mode` (the mode's
label). The line under it shows the mode, or the hint if there is none. If the node panics, its display says
"Stopped: the plugin crashed".

## Rules for `process`

`process` runs on the audio thread, which must deliver every block on time. In it:

- **don't allocate** (no `Vec::push` past capacity, no `Box::new`, no `String`, no `format!`): make buffers
  in `new` (the SDK's `DelayLine` allocates once, in `new`);
- **don't lock, wait or sleep** (no `Mutex`, no channels that block), and do no I/O (no files, no printing);
- write **every output** for all `io.frames()` frames;
- keep the output finite and in range: NaN, infinities and absurd levels are muted by AudioBoard, and
  `dsp::soft_limit` keeps a level under ±1 smoothly.

A panic in `new` or `process` is caught at the plugin boundary: the node falls silent and its display says
so; the rest of the patch plays on. That only works with the default `panic = "unwind"`.

The SDK's helpers are safe on the audio thread: `Smoothed`, `OnePole`, `DelayLine` (after `new`), `Rng`,
`Wander`, `Quiet` (tells an effect it may skip its work while its input and tail are silent),
`dsp::pan_gains`, `mix_gains`, `soft_limit`, `soft_ceiling`, `note_hz`, and `fastmath` (`sin_turns`, `exp2`,
`log2`, `tanh`).

## Keeping projects working

A project stores a plugin node by its id, its knobs **by name**, its mode **by label**, its switch, and its
data values by position. So, from one version of your plugin to the next:

- never change the `id`;
- adding or reordering knobs is fine (a new knob starts at its default); renaming one loses its saved value;
- renaming a mode resets saved nodes to the first mode;
- add new `data` values at the end;
- `version` in `export_nodes!` is shown in Settings → Plugins; bump it as you like.

The **ABI version** is AudioBoard's, not yours: it changes only when the boundary between app and plugin
changes. A plugin built for another one is refused with a message saying to rebuild it against the matching
`audioboard-node`; rebuilding is all it takes.

## Distributing a plugin

Build in release, and ship the one library file. On **macOS**, a library loads only into an AudioBoard of
the same architecture; a universal library serves both Apple silicon and Intel Macs:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
```

```bash
cargo build --release --target aarch64-apple-darwin && cargo build --release --target x86_64-apple-darwin
```

```bash
lipo -create -output libtremolo.dylib target/aarch64-apple-darwin/release/libtremolo.dylib target/x86_64-apple-darwin/release/libtremolo.dylib
```

A library downloaded from the internet is marked by macOS as quarantined, and unsigned code from it may be
refused ("could not be loaded"). Sign what you distribute (`codesign --sign "Developer ID Application: …"`,
then notarize); a user who trusts an unsigned plugin can clear the mark with
`xattr -d com.apple.quarantine libtremolo.dylib`.

On **Linux** ship `libtremolo.so`, on **Windows** `tremolo.dll`, each built on (or for) that system.

## When AudioBoard refuses a plugin

AudioBoard checks a library before using any of its nodes. A refused one shows in a notice at launch or after
Rescan ("refused: libtremolo.dylib: …"), and is printed when AudioBoard runs from a terminal:

| Message | Cause, and what to do |
|---|---|
| `could not be loaded (…)` | Not a library for this system or architecture, a missing dependency, or macOS quarantine (see above). |
| `not an AudioBoard node plugin (no audioboard_node_entry)` | `export_nodes!` is missing, or `crate-type` isn't `["cdylib"]`. |
| `built for node ABI vN, this AudioBoard loads vM: rebuild it with audioboard-node X` | Built against another SDK generation: rebuild with the one named. |
| `built for N-frame blocks, AudioBoard uses 64` | Same: rebuild against the matching SDK. |
| `its node id … is already taken` | Two libraries offer the same id: often two copies of one plugin in the Nodes folder. |
| `a node is not usable: <id>: …` | The description doesn't hold together; the rest says what (below). |
| `it failed to describe its nodes` | Building the description panicked. |

Description problems (all caught by `Descriptor::problem()` in a test):

- `the id "…" is empty or has spaces`, `<id>: no name`;
- `<id>: KNOB: the range a…b is empty or not finite`, `a log taper needs a range above zero`, `the default x is
  out of range`, `a parameter has no name`;
- `<id>: a CV depth names a missing knob or input`;
- `<id>: the panel shows a display value or knob it doesn't have` (a `Display` or `Readout` pointing past
  `viz` or past the knobs);
- `<id>: more than 24 inputs or outputs`, `more than 64 parameters`, `too many modes, data or display values`,
  `display height out of range`.

In a node's display: **Stopped: the plugin crashed** (it panicked: run AudioBoard from a terminal to read the
panic) or **Muted bad samples from the plugin** (it output NaN, infinities or levels over 10 000).

## Reference

**Units** and how a value reads:

| `Unit` | Reads | | `Unit` | Reads |
|---|---|---|---|---|
| `Hz` | 440 Hz, 1.20 kHz | | `Integer` | 3 |
| `Cents` | 6 ct (+6 ct bipolar) | | `Channel` | Omni (0), Ch 3 |
| `Percent` | 60% (from 0.6) | | `Division` | 1/16 (index into 1/4 1/8 1/8T 1/16 1/16T 1/32) |
| `Decibel` | -6.0 dB (from a gain, 0.5) | | `NoteName` | C (0 = C) |
| `Db` | 3.0 dB (already dB) | | `Note` | C4 (MIDI note 60) |
| `Seconds` | 250 ms, 1.20 s | | `NoteValue` | 1/4 (index into 1/1 … 1/32) |
| `Semitones` | +7 st bipolar | | `Sync` | 1/8D (index into 1/32 … 1/1, dotted and triplets) |
| `Octaves` | +1.0 oct bipolar | | `Q` | 0.71 |
| `PerSecond` | 2.5/s | | `Plain` | 0.50 (+0.50 bipolar) |

**Limits**: 24 audio inputs and 24 outputs, 64 knobs, 16 modes, 4096 data values, 1024 display values, a
display up to 400 units tall, 256 node types per library, 512 loaded at once.

**Where things are**: the SDK's documentation is in its source
([`crates/audioboard-node/src`](../crates/audioboard-node/src): `def.rs` for descriptions, `node.rs` for
`Node`, `Io` and `Shared`, `dsp.rs` for the helpers, `abi.rs` for the boundary itself). `cargo doc -p
audioboard-node --open` renders it.
