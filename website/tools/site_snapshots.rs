// Renders AudioBoard's node panels and whole patches offscreen, as the PNGs
// in website/assets/shots. Not part of the build: see website/README.md.
//
// It drives the real canvas (crate::ui::canvas::Editor) over an offline
// engine that plays, tessellates egui's last frame, rasterizes the triangles
// on the CPU into a BMP and converts it with macOS's `sips`.
mod site_snap {
    use super::*;
    use std::collections::HashMap;

    use egui::epaint::{ClippedPrimitive, Primitive};
    use egui::{Color32, Pos2, Rect, TextureId, TexturesDelta, Vec2, vec2};

    use crate::audio::modules as md;
    use crate::audio::engine::NodeId;
    use crate::graph::MIDI_PORT;
    use crate::ui::canvas::Editor;
    use crate::ui::theme;

    const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/website/assets/shots");

    struct Tex {
        w: usize,
        h: usize,
        px: Vec<[f32; 4]>,
    }

    fn rgba(c: Color32) -> [f32; 4] {
        [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0, c.a() as f32 / 255.0]
    }

    fn apply(texs: &mut HashMap<TextureId, Tex>, delta: &TexturesDelta) {
        for (id, list) in &delta.set {
            for d in list {
                let egui::ImageData::Color(img) = &d.image;
                match d.pos {
                    None => {
                        texs.insert(*id, Tex { w: img.size[0], h: img.size[1], px: img.pixels.iter().map(|c| rgba(*c)).collect() });
                    }
                    Some([x0, y0]) => {
                        let t = texs.get_mut(id).expect("texture to patch");
                        for y in 0..img.size[1] {
                            for x in 0..img.size[0] {
                                t.px[(y0 + y) * t.w + x0 + x] = rgba(img.pixels[y * img.size[0] + x]);
                            }
                        }
                    }
                }
            }
        }
    }

    fn sample(t: &Tex, u: f32, v: f32) -> [f32; 4] {
        let x = u * t.w as f32 - 0.5;
        let y = v * t.h as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |xi: f32, yi: f32| {
            let xi = (xi as isize).clamp(0, t.w as isize - 1) as usize;
            let yi = (yi as isize).clamp(0, t.h as isize - 1) as usize;
            t.px[yi * t.w + xi]
        };
        let (a, b, c, d) = (at(x0, y0), at(x0 + 1.0, y0), at(x0, y0 + 1.0), at(x0 + 1.0, y0 + 1.0));
        let mut out = [0.0; 4];
        for i in 0..4 {
            let top = a[i] + (b[i] - a[i]) * fx;
            let bot = c[i] + (d[i] - c[i]) * fx;
            out[i] = top + (bot - top) * fy;
        }
        out
    }

