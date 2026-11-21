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

**Aether**: warm drones and slowly turning pads for ambient music, moved by fractal motion, with glitches
in tune. It plays on its own while the transport plays.

- **Voices.** Five voices hold a chord: a shape (5TH, SUS, MAJ9, MIN9, MIN11, 4THS, LYD) on ROOT. **FREE**
  imposes no chord and no scale, for a plain pad. Each key you play gets a voice of its own and fades in, and
  fades out on its own note when you let it go. With no keys, ROOT sounds alone, in unison on all five voices.
  Each voice is two layers of eight partials, built one by one and DETUNE apart, so every partial can move on
  its own.
  COLOR picks the timbre among four: WARM (a soft flute), SILK (strings), HOLLOW (a reed, odd partials) and
  CHOIR (a vowel). TONE goes from dark to bright.
- **Fractal motion.** Everything moves along one kind of curve: six octaves of smooth noise, each twice as fast
  as the one below and ROUGH times as large, so it looks alike at every time scale: slow swells carry faster
  ripples. EVOLVE sets the slowest period. With **SYNC** it is 1 to 64 bars of the board's tempo, and every
  octave turns on a bar line or a fraction of one; without, it runs from 4 s to 4 minutes. Motion faster than
  0.12 s is left out, as it would tremble. The motion morphs the timbre around COLOR and opens and closes the
  tone (MORPH sets how far). It also drifts each voice's pitch, level and place, and makes the detune breathe.
  SHIMMER ripples each partial's level and tuning, the higher partials faster. Every Start plays the same
  evolution; **RESEED** (by SYNC) picks another.
- **Harmonic glitches.** CHANCE is the chance that a step of the glitch grid (1/4 to 1/32, triplets too)
  glitches. A glitch lands on a harmonic of a chord tone, up to HARM; the sour ones (7th, 11th, 13th–15th) are
  skipped. A *ping* is a glassy sine that starts like a bell and rings pure for DECAY. A *skip* (SKIP sets their
  share) loops a slice of the sound like a CD sticking. The slice is cut to an octave of a harmonic's period, so
  the loop itself is in tune. REPEAT ratchets a glitch into 2, 3, 4, 6 or 8 parts of the step. Half the ratchets
  climb: pings up the harmonic series, skips an octave each time. LEVEL sets how loud they are.
- **Warmth.** DRIVE saturates softly; it is oversampled when partials are high enough to fold back. CHORUS adds
  three swaying copies on each side. SPACE sends the sound into a wide, diffuse space (2.5 to 12.5 s). AIR lifts
  the space's highs and adds a breath of noise through resonances tuned to the chord's upper harmonics. WIDTH
  spreads the voices, and LEVEL sets the output.
- **MIDI.** Keys on the MIDI input (a MIDI cable, or else the keyboards) take over the chord. One key plays the
  shape on it; more keys play themselves, doubled an octave up to fill five voices. In FREE they play exactly
  as pressed, up to five at once (more are thinned evenly). **LATCH** keeps the chord of the last key pressed,
  even as you let go of the keys one by one (the × in the header goes back to ROOT). **GATE** sounds only while
  keys are held. Held keys play even with the transport stopped. **GLIDE** (on the VOICE title line, off by
  default) makes the voices glide to their new notes; off, they step straight there.
- **RANDOM** rolls a new sound: the chord shape, timbre, motion and its seed, the warm stages and the glitches,
  each drawn from the part of its range that stays warm. ROOT, LEVEL, SYNC, GLIDE and the keys' mode stay, and FREE
  stays FREE. Shift-click changes only the timbre and the warm stages.
- **PRESETS** saves the sound under a name, with its chord shape and SYNC, and loads saved ones. Like
  Nullpunkt's, they are kept in the app's data folder, so every project sees them. Seven built-in sounds are
  there to start from: Default, Plain pad (FREE), Glass choir, Deep drone, Hollow fourths, Shards and Night
  air. Loading one keeps your ROOT, LEVEL, GLIDE and keys' mode.

The panel shows the motion. On the left is a fractal flower: a branch per voice forks four times into its
sixteen partials. Each fork is bent by a faster octave of the motion, so the trunks sway slowly and the twigs
quiver. The tips glow with their partials and flare where glitches land. On the right is the aurora: every
partial's track over the last 18 seconds, on a log-frequency scale, with glitches as sparks. Under them, the
timbre's motion over one period around now. The motion is the same every time, so what is coming is drawn
too.

**Larsen**: a feedback drone. A floor of noise excites two plucked strings, one a side, inside a loop that
saturates, so it sounds on its own, with no notes and no input. It is an independent take on the circuit of
Drosera (codeskine) and the Audrey II firmware it grew from, written anew for the board.

- **The loop.** The strings ring, a steep drive bounds them, a LOWPASS and a HIGHPASS and a REVERB (MIX, and
  DECAY up to endless) shape them, and BODY (1–100 ms), the loop's outer delay, brings it all back into the
  strings. FEEDBACK (−60 to +12 dB) is the gain round it: low, the loop dies away; from about −25 dB it
  sustains and saturates. FREQ (notes 16–72) tunes the strings, but the drone settles where their harmonics
  and BODY's resonances meet, which is rarely just that note, and moving BODY can jump it to another partial.
  Everything in the loop changes what sustains, not only the tone. IN feeds the strings (INPUT, 0 dB at the
  middle): with FEEDBACK down it is a resonator for whatever comes in.
- **ECHO** after the loop: SEND, TIME (50 ms to 5 s; it glides like tape when moved) and REPEATS. Every repeat
  is band-passed and soft clipped, so REPEATS can pass 100% and swell without running away. **WARP** quarters
  the time above 50 ms, three to four times shorter: a warp more than a halving. **PONG** bounces the repeats
  from left to right, starting on the left. Its knobs shape them: DRIVE (up to +12 dB into the clip, and each
  bounce comes back louder, so at full the echo runs away from 50%), TONE (the band, 200 Hz to 3.2 kHz), WIDTH
  and GAIN (the repeats' level, not what feeds back).
- **Keys** on the MIDI input (a MIDI cable, or else the keyboards). **PITCH** puts the drone on the key (notes
  16–84, bend ±2). BODY then snaps to a whole number of the note's periods, less what the filters lag there,
  and a band-pass keeps the loop on the fundamental, so it plays in tune (within about 15 cents). The note
  stays after the key; moving FREQ with no key down takes the pitch back. **GATE** opens the loop only while
  a key is held: it swells on the key and dies away after.
- **Three strings.** **POLY** runs three loops. From the knobs they sit on FREQ and two intervals (INTERVAL 2
  and 3, ±24 semitones): with **LOCK** on they are tuned to those notes as keys are, with LOCK off each roams to
  its own resonance. With PITCH, three keys set them: a new note takes a string that has never played, else
  the one let go longest ago, else the one held longest; a string keeps its note after its key is up. One
  key sounds one string. **COUPLING** lets each loop hear the other two, like sympathetic strings.
- **Level.** The output has a limiter and a soft ceiling, since the loop can get loud. A long reverb builds
  the drone up, so the output is trimmed by as much: DECAY and MIX change the sound, not how loud it comes out.
- **PRESETS** saves the settings and the switches under a name. Sixteen built-in sounds: Default, Cellar,
  Whistle, Nave, Drift, Warp, Undertow, Howl, Small room, Keys (PITCH and GATE: play it), Resonator (IN, needs
  an input), Init (silent), Fifths and Sympathetic (three strings from the knobs), Chord keys (three keys,
  coupled) and Embers (PONG). Loading one keeps your LEVEL.

Like every source it plays while the transport runs, or while keys are held. Stopped, the loop closes and dies
away, and once quiet it rests and costs nothing.

The panel shows the loop's field: the output's spectrum on a log-frequency scale, over the band the two filters
let through. The strings' harmonics rise from the bottom and BODY's resonances hang from the top: the drone
settles where they meet. Each string's note floats over its fundamental, glowing with its loop. The knobs
inside the loop wear the node's colour, those after it amber. FREQ and BODY show what really plays: the key's
note (MIDI) and the snapped BODY (SNAP).

**Vesper**: a generative ambient instrument, for soundscapes, cinematic pads and chord progressions. Pick a ROOT
and a mood, and the engine writes the chords and five layers play them. It works the way generative ambient
plugins such as Rast Sound's Cinescapes PRO do, with an engine, sounds and names of its own. There are no sample
files: each sound is a recipe (partials widened into bands, noise, a spectral shape, then swells, sweeps, grit,
dust and chimes) rendered into a looped stereo sample by a thread of the node's own, so playing costs little.

- **The chords.** Sixteen moods, each a seven-degree scale, from Radiant (major) and Elegy (natural minor) to Temple
  (double harmonic) and Cosmos (Lydian augmented). **LOOP** repeats a progression of CHORDS chords, **EVOLVE** writes
  new ones for as long as it plays, and shows what's next. In both, **VARIATION** is how many of the seven degrees
  chords may stand on, **NOTES** how many notes each has (two, three or four) and **DURATION** how long each lasts,
  half a bar to sixteen, on the board's bars. Voices move as little as they can from chord to chord, and a note two
  chords share is held over. The rules can change while it plays: ROOT and mood re-tune the same progression.
- **The layers.** **PAD** plays the chord, **BASS** its root, **AMBIENCE** an atmosphere (tuned ones follow the root),
  **NOISE** a texture, some forty sounds each; **SHIMMER** strikes small high pings on the chord's notes, sixteen
  timbres, at random (PINGS) or as an **OSTINATO**: in time on PULSE's note value, up two octaves of the chord and
  back. Each layer has its sound, LOCK, MUTE and level, and the knobs of the one selected: OCTAVE, ATTACK, **HOLD**
  (a note lets go by itself: a stab; at the top it sustains), RELEASE, **BLOOM** (on each new chord the first filter
  starts below itself and opens as the note comes in, as brass does), WIDTH, two **motion filters** (low, high, band
  or notch, each with FREQ, MOTION and RATE: they move on their own, a little apart on the two sides), DRIVE, CHORUS
  and sends to the ECHO and the SPACE all the layers share.
- **GENERATE** writes new chords on the same sounds. **SHUFFLE** picks new sounds on the layers that aren't locked,
  and leaves the chords. A new sound takes tenths of a second to make; until it's ready the old one plays on, then
  they crossfade.
- **MixType**, over the whole mix: **CLEAN** (neutral), **DREAM** (a slow chorus, softer highs, a longer and brighter
  tail), **DUST** (worn tape: saturation, wow and flutter) and **EDGE** (driven, lifted presence, a touch of bit grit).
- **PLAY.** On, it plays with the transport and keys are ignored; off, the keys on the MIDI input play it (a MIDI
  cable, or else the keyboards): Pad on the keys, Bass and tuned Ambience on the lowest, pings on the keys held.
- **MIDI output.** With PLAY on it sends its score: the chord on channel 1, the bass on 2, Shimmer on 3. Patch it into
  a plugin, or into the MIDI Looper to keep it.
- **PRESETS** saves everything as a scene under a name. Twenty-eight built-in scenes keep your ROOT and LEVEL. The
  last eight are for large-scale film scoring: walls of low brass struck on each chord (Folded City), strings slowed
  down on tape (Limbo), a pulsing ostinato over strings and tuba (Dream Stack), a soft piano-like figure (Last
  Thought), and the pipe organ, huge and steady, with a pulse or a ticking clock over it (Gravity Organ, Full Ascent,
  Dust Field, Tesseract). They sit best at 60–80 BPM.

The panel is a landscape that plays: the Ambience far off, then the Pad, a bank of Noise as fog, the Bass in front,
each ridge rising with its layer and rolling with its filters; the chord's notes hang in the sky as beams, pings come
out as stars, and the moon crosses once for every chord. Under it, the chords by name. Like every source it falls
silent when the transport stops; its notes ring out on their release, and once quiet it rests and costs nothing.

**Random**: a random modulation source. RATE, CHAOS (irregular timing), SMOOTH (from sample & hold to
continuous glide), LEVEL. Patch it into any control input (cutoff, pitch, grain position…).

**Pops 66**: a rhythm box in the spirit of the KORG Mini Pops-7 (1966). No samples: its fifteen instruments
are synthesized the way the original's circuits made them. Ringing resonators (damped sines) make the base
drum, bongos, conga, claves and rimshot; square waves make the cowbell; filtered noise makes the two snares,
hihat, two cymbals and tambourine (with jingles); gated noise bursts make the guiro; and a decaying rattle
makes the quijada.

- **Rhythms.** Twenty rhythms sit in two rows, as on the panel: Waltz, Samba, Mambo, Rumba, Tango,
  Bossanova, Rock 1, Rock 3, March 2/4, Swing on top; Jazz Waltz, Meringue, Cha-Cha, Beguine, Habanera,
  Slow Rock, Rock 2, Rock 4, March 6/8, Latin Swing below. Click a key to play that rhythm alone;
  shift-click or right-click adds it to the ones playing. Combined rhythms are merged on the first one's
  grid, as the original's buttons did. The waltzes, swing and 6/8 rhythms run on triplets.
- **Tempo.** With **SYNC** on, the machine follows the board's tempo and Start/Stop. With SYNC off, **PLAY**
  runs it on its own TEMPO (40–240 BPM). SWING delays the off-beat steps.
- **MIDI.** Notes on the MIDI input play the instruments (General MIDI drum notes: 36 base drum, 38/40
  snares, 42 hihat, 49/51 cymbals…). The guiro scrapes for as long as the key is held. The rhythm also goes
  out as notes on the MIDI output (channel 10), so it can drive another drum machine or a plugin.
- **User patterns.** Eight patterns, P1–P8, of up to 32 steps of sixteenths or triplets. Seven start as
  copies of presets and the eighth is empty. Choose **USER** to play them.

The panel has three detail levels, chosen with its switch:

- **MACHINE**: the rhythm keys and the main knobs: TEMPO, VOLUME, BALANCE (base drum against
  hihat/cymbals), SWING, and the quijada, guiro and tambourine levels, as on the original. Below them are
  the step lights.
- **MIXER**: adds one strip per instrument: a pad that flashes on each hit (click it to hear the
  instrument), LEVEL, PAN and mute.
- **EDITOR**: adds the pattern editor. Set the length, 16ths or triplets, **COPY FROM…** a preset or another
  pattern, and **CLEAR**. Click a step in the grid to toggle it, or drag to paint. A sound panel sets the
  selected instrument's TUNE, ATTACK, DECAY and its own extras: resonance, tick and high-pass for the
  snares; second partial for the conga and cowbell; rate and density for the guiro and quijada.

**Waveforms.** Three more knobs in the sound panel reshape each instrument's waveform:

- **WAVE** morphs the oscillators from sine through triangle to square. The cowbell starts as a square. On
  the noise instruments (hihat, cymbals, tambourine, guiro, quijada), WAVE turns the noise into a metallic
  cluster of six square waves.
- **BEND** sweeps the pitch at the start of the hit: up to two octaves above TUNE (or below), gliding back
  within a fifth of the decay. The filters follow the sweep.
- **DRIVE** saturates the sound, flattening its peaks and lifting its tail.

Beside the knobs, a picture shows the whole hit and a close-up of its first three cycles (4 ms for the noise
instruments). It is drawn again as you turn a knob; click it to hear the sound.

Like any node, it also collapses entirely to its title bar (C).

The preset rhythms are new patterns written in each style, not decoded from the original's diode matrix.

**Sibyl**: a sixteen-voice analog polysynth after the Sequential Prophet-5 (rev 3) and u-he's Repro-5, for
warm, full pads, arpeggios and leads. The panel follows Repro-5's synth view in three rows (VOICE MOD,
OSCILLATOR A, MIXER, FILTER; LFO, OSCILLATOR B, UNISON, FILTER ENVELOPE; WHEEL MOD, MOD MATRIX, AMPLIFIER),
in the board's flat style (a colour per kind of section), with the keys or the effects underneath; its engine,
sounds and names are its own.

- **Voice.** Oscillator A (saw and pulse, hard sync to B) and B (saw, triangle, pulse, LO FREQ seven octaves
  down, keyboard tracking on or off), each with OCTAVE (four positions), FREQUENCY ±12 semitones and a fine
  tune; MIXER levels for A, B and noise (or, with a jumper, the voice's own output fed back); a four-pole
  low pass of the CEM3320 kind (CUTOFF, RESONANCE — it self-oscillates above 60 % and thins the bass as it
  rises —, ENV AMOUNT, KEYBOARD tracking, 100 % = in tune) saturating where its loop meets the input; two
  envelopes with a curved attack (ATTACK 2 ms–15 s, DECAY 2 ms–15 s, SUSTAIN, RELEASE 2 ms–55 s, VEL),
  restarting from where they are on a new note. Oscillators run free and are band-limited (polynomial
  BLEP/BLAMP, one sample late so sync and audio-rate FM are exact).
- **VOICE MOD (poly-mod).** The filter envelope and oscillator B, each with its amount, to oscillator A's
  frequency, A's pulse width and the cutoff: sync sweeps, metallic FM, growls, per voice and at audio rate.
- **LFO and WHEEL MOD.** Saw, triangle and square (they add up), 0.03–27.5 Hz or synced (8 bars to 1/64,
  dotted and triplets); SOURCE MIX from LFO to noise; destinations FREQ A, FREQ B, PW A, PW B, FILTER, under
  the mod wheel between two limits (the triangles beside the wheel: a minimum depth with the wheel down).
- **UNISON, GLIDE, VOICE DETUNE.** VOICES (1–16) are the polyphony; UNISON stacks UNISON VOICES (1–16) on one
  key with LOW, HIGH or LAST priority, spread by VOICE DETUNE; GLIDE slews each voice's pitch (poly or
  unison). VOICE DETUNE also sets how much the voices differ (tuning, cutoff, envelope times, pulse width,
  level) and drift, as real voice cards do; a light per voice shows them work.
- **MOD MATRIX.** Four slots (MM A and MM B, two a page): a source (mod wheel, bend, two controllers,
  LFO, trigger, gate, key, key + bend, velocity, aftertouch, the envelopes, voice index, voice detune), a
  destination (44, from the oscillators, filter, envelopes, mixer and pan to the distortion, the effects
  and the LFO's rate), DEPTH, and modifiers: CURVE, RECT(ify), QUANT(ize to semitones, steps of 12,
  overtones, scales, chords, series of thirds, fifths), S+H (on a rising zero crossing of another signal)
  and SLEW.
- **TWEAKS.** Jumpers: LFO with or without DC, inverted LFO and oscillator B saws, S&H instead of noise in
  WHEEL MOD, note priority, voice reallocation (the same key on the same voice, or round robin), mixer
  feedback, inverted filter envelope, cutoff following the bend. Selectors: the oscillators' model (P5 with
  its inverted pulse, P1, IDEAL, P5 OLD drifting; BOTTOM for B), the filter's (CRISPY, ROUNDED, DRIVEN, POLY:
  darker, only open when modulated), the envelopes' (IDEAL, ANALOG, HIGH SUS, ONE SHOT, PIANO 1 and 2), the
  bend's range up and down (to 24 semitones, then 36 and 48). A pan per voice; the wheels' limits; the two
  controllers' numbers.
