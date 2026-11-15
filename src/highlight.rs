// src/highlight.rs
//! Syntax-Hervorhebung für JSON, XML und YAML.
//!
//! Die Tokenizer arbeiten auf Byte-Bereichen, schneiden aber ausschließlich an
//! UTF-8-Zeichengrenzen, sodass die Bereiche direkt als Textabschnitte
//! verwendet werden können. Nicht hervorgehobene Bereiche (Leerraum usw.)
//! behalten die Standardfarbe.

use crate::Fmt;
use eframe::egui::{text::LayoutJob, Color32, FontId, TextFormat};

/// Art eines hervorgehobenen Abschnitts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Schlüssel (JSON-Objektname, YAML-Key)
    Key,
    /// Zeichenkette / Textwert
    Str,
    /// Zahl
    Num,
    /// Literal wie `true`, `false`, `null`
    Lit,
    /// Klammern, Kommas, Doppelpunkte, Tag-Begrenzer
    Punct,
    /// Kommentar
    Comment,
    /// XML-Tag- oder Attributname
    Tag,
    /// XML-Attributwert
    Attr,
    /// Meta-Angaben (Dokumentmarker, Anker, Alias, Tag, Block-Skalar)
    Meta,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

/// Zerlegt den Text passend zum Format in hervorgehobene Abschnitte.
///
/// Die Abschnitte sind sortiert, überlappen nicht und enthalten keine leeren
/// Bereiche.
pub fn tokens(fmt: Fmt, text: &str) -> Vec<Token> {
    match fmt {
        Fmt::Json => tokens_json(text),
        Fmt::Xml => tokens_xml(text),
        Fmt::Yaml => tokens_yaml(text),
    }
}

/// Erzeugt den fertigen Layout-Job für die Darstellung im Editor.
pub fn layout_job(fmt: Fmt, text: &str, font: FontId, dark: bool, wrap_width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    job.break_on_newline = true;

    let base = TextFormat {
        font_id: font.clone(),
        color: if dark {
            Color32::from_rgb(0xD4, 0xD4, 0xD4)
        } else {
            Color32::from_rgb(0x1F, 0x23, 0x28)
        },
        ..Default::default()
    };

    let mut pos = 0;
    for t in tokens(fmt, text) {
        let start = t.start.max(pos);
        let end = t.end.min(text.len());
        if start >= end {
            continue;
        }
        if start > pos {
            job.append(&text[pos..start], 0.0, base.clone());
        }
        let format = TextFormat {
            font_id: font.clone(),
            color: color(t.kind, dark),
            ..Default::default()
        };
        job.append(&text[start..end], 0.0, format);
        pos = end;
    }
    if pos < text.len() {
        job.append(&text[pos..], 0.0, base);
    }
    job
}

fn color(kind: Kind, dark: bool) -> Color32 {
    let (r, g, b) = if dark {
        match kind {
            Kind::Key => (0x8A, 0xB4, 0xF8),
            Kind::Str => (0xA5, 0xD6, 0xA7),
            Kind::Num => (0xE8, 0xA8, 0x5C),
            Kind::Lit => (0xC7, 0x92, 0xEA),
            Kind::Punct => (0xA8, 0xB4, 0xBE),
            Kind::Comment => (0x7E, 0xA6, 0x6E),
            Kind::Tag => (0x4E, 0xC9, 0xB0),
            Kind::Attr => (0xDC, 0xDC, 0xAA),
            Kind::Meta => (0xC5, 0x86, 0xC0),
        }
    } else {
        match kind {
            Kind::Key => (0x00, 0x5C, 0xC5),
            Kind::Str => (0x22, 0x86, 0x3A),
            Kind::Num => (0xB3, 0x5C, 0x00),
            Kind::Lit => (0x6F, 0x42, 0xC1),
            Kind::Punct => (0x44, 0x4C, 0x56),
            Kind::Comment => (0x6A, 0x73, 0x7D),
            Kind::Tag => (0x0B, 0x72, 0x85),
            Kind::Attr => (0x7A, 0x5C, 0x00),
            Kind::Meta => (0x82, 0x50, 0xDF),
        }
    };
    Color32::from_rgb(r, g, b)
}

// ---------------------------------------------------------------------------
// Hilfsfunktionen
// ---------------------------------------------------------------------------

