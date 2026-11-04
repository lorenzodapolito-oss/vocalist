# Vocalist

Generatore di meditazioni guidate con voce sintetica, scritto in Rust (nato come porta di `deep.py`, "Meditazioni Guidate PRO").

* **Motori**: Piper (locale, serve `piper.exe` + modelli `.onnx` accanto al programma), Edge TTS e Google TTS (cloud, serve internet).
* **Testo**: ogni a capo è una pausa; `[PAUSA 5 SECONDI]` impone una pausa precisa; i testi lunghi vengono spezzati.
* **Pronuncia**: correzione parole ambigue, allungamento vocali, pause extra dopo le frasi.
* **Uscita**: WAV, oppure MP3 se `ffmpeg` è accanto al programma o nel PATH.
* Normalizzazione del volume, tono (solo Piper), annulla, stima durata/ETA, anteprima di lettura, test voce, log con filtro.
* Impostazioni in `meditation_config.json` accanto al programma (compatibile con la versione Python).

```
cargo build --release
cargo test --no-default-features   # senza riproduzione audio (su Linux serve libasound2-dev per compilarla)
```

## Traduttore EN → IT (offline)

Il pulsante «Traduttore EN → IT» in alto apre una finestra che traduce documenti `.txt`, `.docx`, `.pdf` o il testo corrente di Vocalist dall'inglese all'italiano, tutto in Rust (modello Marian con candle, senza Python). La traduzione si può salvare in `.txt` o usare subito come testo da leggere con «Usa come testo di Vocalist».

Metti la cartella del modello (per esempio `Helsinki-NLP/opus-mt-en-it`) accanto al programma con il nome `modello`, oppure sceglila con «Cambia…». Servono `config.json`, `model.safetensors` (o `pytorch_model.bin`), `source.spm` e `vocab.json`. Il modello si carica la prima volta che apri la finestra. Il calcolo è su processore e la generazione è «greedy» (una ipotesi per volta).
