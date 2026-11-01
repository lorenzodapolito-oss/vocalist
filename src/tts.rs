//! Motori di sintesi vocale: Piper (locale), Edge (cloud) e Google (cloud).

use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::audio::no_window;

pub const EDGE_VOICES: [(&str, &str); 8] = [
    ("Isabella (F, Calda)", "it-IT-IsabellaNeural"),
    ("Elsa (F)", "it-IT-ElsaNeural"),
    ("Diego (M)", "it-IT-DiegoNeural"),
    ("Fiamma (F)", "it-IT-FiammaNeural"),
    ("Giuseppe (M)", "it-IT-GiuseppeNeural"),
    ("Aria (F, EN)", "en-US-AriaNeural"),
    ("Emily (F, EN)", "en-US-EmilyNeural"),
    ("Brian (M, EN)", "en-US-BrianNeural"),
];
pub const EDGE_DEFAULT_ID: &str = "it-IT-IsabellaNeural";
pub const GOOGLE_LANGUAGES: [&str; 10] = [
    "it", "en", "fr", "es", "de", "pt", "ru", "ja", "ko", "zh-CN",
];

const PIPER_NOISE_SCALE: &str = "0.667";
const PIPER_NOISE_W: &str = "0.8";
const PIPER_TIMEOUT: Duration = Duration::from_secs(60);