fn push(out: &mut Vec<Token>, start: usize, end: usize, kind: Kind) {
    if end > start {
        out.push(Token { start, end, kind });
    }
}

/// Länge des UTF-8-Zeichens, das mit diesem Byte beginnt.
fn ch_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else if b >> 3 == 0b11110 {
        4
    } else {
        1
    }
}

fn find_sub(s: &str, from: usize, pat: &str) -> Option<usize> {
    s.get(from..)?.find(pat).map(|p| p + from)
}

fn find_byte(s: &str, from: usize, byte: u8) -> Option<usize> {
    let b = s.as_bytes();
    if from >= b.len() {
        return None;
    }
    b[from..].iter().position(|c| *c == byte).map(|p| p + from)
}

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

fn tokens_json(s: &str) -> Vec<Token> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                let start = i;
                i += 1;
                while i < b.len() {
                    match b[i] {
                        b'\\' => i += 2,
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => i += ch_len(b[i]),
                    }
                }
                let end = i.min(b.len());
                // Ein Schlüssel wird von einem Doppelpunkt gefolgt.
                let mut j = end;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                let kind = if j < b.len() && b[j] == b':' {
                    Kind::Key
                } else {
                    Kind::Str
                };
                push(&mut out, start, end, kind);
            }
            b'0'..=b'9' | b'-' | b'+' | b'.' => {
                let start = i;
                i += 1;
                while i < b.len()
                    && (b[i].is_ascii_digit() || matches!(b[i], b'.' | b'e' | b'E' | b'+' | b'-'))
                {
                    i += 1;
                }
                push(&mut out, start, i, Kind::Num);
            }
            b't' | b'f' | b'n' => {
                let start = i;
                while i < b.len() && b[i].is_ascii_alphabetic() {
                    i += 1;
                }
                if matches!(&s[start..i], "true" | "false" | "null") {
                    push(&mut out, start, i, Kind::Lit);
                }
            }
            b'{' | b'}' | b'[' | b']' | b':' | b',' => {
                push(&mut out, i, i + 1, Kind::Punct);
                i += 1;
            }
            c => i += ch_len(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// XML
// ---------------------------------------------------------------------------

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':' | b'.') || b >= 0x80
}

fn tokens_xml(s: &str) -> Vec<Token> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += ch_len(b[i]);
            continue;
        }

        if s[i..].starts_with("<!--") {
            let end = find_sub(s, i + 4, "-->").map_or(b.len(), |p| p + 3);
            push(&mut out, i, end, Kind::Comment);
            i = end;
        } else if s[i..].starts_with("<![CDATA[") {
            let end = find_sub(s, i + 9, "]]>").map_or(b.len(), |p| p + 3);
            push(&mut out, i, end, Kind::Str);
            i = end;
        } else if s[i..].starts_with("<?") || s[i..].starts_with("<!") {
            let end = find_byte(s, i + 2, b'>').map_or(b.len(), |p| p + 1);
            push(&mut out, i, end, Kind::Meta);
            i = end;
        } else if s[i..].starts_with("</") {
            let mut j = i + 2;
            let name_start = j;
            while j < b.len() && is_name_byte(b[j]) {
                j += ch_len(b[j]);
            }
            push(&mut out, i, i + 2, Kind::Punct);
            push(&mut out, name_start, j, Kind::Tag);
            if let Some(p) = find_byte(s, j, b'>') {
                push(&mut out, j, p + 1, Kind::Punct);
                i = p + 1;
            } else {
                i = j;
            }
        } else {
            // Öffnender Tag mit optionalen Attributen.
            let mut j = i + 1;
            let name_start = j;
            while j < b.len() && is_name_byte(b[j]) {
                j += ch_len(b[j]);
            }
            push(&mut out, i, i + 1, Kind::Punct);
            push(&mut out, name_start, j, Kind::Tag);

            let mut closed = false;
            while j < b.len() {
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j >= b.len() {
                    break;
                }
                match b[j] {
                    b'>' => {
                        push(&mut out, j, j + 1, Kind::Punct);
                        j += 1;
                        closed = true;
                        break;
                    }
                    b'/' => {
                        let start = j;
                        j += 1;
                        if j < b.len() && b[j] == b'>' {
                            j += 1;
                            closed = true;
                            push(&mut out, start, j, Kind::Punct);
                            break;
                        }
                        push(&mut out, start, j, Kind::Punct);
                    }
                    b'=' => {
                        push(&mut out, j, j + 1, Kind::Punct);
                        j += 1;
                    }
                    b'"' | b'\'' => {
                        let quote = b[j];
                        let start = j;
                        j += 1;
                        while j < b.len() && b[j] != quote {
                            j += ch_len(b[j]);
                        }
                        if j < b.len() {
                            j += 1;
                        }
                        push(&mut out, start, j, Kind::Attr);
                    }
                    _ => {
                        let start = j;
                        while j < b.len()
                            && !b[j].is_ascii_whitespace()
                            && !matches!(b[j], b'=' | b'>' | b'/')
                        {
                            j += ch_len(b[j]);
                        }
                        push(&mut out, start, j, Kind::Attr);
                    }
                }
            }
            i = if closed { j } else { j.max(i + 1) };
        }
    }
    out
}

