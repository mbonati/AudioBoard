# AudioBoard

A node-based modular synthesizer in Rust, in the spirit of Max/MSP and Pure Data:
small blocks (nodes) joined by virtual cables generate sound in real time.
It is designed especially for ambient sounds, drones, and organic and granular textures.

## Getting started

```bash
cargo run --release
```

For playing large patches, use the most optimized build (whole-program optimization, about 10–15% less
audio CPU, but a few minutes to compile):

```bash
cargo run --profile dist
```

On launch an ambient demo patch opens: a **Drone** (a chord of fifths on A1) goes into a **Filter** whose
cutoff is slowly moved by a **Random**; the filter feeds a **Reverb** with a touch of shimmer and, an octave
up, a **Granular** cloud that ends in a tape **Delay**. Everything flows into the **Mixer**.

Every cable is stereo; mono sources write the same signal to both channels.

## Nodes

### Sources

**Oscillator**: Sine, Triangle, Saw, Pulse and Noise waveforms (saw and pulse are band-limited with PolyBLEP).

| Knob  | Function |
|-------|----------|
| FREQ  | 0.1 Hz – 20 kHz (logarithmic scale; below 20 Hz it works as an LFO) |
| FINE  | ±100 cents |
| LEVEL | output amplitude |
| SHAPE | depends on the waveform: Drive (sine/saw), Skew (tri), Width (pulse), Color (noise) |
| FM    | depth of frequency modulation from the FM input |
| AM    | depth of amplitude modulation from the AM input |

**Drone**: eight detuned voices arranged in a chord (Unison, Fifths, Octaves, Major, Minor, Suspended).
Each voice drifts slowly in pitch and level, so the sound never stands still.
Knobs: PITCH, DETUNE (cents), DRIFT, TONE (from dark sines to bright saws), SPREAD (stereo width),
P.MOD / T.MOD (depth of the PITCH and TONE inputs), LEVEL.

**Random**: a random modulation source. RATE, CHAOS (irregular timing), SMOOTH (from sample & hold to
continuous glide), LEVEL. Patch it into any control input (cutoff, pitch, grain position…).

### Texture

**Granular**: records its input into an 8-second loop and plays it back as a cloud of grains.
POS (how far back to read), SIZE, DENSITY (grains per second), PITCH (semitones), JITTER (randomness of
position and timing), SPRAY (random pitch), SPREAD (stereo), MIX. Direction FWD / REV / BOTH.
**FREEZE** stops recording: the cloud keeps exploring the captured sound. The POS input moves the read
position (e.g. with a Random or an LFO). The display shows the loop and the active grains.

**Resonator**: a bank of eight tuned resonant modes (String, Pipe, Bar, Bell) excited by its input.
With noise or grains as input it gives physical, organic timbres. PITCH, DECAY, BRIGHT, SPREAD, P.MOD, MIX.

### Effects

**Filter**: stereo state-variable filter, LP / BP / HP / Notch. CUTOFF, RESO, DRIVE, MOD
(depth of the CUT input in octaves, negative too). The display shows the frequency response.

**Delay**: stereo tape echo up to 2 s. TIME (changes glide, like tape), FEEDBACK,
TONE (darkens the repeats), WOW (wow & flutter), PING (ping-pong), MIX.

**Reverb**: an 8-line delay network with diffusion, modulation and damping. SIZE (from room to
cathedral), DECAY (0.3 – 60 s), DAMP, MOD, SHIMMER (an octave up, fed back into the tail), MIX.
**FREEZE** holds the tail forever.

### MIDI

MIDI nodes receive from any connected MIDI device: devices are connected automatically, even when plugged
in after launch. The **MIDI** indicator in the status bar shows how many inputs are active
(hover for their names) and lights up when messages arrive.

**MIDI CC**: an external controller (knob, fader, wheel) as a signal. CC, CHANNEL (Omni or 1–16),
SMOOTH (glide), output 0…1 or ±1. **LEARN** picks up the next controller you move.

**MIDI Note**: a keyboard as an envelope. Holding a key, ENV rises over ATTACK; releasing it, it falls over
RELEASE; VEL is the same envelope scaled by velocity. Both outputs go from −1 to +1, so into an
oscillator's AM input (AM at 100%) they go from silence to full level. Last-note priority;
TRANSPOSE in semitones.

**Driving a control (MAP).** Besides patching outputs with cables, a MIDI node can move any knob or fader
directly: press **MAP** on the node, then click the control (Esc cancels). A dashed line joins the node to
the control, and the node shows "→ FILTER 1 · CUTOFF". With MIDI CC the control follows the controller;
with MIDI Note, frequency knobs take the note's frequency (e.g. the Drone's PITCH or the oscillator's FREQ),
semitone knobs the distance from middle C, and the others the key's position. The control moves only with
messages that arrive after mapping. **UNMAP** removes the mapping. Mappings are saved in the project and in
library modules.

