// Text out of Word documents, for the chat.
//
// Claude Code's Read tool takes text, images and PDFs, but a .docx is a zip of
// XML, so Frank pulls the text out into a .txt beside the file in the inbox and
// points Claude at that. Paragraphs become lines; a table becomes one line per
// row with its cells separated by " | ".

use std::io::Read;
use std::path::{Path, PathBuf};

/// Word documents are capped well above any real one: past this, the zip is
/// more likely a bomb than a letter.
const MAX_XML: u64 = 64 * 1024 * 1024;

/// For a document Claude can't read as it is, a readable copy of it: the path
/// of a .txt holding its text. None for files Claude reads directly.
pub fn readable_copy(path: &Path) -> Option<Result<PathBuf, String>> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if ext != "docx" {
        return None;
    }
    Some(docx_to_txt(path))
}

fn docx_to_txt(path: &Path) -> Result<PathBuf, String> {
    let text = docx_text(path)?;
    let mut out = path.as_os_str().to_owned();
    out.push(".txt");
    let out = PathBuf::from(out);
    std::fs::write(&out, text).map_err(|e| format!("cannot write the text: {e}"))?;
    Ok(out)
}

pub fn docx_text(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("cannot open the document: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| "this is not a Word (.docx) document".to_string())?;
    let entry = zip
        .by_name("word/document.xml")
        .map_err(|_| "this is not a Word (.docx) document".to_string())?;
    if entry.size() > MAX_XML {
        return Err("the document is too large".into());
    }
    let mut xml = String::new();
    entry
        .take(MAX_XML)
        .read_to_string(&mut xml)
        .map_err(|e| format!("cannot read the document: {e}"))?;
    Ok(wordml_text(&xml))
}

/// The text of a WordprocessingML body.
fn wordml_text(xml: &str) -> String {
    let mut out = String::new();
    let mut in_text = false;
    let mut cell_depth = 0usize;
    let mut rest = xml;
    while let Some(open) = rest.find('<') {
        if in_text {
            out.push_str(&unescape(&rest[..open]));
        }
        let Some(close) = rest[open..].find('>') else { break };
        let tag = &rest[open + 1..open + close];
        rest = &rest[open + close + 1..];

        let closing = tag.starts_with('/');
        let self_closing = tag.ends_with('/');
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        match (name, closing) {
            ("w:t", false) => in_text = !self_closing,
            ("w:t", true) => in_text = false,
            ("w:tab", false) => out.push('\t'),
            ("w:br" | "w:cr", false) => out.push('\n'),
            ("w:tc", false) => cell_depth += 1,
            ("w:tc", true) => {
                cell_depth = cell_depth.saturating_sub(1);
                out.truncate(out.trim_end_matches(' ').len());
                out.push_str(" | ");
            }
            ("w:tr", true) => {
                out.truncate(out.trim_end_matches([' ', '|']).len());
                out.push('\n');
            }
            // An empty paragraph, <w:p/>, is a blank line all the same.
            ("w:p", true) | ("w:p", false) if closing || self_closing => {
                out.push(if cell_depth > 0 { ' ' } else { '\n' })
            }
            _ => {}
        }
    }
    tidy(&out)
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let Some(semi) = rest[amp..].find(';') else {
            out.push_str(&rest[amp..]);
            return out;
        };
        let entity = &rest[amp + 1..amp + semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => out.push(c),
            None => out.push_str(&rest[amp..amp + semi + 1]),
        }
        rest = &rest[amp + semi + 1..];
    }
    out.push_str(rest);
    out
}

/// Trailing spaces off every line, and no more than one blank line in a row.
fn tidy(text: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in text.lines().map(str::trim_end) {
        if line.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const BODY: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:r><w:t>Frank test belgesi</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">Toplantı </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>Salı 14:00</w:t></w:r><w:r><w:t>'te &amp; sonra</w:t></w:r><w:r><w:tab/><w:t>çay</w:t></w:r></w:p>
<w:p/>
<w:tbl><w:tr><w:tc><w:p><w:r><w:t>Ürün</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Fiyat</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:tc><w:p><w:r><w:t>Kalem</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>12 TL</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
<w:p><w:r><w:t>Son satır</w:t></w:r><w:r><w:br/><w:t>alt satır</w:t></w:r></w:p>
</w:body></w:document>"#;

    #[test]
    fn paragraphs_runs_and_tables_come_out_as_text() {
        assert_eq!(
            wordml_text(BODY),
            "Frank test belgesi\nToplantı Salı 14:00'te & sonra\tçay\n\nÜrün | Fiyat\nKalem | 12 TL\nSon satır\nalt satır\n"
        );
    }

    #[test]
    fn entities_are_decoded() {
        assert_eq!(unescape("a &lt;b&gt; &#252; &#x15F; &bogus; &"), "a <b> ü ş &bogus; &");
    }

    #[test]
    fn a_docx_in_the_inbox_gets_a_text_copy() {
        let dir = std::env::temp_dir().join(format!("frank-office-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rapor.docx");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(BODY.as_bytes()).unwrap();
        zip.finish().unwrap();

        let txt = readable_copy(&path).unwrap().unwrap();
        assert_eq!(txt, dir.join("rapor.docx.txt"));
        assert!(std::fs::read_to_string(&txt).unwrap().contains("Kalem | 12 TL"));

        // Not a zip at all: a clear error, not a crash.
        let fake = dir.join("fake.docx");
        std::fs::write(&fake, b"not a zip").unwrap();
        assert!(readable_copy(&fake).unwrap().is_err());
        // Anything else is read by Claude as it is.
        assert!(readable_copy(&dir.join("notes.pdf")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
