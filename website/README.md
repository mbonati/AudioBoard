# AudioBoard website

A static site: plain HTML, one stylesheet and one small script, no build step.

| Page | What it holds |
|------|---------------|
| `index.html` | The landing page: the hero patch, a playable Web Audio drone sketch, the spotlight instruments, a wall of panels, Listen (three films of patches playing on one screen with a picker, and a player for five one-minute example tracks with their waveforms; the hero and every page's menu link to it), features, the catalogue by group (VST3, Audio Unit and plugin nodes included), getting started, the architecture in brief |
| `nodes.html` | The catalogue: every node with its panel, filter by group, search, and an inspector with the full description (`nodes.html#node-fjord` opens one directly); a node with many pictures (Iris's nineteen scenes) shows them as captioned thumbnails |
| `guide.html` | Build and run, patching and the selection bar, transport and tempo (with external MIDI sync), controls, modules, auto layout, palette, stickers and background, the Iris visuals, the Settings tabs (Audio, MIDI, Sync, Plugins), buffer and limiter, recording and export, projects (Open Recent) and library |
| `architecture.html` | The two threads, the engine, plugins (VST3, the Audio Unit host, plugin nodes and their C ABI) and MIDI (clock sync), performance, the source tree, adding a module |
| `sdk.html` | Writing a node plugin with the node SDK: creating the crate, a complete node (Tremolo), building, installing, testing, other kinds of nodes, the generic panel, audio-thread rules, versions and the ABI, distributing, refusal messages, the UI Kit |

## Viewing it

From the repository root, serve it and open it in the browser:

```bash
website/serve.sh
```

It serves on http://localhost:8000 (or the next free port); `website/serve.sh 9000` picks another port, `--no-open` leaves the browser alone, and Ctrl+C stops it. It needs only `python3`. Opening `index.html` straight from disk works too.

The main font is [Chillax](https://www.fontshare.com/fonts/chillax) (Indian Type Foundry, free for personal and commercial use), loaded from Fontshare's CDN by `assets/css/fonts.css`; labels and code use JetBrains Mono from Google Fonts. Offline, the pages fall back to the system fonts.

The text follows the main `README.md`, which stays the source of truth (and `sdk.html` follows `plugins/README.md`, the plugin guide; its Tremolo listings are that guide's code, which is built and tested): when a node changes there, change its card in `nodes.html` (and its spotlight in `index.html`, if it has one); when a node is added, also update the counts (the filter chips in `nodes.html`, the catalogue tiles and "Thirty-five nodes" in `index.html`).

## The screenshots

Every picture in `assets/shots/` is the app's own canvas, rendered offscreen at 2× while an offline engine plays: no mock-ups. `tools/site_snapshots.rs` builds each scene (a node and the nodes feeding it, or a whole patch), runs the real canvas for 300 frames, rasterizes egui's triangles on the CPU and converts the result with `sips` (macOS, which also provides the SF fonts the app uses).

It is not part of the build. To render the pictures again, add this line at the end of `src/tests.rs`:

```rust
include!("../website/tools/site_snapshots.rs");
```

then run all of them, or only some (`SNAP` takes name prefixes):

```bash
cargo test site_snapshots -- --ignored --nocapture
```

```bash
SNAP=equalizer,patch-hero cargo test site_snapshots -- --ignored --nocapture
```

and remove the line again. `patch-*-1x.png` are half-size copies for the page (`sips -Z 2400 patch-hero.png --out patch-hero-1x.png`). `patch-board` is the demo patch dressed with stickers on the Blueprint background, for the guide's Stickers and background section. `iris-<scene>` are Iris's nineteen scenes, each on a palette of its own (`SNAP_IRIS_PALETTE=2` puts them all on one), following a Drone and Pops 66; `iris-frameless` is the Halo with no frame, right on the board; `iris-layers` is two layers (Flow mirrored with echoes, the Halo on top) moved by the matrix; `iris-ui-layers` and `iris-ui-mod` show the Layers and Modulation tabs, cropped to the node and the popup (`sips -c 1240 1940 --cropOffset 640 2020`, then `sips -Z 970 … --out …-1x.png`); `iris-ui` shows its scene's name, gear and grip with the settings open, cropped to the nodes and the popup (`sips -c 1515 2880 --cropOffset 805 940 iris-ui.png`, then `sips -Z 1440 iris-ui.png --out iris-ui-1x.png`; `SNAP` matches prefixes, so `SNAP=iris-ui` renders the two tabs too). `splitter`, `splitter-seq` and `splitter-midi` are Splitter's three pages: the first and last with a two-bar glitch loop the renderer makes itself, cut at its transients, the second with the built-in atoms on their Euclidean rings. `orrery` and `orrery-cycle` are Orrery in GRID, every ring brought in and two turned, and in CYCLE, eight ring lengths twisted under one hand and kept on pattern button 7 as "Eight rings". `plugin-nodes` and `plugin-glass` build the example plugin (`plugins/example-nodes`, with cargo, into `target/node-plugin-test`) and load it as the app does. `tracks` is the track view (the app's README, "Track view") of a small patch: an instrument played by a sequencer through an arpeggiator, a drone through a modulated filter split into a reverb and grains, drums into Tape with its panel open, and the Console's master row; it runs the track view instead of the canvas. `SNAP_DIR` writes the pictures somewhere else, to look at them without touching the site's.

## The tracks and films

`assets/media/` holds the Listen section's sound: five one-minute tracks (`<name>.m4a`, AAC 192 kb/s) and three 40-second films (`film-<name>.mp4`, H.264 1920×1080 at 30 fps with AAC sound, a `-poster.jpg` and a 480-pixel `-thumb.jpg` for the film picker each). They are recorded, not composed by hand: `tools/site_media.rs` builds each patch from built-in nodes (Aurora: Vesper, Granular and Reverb; Orbits: Orrery playing Pops 66 through Tape, over Vesper; Phases: Orrery in CYCLE playing Fjord's Marimba Box, over a Drone; Raster: Nullpunkt, Splitter and a filtered Drone; Golden Hour: Pops 66's Bossanova over Vesper), plays it on the offline engine with its cues (pattern recalls, scene changes, fader rides), stops the transport a few seconds before the end so the tails ring out, and brings every take to the same loudness (gated RMS −17 dBFS, peaks under −1 dBFS) with `afconvert`. The films run the real canvas over the same engine, a camera gliding from the whole patch to each node and back, rasterized at 1.5× on all cores and encoded by `tools/encode.swift` (AVFoundation; it is compiled on demand into `target/site-media`). `assets/media/peaks.js` holds the waveforms the player draws: the loudest sample and the RMS in each of 240 slices of a track.

On the page, the films share one screen (the thumbnails under it switch it), and the tracks one player: the track on top with its waveform (click or arrow keys to seek, the time shown under the pointer), the list under it with a small waveform per track, previous and next, and the keyboard's media keys through the Media Session API. One thing plays at a time: a track, a film or the hero's drone sketch.

It is included by the snapshot tool, so the same line at the end of `src/tests.rs` brings it in. Then, in release (a film takes about three minutes):

```bash
cargo test --release site_media -- --ignored --nocapture --exact tests::site_snap::site_media
```

```bash
MEDIA=orbits,film-orbits cargo test --release site_media -- --ignored --nocapture --exact tests::site_snap::site_media
```

`MEDIA` takes name prefixes, as `SNAP` does (`film-` for the three films). `site_media_probe` renders each mixer channel alone and prints its levels, and draws a spectrogram of the mix into `SNAP_DIR`: the way to balance a piece without listening to it.
