//! Cataloghi di voci: elenco Edge scaricato dal vivo e catalogo Piper (Hugging Face).

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

pub const PIPER_INDEX_URL: &str =
    "https://huggingface.co/rhasspy/piper-voices/resolve/main/voices.json";
pub const PIPER_BASE_URL: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/main/";
pub const PIPER_RUNTIME_URL: &str =
    "https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_windows_amd64.zip";
pub const EDGE_CACHE_FILE: &str = "edge_voices.json";

// ---------------------------------------------------------------- Edge

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EdgeVoice {
    /// Nome tecnico, es. `it-IT-IsabellaNeural`.
    pub id: String,
    /// Etichetta mostrata, es. `Isabella (F, it-IT)`.
    pub label: String,
    pub locale: String,
}

/// Etichetta leggibile a partire dal nome tecnico.
pub fn edge_label(id: &str, gender: Option<&str>, locale: &str) -> String {
    let name = id.splitn(3, '-').nth(2).unwrap_or(id);
    let name = name.strip_suffix("Neural").unwrap_or(name);
    // "IsabellaMultilingual" -> "Isabella Multilingual"
    let mut pretty = String::new();
    for (i, ch) in name.chars().enumerate() {
        if i > 0 && ch.is_uppercase() {
            pretty.push(' ');
        }
        pretty.push(ch);
    }
    let g = match gender {
        Some("Female") => "F",
        Some("Male") => "M",
        _ => "?",
    };
    format!("{pretty} ({g}, {locale})")
}

/// Elenco ordinato: prima l'italiano, poi l'inglese, poi il resto per lingua.
pub fn sort_edge_voices(voices: &mut [EdgeVoice]) {
    let rank = |v: &EdgeVoice| match v.locale.as_str() {
        l if l.starts_with("it-") => 0,
        l if l.starts_with("en-") => 1,
        _ => 2,
    };
    voices.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.locale.cmp(&b.locale))
            .then_with(|| a.label.cmp(&b.label))
    });
}

/// Scarica l'elenco completo delle voci Edge (richiede internet).
pub fn fetch_edge_voices() -> Result<Vec<EdgeVoice>, String> {
    let list = msedge_tts::voice::get_voices_list().map_err(|e| e.to_string())?;
    let mut out: Vec<EdgeVoice> = list
        .into_iter()
        .filter(|v| v.status.as_deref().is_none_or(|s| s == "GA"))
        .filter_map(|v| {
            let id = v.short_name?;
            let locale = v.locale.unwrap_or_default();
            Some(EdgeVoice {
                label: edge_label(&id, v.gender.as_deref(), &locale),
                id,
                locale,
            })
        })
        .collect();
    sort_edge_voices(&mut out);
    if out.is_empty() {
        return Err("elenco voci vuoto".into());
    }
    Ok(out)
}

