use std::sync::Arc;
use std::sync::atomic::Ordering;

use egui::pos2;

use crate::audio::engine::Engine;
use crate::audio::modules::{ModuleKind, ModuleParams};
use crate::audio::params::{EngineShared, MixerParams, OscParams, Waveform, osc};
use crate::audio::{self, AudioHandle};
use crate::graph::{Graph, NodeKind, PortRef};

fn setup() -> (Graph, AudioHandle, Engine) {
    let (handle, engine) = audio::offline(Arc::new(EngineShared::new()), 48_000);
    (Graph::default(), handle, engine)
}

fn port(node: u32, port: usize) -> PortRef {
    PortRef { node, port }
}

fn osc_params(g: &Graph, id: u32) -> Arc<OscParams> {
    match &g.node(id).unwrap().kind {
        NodeKind::Osc(o) => o.params.clone(),
        _ => panic!("not an oscillator"),
    }
}

fn mixer_params(g: &Graph, id: u32) -> Arc<MixerParams> {
    match &g.node(id).unwrap().kind {
        NodeKind::Mixer(m) => m.params.clone(),
        _ => panic!("not a mixer"),
    }
}

/// Renders `frames` stereo frames and returns (peak, all samples finite).
fn render(engine: &mut Engine, frames: usize) -> (f32, bool) {
    let mut buf = vec![0.0f32; frames * 2];
    // Odd-sized chunks exercise the block splitting.
    for chunk in buf.chunks_mut(2 * 300) {
        engine.render(chunk, 2);
    }
    let peak = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    (peak, buf.iter().all(|s| s.is_finite()))
}

#[test]
fn rejects_feedback_loops() {
    let (mut g, a, _e) = setup();
    let x = g.add_osc(pos2(0.0, 0.0), &a);
    let y = g.add_osc(pos2(0.0, 0.0), &a);
    assert!(g.connect(port(x, 0), port(y, 0), &a));
    assert!(!g.connect(port(y, 0), port(x, 0), &a), "y -> x would close a loop");
    assert!(!g.connect(port(x, 0), port(x, 1), &a), "self-patching is a loop");
    assert_eq!(g.cables.len(), 1);
}

#[test]
fn an_input_holds_one_cable() {
    let (mut g, a, _e) = setup();
    let x = g.add_osc(pos2(0.0, 0.0), &a);
    let y = g.add_osc(pos2(0.0, 0.0), &a);
    let m = g.add_mixer(pos2(0.0, 0.0), &a);
    g.connect(port(x, 0), port(m, 0), &a);
    g.connect(port(y, 0), port(m, 0), &a);
    assert_eq!(g.cables.len(), 1);
    assert_eq!(g.cables[0].from.node, y);
}

#[test]
fn removing_a_node_drops_its_cables() {
    let (mut g, a, mut e) = setup();
    let x = g.add_osc(pos2(0.0, 0.0), &a);
    let m = g.add_mixer(pos2(0.0, 0.0), &a);
    g.connect(port(x, 0), port(m, 0), &a);
    g.remove_node(x, &a);
    assert!(g.cables.is_empty());
    let (peak, finite) = render(&mut e, 4800);
    assert!(finite);
    assert_eq!(peak, 0.0);
}

#[test]
fn titles_reuse_free_numbers() {
    let (mut g, a, _e) = setup();
    let first = g.add_osc(pos2(0.0, 0.0), &a);
    g.rename(first, "LFO");
    let second = g.add_osc(pos2(0.0, 0.0), &a);
    assert_eq!(g.node(second).unwrap().title, "OSC 1");
}

#[test]
fn oscillator_through_mixer_reaches_the_output() {
    let (mut g, a, mut e) = setup();
    let x = g.add_osc(pos2(0.0, 0.0), &a);
    let m = g.add_mixer(pos2(0.0, 0.0), &a);

    // Unpatched: silence.
    assert_eq!(render(&mut e, 4800).0, 0.0);

    g.connect(port(x, 0), port(m, 0), &a);
    let (peak, finite) = render(&mut e, 4800);
    assert!(finite);
    assert!(peak > 0.05 && peak <= 1.0, "peak = {peak}");

    // Mute the channel: after parameter smoothing the output is silent.
    mixer_params(&g, m).mute[0].store(true, Ordering::Relaxed);
    render(&mut e, 4800);
    let (peak, _) = render(&mut e, 4800);
    assert!(peak < 1e-3, "muted peak = {peak}");
}

