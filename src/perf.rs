//! Timing benchmarks, not run by default:
//! `cargo test --release perf -- --ignored --nocapture --test-threads=1`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::pos2;

use crate::audio::engine::Engine;
use crate::audio::modules::ModuleKind;
use crate::audio::params::{EngineShared, osc};
use crate::audio::{self, AudioHandle};
use crate::graph::{Graph, NodeKind, PortRef};

const SR: u32 = 48_000;

fn setup() -> (Graph, AudioHandle, Engine) {
    let (handle, engine) = audio::offline(Arc::new(EngineShared::new()), SR);
    (Graph::default(), handle, engine)
}

fn port(node: u32, port: usize) -> PortRef {
    PortRef { node, port }
}

/// Renders `seconds` of audio in device-sized buffers (512 frames) and
/// returns the share of real time it took (the best of five runs, which
/// filters out scheduling noise).
fn load(e: &mut Engine, seconds: f32) -> f64 {
    let frames = (SR as f32 * seconds / 5.0) as usize;
    let mut buf = vec![0.0f32; 512 * 2];
    // Warm up (buffers, caches, plugin start).
    for _ in 0..40 {
        e.render(&mut buf, 2);
    }
    (0..5)
        .map(|_| {
            let start = Instant::now();
            for _ in 0..frames / 512 {
                e.render(&mut buf, 2);
            }
            start.elapsed().as_secs_f64() / (seconds as f64 / 5.0)
        })
        .fold(f64::INFINITY, f64::min)
}

fn report(name: &str, share: f64) {
    eprintln!("{name:<28} {:>7.3}% of real time  ({:>6.2} µs per 64-frame block)", share * 100.0, share * 64.0 / SR as f64 * 1e6);
}

#[test]
#[ignore]
fn perf_audio_modules() {
    // Every module fed by a saw, into the mixer.
    for kind in ModuleKind::ALL {
        let (mut g, a, mut e) = setup();
        let src = g.add_osc(pos2(0.0, 0.0), &a);
        let m = g.add_module(kind, pos2(0.0, 0.0), &a);
        let mix = g.add_mixer(pos2(0.0, 0.0), &a);
        g.connect(port(src, 0), port(m, 0), &a);
        g.connect(port(m, 0), port(mix, 0), &a);
        report(kind.def().name, load(&mut e, 4.0));
    }
    let (mut g, a, mut e) = setup();
    let src = g.add_osc(pos2(0.0, 0.0), &a);
    let mix = g.add_mixer(pos2(0.0, 0.0), &a);
    g.connect(port(src, 0), port(mix, 0), &a);
    report("Oscillator + mixer", load(&mut e, 4.0));
    let (g, _a, mut e) = setup();
    drop(g);
    report("Empty engine", load(&mut e, 4.0));
}

/// A large patch: `chains` voices of osc → filter → delay → reverb, all
/// modulated by Random and summed in mixers.
fn big_patch(g: &mut Graph, a: &AudioHandle, chains: usize) {
    let mut mixers = Vec::new();
    for c in 0..chains {
        let y = c as f32 * 320.0;
        let lfo = g.add_module(ModuleKind::Random, pos2(0.0, y), a);
        let o = g.add_osc(pos2(300.0, y), a);
        let f = g.add_module(ModuleKind::Filter, pos2(600.0, y), a);
        let d = g.add_module(ModuleKind::Delay, pos2(900.0, y), a);
        let r = g.add_module(ModuleKind::Reverb, pos2(1200.0, y), a);
        if let NodeKind::Osc(p) = &g.node(o).unwrap().kind {
            p.params.set(osc::FREQ, 110.0 * (c + 1) as f32);
        }
        g.connect(port(lfo, 0), port(o, 0), a);
        g.connect(port(o, 0), port(f, 0), a);
        g.connect(port(lfo, 0), port(f, 1), a);
        g.connect(port(f, 0), port(d, 0), a);
        g.connect(port(d, 0), port(r, 0), a);
        if c % 4 == 0 {
            mixers.push(g.add_mixer(pos2(1600.0, y), a));
        }
        g.connect(port(r, 0), port(*mixers.last().unwrap(), c % 4), a);
    }
}

#[test]
#[ignore]
fn perf_audio_patches() {
    let (mut g, a, mut e) = setup();
    crate::ui::app::demo_patch(&mut g, &a);
    report("Demo patch", load(&mut e, 4.0));
    for chains in [4, 16] {
        let (mut g, a, mut e) = setup();
        big_patch(&mut g, &a, chains);
        report(&format!("{} nodes", g.nodes.len()), load(&mut e, 2.0));
    }
}

