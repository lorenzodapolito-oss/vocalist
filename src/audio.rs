//! Elaborazione audio: lettura, silenzi, unione, normalizzazione, cambio tono, scrittura.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use symphonia::core::audio::sample::Sample;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Audio mono in virgola mobile.
#[derive(Clone, Debug, PartialEq)]
pub struct Pcm {
    pub rate: u32,
    pub samples: Vec<f32>,
}

impl Pcm {
    #[cfg(test)]
    pub fn new(rate: u32, samples: Vec<f32>) -> Self {
        Self { rate, samples }
    }

    pub fn silence(seconds: f64, rate: u32) -> Self {
        let n = (seconds.max(0.0) * f64::from(rate)).round() as usize;
        Self {
            rate,
            samples: vec![0.0; n],
        }
    }

    #[cfg(test)]
    pub fn duration(&self) -> f64 {
        if self.rate == 0 {
            0.0
        } else {
            self.samples.len() as f64 / f64::from(self.rate)
        }
    }

    pub fn peak(&self) -> f32 {
        self.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }
}

/// Nasconde la finestra della console sui processi esterni (Windows).
pub fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

/// Legge un file audio (WAV o MP3) e lo converte in mono.
pub fn decode_file(path: &Path) -> Result<Pcm, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| format!("formato non riconosciuto: {e}"))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or("nessuna traccia audio")?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or("parametri audio mancanti")?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| format!("decoder non disponibile: {e}"))?;
    let track_id = track.id;

    let mut rate = 0u32;
    let mut mono: Vec<f32> = Vec::new();
    let mut buf: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => {
                if mono.is_empty() {
                    return Err(format!("lettura fallita: {e}"));
                }
                break;
            }
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(audio) => {
                rate = audio.spec().rate();
                let channels = audio.spec().channels().count().max(1);
                buf.resize(audio.samples_interleaved(), f32::MID);
                audio.copy_to_slice_interleaved(&mut buf);
                for frame in buf.chunks(channels) {
                    mono.push(frame.iter().sum::<f32>() / channels as f32);
                }
            }
            Err(SymError::DecodeError(_)) => {}
            Err(e) => {
                if mono.is_empty() {
                    return Err(format!("decodifica fallita: {e}"));
                }
                break;
            }
        }
    }
    if rate == 0 || mono.is_empty() {
        return Err("audio vuoto".into());
    }
    Ok(Pcm {
        rate,
        samples: mono,
    })
}

fn lanczos(x: f64, a: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else if x.abs() >= a {
        0.0
    } else {
        let px = std::f64::consts::PI * x;
        a * px.sin() * (px / a).sin() / (px * px)
    }
}

/// Cambia la frequenza di campionamento (interpolazione Lanczos).
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() || from == 0 || to == 0 {
        return input.to_vec();
    }
    let ratio = f64::from(to) / f64::from(from);
    let out_len = (input.len() as f64 * ratio).round() as usize;
    // In riduzione si abbassa il taglio per evitare aliasing.
    let scale = ratio.min(1.0);
    let a = 8.0;
    let support = a / scale;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let center = i as f64 / ratio;
        let lo = (center - support).ceil().max(0.0) as usize;
        let hi = ((center + support).floor() as usize).min(input.len() - 1);
        let (mut acc, mut wsum) = (0.0f64, 0.0f64);
        for (j, &sample) in input.iter().enumerate().take(hi + 1).skip(lo) {
            let w = lanczos((j as f64 - center) * scale, a);
            acc += f64::from(sample) * w;
            wsum += w;
        }
        out.push(if wsum.abs() > 1e-9 {
            (acc / wsum) as f32
        } else {
            0.0
        });
    }
    out
}

