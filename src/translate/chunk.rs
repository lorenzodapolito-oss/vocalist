//! Divisione del testo in pezzi traducibili che non spezzano le parole.

/// Un pezzo da tradurre; `Some(testo)` oppure una riga vuota da conservare (`None`).
#[derive(Debug, PartialEq)]
pub struct Paragraph {
    pub chunks: Vec<String>,
}

fn split_long(text: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    // Prima alle fine di frase, poi alle parole.
    let mut pieces: Vec<&str> = Vec::new();
    let mut start = 0;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (k, (i, c)) in chars.iter().enumerate() {
        let next_is_space = chars.get(k + 1).is_none_or(|(_, n)| n.is_whitespace());
        if matches!(c, '.' | '!' | '?' | ';' | ':') && next_is_space {
            let end = i + c.len_utf8();
            if let Some(p) = text.get(start..end) {
                pieces.push(p);
            }
            start = end;
        }
    }
    if let Some(rest) = text.get(start..) {
        pieces.push(rest);
    }
    for p in pieces {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        if p.chars().count() > max {
            for w in p.split_whitespace() {
                if !cur.is_empty() && cur.chars().count() + 1 + w.chars().count() > max {
                    out.push(std::mem::take(&mut cur));
                }
                if !cur.is_empty() {
                    cur.push(' ');
                }
                cur.push_str(w);
            }
            continue;
        }
        if !cur.is_empty() && cur.chars().count() + 1 + p.chars().count() > max {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(p);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Paragrafi (separati da a capo) divisi in pezzi di al massimo `max` caratteri. Una riga vuota
/// diventa un paragrafo senza pezzi.
pub fn split(text: &str, max: usize) -> Vec<Paragraph> {
    let max = max.max(20);
    text.replace("\r\n", "\n")
        .split('\n')
        .map(|line| {
            let line = line.trim();
            Paragraph {
                chunks: if line.is_empty() {
                    Vec::new()
                } else {
                    split_long(line, max)
                },
            }
        })
        .collect()
}

pub fn total_chunks(paras: &[Paragraph]) -> usize {
    paras.iter().map(|p| p.chunks.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_paragraphs_stay_whole_and_blank_lines_are_kept() {
        let p = split("Uno due.\n\nTre", 400);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].chunks, vec!["Uno due."]);
        assert!(p[1].chunks.is_empty());
        assert_eq!(total_chunks(&p), 2);
    }

    #[test]
    fn long_text_splits_on_sentences_then_words_within_the_limit() {
        let text = "Prima frase qui. Seconda frase un po' piu lunga. Terza.";
        let p = split(text, 30);
        for c in &p[0].chunks {
            assert!(c.chars().count() <= 30, "{c}");
        }
        assert_eq!(p[0].chunks.join(" "), text);
        let words = "parola ".repeat(30);
        let p = split(&words, 25);
        assert!(p[0].chunks.len() > 5);
        assert!(p[0].chunks.iter().all(|c| c.chars().count() <= 25));
    }

    #[test]
    fn multibyte_text_and_tiny_limits_do_not_panic() {
        let p = split(
            "È già così: perché? Però… ñandú.\r\nÜber 日本語のテキストです。",
            1,
        );
        assert!(total_chunks(&p) >= 2);
        assert!(split("", 100).iter().all(|p| p.chunks.is_empty()));
    }
}
