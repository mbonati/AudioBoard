//! Vesper's sounds: some forty recipes for each of Pad, Bass, Noise and
//! Ambience, and sixteen timbres for Shimmer's pings. A recipe says what a looped stereo
//! sample should hold (see [`super::library`]): a family of partials, each
//! widened into a band, a noise floor, a spectral shape over both, and then
//! what happens in time (swells, a sweeping band, grit, dust, chimes).

/// `(centre in Hz, width in octaves, gain)`.
pub type Formant = (f32, f32, f32);

const NO_FORMANTS: [Formant; 3] = [(0.0, 1.0, 0.0); 3];
const AH: [Formant; 3] = [(730.0, 0.35, 1.0), (1090.0, 0.3, 0.55), (2440.0, 0.3, 0.3)];
const OH: [Formant; 3] = [(570.0, 0.35, 1.0), (840.0, 0.3, 0.6), (2410.0, 0.3, 0.2)];
const OO: [Formant; 3] = [(300.0, 0.4, 1.0), (870.0, 0.3, 0.35), (2240.0, 0.3, 0.12)];
const EE: [Formant; 3] = [(270.0, 0.35, 0.8), (2290.0, 0.3, 0.7), (3010.0, 0.3, 0.5)];
const EH: [Formant; 3] = [(530.0, 0.35, 1.0), (1840.0, 0.3, 0.55), (2480.0, 0.3, 0.35)];
const BODY: [Formant; 3] = [(420.0, 0.7, 0.9), (1250.0, 0.7, 0.6), (3200.0, 0.8, 0.35)];
const BRASS: [Formant; 3] = [(1100.0, 1.0, 1.0), (2400.0, 0.7, 0.5), (0.0, 1.0, 0.0)];
const METAL: [Formant; 3] = [(1800.0, 0.6, 1.0), (3900.0, 0.5, 0.7), (6400.0, 0.5, 0.5)];
const CHEST: [Formant; 3] = [(110.0, 0.8, 1.0), (260.0, 0.7, 0.7), (620.0, 0.7, 0.3)];
const TUNNEL: [Formant; 3] = [(70.0, 0.9, 1.0), (180.0, 0.7, 0.6), (440.0, 0.8, 0.25)];
const HOWL: [Formant; 3] = [(520.0, 0.5, 1.0), (1040.0, 0.4, 0.45), (2100.0, 0.4, 0.2)];
const RADIO: [Formant; 3] = [(1400.0, 0.8, 1.0), (3100.0, 0.5, 0.6), (0.0, 1.0, 0.0)];
const REED: [Formant; 3] = [(1500.0, 0.8, 1.0), (3400.0, 0.6, 0.6), (0.0, 1.0, 0.0)];
const LOW_BRASS: [Formant; 3] = [(280.0, 1.0, 1.0), (900.0, 0.9, 0.7), (2400.0, 0.7, 0.3)];
const FLOOR: [Formant; 3] = [(90.0, 0.9, 1.0), (240.0, 0.9, 0.8), (700.0, 0.8, 0.35)];

#[derive(Clone, Copy)]
pub struct Recipe {
    pub name: &'static str,
    /// Follows the notes; else plays at the pitch it was made at, always.
    pub tonal: bool,
    /// The note the partials stand on when it isn't tonal.
    pub pitch: f32,
    /// The partials' level against the noise.
    pub partials: f32,
    /// Each partial's level: k^−tilt.
    pub tilt: f32,
    /// How far the even partials are gone.
    pub odd: f32,
    /// A pulse wave's duty (0: not one), its gaps in the spectrum.
    pub pulse: f32,
    /// Partial k stands at k^(1 + stretch): above 0, bells and metal.
    pub stretch: f32,
    /// Each partial off by up to this much, at random.
    pub scatter: f32,
    /// Each partial's band: this many cents for the first, × k^grow.
    pub width: f32,
    pub grow: f32,
    /// A second family a fifth up, and a sub an octave down, by level.
    pub fifth: f32,
    pub sub: f32,
    /// The noise's level against the partials, and its slope in dB/octave.
    pub noise: f32,
    pub slope: f32,
    /// The shape over everything: corners in Hz, and formants.
    pub low: f32,
    pub high: f32,
    pub formants: [Formant; 3],
    /// Saturation of the whole loop: partials beat against each other.
    pub grit: f32,
    /// Slow swells: how deep, how many in a loop.
    pub swell: f32,
    pub cycles: f32,
    /// A band sweeping to and fro: lowest and highest Hz, sweeps in a loop,
    /// and how much of the sound goes through it.
    pub sweep: (f32, f32, f32, f32),
    /// Clicks a second.
    pub dust: f32,
    /// Chimes a second (on harmonics of the pitch), and how long they ring.
    pub chime: f32,
    pub ring: f32,
    /// How far a voice wanders in pitch and level while it plays (1: as a
    /// pad; 0: fixed, as a pipe).
    pub life: f32,
}

impl Recipe {
    const fn new(name: &'static str) -> Self {
        Self {
            name,
            tonal: true,
            pitch: 36.0,
            partials: 1.0,
            tilt: 1.1,
            odd: 0.0,
            pulse: 0.0,
            stretch: 0.0,
            scatter: 0.0,
            width: 18.0,
            grow: 0.6,
            fifth: 0.0,
            sub: 0.0,
            noise: 0.0,
            slope: -3.0,
            low: 30.0,
            high: 9000.0,
            formants: NO_FORMANTS,
            grit: 0.0,
            swell: 0.0,
            cycles: 3.0,
            sweep: (0.0, 0.0, 0.0, 0.0),
            dust: 0.0,
            chime: 0.0,
            ring: 1.2,
            life: 1.0,
        }
    }