    /// Rasterizes egui's triangles (premultiplied, blended in gamma space).
    fn raster(prims: &[ClippedPrimitive], texs: &HashMap<TextureId, Tex>, w: usize, h: usize, ppp: f32) -> Vec<[f32; 3]> {
        let bg = rgba(theme::BG);
        let mut img = vec![[bg[0], bg[1], bg[2]]; w * h];
        for cp in prims {
            let Primitive::Mesh(mesh) = &cp.primitive else { continue };
            let tex = texs.get(&mesh.texture_id);
            let cx0 = (cp.clip_rect.min.x * ppp).max(0.0).floor() as i64;
            let cy0 = (cp.clip_rect.min.y * ppp).max(0.0).floor() as i64;
            let cx1 = (cp.clip_rect.max.x * ppp).min(w as f32).ceil() as i64;
            let cy1 = (cp.clip_rect.max.y * ppp).min(h as f32).ceil() as i64;
            for tri in mesh.indices.chunks_exact(3) {
                let mut vs = [tri[0], tri[1], tri[2]].map(|i| &mesh.vertices[i as usize]);
                let mut p = vs.map(|v| (v.pos.x * ppp, v.pos.y * ppp));
                let mut area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[1].1 - p[0].1) * (p[2].0 - p[0].0);
                if area.abs() < 1e-9 {
                    continue;
                }
                if area < 0.0 {
                    vs.swap(1, 2);
                    p.swap(1, 2);
                    area = -area;
                }
                let minx = p[0].0.min(p[1].0).min(p[2].0);
                let maxx = p[0].0.max(p[1].0).max(p[2].0);
                let miny = p[0].1.min(p[1].1).min(p[2].1);
                let maxy = p[0].1.max(p[1].1).max(p[2].1);
                let x0 = (minx.floor() as i64).max(cx0);
                let x1 = (maxx.ceil() as i64).min(cx1);
                let y0 = (miny.floor() as i64).max(cy0);
                let y1 = (maxy.ceil() as i64).min(cy1);
                if x0 >= x1 || y0 >= y1 {
                    continue;
                }
                let cols = vs.map(|v| rgba(v.color));
                let uvs = vs.map(|v| (v.uv.x, v.uv.y));
                // Edge k is opposite vertex k; a pixel centre exactly on an
                // edge belongs to one of the two triangles sharing it.
                let edges = [(p[1], p[2]), (p[2], p[0]), (p[0], p[1])];
                let owns = edges.map(|(a, b)| {
                    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                    dy > 0.0 || (dy == 0.0 && dx > 0.0)
                });
                for y in y0..y1 {
                    let py = y as f32 + 0.5;
                    for x in x0..x1 {
                        let px = x as f32 + 0.5;
                        let mut wts = [0.0f32; 3];
                        let mut inside = true;
                        for k in 0..3 {
                            let (a, b) = edges[k];
                            let e = (a.0 - px) * (b.1 - py) - (a.1 - py) * (b.0 - px);
                            if e < 0.0 || (e == 0.0 && !owns[k]) {
                                inside = false;
                                break;
                            }
                            wts[k] = e / area;
                        }
                        if !inside {
                            continue;
                        }
                        let mut src = [0.0f32; 4];
                        for i in 0..4 {
                            src[i] = cols[0][i] * wts[0] + cols[1][i] * wts[1] + cols[2][i] * wts[2];
                        }
                        if let Some(t) = tex {
                            let u = uvs[0].0 * wts[0] + uvs[1].0 * wts[1] + uvs[2].0 * wts[2];
                            let v = uvs[0].1 * wts[0] + uvs[1].1 * wts[1] + uvs[2].1 * wts[2];
                            let s = sample(t, u, v);
                            for i in 0..4 {
                                src[i] *= s[i];
                            }
                        }
                        if src[3] <= 0.0 && src[0] <= 0.0 && src[1] <= 0.0 && src[2] <= 0.0 {
                            continue;
                        }
                        let d = &mut img[y as usize * w + x as usize];
                        let k = 1.0 - src[3].clamp(0.0, 1.0);
                        for i in 0..3 {
                            d[i] = (src[i] + d[i] * k).clamp(0.0, 1.0);
                        }
                    }
                }
            }
        }
        img
    }

    fn save(name: &str, img: &[[f32; 3]], w: usize, crop: (usize, usize, usize, usize)) {
        let (cx, cy, cw, ch) = crop;
        let row = (cw * 3 + 3) & !3;
        let mut bmp = Vec::with_capacity(54 + row * ch);
        let size = (54 + row * ch) as u32;
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&size.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&(cw as i32).to_le_bytes());
        bmp.extend_from_slice(&(ch as i32).to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&24u16.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&((row * ch) as u32).to_le_bytes());
        bmp.extend_from_slice(&2835u32.to_le_bytes());
        bmp.extend_from_slice(&2835u32.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        for y in (0..ch).rev() {
            let start = bmp.len();
            for x in 0..cw {
                let c = img[(cy + y) * w + cx + x];
                let b = |v: f32| (v * 255.0 + 0.5) as u8;
                bmp.extend_from_slice(&[b(c[2]), b(c[1]), b(c[0])]);
            }
            while bmp.len() - start < row {
                bmp.push(0);
            }
        }
        // SNAP_DIR: somewhere else than the site's pictures.
        let dir = std::env::var("SNAP_DIR").unwrap_or_else(|_| DIR.to_owned());
        std::fs::create_dir_all(&dir).unwrap();
        let tmp = format!("{dir}/{name}.bmp");
        let png = format!("{dir}/{name}.png");
        std::fs::write(&tmp, bmp).unwrap();
        let ok = std::process::Command::new("sips")
            .args(["-s", "format", "png", &tmp, "--out", &png])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(ok, "sips failed for {name}");
        std::fs::remove_file(&tmp).ok();
        eprintln!("wrote {png} ({cw}×{ch})");
    }

    struct Scene {
        g: Graph,
        a: AudioHandle,
        e: Engine,
    }

    fn scene(tempo: f32) -> Scene {
        let shared = Arc::new(EngineShared::new());
        shared.tempo.set(tempo);
        shared.running.store(true, Ordering::Relaxed);
        let (a, e) = audio::offline(shared, 48_000);
        let mut g = Graph::default();
        g.tempo = tempo;
        Scene { g, a, e }
    }

    /// Runs the canvas over `frames` frames of 1/60 s while the engine
    /// plays, and returns the last frame's pixels.
    fn shoot(s: &mut Scene, size: Vec2, ppp: f32, frames: usize, mut each: impl FnMut(&mut Graph, usize)) -> (Vec<[f32; 3]>, usize, usize) {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        ctx.set_pixels_per_point(ppp);
        ctx.options_mut(|o| o.warn_on_id_clash = false);
        ctx.all_styles_mut(|st| st.debug.warn_if_rect_changes_id = false);
        let mut editor = Editor::new();
        let mut texs = HashMap::new();
        let mut prims = Vec::new();
        for f in 0..frames {
            each(&mut s.g, f);
            // The canvas frames the patch on its first pass, before the 2x
            // scale applies: frame it again once it has.
            if f == 3 {
                editor.reset();
            }
            render_buf(&mut s.e, 800);
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                time: Some(f as f64 / 60.0),
                ..Default::default()
            };
            let (g, a) = (&mut s.g, &s.a);
            let mut out = ctx.run_ui(input, |ui| editor.show(ui, g, a));
            apply(&mut texs, &out.textures_delta);
            out.textures_delta.clear();
            if f + 1 == frames {
                prims = ctx.tessellate(out.shapes, out.pixels_per_point);
            }
        }
        let w = (size.x * ppp).round() as usize;
        let h = (size.y * ppp).round() as usize;
        (raster(&prims, &texs, w, h, ppp), w, h)
    }

    fn bounds(g: &Graph) -> Rect {
        g.nodes.iter().map(|n| Rect::from_min_size(n.pos, n.size())).reduce(|a, b| a.union(b)).unwrap()
    }

    const PPP: f32 = 2.0;

    /// One node, cropped out of a canvas that may hold the nodes feeding it.
    fn node_shot(name: &str, frames: usize, s: &mut Scene, target: NodeId) {
        node_shot_with(name, frames, s, target, |_, _| {});
    }

    fn node_shot_with(name: &str, frames: usize, s: &mut Scene, target: NodeId, each: impl FnMut(&mut Graph, usize)) {
        let b = bounds(&s.g);
        let size = b.size() + vec2(200.0, 200.0);
        let node = s.g.node(target).unwrap();
        let r = Rect::from_min_size(node.pos, node.size());
        // The canvas frames everything at 100%: bounds' centre in the middle.
        let pan = size * 0.5 - b.center().to_vec2();
        let (ml, mr, mt, mb) = (44.0, 44.0, 26.0, 30.0);
        let crop = Rect::from_min_max(r.min + pan - vec2(ml, mt), r.max + pan + vec2(mr, mb));
        let (img, w, h) = shoot(s, size, PPP, frames, each);
        let cx = (crop.min.x * PPP).round().max(0.0) as usize;
        let cy = (crop.min.y * PPP).round().max(0.0) as usize;
        let cw = ((crop.width() * PPP).round() as usize).min(w - cx);
        let ch = ((crop.height() * PPP).round() as usize).min(h - cy);
        if std::env::var("SNAP_FULL").is_ok() {
            save(&format!("{name}-full"), &img, w, (0, 0, w, h));
        }
        save(name, &img, w, (cx, cy, cw, ch));
    }

    fn full_shot(name: &str, frames: usize, s: &mut Scene, size: Vec2, ppp: f32) {
        let (img, w, h) = shoot(s, size, ppp, frames, |_, _| {});
        save(name, &img, w, (0, 0, w, h));
    }

    fn add(s: &mut Scene, kind: ModuleKind, x: f32, y: f32) -> NodeId {
        s.g.add_module(kind, pos2(x, y), &s.a)
    }

    fn link(s: &mut Scene, a: NodeId, ap: usize, b: NodeId, bp: usize) {
        s.g.connect(port(a, ap), port(b, bp), &s.a);
    }

    /// A feeder to the left of the node at the origin.
    fn feeder(s: &mut Scene, kind: ModuleKind, y: f32) -> NodeId {
        let w = md::ModuleParams::new(kind);
        let size = crate::ui::modules::size_of(&w);
        add(s, kind, -260.0 - size.x, y)
    }

    fn arrange(s: &mut Scene) {
        for (id, p) in crate::ui::layout::plan(&s.g, None) {
            if let Some(n) = s.g.node_mut(id) {
                n.pos = p;
            }
        }
    }

    /// A two-bar glitch loop made here (sub kicks, clicks, hiss, pings),
    /// loaded into a Splitter and cut at its transients, a few slices
    /// dressed: a ratchet, a frozen cloud, a crushed hit.
    fn splitter_sample(p: &md::ModuleParams) {
        use md::splitter::{self as sp, slice, slicing};
        let sr = 48_000.0f32;
        let n = (sr * 2.0) as usize;
        let mut l = vec![0.0f32; n];
        let mut r = vec![0.0f32; n];
        let step = (sr * 0.125) as usize;
        let mut seed = 0x1234_5678u32;
        let mut noise = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
        };
        let pattern = "K.c.H.p.K.cHc.pHK..cH.p.KcH.c.pH";
        for (i, ch) in pattern.chars().enumerate() {
            let at = i * step / 2;
            let len = match ch {
                'K' => (sr * 0.22) as usize,
                'c' => (sr * 0.01) as usize,
                'H' => (sr * 0.04) as usize,
                'p' => (sr * 0.12) as usize,
                _ => 0,
            };
            for k in 0..len.min(n - at) {
                let t = k as f32 / sr;
                let y = match ch {
                    'K' => (std::f32::consts::TAU * (48.0 * t + 1.6 * (1.0 - (-t / 0.02).exp()))).sin() * (-t / 0.09).exp() * 0.9,
                    'c' => if k == 0 { 0.9 } else { (std::f32::consts::TAU * 3200.0 * t).sin() * (-t / 0.0015).exp() * 0.5 },
                    'H' => noise() * (-t / 0.012).exp() * 0.35,
                    _ => (std::f32::consts::TAU * 1760.0 * t).sin() * (-t / 0.03).exp() * 0.4,
                };
                let pan = if ch == 'H' { 0.3 } else if ch == 'p' { -0.3 } else { 0.0 };
                l[at + k] += y * (1.0 - pan);
                r[at + k] += y * (1.0 + pan);
            }
        }
        let sample = std::sync::Arc::new(crate::audio::sample::Sample::new("glitch-loop.wav", sr, l, r));
        p.sample.set(Some(sample.clone()));
        let regions = slicing::transients(&sample, 0.5, 16);
        slicing::apply(p, &regions, 0);
        sp::set(p, 1, slice::REPEAT, 4.0);
        sp::set(p, 1, slice::RAMP, 7.0);
        sp::set(p, 1, slice::FALL, 25.0);
        sp::set(p, 2, slice::BITS, 4.0);
        sp::set(p, 2, slice::DOWN, 6.0);
        sp::set(p, 3, slice::GRAIN, 1.0);
        sp::set(p, 3, slice::FREEZE, 1.0);
        sp::set(p, 3, slice::POS, 0.3);
        sp::set(p, 3, slice::SPRAY, 0.4);
        sp::set(p, 3, slice::JITTER, 12.0);
        sp::set(p, 3, slice::PLAY, 2.0);
        sp::set(p, 3, slice::REPEAT, 3.0);
        sp::set(p, 3, slice::REP_RATE, 2.0);
        sp::set(p, 3, slice::RAMP, 12.0);
        sp::set(p, 3, slice::FALL, 30.0);
        sp::set(p, 3, slice::BITS, 8.0);
        sp::set(p, 3, slice::DOWN, 3.0);
        sp::set(p, 3, slice::RELEASE, 120.0);
        sp::set(p, 5, slice::REVERSE, 1.0);
        p.data[sp::data::SELECTED].set(3.0);
    }

    const FRAMES: usize = 300;

    fn only() -> Option<String> {
        std::env::var("SNAP").ok()
    }

    fn want(name: &str) -> bool {
        only().is_none_or(|o| o.split(',').any(|w| name.starts_with(w)))
    }

    #[test]
    #[ignore]
    fn site_snapshots() {
        // --- Sources -----------------------------------------------------
        if want("osc") {
            let mut s = scene(110.0);
            let id = s.g.add_osc(pos2(0.0, 0.0), &s.a);
            let p = osc_params(&s.g, id);
            p.set_waveform(Waveform::Saw);
            p.set(osc::FREQ, 110.0);
            node_shot("osc", 60, &mut s, id);
        }
        for (name, kind) in [
            ("drone", ModuleKind::Drone),
            ("aether", ModuleKind::Aether),
            ("larsen", ModuleKind::Larsen),
            ("vesper", ModuleKind::Vesper),
            ("random", ModuleKind::Random),
            ("nullpunkt", ModuleKind::Nullpunkt),
            ("audio-in", ModuleKind::AudioIn),
            ("automa", ModuleKind::Automa),
            ("colloid", ModuleKind::Colloid),
            ("midicer", ModuleKind::Midicer),
            ("sequencer", ModuleKind::Sequencer),
            ("midi-cc", ModuleKind::MidiCc),
            ("midi-note", ModuleKind::MidiNote),
            ("midi-in", ModuleKind::MidiIn),
        ] {
            if !want(name) {
                continue;
            }
            let mut s = scene(110.0);
            let id = add(&mut s, kind, 0.0, 0.0);
            // Vesper renders its sounds on a thread of its own, in real time.
            let slow = kind == ModuleKind::Vesper;
            node_shot_with(name, FRAMES, &mut s, id, |_, f| {
                if slow && f == 10 {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                }
            });
        }
        // Orrery: every ring brought in, two of them turned; then CYCLE, a
        // polyrhythm of eight ring lengths under one hand, kept on a button.
        for (name, cycle) in [("orrery", false), ("orrery-cycle", true)] {
            if !want(name) {
                continue;
            }
            use md::orrery::{self as or, ring};
            let mut s = scene(104.0);
            let id = add(&mut s, ModuleKind::Orrery, 0.0, 0.0);
            let p = module_params(&s.g, id);
            for r in 0..or::RINGS {
                or::set(&p, r, ring::ON, 1.0);
            }
            if cycle {
                p.set_mode(or::CYCLE_MODE);
                p.set(or::CYCLE, 8.0);
                for (r, (n, k)) in [(16, 7), (12, 5), (10, 4), (8, 3), (7, 3), (6, 2), (5, 2), (3, 1)].into_iter().enumerate() {
                    or::set_steps(&p, r, n);
                    or::set_pulses(&p, r, k);
                    or::set_rotation(&p, r, 0.0);
                }
                p.set(or::TWIST, 0.35);
                or::select(&p, 1);
                // Kept on button 7, under a name of its own.
                or::rename_slot(&p, 6, "Eight rings");
                or::store(&p, 6);
            } else {
                or::set_rotation(&p, 5, 1.5);
                or::set_step(&p, 0, 0, or::ACCENTED);
                or::select(&p, 2);
            }
            node_shot(name, FRAMES, &mut s, id);
        }
        if want("patch-midi") {
            let mut s = scene(96.0);
            let automa = add(&mut s, ModuleKind::Automa, 0.0, 0.0);
            let aether = add(&mut s, ModuleKind::Aether, 0.0, 0.0);
            let reverb = add(&mut s, ModuleKind::Reverb, 0.0, 0.0);
            let seq = add(&mut s, ModuleKind::Sequencer, 0.0, 0.0);
            let arp = add(&mut s, ModuleKind::Arpeggiator, 0.0, 0.0);
            let larsen = add(&mut s, ModuleKind::Larsen, 0.0, 0.0);
            let pp = add(&mut s, ModuleKind::PingPong, 0.0, 0.0);
            let midicer = add(&mut s, ModuleKind::Midicer, 0.0, 0.0);
            let looper = add(&mut s, ModuleKind::Looper, 0.0, 0.0);
            let nullp = add(&mut s, ModuleKind::Nullpunkt, 0.0, 0.0);
            let delay = add(&mut s, ModuleKind::Delay, 0.0, 0.0);
            let mixer = s.g.add_mixer(pos2(0.0, 0.0), &s.a);
            link(&mut s, automa, MIDI_PORT, aether, MIDI_PORT);
            link(&mut s, aether, 0, reverb, 0);
            link(&mut s, reverb, 0, mixer, 0);
            link(&mut s, seq, MIDI_PORT, arp, MIDI_PORT);
            link(&mut s, arp, MIDI_PORT, larsen, MIDI_PORT);
            link(&mut s, larsen, 0, pp, 0);
            link(&mut s, pp, 0, mixer, 1);
            link(&mut s, midicer, MIDI_PORT, looper, MIDI_PORT);
            link(&mut s, nullp, 0, delay, 0);
            link(&mut s, delay, 0, mixer, 2);
            arrange(&mut s);
            full_shot("patch-midi", FRAMES, &mut s, vec2(2400.0, 1350.0), PPP);
        }
        if want("pops") {
            let mut s = scene(118.0);
            let id = add(&mut s, ModuleKind::Pops, 0.0, 0.0);
            module_params(&s.g, id).data[md::pops::data::DETAIL].set(2.0);
            node_shot("pops", FRAMES, &mut s, id);
        }
        if want("pops-machine") {
            let mut s = scene(118.0);
            let id = add(&mut s, ModuleKind::Pops, 0.0, 0.0);
            node_shot("pops-machine", FRAMES, &mut s, id);
        }
        for (name, tab) in [("fjord", 0.0), ("fjord-seq", 1.0)] {
            if !want(name) {
                continue;
            }
            let mut s = scene(112.0);
            let id = add(&mut s, ModuleKind::Fjord, 0.0, 0.0);
            module_params(&s.g, id).data[md::fjord::data::TAB].set(tab);
            node_shot(name, FRAMES, &mut s, id);
        }

        // Splitter: a loop of its own cut at its transients (SLICE, MIDI),
        // and the built-in kit on its Euclidean rings (SEQUENCE).
        for (name, tab, mode, loaded) in [
            ("splitter", 0.0, md::splitter::PATTERN, true),
            ("splitter-seq", 1.0, md::splitter::EUCLID, false),
            ("splitter-midi", 2.0, md::splitter::PATTERN, true),
        ] {
            if !want(name) {
                continue;
            }
            let mut s = scene(118.0);
            let id = add(&mut s, ModuleKind::Splitter, 0.0, 0.0);
            let p = module_params(&s.g, id);
            p.data[md::splitter::data::TAB].set(tab);
            p.data[md::splitter::data::MODE].set(mode as f32);
            if loaded {
                splitter_sample(&p);
            }
            node_shot(name, FRAMES, &mut s, id);
        }

        // --- Texture and effects, fed by a source --------------------------
        for (name, kind, src) in [
            ("granular", ModuleKind::Granular, ModuleKind::Drone),
            ("resonator", ModuleKind::Resonator, ModuleKind::Pops),
            ("magma", ModuleKind::Magma, ModuleKind::Drone),
            ("equalizer", ModuleKind::Equalizer, ModuleKind::Pops),
            ("delay", ModuleKind::Delay, ModuleKind::Pops),
            ("pingpong", ModuleKind::PingPong, ModuleKind::Pops),
            ("reverb", ModuleKind::Reverb, ModuleKind::Drone),
        ] {
            if !want(name) {
                continue;
            }
            let mut s = scene(110.0);
            let id = add(&mut s, kind, 0.0, 0.0);
            let f = feeder(&mut s, src, 0.0);
            link(&mut s, f, 0, id, 0);
            node_shot(name, FRAMES, &mut s, id);
        }
        if want("filter") {
            let mut s = scene(110.0);
            let id = add(&mut s, ModuleKind::Filter, 0.0, 0.0);
            let f = feeder(&mut s, ModuleKind::Drone, 60.0);
            let r = feeder(&mut s, ModuleKind::Random, -200.0);
            link(&mut s, f, 0, id, 0);
            link(&mut s, r, 0, id, 1);
            node_shot("filter", FRAMES, &mut s, id);
        }
        if want("prism") {
            // A few bands of every kind: a cut, shelves, a dynamic bell on
            // the mid, a spectral one, with the dynamic bell selected.
            use md::prism::{self as pq, band, bp, data, design::*};
            let mut s = scene(110.0);
            let id = add(&mut s, ModuleKind::Prism, 0.0, 0.0);
            let p = module_params(&s.g, id);
            for (b, (sh, freq, gain, q)) in [
                (LOW_CUT, 32.0, 0.0, 0.9),
                (LOW_SHELF, 120.0, 3.0, 0.7),
                (BELL, 420.0, -4.5, 1.4),
                (BELL, 2600.0, 3.5, 0.9),
                (NOTCH, 5200.0, 0.0, 6.0),
                (HIGH_SHELF, 11_000.0, 2.5, 0.7),
            ]
            .into_iter()
            .enumerate()
            {
                pq::create_band(&p, b, sh, freq, gain, q);
            }
            p.set(bp(2, band::RANGE), -6.0);
            p.data[data::PLACE + 3].set(pq::MID as f32);
            p.set(bp(3, band::RANGE), 4.0);
            p.data[data::SPECTRAL + 3].set(1.0);
            // A bit mask: the dynamic bell, the third band.
            p.data[data::SELECTED].set((1 << 2) as f32);
            let f = feeder(&mut s, ModuleKind::Pops, 0.0);
            link(&mut s, f, 0, id, 0);
            node_shot("prism", FRAMES, &mut s, id);
        }
        if want("tape") {
            let mut s = scene(110.0);
            let id = add(&mut s, ModuleKind::Tape, 0.0, 0.0);
            module_params(&s.g, id).data[md::tape::data::DETAIL].set(1.0);
            let f = feeder(&mut s, ModuleKind::Drone, 0.0);
            link(&mut s, f, 0, id, 0);
            node_shot("tape", FRAMES, &mut s, id);
        }
        for (name, page) in [("frakture", 0.0), ("frakture-effects", 1.0), ("frakture-mutate", 2.0), ("frakture-motion", 3.0)] {
            if !want(name) {
                continue;
            }
            let mut s = scene(110.0);
            let id = add(&mut s, ModuleKind::Frakture, 0.0, 0.0);
            module_params(&s.g, id).data[md::frakture::data::PAGE].set(page);
            let f = feeder(&mut s, ModuleKind::Pops, 0.0);
            link(&mut s, f, 0, id, 0);
            node_shot(name, FRAMES, &mut s, id);
        }

        // --- MIDI fed by MIDI ---------------------------------------------
        for (name, kind) in [("arpeggiator", ModuleKind::Arpeggiator), ("looper", ModuleKind::Looper)] {
            if !want(name) {
                continue;
            }
            let mut s = scene(110.0);
            let id = add(&mut s, kind, 0.0, 0.0);
            let f = feeder(&mut s, ModuleKind::Sequencer, 0.0);
            link(&mut s, f, MIDI_PORT, id, MIDI_PORT);
            node_shot(name, FRAMES, &mut s, id);
        }

        // --- Output -------------------------------------------------------
        if want("mixer") {
            let mut s = scene(110.0);
            let id = s.g.add_mixer(pos2(0.0, 0.0), &s.a);
            let a = feeder(&mut s, ModuleKind::Drone, -120.0);
            let b = feeder(&mut s, ModuleKind::Pops, 160.0);
            link(&mut s, a, 0, id, 0);
            link(&mut s, b, 0, id, 1);
            node_shot("mixer", FRAMES, &mut s, id);
        }
        if want("console") {
            let mut s = scene(110.0);
            let id = s.g.add_console(pos2(0.0, 0.0), &s.a);
            node_shot("console", 120, &mut s, id);
        }

        // --- Iris: every scene, listening to a drone and a rhythm --------
        for (k, title) in md::iris::SCENES.iter().enumerate() {
            let name = format!("iris-{}", title.to_lowercase());
            if !want(&name) {
                continue;
            }
            let mut s = scene(118.0);
            let id = add(&mut s, ModuleKind::Iris, 0.0, 0.0);
            let p = module_params(&s.g, id);
            p.set_mode(k);
            // A palette for each scene, to show them all (SNAP_IRIS_PALETTE
            // picks one for every scene): Board, Mono, Paper, Riso, Ember,
            // Ice, Acid, Lagoon.
            const LOOKS: [f32; 19] = [0.0, 0.0, 5.0, 3.0, 4.0, 6.0, 0.0, 2.0, 2.0, 3.0, 4.0, 1.0, 5.0, 0.0, 7.0, 0.0, 5.0, 6.0, 4.0];
            let look = std::env::var("SNAP_IRIS_PALETTE").ok().and_then(|v| v.parse().ok()).unwrap_or(LOOKS[k]);
            p.data[md::iris::data::PALETTE].set(look);
            let mix = s.g.add_mixer(pos2(0.0, 520.0), &s.a);
            let drone = add(&mut s, ModuleKind::Drone, -420.0, 420.0);
            let pops = add(&mut s, ModuleKind::Pops, -420.0, 760.0);
            link(&mut s, drone, 0, mix, 0);
            link(&mut s, pops, 0, mix, 1);
            node_shot(&name, FRAMES, &mut s, id);
        }

        // Iris as a visual synth: two layers, echoes, a mirror and the matrix.
        let synth = |p: &ModuleParams| {
            use md::iris::data::{self, layer, slot};
            let l = |k: usize, j: usize, v: f32| p.data[data::LAYER + k * data::LAYER_LEN + j].set(v);
            p.set_mode(5);
            l(0, layer::ECHO, 6.0);
            l(0, layer::FEEDBACK, 1.08);
            l(0, layer::MIRROR, 2.0);
            l(1, layer::ON, 1.0);
            l(1, layer::SCENE, 14.0);
            l(1, layer::ZOOM, 0.72);
            l(1, layer::HUE, 0.12);
            l(1, layer::OPACITY, 0.55);
            // LFO 1 swells layer 2, the bass fades it in, envelope 1 turns
            // layer 1's colours.
            for (s, (src, dst, amount)) in [(1.0, 12.0, 0.3), (7.0, 11.0, 0.4), (5.0, 9.0, 0.25)].into_iter().enumerate() {
                let b = data::MATRIX + s * data::SLOT_LEN;
                p.data[b + slot::SOURCE].set(src);
                p.data[b + slot::DEST].set(dst);
                p.data[b + slot::AMOUNT].set(amount);
            }
        };
        if want("iris-layers") {
            let mut s = scene(118.0);
            let id = add(&mut s, ModuleKind::Iris, 0.0, 0.0);
            synth(&module_params(&s.g, id));
            let mix = s.g.add_mixer(pos2(0.0, 520.0), &s.a);
            let drone = add(&mut s, ModuleKind::Drone, -420.0, 420.0);
            let pops = add(&mut s, ModuleKind::Pops, -420.0, 760.0);
            link(&mut s, drone, 0, mix, 0);
            link(&mut s, pops, 0, mix, 1);
            node_shot("iris-layers", FRAMES, &mut s, id);
        }
        for (tab, name) in [(1u8, "iris-ui-layers"), (2u8, "iris-ui-mod")] {
            if !want(name) {
                continue;
            }
            let mut s = scene(118.0);
            let id = add(&mut s, ModuleKind::Iris, 0.0, 0.0);
            synth(&module_params(&s.g, id));
            let f = feeder(&mut s, ModuleKind::Drone, 0.0);
            link(&mut s, f, 0, id, 0);
            crate::ui::iris::SHOW_CONTROLS.store(true, Ordering::Relaxed);
            crate::ui::iris::SHOW_TAB.store(tab, Ordering::Relaxed);
            full_shot(name, FRAMES, &mut s, vec2(2000.0, 1010.0), PPP);
            crate::ui::iris::SHOW_TAB.store(0, Ordering::Relaxed);
            crate::ui::iris::SHOW_CONTROLS.store(false, Ordering::Relaxed);
        }

        if want("iris-frameless") {
            // No frame: the Halo on the Board palette, right on the board.
            let mut s = scene(118.0);
            let id = add(&mut s, ModuleKind::Iris, 0.0, 0.0);
            let p = module_params(&s.g, id);
            p.set_mode(14);
            p.data[md::iris::data::FRAMELESS].set(1.0);
            let mix = s.g.add_mixer(pos2(0.0, 520.0), &s.a);
            let drone = add(&mut s, ModuleKind::Drone, -420.0, 420.0);
            let pops = add(&mut s, ModuleKind::Pops, -420.0, 760.0);
            link(&mut s, drone, 0, mix, 0);
            link(&mut s, pops, 0, mix, 1);
            node_shot("iris-frameless", FRAMES, &mut s, id);
        }

        if want("iris-ui") {
            // The scene's name, the gear and the grip, with the settings open.
            let mut s = scene(118.0);
            let id = add(&mut s, ModuleKind::Iris, 0.0, 0.0);
            module_params(&s.g, id).set_mode(1);
            let f = feeder(&mut s, ModuleKind::Drone, 0.0);
            link(&mut s, f, 0, id, 0);
            crate::ui::iris::SHOW_CONTROLS.store(true, Ordering::Relaxed);
            full_shot("iris-ui", FRAMES, &mut s, vec2(2000.0, 1160.0), PPP);
            crate::ui::iris::SHOW_CONTROLS.store(false, Ordering::Relaxed);
        }

        // --- Whole patches ------------------------------------------------
        if want("patch-demo") {
            let mut s = scene(90.0);
            crate::ui::app::demo_patch(&mut s.g, &s.a);
            full_shot("patch-demo", FRAMES, &mut s, vec2(1700.0, 760.0), PPP);
        }
        if want("patch-board") {
            // The demo patch on the Blueprint background, with built-in
            // stickers and two of the example SVGs around it.
            use crate::board::{Art, STICKER_MAX_PX};
            use crate::ui::board::EXAMPLES;
            let mut s = scene(90.0);
            crate::ui::app::demo_patch(&mut s.g, &s.a);
            let b = bounds(&s.g);
            eprintln!("patch-board: nodes span {:?}", b);
            let board = &mut s.g.board;
            board.background.preset = "blueprint".to_owned();
            let vinyl = board.import_image(EXAMPLES[0].1, STICKER_MAX_PX, false).unwrap();
            let cloud = board.import_image(EXAMPLES[2].1, STICKER_MAX_PX, false).unwrap();
            let builtin = |n: &str| Art::Builtin(n.to_owned());
            for (art, x, y, w, rot) in [
                (builtin("tag-ambient"), b.min.x + 90.0, b.min.y - 70.0, 200.0, -6.0),
                (Art::Image(cloud), b.center().x + 40.0, b.min.y - 90.0, 170.0, 0.0),
                (builtin("moon"), b.max.x - 40.0, b.min.y - 80.0, 120.0, 12.0),
                (Art::Image(vinyl), b.min.x - 40.0, b.max.y + 110.0, 180.0, -10.0),
                (builtin("sparkle"), b.min.x + 140.0, b.max.y + 80.0, 70.0, 0.0),
                (builtin("cassette"), b.max.x - 60.0, b.max.y + 60.0, 170.0, 8.0),
                (builtin("tag-drone"), b.center().x - 80.0, b.max.y + 80.0, 180.0, 4.0),
            ] {
                let id = board.add(art, [x, y], w);
                board.sticker_mut(id).unwrap().rotation = rot;
            }
            full_shot("patch-board", FRAMES, &mut s, vec2(2000.0, 1000.0), PPP);
        }
        if want("patch-hero") {
            let mut s = scene(100.0);
            let colloid = add(&mut s, ModuleKind::Colloid, 0.0, 0.0);
            let aether = add(&mut s, ModuleKind::Aether, 0.0, 0.0);
            let magma = add(&mut s, ModuleKind::Magma, 0.0, 0.0);
            let reverb = add(&mut s, ModuleKind::Reverb, 0.0, 0.0);
            let fjord = add(&mut s, ModuleKind::Fjord, 0.0, 0.0);
            let tape = add(&mut s, ModuleKind::Tape, 0.0, 0.0);
            let drone = add(&mut s, ModuleKind::Drone, 0.0, 0.0);
            let random = add(&mut s, ModuleKind::Random, 0.0, 0.0);
            let filter = add(&mut s, ModuleKind::Filter, 0.0, 0.0);
            let grains = add(&mut s, ModuleKind::Granular, 0.0, 0.0);
            let delay = add(&mut s, ModuleKind::Delay, 0.0, 0.0);
            let pops = add(&mut s, ModuleKind::Pops, 0.0, 0.0);
            let frak = add(&mut s, ModuleKind::Frakture, 0.0, 0.0);
            let eq = add(&mut s, ModuleKind::Equalizer, 0.0, 0.0);
            let console = s.g.add_console(pos2(0.0, 0.0), &s.a);
            link(&mut s, colloid, MIDI_PORT, aether, MIDI_PORT);
            link(&mut s, aether, 0, magma, 0);
            link(&mut s, magma, 0, reverb, 0);
            link(&mut s, reverb, 0, console, 0);
            link(&mut s, fjord, 0, tape, 0);
            link(&mut s, tape, 0, console, 1);
            link(&mut s, random, 0, filter, 1);
            link(&mut s, drone, 0, filter, 0);
            link(&mut s, filter, 0, grains, 0);
            link(&mut s, grains, 0, delay, 0);
            link(&mut s, delay, 0, console, 2);
            link(&mut s, pops, 0, frak, 0);
            link(&mut s, frak, 0, eq, 0);
            link(&mut s, eq, 0, console, 3);
            arrange(&mut s);
            full_shot("patch-hero", FRAMES, &mut s, vec2(2400.0, 1350.0), PPP);
        }

        // --- Plugin nodes (the SDK page) ---------------------------------
        // The example plugin, built as a dylib and loaded as the app does.
        if want("plugin-") {
            use crate::audio::node_plugin as np;
            let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let target = root.join("target/node-plugin-test");
            let ok = std::process::Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
                .args(["build", "-p", "audioboard-example-nodes", "--target-dir"])
                .arg(&target)
                .current_dir(&root)
                .status()
                .is_ok_and(|s| s.success());
            assert!(ok, "the example plugin builds");
            let scan = np::scan_dirs(&[target.join("debug")]);
            assert!(scan.refused.is_empty(), "{:?}", scan.refused);
            let kind = |id: &str| ModuleKind::Ext(np::find(id).expect("example node loaded"));
            let (haze, glass, pulse) =
                (kind("dev.audioboard.example.haze"), kind("dev.audioboard.example.glass"), kind("dev.audioboard.example.pulse"));
            // Meters and traces update as in the app (Haze traces its LFO).
            let meters = |g: &mut Graph, _: usize| {
                for node in &mut g.nodes {
                    node.update_meters(1.0 / 60.0);
                }
            };
            if want("plugin-nodes") {
                let mut s = scene(120.0);
                let osc = s.g.add_osc(pos2(-300.0, 0.0), &s.a);
                let h = add(&mut s, haze, 0.0, 0.0);
                let p = add(&mut s, pulse, -300.0, 320.0);
                let gl = add(&mut s, glass, 0.0, 320.0);
                let mix = s.g.add_mixer(pos2(300.0, 100.0), &s.a);
                link(&mut s, osc, 0, h, 0);
                link(&mut s, h, 0, mix, 0);
                link(&mut s, p, MIDI_PORT, gl, MIDI_PORT);
                link(&mut s, gl, 0, mix, 1);
                arrange(&mut s);
                let size = bounds(&s.g).size() + vec2(160.0, 160.0);
                let (img, w, h) = shoot(&mut s, size, PPP, FRAMES, meters);
                save("plugin-nodes", &img, w, (0, 0, w, h));
            }
            if want("plugin-glass") {
                let mut s = scene(120.0);
                let gl = add(&mut s, glass, 0.0, 0.0);
                let p = feeder(&mut s, pulse, 0.0);
                link(&mut s, p, MIDI_PORT, gl, MIDI_PORT);
                node_shot_with("plugin-glass", FRAMES, &mut s, gl, meters);
            }
        }
    }
}
