//! Generazione dell'audio completo: sintesi segmento per segmento, unione ed esportazione.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::audio::{self, Pcm};
use crate::text::{self, PronunciationMode, Segment};
use crate::tts::{EngineParams, TtsEngine};

pub struct GenerationRequest {
    pub text: String,
    pub engine: Arc<dyn TtsEngine>,
    pub params: EngineParams,
    pub mode: PronunciationMode,
    pub output_path: PathBuf,
    pub pause_duration: f64,
    pub normalize: bool,
    pub ffmpeg: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct GenerationResult {
    pub success: bool,
    pub output_path: PathBuf,
    pub segments_total: usize,
    pub segments_ok: usize,
    pub duration_seconds: f64,
    pub file_size_bytes: u64,
    pub error: Option<String>,
    /// Segmenti falliti e avvisi (es. ripiego su WAV).
    pub warnings: Vec<String>,
}

pub const CANCELLED: &str = "Annullato dall'utente";

/// Cartella temporanea cancellata alla fine.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> std::io::Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "vocalist_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Sintetizza un solo pezzo di testo e lo legge come audio.
pub fn synth_segment(
    engine: &dyn TtsEngine,
    text: &str,
    params: &EngineParams,
    dir: &Path,
    name: &str,
) -> Result<Pcm, String> {
    let path = dir.join(format!("{name}.{}", engine.output_ext()));
    engine
        .synthesize_to_file(text, &path, params)
        .map_err(|e| e.to_string())?;
    audio::decode_file(&path)
}

fn fail(
    req: &GenerationRequest,
    total: usize,
    ok: usize,
    msg: impl Into<String>,
) -> GenerationResult {
    GenerationResult {
        success: false,
        output_path: req.output_path.clone(),
        segments_total: total,
        segments_ok: ok,
        error: Some(msg.into()),
        ..GenerationResult::default()
    }
}

/// Genera l'audio. `on_progress(ok, total, secondi_rimanenti)` viene chiamato dopo ogni segmento.
pub fn generate(
    req: &GenerationRequest,
    on_progress: &mut dyn FnMut(usize, usize, f64),
    is_cancelled: &dyn Fn() -> bool,
) -> GenerationResult {
    let start = Instant::now();
    let segments =
        text::insert_line_break_pauses(&req.text, req.pause_duration, text::MAX_CHARS_PER_SEGMENT);
    let total = segments
        .iter()
        .filter(|s| matches!(s, Segment::Text(_)))
        .count();
    if total == 0 {
        return fail(req, 0, 0, "Il testo non contiene nulla da leggere");
    }
    let tmp = match TempDir::new() {
        Ok(t) => t,
        Err(e) => {
            return fail(
                req,
                total,
                0,
                format!("Cartella temporanea non creata: {e}"),
            )
        }
    };

    let mut parts: Vec<Pcm> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut ok = 0usize;
    let mut done = 0usize;

    for (i, seg) in segments.iter().enumerate() {
        if is_cancelled() {
            return fail(req, total, ok, CANCELLED);
        }
        match seg {
            Segment::Pause(secs) => parts.push(Pcm::silence(*secs, 22050)),
            Segment::Text(content) => {
                done += 1;
                let processed = text::preprocess_text(content, req.mode);
                match synth_segment(
                    req.engine.as_ref(),
                    &processed,
                    &req.params,
                    tmp.path(),
                    &format!("seg_{i:04}"),
                ) {
                    Ok(pcm) => {
                        parts.push(pcm);
                        ok += 1;
                    }
                    Err(e) => warnings.push(format!("Segmento {done}/{total} fallito: {e}")),
                }
                let remaining = if done > 0 {
                    start.elapsed().as_secs_f64() / done as f64 * (total - done) as f64
                } else {
                    0.0
                };
                on_progress(done, total, remaining);
            }
        }
    }

    if is_cancelled() {
        return fail(req, total, ok, CANCELLED);
    }
    if ok == 0 {
        let first = warnings.first().cloned().unwrap_or_default();
        let mut r = fail(
            req,
            total,
            0,
            format!("Nessun segmento generato. {first}")
                .trim()
                .to_string(),
        );
        r.warnings = warnings;
        return r;
    }

    let Some(mut merged) = audio::merge(&parts) else {
        return fail(req, total, ok, "Unione segmenti fallita");
    };
    if req.engine.supports_pitch() && req.params.pitch.abs() >= 0.1 {
        merged = audio::pitch_shift(&merged, req.params.pitch * 0.6);
    }
    if req.normalize {
        audio::normalize(&mut merged, 0.5);
    }

    let (out_path, warn) = audio::resolve_output(&req.output_path, req.ffmpeg.as_deref());
    warnings.extend(warn);
    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = audio::export(&merged, &out_path, req.ffmpeg.as_deref()) {
        let mut r = fail(req, total, ok, format!("Salvataggio fallito: {e}"));
        r.warnings = warnings;
        return r;
    }
    let size = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
    GenerationResult {
        success: true,
        output_path: out_path,
        segments_total: total,
        segments_ok: ok,
        duration_seconds: start.elapsed().as_secs_f64(),
        file_size_bytes: size,
        error: None,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tts::TtsError;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// Motore finto: scrive un tono la cui durata dipende dalla lunghezza del testo.
    struct Fake {
        fail_on: Option<&'static str>,
        calls: AtomicUsize,
    }

    impl TtsEngine for Fake {
        fn key(&self) -> &'static str {
            "fake"
        }
        fn output_ext(&self) -> &'static str {
            "wav"
        }
        fn supports_pitch(&self) -> bool {
            true
        }
        fn is_available(&self) -> bool {
            true
        }
        fn synthesize_to_file(
            &self,
            text: &str,
            out: &Path,
            _p: &EngineParams,
        ) -> Result<(), TtsError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_on.is_some_and(|f| text.contains(f)) {
                return Err(TtsError::Network("giù la rete".into()));
            }
            let n = 22050 / 10;
            let samples = (0..n).map(|i| (i as f32 * 0.05).sin() * 0.3).collect();
            audio::write_wav(out, &Pcm::new(22050, samples)).map_err(TtsError::Synthesis)
        }
    }

    fn request(
        text: &str,
        fail_on: Option<&'static str>,
        name: &str,
    ) -> (GenerationRequest, Arc<Fake>) {
        let engine = Arc::new(Fake {
            fail_on,
            calls: AtomicUsize::new(0),
        });
        let dir = std::env::temp_dir().join(format!("medit_gen_{name}_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        (
            GenerationRequest {
                text: text.into(),
                engine: engine.clone(),
                params: EngineParams::default(),
                mode: PronunciationMode::Original,
                output_path: dir.join("out.wav"),
                pause_duration: 1.0,
                normalize: true,
                ffmpeg: None,
            },
            engine,
        )
    }

    #[test]
    fn full_generation_joins_speech_and_pauses() {
        let (req, engine) = request("uno\ndue\ntre", None, "ok");
        let mut progress = Vec::new();
        let res = generate(
            &req,
            &mut |ok, total, _| progress.push((ok, total)),
            &|| false,
        );
        assert!(res.success, "{:?}", res.error);
        assert_eq!(res.segments_ok, 3);
        assert_eq!(engine.calls.load(Ordering::SeqCst), 3);
        assert_eq!(progress, vec![(1, 3), (2, 3), (3, 3)]);
        let out = audio::decode_file(&res.output_path).unwrap();
        // 3 x 0.1 s di voce + 2 x 1 s di pausa.
        assert!((out.duration() - 2.3).abs() < 0.05, "{}", out.duration());
        assert!(res.file_size_bytes > 0);
        // Normalizzato a -0.5 dBFS.
        assert!((out.peak() - 0.944).abs() < 0.01);
    }

    #[test]
    fn failed_segment_is_skipped_but_reported() {
        let (req, _) = request("uno\nROTTO\ntre", Some("ROTTO"), "skip");
        let res = generate(&req, &mut |_, _, _| {}, &|| false);
        assert!(res.success);
        assert_eq!(res.segments_ok, 2);
        assert_eq!(res.segments_total, 3);
        assert!(res.warnings.iter().any(|w| w.contains("giù la rete")));
    }

    #[test]
    fn all_segments_failing_is_an_error() {
        let (req, _) = request("ROTTO\nROTTO", Some("ROTTO"), "allfail");
        let res = generate(&req, &mut |_, _, _| {}, &|| false);
        assert!(!res.success);
        assert!(res.error.unwrap().contains("Nessun segmento"));
    }

    #[test]
    fn cancellation_stops_between_segments() {
        let (req, engine) = request("uno\ndue\ntre\nquattro", None, "cancel");
        let flag = AtomicBool::new(false);
        let res = generate(
            &req,
            &mut |ok, _, _| {
                if ok == 1 {
                    flag.store(true, Ordering::SeqCst)
                }
            },
            &|| flag.load(Ordering::SeqCst),
        );
        assert!(!res.success);
        assert_eq!(res.error.as_deref(), Some(CANCELLED));
        assert_eq!(engine.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn empty_text_is_rejected() {
        let (req, _) = request("  \n \n", None, "empty");
        let res = generate(&req, &mut |_, _, _| {}, &|| false);
        assert!(!res.success);
    }

    #[test]
    fn pitch_is_applied_without_changing_duration() {
        let (mut req, _) = request("uno\ndue", None, "pitch");
        req.params.pitch = 4.0;
        req.normalize = false;
        let res = generate(&req, &mut |_, _, _| {}, &|| false);
        assert!(res.success);
        let out = audio::decode_file(&res.output_path).unwrap();
        assert!((out.duration() - 1.2).abs() < 0.1, "{}", out.duration());
    }

    #[test]
    fn mp3_request_without_ffmpeg_falls_back_to_wav_with_warning() {
        let (mut req, _) = request("uno", None, "mp3");
        req.output_path = req.output_path.with_extension("mp3");
        let res = generate(&req, &mut |_, _, _| {}, &|| false);
        assert!(res.success);
        assert_eq!(
            res.output_path.extension().and_then(|e| e.to_str()),
            Some("wav")
        );
        assert!(res.warnings.iter().any(|w| w.contains("ffmpeg")));
    }

    #[test]
    fn temp_dir_is_removed() {
        let path;
        {
            let t = TempDir::new().unwrap();
            path = t.path().to_path_buf();
            assert!(path.exists());
        }
        assert!(!path.exists());
    }
}