    const fn tilt(self, v: f32) -> Self {
        Self { tilt: v, ..self }
    }
    const fn odd(self, v: f32) -> Self {
        Self { odd: v, ..self }
    }
    const fn pulse(self, v: f32) -> Self {
        Self { pulse: v, ..self }
    }
    const fn stretch(self, v: f32) -> Self {
        Self { stretch: v, ..self }
    }
    const fn scatter(self, v: f32) -> Self {
        Self { scatter: v, ..self }
    }
    const fn width(self, v: f32) -> Self {
        Self { width: v, ..self }
    }
    const fn grow(self, v: f32) -> Self {
        Self { grow: v, ..self }
    }
    const fn fifth(self, v: f32) -> Self {
        Self { fifth: v, ..self }
    }
    const fn sub(self, v: f32) -> Self {
        Self { sub: v, ..self }
    }
    const fn noise(self, v: f32, slope: f32) -> Self {
        Self {
            noise: v,
            slope,
            ..self
        }
    }
    const fn low(self, v: f32) -> Self {
        Self { low: v, ..self }
    }
    const fn high(self, v: f32) -> Self {
        Self { high: v, ..self }
    }
    const fn formants(self, v: [Formant; 3]) -> Self {
        Self {
            formants: v,
            ..self
        }
    }
    const fn grit(self, v: f32) -> Self {
        Self { grit: v, ..self }
    }
    const fn swell(self, depth: f32, cycles: f32) -> Self {
        Self {
            swell: depth,
            cycles,
            ..self
        }
    }
    const fn sweep(self, lo: f32, hi: f32, sweeps: f32, mix: f32) -> Self {
        Self {
            sweep: (lo, hi, sweeps, mix),
            ..self
        }
    }
    const fn dust(self, v: f32) -> Self {
        Self { dust: v, ..self }
    }
    const fn chime(self, rate: f32, ring: f32) -> Self {
        Self {
            chime: rate,
            ring,
            ..self
        }
    }
    const fn at(self, pitch: f32) -> Self {
        Self { pitch, ..self }
    }
    const fn partials(self, v: f32) -> Self {
        Self {
            partials: v,
            ..self
        }
    }
    const fn life(self, v: f32) -> Self {
        Self { life: v, ..self }
    }
}

const fn pad(name: &'static str) -> Recipe {
    Recipe::new(name)
}

const fn low(name: &'static str) -> Recipe {
    Recipe::new(name)
        .tilt(2.0)
        .width(2.0)
        .grow(0.5)
        .high(900.0)
        .low(20.0)
}

const fn hiss(name: &'static str) -> Recipe {
    Recipe {
        tonal: false,
        partials: 0.0,
        noise: 1.0,
        low: 60.0,
        high: 16_000.0,
        ..Recipe::new(name)
    }
}

const fn room(name: &'static str) -> Recipe {
    Recipe {
        tonal: false,
        partials: 0.0,
        noise: 1.0,
        slope: -4.0,
        low: 40.0,
        high: 9000.0,
        ..Recipe::new(name)
    }
}

