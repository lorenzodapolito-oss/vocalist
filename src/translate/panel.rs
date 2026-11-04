//! Finestra «Traduttore EN → IT» di Vocalist.

use crate::logger::Tag;
use crate::theme;
use crate::translate::translator::Translator;
use crate::translate::{pdf, translate_text};
use eframe::egui::{self, RichText};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

enum Event {
    Loaded,
    LoadFailed(String),
    Progress(usize, usize),
    Done(Result<String, String>),
}

enum Source {
    File(PathBuf),
    Text(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Use {
    File,
    Current,
}

/// Testo di un documento .txt, .docx o .pdf.
pub fn read_document(path: &Path) -> Result<String, String> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("pdf") => pdf::read_pdf(path),
        Some("txt" | "docx") => crate::docx::load_text_file(path),
        _ => Err("formato non supportato (usa txt, docx o pdf)".into()),
    }
}

pub struct TranslatePanel {
    pub open: bool,
    model_dir: PathBuf,
    use_what: Use,
    file: Option<PathBuf>,
    ready: bool,
    loading: bool,
    busy: bool,
    started: bool,
    progress: (usize, usize),
    result: String,
    jobs: Option<Sender<Source>>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    cancel: Arc<AtomicBool>,
    /// Testo da mettere nell'editor di Vocalist.
    pub take: Option<String>,
    /// Messaggi per il registro di Vocalist.
    pub msgs: Vec<(Tag, String)>,
}

impl TranslatePanel {
    pub fn new(base_dir: &Path) -> Self {
        let (tx, rx) = channel();
        Self {
            open: false,
            model_dir: base_dir.join("modello"),
            use_what: Use::File,
            file: None,
            ready: false,
            loading: false,
            busy: false,
            started: false,
            progress: (0, 0),
            result: String::new(),
            jobs: None,
            tx,
            rx,
            cancel: Arc::new(AtomicBool::new(false)),
            take: None,
            msgs: Vec::new(),
        }
    }

    /// Il modello si carica la prima volta che la finestra si apre (occupa memoria e tempo).
    fn start_worker(&mut self) {
        self.started = true;
        self.ready = false;
        self.loading = true;
        let (jtx, jrx) = channel::<Source>();
        self.jobs = Some(jtx);
        let (dir, tx, cancel) = (self.model_dir.clone(), self.tx.clone(), self.cancel.clone());
        std::thread::spawn(move || {
            let mut tr = match Translator::load(&dir) {
                Ok(t) => t,
                Err(e) => {
                    let _ = tx.send(Event::LoadFailed(e));
                    return;
                }
            };
            let _ = tx.send(Event::Loaded);
            while let Ok(src) = jrx.recv() {
                cancel.store(false, Ordering::SeqCst);
                let text = match src {
                    Source::Text(t) => Ok(t),
                    Source::File(p) => read_document(&p),
                };
                let res = text.and_then(|t| {
                    let tx2 = tx.clone();
                    translate_text(&mut tr, &t, &cancel, |d, n| {
                        let _ = tx2.send(Event::Progress(d, n));
                    })
                });
                let _ = tx.send(Event::Done(res));
            }
        });
    }

