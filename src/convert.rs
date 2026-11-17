// src/convert.rs
//! Formatübergreifende Konvertierung zwischen JSON, XML und YAML.
//!
//! Alle drei Formate werden über das gemeinsame, formatunabhängige Datenmodell
//! [`Value`] ineinander überführt: Quelle → [`Value`] → Ziel. Dadurch gibt es
//! für jede der sechs Konvertierungsrichtungen genau eine Implementierung und
//! die vorhandenen Parser (serde_json, serde_yaml_ng, quick-xml) bleiben die
//! einzige Stelle, an der ein Format gelesen oder geschrieben wird.
//!
//! ## Abbildung von XML
//!
//! XML unterscheidet sich von JSON und YAML durch Attribute, Namensräume und
//! Textknoten. Sie werden wie folgt abgebildet:
//!
//! - Das Wurzelelement wird zum einzigen Schlüssel auf oberster Ebene,
//!   z. B. `<root>…</root>` → `{"root": …}`.
//! - Attribute heißen im Modell `@name`, z. B. `id="1"` → `"@id": "1"`.
//! - Der Textinhalt eines Elements liegt unter `#text`.
//! - Mehrfach vorkommende Kinderelemente werden zu einer Liste.
//! - Namensraumpräfixe bleiben im Elementnamen erhalten (`ns:tag`);
//!   `xmlns`-Deklarationen sind gewöhnliche Attribute (`@xmlns`, `@xmlns:ns`).
//! - Kommentare, Verarbeitungsanweisungen und die XML-Deklaration werden
//!   nicht übernommen.
//!
//! Da ein Element gemischten Inhalt haben kann (Text und Kinderelemente
//! nebeneinander), geht bei `#text` die Reihenfolge gegenüber den
//! Kinderelementen verloren. Für Datenformate ist das unkritisch.
//!
//! YAML-Anker und Aliase werden beim Einlesen aufgelöst; die Zielformate
//! kennen kein Äquivalent dazu. Komplexe YAML-Schlüssel (Folgen oder
//! Abbildungen als Schlüssel) sind im Modell nicht darstellbar und führen zu
//! einer Fehlermeldung.

use crate::{err, pos_at_byte, strip_positions, FormatOptions, Fmt, FmtErr, Pos};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::Serialize;

/// Formatunabhängiges Datenmodell für die Konvertierung.
///
/// Zahlen werden getrennt gehalten, damit beim Runden-trip keine Präzision
/// verloren geht. Abbildungen behalten ihre Dokumentreihenfolge.
#[derive(Clone, PartialEq, Debug)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Str(String),
    Seq(Vec<Value>),
    /// Schlüssel in Dokumentreihenfolge.
    Map(Vec<(String, Value)>),
}

impl Serialize for Value {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Value::Null => s.serialize_unit(),
            Value::Bool(b) => s.serialize_bool(*b),
            Value::Int(i) => s.serialize_i64(*i),
            Value::UInt(u) => s.serialize_u64(*u),
            Value::Float(f) => s.serialize_f64(*f),
            Value::Str(t) => s.serialize_str(t),
            Value::Seq(items) => {
                let mut seq = s.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Value::Map(entries) => {
                let mut map = s.serialize_map(Some(entries.len()))?;
                for (k, v) in entries {
                    map.serialize_key(k)?;
                    map.serialize_value(v)?;
                }
                map.end()
            }
        }
    }
}

/// Liest `src` als Format `fmt` in das Datenmodell ein.
pub fn parse(fmt: Fmt, src: &str) -> Result<Value, FmtErr> {
    match fmt {
        Fmt::Json => json_to_value(src),
        Fmt::Xml => xml_to_value(src),
        Fmt::Yaml => yaml_to_value(src),
    }
}

/// Schreibt `value` als Format `fmt` unter den Optionen `opts`.
pub fn serialize(fmt: Fmt, value: &Value, opts: &FormatOptions) -> Result<String, FmtErr> {
    match fmt {
        Fmt::Json => value_to_json(value, opts),
        Fmt::Xml => value_to_xml(value, opts),
        Fmt::Yaml => value_to_yaml(value, opts),
    }
}

