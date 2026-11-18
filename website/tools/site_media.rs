// Records the site's example tracks and films (website/assets/media): real
// patches played by the offline engine, the films drawn by the real canvas
// with a camera gliding between the nodes. Not part of the build: it is
// included by site_snapshots.rs, inside its module, to share the canvas and
// the rasterizer. See website/README.md:
//
//   MEDIA=aurora,film-orbits cargo test --release site_media -- --ignored --nocapture
//
// MEDIA picks by name prefix (all when unset). Films go through
// website/tools/encode.swift (H.264 + AAC with AVFoundation), compiled on
// demand into target/site-media.

const MEDIA_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/website/assets/media");
const WORK_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/target/site-media");
const SR: usize = 48_000;

/// A patch that plays a piece: its cues run every tick until the transport
/// stops, then the tails ring out.
struct Song {
    s: Scene,
    stop: f64,
    cue: Box<dyn FnMut(f64)>,
    /// Where the film's camera goes: the nodes in the patch.
    nodes: Vec<NodeId>,
}

fn song(tempo: f32) -> Scene {
    let s = scene(tempo);
    s.a.shared.running.store(false, Ordering::Relaxed);
    s
}

fn secs_of_bar(tempo: f32, bar: f64) -> f64 {
    bar * 4.0 * 60.0 / tempo as f64
}

/// `a` → `b` between `t0` and `t1`, eased.
fn ramp(t: f64, t0: f64, t1: f64, a: f32, b: f32) -> f32 {
    let u = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0) as f32;
    a + (b - a) * u * u * (3.0 - 2.0 * u)
}

/// A one-shot: true the first time `t` reaches `at`.
fn once(done: &mut bool, t: f64, at: f64) -> bool {
    if !*done && t >= at {
        *done = true;
        return true;
    }
    false
}

fn mixer(s: &mut Scene) -> (NodeId, Arc<MixerParams>) {
    let id = s.g.add_mixer(pos2(0.0, 0.0), &s.a);
    (id, mixer_params(&s.g, id))
}

fn vesper_scene(name: &str) -> usize {
    md::vesper::FACTORY.iter().position(|s| s.name == name).unwrap_or_else(|| panic!("no Vesper scene {name}"))
}

fn fjord_program(name: &str) -> usize {
    md::fjord::bank::FACTORY.iter().position(|p| p.name == name).unwrap_or_else(|| panic!("no Fjord program {name}"))
}

// --- The pieces -------------------------------------------------------------

/// Ambient: Vesper's Aurora scene, and grains of it an octave up in a long
/// shimmering space, faded in.
fn aurora() -> Song {
    let tempo = 68.0;
    let mut s = song(tempo);
    let vesper = add(&mut s, ModuleKind::Vesper, 0.0, 0.0);
    let grains = add(&mut s, ModuleKind::Granular, 0.0, 0.0);
    let reverb = add(&mut s, ModuleKind::Reverb, 0.0, 0.0);
    let (mix, m) = mixer(&mut s);
    let p = module_params(&s.g, vesper);
    md::vesper::load_factory(&p, vesper_scene("Aurora"));
    p.set_toggle(true);
    let g = module_params(&s.g, grains);
    use md::granular as gr;
    g.set(gr::POSITION, 1.6);
    g.set(gr::SIZE, 0.32);
    g.set(gr::DENSITY, 9.0);
    g.set(gr::PITCH, 12.0);
    g.set(gr::JITTER, 0.55);
    g.set(gr::SPREAD, 1.0);
    g.set(gr::MIX, 1.0);
    let r = module_params(&s.g, reverb);
    use md::reverb as rv;
    r.set(rv::SIZE, 0.92);
    r.set(rv::DECAY, 14.0);
    r.set(rv::DAMP, 0.55);
    r.set(rv::MOD, 0.6);
    r.set(rv::SHIMMER, 0.25);
    r.set(rv::MIX, 0.8);
    link(&mut s, vesper, 0, mix, 0);
    link(&mut s, vesper, 0, grains, 0);
    link(&mut s, grains, 0, reverb, 0);
    link(&mut s, reverb, 0, mix, 1);
    m.fader[1].set(0.0);
    arrange(&mut s);
    let cue = Box::new(move |t: f64| m.fader[1].set(ramp(t, 8.0, 26.0, 0.0, 0.85)));
    Song { s, stop: 53.0, cue, nodes: vec![vesper, grains, reverb, mix] }
}