// ---------------------------------------------------------------------------
// YAML
// ---------------------------------------------------------------------------

fn tokens_yaml(s: &str) -> Vec<Token> {
    let mut out = Vec::new();
    // Einrückung des Block-Skalars, dessen Folgezeilen unverändert bleiben.
    let mut block_indent: Option<usize> = None;
    let mut offset = 0;
    for raw in s.split_inclusive('\n') {
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        let line = line.strip_suffix('\r').unwrap_or(line);
        yaml_line(line, offset, &mut out, &mut block_indent);
        offset += raw.len();
    }
    out
}

fn yaml_line(line: &str, base: usize, out: &mut Vec<Token>, block: &mut Option<usize>) {
    let b = line.as_bytes();
    let indent = b.iter().take_while(|c| **c == b' ').count();
    let rest = &line[indent..];
    if rest.trim().is_empty() {
        return;
    }

    // Inhalt eines Block-Skalars (| oder >) bleibt unangetastet.
    if let Some(bi) = *block {
        if indent > bi {
            push(out, base + indent, base + line.len(), Kind::Str);
            return;
        }
        *block = None;
    }

    let marker_boundary = |p: usize| p >= b.len() || b[p].is_ascii_whitespace();
    if (rest.starts_with("---") || rest.starts_with("...")) && marker_boundary(indent + 3) {
        push(out, base + indent, base + line.len(), Kind::Meta);
        return;
    }
    if rest.starts_with('%') {
        push(out, base + indent, base + line.len(), Kind::Meta);
        return;
    }
    if rest.starts_with('#') {
        push(out, base + indent, base + line.len(), Kind::Comment);
        return;
    }

    let mut p = indent;
    while p < b.len() {
        // Leerraum überspringen
        while p < b.len() && b[p] == b' ' {
            p += 1;
        }
        if p >= b.len() {
            break;
        }

        match b[p] {
            b'#' => {
                push(out, base + p, base + b.len(), Kind::Comment);
                break;
            }
            b'-' if p + 1 == b.len() || b[p + 1] == b' ' => {
                push(out, base + p, base + p + 1, Kind::Punct);
                p += 1;
            }
            b'[' | b']' | b'{' | b'}' | b',' => {
                push(out, base + p, base + p + 1, Kind::Punct);
                p += 1;
            }
            b'?' | b':' if p + 1 == b.len() || b[p + 1] == b' ' => {
                push(out, base + p, base + p + 1, Kind::Punct);
                p += 1;
            }
            b'&' | b'*' | b'!' => {
                let start = p;
                p += 1;
                while p < b.len()
                    && !matches!(b[p], b' ' | b'\t' | b',' | b'[' | b']' | b'{' | b'}' | b'#')
                {
                    p += ch_len(b[p]);
                }
                push(out, base + start, base + p, Kind::Meta);
            }
            b'"' | b'\'' => {
                let end = scan_quoted(line, p);
                let kind = if yaml_colon_follows(b, end) {
                    Kind::Key
                } else {
                    Kind::Str
                };
                push(out, base + p, base + end, kind);
                p = end;
            }
            b'|' | b'>' => {
                let start = p;
                p += 1;
                while p < b.len() && matches!(b[p], b'+' | b'-' | b'0'..=b'9') {
                    p += 1;
                }
                push(out, base + start, base + p, Kind::Meta);
                *block = Some(indent);
                break;
            }
            _ => {
                // Einfacher Skalar: entweder Schlüssel (bis ':') oder Wert.
                let start = p;
                let mut key_end = None;
                while p < b.len() {
                    let c = b[p];
                    if c == b':' && (p + 1 == b.len() || b[p + 1] == b' ') {
                        key_end = Some(p);
                        break;
                    }
                    if c == b'#' && p > start && b[p - 1] == b' ' {
                        break;
                    }
                    if matches!(c, b',' | b']' | b'}' | b'[' | b'{') {
                        break;
                    }
                    p += ch_len(c);
                }

                if let Some(kc) = key_end {
                    let mut end = kc;
                    while end > start && b[end - 1] == b' ' {
                        end -= 1;
                    }
                    push(out, base + start, base + end, Kind::Key);
                    p = kc;
                } else {
                    let mut end = p;
                    while end > start && b[end - 1] == b' ' {
                        end -= 1;
                    }
                    push(
                        out,
                        base + start,
                        base + end,
                        yaml_value_kind(&line[start..end]),
                    );
                    if p < b.len() && matches!(b[p], b',' | b']' | b'}' | b'#') {
                        continue;
                    }
                    break;
                }
            }
        }
    }
}

