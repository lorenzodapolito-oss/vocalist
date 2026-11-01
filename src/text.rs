//! Elaborazione del testo: pronuncia, pause, suddivisione in segmenti e stima della durata.

use regex::{NoExpand, Regex};
use std::ops::Range;
use std::sync::OnceLock;

/// Lunghezza massima (in caratteri) di un segmento inviato al motore vocale.
pub const MAX_CHARS_PER_SEGMENT: usize = 350;

/// Parole che i motori leggono male: vengono sostituite con una forma accentata.
const AMBIGUOUS_WORDS: [(&str, &str); 8] = [
    ("ancora", "àncora"),
    ("capitolo", "càpitolo"),
    ("principi", "prìncipi"),
    ("subito", "sùbito"),
    ("legato", "legàto"),
    ("venti", "vénti"),
    ("terra", "tèrra"),
    ("fine", "fìne"),
];

const VOWEL_DOUBLING: [(char, &str); 16] = [
    ('à', "àà"),
    ('À', "ÀÀ"),
    ('è', "èè"),
    ('È', "ÈÈ"),
    ('é', "éé"),
    ('É', "ÉÉ"),
    ('ì', "ìì"),
    ('Ì', "ÌÌ"),
    ('ò', "òò"),
    ('Ò', "ÒÒ"),
    ('ó', "óó"),
    ('Ó', "ÓÓ"),
    ('ù', "ùù"),
    ('Ù', "ÙÙ"),
    ('ú', "úú"),
    ('Ú', "ÚÚ"),
];

pub const EXAMPLE_TEXT: &str = "Inizio percorso di rilassamento.
Trova la tua posizione di massimo conforto.
Sdraiati, siediti, o appoggiati come preferisci.
Chiudi gli occhi.
Lascia che ogni contatto con la superficie ti supporti completamente.
Ora porta l'attenzione al respiro.
Senti l'aria che entra... e che esce...
Ad ogni espirazione, lascia andare qualsiasi tensione.
Il corpo diventa pesante e rilassato.
La mente si calma naturalmente.
Goditi questo momento di pace.";

/// Modalità di pronuncia scelta dall'utente.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PronunciationMode {
    /// Solo correzione delle parole ambigue.
    Original,
    /// Allunga le vocali accentate (à → àà).
    Vowel,
    /// Aggiunge pause dopo `. ! ?`.
    Paused,
}

impl PronunciationMode {
    pub const ALL: [PronunciationMode; 3] = [Self::Original, Self::Vowel, Self::Paused];

    /// Valore salvato nel file di configurazione (compatibile con la versione Python).
    pub fn key(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Vowel => "vowel",
            Self::Paused => "paused",
        }
    }

    pub fn from_key(key: &str) -> Self {
        match key {
            "vowel" => Self::Vowel,
            "paused" => Self::Paused,
            _ => Self::Original,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "Originale (correzione ambigue)",
            Self::Vowel => "Allungamento vocali (à = àà)",
            Self::Paused => "Pause extra dopo frasi",
        }
    }

    pub fn info(self) -> &'static str {
        match self {
            Self::Original => "Corregge 'ancora', 'subito', ecc.",
            Self::Vowel => "Allungamento: à = àà, è = èè",
            Self::Paused => "Aggiunge pause dopo . ! ?",
        }
    }
}

/// Un pezzo del copione: testo da leggere oppure silenzio (secondi).
#[derive(Clone, Debug, PartialEq)]
pub enum Segment {
    Text(String),
    Pause(f64),
}

fn pause_tag_re() -> Option<&'static Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\[PAUSA\s+(\d+(?:\.\d+)?)\s+SECONDI\]").ok())
        .as_ref()
}

fn ambiguous_res() -> &'static Vec<(Regex, &'static str)> {
    static RES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    RES.get_or_init(|| {
        AMBIGUOUS_WORDS
            .iter()
            .filter_map(|(word, repl)| {
                Regex::new(&format!(r"(?i)\b{word}\b"))
                    .ok()
                    .map(|re| (re, *repl))
            })
            .collect()
    })
}

/// Sostituisce le parole ambigue con la forma accentata.
pub fn fix_ambiguous_words(text: &str) -> String {
    let mut out = text.to_string();
    for (re, repl) in ambiguous_res() {
        out = re.replace_all(&out, NoExpand(repl)).into_owned();
    }
    out
}

