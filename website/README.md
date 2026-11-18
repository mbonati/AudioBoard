# AudioBoard website

A static site: plain HTML, one stylesheet and one small script, no build step.

| Page | What it holds |
|------|---------------|
| `index.html` | The landing page: the hero patch, a playable Web Audio drone sketch, the spotlight instruments, a wall of panels, features, the catalogue by group (VST3, Audio Unit and plugin nodes included), getting started, the architecture in brief |
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

The text follows the main `README.md`, which stays the source of truth (and `sdk.html` follows `plugins/README.md`, the plugin guide; its Tremolo listings are that guide's code, which is built and tested): when a node changes there, change its card in `nodes.html` (and its spotlight in `index.html`, if it has one); when a node is added, also update the counts (the filter chips in `nodes.html`, the catalogue tiles and "Thirty-four nodes" in `index.html`).

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

and remove the line again. `patch-*-1x.png` are half-size copies for the page (`sips -Z 2400 patch-hero.png --out patch-hero-1x.png`). `patch-board` is the demo patch dressed with stickers on the Blueprint background, for the guide's Stickers and background section. `iris-<scene>` are Iris's nineteen scenes, each on a palette of its own (`SNAP_IRIS_PALETTE=2` puts them all on one), following a Drone and Pops 66; `iris-frameless` is the Halo with no frame, right on the board; `iris-layers` is two layers (Flow mirrored with echoes, the Halo on top) moved by the matrix; `iris-ui-layers` and `iris-ui-mod` show the Layers and Modulation tabs, cropped to the node and the popup (`sips -c 1240 1940 --cropOffset 640 2020`, then `sips -Z 970 … --out …-1x.png`); `iris-ui` shows its scene's name, gear and grip with the settings open, cropped to the nodes and the popup (`sips -c 1515 2880 --cropOffset 805 940 iris-ui.png`, then `sips -Z 1440 iris-ui.png --out iris-ui-1x.png`; `SNAP` matches prefixes, so `SNAP=iris-ui` renders the two tabs too). `splitter`, `splitter-seq` and `splitter-midi` are Splitter's three pages: the first and last with a two-bar glitch loop the renderer makes itself, cut at its transients, the second with the built-in atoms on their Euclidean rings. `plugin-nodes` and `plugin-glass` build the example plugin (`plugins/example-nodes`, with cargo, into `target/node-plugin-test`) and load it as the app does. `SNAP_DIR` writes the pictures somewhere else, to look at them without touching the site's.