### MIDI sequences: Step Sequencer, Arpeggiator, MIDI In

Besides audio cables there are **MIDI cables**: square ports (audio ports are round) and dashed cables.
A MIDI cable only joins a MIDI output to a MIDI input. VST3 plugins that accept MIDI (instruments), the
Arpeggiator and MIDI Note have a MIDI input.

The **tempo** is global: the **BPM** field in the toolbar. Drag it to change it (Shift for fine steps),
click it to type a value (Enter sets it, Esc cancels; "97,5" works too), Alt+click to go back to 90.
**TAP** (or the T key) sets it from your taps: tap on each beat, and the last few intervals are averaged;
a pause of two seconds starts a new count. The light flashes on every beat while audio is on. Sequencers
and arpeggiators are locked to the same grid, so they always stay in time with each other; the tempo is
also passed to plugins (synced LFOs and delays). It is saved in the project.

**Step Sequencer**: up to 16 steps on a scale. In the grid each column is a step and the lit cell is its
note: click a cell to move the step there (on the lit cell, to switch it off or back on), drag to draw a
melody; hovering shows the note. RATE (from 1/4 to 1/32, triplets), STEPS (length), GATE (note length),
SWING (delays the even-numbered steps), CHANCE (probability that a step plays), VELOCITY, ROOT and OCTAVE
(the tonic), scale MAJ / MIN / DOR / PENT / CHR. **PLAY** stops and restarts it. Steps are saved in the
project and in library modules.

**Arpeggiator**: plays the held notes one at a time, in time, over 1–4 octaves (OCTAVES), in UP, DOWN,
U+D (up and down), RAND or ORDER (as played) order. RATE, GATE, SWING, CHANCE, VELOCITY as in the
sequencer. Notes come from its MIDI cable or, with no cable, straight from the MIDI keyboard.
**LATCH** keeps the chord going after you let go of the keys, until you play a new one.

**MIDI In**: the MIDI keyboard on a cable, to send it into an arpeggiator or a plugin; CHANNEL filters one
channel, TRANSPOSE shifts the notes (controllers and pitch bend pass through unchanged).

Examples:
- *Step Sequencer → VST instrument*: the plugin plays the sequence (leave **KEYBOARD** on to play over it
  from the keyboard too);
- *MIDI In → Arpeggiator → VST instrument*: arpeggios from what you play;
- *Step Sequencer → MIDI Note*, with ENV into an oscillator's AM and MAP on its FREQ: the sequence plays
  the built-in nodes.

Unplugging a MIDI cable (or deleting the node feeding it) sends the plugin note-offs for the notes still
sounding, so no notes hang.

### VST3 plugins

Installed VST3 plugins (`/Library/Audio/Plug-Ins/VST3` and `~/Library/Audio/Plug-Ins/VST3`) appear in the
**Plugins** section of the palette and of the right-click menu, grouped by maker (foldable groups in the
palette, submenus in the menu), and are added like any other node. No plugin is loaded to list them: the
maker is read from the bundle's files (VST3 module info, copyright notice or bundle identifier), and the
category appears after the first load, which also confirms the maker. Makers that can't be found are under
"Other"; the search matches makers too.
**Rescan plugins** refreshes the list.

A plugin node has a stereo input (effects) and a stereo output; if it accepts MIDI it also has a MIDI
input (square port) for sequencers and arpeggiators. It shows:
- **EDITOR**: opens the plugin's own window (sounds, presets, all its settings). It floats in front of the
  board while AudioBoard is the active app, and hides while you are in another app;
- **PARAMS**: the list of all parameters, with search and sliders; the dot pins up to 8 parameters
  as knobs on the node (at first, the first 8);
- **KEYBOARD**: plays the plugin from the external MIDI keyboard (on by default for instruments), along
  with whatever arrives on the MIDI cable.

The node's knobs can be mapped with a MIDI CC node (MAP), like the others. The project saves the plugin's
path and its complete state, so the sound chosen in its editor too; on another computer the plugin is
looked up by name in the VST3 folders. The same goes for library modules.

Plugins run inside AudioBoard: a faulty plugin can make the application quit. VST3 is supported
(not VST2, AU or CLAP).

### Output

**Mixer (4 channels)**: for each channel a volume fader, pan (balance on stereo signals), Mute/Solo
and a meter; a stereo master bus with L/R meters and a clip indicator. The mixer feeds the audio device.

## Controls

