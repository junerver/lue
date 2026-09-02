//! EPUB text extraction (port of `content_parser._extract_content_epub`):
//! META-INF/container.xml → OPF manifest/spine → one chapter per spine
//! document, with block-level HTML turned into one line per element so the
//! TXT chapter rules and the paragraph pipeline see a familiar shape.

use crate::error::LueError;
use crate::text::clean_visual_text_impl;
use std::io::{Read, Seek};
use std::path::Path;

/// Extract an EPUB into one text blob per spine document (lines joined with
/// `\n`, documents in spine order). Documents that yield no visible text are
/// dropped, mirroring the Python extractor.
pub fn extract_epub_docs(path: &Path) -> Result<Vec<String>, LueError> {
    let file = std::fs::File::open(path).map_err(LueError::Io)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| LueError::Invalid(format!("not a valid EPUB (ZIP): {e}")))?;
    let opf_path = find_opf_path(&mut archive)?;
    let spine = read_spine_hrefs(&mut archive, &opf_path)?;
    let mut docs = Vec::with_capacity(spine.len());
    for href in spine {
        if let Some(doc) = read_document(&mut archive, &href) {
            docs.push(doc);
        }
    }
    Ok(docs)
}

fn find_opf_path<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> Result<String, LueError> {
    let container = read_entry(archive, "META-INF/container.xml").ok_or_else(|| {
        LueError::Invalid("EPUB is missing META-INF/container.xml".into())
    })?;
    let xml = decode_bytes(&container);
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(e)) | Ok(quick_xml::events::Event::Empty(e)) => {
                if e.name().local_name().as_ref() == b"rootfile" {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"full-path" {
                            let value = String::from_utf8_lossy(&attr.value).into_owned();
                            if !value.is_empty() {
                                return Ok(value.replace('\\', "/"));
                            }
                        }
                    }
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    Err(LueError::Invalid("EPUB container.xml has no rootfile".into()))
}

