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

On launch an ambient demo patch opens, stopped: press **Start** in the toolbar (or Space) to hear it. The patch: a **Drone** (a chord of fifths on A1) goes into a **Filter** whose
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

**Frakture**: a granular rhythm effect after Excite Audio's Motion: Fractal. Its panel has three pages.

- **EFFECTS**: four effects in a chain. Drag their names in the ROUTING row to change the order.
  - **Grains** records its input into a buffer locked to the tempo. BUFFER sets its length, from 1/4 to
    4 bars. RETRIG starts it over when the transport starts; FREEZE stops recording. A LO–HI band filters
    the grains. Two grain layers (1, 2, each with its own ON switch) play from the buffer:
    - PLAYHEAD has five modes: MANual position, SCAN (moves through the buffer forwards or backwards),
      and an offset behind the recording, as a note value (SYNC), in ms or in % (a granular delay).
    - RANGE (spread around the playhead), DENSITY and SIZE (free, or note values with SYNC).
    - CHAOS (randomizes width, direction and shape), DIRECTION (−100% reversed … +100% forwards).
    - PITCH and SPREAD, snapped by the pitch mode: free, chromatic, octaves, fourths, fifths, 4ths & 5ths.
    - WIDTH (how likely grains are panned out), SHAPE (from a percussive decay through a bell and a
      trapezoid to a square gate), DRY and WET.
  - **Gate** is a trance gate of up to 32 steps (click to switch them), with RATE, ATTACK, RELEASE and MIX.
  - **Repeat** is a stutter. Once per TRIGGER (OFFSET into it) it takes a slice RATE long (a note value
    with SYNC, else ms) and plays it AMOUNT more times. DUCK lowers the dry signal meanwhile; it has a
    LO–HI band and a VOLUME.
  - **Space** is a hall reverb with LENGTH and MIX.
  - VIEW shows displays: the grains over the buffer (position across, pitch ±36 up and down) with both
    layers' grain shapes, the gate's levels before and after, and the slice being repeated.
- **HEXAGON**: six macro sliders around a centre, and IN, OUT, WET, the LIMIT safety limiter (never
  above 0 dB) and THRU (off, nothing new comes in and the grains play out). BYPASS sits in the header.
  - Mapping: right-click any effect knob to map it to slider 1…6, or to the input level (A,
    audio-reactive with GAIN, ATTACK, RELEASE, THRESH). The list on the right gives each mapping a MIN and
    a MAX (drag them), × to remove, + ADD and CLEAR. A mapped knob follows the hexagon and shows its
    slider's number; click the number to see that slider's list.
  - The mouse over the hexagon is a magnet. ATTRACT pulls the nodes along their sliders towards it,
    REPEL pushes them away; SELECT lets you drag the nodes to set where each slider rests. It works
    CONSTANTly, only while clicked (CLICK ON) or only while not clicked (CLICK OFF). The pixel blob in the
    middle breathes with the output.
