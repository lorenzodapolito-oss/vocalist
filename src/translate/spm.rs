//! Lettore minimo di modelli SentencePiece (unigram) e tokenizzazione Viterbi.
//! Serve a spezzare l'inglese negli stessi pezzi usati dall'addestramento del modello Marian.

use std::collections::HashMap;
use unicode_normalization::UnicodeNormalization;

const SPACE: char = '\u{2581}';

#[derive(Debug)]
pub struct Spm {
    pieces: HashMap<String, f32>,
    max_chars: usize,
    unk_score: f32,
}

fn varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut out = 0u64;
    for shift in (0..70).step_by(7) {
        let b = *buf.get(*pos)?;
        *pos += 1;
        out |= u64::from(b & 0x7f).checked_shl(shift)?;
        if b & 0x80 == 0 {
            return Some(out);
        }
    }
    None
}

/// Un campo protobuf: numero e contenuto (varint, 4 byte, 8 byte o blocco di byte).
enum Field<'a> {
    Varint(u64),
    Fixed32([u8; 4]),
    Bytes(&'a [u8]),
    Skip,
}

fn fields(buf: &[u8]) -> Result<Vec<(u64, Field<'_>)>, String> {
    let bad = || "modello SentencePiece non valido".to_string();
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < buf.len() {
        let tag = varint(buf, &mut pos).ok_or_else(bad)?;
        let (num, wire) = (tag >> 3, tag & 7);
        let f = match wire {
            0 => Field::Varint(varint(buf, &mut pos).ok_or_else(bad)?),
            1 => {
                pos = pos
                    .checked_add(8)
                    .filter(|p| *p <= buf.len())
                    .ok_or_else(bad)?;
                Field::Skip
            }
            2 => {
                let len =
                    usize::try_from(varint(buf, &mut pos).ok_or_else(bad)?).map_err(|_| bad())?;
                let end = pos
                    .checked_add(len)
                    .filter(|e| *e <= buf.len())
                    .ok_or_else(bad)?;
                let b = buf.get(pos..end).ok_or_else(bad)?;
                pos = end;
                Field::Bytes(b)
            }
            5 => {
                let b = buf.get(pos..pos + 4).ok_or_else(bad)?;
                pos += 4;
                let mut a = [0u8; 4];
                a.copy_from_slice(b);
                Field::Fixed32(a)
            }
            _ => return Err(bad()),
        };
        out.push((num, f));
    }
    Ok(out)
}

impl Spm {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let mut pieces = HashMap::new();
        let mut model_type = 1u64;
        for (num, f) in fields(bytes)? {
            match (num, f) {
                (1, Field::Bytes(b)) => {
                    let (mut text, mut score, mut kind) = (None, 0f32, 1u64);
                    for (n, f) in fields(b)? {
                        match (n, f) {
                            (1, Field::Bytes(s)) => {
                                text = Some(String::from_utf8_lossy(s).into_owned())
                            }
                            (2, Field::Fixed32(a)) => score = f32::from_le_bytes(a),
                            (3, Field::Varint(k)) => kind = k,
                            _ => {}
                        }
                    }
                    // Normali (1) e definiti dall'utente (4); controllo, sconosciuto, inutilizzato e byte no.
                    if let (Some(t), 1 | 4) = (text, kind) {
                        pieces.insert(t, score);
                    }
                }
                (2, Field::Bytes(b)) => {
                    for (n, f) in fields(b)? {
                        if let (3, Field::Varint(v)) = (n, f) {
                            model_type = v;
                        }
                    }
                }
                _ => {}
            }
        }
        if model_type != 1 {
            return Err("il modello SentencePiece non è di tipo unigram (non supportato)".into());
        }
        if pieces.is_empty() {
            return Err("il modello SentencePiece non contiene pezzi".into());
        }
        let max_chars = pieces.keys().map(|p| p.chars().count()).max().unwrap_or(1);
        let min_score = pieces.values().copied().fold(0f32, f32::min);
        Ok(Self {
            pieces,
            max_chars,
            unk_score: min_score - 10.0,
        })
    }

    /// Pezzi del testo (un pezzo sconosciuto resta il singolo carattere).
    pub fn encode(&self, text: &str) -> Vec<String> {
        let norm: String = text.nfkc().collect();
        let words: Vec<&str> = norm.split_whitespace().collect();
        if words.is_empty() {
            return Vec::new();
        }
        let joined = format!("{SPACE}{}", words.join(&SPACE.to_string()));
        let s: Vec<char> = joined.chars().collect();
        let n = s.len();
        let mut best = vec![f32::NEG_INFINITY; n + 1];
        let mut back = vec![0usize; n + 1];
        best[0] = 0.0;
        for i in 0..n {
            let base = best[i];
            if base == f32::NEG_INFINITY {
                continue;
            }
            let mut single = false;
            for l in 1..=self.max_chars.min(n - i) {
                let piece: String = s[i..i + l].iter().collect();
                if let Some(sc) = self.pieces.get(&piece) {
                    single |= l == 1;
                    if base + sc > best[i + l] {
                        best[i + l] = base + sc;
                        back[i + l] = i;
                    }
                }
            }
            if !single && base + self.unk_score > best[i + 1] {
                best[i + 1] = base + self.unk_score;
                back[i + 1] = i;
            }
        }
        let mut out = Vec::new();
        let mut end = n;
        while end > 0 {
            let start = back[end];
            out.push(s[start..end].iter().collect::<String>());
            end = start;
        }
        out.reverse();
        out
    }
}