/// Identificativo della voce Edge a partire da etichetta (o già id).
pub fn edge_voice_id(label_or_id: &str) -> String {
    if let Some((_, id)) = EDGE_VOICES
        .iter()
        .find(|(l, id)| *l == label_or_id || *id == label_or_id)
    {
        return (*id).to_string();
    }
    // Un nome tecnico del tipo `fr-FR-DeniseNeural` (voci scaricate dall'elenco completo).
    let mut parts = label_or_id.splitn(3, '-');
    let looks_like_id = matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(l), Some(r), Some(n))
            if (2..=3).contains(&l.len()) && l.chars().all(|c| c.is_ascii_lowercase())
                && (2..=4).contains(&r.len()) && r.chars().all(|c| c.is_ascii_alphabetic())
                && !n.is_empty() && !label_or_id.contains(' ')
    );
    if looks_like_id {
        label_or_id.to_string()
    } else {
        EDGE_DEFAULT_ID.to_string()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TtsError {
    Network(String),
    Synthesis(String),
    Validation(String),
}

impl fmt::Display for TtsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(m) | Self::Synthesis(m) | Self::Validation(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for TtsError {}

#[derive(Clone, Debug)]
pub struct EngineParams {
    pub speed: f64,
    pub pitch: f64,
    pub piper_model_path: Option<PathBuf>,
    pub edge_voice: String,
    pub google_lang: String,
}

impl Default for EngineParams {
    fn default() -> Self {
        Self {
            speed: 1.0,
            pitch: 0.0,
            piper_model_path: None,
            edge_voice: EDGE_DEFAULT_ID.into(),
            google_lang: "it".into(),
        }
    }
}

pub trait TtsEngine: Send + Sync {
    fn key(&self) -> &'static str;
    fn output_ext(&self) -> &'static str;
    fn supports_pitch(&self) -> bool;
    fn max_text_length(&self) -> usize {
        0
    }
    fn is_available(&self) -> bool;
    fn synthesize_to_file(
        &self,
        text: &str,
        output: &Path,
        params: &EngineParams,
    ) -> Result<(), TtsError>;

    fn validate_text(&self, text: &str) -> Result<(), TtsError> {
        let max = self.max_text_length();
        let len = text.chars().count();
        if max > 0 && len > max {
            return Err(TtsError::Validation(format!(
                "Testo troppo lungo: {len} caratteri (max {max})"
            )));
        }
        Ok(())
    }
}

fn looks_like_network_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    [
        "connection",
        "timeout",
        "timed out",
        "dns",
        "socket",
        "connect",
        "network",
        "tls",
        "429",
        "403",
        "io error",
    ]
    .iter()
    .any(|k| m.contains(k))
}

// ---------------------------------------------------------------- Piper

pub struct PiperEngine {
    pub exe: Option<PathBuf>,
}

impl TtsEngine for PiperEngine {
    fn key(&self) -> &'static str {
        "piper"
    }
    fn output_ext(&self) -> &'static str {
        "wav"
    }
    fn supports_pitch(&self) -> bool {
        true
    }
    fn is_available(&self) -> bool {
        self.exe.as_deref().is_some_and(Path::exists)
    }

    fn synthesize_to_file(
        &self,
        text: &str,
        output: &Path,
        params: &EngineParams,
    ) -> Result<(), TtsError> {
        let exe = self
            .exe
            .as_deref()
            .filter(|p| p.exists())
            .ok_or_else(|| TtsError::Synthesis("Piper: eseguibile non trovato".into()))?;
        let model = params
            .piper_model_path
            .as_deref()
            .filter(|p| p.exists())
            .ok_or_else(|| TtsError::Synthesis("Piper: modello non trovato".into()))?;

        let length_scale = format!("{:.4}", 1.0 / params.speed.max(0.5));
        let mut cmd = Command::new(exe);
        cmd.arg("--model")
            .arg(model)
            .arg("--output_file")
            .arg(output)
            .args(["--length_scale", &length_scale])
            .args(["--noise_scale", PIPER_NOISE_SCALE])
            .args(["--noise_w", PIPER_NOISE_W])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        no_window(&mut cmd);
        let mut child = cmd
            .spawn()
            .map_err(|e| TtsError::Synthesis(format!("Impossibile avviare piper: {e}")))?;

        if let Some(mut stdin) = child.stdin.take() {
            // Se piper si chiude prima, l'errore emerge dal codice di uscita.
            let _ = stdin.write_all(text.as_bytes());
        }
        let mut stderr = child.stderr.take();
        let reader = std::thread::spawn(move || {
            let mut buf = String::new();
            if let Some(s) = stderr.as_mut() {
                let _ = s.read_to_string(&mut buf);
            }
            buf
        });

        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) => {
                    if start.elapsed() > PIPER_TIMEOUT {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(TtsError::Synthesis("Piper: timeout (60s)".into()));
                    }
                    std::thread::sleep(Duration::from_millis(30));
                }
                Err(e) => return Err(TtsError::Synthesis(format!("Piper: {e}"))),
            }
        };
        let err_text = reader.join().unwrap_or_default();
        if !status.success() || !output.exists() {
            let msg = err_text.trim();
            return Err(TtsError::Synthesis(format!(
                "Piper fallito: {}",
                if msg.is_empty() {
                    "errore sconosciuto"
                } else {
                    msg
                }
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- Edge

pub struct EdgeEngine;

pub fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// Variazione di velocità in percentuale come la vuole Edge (`speed` 0.85 → -15).
pub fn edge_rate_percent(speed: f64) -> i32 {
    ((speed - 1.0) * 100.0).trunc() as i32
}

impl TtsEngine for EdgeEngine {
    fn key(&self) -> &'static str {
        "edge"
    }
    fn output_ext(&self) -> &'static str {
        "mp3"
    }
    fn supports_pitch(&self) -> bool {
        false
    }
    fn is_available(&self) -> bool {
        true
    }

    fn synthesize_to_file(
        &self,
        text: &str,
        output: &Path,
        params: &EngineParams,
    ) -> Result<(), TtsError> {
        let config = msedge_tts::tts::SpeechConfig {
            voice_name: edge_voice_id(&params.edge_voice),
            audio_format: "audio-24khz-48kbitrate-mono-mp3".to_string(),
            pitch: 0,
            rate: edge_rate_percent(params.speed),
            volume: 0,
        };
        let wrap = |e: String| {
            if looks_like_network_error(&e) {
                TtsError::Network(format!("Errore di rete Edge TTS: {e}"))
            } else {
                TtsError::Synthesis(format!("Edge TTS fallito: {e}"))
            }
        };
        let mut client = msedge_tts::tts::client::connect().map_err(|e| wrap(e.to_string()))?;
        let audio = client
            .synthesize(&xml_escape(text), &config)
            .map_err(|e| wrap(e.to_string()))?;
        if audio.audio_bytes.is_empty() {
            return Err(TtsError::Synthesis(
                "Edge TTS: nessun audio ricevuto".into(),
            ));
        }
        std::fs::write(output, &audio.audio_bytes)
            .map_err(|e| TtsError::Synthesis(format!("Edge TTS: {e}")))
    }
}

// ---------------------------------------------------------------- Google

pub struct GoogleEngine;

const GOOGLE_RPC: &str = "jQ1olc";
const GOOGLE_URL: &str = "https://translate.google.com/_/TranslateWebserverUi/data/batchexecute";
const GOOGLE_CHUNK: usize = 100;

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Corpo della richiesta `batchexecute` per un frammento di testo.
pub fn google_request_body(text: &str, lang: &str) -> String {
    let param = serde_json::json!([text, lang, null, "null"]).to_string();
    let rpc = serde_json::json!([[[GOOGLE_RPC, param, null, "generic"]]]).to_string();
    format!("f.req={}&", percent_encode(&rpc))
}

/// Estrae l'audio (base64) dalla risposta di Google.
pub fn google_extract_audio(response: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let marker = "jQ1olc\",\"[\\\"";
    for line in response.lines() {
        let Some(pos) = line.find(marker) else {
            continue;
        };
        let rest = &line[pos + marker.len()..];
        let end = rest.find("\\\"]")?;
        return base64::engine::general_purpose::STANDARD
            .decode(&rest.as_bytes()[..end])
            .ok();
    }
    None
}

/// Spezza il testo in frammenti da al massimo `max` caratteri, tagliando su spazi.
pub fn google_chunks(text: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let wlen = word.chars().count();
        if wlen > max {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            let chars: Vec<char> = word.chars().collect();
            out.extend(chars.chunks(max).map(|c| c.iter().collect::<String>()));
            continue;
        }
        if cur.chars().count() + wlen + 1 > max && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

impl TtsEngine for GoogleEngine {
    fn key(&self) -> &'static str {
        "google"
    }
    fn output_ext(&self) -> &'static str {
        "mp3"
    }
    fn supports_pitch(&self) -> bool {
        false
    }
    fn max_text_length(&self) -> usize {
        4000
    }
    fn is_available(&self) -> bool {
        true
    }

    fn synthesize_to_file(
        &self,
        text: &str,
        output: &Path,
        params: &EngineParams,
    ) -> Result<(), TtsError> {
        self.validate_text(text)?;
        let wrap = |e: String| {
            if looks_like_network_error(&e) {
                TtsError::Network(format!("Errore di rete Google TTS: {e}"))
            } else {
                TtsError::Synthesis(format!("Google TTS fallito: {e}"))
            }
        };
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        let mut audio: Vec<u8> = Vec::new();
        for chunk in google_chunks(text, GOOGLE_CHUNK) {
            let body = google_request_body(&chunk, &params.google_lang);
            let mut resp = agent
                .post(GOOGLE_URL)
                .query("rpcids", GOOGLE_RPC)
                .query("rt", "c")
                .header("Content-Type", "application/x-www-form-urlencoded;charset=utf-8")
                .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120 Safari/537.36")
                .send(body)
                .map_err(|e| wrap(e.to_string()))?;
            let text = resp
                .body_mut()
                .read_to_string()
                .map_err(|e| wrap(e.to_string()))?;
            let bytes = google_extract_audio(&text).ok_or_else(|| {
                TtsError::Synthesis("Google TTS fallito: risposta senza audio".into())
            })?;
            audio.extend_from_slice(&bytes);
        }
        if audio.is_empty() {
            return Err(TtsError::Synthesis("Google TTS: testo vuoto".into()));
        }
        std::fs::write(output, audio).map_err(|e| TtsError::Synthesis(format!("Google TTS: {e}")))
    }
}

// ---------------------------------------------------------------- Registro

/// Percorsi in cui cercare `piper`.
pub fn find_piper(base: &Path) -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["piper.exe"]
    } else {
        &["piper"]
    };
    let dirs = [
        base.to_path_buf(),
        base.join("piper_win"),
        base.join("piper"),
    ];
    dirs.iter()
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find(|p| p.is_file())
}