/// Unisce più parti (anche con frequenze diverse) in un unico audio, senza dissolvenze.
pub fn merge(parts: &[Pcm]) -> Option<Pcm> {
    let rate = parts.iter().map(|p| p.rate).max()?;
    let mut samples = Vec::with_capacity(parts.iter().map(|p| p.samples.len()).sum());
    for p in parts {
        if p.rate == rate {
            samples.extend_from_slice(&p.samples);
        } else {
            samples.extend(resample(&p.samples, p.rate, rate));
        }
    }
    Some(Pcm { rate, samples })
}

/// Normalizza il picco a -`headroom_db` dBFS (come `pydub.effects.normalize`).
pub fn normalize(pcm: &mut Pcm, headroom_db: f64) {
    let peak = f64::from(pcm.peak());
    if peak <= 1e-9 {
        return;
    }
    let target = 10f64.powf(-headroom_db / 20.0);
    let gain = (target / peak) as f32;
    for s in &mut pcm.samples {
        *s *= gain;
    }
}

/// Allunga o accorcia l'audio senza cambiare il tono (WSOLA). `ratio` = durata_uscita / durata_ingresso.
pub fn time_stretch(input: &[f32], rate: u32, ratio: f64) -> Vec<f32> {
    if input.len() < 64 || !(0.25..=4.0).contains(&ratio) || (ratio - 1.0).abs() < 1e-4 {
        return input.to_vec();
    }
    let mut win = ((f64::from(rate) * 0.04) as usize).max(128);
    win -= win % 2;
    if input.len() < win * 2 {
        // Troppo corto per la sovrapposizione: ricampionamento semplice.
        let n = (input.len() as f64 * ratio) as usize;
        return (0..n)
            .map(|i| input[((i as f64 / ratio) as usize).min(input.len() - 1)])
            .collect();
    }
    let hs = win / 2; // passo di sintesi
    let ha = hs as f64 / ratio; // passo di analisi nominale
    let tol = hs / 2;
    let out_len = (input.len() as f64 * ratio) as usize;
    let hann: Vec<f32> = (0..win)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / win as f64).cos() as f32)
        .collect();

    let mut out = vec![0.0f32; out_len + win * 2];
    let mut norm = vec![0.0f32; out_len + win * 2];
    let mut prev_start = 0usize;
    let mut k = 0usize;
    loop {
        let out_pos = k * hs;
        if out_pos >= out_len {
            break;
        }
        let nominal = (k as f64 * ha) as usize;
        let mut start = nominal.min(input.len().saturating_sub(win));
        if k > 0 {
            // Continuazione naturale del frammento precedente.
            let target_start = prev_start + hs;
            if target_start + win <= input.len() {
                let lo = nominal.saturating_sub(tol);
                let hi = (nominal + tol).min(input.len() - win);
                let mut best = f64::MIN;
                let mut cand = lo;
                while cand <= hi {
                    let mut c = 0.0f64;
                    for i in (0..win).step_by(4) {
                        c += f64::from(input[cand + i]) * f64::from(input[target_start + i]);
                    }
                    if c > best {
                        best = c;
                        cand = cand.max(lo);
                        start = cand;
                    }
                    cand += 2;
                }
            }
        }
        if start + win > input.len() {
            break;
        }
        for i in 0..win {
            out[out_pos + i] += input[start + i] * hann[i];
            norm[out_pos + i] += hann[i];
        }
        prev_start = start;
        k += 1;
    }
    out.truncate(out_len);
    for (o, n) in out.iter_mut().zip(norm.iter()) {
        if *n > 1e-3 {
            *o /= *n;
        }
    }
    out
}

/// Cambia il tono di `semitones` semitoni mantenendo la durata.
pub fn pitch_shift(pcm: &Pcm, semitones: f64) -> Pcm {
    if semitones.abs() < 0.05 || pcm.samples.is_empty() {
        return pcm.clone();
    }
    let factor = 2f64.powf(semitones / 12.0);
    // Ricampionare come se fosse a frequenza diversa cambia tono e durata...
    let faster = resample(
        &pcm.samples,
        (f64::from(pcm.rate) * factor).round() as u32,
        pcm.rate,
    );
    // ...poi si ripristina la durata originale senza toccare il tono.
    let samples = time_stretch(
        &faster,
        pcm.rate,
        pcm.samples.len() as f64 / faster.len().max(1) as f64,
    );
    Pcm {
        rate: pcm.rate,
        samples,
    }
}

