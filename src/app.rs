//! Interfaccia grafica: tre colonne (motore/voce, testo, parametri/controlli) e registro in basso.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, FontId, RichText, TextFormat};

use crate::audio;
use crate::config::{self, AppConfig, PITCH_RANGE, SILENCE_RANGE, SPEED_RANGE};
use crate::docx;
use crate::generate::{self, GenerationRequest, GenerationResult};
use crate::logger::{Filter, LogBuffer, LogLine, Logger, Tag};
use crate::player::{open_with_default_app, AudioPlayer};
use crate::text::{self, PronunciationMode, Segment};
use crate::theme;
use crate::translate::panel::TranslatePanel;
use crate::tts::{self, EngineParams, TtsEngine};
use crate::voices::{self, EdgeVoice, PiperVoice};

pub const APP_TITLE: &str = "Vocalist";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const AUTO_SAVE_EVERY: Duration = Duration::from_millis(500);
const TEST_SENTENCE: &str = "Ciao, questa è una prova della voce.";

/// Messaggi dai thread di lavoro verso l'interfaccia.
enum Event {
    Progress {
        ok: usize,
        total: usize,
        eta: f64,
    },
    Done(Box<GenerationResult>),
    TestReady(PathBuf),
    TestFailed(String),
    PreviewShow {
        content: String,
        status: String,
    },
    PreviewStatus(String),
    PreviewEnd {
        stopped: bool,
    },
    EdgeVoices(Result<Vec<EdgeVoice>, String>),
    PiperIndex(Result<Vec<PiperVoice>, String>),
    PiperProgress(u64),
    PiperRuntimeProgress(u64),
    PiperRuntimeDone(Result<PathBuf, String>),
    PiperDone {
        key: String,
        result: Result<(), String>,
    },
}

/// Finestra per scaricare nuove voci Piper.
struct PiperWin {
    voices: Vec<PiperVoice>,
    loading: bool,
    error: Option<String>,
    lang: String,
    /// (chiave voce, byte scaricati) mentre scarica.
    downloading: Option<(String, u64)>,
    cancel: Arc<AtomicBool>,
}

struct Preview {
    stop: Arc<AtomicBool>,
    content: String,
    status: String,
    finished: bool,
    finished_at: Option<Instant>,
}

pub struct App {
    translate: TranslatePanel,
    base_dir: PathBuf,
    cfg_path: PathBuf,
    cfg: AppConfig,
    saved: AppConfig,
    last_save_check: Instant,

    text: String,
    output_file: String,
    models: Vec<String>,

    engines: HashMap<&'static str, Arc<dyn TtsEngine>>,
    piper_exe: Option<PathBuf>,
    ffmpeg: Option<PathBuf>,

    player: AudioPlayer,
    logger: Logger,
    log_rx: Receiver<LogLine>,
    log_buf: LogBuffer,
    log_filter: Filter,

    ev_tx: Sender<Event>,
    ev_rx: Receiver<Event>,
    cancel: Arc<AtomicBool>,
    generating: bool,
    progress: f32,
    eta_text: String,
    status: String,
    status_color: Color32,
    preview: Option<Preview>,
    window_size: (f32, f32),
    edge_voices: Vec<EdgeVoice>,
    edge_loading: bool,
    edge_filter: String,
    piper_win: Option<PiperWin>,
    /// Byte scaricati mentre si installa il programma Piper.
    piper_rt: Option<u64>,
    piper_rt_cancel: Arc<AtomicBool>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let base_dir = config::app_dir();
        let cfg_path = base_dir.join(config::CONFIG_FILE);
        let cfg = config::load(&cfg_path);

        let (log_tx, log_rx) = channel();
        let logger = Logger::new(log_tx, Some(base_dir.join("logs")));
        let (ev_tx, ev_rx) = channel();

        let piper_exe = tts::find_piper(&base_dir);
        let ffmpeg = audio::find_ffmpeg();
        let mut engines: HashMap<&'static str, Arc<dyn TtsEngine>> = HashMap::new();
        engines.insert(
            "piper",
            Arc::new(tts::PiperEngine {
                exe: piper_exe.clone(),
            }),
        );
        engines.insert("edge", Arc::new(tts::EdgeEngine));
        engines.insert("google", Arc::new(tts::GoogleEngine));

        let models = tts::find_models(&base_dir);
        let text = if cfg.last_text.trim().is_empty() {
            text::EXAMPLE_TEXT.to_string()
        } else {
            cfg.last_text.clone()
        };
        let default_name = if cfg.output_dir.is_empty() {
            base_dir.join("meditazione.wav")
        } else {
            Path::new(&cfg.output_dir).join("meditazione.wav")
        };