/// Manifest (id → resolved href) entries in spine order; the NCX and the
/// `properties="nav"` document are skipped like the Python extractor does.
fn read_spine_hrefs<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    opf_path: &str,
) -> Result<Vec<String>, LueError> {
    let data = read_entry(archive, opf_path)
        .ok_or_else(|| LueError::Invalid(format!("EPUB is missing its OPF file {opf_path}")))?;
    let xml = decode_bytes(&data);
    let opf_dir = match opf_path.rfind('/') {
        Some(i) => &opf_path[..=i],
        None => "",
    };
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut buf = Vec::new();
    let mut manifest: Vec<(String, String)> = Vec::new();
    let mut spine_ids: Vec<String> = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(e)) | Ok(quick_xml::events::Event::Empty(e)) => {
                let name = e.name().local_name().as_ref().to_vec();
                let attrs: Vec<(Vec<u8>, String)> = e
                    .attributes()
                    .flatten()
                    .map(|a| {
                        (
                            a.key.as_ref().to_vec(),
                            String::from_utf8_lossy(&a.value).into_owned(),
                        )
                    })
                    .collect();
                let get = |k: &[u8]| {
                    attrs
                        .iter()
                        .find(|(key, _)| key == k)
                        .map(|(_, v)| v.clone())
                };
                match name.as_slice() {
                    b"item" => {
                        let id = get(b"id");
                        let href = get(b"href").map(|h| resolve_href(opf_dir, &h));
                        let is_ncx =
                            get(b"media-type").is_some_and(|m| m == "application/x-dtbncx+xml");
                        let is_nav = get(b"properties")
                            .is_some_and(|p| p.split_whitespace().any(|word| word == "nav"));
                        if let (Some(id), Some(href)) = (id, href) {
                            if !is_ncx && !is_nav {
                                manifest.push((id, href));
                            }
                        }
                    }
                    b"itemref" => {
                        if let Some(idref) = get(b"idref") {
                            spine_ids.push(idref);
                        }
                    }
                    _ => {}
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(spine_ids
        .into_iter()
        .filter_map(|id| {
            manifest
                .iter()
                .find(|(mid, _)| *mid == id)
                .map(|(_, href)| href.clone())
        })
        .collect())
}

fn read_document<R: Read + Seek>(archive: &mut zip::ZipArchive<R>, href: &str) -> Option<String> {
    // Zip entry names are usually raw; try the percent-decoded form too.
    let bytes = read_entry(archive, href)
        .or_else(|| read_entry(archive, &percent_decode(href)))?;
    let html = decode_bytes(&bytes);
    let lines = html_to_lines(&html);
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn read_entry<R: Read + Seek>(archive: &mut zip::ZipArchive<R>, name: &str) -> Option<Vec<u8>> {
    let mut entry = archive.by_name(name).ok()?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// Same cascade as the TXT reader: UTF-8 (with BOM) first, GB18030 for the
/// rare legacy document, lossy at the last step.
fn decode_bytes(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::GB18030.decode(bytes).0.into_owned(),
    }
}

fn resolve_href(opf_dir: &str, href: &str) -> String {
    let href = href.replace('\\', "/");
    let href = href.trim_start_matches("./");
    if let Some(stripped) = href.strip_prefix('/') {
        stripped.to_string()
    } else {
        format!("{opf_dir}{href}")
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 3 <= bytes.len() {
            if let Ok(v) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// HTML → lines: block-level elements (p/div/h1-6/li/blockquote/dt/dd/tr…)
/// each become one line, `<br>` breaks a line, script/style/head/sup/sub
/// content is skipped, entities are decoded, inline markup is flattened.
fn html_to_lines(html: &str) -> Vec<String> {
    fn is_hidden(tag: &str) -> bool {
        matches!(tag, "script" | "style" | "head" | "title" | "sup" | "sub")
    }

    fn is_block(tag: &str) -> bool {
        matches!(
            tag,
            "p"
                | "div"
                | "li"
                | "blockquote"
                | "dt"
                | "dd"
                | "tr"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "section"
                | "article"
        )
    }

    fn flush(line: &mut String, lines: &mut Vec<String>) {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            lines.push(trimmed.to_string());
        }
        line.clear();
    }

    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut hide_depth = 0usize;
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if html[i..].starts_with("<!--") {
                match html[i + 4..].find("-->") {
                    Some(end) => i += 4 + end + 3,
                    None => break,
                }
                continue;
            }
            // A `<` that does not open a tag shape stays literal text.
            let next = *bytes.get(i + 1).unwrap_or(&0);
            if !(next.is_ascii_alphabetic() || next == b'/' || next == b'!' || next == b'?') {
                if hide_depth == 0 {
                    line.push('<');
                }
                i += 1;
                continue;
            }
            // Find the tag's `>`, ignoring quotes so attributes may contain it.
            let mut j = i + 1;
            let mut in_quote = false;
            while j < bytes.len() {
                let b = bytes[j];
                if b == b'"' {
                    in_quote = !in_quote;
                } else if b == b'>' && !in_quote {
                    break;
                }
                j += 1;
            }
            if j >= bytes.len() {
                break; // unterminated tag: drop the rest
            }
            let inner = &html[i + 1..j];
            let is_close = inner.starts_with('/');
            let self_closing = inner.ends_with('/');
            let name = inner.trim_start_matches('/');
            let name_end = name
                .bytes()
                .position(|b| !(b.is_ascii_alphanumeric() || b == b'-' || b == b':'))
                .unwrap_or(name.len());
            let name = &name[..name_end];
            if is_hidden(name) {
                if is_close {
                    hide_depth = hide_depth.saturating_sub(1);
                } else if !self_closing {
                    hide_depth += 1;
                }
            } else if is_block(name) || name == "br" {
                flush(&mut line, &mut lines);
            }
            i = j + 1;
        } else {
            let end = html[i..]
                .find('<')
                .map_or(bytes.len(), |p| i + p);
            if hide_depth == 0 {
                line.push_str(&decode_entities(&html[i..end]));
            }
            i = end;
        }
    }
    flush(&mut line, &mut lines);
    lines
}

/// Decode the HTML entities the reader's texts actually contain; unknown
/// entities pass through untouched.
fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text.as_bytes()[i] == b'&' {
            if let Some(semi) = text[i..].find(';') {
                let entity = &text[i + 1..i + semi];
                let decoded = match entity {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some('\u{a0}'),
                    "hellip" => Some('…'),
                    "mdash" => Some('—'),
                    "ldquo" => Some('“'),
                    "rdquo" => Some('”'),
                    "lsquo" => Some('‘'),
                    "rsquo" => Some('’'),
                    _ => {
                        if let Some(num) = entity
                            .strip_prefix("#x")
                            .or_else(|| entity.strip_prefix("#X"))
                        {
                            u32::from_str_radix(num, 16).ok().and_then(char::from_u32)
                        } else if let Some(num) = entity.strip_prefix('#') {
                            num.parse::<u32>().ok().and_then(char::from_u32)
                        } else {
                            None
                        }
                    }
                };
                if let Some(c) = decoded {
                    out.push(c);
                    i += semi + 1;
                    continue;
                }
            }
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// DOCX → one chapter of cleaned paragraph lines (port of
/// `_extract_content_docx`): text of every `<w:p>` in word/document.xml,
/// visually cleaned, keeping lines longer than 3 chars.
pub fn extract_docx_docs(path: &Path) -> Result<Vec<String>, LueError> {
    let file = std::fs::File::open(path).map_err(LueError::Io)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| LueError::Invalid(format!("not a valid DOCX (ZIP): {e}")))?;
    let data = read_entry(&mut archive, "word/document.xml")
        .ok_or_else(|| LueError::Invalid("DOCX is missing word/document.xml".into()))?;
    let xml = decode_bytes(&data);
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut buf = Vec::new();
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(e)) => {
                if e.name().local_name().as_ref() == b"p" {
                    current.clear();
                } else if e.name().local_name().as_ref() == b"tab" {
                    current.push('\t');
                } else if e.name().local_name().as_ref() == b"br" {
                    current.push(' ');
                }
            }
            Ok(quick_xml::events::Event::Empty(e)) => {
                match e.name().local_name().as_ref() {
                    b"tab" => current.push('\t'),
                    b"br" => current.push(' '),
                    _ => {}
                }
            }
            Ok(quick_xml::events::Event::Text(t)) => {
                let text = t
                    .unescape()
                    .map_err(|e| LueError::Invalid(format!("bad DOCX XML entity: {e}")))?;
                current.push_str(&text);
            }
            Ok(quick_xml::events::Event::End(e)) => {
                if e.name().local_name().as_ref() == b"p" {
                    let line = clean_visual_text_impl(current.trim());
                    if line.chars().count() > 3 {
                        paragraphs.push(line);
                    }
                    current.clear();
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(e) => return Err(LueError::Invalid(format!("bad DOCX XML: {e}"))),
            _ => {}
        }
        buf.clear();
    }
    Ok(single_chapter(paragraphs))
}

/// Wrap one document's lines as the single chapter the Python extractors
/// return (`[lines]`), joined with newlines.
fn single_chapter(lines: Vec<String>) -> Vec<String> {
    if lines.is_empty() {
        Vec::new()
    } else {
        vec![lines.join("\n")]
    }
}

/// HTML file → one chapter of block-per-line text (`_extract_content_html`).
pub fn extract_html_docs(path: &Path) -> Result<Vec<String>, LueError> {
    let bytes = std::fs::read(path).map_err(LueError::Io)?;
    let html = decode_bytes(&bytes);
    Ok(single_chapter(html_to_lines(&html)))
}

/// Markdown file → one chapter (`_parse_raw_markdown` port): fenced code
/// blocks are marked `__CODE_BLOCK__` (the cleaner turns them back into
/// indented code), headers/lists/quotes keep their visual shape.
pub fn extract_markdown_docs(path: &Path) -> Result<Vec<String>, LueError> {
    let bytes = std::fs::read(path).map_err(LueError::Io)?;
    let md = decode_bytes(&bytes).replace("\r\n", "\n").replace('\r', "\n");
    Ok(single_chapter(parse_raw_markdown(&md)))
}

fn parse_raw_markdown(md: &str) -> Vec<String> {
    use regex::Regex;

    let numbered = Regex::new(r"^\d+\.\s").unwrap();
    let indented_list = Regex::new(r"^\s+(-|\*|\+|\d+\.)\s+").unwrap();

    let mut result: Vec<String> = Vec::new();
    let mut in_code_block = false;
    let mut code_fence: Option<String> = None;

    for raw_line in md.split('\n') {
        let line = raw_line.trim_end();
        if line.starts_with("```") || line.starts_with("~~~") {
            if !in_code_block {
                in_code_block = true;
                code_fence = Some(line[..3].to_string());
                result.push(String::new());
                let lang = line[3..].trim();
                result.push(if lang.is_empty() {
                    "Code:".to_string()
                } else {
                    format!("Code ({lang}):")
                });
                continue;
            } else if code_fence.as_deref().is_some_and(|f| line.starts_with(f)) {
                in_code_block = false;
                code_fence = None;
                result.push(String::new());
                continue;
            }
        }

        if in_code_block {
            result.push(format!("__CODE_BLOCK__    {line}"));
        } else if line.starts_with('#') {
            let header_text = line.trim_start_matches('#').trim().to_string();
            if !header_text.is_empty() {
                result.push(String::new());
                result.push(header_text);
                result.push(String::new());
            }
        } else if line.starts_with("- ") || line.starts_with("* ") || line.starts_with("+ ") {
            result.push(format!("• {}", line[2..].trim()));
        } else if numbered.is_match(line) {
            let text = numbered.replace(line, "").trim().to_string();
            result.push(format!("• {text}"));
        } else if indented_list.is_match(line) {
            let text = indented_list.replace(line, "").trim().to_string();
            result.push(format!("    • {text}"));
        } else if line.starts_with("    ") || line.starts_with('\t') {
            result.push(format!("__CODE_BLOCK__    {}", line.trim()));
        } else if line.starts_with('>') {
            let quote_text = line.trim_start_matches('>').trim().to_string();
            if !quote_text.is_empty() {
                result.push(format!("    {quote_text}"));
            }
        } else if line.trim().is_empty() {
            if result.last().is_some_and(|last| !last.is_empty()) {
                result.push(String::new());
            }
        } else {
            result.push(line.trim().to_string());
        }
    }

    // Global visual cleaning, then cap runs of empty lines at two.
    let result: Vec<String> = result
        .into_iter()
        .map(|line| {
            if line.trim().is_empty() {
                line
            } else {
                clean_visual_text_impl(&line)
            }
        })
        .collect();
    let mut clean: Vec<String> = Vec::with_capacity(result.len());
    let mut empty_count = 0;
    for line in result {
        if line.is_empty() {
            empty_count += 1;
            if empty_count <= 2 {
                clean.push(line);
            }
        } else {
            empty_count = 0;
            clean.push(line);
        }
    }
    // Python keeps the empty lines only in short documents.
    if clean.len() < 100 {
        clean
    } else {
        clean.into_iter().filter(|l| !l.is_empty()).collect()
    }
}

/// RTF file → one chapter of cleaned lines (`_extract_content_rtf`). The
/// parser is a pragmatic subset of striprtf: groups, ignorable destinations
/// (`\*`, fonttbl/colortbl/pict/…), `\par`/`\line`, `\tab`, `\'hh` (CP1252)
/// and `\uN?` unicode escapes are understood.
pub fn extract_rtf_docs(path: &Path) -> Result<Vec<String>, LueError> {
    let bytes = std::fs::read(path).map_err(LueError::Io)?;
    let content = decode_bytes(&bytes);
    let text = rtf_to_text(&content).replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<String> = text
        .split('\n')
        .map(|line| clean_visual_text_impl(line.trim()))
        .filter(|line| line.chars().count() > 3)
        .collect();
    Ok(single_chapter(lines))
}

fn rtf_to_text(rtf: &str) -> String {
    /// Destinations whose whole group holds metadata, not text.
    fn is_destination(word: &str) -> bool {
        matches!(
            word,
            "fonttbl"
                | "colortbl"
                | "stylesheet"
                | "info"
                | "pict"
                | "object"
                | "header"
                | "footer"
                | "listtable"
                | "listoverridetable"
                | "revtbl"
                | "generator"
                | "rsidtbl"
                | "themedata"
                | "colorschememapping"
                | "datastore"
                | "xmlnstbl"
        )
    }

    let chars: Vec<char> = rtf.chars().collect();
    let mut out = String::new();
    let mut skip_stack: Vec<bool> = vec![false];
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '{' => {
                skip_stack.push(*skip_stack.last().unwrap_or(&false));
                i += 1;
            }
            '}' => {
                skip_stack.pop();
                if skip_stack.is_empty() {
                    skip_stack.push(false);
                }
                i += 1;
            }
            '\\' => {
                i += 1;
                let c2 = chars.get(i).copied().unwrap_or('\0');
                if c2.is_ascii_alphabetic() {
                    let start = i;
                    while i < chars.len() && chars[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    let word: String = chars[start..i].iter().collect();
                    let mut param: Option<i64> = None;
                    if chars.get(i).is_some_and(|c| *c == '-' || c.is_ascii_digit()) {
                        let ps = i;
                        if chars.get(i) == Some(&'-') {
                            i += 1;
                        }
                        while i < chars.len() && chars[i].is_ascii_digit() {
                            i += 1;
                        }
                        param = chars[ps..i].iter().collect::<String>().parse().ok();
                    }
                    if chars.get(i) == Some(&' ') {
                        i += 1;
                    }
                    if *skip_stack.last().unwrap_or(&false) {
                        continue;
                    }
                    match word.as_str() {
                        "par" | "line" => out.push('\n'),
                        "tab" => out.push('\t'),
                        "u" => {
                            if let Some(cp) = param {
                                out.push(char::from_u32(cp as u32).unwrap_or('\u{fffd}'));
                                // \uN is followed by one fallback char to skip
                                if chars.get(i) == Some(&'\\')
                                    && chars.get(i + 1) == Some(&'\'')
                                {
                                    i += 4;
                                } else if i < chars.len() {
                                    i += 1;
                                }
                            }
                        }
                        other => {
                            if is_destination(other) {
                                *skip_stack.last_mut().expect("stack never empty") = true;
                            }
                        }
                    }
                } else {
                    match c2 {
                        '\\' | '{' | '}' => {
                            if !*skip_stack.last().unwrap_or(&false) {
                                out.push(c2);
                            }
                            i += 1;
                        }
                        '\'' => {
                            let hex: String =
                                chars.get(i + 1..i + 3).unwrap_or(&[]).iter().collect();
                                if let Ok(b) = u8::from_str_radix(&hex, 16) {
                                    if !*skip_stack.last().unwrap_or(&false) {
                                        out.push_str(encoding_rs::WINDOWS_1252.decode(&[b]).0.into_owned().as_str());
                                    }
                                }
                            i += 3;
                        }
                        '*' => {
                            // \* marks the current group ignorable
                            if let Some(top) = skip_stack.last_mut() {
                                *top = true;
                            }
                            i += 1;
                        }
                        _ => i += 1,
                    }
                }
            }
            '\n' | '\r' => i += 1, // raw newlines carry no meaning in RTF
            other => {
                if !*skip_stack.last().unwrap_or(&false) {
                    out.push(other);
                }
                i += 1;
            }
        }
    }
    out
}