pub const PADS: [Recipe; 48] = [
    pad("Velvet Dusk")
        .tilt(1.4)
        .width(16.0)
        .grow(0.7)
        .high(5000.0)
        .swell(0.15, 3.0),
    pad("Slow Silk")
        .tilt(1.1)
        .width(24.0)
        .high(7000.0)
        .swell(0.1, 2.0),
    pad("Amber Strings")
        .tilt(0.9)
        .width(28.0)
        .grow(0.5)
        .formants(BODY)
        .high(8000.0),
    pad("Cold Cathedral")
        .odd(0.5)
        .tilt(1.2)
        .width(10.0)
        .fifth(0.2)
        .high(6000.0),
    pad("First Light")
        .width(20.0)
        .high(11_000.0)
        .noise(0.08, -2.0),
    pad("Ah Choir")
        .tilt(0.8)
        .width(22.0)
        .formants(AH)
        .noise(0.06, -3.0),
    pad("Oo Choir")
        .tilt(1.2)
        .width(20.0)
        .formants(OO)
        .noise(0.05, -4.0),
    pad("Ee Voices")
        .tilt(0.8)
        .width(18.0)
        .formants(EE)
        .noise(0.05, -2.0),
    pad("Distant Monks")
        .tilt(1.0)
        .width(14.0)
        .formants(OH)
        .sub(0.4)
        .high(3000.0),
    pad("Glass Sea")
        .tilt(0.7)
        .stretch(0.01)
        .width(6.0)
        .low(300.0)
        .high(12_000.0)
        .swell(0.2, 2.0),
    pad("Frozen Bells")
        .tilt(0.9)
        .stretch(0.03)
        .scatter(0.01)
        .width(8.0)
        .high(10_000.0)
        .chime(0.5, 2.0),
    pad("Hollow Wind")
        .odd(1.0)
        .tilt(1.6)
        .width(14.0)
        .noise(0.3, -2.0)
        .high(5000.0),
    pad("Pulse Drift")
        .pulse(0.18)
        .tilt(1.0)
        .width(18.0)
        .high(7000.0),
    pad("Saw Ensemble")
        .tilt(0.95)
        .width(40.0)
        .grow(0.4)
        .high(12_000.0),
    pad("Analog Horizon")
        .tilt(1.05)
        .width(32.0)
        .high(6000.0)
        .grit(0.15),
    pad("Tape Memory")
        .tilt(1.2)
        .width(30.0)
        .high(4500.0)
        .noise(0.05, -1.0)
        .grit(0.2)
        .swell(0.25, 4.0),
    pad("Felt Organ")
        .odd(0.7)
        .tilt(1.5)
        .width(5.0)
        .fifth(0.25)
        .high(5000.0),
    pad("Ghost Organ")
        .odd(0.4)
        .tilt(1.3)
        .width(9.0)
        .fifth(0.15)
        .noise(0.05, -3.0)
        .swell(0.18, 6.0),
    pad("Brass Swell")
        .tilt(0.6)
        .width(18.0)
        .formants(BRASS)
        .high(7000.0)
        .swell(0.3, 2.0),
    pad("Horn Section")
        .tilt(0.8)
        .width(14.0)
        .formants(OH)
        .high(4500.0),
    pad("Low Strings")
        .tilt(1.0)
        .width(26.0)
        .formants(BODY)
        .sub(0.3)
        .high(4000.0),
    pad("Nebula")
        .tilt(1.2)
        .width(70.0)
        .grow(0.2)
        .noise(0.12, -3.0)
        .swell(0.2, 3.0),
    pad("Aurora")
        .tilt(0.8)
        .stretch(0.004)
        .width(50.0)
        .grow(0.3)
        .low(150.0)
        .high(13_000.0)
        .sweep(500.0, 5000.0, 2.0, 0.5),
    pad("Stardust")
        .tilt(0.7)
        .width(12.0)
        .low(200.0)
        .high(14_000.0)
        .chime(1.2, 1.5),
    pad("Deep Ocean")
        .tilt(1.8)
        .width(30.0)
        .sub(0.5)
        .high(1800.0)
        .swell(0.3, 2.0),
    pad("Iron Drone")
        .tilt(0.9)
        .stretch(0.015)
        .scatter(0.006)
        .width(10.0)
        .grit(0.35)
        .high(7000.0),
    pad("Rust Choir")
        .tilt(0.8)
        .width(20.0)
        .formants(AH)
        .grit(0.4)
        .high(6000.0),
    pad("Bowed Metal")
        .stretch(0.02)
        .scatter(0.01)
        .width(9.0)
        .formants(METAL)
        .high(11_000.0),
    pad("Bowed Glass")
        .tilt(0.6)
        .stretch(0.008)
        .width(5.0)
        .low(250.0)
        .high(9000.0)
        .swell(0.15, 3.0),
    pad("Fifth Light").tilt(1.2).width(16.0).fifth(0.6),
    pad("Open Fifths")
        .tilt(1.3)
        .width(12.0)
        .fifth(0.8)
        .sub(0.3)
        .high(6000.0),
    pad("Pale Flute")
        .tilt(2.4)
        .width(8.0)
        .noise(0.25, -1.0)
        .formants([(1500.0, 1.2, 1.0), (0.0, 1.0, 0.0), (0.0, 1.0, 0.0)]),
    pad("Breath Pad")
        .tilt(2.0)
        .width(14.0)
        .noise(0.5, -2.0)
        .high(5000.0)
        .swell(0.3, 3.0),
    pad("Soft Reed").odd(0.8).tilt(1.1).width(10.0).high(3500.0),
    pad("Dusty Harmonium")
        .odd(0.4)
        .tilt(0.9)
        .width(7.0)
        .high(5000.0)
        .grit(0.1)
        .swell(0.1, 9.0),
    pad("Night Vowels")
        .tilt(0.9)
        .width(20.0)
        .formants(EH)
        .swell(0.3, 2.0)
        .sweep(400.0, 1800.0, 1.0, 0.5),
    pad("Silver Mist")
        .tilt(0.6)
        .width(26.0)
        .low(400.0)
        .high(14_000.0)
        .noise(0.1, 0.0),
    pad("Tidal Sweep")
        .width(20.0)
        .swell(0.5, 2.0)
        .sweep(300.0, 4000.0, 1.0, 0.7),
    pad("Trembling Air")
        .width(16.0)
        .swell(0.35, 28.0)
        .high(6000.0),
    pad("Lost Signal")
        .pulse(0.08)
        .width(12.0)
        .scatter(0.004)
        .noise(0.1, 0.0)
        .grit(0.25)
        .high(3000.0),
    // For the big screen: pipe organs, walls of low brass, strings slowed down.
    pad("Temple Organ")
        .odd(0.25)
        .tilt(1.0)
        .width(1.5)
        .grow(0.2)
        .fifth(0.18)
        .sub(0.25)
        .noise(0.02, -2.0)
        .low(40.0)
        .high(7000.0)
        .life(0.08),
    pad("Full Organ")
        .odd(0.15)
        .tilt(0.6)
        .width(2.0)
        .grow(0.2)
        .fifth(0.35)
        .sub(0.35)
        .noise(0.015, -1.0)
        .high(11_000.0)
        .grit(0.12)
        .life(0.08),
    pad("Distant Organ")
        .odd(0.6)
        .tilt(1.9)
        .width(1.2)
        .grow(0.2)
        .fifth(0.1)
        .noise(0.03, -3.0)
        .high(3000.0)
        .life(0.08),
    pad("Organ Reeds")
        .odd(0.5)
        .tilt(0.55)
        .width(2.5)
        .grow(0.2)
        .formants(REED)
        .high(9000.0)
        .swell(0.25, 1.0)
        .life(0.1),
    pad("Low Brass Wall")
        .tilt(0.55)
        .width(12.0)
        .grow(0.35)
        .formants(LOW_BRASS)
        .sub(0.5)
        .grit(0.5)
        .low(30.0)
        .high(4800.0)
        .life(0.5),
    pad("Horn Choir")
        .tilt(0.75)
        .width(16.0)
        .grow(0.4)
        .formants(LOW_BRASS)
        .fifth(0.15)
        .grit(0.12)
        .high(3500.0)
        .life(0.6),
    pad("Tense Strings")
        .tilt(0.9)
        .width(20.0)
        .grow(0.45)
        .formants(BODY)
        .noise(0.03, -1.0)
        .grit(0.05)
        .high(9000.0)
        .swell(0.1, 3.0),
    pad("Slowed Strings")
        .tilt(1.1)
        .width(34.0)
        .grow(0.5)
        .formants(BODY)
        .noise(0.04, -1.0)
        .grit(0.15)
        .high(3800.0)
        .swell(0.3, 2.0),
];