// --- Quelldokument → Wert ---

fn json_to_value(src: &str) -> Result<Value, FmtErr> {
    let v: serde_json::Value = serde_json::from_str(src).map_err(|e| {
        let pos = (e.line() > 0).then(|| Pos::new(e.line(), e.column()));
        err(strip_positions(&e.to_string()), pos)
    })?;
    Ok(from_json(&v))
}

fn from_json(v: &serde_json::Value) -> Value {
    use serde_json::Value as JValue;
    match v {
        JValue::Null => Value::Null,
        JValue::Bool(b) => Value::Bool(*b),
        JValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(u) = n.as_u64() {
                Value::UInt(u)
            } else {
                Value::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        JValue::String(s) => Value::Str(s.clone()),
        JValue::Array(items) => Value::Seq(items.iter().map(from_json).collect()),
        JValue::Object(map) => Value::Map(
            map.iter()
                .map(|(k, v)| (k.clone(), from_json(v)))
                .collect(),
        ),
    }
}

fn yaml_to_value(src: &str) -> Result<Value, FmtErr> {
    let v: serde_yaml_ng::Value = serde_yaml_ng::from_str(src).map_err(|e| {
        let pos = e.location().map(|l| Pos::new(l.line(), l.column()));
        err(strip_positions(&e.to_string()), pos)
    })?;
    from_yaml(&v)
}

fn from_yaml(v: &serde_yaml_ng::Value) -> Result<Value, FmtErr> {
    use serde_yaml_ng::Value as YValue;
    match v {
        YValue::Null => Ok(Value::Null),
        YValue::Bool(b) => Ok(Value::Bool(*b)),
        YValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Int(i))
            } else if let Some(u) = n.as_u64() {
                Ok(Value::UInt(u))
            } else {
                Ok(Value::Float(n.as_f64().unwrap_or(0.0)))
            }
        }
        YValue::String(s) => Ok(Value::Str(s.clone())),
        YValue::Sequence(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(from_yaml(item)?);
            }
            Ok(Value::Seq(out))
        }
        YValue::Mapping(map) => {
            let mut out = Vec::with_capacity(map.len());
            for (k, v) in map.iter() {
                out.push((map_key(k)?, from_yaml(v)?));
            }
            Ok(Value::Map(out))
        }
        YValue::Tagged(t) => from_yaml(&t.value),
    }
}

/// Wandelt einen YAML-Schlüssel in einen Modell-Schlüssel um.
fn map_key(k: &serde_yaml_ng::Value) -> Result<String, FmtErr> {
    match k {
        serde_yaml_ng::Value::String(s) => Ok(s.clone()),
        serde_yaml_ng::Value::Bool(b) => Ok(b.to_string()),
        serde_yaml_ng::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i.to_string())
            } else if let Some(u) = n.as_u64() {
                Ok(u.to_string())
            } else {
                Ok(n.as_f64().unwrap_or(0.0).to_string())
            }
        }
        serde_yaml_ng::Value::Null => Ok("null".into()),
        _ => Err(err(
            "Komplexe YAML-Schlüssel (Folgen oder Abbildungen) sind nicht darstellbar",
            None,
        )),
    }
}

/// Ein Element beim Einlesen von XML.
struct XmlNode {
    name: String,
    attrs: Vec<(String, String)>,
    /// Zusammengefasster direkter Textinhalt.
    text: String,
    children: Vec<XmlNode>,
}