/// Euclidean groove: Orrery's rings on Pops 66's instruments through tape,
/// over Vesper's Neon Rain; the rings change pattern every eight bars.
fn orbits(changes: [f64; 2]) -> Song {
    let tempo = 104.0;
    let mut s = song(tempo);
    let orrery = add(&mut s, ModuleKind::Orrery, 0.0, 0.0);
    let pops = add(&mut s, ModuleKind::Pops, 0.0, 0.0);
    let tape = add(&mut s, ModuleKind::Tape, 0.0, 0.0);
    let vesper = add(&mut s, ModuleKind::Vesper, 0.0, 0.0);
    let (mix, m) = mixer(&mut s);
    let o = module_params(&s.g, orrery);
    use md::orrery as or;
    // Base drum, snare, hihat, open hihat, rimshot, conga, claves, cowbell.
    for (r, note) in [36, 38, 42, 46, 37, 63, 75, 56].into_iter().enumerate() {
        or::set(&o, r, or::ring::NOTE, note as f32);
    }
    o.set_toggle(true);
    let pp = module_params(&s.g, pops);
    pp.data[md::pops::data::RHYTHM + 6].set(0.0); // its own rhythm off: the rings play it
    pp.data[md::pops::data::DETAIL].set(1.0); // the mixer: a pad flashes on each hit
    let t = module_params(&s.g, tape);
    t.set(md::tape::SATURATION, 0.5);
    t.set(md::tape::WOW, 0.08);
    t.set(md::tape::FLUTTER, 0.1);
    let v = module_params(&s.g, vesper);
    md::vesper::load_factory(&v, vesper_scene("Neon Rain"));
    v.set_toggle(true);
    link(&mut s, orrery, MIDI_PORT, pops, MIDI_PORT);
    link(&mut s, pops, 0, tape, 0);
    link(&mut s, tape, 0, mix, 0);
    link(&mut s, vesper, 0, mix, 1);
    m.fader[1].set(0.72);
    arrange(&mut s);
    let mut done = [false; 2];
    let cue = Box::new(move |t: f64| {
        // Asked for two beats early; LAUNCH (BAR) lands it on the bar.
        let early = 2.0 * 60.0 / tempo as f64;
        if once(&mut done[0], t, secs_of_bar(tempo, changes[0]) - early) {
            or::recall(&o, 5); // Breaks
        }
        if once(&mut done[1], t, secs_of_bar(tempo, changes[1]) - early) {
            or::recall(&o, 1); // Four floor
        }
    });
    Song { s, stop: secs_of_bar(tempo, 24.0), cue, nodes: vec![orrery, pops, tape, vesper, mix] }
}

/// Polyrhythm: four of Orrery's rings in CYCLE (3 in 8, 5 in 12, 7 in 16,
/// 4 in 10 over one bar) on Fjord's Marimba Box, over a low drone in fifths.
/// TWIST turns the rings slowly apart, the way Reich's phase pieces drift.
fn phases() -> Song {
    let tempo = 84.0;
    let mut s = song(tempo);
    let orrery = add(&mut s, ModuleKind::Orrery, 0.0, 0.0);
    let fjord = add(&mut s, ModuleKind::Fjord, 0.0, 0.0);
    let pingpong = add(&mut s, ModuleKind::PingPong, 0.0, 0.0);
    let reverb = add(&mut s, ModuleKind::Reverb, 0.0, 0.0);
    let drone = add(&mut s, ModuleKind::Drone, 0.0, 0.0);
    let (mix, m) = mixer(&mut s);
    let o = module_params(&s.g, orrery);
    use md::orrery as or;
    o.set_mode(or::CYCLE_MODE);
    o.set(or::CYCLE, 4.0);
    o.set(or::ACCENT, 0.4);
    for r in 0..or::RINGS {
        or::set(&o, r, or::ring::ON, 0.0);
    }
    // Fjord's notes for its four channels (C3, G3, C4, E4 in Marimba Box).
    for (r, (note, steps, pulses, chance)) in [(36, 8, 3, 1.0), (38, 12, 5, 1.0), (50, 16, 7, 0.9), (47, 10, 4, 0.8)].into_iter().enumerate() {
        or::set(&o, r, or::ring::ON, 1.0);
        or::set(&o, r, or::ring::NOTE, note as f32);
        or::set(&o, r, or::ring::CHANNEL, 10.0);
        or::set(&o, r, or::ring::CHANCE, chance);
        or::set_steps(&o, r, steps);
        or::set_pulses(&o, r, pulses);
    }
    o.set_toggle(true);
    let f = module_params(&s.g, fjord);
    md::fjord::load_program(&f, fjord_program("Marimba Box"));
    f.data[md::fjord::data::RUN].set(0.0); // its own steps off
    let pp = module_params(&s.g, pingpong);
    use md::pingpong as pg;
    pp.set(pg::FEEDBACK, 0.45);
    pp.set(pg::TONE, 0.4);
    pp.set(pg::MIX, 0.3);
    let r = module_params(&s.g, reverb);
    r.set(md::reverb::SIZE, 0.75);
    r.set(md::reverb::DECAY, 4.5);
    r.set(md::reverb::MIX, 0.3);
    let d = module_params(&s.g, drone);
    use md::drone as dr;
    d.set_mode(1); // fifths
    d.set(dr::PITCH, 65.41); // C2
    d.set(dr::DETUNE, 8.0);
    d.set(dr::DRIFT, 0.3);
    d.set(dr::TONE, 0.22);
    d.set(dr::SPREAD, 0.85);
    d.set(dr::LEVEL, 0.45);
    link(&mut s, orrery, MIDI_PORT, fjord, MIDI_PORT);
    link(&mut s, fjord, 0, pingpong, 0);
    link(&mut s, pingpong, 0, reverb, 0);
    link(&mut s, reverb, 0, mix, 0);
    link(&mut s, drone, 0, mix, 1);
    m.fader[1].set(0.0);
    arrange(&mut s);
    let cue = Box::new(move |t: f64| {
        m.fader[1].set(ramp(t, 2.0, 12.0, 0.0, 0.4));
        o.set(or::TWIST, ramp(t, 14.0, 46.0, 0.0, 1.0));
    });
    Song { s, stop: 54.0, cue, nodes: vec![orrery, fjord, pingpong, drone, mix] }
}