pub const BASSES: [Recipe; 44] = [
    low("Pure Sub").tilt(6.0).width(0.0).high(300.0),
    low("Soft Sub").tilt(4.0).width(1.0).high(400.0),
    low("Warm Triangle").odd(1.0).tilt(2.0).width(1.0),
    low("Round Triangle")
        .odd(0.8)
        .tilt(2.2)
        .width(2.0)
        .high(700.0),
    low("Felt Bass").tilt(2.6).width(3.0).high(500.0),
    low("Deep Cello")
        .tilt(1.2)
        .width(5.0)
        .formants(CHEST)
        .high(2000.0),
    low("Bowed Bass")
        .tilt(1.1)
        .width(7.0)
        .formants(CHEST)
        .noise(0.04, -4.0)
        .high(2400.0),
    low("Low Brass")
        .tilt(0.9)
        .width(4.0)
        .formants([(280.0, 1.0, 1.0), (700.0, 0.8, 0.6), (0.0, 1.0, 0.0)])
        .high(1800.0),
    low("Low Reed").odd(0.9).tilt(1.2).width(3.0).high(1400.0),
    low("Hollow Sub").odd(1.0).tilt(2.8).width(2.0).high(600.0),
    low("Pulse Bass")
        .pulse(0.25)
        .tilt(1.0)
        .width(2.0)
        .high(1200.0),
    low("Saw Shadow").tilt(1.0).width(4.0).high(800.0),
    low("Dark Saw").tilt(1.1).width(6.0).high(450.0),
    low("Tape Bass").tilt(1.5).width(4.0).grit(0.25).high(900.0),
    low("Worn Tape")
        .tilt(1.4)
        .width(8.0)
        .grit(0.35)
        .noise(0.04, -3.0)
        .high(700.0)
        .swell(0.2, 3.0),
    low("Grit Drone")
        .tilt(1.0)
        .width(5.0)
        .grit(0.55)
        .high(1500.0),
    low("Rust Drone")
        .tilt(0.9)
        .stretch(0.004)
        .scatter(0.002)
        .width(4.0)
        .grit(0.5)
        .high(1600.0),
    low("Engine Room")
        .tilt(1.2)
        .width(3.0)
        .grit(0.4)
        .swell(0.3, 24.0)
        .high(1000.0),
    low("Earth Hum")
        .tilt(2.2)
        .width(3.0)
        .fifth(0.2)
        .noise(0.03, -6.0)
        .high(500.0),
    low("Sea Floor")
        .tilt(2.6)
        .width(8.0)
        .noise(0.06, -6.0)
        .high(350.0)
        .swell(0.35, 2.0),
    low("Tidal Sub")
        .tilt(4.0)
        .width(2.0)
        .high(320.0)
        .swell(0.5, 2.0),
    low("Slow Pulse")
        .tilt(2.2)
        .width(2.0)
        .high(600.0)
        .swell(0.6, 4.0),
    low("Breathing Sub")
        .tilt(3.0)
        .width(3.0)
        .high(420.0)
        .swell(0.4, 3.0),
    low("Throb")
        .tilt(1.6)
        .width(2.0)
        .high(800.0)
        .swell(0.7, 16.0),
    low("Organ Pedal")
        .odd(0.6)
        .tilt(1.4)
        .width(1.0)
        .fifth(0.2)
        .high(1200.0),
    low("Cathedral Pedal")
        .odd(0.3)
        .tilt(1.7)
        .width(2.0)
        .fifth(0.35)
        .sub(0.3)
        .high(900.0),
    low("Fifth Pedal")
        .tilt(2.0)
        .width(2.0)
        .fifth(0.7)
        .high(800.0),
    low("Octave Stack")
        .tilt(1.6)
        .width(3.0)
        .sub(0.8)
        .high(900.0),
    low("Sub Choir")
        .tilt(1.0)
        .width(5.0)
        .formants(OO)
        .sub(0.4)
        .high(1600.0),
    low("Monk Low")
        .tilt(0.9)
        .width(4.0)
        .formants(OH)
        .high(1800.0),
    low("Glass Low")
        .tilt(1.0)
        .stretch(0.006)
        .width(2.0)
        .high(3000.0),
    low("Metal Floor")
        .tilt(0.9)
        .stretch(0.012)
        .scatter(0.004)
        .width(3.0)
        .formants(METAL)
        .grit(0.2)
        .high(4000.0),
    low("Bell Low")
        .tilt(1.1)
        .stretch(0.02)
        .scatter(0.006)
        .width(2.0)
        .high(3200.0)
        .swell(0.25, 3.0),
    low("Cave Drone")
        .tilt(1.8)
        .width(10.0)
        .noise(0.05, -5.0)
        .formants(TUNNEL)
        .high(700.0),
    low("Frost Sub")
        .tilt(3.2)
        .width(1.0)
        .noise(0.03, -2.0)
        .high(600.0),
    low("Velvet Low").tilt(1.9).width(9.0).high(650.0),
    low("Amber Low")
        .tilt(1.5)
        .width(6.0)
        .formants(BODY)
        .high(1000.0),
    low("Noise Sub")
        .tilt(4.0)
        .width(3.0)
        .noise(0.2, -6.0)
        .high(450.0),
    low("Thunderhead")
        .tilt(1.6)
        .width(12.0)
        .grit(0.45)
        .noise(0.1, -6.0)
        .high(800.0)
        .swell(0.45, 2.0),
    low("Black Ice")
        .tilt(1.1)
        .stretch(0.003)
        .width(1.0)
        .grit(0.2)
        .high(2200.0),
    low("Braam Floor")
        .tilt(0.7)
        .width(6.0)
        .formants(FLOOR)
        .sub(0.6)
        .grit(0.6)
        .high(1800.0),
    low("Contra Tuba")
        .tilt(0.95)
        .width(4.0)
        .formants(CHEST)
        .grit(0.2)
        .high(1400.0),
    low("Pedal 32")
        .odd(0.3)
        .tilt(1.5)
        .width(0.5)
        .sub(1.0)
        .fifth(0.12)
        .noise(0.01, -4.0)
        .high(900.0)
        .life(0.05),
    low("Pressure Sub")
        .tilt(3.5)
        .width(1.0)
        .grit(0.25)
        .high(420.0)
        .swell(0.25, 2.0),
];