/// Liefert die Position direkt hinter einer (einfachen oder doppelten)
/// Zeichenkette. `start` zeigt auf das öffnende Anführungszeichen.
fn scan_quoted(s: &str, start: usize) -> usize {
    let b = s.as_bytes();
    let quote = b[start];
    let mut i = start + 1;
    while i < b.len() {
        match b[i] {
            b'\\' if quote == b'"' => i += 2,
            c if c == quote => {
                if quote == b'\'' && i + 1 < b.len() && b[i + 1] == b'\'' {
                    i += 2; // '' ist ein escaped Apostroph
                } else {
                    return i + 1;
                }
            }
            c => i += ch_len(c),
        }
    }
    b.len()
}

/// Prüft, ob nach dem Ende eines Schlüssels ein Doppelpunkt folgt.
fn yaml_colon_follows(b: &[u8], from: usize) -> bool {
    let mut i = from;
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    i < b.len() && b[i] == b':' && (i + 1 == b.len() || b[i + 1] == b' ')
}

fn yaml_value_kind(v: &str) -> Kind {
    let t = v.trim();
    if t.is_empty() {
        return Kind::Str;
    }
    match t.to_ascii_lowercase().as_str() {
        "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~" => Kind::Lit,
        _ if is_number(t) => Kind::Num,
        _ => Kind::Str,
    }
}

