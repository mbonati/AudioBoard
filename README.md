# AudioBoard

Sintetizzatore modulare a nodi in Rust, nello spirito di Max/MSP e Pure Data:
piccoli blocchi (nodi) collegati con cavi virtuali generano il suono in tempo reale.

## Avvio

```bash
cargo run --release
```

All'avvio si apre una patch demo: un **LFO** module l'ampiezza (AM) di un'onda a dente di sega,
che insieme a un'onda triangolare entra nel **mixer**, collegato all'uscita audio.

## Nodi

**Oscillatore**: forme d'onda Sine, Triangle, Saw, Pulse e Noise (saw e pulse sono band-limited con PolyBLEP).

| Knob  | Funzione |
|-------|----------|
| FREQ  | 0.1 Hz – 20 kHz (scala logaritmica; sotto 20 Hz funziona da LFO) |
| FINE  | ±100 cent |
| LEVEL | ampiezza di uscita |
| SHAPE | dipende dall'onda: Drive (sine/saw), Skew (tri), Width (pulse), Color (noise) |
| FM    | profondità della modulazione di frequenza dall'ingresso FM |
| AM    | profondità della modulazione d'ampiezza dall'ingresso AM |

Ingressi `FM` e `AM`, uscita `OUT`. Un oscillatore può modularne un altro.

**Mixer (4 canali)**: per ogni canale fader di volume, pan, Mute/Solo e meter LED;
bus master stereo con meter L/R e indicatore di clip. Il mixer alimenta la scheda audio.

## Comandi

| Azione | Gesto |
|--------|-------|
| Aggiungere un nodo | tasto destro sul canvas, oppure i pulsanti in alto |
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

## Architettura

```
src/
├── audio/
│   ├── params.rs   parametri condivisi lock-free (atomics) tra UI e thread audio
│   ├── engine.rs   grafo lato audio: blocchi da 64 frame, ordine topologico precalcolato
│   ├── dsp.rs      processori: Oscillator e Mixer
│   └── mod.rs      apertura dello stream di uscita (cpal)
├── graph.rs        modello della patch lato UI (nodi, cavi, ordinamento di Kahn)
├── ui/
│   ├── canvas.rs   canvas con pan/zoom, drag & drop dei cavi, menu contestuali
│   ├── nodes.rs    layout e disegno dei nodi
│   ├── widgets.rs  knob, fader, meter, porte e cavi disegnati a mano
│   ├── monitor.rs  pannello inferiore: oscilloscopio e spettro (FFT)
│   ├── theme.rs    palette, font di sistema e stile
│   └── app.rs      finestra, toolbar, pannelli
└── tests.rs        test del grafo e rendering offline del motore
```

La UI non blocca mai il thread audio: i valori dei knob passano tramite atomics, mentre le modifiche
strutturali (nodi aggiunti/rimossi, nuovo ordine di elaborazione) arrivano al motore come comandi su un canale.
Per aggiungere un nuovo tipo di nodo basta implementare il trait `Processor` e descriverne porte e disegno.

## Test

```bash
cargo test
```
