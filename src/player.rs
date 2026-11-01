//! Riproduzione audio (rodio) con ripiego sul lettore predefinito del sistema.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::audio::no_window;

/// Apre un file con l'applicazione predefinita del sistema.
pub fn open_with_default_app(path: &Path) -> Result<(), String> {
    let mut cmd;
    if cfg!(windows) {
        cmd = Command::new("cmd");
        cmd.args(["/C", "start", ""]).arg(path);
    } else if cfg!(target_os = "macos") {
        cmd = Command::new("open");
        cmd.arg(path);
    } else {
        cmd = Command::new("xdg-open");
        cmd.arg(path);
    }
    no_window(&mut cmd);
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[derive(Default)]
pub struct AudioPlayer {
    #[cfg(feature = "audio-out")]
    sink: Option<rodio::MixerDeviceSink>,
    #[cfg(feature = "audio-out")]
    player: Option<rodio::Player>,
    pub is_playing: bool,
    pub is_paused: bool,
    pub current: Option<PathBuf>,
}

impl AudioPlayer {
    #[cfg(feature = "audio-out")]
    fn ensure_sink(&mut self) -> Result<(), String> {
        if self.sink.is_none() {
            let mut sink =
                rodio::DeviceSinkBuilder::open_default_sink().map_err(|e| e.to_string())?;
            sink.log_on_drop(false);
            self.sink = Some(sink);
        }
        Ok(())
    }

    #[cfg(feature = "audio-out")]
    fn play_internal(&mut self, path: &Path) -> Result<(), String> {
        use std::num::NonZero;
        self.ensure_sink()?;
        let pcm = crate::audio::decode_file(path)?;
        let channels = NonZero::new(1u16).ok_or("canali non validi")?;
        let rate = NonZero::new(pcm.rate).ok_or("frequenza non valida")?;
        let sink = self.sink.as_ref().ok_or("uscita audio non disponibile")?;
        let player = rodio::Player::connect_new(sink.mixer());
        player.append(rodio::buffer::SamplesBuffer::new(
            channels,
            rate,
            pcm.samples,
        ));
        self.player = Some(player);
        Ok(())
    }

    #[cfg(not(feature = "audio-out"))]
    fn play_internal(&mut self, path: &Path) -> Result<(), String> {
        open_with_default_app(path)
    }

    /// Avvia (o riprende, se in pausa sullo stesso file) la riproduzione.
    pub fn play(&mut self, path: &Path) -> Result<(), String> {
        if !path.exists() {
            return Err(format!("File non trovato: {}", path.display()));
        }
        if self.is_paused && self.current.as_deref() == Some(path) {
            self.pause_toggle();
            return Ok(());
        }
        self.stop();
        if let Err(e) = self.play_internal(path) {
            // Ripiego: lettore predefinito del sistema.
            open_with_default_app(path).map_err(|e2| format!("{e}; {e2}"))?;
        }
        self.is_playing = true;
        self.is_paused = false;
        self.current = Some(path.to_path_buf());
        Ok(())
    }

    pub fn pause_toggle(&mut self) {
        #[cfg(feature = "audio-out")]
        if let Some(p) = &self.player {
            if self.is_paused {
                p.play();
                self.is_paused = false;
            } else if !p.empty() {
                p.pause();
                self.is_paused = true;
            }
        }
    }

    pub fn stop(&mut self) {
        #[cfg(feature = "audio-out")]
        if let Some(p) = self.player.take() {
            p.stop();
        }
        self.is_playing = false;
        self.is_paused = false;
    }

    /// Sta suonando (non in pausa)?
    pub fn is_busy(&self) -> bool {
        #[cfg(feature = "audio-out")]
        if let Some(p) = &self.player {
            return !p.empty();
        }
        false
    }

    /// Da chiamare di continuo: `true` una sola volta quando la riproduzione finisce da sola.
    pub fn poll_end(&mut self) -> bool {
        #[cfg(feature = "audio-out")]
        if self.is_playing && !self.is_paused {
            if let Some(p) = &self.player {
                if p.empty() {
                    self.is_playing = false;
                    return true;
                }
            }
        }
        false
    }
}