        let translate = TranslatePanel::new(&base_dir);
        let mut app = Self {
            base_dir,
            cfg_path,
            saved: cfg.clone(),
            window_size: cfg.window_size(),
            cfg,
            last_save_check: Instant::now(),
            text,
            output_file: default_name.to_string_lossy().into_owned(),
            models,
            engines,
            piper_exe,
            ffmpeg,
            player: AudioPlayer::default(),
            logger,
            log_rx,
            log_buf: LogBuffer::default(),
            log_filter: Filter::All,
            ev_tx,
            ev_rx,
            cancel: Arc::new(AtomicBool::new(false)),
            generating: false,
            progress: 0.0,
            eta_text: String::new(),
            status: "Pronto".into(),
            status_color: theme::PRIMARY,
            preview: None,
            edge_voices: Vec::new(),
            edge_loading: false,
            edge_filter: "it-IT".into(),
            translate,
            piper_win: None,
            piper_rt: None,
            piper_rt_cancel: Arc::new(AtomicBool::new(false)),
        };
        if !app.models.is_empty() {
            app.logger
                .info(format!("Trovati {} modelli Piper", app.models.len()));
            if !app.models.contains(&app.cfg.piper_model) {
                app.cfg.piper_model = app.models[0].clone();
            }
        } else {
            app.logger
                .warn("Nessun modello .onnx trovato (serve solo per Piper)");
        }
        if app.ffmpeg.is_none() {
            app.logger
                .warn("ffmpeg non trovato: i file MP3 verranno salvati in WAV");
        }
        app.sync_output_extension();
        app.cfg.edge_voice = tts::edge_voice_id(&app.cfg.edge_voice);
        app.edge_voices = voices::load_edge_cache(&app.base_dir);
        if app.edge_voices.is_empty() {
            app.edge_voices = builtin_edge_voices();
        }
        app.refresh_edge_voices();
        app
    }

    /// Scarica in background l'elenco aggiornato delle voci Edge.
    fn refresh_edge_voices(&mut self) {
        if self.edge_loading {
            return;
        }
        self.edge_loading = true;
        let tx = self.ev_tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Event::EdgeVoices(voices::fetch_edge_voices()));
        });
    }

    fn edge_label_for(&self, id: &str) -> String {
        self.edge_voices
            .iter()
            .find(|v| v.id == id)
            .map_or_else(|| id.to_string(), |v| v.label.clone())
    }

    fn start_piper_runtime_install(&mut self) {
        if self.piper_rt.is_some() {
            return;
        }
        self.piper_rt = Some(0);
        self.piper_rt_cancel.store(false, Ordering::SeqCst);
        let cancel = self.piper_rt_cancel.clone();
        let tx = self.ev_tx.clone();
        let base = self.base_dir.clone();
        self.logger
            .info("Scarico il programma Piper (circa 22 MB)...");
        std::thread::spawn(move || {
            let tx_p = tx.clone();
            let res = voices::install_piper_runtime(
                voices::PIPER_RUNTIME_URL,
                &base,
                &mut |b| {
                    let _ = tx_p.send(Event::PiperRuntimeProgress(b));
                },
                &|| cancel.load(Ordering::SeqCst),
            );
            let _ = tx.send(Event::PiperRuntimeDone(res));
        });
    }

    fn open_piper_window(&mut self) {
        if self.piper_win.is_some() {
            return;
        }
        self.piper_win = Some(PiperWin {
            voices: Vec::new(),
            loading: true,
            error: None,
            lang: "it_IT".into(),
            downloading: None,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        let tx = self.ev_tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Event::PiperIndex(voices::fetch_piper_index()));
        });
    }

    fn start_piper_download(&mut self, voice: PiperVoice) {
        let Some(win) = &mut self.piper_win else {
            return;
        };
        if win.downloading.is_some() {
            return;
        }
        win.cancel.store(false, Ordering::SeqCst);
        win.downloading = Some((voice.key.clone(), 0));
        let cancel = win.cancel.clone();
        let tx = self.ev_tx.clone();
        let dir = self.base_dir.join("models");
        self.logger
            .info(format!("Scarico la voce Piper {}...", voice.key));
        std::thread::spawn(move || {
            let tx_p = tx.clone();
            let result = voices::download_piper_voice(
                &voice,
                &dir,
                &mut |b| {
                    let _ = tx_p.send(Event::PiperProgress(b));
                },
                &|| cancel.load(Ordering::SeqCst),
            );
            let _ = tx.send(Event::PiperDone {
                key: voice.key.clone(),
                result,
            });
        });
    }

    pub fn initial_window_size() -> (f32, f32) {
        config::load(&config::app_dir().join(config::CONFIG_FILE)).window_size()
    }

    // ------------------------------------------------------------ stato

    fn engine(&self) -> Arc<dyn TtsEngine> {
        self.engines
            .get(self.cfg.tts_engine.as_str())
            .or_else(|| self.engines.get("edge"))
            .cloned()
            .unwrap_or_else(|| Arc::new(tts::EdgeEngine))
    }

    fn mode(&self) -> PronunciationMode {
        PronunciationMode::from_key(&self.cfg.pronunciation_mode)
    }

    fn snapshot(&self) -> AppConfig {
        let mut c = self.cfg.clone();
        c.last_text = self.text.trim().to_string();
        c.output_dir = Path::new(&self.output_file)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        c.window_geometry = format!(
            "{}x{}",
            self.window_size.0 as u32, self.window_size.1 as u32
        );
        c.models = self.models.clone();
        c
    }

    fn save_now(&mut self) {
        let snap = self.snapshot();
        if snap != self.saved {
            if let Err(e) = config::save(&self.cfg_path, &snap) {
                self.logger
                    .warn(format!("Errore salvataggio configurazione: {e}"));
            }
            self.saved = snap;
        }
    }

    /// Estensione del file di uscita coerente con il motore scelto.
    fn sync_output_extension(&mut self) {
        let mut ext = self.engine().output_ext();
        if ext == "mp3" && self.ffmpeg.is_none() {
            ext = "wav";
        }
        self.output_file = Path::new(&self.output_file)
            .with_extension(ext)
            .to_string_lossy()
            .into_owned();
    }

    fn set_status(&mut self, text: impl Into<String>, color: Color32) {
        self.status = text.into();
        self.status_color = color;
    }

    fn engine_params(&self) -> EngineParams {
        let piper_model_path = (self.cfg.tts_engine == "piper" && !self.cfg.piper_model.is_empty())
            .then(|| self.base_dir.join(&self.cfg.piper_model));
        EngineParams {
            speed: self.cfg.speed,
            pitch: self.cfg.pitch,
            piper_model_path,
            edge_voice: tts::edge_voice_id(&self.cfg.edge_voice),
            google_lang: self.cfg.google_lang.clone(),
        }
    }

    fn validate_engine_ready(&self) -> Result<(), String> {
        let engine = self.engine();
        if !engine.is_available() {
            return Err(format!(
                "Motore {} non disponibile! Metti piper.exe accanto al programma.",
                engine.key()
            ));
        }
        if engine.key() == "piper" && self.cfg.piper_model.is_empty() {
            return Err("Seleziona un modello Piper!".into());
        }
        Ok(())
    }

    fn alert(&self, title: &str, msg: &str, level: rfd::MessageLevel) {
        rfd::MessageDialog::new()
            .set_title(title)
            .set_description(msg)
            .set_level(level)
            .show();
    }

    // ------------------------------------------------------------ azioni

    fn start_generation(&mut self) {
        let text = self.text.trim().to_string();
        if text.is_empty() {
            self.alert("Errore", "Inserisci del testo!", rfd::MessageLevel::Error);
            return;
        }
        if let Err(e) = self.validate_engine_ready() {
            self.alert("Errore", &e, rfd::MessageLevel::Error);
            return;
        }
        let engine = self.engine();
        if engine.max_text_length() > 0 && text.chars().count() > engine.max_text_length() {
            let go = rfd::MessageDialog::new()
                .set_title("Testo lungo")
                .set_description(format!(
                    "Il testo è molto lungo ({} caratteri). Google TTS potrebbe avere problemi. Continuare?",
                    text.chars().count()
                ))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show();
            if go != rfd::MessageDialogResult::Yes {
                return;
            }
        }
        self.save_now();
        self.generating = true;
        self.cancel.store(false, Ordering::SeqCst);
        self.progress = 0.0;
        self.eta_text.clear();
        self.set_status("Generazione in corso...", theme::PRIMARY);
        self.logger
            .info(format!("Generazione con {}...", engine.key()));

        let req = GenerationRequest {
            text,
            engine,
            params: self.engine_params(),
            mode: self.mode(),
            output_path: PathBuf::from(&self.output_file),
            pause_duration: self.cfg.silence,
            normalize: self.cfg.normalize_audio,
            ffmpeg: self.ffmpeg.clone(),
        };
        let tx = self.ev_tx.clone();
        let cancel = self.cancel.clone();
        std::thread::spawn(move || {
            let tx_p = tx.clone();
            let res = generate::generate(
                &req,
                &mut |ok, total, eta| {
                    let _ = tx_p.send(Event::Progress { ok, total, eta });
                },
                &|| cancel.load(Ordering::SeqCst),
            );
            let _ = tx.send(Event::Done(Box::new(res)));
        });
    }

    fn cancel_generation(&mut self) {
        if self.generating {
            self.cancel.store(true, Ordering::SeqCst);
            self.set_status("Annullamento in corso...", theme::MUTED);
            self.logger.info("Annullamento richiesto dall'utente");
        }
    }

    fn on_done(&mut self, res: GenerationResult) {
        self.generating = false;
        for w in &res.warnings {
            self.logger.warn(w);
        }
        if res.success {
            self.output_file = res.output_path.to_string_lossy().into_owned();
            self.progress = 1.0;
            self.eta_text = format!(
                "{} - {}",
                text::format_file_size(res.file_size_bytes),
                text::format_time(res.duration_seconds)
            );
            self.set_status("Completato!", theme::PRIMARY);
            self.logger.success(format!(
                "COMPLETATO! ({:.2} MB, {}/{} segmenti, {:.1}s) -> {}",
                res.file_size_bytes as f64 / 1_048_576.0,
                res.segments_ok,
                res.segments_total,
                res.duration_seconds,
                res.output_path.display()
            ));
            if self.cfg.open_when_done {
                if let Err(e) = open_with_default_app(&res.output_path) {
                    self.logger.warn(format!("Impossibile aprire il file: {e}"));
                }
            }
        } else if res.error.as_deref() == Some(generate::CANCELLED) {
            self.logger.info("Generazione annullata dall'utente");
            self.set_status("Annullato", theme::MUTED);
        } else {
            let err = res.error.unwrap_or_else(|| "sconosciuto".into());
            self.logger.error(format!("Nessun audio generato: {err}"));
            self.set_status("Errore", theme::ERROR);
        }
        self.save_now();
    }

    fn test_voice(&mut self) {
        if let Err(e) = self.validate_engine_ready() {
            self.alert("Errore", &e, rfd::MessageLevel::Error);
            return;
        }
        let engine = self.engine();
        let params = self.engine_params();
        let tx = self.ev_tx.clone();
        self.set_status("Test in corso...", theme::PRIMARY);
        std::thread::spawn(move || {
            let dir = std::env::temp_dir().join(format!(
                "test_voice_{}_{}",
                engine.key(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            ));
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join(format!("prova.{}", engine.output_ext()));
            match engine.synthesize_to_file(TEST_SENTENCE, &path, &params) {
                Ok(()) => {
                    let _ = tx.send(Event::TestReady(path));
                }
                Err(e) => {
                    let _ = tx.send(Event::TestFailed(e.to_string()));
                }
            }
        });
    }

    fn start_preview(&mut self) {
        let text = self.text.trim().to_string();
        if text.is_empty() {
            self.alert(
                "Attenzione",
                "Inserisci del testo!",
                rfd::MessageLevel::Warning,
            );
            return;
        }
        if let Err(e) = self.validate_engine_ready() {
            self.alert("Errore", &e, rfd::MessageLevel::Error);
            return;
        }
        let engine = self.engine();
        let params = self.engine_params();
        let mode = self.mode();
        let pause = self.cfg.silence;
        let segments = text::insert_line_break_pauses(&text, pause, text::MAX_CHARS_PER_SEGMENT);
        let stop = Arc::new(AtomicBool::new(false));
        self.preview = Some(Preview {
            stop: stop.clone(),
            content: String::new(),
            status: "Avvio...".into(),
            finished: false,
            finished_at: None,
        });
        self.player.stop();
        let tx = self.ev_tx.clone();
        let logger = self.logger.clone();
        std::thread::spawn(move || {
            run_preview(segments, engine, params, mode, pause, stop, tx, logger)
        });
    }

    fn load_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Seleziona file")
            .add_filter("File supportati", &["txt", "docx"])
            .add_filter("Tutti", &["*"])
            .pick_file()
        else {
            return;
        };
        match docx::load_text_file(&path) {
            Ok(content) => {
                self.text = content;
                self.logger.info(format!(
                    "Caricato {}",
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                ));
                self.save_now();
            }
            Err(e) => {
                self.logger.error(format!("Errore caricamento file: {e}"));
                self.alert(
                    "Errore",
                    &format!("Impossibile aprire: {e}"),
                    rfd::MessageLevel::Error,
                );
            }
        }
    }

    fn choose_output(&mut self) {
        let ext = self.engine().output_ext();
        let current = PathBuf::from(&self.output_file);
        let mut dlg = rfd::FileDialog::new()
            .set_title("Salva audio")
            .add_filter("WAV", &["wav"]);
        if self.ffmpeg.is_some() {
            dlg = dlg.add_filter("MP3", &["mp3"]);
        }
        if let Some(dir) = current.parent() {
            dlg = dlg.set_directory(dir);
        }
        dlg = dlg.set_file_name(format!("meditazione.{ext}"));
        if let Some(path) = dlg.save_file() {
            self.output_file = path.to_string_lossy().into_owned();
            self.save_now();
        }
    }

    fn play_output(&mut self) {
        let path = PathBuf::from(&self.output_file);
        if !path.exists() {
            self.alert(
                "Attenzione",
                "Genera prima l'audio!",
                rfd::MessageLevel::Warning,
            );
            return;
        }
        match self.player.play(&path) {
            Ok(()) => self.set_status("Riproduzione...", theme::PRIMARY),
            Err(e) => {
                self.logger.error(format!("Errore riproduzione: {e}"));
                self.set_status("Errore riproduzione", theme::ERROR);
            }
        }
    }

    // ------------------------------------------------------------ eventi

    fn pump_events(&mut self) {
        while let Ok(line) = self.log_rx.try_recv() {
            self.log_buf.push(line);
        }
        while let Ok(ev) = self.ev_rx.try_recv() {
            match ev {
                Event::Progress { ok, total, eta } => {
                    self.logger.info(format!("Segmento {ok}/{total} elaborato"));
                    if total > 0 {
                        self.progress = ok as f32 / total as f32;
                    }
                    if eta > 0.0 {
                        self.eta_text = format!("{} rimanenti", text::format_time(eta));
                    }
                }
                Event::Done(res) => self.on_done(*res),
                Event::TestReady(path) => match self.player.play(&path) {
                    Ok(()) => self.set_status("Test in corso...", theme::PRIMARY),
                    Err(e) => {
                        self.logger.error(format!("Test fallito: {e}"));
                        self.set_status("Test fallito", theme::ERROR);
                    }
                },
                Event::TestFailed(e) => {
                    self.logger.error(format!("Test fallito: {e}"));
                    self.set_status("Test fallito", theme::ERROR);
                }
                Event::PreviewShow { content, status } => {
                    if let Some(p) = &mut self.preview {
                        p.content = content;
                        p.status = status;
                    }
                }
                Event::PreviewStatus(s) => {
                    if let Some(p) = &mut self.preview {
                        p.status = s;
                    }
                }
                Event::EdgeVoices(res) => {
                    self.edge_loading = false;
                    match res {
                        Ok(list) => {
                            self.logger
                                .info(format!("Elenco voci Edge aggiornato: {} voci", list.len()));
                            voices::save_edge_cache(&self.base_dir, &list);
                            self.edge_voices = list;
                        }
                        Err(e) => self.logger.warn(format!(
                            "Elenco voci Edge non aggiornato ({e}): uso quello salvato"
                        )),
                    }
                }
                Event::PiperIndex(res) => {
                    if let Some(w) = &mut self.piper_win {
                        w.loading = false;
                        match res {
                            Ok(list) => {
                                if !list.iter().any(|v| v.lang == w.lang) {
                                    w.lang =
                                        list.first().map(|v| v.lang.clone()).unwrap_or_default();
                                }
                                w.voices = list;
                            }
                            Err(e) => w.error = Some(e),
                        }
                    }
                }
                Event::PiperRuntimeProgress(b) => self.piper_rt = Some(b),
                Event::PiperRuntimeDone(res) => {
                    self.piper_rt = None;
                    match res {
                        Ok(exe) => {
                            self.logger
                                .success(format!("Piper installato: {}", exe.display()));
                            self.piper_exe = Some(exe.clone());
                            self.engines
                                .insert("piper", Arc::new(tts::PiperEngine { exe: Some(exe) }));
                        }
                        Err(e) => self
                            .logger
                            .error(format!("Installazione di Piper fallita: {e}")),
                    }
                }
                Event::PiperProgress(b) => {
                    if let Some((_, done)) =
                        self.piper_win.as_mut().and_then(|w| w.downloading.as_mut())
                    {
                        *done = b;
                    }
                }
                Event::PiperDone { key, result } => {
                    if let Some(w) = &mut self.piper_win {
                        w.downloading = None;
                    }
                    match result {
                        Ok(()) => {
                            self.logger.success(format!("Voce Piper {key} installata"));
                            self.models = tts::find_models(&self.base_dir);
                            if let Some(m) = self.models.iter().find(|m| m.contains(&key)) {
                                self.cfg.piper_model = m.clone();
                            }
                        }
                        Err(e) => self
                            .logger
                            .error(format!("Scaricamento di {key} fallito: {e}")),
                    }
                }
                Event::PreviewEnd { stopped } => {
                    if let Some(p) = &mut self.preview {
                        p.status = if stopped {
                            "Fermato".into()
                        } else {
                            "Lettura completata".into()
                        };
                        p.finished = true;
                        p.finished_at = Some(Instant::now());
                    }
                }
            }
        }
        if self.player.poll_end() {
            self.set_status("Riproduzione completata", theme::PRIMARY);
        }
    }

    // ------------------------------------------------------------ disegno

    fn card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
        egui::Frame::new()
            .fill(theme::SURFACE)
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .corner_radius(10)
            .inner_margin(16)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(title).strong().size(16.0));
                ui.add_space(6.0);
                add(ui)
            })
            .inner
    }

    fn big_button(
        ui: &mut egui::Ui,
        label: &str,
        fill: Color32,
        fg: Color32,
        enabled: bool,
        h: f32,
    ) -> egui::Response {
        let b = egui::Button::new(RichText::new(label).strong().color(fg))
            .fill(if enabled { fill } else { theme::DISABLED })
            .corner_radius(6);
        ui.add_enabled_ui(enabled, |ui| ui.add_sized([ui.available_width(), h], b))
            .inner
    }

    fn slider_row(
        ui: &mut egui::Ui,
        label: &str,
        value: &mut f64,
        range: (f64, f64),
        shown: String,
    ) -> bool {
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(shown).strong());
            });
        });
        ui.spacing_mut().slider_width = ui.available_width();
        let changed = ui
            .add(egui::Slider::new(value, range.0..=range.1).show_value(false))
            .changed();
        ui.add_space(8.0);
        changed
    }

    fn left_column(&mut self, ui: &mut egui::Ui) {
        Self::card(ui, "Motore di Sintesi", |ui| {
            let piper_ok = self.engines.get("piper").is_some_and(|e| e.is_available());
            let mut changed = false;
            for (key, label, enabled) in [
                ("piper", "Piper (Locale)", piper_ok),
                ("edge", "Edge (Cloud)", true),
                ("google", "Google (Cloud)", true),
            ] {
                let sel = self.cfg.tts_engine == key;
                let r = ui.add_enabled(enabled, egui::RadioButton::new(sel, label));
                if r.clicked() && !sel {
                    self.cfg.tts_engine = key.into();
                    changed = true;
                }
                if !enabled {
                    r.on_disabled_hover_text("Metti piper.exe accanto al programma");
                }
            }
            if changed {
                self.sync_output_extension();
            }
            if !piper_ok && cfg!(windows) {
                ui.add_space(4.0);
                match self.piper_rt {
                    Some(done) => {
                        ui.label(
                            RichText::new(format!("Scarico Piper... {:.1} MB", done as f64 / 1_048_576.0))
                                .color(theme::MUTED),
                        );
                        if ui.button("Annulla").clicked() {
                            self.piper_rt_cancel.store(true, Ordering::SeqCst);
                        }
                    }
                    None => {
                        if Self::big_button(ui, "Scarica Piper (programma)", theme::PRIMARY, Color32::WHITE, true, 32.0)
                            .on_hover_text("Scarica piper.exe (circa 22 MB) nella cartella 'piper' accanto al programma")
                            .clicked()
                        {
                            self.start_piper_runtime_install();
                        }
                    }
                }
            }
        });
        ui.add_space(8.0);

        Self::card(ui, "Modalità Pronuncia", |ui| {
            for m in PronunciationMode::ALL {
                let sel = self.mode() == m;
                if ui.radio(sel, m.label()).on_hover_text(m.info()).clicked() {
                    self.cfg.pronunciation_mode = m.key().into();
                }
            }
            ui.label(
                RichText::new(self.mode().info())
                    .color(theme::MUTED)
                    .size(12.0),
            );
        });
        ui.add_space(8.0);

        Self::card(ui, "Configurazione Voce", |ui| {
            match self.cfg.tts_engine.as_str() {
                "piper" => {
                    ui.label("Modello Piper (.onnx):");
                    let current = if self.cfg.piper_model.is_empty() {
                        "—".to_string()
                    } else {
                        self.cfg.piper_model.clone()
                    };
                    egui::ComboBox::from_id_salt("model")
                        .width(ui.available_width() - 8.0)
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            for m in &self.models {
                                ui.selectable_value(&mut self.cfg.piper_model, m.clone(), m);
                            }
                        });
                    if Self::big_button(
                        ui,
                        "Aggiorna Modelli",
                        theme::SECONDARY,
                        theme::TEXT,
                        true,
                        32.0,
                    )
                    .clicked()
                    {
                        self.models = tts::find_models(&self.base_dir);
                        self.logger
                            .info(format!("Trovati {} modelli Piper", self.models.len()));
                    }
                    if Self::big_button(
                        ui,
                        "Scarica nuove voci Piper...",
                        theme::PRIMARY,
                        Color32::WHITE,
                        true,
                        32.0,
                    )
                    .clicked()
                    {
                        self.open_piper_window();
                    }
                }
                "google" => {
                    ui.label("Lingua:");
                    egui::ComboBox::from_id_salt("glang")
                        .width(ui.available_width() - 8.0)
                        .selected_text(self.cfg.google_lang.clone())
                        .show_ui(ui, |ui| {
                            for l in tts::GOOGLE_LANGUAGES {
                                ui.selectable_value(&mut self.cfg.google_lang, l.to_string(), l);
                            }
                        });
                }
                _ => {
                    ui.label("Voce Microsoft Neural:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.edge_filter)
                            .hint_text("Filtra: it-IT, en, Multilingual...")
                            .desired_width(f32::INFINITY),
                    );
                    let selected = self.edge_label_for(&self.cfg.edge_voice);
                    let filter = self.edge_filter.trim().to_lowercase();
                    egui::ComboBox::from_id_salt("evoice")
                        .width(ui.available_width() - 8.0)
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            let mut shown = 0usize;
                            for v in &self.edge_voices {
                                let hay = format!("{} {}", v.label, v.id).to_lowercase();
                                if filter.is_empty() || hay.contains(&filter) {
                                    shown += 1;
                                    ui.selectable_value(
                                        &mut self.cfg.edge_voice,
                                        v.id.clone(),
                                        &v.label,
                                    );
                                }
                            }
                            if shown == 0 {
                                ui.label("Nessuna voce con questo filtro");
                            }
                        });
                    ui.label(
                        RichText::new(format!("{} voci disponibili", self.edge_voices.len()))
                            .color(theme::MUTED)
                            .size(12.0),
                    );
                    let label = if self.edge_loading {
                        "Aggiorno..."
                    } else {
                        "Aggiorna elenco voci"
                    };
                    if Self::big_button(
                        ui,
                        label,
                        theme::SECONDARY,
                        theme::TEXT,
                        !self.edge_loading,
                        32.0,
                    )
                    .clicked()
                    {
                        self.refresh_edge_voices();
                    }
                }
            }
        });
        ui.add_space(8.0);

        Self::card(ui, "Gestione File", |ui| {
            if Self::big_button(
                ui,
                "Carica TXT/DOCX",
                theme::PRIMARY,
                Color32::WHITE,
                true,
                34.0,
            )
            .on_hover_text("Carica un file di testo o DOCX")
            .clicked()
            {
                self.load_file();
            }
            if Self::big_button(
                ui,
                "Cancella Testo",
                theme::ERROR,
                Color32::WHITE,
                true,
                34.0,
            )
            .clicked()
            {
                self.text.clear();
            }
            ui.add_space(6.0);
            ui.add(egui::TextEdit::singleline(&mut self.output_file).desired_width(f32::INFINITY));
            if Self::big_button(
                ui,
                "Scegli Percorso Output",
                theme::SECONDARY,
                theme::TEXT,
                true,
                34.0,
            )
            .clicked()
            {
                self.choose_output();
            }
        });
    }

    fn center_column(&mut self, ui: &mut egui::Ui) {
        Self::card(ui, "Testo della Meditazione", |ui| {
            egui::Frame::new().fill(theme::PRIMARY_LIGHT).corner_radius(6).inner_margin(10).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("Ogni a capo = pausa. Usa [PAUSA 5 SECONDI] per una pausa precisa. I testi lunghi vengono spezzati automaticamente.")
                        .strong(),
                );
            });
            ui.add_space(8.0);
            let size = egui::vec2(
                ui.available_width(),
                (ui.available_height() - 4.0).max(200.0),
            );
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let mut layouter = |ui: &egui::Ui, s: &dyn egui::TextBuffer, wrap: f32| {
                        let job = highlight_job(s.as_str(), wrap);
                        ui.fonts_mut(|f| f.layout_job(job))
                    };
                    ui.add_sized(
                        size,
                        egui::TextEdit::multiline(&mut self.text)
                            .layouter(&mut layouter)
                            .font(FontId::proportional(16.0))
                            .desired_width(f32::INFINITY),
                    );
                });
        });
    }

    fn right_column(&mut self, ui: &mut egui::Ui) {
        Self::card(ui, "Parametri Audio", |ui| {
            let s = format!("{:.2}x", self.cfg.speed);
            Self::slider_row(ui, "Velocità:", &mut self.cfg.speed, SPEED_RANGE, s);
            let s = format!("{:.1}s", self.cfg.silence);
            Self::slider_row(
                ui,
                "Pausa tra frasi:",
                &mut self.cfg.silence,
                SILENCE_RANGE,
                s,
            );
            let pitch_ok = self.engine().supports_pitch();
            ui.add_enabled_ui(pitch_ok, |ui| {
                let s = format!("{:.1}", self.cfg.pitch);
                Self::slider_row(
                    ui,
                    "Tono (solo Piper):",
                    &mut self.cfg.pitch,
                    PITCH_RANGE,
                    s,
                );
            });
            ui.checkbox(&mut self.cfg.normalize_audio, "Normalizza volume finale");
            ui.checkbox(&mut self.cfg.open_when_done, "Apri il file al termine");
        });
        ui.add_space(8.0);

        Self::card(ui, "Controlli", |ui| {
            let gen_label = if self.generating {
                "Generazione..."
            } else {
                "GENERA AUDIO"
            };
            if Self::big_button(
                ui,
                gen_label,
                theme::PRIMARY,
                Color32::WHITE,
                !self.generating,
                44.0,
            )
            .clicked()
            {
                self.start_generation();
            }
            if Self::big_button(
                ui,
                "✖ ANNULLA",
                theme::ERROR,
                Color32::WHITE,
                self.generating,
                44.0,
            )
            .clicked()
            {
                self.cancel_generation();
            }
            ui.add_space(4.0);
            ui.add(
                egui::ProgressBar::new(self.progress)
                    .fill(theme::PRIMARY)
                    .desired_height(14.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                ui.label(RichText::new(&self.eta_text).color(theme::MUTED).size(12.0));
            });
            let est = text::estimate_audio_duration(&self.text, self.cfg.speed, self.cfg.silence);
            let est_text = if self.text.trim().is_empty() {
                "--:--".to_string()
            } else {
                text::format_time(est)
            };
            ui.label(RichText::new(format!("Durata stimata: {est_text}")).color(theme::MUTED));
            ui.add_space(4.0);
            if Self::big_button(
                ui,
                "Anteprima Lettura",
                theme::SECONDARY,
                theme::TEXT,
                self.preview.is_none(),
                34.0,
            )
            .on_hover_text("Ascolta il testo segmento per segmento")
            .clicked()
            {
                self.start_preview();
            }
            if Self::big_button(ui, "Test Voce", theme::SECONDARY, theme::TEXT, true, 34.0)
                .on_hover_text("Test rapido della voce selezionata")
                .clicked()
            {
                self.test_voice();
            }
        });
        ui.add_space(8.0);

        Self::card(ui, "Controllo Riproduzione", |ui| {
            let has_file = Path::new(&self.output_file).exists();
            ui.columns(3, |cols| {
                let play_label = if self.player.is_paused {
                    "▶ Riprendi"
                } else {
                    "▶ Play"
                };
                let play_enabled = has_file && !(self.player.is_playing && !self.player.is_paused);
                if Self::big_button(
                    &mut cols[0],
                    play_label,
                    theme::PRIMARY,
                    Color32::WHITE,
                    play_enabled,
                    34.0,
                )
                .clicked()
                {
                    self.play_output();
                }
                if Self::big_button(
                    &mut cols[1],
                    "⏸ Pausa",
                    theme::PRIMARY,
                    Color32::WHITE,
                    self.player.is_playing,
                    34.0,
                )
                .clicked()
                {
                    self.player.pause_toggle();
                    let msg = if self.player.is_paused {
                        "Pausa"
                    } else {
                        "Riproduzione..."
                    };
                    self.set_status(msg, theme::PRIMARY);
                }
                if Self::big_button(
                    &mut cols[2],
                    "⏹ Stop",
                    theme::ERROR,
                    Color32::WHITE,
                    self.player.is_playing,
                    34.0,
                )
                .clicked()
                {
                    self.player.stop();
                    self.set_status("Stop", theme::MUTED);
                }
            });
        });
        ui.add_space(14.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(&self.status)
                    .strong()
                    .size(16.0)
                    .color(self.status_color),
            );
        });
    }

    fn log_panel(&mut self, ui: &mut egui::Ui) {
        Self::card(ui, "Log Operazioni", |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Filtro:").color(theme::MUTED));
                ui.radio_value(&mut self.log_filter, Filter::All, "Tutti");
                ui.radio_value(&mut self.log_filter, Filter::Errors, "Errori");
                if ui.button("Cancella").clicked() {
                    self.log_buf.clear();
                }
            });
            egui::Frame::new()
                .fill(theme::LOG_BG)
                .corner_radius(6)
                .inner_margin(10)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .max_height(56.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for l in self.log_buf.filtered(self.log_filter) {
                                let color = match l.tag {
                                    Tag::Info => theme::LOG_INFO,
                                    Tag::Warning => theme::LOG_WARN,
                                    Tag::Error => theme::LOG_ERR,
                                    Tag::Success => theme::LOG_OK,
                                };
                                ui.label(RichText::new(&l.text).color(color).monospace());
                            }
                        });
                });
        });
    }

    fn piper_window(&mut self, ctx: &egui::Context) {
        let models_dir = self.base_dir.join("models");
        let mut close = false;
        let mut to_download: Option<PiperVoice> = None;
        let mut cancel_download = false;
        let Some(win) = &mut self.piper_win else {
            return;
        };
        egui::Window::new("Voci Piper")
            .collapsible(false)
            .resizable(true)
            .default_size([560.0, 460.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                if win.loading {
                    ui.label("Scarico il catalogo delle voci...");
                    return;
                }
                if let Some(e) = &win.error {
                    ui.label(
                        RichText::new(format!("Catalogo non disponibile: {e}")).color(theme::ERROR),
                    );
                    if ui.button("Chiudi").clicked() {
                        close = true;
                    }
                    return;
                }
                ui.label(
                    "Le voci di qualità medium/high suonano meglio; le x_low/low sono più veloci.",
                );
                let langs = voices::piper_languages(&win.voices);
                let current = langs
                    .iter()
                    .find(|(c, _)| *c == win.lang)
                    .map_or(win.lang.clone(), |(c, n)| format!("{n} ({c})"));
                ui.horizontal(|ui| {
                    ui.label("Lingua:");
                    egui::ComboBox::from_id_salt("piperlang")
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            for (code, name) in &langs {
                                ui.selectable_value(
                                    &mut win.lang,
                                    code.clone(),
                                    format!("{name} ({code})"),
                                );
                            }
                        });
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for v in win.voices.iter().filter(|v| v.lang == win.lang) {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&v.name).strong());
                                ui.label(RichText::new(&v.quality).color(theme::MUTED));
                                ui.label(
                                    RichText::new(format!(
                                        "{:.0} MB",
                                        v.size_bytes as f64 / 1_048_576.0
                                    ))
                                    .color(theme::MUTED),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| match &win.downloading {
                                        Some((key, done)) if *key == v.key => {
                                            let frac = if v.size_bytes > 0 {
                                                *done as f32 / v.size_bytes as f32
                                            } else {
                                                0.0
                                            };
                                            if ui.button("Annulla").clicked() {
                                                cancel_download = true;
                                            }
                                            ui.add(
                                                egui::ProgressBar::new(frac.min(1.0))
                                                    .desired_width(110.0)
                                                    .show_percentage(),
                                            );
                                        }
                                        _ if v.is_installed(&models_dir) => {
                                            ui.label(
                                                RichText::new("Installata").color(theme::PRIMARY),
                                            );
                                        }
                                        _ => {
                                            let busy = win.downloading.is_some();
                                            if ui
                                                .add_enabled(!busy, egui::Button::new("Scarica"))
                                                .clicked()
                                            {
                                                to_download = Some(v.clone());
                                            }
                                        }
                                    },
                                );
                            });
                        }
                    });
                ui.separator();
                if ui.button("Chiudi").clicked() {
                    close = true;
                }
            });
        if cancel_download {
            win.cancel.store(true, Ordering::SeqCst);
        }
        if close {
            win.cancel.store(true, Ordering::SeqCst);
            self.piper_win = None;
        }
        if let Some(v) = to_download {
            self.start_piper_download(v);
        }
    }

    fn preview_window(&mut self, ctx: &egui::Context) {
        let Some(p) = &self.preview else { return };
        let (content, status, finished, stop) = (
            p.content.clone(),
            p.status.clone(),
            p.finished,
            p.stop.clone(),
        );
        let mut close = false;
        egui::Window::new("Anteprima Lettura")
            .collapsible(false)
            .resizable(true)
            .default_size([560.0, 320.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(180.0)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(content)
                                .size(16.0)
                                .strong()
                                .background_color(theme::SECONDARY),
                        );
                    });
                ui.add_space(8.0);
                ui.label(RichText::new(status).strong());
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if Self::big_button(ui, "⏹ Stop", theme::ERROR, Color32::WHITE, !finished, 32.0)
                        .clicked()
                    {
                        stop.store(true, Ordering::SeqCst);
                    }
                    if ui.button("Chiudi").clicked() {
                        close = true;
                    }
                });
            });
        let auto_close = self
            .preview
            .as_ref()
            .and_then(|p| p.finished_at)
            .is_some_and(|t| t.elapsed() > Duration::from_millis(800));
        if close || auto_close {
            stop.store(true, Ordering::SeqCst);
            self.preview = None;
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.pump_events();
        if let Some(r) = ctx.input(|i| i.viewport().inner_rect) {
            self.window_size = (r.width(), r.height());
        }
        if self.last_save_check.elapsed() >= AUTO_SAVE_EVERY {
            self.last_save_check = Instant::now();
            self.save_now();
        }

        egui::Panel::top("top").show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(20, 12))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(APP_TITLE)
                                .size(24.0)
                                .strong()
                                .color(theme::PRIMARY),
                        );
                        ui.label(RichText::new(format!("v{APP_VERSION}")).color(theme::MUTED));
                        if ui.button("Traduttore EN → IT").clicked() {
                            self.translate.open = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let piper_ok = self.piper_exe.is_some();
                            for (ok, name) in [
                                (true, "Google TTS"),
                                (true, "Edge TTS"),
                                (piper_ok, "Piper"),
                            ] {
                                let c = if ok { theme::PRIMARY } else { theme::ERROR };
                                let t = if ok {
                                    format!("• {name}")
                                } else {
                                    format!("• {name} (assente)")
                                };
                                ui.label(RichText::new(t).color(c));
                                ui.add_space(8.0);
                            }
                        });
                    });
                });
        });

        egui::Panel::bottom("log").show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(20, 10))
                .show(ui, |ui| self.log_panel(ui));
        });

        egui::Panel::left("left")
            .exact_size(360.0)
            .resizable(false)
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin {
                        left: 20,
                        right: 8,
                        top: 4,
                        bottom: 8,
                    })
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| self.left_column(ui));
                    });
            });

        egui::Panel::right("right")
            .exact_size(360.0)
            .resizable(false)
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin {
                        left: 8,
                        right: 20,
                        top: 4,
                        bottom: 8,
                    })
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| self.right_column(ui));
                    });
            });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(8, 4))
                .show(ui, |ui| self.center_column(ui));
        });

        self.preview_window(&ctx);
        self.piper_window(&ctx);
        self.translate.window(&ctx, &self.text);
        for (tag, m) in std::mem::take(&mut self.translate.msgs) {
            match tag {
                Tag::Error => self.logger.error(m),
                Tag::Warning => self.logger.warn(m),
                Tag::Success => self.logger.success(m),
                _ => self.logger.info(m),
            }
        }
        if let Some(t) = self.translate.take.take() {
            self.text = t;
            self.logger.success("Testo sostituito con la traduzione.");
        }

        if self.generating || self.player.is_playing || self.preview.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        } else {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.generating {
            self.cancel.store(true, Ordering::SeqCst);
        }
        if let Some(p) = &self.preview {
            p.stop.store(true, Ordering::SeqCst);
        }
        self.player.stop();
        self.save_now();
    }
}