pub const NOISES: [Recipe; 40] = [
    hiss("White Air").noise(1.0, 0.0),
    hiss("Pink Air"),
    hiss("Brown Air").noise(1.0, -6.0).high(6000.0),
    hiss("Tape Hiss")
        .noise(1.0, 1.0)
        .low(800.0)
        .swell(0.05, 7.0),
    hiss("Old Tape")
        .noise(1.0, -1.0)
        .low(300.0)
        .high(9000.0)
        .swell(0.12, 11.0)
        .dust(1.5),
    hiss("Vinyl Crackle")
        .noise(0.25, -2.0)
        .low(500.0)
        .high(11_000.0)
        .dust(12.0),
    hiss("Worn Vinyl")
        .noise(0.35, -3.0)
        .low(200.0)
        .high(7000.0)
        .dust(26.0)
        .swell(0.08, 5.0),
    hiss("Rain on Glass").noise(0.7, 0.0).low(1500.0).dust(90.0),
    hiss("Steady Rain")
        .noise(1.0, -1.0)
        .low(600.0)
        .high(12_000.0)
        .dust(40.0)
        .swell(0.08, 3.0),
    hiss("Distant Storm")
        .noise(1.0, -6.0)
        .high(1800.0)
        .low(25.0)
        .swell(0.55, 2.0)
        .dust(3.0),
    hiss("Soft Wind")
        .noise(1.0, -5.0)
        .high(3500.0)
        .swell(0.45, 3.0)
        .sweep(200.0, 1400.0, 2.0, 0.7),
    hiss("Mountain Wind")
        .noise(1.0, -4.0)
        .formants(HOWL)
        .high(5000.0)
        .swell(0.5, 2.0)
        .sweep(300.0, 2600.0, 3.0, 0.85),
    hiss("Desert Gusts")
        .noise(1.0, -3.0)
        .low(200.0)
        .high(7000.0)
        .swell(0.7, 4.0)
        .sweep(500.0, 3500.0, 4.0, 0.6),
    hiss("Arctic Wind")
        .noise(1.0, -3.0)
        .low(500.0)
        .high(9000.0)
        .swell(0.4, 2.0)
        .sweep(900.0, 5000.0, 2.0, 0.9),
    hiss("Ocean Surf")
        .noise(1.0, -3.0)
        .high(9000.0)
        .swell(0.8, 1.0)
        .sweep(300.0, 6000.0, 1.0, 0.8),
    hiss("Far Shore")
        .noise(1.0, -5.0)
        .high(3000.0)
        .swell(0.75, 1.0)
        .sweep(150.0, 1800.0, 1.0, 0.7),
    hiss("River")
        .noise(1.0, -2.0)
        .low(250.0)
        .high(6000.0)
        .swell(0.12, 13.0)
        .dust(30.0),
    hiss("Waterfall")
        .noise(1.0, -2.0)
        .low(80.0)
        .high(11_000.0)
        .swell(0.06, 9.0),
    hiss("Steam").noise(1.0, 1.0).low(2500.0).swell(0.4, 3.0),
    hiss("Vent")
        .noise(1.0, -4.0)
        .formants(RADIO)
        .high(5000.0)
        .swell(0.15, 2.0),
    hiss("Air Duct")
        .noise(1.0, -5.0)
        .formants(TUNNEL)
        .high(1500.0)
        .low(30.0)
        .swell(0.2, 2.0),
    hiss("Machine Air")
        .noise(1.0, -4.0)
        .formants(CHEST)
        .partials(0.3)
        .at(31.0)
        .tilt(1.0)
        .width(1.0)
        .high(2500.0)
        .swell(0.1, 20.0),
    hiss("Tunnel Rumble")
        .noise(1.0, -7.0)
        .formants(TUNNEL)
        .high(700.0)
        .low(22.0)
        .swell(0.3, 3.0),
    hiss("Subway")
        .noise(1.0, -6.0)
        .formants(TUNNEL)
        .high(1200.0)
        .low(25.0)
        .swell(0.65, 1.0)
        .dust(2.0),
    hiss("Radio Static")
        .noise(1.0, 0.0)
        .formants(RADIO)
        .grit(0.5)
        .swell(0.2, 17.0),
    hiss("Shortwave")
        .noise(0.8, -1.0)
        .formants(RADIO)
        .grit(0.3)
        .sweep(600.0, 3800.0, 3.0, 0.9)
        .dust(4.0),
    hiss("Dead Channel")
        .noise(1.0, 0.0)
        .grit(0.7)
        .low(1000.0)
        .swell(0.15, 23.0),
    hiss("Power Hum")
        .partials(1.0)
        .noise(0.12, -3.0)
        .at(31.0)
        .tilt(1.0)
        .width(0.0)
        .high(1500.0)
        .grit(0.2),
    hiss("Transformer")
        .partials(1.0)
        .noise(0.06, -3.0)
        .at(38.0)
        .odd(0.9)
        .tilt(0.8)
        .width(1.0)
        .high(2500.0)
        .grit(0.4)
        .swell(0.08, 7.0),
    hiss("Geiger").noise(0.06, -2.0).low(1200.0).dust(5.0),
    hiss("Embers")
        .noise(0.2, -5.0)
        .high(4000.0)
        .dust(9.0)
        .swell(0.2, 3.0),
    hiss("Campfire")
        .noise(0.45, -5.0)
        .high(5000.0)
        .dust(22.0)
        .swell(0.3, 5.0),
    hiss("Sand")
        .noise(1.0, 0.0)
        .low(3000.0)
        .swell(0.45, 6.0)
        .dust(60.0),
    hiss("Gravel Hush")
        .noise(0.5, -2.0)
        .low(700.0)
        .high(6000.0)
        .dust(45.0)
        .swell(0.5, 4.0),
    hiss("Breath")
        .noise(1.0, -3.0)
        .formants(AH)
        .high(6000.0)
        .swell(0.85, 2.0),
    hiss("Slow Breath")
        .noise(1.0, -4.0)
        .formants(OH)
        .high(4000.0)
        .swell(0.9, 1.0),
    hiss("Whisper")
        .noise(1.0, -1.0)
        .formants(EE)
        .low(600.0)
        .swell(0.55, 7.0),
    hiss("Forest Hush")
        .noise(1.0, -2.0)
        .low(400.0)
        .high(7000.0)
        .swell(0.22, 3.0)
        .chime(0.4, 0.2)
        .at(72.0),
    hiss("Night Insects")
        .noise(0.4, 0.0)
        .low(3500.0)
        .swell(0.5, 90.0)
        .chime(0.5, 0.15)
        .at(84.0),
    hiss("Cosmic Dust")
        .noise(0.6, -1.0)
        .low(2000.0)
        .high(14_000.0)
        .dust(7.0)
        .chime(0.4, 1.5)
        .at(84.0),
];

