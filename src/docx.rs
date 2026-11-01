//! Lettura del testo dai file .docx (paragrafi, uno per riga).

use regex::Regex;
use std::io::Read;
use std::path::Path;

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let rep = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match rep {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Estrae i paragrafi dall'XML di `word/document.xml`.
pub fn paragraphs_from_xml(xml: &str) -> Vec<String> {
    let (Ok(para), Ok(token)) = (
        Regex::new(r"(?s)<w:p(?:\s[^>]*)?/>|<w:p(?:\s[^>]*)?>.*?</w:p>"),
        Regex::new(r"(?s)<w:t(?:\s[^>]*)?>(.*?)</w:t>|<w:tab\s*/>|<w:br\s*/>|<w:br\s[^>]*/>"),
    ) else {
        return Vec::new();
    };
    para.find_iter(xml)
        .map(|p| {
            let mut line = String::new();
            for cap in token.captures_iter(p.as_str()) {
                match cap.get(1) {
                    Some(t) => line.push_str(&unescape(t.as_str())),
                    None if cap[0].starts_with("<w:tab") => line.push('\t'),
                    None => line.push('\n'),
                }
            }
            line.trim().to_string()
        })
        .collect()
}

pub fn read_docx(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("file DOCX non valido: {e}"))?;
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .map_err(|e| format!("document.xml mancante: {e}"))?
        .read_to_string(&mut xml)
        .map_err(|e| e.to_string())?;
    Ok(paragraphs_from_xml(&xml).join("\n"))
}

/// Legge un file .txt o .docx.
pub fn load_text_file(path: &Path) -> Result<String, String> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("txt") => std::fs::read_to_string(path).map_err(|e| e.to_string()),
        Some("docx") => read_docx(path),
        _ => Err("Formato non supportato".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const XML: &str = r#"<w:document><w:body>
<w:p><w:pPr><w:pStyle w:val="x"/></w:pPr><w:r><w:t>Chiudi gli occhi.</w:t></w:r></w:p>
<w:p/>
<w:p><w:r><w:t xml:space="preserve">Respira </w:t></w:r><w:r><w:t>&amp; rilassati &#232;</w:t></w:r></w:p>
<w:p><w:r><w:t>a</w:t><w:tab/><w:t>b</w:t><w:br/><w:t>c</w:t></w:r></w:p>
</w:body></w:document>"#;

    #[test]
    fn paragraphs_are_extracted_with_entities_and_empty_lines() {
        let p = paragraphs_from_xml(XML);
        assert_eq!(
            p,
            vec!["Chiudi gli occhi.", "", "Respira & rilassati è", "a\tb\nc"]
        );
    }

    #[test]
    fn unescape_handles_odd_input() {
        assert_eq!(unescape("a & b &unknown; &#x41;"), "a & b &unknown; A");
    }

    #[test]
    fn reads_a_real_zip() {
        let path = std::env::temp_dir().join(format!("medit_{}.docx", std::process::id()));
        {
            let f = std::fs::File::create(&path).unwrap();
            let mut z = zip::ZipWriter::new(f);
            z.start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            z.write_all(XML.as_bytes()).unwrap();
            z.finish().unwrap();
        }
        let text = load_text_file(&path).unwrap();
        assert!(text.starts_with("Chiudi gli occhi.\n\nRespira"));
    }

    #[test]
    fn bad_files_are_errors() {
        let path = std::env::temp_dir().join(format!("medit_bad_{}.docx", std::process::id()));
        std::fs::write(&path, b"non e' uno zip").unwrap();
        assert!(read_docx(&path).is_err());
        assert!(load_text_file(Path::new("x.pdf")).is_err());
    }

    #[test]
    fn txt_is_read_as_is() {
        let path = std::env::temp_dir().join(format!("medit_{}.txt", std::process::id()));
        std::fs::write(&path, "ciao\nmondo").unwrap();
        assert_eq!(load_text_file(&path).unwrap(), "ciao\nmondo");
    }
}