/// PDF → chapters. The text layer is extracted with pdf-extract (pure
/// Rust); if the usual chapter-title rules find structure in the text, the
/// book is split into real chapters, otherwise it stays one chapter of
/// cleaned lines. PyMuPDF's positional footnote/header filtering has no
/// equivalent in a plain text flow, so noisy-margin PDFs read noisier here.
pub fn extract_pdf_docs(path: &Path) -> Result<Vec<String>, LueError> {
    // pdf-extract is written around `Identity-H`/`Identity-V` CID fonts and
    // `assert!`s (or panics) on anything else — e.g. Chinese PDFs that set
    // /Encoding to /GBK2K-H. That abort would kill the whole reader, so catch
    // the unwind here and surface a readable error instead. The book is
    // usually a scanned or CJK document whose text layer we cannot decode;
    // the caller shows "no readable content" and the user gets a clean exit
    // rather than a crash. The panic hook is silenced for the duration so the
    // expected condition does not spew a stack trace over a UI the user is
    // trying to read.
    let result = {
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pdf_extract::extract_text(path)
        }));
        std::panic::set_hook(hook);
        result
    };
    let text = match result {
        Ok(Ok(text)) => text,
        Ok(Err(e)) => {
            return Err(LueError::Invalid(format!("PDF extraction failed: {e}")));
        }
        Err(_) => {
            return Err(LueError::Invalid(
                "PDF text extraction aborted: unsupported font encoding \
                 (e.g. a Chinese /GBK2K-H CID font). The document may be a \
                 scan whose text layer cannot be decoded."
                    .into(),
            ));
        }
    };
    Ok(pdf_text_to_docs(&text))
}