/// Glitch: Nullpunkt's Raster scene bouncing in a ping pong, Splitter's
/// sliced loop coming in, a dark drone in octaves under a wandering filter;
/// halfway, Nullpunkt turns to its Pulse scene.
fn glitch() -> Song {
    let tempo = 120.0;
    let mut s = song(tempo);
    let null = add(&mut s, ModuleKind::Nullpunkt, 0.0, 0.0);
    let pingpong = add(&mut s, ModuleKind::PingPong, 0.0, 0.0);
    let splitter = add(&mut s, ModuleKind::Splitter, 0.0, 0.0);
    let drone = add(&mut s, ModuleKind::Drone, 0.0, 0.0);
    let random = add(&mut s, ModuleKind::Random, 0.0, 0.0);
    let filter = add(&mut s, ModuleKind::Filter, 0.0, 0.0);
    let (mix, m) = mixer(&mut s);
    let n = module_params(&s.g, null);
    md::nullpunkt::load_factory(&n, 1); // Raster
    let pp = module_params(&s.g, pingpong);
    pp.set(md::pingpong::FEEDBACK, 0.35);
    pp.set(md::pingpong::MIX, 0.22);
    splitter_sample(&module_params(&s.g, splitter));
    let d = module_params(&s.g, drone);
    use md::drone as dr;
    d.set_mode(2); // octaves
    d.set(dr::PITCH, 41.2); // E1
    d.set(dr::DETUNE, 6.0);
    d.set(dr::DRIFT, 0.25);
    d.set(dr::TONE, 0.15);
    d.set(dr::SPREAD, 0.9);
    d.set(dr::LEVEL, 0.4);
    let rn = module_params(&s.g, random);
    rn.set(md::random::RATE, 0.25);
    rn.set(md::random::SMOOTH, 1.0);
    let fl = module_params(&s.g, filter);
    fl.set(md::filter::CUTOFF, 320.0);
    fl.set(md::filter::RESO, 0.25);
    fl.set(md::filter::MOD, 1.2);
    link(&mut s, null, 0, pingpong, 0);
    link(&mut s, pingpong, 0, mix, 0);
    link(&mut s, splitter, 0, mix, 1);
    link(&mut s, drone, 0, filter, 0);
    link(&mut s, random, 0, filter, 1);
    link(&mut s, filter, 0, mix, 2);
    m.fader[1].set(0.0);
    m.fader[2].set(0.4);
    arrange(&mut s);
    let mut turned = false;
    let cue = Box::new(move |t: f64| {
        m.fader[1].set(ramp(t, 12.0, 16.0, 0.0, 0.62));
        if once(&mut turned, t, secs_of_bar(tempo, 16.0)) {
            md::nullpunkt::load_factory(&n, 2); // Pulse
        }
    });
    Song { s, stop: secs_of_bar(tempo, 27.0), cue, nodes: vec![null, splitter, drone, filter, mix] }
}