pub fn load_edge_cache(dir: &Path) -> Vec<EdgeVoice> {
    std::fs::read_to_string(dir.join(EDGE_CACHE_FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_edge_cache(dir: &Path, voices: &[EdgeVoice]) {
    if let Ok(json) = serde_json::to_string(voices) {
        let _ = std::fs::write(dir.join(EDGE_CACHE_FILE), json);
    }
}

// ---------------------------------------------------------------- Piper

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PiperVoice {
    pub key: String,
    pub name: String,
    /// Codice lingua, es. `it_IT`.
    pub lang: String,
    pub lang_name: String,
    pub quality: String,
    pub onnx_path: String,
    pub json_path: String,
    pub size_bytes: u64,
}

impl PiperVoice {
    pub fn onnx_file_name(&self) -> &str {
        self.onnx_path.rsplit('/').next().unwrap_or(&self.onnx_path)
    }
    pub fn json_file_name(&self) -> &str {
        self.json_path.rsplit('/').next().unwrap_or(&self.json_path)
    }
    pub fn is_installed(&self, models_dir: &Path) -> bool {
        models_dir.join(self.onnx_file_name()).is_file()
            && models_dir.join(self.json_file_name()).is_file()
    }
}

fn quality_rank(q: &str) -> u8 {
    match q {
        "high" => 0,
        "medium" => 1,
        "low" => 2,
        _ => 3,
    }
}

/// Interpreta `voices.json` del catalogo Piper.
pub fn parse_piper_index(json: &str) -> Result<Vec<PiperVoice>, String> {
    let root: Value =
        serde_json::from_str(json).map_err(|e| format!("catalogo non valido: {e}"))?;
    let obj = root.as_object().ok_or("catalogo non valido")?;
    let mut out = Vec::new();
    for (key, v) in obj {
        let files = v.get("files").and_then(Value::as_object);
        let Some(files) = files else { continue };
        let onnx = files.iter().find(|(p, _)| p.ends_with(".onnx"));
        let cfg = files.keys().find(|p| p.ends_with(".onnx.json"));
        let (Some((onnx_path, onnx_meta)), Some(json_path)) = (onnx, cfg) else {
            continue;
        };
        let lang = v
            .pointer("/language/code")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let lang_name = v
            .pointer("/language/name_english")
            .and_then(Value::as_str)
            .unwrap_or(&lang)
            .to_string();
        out.push(PiperVoice {
            key: key.clone(),
            name: v
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(key)
                .to_string(),
            lang,
            lang_name,
            quality: v
                .get("quality")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            onnx_path: onnx_path.clone(),
            json_path: json_path.clone(),
            size_bytes: onnx_meta
                .get("size_bytes")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        });
    }
    out.sort_by(|a, b| {
        a.lang
            .cmp(&b.lang)
            .then(quality_rank(&a.quality).cmp(&quality_rank(&b.quality)))
            .then(a.name.cmp(&b.name))
    });
    if out.is_empty() {
        return Err("catalogo vuoto".into());
    }
    Ok(out)
}

pub fn fetch_piper_index() -> Result<Vec<PiperVoice>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .into();
    let mut resp = agent
        .get(PIPER_INDEX_URL)
        .call()
        .map_err(|e| e.to_string())?;
    let body = resp
        .body_mut()
        .with_config()
        .limit(50 * 1024 * 1024)
        .read_to_string()
        .map_err(|e| e.to_string())?;
    parse_piper_index(&body)
}

/// Scarica `url` in `dest` (tramite file `.part`), segnalando i byte scaricati.
pub fn download_file(
    url: &str,
    dest: &Path,
    expected: Option<u64>,
    on_progress: &mut dyn FnMut(u64),
    is_cancelled: &dyn Fn() -> bool,
) -> Result<u64, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut resp = agent.get(url).call().map_err(|e| e.to_string())?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let part = dest.with_extension("part");
    let result = (|| {
        let mut file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
        let mut reader = resp.body_mut().as_reader();
        let mut buf = vec![0u8; 64 * 1024];
        let (mut total, mut last_report) = (0u64, 0u64);
        loop {
            if is_cancelled() {
                return Err("Scaricamento annullato".to_string());
            }
            let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            total += n as u64;
            if total - last_report >= 256 * 1024 {
                last_report = total;
                on_progress(total);
            }
        }
        file.flush().map_err(|e| e.to_string())?;
        on_progress(total);
        match expected {
            Some(exp) if exp > 0 && exp != total => {
                Err(format!("file incompleto ({total} di {exp} byte)"))
            }
            _ => Ok(total),
        }
    })();
    match result {
        Ok(total) => {
            std::fs::rename(&part, dest).map_err(|e| e.to_string())?;
            Ok(total)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

/// Scarica una voce Piper (modello + configurazione) in `models_dir`.
pub fn download_piper_voice(
    voice: &PiperVoice,
    models_dir: &Path,
    on_progress: &mut dyn FnMut(u64),
    is_cancelled: &dyn Fn() -> bool,
) -> Result<(), String> {
    let json_dest = models_dir.join(voice.json_file_name());
    download_file(
        &format!("{PIPER_BASE_URL}{}", voice.json_path),
        &json_dest,
        None,
        &mut |_| {},
        is_cancelled,
    )?;
    let onnx_dest = models_dir.join(voice.onnx_file_name());
    let size = (voice.size_bytes > 0).then_some(voice.size_bytes);
    if let Err(e) = download_file(
        &format!("{PIPER_BASE_URL}{}", voice.onnx_path),
        &onnx_dest,
        size,
        on_progress,
        is_cancelled,
    ) {
        let _ = std::fs::remove_file(&json_dest);
        return Err(e);
    }
    Ok(())
}

/// Estrae uno zip in `dest`, ignorando le voci con percorsi pericolosi (`..`, percorsi assoluti).
pub fn extract_zip(
    zip_path: &Path,
    dest: &Path,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<usize, String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("archivio non valido: {e}"))?;
    let mut count = 0usize;
    for i in 0..archive.len() {
        if is_cancelled() {
            return Err("Installazione annullata".into());
        }
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let out = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut f = std::fs::File::create(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        std::io::copy(&mut entry, &mut f).map_err(|e| e.to_string())?;
        count += 1;
    }
    Ok(count)
}

/// Scarica ed estrae il programma Piper (cartella `piper/` accanto al programma).
pub fn install_piper_runtime(
    url: &str,
    base_dir: &Path,
    on_progress: &mut dyn FnMut(u64),
    is_cancelled: &dyn Fn() -> bool,
) -> Result<std::path::PathBuf, String> {
    let zip_path = base_dir.join("piper_download.zip");
    download_file(url, &zip_path, None, on_progress, is_cancelled)?;
    let result = extract_zip(&zip_path, base_dir, is_cancelled);
    let _ = std::fs::remove_file(&zip_path);
    result?;
    crate::tts::find_piper(base_dir)
        .ok_or_else(|| "piper.exe non trovato nell'archivio scaricato".to_string())
}

/// Lingue presenti nel catalogo: (codice, nome inglese), senza doppioni.
pub fn piper_languages(voices: &[PiperVoice]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for v in voices {
        if !out.iter().any(|(c, _)| *c == v.lang) {
            out.push((v.lang.clone(), v.lang_name.clone()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    const INDEX: &str = r#"{
      "it_IT-paola-medium": {"key":"it_IT-paola-medium","name":"paola",
        "language":{"code":"it_IT","name_english":"Italian"},"quality":"medium",
        "files":{"it/it_IT/paola/medium/it_IT-paola-medium.onnx":{"size_bytes":100,"md5_digest":"x"},
                 "it/it_IT/paola/medium/it_IT-paola-medium.onnx.json":{"size_bytes":10},
                 "it/it_IT/paola/medium/MODEL_CARD":{"size_bytes":1}}},
      "it_IT-riccardo-x_low": {"key":"it_IT-riccardo-x_low","name":"riccardo",
        "language":{"code":"it_IT","name_english":"Italian"},"quality":"x_low",
        "files":{"a/it_IT-riccardo-x_low.onnx":{"size_bytes":5},"a/it_IT-riccardo-x_low.onnx.json":{}}},
      "en_US-lessac-high": {"key":"en_US-lessac-high","name":"lessac",
        "language":{"code":"en_US","name_english":"English"},"quality":"high",
        "files":{"b/en_US-lessac-high.onnx":{"size_bytes":9},"b/en_US-lessac-high.onnx.json":{}}},
      "rotta": {"name":"x"}
    }"#;

    #[test]
    fn piper_index_is_parsed_and_sorted() {
        let v = parse_piper_index(INDEX).unwrap();
        let keys: Vec<&str> = v.iter().map(|x| x.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "en_US-lessac-high",
                "it_IT-paola-medium",
                "it_IT-riccardo-x_low"
            ]
        );
        let p = &v[1];
        assert_eq!(p.onnx_file_name(), "it_IT-paola-medium.onnx");
        assert_eq!(p.json_file_name(), "it_IT-paola-medium.onnx.json");
        assert_eq!(p.size_bytes, 100);
        assert_eq!(p.lang_name, "Italian");
        assert_eq!(
            piper_languages(&v),
            vec![
                ("en_US".to_string(), "English".to_string()),
                ("it_IT".to_string(), "Italian".to_string())
            ]
        );
    }

    #[test]
    fn bad_index_is_an_error() {
        assert!(parse_piper_index("non json").is_err());
        assert!(parse_piper_index("[]").is_err());
        assert!(parse_piper_index("{}").is_err());
    }

    #[test]
    fn edge_labels_are_readable() {
        assert_eq!(
            edge_label("it-IT-IsabellaNeural", Some("Female"), "it-IT"),
            "Isabella (F, it-IT)"
        );
        assert_eq!(
            edge_label("it-IT-GiuseppeMultilingualNeural", Some("Male"), "it-IT"),
            "Giuseppe Multilingual (M, it-IT)"
        );
        assert_eq!(edge_label("strano", None, "xx"), "strano (?, xx)");
    }

    #[test]
    fn edge_voices_sort_italian_first() {
        let mk = |id: &str, loc: &str| EdgeVoice {
            id: id.into(),
            label: id.into(),
            locale: loc.into(),
        };
        let mut v = vec![
            mk("fr-FR-A", "fr-FR"),
            mk("en-US-B", "en-US"),
            mk("it-IT-C", "it-IT"),
            mk("it-IT-A", "it-IT"),
        ];
        sort_edge_voices(&mut v);
        let ids: Vec<&str> = v.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, vec!["it-IT-A", "it-IT-C", "en-US-B", "fr-FR-A"]);
    }

    #[test]
    fn edge_cache_round_trip() {
        let dir = std::env::temp_dir().join(format!("voc_edge_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load_edge_cache(&dir).is_empty());
        let v = vec![EdgeVoice {
            id: "it-IT-X".into(),
            label: "X".into(),
            locale: "it-IT".into(),
        }];
        save_edge_cache(&dir, &v);
        assert_eq!(load_edge_cache(&dir), v);
    }

    /// Mini server HTTP che risponde a `n` richieste con lo stesso corpo.
    fn serve(body: Vec<u8>, n: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for _ in 0..n {
                let Ok((mut s, _)) = listener.accept() else {
                    return;
                };
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                while r.read_line(&mut line).map(|n| n > 2).unwrap_or(false) {
                    line.clear();
                }
                let _ = write!(
                    s,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(&body);
            }
        });
        format!("http://{addr}/f")
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("voc_dl_{name}_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    #[test]
    fn download_writes_file_and_reports_progress() {
        let body = vec![7u8; 600_000];
        let url = serve(body.clone(), 1);
        let dest = tmp("ok").join("m.onnx");
        let mut seen = Vec::new();
        let n =
            download_file(&url, &dest, Some(600_000), &mut |b| seen.push(b), &|| false).unwrap();
        assert_eq!(n, 600_000);
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert!(seen.len() >= 2 && seen.windows(2).all(|w| w[0] <= w[1]));
        assert!(!dest.with_extension("part").exists());
    }

    #[test]
    fn download_rejects_wrong_size_and_cleans_up() {
        let url = serve(vec![1u8; 1000], 1);
        let dest = tmp("size").join("m.onnx");
        let err = download_file(&url, &dest, Some(2000), &mut |_| {}, &|| false).unwrap_err();
        assert!(err.contains("incompleto"), "{err}");
        assert!(!dest.exists() && !dest.with_extension("part").exists());
    }

    #[test]
    fn download_can_be_cancelled() {
        let url = serve(vec![1u8; 1_000_000], 1);
        let dest = tmp("cancel").join("m.onnx");
        let err = download_file(&url, &dest, None, &mut |_| {}, &|| true).unwrap_err();
        assert!(err.contains("annullato"));
        assert!(!dest.exists() && !dest.with_extension("part").exists());
    }

    #[test]
    fn installed_detection_needs_both_files() {
        let v = parse_piper_index(INDEX).unwrap();
        let p = &v[1];
        let dir = tmp("inst");
        assert!(!p.is_installed(&dir));
        std::fs::write(dir.join(p.onnx_file_name()), b"x").unwrap();
        assert!(!p.is_installed(&dir));
        std::fs::write(dir.join(p.json_file_name()), b"x").unwrap();
        assert!(p.is_installed(&dir));
    }

    fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        for (name, data) in entries {
            z.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn zip_extraction_skips_dangerous_paths() {
        let dir = tmp("zipx");
        let zp = dir.join("a.zip");
        make_zip(
            &zp,
            &[
                ("piper/piper.exe", b"EXE"),
                ("piper/data/x.txt", b"X"),
                ("../evil.txt", b"BAD"),
            ],
        );
        let out = dir.join("out");
        let n = extract_zip(&zp, &out, &|| false).unwrap();
        assert_eq!(n, 2);
        assert_eq!(std::fs::read(out.join("piper/piper.exe")).unwrap(), b"EXE");
        assert!(!dir.join("evil.txt").exists());
        assert!(extract_zip(&dir.join("manca.zip"), &out, &|| false).is_err());
    }

    #[test]
    fn piper_runtime_is_downloaded_and_installed() {
        let dir = tmp("rt");
        let zp = dir.join("src.zip");
        let exe_name = if cfg!(windows) {
            "piper/piper.exe"
        } else {
            "piper/piper"
        };
        make_zip(
            &zp,
            &[(exe_name, b"EXE"), ("piper/espeak-ng-data/it", b"d")],
        );
        let url = serve(std::fs::read(&zp).unwrap(), 1);
        let base = dir.join("app");
        std::fs::create_dir_all(&base).unwrap();
        let mut last = 0;
        let exe = install_piper_runtime(&url, &base, &mut |b| last = b, &|| false).unwrap();
        assert!(exe.is_file());
        assert!(last > 0);
        assert!(!base.join("piper_download.zip").exists());
        assert!(base.join("piper/espeak-ng-data/it").is_file());
    }

    #[test]
    fn piper_runtime_without_exe_is_reported() {
        let dir = tmp("rt2");
        let zp = dir.join("src.zip");
        make_zip(&zp, &[("altro/file.txt", b"x")]);
        let url = serve(std::fs::read(&zp).unwrap(), 1);
        let base = dir.join("app");
        std::fs::create_dir_all(&base).unwrap();
        let err = install_piper_runtime(&url, &base, &mut |_| {}, &|| false).unwrap_err();
        assert!(err.contains("non trovato"), "{err}");
    }
}