- **Effects.** A DISTORTION per voice (soft clip, hard clip, foldback, or CORRODE: rate reduction and bit
  crushing; AMOUNT, TONE, MIX), then five effects in a chain you reorder by dragging and switch by clicking:
  TAPE (saturation at twice the rate, head bump, roll-off, hiss, wow and flutter on WORN), ECHO (a bucket
  brigade: CHORUS, LONG, synced to 1/16 or 1/4 × 1–8; ECHO, PPONG, SWING, GROOVE; modulation; CLEAN, BRIGHT,
  DARK repeats; changing TIME glides the pitch), RES·EQ (low shelf, mid bell and high shelf, or three
  resonant band passes), PLATE (pre-delay, DECAY from 0.35 s through about 2 s in the middle to a minute, TONE, DRY/WET at equal
  power, so the dry sound leads until the middle) and CONDITIONER (GAIN that
  saturates above the middle, TRANSIENT, WIDTH). The EFFECTS tab, under KEYS on the left of the lower panel,
  opens them and shows with a dot each which ones are on.
- **ARPEGGIATOR** (ours): UP, DOWN, UP/DN, UP+DN, ORDER, RANDOM or CHORD, 1/4 to 1/32 with dotted and
  triplet values, one to four octaves, GATE, SWING, LATCH. It steps on the board's grid while the
  transport plays and at the board's tempo from the first key while it is stopped.
- **Programs.** Forty-eight, in banks of eight as on the original (five there, six here), our own sounds
  (arpeggios, pads, leads, keys and basses, textures, and bank 6: bouncing sequences, where each note
  springs — the filter snaps shut and the pitch falls a little — over swung steps and echoes that answer
  from the other side); BANK and PROGRAM recall them, RECORD then a program button stores the panel
  there (kept with the node; right-click a program to put its factory sound back), Program Change 1–48
  picks one. The display shows the sound's name (`*` once edited).
- **Presets.** SAVE asks for a name (24 characters) and a category (ARP, PAD, LEAD, KEYS, BASS, TEXTURE,
  BOUNCE, OTHER) and keeps the sound in your library, in the app's preset folder (`presets/Sibyl.json`), there
  in every project (a preset of the same name is replaced); or stores it, with its name and effects order, in
  the current program. PRESETS ▾ lists your sounds by category (click to load, × to delete), the factory
  banks, and exports the sound or the whole library to a file, or imports one (a file with one sound loads it
  too). A preset keeps the sound only: never the master section (TUNE, OUTPUT, HQ, A-440, the mod wheel,
  VOICES, the controllers) nor the programs recorded in the node. The two-digit display reads bank and
  program; `*` marks an edited panel. HQ runs the voices at twice the rate; A-440 replaces the output with a
  reference tone; TUNE (±12 semitones) and OUTPUT stay with the panel.
- **Keys.** The MIDI input (or the keyboards while it is free), the panel's five octaves (keys about six times
  as long as wide) and its bend and mod wheels; mod wheel CC 1, sustain CC 64, aftertouch, bend.

Performance: the slow parts of a voice (envelopes, glide, the matrix, pitches, levels) are worked out once
every 16 frames (or at each MIDI event) and ramped; the oscillators run per voice, the filter four voices
at a time in lanes; idle voices cost nothing and the node rests when silent.

**Fjord**: a four-channel drum synthesizer after the Clavia Nord Drum (the first one, 2012), with a step
sequencer. Each channel plays one sound at a time, blended from three components, each with its own
parameters, as on the Nord Drum:

- **Tone**, the body of the sound: a waveform (An1–3 analog: sine, triangle, sawtooth; b1–9 drum-like:
  kick, tom, snare shell, timpani, tabla, conga, electronic FM tom, steel drum, log drum, made from the modes
  of membranes and bars; c1–5 percussive: bell, cowbell, six-square metal, FM gong, free bar), at a pitch
  in half semitones shown as MIDI note numbers (11.0–98.5; 69.0 is A 440 Hz), through a low-pass filter.
- **Noise**: white noise tilted by COLOR (below 0 more bass, above 0 less), through a low-pass filter of
  seven types, each more resonant (at 6 and 7 the noise whistles at the cutoff).
- **Click**: a sharp transient at the very start: n1–n9 noise bursts in nine bands, c1–c9 the same with an
  added attack, t1–t9 pitched ticks.