fn pdf_text_to_docs(text: &str) -> Vec<String> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");

    // Chapter-title rules first, sampled like build_txt_index (first 500 KiB).
    let sample: String = {
        let bytes = normalized.as_bytes();
        let mut cut = bytes.len().min(500 * 1024);
        while cut < bytes.len() && !normalized.is_char_boundary(cut) {
            cut += 1;
        }
        normalized[..cut].to_string()
    };
    if let Some(rule) = crate::toc::pick_toc_rule_impl(&sample) {
        let title_lines = crate::toc::collect_title_lines_impl(&normalized, rule);
        if title_lines.len() >= 2 {
            let all: Vec<&str> = normalized.split('\n').collect();
            let mut docs: Vec<String> = Vec::with_capacity(title_lines.len() + 1);
            let first = title_lines[0] as usize;
            if first > 0 {
                let preface = all[..first].join("\n");
                if preface.split('\n').any(|l| !l.trim().is_empty()) {
                    docs.push(preface);
                }
            }
            for (i, &start) in title_lines.iter().enumerate() {
                let end = title_lines.get(i + 1).map_or(all.len(), |&n| n as usize);
                docs.push(all[start as usize..end].join("\n"));
            }
            let docs: Vec<String> =
                docs.into_iter().filter(|d| !d.trim().is_empty()).collect();
            if !docs.is_empty() {
                return docs;
            }
        }
    }

    // Fallback: one chapter of cleaned, non-trivial lines.
    let lines: Vec<String> = normalized
        .split('\n')
        .map(|line| clean_visual_text_impl(line.trim()))
        .filter(|line| line.chars().count() > 3)
        .collect();
    single_chapter(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_fixture_epub(path: &Path) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::SimpleFileOptions = Default::default();
        zip.start_file("META-INF/container.xml", options).unwrap();
        zip.write_all(
            br#"<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
        )
        .unwrap();
        zip.start_file("OEBPS/content.opf", options).unwrap();
        zip.write_all(
            br#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0"><metadata/><manifest><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="text/ch2.xhtml" media-type="application/xhtml+xml"/><item id="c3" href="empty.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/><itemref idref="c2"/><itemref idref="c3"/></spine></package>"#,
        )
        .unwrap();
        zip.start_file("OEBPS/ch1.xhtml", options).unwrap();
        zip.write_all(
            "<html><head><title>ignore</title><style>p{}</style></head><body><h2>第一章 风起</h2><p>山里的雾还没散。</p><p>他 &amp; 她一起下山。</p></body></html>".as_bytes(),
        )
        .unwrap();
        zip.start_file("OEBPS/text/ch2.xhtml", options).unwrap();
        zip.write_all(
            "<html><body><h2>第二章 云涌</h2><p>事情开始起变化&#x2026;&#x2026;</p><br/><p>未完待续。</p></body></html>".as_bytes(),
        )
        .unwrap();
        zip.start_file("OEBPS/empty.xhtml", options).unwrap();
        zip.write_all(b"<html><body></body></html>").unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn epub_extracts_one_chapter_per_spine_document() {
        let dir = std::env::temp_dir().join("lue_epub_extract");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("book.epub");
        write_fixture_epub(&path);
        let docs = extract_epub_docs(&path).unwrap();
        // The empty spine document is dropped like the Python extractor does.
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0], "第一章 风起\n山里的雾还没散。\n他 & 她一起下山。");
        assert_eq!(docs[1], "第二章 云涌\n事情开始起变化……\n未完待续。");
    }

    #[test]
    fn html_entities_and_hidden_tags() {
        let lines = html_to_lines(
            "<p>a&nbsp;b&lt;c</p><script>hide()</script><p>可见</p><p>带<sup>注</sup>文</p>",
        );
        assert_eq!(lines, vec!["a\u{a0}b<c", "可见", "带文"]);
    }

    #[test]
    fn stray_lt_stays_literal_text() {
        let lines = html_to_lines("<p>a < b</p>");
        assert_eq!(lines, vec!["a < b"]);
    }

    #[test]
    fn docx_extracts_paragraph_lines() {
        let dir = std::env::temp_dir().join("lue_docx_extract");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("book.docx");
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::SimpleFileOptions = Default::default();
        zip.start_file("word/document.xml", options).unwrap();
        zip.write_all(
            r#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>第一章 风起</w:t></w:r></w:p><w:p><w:r><w:t>短</w:t></w:r></w:p><w:p><w:r><w:t>他 &amp; 她下山了。</w:t></w:r><w:r><w:tab/></w:r></w:p></w:body></w:document>"#.as_bytes(),
        )
        .unwrap();
        zip.finish().unwrap();
        let docs = extract_docx_docs(&path).unwrap();
        // The 1-char paragraph is dropped (>3 filter), entities decoded.
        assert_eq!(docs, vec!["第一章 风起\n他 & 她下山了。"]);
    }

    #[test]
    fn html_file_extraction() {
        let dir = std::env::temp_dir().join("lue_html_extract");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("book.html");
        std::fs::write(
            &path,
            "<html><head><style>p{}</style></head><body><h1>标题</h1><p>段落一。</p><p>段落二。</p></body></html>",
        )
        .unwrap();
        let docs = extract_html_docs(&path).unwrap();
        assert_eq!(docs, vec!["标题\n段落一。\n段落二。"]);
    }

    #[test]
    fn markdown_transforms_structure() {
        let dir = std::env::temp_dir().join("lue_md_extract");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("book.md");
        std::fs::write(
            &path,
            "# 标题\n\n- 项目一\n1. 项目二\n\n> 引用文\n\n```rust\nfn main() {}\n```\n\n    缩进代码\n正文一段。\n",
        )
        .unwrap();
        let docs = extract_markdown_docs(&path).unwrap();
        assert_eq!(docs.len(), 1);
        let lines: Vec<&str> = docs[0].split('\n').collect();
        assert!(lines.contains(&"标题"));
        assert!(lines.contains(&"• 项目一"));
        assert!(lines.contains(&"• 项目二"));
        assert!(lines.iter().any(|l| l.ends_with("引用文")));
        assert!(lines.contains(&"Code (rust):"));
        // The __CODE_BLOCK__ marker is consumed by clean_visual_text into
        // indented code text.
        assert!(lines.contains(&"    fn main() {}"));
        assert!(lines.iter().all(|l| !l.starts_with("__CODE_BLOCK__")));
        assert!(lines.contains(&"正文一段。"));
    }

    #[test]
    fn rtf_scanner_handles_groups_and_escapes() {
        // Direct scanner checks: destination groups vanish, escapes decode.
        assert_eq!(
            rtf_to_text(r"{\rtf1\ansi{\fonttbl{\f0 Times;}}{\*\generator lue}正文\par一段\{花括号\}。"),
            "正文\n一段{花括号}。"
        );
        assert_eq!(rtf_to_text(r"caf\'e9 与\u20013?文中。"), "café 与中文中。");
    }

    #[test]
    fn rtf_file_lines_are_cleaned_and_filtered() {
        let dir = std::env::temp_dir().join("lue_rtf_extract");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("book.rtf");
        std::fs::write(
            &path,
            r"{\rtf1\ansi{\fonttbl{\f0 Times New Roman;}}\par第一章 风起。\par咖啡 café 与中文出现。\par短\par}",
        )
        .unwrap();
        let docs = extract_rtf_docs(&path).unwrap();
        assert_eq!(docs, vec!["第一章 风起。\n咖啡 café 与中文出现。"]);
    }

    #[test]
    fn pdf_text_splits_by_chapter_rules() {
        // Chapter bodies must exceed the 1000-char TOC validity gap, like
        // real books do.
        let filler = "第一章的正文内容重复填充以超过规则有效门槛的一千字符呀。".repeat(40);
        let text = format!(
            "前言的内容足够长以便成为独立章节呀。\n第一章 风起\n{filler}\n第二章 云涌\n事情开始起变化。\n结尾的一些话。\n"
        );
        let docs = pdf_text_to_docs(&text);
        assert_eq!(docs.len(), 3);
        assert!(docs[0].starts_with("前言"));
        assert!(docs[1].starts_with("第一章 风起"));
        assert!(docs[2].starts_with("第二章 云涌"));
    }

    #[test]
    fn pdf_text_without_titles_stays_single_chapter() {
        let docs = pdf_text_to_docs("第一段的话够长了。\n第二段也是一句话内容呀。\n");
        assert_eq!(docs.len(), 1);
        assert!(docs[0].contains("第二段"));
    }

    /// Minimal two-page Helvetica PDF assembled by hand (xref offsets
    /// computed), exercising the real pdf-extract path end to end.
    #[test]
    fn pdf_fixture_extracts_text() {
        let dir = std::env::temp_dir().join("lue_pdf_extract");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("book.pdf");
        std::fs::write(&path, build_fixture_pdf()).unwrap();
        let docs = extract_pdf_docs(&path).unwrap();
        let joined = docs.join("\n");
        assert!(joined.contains("Beginning"), "got: {joined:?}");
        assert!(joined.contains("chapter two body"), "got: {joined:?}");
    }

    /// A Chinese PDF that sets /Encoding /GBK2K-H on a Type0 font used to
    /// crash pdf-extract's `assert!(name == "Identity-H")` and abort the
    /// whole reader. It must now surface as a readable error, not a panic.
    #[test]
    fn pdf_gbk_encoded_cid_font_returns_error_instead_of_panicking() {
        let dir = std::env::temp_dir().join("lue_pdf_gbk_extract");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("gbk.pdf");
        std::fs::write(&path, build_gbk_cid_fixture_pdf()).unwrap();
        let err = extract_pdf_docs(&path).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("PDF") || msg.contains("encoding") || msg.contains("scan"),
            "expected a readable PDF error, got: {msg}"
        );
    }

    /// One-page PDF whose single Type0 font carries /Encoding /GBK2K-H —
    /// exactly what trips pdf-extract's Identity-H assertion.
    fn build_gbk_cid_fixture_pdf() -> Vec<u8> {
        let content = "BT /F1 16 Tf 72 720 Td <D6D0> Tj ET";
        let plain: [(usize, &str); 5] = [
            (1, "<< /Type /Catalog /Pages 2 0 R >>"),
            (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            (
                3,
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
                 /Resources << /Font << /F1 5 0 R >> >> >>",
            ),
            (
                5,
                "<< /Type /Font /Subtype /Type0 /BaseFont /SimHei \
                 /Encoding /GBK2K-H /DescendantFonts [6 0 R] >>",
            ),
            (
                6,
                "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /SimHei \
                 /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 2 >> >>",
            ),
        ];
        let streams: [(usize, &str); 1] = [(4, content)];

        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(b"%PDF-1.4\n");
        let mut offsets = [0usize; 7];
        for (num, body) in &plain {
            offsets[*num] = out.len();
            out.extend_from_slice(format!("{num} 0 obj\n{body}\nendobj\n").as_bytes());
        }
        for (num, stream) in &streams {
            offsets[*num] = out.len();
            out.extend_from_slice(
                format!(
                    "{num} 0 obj\n<< /Length {} >>\nstream\n{stream}\nendstream\nendobj\n",
                    stream.len()
                )
                .as_bytes(),
            );
        }
        let xref = out.len();
        out.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
        for off in &offsets[1..] {
            out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        out
    }

    fn build_fixture_pdf() -> Vec<u8> {
        let content1 = "BT /F1 16 Tf 72 720 Td (Chapter 1 The Beginning) Tj 0 -24 Td \
                        (This is body text of chapter one for the fixture.) Tj ET";
        let content2 = "BT /F1 16 Tf 72 720 Td (Chapter 2 The Middle) Tj 0 -24 Td \
                        (This is body text of chapter two body for the fixture.) Tj ET";
        let plain: [(usize, &str); 5] = [
            (1, "<< /Type /Catalog /Pages 2 0 R >>"),
            (2, "<< /Type /Pages /Kids [3 0 R 6 0 R] /Count 2 >>"),
            (
                3,
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
                 /Resources << /Font << /F1 5 0 R >> >> >>",
            ),
            (
                5,
                "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
            ),
            (
                6,
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 7 0 R \
                 /Resources << /Font << /F1 5 0 R >> >> >>",
            ),
        ];
        let streams: [(usize, &str); 2] = [(4, content1), (7, content2)];

        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(b"%PDF-1.4\n");
        let mut offsets = [0usize; 8];
        for (num, body) in &plain {
            offsets[*num] = out.len();
            out.extend_from_slice(format!("{num} 0 obj\n{body}\nendobj\n").as_bytes());
        }
        for (num, content) in &streams {
            offsets[*num] = out.len();
            out.extend_from_slice(
                format!(
                    "{num} 0 obj\n<< /Length {} >>\nstream\n{content}\nendstream\nendobj\n",
                    content.len()
                )
                .as_bytes(),
            );
        }
        let xref = out.len();
        out.extend_from_slice(b"xref\n0 8\n0000000000 65535 f \n");
        for off in &offsets[1..] {
            out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size 8 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        out
    }
}