| Action | Gesture |
|--------|---------|
| Add a node or a library module | drag it from the palette onto the canvas (or click it), or right-click the canvas |
| Connect | drag from a port to the opposite kind (output → input or vice versa) |
| Reconnect / unplug a cable | drag it away from the input it is plugged into |
| Delete a cable | double-click the cable, or right-click → Disconnect |
| Adjust knobs / faders | drag (Shift = fine adjustment), double-click = reset |
| Select several nodes | drag a rectangle on empty canvas, or Shift+click (Cmd+A all, Esc none) |
| Move the view | two-finger scroll, drag with the right or middle button, or Alt+drag |
| Zoom | pinch or Cmd+scroll; Cmd+= / Cmd+− / Cmd+0 (100%); or click the percentage in the status bar |
| Move / delete / duplicate the selection | drag a selected node / Del / Cmd+D (inner cables are copied) |
| Make a module from the selection | Cmd+G, or right-click → Make module |
| Rename a module | Enter or F2 with the module selected, or right-click → Rename… (Enter confirms, Esc cancels) |
| Dissolve a module | Cmd+Shift+G, the × in the module's header, or right-click → Ungroup |
| Collapse / expand a node or a module | arrow in the header, double-click the title, the C key, or right-click → Collapse |
| Collapse / expand everything | Shift+C, the toolbar button, or right-click the canvas |
| Frame everything / the selection | F / Shift+F |
| Auto layout | L, the toolbar button or right-click the canvas; Cmd+Z undoes it |
| Auto-fit (keep the whole patch in view) | A or the toolbar button |
| New / open project | Cmd+N / Cmd+O, or the project name in the toolbar |
| Save / save as | Cmd+S / Shift+Cmd+S |
| Audio on/off | Space |
| Tempo | drag the BPM field, click it to type, TAP or T to tap it |
| Show / hide the oscilloscope and spectrum | panel button in the toolbar |

The full list of shortcuts appears when hovering the **?** button in the toolbar.

Resting the pointer on a node or a module for a moment shows a small translucent label just above it with
its type, since titles can be renamed: "Reverb · hall · shimmer · freeze", "Originals - Cinematic Pads ·
VST3 instrument · Spitfire Audio", or for a module the nodes it holds ("Module · Drone · Filter · 2× Reverb").
The label stays put rather than following the pointer, takes no clicks, and hides while dragging, in menus
and while renaming.

Right-click (or Ctrl+click on macOS) opens the menu for whatever is under the pointer: canvas, node
(also when clicking a knob or a display), module or cable.

A collapsed node takes up only a strip with its title, ports and main value (frequency, cutoff, RT…);
a collapsed mixer also shows the master meters. Cables and audio keep working normally.

Connections that would create a feedback loop are refused; each input takes a single cable.
Feedback is available inside nodes (Delay, Reverb).

### Modules

A **module** groups several nodes into one, to move, collapse and duplicate them together.
Create one by selecting nodes and pressing Cmd+G. To rename it, select it and press Enter (or F2, or
right-click → Rename…): the name is edited right in the header.

- **Expanded**: a frame around the nodes, which stay editable in place. Dragging the header moves the
  whole module; the frame adapts when you move a node inside it.
- **Collapsed**: a single card showing only the boundary ports, that is the inputs not fed by another node
  of the module and the outputs that are free or patched outside (e.g. `DRONE 1 · PITCH`, `FILTER 1 · OUT`).
  External cables attach to the card and can be plugged and unplugged as on a normal node.
- **Duplicating** a module (select it by clicking its header, then Cmd+D) copies the nodes, the cables
  between them and the module itself. **Del** deletes the module with its nodes; **Ungroup** dissolves it
  and keeps the nodes.

Grouping is only visual: the sound does not change. A node belongs to at most one module.

## Auto layout and zoom

**Auto layout** (L) arranges the patch following the signal flow: columns from left to right, from the
sources to the mixer. In each column, blocks line up with the ports they are connected to, so cables are as
straight as possible; modulators (Random, LFOs, MIDI nodes) end up right before what they modulate;
cables that skip a column get a clear corridor, so they don't run behind other nodes. Modules move as
whole blocks and are rearranged inside too. With two or more nodes selected (or a module), only the
selection is arranged. Nodes glide into their new positions and the view adapts to the result;
**Cmd+Z** restores the previous arrangement.

**Zoom.** F fits the view to the whole patch, Shift+F to the selection, with a short animation. Clicking the
zoom percentage in the status bar opens the view menu. **Auto-fit** (A) keeps the whole patch in view
while you add, remove, collapse or move nodes or resize the window; "auto" appears in the status bar. It
turns itself off as soon as you zoom or move the view by hand.

## Palette

The panel on the left (shown and hidden with the button next to the project name) lists all nodes by
category and the library modules, with a search field. Click a category's heading to fold or unfold it
(a folded one shows how many entries it holds); while searching, every category with matches opens. Drag an entry onto the canvas to place it where you
want (an outline shows where it lands), or click it to add it in the middle of the view. On library
entries, the × removes the module from the library.