#[test]
fn every_waveform_with_modulation_stays_finite_and_bounded() {
    let (mut g, a, mut e) = setup();
    let lfo = g.add_osc(pos2(0.0, 0.0), &a);
    let x = g.add_osc(pos2(0.0, 0.0), &a);
    let m = g.add_mixer(pos2(0.0, 0.0), &a);
    g.connect(port(lfo, 0), port(x, 0), &a); // FM
    g.connect(port(lfo, 0), port(x, 1), &a); // AM
    g.connect(port(x, 0), port(m, 0), &a);

    let lp = osc_params(&g, lfo);
    lp.set(osc::FREQ, 3.0);
    lp.set(osc::LEVEL, 1.0);
    let xp = osc_params(&g, x);
    xp.set(osc::LEVEL, 1.0);
    xp.set(osc::FM, 1.0);
    xp.set(osc::AM, 1.0);

    for wave in Waveform::ALL {
        for shape in [0.0, 0.5, 1.0] {
            xp.set_waveform(wave);
            xp.set(osc::SHAPE, shape);
            xp.set(osc::FREQ, 12_000.0);
            let (peak, finite) = render(&mut e, 9600);
            assert!(finite, "{wave:?} shape {shape} produced NaN/inf");
            assert!(peak <= 1.0);
            assert!(peak > 0.0, "{wave:?} shape {shape} is silent");
        }
    }
}

// ---------------------------------------------------------------------------
// Ambient modules
// ---------------------------------------------------------------------------

fn module_params(g: &Graph, id: u32) -> Arc<ModuleParams> {
    match &g.node(id).unwrap().kind {
        NodeKind::Module(m) => m.params.clone(),
        _ => panic!("not a module"),
    }
}

/// Renders `frames` stereo frames and returns them interleaved.
fn render_buf(engine: &mut Engine, frames: usize) -> Vec<f32> {
    let mut buf = vec![0.0f32; frames * 2];
    for chunk in buf.chunks_mut(2 * 300) {
        engine.render(chunk, 2);
    }
    buf
}