/// Rhythm box: Pops 66's Bossanova through a worn tape, over Vesper's Golden
/// Hour; the machine comes in after two bars.
fn golden_hour() -> Song {
    let tempo = 96.0;
    let mut s = song(tempo);
    let pops = add(&mut s, ModuleKind::Pops, 0.0, 0.0);
    let tape = add(&mut s, ModuleKind::Tape, 0.0, 0.0);
    let vesper = add(&mut s, ModuleKind::Vesper, 0.0, 0.0);
    let (mix, m) = mixer(&mut s);
    let pp = module_params(&s.g, pops);
    pp.data[md::pops::data::RHYTHM + 6].set(0.0);
    pp.data[md::pops::data::RHYTHM + 5].set(1.0); // Bossanova
    pp.data[md::pops::data::DETAIL].set(1.0);
    let t = module_params(&s.g, tape);
    t.set(md::tape::SATURATION, 0.45);
    t.set(md::tape::WOW, 0.2);
    t.set(md::tape::FLUTTER, 0.15);
    t.set(md::tape::AGE, 0.25);
    let v = module_params(&s.g, vesper);
    md::vesper::load_factory(&v, vesper_scene("Golden Hour"));
    v.set_toggle(true);
    link(&mut s, pops, 0, tape, 0);
    link(&mut s, tape, 0, mix, 0);
    link(&mut s, vesper, 0, mix, 1);
    m.fader[0].set(0.0);
    m.fader[1].set(0.8);
    arrange(&mut s);
    let cue = Box::new(move |t: f64| m.fader[0].set(ramp(t, secs_of_bar(tempo, 2.0) - 0.3, secs_of_bar(tempo, 2.0), 0.0, 0.64)));
    Song { s, stop: secs_of_bar(tempo, 22.0), cue, nodes: vec![pops, tape, vesper, mix] }
}

// --- Recording --------------------------------------------------------------

