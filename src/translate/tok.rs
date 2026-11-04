//! Tokenizzatore Marian: pezzi SentencePiece -> identificatori del vocabolario.

use crate::translate::spm::Spm;
use std::collections::HashMap;
use std::path::Path;

pub struct Tok {
    spm: Spm,
    vocab: HashMap<String, u32>,
    inverse: HashMap<u32, String>,
    pub unk: u32,
    pub eos: u32,
    pub pad: u32,
    /// Lunghezza massima della frase in ingresso, eos compreso.
    pub max_len: usize,
}

impl Tok {
    pub fn new(spm: Spm, vocab: HashMap<String, u32>, eos: u32, pad: u32, max_len: usize) -> Self {
        let unk = vocab.get("<unk>").copied().unwrap_or(1);
        let inverse = vocab.iter().map(|(k, v)| (*v, k.clone())).collect();
        Self {
            spm,
            vocab,
            inverse,
            unk,
            eos,
            pad,
            max_len: max_len.max(2),
        }
    }

    pub fn load(dir: &Path, eos: u32, pad: u32, max_len: usize) -> Result<Self, String> {
        let spm_path = dir.join("source.spm");
        let spm_bytes = std::fs::read(&spm_path).map_err(|e| {
            format!(
                "{}: {e} (serve il file source.spm del modello)",
                spm_path.display()
            )
        })?;
        let spm = Spm::parse(&spm_bytes)?;
        let vocab_path = dir.join("vocab.json");
        let text = std::fs::read_to_string(&vocab_path).map_err(|e| {
            format!(
                "{}: {e} (serve il file vocab.json del modello)",
                vocab_path.display()
            )
        })?;
        let vocab: HashMap<String, u32> =
            serde_json::from_str(&text).map_err(|e| format!("vocab.json non valido: {e}"))?;
        Ok(Self::new(spm, vocab, eos, pad, max_len))
    }

    /// Identificatori della frase, con `</s>` in fondo.
    pub fn encode(&self, text: &str) -> Vec<u32> {
        let mut ids: Vec<u32> = self
            .spm
            .encode(text)
            .iter()
            .map(|p| self.vocab.get(p).copied().unwrap_or(self.unk))
            .collect();
        ids.truncate(self.max_len - 1);
        ids.push(self.eos);
        ids
    }

    /// Testo dagli identificatori, senza simboli speciali.
    pub fn decode(&self, ids: &[u32]) -> String {
        let mut s = String::new();
        for id in ids {
            if *id == self.eos || *id == self.pad || *id == self.unk {
                continue;
            }
            if let Some(p) = self.inverse.get(id) {
                s.push_str(p);
            }
        }
        s.replace('\u{2581}', " ").trim().to_string()
    }
}

#[cfg(test)]
pub fn test_tok() -> Tok {
    let spm = Spm::parse(&crate::translate::spm::build_model(
        &[
            ("\u{2581}", -3.0, 1),
            ("\u{2581}hello", -1.0, 1),
            ("\u{2581}world", -1.0, 1),
        ],
        1,
    ))
    .unwrap();
    let vocab: HashMap<String, u32> = [
        ("</s>", 0),
        ("<unk>", 1),
        ("<pad>", 2),
        ("\u{2581}hello", 3),
        ("\u{2581}world", 4),
        ("\u{2581}ciao", 5),
        ("\u{2581}mondo", 6),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    Tok::new(spm, vocab, 0, 2, 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_with_eos_and_unknowns_and_truncates() {
        let t = test_tok();
        assert_eq!(t.encode("hello world"), vec![3, 4, 0]);
        let unk = t.encode("hello zzz");
        assert_eq!(unk.first(), Some(&3));
        assert!(unk.contains(&1) && unk.last() == Some(&0));
        let long = "hello ".repeat(100);
        let ids = t.encode(&long);
        assert_eq!(ids.len(), 16);
        assert_eq!(ids.last(), Some(&0));
    }

    #[test]
    fn decodes_skipping_special_tokens() {
        let t = test_tok();
        assert_eq!(t.decode(&[2, 5, 6, 0, 1]), "ciao mondo");
        assert_eq!(t.decode(&[]), "");
        assert_eq!(t.decode(&[9999]), "");
    }
}