pub const AMBIENCES: [Recipe; 44] = [
    room("Cathedral Air")
        .formants(CHEST)
        .noise(1.0, -4.0)
        .swell(0.2, 2.0)
        .chime(0.1, 3.0)
        .at(60.0),
    room("Empty Hall")
        .noise(1.0, -5.0)
        .high(4000.0)
        .swell(0.3, 2.0),
    room("Stone Chamber")
        .formants(TUNNEL)
        .high(2500.0)
        .swell(0.2, 3.0)
        .dust(0.5),
    room("Cave Drips")
        .noise(0.5, -6.0)
        .high(2500.0)
        .chime(1.2, 0.35)
        .at(72.0),
    room("Deep Cave")
        .noise(1.0, -6.0)
        .formants(TUNNEL)
        .high(1400.0)
        .chime(0.3, 0.6)
        .at(60.0),
    room("Ice Cavern")
        .noise(0.4, -2.0)
        .low(1000.0)
        .chime(0.7, 1.8)
        .at(84.0)
        .swell(0.3, 2.0),
    room("Old Station")
        .noise(1.0, -4.0)
        .formants(TUNNEL)
        .partials(0.25)
        .at(36.0)
        .tilt(1.0)
        .width(1.0)
        .swell(0.3, 2.0)
        .dust(1.0),
    room("Machine Hall")
        .noise(0.8, -3.0)
        .partials(0.5)
        .at(29.0)
        .tilt(0.8)
        .width(1.0)
        .grit(0.4)
        .high(5000.0)
        .swell(0.2, 24.0),
    room("Reactor Hum")
        .partials(1.0)
        .noise(0.2, -4.0)
        .at(33.0)
        .odd(0.5)
        .tilt(0.9)
        .width(3.0)
        .grit(0.3)
        .high(3000.0)
        .swell(0.15, 3.0),
    room("Sonar")
        .noise(0.5, -5.0)
        .high(1800.0)
        .chime(0.16, 3.0)
        .at(67.0),
    room("Deep Sea")
        .noise(1.0, -6.0)
        .high(900.0)
        .swell(0.5, 1.0)
        .sweep(120.0, 800.0, 1.0, 0.7),
    room("Submarine")
        .noise(0.8, -5.0)
        .partials(0.4)
        .tilt(1.2)
        .width(2.0)
        .high(1100.0)
        .swell(0.3, 2.0)
        .chime(0.1, 2.0)
        .at(29.0),
    room("Space Station")
        .noise(0.8, -2.0)
        .formants(RADIO)
        .partials(0.3)
        .at(43.0)
        .tilt(1.0)
        .width(2.0)
        .high(5000.0)
        .swell(0.1, 5.0)
        .dust(0.8),
    room("Solar Wind")
        .noise(1.0, -1.0)
        .low(500.0)
        .high(12_000.0)
        .swell(0.5, 2.0)
        .sweep(1000.0, 9000.0, 2.0, 0.8),
    room("Pulsar")
        .noise(0.3, -3.0)
        .partials(1.0)
        .at(48.0)
        .stretch(0.01)
        .tilt(1.0)
        .width(4.0)
        .low(200.0)
        .swell(0.8, 12.0),
    room("Comet Tail")
        .noise(0.7, -1.0)
        .low(800.0)
        .high(13_000.0)
        .swell(0.7, 1.0)
        .sweep(800.0, 8000.0, 1.0, 0.9)
        .chime(0.25, 2.5)
        .at(84.0),
    room("Event Horizon")
        .noise(0.6, -7.0)
        .partials(1.0)
        .at(26.0)
        .stretch(0.012)
        .scatter(0.01)
        .tilt(1.0)
        .width(30.0)
        .high(2500.0)
        .swell(0.5, 1.0),
    pad("Glass Harmonics")
        .tilt(0.4)
        .stretch(0.002)
        .width(4.0)
        .low(900.0)
        .high(13_000.0)
        .swell(0.5, 4.0),
    pad("Wind Chimes")
        .partials(0.1)
        .noise(0.25, -3.0)
        .low(500.0)
        .high(11_000.0)
        .chime(2.2, 1.6)
        .swell(0.3, 2.0),
    pad("Temple Bells")
        .partials(0.25)
        .stretch(0.025)
        .scatter(0.01)
        .width(3.0)
        .noise(0.05, -5.0)
        .chime(0.45, 3.5),
    pad("Distant Gongs")
        .partials(0.7)
        .stretch(0.04)
        .scatter(0.02)
        .tilt(0.9)
        .width(10.0)
        .formants(METAL)
        .swell(0.6, 1.0),
    pad("Singing Bowls")
        .partials(0.8)
        .stretch(0.018)
        .scatter(0.008)
        .tilt(1.4)
        .width(3.0)
        .high(5000.0)
        .swell(0.4, 3.0),
    pad("Metal Rain")
        .partials(0.3)
        .stretch(0.03)
        .scatter(0.02)
        .width(6.0)
        .noise(0.4, 0.0)
        .low(1500.0)
        .dust(40.0)
        .chime(2.0, 0.6),
    pad("Ghost Choir")
        .tilt(0.8)
        .width(40.0)
        .formants(AH)
        .noise(0.1, -3.0)
        .swell(0.55, 2.0),
    pad("Far Voices")
        .tilt(0.9)
        .width(50.0)
        .formants(OH)
        .high(2600.0)
        .swell(0.65, 1.0),
    pad("Phantom Strings")
        .tilt(0.9)
        .width(60.0)
        .grow(0.3)
        .formants(BODY)
        .low(200.0)
        .swell(0.5, 2.0),
    pad("Drone Field")
        .tilt(1.2)
        .width(14.0)
        .fifth(0.5)
        .sub(0.3)
        .noise(0.1, -4.0)
        .high(3500.0)
        .swell(0.2, 2.0),
    pad("Harmonic Mist")
        .tilt(0.5)
        .width(35.0)
        .low(700.0)
        .high(12_000.0)
        .swell(0.3, 3.0)
        .sweep(800.0, 7000.0, 2.0, 0.8),
    pad("Overtone Cloud")
        .tilt(0.4)
        .width(12.0)
        .low(1200.0)
        .high(14_000.0)
        .sweep(1200.0, 9000.0, 3.0, 1.0),
    pad("Granular Haze")
        .tilt(0.9)
        .width(45.0)
        .noise(0.2, -2.0)
        .swell(0.5, 70.0)
        .high(7000.0),
    room("Frozen Lake")
        .noise(0.5, -3.0)
        .low(400.0)
        .high(6000.0)
        .chime(0.22, 2.8)
        .at(60.0)
        .swell(0.3, 1.0),
    room("Night Forest")
        .noise(0.9, -2.0)
        .low(500.0)
        .high(8000.0)
        .swell(0.3, 3.0)
        .chime(0.6, 0.25)
        .at(84.0),
    room("Thunder Far")
        .noise(1.0, -7.0)
        .high(600.0)
        .low(22.0)
        .swell(0.9, 1.0),
    room("Heat Haze")
        .noise(0.8, -1.0)
        .low(1000.0)
        .high(7000.0)
        .swell(0.5, 30.0),
    room("Dust Storm")
        .noise(1.0, -2.0)
        .low(300.0)
        .high(9000.0)
        .swell(0.5, 3.0)
        .sweep(400.0, 5000.0, 2.0, 0.8)
        .dust(25.0),
    room("Underpass")
        .noise(1.0, -5.0)
        .formants(TUNNEL)
        .high(2000.0)
        .swell(0.5, 1.0)
        .dust(1.5),
    room("Elevator Shaft")
        .noise(0.8, -5.0)
        .formants(CHEST)
        .partials(0.25)
        .at(31.0)
        .tilt(1.0)
        .width(2.0)
        .high(1600.0)
        .swell(0.35, 2.0)
        .chime(0.1, 1.2),
    room("Radio Ghosts")
        .noise(0.7, -1.0)
        .formants(RADIO)
        .grit(0.3)
        .sweep(500.0, 3000.0, 2.0, 0.8)
        .chime(0.25, 1.2)
        .at(72.0),
    pad("Last Light")
        .tilt(1.1)
        .width(30.0)
        .low(250.0)
        .high(6000.0)
        .swell(0.45, 1.0)
        .chime(0.2, 2.2),
    room("Slow Machines")
        .noise(0.7, -3.0)
        .formants(METAL)
        .grit(0.25)
        .high(5000.0)
        .swell(0.6, 6.0)
        .chime(0.1, 1.8)
        .at(48.0),
    room("Dust Field")
        .noise(1.0, -2.0)
        .low(300.0)
        .high(7000.0)
        .swell(0.35, 2.0)
        .dust(6.0),
    room("Falling City")
        .noise(1.0, -6.0)
        .formants(TUNNEL)
        .grit(0.4)
        .high(2600.0)
        .low(25.0)
        .swell(0.8, 1.0)
        .dust(2.5),
    room("Deep Space Hush")
        .noise(1.0, -3.0)
        .low(100.0)
        .high(5000.0)
        .swell(0.15, 1.0),
    pad("Time Ripple")
        .tilt(0.6)
        .width(40.0)
        .grow(0.3)
        .low(400.0)
        .high(9000.0)
        .swell(0.7, 2.0)
        .sweep(700.0, 5000.0, 1.0, 0.7),
];