#[test]
#[ignore]
fn perf_snapshot() {
    let (mut g, a, _e) = setup();
    big_patch(&mut g, &a, 16);
    let start = Instant::now();
    for _ in 0..50 {
        std::hint::black_box(crate::project::snapshot(&g));
    }
    eprintln!("snapshot, {} nodes: {:?}", g.nodes.len(), start.elapsed() / 50);
    let path = std::path::Path::new("/Library/Audio/Plug-Ins/VST3/Originals - Cinematic Pads.vst3");
    if path.exists() {
        g.add_plugin(path, pos2(0.0, 0.0), &a).unwrap();
        let start = Instant::now();
        for _ in 0..10 {
            std::hint::black_box(crate::project::snapshot(&g));
        }
        eprintln!("snapshot with an instrument plugin: {:?}", start.elapsed() / 10);
    }
}

/// Mean CPU time of editor frames (UI + tessellation) and the vertices of
/// the last one. `zooming`: the view zooms every frame (as in auto-fit and
/// layout animations).
fn ui_frames(g: &mut Graph, a: &AudioHandle, zooming: bool) -> (Duration, Duration, usize) {
    let ctx = egui::Context::default();
    let mut editor = crate::ui::canvas::Editor::new();
    let mut run = Duration::ZERO;
    let mut tess = Duration::ZERO;
    let mut vertices = 0;
    let frames = 120;
    for f in 0..frames + 10 {
        let time = f as f64 / 60.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1500.0, 900.0))),
            time: Some(time),
            ..Default::default()
        };
        for node in &mut g.nodes {
            node.update_meters(1.0 / 60.0);
        }
        if zooming && f >= 10 {
            // Back and forth between 40% and 160%.
            let target = 1.0 + 0.6 * ((f - 10) as f32 * 0.05).sin();
            editor.zoom_by(target / editor.zoom, time);
        }
        let t = Instant::now();
        let mut out = ctx.run_ui(input, |ui| editor.show(ui, g, a));
        let t1 = Instant::now();
        let shapes = std::mem::take(&mut out.shapes);
        let meshes = ctx.tessellate(shapes, out.pixels_per_point);
        let t2 = Instant::now();
        out.textures_delta.clear();
        if f >= 10 {
            run += t1 - t;
            tess += t2 - t1;
            vertices = meshes
                .iter()
                .map(|m| match &m.primitive {
                    egui::epaint::Primitive::Mesh(mesh) => mesh.vertices.len(),
                    _ => 0,
                })
                .sum();
        }
    }
    (run / frames, tess / frames, vertices)
}

#[test]
#[ignore]
fn perf_ui() {
    for zooming in [false, true] {
        let what = if zooming { "zooming" } else { "still" };
        let (mut g, a, mut e) = setup();
        crate::ui::app::demo_patch(&mut g, &a);
        let mut buf = vec![0.0f32; 1024];
        e.render(&mut buf, 2);
        let (run, tess, vertices) = ui_frames(&mut g, &a, zooming);
        eprintln!("UI demo, {what:<8} build {run:>10.2?}  tessellate {tess:>10.2?}  {vertices:>6} vertices");
        for chains in [4, 16] {
            let (mut g, a, _e) = setup();
            big_patch(&mut g, &a, chains);
            let (run, tess, vertices) = ui_frames(&mut g, &a, zooming);
            let n = g.nodes.len();
            eprintln!("UI {n:>3} nodes, {what:<8} build {run:>10.2?}  tessellate {tess:>10.2?}  {vertices:>6} vertices");
        }
    }
}