    fn pump(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Event::Loaded => {
                    self.ready = true;
                    self.loading = false;
                    self.msgs.push((
                        Tag::Success,
                        "Traduttore: modello caricato (calcolo su processore).".into(),
                    ));
                }
                Event::LoadFailed(e) => {
                    self.loading = false;
                    self.msgs
                        .push((Tag::Error, format!("Traduttore: modello non caricato: {e}")));
                }
                Event::Progress(d, n) => self.progress = (d, n),
                Event::Done(res) => {
                    self.busy = false;
                    match res {
                        Ok(t) => {
                            self.msgs.push((
                                Tag::Success,
                                format!("Traduzione completata ({} caratteri).", t.chars().count()),
                            ));
                            self.result = t;
                        }
                        Err(e) => self.msgs.push((Tag::Error, format!("Traduzione: {e}"))),
                    }
                }
            }
        }
    }

    fn start(&mut self, current: &str) {
        let src = match (self.use_what, &self.file) {
            (Use::File, Some(f)) => Source::File(f.clone()),
            (Use::File, None) => return,
            (Use::Current, _) => Source::Text(current.to_string()),
        };
        let Some(jobs) = self.jobs.clone() else {
            return;
        };
        self.busy = true;
        self.progress = (0, 0);
        if jobs.send(src).is_err() {
            self.busy = false;
            self.msgs.push((
                Tag::Error,
                "Traduttore: il modello non è in esecuzione, ricaricalo.".into(),
            ));
        }
    }

    fn save(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("traduzione.txt")
            .add_filter("Testo", &["txt"])
            .save_file()
        else {
            return;
        };
        match std::fs::write(&path, &self.result) {
            Ok(()) => self.msgs.push((
                Tag::Success,
                format!("Traduzione salvata in {}", path.display()),
            )),
            Err(e) => self
                .msgs
                .push((Tag::Error, format!("{}: {e}", path.display()))),
        }
    }

    pub fn window(&mut self, ctx: &egui::Context, current_text: &str) {
        self.pump();
        if !self.open {
            return;
        }
        if !self.started {
            self.start_worker();
        }
        let mut open = self.open;
        egui::Window::new("Traduttore EN → IT")
            .open(&mut open)
            .default_size([620.0, 520.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Modello:");
                    ui.label(RichText::new(self.model_dir.display().to_string()).strong());
                    if ui
                        .add_enabled(!self.busy, egui::Button::new("Cambia…"))
                        .clicked()
                    {
                        if let Some(d) = rfd::FileDialog::new()
                            .set_title("Cartella del modello Marian")
                            .pick_folder()
                        {
                            self.model_dir = d;
                            self.start_worker();
                        }
                    }
                    if self.loading {
                        ui.spinner();
                    } else if self.ready {
                        ui.label(RichText::new("pronto").color(theme::PRIMARY));
                    } else {
                        ui.label(RichText::new("non caricato (vedi il log)").color(theme::ERROR));
                    }
                });
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.use_what, Use::File, "Documento");
                    ui.radio_value(&mut self.use_what, Use::Current, "Testo di Vocalist");
                    if self.use_what == Use::File {
                        if ui
                            .add_enabled(!self.busy, egui::Button::new("Scegli file…"))
                            .clicked()
                        {
                            self.file = rfd::FileDialog::new()
                                .add_filter("Documenti", &["txt", "docx", "pdf"])
                                .pick_file();
                        }
                        let name = self
                            .file
                            .as_ref()
                            .and_then(|f| f.file_name())
                            .map(|n| n.to_string_lossy().into_owned());
                        ui.label(name.unwrap_or_else(|| "nessun file".into()));
                    }
                });
                let can = self.ready
                    && !self.busy
                    && (self.use_what == Use::Current && !current_text.trim().is_empty()
                        || self.use_what == Use::File && self.file.is_some());
                ui.horizontal(|ui| {
                    let b = egui::Button::new(
                        RichText::new("Traduci")
                            .strong()
                            .color(egui::Color32::WHITE),
                    )
                    .fill(theme::PRIMARY)
                    .min_size(egui::vec2(120.0, 30.0));
                    if ui.add_enabled(can, b).clicked() {
                        self.start(current_text);
                    }
                    if self.busy && ui.button("Interrompi").clicked() {
                        self.cancel.store(true, Ordering::SeqCst);
                    }
                });
                let (d, n) = self.progress;
                if self.busy || n > 0 {
                    let frac = if n > 0 { d as f32 / n as f32 } else { 0.0 };
                    ui.add(egui::ProgressBar::new(frac).text(format!("{d} / {n} pezzi")));
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.result)
                                .desired_width(f32::INFINITY)
                                .desired_rows(12)
                                .hint_text("Qui compare la traduzione"),
                        );
                    });
                ui.horizontal(|ui| {
                    let has = !self.result.trim().is_empty();
                    if ui
                        .add_enabled(has, egui::Button::new("Usa come testo di Vocalist"))
                        .clicked()
                    {
                        self.take = Some(self.result.clone());
                    }
                    if ui
                        .add_enabled(has, egui::Button::new("Salva .txt…"))
                        .clicked()
                    {
                        self.save();
                    }
                });
            });
        self.open = open;
        if self.busy || self.loading {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
    }
}