/// Testo con parole ambigue e tag di pausa evidenziati.
pub fn highlight_job(text: &str, wrap_width: f32) -> egui::text::LayoutJob {
    let font = FontId::proportional(16.0);
    let normal = TextFormat {
        font_id: font.clone(),
        color: theme::TEXT,
        ..Default::default()
    };
    let amb = TextFormat {
        font_id: font.clone(),
        color: theme::AMBIGUOUS_FG,
        background: theme::AMBIGUOUS_BG,
        ..Default::default()
    };
    let tag = TextFormat {
        font_id: font,
        color: theme::PRIMARY,
        background: theme::PRIMARY_LIGHT,
        ..Default::default()
    };

    let mut spans: Vec<(std::ops::Range<usize>, bool)> = text::ambiguous_ranges(text)
        .into_iter()
        .map(|r| (r, true))
        .chain(text::pause_tag_ranges(text).into_iter().map(|r| (r, false)))
        .collect();
    spans.sort_by_key(|(r, _)| r.start);

    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap_width;
    let mut pos = 0usize;
    for (range, is_amb) in spans {
        if range.start < pos {
            continue;
        }
        if range.start > pos {
            job.append(&text[pos..range.start], 0.0, normal.clone());
        }
        job.append(
            &text[range.clone()],
            0.0,
            if is_amb { amb.clone() } else { tag.clone() },
        );
        pos = range.end;
    }
    if pos < text.len() {
        job.append(&text[pos..], 0.0, normal);
    }
    job
}