/// Scrive un WAV a 16 bit mono.
pub fn write_wav(path: &Path, pcm: &Pcm) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: pcm.rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for s in &pcm.samples {
        let v = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
        w.write_sample(v).map_err(|e| e.to_string())?;
    }
    w.finalize().map_err(|e| e.to_string())
}

/// Cerca ffmpeg accanto al programma o nel PATH.
pub fn find_ffmpeg() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let local = crate::config::app_dir().join(name);
    if local.is_file() {
        return Some(local);
    }
    let mut cmd = Command::new(name);
    cmd.arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    no_window(&mut cmd);
    match cmd.status() {
        Ok(s) if s.success() => Some(PathBuf::from(name)),
        _ => None,
    }
}

/// Dove scrivere davvero: l'MP3 richiede ffmpeg, altrimenti si ripiega su WAV.
pub fn resolve_output(path: &Path, ffmpeg: Option<&Path>) -> (PathBuf, Option<String>) {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "mp3" if ffmpeg.is_none() => (
            path.with_extension("wav"),
            Some("ffmpeg non trovato: salvo in WAV invece che MP3".to_string()),
        ),
        "mp3" | "wav" => (path.to_path_buf(), None),
        _ => (path.with_extension("wav"), None),
    }
}

/// Salva l'audio nel formato indicato dall'estensione (wav, oppure mp3 tramite ffmpeg).
pub fn export(pcm: &Pcm, path: &Path, ffmpeg: Option<&Path>) -> Result<(), String> {
    let is_mp3 = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("mp3"));
    if !is_mp3 {
        return write_wav(path, pcm);
    }
    let ff = ffmpeg.ok_or("ffmpeg non disponibile")?;
    let tmp = path.with_extension("tmp.wav");
    write_wav(&tmp, pcm)?;
    let mut cmd = Command::new(ff);
    cmd.args(["-y", "-loglevel", "error", "-i"])
        .arg(&tmp)
        .args(["-codec:a", "libmp3lame", "-q:a", "2"])
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    no_window(&mut cmd);
    let out = cmd.output().map_err(|e| format!("ffmpeg: {e}"));
    let _ = std::fs::remove_file(&tmp);
    let out = out?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "ffmpeg ha fallito: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, freq: f64, secs: f64) -> Pcm {
        let n = (f64::from(rate) * secs) as usize;
        let samples = (0..n)
            .map(|i| {
                (2.0 * std::f64::consts::PI * freq * i as f64 / f64::from(rate)).sin() as f32 * 0.5
            })
            .collect();
        Pcm::new(rate, samples)
    }

    /// Frequenza stimata con i passaggi per lo zero.
    fn freq_of(p: &Pcm) -> f64 {
        let crossings = p
            .samples
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        crossings as f64 / p.duration()
    }

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("medit_audio_{name}_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    #[test]
    fn silence_has_right_length() {
        let s = Pcm::silence(1.5, 22050);
        assert_eq!(s.samples.len(), 33075);
        assert_eq!(s.peak(), 0.0);
        assert!(Pcm::silence(-1.0, 22050).samples.is_empty());
    }

    #[test]
    fn resample_keeps_frequency_and_duration() {
        let p = sine(22050, 440.0, 1.0);
        let r = resample(&p.samples, 22050, 24000);
        let q = Pcm::new(24000, r);
        assert!((q.duration() - 1.0).abs() < 0.01);
        assert!((freq_of(&q) - 440.0).abs() < 5.0);
        let d = Pcm::new(11025, resample(&p.samples, 22050, 11025));
        assert!((freq_of(&d) - 440.0).abs() < 5.0);
    }

    #[test]
    fn merge_unifies_rates_to_the_highest() {
        let a = sine(22050, 300.0, 0.5);
        let b = sine(24000, 300.0, 0.5);
        let m = merge(&[a, b]).unwrap();
        assert_eq!(m.rate, 24000);
        assert!((m.duration() - 1.0).abs() < 0.01);
        assert!(merge(&[]).is_none());
    }

    #[test]
    fn normalize_sets_peak_to_minus_half_db() {
        let mut p = sine(22050, 440.0, 0.2);
        normalize(&mut p, 0.5);
        let target = 10f32.powf(-0.5 / 20.0);
        assert!((p.peak() - target).abs() < 1e-3);
        let mut silent = Pcm::silence(0.1, 22050);
        normalize(&mut silent, 0.5);
        assert_eq!(silent.peak(), 0.0);
    }

    #[test]
    fn pitch_shift_changes_frequency_but_not_length() {
        let p = sine(22050, 220.0, 2.0);
        let up = pitch_shift(&p, 12.0);
        assert!(
            (up.duration() - 2.0).abs() < 0.1,
            "durata {}",
            up.duration()
        );
        assert!((freq_of(&up) - 440.0).abs() < 25.0, "freq {}", freq_of(&up));
        let down = pitch_shift(&p, -12.0);
        assert!((down.duration() - 2.0).abs() < 0.1);
        assert!(
            (freq_of(&down) - 110.0).abs() < 15.0,
            "freq {}",
            freq_of(&down)
        );
        assert_eq!(pitch_shift(&p, 0.0), p);
    }

    #[test]
    fn time_stretch_changes_length_only() {
        let p = sine(22050, 300.0, 1.0);
        let s = Pcm::new(22050, time_stretch(&p.samples, 22050, 1.5));
        assert!((s.duration() - 1.5).abs() < 0.05);
        assert!((freq_of(&s) - 300.0).abs() < 15.0, "freq {}", freq_of(&s));
    }

    #[test]
    fn wav_round_trip_through_decoder() {
        let dir = tmpdir("wav");
        let path = dir.join("t.wav");
        let p = sine(22050, 440.0, 0.5);
        write_wav(&path, &p).unwrap();
        let back = decode_file(&path).unwrap();
        assert_eq!(back.rate, 22050);
        assert!((back.duration() - 0.5).abs() < 0.01);
        assert!((back.peak() - 0.5).abs() < 0.01);
    }

    #[test]
    fn decoding_garbage_fails_cleanly() {
        let dir = tmpdir("bad");
        let path = dir.join("x.wav");
        std::fs::write(&path, b"non e' audio").unwrap();
        assert!(decode_file(&path).is_err());
        assert!(decode_file(&dir.join("manca.wav")).is_err());
    }

    #[test]
    fn output_falls_back_to_wav_without_ffmpeg() {
        let (p, warn) = resolve_output(Path::new("/x/med.mp3"), None);
        assert_eq!(p, PathBuf::from("/x/med.wav"));
        assert!(warn.is_some());
        let (p, warn) = resolve_output(Path::new("/x/med.mp3"), Some(Path::new("ffmpeg")));
        assert_eq!(p, PathBuf::from("/x/med.mp3"));
        assert!(warn.is_none());
        assert_eq!(
            resolve_output(Path::new("/x/med"), None).0,
            PathBuf::from("/x/med.wav")
        );
    }

    #[test]
    fn mp3_export_and_decode_with_ffmpeg() {
        let Some(ff) = find_ffmpeg() else { return };
        let dir = tmpdir("mp3");
        let path = dir.join("t.mp3");
        let p = sine(24000, 440.0, 1.0);
        export(&p, &path, Some(&ff)).unwrap();
        let back = decode_file(&path).unwrap();
        assert!((back.duration() - 1.0).abs() < 0.1);
        assert!(export(&p, &path, None).is_err());
    }
}
