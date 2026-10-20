# AudioBoard

Sintetizzatore modulare a nodi in Rust, nello spirito di Max/MSP e Pure Data:
piccoli blocchi (nodi) collegati con cavi virtuali generano il suono in tempo reale.
È pensato in particolare per suoni ambient, droni, texture organiche e granulari.

## Avvio

```bash
cargo run --release
```

All'avvio si apre una patch demo ambient: un **Drone** (accordo di quinte su La1) passa in un **Filter**
il cui cutoff è mosso lentamente da un **Random**; il filtro alimenta un **Reverb** con un po' di shimmer
e, un'ottava sopra, una nuvola **Granular** che finisce in un **Delay** a nastro. Tutto confluisce nel **Mixer**.

Ogni cavo è stereo; le sorgenti mono scrivono lo stesso segnale su entrambi i canali.

## Nodi

### Sorgenti

**Oscillator**: forme d'onda Sine, Triangle, Saw, Pulse e Noise (saw e pulse sono band-limited con PolyBLEP).

| Knob  | Funzione |
|-------|----------|
| FREQ  | 0.1 Hz – 20 kHz (scala logaritmica; sotto 20 Hz funziona da LFO) |
| FINE  | ±100 cent |
| LEVEL | ampiezza di uscita |
| SHAPE | dipende dall'onda: Drive (sine/saw), Skew (tri), Width (pulse), Color (noise) |
| FM    | profondità della modulazione di frequenza dall'ingresso FM |
| AM    | profondità della modulazione d'ampiezza dall'ingresso AM |

**Drone**: otto voci detunate disposte in un accordo (Unison, Fifths, Octaves, Major, Minor, Suspended).
Ogni voce deriva lentamente in intonazione e volume, così il suono non sta mai fermo.
Knob: PITCH, DETUNE (cent), DRIFT, TONE (da sinusoidi scure a saw brillanti), SPREAD (ampiezza stereo),
P.MOD / T.MOD (profondità degli ingressi PITCH e TONE), LEVEL.

**Random**: sorgente di modulazione casuale. RATE, CHAOS (tempi irregolari), SMOOTH (da sample & hold a
glide continuo), LEVEL. Da collegare a qualunque ingresso di controllo (cutoff, pitch, posizione dei grani…).

### Texture

**Granular**: registra l'ingresso in un loop di 8 secondi e lo riproduce come una nuvola di grani.
POS (quanto indietro leggere), SIZE, DENSITY (grani al secondo), PITCH (semitoni), JITTER (casualità di
posizione e tempi), SPRAY (pitch casuale), SPREAD (stereo), MIX. Direzione FWD / REV / BOTH.
**FREEZE** smette di registrare: la nuvola continua a esplorare il suono catturato. L'ingresso POS
sposta la posizione di lettura (es. con un Random o un LFO). Il display mostra il loop e i grani attivi.

**Resonator**: banco di otto modi risonanti accordati (String, Pipe, Bar, Bell) eccitato dall'ingresso.
Con rumore o grani in ingresso dà timbri fisici e organici. PITCH, DECAY, BRIGHT, SPREAD, P.MOD, MIX.

### Effetti

**Filter**: state-variable filter stereo LP / BP / HP / Notch. CUTOFF, RESO, DRIVE, MOD
(profondità dell'ingresso CUT in ottave, anche negativa). Il display mostra la risposta in frequenza.

**Delay**: eco stereo a nastro fino a 2 s. TIME (le variazioni scivolano, come un nastro), FEEDBACK,
TONE (scurisce le ripetizioni), WOW (wow & flutter), PING (ping-pong), MIX.

**Reverb**: rete di 8 linee di ritardo con diffusione, modulazione e smorzamento. SIZE (da stanza a
cattedrale), DECAY (0.3 – 60 s), DAMP, MOD, SHIMMER (un'ottava sopra, rientra nella coda), MIX.
**FREEZE** congela la coda all'infinito.

### Uscita

**Mixer (4 canali)**: per ogni canale fader di volume, pan (bilanciamento sui segnali stereo), Mute/Solo
e meter; bus master stereo con meter L/R e indicatore di clip. Il mixer alimenta la scheda audio.

## Comandi

| Azione | Gesto |
|--------|-------|
| Aggiungere un nodo | tasto destro sul canvas, oppure **Add node** nella toolbar |
| Collegare | trascinare da una porta a quella opposta (output → input o viceversa) |
| Ricollegare / staccare un cavo | trascinarlo via dall'ingresso a cui è collegato |
| Eliminare un cavo | doppio clic sul cavo, oppure tasto destro → Disconnect |
| Regolare knob / fader | trascinare (Shift = regolazione fine), doppio clic = reset |
| Spostare la vista / zoom | scroll / pinch o Cmd+scroll |
| Eliminare / duplicare un nodo | Del / Cmd+D sul nodo selezionato |
| Inquadrare tutti i nodi | F |
| Audio on/off | Spazio |
| Mostrare / nascondere oscilloscopio e spettro | pulsante pannello nella toolbar |

L'elenco completo delle scorciatoie compare passando il mouse sul pulsante **?** della toolbar.

I collegamenti che creerebbero un loop di feedback vengono rifiutati; ogni ingresso accetta un solo cavo.
Il feedback si ottiene dentro i nodi (Delay, Reverb).

## Architettura

```
src/
├── audio/
│   ├── params.rs   parametri condivisi lock-free (atomics) tra UI e thread audio
│   ├── engine.rs   grafo lato audio: blocchi stereo da 64 frame, ordine topologico precalcolato
│   ├── dsp.rs      Oscillator e Mixer
│   ├── modules/    nodi ambient: drone, random, granular, resonator, filter, delay, reverb
│   │   └── mod.rs  descrizione dei moduli (ModuleDef), parametri generici e helper DSP
│   └── mod.rs      apertura dello stream di uscita (cpal)
├── graph.rs        modello della patch lato UI (nodi, cavi, ordinamento di Kahn)
├── ui/
│   ├── canvas.rs   canvas con pan/zoom, drag & drop dei cavi, menu contestuali
│   ├── nodes.rs    layout e disegno di oscillatore e mixer, controlli condivisi
│   ├── modules.rs  pannello comune dei moduli e i loro display
│   ├── widgets.rs  knob, fader, meter, porte e cavi disegnati a mano
│   ├── monitor.rs  pannello inferiore: oscilloscopio e spettro (FFT)
│   ├── theme.rs    palette, font di sistema e stile
│   └── app.rs      finestra, toolbar, pannelli, patch demo
└── tests.rs        test del grafo, dei moduli e rendering offline del motore
```

La UI non blocca mai il thread audio: i valori dei knob passano tramite atomics, mentre le modifiche
strutturali (nodi aggiunti/rimossi, nuovo ordine di elaborazione) arrivano al motore come comandi su un canale.
I buffer (delay, riverbero, loop granulare) sono allocati sul thread della UI quando il nodo viene creato.

Per aggiungere un nuovo modulo: un file in `audio/modules/` con la sua `ModuleDef` (knob, porte, modi,
toggle) e un `Processor`, una voce in `ModuleKind` e, in `ui/modules.rs`, la lettura e il display.
Il pannello (porte, selettore, knob) è generato dalla definizione.

## Test

```bash
cargo test
```