#[cfg(test)]
pub fn build_model(pieces: &[(&str, f32, u64)], model_type: u64) -> Vec<u8> {
    fn vi(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                break;
            }
            out.push(b | 0x80);
        }
    }
    let mut m = Vec::new();
    for (t, sc, k) in pieces {
        let mut p = Vec::new();
        vi(10, &mut p);
        vi(t.len() as u64, &mut p);
        p.extend_from_slice(t.as_bytes());
        vi(21, &mut p);
        p.extend_from_slice(&sc.to_le_bytes());
        vi(24, &mut p);
        vi(*k, &mut p);
        vi(10, &mut m);
        vi(p.len() as u64, &mut m);
        m.extend(p);
    }
    let mut ts = Vec::new();
    vi(24, &mut ts);
    vi(model_type, &mut ts);
    vi(18, &mut m);
    vi(ts.len() as u64, &mut m);
    m.extend(ts);
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Spm {
        let bytes = build_model(
            &[
                ("<unk>", 0.0, 2),
                ("<s>", 0.0, 3),
                ("\u{2581}", -3.0, 1),
                ("\u{2581}the", -1.0, 1),
                ("\u{2581}th", -4.0, 1),
                ("e", -3.0, 1),
                ("\u{2581}cat", -2.0, 1),
                ("s", -3.0, 1),
                ("\u{2581}c", -4.0, 1),
                ("a", -3.0, 1),
                ("t", -3.0, 1),
            ],
            1,
        );
        Spm::parse(&bytes).unwrap()
    }

    #[test]
    fn viterbi_prefers_the_best_scoring_split() {
        let m = model();
        assert_eq!(m.encode("the cat"), vec!["\u{2581}the", "\u{2581}cat"]);
        assert_eq!(
            m.encode("  the   cats "),
            vec!["\u{2581}the", "\u{2581}cat", "s"]
        );
        assert!(m.encode("   ").is_empty());
    }

    #[test]
    fn unknown_characters_stay_single_and_normalization_applies() {
        let m = model();
        let p = m.encode("the \u{ff54}"); // «ｔ» a larghezza intera → NFKC «t»
        assert_eq!(p, vec!["\u{2581}the", "\u{2581}", "t"]);
        let p = m.encode("zz");
        assert_eq!(p, vec!["\u{2581}", "z", "z"]);
    }

    #[test]
    fn bad_models_are_rejected_without_panicking() {
        assert!(Spm::parse(&[]).is_err(), "nessun pezzo");
        assert!(Spm::parse(&[0xff, 0xff, 0xff]).is_err());
        assert!(
            Spm::parse(&[0x0a, 0x7f, 0x01]).is_err(),
            "lunghezza oltre il file"
        );
        let bpe = build_model(&[("a", 0.0, 1)], 2);
        assert!(Spm::parse(&bpe).unwrap_err().contains("unigram"));
        for n in 0..40u8 {
            let junk: Vec<u8> = (0..n).map(|i| i.wrapping_mul(37).wrapping_add(n)).collect();
            let _ = Spm::parse(&junk);
        }
    }
}