/// File `.onnx` nella cartella del programma (e in `models/`).
pub fn find_models(base: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for dir in [base.to_path_buf(), base.join("models")] {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.to_ascii_lowercase().ends_with(".onnx") {
                    let rel = if dir == base {
                        name
                    } else {
                        format!("models/{name}")
                    };
                    out.push(rel);
                }
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_voice_lookup() {
        assert_eq!(edge_voice_id("Diego (M)"), "it-IT-DiegoNeural");
        assert_eq!(edge_voice_id("it-IT-ElsaNeural"), "it-IT-ElsaNeural");
        assert_eq!(edge_voice_id("sconosciuta"), EDGE_DEFAULT_ID);
        assert_eq!(edge_voice_id("fr-FR-DeniseNeural"), "fr-FR-DeniseNeural");
        assert_eq!(
            edge_voice_id("it-IT-GiuseppeMultilingualNeural"),
            "it-IT-GiuseppeMultilingualNeural"
        );
        assert_eq!(edge_voice_id("Voce strana (F)"), EDGE_DEFAULT_ID);
    }

    #[test]
    fn edge_rate_matches_python_int_truncation() {
        assert_eq!(edge_rate_percent(0.85), -15);
        assert_eq!(edge_rate_percent(1.0), 0);
        assert_eq!(edge_rate_percent(1.8), 80);
        assert_eq!(edge_rate_percent(0.5), -50);
    }

    #[test]
    fn xml_is_escaped() {
        assert_eq!(
            xml_escape("a & b <c> \"d\" 'e'"),
            "a &amp; b &lt;c&gt; &quot;d&quot; &apos;e&apos;"
        );
    }

    #[test]
    fn google_body_has_expected_shape() {
        let b = google_request_body("ciao", "it");
        assert!(b.starts_with("f.req="));
        assert!(b.ends_with('&'));
        assert!(b.contains("jQ1olc"));
        assert!(!b.contains(' ') && !b.contains('"'));
        assert!(b.contains("ciao"));
    }

    #[test]
    fn google_audio_is_extracted_from_response() {
        use base64::Engine as _;
        let payload = base64::engine::general_purpose::STANDARD.encode(b"ID3audiobytes");
        let line =
            format!("[[\"wrb.fr\",\"jQ1olc\",\"[\\\"{payload}\\\"]\",null,null,null,\"generic\"]]");
        let resp = format!(")]}}'\n\n123\n{line}\n25\n[[\"e\",4]]");
        assert_eq!(google_extract_audio(&resp).unwrap(), b"ID3audiobytes");
        assert!(google_extract_audio("niente").is_none());
    }

    #[test]
    fn google_chunks_respect_limit_and_keep_words() {
        let text = vec!["parola"; 60].join(" ");
        let chunks = google_chunks(&text, 100);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|c| c.chars().count() <= 100));
        assert_eq!(chunks.join(" "), text);
        let giant = "a".repeat(250);
        assert_eq!(google_chunks(&giant, 100).len(), 3);
        assert!(google_chunks("   ", 100).is_empty());
    }

    #[test]
    fn google_rejects_overlong_text_before_any_network_call() {
        let long = "a".repeat(4001);
        let err = GoogleEngine
            .synthesize_to_file(&long, Path::new("/tmp/x.mp3"), &EngineParams::default())
            .unwrap_err();
        assert!(matches!(err, TtsError::Validation(_)));
    }

    #[test]
    fn piper_reports_missing_exe_and_model() {
        let p = PiperEngine { exe: None };
        assert!(!p.is_available());
        let err = p
            .synthesize_to_file("x", Path::new("/tmp/x.wav"), &EngineParams::default())
            .unwrap_err();
        assert!(err.to_string().contains("eseguibile"));
    }

    #[test]
    fn piper_runs_external_program_with_text_on_stdin() {
        // Finto "piper": uno script che copia un WAV preparato nel file di uscita.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = std::env::temp_dir().join(format!("medit_piper_{}", std::process::id()));
            let _ = std::fs::create_dir_all(&dir);
            let model = dir.join("voce.onnx");
            std::fs::write(&model, b"x").unwrap();
            let exe = dir.join("piper");
            std::fs::write(
                &exe,
                "#!/bin/sh\nwhile [ $# -gt 0 ]; do [ \"$1\" = \"--output_file\" ] && out=\"$2\"; shift; done\ncat > \"$out\"\n",
            )
            .unwrap();
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
            let engine = PiperEngine { exe: Some(exe) };
            let out = dir.join("out.wav");
            let params = EngineParams {
                piper_model_path: Some(model),
                ..EngineParams::default()
            };
            engine
                .synthesize_to_file("testo di prova", &out, &params)
                .unwrap();
            assert_eq!(std::fs::read_to_string(&out).unwrap(), "testo di prova");
        }
    }

    #[test]
    fn piper_failure_is_reported() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = std::env::temp_dir().join(format!("medit_piperf_{}", std::process::id()));
            let _ = std::fs::create_dir_all(&dir);
            let model = dir.join("voce.onnx");
            std::fs::write(&model, b"x").unwrap();
            let exe = dir.join("piper");
            std::fs::write(&exe, "#!/bin/sh\necho modello rotto >&2\nexit 3\n").unwrap();
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
            let engine = PiperEngine { exe: Some(exe) };
            let params = EngineParams {
                piper_model_path: Some(model),
                ..EngineParams::default()
            };
            let err = engine
                .synthesize_to_file("x", &dir.join("o.wav"), &params)
                .unwrap_err();
            assert!(err.to_string().contains("modello rotto"), "{err}");
        }
    }

    #[test]
    fn network_error_detection() {
        assert!(looks_like_network_error("Connection refused"));
        assert!(looks_like_network_error("operation timed out"));
        assert!(!looks_like_network_error("voce inesistente"));
    }

    #[test]
    fn finds_models_and_piper() {
        let dir = std::env::temp_dir().join(format!("medit_find_{}", std::process::id()));
        let _ = std::fs::create_dir_all(dir.join("models"));
        std::fs::write(dir.join("a.onnx"), b"").unwrap();
        std::fs::write(dir.join("models").join("b.ONNX"), b"").unwrap();
        std::fs::write(dir.join("c.txt"), b"").unwrap();
        assert_eq!(
            find_models(&dir),
            vec!["a.onnx".to_string(), "models/b.ONNX".to_string()]
        );
        assert!(find_piper(&dir).is_none());
    }
}