- **MOTION**:
  - Cursor: the magnet's RANGE and STRENGTH. A node the magnet lets go stays put for HOLD (∞ at the
    top), then returns to its start over RELEASE.
  - Path (DRAW): tools LINE, CIRCLE, SELECT (move points, drag a line's handle to bend it), ADD, REMOVE,
    SCALE (corners resize, elsewhere moves) and MOUSE. MOUSE, or a right-click on a line, sets whether the
    mouse counts as clicked along that line.
  - Path (RECORD): ● REC, or R over the hexagon, records the mouse and its clicks (up to 30 s). START–END
    and ✂ TRIM cut it.
  - Timeline: ▶ plays the path, and the magnet follows it. The pass lasts TIME × MULTIPLY (note values
    with TIME SYNC, else seconds), or FREE (the recording's own length). The cursor moves evenly
    (TIMELINE) or at the recorded speed (RECORDED). Direction →, ← or ⇄ pendulum; LOOP; OFFSET into the
    path. Retrigger: OFF, ON (starts over with the transport) or SYNC (locked to the transport).

A new Frakture starts with a circular path and six mappings on grain layer 1 and the reverb: press ▶ PATH
and it moves on its own. Its settings, paths and mappings are saved with the project. To keep one as a
preset, group the node (Cmd+G) and save it to the library.

**Resonator**: a bank of eight tuned resonant modes (String, Pipe, Bar, Bell) excited by its input.
With noise or grains as input it gives physical, organic timbres. PITCH, DECAY, BRIGHT, SPREAD, P.MOD, MIX.

### Effects

**Filter**: stereo state-variable filter, LP / BP / HP / Notch. CUTOFF, RESO, DRIVE, MOD
(depth of the CUT input in octaves, negative too). The display shows the frequency response.

**Delay**: stereo tape echo up to 2 s. TIME (changes glide, like tape), FEEDBACK,
TONE (darkens the repeats), WOW (wow & flutter), PING (ping-pong), MIX. **SYNC** locks the time to the
global tempo: DIV picks the note value (1/32 to 1/1, with dotted "D" and triplet "T" values, up to 4 s) and
follows tempo changes; the knob not in use (TIME or DIV) is dimmed. The readout shows the value and the
time ("1/8D · 375 ms").

**Ping Pong**: a stereo delay whose echoes bounce from side to side: the input (summed to mono) enters one
side and every repeat crosses to the other. TIME or, with **SYNC** (on by default), DIV as for the Delay;
FEEDBACK; WIDTH (100%: hard left and right; 0%: every echo in the middle); TONE (darkens the repeats); DUCK
(lowers the echoes while the input plays, so they bloom in the gaps); MIX. **L → R / R → L** picks the side
of the first echo. The display shows the echoes alternating left (up) and right (down).

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

### MIDI sequences: Step Sequencer, MIDIcer, Arpeggiator, MIDI In

Besides audio cables there are **MIDI cables**: square ports (audio ports are round) and dashed cables.
A MIDI cable only joins a MIDI output to a MIDI input. VST3 plugins that accept MIDI (instruments), the
Arpeggiator, MIDIcer and MIDI Note have a MIDI input.

**Start / Stop** (toolbar, or Space) drive the transport, not the audio: Stop ends the sound's generation
but lets it die away. Sequencers, arpeggiators and MIDIcer stop and end the notes they were holding (VST
instruments and MIDI Note release them as usual), oscillators and Drones fade out in a few milliseconds,
while effects (Delay, Ping Pong, Reverb, Granular, Resonator, Filter) and plugins keep running, so their
tails ring on. Stop also goes back to the start: on the next Start everything begins again from its first
step. The MIDI keyboard keeps playing plugins while stopped. The app opens stopped.

The **tempo** is global: the **BPM** field in the toolbar. Drag it to change it (Shift for fine steps),
click it to type a value (Enter sets it, Esc cancels; "97,5" works too), Alt+click to go back to 90.
**TAP** (or the T key) sets it from your taps: tap on each beat, and the last few intervals are averaged;
a pause of two seconds starts a new count. The light flashes on every beat while playing. Sequencers
and arpeggiators are locked to the same grid, so they always stay in time with each other; the tempo is
also passed to plugins (synced LFOs and delays). It is saved in the project.

**Step Sequencer**: up to 16 steps on a scale. In the grid each column is a step and the lit cell is its
note: click a cell to move the step there (on the lit cell, to switch it off or back on), drag to draw a
melody; hovering shows the note. RATE (from 1/4 to 1/32, triplets), STEPS (length), GATE (note length),
SWING (delays the even-numbered steps), CHANCE (probability that a step plays), VELOCITY, ROOT and OCTAVE
(the tonic), scale MAJ / MIN / DOR / PENT / CHR. **PLAY** stops and restarts it. Steps are saved in the
project and in library modules.

**MIDIcer**: a stochastic sequencer after Vermona's meloDICER: random values turn into notes only through
your settings, so it is played like an instrument.
- *Rhythm*: **NOTE** (base note value, 1/1 to 1/32 with triplets), **VARIATION** (off at the centre; to the
  left it randomly brings in longer values, to the right shorter ones), **LEGATO** (probability that a note
  runs into the next one without a new attack: overlapping notes, or a tie on the same pitch; at 100% every
  note is legato) and **REST** (probability that a note becomes a rest).
- *Melody*: **LOW** and **HIGH** set the note range; the display has a probability fader for each semitone
  (click or drag; at the bottom it is off). Each note in the range plays as often as its semitone's fader
  says; a fader lights while its note plays.
- **DICE R / DICE M** keep a set of random values for the rhythm / melody, so the pattern repeats; click
  again for new values. **LIVE R / LIVE M** draw new values all the time (no repeats); DICE brings the kept
  values back. With separate sections, the rhythm can repeat while the melody keeps changing, or vice versa.
- **FIRST / LAST** set the pattern's length in sixteenths (up to four beats), shown on the step strip under
  the faders. **RESTART** starts it again from FIRST on the next beat. **MUTE** silences it while the pattern
  runs. **LOCK** keeps playing with the current settings while you prepare new ones; unlocking applies them.
- MIDI: **GATE** (note length for non-legato notes), **VELOCITY**, **DYNAMICS** (random softening) and the
  output **CHANNEL**.
- **SETUP** holds meloDICER's edit parameters: whether VARIATION may pick triplets and 1/32 notes, whether
  unmuting restarts the pattern, and the routing of the two CV inputs: **CV1** (pitch) as pitch bend
  (unquantized, like ADD SEQ) or transposition (±24 semitones, like TRANS SEQ), and/or moving LOW and HIGH;
  **CV2** (rhythm) added to NOTE, VARIATION, LEGATO and/or REST.
- Modes: **SEQ** (the sequencer); **SEQ+G** (no rhythm of its own: every note arriving on its MIDI input
  plays a new note from the melody section, with LEGATO and REST); **QUANT1** (CV2 quantized to the enabled
  notes on every beat; a raised fader pulls from further away); **QUANT2** (CV2 quantized while a note is held
  on the MIDI input).
- In SEQ mode, notes on its MIDI input are commands, like meloDICER's gate inputs: **C** restart, **D** / **E**
  dice the rhythm / melody anew, **F** / **G** switch the rhythm / melody between DICE and LIVE, **A** mutes
  while held. Without a MIDI cable these come from the MIDI keyboard.
- The tempo is the global one (with TAP in the toolbar). Settings, faders and the kept random values are
  saved in the project and in library modules, like meloDICER's patterns.

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

A plugin is called once per audio buffer (as in a DAW), so a heavy instrument costs the same whatever the
patch around it. Only its main stereo outputs are used: extra outputs (a drum machine's separate drum
channels) are switched off at load so the plugin doesn't render them. While the transport is stopped, a
plugin with nothing coming in rests after a second of silence; any MIDI or sound wakes it at once. A
plugin's sound (its complete state) is read when the project is saved: some plugins take a while to
answer, so saving while playing may briefly interrupt them.

Plugins run inside AudioBoard: a faulty plugin can make the application quit. VST3 is supported
(not VST2, AU or CLAP).

### Output

**Mixer (4 channels)**: for each channel a volume fader, pan (balance on stereo signals), Mute/Solo
and a meter; a stereo master bus with L/R meters and a clip indicator. The mixer feeds the audio device.

### Audio buffer, load and limiter

The status bar shows the audio buffer in samples and its latency ("512 smp · 10.7 ms"): click it to choose
another size, as in a DAW. Larger buffers give heavy patches and plugins more headroom (fewer clicks) at
the cost of latency; smaller ones respond faster. The choice is kept between launches; "Device default"
uses the sound card's own size. Changing it keeps the patch playing.

Next to it, **DSP** is the average load and **peak** the heaviest callback of the last seconds: near 100%
the audio can't keep up. If that happens, a red **dropouts** counter appears for a while (it also counts
the gaps the sound card reports): try a larger buffer.

The output goes through a safety **limiter** just under full scale (−0.3 dBFS) with 1.3 ms of lookahead:
when the sum gets too loud (several plugins, high faders) it turns the level down smoothly instead of
clipping it into crackles, and recovers over about 150 ms. While it works, the status bar shows
"limit −x dB": lower the master or the plugins. Below full scale it does nothing.

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
| Start / stop | the Start / Stop button in the toolbar, or Space |
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
│   ├── engine.rs   audio-side graph: one pass per device buffer (built-ins in 64-frame stereo blocks,
│   │               plugins in one call), per-block MIDI events, global clock in beats, precomputed
│   │               topological order, lookahead limiter, denormals flushed to zero
│   ├── fastmath.rs fast sine, exp2 and tanh for per-sample DSP (error < 1e-6)
│   ├── dsp.rs      Oscillator and Mixer
│   ├── modules/    drone, random, granular, resonator, filter, delay, pingpong, reverb, midi_cc, midi_note,
│   │               sequencer, arpeggiator, midi_in, midicer (after Vermona's meloDICER)
│   │   ├── frakture.rs Frakture (after Excite Audio's Motion: Fractal): parameters, mappings, the chain
│   │   ├── frakture/   grains (tempo-locked buffer, two layers), rhythm (gate, repeat), space (hall),
│   │   │               motion (hexagon sliders, magnet, drawn/recorded paths, timeline)
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
│   ├── frakture.rs the Frakture node's panel: HEXAGON, EFFECTS and MOTION pages
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
VST3 plugins are shared through a lock: the UI calls into a plugin only between audio callbacks (when
the audio thread has just finished and won't need the plugin for most of a buffer), one call at a time, at
most ten times a second, and the plugin's state is read only when saving, so the audio thread is not kept
waiting. Each buffer is processed as one pass: built-in nodes run through it in 64-frame blocks (so MIDI
timing and modulation stay fine-grained), while a plugin gets the whole buffer in a single call with its
MIDI events placed at their exact offsets, since every plugin call has a fixed cost. Removed nodes and old
processing orders are handed back to the UI thread to be freed: freeing memory (a delay line is megabytes)
inside the audio callback could stall it into a click.

**Performance.** Effects rest when they have gone quiet: with a silent input, once their own signal (echoes,
tails) has stayed below −100 dB for as long as they can remember sound, they skip their work and output
silence until input returns; a frozen reverb or an endless delay never rests. Stopped sources rest too, so a
stopped patch costs almost nothing once its tails are gone. Denormal numbers are flushed to zero, and the
per-sample maths uses fast approximations of sine, exp2 and tanh. The UI redraws at full rate only during interaction and animations; meters, analyzers and
step lights update at 30 fps (not at all while the window is minimized). Zoom uses font sizes rounded to
half a point, so animations don't re-create glyphs every frame; from far away, unreadable text is not drawn
and the grid is a single mesh. Auto layout and zoom transitions follow a spring curve with no bounce: nodes
set off in a wave from left to right, and zoom proceeds on a logarithmic scale around the center, with no
sideways drift.

To add a new module: a file in `audio/modules/` with its `ModuleDef` (knobs, ports, modes, toggle) and a
`Processor`, an entry in `ModuleKind` and, in `ui/modules.rs`, the readout and the display.
The panel (ports, selector, knobs) is generated from the definition; Frakture, too large for the generic
panel, draws its own (`ui/frakture.rs`). Its hexagon runs on the audio thread, block by block: the panel
hands it the mouse through the module's display values, so paths keep time even when the window doesn't
redraw.

## Tests

```bash
cargo test
```

Benchmarks don't run with the normal tests:

```bash
cargo test --release perf -- --ignored --nocapture --test-threads=1
```