fn is_number(t: &str) -> bool {
    let t = t.strip_prefix(['-', '+']).unwrap_or(t);
    if t.is_empty() {
        return false;
    }
    if t.starts_with("0x") || t.starts_with("0o") {
        return t[2..].chars().all(|c| c.is_ascii_hexdigit());
    }
    t.parse::<i64>().is_ok() || t.parse::<f64>().is_ok()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(fmt: Fmt, text: &str) -> Vec<(Kind, &str)> {
        tokens(fmt, text)
            .into_iter()
            .map(|t| (t.kind, &text[t.start..t.end]))
            .collect()
    }

    fn assert_invariants(fmt: Fmt, text: &str) {
        let mut pos = 0;
        for t in tokens(fmt, text) {
            assert!(t.start >= pos, "überlappende Tokens: {t:?}");
            assert!(t.end > t.start, "leeres Token: {t:?}");
            assert!(
                text.is_char_boundary(t.start),
                "Start auf Zeichengrenze: {t:?}"
            );
            assert!(
                text.is_char_boundary(t.end),
                "Ende auf Zeichengrenze: {t:?}"
            );
            pos = t.end;
        }
        let job = layout_job(fmt, text, FontId::monospace(12.0), true, 100.0);
        assert_eq!(
            job.text, text,
            "Layout-Job muss den Text vollständig enthält"
        );
    }

    #[test]
    fn json_schluessel_werte_und_literale() {
        let text = r#"{"a": 1, "b": [true, null], "c": "text"}"#;
        let k = kinds(Fmt::Json, text);
        assert!(k.contains(&(Kind::Key, "\"a\"")));
        assert!(k.contains(&(Kind::Key, "\"b\"")));
        assert!(k.contains(&(Kind::Str, "\"text\"")));
        assert!(k.contains(&(Kind::Num, "1")));
        assert!(k.contains(&(Kind::Lit, "true")));
        assert!(k.contains(&(Kind::Lit, "null")));
        assert_invariants(Fmt::Json, text);
    }

    #[test]
    fn json_unicode_und_escapes() {
        let text = r#"{"grüße": "wörld \"x\"", "z": -1.5e3}"#;
        let k = kinds(Fmt::Json, text);
        assert!(k.contains(&(Kind::Key, "\"grüße\"")));
        assert!(k.contains(&(Kind::Str, r#""wörld \"x\"""#)));
        assert!(k.contains(&(Kind::Num, "-1.5e3")));
        assert_invariants(Fmt::Json, text);
    }

    #[test]
    fn xml_tags_attribute_und_kommentare() {
        let text = "<a x=\"1\">text<!-- c --></a>";
        let k = kinds(Fmt::Xml, text);
        assert!(k.contains(&(Kind::Tag, "a")));
        assert!(k.contains(&(Kind::Attr, "x")));
        assert!(k.contains(&(Kind::Attr, "\"1\"")));
        assert!(k.contains(&(Kind::Comment, "<!-- c -->")));
        assert_invariants(Fmt::Xml, text);
    }

    #[test]
    fn xml_deklaration_und_selbstschliessend() {
        let text = "<?xml version=\"1.0\"?><b/>";
        let k = kinds(Fmt::Xml, text);
        assert!(k.contains(&(Kind::Meta, "<?xml version=\"1.0\"?>")));
        assert!(k.contains(&(Kind::Tag, "b")));
        assert_invariants(Fmt::Xml, text);
    }

    #[test]
    fn yaml_schluessel_werte_und_kommentare() {
        let text = "# oben\nname: Parsix\nzahl: 42\naktiv: true\n- liste\n";
        let k = kinds(Fmt::Yaml, text);
        assert!(k.contains(&(Kind::Comment, "# oben")));
        assert!(k.contains(&(Kind::Key, "name")));
        assert!(k.contains(&(Kind::Str, "Parsix")));
        assert!(k.contains(&(Kind::Num, "42")));
        assert!(k.contains(&(Kind::Lit, "true")));
        assert!(k.contains(&(Kind::Punct, "-")));
        assert_invariants(Fmt::Yaml, text);
    }

    #[test]
    fn yaml_block_skalar_behaelt_rautezeichen() {
        let text = "text: |\n  zeile # kein Kommentar\n  mehr\nrest: 1\n";
        let k = kinds(Fmt::Yaml, text);
        assert!(k.contains(&(Kind::Str, "zeile # kein Kommentar")));
        assert!(k.contains(&(Kind::Str, "mehr")));
        assert!(k.contains(&(Kind::Key, "rest")));
        assert!(!k
            .iter()
            .any(|(kind, s)| *kind == Kind::Comment && s.contains('#')));
        assert_invariants(Fmt::Yaml, text);
    }

    #[test]
    fn yaml_anker_alias_und_dokumentmarker() {
        let text = "---\nbasis: &b {x: 1}\nkopie: *b\n";
        let k = kinds(Fmt::Yaml, text);
        assert!(k.contains(&(Kind::Meta, "---")));
        assert!(k.contains(&(Kind::Meta, "&b")));
        assert!(k.contains(&(Kind::Meta, "*b")));
        assert!(k.contains(&(Kind::Key, "basis")));
        assert_invariants(Fmt::Yaml, text);
    }

    #[test]
    fn yaml_zeichenketten_mit_raute() {
        let text = "url: \"http://x/#y\" # Kommentar\n";
        let k = kinds(Fmt::Yaml, text);
        assert!(k.contains(&(Kind::Key, "url")));
        assert!(k.contains(&(Kind::Str, "\"http://x/#y\"")));
        assert!(k.contains(&(Kind::Comment, "# Kommentar")));
        assert_invariants(Fmt::Yaml, text);
    }

    #[test]
    fn alle_formate_markieren_immer_den_ganzen_text() {
        for fmt in [Fmt::Json, Fmt::Xml, Fmt::Yaml] {
            for text in [
                "",
                "  \n\n",
                "äöü",
                "{\"ä\": [1, 2]}",
                "<tag attr='ä'/>, Text",
                "- ä: \"ß\"\n  - &x *y\n",
            ] {
                assert_invariants(fmt, text);
            }
        }
    }
}
