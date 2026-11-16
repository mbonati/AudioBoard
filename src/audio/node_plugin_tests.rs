//! Node plugins end to end: nodes written with the SDK go through the real
//! C ABI (registered in process with `register_static`), become modules,
//! sound in a patch, survive a project round trip; broken plugins are
//! refused with a reason. The `#[ignore]`d test builds the example plugin
//! crate as a dynamic library and loads it from a folder, as the app does.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

use audioboard_node::abi::PluginInfo;
use audioboard_node::prelude::*;
use egui::pos2;

use super::*;
use crate::audio::engine::Engine;
use crate::audio::modules::ModuleKind;
use crate::audio::params::EngineShared;
use crate::audio::{self, AudioHandle};
use crate::graph::{Graph, MIDI_PORT, NodeKind, PortRef};
use crate::project;

// ---------------------------------------------------------------------------
// Test nodes, written against the SDK
// ---------------------------------------------------------------------------

const TONE_PARAMS: &[ParamSpec] =
    &[ParamSpec::log("FREQ", 20.0, 2000.0, 330.0, Unit::Hz), ParamSpec::lin("LEVEL", 0.0, 1.0, 0.5, Unit::Percent)];

/// A sine; its display traces its phase.
struct TestTone {
    params: Shared,
    sample_rate: f32,
    phase: f32,
}

impl Node for TestTone {
    const DESCRIPTOR: Descriptor = Descriptor {
        id: "test.audioboard.tone",
        maker: "Test",
        def: NodeDef {
            name: "Test Tone",
            prefix: "TONE",
            hint: "sine",
            outputs: &["OUT"],
            params: TONE_PARAMS,
            modes: &["A", "B"],
            display_h: 30.0,
            viz: 1,
            ..NodeDef::EMPTY
        },
        panel: Panel { display: Display::Trace { index: 0 }, readout: Readout::Param(0) },
    };

    fn new(init: &Init) -> Self {
        TestTone { params: init.params, sample_rate: init.sample_rate, phase: 0.0 }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io<'_>) {
        let n = io.frames();
        let (f, level) = (self.params.get(0), self.params.get(1));
        let out = io.output(0);
        for i in 0..n {
            let s = fastmath::sin_turns(self.phase) * level;
            self.phase = (self.phase + f / self.sample_rate).fract();
            out[0][i] = s;
            out[1][i] = s;
        }
        self.params.set_viz(0, fastmath::sin_turns(self.phase));
    }
}

const GAIN_PARAMS: &[ParamSpec] = &[ParamSpec::lin("GAIN", 0.0, 2.0, 1.0, Unit::Percent)];

/// Input times GAIN (negated with INVERT); keeps two extra saved values.
struct TestGain {
    params: Shared,
}

impl Node for TestGain {
    const DESCRIPTOR: Descriptor = Descriptor {
        id: "test.audioboard.gain",
        maker: "Test",
        def: NodeDef {
            name: "Test Gain",
            prefix: "GAIN",
            hint: "gain",
            inputs: &["IN"],
            outputs: &["OUT"],
            params: GAIN_PARAMS,
            toggle: Some("INVERT"),
            data: &[0.25, 0.5],
            ..NodeDef::EMPTY
        },
        panel: Panel { display: Display::Level, readout: Readout::Param(0) },
    };

    fn new(init: &Init) -> Self {
        TestGain { params: init.params }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io<'_>) {
        let n = io.frames();
        let g = self.params.get(0) * if self.params.toggled() { -1.0 } else { 1.0 };
        let (input, out) = io.through(0, 0);
        for c in 0..2 {
            for i in 0..n {
                out[c][i] = input.map_or(0.0, |b| b[c][i]) * g;
            }
        }
    }
}

/// Panics on its first block.
struct TestPanic;

impl Node for TestPanic {
    const DESCRIPTOR: Descriptor = Descriptor {
        id: "test.audioboard.panic",
        maker: "Test",
        def: NodeDef { name: "Test Panic", outputs: &["OUT"], ..NodeDef::EMPTY },
        panel: Panel { display: Display::None, readout: Readout::Name },
    };

    fn new(_init: &Init) -> Self {
        TestPanic
    }

    fn process(&mut self, _ctx: &ProcessCtx, _io: &mut Io<'_>) {
        panic!("test node panics on purpose");
    }
}

audioboard_node::static_nodes! {
    fn test_entry, name: "Test nodes", maker: "Test", version: "1.0", nodes: [TestTone, TestGain, TestPanic]
}