/// Prepara il testo per il motore vocale secondo la modalità scelta.
pub fn preprocess_text(text: &str, mode: PronunciationMode) -> String {
    let fixed = fix_ambiguous_words(text);
    match mode {
        PronunciationMode::Original => fixed,
        PronunciationMode::Vowel => {
            let mut out = String::with_capacity(fixed.len() + 8);
            for ch in fixed.chars() {
                match VOWEL_DOUBLING.iter().find(|(c, _)| *c == ch) {
                    Some((_, doubled)) => out.push_str(doubled),
                    None => out.push(ch),
                }
            }
            out
        }
        PronunciationMode::Paused => {
            let mut out = String::with_capacity(fixed.len() + 16);
            for ch in fixed.chars() {
                out.push(ch);
                if matches!(ch, '.' | '!' | '?') {
                    out.push_str(" ... ");
                }
            }
            out
        }
    }
}

/// Divide il testo dopo `. ! ? : ;` seguiti da spazi (equivale a `(?<=[.!?:;])\s+`).
fn split_sentences(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut prev: Option<char> = None;
    let mut iter = text.char_indices().peekable();
    while let Some((i, ch)) = iter.next() {
        if ch.is_whitespace() && prev.is_some_and(|p| matches!(p, '.' | '!' | '?' | ':' | ';')) {
            // Inizio di una sequenza di spazi preceduta da punteggiatura: taglio qui.
            let mut end = i + ch.len_utf8();
            while let Some(&(j, c2)) = iter.peek() {
                if c2.is_whitespace() {
                    end = j + c2.len_utf8();
                    iter.next();
                } else {
                    break;
                }
            }
            parts.push(&text[start..i]);
            start = end;
            prev = Some(' ');
            continue;
        }
        prev = Some(ch);
    }
    parts.push(&text[start..]);
    parts
}

fn chunk_chars(text: &str, max_chars: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    chars
        .chunks(max_chars.max(1))
        .map(|c| c.iter().collect())
        .collect()
}

/// Spezza un testo lungo in parti di al massimo `max_chars` caratteri, preferendo i confini di frase.
pub fn split_long_text(text: &str, max_chars: usize) -> Vec<String> {
    if text.chars().count() <= max_chars {
        return vec![text.to_string()];
    }
    let sentences: Vec<&str> = split_sentences(text)
        .into_iter()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if sentences.is_empty() {
        return chunk_chars(text, max_chars);
    }

    let mut result: Vec<String> = Vec::new();
    let mut current = String::new();
    for sentence in sentences {
        let slen = sentence.chars().count();
        if slen > max_chars {
            if !current.is_empty() {
                result.push(std::mem::take(&mut current));
            }
            let mut chunk = String::new();
            for word in sentence.split_whitespace() {
                if chunk.chars().count() + word.chars().count() + 1 > max_chars {
                    if !chunk.is_empty() {
                        result.push(std::mem::take(&mut chunk));
                    }
                    chunk = word.to_string();
                } else if chunk.is_empty() {
                    chunk = word.to_string();
                } else {
                    chunk.push(' ');
                    chunk.push_str(word);
                }
            }
            if !chunk.is_empty() {
                result.push(chunk);
            }
        } else if current.chars().count() + slen + 1 > max_chars {
            if !current.is_empty() {
                result.push(std::mem::take(&mut current));
            }
            current = sentence.to_string();
        } else if current.is_empty() {
            current = sentence.to_string();
        } else {
            current.push(' ');
            current.push_str(sentence);
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

/// Rimuove i tag `[PAUSA n SECONDI]` dal testo.
pub fn parse_pause_tags(text: &str) -> String {
    match pause_tag_re() {
        Some(re) => re.replace_all(text, "").trim().to_string(),
        None => text.trim().to_string(),
    }
}

/// Secondi indicati da un tag `[PAUSA n SECONDI]` presente nella riga, se c'è.
fn explicit_pause(line: &str) -> Option<f64> {
    pause_tag_re()?
        .captures(line)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<f64>().ok())
}

/// Trasforma il copione in segmenti: ogni a capo diventa una pausa.
///
/// * una riga vuota è una pausa normale;
/// * `[PAUSA 5 SECONDI]` dopo il testo (o da sola su una riga) impone quella pausa;
/// * le righe troppo lunghe vengono spezzate (pausa più corta alla fine);
/// * pause consecutive si fondono nella più lunga e non restano pause finali.
pub fn insert_line_break_pauses(text: &str, pause_duration: f64, max_chars: usize) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();

    for raw_line in text.split('\n') {
        let clean = parse_pause_tags(raw_line);
        let explicit = explicit_pause(raw_line);

        if clean.trim().is_empty() {
            segments.push(Segment::Pause(explicit.unwrap_or(pause_duration)));
            continue;
        }

        let parts = split_long_text(&clean, max_chars);
        let was_split = parts.len() > 1;
        for part in parts {
            segments.push(Segment::Text(part));
        }
        let after = match explicit {
            Some(secs) => secs,
            None if was_split => pause_duration * 0.6,
            None => pause_duration,
        };
        segments.push(Segment::Pause(after));
    }

    let mut merged: Vec<Segment> = Vec::new();
    for seg in segments {
        match (&seg, merged.last_mut()) {
            (Segment::Pause(new), Some(Segment::Pause(old))) => {
                if *new > *old {
                    *old = *new;
                }
            }
            _ => merged.push(seg),
        }
    }
    while matches!(merged.last(), Some(Segment::Pause(_))) {
        merged.pop();
    }
    merged
}

/// Stima approssimativa della durata dell'audio, in secondi.
pub fn estimate_audio_duration(text: &str, speed: f64, pause_duration: f64) -> f64 {
    let char_count = text.chars().filter(|c| !c.is_whitespace()).count() as f64;
    let speech_seconds = char_count / (15.0 * speed.max(0.3));

    let lines = text.split('\n').filter(|l| !l.trim().is_empty()).count();
    if lines == 0 {
        return speech_seconds;
    }
    let pause_count = lines.saturating_sub(1) as f64;
    let mut total_pause = pause_count * pause_duration * 0.7;

    // Una sequenza di `. ! ?` conta come una sola frase.
    let mut sentences = 0usize;
    let mut in_run = false;
    for ch in text.chars() {
        let is_end = matches!(ch, '.' | '!' | '?');
        if is_end && !in_run {
            sentences += 1;
        }
        in_run = is_end;
    }
    total_pause += sentences as f64 * 0.3;
    speech_seconds + total_pause
}

/// Intervalli (in byte) delle parole ambigue, per evidenziarle nell'editor.
pub fn ambiguous_ranges(text: &str) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = ambiguous_res()
        .iter()
        .flat_map(|(re, _)| re.find_iter(text).map(|m| m.range()))
        .collect();
    out.sort_by_key(|r| r.start);
    out
}