| Section | Parameters (red ones are the second function of a button on the Nord Drum) |
|---------|------------|
| NOISE   | COLOR −10…10 · FILTER 0–99 / TYPE 1–7 · SWEEP 0–10 / TIME 0–9 · DECAY 0–50 / GATE n, S, G |
| TONE    | WAVE · FILTER 0–99 / PUNCH 0–9 · SWEEP 0–10 / TIME 0–9 · DECAY 0–50 / GATE · PITCH · BEND −9…30 / BEND TIME 0–9 |
| CLICK   | TYPE n, c, t 1–9 · DECAY 0–20 / GATE |
| MIXER   | TONE, NOISE, CLICK 0–10 · LEVEL 0–99 |
| AMP ENV (Fjord's own) | DECAY or ADSR · A 0–50 · D 0–50 · S 0–99 · R 0–50 |
| FILTER ENV (Fjord's own) | AMOUNT −10…10 · A 0–50 · D 0–50 · S 0–99 · R 0–50 |

- **Decay and Gate.** DECAY runs from 8 ms to 6 s (the click's from 1 ms to 250 ms), as the time to fall
  60 dB. GATE shapes it: **n** decays gradually, **S** holds with a soft knee and then falls, **G** is gated:
  held for a third of the decay, sagging a little, then cut.
- **Sweep and Time.** SWEEP opens the filter at the strike, up to 7.5 octaves, and falls back over TIME. Each
  TIME is a share of its decay: 9 lasts as long as the decay, so a sound keeps its shape when the decay
  changes.
- **Bend.** The pitch starts away from PITCH and glides to it over BEND TIME (a share of the tone's decay): a
  positive BEND starts above (a step per semitone, up to 30), a negative one below.
- **Punch** adds a short, velocity-controlled attack: a boost of a few milliseconds with its peaks rounded.
- **Velocity** sets the level and how far the sweeps, the bend and the punch go.
- **FILTER** 99 is fully open; lower settings close it to 20 Hz.
- **The tone's envelopes** go beyond the Nord Drum, to widen what the tone can be (swells, pads, basses,
  plucks). **AMP ENV** picks what shapes the tone's level: **DECAY**, the Nord Drum's DECAY and GATE, or an
  **ADSR** (attack 0.5 ms to 5 s, decay and release 8 ms to 6 s, sustain level). With the ADSR the tone's
  DECAY and GATE stand aside, and the sweep, bend and partials' times follow the ADSR's decay. **FILTER ENV**
  is an ADSR on the tone filter, on top of the sweep: AMOUNT moves the cutoff up to 8 octaves up, or down when
  negative (it can close a fully open filter), scaled by velocity; at 0 it is off. Both are drawn beside their
  knobs. The sustain lasts as long as the note: a MIDI key until its note-off, a step for its GATE, TRIG
  while it is held down, a Trig input for 80 ms. The display marks where its strike lets go (0.4 s).

**Programs.** 99 programs hold the four channels. P1–P34 are factory programs (our own sounds, in the Nord
Drum's styles: Real, Retro, Ethno, FX; and categories: *Drums* for kick, snare and two toms, *Perc* for four
pads, *Kit* for kick, snare, tom and hihat; the last two, Slow Bloom and Acid Kit, use the tone's
envelopes); P35–P99 start empty. The knobs are the edit buffer. The programs
show as a bank map: 99 cells, outlined in their style's colour (white for your own, faint when empty), the
current one filled. Hovering a cell shows its name, a click loads it, and dragging the program number up or
down steps through them, like the Nord Drum's dial. A red dot shows when the knobs differ from the stored
program. **STORE**, then a click on a cell, writes them there (Esc cancels), unless **PROTECT** (Memory
Protect, on from the factory) is on. Double-clicking a knob gives back its stored value. **COPY** and **PASTE** move a
channel's sound to another channel, program or Fjord; **INIT** (Sound Init) resets the edited channel. MASTER
sets the overall level and is not stored. The programs are kept with the node, in the project and in the
library.

**The panel.** The top is always there: the programs, the channel keys, SOLO EDIT, MASTER and ALEA. Under
it, two pages, and the node takes the size of the one shown: **SOUND**, the edited channel's sound (the
display, NOISE, TONE, CLICK, MIXER and the tone's envelopes), and **SEQUENCER · MIDI**, the step lanes and
the SYSTEM settings below.

**Playing.** The channel keys pick the channel to edit; their lights flash green on a hit and red on a hard
one (longer for the hardest, as the Nord Drum's input lights do). **TRIG** plays the edited channel at its
velocity (VEL), and holds its note while it is held down. **SOLO EDIT** lets only the edited channel's tone,
noise or click be heard. The display draws the edited channel's strike: tone, noise and click as outlines,
the bend's pitch over them, and a line that runs through on each hit.

**ALEA: throwing the dice.** *Alea* is Latin for the die and for chance and fortune; it gave aleatoric music,
music left partly to chance, its name. Three throws: **SOUND** (new sounds for the four channels; with Shift,
for the edited channel only), **STEPS** (new steps) and **ALL** (new sounds, then steps for them). **CHANCE**
sets how far a throw goes: a little, and it nudges what is there (each amount moves that share of the way
to the new sound, each choice changes with that chance, each step is redrawn with that chance); at 100% it
starts afresh. The die shows each throw's face. **BACK** takes back the last throw. Sounds are bred, not drawn
blind: each section of a channel (noise, tone, click, mixer, the envelopes) comes from the same channel of a
factory program, so a kit's channel 1 stays a kick, the sections crossed between programs, then jittered,
some choices (waveform, filter type, click type, gates) thrown anew, and now and then the tone's ADSRs
switched on. Steps follow what each channel sounds like, shown under its lane: a kick starts every bar and
lands on some beats and offbeats, a snare takes the backbeats with ghost notes, a hihat runs on eighths,
sixteenths or the offbeats with accents, a tom or percussion plays a Euclidean rhythm with a fill now and
then. A throw changes the knobs only: STORE keeps what you like.

**SYSTEM: MIDI and the Trig inputs.**

- **MIDI CH** (1–16, or 0: Off) is the channel received and sent; **LRN** takes it from the next message.
- **NOTE**: each channel's MIDI note, 36, 38, 50 and 47 to start (General MIDI bass drum, snare, high tom,
  low-mid tom); **LRN** takes it from the next note. Its note-off ends a held note (for the envelopes).
- Hits from TRIG, the Trig inputs and the steps go out as notes on the MIDI output, as the pads' hits do on
  the Nord Drum. **LOCAL OFF** makes them only go out (for a sequencer that echoes them back); MIDI notes in
  still play.
- **PROG CHANGE**: OFF, SEND (when a program is picked here), RECV (program changes load programs) or BOTH.
- **TRIG 1–4** are audio inputs that play the channels: a pad through the audio interface, or any
  percussive signal (drum replacement: a drum loop into TRIG 1 replays its kicks on channel 1). For the
  edited channel: **TRIG TYPE** (Pd1 rubber pads, Pd2 plastic heads, Pd3 mesh, Ac1 trigger on a kick or
  snare, Ac2 on a high or mid tom, Ac3 on a low tom: how long it scans for the peak and how long it ignores the
  ringing), **DYN** (Dn1 linear, Dn2 hotter; it shapes the MIDI notes' velocity too), **THRES** (hits quieter
  than this are ignored, up to 48 dB under the hardest) and **SENS** (the input level that plays the hardest
  hit, 0 to −45 dBFS).
- **FOLLOW PADS**: MIDI notes and Trig input hits pick the channel to edit, until one is picked on the panel.

**STEPS: the sequencer.** A lane per channel, up to 32 steps of 1/8, 1/16, 1/16T or 1/32 on the board's grid,
playing with the board's transport while **RUN** is on. Click a step to switch it on or off, right-click (or
shift-click) to step it through soft, medium and accent, drag to paint. SWING delays every other step, GATE sets
how long a step's note is held (5–100% of the step: the envelopes' sustain, and the MIDI notes sent), M mutes
a lane, CLEAR empties the pattern. It starts with a beat for the first program's kit.

**Outputs.** OUT carries the mix (mono, on both sides, as the Nord Drum's single output); CH 1–4 carry each
channel on its own, after its level, for separate effects.

**What differs from the Nord Drum.** The manual describes the waveforms, the click types and the filter types
only in words, so these are our own designs, as are the factory programs (34 here, 79 on the Nord Drum). The
manual lists the third default note as 59, but the General MIDI high tom it names is 50, which Fjord uses. The
panel shows every parameter at once instead of a grid of buttons, a dial and Shift, and the programs as a map
instead of a number on a display. The Trig inputs are audio
inputs, and their types are timings rather than settings for particular pads. Program dumps over SysEx, the
OS update and the demo are left out: programs are saved with the node (save it to the library to carry its
bank to other projects), and the sequencer's starting beat plays the part of the demo. Local stays as set
instead of returning to On; the tone's envelopes, the channel outputs, the steps and ALEA are new.

**Splitter**: a sample slicer and granular glitch engine on the board's grid, for fractured, minimal, very digital
music in the spirit of Alva Noto's live sets: sine tones and pings, clicks, data noise, stuttering micro-loops,
frozen grains, sub pulses, strict grids. Load a sound, cut it into up to 32 slices, give each its own pitch,
stutter, crusher and grains, and let the animation play them in time, or play them from MIDI.

- **Sources.** **LOAD** takes a WAV or AIFF file (8 to 32-bit integer or float, mono or stereo; more channels:
  the first two), up to 30 s. The sample is kept **inside the project** (and the library), as 16-bit PCM, so a
  project carries its sounds; **EJECT** lets it go. With nothing loaded, Splitter still plays: eight built-in
  glitch **atoms** stand in, each a second long: **SINE** (880 Hz at pitch 0), **PING** (a sine dying in 200 ms),
  **CLICK** (an impulse with a short ring), **SUB** (a 50 Hz pulse falling onto its pitch), **NOISE**
  (sample-and-hold noise at 8× the pitch: hiss to data bursts), **DATA** (random bits in packets: modem and
  fax chatter), **PULSE** (a narrow pulse wave) and **CRACK** (sparse impulses: crackle and dust). It starts
  with a kit of eight slices of them on Euclidean rhythms, so it plays as soon as the board does.
- **Cutting.** The display shows the selected slice's source, the sample or the atom it reads, with every
  slice of that source outlined in its own colour, the playheads and the grains moving over them; a strip
  above shows the whole source and the part shown (**VIEW**: ALL, or zoomed on the selected **SLICE**). Drag
  across empty space to cut a new slice, drag a slice's edge to move it, drag inside the selected slice to move
  it whole; a click selects, a double-click plays, a right-click deletes. **CUT** does it in one go: an equal
  grid (**÷4**, **÷8**, **÷16**, **÷32**), at the **TRANSIENT**s (**SENS**: how readily; cuts land on the zero
  crossing before each strike), or **RANDOM** fragments of 8 to 400 ms. A freshly loaded sample with no slices
  of its own is cut at its transients. The keys 1–32 under the display pick the slice to edit (and flash when
  it plays); **+** adds a copy of the selected slice, **DEL** deletes it, **CLEAR** deletes them all.

The **SLICE** page edits the selected slice:

| Section | Settings |
|---------|----------|
| SOURCE  | SAMPLE or an atom · **ONE** (plays once, through its release), **GATE** (while held: a key, a step's GATE, PLAY), **LOOP** (loops while held: micro-loops) · **REV** · **M** (left out of the animation) · ▶ **PLAY** · COPY, PASTE, ↺ |
| SOUND   | PITCH ±48 st · GAIN −40…+12 dB · PAN · ATTACK 0–2 s · RELEASE 1 ms – 4 s · FROM / TO (the region, to the millisecond) |
| STUTTER | REPEAT 1×–16× · RATE 1/8 … 1/128 (on the grid) · FALL (each repeat quieter) · RAMP ±12 st (each repeat higher or lower) |
| CRUSH   | BITS 16 … 2 · DOWN (each value held for up to 64 samples) · CRUSH ALL (the node's: every slice further) |
| GRAINS  | GRAIN · FREEZE · SYNC · SIZE 2–500 ms · DENSITY 1–400 a second · POS · SPRAY · JITTER ±12 st · SPREAD (the node's) |

- A sample slice's pitch is its speed, as on a sampler (an octave up plays twice as fast); an atom keeps its
  timing and changes its frequency. The node's **PITCH** transposes every slice.
- **RELEASE** is the fade after a key or a step's GATE, and on a ONE slice the fade before its end (at most
  its second half). The display under the knobs draws the envelope over the slice's length.
- **REPEAT** turns each strike into a ratchet: that many strikes at RATE of the board's tempo, each restarting
  the slice. A note-off takes a GATE or LOOP slice's remaining repeats with it.
- **GRAIN** plays the slice as a cloud of Hann-windowed grains read at its pitch around a playhead that moves
  through the slice in real time (so the pitch no longer changes its length), from **POS**; **FREEZE** stops
  the playhead at POS. **SPRAY** scatters where grains start, **JITTER** their pitch, the node's **SPREAD**
  their place in the stereo field (and every hit's). **SYNC** puts the grains on the board's grid, DENSITY
  snapped to the nearest note value (1/4 to 1/256, triplets included). A ONE slice's cloud lasts as long as
  the slice would; a GATE or LOOP slice's, as long as it is held.
- The crusher comes last: the hold, then the bits.

The **SEQUENCE** page: while **RUN** is on and the board plays, steps of 1/2 to 1/64 (**RATE**, with
**SWING**) on the board's grid play the slices, in one of six modes:

- **ORDER**: the next slice each step (muted ones skipped).
- **RANDOM**: a slice at random (never the same twice running, unless LOCK is on).
- **PATTERN**: the lane's slice on each step, or a rest. Click a step to put the selected slice there (again:
  a rest), right-click for a rest, drag to paint; **IN ORDER** fills it with the slices in turn.
- **PROB**: every step, each slice rolls its own dice (its bar, dragged up or down).
- **EUCLID**: each slice plays its own Euclidean rhythm (**HITS** over **STEPS**, turned by **ROTATE**), drawn
  as a ring of its steps with its hits joined. Each counts on its own length, so rhythms of 7, 11 or 13 steps
  drift against the bar. Drag a ring up or down for more or fewer hits.
- **STUTTER**: the lane's slices (or the slices in order, with an empty lane) rolled into ratchets of 2 to 12
  strikes at 1/32 to 1/128, rising or falling now and then, and now and then the last slice held instead.

**LENGTH** (1–32 steps) wraps the lane. **FILL** is the chance a step (or a slice's hit) plays at all, in every
mode. **CHAOS** varies hits now and then: reversed, an octave or a fifth away, skipped into, softer, or
doubled into a short ratchet (in STUTTER it also makes the rolls longer and the holds more frequent).
**LOCK** makes the random choices (RANDOM, PROB, STUTTER, CHAOS) repeat every LENGTH steps; **NEW** draws
others. **GATE** (5–100% of a step) sets how long a step holds a GATE or LOOP slice.

**DICE** throws new settings: **SLICES** (pitches by pure intervals, reversals, ratchets, crushing, grain
clouds, and a sample slice's region shifted or stretched now and then; with Shift, the selected slice only),
**RHYTHM** (each slice's Euclidean rhythm and chance, and the lane) or **ALL**. **CHANCE** sets how far a
throw goes: a little nudges what is there, 100% starts afresh. Throws follow what each slice sounds like
(shown under its number: low, hit, tone, noise or texture, from the atom, or from a sample slice's length
and how often it crosses zero): lows land on the strong beats, hits on busy Euclidean patterns, tones sparse
and on odd lengths so they drift, noise on the offbeats, textures rarely and often as grain clouds. **BACK**
takes back the last throw (or CLEAR, DEL, PASTE, ↺, a right-click delete).

The **MIDI** page: **CH** (OMNI, or 1–16) is the channel heard and sent on. Each slice can have a note: click
a key on the keyboard (C1–C7) to give it to the selected slice (again: take it back), or **LEARN** (or a
slice's **LRN**) to take the next note played; **MAP FROM C2** maps the slices to notes from C2 up, **UNMAP**
clears them (a freshly cut sample's slices start from C2). A note plays every slice mapped to it, at its
velocity; its note-off ends a GATE or LOOP slice. **CHROMA** makes the notes no slice is mapped to play the
selected slice in pitch (C4, note 60: as it is). With **MIDI OUT**, the steps' hits go out as notes (each slice's
note, or C2 up by slice), held for the step's GATE, to drive other nodes. Sixteen voices play at once; a
slice struck again cuts its last strike short in 2 ms.

**What differs from Nullpunkt.** Nullpunkt synthesizes four fixed micro-voices on step lanes and works on
its whole output (stutter, skip, warp, static); Splitter cuts sounds of your own into up to 32 slices, each
with its own pitch, envelope, ratchet, crusher and grains, plays them from six kinds of animation or from
MIDI note by note, and its built-in sources are a second of each atom, read like a sample (backwards, in
grains, looped). Splitter's sample lives in the project: a project with a 30 s stereo sample grows by about
7 MB.

**Audio In**: the audio interface's inputs on a cable: a microphone, a guitar, a synth, another app through
a loopback device. **INPUT** picks the channel. **STEREO** takes it and the next as left and right (INPUT 3:
inputs 3–4); **MONO** puts it on both sides. **GAIN** goes from −24 to +24 dB. The display has a meter per
side, with marks at −18 and −6 dBFS. Put one Audio In per source, each on its own inputs. The input device is
chosen in **Settings** (see Audio settings) and opens only while an Audio In node is on the board. The first
time, macOS asks for permission to use the microphone; until it's granted, the input is silent.

**Nullpunkt**: an instrument for clicks, pulses and glitches, in the manner of Alva Noto. A tone to lay under it
can come from an Oscillator or a Drone.

**Four micro-voices**, each with its own lane of steps:

- **CLICK**: impulses through a short resonance. COLOR sets its pitch; DECAY sets how long it rings.
- **SINE**: high sine blips.
- **SUB**: sub-bass sine pulses.
- **DATA**: 1-bit digital noise, held at the GRAIN rate.

Each voice has **HITS** and **LEN**, **PROB** (the chance a step that hits actually plays), TONE, DECAY,
LEVEL and **M** (mute).

- HITS and LEN lay down a Euclidean pattern: HITS spread as evenly as possible over LEN steps.
- Each voice has its own length, so the patterns phase against each other.
- Click a step to toggle it, drag to paint, Shift-drag sideways to rotate the lane.
- Turning HITS or LEN lays the Euclidean pattern down again, over your edits.

**The grid** runs on the board's clock, while the board plays. Its division can be 1/4, 1/8, 1/8T, 1/16,
1/16T, 1/32, 1/32T or 1/64.

**STUTTER** repeats grains of the sound, always in time:

- **CHANCE**: how likely a step is to start a stutter.
- **LENGTH**: the most steps it runs on.
- **GRAIN**: how fine its grains may get. Each grain is a subdivision of the step (1/2, 1/3, 1/4, 1/6, 1/8,
  1/12, 1/16, 1/24 or 1/32), so the repeats stay on the board's tempo, triplets included.
- **SCATTER**: gives some grains another shape (an octave up or down, backwards, or taken from a grain before
  the stutter) and lets longer stutters start afresh on a new step.

**PLAY** shapes how the patterns are performed:

- **MUTATE**: flips steps now and then and adds ratchets (quick repeats inside a step).
- **JITTER**: varies each hit's level, tone and decay.
- **WIDTH**: scatters hits across the stereo field.
- **LEVEL**: the output level.

**GLITCH** works on everything:

- **SKIP**: a step sticks on a slice of what just played, like a CD skipping.
- **REVERSE**: a step plays the one before it backwards.
- **WARP**: a step loops a slice an octave up or down.
- **CUT**: a step drops to digital silence.
- **STATIC**: bursts of digital errors.
- **CRUSH**: fewer bits, from 16 down to 2.
- **RATE**: holds samples, from every sample down to 1 in 48.

The header has the division and two buttons:

- **RANDOMIZE** rolls new patterns and sounds for the voices that aren't muted (Shift-click: patterns only).
- **PRESETS** saves the current scene under a name and loads saved ones. Presets are kept in the app's data
  folder, so every project sees them. Five built-in scenes are there to start from: Default, Raster, Pulse,
  Static and Broken.

The glitches light up in the header as they happen. The output shows as a barcode of recent peaks.

### Texture

**Granular**: records its input into an 8-second loop and plays it back as a cloud of grains.
POS (how far back to read), SIZE, DENSITY (grains per second), PITCH (semitones), JITTER (randomness of
position and timing), SPRAY (random pitch), SPREAD (stereo), MIX. Direction FWD / REV / BOTH.
**FREEZE** stops recording: the cloud keeps exploring the captured sound. The POS input moves the read
position (e.g. with a Random or an LFO). The display shows the loop and the active grains.

**Frakture**: a granular rhythm effect after Excite Audio's Motion: Fractal. Its panel has four pages.

- **EFFECTS**: four effects in a chain. Drag their names in the ROUTING row to change the order.
  - **Grains** records its input into a buffer locked to the tempo. BUFFER sets its length, from 1/4 to
    4 bars. RETRIG starts it over when the transport starts; FREEZE stops recording. A LO–HI band filters
    the grains. Two grain layers (1, 2, each with its own ON switch) play from the buffer:
    - PLAYHEAD has five modes: MANual position, SCAN (moves through the buffer forwards or backwards),
      and an offset behind the recording, as a note value (SYNC), in ms or in % (a granular delay).
      SCAN with **SYNC** is locked to the transport: −2× … 2× passes of the buffer per BUFFER length, so
      the playhead is always in the same place on the same beat.
    - RANGE (spread around the playhead), DENSITY and SIZE (free, or note values with SYNC).
    - CHAOS (randomizes width, direction and shape), DIRECTION (−100% reversed … +100% forwards).
    - PITCH and SPREAD, snapped by the pitch mode: free, chromatic, octaves, fourths, fifths, 4ths & 5ths.
    - WIDTH (how likely grains are panned out), SHAPE (from a percussive decay through a bell and a
      trapezoid to a square gate), DRY and WET.
  - **Gate** is a trance gate of up to 32 steps (click to switch them), with RATE, ATTACK, RELEASE and MIX.
  - **Repeat** is a stutter. Once per TRIGGER (OFFSET into it) it takes a slice RATE long (a note value
    with SYNC, else ms) and plays it AMOUNT more times. DUCK lowers the dry signal meanwhile; it has a
    LO–HI band and a VOLUME.
  - **Space** is a hall reverb with LENGTH and MIX; with **SYNC** the LENGTH is a note value (1/4 … 4/1).
  - VIEW shows displays: the grains over the buffer (position across, pitch ±36 up and down) with both
    layers' grain shapes, the gate's levels before and after, and the slice being repeated.
- **MUTATE**: pushes the grains further, past a granular delay, into textures that keep changing. It
  works on the layers switched on with LAYER 1 / LAYER 2, and **DICE** rolls new mutations.
  - **FEEDBACK**: the grains are written back into the buffer, so they regenerate. Each pass is a little
    darker and softly clipped; at the top the texture lives on after the input stops.
  - **SWARM**: a cloud of up to 8 grains per trigger, scattered a few milliseconds apart and slightly
    detuned.
  - **GLIDE**: each grain bends its pitch as it plays, up to ±24 semitones, each by its own amount; with
    CHAOS some bend the other way.
  - **SHIMMER**: the chance a grain jumps up an octave, a twelfth or two octaves.
  - **RESONATE**: each grain rings through a band-pass on a harmonic of **KEY** (1st to 8th, following
    the grain's pitch), so noise and drums turn into chords on KEY.
  - **BLUR**: four allpasses per channel melt the grains into a wash.
  - **ERODE**: the chance a grain is worn down: its samples held and rounded to a few bits.
  - A live cloud shows the grains: across, where they are in the buffer; up, their pitch now. A tail
    shows how far a grain has glided, a halo BLUR, a ring a resonating grain, a square an eroded one, a
    + a shimmering one. FEEDBACK's echoes ripple out from the buffer.
  - **SYNC** under FEEDBACK, SWARM and GLIDE puts them in time, with a note value each:
    - FEEDBACK: the grains come back that much later, as echoes in time (1/16 … 1/1).
    - SWARM: a cloud's grains start one after another, a note value apart, on the same slice: rolls and
      ratchets (1/64 … 1/8).
    - GLIDE: the bend takes the note value, however long the grain (1/16 … 4/1).
  - Every MUTATE knob can be mapped to the hexagon (right-click it), like the other effect knobs. They
    start with a moderate amount of feedback, swarm, shimmer, resonance and blur.
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

**Magma**: four hot, analog-flavoured filters after FabFilter Volcano 3. It has three inputs: IN, SC (side
chain) and MIDI.

- **Filters.** Each filter has:
  - a shape: low pass, high pass, band pass, bell, low shelf, high shelf, notch or all pass;
  - a slope: 6, 12, 24 or 48 dB/octave (band pass and notch start at 12);
  - one of eleven non-linear **styles**: Classic, Smooth, Raw, Hard, Hollow, Extreme, Gentle, Tube, Metal,
    Easy Going and Clean. Each drives the input through its own curve and saturates the resonance with its
    own shape, and most of them self-oscillate at the top of PEAK.
- **Knobs per filter.** DRIVE; FREQ with F.PAN (the left channel's cutoff goes down and the right's up,
  by up to ±2 octaves); PEAK (resonance, or ±24 dB of gain for bells and shelves); DELAY (up to 50 ms on
  the filter's output, for combs, flanging and Haas effects); LEVEL and PAN.
- **Routing.** Six routings: serial, parallel, two serial pairs in parallel, two parallel pairs in
  series, and the two mixes. ST runs every filter on both channels; L/R and M/S put filters 1–2 on the left
  (or mid) and 3–4 on the right (or side).
- **The display.**
  - It shows each filter's curve and the total curve in yellow. In L/R or M/S, or with a frequency pan,
    it shows one curve per channel; while modulation moves things, a thin curve shows the live result.
  - Optional: the output's spectrum and a piano along the scale.
  - Drag a dot for frequency and peak. Shift: fine; Alt: one axis; Cmd: frequency pan sideways and slope
    up/down.
  - Alt-click a dot: on/off. Cmd+Alt-click: next shape. Alt+Shift-click: next slope.
  - Double-click the background for a new filter: high pass at the far left, low pass at the far right,
    notch along the bottom, bell elsewhere.
- **The bar.** HQ (2× oversampling against aliasing), AUTO MUTE (stops self-oscillation once the input
  is silent), ANALYZER, PIANO, BYPASS (crossfaded), IN and OUT with their pans (±36 dB), and MIX.
- **Modulation.** Almost any knob can be modulated, the bar's and the sources' own included.
  - There are two each of:
    - **XLFO**: up to 16 steps with value, random, curve and glide; free (0.02–500 Hz) or synced (16
      bars to 1/64) with an offset multiplier; BALANCE, GLIDE and PHASE; restart on MIDI notes (or legato
      only); SNAP turns steps into semitones over two octaves.
    - **EG**: DAHDSR with curves for attack, decay and release, triggered by the input, the SC input or
      MIDI notes. It has a threshold with a level meter, a NEUTRAL range (0 while sustaining) and
      AUDITION to hear the side chain.
    - **EF**: attack and release; ENVELOPE, or TRANSIENT to catch drum hits.
    - **MIDI**: velocity, note, pitch bend, mod wheel, aftertouch or any controller, with six response
      curves. It reads the MIDI input, or the keyboards when nothing is patched there.
    - **XY**: a pad, or a slider, bipolar or unipolar.
  - Drag a source's ◯ handle from its tab onto any knob, or right-click a knob → Modulate with.
  - Up to 16 slots set source, target, level (±100% of the target's range), invert and remove. Slot
    levels are knobs too, so a slot can modulate another slot.
  - A modulated knob shows a ring to its live value and a dot in its sources' colours.

**Equalizer**: a dynamic EQ after Waves F6 Floating-Band Dynamic EQ, with zero latency.

- **Bands.** Six floating bands sit between a high-pass and a low-pass filter. Each band is a low shelf,
  bell or high shelf: FREQ 16 Hz–21 kHz, Q 0.4–60, GAIN ±18 dB. Each band and filter works on the
  stereo signal (ST), the MID only or the SIDE only. HP and LP have their own frequency and a slope of
  6, 12, 18 or 24 dB/octave.
- **Dynamics.** When the level a band's side chain sees goes above THRESHOLD (−60…0 dB), the band's gain
  moves by up to RANGE (±18 dB: negative compresses, positive expands), with ATTACK (0.5–500 ms) and
  RELEASE (5 ms–5 s, longer the harder it works).
  - The side chain is the node's own input (INT) or the **SC** input (EXT). SPLIT filters it around the
    band; WIDE uses the full range. The ring around THRESHOLD shows its level.
  - SOLO plays a band's side chain, or what HP/LP let through.
  - GLOBAL RELEASE: MNL uses each band's own release; ARC shortens it on transients.
  - The ▲▼ arrows under GAIN, RANGE, THRESHOLD, ATTACK and RELEASE move that control on every band at
    once.
- **Graph.** It shows each band's curve (live, with its dynamics, and dashed where RANGE can take it)
  and the total curve.
  - Drag a marker to set frequency and gain (HP/LP sideways only). Alt+drag sideways sets Q, Ctrl+drag
    keeps to one axis. Cmd+click changes the band type; double-click turns it on or off.
  - Right-click and hold a marker to solo it; moving the marker then solos other frequencies.
  - The band selectors (HP, 1–6, LP) select a band too; double-click one to turn it on or off.
- **MIX and OUT.** MIX (0–100) blends in the dry signal for parallel processing; OUT is ±18 dB, with an
  output meter.
- **Analyzer (RTA).** The toolbar above the graph sets:
  - FILL or BARS, PEAK or RMS, PINK (equal energy per octave) or WHITE slope;
  - banding from 1/24 octave to 1 octave, and averaging (fast to slowest);
  - an MA (moving average) line and a MAX line;
  - the source (L, R, L+R, sides) and the listening point (PRE, POST, SC or OFF);
  - RTA on or off.

  Hovering the graph shows the frequency, the note with its cents and the analyzer's level there.

**Prism**: a 24-band equalizer after FabFilter Pro-Q 4, with dynamic and spectral bands, three processing
modes and an interactive display. It is a node of its own: the Equalizer above stays as it is.

- **Bands.** Up to 24, added on the display. Each has a shape, a slope, FREQ (10 Hz–30 kHz), GAIN (±30 dB), Q
  (0.025–40) and a stereo placement. The shapes:
  - **Bell**: Q is the width at half the gain;
  - **Low Shelf**, **High Shelf**: the gain below or above, half of it at FREQ;
  - **Low Cut**, **High Cut**: Butterworth, 3 dB down at FREQ; Q above 1 adds a resonant peak at the knee;
  - **Notch**, **Band Pass**: they cut out, or keep, a band Q wide;
  - **Tilt Shelf**: a shelf centred on 0 dB, −GAIN/2 below FREQ and +GAIN/2 above;
  - **Flat Tilt**: a straight line across the spectrum through 0 dB at FREQ, GAIN from 20 Hz to 20 kHz;
  - **All Pass**: changes only the phase, as an alternative to inverting it.
- **Slopes.** 6, 12, 18, 24, 30, 36, 48, 72 and 96 dB/octave; the cuts also have **BW**, a brickwall (100 dB down
  within about 0.4 octave). Bells and notches go 12 to 96 in steps of 12: steeper, a bell's top gets flatter and
  its sides steeper, at the same width. Q does nothing at 6 dB/octave (bells, notches and band passes aside).
  **GAIN-Q** (bottom bar) makes bells narrow as they gain, and gain a touch when narrow, as analog ones do.
- **Placement.** **ST** works on both channels, **L** or **R** on one, **M** on the mid (L+R) and **S** on the side
  (L−R). **SPLIT** turns a band into a left and a right band (or mid and side) to set apart. Curves for one channel
  only are drawn in its colour: white left, red right, green mid, blue side; yellow is what every channel gets.
- **Processing modes** (bottom bar):
  - **ZERO LATENCY**: analog-matched filters. A bell high up keeps its analog shape instead of being squeezed
    against Nyquist (within half a dB up to 15 kHz at 44.1 kHz, and a little more above, as in Pro-Q).
  - **NATURAL PHASE**: the same filters at twice the rate, where they match the analog ones in phase too, up to
    the top of the audible range. Latency: 30.5 samples (0.6 ms), from the oversampling filters.
  - **LINEAR PHASE**: only the magnitude changes, phase stays untouched (for parallel chains, or bands on mid and
    side). **RESOLUTION** picks the frame: finer resolves narrow low bands better, at more latency. At 48 kHz
    (frames scale with the rate, so the times stay): LOW 53 ms, MED 107 ms, HIGH 213 ms, V.HIGH 427 ms, MAX
    853 ms (where steep low bands can pre-echo). The board doesn't compensate latency, so mixing it with the dry
    signal elsewhere combs; the latency is shown beside RESOLUTION.
- **Dynamic EQ.** Bells, shelves and tilts can be dynamic: **RANGE** (±30 dB) is how far the gain may move when
  the band's trigger crosses **THRESH**. Below 0 it turns loud moments down (compression), above 0 it lifts them
  (expansion).
  - **A** (on by default) is the automatic threshold: it rides 3 dB over the trigger's average level, rising in
    about half a second and falling over two, so the band reacts to what stands out. Off, THRESH sets it
    (−60…0 dB); the ring round the knob shows the trigger's level.
  - **ATTACK** and **RELEASE** at 50% are automatic: they follow the band's frequency (slower low down, where a
    period lasts longer), and the release lengthens as the band works harder. Lower is faster, higher slower,
    down to a tenth or up to ten times.
  - The trigger: **INT** (the node's input) or **EXT** (the SC input); **BAND** (the band's own range: a band pass
    for bells, a low or high pass for shelves; tilts listen to everything) or **FREE**, between **SC LO** and
    **SC HI**. Hold **AUDITION** to hear it.
  - On the display, the dashed curve is where RANGE can take the band, the yellow bar from its dot is where the
    dynamics have it now, and the diamond at the end of the line drags RANGE. The GAIN knob's ring shows the same.
- **Spectral dynamics.** **SPECTRAL** makes a dynamic band measure each frequency in its range on its own: only the
  frequencies that cross the threshold move, the rest of the band stays. **DENSITY** sets how many neighbours
  trigger together (low: wide ranges; high: narrow, single frequencies). **TILT** (on by default) adds 3 dB/octave
  to the trigger, so highs trigger a little sooner, which suits whole mixes. Spectral bands run in the linear-phase
  stage at RESOLUTION LOW to HIGH, so they bring its latency in any mode. The dotted line on the display is what
  they are doing along the spectrum.
- **The display.**
  - Hovering shows the band a click would make; its place picks the shape: cuts at the far left and right, shelves
    just inside them, a notch along the bottom, a bell elsewhere. Click to add it (with bands selected: double-click
    or Cmd-click). **Alt**-click makes it dynamic, **Alt+Shift**-click spectral. Dragging the yellow curve also makes
    a band (shelves near the ends).
  - Drag a dot for frequency and gain (cuts, notches, band and all passes: frequency and Q, whose dots sit at a
    height for Q). **Shift**: fine. **Alt**: one axis. **Cmd**-drag up and down: Q. With **PIANO** on, frequencies
    snap to semitones (Shift lets them go).
  - Click a dot to select it, **Cmd**-click to add or remove one, **Shift**-click for every band between, or drag a
    rectangle on the background. Dragging moves every selected band; the strip's knobs move them all too. Click the
    background to deselect.
  - **Alt**-click a dot: bypass. **Cmd+Alt**-click: next shape. **Alt+Shift**-click: next slope.
  - A tag by the dot under the pointer gives its shape and values; elsewhere, the frequency and note under the
    pointer. The EQ scale is ±3, 6, 12 or 30 dB (top right), and grows by itself when a band is dragged past it.
- **The band strip** (under the display) shows the selected band: bypass, the ten shapes drawn as their curves, the
  slopes the shape has, placement, SPLIT, previous and next band (by frequency), **S** (hold to solo: you hear what
  the band works on, the band of a bell, notch or band pass, the side of a shelf, what a cut removes; drag sideways
  for the frequency, with Cmd for Q) and × (delete).
- **The analyzer** (top bar): **PRE** and **POST** spectra, and the side chain's (**SC**, red); **RANGE** (60, 90,
  120 dB), **RES** (1024 to 8192 points: finer lows, slower), **SPEED**, **TILT** (0–6 dB/octave around 1 kHz;
  4.5 is close to how loud we hear), **FREEZE** (holds the peaks). With **GRAB** on, resting the pointer on the
  spectrum for a second freezes it and marks its six highest peaks: drag one to make a bell there, its width taken
  from the peak.
- **EQ Match** (**MATCH**): bands that make the input sound like a reference, which is the SC input or, with
  **CAPTURE**, what the input has played so far (then play the part to change). The panel averages both spectra and
  draws the difference in red; **DETAIL** sets how many bands (2–16); **MATCH** fits them (again as often as you
  like), **FINISH** keeps them, **CANCEL** puts the bands back as they were.
- **EQ Sketch** (**SKETCH**): draw a curve across the display in one stroke (moving back redraws), and bands follow it.
- **Output** (bottom bar): **CHARACTER** (CLEAN; SUBTLE, a mild saturation; WARM, a stronger one with even
  harmonics), **AUTO GAIN** (makes up the level the curve adds or takes, an estimate from the curve, shown beside
  it), **Ø** (polarity), **SCALE** (every gain and range, 0–200%), **PAN** (between left and right, or mid and
  side) and **OUT** (±36 dB). The meter right of the display shows the output and its peak. As on every node, a
  soft limit keeps the output under full scale (it does nothing below −1 dBFS).

What differs from Pro-Q 4: the slopes are fixed steps, not fractions in between. The board's mouse wheel moves the
canvas, so Q is Cmd-drag or the knob instead of the wheel; values can't be typed; there's no right-click band menu
(the strip has it all), no Delete key for bands (Del deletes the node; × in the strip does), no horizontal zoom, no
undo, A/B or presets of its own (save the node to the library). Pro-Q never clips; Prism's output is soft-limited
near full scale like the other nodes'. Linear Phase works on overlapping FFT frames, and
Natural Phase oversamples: our own ways to the same ends. In Linear Phase, dynamic bands hear their trigger ahead of
the delayed sound, and at V.HIGH and MAX dynamics are off, as in Pro-Q. The automatic threshold, attack, release and
knee (soft, 6 dB) are our own, as are the spectral detector, the saturation curves and the readings of Tilt Shelf
and Flat Tilt. EQ Match listens only while its panel is open, and can't take another instance or an audio file as
reference. The instance list, surround and Atmos, collision detection, MIDI learn (the board's MAP does it) and the
solo's listening level are left out.

**Tape**: a tape machine, after the classic tape emulation plugins. The sound is recorded, carried by a moving
tape and played back:

- **Record.** The record EQ lifts the highs into the tape, so loud highs saturate first. The magnetic curve
  runs on 2× oversampling: soft saturation with a touch of even harmonics. Loud swings lose some top, as
  real tape does.
- **The tape.** Wow, flutter (capstan, pinch roller and scrape), and wear's dropouts.
- **Playback.** The playback EQ undoes the record EQ. Then come the head bump (a lift in the lows), its dip,
  the low rolloff, gap loss (the top end) and hiss.

The switches:

- **MACHINE**: STUDIO (a warm 2-track, round head bump, soft saturation), MASTER (a clean mastering deck,
  tighter lows, more open top, steadier) or CASSETTE (1⅞ ips: narrow, hissy and wobbly).
- **SPEED** (reel machines): 7½, 15 or 30 ips. Faster is brighter, with the head bump higher and the
  flutter faster.
- **TAPE**: CLASSIC (little headroom, early warm saturation), MODERN or HOT (most headroom).
- **EQ**: NAB (fuller lows) or IEC/CCIR.
- **AUTO GAIN**: OUTPUT follows INPUT, so driving the tape doesn't make it louder.

The knobs:

- **INPUT**: record level, −12 to +18 dB.
- **SATURATION**: how hard the signal goes into the tape's curve, at the same level. It starts at 35%, so
  the tape is heard: about 3–4% distortion on a −12 dBFS signal, 2nd and 3rd harmonics alike. Turning it
  up pushes the curve up to 21 dB harder and adds even-harmonic warmth. Quiet passages come up a little,
  loud ones are rounded and compressed (tape's "glue"), and makeup gain keeps typical levels within a
  few dB. At 0 the tape is clean.
- **BIAS**: under is brighter and grittier, saturating sooner; over is cleaner and darker.
- **WOW** and **FLUTTER**: shown as the pitch deviation they cause.
- **HISS**.
- **AGE**: worn tape, with dropouts and a duller top.
- **OUTPUT** and **MIX**.

The deck is only sketched, and runs with the sound:

- Two reels turn at their real ratio as the tape winds from one to the other.
- The tape line sags and sways with wow and flutter, and fades in a dropout.
- The head under it glows with the signal, and turns orange when the tape squeezes the peaks.
- Two needles on bare arcs are the VU meters for input and output (0 VU = −12 dBFS RMS).
- The **■** and **▶** keys stop the tape: it slows to a halt with its pitch falling, and spins back up when
  restarted.

**COMPACT / FULL** in the header picks the view. COMPACT has the deck and INPUT, SATURATION, WOW, FLUTTER and MIX;
FULL adds the switches, every knob and the tape time.

The dry signal is lined up with the tape's latency, so MIX doesn't comb.

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

### MIDI sequences: Step Sequencer, MIDIcer, Automa, Colloid, Orrery, MIDI Looper, Arpeggiator, MIDI In

Besides audio cables there are **MIDI cables**: square ports (audio ports are round) and dashed cables.
A MIDI cable only joins a MIDI output to a MIDI input. Plugins that accept MIDI (VST3 or Audio Unit
instruments), the Arpeggiator, MIDIcer, Colloid, Orrery, the MIDI Looper and MIDI Note have a MIDI input.

**Start / Stop** (toolbar, or Space) drive the transport, not the audio: Stop ends the sound's generation
but lets it die away. Sequencers, arpeggiators, MIDIcer, Automa, Colloid and Orrery stop and end the notes they were holding (VST
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

**External MIDI sync.** In **Settings → Sync** the transport can follow a **MIDI clock** from a chosen MIDI
input instead of keeping its own time (a drum machine, a DAW, a hardware sequencer):
- the tempo follows the clock's 24 ticks a beat, smoothed over the last beat of ticks (a late or early tick
  barely moves it), and a jump to a new tempo is caught within two ticks;
- **Start** plays from the top, **Stop** stops and holds the position, **Continue** plays on, and the **song
  position** moves to the clock's place. The board's beat is steered onto the clock's (nudged ahead or back,
  jumping only when far out), one audio buffer ahead, so what you hear lands on the clock's beat;
- while it follows, the toolbar shows the clock's tempo next to **SYNC** (click either for the Sync settings),
  the BPM field and TAP can't be changed, and Start/Stop and Space say that the clock drives the transport,
  whose button shows EXT. If the clock stops ticking, the transport stops. Back to Internal, the board stops
  and keeps its own tempo again (the project's tempo is never changed by the clock).

The board can also **send its clock** to MIDI outputs (Settings → Sync → Send clock, one switch per output):
24 ticks a beat at the board's tempo (also while stopped, so a device can show the tempo), **Start** when the
board starts, **Stop**, and the song position plus **Continue** when it starts elsewhere. Ticks are timed to
leave when their beat is heard, one audio buffer after it is rendered. Following a clock and sending one
relays it; the clock source's own output is skipped, so the clock can't go round in a loop.

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

**Automa**: a generative melody node. A generator offers notes on every step of the global grid, and a
harmony layer fits each one to the chord under it, so the lines stay consonant whatever the generator does:
the first step of each chord always lands on one of its tones, and colour notes come only on weaker steps,
as far as TENSION allows. The panel shows the generator's world on the left and, on the right, a roll of
the notes with the melody's line, over lanes for each chord's tones (the coming chords too, behind a veil),
coloured by function: blue tonic, green subdominant, orange dominant.
- Generators (the selector at the top):
  - **LIFE**: a cellular automaton on a 16 × 16 torus, read column by column like a sequencer; each row is
    a scale degree, the lowest at the bottom. RULE picks Conway's Game of Life or a relative (HighLife,
    Day & Night, Amoeba, Maze, Seeds); GEN is how many steps each generation lasts (16: once a sweep). Click
    a cell to bring it to life or kill it, drag to paint; **KEEP** makes the grid as it is LIFE's start
    (drawn while stopped, it is the start already), **CLEAR** kills every cell.
  - **CASCADE**: an elementary (one-dimensional) automaton, Wolfram's rules 90, 30, 110, 150, 45, 73, 105
    and 22. Each generation is a row of sixteen degrees (low on the left), and the generations pour down the
    display; rule 90 draws Sierpiński's triangle.
  - **TREE**: Per Nørgård's infinity series, a melody that holds itself at every scale: every second note
    plays it upside down, every fourth plays it again. It is drawn as a binary tree, its branches coloured
    by their term, with the path to the term playing lit. STRIDE reads every so many terms (2 inverts the
    melody, 4 repeats it, odd strides make new ones), INTERVAL is the degrees per step of the series.
  - **JULIA**: the orbit of a point under z → z² + c, its angle round the fixed point the pitch. SHAPE
    places c round the edge of the Mandelbrot set's main cardioid, and so the angle the orbit turns by: the
    figure the melody traces. CHAOS takes c inside the cardioid (the orbit spirals in, a phrase that
    settles), onto its edge (figures that circle) or past it (escapes: rests). MORPH lets c drift with the
    beat. The display shows the Julia set with the orbit, and the Mandelbrot set in a corner with c on it.
- **MUTATE** keeps the generators going: LIFE gets cells born now and then, and a new shape when its world
  dies or settles into a loop; CASCADE has cells flipped; TREE jumps to other branches; JULIA gets noise
  on its orbit. **PHRASE** starts the generator again every so many steps, on the grid (∞: never).
- *Rhythm*: RATE (1/4 to 1/32, triplets), DENSITY (how many steps play; the strong ones more often), SWING,
  GATE.
- *Melody*: **LEAP** (low: the line moves to the nearest offered note; high: it may jump anywhere. TREE and
  JULIA: how wide a range they play in), **TENSION** (at 0 only chord tones; higher, 9ths, 11ths and 13ths
  on the off-beats, then on the beats), **VOICES** (up to four parts: the melody, then harmony from the other
  cells that offered, as chord tones), OCTAVE (of the bottom row).
- *Harmony*: KEY, SCALE (major, minor, dorian, phrygian, lydian, mixolydian, harmonic and melodic minor),
  CHANGE (each chord lasts half a bar to four bars). The progression is the strip under the display, up to
  eight chords: drag a chord up or down to change it, click one past the end to lengthen the progression;
  **CHORDS ▾** holds the presets (Pop, Axis, Doo-wop, Jazz turnaround, Circle, Canon, Royal road, Epic,
  Andalusian, Modal vamp, Floating, Drone) and the length. **7THS** adds the seventh to the chords, **PAD**
  plays them too, under the melody's range and voiced by inversion, and **WANDER** lets a chord give way to
  another of the same function after the first time round (a "~" marks it).
- **FREEZE** loops what is playing: LIFE stops evolving and its grid keeps sounding (drawing on it still
  works), the others repeat their last bar (PHRASE steps, if set). The loop is chosen from as it was but
  fitted to the chords as they change. **RESEED** gives every generator a new start and LIFE a random grid.
  **PLAY** pauses it while the transport runs.
- Everything follows the transport: Start plays from the start state, so the same settings play the same
  music again. VELOCITY, DYNAMICS (how much the beat and the generator's energy accent the notes) and the
  output CHANNEL. Settings, progression, LIFE's start grid and the seed are saved in the project and in
  library modules.

**Colloid**: a generative MIDI node played by physics. A handpan's notes, or a chord's, become molecules floating
in a liquid; while the transport plays, atoms are let into the tank, fall towards the molecules and strike them, and
every strike plays the struck molecule's note. The panel is the tank itself, drawn flat and graphic like a diagram
that moves: the liquid in flat bands of depth, a matrix of dots drawn towards the molecules as far as they pull, a
dashed ring turning round each molecule for its well. Molecules are line art, coloured outlines only, bonded into a
shape by their role (the root a tetrahedron, the third a triangle, the fifth a line, the seventh a star, any other
note a benzene ring), coloured by role (amber, pink, cyan, lavender, mint), tumbling with an electron on its orbit
and the note written in the core (under the molecule when it is small); a note brightens and thickens its lines; nearby molecules are linked by flowing dashes into clusters that keep shifting. While a molecule rings,
rings of sound spread from it. Atoms are dots trailing a wake of smaller dots, pale when slow, pink when fast, ringed
in amber while charged. A strike draws a crisp ring and a burst of ticks; a note flashes its molecule and floats its
name up in a tag. New notes sweep a line through the tank. **Click** in the tank to push the liquid outwards,
**drag** to stir it. Under the tank, a cell per molecule, low to high, fills while its note rings.
- *The notes*: **HANDPAN** or **CHORD** at the top, then the shape. The handpans (the default) are tuned like the
  instruments: a ding, the low note, under its tone fields, every note in tune with every other, so whatever the
  atoms strike the line stays a melody: **KURD** (the classic minor, D | A Bb C D E F G A C on D), **CELTIC** (minor
  without the sixth), **AMARA** (Celtic without the fourth, open), **INTEGRAL** (the first Hang's sound), **HIJAZ**
  (Middle Eastern, the raised seventh), **PYGMY** (a bright minor pentatonic), **AKEBONO** (Japanese pentatonic),
  **SABYE** (sunny major without the fourth) and **AEGEAN** (dreamy major, the raised fourth). The root is the row of
  keys under the selector, C to B (click one; it can be mapped to a MIDI controller like a knob): the ding, with
  **OCTAVE** its octave; the molecules take the layout's notes from the ding up (with more molecules than notes, the
  layout again an octave up). The chords are MAJ, MIN, SUS2, SUS4, MAJ7, MIN7, DOM7, MAJ9, MIN9, 6/9, QUART (fourths)
  and PENTA, on the same root: **COLOR** adds colour tones (9ths, 11ths, 13ths; never a half step above a chord tone),
  **SPREAD** is how many octaves the molecules cover (both are for chords: dimmed while a handpan plays), the notes
  evenly spaced from the root up, every tone in turn. Low notes make big, heavy molecules that float low; high ones are
  small, light and float high. When the notes change, each molecule moves to the nearest new one.
- *MIDI control*: on its MIDI input (its cable or, with none, the MIDI keyboard) a single key moves the root (the ding;
  the shape stays; the row of keys rings the root they play), two or more keys played together become the chord
  itself, whatever it is. The notes from the keys stay until **× KEYS** in the tank, or a click on the row of keys,
  lets go of them. Program Change 1–21 picks the shape: 1–9 the handpans,
  10–21 the chords.
- *PARTICLES*: **MOLECULES** (2–16), **ATOMS** (1–48, let in from the edges on Start), **MASS** (heavier atoms strike
  harder).
- *FIELD*: **ATTRACT** (how strongly the molecules pull: low, atoms drift by; high, they orbit and fall in),
  **RANGE** (how far the pull reaches), **CHARGE** (after a strike the atom is pushed off that molecule for a while,
  then falls back: bouncing rhythms; at 0 atoms cling to a molecule and rattle it with quick, soft notes) and
  **BOUNCE** (the speed a strike gives back; past 85% strikes add energy).
- *LIQUID*: **VISCOSITY** (slows the atoms and damps the rings), **HEAT** (Brownian kicks), **FLOW** (swirling
  currents) and **DRIFT** (how much the molecules wander and gather).
- *NOTES*: the physics plays them. The **velocity** follows the strike's force (the atom's speed and mass) as far as
  **DYNAMICS** says, up to **VELOCITY**. The **length** is how long the molecule rings: the harder the strike the
  longer, longer in big (low) molecules, shorter in a thick liquid; **LENGTH** is how long the hardest strike rings.
  A molecule plays at most every 45 ms; a new strike while it rings strikes the note again. **QUANT** holds notes
  back to the next step of the board's grid (1/4 to 1/32, triplets), or plays them as they strike (FREE).
  **CHANNEL** is the output channel, for the notes and the CC lanes.
- *CC EVENTS*: four lanes, each sending a controller with the notes. **ON** switches a lane, the controller menu
  picks its number (Mod wheel, Breath, Foot, Volume, Pan, Expression, Sustain, Sostenuto, Soft pedal, Resonance,
  Cutoff, Reverb, Chorus, or any number 0–127), the source menu what drives it, and the two boxes the values it sends
  for the source's lowest and highest (drag them; the low one above the high one turns it upside down); the bar shows
  the value going out. Sources: **PEDAL** (a sustain pedal: down while the atoms play, lifted for a moment at each new
  chord so the old one doesn't blur into it, let go on Stop), **KEYS** (the same controller from the MIDI input,
  passed through: your sustain pedal, mod wheel or expression pedal), and what the tank is doing: **ENERGY** (how hard
  the molecules ring), **DENSITY** (how many notes play), **SPEED** (how fast the atoms go), **STRIKE** (each note's
  force, falling away after it), **SWIRL** (which way and how fast the atoms circle the tank) and **CHARGE** (how
  charged the atoms are). The tank's sources send while the atoms play, at most every 10 ms and only on a change. The
  lanes start off, set up as Sustain from PEDAL, Mod wheel from ENERGY, Expression from DENSITY and Cutoff from SPEED.
- **PLAY** lets the atoms in while the transport plays; stopped or paused, they leave and the molecules keep floating.
  Start lets them in again from the seed (**RESEED** picks a new one, scattering the molecules too). The settings, the
  CC lanes and the seed are saved in the project and in library modules.

**Orrery**: a Euclidean rhythm generator drawn as concentric rings, like the orbits of an orrery (the clockwork model
of the planets). Each of its eight rings is a rhythm of its own with a note on a MIDI channel of its own, so one node can
play a whole drum kit, or eight instruments at once. A Euclidean rhythm spreads PULSES hits over STEPS steps as evenly
as they go (the rhythms of Bjorklund's algorithm, after Toussaint's *The Euclidean Algorithm Generates Traditional
Musical Rhythms*): 3 over 8 is the Cuban tresillo, 5 over 8 the cinquillo, 2 over 16 turned by four steps the backbeat.
It works in the manner of Euclidean sequencers such as Vitling's Euclidean Circles, Olympia Noise Co's Patterning and
Torso's T-1, with a panel of its own.

- **The rings.** A row per ring, from the outside in: **ON**, **STEPS** (1–32), **PULSES** (0 to STEPS), **ROT**
  (how far the ring is turned, in steps), **NOTE** (with the General MIDI drum's name, for channel 10), **CH** (the MIDI
  channel, 1–16) and its pattern laid out flat. Drag a number up or down; double-click resets it. STEPS and PULSES lay
  the Euclidean pattern again, over your edits (a step that stays a hit keeps its accent).
- **Every step by hand.** Click a step on its ring, or in the selected ring's strip under the table, to make it a hit
  or a rest; right-click (or Shift-click) accents it. In the strip, drag to paint. **EUCLID** lays the Euclidean pattern
  again, **INVERT** swaps hits and rests, **CLEAR** rests every step, **DICE** throws new pulses and a new turn for the
  ring.
- **Turning the rings.** A ring's hits come ROT steps later: turning one ring against the others shifts its rhythm
  against theirs (the snare's 2 over 16 turned by four is the backbeat; turned one more, it pushes). Drag a ring round on
  the orbits to turn it, by whole steps, or with **Shift** freely: half a step turned delays every hit by half a step, for
  flams and lazy grooves. **◀ ▶** turn the selected ring a step, **0** brings it back. **TWIST** turns every ring
  further than the one outside it (ring 2 by TWIST steps, ring 3 by twice as much…, ±2 steps, any part of a step): the
  rings fall out of phase together, the inner ones the most. A notch on each ring marks its first step, so its turn
  shows.
- **GRID or CYCLE.** **GRID**: every ring steps on the board's grid (**RATE**, 1/4 to 1/32, triplets; **SWING** delays
  every other step), so a ring of 5 steps comes round sooner than one of 16 and they drift against each other
  (polymeter). **CYCLE**: every ring goes round once in **CYCLE** beats (1 beat to 8 bars), its steps sharing them out
  evenly, so 3 steps play against 4 and 5 (polyrhythm).
- **Notes.** The selected ring's **VELOCITY**, **GATE** (how long its notes last, as a part of its step) and **CHANCE**
  (the chance each hit plays). **ACCENT** is how much louder accented steps play, from the ring's velocity up to 127.
  A ring holds one note at a time: a new hit ends the one before. **DICE** at the top throws new pulses and turns for
  every ring that plays (steps, notes and channels stay); Shift-click it for the selected ring only.
- **Patterns.** Eight buttons under the header each hold a pattern: everything but the rings' notes and channels (which
  rings play, their steps, pulses, turns, hits and accents, velocity, gate and chance, and GRID or CYCLE, RATE, CYCLE,
  SWING, ACCENT and TWIST), so recalling one changes the rhythm and leaves the kit as it is. Click a button to recall
  it; it comes in as **LAUNCH** says: **NOW**, on the next **BEAT**, or on the next **BAR** (in CYCLE, the next turn),
  so a change made mid-bar still lands in time, while the button blinks. Shift-click a button, or press **STORE** and
  then click it, to keep the rings there. The lit button is the one recalled last; a dot after its name says the rings
  have changed since, so STORE again to keep the changes. Each button shows its pattern as a little orrery (the rings
  that play, their hits as dots), its name and GRID or CYCLE. Right-click a button to name it (up to twelve
  characters), recall it at once, clear it, or fill it from the library. On the MIDI input (its cable or, with none,
  the keyboards), **Program Change 1–8** (any channel) recalls buttons 1–8, as LAUNCH says.
- **The library.** **PATTERNS ▾** saves the rings as a pattern under a name, in the app's data folder (`presets/Orrery
  patterns.json`), so every project sees it. Click a pattern's name there to load it into the rings (as LAUNCH says, no
  button lit), or one of its **1–8** to put it on that button. Eight patterns are built in, and are on the buttons to
  start with: Groove, Four floor (kick on every beat, offbeat hats, clap on 2 and 4), Tresillo (3 over 8 in the kick,
  the cinquillo on the side stick), Bossa (the bossa clave on the side stick), Shuffle (triplet eighths, the hi-hat's 8
  over 12), Breaks (ghost notes, a touch of swing), Poly 3:4:5 (CYCLE: 4 against 3, 5, 6 and 7 in a bar) and Drift
  (eight ring lengths drifting apart, twisted).
- **PLAY** plays the rings while the transport plays. Start plays them from their first step, Stop ends their notes.

The rings start as the Groove pattern, a General MIDI groove on channel 10 (bass drum 5 over 16, snare 2 over 16 turned
by four, closed hi-hat 11 over 16 with accents on the beats, open hi-hat on the offbeats), so it plays Pops 66 or any
General MIDI kit at once; the four inner rings (side stick 5 over 12, low tom 3 over 7, hand clap 4 over 9, cowbell 2
over 5) wait, muted, to be brought in. The rings, the buttons and LAUNCH are saved in the project and in library
modules.

The panel is the orrery itself, drawn flat and graphic. The rings are orbits round a hub, each in a colour of its own; a
ring's steps sit round it, a small dot for a rest, a coloured outline for a hit, a bullseye for an accent, and its hits
are joined into the rhythm's polygon. A planet travels each orbit, trailing a wake of dots, and plays the hits it
passes: they flash and send out a ring. In CYCLE mode the planets line up on one hand that sweeps the dial, the cycle's
beats round the bezel. The hub shows the selected ring's name, E(5,16), and its turn.

**Arpeggiator**: plays the held notes one at a time, in time, over 1–4 octaves (OCTAVES), in UP, DOWN,
U+D (up and down), RAND or ORDER (as played) order. RATE, GATE, SWING, CHANCE, VELOCITY as in the
sequencer. Notes come from its MIDI cable or, with no cable, straight from the MIDI keyboard.
**LATCH** keeps the chord going after you let go of the keys, until you play a new one.

**MIDI Looper**: records what you play on its MIDI input (its cable or, with none, the MIDI keyboard) into
loops of a given number of bars, in time, and plays them back on its MIDI output, to a VST instrument or
anything else with a MIDI input.

- **Eight scenes**, each a loop of its own: BARS (1–16), REPEATS (∞, or 1 to 16 times), direction and what
  follows its last repeat (THEN: the NEXT scene with notes, a RANDOM one, or STOP). **FWD** plays forward,
  **REV** reversed (each note ends where it started, from the end back), **RANDOM** picks: every GRID step
  plays a slice of the loop chosen at random. Click a scene to play it: it starts on the next beat, bar,
  or when the loop playing ends (LAUNCH); stopped, the click just picks it. **COPY →** copies the scene
  into the next empty one.
- **REC** arms the scene. Recording starts as **REC ON** says: on the next bar, with the first note you
  play (on its bar; played up to an eighth early, on the next one), or on a Start (the board's Stop and
  Start, or a MIDI clock's). It runs for the scene's bars, then the scene plays. REC again stops recording
  early (or cancels the arm). **DUB** plays over the loop and adds what you play; pressed while recording,
  it keeps layering after the first pass. Recorded notes join the loop at the end of their pass, pulled
  onto the GRID as far as **QUANTIZE** says. **UNDO** takes back the last recording, overdub, step session
  or **CLEAR** (pressed again, it redoes it). **THRU** lets what you play through, so you hear it.
- **Step sequencer.** The roll shows the scene's notes over two octaves (▲ ▼), four bars at a time (‹ ›; it
  follows the playhead while playing). Click to add a note a GRID step long, drag to stretch it, click a
  note to take it away. With **STEP**, each note or chord you play goes in at the cursor, a GRID step
  long, and the cursor moves on a step; click the ruler to move the cursor.
- **SYNC**: **BOARD** follows the board's tempo and transport; **CLOCK** follows a MIDI clock on the MIDI
  input instead (24 ticks a beat, with Start, Stop, Continue and the song position), between ticks at the
  tempo they keep. MIDI inputs now let the clock and transport through to the MIDI nodes (MIDI In passes
  them on, whatever its channel); they do not count as MIDI activity.
- **MIDI control.** Program Change 1–8 (any channel) picks scenes 1–8. On the **CTRL CH** channel (16 by
  default; off turns it off) notes are commands, neither heard nor recorded: C2 REC, C#2 DUB, D2 PLAY/STOP,
  D#2 UNDO, E2 CLEAR, F2 and F#2 reverse or random pick on and off, C3–G3 scenes 1–8.
- **TRANSPOSE** shifts what plays back, **OUT CH** puts it all on one channel (or as played). A scene holds
  up to 256 notes; the scenes, their settings and the switches are saved in the project.

**MIDI In**: the MIDI keyboard on a cable, to send it into an arpeggiator or a plugin; CHANNEL filters one
channel, TRANSPOSE shifts the notes (controllers and pitch bend pass through unchanged).

Examples:
- *Step Sequencer → VST instrument*: the plugin plays the sequence (leave **KEYBOARD** on to play over it
  from the keyboard too);
- *MIDI In → Arpeggiator → VST instrument*: arpeggios from what you play;
- *Automa → VST instrument*: generative lines in key (with **PAD**, the chords as well);
- *Colloid → VST instrument*: handpan melodies from the atoms (with a Sustain lane from PEDAL for a piano);
- *MIDI In → Colloid → VST instrument*: play chords on the keyboard, and the atoms play them;
- *Orrery → Pops 66* (or a drum plugin): Euclidean grooves on General MIDI drums; give each ring a channel
  of its own and send the node into a multitimbral plugin to play several instruments; recall its pattern buttons from a
  controller's program changes;
- *MIDI Looper → VST instrument*: play a phrase, loop it, layer it, and switch between scenes;
- *Step Sequencer → MIDI Note*, with ENV into an oscillator's AM and MAP on its FREQ: the sequence plays
  the built-in nodes.

Unplugging a MIDI cable (or deleting the node feeding it) sends the plugin note-offs for the notes still
sounding, so no notes hang.

### Plugins: VST3 and Audio Units

Installed plugins appear in the **Plugins** section of the palette and of the right-click menu, grouped by
maker (foldable groups in the palette, submenus in the menu), and are added like any other node:
- **VST3** plugins from `/Library/Audio/Plug-Ins/VST3` and `~/Library/Audio/Plug-Ins/VST3`. No plugin is loaded
  to list them: the maker is read from the bundle's files (VST3 module info, copyright notice or bundle
  identifier), and the category appears after the first load, which also confirms the maker;
- **Audio Units** (macOS): **AUv3** app extensions and classic **AU** (v2) components, Apple's own included
  (AUDelay, AUReverb2, AUNBandEQ, DLSMusicDevice, AUSampler…), as the system lists them (none of their code
  runs to list them; the list is read in the background at launch). Their hint says "AUv3 · Instrument",
  "AU · Fx"…, so a plugin installed in both formats shows twice, VST3 first. **Settings → Plugins** can hide
  Audio Units, or only Apple's.

Makers that can't be found are under "Other"; the search matches makers too.
**Rescan plugins** (in the palette, or in Settings → Plugins) refreshes the list.

A plugin node has a stereo input (effects) and a stereo output; if it accepts MIDI it also has a MIDI
input (square port) for sequencers and arpeggiators. It shows:
- **EDITOR**: opens the plugin's own window (sounds, presets, all its settings). It floats in front of the
  board while AudioBoard is the active app, and hides while you are in another app;
- **PARAMS**: the list of all parameters, with search and sliders; the dot pins up to 8 parameters
  as knobs on the node (at first, the first 8);
- **KEYBOARD**: plays the plugin from the external MIDI keyboard (on by default for instruments), along
  with whatever arrives on the MIDI cable.

The node's knobs can be mapped with a MIDI CC node (MAP), like the others. The project saves the plugin's
path and its complete state, so the sound chosen in its editor too; on another computer a VST3 plugin is
looked up by name in the VST3 folders, and an Audio Unit by its component code (type, subtype and maker),
wherever it is installed. The same goes for library modules.

An Audio Unit node works like a VST3 one: stereo in and out (effects), MIDI in (instruments and MIDI-driven
effects), **EDITOR**, **PARAMS**, **KEYBOARD**, knobs, tempo and transport (for synced LFOs, delays and
arpeggiators). Its maker line says **AUv3** or **AU**. EDITOR opens the unit's own view in a window that
floats like a VST3 editor; a unit without a view of its own gets the system's generic one (sliders) where
macOS has it. Parameters move evenly in octaves for frequencies (those the unit shows logarithmically),
and the values read as the unit writes them ("1200 Hz", "25 %").

A plugin is called once per audio buffer (as in a DAW), so a heavy instrument costs the same whatever the
patch around it. Only its main stereo outputs are used: extra outputs (a drum machine's separate drum
channels) are switched off at load so the plugin doesn't render them. While the transport is stopped, a
plugin with nothing coming in rests after a second of silence; any MIDI or sound wakes it at once. A
plugin's sound (its complete state) is read when the project is saved: some plugins take a while to
answer, so saving while playing may briefly interrupt them.

VST3 plugins and AU (v2) components run inside AudioBoard: a faulty one can make the application quit.
**AUv3** extensions run in a process of their own, as in Logic, so a crash there leaves AudioBoard playing
(the node falls silent). VST3 and Audio Units are supported (not VST2 or CLAP).

### Plugin nodes

**Plugin nodes** are AudioBoard's own plugin format: nodes written in Rust with the node SDK and shipped
precompiled, one dynamic library each (`.dylib` on macOS, `.so` on Linux, `.dll` on Windows). Put the library in
`~/Library/Application Support/AudioBoard/Nodes` (the palette's **Open Nodes folder** link opens it, creating
it if needed) and press **Rescan** (in the palette, or **Rescan plugins** in Settings → Plugins), or restart.
Folders listed in the `AUDIOBOARD_NODE_PATH` environment variable are searched too (separated like `PATH`).

Their nodes appear in the **Plugin nodes** section of the palette and of the right-click menu, by maker,
and from there on they are nodes like the built-in ones: ports, cables (audio and MIDI), knobs that a MIDI
CC node can MAP, presets, bypass, modules and the library. Their panel is the generic module panel, laid
out from the node's description: port labels, a readout line, an optional switch, a display, a mode
selector and the knobs. The display is one of a few pictures the node feeds with numbers: its output
level, bars, a curve, a scrolling trace or an XY dot.

A project saves a plugin node by its stable id (`"module": "dev.audioboard.example.haze"`), with its
parameters by name, its mode, switch and extra data, like a built-in module. Opening a project whose plugin
isn't installed skips that node and says which ids are missing; the rest of the project opens.

A plugin library is checked before any of its nodes is used: a library built for another version of the
plugin interface, or whose description doesn't hold together (a knob with an empty range, a display value
it doesn't have), is refused with the reason, and AudioBoard carries on. Plugin nodes run inside
AudioBoard, on the audio thread: a node that panics is caught, stops, and shows "Stopped: the plugin
crashed" in its display while the rest of the patch plays on; samples that are not finite (or absurdly
loud) are muted before they reach the mix. A library is never unloaded while AudioBoard runs (its nodes run
its code), so a rebuilt library loads after a restart; a new one loads on Rescan. **Settings → Plugins**
lists the folders and the libraries loaded, with their version and the SDK version they were built with.

An example library, with an instrument, an effect and a MIDI generator, is in `plugins/example-nodes`; see
[Writing nodes](#writing-nodes-sdk-plugins-and-ui-kit) to build and install it, and
[plugins/README.md](plugins/README.md) for a step-by-step guide to making your own.

### Output

**Mixer (4 channels)**: for each channel a volume fader, pan (balance on stereo signals), an **AUX** send,
Mute/Solo and a meter. Below the channels are a stereo master bus with **MUTE**, L/R meters and a clip
indicator, and the AUX bus with its own level and meters.

**Console (12 or 24 channels)**: the same mixer, larger, with two shapes chosen with its switches.
**H** (horizontal) lays the channel strips side by side, as on a mixing desk, with vertical faders; the
inputs line the node's top edge and their cables drop in from above. **V** (vertical) stacks the strips as
rows, with the inputs down the left edge. **12 / 24** sets the number of channels. Going back to 12 asks first
if channels 13–24 have cables, since it unplugs them.

**Outputs, AUX and MUTE.** Every mixer has two outputs on its right edge:

- **OUT** carries the master mix. While OUT is unpatched, the mixer feeds the audio device ("Main out").
  Patch OUT into another mixer to use it as a submix: it then reaches the device only through that cable,
  never twice.
- **AUX** carries the sum of the channels' AUX sends. The sends are taken after each channel's fader, pan
  and mute.
- Master **MUTE** silences the main mix only: the AUX sends keep playing.

For example, a dry mix plus an effects layer: the tracks go into Mixer 1, which plays on the device. Turn up
AUX on the tracks you want effected, patch Mixer 1's AUX into a Reverb (or a chain of effects), and the
Reverb into a second mixer. That mixer also plays on the device, so you hear the clean tracks plus the
effected layer, each with its own level. MUTE on Mixer 1 leaves only the effected layer.

### Visual: Iris

**Iris** is a video node: a screen on the board where visuals react to the sound, drawn in the board's flat,
graphic style (line art, dot matrices, coloured outlines; no glows). It is also a visual synthesizer: up to
four layers of scenes, each blended, moved, tinted, mirrored and echoed, and modulators (LFOs, envelopes,
audio bands, MIDI) routed to them by a matrix. It is in the **Visual** group of the palette and of the
right-click menu.

- **What it listens to**: by default (**Auto**) the node patched into its **IN**, and only that one (a drum
  machine, a bass line, a reverb's tail), or the **main mix** (everything the board plays) while IN is free.
  The settings can also force **IN** (nothing while unpatched) or **Main mix** (even with IN patched).
  **THRU** passes IN on unchanged, so Iris can sit inside a chain; it is never an effect (no bypass), and it keeps running, with the nodes feeding it,
  even when THRU goes nowhere.
- **Nineteen scenes**:
  - geometric: **Mandala** (rings of petals with 5- to 16-fold symmetry, each ring a band), **Halo** (a
    halftone ring of dots on rays round an empty centre, biggest half way out and shrinking to specks inside
    and out, with faint streaks along the rays: it turns, swells with the bass, sends a ripple out on
    onsets, and its colours travel round it; the spectrum goes round the ring), **Tunnel**
    (polygons flying out of a vanishing point, twisting with the mids, new sides on onsets), **Truchet**
    (quarter-circle tiles joining into endless paths; onsets send a wave of flips across the grid), **Moiré**
    (overprinted sets of rings whose centres orbit, the bass pulling them apart), **Dots** (a dot-matrix
    spectrum with falling peaks), **Orbits** (planets on dashed orbits in a polyrhythm, one orbit a band;
    onsets draw the constellation);
  - mathematical: **Harmonograph** (a damped pendulum drawing in small ratios, which jump on onsets),
    **Spiro** (nested hypotrochoids), **Attractor** (a Clifford attractor's cloud of points, its constants
    drifting with the sound), **Phase** (the sound against itself a quarter period later: a tone draws a
    circle, a chord a knot, with echoes of past frames spiralling out), **Scope** (an oscilloscope: the
    waveform held still on a rising zero crossing over three periods of the pitch, on a faint graticule,
    its past traces fading behind it), **Ridges** (pulsar lines, the
    spectrum scrolling up the screen, each line hiding the ones behind);
  - generative: **Flow** (particles on a flow field; onsets burst them outwards), **Life** (Conway's Game of
    Life on a dot matrix: onsets drop gliders and R-pentominoes where the spectrum peaks, the highs spark
    cells, and it never dies out), **Automaton** (an elementary cellular automaton scrolling up, the spectrum
    writing into its newest row, onsets changing the rule and the ink);
  - 3D, projected in software with the same flat dots and lines: **Globe** (a cloud of dots on a sphere, a
    torus or a trefoil knot turning in space, each dot pushed out by its band; onsets spin it and now and
    then morph it into the next shape), **Terrain** (the spectrum as a wireframe landscape: each new row is
    the sound now, in front, and the rows move away into the distance, hiding what is behind them, under a
    sun in outline; onsets mark their row), **Flight** (flying through a field of dots: the bass and the
    level set the speed, onsets kick it, the mids roll the view, each dot swelling with its band and
    streaking as it rushes past).
- **Layers**: up to four, drawn in order (layer 1 first, 4 on top), each with its own scene (layer 1's is
  the node's scene), on or off, **Solo**, and a blend: **Normal**, or **Add** (light adds up, for dark
  palettes). Each has its **Opacity**, **Zoom**, **Rotate**, **Spin** (turns a second), **X** and **Y**,
  **Hue** (turns the colours round the wheel), its own **Speed**, a **Mirror** (**Mirror**: the left half
  reflected on the right; **Quad**: the top left quarter reflected into the other three; **×3** and **×6**
  turned copies) and **Echoes**: up to eight past frames behind it, each grown by **Feedback** and fading
  (above ×1 an endless zoom, below it a fall inwards). A plain layer draws straight onto the screen; any
  other is drawn into a layer of its own, tessellated into one mesh, moved and tinted vertex by vertex and
  put on the screen, all on the CPU. Each layer keeps to a budget of vertices: a heavy scene gets fewer
  echoes rather than a slow frame.
- **Modulation**: four **LFOs** (sine, triangle, saw, square, random; free from 0.02 to 10 Hz, or synced to
  the transport from 1/16 to 8 bars, on the beat while it plays; a phase), two **envelopes** (attack,
  decay, sustain, release; fired by MIDI notes, by onsets or by both, the gate held while a note is), four
  **audio** modulators (a range of the 32 bands and a gain: bass, mids, highs and everything to begin
  with), and MIDI: **Velocity**, **Note**, **Mod wheel** and two controllers, **CC A** and **CC B**, by
  number. The onset **Beat** is a source too. The **matrix** has twelve slots, each a source, a destination
  and a bipolar amount: the destinations are the global Speed, Detail and Trail and every layer's Opacity,
  Zoom, Rotate, X, Y, Hue and Speed.
- **MIDI**: notes and controllers arrive on its **MIDI** input, or from the keyboards while that is free.
- **The analysis**, on the UI thread (the audio thread only copies the samples): a 2048-point spectrum in 32
  bands from 30 Hz to 16 kHz (tilted +3 dB an octave, as music has less energy up high), the level, bass, mids
  and highs, the strongest pitch, and onsets (spectral flux over its own running average).
- **The screen** fills the node. Drag the picture to move the node. While the pointer is over it, the
  scene's name and what it listens to now (IN or MAIN) show at the top left, a **gear** at the top right and a **grip** at the
  bottom right: drag the grip to resize the node (Shift keeps its shape), from 240 × 170 to 2400 × 1600.
  When a scene changes by itself, its name shows for a moment.
- **The gear** opens the settings in a popup beside the node, so the picture stays in view, in three tabs:
  **Picture**, **Layers** (the four layers, then the one selected) and **Modulation** (the matrix, with
  the slots in use and a free one, then the LFOs, envelopes, audio and MIDI, each with a small meter of
  what it does now). Picture holds the scene;
  **Auto**, **IN** or **Main mix**, with a line saying what it listens to now; the **palette** (Board, Mono in the node's colour, Paper and Riso on light paper,
  Ember, Ice, Acid, and Lagoon: blue through cyan to green on white); **React** (how strongly it follows the sound), **Speed**, **Detail** (lines, particles,
  cells, symmetry), **Smooth** (how slowly the analysis lets go) and **Trail** (trails, echoes, histories);
  **Next scene every** 1 to 16 bars at the board's tempo (or never); **Frame** or **No frame**; the
  screen's shape (16:9, 4:3, 1:1, 9:16, Reset); **New seed** for the generative scenes.
- **No frame**: the node's card (title bar, border, bezel, shadow) goes and the picture fills the whole
  node, as if stuck on the board; the ports stay on its edges, an outline shows while it is selected, and
  the scene's name, gear and grip still show under the pointer. Collapsed, it has its card again.
- **Open in a window**: the picture in a window of its own (its own generative state, the same sound),
  for a second display or a projector. Double-click it for full screen (the pointer hides while it rests),
  Esc leaves full screen, then closes the window. It keeps playing when the node is off the view or
  collapsed.

The scene, the knobs, the size, the look, the frame, the layers, the modulators and the matrix are saved
with the project (the scene by name, so projects keep their scene as new ones are added; a project saved
before the layers opens with layer 1 alone, as it was); the window is not. A scene costs the UI about 0.1 to
3.5 ms a frame at full detail (the Attractor the most); a layer with echoes and a mirror up to about 3 ms
more, and four such layers together about 12 ms. The board redraws at full rate while an Iris is in view.

### Settings

**Settings** (Cmd+,, the sliders button in the toolbar, or click the device name next to it) has four tabs
down its left side: **Audio**, **MIDI**, **Sync** and **Plugins**. Each is made of titled cards of rows, with
the labels in one column, the controls in the next and a status line under them (green when all is well).
Every change applies at once and is kept between launches.

**Audio** chooses the sound card:

- **Output.**
  - **Device**: the system default (it follows the Mac's choice) or a specific interface. A device that is
    missing at launch falls back to the default.
  - **Channels**: the pair the board plays on (1–2, 3–4…), for interfaces with more than two outputs.
  - **Sample rate**: the device's own, or 44.1 to 192 kHz where the device supports it. A new rate builds
    every node again for it. The patch stays as it is, and so do its settings and cables.
  - **Buffer size**: see below.
- **Input**: off, the system default, or a specific device. It runs at the output's rate: a device that
  can't is marked "other rate". While the window is open, a meter per input channel shows the levels.
  - Input and output can be different devices. A buffer of cushion keeps them in step. If the two clocks
    drift apart, the input now and then skips ahead or refills, which may be heard as a tiny gap. The same
    interface for both (or an aggregate device) avoids it.
  - The latency added is one input buffer.
- **Audio system**: shown when the platform has more than one (CoreAudio, JACK…).

**Refresh devices** reads the list again. It also refreshes itself every few seconds while the window is open,
so an interface plugged in meanwhile appears.

**MIDI** lists the MIDI inputs connected (every input is used, and one plugged in is picked up within
seconds), each with a **notes** light (notes and controllers arriving) and a **clock** light (clock and
transport arriving); the one followed as the clock is marked SYNC SOURCE.

**Sync** chooses the transport's clock: **Internal** (the board's BPM, TAP, Start/Stop) or **MIDI clock** from
one of the inputs; a status line shows the tempo received, whether it plays and where (bar and beat), or
what it is waiting for. **Send clock** has a switch for each MIDI output. See *External MIDI sync* above.

**Plugins** shows the VST3 folders and how many plugins they hold, switches for listing Audio Units and
Apple's own, with how many there are (AUv3 and AU), the plugin nodes' folders and the libraries loaded from
them, and **Rescan plugins** (which rescans plugin nodes too).

### Audio buffer, load and limiter

The status bar shows the audio buffer in samples and its latency ("512 smp · 10.7 ms"): click it to choose
another size, as in a DAW. Larger buffers give heavy patches and plugins more headroom (fewer clicks) at
the cost of latency; smaller ones respond faster. The choice is kept between launches; "Device default"
uses the sound card's own size. Changing it keeps the patch playing.

Next to it, **DSP** is the average load and **peak** the heaviest callback of the last seconds: near 100%
the audio can't keep up. If that happens, a red **dropouts** counter appears for a while (it also counts
the gaps the sound card reports): try a larger buffer. The load also counts the time the processor takes to
wake up for each buffer, which on a laptop can be a good part of it (on an Intel MacBook Pro at 512 samples,
about a third: callbacks cost 32% of real time sleeping in between, 18% kept awake); a larger buffer wakes it
less often.

**Nodes nobody hears rest.** A node whose sound doesn't reach the device (nothing patched from it leads to a
mixer that plays) stops working 2 seconds after it was last heard, and costs nothing until it is patched in
again, so chains left unplugged don't weigh on the DSP load. Its panel stands still meanwhile (meters,
displays, step lights). Patched in again, it carries on from where it stopped, fading in over a few
milliseconds; a delay, reverb or grain cloud may first replay a moment of what it held, and a frozen one
keeps its sound. A cable moved from one input to another within 2 seconds changes nothing. The MIDI Looper
(which records), MIDI CC and MIDI Note (which can drive mapped controls) and Iris always run.

The output goes through a safety **limiter** just under full scale (−0.3 dBFS) with 1.3 ms of lookahead:
when the sum gets too loud (several plugins, high faders) it turns the level down smoothly instead of
clipping it into crackles, and recovers over about 150 ms. While it works, the status bar shows
"limit −x dB": lower the master or the plugins. Below full scale it does nothing.

**OUT** in the toolbar sets the board's overall output level, from −24 to +12 dB (0 dB by default). Drag it, or
click it to type a value. It sits before the limiter, so turning it up makes everything louder without
clipping, and it is kept between launches.

Gain staging follows a DAW's:

- A new mixer's MASTER starts at 0 dB.
- A channel's pan works as a stereo balance: both sides at unity in the centre, the far side fading out as you
  turn, so a centred channel keeps its level.
- A source at −6 dBFS through a fresh mixer reaches the device at −6 dBFS.

## Controls

| Action | Gesture |
|--------|---------|
| Switch between the node view and the track view | V, or the two buttons right of the project name (see Track view) |
| Add a node or a library module | drag it from the palette onto the canvas (or click it), or right-click the canvas |
| Connect | drag from a port to the opposite kind (output → input or vice versa) |
| Reconnect / unplug a cable | drag it away from the input it is plugged into |
| Delete a cable | double-click the cable, or right-click → Disconnect |
| Adjust knobs / faders | drag (Shift = fine adjustment), double-click = reset. Stepped knobs (notes, whole numbers, divisions) step on smoothly however slowly you drag, and their pointer glides from step to step |
| Select several nodes | drag a rectangle on empty canvas, or Shift+click (Cmd+A all, Esc none); Shift+click a module's header or card adds or removes the whole module; Shift+right-click adds what is under the pointer and opens its menu |
| Move the view | two-finger scroll, drag with the right or middle button (also starting on a node or a module), or Alt+drag |
| Zoom | pinch or Cmd+scroll; Cmd+= / Cmd+− / Cmd+0 (100%); or click the percentage in the status bar |
| Move / delete / duplicate the selection | drag a selected node / Del / Cmd+D (inner cables are copied) |
| Delete a node | the × in its header, Del, or right-click → Delete: AudioBoard asks first (Enter deletes, Esc cancels), since deleting can't be undone |
| Make a module from the selection | **Make module** on the selection bar, Cmd+G, or right-click → Make module |
| Rename a module | Enter or F2 with the module selected, or right-click → Rename… (Enter confirms, Esc cancels) |
| Dissolve a module | Cmd+Shift+G, the × in the module's header, or right-click → Ungroup |
| Collapse / expand a node or a module | arrow in the header, double-click the title, the C key, or right-click → Collapse |
| Collapse / expand everything | Shift+C, the toolbar button, or right-click the canvas |
| Frame everything / the selection | F / Shift+F |
| Bypass an effect | the power button in its header, B with the mouse over it (or on the selected effects), or right-click → Bypass on / off: its input goes straight to its output, crossfaded, and the effect keeps running underneath so switching back is seamless. A bypassed effect is dimmed and says BYPASS; it's saved with the project. Every node that processes a signal on its IN can be bypassed (filters, Magma, Equalizer, Prism, Tape, delays, Reverb, the texture nodes, plugin effects) |
| Zoom in on one node | Z with the mouse over it (or on the selection), or right-click → Zoom to node: it fills the view, in front of the others; Z again or Esc goes back to the view you had |
| Auto layout | L, the toolbar button or right-click the canvas; Cmd+Z undoes it |
| Auto-fit (keep the whole patch in view) | A or the toolbar button |
| Add a sticker / change the background | right-click empty canvas → Stickers ▸ / Background ▸, or drag image files onto the canvas (see Stickers and background) |
| New / open project | Cmd+N / Cmd+O, or the project name in the toolbar (Open Recent lists the last ten) |
| Save / save as | Cmd+S / Shift+Cmd+S |
| Start / stop | the Start / Stop button in the toolbar, or Space |
| Settings (audio, MIDI, sync, plugins) | Cmd+, the sliders button in the toolbar, or the device name |
| Record / stop recording | REC in the toolbar, or R; stopping opens the export (see Recording and export) |
| Tempo | drag the BPM field, click it to type, TAP or T to tap it |
| Show / hide the oscilloscope and spectrum | panel button in the toolbar |

The full list of shortcuts appears when hovering the **?** button in the toolbar.

Resting the pointer on a node or a module for a moment shows a small translucent label just above it with
its type, since titles can be renamed: "Reverb · hall · shimmer · freeze", "Originals - Cinematic Pads ·
VST3 instrument · Spitfire Audio" ("AUv3 instrument", "AU effect" for Audio Units), or for a module the nodes
it holds ("Module · Drone · Filter · 2× Reverb").
The label stays put rather than following the pointer, takes no clicks, and hides while dragging, in menus
and while renaming.

Right-click (or Ctrl+click on macOS) opens the menu for whatever is under the pointer: canvas, node
(also when clicking a knob or a display), module or cable. Its first line says what it acts on: "4 nodes
selected", or the node's or module's name.
- Right-clicking something already selected keeps the whole selection, and the menu acts on all of it.
  Right-clicking something not selected selects it alone; with Shift (or Cmd) held, it joins the selection.
- Menus never lose the selection by themselves: the click (or Esc) that closes a menu does only that, a
  Ctrl+click is a right-click, not a click, and a right drag only moves the view.
- With a selection, the canvas menu starts with **Make module from selection** and **Deselect**. A module's
  menu offers **Make module from selection** too when more than that module is selected.

**Selection bar.** While two or more nodes are selected, a small bar floats above them (below, when there is
no room above) with the count and **Make module**, **Collapse** (or **Expand**), **Duplicate** and **Delete**,
so a module can be made without any menu. It keeps a fixed size whatever the zoom, stays clear of the module
headers, and hides while nodes are dragged, a selection rectangle is drawn, a module is renamed or a
control is mapped. Make module is left out when the selection is exactly one module already.

A collapsed node takes up only a strip with its title, ports and main value (frequency, cutoff, RT…);
a collapsed mixer also shows the master meters. Cables and audio keep working normally.

Connections that would create a feedback loop are refused; each input takes a single cable.
Feedback is available inside nodes (Delay, Reverb).

### Modules

A **module** groups several nodes into one, to move, collapse and duplicate them together.
Create one by selecting nodes and clicking **Make module** on the selection bar (or Cmd+G, or right-click →
Make module). To rename it, select it and press Enter (or F2, or right-click → Rename…): the name is edited
right in the header.

Modules grow by merging: select a module (click its header or card) and Shift+click more nodes or other
modules, then Make module. The new module holds them all; the modules it took in dissolve into it, since
a node belongs to one module only.

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

## Track view

Besides the free canvas, the same patch can be seen as **tracks**, in the spirit of Ableton Live: one row
per track, its chain of devices from left to right, down to the mixer channel it feeds. Switch with **V** or
the two buttons right of the project name (nodes / tracks). It is a second view of the same patch, not a
copy: whatever is done in one shows in the other, and switching back and forth changes nothing (node
positions on the canvas, cables, settings, folded nodes all stay as they were). The view in use is saved
with the project, with the track view's own layout (track order, folded rows, open panels); projects saved
before the track view open in the node view.

**What a row shows.** On the left, the track's header: its name (the first device's), what kind of track it
is (INSTRUMENT, AUDIO, EFFECTS, BRANCH, RETURN, BUS), where it goes (click it to send the track to another
free channel, or to none), and its channel's mute, solo, fader and meter. On the right, the devices: compact
boxes with the title, the activity light, the bypass switch, the main value and notes on their cables, or
with their panel open (the arrow in the header, a double-click on it, or C), scaled down to fit when tall.
A chip at the end says where the audio goes. Each Mixer or Console has a **master row** of its own (its
master fader, mute and meters, and its panel); a last row, "Not in a track", holds whatever plays into no
track (a MIDI In patched to nothing). A track folds to a thin row with the arrow left of its name.

**How the patch becomes tracks.** Every input takes one cable, so walking back from any point follows a
single path; a track is that path:

- A cable into a node's main input (IN on effects, plugin effects, Iris) or into a mixer channel is part of a
  chain. Every other audio cable (CUT, SC, PITCH, TRIG…) is a side cable (modulation, sidechain); MIDI
  cables are their own kind.
- There is one track per patched mixer channel, and one per chain that ends nowhere (an unrouted source, the
  last of a row of effects). Walking back from there, the track stops at its own source, at an effect fed by
  nothing, or at a mixer's OUT or AUX: a **bus** or **return** track (a Mixer's AUX through a reverb into the
  Console is a return).
- **Splits.** When a node feeds two places (a filter into a reverb and into grains), its devices are shown
  once, in the first of the tracks; the other one starts with a chip "◂ FILTER 1" and says "from FILTER 1"
  in its header. Moving a track up or down moves the shared devices with it.
- Nodes that make no audio for the chain sit in the lane just before the device they serve, with a dashed
  arrow into it, while the audio runs round under them: sequencers, arpeggiators and other MIDI nodes in
  front of the instrument they play, modulators (Random, a MIDI Note's envelope) in front of what they
  modulate, a MIDI CC mapped to a knob in front of that knob's device.
- **No cable is hidden.** A cable is the arrow between two neighbouring devices, a track's input or
  destination, the dashed arrow of an attached device, or else a note on the devices at both ends:
  "CUT ◂ RANDOM 1" and "▸ FILTER 1 · CUT", "SC ◂ SIBYL 1", "▸ GRAINS 1" on a device that also feeds
  another track, "MIDI ◂ SEQ 1", "CH 2 ▸ CONSOLE 1" for an output other than the first. The notes are
  listed in full when resting the pointer on a device.

**Editing.** Each edit is a few cable changes, the same as patching them on the canvas:

| Action | Gesture | Cables |
|--------|---------|--------|
| Add a device anywhere in a chain | drag an effect from the palette between two devices (a line shows where it goes), or the **+** at the end of a track; a click in the palette adds it after the selected device | what fed that point feeds the new device, which feeds what came after |
| Add a MIDI device | drag a sequencer, an arpeggiator… onto an instrument's track | it takes what played the instrument (if it takes MIDI) and plays the instrument; refused if that would unplug what played it |
| Move a device | drag it to another place, in its track or another one | its old neighbours are joined, and it goes between the new ones; its other cables (splits, sidechains) go with it. A move that would make a loop is refused and changes nothing |
| Remove a device | the × in its header, Del, or right-click → Remove (it asks first) | what fed it now feeds everything it fed, in every track; the same for MIDI through a MIDI device |
| Add a track | drop a source anywhere but between devices, or click **New track** at the bottom | the new node goes below the patch on the canvas and into the first free mixer channel |
| Route a track | click where it goes in its header | to a free channel only: a track never takes another's channel |
| Reorder tracks | drag a track's name up or down | none: only the order of the rows |
| Bypass | the power switch, B over a device, or right-click | none |

New devices land on the canvas between their neighbours (what lies downstream moves right to make room
when needed); **L** in the node view tidies everything up. MIDI mapping (MAP) is done in the node view.

## Palette

The panel on the left (shown and hidden with the button next to the project name) lists all nodes by
category, the plugin nodes and plugins by maker, and the library modules, with a search field. Click a category's heading to fold or unfold it
(a folded one shows how many entries it holds); while searching, every category with matches opens. Drag an entry onto the canvas to place it where you
want (an outline shows where it lands), or click it to add it in the middle of the view. On library
entries, the × removes the module from the library.

## Stickers and background

The board can be made your own with **stickers** and a different **background**, both from the right-click
menu on empty canvas (**Stickers ▸** and **Background ▸**). They are saved with the project, images included,
so a project opens looking the same on another computer.

**Stickers** sit on the board behind modules, cables and nodes, so they never get in the way of patching:
- **Built-in stickers**: fourteen designs (wave, moon, cassette, knob, bolt, mushroom, planet, heart,
  sparkle, headphones, speaker, leaf, sun, eye) and six tags (AMBIENT, DRONE, LOOP, LIVE, WIP, NO INPUT),
  drawn as flat line art in the board's colours on a dark badge; they stay sharp at any zoom. The menu
  shows them as a grid of previews: click one to place it where you right-clicked.
- **Image stickers**: **PNG** (with its transparent background), **JPG**, **SVG** (and WebP). Add them with
  **Image file…**, or drag image files from the Finder onto the canvas (the canvas says when it will take
  them). An SVG stays a vector, drawn again at the size it is shown, so it is sharp at any zoom; PNG and JPG
  are shrunk to 1024 pixels on their longest side if larger. Images in the **stickers folder**
  (`~/Library/Application Support/AudioBoard/Stickers`, **Open stickers folder** in the menu) are listed in
  the menu too. **Example files** adds the four SVG examples in `assets/stickers` (vinyl, synth, rain
  cloud, patch cable).
- **Moving and shaping**: drag a sticker to move it; click it to select it (its outline shows), then drag
  the square handle at its corner to resize it or the round one to rotate it (Shift: 15° steps). Del deletes
  the selected sticker, Esc lets it go. A right or middle drag on a sticker pans the canvas as anywhere else.
- **Its menu** (right-click a sticker): Bring to front, Send to back, Rotate left / right 15°, Straighten,
  Duplicate, **Lock** and Delete. A locked sticker lets clicks through, so rectangle selection and panning
  work over it; **Unlock all stickers** in the Stickers menu frees them again.

**Background**: eight presets, each a colour and a grid: Graphite (the usual), Dots, Blueprint, Night, Moss,
Ember, Plum and Plain (no grid), shown as swatches. **Image…** puts a picture (PNG, JPG, SVG) behind the
board, filling the view without stretching; **Veil** sets how much the preset's colour covers it, so nodes
stay readable, and **Grid** shows or hides the grid. A photo is kept as a JPEG of up to 2560 pixels.

## Recording and export

**REC** in the toolbar (or **R**) records what the board plays: the device's output, after the limiter, so
exactly what you hear. While it records, the button turns red and counts the time. The take goes straight to
a temporary file on disk, so it can be as long as you like, and the audio is never held up.

**R** again stops recording and opens **Export recording**, which shows the take's length, sample rate and
peak. To export it:

1. Pick a format:
   - **WAV**: 24-bit, 16-bit (with TPDF dither) or 32-bit float.
   - **AIFF**: 24-bit or 16-bit.
   - **FLAC**, **Apple Lossless** (.m4a) or **AAC 256 kbps** (.m4a), encoded by macOS's own `afconvert`.
   - **MP3 320 kbps**, available when `ffmpeg` or `lame` is installed (e.g. `brew install lame`).
2. Choose the options: normalize the peak to −1 dBFS, and trim the silence at both ends (on by default; 10 ms
   are kept around the sound).
3. Click **Export…** to pick where to save it. The export runs in the background.

The take stays available for more exports, in other formats, from **EXPORT** in the toolbar, until the next
recording, **Discard take**, or quitting.

## Projects and library

**Projects.** The project name in the toolbar opens the File menu (New project, Open…, Open Recent, Save,
Save as…). **Open Recent** lists the last ten projects opened or saved, newest first, with their folder; it is
kept between launches. A project that is no longer there is shown greyed out ("missing"); **Clear menu**
empties the list.
A dot next to the name, and "(edited)" in the window title, flag unsaved changes: before creating a new
project, opening another one or closing the window, AudioBoard asks whether to save them.
A project (`.audioboard`) is a JSON file with nodes and parameters, positions, colors, collapsed nodes,
cables and modules, the view in use and the track view's layout (written only once the track view has been
used), the board's stickers and background (images inside, base64) and Splitter's samples (16-bit
PCM inside, base64); parameters are saved by
name, built-in modules by name and plugin nodes by their stable id. The demo patch always opens on launch.

**Module library.** Right-click a module's header → **Save to library** saves it under its name (if it
already exists, it asks whether to replace it). Saved modules appear in the **Library** section of the
palette and of the right-click menu: inserting one adds the nodes, the cables between them and their
settings as a new module, already **collapsed** to a card. The × next to the name removes it from the
library, and **Open library folder** opens the folder of `.abmodule` files
(`~/Library/Application Support/AudioBoard/Library` on macOS), useful for copying or sharing them.

## Writing nodes: SDK, plugins and UI Kit

AudioBoard is a Cargo workspace. Besides the app it holds three crates:

```
crates/audioboard-node   the node SDK (the "foundation"): what a node is and how it runs; no dependencies
crates/audioboard-ui     the UI Kit: theme, controls, node card, displays and the generic panel's layout (egui)
plugins/example-nodes    an example plugin library: Glass (instrument), Haze (effect), Pulse (MIDI)
```

`cargo build` and `cargo test` work on the app alone (they build the two crates it uses); add `--workspace`
to build and test everything, plugins included.

### The node SDK (`audioboard-node`)

Every node, built-in or plugin, is written in the SDK's terms; the app re-exports them where its modules
always found them (`audio::params`, `audio::engine`, `audio::fastmath`, `audio::modules`), so built-in
modules already run on the SDK's types. Built-in modules keep their own `Processor` for now; a plugin node,
or a new module, implements the SDK's `Node` trait.

- **Description.** `NodeDef`: name, title prefix, menu hint, audio inputs and outputs, MIDI in and out,
  knobs (`ParamSpec`: range, `Taper` linear or log, `Unit` for how the value reads, bipolar arc), CV depth
  knobs, mode labels, a switch, extra saved values (`data`, a sequencer's steps), the display's height and
  how many display values the node publishes. A plugin node adds a `Descriptor`: a **stable id** (reverse-DNS,
  saved in projects; never change it), its maker, and its `Panel` (below).
- **Processing.** `Node::new(&Init)` runs on the UI thread (sample rate, the node's settings): allocate
  everything there. `Node::process(&ProcessCtx, &mut Io)` runs on the audio thread, one block of at most 64
  stereo frames (`BLOCK`, `Buf = [[f32; 64]; 2]`) at a time. `Io` hands it its inputs (`input(i)`, `cv(i)`,
  unpatched as `None`), its outputs (`output(i)`, or `through(i, j)` for an effect), the MIDI that arrived
  (`midi()`: `MidiMsg` with the frame offset within the block) and `send_midi(frame, bytes)` for its MIDI
  output. `ProcessCtx` gives the sample rate, the ~10 ms smoothing coefficient, the transport's beat at the
  block's start, beats per sample (`tempo()`, `beat_at(i)`), whether the transport plays, and `restart`
  (rewound to beat 0: start over, end held notes).
- **Settings, lock-free.** The app owns a node's `Params` (knob values, mode, switch, data in atomics; the
  output meter and display values coming back). The node reads them through `Shared`: `get(i)`, `mode()`,
  `toggled()`, `data(i)` / `set_data(i, v)`, `set_viz(i, v)`. Out-of-range indexes read 0 and write nothing.
- **DSP helpers** (`dsp`, `fastmath`): `Rng` (xorshift), `Wander` (smooth random drift), `DelayLine`
  (fractional reads), `Quiet` (tells when an effect may rest), `Smoothed`, `OnePole`, `pan_gains`,
  `mix_gains`, `soft_limit`, `soft_ceiling`, `time_coeff`, `note_hz`, `peak`; fast `sin_turns`, `exp2`,
  `log2`, `tanh`.

**On the audio thread**, `process` must not allocate, lock, wait or do I/O; everything it needs is made in
`new`. A panic is caught at the plugin boundary: the node stops and stays silent.

A whole effect, from `crates/audioboard-node/src/lib.rs`:

```rust
use audioboard_node::prelude::*;

const PARAMS: &[ParamSpec] = &[ParamSpec::lin("GAIN", 0.0, 2.0, 1.0, Unit::Percent)];

pub struct Gain { params: Shared, gain: Smoothed }

impl Node for Gain {
    const DESCRIPTOR: Descriptor = Descriptor {
        id: "com.example.gain",
        maker: "Example",
        def: NodeDef { name: "Gain", prefix: "GAIN", hint: "louder · softer",
                       inputs: &["IN"], outputs: &["OUT"], params: PARAMS, ..NodeDef::EMPTY },
        panel: Panel { display: Display::Level, readout: Readout::Param(0) },
    };

    fn new(init: &Init) -> Self {
        Gain { params: init.params, gain: Smoothed::new(1.0) }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io<'_>) {
        let n = io.frames();
        let target = self.params.get(0);
        let (input, out) = io.through(0, 0);
        for i in 0..n {
            let g = self.gain.next(target, ctx.smooth);
            for c in 0..2 {
                out[c][i] = input.map_or(0.0, |b| b[c][i]) * g;
            }
        }
    }
}

audioboard_node::export_nodes! { name: "Gain", maker: "Example", version: "1.0", nodes: [Gain] }
```

### Plugin format and ABI versioning

A plugin is a `cdylib` crate that depends on `audioboard-node` and ends with `export_nodes!` (any number of
nodes in one library). Rust has no stable ABI, so the app and a plugin, possibly built by different compilers
and SDK versions, share only a C ABI (`audioboard_node::abi`): `#[repr(C)]` tables and `extern "C"` functions,
no Rust types.

- The library exports one symbol, `audioboard_node_entry(host_abi: u32) -> *const PluginInfo`. `PluginInfo`
  stays valid for the life of the process; it carries the library's name, maker and version, the SDK version,
  the block size and the nodes.
- **The first field of `PluginInfo` is the ABI version (`u32`), at the same place in every version.** The app
  reads it before anything else and refuses a library built for another version, with a message saying to
  rebuild it ("built for node ABI v2, this AudioBoard loads v1: rebuild it with audioboard-node 0.1.0"). Any
  change to the tables or functions bumps `abi::ABI_VERSION`; units and display kinds have stable numbers,
  and unknown ones read as a plain number and no picture.
- Each node is a description (text as UTF-8 pointer and length, arrays as pointer and length) and three
  functions: `create` (UI thread; gets the sample rate and pointers to its settings' atomics, valid until
  `destroy`), `process` (audio thread; one block: buffers, MIDI in and room for MIDI out, the transport) and
  `destroy` (UI thread). The app copies the description at load and checks it (sizes, ranges, ids, what the
  panel refers to); text that isn't UTF-8, an id already taken, or anything inconsistent refuses the library.
- `export_nodes!` wraps each call in `catch_unwind`, so a panic becomes an error code instead of crossing the
  boundary. Nothing crosses it per block but pointers: no allocation, no lock. (A plugin built with
  `panic = "abort"` would take the app down with it: keep the default.)

**Build and install** the example (macOS; `.so` and `~/.local/share/audioboard/Nodes` on Linux):

```bash
cargo build --release -p audioboard-example-nodes
```

```bash
mkdir -p ~/Library/Application\ Support/AudioBoard/Nodes && cp target/release/libaudioboard_example_nodes.dylib ~/Library/Application\ Support/AudioBoard/Nodes/
```

then Rescan in the palette. A plugin of your own is the same: a crate with `crate-type = ["cdylib"]` and
`audioboard-node` as a dependency (a path into this repository, or a git dependency), built in release.

**[plugins/README.md](plugins/README.md) is the step-by-step guide**: creating the crate, writing a node
(a complete Tremolo effect), building, installing (or `AUDIOBOARD_NODE_PATH` while developing), testing a
node without the app, instruments, MIDI generators, CV inputs and several outputs, the generic panel, the
audio-thread rules, keeping saved projects working across versions, universal macOS libraries and signing,
and what each refusal message means.

### The UI Kit (`audioboard-ui`)

The parts every panel is drawn from, in the app's flat, line-art style:

- `theme`: the colour palette, fonts (`font`, `mono`, `strong`, sized by the canvas zoom), the egui style.
- `view::NodeView`: draws in node units at any zoom (`p`, `r`, `text`, `font`) and records the knobs and
  faders drawn, so MIDI can be mapped to them.
- `widgets`: knob, horizontal and vertical faders, toggle, peak meters (`MeterState`), ports, cables, shadow.
- `controls`: `knob_cell` (label, knob, value as its unit reads), `segmented` selector, `pill`,
  `port_labels`, `caption`.
- `card`: the node card and header (collapse chevron, activity light, title, bypass, close), bypass veil.
- `display`: the dark inset and its pictures (`Screen::bars`, `curve`, `trace`, `dot`, `level`, `area`,
  `message`).
- `panel`: the generic panel's layout from a `NodeDef` (columns, display, selector and knob rows, ports).

Built-in panels use these parts directly (`ui/nodes.rs` and `ui/modules.rs` take `NodeView`, `knob_cell`,
`segmented`, the card and the panel layout from the kit). **A plugin node can't**: egui's types change from
one version to the next and can't cross the C ABI. Its panel is therefore described, not drawn: the generic
panel from its `NodeDef`, plus a `Panel` saying which picture its display shows (`Display::Level`, `Bars`,
`Curve`, `Trace` or `Dot`, fed by its display values) and what its readout line says (its name, a knob's
value, a display value in a unit, or the mode). The app draws it with the kit (`ui/node_panel.rs`).

## Architecture

```
crates/
├── audioboard-node/src/   the node SDK: def (NodeDef, Params, Descriptor, Panel), node (Node, Init, Io,
│                          Shared), buffer (BLOCK, Buf, MidiBuf, ProcessCtx), param (ParamSpec, Taper, Unit,
│                          AtomicF32, formatting), dsp, fastmath, abi (the C ABI, export_nodes!)
└── audioboard-ui/src/     the UI Kit: theme, view (NodeView), widgets, controls, card, display, panel
plugins/
├── README.md              writing, building, testing and distributing a node plugin, step by step
└── example-nodes/         an example plugin library (cdylib): glass, haze, pulse
src/
├── audio/
│   ├── params.rs   parameters shared lock-free (atomics) between the UI and the audio thread (the
│   │               vocabulary, ParamSpec, units and formatting, comes from the node SDK)
│   ├── engine.rs   audio-side graph: one pass per device buffer (built-ins in 64-frame stereo blocks,
│   │               plugins in one call), per-block MIDI events, global clock in beats, precomputed
│   │               topological order, lookahead limiter, denormals flushed to zero
│   ├── fastmath.rs fast sine, exp2, log2 and tanh for per-sample DSP (error < 1e-6), from the node SDK
│   ├── dsp.rs      Oscillator and Mixer (any number of channels, main and AUX outputs)
│   ├── modules/    audio_in, drone, random, granular, resonator, filter, equalizer (after Waves F6), delay, pingpong,
│   │               reverb, midi_cc, midi_note, sequencer, arpeggiator, midi_in, midicer (after Vermona's
│   │               meloDICER), iris (passes IN through, hands the panel the sound it listens to, the MIDI it
│   │               receives and the transport; the layers', modulators' and matrix's settings)
│   │   ├── magma.rs    Magma (after FabFilter Volcano 3): parameters, routing, oversampling, slots
│   │   ├── magma/      filter (non-linear SVF styles, responses), modulation (XLFO, EG, EF, MIDI, XY)
│   │   ├── prism.rs    Prism (after FabFilter Pro-Q 4): the bands' knobs and settings, the minimum-phase cascade
│   │   │               (at twice the rate for Natural Phase), triggers, solo and audition, character, auto
│   │   │               gain, output, the analyzer's rings
│   │   ├── prism/      design (the ten shapes as analog prototypes, their responses, analog-matched digital
│   │   │               sections), spectral (FFT frames: linear phase, spectral dynamics, placement), fft (sliced
│   │   │               over a hop), dynamics (triggers, automatic threshold and times), fit (EQ Match, Sketch)
│   │   ├── frakture.rs Frakture (after Excite Audio's Motion: Fractal): parameters, mappings, the chain
│   │   ├── frakture/   grains (tempo-locked buffer, two layers, MUTATE), rhythm (gate, repeat), space (hall),
│   │   │               motion (hexagon sliders, magnet, drawn/recorded paths, timeline)
│   │   ├── pops.rs     Pops 66 (after the KORG Mini Pops-7): instruments, user patterns, clock, MIDI in/out
│   │   ├── pops/       voices (the fifteen synthesized instruments), rhythms (the twenty presets, combining)
│   │   ├── sibyl.rs    Sibyl (after the Prophet-5 and Repro-5): parameters, programs (record, restore), keys,
│   │   │               pedal, voice allocation and unison, MIDI, segments, the output
│   │   ├── sibyl/      voice (oscillators with BLEP, sync and FM; the CEM3320-style filter in lanes of four
│   │   │               voices; envelopes; distortion; the patch decoded), mods (LFO, wheel noise, the matrix
│   │   │               and its modifiers), fx (tape, echo, res·eq, plate, conditioner), arp, bank (programs)
│   │   ├── fjord.rs    Fjord (after the Clavia Nord Drum): the channels' knobs, programs (packed three settings
│   │   │               to a value, store, Memory Protect), MIDI in and out, learning, Local, the step lanes
│   │   ├── fjord/      voice (tone, noise and click, gates, sweeps, bend, punch, the tone's ADSRs, held notes,
│   │   │               the channel), bank (the factory programs), trigger (the Trig inputs: scan, mask,
│   │   │               falling threshold), alea (the dice: sounds bred from the bank, grooves by role)
│   │   ├── splitter.rs Splitter: the slices' settings, the six animation modes on the grid, MIDI in (notes,
│   │   │               LEARN, CHROMA) and out, ratchets, the voices and their mix
│   │   ├── splitter/   voice (straight and granular playback, envelope, crusher), atoms (the eight built-in
│   │   │               glitch sources, pure functions of position), slicing (grid, transients, random; what a
│   │   │               slice sounds like), dice (throws by role, BACK)
│   │   ├── nullpunkt.rs Nullpunkt: step patterns, synced stutter, glitch stage, built-in scenes
│   │   ├── aether.rs   Aether: chords, FREE and keys, additive voices in vector banks, the motion's targets,
│   │   │               RANDOM, built-in sounds
│   │   ├── aether/     fractal (octaves of smooth noise, the spectra), glitch (pings, tuned skips, ratchets),
│   │   │               space (soft saturation, chorus, diffuse space with tuned air)
│   │   ├── larsen.rs   Larsen: the loop (strings, drive, filters, reverb, BODY), tuning to notes, three strings
│   │   │               and COUPLING, the output's limiter, built-in sounds
│   │   ├── larsen/     parts (string, drive, filters, the loop's reverb), echo (CLASSIC and PONG), keys (the
│   │   │               strings' notes)
│   │   ├── vesper.rs   Vesper: knobs and scenes, the score (bars, chords, MIDI), the layers, shared effects
│   │   ├── vesper/     harmony (moods, progressions, voicing), sounds (the recipes), library (rendering them
│   │   │               into loops on a thread of its own), layer (playing loops, pings), fx (motion filters,
│   │   │               strips, the MixTypes)
│   │   ├── automa.rs   Automa: steps, choosing and fitting notes, voices, pad, FREEZE, what the panel shows
│   │   ├── automa/     engines (LIFE, CASCADE, TREE, JULIA), harmony (scales, chords, progressions,
│   │   │               consonance rules, functional substitutions)
│   │   ├── colloid.rs  Colloid: knobs, the notes (panel or keys), molecules' notes, strikes into notes, QUANT,
│   │   │               the CC lanes
│   │   ├── colloid/    physics (the tank: molecules, atoms, gravity, liquid, strikes, stirring), chords
│   │   │               (handpan layouts, chord shapes, colour tones, roles, spreading notes over the molecules)
│   │   ├── orrery.rs   Orrery: the rings (steps, pulses, turns, hits), Euclidean rhythms, GRID and CYCLE,
│   │   │               a note per ring on its channel, DICE, the pattern buttons and the built-in patterns,
│   │   │               recalls on the LAUNCH grid, Program Change
│   │   ├── looper.rs   MIDI Looper: scenes and their notes, transport, recording and overdub, playing
│   │   │               forward, reversed or by random slices, launching and following scenes, MIDI control
│   │   ├── looper/     clock (following a MIDI clock: ticks, Start, Stop, Continue, song position)
│   │   ├── tape.rs     Tape: record EQ, magnetic curve, transport (wow, flutter, stop/start), playback head
│   │   ├── oversample.rs 2× oversampling (half-band FIR) shared by Magma, Tape, Aether and Prism
│   │   ├── steps.rs step grid on the global tempo, outgoing notes, held keys
│   │   └── mod.rs  ModuleKind (built-ins, and Ext for plugin nodes), their descriptions (the SDK's NodeDef),
│   │               ModuleParams (the SDK's Params with the kind), the shared DSP helpers from the SDK
│   ├── node_plugin.rs plugin nodes: scanning the Nodes folders, loading dylibs (libloading), checking the
│   │               ABI version and descriptions, the lock-free registry behind ModuleKind::Ext, the processor
│   │               that runs a plugin's node block by block (panics and bad samples contained)
│   ├── node_plugin_tests.rs SDK nodes through the ABI, patches, projects, refusals; the example dylib
│   ├── midi.rs     MIDI inputs (midir) and shared state read by the MIDI nodes (clock and transport too),
│   │               activity per input, the clock source's follower
│   ├── sync.rs     MIDI sync: timing an external clock (least-squares tempo, transport, song position),
│   │               steering the engine's beat onto it, the clock-out thread
│   ├── plugin.rs   plugins of both formats behind one lock (VST3 through vst3-host, Audio Units): loading,
│   │               scanning, processing on the audio thread
│   ├── au.rs       Audio Units (macOS): listing, pseudo-paths, parameter ranges, the Rust face of au_host.m
│   ├── au_host.m   the Objective-C host over AUAudioUnit (built by build.rs): AUv3 out of process, AU in
│   │               process, render and MIDI blocks, host clock, parameters, state, the editor window
│   ├── sample.rs   audio files a module holds (Splitter's): WAV/AIFF decoding, kept in the project as 16-bit
│   │               PCM in base64, handed to the audio thread without locking it
│   ├── record.rs   recording the output: a lock-free tap on the audio thread, a writer thread, takes
│   ├── export.rs   takes to WAV/AIFF (written here), FLAC/ALAC/AAC (afconvert), MP3 (ffmpeg or lame)
│   ├── device.rs   the audio setup (devices by id, channel pair, rate, buffer), listing what the system offers
│   ├── input.rs    the audio input: a lock-free ring from the input device to the engine, with a cushion
│   │               against drift between the two devices' clocks
│   └── mod.rs      opening the output and input streams (cpal); a new rate builds a new engine
├── graph.rs        UI-side patch model (nodes, cables, modules, Kahn ordering)
├── tracks.rs       the track view's reading of the patch (graph → tracks, its rules) and its edits as
│                   cable changes (insert, move, remove, route, new track); the track view's saved layout
├── board.rs        the board's stickers, images (PNG/JPEG kept encoded, SVG kept as text) and background
├── project.rs      saving/loading projects and the module library (JSON)
├── presets.rs      named presets of a module (knobs, settings, mode, toggle), one file per kind in the app's
│                   data folder (Orrery's patterns in a file of their own)
├── ui/
│   ├── canvas.rs   canvas with pan/zoom, selection, cable drag & drop, context menus
│   ├── tracks.rs   the track view: headers (route, mute, solo, fader, meter), lanes of devices (compact
│   │               or with their panel), dragging devices and tracks, drops from the palette
│   ├── groups.rs   modules: frame (expanded) and card with the boundary ports (collapsed)
│   ├── layout.rs   auto layout by signal flow (columns, aligned ports, corridors for cables)
│   ├── palette.rs  palette of nodes, plugin nodes, plugins and library modules, with dragging onto the canvas
│   ├── plugin_node.rs panel of plugin nodes (editor, parameters, MIDI, knobs)
│   ├── nodes.rs    layout and drawing of the oscillator and mixer; NodeView, the card, knob cells and
│   │               selectors from the UI Kit
│   ├── console.rs  the Console's panel: horizontal (inputs on top) or vertical, 12 or 24 channels
│   ├── modules.rs  common module panel and their displays (layout from the UI Kit's panel)
│   ├── node_panel.rs a plugin node's display (its Panel, drawn with the UI Kit) and readout
│   ├── frakture.rs the Frakture node's panel: HEXAGON, EFFECTS, MUTATE (the grain cloud) and MOTION pages
│   ├── equalizer.rs the Equalizer's panel: graph and band markers, analyzer (FFT), band controls
│   ├── prism.rs    the Prism panel, flat: analyzer bar, the display (spectra, curves as outlines, band dots,
│   │               dynamics, Spectrum Grab, EQ Match, Sketch, piano), the band strip, the bottom bar
│   ├── magma.rs    the Magma panel: display and dots, filter controls, sources and slots (drag & drop)
│   ├── tape.rs     the Tape panel: the sketched deck (reels, tape, head, needles, keys), COMPACT / FULL
│   ├── nullpunkt.rs the Nullpunkt panel: editable lanes, randomize, presets, knob groups
│   ├── aether.rs   the Aether panel: the fractal flower, the partials' aurora, the motion strip, knob groups
│   ├── larsen.rs   the Larsen panel: the loop's field (spectrum, filter band, harmonics, BODY's resonances),
│   │               switches, knob groups
│   ├── vesper.rs   the Vesper panel: the landscape, moods and root, the chords, a strip per layer, knob groups
│   ├── preset_menu.rs the PRESETS ▾ menu shared by Nullpunkt, Aether, Larsen and Vesper: save, load, delete
│   ├── looper.rs   the MIDI Looper panel: transport, scenes and their settings, the editable roll, knobs
│   ├── automa.rs   the Automa panel: the generator's world (grid, waterfall, tree, Julia set), the roll over
│   │               the chord lanes, the progression strip, knob groups
│   ├── colloid.rs  the Colloid panel, flat and graphic: the tank (bands, dot matrix, wells, molecule icons, wakes,
│   │               strikes, stirring), HANDPAN / CHORD and the shape, the notes' cells, knob groups, CC lanes
│   ├── orrery.rs   the Orrery panel, flat and graphic: the orbits (steps, polygons, notches, planets and their
│   │               wakes, pings, the CYCLE hand), turning rings by dragging, the pattern buttons (glyphs,
│   │               STORE, LAUNCH, the PATTERNS ▾ library), the rings' table, the strip, knobs
│   ├── settings.rs the Settings window: tabs Audio (output, input and its meters), MIDI (inputs and their
│   │               lights), Sync (clock source, clock out), Plugins (VST3 folders, Audio Units, plugin
│   │               nodes)
│   ├── iris.rs     the Iris panel: the analysis (spectrum, bands, onsets), the nineteen scenes (the 3D ones
│   │               projected in software), palettes, the scene's name, gear and grip, the frameless look, the
│   │               settings popup (Picture tab), the window of its own
│   ├── iris/synth.rs  Iris as a visual synth: the layers (blend, transform, hue, mirror, echoes) composited on
│   │               the CPU, the LFOs, envelopes, audio and MIDI modulators, the matrix, the Layers and
│   │               Modulation tabs
│   ├── recent.rs   Open Recent: the list of recent projects and its submenu
│   ├── board.rs    background presets and image, stickers on the canvas (drawing, moving, handles, menu,
│   │               dropped files, textures), the built-in designs, the Stickers and Background submenus
│   ├── pops.rs     the Pops 66 panel: MACHINE, MIXER and EDITOR detail levels (the node resizes), waveform preview
│   ├── splitter.rs the Splitter panel: LOAD, CUT, VIEW, DICE; the display (the source, its slices, playheads
│   │               and grains; cutting and moving slices), the slice keys; pages SLICE (SOUND, STUTTER,
│   │               CRUSH, GRAINS, each drawn), SEQUENCE (the modes, the lane, PROB bars, Euclidean rings),
│   │               MIDI (the keyboard, LEARN)
│   ├── sibyl.rs    the Sibyl panel: control bar and programmer, the three rows, the tweaks' boards, keys
│   │               with wheels and arpeggiator, the effects and their chain
│   ├── fjord.rs    the Fjord panel: the bank map of the 99 programs, channel keys and their lights, ALEA and
│   │               its die; pages SOUND (the strike's outlines, NOISE, TONE, CLICK, MIXER, AMP and FILTER
│   │               ENV) and SEQUENCER · MIDI (the lanes, SYSTEM); the node resizes with the page
│   ├── widgets.rs  hand-drawn knobs, faders, meters, ports and cables (the UI Kit's), waveform previews
│   ├── monitor.rs  bottom panel: oscilloscope and spectrum (FFT)
│   ├── tempo.rs    the toolbar's tempo: BPM field (drag or type), beat light, tap tempo; a MIDI clock's
│   │               tempo while following one
│   ├── macos.rs    native windows: toolbar controls don't drag the window; plugin editors float
│   ├── theme.rs    colors, system fonts and style (the UI Kit's)
│   └── app.rs      window, toolbar, panels, demo patch
├── tests.rs        tests of the graph, the modules and offline engine rendering
└── perf.rs         benchmarks (audio per module, large patches, UI frames, auto layout)
```

The UI never blocks the audio thread: knob values travel through atomics, while structural changes
(nodes added/removed, a new processing order) reach the engine as commands on a channel.
Buffers (delay, reverb, granular loop) are allocated on the UI thread when the node is created.
Audio Units are hosted through `audio/au_host.m`, a small Objective-C layer over `AUAudioUnit` that
`build.rs` compiles with the `cc` crate on macOS (manual reference counting, so nothing on the render path
retains, releases or allocates); `audio/au.rs` is its Rust face. An Audio Unit has no file, so it travels as
a pseudo-path (`audiounit:<type>-<subtype>-<maker>` in hex) wherever a plugin path goes. Its parameters are
set from the UI directly (`AUParameter` is thread-safe), not through the audio thread.
Plugins are shared through a lock: the UI calls into a plugin only between audio callbacks (when
the audio thread has just finished and won't need the plugin for most of a buffer), one call at a time, at
most ten times a second, and the plugin's state is read only when saving, so the audio thread is not kept
waiting. Each buffer is processed as one pass: built-in nodes run through it in 64-frame blocks (so MIDI
timing and modulation stay fine-grained), while a plugin gets the whole buffer in a single call with its
MIDI events placed at their exact offsets, since every plugin call has a fixed cost. Removed nodes and old
processing orders are handed back to the UI thread to be freed: freeing memory (a delay line is megabytes)
inside the audio callback could stall it into a click. A sample loaded into a Splitter goes the same way
(`audio/sample.rs`): the UI swaps it in under a lock and bumps a version, the processor takes it with
`try_lock` (never waiting; it tries again next block) and hands back the one it let go, freed by the UI.

**Performance.** Effects rest when they have gone quiet: with a silent input, once their own signal (echoes,
tails) has stayed below −100 dB for as long as they can remember sound, they skip their work and output
silence until input returns; a frozen reverb or an endless delay never rests. Stopped sources rest too, so a
stopped patch costs almost nothing once its tails are gone. Nodes nobody hears rest as well: `Graph::sync`
marks in the processing order which nodes reach the device (a mixer whose OUT is unpatched, and everything
with a path of cables, audio or MIDI, into one), plus those that work for their own sake (`runs_unheard`:
the MIDI Looper, MIDI CC and MIDI Note, which can drive mapped controls, Iris, nodes with no outputs). The
engine runs any other node for 2 more seconds, so a cable moved to another input comes back seamlessly, then
skips it: whatever reads its outputs isn't heard either, so it rests too. Heard again, it carries on from
the state it stopped in, its outputs faded in over 20 ms. What is heard doesn't change, bit for bit; in a
project with almost half its nodes unpatched (Fjord, below; on an Intel MacBook Pro) the audio load fell from
about 18% to 11% of real time offline, and in real time at 512 samples the heaviest callbacks from about
41% to 28%. Denormal numbers are flushed to zero, and the
per-sample maths uses fast approximations of sine, exp2 and tanh. The UI redraws at full rate only during interaction and animations; meters, analyzers and
step lights update at 30 fps (not at all while the window is minimized). Zoom uses font sizes rounded to
half a point, so animations don't re-create glyphs every frame; from far away, unreadable text is not drawn
and the grid is a single mesh. Auto layout and zoom transitions follow a spring curve with no bounce: nodes
set off in a wave from left to right, and zoom proceeds on a logarithmic scale around the center, with no
sideways drift.

To add a new module: a file in `audio/modules/` with its `ModuleDef` (the SDK's `NodeDef`: knobs, ports,
modes, toggle) and a `Processor`, an entry in `ModuleKind` and, in `ui/modules.rs`, the readout and the display.
A node that should ship on its own is a plugin instead (see [Writing nodes](#writing-nodes-sdk-plugins-and-ui-kit)):
it needs no change to the app.
The panel (ports, selector, knobs) is generated from the definition; Frakture, too large for the generic
panel, draws its own (`ui/frakture.rs`), and so do the Equalizer (`ui/equalizer.rs`), Prism (`ui/prism.rs`), Magma (`ui/magma.rs`), Tape (`ui/tape.rs`), Nullpunkt (`ui/nullpunkt.rs`), Aether (`ui/aether.rs`), Larsen (`ui/larsen.rs`), Vesper (`ui/vesper.rs`), the MIDI Looper (`ui/looper.rs`), Automa (`ui/automa.rs`), Colloid (`ui/colloid.rs`), Orrery (`ui/orrery.rs`), Sibyl (`ui/sibyl.rs`), Fjord (`ui/fjord.rs`), Splitter (`ui/splitter.rs`), Iris (`ui/iris.rs`, sized by the user) and Pops 66
(`ui/pops.rs`; its size, like Fjord's and Iris's, follows its settings through `modules::size_of`). Frakture's hexagon runs on the audio thread, block by block: the panel
hands it the mouse through the module's display values, so paths keep time even when the window doesn't
redraw.

## Tests

```bash
cargo test
```

With `--workspace`, the SDK's, the UI Kit's and the example plugin's own tests run too:

```bash
cargo test --workspace
```

Building the example plugin as a dynamic library (with cargo, into `target/node-plugin-test`), loading it
from a folder as the app does, playing its nodes in a patch (Haze on an oscillator, Pulse playing Glass)
and saving and restoring that patch is a test of its own, not run by default:

```bash
cargo test example_plugin_builds_loads_and_plays -- --ignored
```

Loading and playing real Audio Units (Apple's, always installed, and a few others found on the Mac) is a
test of its own, not run by default:

```bash
cargo test -- --ignored audio_units_load_and_play --nocapture
```

What each Iris scene costs the UI per frame, at full detail, at the node's size and at 1920 × 1080:

```bash
cargo test --release iris_frame_cost -- --ignored --nocapture
```

Benchmarks don't run with the normal tests:

```bash
cargo test --release perf -- --ignored --nocapture --test-threads=1
```

`perf_project` measures a saved project: its load offline, its callbacks made in real time at a 512-frame
buffer (mean, 99th percentile, worst) and what each node costs, in rounds that alternate every node running
and unheard nodes resting (a laptop's thermal drift then shows in both). `PERF_PROJECT` picks the project
(by default `~/Documents/AudioBoard/Fjord.audioboard`); `PERF_SPIN=1` waits busy between callbacks instead
of sleeping, to leave the CPU's idle states out:

```bash
PERF_PROJECT=path/to/project.audioboard cargo test --release perf_project -- --ignored --nocapture --exact perf::perf_project
```

`perf_project_render` renders a project offline into a raw file (`PERF_RENDER_OUT`, `PERF_SECONDS`), to
check that an optimization leaves what is heard as it was: the render is deterministic, plugins included, so
two renders compare equal byte for byte.