/// One module for a while, to profile it: `PERF_MODULE=Drone`, and
/// `PERF_SECS` of audio (default 20).
#[test]
#[ignore]
fn perf_one_module() {
    let name = std::env::var("PERF_MODULE").unwrap_or_else(|_| "Drone".into());
    let kind = ModuleKind::ALL.into_iter().find(|k| k.def().name == name).expect("module name");
    let (mut g, a, mut e) = setup();
    let src = g.add_osc(pos2(0.0, 0.0), &a);
    let m = g.add_module(kind, pos2(0.0, 0.0), &a);
    let mix = g.add_mixer(pos2(0.0, 0.0), &a);
    g.connect(port(src, 0), port(m, 0), &a);
    g.connect(port(m, 0), port(mix, 0), &a);
    let secs = std::env::var("PERF_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(20.0);
    report(kind.def().name, load(&mut e, secs));
}

#[test]
#[ignore]
fn perf_fastmath() {
    use crate::audio::fastmath;
    let n = 50_000_000;
    let time = |name: &str, f: &dyn Fn(f32) -> f32| {
        let start = Instant::now();
        let mut acc = 0.0f32;
        let mut x = 0.0f32;
        for _ in 0..n {
            acc += f(x);
            x += 0.000_013;
            if x > 1.0 {
                x -= 1.0;
            }
        }
        std::hint::black_box(acc);
        eprintln!("{name:<12} {:.2} ns", start.elapsed().as_secs_f64() * 1e9 / n as f64);
    };
    time("empty", &|x| x);
    time("sin_turns", &|x| fastmath::sin_turns(x));
    time("std sin", &|x| (x * std::f32::consts::TAU).sin());
    time("exp2", &|x| fastmath::exp2(x));
    time("std exp2", &|x| x.exp2());
    time("tanh", &|x| fastmath::tanh(x * 3.0));
    time("std tanh", &|x| (x * 3.0).tanh());
    time("poly_blep", &|x| crate::audio::dsp::poly_blep(x, 0.01));
}

/// Which kinds of shapes the vertices come from (demo patch, one frame).
#[test]
#[ignore]
fn perf_ui_vertices() {
    use egui::epaint::{ClippedShape, Primitive, Shape};
    let (mut g, a, _e) = setup();
    crate::ui::app::demo_patch(&mut g, &a);
    let ctx = egui::Context::default();
    let mut editor = crate::ui::canvas::Editor::new();
    let mut shapes = Vec::new();
    for f in 0..3 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1500.0, 900.0))),
            time: Some(f as f64 / 60.0),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| editor.show(ui, &mut g, &a));
        out.textures_delta.clear();
        shapes = out.shapes;
    }
    fn kind(s: &Shape) -> &'static str {
        match s {
            Shape::Text(_) => "text",
            Shape::Path(_) => "path",
            Shape::LineSegment { .. } => "line",
            Shape::Circle(_) => "circle",
            Shape::Rect(r) if r.blur_width > 0.0 => "shadow",
            Shape::Rect(_) => "rect",
            Shape::CubicBezier(_) => "bezier",
            Shape::Mesh(_) => "mesh",
            Shape::Vec(_) => "vec",
            _ => "other",
        }
    }
    let mut totals: std::collections::BTreeMap<&str, (usize, usize)> = Default::default();
    let mut flat = Vec::new();
    for ClippedShape { clip_rect, shape, transform_after_tessellation } in shapes {
        let wrap = |s| ClippedShape { clip_rect, shape: s, transform_after_tessellation };
        match shape {
            Shape::Vec(v) => flat.extend(v.into_iter().map(wrap)),
            s => flat.push(wrap(s)),
        }
    }
    for cs in flat {
        let k = kind(&cs.shape);
        let verts: usize = ctx
            .tessellate(vec![cs], 2.0)
            .iter()
            .map(|m| match &m.primitive {
                Primitive::Mesh(mesh) => mesh.vertices.len(),
                _ => 0,
            })
            .sum();
        let e = totals.entry(k).or_default();
        e.0 += 1;
        e.1 += verts;
    }
    for (k, (n, v)) in totals {
        eprintln!("{k:<8} {n:>5} shapes {v:>7} vertices (at 2x)");
    }
}

#[test]
#[ignore]
fn perf_layout() {
    for chains in [4, 16] {
        let (mut g, a, _e) = setup();
        big_patch(&mut g, &a, chains);
        let start = Instant::now();
        for _ in 0..10 {
            std::hint::black_box(crate::ui::layout::plan(&g, None));
        }
        eprintln!("layout {} nodes: {:?}", g.nodes.len(), start.elapsed() / 10);
    }
}