fn xml_to_value(src: &str) -> Result<Value, FmtErr> {
    let mut r = Reader::from_str(src);
    // XML-Whitespace kann in Textknoten bedeutungstragend sein (z. B.
    // `<name> Ada </name>`). Beim Einlesen nicht trimmen, damit die
    // Konvertierung führende und nachgestellte Leerzeichen erhält.
    r.config_mut().trim_text(false);
    let mut stack: Vec<XmlNode> = Vec::new();
    let mut roots: Vec<XmlNode> = Vec::new();
    loop {
        match r.read_event() {
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => stack.push(element(&e)?),
            Ok(Event::Empty(e)) => {
                let node = element(&e)?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                } else {
                    roots.push(node);
                }
            }
            Ok(Event::End(e)) => {
                let name = qname_str(&e.name())?;
                let node = stack
                    .pop()
                    .ok_or_else(|| err("Schließendes Tag ohne öffnendes Tag", None))?;
                if node.name != name {
                    return Err(err(
                        format!("Schließendes Tag „{name}“ passt nicht zu „{}“", node.name),
                        None,
                    ));
                }
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                } else {
                    roots.push(node);
                }
            }
            Ok(Event::Text(e)) => {
                let text = e.unescape().map_err(|e| err(&e, None))?;
                if let Some(parent) = stack.last_mut() {
                    parent.text.push_str(&text);
                } else if !text.trim().is_empty() {
                    return Err(err("Text außerhalb des Wurzelelements", None));
                }
            }
            // CDATA ist bereits Rohtext und wird nicht entschlüsselt.
            Ok(Event::CData(e)) => {
                let text = std::str::from_utf8(&e)
                    .map_err(|_| err("CDATA-Inhalt ist kein gültiger UTF-8-Text", None))?;
                if let Some(parent) = stack.last_mut() {
                    parent.text.push_str(text);
                }
            }
            Ok(_) => {}
            Err(e) => {
                let pos = pos_at_byte(src, r.error_position() as usize);
                return Err(err(&e, Some(pos)));
            }
        }
    }
    if roots.len() != 1 {
        return Err(err(
            "XML-Dokument muss genau ein Wurzelelement haben",
            None,
        ));
    }
    let root = roots.into_iter().next().expect("Genau ein Wurzelelement");
    Ok(Value::Map(vec![(
        root.name.clone(),
        node_to_value(&root),
    )]))
}

/// Liest Name und Attribute eines Start-Tags.
fn element(e: &BytesStart) -> Result<XmlNode, FmtErr> {
    let name = qname_str(&e.name())?;
    let mut attrs = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(|e| err(&e, None))?;
        let key = qname_str(&a.key)?;
        let value = a.unescape_value().map_err(|e| err(&e, None))?;
        attrs.push((key, value.into_owned()));
    }
    Ok(XmlNode {
        name,
        attrs,
        text: String::new(),
        children: Vec::new(),
    })
}

/// Wandelt ein `QName` in einen String um.
fn qname_str(q: &quick_xml::name::QName) -> Result<String, FmtErr> {    std::str::from_utf8(q.as_ref())
        .map(str::to_string)
        .map_err(|_| err("Element- oder Attributname ist kein gültiger UTF-8-Text", None))
}

/// Wandelt einen XML-Elementbaum in das Datenmodell um.
fn node_to_value(node: &XmlNode) -> Value {
    let mut entries: Vec<(String, Value)> = Vec::new();
    for (k, v) in &node.attrs {
        entries.push((format!("@{k}"), Value::Str(v.clone())));
    }
    // Whitespace-only text between child elements is normally formatting
    // indentation. Keep it for leaf elements, where it is the element's
    // actual text value, but omit it from element-only content.
    if !node.text.is_empty() && (!node.text.trim().is_empty() || node.children.is_empty()) {
        entries.push(("#text".into(), Value::Str(node.text.clone())));
    }
    // Gleichnamige Kinderelemente werden in Dokumentreihenfolge gruppiert.
    let mut groups: Vec<(String, Vec<Value>)> = Vec::new();
    for child in &node.children {
        let value = node_to_value(child);
        if let Some(g) = groups.iter_mut().find(|g| g.0 == child.name) {
            g.1.push(value);
        } else {
            groups.push((child.name.clone(), vec![value]));
        }
    }
    for (name, values) in groups {
        if values.len() == 1 {
            entries.push((name, values.into_iter().next().expect("Gruppe ist nicht leer")));
        } else {
            entries.push((name, Value::Seq(values)));
        }
    }
    Value::Map(entries)
}

// --- Wert → Zieldokument ---