/// Lets the engine take the patch in, and Vesper's thread render its sounds,
/// with the transport stopped.
fn warm_up(s: &mut Scene) {
    for _ in 0..80 {
        render_buf(&mut s.e, 600);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Plays `secs` of the song in ticks of 1/`fps` s, drawing a film frame
/// before each tick when there is a film; returns the sound.
fn play(song: &mut Song, secs: f64, fps: usize, mut film: Option<&mut Film>) -> Vec<f32> {
    warm_up(&mut song.s);
    let per = SR / fps;
    let ticks = (secs * fps as f64).round() as usize;
    let mut out = Vec::with_capacity(ticks * per * 2);
    song.s.a.shared.running.store(true, Ordering::Relaxed);
    let started = std::time::Instant::now();
    for tick in 0..ticks {
        let t = tick as f64 / fps as f64;
        if t >= song.stop {
            song.s.a.shared.running.store(false, Ordering::Relaxed);
        } else {
            (song.cue)(t);
        }
        if let Some(f) = film.as_deref_mut() {
            f.frame(&mut song.s, tick, t);
            if tick % (fps * 5) == 0 {
                eprintln!("  {t:>4.0} s ({:.1} s)", started.elapsed().as_secs_f32());
            }
        }
        out.extend(render_buf(&mut song.s.e, per));
    }
    out
}

struct Mastered {
    gain_db: f32,
    peak_db: f32,
    loud_db: f32,
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

/// Brings a take to a common loudness (gated RMS −17 dBFS, peaks under
/// −1 dBFS, a gentle limiter above −6 dBFS for the odd peak), with a click-
/// free start and a short fade at the end.
fn master(buf: &mut [f32]) -> Mastered {
    let win = SR * 2 * 4 / 10;
    let (mut sum, mut n) = (0.0f64, 0usize);
    for w in buf.chunks(win) {
        let ms = w.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / w.len() as f64;
        if ms > 10f64.powf(-50.0 / 10.0) {
            sum += ms;
            n += 1;
        }
    }
    let rms = ((sum / n.max(1) as f64).sqrt()) as f32;
    let gain = 10f32.powf(-17.0 / 20.0) / rms.max(1e-6);
    // Soft knee from −6 dBFS to the −1 dBFS ceiling.
    let (knee, ceil) = (10f32.powf(-6.0 / 20.0), 10f32.powf(-1.0 / 20.0));
    let frames = buf.len() / 2;
    let fade_in = SR / 50;
    let fade_out = SR * 5 / 2;
    for i in 0..frames {
        let mut env = 1.0;
        if i < fade_in {
            env = i as f32 / fade_in as f32;
        }
        if i + fade_out > frames {
            let u = (frames - i) as f32 / fade_out as f32;
            env *= u * u;
        }
        for c in 0..2 {
            let x = buf[i * 2 + c] * gain;
            let a = x.abs();
            let y = if a <= knee { a } else { knee + (ceil - knee) * ((a - knee) / (ceil - knee)).tanh() };
            buf[i * 2 + c] = y.copysign(x) * env;
        }
    }
    let peak = buf.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    Mastered { gain_db: db(gain), peak_db: db(peak), loud_db: db(rms * gain) }
}

fn write_wav(path: &str, buf: &[f32]) {
    let data: Vec<u8> = buf.iter().flat_map(|x| ((x.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes()).collect();
    let mut w = Vec::with_capacity(44 + data.len());
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&(SR as u32).to_le_bytes());
    w.extend_from_slice(&(SR as u32 * 4).to_le_bytes());
    w.extend_from_slice(&4u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(&data);
    std::fs::write(path, w).unwrap();
}

fn run(cmd: &str, args: &[&str]) {
    let out = std::process::Command::new(cmd).args(args).output().unwrap_or_else(|e| panic!("{cmd}: {e}"));
    assert!(out.status.success(), "{cmd} {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
}

/// The take as AAC (192 kb/s) in an .m4a.
fn to_aac(buf: &[f32], name: &str, dir: &str) -> String {
    std::fs::create_dir_all(dir).unwrap();
    let wav = format!("{WORK_DIR}/{name}.wav");
    let m4a = format!("{dir}/{name}.m4a");
    write_wav(&wav, buf);
    std::fs::remove_file(&m4a).ok();
    run("afconvert", &["-f", "m4af", "-d", "aac", "-b", "192000", "-q", "127", &wav, &m4a]);
    m4a
}

/// The waveform drawn by the site's player, as a line of `peaks.js`: the
/// loudest sample and the RMS in each of `bins` slices, 0–100 of full scale.
fn waveform(name: &str, buf: &[f32], bins: usize) -> String {
    let frames = buf.len() / 2;
    let (mut peak, mut rms) = (Vec::new(), Vec::new());
    for b in 0..bins {
        let (a, z) = (b * frames / bins, (b + 1) * frames / bins);
        let s = &buf[a * 2..z * 2];
        let m = s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let r = (s.iter().map(|x| x * x).sum::<f32>() / s.len().max(1) as f32).sqrt();
        peak.push(((m * 100.0).round().min(100.0) as u8).to_string());
        rms.push(((r * 100.0).round().min(100.0) as u8).to_string());
    }
    format!("  \"{name}\": {{\"peak\": [{}], \"rms\": [{}]}}", peak.join(","), rms.join(","))
}

// --- Films ------------------------------------------------------------------

/// Where the camera looks at a time: a world point at the centre, and a zoom.
#[derive(Clone, Copy)]
struct Shot {
    at: f64,
    centre: Pos2,
    zoom: f32,
}

struct Film {
    ctx: egui::Context,
    editor: Editor,
    texs: HashMap<TextureId, Tex>,
    size: Vec2,
    ppp: f32,
    w: usize,
    h: usize,
    shots: Vec<Shot>,
    poster: (f64, String),
    encoder: std::process::Child,
    bgra: Vec<u8>,
}

fn encoder() -> String {
    let bin = format!("{WORK_DIR}/encode");
    let src = concat!(env!("CARGO_MANIFEST_DIR"), "/website/tools/encode.swift");
    let stale = match (std::fs::metadata(&bin), std::fs::metadata(src)) {
        (Ok(b), Ok(s)) => b.modified().unwrap() < s.modified().unwrap(),
        _ => true,
    };
    if stale {
        std::fs::create_dir_all(WORK_DIR).unwrap();
        run("swiftc", &["-O", "-suppress-warnings", src, "-o", &bin]);
    }
    bin
}

impl Film {
    fn new(name: &str, size: Vec2, ppp: f32, fps: usize, shots: Vec<Shot>, poster_at: f64) -> Self {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        ctx.set_pixels_per_point(ppp);
        ctx.options_mut(|o| o.warn_on_id_clash = false);
        #[cfg(debug_assertions)]
        ctx.all_styles_mut(|st| st.debug.warn_if_rect_changes_id = false);
        let (w, h) = ((size.x * ppp).round() as usize, (size.y * ppp).round() as usize);
        let encoder = std::process::Command::new(encoder())
            .args(["video", &w.to_string(), &h.to_string(), &fps.to_string(), &format!("{WORK_DIR}/{name}-video.mp4")])
            .env("BITRATE", "2400000")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("start the encoder");
        Film { ctx, editor: Editor::new(), texs: HashMap::new(), size, ppp, w, h, shots, poster: (poster_at, name.to_owned()), encoder, bgra: vec![0; w * h * 4] }
    }

    /// The camera at `t`: eased between the shots, the zoom on a log scale.
    fn camera(&self, t: f64) -> (Pos2, f32) {
        let s = &self.shots;
        let k = s.iter().rposition(|x| x.at <= t).unwrap_or(0);
        if k + 1 >= s.len() {
            return (s[k].centre, s[k].zoom);
        }
        let (a, b) = (s[k], s[k + 1]);
        let u = ((t - a.at) / (b.at - a.at)).clamp(0.0, 1.0) as f32;
        let u = u * u * (3.0 - 2.0 * u);
        let zoom = (a.zoom.ln() + (b.zoom.ln() - a.zoom.ln()) * u).exp();
        (a.centre + (b.centre - a.centre) * u, zoom)
    }

    fn frame(&mut self, s: &mut Scene, tick: usize, t: f64) {
        let (centre, zoom) = self.camera(t);
        // The first pass frames the patch by itself: let it, then steer.
        let passes = if tick == 0 { 2 } else { 1 };
        let mut shapes = Vec::new();
        let mut ppp = self.ppp;
        for _ in 0..passes {
            self.editor.zoom = zoom;
            self.editor.pan = self.size * 0.5 - centre.to_vec2() * zoom;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
                time: Some(t),
                ..Default::default()
            };
            let (g, a, editor) = (&mut s.g, &s.a, &mut self.editor);
            let mut out = self.ctx.run_ui(input, |ui| editor.show(ui, g, a));
            apply(&mut self.texs, &out.textures_delta);
            out.textures_delta.clear();
            shapes = out.shapes;
            ppp = out.pixels_per_point;
        }
        let prims = self.ctx.tessellate(shapes, ppp);
        let (w, h) = (self.w, self.h);
        let bands = 24;
        let rows = h.div_ceil(bands);
        let texs = &self.texs;
        let parts: Vec<Vec<[f32; 3]>> = std::thread::scope(|sc| {
            let jobs: Vec<_> = (0..bands)
                .map(|b| {
                    let (y0, y1) = ((b * rows).min(h), ((b + 1) * rows).min(h));
                    let prims = &prims;
                    sc.spawn(move || raster_rows(prims, texs, w, (y0, y1), ppp))
                })
                .collect();
            jobs.into_iter().map(|j| j.join().unwrap()).collect()
        });
        let img: Vec<[f32; 3]> = parts.into_iter().flatten().collect();
        for (px, c) in self.bgra.chunks_exact_mut(4).zip(&img) {
            let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            px.copy_from_slice(&[b(c[2]), b(c[1]), b(c[0]), 255]);
        }
        use std::io::Write;
        self.encoder.stdin.as_mut().unwrap().write_all(&self.bgra).expect("feed the encoder");
        let fps_tick = 1.0 / 30.0;
        if (t - self.poster.0).abs() < fps_tick * 0.5 {
            let name = format!("{}-poster", self.poster.1);
            save_in(MEDIA_DIR, &name, &img, w, (0, 0, w, h));
            let png = format!("{MEDIA_DIR}/{name}.png");
            run("sips", &["-s", "format", "jpeg", "-s", "formatOptions", "84", &png, "--out", &format!("{MEDIA_DIR}/{name}.jpg")]);
            // The thumbnail the site's film picker shows.
            let thumb = format!("{MEDIA_DIR}/{}-thumb.jpg", self.poster.1);
            run("sips", &["-Z", "480", "-s", "format", "jpeg", "-s", "formatOptions", "80", &png, "--out", &thumb]);
            std::fs::remove_file(png).ok();
        }
    }

    fn finish(mut self) {
        drop(self.encoder.stdin.take());
        assert!(self.encoder.wait().unwrap().success(), "the encoder failed");
    }
}

/// The node's frame and the zoom that fits it in the picture.
fn close_up(s: &Scene, id: NodeId, size: Vec2, at: f64) -> Shot {
    let n = s.g.node(id).unwrap();
    let r = Rect::from_min_size(n.pos, n.size());
    let zoom = ((size.x - 120.0) / r.width()).min((size.y - 110.0) / r.height()).min(1.35);
    Shot { at, centre: r.center(), zoom }
}

fn wide(s: &Scene, size: Vec2, at: f64) -> Shot {
    let b = bounds(&s.g);
    let zoom = ((size.x - 90.0) / b.width()).min((size.y - 90.0) / b.height()).min(1.0);
    Shot { at, centre: b.center(), zoom }
}

/// A camera that opens on the whole patch, visits `visits` (node, seconds
/// held) gliding between them, and ends on the whole patch again.
fn tour(s: &Scene, size: Vec2, open: f64, visits: &[(NodeId, f64)], glide: f64) -> Vec<Shot> {
    let mut shots = vec![wide(s, size, 0.0), wide(s, size, open)];
    let mut t = open;
    for &(id, hold) in visits {
        t += glide;
        shots.push(close_up(s, id, size, t));
        t += hold;
        shots.push(close_up(s, id, size, t));
    }
    shots.push(wide(s, size, t + glide));
    shots
}

fn film(name: &str, mut song: Song, secs: f64, visits: impl Fn(&Song) -> Vec<(NodeId, f64)>, poster_at: f64) {
    const FPS: usize = 30;
    song.stop = song.stop.min(secs - 4.5);
    let size = vec2(1280.0, 720.0);
    let shots = tour(&song.s, size, 3.0, &visits(&song), 1.8);
    let mut f = Film::new(name, size, 1.5, FPS, shots, poster_at);
    eprintln!("{name}: {} × {} at {FPS} fps, {secs} s", f.w, f.h);
    let mut audio = play(&mut song, secs, FPS, Some(&mut f));
    f.finish();
    let m = master(&mut audio);
    eprintln!("{name}: gain {:+.1} dB, loudness {:.1} dBFS, peak {:.1} dBFS", m.gain_db, m.loud_db, m.peak_db);
    let m4a = to_aac(&audio, name, WORK_DIR);
    let out = format!("{MEDIA_DIR}/{name}.mp4");
    run(&encoder(), &["mux", &format!("{WORK_DIR}/{name}-video.mp4"), &m4a, &out]);
    eprintln!("wrote {out}");
}

fn wanted(name: &str) -> bool {
    std::env::var("MEDIA").ok().is_none_or(|o| o.split(',').any(|w| name.starts_with(w)))
}

#[test]
#[ignore]
fn site_media() {
    std::fs::create_dir_all(WORK_DIR).unwrap();
    std::fs::create_dir_all(MEDIA_DIR).unwrap();
    let tracks: [(&str, fn() -> Song); 5] = [
        ("aurora", aurora),
        ("orbits", || orbits([8.0, 16.0])),
        ("phases", phases),
        ("raster", glitch),
        ("golden-hour", golden_hour),
    ];
    let mut waves = Vec::new();
    for (name, make) in tracks {
        if !wanted(name) {
            continue;
        }
        let mut song = make();
        let mut audio = play(&mut song, 60.0, 60, None);
        let m = master(&mut audio);
        let out = to_aac(&audio, name, MEDIA_DIR);
        eprintln!("{name}: gain {:+.1} dB, loudness {:.1} dBFS, peak {:.1} dBFS → {out}", m.gain_db, m.loud_db, m.peak_db);
        waves.push(waveform(name, &audio, 240));
    }
    if !waves.is_empty() {
        // Merged into what is there, so one track can be redone alone.
        let path = format!("{MEDIA_DIR}/peaks.js");
        let mut all: Vec<String> = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("  \"") && !waves.iter().any(|w| w.split(':').next() == l.split(':').next()))
            .map(|l| l.trim_end_matches(',').to_owned())
            .collect();
        all.extend(waves);
        all.sort();
        let js = format!(
            "// Waveforms of the example tracks: the loudest sample and the RMS in each of 240 slices,\n// 0-100 of full scale (written by website/tools/site_media.rs).\nwindow.TRACK_PEAKS = {{\n{}\n}};\n",
            all.join(",\n")
        );
        std::fs::write(&path, js).unwrap();
    }
    // The films: a camera tour of three of the patches as they play.
    if wanted("film-orbits") {
        film("film-orbits", orbits([5.0, 11.0]), 40.0, |s| vec![(s.nodes[0], 12.0), (s.nodes[1], 7.0), (s.nodes[3], 6.0)], 9.0);
    }
    if wanted("film-aurora") {
        film("film-aurora", aurora(), 40.0, |s| vec![(s.nodes[0], 16.0), (s.nodes[1], 8.0)], 12.0);
    }
    if wanted("film-raster") {
        film("film-raster", glitch(), 40.0, |s| vec![(s.nodes[0], 10.0), (s.nodes[1], 8.0), (s.nodes[2], 5.0)], 9.0);
    }
}

// --- Looking at the sound (for tuning the pieces) -----------------------------

fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -std::f32::consts::TAU / len as f32;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (ang * k as f32).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * c - im[b] * s, re[b] * s + im[b] * c);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

/// A spectrogram (30 Hz–16 kHz, log) under a loudness strip, as a PNG.
fn spectrogram(buf: &[f32], name: &str, dir: &str) {
    let n = 4096;
    let hop = SR / 25;
    let frames = buf.len() / 2;
    let cols = (frames.saturating_sub(n)) / hop;
    let (rows, strip) = (320usize, 60usize);
    let h = rows + strip;
    let mut img = vec![[0.0f32; 3]; cols * h];
    let win: Vec<f32> = (0..n).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos()).collect();
    for c in 0..cols {
        let at = c * hop;
        let mut re: Vec<f32> = (0..n).map(|i| 0.5 * (buf[(at + i) * 2] + buf[(at + i) * 2 + 1]) * win[i]).collect();
        let mut im = vec![0.0; n];
        let rms = (re.iter().map(|x| x * x).sum::<f32>() / n as f32 * 2.67).sqrt();
        fft(&mut re, &mut im);
        for r in 0..rows {
            let f = 30.0 * (16000.0f32 / 30.0).powf(1.0 - r as f32 / rows as f32);
            let bin = (f / SR as f32 * n as f32) as usize;
            let mag = (re[bin].hypot(im[bin]) / (n as f32 * 0.25)).max(1e-9);
            let v = ((db(mag) + 100.0) / 85.0).clamp(0.0, 1.0);
            img[(strip + r) * cols + c] = [v.powf(0.7), v.powf(1.6), (0.35 + v * 0.9).min(1.0) * (1.0 - v * 0.6)];
        }
        let level = ((db(rms) + 60.0) / 60.0).clamp(0.0, 1.0);
        for y in 0..strip {
            if (strip - y) as f32 <= level * strip as f32 {
                img[y * cols + c] = [0.9, 0.8, 0.4];
            }
        }
    }
    save_in(dir, name, &img, cols, (0, 0, cols, h));
}

/// Each mixer channel alone, and the whole mix as a spectrogram, in
/// SNAP_DIR: MEDIA=… cargo test --release site_media_probe -- --ignored
#[test]
#[ignore]
fn site_media_probe() {
    let dir = std::env::var("SNAP_DIR").unwrap_or_else(|_| WORK_DIR.to_owned());
    let tracks: [(&str, fn() -> Song); 5] = [
        ("aurora", aurora),
        ("orbits", || orbits([8.0, 16.0])),
        ("phases", phases),
        ("raster", glitch),
        ("golden-hour", golden_hour),
    ];
    for (name, make) in tracks {
        if !wanted(name) {
            continue;
        }
        let mut song = make();
        let audio = play(&mut song, 60.0, 60, None);
        spectrogram(&audio, &format!("spec-{name}"), &dir);
        let stat = |b: &[f32]| {
            let rms = (b.iter().map(|x| x * x).sum::<f32>() / b.len() as f32).sqrt();
            let peak = b.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            (db(rms), db(peak))
        };
        let per5: Vec<String> = audio.chunks(SR * 2 * 5).map(|c| format!("{:.0}", stat(c).0)).collect();
        let (r, p) = stat(&audio);
        eprintln!("{name}: mix rms {r:.1} peak {p:.1} | every 5 s: {}", per5.join(" "));
        for ch in 0..4 {
            let mut song = make();
            let id = song.nodes[song.nodes.len() - 1];
            let m = mixer_params(&song.s.g, id);
            if !song.s.g.cables.iter().any(|l| l.to.node == id && l.to.port == ch) {
                continue;
            }
            m.solo[ch].store(true, Ordering::Relaxed);
            let a = play(&mut song, 60.0, 60, None);
            let (r, p) = stat(&a);
            let per5: Vec<String> = a.chunks(SR * 2 * 5).map(|c| format!("{:.0}", stat(c).0)).collect();
            eprintln!("  ch{ch}: rms {r:.1} peak {p:.1} crest {:.1} | {}", p - r, per5.join(" "));
        }
    }
}