/// Cost of real plugins per second of audio, processed in blocks of 64 vs 512.
#[test]
#[ignore]
fn perf_plugins_block_size() {
    let dir = std::path::Path::new("/Library/Audio/Plug-Ins/VST3");
    for name in ["Vital", "ValhallaSupermassive", "ValhallaFreqEcho", "Originals - Cinematic Pads"] {
        let path = dir.join(format!("{name}.vst3"));
        if !path.exists() {
            continue;
        }
        for block in [64usize, 512] {
            let mut host = vst3_host::Vst3Host::builder()
                .sample_rate(48_000.0)
                .block_size(block)
                .with_process_isolation(false)
                .build()
                .unwrap();
            let mut l = crate::audio::plugin::load(&mut host, &path).unwrap();
            let mut b = vst3_host::AudioBuffers::new(l.meta.inputs, l.meta.outputs, block, 48_000.0);
            for ch in b.inputs.iter_mut() {
                for (i, s) in ch.iter_mut().enumerate() {
                    *s = ((i as f32) * 0.05).sin() * 0.3;
                }
            }
            let chan = vst3_host::MidiChannel::from_index(0).unwrap();
            let _ = l.plugin.send_midi_event(vst3_host::MidiEvent::NoteOn { channel: chan, note: 60, velocity: 100 });
            let blocks = 48_000 * 4 / block;
            for _ in 0..blocks / 4 {
                let _ = l.plugin.process_audio(&mut b);
            }
            let start = Instant::now();
            for _ in 0..blocks {
                let _ = l.plugin.process_audio(&mut b);
            }
            let share = start.elapsed().as_secs_f64() / 4.0;
            eprintln!("{name:<28} block {block:>4}: {:>6.2}% of real time", share * 100.0);
        }
    }
}

/// The big patch playing, then stopped: once the tails are gone, effects
/// and sources rest.
#[test]
#[ignore]
fn perf_stopped_patch() {
    let shared = Arc::new(EngineShared::new());
    let (a, mut e) = audio::offline(shared.clone(), SR);
    let mut g = Graph::default();
    big_patch(&mut g, &a, 16);
    report("84 nodes, playing", load(&mut e, 2.0));
    shared.running.store(false, std::sync::atomic::Ordering::Relaxed);
    let mut buf = vec![0.0f32; 512 * 2];
    for _ in 0..(SR as usize * 20 / 512) {
        e.render(&mut buf, 2);
    }
    report("84 nodes, stopped (tails gone)", load(&mut e, 2.0));
}

/// One plugin's cost per process call: mean, 99th percentile and worst,
/// idle and playing notes. `PERF_PLUGIN=Triaz Player`.
#[test]
#[ignore]
fn perf_plugin_calls() {
    let name = std::env::var("PERF_PLUGIN").unwrap_or_else(|_| "Triaz Player".into());
    let path = std::path::Path::new("/Library/Audio/Plug-Ins/VST3").join(format!("{name}.vst3"));
    for block in [64usize, 512] {
        let mut host = vst3_host::Vst3Host::builder()
            .sample_rate(48_000.0)
            .block_size(block)
            .with_process_isolation(false)
            .build()
            .unwrap();
        let mut l = crate::audio::plugin::load(&mut host, &path).unwrap();
        eprintln!("{name}: inputs {} outputs {} midi {}", l.meta.inputs, l.meta.outputs, l.meta.midi);
        let mut b = vst3_host::AudioBuffers::new(l.meta.inputs, l.meta.outputs, block, 48_000.0);
        let period = block as f64 / 48_000.0;
        let chan = vst3_host::MidiChannel::from_index(9).unwrap();
        for phase in ["idle", "notes"] {
            let calls = 48_000 * 6 / block;
            let mut times = Vec::with_capacity(calls);
            for k in 0..calls {
                if phase == "notes" && k % (48_000 / 8 / block).max(1) == 0 {
                    for note in [36u8, 38, 42] {
                        let _ = l.plugin.send_midi_event(vst3_host::MidiEvent::NoteOn { channel: chan, note, velocity: 110 });
                    }
                }
                let t = Instant::now();
                let _ = l.plugin.process_audio(&mut b);
                times.push(t.elapsed().as_secs_f64());
                // Real time, as a device would call it.
                let left = period - times[k];
                if left > 0.0 {
                    std::thread::sleep(Duration::from_secs_f64(left));
                }
            }
            let peak = b.outputs.iter().flatten().fold(0.0f32, |m, s| m.max(s.abs()));
            times.sort_by(f64::total_cmp);
            let mean = times.iter().sum::<f64>() / times.len() as f64;
            let p99 = times[times.len() * 99 / 100];
            let worst = times[times.len() - 1];
            eprintln!(
                "  block {block:>4} {phase:<5}: mean {:>5.1}% p99 {:>6.1}% worst {:>7.1}% of the block's time (out peak {peak:.3})",
                mean / period * 100.0,
                p99 / period * 100.0,
                worst / period * 100.0
            );
        }
    }
}