fn value_to_json(v: &Value, opts: &FormatOptions) -> Result<String, FmtErr> {
    let jv: serde_json::Value = serde_json::to_value(v).map_err(|e| err(e, None))?;
    let mut out = String::new();
    if opts.minify {
        crate::json_compact(&jv, opts.sort_json_keys, &mut out);
    } else {
        crate::json_pretty(&jv, opts, 0, &mut out);
    }
    // Einheitliche Formatierungsregel: genau ein abschließender Umbruch.
    out.push('\n');
    Ok(out)
}

fn value_to_yaml(v: &Value, opts: &FormatOptions) -> Result<String, FmtErr> {
    let yv: serde_yaml_ng::Value = serde_yaml_ng::to_value(v).map_err(|e| err(e, None))?;
    let s = serde_yaml_ng::to_string(&yv).map_err(|e| err(e, None))?;
    let mut out = crate::format_yaml_source(&s, opts);
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

fn value_to_xml(v: &Value, opts: &FormatOptions) -> Result<String, FmtErr> {
    let mut w = if opts.minify {
        Writer::new(Vec::new())
    } else {
        let (ch, n) = opts.indent.xml();
        Writer::new_with_indent(Vec::new(), ch, n)
    };
    // Eine einzige Abbildung mit einem Schlüssel liefert den Wurzelnamen;
    // alles andere wird in ein Wurzelelement «root» eingeschlossen.
    let (name, content) = match v {
        Value::Map(entries) if entries.len() == 1 => (&entries[0].0[..], &entries[0].1),
        other => ("root", other),
    };
    write_element(&mut w, name, content)?;
    let mut text = String::from_utf8(w.into_inner()).map_err(|e| err(e, None))?;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    Ok(text)
}

/// Schreibt ein Element mit seinem Inhalt in den XML-Writer.
fn write_element(w: &mut Writer<Vec<u8>>, name: &str, content: &Value) -> Result<(), FmtErr> {
    if !is_xml_name(name) {
        return Err(err(format!("„{name}“ ist kein zulässiger XML-Name"), None));
    }
    match content {
        Value::Map(entries) => {
            let mut start = BytesStart::new(name);
            let mut text: Option<String> = None;
            let mut children: Vec<(&str, &Value)> = Vec::new();
            for (k, v) in entries {
                if let Some(a) = k.strip_prefix('@') {
                    if !is_xml_name(a) {
                        return Err(err(
                            format!("„{a}“ ist kein zulässiger XML-Attributname"),
                            None,
                        ));
                    }
                    let value = scalar_text(v)?;
                    start.push_attribute((a, value.as_str()));
                } else if k == "#text" {
                    text = Some(scalar_text(v)?);
                } else {
                    children.push((k.as_str(), v));
                }
            }
            let has_children = !children.is_empty();
            let has_text = text.as_deref().is_some_and(|t| !t.is_empty());
            if !has_children && !has_text {
                w.write_event(Event::Empty(start)).map_err(|e| err(e, None))?;
                return Ok(());
            }
            w.write_event(Event::Start(start)).map_err(|e| err(e, None))?;
            if has_text {
                w.write_event(Event::Text(BytesText::new(text.as_deref().unwrap())))
                    .map_err(|e| err(e, None))?;
            }
            for (k, v) in children {
                match v {
                    Value::Seq(items) => {
                        for item in items {
                            write_element(w, k, item)?;
                        }
                    }
                    other => write_element(w, k, other)?,
                }
            }
            w.write_event(Event::End(BytesEnd::new(name))).map_err(|e| err(e, None))?;
        }
        Value::Seq(items) => {
            // Eine Sequenz als Elementinhalt wird zu wiederholten
            // Kind-Elementen mit dem Namen «item».
            w.write_event(Event::Start(BytesStart::new(name)))
                .map_err(|e| err(e, None))?;
            for item in items {
                write_element(w, "item", item)?;
            }
            w.write_event(Event::End(BytesEnd::new(name)))
                .map_err(|e| err(e, None))?;
        }
        scalar => {
            let text = scalar_text(scalar)?;
            if text.is_empty() {
                w.write_event(Event::Empty(BytesStart::new(name)))
                    .map_err(|e| err(e, None))?;
                return Ok(());
            }
            w.write_event(Event::Start(BytesStart::new(name)))
                .map_err(|e| err(e, None))?;
            w.write_event(Event::Text(BytesText::new(&text)))
                .map_err(|e| err(e, None))?;
            w.write_event(Event::End(BytesEnd::new(name)))
                .map_err(|e| err(e, None))?;
        }
    }
    Ok(())
}

/// Liefert die Textdarstellung eines einfachen Werts.
fn scalar_text(v: &Value) -> Result<String, FmtErr> {
    match v {
        Value::Null => Ok(String::new()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Int(i) => Ok(i.to_string()),
        Value::UInt(u) => Ok(u.to_string()),
        Value::Float(f) => Ok(f.to_string()),
        Value::Str(s) => Ok(s.clone()),
        _ => Err(err(
            "Dieser Wert ist keine einfache Angabe und kann nicht als Text geschrieben werden",
            None,
        )),
    }
}

/// Prüft, ob `name` als XML-Element- oder Attributname zulässig ist.
///
/// Die Prüfung folgt der XML-Namensregel in der üblichen Vereinfachung:
/// ein Buchstabe, Unterstrich oder Doppelpunkt am Anfang, danach zusätzlich
/// Ziffern, Bindestrich, Punkt und Mittelpunkt. Namen, die mit «xml»
/// (groß- und kleinschreibungsunabhängig) beginnen, sind reserviert – mit
/// Ausnahme der Namensraum-Deklarationen (`xmlns`, `xmlns:…`) und der
/// vordefinierten `xml:`-Präfixe.
fn is_xml_name(name: &str) -> bool {
    let mut chars = name.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !(first.is_alphabetic() || first == '_' || first == ':') {
        return false;
    }
    if name.len() >= 3 && name[..3].eq_ignore_ascii_case("xml") {
        let erlaubt = name.eq_ignore_ascii_case("xmlns")
            || (name.len() >= 6 && name[..6].eq_ignore_ascii_case("xmlns:"))
            || name[3..].starts_with(':');
        if !erlaubt {
            return false;
        }
    }
    chars.all(is_xml_name_char)
}

/// Prüft, ob `c` als weiteres Zeichen eines XML-Namens zulässig ist.
fn is_xml_name_char(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(c, '_' | '-' | '.' | ':' | '·')
        || ('\u{0300}'..='\u{036F}').contains(&c)
        || ('\u{203F}'..='\u{2040}').contains(&c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(minify: bool) -> FormatOptions {
        FormatOptions {
            indent: crate::Indent::Spaces2,
            minify,
            sort_json_keys: false,
        }
    }

    fn convert(src: &str, from: Fmt, to: Fmt, minify: bool) -> String {
        let v = parse(from, src).expect("Quelldokument ist gültig");
        serialize(to, &v, &opts(minify)).expect("Serialisierung gelingt")
    }

    #[test]
    fn json_nach_xml() {
        let src = r#"{"person":{"name":"Ada","aktiv":true,"alter":36,"tags":["a","b"]}}"#;
        let out = convert(src, Fmt::Json, Fmt::Xml, false);
        assert!(out.contains("<person"), "{out}");
        assert!(out.contains("<name>Ada</name>"), "{out}");
        assert!(out.contains("<aktiv>true</aktiv>"), "{out}");
        assert!(out.contains("<alter>36</alter>"), "{out}");
        // Eine Liste wird zu wiederholten Elementen.
        assert!(out.matches("<tags>").count() >= 2, "{out}");
        assert!(out.contains("<tags>a</tags>"), "{out}");
    }

    #[test]
    fn xml_nach_json() {
        let src = r#"<book id="9" lang="de">Rust</book>"#;
        let out = convert(src, Fmt::Xml, Fmt::Json, false);
        assert!(out.contains("\"@id\": \"9\""), "{out}");
        assert!(out.contains("\"@lang\": \"de\""), "{out}");
        assert!(out.contains("\"#text\": \"Rust\""), "{out}");
        assert!(out.starts_with("{\n  \"book\": {"), "{out}");
    }

    #[test]
    fn wiederholte_elemente_werden_liste() {
        let src = "<root><item>1</item><item>2</item></root>";
        let out = convert(src, Fmt::Xml, Fmt::Json, false);
        assert!(out.contains("\"item\": ["), "{out}");
        // Jedes Element wird wegen seines Textinhalts zum eigenen Objekt.
        assert!(out.contains("\"#text\": \"1\""), "{out}");
        assert!(out.contains("\"#text\": \"2\""), "{out}");
    }

    #[test]
    fn leeres_element_bleibt_leer() {
        let src = "<root><leer/></root>";
        let out = convert(src, Fmt::Xml, Fmt::Xml, false);
        assert!(out.contains("<leer/>"), "{out}");
    }

    #[test]
    fn xml_rundlauf_ist_stabil() {
        let src = "<config id=\"1\" aktiv=\"true\">\n  <name>Parsix</name>\n  <wert>7</wert>\n  <wert>8</wert>\n</config>";
        let once = convert(src, Fmt::Xml, Fmt::Xml, false);
        let twice = convert(&once, Fmt::Xml, Fmt::Xml, false);
        assert_eq!(once, twice);
        assert!(once.contains("<wert>7</wert>"), "{once}");
        assert!(once.contains("<wert>8</wert>"), "{once}");
    }

    #[test]
    fn json_rundlauf_ist_stabil() {
        let src = r#"{"a":1,"b":{"c":[1,2,3],"d":"x"},"e":true,"f":null,"g":1.5}"#;
        let once = convert(src, Fmt::Json, Fmt::Json, false);
        let twice = convert(&once, Fmt::Json, Fmt::Json, false);
        assert_eq!(once, twice);
    }

    #[test]
    fn yaml_nach_json_und_zurueck() {
        let src = "name: Ada\nalter: 36\norte:\n  - Berlin\n  - Rom\n";
        let json = convert(src, Fmt::Yaml, Fmt::Json, false);
        assert!(json.contains("\"name\": \"Ada\""), "{json}");
        assert!(json.contains("\"orte\": ["), "{json}");
        let yaml = convert(&json, Fmt::Json, Fmt::Yaml, false);
        assert!(yaml.contains("name: Ada"), "{yaml}");
        assert!(yaml.contains("- Berlin"), "{yaml}");
    }

    #[test]
    fn yaml_anker_und_aliase_werden_aufgeloest() {
        let src = "a: &x 1\nb: *x\n";
        let out = convert(src, Fmt::Yaml, Fmt::Json, false);
        assert!(out.contains("\"a\": 1"), "{out}");
        assert!(out.contains("\"b\": 1"), "{out}");
    }

    #[test]
    fn yaml_komplexer_schluessel_schlaegt_fehl() {
        let src = "? [1, 2]\n: wert\n";
        let r = parse(Fmt::Yaml, src);
        assert!(r.is_err());
    }

    #[test]
    fn json_nach_yaml_minify() {
        let src = r#"{"a":1,"b":[2,3]}"#;
        let out = convert(src, Fmt::Json, Fmt::Yaml, true);
        // Minify nutzt bei YAML eine Leerzeichen pro Ebene.
        assert!(out.contains("b:\n- 2\n- 3"), "{out}");
    }

    #[test]
    fn json_minify_ist_einzeilig() {
        let src = r#"{"a":1,"b":[2,3]}"#;
        let out = convert(src, Fmt::Json, Fmt::Json, true);
        assert_eq!(out, "{\"a\":1,\"b\":[2,3]}\n");
    }

    #[test]
    fn xml_minify_ist_einzeilig() {
        let src = "<a><b>1</b><b>2</b></a>";
        let out = convert(src, Fmt::Xml, Fmt::Xml, true);
        assert_eq!(out, "<a><b>1</b><b>2</b></a>\n");
    }

    #[test]
    fn mehrere_wurzelelemente_sind_fehler() {
        let src = "<a/><b/>";
        let r = parse(Fmt::Xml, src);
        assert!(r.is_err());
    }

    #[test]
    fn ungueltiges_json_mit_position() {
        let src = "{\n  \"a\": ,\n}";
        let r = parse(Fmt::Json, src);
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert_eq!(e.pos, Some(Pos::new(2, 8)), "Meldung: {}", e.msg);
    }

    #[test]
    fn ungueltiges_xml_mit_position() {
        let src = "<a>\n  <b></a>\n</b>";
        let r = parse(Fmt::Xml, src);
        assert!(r.is_err());
        assert!(r.unwrap_err().pos.is_some());
    }

    #[test]
    fn keine_wurzel_ist_fehler() {
        let r = parse(Fmt::Xml, "   ");
        assert!(r.is_err());
    }

    #[test]
    fn skaliere_wurzel_ein() {
        // Kein einschlüssiges Objekt: Wurzelelement heißt «root».
        let src = r#"{"a":1,"b":2}"#;
        let out = convert(src, Fmt::Json, Fmt::Xml, false);
        assert!(out.starts_with("<root>"), "{out}");
        assert!(out.contains("<a>1</a>"), "{out}");
    }

    #[test]
    fn liste_an_der_wurzel() {
        let src = r#"[1,2]"#;
        let out = convert(src, Fmt::Json, Fmt::Xml, false);
        assert!(out.starts_with("<root>"), "{out}");
        assert!(out.contains("<item>1</item>"), "{out}");
        assert!(out.contains("<item>2</item>"), "{out}");
    }

    #[test]
    fn xml_sonderzeichen_werden_escapet() {
        let src = r#"<t>&amp;<b>x&lt;y</b></t>"#;
        let json = convert(src, Fmt::Xml, Fmt::Json, false);
        assert!(json.contains("x<y"), "{json}");
        let xml = convert(&json, Fmt::Json, Fmt::Xml, false);
        assert!(xml.contains("x&lt;y"), "{xml}");
    }

    #[test]
    fn namensraum_praefixe_bleiben_erhalten() {
        let src = r#"<ns:tag xmlns:ns="urn:x">1</ns:tag>"#;
        let json = convert(src, Fmt::Xml, Fmt::Json, false);
        assert!(json.contains("\"ns:tag\""), "{json}");
        assert!(json.contains("\"@xmlns:ns\": \"urn:x\""), "{json}");
        let xml = convert(&json, Fmt::Json, Fmt::Xml, false);
        assert!(xml.contains("<ns:tag xmlns:ns=\"urn:x\">1</ns:tag>"), "{xml}");
    }

    #[test]
    fn xml_cdata_wird_als_text_uebernommen() {
        let src = "<t><![CDATA[x<y]]></t>";
        let out = convert(src, Fmt::Xml, Fmt::Json, false);
        assert!(out.contains("\"#text\": \"x<y\""), "{out}");
    }

    #[test]
    fn ungueltiger_xml_name_wird_berichtet() {
        let v = Value::Map(vec![("a b".into(), Value::Int(1))]);
        let r = serialize(Fmt::Xml, &v, &opts(false));
        assert!(r.is_err());
    }

    #[test]
    fn schluesselsortierung_wird_uebernommen() {
        let mut o = opts(false);
        o.sort_json_keys = true;
        let v = parse(Fmt::Json, r#"{"z":1,"a":2}"#).unwrap();
        let out = serialize(Fmt::Json, &v, &o).unwrap();
        assert!(out.find("\"a\"").unwrap() < out.find("\"z\"").unwrap(), "{out}");
    }

    #[test]
    fn toplevel_skalar() {
        let src = "42";
        let json = convert(src, Fmt::Yaml, Fmt::Json, false);
        assert_eq!(json, "42\n");
        let xml = convert(src, Fmt::Yaml, Fmt::Xml, false);
        assert!(xml.starts_with("<root>42</root>"), "{xml}");
    }

    #[test]
    fn yaml_dokumente_mit_mehreren_wurzeln_schlagen_fehl() {
        let src = "a: 1\n---\nb: 2\n";
        let r = parse(Fmt::Yaml, src);
        assert!(r.is_err());
    }
}