/// The test nodes, registered once for every test in the process.
fn test_nodes() -> [ExtId; 3] {
    static IDS: OnceLock<Vec<ExtId>> = OnceLock::new();
    let ids = IDS.get_or_init(|| register_static(test_entry, "test-nodes").expect("test nodes register"));
    [ids[0], ids[1], ids[2]]
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup() -> (Graph, AudioHandle, Engine) {
    let (handle, engine) = audio::offline(Arc::new(EngineShared::new()), 48_000);
    (Graph::default(), handle, engine)
}

fn port(node: u32, port: usize) -> PortRef {
    PortRef { node, port }
}

fn params(g: &Graph, id: u32) -> Arc<crate::audio::modules::ModuleParams> {
    match &g.node(id).expect("node").kind {
        NodeKind::Module(m) => m.params.clone(),
        _ => panic!("not a module"),
    }
}

/// Renders `frames` stereo frames; returns (peak, all finite).
fn render(engine: &mut Engine, frames: usize) -> (f32, bool) {
    let mut buf = vec![0.0f32; frames * 2];
    for chunk in buf.chunks_mut(2 * 256) {
        engine.render(chunk, 2);
    }
    (buf.iter().fold(0.0f32, |m, s| m.max(s.abs())), buf.iter().all(|s| s.is_finite()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn sdk_nodes_load_through_the_abi() {
    let [tone, gain, panic] = test_nodes();
    let t = ext(tone);
    assert_eq!(t.id, "test.audioboard.tone");
    assert_eq!(t.def.params[0].name, "FREQ");
    assert_eq!(t.def.params[0].taper, Taper::Log);
    assert_eq!(t.def.modes, ["A", "B"]);
    assert_eq!(t.panel.display, Display::Trace { index: 0 });
    assert_eq!(ext(gain).def.toggle, Some("INVERT"));
    assert_eq!(ext(gain).def.data, [0.25, 0.5]);
    assert_eq!(ext(panic).def.prefix, "TEST PANIC", "an empty prefix is made from the name");
    assert_eq!(find("test.audioboard.gain"), Some(gain));
    assert_eq!(ModuleKind::from_key("test.audioboard.gain"), Some(ModuleKind::Ext(gain)));
    assert_eq!(ModuleKind::Ext(gain).key(), "test.audioboard.gain");
    assert!(by_maker().iter().any(|(m, list)| *m == "Test" && list.contains(&tone)));
    // Registering again changes nothing.
    assert_eq!(register_static(test_entry, "test-nodes").unwrap(), vec![tone, gain, panic]);
}

#[test]
fn plugin_nodes_sound_in_a_patch_and_survive_a_project() {
    let [tone, gain, _] = test_nodes();
    let (mut g, a, mut e) = setup();
    let t = g.add_module(ModuleKind::Ext(tone), pos2(0.0, 0.0), &a);
    let m = g.add_module(ModuleKind::Ext(gain), pos2(300.0, 0.0), &a);
    let mix = g.add_mixer(pos2(600.0, 0.0), &a);
    assert!(g.connect(port(t, 0), port(m, 0), &a));
    assert!(g.connect(port(m, 0), port(mix, 0), &a));
    let (peak, finite) = render(&mut e, 9600);
    assert!(finite && peak > 0.2, "tone through gain reaches the output: {peak}");

    // Settings: knob, mode, switch and saved data.
    params(&g, t).set(0, 440.0);
    params(&g, t).set_mode(1);
    params(&g, m).set(0, 0.0);
    params(&g, m).set_toggle(true);
    params(&g, m).data[1].set(0.75);
    // (The output limiter's lookahead still holds the last audio: settle.)
    render(&mut e, 4800);
    let (peak, _) = render(&mut e, 9600);
    assert!(peak < 0.05, "GAIN at 0 silences it: {peak}");
    assert!(params(&g, t).viz[0].get().abs() <= 1.0);

    let snap = project::snapshot(&g);
    let json = serde_json::to_string(&snap).unwrap();
    assert!(json.contains("\"module\":\"test.audioboard.tone\""), "saved by stable id: {json}");
    assert!(json.contains("\"FREQ\":440.0"), "parameters by name");

    let (mut g2, a2, mut e2) = setup();
    let back: project::Project = serde_json::from_str(&json).unwrap();
    assert!(project::missing_modules(&back).is_empty());
    assert_eq!(project::restore(&mut g2, &back, &a2), 0);
    let ids: Vec<u32> = g2.nodes.iter().map(|n| n.id).collect();
    let find_kind = |k: ModuleKind| *ids.iter().find(|id| params_kind(&g2, **id) == Some(k)).expect("restored node");
    let (t2, m2) = (find_kind(ModuleKind::Ext(tone)), find_kind(ModuleKind::Ext(gain)));
    assert_eq!(params(&g2, t2).get(0), 440.0);
    assert_eq!(params(&g2, t2).mode(), 1);
    assert!(params(&g2, m2).toggled());
    assert_eq!(params(&g2, m2).data[1].get(), 0.75);
    assert_eq!(g2.cables.len(), 2);
    params(&g2, m2).set(0, 1.0);
    let (peak, finite) = render(&mut e2, 9600);
    assert!(finite && peak > 0.2, "the restored patch plays: {peak}");
}

fn params_kind(g: &Graph, id: u32) -> Option<ModuleKind> {
    match &g.node(id)?.kind {
        NodeKind::Module(m) => Some(m.params.kind),
        _ => None,
    }
}

#[test]
fn a_missing_plugin_is_reported_not_fatal() {
    let [tone, ..] = test_nodes();
    let (mut g, a, _e) = setup();
    g.add_module(ModuleKind::Ext(tone), pos2(0.0, 0.0), &a);
    let json = serde_json::to_string(&project::snapshot(&g)).unwrap().replace("test.audioboard.tone", "com.nobody.missing");
    let p: project::Project = serde_json::from_str(&json).unwrap();
    assert_eq!(project::missing_modules(&p), vec!["com.nobody.missing".to_owned()]);
    let (mut g2, a2, _e2) = setup();
    assert_eq!(project::restore(&mut g2, &p, &a2), 1, "the node is skipped and counted");
    assert!(g2.nodes.is_empty());
}

#[test]
fn a_panicking_plugin_node_goes_silent() {
    let [tone, _, panic] = test_nodes();
    let (mut g, a, mut e) = setup();
    let bad = g.add_module(ModuleKind::Ext(panic), pos2(0.0, 0.0), &a);
    let good = g.add_module(ModuleKind::Ext(tone), pos2(0.0, 200.0), &a);
    let mix = g.add_mixer(pos2(400.0, 0.0), &a);
    g.connect(port(bad, 0), port(mix, 0), &a);
    g.connect(port(good, 0), port(mix, 1), &a);
    let (peak, finite) = render(&mut e, 4800);
    assert!(finite && peak > 0.1, "the rest of the patch plays on: {peak}");
    assert_eq!(ext(panic).trouble(), Some("Stopped: the plugin crashed"));
    assert_eq!(ext(tone).trouble(), None);
}

#[test]
fn incompatible_plugins_are_refused_with_a_reason() {
    // Another ABI version: only the first field is read.
    static OTHER_ABI: u32 = 99;
    unsafe extern "C" fn other_abi(_host: u32) -> *const PluginInfo {
        &OTHER_ABI as *const u32 as *const PluginInfo
    }
    let e = register_static(other_abi, "other-abi").unwrap_err();
    assert!(e.contains("ABI v99") && e.contains("rebuild"), "{e}");

    unsafe extern "C" fn nothing(_host: u32) -> *const PluginInfo {
        std::ptr::null()
    }
    assert!(register_static(nothing, "nothing").is_err());

    // A broken description.
    const BAD: &[ParamSpec] = &[ParamSpec::log("FREQ", 0.0, 100.0, 10.0, Unit::Hz)];
    struct Broken;
    impl Node for Broken {
        const DESCRIPTOR: Descriptor = Descriptor {
            id: "test.audioboard.broken",
            maker: "Test",
            def: NodeDef { name: "Broken", outputs: &["OUT"], params: BAD, ..NodeDef::EMPTY },
            panel: Panel { display: Display::None, readout: Readout::Name },
        };
        fn new(_: &Init) -> Self {
            Broken
        }
        fn process(&mut self, _: &ProcessCtx, _: &mut Io<'_>) {}
    }
    audioboard_node::static_nodes! { fn broken_entry, name: "Broken", maker: "Test", version: "1", nodes: [Broken] }
    let e = register_static(broken_entry, "broken").unwrap_err();
    assert!(e.contains("log taper"), "{e}");
    assert_eq!(find("test.audioboard.broken"), None);

    // An id that is already taken.
    test_nodes();
    audioboard_node::static_nodes! { fn twin_entry, name: "Twin", maker: "Test", version: "1", nodes: [TestTone] }
    let e = register_static(twin_entry, "twin").unwrap_err();
    assert!(e.contains("already taken"), "{e}");
}

#[test]
fn scanning_skips_files_that_are_not_plugins() {
    let dir = std::env::temp_dir().join(format!("audioboard-nodes-junk-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let junk = dir.join(format!("junk.{}", std::env::consts::DLL_EXTENSION));
    std::fs::write(&junk, b"not a library").unwrap();
    let scan = scan_dirs(std::slice::from_ref(&dir));
    assert!(scan.loaded.is_empty());
    assert_eq!(scan.refused.len(), 1, "{:?}", scan.refused);
    assert!(scan.summary().1, "a refusal is reported as a problem");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Builds `plugins/example-nodes` as a dynamic library (with cargo, into a
/// target folder of its own), copies it into a fresh Nodes folder, scans
/// it, and plays its nodes. Run with `cargo test example_plugin -- --ignored`.
#[test]
#[ignore = "builds the example plugin with cargo"]
fn example_plugin_builds_loads_and_plays() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target = root.join("target/node-plugin-test");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = std::process::Command::new(cargo)
        .args(["build", "-p", "audioboard-example-nodes", "--target-dir"])
        .arg(&target)
        .current_dir(&root)
        .status()
        .expect("cargo runs");
    assert!(status.success(), "the example plugin builds");
    let file = format!("{}audioboard_example_nodes.{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_EXTENSION);
    let built = target.join("debug").join(&file);
    assert!(built.exists(), "{} exists", built.display());

    let nodes_dir = std::env::temp_dir().join(format!("audioboard-nodes-{}", std::process::id()));
    std::fs::create_dir_all(&nodes_dir).unwrap();
    std::fs::copy(&built, nodes_dir.join(&file)).unwrap();
    let scan = scan_dirs(std::slice::from_ref(&nodes_dir));
    assert_eq!(scan.refused, Vec::<String>::new());
    assert_eq!(scan.loaded, vec![("Example nodes".to_owned(), 3)]);
    // Scanning again loads nothing new.
    assert!(scan_dirs(std::slice::from_ref(&nodes_dir)).loaded.is_empty());

    let haze = find("dev.audioboard.example.haze").expect("Haze loaded");
    let glass = find("dev.audioboard.example.glass").expect("Glass loaded");
    let pulse = find("dev.audioboard.example.pulse").expect("Pulse loaded");
    assert!(by_maker().iter().any(|(m, l)| *m == "AudioBoard Examples" && l.len() == 3));
    assert!(libraries().iter().any(|l| l.0 == file && l.3 == 3));

    // An oscillator through Haze, and Pulse playing Glass, into a mixer.
    let (mut g, a, mut e) = setup();
    let osc = g.add_osc(pos2(0.0, 0.0), &a);
    let h = g.add_module(ModuleKind::Ext(haze), pos2(300.0, 0.0), &a);
    let p = g.add_module(ModuleKind::Ext(pulse), pos2(0.0, 300.0), &a);
    let gl = g.add_module(ModuleKind::Ext(glass), pos2(300.0, 300.0), &a);
    let mix = g.add_mixer(pos2(700.0, 0.0), &a);
    assert!(g.connect(port(osc, 0), port(h, 0), &a));
    assert!(g.connect(port(h, 0), port(mix, 0), &a));
    assert!(g.connect(port(p, MIDI_PORT), port(gl, MIDI_PORT), &a), "Pulse's MIDI into Glass");
    assert!(g.connect(port(gl, 0), port(mix, 1), &a));
    let (peak, finite) = render(&mut e, 48_000);
    assert!(finite && peak > 0.1, "the patch plays: {peak}");
    assert!(params(&g, gl).viz[32].get() == 36.0, "Glass heard Pulse's C2");
    assert!(params(&g, h).viz[0].get().abs() <= 1.0, "Haze publishes its LFO");

    // Glass alone, played by Pulse.
    let cable = *g.cables.iter().find(|c| c.from.node == h).unwrap();
    g.disconnect(cable, &a);
    let (peak, _) = render(&mut e, 48_000);
    assert!(peak > 0.05, "Glass sounds from Pulse's notes: {peak}");

    // Saved and restored by id, settings included.
    params(&g, h).set(0, 2.5);
    params(&g, p).set_mode(1);
    let json = serde_json::to_string(&project::snapshot(&g)).unwrap();
    assert!(json.contains("dev.audioboard.example.haze"));
    let back: project::Project = serde_json::from_str(&json).unwrap();
    let (mut g2, a2, mut e2) = setup();
    assert_eq!(project::restore(&mut g2, &back, &a2), 0);
    let restored = |k: ModuleKind| g2.nodes.iter().map(|n| n.id).find(|id| params_kind(&g2, *id) == Some(k)).unwrap();
    assert_eq!(params(&g2, restored(ModuleKind::Ext(haze))).get(0), 2.5);
    assert_eq!(params(&g2, restored(ModuleKind::Ext(pulse))).mode(), 1);
    a2.shared.running.store(true, Ordering::Relaxed);
    let (peak, finite) = render(&mut e2, 48_000);
    assert!(finite && peak > 0.05, "the restored patch plays: {peak}");
    let _ = std::fs::remove_dir_all(&nodes_dir);
}