/// The Equalizer with everything working: six dynamic bands, HP and LP.
#[test]
#[ignore]
fn perf_equalizer_busy() {
    use crate::audio::modules::equalizer::{self, BANDS, band, bp, data};
    let (mut g, a, mut e) = setup();
    let src = g.add_osc(pos2(0.0, 0.0), &a);
    let m = g.add_module(ModuleKind::Equalizer, pos2(0.0, 0.0), &a);
    let mix = g.add_mixer(pos2(0.0, 0.0), &a);
    g.connect(port(src, 0), port(m, 0), &a);
    g.connect(port(m, 0), port(mix, 0), &a);
    if let NodeKind::Module(node) = &g.node(m).unwrap().kind {
        let p = &node.params;
        for b in 0..BANDS {
            p.set(bp(b, band::GAIN), 3.0);
            p.set(bp(b, band::RANGE), -6.0);
            p.set(bp(b, band::THRESHOLD), -30.0);
        }
        p.data[data::BAND_MODE + 2].set(equalizer::MID as f32);
        p.data[data::HP_ON].set(1.0);
        p.data[data::LP_ON].set(1.0);
        p.data[data::HP_SLOPE].set(3.0);
    }
    report("Equalizer, all bands dynamic", load(&mut e, 4.0));
}

/// Magma with everything working: four filters (48 dB, styles), two
/// XLFOs on their frequencies, then the same in High Quality.
#[test]
#[ignore]
fn perf_magma_busy() {
    use crate::audio::modules::magma::{self, FILTERS, data, filt, fp, modulation};
    for hq in [false, true] {
        let (mut g, a, mut e) = setup();
        let src = g.add_osc(pos2(0.0, 0.0), &a);
        let m = g.add_module(ModuleKind::Magma, pos2(0.0, 0.0), &a);
        let mix = g.add_mixer(pos2(0.0, 0.0), &a);
        g.connect(port(src, 0), port(m, 0), &a);
        g.connect(port(m, 0), port(mix, 0), &a);
        if let NodeKind::Module(node) = &g.node(m).unwrap().kind {
            let p = &node.params;
            for f in 0..FILTERS {
                p.data[data::FILTER_ON + f].set(1.0);
                p.data[data::SLOPE + f].set(3.0);
                p.data[data::STYLE + f].set(f as f32);
                magma::add_slot(p, modulation::SRC_XLFO + f % 2, fp(f, filt::FREQ));
            }
            p.data[data::ROUTING].set(3.0);
            p.data[data::HQ].set(if hq { 1.0 } else { 0.0 });
        }
        report(if hq { "Magma, busy, High Quality" } else { "Magma, busy" }, load(&mut e, 4.0));
    }
}

/// Pops 66 at its busiest: all fifteen instruments on every step at 240
/// BPM, every waveform reshaped (WAVE, BEND, DRIVE all the way up).
#[test]
#[ignore]
fn perf_pops_busy() {
    use crate::audio::modules::pops::rhythms::{MAX_LEN, Rhythm};
    use crate::audio::modules::pops::voices::COUNT;
    use crate::audio::modules::pops::{self, TEMPO, data, ip, knob};
    for shaped in [false, true] {
        let (mut g, a, mut e) = setup();
        let m = g.add_module(ModuleKind::Pops, pos2(0.0, 0.0), &a);
        let mix = g.add_mixer(pos2(0.0, 0.0), &a);
        g.connect(port(m, 0), port(mix, 0), &a);
        if let NodeKind::Module(node) = &g.node(m).unwrap().kind {
            let p = &node.params;
            let all = Rhythm { name: "ALL", per_beat: 4, len: 16, rows: [(1u32 << MAX_LEN.min(16)) - 1; COUNT] };
            pops::set_user(p, 0, &all);
            p.data[data::SOURCE].set(1.0);
            p.data[data::PATTERN].set(0.0);
            p.data[data::SYNC].set(0.0);
            p.data[data::RUNNING].set(1.0);
            p.set(TEMPO, 240.0);
            if shaped {
                for i in 0..COUNT {
                    p.set(ip(i, knob::WAVE), 0.8);
                    p.set(ip(i, knob::BEND), 0.5);
                    p.set(ip(i, knob::DRIVE), 0.5);
                }
            }
        }
        report(if shaped { "Pops 66, busy, reshaped" } else { "Pops 66, busy" }, load(&mut e, 4.0));
    }
}
