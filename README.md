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

### MIDI

I nodi MIDI ricevono da qualsiasi dispositivo MIDI collegato: vengono collegati automaticamente, anche se
inseriti dopo l'avvio. L'indicatore **MIDI** nella barra di stato mostra quanti ingressi sono attivi
(passa il mouse per i nomi) e si accende quando arrivano messaggi.

**MIDI CC**: un controller esterno (knob, fader, rotella) come segnale. CC, CHANNEL (Omni o 1–16),
SMOOTH (glide), uscita 0…1 o ±1. **LEARN** prende il prossimo controller che muovi.

**MIDI Note**: una tastiera come inviluppo. Tenendo un tasto, ENV sale in ATTACK; rilasciandolo scende in
RELEASE; VEL è lo stesso inviluppo scalato dalla velocity. Entrambe le uscite vanno da −1 a +1, quindi
nell'ingresso AM di un oscillatore (AM al 100%) passano dal silenzio al volume pieno. Priorità all'ultima
nota premuta; TRANSPOSE in semitoni.

**Pilotare un controllo (MAP).** Oltre a collegare le uscite con i cavi, un nodo MIDI può muovere
direttamente qualsiasi knob o fader: premi **MAP** sul nodo, poi clicca il controllo (Esc annulla). Una
linea tratteggiata collega il nodo al controllo, e il nodo mostra "→ FILTER 1 · CUTOFF". Con MIDI CC il
controllo segue il controller; con MIDI Note i knob di frequenza prendono la frequenza della nota (es.
PITCH del Drone o FREQ dell'oscillatore), quelli in semitoni la distanza dal Do centrale, gli altri la
posizione del tasto. Il controllo si muove solo con i messaggi che arrivano dopo la mappatura. **UNMAP**
la rimuove. Le mappature si salvano nel progetto e nei moduli della libreria.

### Plugin VST3

I plugin VST3 installati (`/Library/Audio/Plug-Ins/VST3` e `~/Library/Audio/Plug-Ins/VST3`) compaiono nella
sezione **Plugins** della palette e del menu del tasto destro, e si inseriscono come qualsiasi altro nodo.
Per elencarli non viene caricato nessun plugin; categoria e produttore compaiono dopo il primo caricamento.
**Rescan plugins** aggiorna l'elenco.

Un nodo plugin ha un ingresso stereo (effetti) e un'uscita stereo, e mostra:
- **EDITOR**: apre la finestra del plugin stesso (suoni, preset, tutte le sue impostazioni);
- **PARAMS**: l'elenco di tutti i parametri, con ricerca e cursori; il pallino fissa fino a 8 parametri
  come knob sul nodo (all'inizio sono i primi 8);
- **MIDI IN**: suona il plugin dalla tastiera MIDI esterna (attivo da solo per gli strumenti).

I knob del nodo si possono mappare con un nodo MIDI CC (MAP), come gli altri. Il progetto salva il percorso
del plugin e il suo stato completo, quindi anche il suono scelto nel suo editor; su un altro computer il
plugin viene cercato per nome nelle cartelle VST3. Lo stesso vale per i moduli della libreria.

I plugin girano dentro AudioBoard: un plugin difettoso può far chiudere l'applicazione. Sono supportati i VST3
(non VST2, AU o CLAP).

### Uscita

**Mixer (4 canali)**: per ogni canale fader di volume, pan (bilanciamento sui segnali stereo), Mute/Solo
e meter; bus master stereo con meter L/R e indicatore di clip. Il mixer alimenta la scheda audio.

## Comandi

| Azione | Gesto |
|--------|-------|
| Aggiungere un nodo o un modulo della libreria | trascinarlo dalla palette sul canvas (o cliccarlo), oppure tasto destro sul canvas |
| Collegare | trascinare da una porta a quella opposta (output → input o viceversa) |
| Ricollegare / staccare un cavo | trascinarlo via dall'ingresso a cui è collegato |
| Eliminare un cavo | doppio clic sul cavo, oppure tasto destro → Disconnect |
| Regolare knob / fader | trascinare (Shift = regolazione fine), doppio clic = reset |
| Selezionare più nodi | trascinare un rettangolo sul canvas vuoto, oppure Shift+clic (Cmd+A tutti, Esc nessuno) |
| Spostare la vista | scroll a due dita, trascinare col tasto destro o centrale, oppure Alt+trascina |
| Zoom | pinch o Cmd+scroll; Cmd+= / Cmd+− / Cmd+0 (100%); oppure clic sulla percentuale nella barra di stato |
| Spostare / eliminare / duplicare la selezione | trascinare un nodo selezionato / Del / Cmd+D (i cavi interni vengono copiati) |
| Creare un modulo dalla selezione | Cmd+G, oppure tasto destro → Make module |
| Rinominare un modulo | Invio o F2 con il modulo selezionato, oppure tasto destro → Rename… (Invio conferma, Esc annulla) |
| Sciogliere un modulo | Cmd+Shift+G, la × nell'intestazione del modulo, oppure tasto destro → Ungroup |
| Ridurre / espandere un nodo o un modulo | freccia nell'intestazione, doppio clic sul titolo, tasto C, oppure tasto destro → Collapse |
| Ridurre / espandere tutto | Shift+C, pulsante nella toolbar, oppure tasto destro sul canvas |
| Inquadrare tutto / la selezione | F / Shift+F |
| Auto layout | L, il pulsante nella toolbar o tasto destro sul canvas; Cmd+Z annulla |
| Auto-fit (tenere tutta la patch in vista) | A o il pulsante nella toolbar |
| Nuovo / apri progetto | Cmd+N / Cmd+O, oppure il nome del progetto nella toolbar |
| Salvare / salvare con nome | Cmd+S / Shift+Cmd+S |
| Audio on/off | Spazio |
| Mostrare / nascondere oscilloscopio e spettro | pulsante pannello nella toolbar |

L'elenco completo delle scorciatoie compare passando il mouse sul pulsante **?** della toolbar.

Il tasto destro (o Ctrl+clic su macOS) apre il menu di ciò che si trova sotto il puntatore: canvas, nodo
(anche cliccando su un knob o un display), modulo o cavo.

Un nodo ridotto occupa solo una striscia con titolo, porte e valore principale (frequenza, cutoff, RT…);
il mixer ridotto mostra anche i meter del master. Cavi e audio continuano a funzionare normalmente.

I collegamenti che creerebbero un loop di feedback vengono rifiutati; ogni ingresso accetta un solo cavo.
Il feedback si ottiene dentro i nodi (Delay, Reverb).

### Moduli

Un **modulo** raggruppa più nodi in uno solo, per spostarli, ridurli e duplicarli insieme.
Si crea selezionando i nodi e premendo Cmd+G. Per rinominarlo selezionalo e premi Invio (o F2, o tasto destro →
Rename…): il nome si modifica direttamente nell'intestazione.

- **Espanso**: una cornice attorno ai nodi, che restano modificabili sul posto. Trascinando l'intestazione
  si sposta tutto il modulo; la cornice si adatta quando sposti un nodo al suo interno.
- **Ridotto**: un'unica scheda che mostra solo le porte di confine, cioè gli ingressi non alimentati da un
  altro nodo del modulo e le uscite libere o collegate all'esterno (es. `DRONE 1 · PITCH`, `FILTER 1 · OUT`).
  I cavi esterni si attaccano alla scheda e si possono collegare e staccare come su un nodo normale.
- **Duplicare** un modulo (selezionalo cliccando l'intestazione, poi Cmd+D) copia i nodi, i cavi tra loro e
  il modulo stesso. **Del** elimina il modulo con i suoi nodi; **Ungroup** lo scioglie e tiene i nodi.

Il raggruppamento è solo visivo: il suono non cambia. Un nodo appartiene al massimo a un modulo.

## Auto layout e zoom

**Auto layout** (L) dispone la patch seguendo il flusso del segnale: colonne da sinistra a destra, dalle
sorgenti al mixer. In ogni colonna i blocchi si allineano alle porte a cui sono collegati, così i cavi sono
il più dritti possibile; i modulatori (Random, LFO, nodi MIDI) finiscono subito prima di ciò che modulano;
i cavi che saltano una colonna hanno un corridoio libero, così non passano dietro altri nodi. I moduli si
spostano come blocchi interi e vengono riordinati anche all'interno. Con due o più nodi selezionati (o un
modulo) viene sistemata solo la selezione. I nodi scivolano nelle nuove posizioni e la vista si adatta al
risultato; **Cmd+Z** riporta la disposizione precedente.

**Zoom.** F adatta la vista a tutta la patch, Shift+F alla selezione, con un'animazione breve. Cliccando la
percentuale di zoom nella barra di stato si apre il menu della vista. **Auto-fit** (A) tiene sempre tutta la
patch in vista mentre aggiungi, togli, riduci o sposti nodi o ridimensioni la finestra; nella barra di stato
compare "auto". Si spegne da solo appena fai zoom o sposti la vista a mano.

## Palette

Il pannello a sinistra (si mostra e si nasconde con il pulsante accanto al nome del progetto) elenca tutti i
nodi per categoria e i moduli della libreria, con un campo di ricerca. Trascina una voce sul canvas per
posizionarla dove vuoi (un riquadro mostra dove atterra), oppure cliccala per aggiungerla al centro della
vista. Sulle voci della libreria, la × rimuove il modulo dalla libreria.

## Progetti e libreria

**Progetti.** Il nome del progetto nella toolbar apre il menu File (New project, Open…, Save, Save as…).
Un pallino accanto al nome, e "(edited)" nel titolo della finestra, segnalano modifiche non salvate:
prima di creare un nuovo progetto, aprirne un altro o chiudere la finestra, AudioBoard chiede se salvarle.
Un progetto (`.audioboard`) è un file JSON con nodi e parametri, posizioni, colori, nodi ridotti,
cavi e moduli; i parametri sono salvati per nome. All'avvio si apre sempre la patch demo.

**Libreria dei moduli.** Tasto destro sull'intestazione di un modulo → **Save to library** lo salva con
il suo nome (se esiste già, chiede se sostituirlo). I moduli salvati compaiono nella sezione **Library**
della palette e del menu del tasto destro: inserirne uno aggiunge i nodi, i cavi tra loro e le impostazioni
come nuovo modulo, già **ridotto** a una scheda. La × accanto al nome lo rimuove dalla libreria, e
**Open library folder** apre la cartella dei file `.abmodule`
(`~/Library/Application Support/AudioBoard/Library` su macOS), utile per copiarli o condividerli.

## Architettura

```
src/
├── audio/
│   ├── params.rs   parametri condivisi lock-free (atomics) tra UI e thread audio
│   ├── engine.rs   grafo lato audio: blocchi stereo da 64 frame, ordine topologico precalcolato
│   ├── dsp.rs      Oscillator e Mixer
│   ├── modules/    drone, random, granular, resonator, filter, delay, reverb, midi_cc, midi_note
│   │   └── mod.rs  descrizione dei moduli (ModuleDef), parametri generici e helper DSP
│   ├── midi.rs     ingressi MIDI (midir) e stato condiviso letto dai nodi MIDI
│   ├── plugin.rs   plugin VST3 (vst3-host): caricamento, scansione, elaborazione sul thread audio
│   └── mod.rs      apertura dello stream di uscita (cpal)
├── graph.rs        modello della patch lato UI (nodi, cavi, moduli, ordinamento di Kahn)
├── project.rs      salvataggio/caricamento dei progetti e libreria dei moduli (JSON)
├── ui/
│   ├── canvas.rs   canvas con pan/zoom, selezione, drag & drop dei cavi, menu contestuali
│   ├── groups.rs   moduli: cornice (espansi) e scheda con le porte di confine (ridotti)
│   ├── layout.rs   auto layout per flusso del segnale (colonne, porte allineate, corridoi per i cavi)
│   ├── palette.rs  palette dei nodi e della libreria, con trascinamento sul canvas
│   ├── plugin_node.rs pannello dei nodi plugin (editor, parametri, MIDI, knob)
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
