//! Traduzione offline EN → IT con un modello Marian (candle), tutta in Rust.

pub mod chunk;
pub mod panel;
pub mod pdf;
pub mod spm;
pub mod tok;
pub mod translator;

use crate::translate::translator::Translator;
use std::sync::atomic::{AtomicBool, Ordering};

pub const CHUNK_CHARS: usize = 400;

/// Traduce `text` pezzo per pezzo (paragrafi e frasi, mai a metà parola). Mantiene i paragrafi.
/// `progress(fatti, totali)` viene chiamata dopo ogni pezzo.
pub fn translate_text(
    tr: &mut Translator,
    text: &str,
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize, usize),
) -> Result<String, String> {
    if text.trim().is_empty() {
        return Err("testo vuoto".into());
    }
    let paras = chunk::split(text, CHUNK_CHARS);
    let total = chunk::total_chunks(&paras);
    let mut done = 0;
    let mut out: Vec<String> = Vec::new();
    for p in &paras {
        let mut parts = Vec::new();
        for c in &p.chunks {
            if cancel.load(Ordering::SeqCst) {
                return Err("traduzione interrotta".into());
            }
            parts.push(tr.translate(c)?);
            done += 1;
            progress(done, total);
        }
        out.push(parts.join(" "));
    }
    Ok(out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_paragraphs_reports_progress_and_can_be_cancelled() {
        let mut tr = crate::translate::translator::test_translator();
        let cancel = AtomicBool::new(false);
        let mut seen = Vec::new();
        let out = translate_text(&mut tr, "hello world\n\nhello", &cancel, |d, t| {
            seen.push((d, t))
        })
        .unwrap();
        assert_eq!(seen, vec![(1, 2), (2, 2)]);
        assert_eq!(
            out.split('\n').count(),
            3,
            "paragrafi e riga vuota conservati"
        );
        assert!(translate_text(&mut tr, "  \n ", &cancel, |_, _| {}).is_err());
        cancel.store(true, Ordering::SeqCst);
        assert!(translate_text(&mut tr, "hello", &cancel, |_, _| {})
            .unwrap_err()
            .contains("interrotta"));
    }
}