#[allow(clippy::too_many_arguments)]
fn run_preview(
    segments: Vec<Segment>,
    engine: Arc<dyn TtsEngine>,
    params: EngineParams,
    mode: PronunciationMode,
    pause: f64,
    stop: Arc<AtomicBool>,
    tx: Sender<Event>,
    logger: Logger,
) {
    let total = segments
        .iter()
        .filter(|s| matches!(s, Segment::Text(_)))
        .count();
    let mut idx = 0usize;
    let mut player = AudioPlayer::default();
    let dir = match generate::TempDir::new() {
        Ok(d) => d,
        Err(e) => {
            logger.error(format!("Anteprima: {e}"));
            let _ = tx.send(Event::PreviewEnd { stopped: true });
            return;
        }
    };
    for seg in &segments {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        match seg {
            Segment::Text(content) => {
                idx += 1;
                let _ = tx.send(Event::PreviewShow {
                    content: content.clone(),
                    status: format!("Segmento {idx}/{total} — pausa: {pause:.1}s"),
                });
                let processed = text::preprocess_text(content, mode);
                let pcm = generate::synth_segment(
                    engine.as_ref(),
                    &processed,
                    &params,
                    dir.path(),
                    "prev",
                )
                .map(|p| {
                    if engine.supports_pitch() && params.pitch.abs() >= 0.1 {
                        audio::pitch_shift(&p, params.pitch * 0.6)
                    } else {
                        p
                    }
                });
                match pcm {
                    Ok(pcm) => {
                        let wav = dir.path().join(format!("play_{idx}.wav"));
                        if let Err(e) = audio::write_wav(&wav, &pcm) {
                            logger.warn(format!("Errore segmento {idx}: {e}"));
                            continue;
                        }
                        if let Err(e) = player.play(&wav) {
                            logger.warn(format!("Errore segmento {idx}: {e}"));
                            continue;
                        }
                        while player.is_busy() && !stop.load(Ordering::SeqCst) {
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        player.stop();
                    }
                    Err(e) => {
                        logger.warn(format!("Errore segmento {idx}: {e}"));
                        let _ = tx.send(Event::PreviewStatus(format!("Errore segmento {idx}")));
                    }
                }
            }
            Segment::Pause(secs) => {
                let _ = tx.send(Event::PreviewStatus(format!("Pausa {secs:.1}s...")));
                let steps = (secs / 0.1) as usize;
                for _ in 0..steps {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }
    let _ = tx.send(Event::PreviewEnd {
        stopped: stop.load(Ordering::SeqCst),
    });
}

/// Elenco minimo di voci Edge, usato finché non si scarica quello completo.
fn builtin_edge_voices() -> Vec<EdgeVoice> {
    tts::EDGE_VOICES
        .iter()
        .map(|(label, id)| EdgeVoice {
            id: (*id).to_string(),
            label: (*label).to_string(),
            locale: id.splitn(3, '-').take(2).collect::<Vec<_>>().join("-"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_covers_whole_text_in_order() {
        let t = "è ancora qui [PAUSA 3 SECONDI] subito";
        let job = highlight_job(t, 400.0);
        assert_eq!(job.text, t);
        assert_eq!(job.sections.len(), 6);
        assert_eq!(job.sections[1].format.background, theme::AMBIGUOUS_BG);
        assert_eq!(job.sections[3].format.background, theme::PRIMARY_LIGHT);
    }

    #[test]
    fn highlight_plain_text_is_single_section() {
        let job = highlight_job("niente di speciale", 400.0);
        assert_eq!(job.sections.len(), 1);
        assert_eq!(highlight_job("", 400.0).sections.len(), 0);
    }
}