/// Intervalli (in byte) dei tag `[PAUSA n SECONDI]`.
pub fn pause_tag_ranges(text: &str) -> Vec<Range<usize>> {
    match pause_tag_re() {
        Some(re) => re.find_iter(text).map(|m| m.range()).collect(),
        None => Vec::new(),
    }
}

/// Formatta i secondi come `mm:ss`.
pub fn format_time(seconds: f64) -> String {
    let s = if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    };
    let total = s as u64;
    format!("{:02}:{:02}", total / 60, total % 60)
}

/// Formatta una dimensione in byte (B, KB, MB, GB, TB).
pub fn format_file_size(bytes: u64) -> String {
    let mut size = bytes as f64;
    for unit in ["B", "KB", "MB", "GB"] {
        if size < 1024.0 {
            return format!("{size:.1} {unit}");
        }
        size /= 1024.0;
    }
    format!("{size:.1} TB")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ambiguous_words_are_fixed_case_insensitively() {
        assert_eq!(fix_ambiguous_words("Ancora subito"), "àncora sùbito");
        // Solo parole intere.
        assert_eq!(fix_ambiguous_words("finestra"), "finestra");
        assert_eq!(
            fix_ambiguous_words("la terra e il fine"),
            "la tèrra e il fìne"
        );
    }

    #[test]
    fn vowel_mode_doubles_accented_vowels() {
        let out = preprocess_text("già è così", PronunciationMode::Vowel);
        assert_eq!(out, "giàà èè cosìì");
    }

    #[test]
    fn paused_mode_adds_ellipsis_after_sentence_marks() {
        let out = preprocess_text("Ciao. Come?", PronunciationMode::Paused);
        assert_eq!(out, "Ciao. ...  Come? ... ");
    }

    #[test]
    fn mode_keys_round_trip() {
        for m in PronunciationMode::ALL {
            assert_eq!(PronunciationMode::from_key(m.key()), m);
        }
        assert_eq!(
            PronunciationMode::from_key("boh"),
            PronunciationMode::Original
        );
    }

    #[test]
    fn short_text_is_not_split() {
        assert_eq!(split_long_text("ciao", 350), vec!["ciao".to_string()]);
    }

    #[test]
    fn long_text_splits_on_sentences_within_limit() {
        let text = "Prima frase qui. Seconda frase qui! Terza frase qui? Quarta frase qui.";
        let parts = split_long_text(text, 30);
        assert!(parts.len() >= 2);
        assert!(parts.iter().all(|p| p.chars().count() <= 30), "{parts:?}");
        // Nessun testo perso.
        let joined = parts.join(" ");
        assert_eq!(joined, text);
    }

    #[test]
    fn very_long_sentence_splits_on_words() {
        let text = vec!["parola"; 100].join(" ");
        let parts = split_long_text(&text, 50);
        assert!(parts.iter().all(|p| p.chars().count() <= 50));
        assert_eq!(parts.join(" "), text);
    }

    #[test]
    fn line_breaks_become_pauses_without_trailing_pause() {
        let segs = insert_line_break_pauses("uno\ndue\ntre", 1.0, 350);
        assert_eq!(
            segs,
            vec![
                Segment::Text("uno".into()),
                Segment::Pause(1.0),
                Segment::Text("due".into()),
                Segment::Pause(1.0),
                Segment::Text("tre".into()),
            ]
        );
    }

    #[test]
    fn blank_lines_merge_into_one_pause() {
        let segs = insert_line_break_pauses("uno\n\n\ndue", 2.0, 350);
        assert_eq!(
            segs,
            vec![
                Segment::Text("uno".into()),
                Segment::Pause(2.0),
                Segment::Text("due".into())
            ]
        );
    }

    #[test]
    fn explicit_pause_tag_overrides_default() {
        let segs = insert_line_break_pauses("respira [PAUSA 5 SECONDI]\nora", 1.0, 350);
        assert_eq!(
            segs,
            vec![
                Segment::Text("respira".into()),
                Segment::Pause(5.0),
                Segment::Text("ora".into())
            ]
        );
    }

    #[test]
    fn pause_tag_alone_on_a_line_is_honoured_and_longest_pause_wins() {
        let segs = insert_line_break_pauses("uno\n[PAUSA 4.5 SECONDI]\ndue", 1.0, 350);
        assert_eq!(
            segs,
            vec![
                Segment::Text("uno".into()),
                Segment::Pause(4.5),
                Segment::Text("due".into())
            ]
        );
    }

    #[test]
    fn tag_is_case_insensitive_and_removed_from_text() {
        assert_eq!(parse_pause_tags("ciao [pausa 2 secondi]"), "ciao");
    }

    #[test]
    fn long_line_gets_shorter_closing_pause() {
        let line = vec!["parola"; 120].join(" ");
        let segs = insert_line_break_pauses(&format!("{line}\nfine"), 2.0, 350);
        let pauses: Vec<f64> = segs
            .iter()
            .filter_map(|s| {
                if let Segment::Pause(p) = s {
                    Some(*p)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(pauses, vec![1.2]);
    }

    #[test]
    fn empty_text_has_no_segments() {
        assert!(insert_line_break_pauses("", 1.0, 350).is_empty());
        assert!(insert_line_break_pauses("\n\n", 1.0, 350).is_empty());
    }

    #[test]
    fn duration_estimate_grows_with_text_and_shrinks_with_speed() {
        let short = estimate_audio_duration("Ciao a tutti.", 1.0, 1.0);
        let long = estimate_audio_duration("Ciao a tutti.\nCome state oggi?\nBene.", 1.0, 1.0);
        assert!(long > short);
        let fast = estimate_audio_duration("Ciao a tutti. Come state oggi?", 1.8, 1.0);
        let slow = estimate_audio_duration("Ciao a tutti. Come state oggi?", 0.5, 1.0);
        assert!(slow > fast);
        assert_eq!(estimate_audio_duration("", 1.0, 1.0), 0.0);
    }

    #[test]
    fn duration_counts_runs_of_punctuation_once() {
        let a = estimate_audio_duration("Ah...", 1.0, 1.0);
        let b = estimate_audio_duration("Ah.", 1.0, 1.0);
        assert!((a - b).abs() < 0.5);
    }

    #[test]
    fn highlight_ranges_point_at_the_right_bytes() {
        let text = "è ancora qui [PAUSA 3 SECONDI]";
        let amb = ambiguous_ranges(text);
        assert_eq!(amb.len(), 1);
        assert_eq!(&text[amb[0].clone()], "ancora");
        let tags = pause_tag_ranges(text);
        assert_eq!(&text[tags[0].clone()], "[PAUSA 3 SECONDI]");
    }

    #[test]
    fn formatting_helpers() {
        assert_eq!(format_time(75.4), "01:15");
        assert_eq!(format_time(-3.0), "00:00");
        assert_eq!(format_time(f64::NAN), "00:00");
        assert_eq!(format_file_size(512), "512.0 B");
        assert_eq!(format_file_size(1536), "1.5 KB");
        assert_eq!(format_file_size(5 * 1024 * 1024), "5.0 MB");
    }
}