## Projects and library

**Projects.** The project name in the toolbar opens the File menu (New project, Open…, Save, Save as…).
A dot next to the name, and "(edited)" in the window title, flag unsaved changes: before creating a new
project, opening another one or closing the window, AudioBoard asks whether to save them.
A project (`.audioboard`) is a JSON file with nodes and parameters, positions, colors, collapsed nodes,
cables and modules; parameters are saved by name. The demo patch always opens on launch.

**Module library.** Right-click a module's header → **Save to library** saves it under its name (if it
already exists, it asks whether to replace it). Saved modules appear in the **Library** section of the
palette and of the right-click menu: inserting one adds the nodes, the cables between them and their
settings as a new module, already **collapsed** to a card. The × next to the name removes it from the
library, and **Open library folder** opens the folder of `.abmodule` files
(`~/Library/Application Support/AudioBoard/Library` on macOS), useful for copying or sharing them.

## Architecture

```
src/
├── audio/
│   ├── params.rs   parameters shared lock-free (atomics) between the UI and the audio thread
│   ├── engine.rs   audio-side graph: 64-frame stereo blocks, per-block MIDI events, global clock
│   │               in beats, precomputed topological order, denormals flushed to zero
│   ├── fastmath.rs fast sine, exp2 and tanh for per-sample DSP (error < 1e-6)
│   ├── dsp.rs      Oscillator and Mixer
│   ├── modules/    drone, random, granular, resonator, filter, delay, reverb, midi_cc, midi_note,
│   │               sequencer, arpeggiator, midi_in
│   │   ├── steps.rs step grid on the global tempo, outgoing notes, held keys
│   │   └── mod.rs  module descriptions (ModuleDef), generic parameters and DSP helpers
│   ├── midi.rs     MIDI inputs (midir) and shared state read by the MIDI nodes
│   ├── plugin.rs   VST3 plugins (vst3-host): loading, scanning, processing on the audio thread
│   └── mod.rs      opening the output stream (cpal)
├── graph.rs        UI-side patch model (nodes, cables, modules, Kahn ordering)
├── project.rs      saving/loading projects and the module library (JSON)
├── ui/
│   ├── canvas.rs   canvas with pan/zoom, selection, cable drag & drop, context menus
│   ├── groups.rs   modules: frame (expanded) and card with the boundary ports (collapsed)
│   ├── layout.rs   auto layout by signal flow (columns, aligned ports, corridors for cables)
│   ├── palette.rs  palette of nodes and library modules, with dragging onto the canvas
│   ├── plugin_node.rs panel of plugin nodes (editor, parameters, MIDI, knobs)
│   ├── nodes.rs    layout and drawing of the oscillator and mixer, shared controls
│   ├── modules.rs  common module panel and their displays
│   ├── widgets.rs  hand-drawn knobs, faders, meters, ports and cables
│   ├── monitor.rs  bottom panel: oscilloscope and spectrum (FFT)
│   ├── tempo.rs    the toolbar's tempo: BPM field (drag or type), beat light, tap tempo
│   ├── macos.rs    native windows: toolbar controls don't drag the window; plugin editors float
│   ├── theme.rs    colors, system fonts and style
│   └── app.rs      window, toolbar, panels, demo patch
├── tests.rs        tests of the graph, the modules and offline engine rendering
└── perf.rs         benchmarks (audio per module, large patches, UI frames, auto layout)
```

The UI never blocks the audio thread: knob values travel through atomics, while structural changes
(nodes added/removed, a new processing order) reach the engine as commands on a channel.
Buffers (delay, reverb, granular loop) are allocated on the UI thread when the node is created.
VST3 plugins are shared through a lock: the UI takes it for one call at a time, at most ten times a
second, and the plugin's state is read again only when it may have changed, so the audio thread is not
kept waiting.

**Performance.** The UI redraws at full rate only during interaction and animations; meters, analyzers and
step lights update at 30 fps (not at all while the window is minimized). Zoom uses font sizes rounded to
half a point, so animations don't re-create glyphs every frame; from far away, unreadable text is not drawn
and the grid is a single mesh. Auto layout and zoom transitions follow a spring curve with no bounce: nodes
set off in a wave from left to right, and zoom proceeds on a logarithmic scale around the center, with no
sideways drift.

To add a new module: a file in `audio/modules/` with its `ModuleDef` (knobs, ports, modes, toggle) and a
`Processor`, an entry in `ModuleKind` and, in `ui/modules.rs`, the readout and the display.
The panel (ports, selector, knobs) is generated from the definition.

## Tests

```bash
cargo test
```

Benchmarks don't run with the normal tests:

```bash
cargo test --release perf -- --ignored --nocapture --test-threads=1
```