/// One of Shimmer's pings: two operators, and how it rings.
#[derive(Clone, Copy)]
pub struct Ping {
    pub name: &'static str,
    /// The modulator's frequency against the carrier's.
    pub ratio: f32,
    /// How far the modulator bends the carrier, in turns, at the start (it
    /// fades as the ping does: bright first, then pure).
    pub index: f32,
    /// How long it rings against RELEASE.
    pub ring: f32,
    /// The chance of an octave up.
    pub lift: f32,
}

pub const PINGS: [Ping; 16] = [
    Ping {
        name: "Glass Tear",
        ratio: 1.0,
        index: 0.0,
        ring: 1.0,
        lift: 0.3,
    },
    Ping {
        name: "Crystal",
        ratio: 3.0,
        index: 0.25,
        ring: 1.0,
        lift: 0.3,
    },
    Ping {
        name: "Music Box",
        ratio: 5.4,
        index: 0.18,
        ring: 0.5,
        lift: 0.2,
    },
    Ping {
        name: "Bell Dust",
        ratio: 3.51,
        index: 0.4,
        ring: 1.2,
        lift: 0.3,
    },
    Ping {
        name: "Star Field",
        ratio: 1.0,
        index: 0.03,
        ring: 1.6,
        lift: 0.5,
    },
    Ping {
        name: "Thin Ice",
        ratio: 7.01,
        index: 0.12,
        ring: 0.8,
        lift: 0.4,
    },
    Ping {
        name: "Kalimba",
        ratio: 2.0,
        index: 0.3,
        ring: 0.45,
        lift: 0.1,
    },
    Ping {
        name: "Celesta",
        ratio: 4.0,
        index: 0.2,
        ring: 0.7,
        lift: 0.2,
    },
    Ping {
        name: "Tine",
        ratio: 14.0,
        index: 0.1,
        ring: 0.9,
        lift: 0.1,
    },
    Ping {
        name: "Harmonic",
        ratio: 2.0,
        index: 0.08,
        ring: 1.3,
        lift: 0.6,
    },
    Ping {
        name: "Spark",
        ratio: 11.3,
        index: 0.22,
        ring: 0.3,
        lift: 0.5,
    },
    Ping {
        name: "Soft Rain",
        ratio: 1.0,
        index: 0.0,
        ring: 0.25,
        lift: 0.1,
    },
    Ping {
        name: "Organ Pipe",
        ratio: 2.0,
        index: 0.06,
        ring: 0.5,
        lift: 0.0,
    },
    Ping {
        name: "Felt Piano",
        ratio: 1.0,
        index: 0.18,
        ring: 0.9,
        lift: 0.0,
    },
    Ping {
        name: "Clock Tick",
        ratio: 9.3,
        index: 0.5,
        ring: 0.012,
        lift: 0.0,
    },
    Ping {
        name: "Pizzicato",
        ratio: 2.0,
        index: 0.2,
        ring: 0.1,
        lift: 0.0,
    },
];

/// The recipes of a sampled layer (Pad, Bass, Noise, Ambience).
pub fn recipes(layer: usize) -> &'static [Recipe] {
    match layer {
        0 => &PADS,
        1 => &BASSES,
        2 => &NOISES,
        _ => &AMBIENCES,
    }
}
