//! Impostazioni persistenti (file JSON accanto al programma, compatibile con la versione Python).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const CONFIG_FILE: &str = "meditation_config.json";
pub const DEFAULT_SPEED: f64 = 0.85;
pub const DEFAULT_PITCH: f64 = 0.0;
pub const DEFAULT_SILENCE: f64 = 1.0;
pub const SPEED_RANGE: (f64, f64) = (0.5, 1.8);
pub const PITCH_RANGE: (f64, f64) = (-8.0, 8.0);
pub const SILENCE_RANGE: (f64, f64) = (0.3, 6.0);
pub const DEFAULT_WINDOW: &str = "1300x900";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub tts_engine: String,
    /// Nome tecnico della voce Edge (es. "it-IT-IsabellaNeural"); le vecchie etichette vengono convertite.
    pub edge_voice: String,
    pub google_lang: String,
    pub pronunciation_mode: String,
    pub speed: f64,
    pub pitch: f64,
    pub silence: f64,
    pub output_dir: String,
    pub last_text: String,
    #[serde(alias = "normalize")]
    pub normalize_audio: bool,
    pub window_geometry: String,
    pub models: Vec<String>,
    pub piper_model: String,
    pub open_when_done: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            tts_engine: "edge".into(),
            edge_voice: crate::tts::EDGE_DEFAULT_ID.into(),
            google_lang: "it".into(),
            pronunciation_mode: "original".into(),
            speed: DEFAULT_SPEED,
            pitch: DEFAULT_PITCH,
            silence: DEFAULT_SILENCE,
            output_dir: String::new(),
            last_text: String::new(),
            normalize_audio: true,
            window_geometry: DEFAULT_WINDOW.into(),
            models: Vec::new(),
            piper_model: String::new(),
            open_when_done: true,
        }
    }
}

impl AppConfig {
    /// Porta i valori nei limiti ammessi (file modificati a mano o corrotti).
    pub fn sanitized(mut self) -> Self {
        let clamp = |v: f64, r: (f64, f64), d: f64| {
            if v.is_finite() {
                v.clamp(r.0, r.1)
            } else {
                d
            }
        };
        self.speed = clamp(self.speed, SPEED_RANGE, DEFAULT_SPEED);
        self.pitch = clamp(self.pitch, PITCH_RANGE, DEFAULT_PITCH);
        self.silence = clamp(self.silence, SILENCE_RANGE, DEFAULT_SILENCE);
        if !matches!(self.tts_engine.as_str(), "piper" | "edge" | "google") {
            self.tts_engine = "edge".into();
        }
        self
    }

    /// Dimensione finestra salvata come "LxA".
    pub fn window_size(&self) -> (f32, f32) {
        parse_geometry(&self.window_geometry).unwrap_or((1300.0, 900.0))
    }
}

/// Legge "1300x900" (anche "1300x900+10+20" come scrive Tk).
pub fn parse_geometry(s: &str) -> Option<(f32, f32)> {
    let head = s.split(['+', '-']).next()?;
    let (w, h) = head.split_once('x')?;
    let (w, h) = (w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?);
    (w >= 300.0 && h >= 300.0).then_some((w, h))
}

/// Cartella del programma (dove stanno configurazione, log, piper e modelli).
pub fn app_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn load(path: &Path) -> AppConfig {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<AppConfig>(&s).ok())
        .unwrap_or_default()
        .sanitized()
}

pub fn save(path: &Path, cfg: &AppConfig) -> Result<(), String> {
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("medit_cfg_{name}_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir.join(CONFIG_FILE)
    }

    #[test]
    fn round_trip() {
        let path = tmp("rt");
        let cfg = AppConfig {
            speed: 1.2,
            last_text: "ciao\nmondo".into(),
            ..AppConfig::default()
        };
        save(&path, &cfg).unwrap();
        assert_eq!(load(&path), cfg);
    }

    #[test]
    fn missing_or_corrupt_file_gives_defaults() {
        let path = tmp("bad");
        assert_eq!(load(&path), AppConfig::default());
        std::fs::write(&path, "{non json").unwrap();
        assert_eq!(load(&path), AppConfig::default());
    }

    #[test]
    fn old_python_config_is_accepted() {
        let path = tmp("old");
        // File della versione Python: chiave "normalize", campi extra, voce come id.
        std::fs::write(
            &path,
            r#"{"tts_engine":"google","normalize":false,"speed":0.9,"unknown":1,"edge_voice":"it-IT-DiegoNeural"}"#,
        )
        .unwrap();
        let cfg = load(&path);
        assert_eq!(cfg.tts_engine, "google");
        assert!(!cfg.normalize_audio);
        assert!((cfg.speed - 0.9).abs() < 1e-9);
        assert!(cfg.open_when_done);
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let cfg = AppConfig {
            speed: 99.0,
            pitch: f64::NAN,
            silence: -1.0,
            tts_engine: "x".into(),
            ..AppConfig::default()
        }
        .sanitized();
        assert_eq!(cfg.speed, SPEED_RANGE.1);
        assert_eq!(cfg.pitch, DEFAULT_PITCH);
        assert_eq!(cfg.silence, SILENCE_RANGE.0);
        assert_eq!(cfg.tts_engine, "edge");
    }

    #[test]
    fn geometry_parsing() {
        assert_eq!(parse_geometry("1300x900"), Some((1300.0, 900.0)));
        assert_eq!(parse_geometry("1000x700+20+30"), Some((1000.0, 700.0)));
        assert_eq!(parse_geometry("boh"), None);
        assert_eq!(parse_geometry("10x10"), None);
    }
}
