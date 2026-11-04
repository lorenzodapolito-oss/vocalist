//! Testo dei file PDF.

use std::path::Path;

pub fn read_pdf(path: &Path) -> Result<String, String> {
    // La libreria PDF può fallire male con file strani: il guasto diventa un errore normale.
    let p = path.to_path_buf();
    match std::panic::catch_unwind(move || pdf_extract::extract_text(&p)) {
        Ok(Ok(t)) => Ok(join_pdf_lines(&t)),
        Ok(Err(e)) => Err(format!("PDF illeggibile: {e}")),
        Err(_) => Err("PDF non leggibile (file danneggiato o protetto)".into()),
    }
}

/// Nei PDF ogni riga va a capo: unisce le righe dello stesso paragrafo (separati da riga vuota)
/// e ricompone le parole spezzate con il trattino.
pub fn join_pdf_lines(text: &str) -> String {
    let mut paras: Vec<String> = Vec::new();
    let mut cur = String::new();
    for line in text.replace("\r\n", "\n").lines() {
        let line = line.trim();
        if line.is_empty() {
            if !cur.is_empty() {
                paras.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if cur.ends_with('-') && line.chars().next().is_some_and(char::is_lowercase) {
            cur.pop();
        } else if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(line);
    }
    if !cur.is_empty() {
        paras.push(cur);
    }
    paras.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_lines_are_joined_per_paragraph() {
        let t = "This is a sen-\ntence split\nover lines.\n\nNew paragraph\nhere.";
        assert_eq!(
            join_pdf_lines(t),
            "This is a sentence split over lines.\nNew paragraph here."
        );
        assert_eq!(join_pdf_lines(""), "");
    }

    #[test]
    fn broken_pdf_is_an_error_not_a_crash() {
        let p = std::env::temp_dir().join(format!("voc_bad_{}.pdf", std::process::id()));
        std::fs::write(&p, b"%PDF-1.4 rotto").unwrap();
        assert!(read_pdf(&p).is_err());
        let _ = std::fs::remove_file(&p);
    }
}