fn peak_of(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

fn rms_of(buf: &[f32]) -> f32 {
    (buf.iter().map(|s| s * s).sum::<f32>() / buf.len().max(1) as f32).sqrt()
}

/// A noisy, harmonically rich source: saw at 220 Hz.
fn source(g: &mut Graph, a: &AudioHandle) -> u32 {
    let x = g.add_osc(pos2(0.0, 0.0), a);
    let p = osc_params(g, x);
    p.set_waveform(Waveform::Saw);
    p.set(osc::FREQ, 220.0);
    p.set(osc::LEVEL, 0.5);
    x
}

/// Source → module → mixer; returns (module id, its params).
fn patch(g: &mut Graph, a: &AudioHandle, kind: ModuleKind) -> (u32, Arc<ModuleParams>) {
    let src = source(g, a);
    let m = g.add_module(kind, pos2(0.0, 0.0), a);
    let mix = g.add_mixer(pos2(0.0, 0.0), a);
    if !kind.def().inputs.is_empty() {
        g.connect(port(src, 0), port(m, 0), a);
    }
    g.connect(port(m, 0), port(mix, 0), a);
    (m, module_params(g, m))
}

/// Removes the cable feeding the module's audio input.
fn unplug_input(g: &mut Graph, a: &AudioHandle, m: u32) {
    let cable = *g.cables.iter().find(|c| c.to == port(m, 0)).expect("input cable");
    g.disconnect(cable, a);
}

#[test]
fn every_module_stays_finite_across_settings() {
    for kind in ModuleKind::ALL {
        let def = kind.def();
        let (mut g, a, mut e) = setup();
        let (m, p) = patch(&mut g, &a, kind);
        // Modulate every CV input with a fast LFO.
        let lfo = g.add_osc(pos2(0.0, 0.0), &a);
        osc_params(&g, lfo).set(osc::FREQ, 7.0);
        osc_params(&g, lfo).set(osc::LEVEL, 1.0);
        for input in 1..def.inputs.len() {
            g.connect(port(lfo, 0), port(m, input), &a);
        }

        for setting in [0.0, 0.5, 1.0] {
            for (i, spec) in def.params.iter().enumerate() {
                p.set(i, spec.denorm(setting));
            }
            for mode in 0..def.modes.len().max(1) {
                for toggle in [false, true] {
                    p.set_mode(mode);
                    p.set_toggle(toggle);
                    let buf = render_buf(&mut e, 4800);
                    assert!(
                        buf.iter().all(|s| s.is_finite()),
                        "{kind:?} at {setting} mode {mode} toggle {toggle} produced NaN/inf"
                    );
                    assert!(p.meter.take() <= 1.0, "{kind:?} left its output range");
                }
            }
        }
        // Back to defaults it still makes sound.
        let fresh = ModuleParams::new(kind);
        for i in 0..def.params.len() {
            p.set(i, fresh.get(i));
        }
        p.set_mode(0);
        p.set_toggle(false);
        render_buf(&mut e, 48_000);
        let buf = render_buf(&mut e, 24_000);
        assert!(rms_of(&buf) > 0.005, "{kind:?} is silent at its defaults");
    }
}

#[test]
fn reverb_tail_decays_and_freeze_holds_it() {
    use crate::audio::modules::reverb;
    let (mut g, a, mut e) = setup();
    let (m, p) = patch(&mut g, &a, ModuleKind::Reverb);
    p.set(reverb::MIX, 1.0);
    p.set(reverb::DECAY, 3.0);
    render_buf(&mut e, 48_000);
    unplug_input(&mut g, &a, m);

    let early = rms_of(&render_buf(&mut e, 12_000));
    let late = rms_of(&render_buf(&mut e, 96_000)[96_000..]);
    assert!(early > 0.01, "no tail: {early}");
    assert!(late < early * 0.3, "tail does not decay: {early} → {late}");

    // Freeze while it rings: the level holds instead of decaying.
    g.connect(port(1, 0), port(m, 0), &a);
    render_buf(&mut e, 48_000);
    p.set_toggle(true);
    unplug_input(&mut g, &a, m);
    let held_a = rms_of(&render_buf(&mut e, 24_000));
    render_buf(&mut e, 96_000);
    let held_b = rms_of(&render_buf(&mut e, 24_000));
    assert!(held_b > held_a * 0.7, "freeze lost energy: {held_a} → {held_b}");
}

#[test]
fn granular_freeze_keeps_the_cloud_playing() {
    use crate::audio::modules::granular;
    let (mut g, a, mut e) = setup();
    let (m, p) = patch(&mut g, &a, ModuleKind::Granular);
    p.set(granular::MIX, 1.0);
    p.set(granular::PITCH, 7.0);
    render_buf(&mut e, 96_000);
    assert!(p.viz[granular::viz::COUNT].get() > 0.0, "no grains playing");

    p.set_toggle(true);
    unplug_input(&mut g, &a, m);
    render_buf(&mut e, 48_000);
    let frozen = rms_of(&render_buf(&mut e, 48_000));
    assert!(frozen > 0.01, "frozen cloud is silent: {frozen}");

    // Unfrozen with no input, the loop fills with silence.
    p.set_toggle(false);
    render_buf(&mut e, 48_000 * 9);
    let empty = rms_of(&render_buf(&mut e, 24_000));
    assert!(empty < 1e-3, "cloud should fade with no input: {empty}");
}

#[test]
fn delay_echoes_outlast_the_input() {
    use crate::audio::modules::delay;
    let (mut g, a, mut e) = setup();
    let (m, p) = patch(&mut g, &a, ModuleKind::Delay);
    p.set(delay::TIME, 0.25);
    p.set(delay::FEEDBACK, 0.5);
    p.set(delay::MIX, 1.0);
    render_buf(&mut e, 24_000);
    unplug_input(&mut g, &a, m);
    let echoes = rms_of(&render_buf(&mut e, 24_000));
    assert!(echoes > 0.01, "no echoes: {echoes}");
    render_buf(&mut e, 48_000 * 4);
    let gone = rms_of(&render_buf(&mut e, 24_000));
    assert!(gone < echoes * 0.05, "echoes do not die away: {echoes} → {gone}");
}

#[test]
fn filter_modes_shape_the_spectrum() {
    use crate::audio::modules::filter;
    let (mut g, a, mut e) = setup();
    let (_, p) = patch(&mut g, &a, ModuleKind::Filter);
    osc_params(&g, 1).set(osc::FREQ, 6000.0);
    p.set(filter::CUTOFF, 300.0);
    p.set(filter::RESO, 0.0);
    p.set_mode(filter::LOWPASS);
    render_buf(&mut e, 4800);
    let low = rms_of(&render_buf(&mut e, 9600));
    p.set_mode(filter::HIGHPASS);
    render_buf(&mut e, 4800);
    let high = rms_of(&render_buf(&mut e, 9600));
    assert!(low < high * 0.05, "6 kHz through a 300 Hz low-pass: {low} vs high-pass {high}");
}

#[test]
fn random_wanders_within_range() {
    use crate::audio::modules::random;
    let (mut g, a, mut e) = setup();
    let (_, p) = patch(&mut g, &a, ModuleKind::Random);
    p.set(random::RATE, 20.0);
    let mut seen = Vec::new();
    for _ in 0..40 {
        render_buf(&mut e, 600);
        seen.push(p.viz[random::VALUE].get());
    }
    let (lo, hi) = seen.iter().fold((1.0f32, -1.0f32), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    assert!(lo >= -1.0 && hi <= 1.0);
    assert!(hi - lo > 0.5, "barely moves: {lo}..{hi}");
}

#[test]
fn resonator_rings_after_the_excitation_stops() {
    use crate::audio::modules::resonator;
    let (mut g, a, mut e) = setup();
    let (m, p) = patch(&mut g, &a, ModuleKind::Resonator);
    p.set(resonator::DECAY, 4.0);
    render_buf(&mut e, 24_000);
    unplug_input(&mut g, &a, m);
    let ring = rms_of(&render_buf(&mut e, 24_000));
    assert!(ring > 0.005, "no ringing: {ring}");
}

#[test]
fn duplicating_a_module_copies_its_settings() {
    use crate::audio::modules::reverb;
    let (mut g, a, _e) = setup();
    let m = g.add_module(ModuleKind::Reverb, pos2(0.0, 0.0), &a);
    let p = module_params(&g, m);
    p.set(reverb::DECAY, 33.0);
    p.set_toggle(true);
    let copy = g.duplicate(m, &a).unwrap();
    let q = module_params(&g, copy);
    assert_eq!(q.get(reverb::DECAY), 33.0);
    assert!(q.toggled());
    assert_eq!(g.node(copy).unwrap().title, "REVERB 2");
}

#[test]
fn cables_carry_stereo_through_the_mixer() {
    let (mut g, a, mut e) = setup();
    let x = source(&mut g, &a);
    let m = g.add_mixer(pos2(0.0, 0.0), &a);
    g.connect(port(x, 0), port(m, 0), &a);
    mixer_params(&g, m).pan[0].set(-1.0);
    render_buf(&mut e, 4800);
    let buf = render_buf(&mut e, 4800);
    let left: Vec<f32> = buf.iter().step_by(2).copied().collect();
    let right: Vec<f32> = buf.iter().skip(1).step_by(2).copied().collect();
    assert!(peak_of(&left) > 0.1);
    assert!(peak_of(&right) < 1e-3, "hard-left pan leaks right: {}", peak_of(&right));
}

#[test]
fn every_node_type_draws_without_panicking() {
    let (mut g, a, mut e) = setup();
    let mut editor = crate::ui::canvas::Editor::new();
    for (_, types) in crate::graph::NodeType::MENU {
        for &ty in types {
            let id = g.add(ty, pos2(0.0, 0.0), &a);
            // Patch the previous node in, so connected states are drawn too.
            if id > 1 && !g.node(id).unwrap().inputs().is_empty() {
                g.connect(port(id - 1, 0), port(id, 0), &a);
            }
        }
    }
    // The first three nodes form a module.
    let first: Vec<u32> = g.nodes.iter().take(3).map(|n| n.id).collect();
    let gid = g.create_group(&first).unwrap();
    render_buf(&mut e, 4800);
    let ctx = egui::Context::default();
    for frame in 0..6 {
        // Expanded, then the module folded, then everything folded.
        g.set_all_collapsed(frame >= 4);
        g.set_group_collapsed(gid, frame >= 2);
        for node in &mut g.nodes {
            node.update_meters(1.0 / 60.0);
        }
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| editor.show(ui, &mut g, &a));
        out.textures_delta.clear(); // no renderer here to upload them
    }
}

#[test]
fn collapsing_shrinks_a_node_and_keeps_it_patchable() {
    let (mut g, a, mut e) = setup();
    let x = source(&mut g, &a);
    let m = g.add_mixer(pos2(0.0, 0.0), &a);
    g.connect(port(x, 0), port(m, 2), &a);

    let full = g.node(m).unwrap().size();
    let input = g.node(m).unwrap().input_pos(2);
    g.toggle_collapsed(m);
    let small = g.node(m).unwrap().size();
    assert!(small.x < full.x && small.y < full.y, "{full:?} → {small:?}");
    assert_ne!(g.node(m).unwrap().input_pos(2), input, "ports follow the compact layout");
    assert!(g.node(m).unwrap().input_pos(3).y < g.node(m).unwrap().pos.y + small.y);

    // Collapsing is visual only: the audio keeps flowing.
    render_buf(&mut e, 4800);
    assert!(rms_of(&render_buf(&mut e, 4800)) > 0.01);

    let copy = g.duplicate(m, &a).unwrap();
    assert!(g.node(copy).unwrap().collapsed, "duplicates keep the collapsed state");
    g.set_all_collapsed(false);
    assert!(g.nodes.iter().all(|n| !n.collapsed));
    assert_eq!(g.node(m).unwrap().size(), full);
}

/// Random → Filter (CUT), Drone → Filter → Reverb → Mixer.
fn ambient_chain(g: &mut Graph, a: &AudioHandle) -> [u32; 5] {
    let random = g.add_module(ModuleKind::Random, pos2(0.0, 0.0), a);
    let drone = g.add_module(ModuleKind::Drone, pos2(0.0, 300.0), a);
    let filter = g.add_module(ModuleKind::Filter, pos2(340.0, 150.0), a);
    let reverb = g.add_module(ModuleKind::Reverb, pos2(700.0, 150.0), a);
    let mixer = g.add_mixer(pos2(1000.0, 150.0), a);
    g.connect(port(random, 0), port(filter, 1), a);
    g.connect(port(drone, 0), port(filter, 0), a);
    g.connect(port(filter, 0), port(reverb, 0), a);
    g.connect(port(reverb, 0), port(mixer, 0), a);
    [random, drone, filter, reverb, mixer]
}

#[test]
fn modules_expose_only_their_outer_ports() {
    let (mut g, a, _e) = setup();
    let [random, drone, filter, reverb, _mixer] = ambient_chain(&mut g, &a);
    let gid = g.create_group(&[random, drone, filter, reverb]).unwrap();

    let (ins, outs) = g.exposed_ports(g.group(gid).unwrap());
    // Only the drone's free CV inputs and the reverb feeding the mixer.
    assert_eq!(ins, vec![port(drone, 0), port(drone, 1)]);
    assert_eq!(outs, vec![port(reverb, 0)]);

    // Tapping the filter from outside exposes its output as well.
    let x = g.add_osc(pos2(0.0, 0.0), &a);
    g.connect(port(filter, 0), port(x, 0), &a);
    let (_, outs) = g.exposed_ports(g.group(gid).unwrap());
    assert_eq!(outs, vec![port(filter, 0), port(reverb, 0)]);
}

#[test]
fn modules_are_created_renamed_and_dissolved() {
    let (mut g, a, mut e) = setup();
    let [random, drone, filter, reverb, mixer] = ambient_chain(&mut g, &a);
    let first = g.create_group(&[random, drone]).unwrap();
    assert_eq!(g.group(first).unwrap().title, "MODULE 1");

    // A node joins at most one module: it leaves the old one.
    let second = g.create_group(&[drone, filter]).unwrap();
    assert_eq!(g.group(second).unwrap().title, "MODULE 2");
    assert_eq!(g.group(first).unwrap().members, vec![random]);
    assert_eq!(g.group_of(drone).unwrap().id, second);

    g.rename_group(second, "Pad");
    assert_eq!(g.group(second).unwrap().title, "Pad");

    // Grouping is visual: the sound is the same before and after.
    render_buf(&mut e, 9600);
    let grouped = rms_of(&render_buf(&mut e, 9600));
    g.ungroup(second);
    assert!(g.group_of(drone).is_none());
    assert_eq!(g.nodes.len(), 5, "ungrouping keeps the nodes");
    assert!(grouped > 0.005);

    // Deleting its last node removes a module.
    g.remove_node(random, &a);
    assert!(g.group(first).is_none());
    let _ = (reverb, mixer);
}

#[test]
fn duplicating_a_module_copies_its_inner_cables() {
    let (mut g, a, _e) = setup();
    let [random, drone, filter, _reverb, _mixer] = ambient_chain(&mut g, &a);
    let gid = g.create_group(&[random, drone, filter]).unwrap();
    g.set_group_collapsed(gid, true);
    let cables_before = g.cables.len();

    let copies = g.duplicate_nodes(&[random, drone, filter], &a);
    assert_eq!(copies.len(), 3);
    // Random → Filter and Drone → Filter are recreated between the copies;
    // the cable leaving the module (Filter → Reverb) is not.
    assert_eq!(g.cables.len(), cables_before + 2);
    let copy = g.group_of(copies[0]).expect("the copies form a module");
    assert_ne!(copy.id, gid);
    assert_eq!(copy.members.len(), 3);
    assert!(copy.collapsed);
}

#[test]
fn keyboard_makes_and_folds_a_module() {
    let (mut g, a, _e) = setup();
    let [random, drone, filter, _reverb, _mixer] = ambient_chain(&mut g, &a);
    let mut editor = crate::ui::canvas::Editor::new();
    let ctx = egui::Context::default();
    let frame = |editor: &mut crate::ui::canvas::Editor, g: &mut Graph, key: Option<(egui::Key, egui::Modifiers)>| {
        let mut input = egui::RawInput::default();
        if let Some((key, modifiers)) = key {
            input.events.push(egui::Event::ModifiersChanged(modifiers));
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            });
        }
        let mut out = ctx.run_ui(input, |ui| editor.show(ui, g, &a));
        out.textures_delta.clear();
    };

    frame(&mut editor, &mut g, None);
    editor.selected = [random, drone, filter].into_iter().collect();
    frame(&mut editor, &mut g, Some((egui::Key::G, egui::Modifiers::COMMAND)));
    assert_eq!(g.groups.len(), 1);
    let gid = g.groups[0].id;
    assert_eq!(editor.selected_group(&g).map(|m| m.id), Some(gid));

    frame(&mut editor, &mut g, Some((egui::Key::C, egui::Modifiers::NONE)));
    assert!(g.group(gid).unwrap().collapsed, "C folds a selected module");
    frame(&mut editor, &mut g, Some((egui::Key::C, egui::Modifiers::NONE)));
    assert!(!g.group(gid).unwrap().collapsed);

    frame(&mut editor, &mut g, Some((egui::Key::G, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT)));
    assert!(g.groups.is_empty(), "Cmd+Shift+G ungroups");
}

