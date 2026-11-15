# AudioBoard website

A static site: plain HTML, one stylesheet and one small script, no build step.

| Page | What it holds |
|------|---------------|
| `index.html` | The landing page: the hero patch, a playable Web Audio drone sketch, the spotlight instruments, a wall of panels, features, the catalogue by group, getting started, the architecture in brief |
| `nodes.html` | The catalogue: every node with its panel, filter by group, search, and an inspector with the full description (`nodes.html#node-fjord` opens one directly) |
| `guide.html` | Build and run, patching, transport and tempo, controls, modules, auto layout, palette, audio settings, recording and export, projects and library |
| `architecture.html` | The two threads, the engine, plugins and MIDI, performance, the source tree, adding a module |

## Viewing it

From the repository root, serve it and open it in the browser:

```bash
website/serve.sh
```

It serves on http://localhost:8000 (or the next free port); `website/serve.sh 9000` picks another port, `--no-open` leaves the browser alone, and Ctrl+C stops it. It needs only `python3`. Opening `index.html` straight from disk works too.

The fonts (Unbounded, Space Grotesk, JetBrains Mono) come from Google Fonts; offline, the pages fall back to the system fonts.

The text follows the main `README.md`, which stays the source of truth: when a node changes there, change its card in `nodes.html` (and its spotlight in `index.html`, if it has one).

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

and remove the line again. `patch-*-1x.png` are half-size copies for the page (`sips -Z 2400 patch-hero.png --out patch-hero-1x.png`).