#[test]
fn right_click_opens_a_menu_on_anything() {
    let (mut g, a, _e) = setup();
    let x = source(&mut g, &a);
    let y = g.add_osc(pos2(500.0, 0.0), &a);
    g.connect(port(x, 0), port(y, 0), &a);
    let pair = [g.add_osc(pos2(0.0, 400.0), &a), g.add_osc(pos2(300.0, 400.0), &a)];
    let gid = g.create_group(&pair).unwrap();
    g.set_group_collapsed(gid, true);

    let mut editor = crate::ui::canvas::Editor::new();
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0));
    let run = |editor: &mut crate::ui::canvas::Editor, g: &mut Graph, events: Vec<egui::Event>| {
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| editor.show(ui, g, &a));
        out.textures_delta.clear();
    };
    run(&mut editor, &mut g, vec![]);
    let to_screen = |e: &crate::ui::canvas::Editor, p: egui::Pos2| (e.pan + p.to_vec2() * e.zoom).to_pos2();
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let escape = egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };

    let knob = to_screen(&editor, g.node(x).unwrap().pos + egui::vec2(44.0, 175.0));
    let card = to_screen(&editor, crate::ui::groups::frame_rect(&g, g.group(gid).unwrap()).unwrap().min)
        + egui::vec2(120.0, 40.0) * editor.zoom;
    let mid_cable = {
        let (a_, b_) = (g.node(x).unwrap().output_pos(0), g.node(y).unwrap().input_pos(0));
        to_screen(&editor, a_ + (b_ - a_) * 0.5)
    };
    let empty = egui::pos2(1380.0, 880.0);
    for (what, p, selects) in [
        ("a knob", knob, vec![x]),
        ("a module card", card, pair.to_vec()),
        ("a cable", mid_cable, vec![]),
        ("empty canvas", empty, vec![]),
    ] {
        editor.selected.clear();
        run(&mut editor, &mut g, vec![egui::Event::PointerMoved(p)]);
        run(&mut editor, &mut g, vec![button(p, true)]);
        run(&mut editor, &mut g, vec![button(p, false)]);
        run(&mut editor, &mut g, vec![]);
        assert!(egui::Popup::is_any_open(&ctx), "right-click on {what} opened no menu");
        let mut got: Vec<u32> = editor.selected.iter().copied().collect();
        got.sort();
        assert_eq!(got, selects, "right-click on {what} selects it");
        run(&mut editor, &mut g, vec![escape.clone()]);
        run(&mut editor, &mut g, vec![]);
        assert!(!egui::Popup::is_any_open(&ctx));
    }
}
