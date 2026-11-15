#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// src/main.rs
mod convert;
mod highlight;
mod icons;

use eframe::egui::{
    self, text::CCursor, text::CCursorRange, Color32, Key, KeyboardShortcut, Modifiers,
};
use icons::Icon;
use quick_xml::{events::Event, Reader, Writer};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[derive(Default, PartialEq, Clone, Copy, Debug, Serialize, Deserialize)]
enum Fmt {
    #[default]
    Json,
    Xml,
    Yaml,
}

impl Fmt {
    /// Anzeigename für die Statusleiste.
    fn name(self) -> &'static str {
        match self {
            Fmt::Json => "JSON",
            Fmt::Xml => "XML",
            Fmt::Yaml => "YAML",
        }
    }
}

/// Registerkarten der Ribbon-Oberfläche.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum RibbonTab {
    #[default]
    Datei,
    Bearbeiten,
    Formatieren,
    Vergleichen,
    Ansicht,
}

impl RibbonTab {
    /// Alle Registerkarten in Anzeigereihenfolge.
    const ALL: [RibbonTab; 5] = [
        RibbonTab::Datei,
        RibbonTab::Bearbeiten,
        RibbonTab::Formatieren,
        RibbonTab::Vergleichen,
        RibbonTab::Ansicht,
    ];

    fn name(self) -> &'static str {
        match self {
            RibbonTab::Datei => "Datei",
            RibbonTab::Bearbeiten => "Bearbeiten",
            RibbonTab::Formatieren => "Formatieren",
            RibbonTab::Vergleichen => "Vergleichen",
            RibbonTab::Ansicht => "Ansicht",
        }
    }
}

// --- Formatierungsoptionen und -profile ---

/// Einrückungsart für die Formatierung.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
enum Indent {
    #[default]
    Spaces2,
    Spaces4,
    Tabs,
}

impl Indent {
    /// Einrückungseinheit für JSON.
    fn unit(self) -> &'static str {
        match self {
            Indent::Spaces2 => "  ",
            Indent::Spaces4 => "    ",
            Indent::Tabs => "\t",
        }
    }

    /// Zeichen und Breite für den XML-Writer.
    fn xml(self) -> (u8, usize) {
        match self {
            Indent::Spaces2 => (b' ', 2),
            Indent::Spaces4 => (b' ', 4),
            Indent::Tabs => (b'\t', 1),
        }
    }

    /// Breite pro Ebene für YAML.
    ///
    /// YAML erlaubt keine Tabulatoren zur Einrückung; Tabs fallen daher auf
    /// zwei Leerzeichen zurück.
    fn yaml_width(self) -> usize {
        match self {
            Indent::Spaces4 => 4,
            Indent::Spaces2 | Indent::Tabs => 2,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Indent::Spaces2 => "2 Leerzeichen",
            Indent::Spaces4 => "4 Leerzeichen",
            Indent::Tabs => "Tabs",
        }
    }
}

/// Einstellungen für die Formatierung.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub(crate) struct FormatOptions {
    indent: Indent,
    minify: bool,
    sort_json_keys: bool,
}

/// Ein benanntes Formatierungsprofil.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
struct FormatProfile {
    name: String,
    #[serde(flatten)]
    options: FormatOptions,
}

/// Sammlung von Formatierungsprofilen mit aktivem Profil.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
struct Profiles {
    profiles: Vec<FormatProfile>,
    active: usize,
}

impl Default for Profiles {
    fn default() -> Self {
        let std_opts = FormatOptions::default();
        Self {
            profiles: vec![
                FormatProfile {
                    name: "Standard (2 Leerzeichen)".into(),
                    options: std_opts,
                },
                FormatProfile {
                    name: "Breit (4 Leerzeichen)".into(),
                    options: FormatOptions {
                        indent: Indent::Spaces4,
                        ..std_opts
                    },
                },
                FormatProfile {
                    name: "Tabs".into(),
                    options: FormatOptions {
                        indent: Indent::Tabs,
                        ..std_opts
                    },
                },
                FormatProfile {
                    name: "Kompakt".into(),
                    options: FormatOptions {
                        minify: true,
                        ..std_opts
                    },
                },
                FormatProfile {
                    name: "Sortiert".into(),
                    options: FormatOptions {
                        sort_json_keys: true,
                        ..std_opts
                    },
                },
            ],
            active: 0,
        }
    }
}

impl Profiles {
    /// Stellt sicher, dass mindestens ein Profil existiert und der Index gültig ist.
    fn normalize(&mut self) {
        if self.profiles.is_empty() {
            *self = Self::default();
            return;
        }
        self.active = self.active.min(self.profiles.len() - 1);
    }

    fn active_index(&self) -> usize {
        self.active.min(self.profiles.len().saturating_sub(1))
    }

    fn active(&self) -> &FormatProfile {
        &self.profiles[self.active_index()]
    }

    fn active_mut(&mut self) -> &mut FormatProfile {
        let i = self.active_index();
        &mut self.profiles[i]
    }

    fn options(&self) -> FormatOptions {
        self.active().options
    }

    fn set_options(&mut self, options: FormatOptions) {
        self.active_mut().options = options;
    }

    /// Legt ein neues Profil an. Leere oder doppelte Namen werden abgelehnt.
    fn add(&mut self, name: &str, options: FormatOptions) -> bool {
        let name = name.trim();
        if name.is_empty() || self.profiles.iter().any(|p| p.name == name) {
            return false;
        }
        self.profiles.push(FormatProfile {
            name: name.to_string(),
            options,
        });
        self.active = self.profiles.len() - 1;
        true
    }

    /// Löscht das aktive Profil. Das letzte Profil bleibt erhalten.
    fn remove_active(&mut self) -> bool {
        if self.profiles.len() <= 1 {
            return false;
        }
        self.profiles.remove(self.active_index());
        if self.active >= self.profiles.len() {
            self.active = self.profiles.len() - 1;
        }
        true
    }
}

/// Verzeichnis für benutzerspezifische Einstellungen.
fn config_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("APPDATA") {
        return Some(PathBuf::from(p).join("parsix"));
    }
    if let Ok(p) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(p).join("parsix"));
    }
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".config").join("parsix"))
}

/// Pfad der Profildatei.
fn profiles_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("profiles.json"))
}

/// Farbschema der Oberfläche.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeMode {
    /// Alle Modi in Anzeigereihenfolge.
    const ALL: [ThemeMode; 3] = [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark];

    fn name(self) -> &'static str {
        match self {
            ThemeMode::System => "System",
            ThemeMode::Light => "Hell",
            ThemeMode::Dark => "Dunkel",
        }
    }

    /// Entsprechende Vorgabe für egui.
    fn preference(self) -> egui::ThemePreference {
        match self {
            ThemeMode::System => egui::ThemePreference::System,
            ThemeMode::Light => egui::ThemePreference::Light,
            ThemeMode::Dark => egui::ThemePreference::Dark,
        }
    }
}

/// Schriftgrößenbereich des Editors in Punkt.
const EDITOR_FONT_MIN: f32 = 9.0;
const EDITOR_FONT_MAX: f32 = 28.0;
const EDITOR_FONT_DEFAULT: f32 = 14.0;

/// Höchstzahl gemerkter zuletzt verwendeter Dateien.
const MAX_RECENT: usize = 10;

fn default_editor_font_size() -> f32 {
    EDITOR_FONT_DEFAULT
}

/// Dauerhafte Einstellungen der Oberfläche.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
struct Settings {
    #[serde(default)]
    theme: ThemeMode,
    #[serde(default = "default_editor_font_size")]
    editor_font_size: f32,
    /// Name der gewählten Systemschrift; leer bedeutet Standardschrift.
    #[serde(default)]
    editor_font: String,
    /// Zuletzt geöffnete Dateien, neueste zuerst.
    #[serde(default)]
    recent: Vec<String>,
    /// Teilung der Vergleichsansicht: `true` = nebeneinander.
    #[serde(default = "default_compare_side_by_side")]
    compare_side_by_side: bool,
}

fn default_compare_side_by_side() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeMode::System,
            editor_font_size: EDITOR_FONT_DEFAULT,
            editor_font: String::new(),
            recent: Vec::new(),
            compare_side_by_side: true,
        }
    }
}

impl Settings {
    /// Begrenzt Werte auf sinnvolle Bereiche (z. B. nach manuellem Bearbeiten).
    fn normalize(&mut self) {
        if !self.editor_font_size.is_finite() {
            self.editor_font_size = EDITOR_FONT_DEFAULT;
        }
        self.editor_font_size = self
            .editor_font_size
            .clamp(EDITOR_FONT_MIN, EDITOR_FONT_MAX);
        self.recent.truncate(MAX_RECENT);
    }

    /// Merkt eine geöffnete Datei an erster Stelle.
    fn remember_file(&mut self, path: &Path) {
        let entry = path.to_string_lossy().to_string();
        self.recent.retain(|p| p != &entry);
        self.recent.insert(0, entry);
        self.recent.truncate(MAX_RECENT);
    }
}

/// Zwischenstand nicht gespeicherter Änderungen für die Wiederherstellung.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
struct Recovery {
    /// Zugehörige Datei, falls der Inhalt aus einer Datei stammt.
    #[serde(default)]
    path: Option<String>,
    fmt: Fmt,
    text: String,
}

/// Pfad der Wiederherstellungsdatei.
fn recovery_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("recovery.json"))
}

/// Lädt einen Zwischenstand; leere oder unlesbare Dateien werden ignoriert.
fn load_recovery(path: &Path) -> Option<Recovery> {
    let recovery: Recovery = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())?;
    (!recovery.text.is_empty()).then_some(recovery)
}

/// Schreibt den Zwischenstand als JSON.
fn save_recovery(path: &Path, recovery: &Recovery) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string(recovery)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(path, json)
}

/// Pfad der Einstellungsdatei.
fn settings_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("settings.json"))
}

/// Lädt die Einstellungen; bei Fehlern gelten die Standardwerte.
fn load_settings(path: &Path) -> Settings {
    let mut settings = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Settings>(&s).ok())
        .unwrap_or_default();
    settings.normalize();
    settings
}

/// Speichert die Einstellungen als JSON.
fn save_settings(path: &Path, settings: &Settings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(path, json)
}

/// Prüft grob den Dateikopf, damit nur gültige Schriftdateien an den
/// Renderer gereicht werden (ungültige Daten führen dort zu einem Abbruch).
fn read_font_file(path: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    let known = bytes.starts_with(&[0x00, 0x01, 0x00, 0x00])
        || bytes.starts_with(b"true")
        || bytes.starts_with(b"OTTO")
        || bytes.starts_with(b"ttcf");
    known.then_some(bytes)
}

/// Sucht gängige Monospace-Schriften des Systems.
fn font_candidates() -> Vec<(String, PathBuf)> {
    #[allow(unused_mut)]
    let mut list: Vec<(String, PathBuf)> = Vec::new();

    #[cfg(target_os = "windows")]
    {
        let dir = PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into()))
            .join("Fonts");
        list.push(("Consolas".into(), dir.join("consola.ttf")));
        list.push(("Cascadia Mono".into(), dir.join("CascadiaMono.ttf")));
        list.push(("Cascadia Code".into(), dir.join("CascadiaCode.ttf")));
        list.push(("Lucida Console".into(), dir.join("lucon.ttf")));
        list.push(("Courier New".into(), dir.join("cour.ttf")));
    }

    #[cfg(target_os = "macos")]
    {
        list.push((
            "Menlo".into(),
            PathBuf::from("/System/Library/Fonts/Menlo.ttc"),
        ));
        list.push((
            "Courier New".into(),
            PathBuf::from("/Library/Fonts/Courier New.ttf"),
        ));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        list.push((
            "DejaVu Sans Mono".into(),
            PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"),
        ));
        list.push((
            "Liberation Mono".into(),
            PathBuf::from("/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf"),
        ));
        list.push((
            "Ubuntu Mono".into(),
            PathBuf::from("/usr/share/fonts/truetype/ubuntu/UbuntuMono-R.ttf"),
        ));
    }

    list.retain(|(_, path)| path.is_file());
    list
}

/// Lädt Profile; bei Fehlern werden die Standardprofile verwendet.
fn load_profiles(path: &Path) -> Profiles {
    let mut profiles = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Profiles>(&s).ok())
        .unwrap_or_default();
    profiles.normalize();
    profiles
}

/// Speichert Profile als JSON.
fn save_profiles(path: &Path, profiles: &Profiles) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(profiles)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(path, json)
}

/// Zustand einer Datei auf dem Datenträger.
///
/// Änderungszeit und Länge reichen aus, um externe Bearbeitungen zu erkennen,
/// ohne die Datei bei jeder Prüfung neu einlesen zu müssen.
#[derive(Clone, Copy, PartialEq, Debug)]
struct DiskState {
    modified: Option<SystemTime>,
    len: u64,
}

impl DiskState {
    fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        Some(Self {
            modified: meta.modified().ok(),
            len: meta.len(),
        })
    }
}

/// Einrückungsschritt für automatisch eingefügte Zeilen.
const INDENT_UNIT: &str = "  ";

/// Eingabe, die vor dem Editor ausgewertet wird.
#[derive(Clone, Copy)]
enum Typing {
    Char(char),
    Backspace,
}

/// Führender Leerraum einer Zeile.
fn leading_ws(s: &str) -> &str {
    let rest = s.trim_start_matches([' ', '\t']);
    &s[..s.len() - rest.len()]
}

/// Prüft, ob nach dieser Zeile ein eingerückter Block folgt.
fn opens_block(fmt: Fmt, line: &str) -> bool {
    let line = line.trim_end();
    match fmt {
        Fmt::Json => line.ends_with('{') || line.ends_with('['),
        Fmt::Xml => {
            // Letzter Tag der Zeile: nur ein offener Tag am Zeilenende öffnet einen Block.
            let Some(lt) = line.rfind('<') else {
                return false;
            };
            let tag = &line[lt..];
            tag.ends_with('>')
                && !tag.ends_with("/>")
                && !tag.starts_with("</")
                && !tag.starts_with("<!")
                && !tag.starts_with("<?")
        }
        Fmt::Yaml => {
            line.ends_with(':')
                || line.trim() == "-"
                || line.ends_with(" |")
                || line.ends_with(" >")
        }
    }
}

/// Text, der bei einem Zeilenumbruch an der Byte-Position `cursor` eingefügt
/// wird: Zeilenumbruch plus Einrückung der aktuellen Zeile, bei geöffneten
/// Blöcken um einen Schritt vertieft.
fn newline_with_indent(fmt: Fmt, text: &str, cursor: usize) -> String {
    let cursor = cursor.min(text.len());
    let line_start = text[..cursor].rfind('\n').map_or(0, |p| p + 1);
    let line = &text[line_start..cursor];
    let indent = leading_ws(line);

    let mut out = String::from("\n");
    out.push_str(indent);
    if opens_block(fmt, line) {
        if indent.contains('\t') {
            out.push('\t');
        } else {
            out.push_str(INDENT_UNIT);
        }
    }
    out
}

/// Byte-Position des Zeichens mit dem Index `char_idx`.
fn byte_index(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map_or(text.len(), |(i, _)| i)
}

/// Zeichen, bei denen eine automatische Vervollständigung geprüft wird.
fn is_completion_char(fmt: Fmt, c: char) -> bool {
    match fmt {
        Fmt::Json => matches!(c, '{' | '[' | '"' | '}' | ']'),
        Fmt::Yaml => matches!(c, '{' | '[' | '"' | '\'' | '}' | ']'),
        Fmt::Xml => matches!(c, '"' | '\'' | '>'),
    }
}

/// Schließende Zeichen, die ein bereits vorhandenes Zeichen überspringen.
fn is_closing_char(fmt: Fmt, c: char) -> bool {
    match fmt {
        Fmt::Json => matches!(c, '}' | ']' | '"'),
        Fmt::Yaml => matches!(c, '}' | ']' | '"' | '\''),
        Fmt::Xml => matches!(c, '"' | '\''),
    }
}

/// Passendes schließendes Zeichen, das automatisch ergänzt wird.
fn auto_pair(fmt: Fmt, text: &str, cursor: usize, open: char) -> Option<char> {
    match (fmt, open) {
        (Fmt::Json, '{') | (Fmt::Yaml, '{') => Some('}'),
        (Fmt::Json, '[') | (Fmt::Yaml, '[') => Some(']'),
        (Fmt::Json, '"') | (Fmt::Yaml, '"') => Some('"'),
        (Fmt::Yaml, '\'') => Some('\''),
        // In XML nur innerhalb eines Tags ergänzen, nicht im Textinhalt.
        (Fmt::Xml, '"' | '\'') if inside_tag(text, cursor) => Some(open),
        _ => None,
    }
}

/// Befindet sich die Position innerhalb eines XML-Tags?
fn inside_tag(text: &str, cursor: usize) -> bool {
    let before = &text[..cursor.min(text.len())];
    match (before.rfind('<'), before.rfind('>')) {
        (Some(lt), Some(gt)) => lt > gt,
        (Some(_), None) => true,
        _ => false,
    }
}

/// Name des Tags, das durch das gerade eingefügte `>` geöffnet wurde.
///
/// `cursor` zeigt direkt hinter das `>`. Liefert `None` bei schließenden Tags,
/// Kommentaren, Deklarationen und selbstschließenden Tags.
fn open_tag_name(text: &str, cursor: usize) -> Option<&str> {
    let before = text.get(..cursor)?;
    let inner_end = before.strip_suffix('>').map(str::len)?;
    let lt = before[..inner_end].rfind('<')?;
    let tag = &before[lt + 1..inner_end];
    if tag.is_empty()
        || tag.contains('>')
        || tag.contains('<')
        || tag.starts_with('/')
        || tag.starts_with('!')
        || tag.starts_with('?')
        || tag.ends_with('/')
    {
        return None;
    }
    let name = tag.split_whitespace().next()?;
    match name.chars().next() {
        Some(c) if c.is_alphabetic() || c == '_' || c == ':' => Some(name),
        _ => None,
    }
}

/// Ergebnis einer automatischen Bearbeitung.
#[derive(Debug, PartialEq)]
struct Typed {
    text: String,
    /// Cursorposition danach (Zeichenindex).
    cursor: usize,
}

/// Wendet ein getipptes Zeichen inklusive Vervollständigung an.
///
/// `None` bedeutet: Standardverhalten des Editors (Zeichen normal einfügen).
fn apply_char(fmt: Fmt, text: &str, start: usize, end: usize, ch: char) -> Option<Typed> {
    let chars = text.chars().count();
    let start = start.min(chars);
    let end = end.min(chars).max(start);
    let start_b = byte_index(text, start);
    let end_b = byte_index(text, end);
    let has_selection = start != end;

    // Anführungszeichen in XML nur innerhalb von Tags ergänzen.
    if fmt == Fmt::Xml && matches!(ch, '"' | '\'') && !inside_tag(text, start_b) {
        return None;
    }

    // Bereits vorhandenes schließendes Zeichen überspringen.
    if !has_selection && is_closing_char(fmt, ch) && text[start_b..].starts_with(ch) {
        return Some(Typed {
            text: text.to_owned(),
            cursor: start + 1,
        });
    }

    if let Some(close) = auto_pair(fmt, text, start_b, ch) {
        let mut out = text.to_owned();
        if has_selection {
            // Auswahl wird vom Paar umschlossen.
            out.insert(end_b, close);
            out.insert(start_b, ch);
            Some(Typed {
                text: out,
                cursor: end + 1,
            })
        } else {
            let pair: String = [ch, close].iter().collect();
            out.replace_range(start_b..end_b, &pair);
            Some(Typed {
                text: out,
                cursor: start + 1,
            })
        }
    } else if fmt == Fmt::Xml && ch == '>' && !has_selection {
        // Tag-Vervollständigung: <name> wird zu <name></name>.
        let mut out = text.to_owned();
        out.insert(start_b, '>');
        let after_gt = start_b + 1;
        if let Some(name) = open_tag_name(&out, after_gt).map(str::to_owned) {
            out.insert_str(after_gt, &format!("</{name}>"));
        }
        Some(Typed {
            text: out,
            cursor: start + 1,
        })
    } else {
        None
    }
}

/// Entfernt ein leeres Paar beim Löschen (z. B. `{}` → leer).
fn apply_backspace(fmt: Fmt, text: &str, start: usize, end: usize) -> Option<Typed> {
    let chars = text.chars().count();
    let start = start.min(chars);
    let end = end.min(chars).max(start);
    if start != end || start == 0 {
        return None;
    }
    let pos_b = byte_index(text, start);
    let before = text[..pos_b].chars().next_back()?;
    let after = text[pos_b..].chars().next()?;
    if auto_pair(fmt, text, pos_b, before) != Some(after) {
        return None;
    }
    let from = byte_index(text, start - 1);
    let to = byte_index(text, start + 1);
    let mut out = text.to_owned();
    out.replace_range(from..to, "");
    Some(Typed {
        text: out,
        cursor: start - 1,
    })
}

// --- Zeilen markieren, kopieren und verschieben ---

/// Zerlegt den Text in Zeilen ohne Zeilenende.
///
/// Der zweite Rückgabewert sagt, ob das Dokument mit einem Umbruch endet.
fn split_lines(text: &str) -> (Vec<&str>, bool) {
    let trailing = text.ends_with('\n');
    let mut lines: Vec<&str> = text.split('\n').collect();
    if trailing {
        // Der letzte Eintrag ist der leere Text hinter dem abschließenden Umbruch.
        lines.pop();
    }
    (lines, trailing)
}

/// Zeilennummer (0-basiert) für einen Zeichenindex.
fn line_of_char(text: &str, char_idx: usize) -> usize {
    text.chars().take(char_idx).filter(|c| *c == '\n').count()
}

/// Zeichenindex, an dem eine Zeile beginnt.
fn line_start_char(lines: &[&str], line: usize) -> usize {
    lines.iter().take(line).map(|l| l.chars().count() + 1).sum()
}

/// Auswahlbereich ganzer Zeilen (Zeichenindizes, Ende exklusiv).
fn line_span(lines: &[&str], trailing: bool, first: usize, last: usize) -> (usize, usize) {
    let start = line_start_char(lines, first);
    let end = if last + 1 < lines.len() {
        line_start_char(lines, last + 1)
    } else {
        line_start_char(lines, last) + lines[last].chars().count() + usize::from(trailing)
    };
    (start, end)
}

/// Wählt die Zeile an der Cursorposition aus (Zeichenindizes).
///
/// Der abschließende Zeilenumbruch gehört zur Auswahl, sofern vorhanden.
fn select_line(text: &str, cursor: usize) -> (usize, usize) {
    let chars = text.chars().count();
    let cursor = cursor.min(chars);
    let mut start = 0;
    for (i, c) in text.chars().take(cursor).enumerate() {
        if c == '\n' {
            start = i + 1;
        }
    }
    let mut end = chars;
    for (i, c) in text.chars().enumerate().skip(cursor) {
        if c == '\n' {
            end = i + 1;
            break;
        }
    }
    (start, end)
}

/// Text der Zeile an der Cursorposition, einschließlich Zeilenumbruch.
fn current_line_text(text: &str, cursor: usize) -> String {
    let (start, end) = select_line(text, cursor);
    text.chars().skip(start).take(end - start).collect()
}

/// Entfernt ein Kopier-Ereignis aus der Eingabe.
///
/// egui-winit wandelt `Strg+C` bereits vor dem Widget in `Event::Copy` um,
/// deshalb kann die Tastenkombination nicht über `consume_key` abgefangen werden.
fn consume_copy_event(ctx: &egui::Context) -> bool {
    ctx.input_mut(|i| {
        if let Some(p) = i.events.iter().position(|e| matches!(e, egui::Event::Copy)) {
            i.events.remove(p);
            true
        } else {
            false
        }
    })
}

/// Ergebnis einer Zeilenverschiebung.
#[derive(Debug, PartialEq)]
struct LineMove {
    text: String,
    /// Auswahl der verschobenen Zeilen danach (Zeichenindizes).
    sel: (usize, usize),
}

/// Verschiebt die von der Auswahl berührten Zeilen um eine Zeile nach oben oder unten.
///
/// Ohne Auswahl wird die Zeile an der Cursorposition bewegt und der Cursor
/// behält seine Spalte. Mit Auswahl bleiben die verschobenen Zeilen markiert.
fn move_lines(text: &str, start: usize, end: usize, up: bool) -> Option<LineMove> {
    let chars = text.chars().count();
    let start = start.min(chars);
    let end = end.min(chars).max(start);
    let (lines, trailing) = split_lines(text);
    if lines.is_empty() {
        return None;
    }

    let first = line_of_char(text, start);
    // Endet die Auswahl genau am Anfang einer Zeile, gehört diese nicht mehr dazu.
    let mut last = if end > start && text.chars().nth(end - 1) == Some('\n') {
        line_of_char(text, end).saturating_sub(1)
    } else {
        line_of_char(text, end)
    };
    last = last.min(lines.len() - 1);

    if up {
        if first == 0 {
            return None;
        }
    } else if last + 1 >= lines.len() {
        return None;
    }

    let mut new_lines = lines.clone();
    let (new_first, new_last) = if up {
        let prev = new_lines.remove(first - 1);
        new_lines.insert(last, prev);
        (first - 1, last - 1)
    } else {
        let next = new_lines.remove(last + 1);
        new_lines.insert(first, next);
        (first + 1, last + 1)
    };

    let mut new_text = new_lines.join("\n");
    if trailing {
        new_text.push('\n');
    }

    let sel = if start == end {
        // Cursor: Spalte beibehalten.
        let col = start - line_start_char(&lines, first);
        let cursor =
            line_start_char(&new_lines, new_first) + col.min(new_lines[new_first].chars().count());
        (cursor, cursor)
    } else {
        line_span(&new_lines, trailing, new_first, new_last)
    };

    Some(LineMove {
        text: new_text,
        sel,
    })
}

/// Position eines Fehlers im Text: Zeile und Spalte, jeweils 1-basiert.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Pos {
    line: usize,
    col: usize,
}

impl Pos {
    fn new(line: usize, col: usize) -> Self {
        Self {
            line: line.max(1),
            col: col.max(1),
        }
    }
}

#[derive(Debug)]
pub(crate) struct FmtErr {
    msg: String,
    pos: Option<Pos>,
}

pub(crate) fn err(msg: impl ToString, pos: Option<Pos>) -> FmtErr {
    FmtErr {
        msg: msg.to_string(),
        pos,
    }
}

/// Rechnet einen Byte-Offset in Zeile und Zeichen-Spalte um (1-basiert).
///
/// Liegt der Offset mitten in einem UTF-8-Zeichen, wird auf die vorherige
/// Zeichengrenze zurückgegangen.
pub(crate) fn pos_at_byte(text: &str, offset: usize) -> Pos {
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let col = text[line_start..offset].chars().count() + 1;
    Pos::new(line, col)
}

/// Liefert den Zeichenindex zu einer Zeile/Spalte (1-basiert).
///
/// Positionen hinter dem Zeilenende werden auf das Zeilenende begrenzt.
fn char_index_at(text: &str, pos: Pos) -> usize {
    let mut line = 1;
    let mut col = 1;
    for (i, c) in text.chars().enumerate() {
        if line > pos.line || (line == pos.line && col >= pos.col) {
            return i;
        }
        if c == '\n' {
            if line == pos.line {
                return i;
            }
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    text.chars().count()
}

/// Ermittelt Zeile und Spalte (1-basiert) zu einem Zeichenindex.
fn line_col(text: &str, cursor: usize) -> Pos {
    let chars = text.chars().count();
    let mut line = 1;
    let mut col = 1;
    for c in text.chars().take(cursor.min(chars)) {
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    Pos::new(line, col)
}

/// Länge einer Positionsangabe am Anfang von `s`.
///
/// Erkennt `line 1 column 2` und `position 3`.
fn position_len(s: &str) -> Option<usize> {
    let digits = |t: &str| t.chars().take_while(char::is_ascii_digit).count();
    if let Some(t) = s.strip_prefix("line ") {
        let n = digits(t);
        if n == 0 {
            return None;
        }
        let rest = t[n..].strip_prefix(" column ")?;
        let m = digits(rest);
        if m == 0 {
            return None;
        }
        return Some("line ".len() + n + " column ".len() + m);
    }
    if let Some(t) = s.strip_prefix("position ") {
        let n = digits(t);
        if n == 0 {
            return None;
        }
        return Some("position ".len() + n);
    }
    None
}

/// Entfernt Positionsangaben aus einer Fehlermeldung.
///
/// serde_json und serde_yaml_ng hängen selbst Zeilen- und Spaltenangaben an
/// ihre Meldungen an. Da Parsix die Position getrennt anzeigt, werden diese
/// Angaben hier entfernt, damit sie nicht doppelt erscheinen.
pub(crate) fn strip_positions(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    let mut rest = msg;
    while let Some(i) = rest.find(" at ") {
        let tail = &rest[i + 4..];
        match position_len(tail) {
            Some(len) => {
                out.push_str(&rest[..i]);
                rest = &tail[len..];
            }
            None => {
                out.push_str(&rest[..i + 4]);
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out.trim_end_matches([',', ' ']).to_string()
}

fn format_json(s: &str, opts: &FormatOptions) -> Result<String, FmtErr> {
    let v: serde_json::Value = serde_json::from_str(s).map_err(|e| {
        let pos = (e.line() > 0).then(|| Pos::new(e.line(), e.column()));
        err(strip_positions(&e.to_string()), pos)
    })?;
    let mut out = String::new();
    if opts.minify {
        json_compact(&v, opts.sort_json_keys, &mut out);
    } else {
        json_pretty(&v, opts, 0, &mut out);
    }
    // Einheitliche Formatierungsregel: genau ein abschließender Umbruch.
    out.push('\n');
    Ok(out)
}

/// Schreibt eine Einrückung der angegebenen Tiefe.
fn json_indent(opts: &FormatOptions, depth: usize, out: &mut String) {
    for _ in 0..depth {
        out.push_str(opts.indent.unit());
    }
}

/// Serialisiert einen JSON-Wert lesbar mit konfigurierbarer Einrückung.
pub(crate) fn json_pretty(v: &serde_json::Value, opts: &FormatOptions, depth: usize, out: &mut String) {
    use serde_json::Value;
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => {
            out.push_str(&serde_json::to_string(s).expect("Zeichenkette ist serialisierbar"))
        }
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                json_indent(opts, depth + 1, out);
                json_pretty(item, opts, depth + 1, out);
                if i + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            json_indent(opts, depth, out);
            out.push(']');
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            if opts.sort_json_keys {
                entries.sort_by(|a, b| a.0.cmp(b.0));
            }
            out.push_str("{\n");
            for (i, (key, value)) in entries.iter().enumerate() {
                json_indent(opts, depth + 1, out);
                out.push_str(&serde_json::to_string(key).expect("Schlüssel ist serialisierbar"));
                out.push_str(": ");
                json_pretty(value, opts, depth + 1, out);
                if i + 1 < entries.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            json_indent(opts, depth, out);
            out.push('}');
        }
    }
}

/// Serialisiert einen JSON-Wert ohne Leerraum (Minify).
pub(crate) fn json_compact(v: &serde_json::Value, sort_keys: bool, out: &mut String) {
    use serde_json::Value;
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => {
            out.push_str(&serde_json::to_string(s).expect("Zeichenkette ist serialisierbar"))
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                json_compact(item, sort_keys, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            if sort_keys {
                entries.sort_by(|a, b| a.0.cmp(b.0));
            }
            out.push('{');
            for (i, (key, value)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).expect("Schlüssel ist serialisierbar"));
                out.push(':');
                json_compact(value, sort_keys, out);
            }
            out.push('}');
        }
    }
}

// --- Quelltexterhaltende YAML-Formatierung ---

/// Zustand beim zeilenweisen Scannen von YAML-Quelltext.
#[derive(Default, Clone, Copy)]
struct ScanState {
    /// Offenes Anführungszeichen, das über das Zeilenende hinausgeht.
    quote: Option<char>,
    /// Verschachtelungstiefe von `[` und `{`.
    flow: usize,
}

/// Scannt eine Zeile und liefert den Beginn eines Kommentars sowie den neuen Zustand.
///
/// Anführungszeichen und Klammern innerhalb von Zeichenketten werden ignoriert.
fn scan_yaml_line(line: &str, mut state: ScanState) -> (Option<usize>, ScanState) {
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut comment = None;
    while i < bytes.len() {
        let b = bytes[i];
        match state.quote {
            Some('\'') => {
                if b == b'\'' {
                    // Zwei Apostrophe stehen für ein einzelnes Zeichen.
                    if bytes.get(i + 1) == Some(&b'\'') {
                        i += 2;
                        continue;
                    }
                    state.quote = None;
                }
            }
            Some(_) => {
                if b == b'\\' {
                    i += 2;
                    continue;
                }
                if b == b'"' {
                    state.quote = None;
                }
            }
            None => {
                if b == b'"' || b == b'\'' {
                    state.quote = Some(b as char);
                } else if b == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
                    comment = Some(i);
                    break;
                } else if b == b'[' || b == b'{' {
                    state.flow += 1;
                } else if b == b']' || b == b'}' {
                    state.flow = state.flow.saturating_sub(1);
                }
            }
        }
        i += 1;
    }
    (comment, state)
}

/// Verschiebt die Einrückung einer Zeile um `delta` Leerzeichen.
fn shift_spaces(line: &str, delta: isize) -> String {
    if delta == 0 {
        return line.to_string();
    }
    let indent = line.len() - line.trim_start_matches(' ').len();
    if delta > 0 {
        format!("{}{}", " ".repeat(delta as usize), line)
    } else {
        let remove = ((-delta) as usize).min(indent);
        line[remove..].to_string()
    }
}

/// Ordnet einer Einrückung die passende Ebene zu und pflegt den Einrückungsstapel.
fn yaml_level(stack: &mut Vec<usize>, indent: usize) -> usize {
    if stack.is_empty() {
        stack.push(indent);
        return 0;
    }
    while stack.len() > 1 && indent < *stack.last().unwrap() {
        stack.pop();
    }
    if indent > *stack.last().unwrap() {
        stack.push(indent);
    }
    stack.len() - 1
}

/// Erkennt, ob der Code einer Zeile einen Block-Skalar (`|` oder `>`) einleitet.
///
/// Ein `|` mitten in einem Klartextwert (z. B. `key: a |`) zählt nicht.
fn starts_block_scalar(code: &str) -> bool {
    let mut toks: Vec<&str> = code.split_whitespace().collect();
    let Some(last) = toks.pop() else { return false };
    let mut chars = last.chars();
    if !matches!(chars.next(), Some('|') | Some('>')) {
        return false;
    }
    let rest: String = chars.collect();
    if rest.len() > 2
        || !rest
            .chars()
            .all(|c| c == '+' || c == '-' || c.is_ascii_digit())
    {
        return false;
    }
    // Vor dem Indikator muss ein Blockkontext stehen (Schlüssel, Sequenz, Tag, Anker).
    while let Some(t) = toks.last() {
        if t.starts_with('!') || t.starts_with('&') {
            toks.pop();
        } else {
            break;
        }
    }
    match toks.last() {
        None => true,
        Some(t) => *t == "-" || *t == "?" || t.ends_with(':'),
    }
}

/// Formatiert YAML quelltextnah.
///
/// Kommentare, Leerzeilen, Anker, Aliase, Zeichenketten und Block-Skalare
/// bleiben erhalten. Geändert wird nur die Einrückung, die auf die gewählte
/// Breite pro Ebene normalisiert wird.
///
/// Bei `minify` werden zusätzlich Leerzeilen, reine Kommentarzeilen und
/// Inline-Kommentare entfernt. Block-Skalare und mehrzeilige Zeichenketten
/// bleiben dabei unverändert, da ihr Inhalt bedeutungstragend ist.
fn reindent_yaml(src: &str, opts: &FormatOptions) -> Option<String> {
    // YAML erlaubt keine Tabulatoren zur Einrückung; im Minify-Modus ist eine
    // Ebene genau ein Leerzeichen breit.
    let width = if opts.minify {
        1
    } else {
        opts.indent.yaml_width()
    };
    let minify = opts.minify;
    let lines: Vec<&str> = src.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut stack: Vec<usize> = Vec::new();
    // Block-Skalar: (Einrückung der Indikatorzeile, Verschiebung)
    let mut block: Option<(usize, isize)> = None;
    let mut scan = ScanState::default();
    let mut flow_shift: isize = 0;

    for line in lines {
        let indent = line.len() - line.trim_start_matches(' ').len();
        let content = &line[indent..];

        // Tabulatoren als Einrückung sind in YAML nicht erlaubt.
        if content.starts_with('\t') {
            return None;
        }

        // Inhalt eines Block-Skalars: unverändert lassen, nur mitschieben.
        if let Some((parent, delta)) = block {
            if content.trim().is_empty() {
                out.push(String::new());
                continue;
            }
            if indent > parent {
                out.push(shift_spaces(line, delta));
                continue;
            }
            block = None;
        }

        let quote_continuation = scan.quote.is_some();
        let (comment, new_scan) = scan_yaml_line(line, scan);

        // Fortsetzung einer mehrzeiligen Zeichenkette: unverändert lassen.
        if quote_continuation {
            if content.trim().is_empty() {
                out.push(String::new());
            } else {
                out.push(line.to_string());
            }
            scan = new_scan;
            continue;
        }

        // Innerhalb einer mehrzeiligen Flow-Sammlung: unverändert lassen,
        // aber gemeinsam mit der öffnenden Zeile verschieben.
        if scan.flow > 0 {
            if content.trim().is_empty() {
                out.push(String::new());
            } else {
                out.push(shift_spaces(line, flow_shift));
            }
            scan = new_scan;
            continue;
        }

        if content.trim().is_empty() {
            if !minify {
                out.push(String::new());
            }
            continue;
        }

        // Kommentarzeile: an der aktuellen Ebene ausrichten. Kommentare vor
        // dem ersten Inhalt bleiben am linken Rand.
        if content.starts_with('#') {
            if minify {
                continue;
            }
            let new_indent = if stack.is_empty() {
                0
            } else {
                yaml_level(&mut stack, indent) * width
            };
            out.push(format!("{}{}", " ".repeat(new_indent), content.trim_end()));
            continue;
        }

        // Dokumentmarker und Direktiven stehen immer am Zeilenanfang.
        let marker = content.starts_with('%')
            || content == "---"
            || content == "..."
            || content.starts_with("--- ")
            || content.starts_with("... ");
        if marker {
            stack.clear();
            out.push(content.trim_end().to_string());
            // Ein Dokumentmarker kann eine mehrzeilige Zeichenkette eröffnen.
            scan = new_scan;
            continue;
        }

        let new_indent = yaml_level(&mut stack, indent) * width;
        let content_comment = comment.map(|c| c - indent);
        let code = match content_comment {
            Some(c) => content[..c].trim_end(),
            None => content.trim_end(),
        };

        let mut formatted = " ".repeat(new_indent);
        formatted.push_str(code);
        if let (Some(c), false) = (content_comment, minify) {
            // Abstand zwischen Code und Kommentar erhalten.
            formatted.push_str(&content[code.len()..c]);
            formatted.push_str(content[c..].trim_end());
        }
        out.push(formatted);

        if starts_block_scalar(code) {
            block = Some((indent, new_indent as isize - indent as isize));
        } else if new_scan.flow > 0 && scan.flow == 0 {
            // Die folgende Flow-Sammlung mit der öffnenden Zeile verschieben.
            flow_shift = new_indent as isize - indent as isize;
        }
        scan = new_scan;
    }

    // Einheitliche Formatierungsregel: genau ein abschließender Umbruch.
    // Endet das Dokument in einem Block-Skalar, bleiben dessen abschließende
    // Leerzeilen erhalten (bedeutungstragend).
    let mut text = out.join("\n");
    if block.is_none() {
        while text.ends_with("\n\n") {
            text.pop();
        }
        if !text.ends_with('\n') {
            text.push('\n');
        }
    }
    Some(text)
}

/// Formatiert YAML unter Erhalt von Kommentaren und Quelltextstruktur.
pub(crate) fn format_yaml_source(s: &str, opts: &FormatOptions) -> String {
    if s.trim().is_empty() {
        return String::new();
    }
    let src = s.replace("\r\n", "\n").replace('\r', "\n");
    reindent_yaml(&src, opts).unwrap_or(src)
}

fn format_yaml(s: &str, opts: &FormatOptions) -> Result<String, FmtErr> {
    let _: serde_yaml_ng::Value = serde_yaml_ng::from_str(s).map_err(|e| {
        let pos = e.location().map(|l| Pos::new(l.line(), l.column()));
        err(strip_positions(&e.to_string()), pos)
    })?;
    Ok(format_yaml_source(s, opts))
}

fn format_xml(s: &str, opts: &FormatOptions) -> Result<String, FmtErr> {
    let mut r = Reader::from_str(s);
    r.config_mut().trim_text(true);
    let mut w = if opts.minify {
        Writer::new(Vec::new())
    } else {
        let (ch, n) = opts.indent.xml();
        Writer::new_with_indent(Vec::new(), ch, n)
    };
    loop {
        match r.read_event() {
            Ok(Event::Eof) => break,
            Ok(ev) => w.write_event(ev).map_err(|e| err(e, None))?,
            Err(e) => {
                let pos = pos_at_byte(s, r.error_position() as usize);
                return Err(err(&e, Some(pos)));
            }
        }
    }
    let text = String::from_utf8(w.into_inner()).map_err(|e| err(e, None))?;
    // Einheitliche Formatierungsregel: genau ein abschließender Umbruch.
    let mut text = text.trim_end().to_string();
    text.push('\n');
    Ok(text)
}

/// Wartezeit in Sekunden, nach der eine Tipp-Pause als ein Undo-Schritt gilt.
const UNDO_DEBOUNCE_SECS: f64 = 0.5;
/// Obergrenze für die Anzahl der Undo-Schritte.
const MAX_UNDO_STATES: usize = 100;
/// Obergrenze für den Speicherbedarf der Undo-Historie (Bytes).
const MAX_UNDO_BYTES: usize = 32 * 1024 * 1024;

/// Ein einzelner Zustand des Dokuments für Undo / Redo.
#[derive(Clone, PartialEq)]
struct Snapshot {
    text: String,
    /// Cursorposition als Zeichenindex (nicht Byte-Index).
    cursor: usize,
}

/// Undo-/Redo-Historie als Liste von Dokumentzuständen.
///
/// `states[pos]` ist immer der aktuelle Stand. Alles nach `pos` ist der Redo-Zweig.
struct History {
    states: Vec<Snapshot>,
    pos: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::new("", 0)
    }
}

impl History {
    fn new(text: &str, cursor: usize) -> Self {
        Self {
            states: vec![Snapshot {
                text: text.to_owned(),
                cursor,
            }],
            pos: 0,
        }
    }

    fn current(&self) -> &Snapshot {
        &self.states[self.pos]
    }

    fn can_undo(&self) -> bool {
        self.pos > 0
    }

    fn can_redo(&self) -> bool {
        self.pos + 1 < self.states.len()
    }

    /// Nimmt den übergebenen Stand auf, falls er sich vom aktuellen unterscheidet.
    ///
    /// Gibt `true` zurück, wenn ein neuer Eintrag entstanden ist.
    fn record(&mut self, text: &str, cursor: usize) -> bool {
        if text == self.current().text {
            self.states[self.pos].cursor = cursor;
            return false;
        }
        // Ein neuer Bearbeitungsschritt verwirft den Redo-Zweig.
        self.states.truncate(self.pos + 1);
        self.states.push(Snapshot {
            text: text.to_owned(),
            cursor,
        });
        self.pos += 1;
        self.trim();
        true
    }

    /// Begrenzt die Historie nach Anzahl und Speicherbedarf.
    fn trim(&mut self) {
        let mut bytes: usize = self.states.iter().map(|s| s.text.len()).sum();
        while self.states.len() > 1
            && (self.states.len() > MAX_UNDO_STATES || bytes > MAX_UNDO_BYTES)
        {
            bytes -= self.states.remove(0).text.len();
            self.pos = self.pos.saturating_sub(1);
        }
    }

    /// Macht den letzten Schritt rückgängig. Noch nicht aufgenommene Änderungen
    /// (z. B. laufende Tipp-Eingaben) werden vorher gesichert.
    fn undo(&mut self, text: &str, cursor: usize) -> Option<Snapshot> {
        self.record(text, cursor);
        if !self.can_undo() {
            return None;
        }
        self.pos -= 1;
        Some(self.states[self.pos].clone())
    }

    /// Stellt einen rückgängig gemachten Schritt wieder her.
    fn redo(&mut self, text: &str, cursor: usize) -> Option<Snapshot> {
        // Offene Änderungen würden den Redo-Zweig ungültig machen.
        self.record(text, cursor);
        if !self.can_redo() {
            return None;
        }
        self.pos += 1;
        Some(self.states[self.pos].clone())
    }
}

// --- Erweiterte Suche ---

/// Optionen für die Suche.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct SearchOpts {
    case_sensitive: bool,
    whole_word: bool,
    use_regex: bool,
}

/// Ist das Zeichen Teil eines Worts im Sinne der Ganzwortsuche?
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Prüft, ob ein Treffer an beiden Rändern auf einer Wortgrenze liegt.
///
/// Ein Treffer gilt als ganzes Wort, wenn ein Wortzeichen am Rand des Treffers
/// nicht direkt an ein weiteres Wortzeichen grenzt. Bei Treffern, die selbst
/// nicht mit einem Wortzeichen beginnen bzw. enden, entfällt die Prüfung.
fn is_whole_word(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    let starts_word = text[start..end].chars().next().is_some_and(is_word_char);
    let ends_word = text[start..end]
        .chars()
        .next_back()
        .is_some_and(is_word_char);
    (!starts_word || !before.is_some_and(is_word_char))
        && (!ends_word || !after.is_some_and(is_word_char))
}

/// Baut den regulären Ausdruck für Muster und Optionen.
///
/// Ohne Regex-Modus wird das Muster maskiert, sodass Sonderzeichen wörtlich
/// gesucht werden.
fn build_search_regex(pattern: &str, opts: SearchOpts) -> Result<regex::Regex, String> {
    let src = if opts.use_regex {
        pattern.to_string()
    } else {
        regex::escape(pattern)
    };
    regex::RegexBuilder::new(&src)
        .case_insensitive(!opts.case_sensitive)
        .build()
        .map_err(|e| e.to_string())
}

/// Liefert alle Treffer als Byte-Bereiche.
///
/// Leere Treffer (z. B. durch `^` oder `a*`) werden übersprungen, damit
/// Hervorhebung und Navigation eindeutig bleiben.
fn find_all(text: &str, pattern: &str, opts: SearchOpts) -> Result<Vec<(usize, usize)>, String> {
    if pattern.is_empty() {
        return Ok(Vec::new());
    }
    let re = build_search_regex(pattern, opts)?;
    let mut out = Vec::new();
    for m in re.find_iter(text) {
        if m.start() == m.end() {
            continue;
        }
        if opts.whole_word && !is_whole_word(text, m.start(), m.end()) {
            continue;
        }
        out.push((m.start(), m.end()));
    }
    Ok(out)
}

/// Ersetzt alle Treffer.
///
/// Im Regex-Modus werden Rückverweise wie `$1` in der Ersetzung expandiert,
/// im Literalmodus bleibt die Ersetzung unverändert.
fn replace_all_matches(
    text: &str,
    pattern: &str,
    replacement: &str,
    opts: SearchOpts,
) -> Result<String, String> {
    if pattern.is_empty() {
        return Ok(text.to_string());
    }
    let re = build_search_regex(pattern, opts)?;
    let out = re.replace_all(text, |caps: &regex::Captures| {
        let m = caps.get(0).expect("Gruppe 0 existiert immer");
        if m.start() == m.end() {
            return String::new();
        }
        if opts.whole_word && !is_whole_word(text, m.start(), m.end()) {
            return m.as_str().to_string();
        }
        if opts.use_regex {
            let mut dst = String::new();
            caps.expand(replacement, &mut dst);
            dst
        } else {
            replacement.to_string()
        }
    });
    Ok(out.into_owned())
}

// --- Vorher-/Nachher-Vergleich ---

/// Art einer Zeile im Vergleich.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum LineChange {
    #[default]
    Equal,
    /// Zeile wurde geändert (auf beiden Seiten vorhanden).
    Changed,
    /// Zeile ist nur links vorhanden.
    Removed,
    /// Zeile ist nur rechts vorhanden.
    Added,
}

/// Ein zusammenhängender Änderungsblock.
///
/// `left_start` bzw. `right_start` sind 0-basierte Zeilenindizes. Bei reinen
/// Einfügungen ist `left_len` 0 und `left_start` die Einfügeposition.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ChangeBlock {
    left_start: usize,
    left_len: usize,
    right_start: usize,
    right_len: usize,
}

/// Ergebnis eines zeilenweisen Vergleichs.
#[derive(Clone, Debug, Default)]
struct DiffResult {
    /// Status je Zeile der linken Seite.
    left: Vec<LineChange>,
    /// Status je Zeile der rechten Seite.
    right: Vec<LineChange>,
    /// Änderungsblöcke in Dokumentreihenfolge.
    changes: Vec<ChangeBlock>,
}

impl DiffResult {
    fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// Höchstzahl an Zeilen, für die der vollständige Myers-Vergleich läuft.
///
/// Bei sehr großen Dokumenten wird zeilenweise positionsbasiert verglichen.
/// Das hält Speicherbedarf und Laufzeit in einem sicheren Rahmen.
const DIFF_MAX_LINES: usize = 4000;

/// Eine Operation des zeilenweisen Vergleichs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DiffOp {
    /// Beide Seiten haben an dieser Stelle dieselbe Zeile.
    Equal(usize, usize),
    /// Zeile `usize` existiert nur links.
    Delete(usize),
    /// Zeile `usize` existiert nur rechts.
    Insert(usize),
}

/// Vergleicht zwei Dokumente zeilenweise.
fn diff_lines(left: &str, right: &str) -> DiffResult {
    let a: Vec<&str> = left.split('\n').collect();
    let b: Vec<&str> = right.split('\n').collect();
    let ops = diff_ops(&a, &b);
    build_diff(&ops, a.len(), b.len())
}

/// Ermittelt die Vergleichsoperationen.
///
/// Gemeinsamer Anfang und gemeinsames Ende werden vorab abgeschnitten. Der
/// verbleibende Bereich wird mit dem Myers-Algorithmus verglichen; ist er zu
/// groß, kommt ein einfacher positionsbasierter Vergleich zum Einsatz.
fn diff_ops(a: &[&str], b: &[&str]) -> Vec<DiffOp> {
    let n = a.len();
    let m = b.len();

    let mut prefix = 0;
    while prefix < n && prefix < m && a[prefix] == b[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < n - prefix && suffix < m - prefix && a[n - 1 - suffix] == b[m - 1 - suffix] {
        suffix += 1;
    }

    let ac = &a[prefix..n - suffix];
    let bc = &b[prefix..m - suffix];
    let mut ops = Vec::with_capacity(n + m);
    for i in 0..prefix {
        ops.push(DiffOp::Equal(i, i));
    }

    if ac.len() + bc.len() > DIFF_MAX_LINES {
        // Positionsbasierter Ersatzvergleich für sehr große Dokumente.
        for i in 0..ac.len().max(bc.len()) {
            match (i < ac.len(), i < bc.len()) {
                (true, true) if ac[i] == bc[i] => {
                    ops.push(DiffOp::Equal(prefix + i, prefix + i));
                }
                (true, true) => {
                    ops.push(DiffOp::Delete(prefix + i));
                    ops.push(DiffOp::Insert(prefix + i));
                }
                (true, false) => ops.push(DiffOp::Delete(prefix + i)),
                (false, true) => ops.push(DiffOp::Insert(prefix + i)),
                (false, false) => {}
            }
        }
    } else {
        ops.extend(myers(ac, bc, prefix));
    }

    for i in 0..suffix {
        ops.push(DiffOp::Equal(n - suffix + i, m - suffix + i));
    }
    ops
}

/// Myers-Differenzalgorithmus mit Rückverfolgung.
///
/// `offset` wird auf alle Zeilenindizes addiert, damit der bereits
/// abgeschnittene gemeinsame Anfang berücksichtigt wird.
fn myers(a: &[&str], b: &[&str], offset: usize) -> Vec<DiffOp> {
    let n = a.len();
    let m = b.len();
    if n == 0 {
        return (0..m).map(|j| DiffOp::Insert(offset + j)).collect();
    }
    if m == 0 {
        return (0..n).map(|i| DiffOp::Delete(offset + i)).collect();
    }

    let max = n + m;
    let off = max as isize;
    let mut v = vec![0isize; 2 * max + 1];
    // `trace[d]` enthält die Werte des Durchgangs d-1 für |k| <= d-1.
    let mut trace: Vec<Vec<i32>> = Vec::new();
    let mut final_d = 0usize;

    'outer: for d in 0..=max {
        let dd = d as isize;
        if d == 0 {
            trace.push(vec![0]);
        } else {
            let span = (dd - 1) as usize;
            trace.push(
                v[(off as usize - span)..=(off as usize + span)]
                    .iter()
                    .map(|&x| x as i32)
                    .collect(),
            );
        }

        let mut k = -dd;
        while k <= dd {
            let mut x =
                if k == -dd || (k != dd && v[(k - 1 + off) as usize] < v[(k + 1 + off) as usize]) {
                    v[(k + 1 + off) as usize]
                } else {
                    v[(k - 1 + off) as usize] + 1
                };
            let mut y = x - k;
            while x < n as isize && y < m as isize && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[(k + off) as usize] = x;
            if x >= n as isize && y >= m as isize {
                final_d = d;
                break 'outer;
            }
            k += 2;
        }
    }

    // Rückverfolgung vom Endpunkt (n, m) zum Anfang.
    let mut x = n as isize;
    let mut y = m as isize;
    let mut rev = Vec::new();
    for d in (0..=final_d).rev() {
        let dd = d as isize;
        let k = x - y;
        let (prev_x, prev_y) = if d == 0 {
            (0, 0)
        } else {
            let span = (dd - 1) as usize;
            let get = |k: isize| trace[d][(k + span as isize) as usize] as isize;
            let prev_k = if k == -dd || (k != dd && get(k - 1) < get(k + 1)) {
                k + 1
            } else {
                k - 1
            };
            let px = get(prev_k);
            (px, px - prev_k)
        };

        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
            rev.push(DiffOp::Equal(offset + x as usize, offset + y as usize));
        }
        if d > 0 {
            if x == prev_x {
                y -= 1;
                rev.push(DiffOp::Insert(offset + y as usize));
            } else {
                x -= 1;
                rev.push(DiffOp::Delete(offset + x as usize));
            }
        }
    }
    rev.reverse();
    rev
}

/// Wandelt Vergleichsoperationen in Zeilenstatus und Änderungsblöcke um.
fn build_diff(ops: &[DiffOp], left_lines: usize, right_lines: usize) -> DiffResult {
    let mut diff = DiffResult {
        left: vec![LineChange::Equal; left_lines],
        right: vec![LineChange::Equal; right_lines],
        changes: Vec::new(),
    };
    let mut li = 0;
    let mut ri = 0;
    let mut i = 0;
    while i < ops.len() {
        if matches!(ops[i], DiffOp::Equal(..)) {
            li += 1;
            ri += 1;
            i += 1;
            continue;
        }

        let left_start = li;
        let right_start = ri;
        let mut deleted = 0;
        let mut inserted = 0;
        while i < ops.len() && !matches!(ops[i], DiffOp::Equal(..)) {
            match ops[i] {
                DiffOp::Delete(_) => {
                    diff.left[li] = LineChange::Removed;
                    li += 1;
                    deleted += 1;
                }
                DiffOp::Insert(_) => {
                    diff.right[ri] = LineChange::Added;
                    ri += 1;
                    inserted += 1;
                }
                DiffOp::Equal(..) => unreachable!("Schleifenbedingung schließt Equal aus"),
            }
            i += 1;
        }

        // Gegenüberliegende Zeilen eines Blocks gelten als geändert.
        for k in 0..deleted.min(inserted) {
            diff.left[left_start + k] = LineChange::Changed;
            diff.right[right_start + k] = LineChange::Changed;
        }
        diff.changes.push(ChangeBlock {
            left_start,
            left_len: deleted,
            right_start,
            right_len: inserted,
        });
    }
    diff
}

/// Hintergrundfarbe für den Zeilenstatus im Vergleich.
fn diff_bg(change: LineChange) -> Color32 {
    match change {
        LineChange::Changed => Color32::from_rgba_unmultiplied(255, 190, 0, 55),
        LineChange::Removed => Color32::from_rgba_unmultiplied(255, 80, 80, 55),
        LineChange::Added => Color32::from_rgba_unmultiplied(70, 200, 90, 55),
        LineChange::Equal => Color32::TRANSPARENT,
    }
}

/// Kräftigere Farbe für die Legende.
fn diff_swatch(change: LineChange) -> Color32 {
    match change {
        LineChange::Changed => Color32::from_rgb(214, 160, 0),
        LineChange::Removed => Color32::from_rgb(214, 70, 70),
        LineChange::Added => Color32::from_rgb(60, 170, 80),
        LineChange::Equal => Color32::TRANSPARENT,
    }
}

/// Beschriftung für eine Zeilenzahl.
fn lines_text(n: usize) -> String {
    if n == 1 {
        "1 Zeile".to_owned()
    } else {
        format!("{n} Zeilen")
    }
}

/// Beschriftung für eine Anzahl Änderungen.
fn changes_text(n: usize) -> String {
    if n == 1 {
        "1 Änderung".to_owned()
    } else {
        format!("{n} Änderungen")
    }
}

/// Beschriftung für eine Anzahl Änderungsblöcke.
fn blocks_text(n: usize) -> String {
    if n == 1 {
        "1 Änderungsblock".to_owned()
    } else {
        format!("{n} Änderungsblöcke")
    }
}

/// Zwischenspeicher für die Syntax-Hervorhebung einer Ansicht.
#[derive(Default)]
struct HighlightCache {
    job: Option<egui::text::LayoutJob>,
    fmt: Fmt,
    dark: bool,
    font: Option<egui::FontId>,
}

impl HighlightCache {
    /// Liefert den Hervorhebungsjob und baut ihn bei Bedarf neu auf.
    fn job(&mut self, ui: &egui::Ui, text: &str, fmt: Fmt) -> egui::text::LayoutJob {
        let dark = ui.visuals().dark_mode;
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let valid = self.job.as_ref().is_some_and(|j| j.text.as_str() == text)
            && self.fmt == fmt
            && self.dark == dark
            && self.font.as_ref() == Some(&font);
        if !valid {
            self.job = Some(highlight::layout_job(
                fmt,
                text,
                font.clone(),
                dark,
                f32::INFINITY,
            ));
            self.fmt = fmt;
            self.dark = dark;
            self.font = Some(font);
        }
        self.job
            .clone()
            .expect("Hervorhebungsjob wurde gerade gesetzt")
    }
}

/// Zustand der geteilten Vergleichsansicht.
struct CompareState {
    /// Linke Seite (Alt). Sie ist ein Vergleichsstand und wird nicht gespeichert.
    left: String,
    left_hl: HighlightCache,
    right_hl: HighlightCache,
    diff: DiffResult,
    diff_dirty: bool,
    /// `true`: Alt und Neu nebeneinander, `false`: untereinander.
    side_by_side: bool,
    /// Gemeinsamer senkrechter Scroll-Offset beider Ansichten.
    scroll: f32,
    /// Aktuell angesteuerter Änderungsblock.
    change_idx: Option<usize>,
    /// Beim nächsten Zeichnen zum Änderungsblock springen.
    jump: bool,
}

impl CompareState {
    fn new(left: String, side_by_side: bool) -> Self {
        Self {
            left,
            left_hl: HighlightCache::default(),
            right_hl: HighlightCache::default(),
            diff: DiffResult::default(),
            diff_dirty: true,
            side_by_side,
            scroll: 0.0,
            change_idx: None,
            jump: false,
        }
    }
}

/// Ergebnis einer gezeichneten Vergleichsansicht.
struct PaneOutput {
    id: egui::Id,
    offset: f32,
    hovered: bool,
    cursor: usize,
    changed: bool,
}

/// Zeichnet eine Seite der Vergleichsansicht.
///
/// Die Zeilennummern stehen links, danach folgt der Editor. Geänderte Zeilen
/// werden nach dem Zeichnen mit einer halbtransparenten Farbe hinterlegt, damit
/// der Text gut lesbar bleibt.
#[allow(clippy::too_many_arguments)]
fn compare_pane(
    ui: &mut egui::Ui,
    title: &str,
    text: &mut String,
    status: &[LineChange],
    fmt: Fmt,
    cache: &mut HighlightCache,
    scroll: f32,
    editable: bool,
    restore_cursor: bool,
    cursor: usize,
    jump_line: Option<usize>,
    matches: &[(usize, usize)],
    current_match: Option<usize>,
) -> PaneOutput {
    let mut out = PaneOutput {
        id: egui::Id::NULL,
        offset: scroll,
        hovered: false,
        cursor,
        changed: false,
    };
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.strong(title);
            ui.label(
                egui::RichText::new(lines_text(text.split('\n').count()))
                    .weak()
                    .small(),
            );
        });

        let area = egui::ScrollArea::both()
            .id_salt(("vergleich", title))
            .auto_shrink([false, false])
            .vertical_scroll_offset(scroll)
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    let lines = text.split('\n').count().max(1);
                    let nums = (1..=lines)
                        .map(|n| n.to_string())
                        .collect::<Vec<_>>()
                        .join("\n");
                    ui.label(
                        egui::RichText::new(nums)
                            .monospace()
                            .color(ui.visuals().weak_text_color()),
                    );

                    let cached = cache.job(ui, text, fmt);
                    let hl_fmt = fmt;
                    let hl_dark = ui.visuals().dark_mode;
                    let hl_font = egui::TextStyle::Monospace.resolve(ui.style());
                    let mut layouter = move |ui: &egui::Ui, text: &str, _wrap_width: f32| {
                        let job = if cached.text.as_str() == text {
                            cached.clone()
                        } else {
                            highlight::layout_job(
                                hl_fmt,
                                text,
                                hl_font.clone(),
                                hl_dark,
                                f32::INFINITY,
                            )
                        };
                        // Im Vergleich wird nicht umbrochen, damit jede
                        // Zeile genau einer Bildschirmzeile entspricht.
                        ui.fonts(|f| f.layout_job(job))
                    };
                    let mut edit = egui::TextEdit::multiline(text)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(30)
                        .layouter(&mut layouter);
                    if !editable {
                        edit = edit.interactive(false);
                    }
                    let mut te = edit.show(ui);
                    out.id = te.response.id;
                    out.changed = te.response.changed();

                    if editable && restore_cursor {
                        let chars = text.chars().count();
                        let idx = cursor.min(chars);
                        te.state
                            .cursor
                            .set_char_range(Some(CCursorRange::one(CCursor::new(idx))));
                        te.state.store(ui.ctx(), te.response.id);
                        out.cursor = idx;
                    } else if let Some(range) = te.cursor_range {
                        out.cursor = range.primary.ccursor.index;
                    }

                    let rows = te.galley.rows.len().max(1);
                    let lh = te.galley.size().y / rows as f32;
                    let row_rect = |line: usize| {
                        let y = te.galley_pos.y + line as f32 * lh;
                        egui::Rect::from_x_y_ranges(ui.clip_rect().x_range(), y..=y + lh)
                    };

                    // Farbige Hinterlegung geänderter Zeilen.
                    for (i, change) in status.iter().enumerate() {
                        if *change == LineChange::Equal || i >= rows {
                            continue;
                        }
                        ui.painter().rect_filled(row_rect(i), 0.0, diff_bg(*change));
                    }

                    // Suchtreffer nur in der bearbeitbaren Ansicht hervorheben.
                    let mut last_b = 0;
                    let mut last_c = 0;
                    for (i, &(s, e)) in matches.iter().enumerate() {
                        if e > text.len()
                            || s > e
                            || !text.is_char_boundary(s)
                            || !text.is_char_boundary(e)
                        {
                            continue;
                        }
                        let cs = last_c + text[last_b..s].chars().count();
                        let ce = cs + text[s..e].chars().count();
                        last_b = e;
                        last_c = ce;
                        let a = te.galley.pos_from_ccursor(egui::text::CCursor::new(cs));
                        let b = te.galley.pos_from_ccursor(egui::text::CCursor::new(ce));
                        let rect = egui::Rect::from_min_max(
                            te.galley_pos + a.min.to_vec2(),
                            te.galley_pos + egui::vec2(b.max.x, a.max.y),
                        );
                        let color = if Some(i) == current_match {
                            Color32::from_rgba_unmultiplied(255, 190, 0, 150)
                        } else {
                            Color32::from_rgba_unmultiplied(255, 225, 120, 90)
                        };
                        ui.painter().rect_filled(rect, 2.0, color);
                    }

                    if let Some(line) = jump_line {
                        let line = line.min(rows - 1);
                        ui.scroll_to_rect(row_rect(line), Some(egui::Align::Center));
                    }
                });
            });

        out.offset = area.state.offset.y;
        out.hovered = ui.rect_contains_pointer(area.inner_rect.expand(6.0));
    });
    out
}

/// Größe eines Ribbon-Knopfes (Symbol über Beschriftung).
const RIBBON_BUTTON_SIZE: egui::Vec2 = egui::vec2(62.0, 46.0);

/// Abstand zwischen zwei Knöpfen einer Gruppe.
const RIBBON_BUTTON_SPACING: f32 = 2.0;

/// Zusätzlicher Abstand zwischen zwei Gruppen.
const RIBBON_GROUP_SPACING: f32 = 10.0;

/// Breite einer Gruppe mit `buttons` Knöpfen.
fn ribbon_group_width(buttons: usize) -> f32 {
    let n = buttons.max(1) as f32;
    n * RIBBON_BUTTON_SIZE.x + (n - 1.0) * RIBBON_BUTTON_SPACING
}

/// Zeichnet eine Themengruppe im Ribbon und beschriftet sie unten.
///
/// Die Gruppe reserviert ihre volle Breite vorab. Nur so erkennt das
/// umbrechende Layout die tatsächliche Größe und bricht bei schmalen
/// Fenstern in die nächste Zeile um, statt den Inhalt abzuschneiden.
fn ribbon_group(ui: &mut egui::Ui, title: &str, buttons: usize, add: impl FnOnce(&mut egui::Ui)) {
    let label_font = egui::FontId::proportional(10.0);
    let label_height = ui.fonts(|f| f.row_height(&label_font));
    let size = egui::vec2(
        ribbon_group_width(buttons),
        RIBBON_BUTTON_SIZE.y + 2.0 + label_height,
    );
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let mut group = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    group.spacing_mut().item_spacing = egui::vec2(RIBBON_BUTTON_SPACING, 1.0);
    group.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = RIBBON_BUTTON_SPACING;
        add(ui);
    });
    group.label(
        egui::RichText::new(title)
            .font(label_font)
            .color(group.visuals().weak_text_color()),
    );
    ui.add_space(RIBBON_GROUP_SPACING);
}

/// Ein Ribbon-Knopf mit Symbol und Beschriftung.
fn ribbon_button(ui: &mut egui::Ui, icon: Icon, label: &str, enabled: bool, tooltip: &str) -> bool {
    ribbon_button_impl(ui, icon, label, enabled, false, tooltip)
}

/// Ein Ribbon-Knopf, der einen aktiven Zustand hervorhebt.
fn ribbon_toggle(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    selected: bool,
    tooltip: &str,
) -> bool {
    ribbon_button_impl(ui, icon, label, true, selected, tooltip)
}

/// Verkleinert eine Knopfbeschriftung so weit, dass sie in die Knopfbreite passt.
fn ribbon_label_font(ui: &egui::Ui, label: &str, max_width: f32, color: Color32) -> egui::FontId {
    let mut size = 10.5_f32;
    loop {
        let font = egui::FontId::proportional(size);
        let width = ui.fonts(|f| {
            f.layout_no_wrap(label.to_owned(), font.clone(), color)
                .size()
                .x
        });
        if width <= max_width || size <= 8.0 {
            return font;
        }
        size -= 0.5;
    }
}

fn ribbon_button_impl(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    enabled: bool,
    selected: bool,
    tooltip: &str,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(RIBBON_BUTTON_SIZE, egui::Sense::click());
    let visuals = ui.visuals();
    if selected {
        ui.painter()
            .rect_filled(rect, 4.0, visuals.selection.bg_fill.gamma_multiply(0.35));
    } else if enabled && response.hovered() {
        ui.painter()
            .rect_filled(rect, 4.0, visuals.widgets.hovered.weak_bg_fill);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            4.0,
            visuals.selection.stroke,
            egui::StrokeKind::Inside,
        );
    }

    let color = if enabled {
        visuals.text_color()
    } else {
        visuals.weak_text_color()
    };
    let icon_rect = egui::Rect::from_center_size(
        rect.center_top() + egui::vec2(0.0, 14.0),
        egui::vec2(20.0, 20.0),
    );
    icons::draw(ui.painter(), icon_rect, icon, color);
    let font = ribbon_label_font(ui, label, rect.width() - 4.0, color);
    ui.painter().text(
        rect.center_bottom() - egui::vec2(0.0, 8.5),
        egui::Align2::CENTER_CENTER,
        label,
        font,
        color,
    );

    let response = response.on_hover_text(tooltip);
    enabled && response.clicked()
}

#[derive(Default)]
struct App {
    text: String,
    fmt: Fmt,
    msg: String,
    path: Option<PathBuf>,
    // Fehleranzeige
    err_pos: Option<Pos>,
    jump: bool,
    // Suchen/Ersetzen
    find_open: bool,
    focus_find: bool,
    focus_replace: bool,
    find: String,
    replace: String,
    case_sensitive: bool,
    whole_word: bool,
    use_regex: bool,
    match_range: Option<(usize, usize)>, // Byte-Bereich des aktuellen Treffers
    matches: Vec<(usize, usize)>,        // alle Treffer (Byte-Bereiche)
    match_idx: Option<usize>,
    search_error: Option<String>,
    search_dirty: bool,
    scroll_line: Option<usize>,
    // Undo / Redo
    history: History,
    cursor: usize, // Cursorposition als Zeichenindex
    last_edit: f64,
    restore_cursor: bool,
    /// Auswahl, die nach dem Zeichnen gesetzt werden soll (Zeichenindizes).
    restore_sel: Option<(usize, usize)>,
    // Syntax-Hervorhebung
    hl: Option<egui::text::LayoutJob>,
    hl_fmt: Fmt,
    hl_dark: bool,
    hl_font: Option<egui::FontId>,
    editor_id: Option<egui::Id>,
    // Dateiüberwachung
    disk: Option<DiskState>,
    saved_text: String,
    external_change: bool,
    last_poll: f64,
    title: String,
    confirm_close: bool,
    allow_close: bool,
    // Formatierungsprofile
    profiles: Profiles,
    profiles_path: Option<PathBuf>,
    format_window: bool,
    new_profile_name: String,
    profile_msg: String,
    // Ribbon-Oberfläche
    ribbon_tab: RibbonTab,
    ribbon_collapsed: bool,
    hide_status: bool,
    // Darstellung
    settings: Settings,
    settings_path: Option<PathBuf>,
    settings_window: bool,
    settings_msg: String,
    fonts: Vec<(String, PathBuf)>,
    applied_font: String,
    // Zuletzt verwendet, Drag & Drop, Wiederherstellung
    recent_open: bool,
    pending_open: Option<PathBuf>,
    recovery: Option<Recovery>,
    recovery_path: Option<PathBuf>,
    last_autosave: f64,
    // Vorher-/Nachher-Vergleich
    compare: Option<CompareState>,
    // Formatkonvertierung
    convert_window: bool,
    convert_target: Fmt,
    convert_compare: bool,
    convert_error: Option<String>,
}

impl App {
    /// Setzt bei Enter einen Zeilenumbruch mit passender Einrückung ein.
    ///
    /// egui fügt standardmäßig nur `\n` ein, deshalb wird die Enter-Taste vor
    /// dem Editor abgefangen und der Umbruch selbst eingefügt.
    fn auto_indent(&mut self, ctx: &egui::Context, now: f64) -> bool {
        if !self.editor_has_focus(ctx) {
            return false;
        }
        if !ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
            return false;
        }

        let (start, end) = self.selection_chars(ctx);
        let start_b = byte_index(&self.text, start);
        let end_b = byte_index(&self.text, end);
        let insert = newline_with_indent(self.fmt, &self.text, start_b);
        self.text.replace_range(start_b..end_b, &insert);

        self.cursor = start + insert.chars().count();
        self.after_edit(now);
        true
    }

    fn editor_has_focus(&self, ctx: &egui::Context) -> bool {
        self.editor_id
            .is_some_and(|id| ctx.memory(|m| m.has_focus(id)))
    }

    /// Liefert die Auswahl als Zeichenindizes (Start, Ende).
    fn selection_chars(&self, ctx: &egui::Context) -> (usize, usize) {
        let range = self
            .editor_id
            .and_then(|id| egui::text_edit::TextEditState::load(ctx, id))
            .and_then(|s| s.cursor.char_range());
        let chars = self.text.chars().count();
        let (start, end) = match range {
            Some(r) => (
                r.primary.index.min(r.secondary.index),
                r.primary.index.max(r.secondary.index),
            ),
            None => (self.cursor, self.cursor),
        };
        let start = start.min(chars);
        (start, end.min(chars).max(start))
    }

    fn after_edit(&mut self, now: f64) {
        self.restore_cursor = true;
        self.last_edit = now;
        self.err_pos = None;
        self.match_range = None;
        self.search_dirty = true;
        self.mark_compare_dirty();
    }

    /// Ergänzt Klammern, Anführungszeichen und XML-Tags beim Tippen,
    /// überspringt vorhandene schließende Zeichen und entfernt leere Paare
    /// beim Löschen.
    fn handle_typing(&mut self, ctx: &egui::Context, now: f64) -> bool {
        if !self.editor_has_focus(ctx) {
            return false;
        }
        let Some(trigger) = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Text(t) => {
                    let mut it = t.chars();
                    match (it.next(), it.next()) {
                        (Some(c), None) if is_completion_char(self.fmt, c) => Some(Typing::Char(c)),
                        _ => None,
                    }
                }
                egui::Event::Key {
                    key: Key::Backspace,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.is_none() => Some(Typing::Backspace),
                _ => None,
            })
        }) else {
            return false;
        };

        let (start, end) = self.selection_chars(ctx);
        let result = match trigger {
            Typing::Char(ch) => apply_char(self.fmt, &self.text, start, end, ch),
            Typing::Backspace => apply_backspace(self.fmt, &self.text, start, end),
        };
        let Some(result) = result else {
            return false;
        };

        self.text = result.text;
        self.cursor = result.cursor;
        match trigger {
            Typing::Char(ch) => self.consume_text(ctx, ch),
            Typing::Backspace => self.consume_backspace(ctx),
        }
        self.after_edit(now);
        true
    }

    fn consume_text(&self, ctx: &egui::Context, c: char) {
        ctx.input_mut(|i| {
            if let Some(p) = i.events.iter().position(
                |e| matches!(e, egui::Event::Text(t) if t.chars().count() == 1 && t.starts_with(c)),
            ) {
                i.events.remove(p);
            }
        });
    }

    fn consume_backspace(&self, ctx: &egui::Context) {
        ctx.input_mut(|i| {
            if let Some(p) = i.events.iter().position(|e| {
                matches!(
                    e,
                    egui::Event::Key {
                        key: Key::Backspace,
                        pressed: true,
                        ..
                    }
                )
            }) {
                i.events.remove(p);
            }
        });
    }

    /// Merkt eine Auswahl, die nach dem Zeichnen des Editors gesetzt wird.
    fn set_selection(&mut self, start: usize, end: usize) {
        self.cursor = end;
        self.restore_sel = Some((start, end));
        self.restore_cursor = true;
    }

    /// Zeilenbezogene Bearbeitung:
    ///
    /// - `Strg+L` markiert die aktuelle Zeile.
    /// - `Alt+↑` / `Alt+↓` verschieben die markierten Zeilen.
    /// - `Strg+C` ohne Auswahl kopiert die aktuelle Zeile.
    fn handle_line_ops(&mut self, ctx: &egui::Context, now: f64) -> bool {
        if !self.editor_has_focus(ctx) {
            return false;
        }
        let (start, end) = self.selection_chars(ctx);

        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::L)) {
            let (s, e) = select_line(&self.text, self.cursor);
            self.set_selection(s, e);
            return true;
        }

        let up = ctx.input_mut(|i| i.consume_key(Modifiers::ALT, Key::ArrowUp));
        let down = ctx.input_mut(|i| i.consume_key(Modifiers::ALT, Key::ArrowDown));
        if up || down {
            if let Some(m) = move_lines(&self.text, start, end, up) {
                self.commit_undo();
                self.text = m.text;
                self.commit_undo();
                self.set_selection(m.sel.0, m.sel.1);
                self.after_edit(now);
            }
            return true;
        }

        // Ohne Auswahl kopiert Strg+C die aktuelle Zeile.
        if start == end && consume_copy_event(ctx) {
            ctx.copy_text(current_line_text(&self.text, self.cursor));
            self.msg = "Zeile kopiert.".into();
            return true;
        }
        false
    }

    /// Baut den hervorgehobenen Layout-Job neu auf, wenn sich Text, Format,
    /// Farbschema oder Schriftart geändert haben.
    fn update_highlight(&mut self, ui: &egui::Ui) {
        let dark = ui.visuals().dark_mode;
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let valid = self
            .hl
            .as_ref()
            .is_some_and(|j| j.text.as_str() == self.text)
            && self.hl_fmt == self.fmt
            && self.hl_dark == dark
            && self.hl_font.as_ref() == Some(&font);
        if valid {
            return;
        }
        self.hl = Some(highlight::layout_job(
            self.fmt,
            &self.text,
            font.clone(),
            dark,
            f32::INFINITY,
        ));
        self.hl_fmt = self.fmt;
        self.hl_dark = dark;
        self.hl_font = Some(font);
    }

    /// Übernimmt den aktuellen Text in die Undo-Historie.
    fn commit_undo(&mut self) {
        self.history.record(&self.text, self.cursor);
    }

    fn apply_snapshot(&mut self, s: Snapshot) {
        self.text = s.text;
        self.cursor = s.cursor;
        self.restore_cursor = true;
        self.err_pos = None;
        self.match_range = None;
        self.search_dirty = true;
        self.mark_compare_dirty();
        self.msg.clear();
    }

    fn undo(&mut self) {
        if let Some(s) = self.history.undo(&self.text, self.cursor) {
            self.apply_snapshot(s);
        }
    }

    fn redo(&mut self) {
        if let Some(s) = self.history.redo(&self.text, self.cursor) {
            self.apply_snapshot(s);
        }
    }

    /// Formatiert den aktuellen Inhalt, ohne ihn zu übernehmen.
    fn formatted_text(&self) -> Result<String, FmtErr> {
        let opts = self.profiles.options();
        match self.fmt {
            Fmt::Json => format_json(&self.text, &opts),
            Fmt::Xml => format_xml(&self.text, &opts),
            Fmt::Yaml => format_yaml(&self.text, &opts),
        }
    }

    /// Zeigt einen Formatierungsfehler an und springt zur Fehlerstelle.
    fn show_format_error(&mut self, e: FmtErr) {
        self.err_pos = e.pos;
        self.jump = e.pos.is_some();
        self.msg = match e.pos {
            Some(p) => format!("Fehler: {} (Zeile {}, Spalte {})", e.msg, p.line, p.col),
            None => format!("Fehler: {}", e.msg),
        };
        // Cursor an die Fehlerstelle setzen.
        if let Some(p) = e.pos {
            self.cursor = char_index_at(&self.text, p);
            self.restore_cursor = true;
        }
    }

    /// Markiert den Vergleich als veraltet, nachdem sich der rechte Text geändert hat.
    fn mark_compare_dirty(&mut self) {
        if let Some(cmp) = self.compare.as_mut() {
            cmp.diff_dirty = true;
        }
    }

    fn format(&mut self) {
        self.commit_undo();
        match self.formatted_text() {
            Ok(s) => {
                self.text = s;
                self.msg = "OK".into();
                self.err_pos = None;
                self.match_range = None;
                self.search_dirty = true;
                self.mark_compare_dirty();
                self.commit_undo();
            }
            Err(e) => self.show_format_error(e),
        }
    }

    /// Startet einen Vorher-/Nachher-Vergleich: links der aktuelle Inhalt,
    /// rechts das formatierte Ergebnis.
    fn start_compare_format(&mut self) {
        let original = self.text.clone();
        match self.formatted_text() {
            Ok(formatted) => {
                if formatted != self.text {
                    self.commit_undo();
                    self.text = formatted;
                    self.commit_undo();
                }
                self.err_pos = None;
                self.match_range = None;
                self.search_dirty = true;
                self.open_compare(original);
                self.msg =
                    "Vergleich: links der Ausgangsstand, rechts das formatierte Ergebnis.".into();
            }
            Err(e) => self.show_format_error(e),
        }
    }

    /// Startet einen Vergleich mit dem zuletzt gespeicherten Dateistand.
    fn start_compare_saved(&mut self) {
        if self.saved_text.is_empty() && self.path.is_none() {
            self.msg = "Es gibt noch keinen gespeicherten Dateistand zum Vergleichen.".into();
            return;
        }
        let saved = self.saved_text.clone();
        self.open_compare(saved);
        self.msg = "Vergleich: links der gespeicherte Stand, rechts der aktuelle Inhalt.".into();
    }

    /// Wandelt das aktuelle Dokument in das Zielformat um.
    ///
    /// Die Syntaxprüfung des Quelldokuments findet dabei über dessen Parser
    /// statt; Fehler werden mit Zeilen- und Spaltenangabe gemeldet.
    fn convert_document(&self, target: Fmt) -> Result<String, FmtErr> {
        let value = convert::parse(self.fmt, &self.text)?;
        convert::serialize(target, &value, &self.profiles.options())
    }

    /// Übernimmt eine Konvertierung in den Editor, inklusive Undo-Schritt.
    fn apply_conversion(&mut self, target: Fmt, compare: bool) {
        let source = self.fmt;
        let original = self.text.clone();
        match self.convert_document(target) {
            Ok(text) => {
                self.commit_undo();
                self.text = text;
                self.fmt = target;
                self.err_pos = None;
                self.match_range = None;
                self.search_dirty = true;
                self.msg = format!("Konvertiert: {} → {}", source.name(), target.name());
                if compare {
                    self.open_compare(original);
                }
                self.commit_undo();
            }
            Err(e) => {
                self.convert_error = Some(e.msg.clone());
                self.show_format_error(e);
            }
        }
    }

    /// Konvertiert das Dokument und schreibt das Ergebnis in eine neue Datei.
    fn save_conversion(&mut self, target: Fmt) {
        match self.convert_document(target) {
            Ok(text) => {
                let ext = match target {
                    Fmt::Json => "json",
                    Fmt::Xml => "xml",
                    Fmt::Yaml => "yaml",
                };
                let stem = self
                    .path
                    .as_ref()
                    .and_then(|p| p.file_stem().and_then(|s| s.to_str()))
                    .unwrap_or("Dokument");
                let name = format!("{stem}.{ext}");
                let Some(dest) = rfd::FileDialog::new().set_file_name(&name).save_file() else {
                    return;
                };
                match std::fs::write(&dest, &text) {
                    Ok(_) => {
                        self.msg = format!("Konvertiert gespeichert: {}", dest.display());
                        self.settings.remember_file(&dest);
                        self.persist_settings();
                    }
                    Err(e) => self.msg = format!("Speichern fehlgeschlagen: {e}"),
                }
            }
            Err(e) => {
                self.convert_error = Some(e.msg.clone());
                self.show_format_error(e);
            }
        }
    }

    /// Öffnet die geteilte Vergleichsansicht mit dem angegebenen linken Inhalt.
    fn open_compare(&mut self, left: String) {
        let mut cmp = CompareState::new(left, self.settings.compare_side_by_side);
        cmp.diff = diff_lines(&cmp.left, &self.text);
        cmp.diff_dirty = false;
        cmp.change_idx = if cmp.diff.is_empty() { None } else { Some(0) };
        cmp.jump = !cmp.diff.is_empty();
        self.compare = Some(cmp);
        self.ribbon_tab = RibbonTab::Vergleichen;
        self.ribbon_collapsed = false;
        // Cursor der rechten Ansicht an der bisherigen Position halten.
        self.restore_cursor = true;
    }

    /// Schließt die Vergleichsansicht; der rechte Inhalt bleibt erhalten.
    fn close_compare(&mut self) {
        if self.compare.take().is_some() {
            self.msg = "Vergleich beendet.".into();
        }
    }

    /// Wechselt zwischen nebeneinander und untereinander.
    fn set_compare_layout(&mut self, side_by_side: bool) {
        self.settings.compare_side_by_side = side_by_side;
        if let Some(cmp) = self.compare.as_mut() {
            cmp.side_by_side = side_by_side;
        }
        self.persist_settings();
    }

    /// Springt zum nächsten oder vorherigen Änderungsblock.
    fn compare_step(&mut self, forward: bool) {
        let Some(cmp) = self.compare.as_mut() else {
            return;
        };
        let n = cmp.diff.changes.len();
        if n == 0 {
            cmp.change_idx = None;
            return;
        }
        let idx = match (cmp.change_idx, forward) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
        };
        cmp.change_idx = Some(idx);
        cmp.jump = true;
    }

    fn open(&mut self) {
        let Some(p) = rfd::FileDialog::new().pick_file() else {
            return;
        };
        self.request_open(p);
    }

    /// Öffnet eine Datei; bei ungespeicherten Änderungen wird erst gefragt.
    fn request_open(&mut self, path: PathBuf) {
        if self.is_modified() {
            self.pending_open = Some(path);
        } else {
            self.open_path(path);
        }
    }

    /// Lädt die angegebene Datei und merkt sie in der Zuletzt-Liste.
    fn open_path(&mut self, path: PathBuf) {
        self.pending_open = None;
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                self.set_loaded(path, text);
                self.remember_recent();
            }
            Err(e) => self.msg = format!("Öffnen fehlgeschlagen: {e}"),
        }
    }

    /// Merkt die aktuelle Datei in der Zuletzt-Liste.
    fn remember_recent(&mut self) {
        if let Some(path) = self.path.clone() {
            self.settings.remember_file(&path);
            self.persist_settings();
        }
    }

    /// Löscht einen vorhandenen Zwischenstand.
    fn discard_recovery(&mut self) {
        self.recovery = None;
        if let Some(path) = self.recovery_path.clone() {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Stellt den gefundenen Zwischenstand wieder her.
    fn restore_recovery(&mut self) {
        let Some(rec) = self.recovery.take() else {
            return;
        };
        let path = rec.path.clone().map(PathBuf::from);
        self.fmt = path
            .as_ref()
            .map(|p| Self::fmt_for_path(p, rec.fmt))
            .unwrap_or(rec.fmt);
        // Gespeicherter Stand bleibt die Datei auf dem Datenträger.
        self.saved_text = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default();
        self.text = rec.text;
        self.history = History::new(&self.text, 0);
        self.cursor = 0;
        self.restore_cursor = true;
        self.err_pos = None;
        self.match_range = None;
        self.search_dirty = true;
        self.jump = false;
        self.disk = path.as_ref().and_then(|p| DiskState::of(p));
        self.external_change = false;
        self.path = path;
        self.last_edit = 0.0;
        self.msg = "Nicht gespeicherte Änderungen wiederhergestellt.".into();
        if let Some(p) = self.recovery_path.clone() {
            let _ = std::fs::remove_file(p);
        }
    }

    /// Schreibt ungespeicherte Änderungen regelmäßig als Zwischenstand.
    fn autosave(&mut self, now: f64) {
        const INTERVAL_SECS: f64 = 2.0;
        // Erst entscheiden lassen, ob ein alter Zwischenstand übernommen wird.
        if self.recovery.is_some() {
            return;
        }
        let Some(path) = self.recovery_path.clone() else {
            return;
        };
        if self.is_modified() {
            if now - self.last_autosave < INTERVAL_SECS {
                return;
            }
            self.last_autosave = now;
            let recovery = Recovery {
                path: self.path.as_ref().map(|p| p.to_string_lossy().to_string()),
                fmt: self.fmt,
                text: self.text.clone(),
            };
            let _ = save_recovery(&path, &recovery);
        } else if path.is_file() {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Lädt die Datei erneut vom Datenträger (externe Änderungen übernehmen).
    fn reload(&mut self) {
        let Some(p) = self.path.clone() else { return };
        match std::fs::read_to_string(&p) {
            Ok(t) => {
                self.set_loaded(p, t);
                self.msg = "Datei neu geladen.".into();
            }
            Err(e) => self.msg = format!("Neu laden fehlgeschlagen: {e}"),
        }
    }

    fn save(&mut self) {
        if self.external_change {
            self.msg =
                "Datei wurde außerhalb von Parsix geändert – bitte neu laden oder bewusst überschreiben."
                    .into();
            return;
        }
        if self.path.is_none() {
            self.path = rfd::FileDialog::new().save_file();
        }
        let Some(p) = self.path.clone() else { return };
        match std::fs::write(&p, &self.text) {
            Ok(_) => {
                self.msg = format!("Gespeichert: {}", p.display());
                self.saved_text = self.text.clone();
                self.disk = DiskState::of(&p);
                self.external_change = false;
                self.remember_recent();
                self.discard_recovery();
            }
            Err(e) => self.msg = format!("Speichern fehlgeschlagen: {e}"),
        }
    }

    /// Speichert trotz erkannter externer Änderung und überschreibt die Datei.
    fn overwrite(&mut self) {
        self.external_change = false;
        self.save();
    }

    /// Übernimmt einen neu geladenen Dateiinhalt und setzt den Zustand zurück.
    fn set_loaded(&mut self, path: PathBuf, text: String) {
        self.fmt = Self::fmt_for_path(&path, self.fmt);
        self.text = text;
        self.saved_text = self.text.clone();
        // Neue Datei = neue Undo-Historie.
        self.history = History::new(&self.text, 0);
        self.cursor = 0;
        self.restore_cursor = true;
        self.err_pos = None;
        self.match_range = None;
        self.search_dirty = true;
        self.jump = false;
        self.msg.clear();
        self.disk = DiskState::of(&path);
        self.external_change = false;
        self.last_edit = 0.0;
        self.last_autosave = 0.0;
        self.path = Some(path);
        // Ein alter Zwischenstand gehört zu einer anderen Datei.
        self.discard_recovery();
        // Eine neue Datei beendet den Vergleich.
        self.compare = None;
    }

    fn fmt_for_path(path: &Path, current: Fmt) -> Fmt {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("json") => Fmt::Json,
            Some("xml") => Fmt::Xml,
            Some("yaml" | "yml") => Fmt::Yaml,
            _ => current,
        }
    }

    /// Gibt es ungespeicherte Änderungen?
    fn is_modified(&self) -> bool {
        self.text != self.saved_text
    }

    fn window_title(&self) -> String {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("Unbenannt");
        if self.is_modified() {
            format!("Parsix – {name} *")
        } else {
            format!("Parsix – {name}")
        }
    }

    /// Prüft regelmäßig, ob die Datei außerhalb von Parsix verändert wurde.
    fn check_disk(&mut self, ctx: &egui::Context, now: f64) {
        let Some(path) = self.path.clone() else {
            return;
        };
        ctx.request_repaint_after(Duration::from_secs(1));
        if now - self.last_poll < 1.0 {
            return;
        }
        self.last_poll = now;
        let Some(expected) = self.disk else { return };
        let current = DiskState::of(&path);
        if current != Some(expected) {
            self.external_change = true;
            // Neue Ausgangslage, damit dieselbe Änderung nur einmal gemeldet wird.
            self.disk = current;
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = self.window_title();
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    /// Verhindert das Schließen mit ungespeicherten Änderungen.
    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && self.is_modified() && !self.allow_close
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close = true;
        }
    }

    fn search_opts(&self) -> SearchOpts {
        SearchOpts {
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            use_regex: self.use_regex,
        }
    }

    /// Berechnet alle Treffer neu, wenn sich Text, Muster oder Optionen geändert haben.
    fn refresh_matches(&mut self) {
        if !self.search_dirty {
            return;
        }
        self.search_dirty = false;
        match find_all(&self.text, &self.find, self.search_opts()) {
            Ok(m) => {
                self.matches = m;
                self.search_error = None;
            }
            Err(e) => {
                self.matches.clear();
                self.search_error = Some(e);
            }
        }
        // Nach einer Änderung ist der bisherige Treffer nicht mehr gültig.
        self.match_idx = None;
        self.match_range = None;
    }

    fn select_match(&mut self, idx: usize) {
        if let Some(&(s, e)) = self.matches.get(idx) {
            self.match_range = Some((s, e));
            self.match_idx = Some(idx);
            self.scroll_line = Some(self.text[..s].matches('\n').count() + 1);
        }
    }

    fn find_next(&mut self) {
        self.refresh_matches();
        if self.matches.is_empty() {
            self.match_range = None;
            self.match_idx = None;
            return;
        }
        let idx = match self.match_idx {
            Some(i) => (i + 1) % self.matches.len(),
            None => 0,
        };
        self.select_match(idx);
    }

    fn find_prev(&mut self) {
        self.refresh_matches();
        if self.matches.is_empty() {
            self.match_range = None;
            self.match_idx = None;
            return;
        }
        let idx = match self.match_idx {
            Some(0) | None => self.matches.len() - 1,
            Some(i) => i - 1,
        };
        self.select_match(idx);
    }

    /// Ersetzungstext für den aktuellen Treffer, inklusive Rückverweisen im Regex-Modus.
    fn expanded_replacement(&self, s: usize, e: usize) -> String {
        if !self.use_regex {
            return self.replace.clone();
        }
        let Ok(re) = build_search_regex(&self.find, self.search_opts()) else {
            return self.replace.clone();
        };
        for caps in re.captures_iter(&self.text) {
            let m = caps.get(0).expect("Gruppe 0 existiert immer");
            if m.start() > s {
                break;
            }
            if m.start() == s && m.end() == e {
                let mut dst = String::new();
                caps.expand(&self.replace, &mut dst);
                return dst;
            }
        }
        self.replace.clone()
    }

    fn replace_current(&mut self) {
        self.refresh_matches();
        let Some((s, e)) = self.match_range else {
            self.find_next();
            return;
        };
        let replacement = self.expanded_replacement(s, e);
        self.commit_undo();
        self.text.replace_range(s..e, &replacement);
        self.commit_undo();
        self.err_pos = None;
        self.search_dirty = true;
        self.mark_compare_dirty();
        self.refresh_matches();
        // Danach den nächsten Treffer ab dem Ende der Ersetzung auswählen.
        if !self.matches.is_empty() {
            let from = s + replacement.len();
            let idx = self
                .matches
                .iter()
                .position(|&(ms, _)| ms >= from)
                .unwrap_or(0);
            self.select_match(idx);
        }
    }

    fn replace_all(&mut self) {
        if self.find.is_empty() {
            return;
        }
        match replace_all_matches(&self.text, &self.find, &self.replace, self.search_opts()) {
            Ok(new_text) => {
                if new_text != self.text {
                    self.commit_undo();
                    self.text = new_text;
                    self.commit_undo();
                }
                self.err_pos = None;
                self.search_dirty = true;
                self.mark_compare_dirty();
                self.refresh_matches();
            }
            Err(e) => {
                self.search_error = Some(e);
                self.matches.clear();
                self.match_idx = None;
                self.match_range = None;
            }
        }
    }

    fn find_bar(&mut self, ui: &mut egui::Ui) {
        // Erste Zeile: Suchfelder und Optionen.
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.find)
                    .hint_text("Suchen")
                    .desired_width(160.0),
            );
            if self.focus_find {
                r.request_focus();
                self.focus_find = false;
            }
            if r.changed() {
                self.search_dirty = true;
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                self.find_next();
                r.request_focus();
            }
            let r2 = ui.add(
                egui::TextEdit::singleline(&mut self.replace)
                    .hint_text("Ersetzen")
                    .desired_width(160.0),
            );
            if self.focus_replace {
                r2.request_focus();
                self.focus_replace = false;
            }

            if ui
                .checkbox(&mut self.case_sensitive, "Aa")
                .on_hover_text("Groß-/Kleinschreibung beachten")
                .changed()
            {
                self.search_dirty = true;
            }
            if ui
                .checkbox(&mut self.whole_word, "Wort")
                .on_hover_text("Nur ganze Wörter")
                .changed()
            {
                self.search_dirty = true;
            }
            if ui
                .checkbox(&mut self.use_regex, "Regex")
                .on_hover_text("Reguläre Ausdrücke")
                .changed()
            {
                self.search_dirty = true;
            }

            if ui.button("✕").on_hover_text("Suche schließen").clicked() {
                self.find_open = false;
                self.matches.clear();
                self.match_idx = None;
                self.match_range = None;
            }
        });

        // Zweite Zeile: Navigation, Ersetzen und Trefferzahl.
        ui.horizontal(|ui| {
            if ui
                .button("◀ Zurück")
                .on_hover_text("Vorheriger Treffer")
                .clicked()
            {
                self.find_prev();
            }
            if ui
                .button("Weiter ▶")
                .on_hover_text("Nächster Treffer")
                .clicked()
            {
                self.find_next();
            }
            if ui.button("Ersetzen").clicked() {
                self.replace_current();
            }
            if ui.button("Alle ersetzen").clicked() {
                self.replace_all();
            }

            // Trefferzahl bzw. Fehler des regulären Ausdrucks anzeigen.
            self.refresh_matches();
            if let Some(e) = &self.search_error {
                let msg = e.clone();
                ui.colored_label(ui.visuals().error_fg_color, "Ungültiger Ausdruck")
                    .on_hover_text(msg);
            } else if self.find.is_empty() {
                ui.label("0 Treffer");
            } else {
                ui.label(format!("{} Treffer", self.matches.len()));
            }
        });
    }

    /// Geteilte Vergleichsansicht (Alt links bzw. oben, Neu rechts bzw. unten).
    fn compare_ui(&mut self, ui: &mut egui::Ui, now: f64) {
        let Some(mut cmp) = self.compare.take() else {
            return;
        };
        if cmp.diff_dirty {
            cmp.diff = diff_lines(&cmp.left, &self.text);
            cmp.diff_dirty = false;
            cmp.change_idx = None;
        }

        // Kopfzeile mit Änderungszahl und Farblegende.
        ui.horizontal_wrapped(|ui| {
            let n = cmp.diff.changes.len();
            if n == 0 {
                ui.label("Keine Unterschiede.");
            } else {
                ui.label(blocks_text(n));
            }
            ui.separator();
            for (change, text) in [
                (LineChange::Changed, "geändert"),
                (LineChange::Added, "hinzugefügt"),
                (LineChange::Removed, "entfernt"),
            ] {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(11.0, 11.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 2.0, diff_swatch(change));
                ui.label(text);
            }
        });

        let fmt = self.fmt;
        let scroll = cmp.scroll;
        let block = if cmp.jump {
            cmp.diff.changes.get(cmp.change_idx.unwrap_or(0)).copied()
        } else {
            None
        };
        let jump_left = block.map(|b| b.left_start);
        let jump_right = block.map(|b| b.right_start);
        let restore = self.restore_cursor;
        let cursor = self.cursor;

        let (left, right) = if cmp.side_by_side {
            let mut left_out = None;
            let mut right_out = None;
            ui.columns(2, |cols| {
                left_out = Some(compare_pane(
                    &mut cols[0],
                    "Alt",
                    &mut cmp.left,
                    &cmp.diff.left,
                    fmt,
                    &mut cmp.left_hl,
                    scroll,
                    false,
                    false,
                    0,
                    jump_left,
                    &[],
                    None,
                ));
                right_out = Some(compare_pane(
                    &mut cols[1],
                    "Neu",
                    &mut self.text,
                    &cmp.diff.right,
                    fmt,
                    &mut cmp.right_hl,
                    scroll,
                    true,
                    restore,
                    cursor,
                    jump_right,
                    &self.matches,
                    self.match_idx,
                ));
            });
            (
                left_out.expect("linke Ansicht wurde gezeichnet"),
                right_out.expect("rechte Ansicht wurde gezeichnet"),
            )
        } else {
            let height = ((ui.available_height() - 10.0) / 2.0).max(60.0);
            let width = ui.available_width();
            let layout = egui::Layout::top_down(egui::Align::Min);
            let l = ui
                .allocate_ui_with_layout(egui::vec2(width, height), layout, |ui| {
                    compare_pane(
                        ui,
                        "Alt",
                        &mut cmp.left,
                        &cmp.diff.left,
                        fmt,
                        &mut cmp.left_hl,
                        scroll,
                        false,
                        false,
                        0,
                        jump_left,
                        &[],
                        None,
                    )
                })
                .inner;
            ui.separator();
            let r = ui
                .allocate_ui_with_layout(egui::vec2(width, height), layout, |ui| {
                    compare_pane(
                        ui,
                        "Neu",
                        &mut self.text,
                        &cmp.diff.right,
                        fmt,
                        &mut cmp.right_hl,
                        scroll,
                        true,
                        restore,
                        cursor,
                        jump_right,
                        &self.matches,
                        self.match_idx,
                    )
                })
                .inner;
            (l, r)
        };

        // Die rechte Ansicht ist der eigentliche Editor; Tippen, Undo usw.
        // wirken auf sie.
        self.editor_id = Some(right.id);
        self.cursor = right.cursor;
        if restore {
            self.restore_cursor = false;
        }
        if right.changed {
            self.err_pos = None;
            self.match_range = None;
            self.search_dirty = true;
            self.last_edit = now;
            cmp.diff_dirty = true;
        }
        if block.is_some() {
            cmp.jump = false;
        }

        // Synchrones Scrollen: Die Ansicht unter dem Mauszeiger gibt den
        // senkrechten Offset vor, die andere folgt.
        if left.hovered && (left.offset - cmp.scroll).abs() > 0.5 {
            cmp.scroll = left.offset;
        } else if right.hovered && (right.offset - cmp.scroll).abs() > 0.5 {
            cmp.scroll = right.offset;
        }
        if block.is_some() {
            // Nach dem Sprung folgt die linke Ansicht der rechten.
            cmp.scroll = right.offset;
        }

        self.compare = Some(cmp);
    }

    /// Statusleiste am unteren Fensterrand.
    ///
    /// Bei schmalen Fenstern brechen die Angaben um; das Format steht
    /// rechtsbündig, solange dort noch Platz ist, sonst im Fluss dahinter.
    fn status_bar(&self, ui: &mut egui::Ui) {
        let pos = line_col(&self.text, self.cursor);
        let lines = self.text.split('\n').count().max(1);
        let chars = self.text.chars().count();
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("Zeile {}, Spalte {}", pos.line, pos.col));
            ui.separator();
            ui.label(lines_text(lines));
            ui.separator();
            ui.label(format!("{chars} Zeichen"));
            if let Some(cmp) = &self.compare {
                ui.separator();
                ui.label(changes_text(cmp.diff.changes.len()));
            }

            let modified = self.is_modified();
            let fmt_name = self.fmt.name();
            // Breite der rechtsbündigen Angaben abschätzen.
            let body = egui::TextStyle::Body.resolve(ui.style());
            let text_width = |text: &str| {
                ui.fonts(|f| {
                    f.layout_no_wrap(text.to_owned(), body.clone(), egui::Color32::WHITE)
                        .size()
                        .x
                })
            };
            let mut needed = text_width(fmt_name) + 8.0;
            if modified {
                needed += text_width("geändert") + 12.0;
            }
            let remaining = ui.max_rect().right() - ui.cursor().min.x;
            if remaining >= needed {
                let size = egui::vec2(remaining, ui.spacing().interact_size.y);
                ui.allocate_ui_with_layout(
                    size,
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if modified {
                            ui.label(
                                egui::RichText::new("geändert").color(ui.visuals().warn_fg_color),
                            );
                            ui.separator();
                        }
                        ui.label(fmt_name);
                    },
                );
            } else {
                ui.separator();
                if modified {
                    ui.label(egui::RichText::new("geändert").color(ui.visuals().warn_fg_color));
                    ui.separator();
                }
                ui.label(fmt_name);
            }
        });
    }

    /// Lädt die gespeicherten Profile und Einstellungen (falls vorhanden).
    fn load_config(&mut self) {
        self.profiles_path = profiles_path();
        if let Some(p) = &self.profiles_path {
            self.profiles = load_profiles(p);
        }
        self.settings_path = settings_path();
        if let Some(p) = &self.settings_path {
            self.settings = load_settings(p);
        }
        self.fonts = font_candidates();
        // Einen liegen gebliebenen Zwischenstand nur anbieten, wenn er sich
        // noch vom Dateistand unterscheidet.
        self.recovery_path = recovery_path();
        if let Some(p) = &self.recovery_path {
            self.recovery = load_recovery(p);
            if let Some(rec) = &self.recovery {
                let gleich = rec
                    .path
                    .as_ref()
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .map(|t| t == rec.text)
                    .unwrap_or(false);
                if gleich {
                    self.recovery = None;
                    // Der Zwischenstand ist bereits gespeichert.
                    let _ = std::fs::remove_file(p);
                }
            }
        }
    }

    /// Schreibt die Profile auf den Datenträger.
    fn persist_profiles(&mut self) {
        let Some(path) = self.profiles_path.clone() else {
            self.profile_msg = "Kein Speicherort für Profile gefunden.".into();
            return;
        };
        match save_profiles(&path, &self.profiles) {
            Ok(()) => self.profile_msg.clear(),
            Err(e) => self.profile_msg = format!("Speichern fehlgeschlagen: {e}"),
        }
    }

    /// Schreibt die Einstellungen auf den Datenträger.
    fn persist_settings(&mut self) {
        let Some(path) = self.settings_path.clone() else {
            self.settings_msg = "Kein Speicherort für Einstellungen gefunden.".into();
            return;
        };
        match save_settings(&path, &self.settings) {
            Ok(()) => self.settings_msg.clear(),
            Err(e) => self.settings_msg = format!("Speichern fehlgeschlagen: {e}"),
        }
    }

    /// Wendet Farbschema, Schriftgröße und Editor-Schriftart an.
    fn apply_appearance(&mut self, ctx: &egui::Context) {
        ctx.set_theme(self.settings.theme.preference());
        let size = self.settings.editor_font_size;
        // Beide Themes setzen, damit die Größe auch nach einem Themenwechsel gilt.
        ctx.all_styles_mut(|style| {
            style
                .text_styles
                .insert(egui::TextStyle::Monospace, egui::FontId::monospace(size));
        });
        self.apply_editor_font(ctx);
    }

    /// Lädt die gewählte Systemschrift neu, sofern sie sich geändert hat.
    fn apply_editor_font(&mut self, ctx: &egui::Context) {
        if self.applied_font == self.settings.editor_font {
            return;
        }
        let mut definitions = egui::FontDefinitions::default();
        if !self.settings.editor_font.is_empty() {
            let chosen = self
                .fonts
                .iter()
                .find(|(name, _)| *name == self.settings.editor_font);
            if let Some((name, path)) = chosen {
                if let Some(bytes) = read_font_file(path) {
                    let key = format!("system:{name}");
                    definitions.font_data.insert(
                        key.clone(),
                        std::sync::Arc::new(egui::FontData::from_owned(bytes)),
                    );
                    definitions
                        .families
                        .entry(egui::FontFamily::Monospace)
                        .or_default()
                        .insert(0, key);
                }
            }
        }
        ctx.set_fonts(definitions);
        self.applied_font = self.settings.editor_font.clone();
    }

    /// Einstellungsfenster für Farbschema und Editor-Darstellung.
    fn settings_window_ui(&mut self, ctx: &egui::Context) {
        if !self.settings_window {
            return;
        }
        let mut open = self.settings_window;
        let mut changed = false;
        let fonts = self.fonts.clone();
        egui::Window::new("Einstellungen")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Farbschema");
                for mode in ThemeMode::ALL {
                    if ui
                        .radio_value(&mut self.settings.theme, mode, mode.name())
                        .changed()
                    {
                        changed = true;
                    }
                }

                ui.separator();
                ui.label("Editor");
                ui.horizontal(|ui| {
                    ui.label("Schriftgröße");
                    changed |= ui
                        .add(
                            egui::Slider::new(
                                &mut self.settings.editor_font_size,
                                EDITOR_FONT_MIN..=EDITOR_FONT_MAX,
                            )
                            .suffix(" pt")
                            .fixed_decimals(0),
                        )
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Schriftart");
                    let selected = if self.settings.editor_font.is_empty() {
                        "Standard".to_owned()
                    } else {
                        self.settings.editor_font.clone()
                    };
                    egui::ComboBox::from_id_salt("editor_schrift")
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            changed |= ui
                                .selectable_value(
                                    &mut self.settings.editor_font,
                                    String::new(),
                                    "Standard",
                                )
                                .changed();
                            for (name, _) in &fonts {
                                changed |= ui
                                    .selectable_value(
                                        &mut self.settings.editor_font,
                                        name.clone(),
                                        name,
                                    )
                                    .changed();
                            }
                        });
                });
                if fonts.is_empty() {
                    ui.small("Keine zusätzlichen Systemschriften gefunden.");
                }

                if !self.settings_msg.is_empty() {
                    ui.separator();
                    ui.label(&self.settings_msg);
                }
            });
        self.settings_window = open;
        if changed {
            self.settings.normalize();
            self.persist_settings();
            self.apply_appearance(ctx);
        }
    }

    /// Fenster mit den zuletzt verwendeten Dateien.
    fn recent_window_ui(&mut self, ctx: &egui::Context) {
        if !self.recent_open {
            return;
        }
        let mut open = self.recent_open;
        let mut chosen: Option<PathBuf> = None;
        let mut clear = false;
        let entries = self.settings.recent.clone();
        egui::Window::new("Zuletzt verwendet")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                if entries.is_empty() {
                    ui.label("Noch keine Dateien geöffnet.");
                    return;
                }
                for entry in &entries {
                    let name = Path::new(entry)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(entry);
                    if ui.link(name).on_hover_text(entry).clicked() {
                        chosen = Some(PathBuf::from(entry));
                    }
                }
                ui.separator();
                if ui.button("Liste leeren").clicked() {
                    clear = true;
                }
            });
        self.recent_open = open;
        if clear {
            self.settings.recent.clear();
            self.persist_settings();
        }
        if let Some(path) = chosen {
            self.recent_open = false;
            self.request_open(path);
        }
    }

    /// Rückfrage, bevor ungespeicherte Änderungen durch eine andere Datei ersetzt werden.
    fn pending_open_ui(&mut self, ctx: &egui::Context) {
        let Some(path) = self.pending_open.clone() else {
            return;
        };
        let target = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Datei")
            .to_owned();
        let mut open_it = false;
        let mut cancel = false;
        egui::Window::new("Ungespeicherte Änderungen")
            .id(egui::Id::new("oeffnen_rueckfrage"))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Die aktuelle Datei enthält ungespeicherte Änderungen.");
                ui.label(format!("Trotzdem „{target}“ öffnen?"));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Öffnen (Änderungen verwerfen)").clicked() {
                        open_it = true;
                    }
                    if ui.button("Abbrechen").clicked() {
                        cancel = true;
                    }
                });
            });
        if open_it {
            self.pending_open = None;
            self.open_path(path);
        } else if cancel {
            self.pending_open = None;
        }
    }

    /// Bietet einen gefundenen Zwischenstand zur Wiederherstellung an.
    fn recovery_window_ui(&mut self, ctx: &egui::Context) {
        let Some(rec) = self.recovery.clone() else {
            return;
        };
        let name = rec
            .path
            .as_ref()
            .and_then(|p| {
                Path::new(p)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "Unbenannt".to_owned());
        let mut restore = false;
        let mut discard = false;
        egui::Window::new("Nicht gespeicherte Änderungen")
            .id(egui::Id::new("wiederherstellung"))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!("Für „{name}“ wurde ein Zwischenstand gefunden."));
                ui.label("Er enthält Änderungen, die nicht gespeichert wurden.");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Wiederherstellen").clicked() {
                        restore = true;
                    }
                    if ui.button("Verwerfen").clicked() {
                        discard = true;
                    }
                });
            });
        if restore {
            self.restore_recovery();
        } else if discard {
            self.discard_recovery();
        }
    }

    /// Einstellungsfenster für Einrückung, Minify und Profile.
    fn format_window_ui(&mut self, ctx: &egui::Context) {
        if !self.format_window {
            return;
        }
        let mut open = self.format_window;
        egui::Window::new("Formatierung")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                // Profilauswahl
                ui.horizontal(|ui| {
                    ui.label("Profil:");
                    let names: Vec<String> = self
                        .profiles
                        .profiles
                        .iter()
                        .map(|p| p.name.clone())
                        .collect();
                    let mut selection = self.profiles.active_index();
                    egui::ComboBox::from_id_salt("profilwahl")
                        .selected_text(names[selection].clone())
                        .show_ui(ui, |ui| {
                            for (i, name) in names.iter().enumerate() {
                                ui.selectable_value(&mut selection, i, name);
                            }
                        });
                    if selection != self.profiles.active {
                        self.profiles.active = selection;
                        self.persist_profiles();
                    }
                });

                ui.separator();
                ui.label("Einrückung");
                let mut opts = self.profiles.options();
                let before = opts;
                ui.radio_value(&mut opts.indent, Indent::Spaces2, Indent::Spaces2.name());
                ui.radio_value(&mut opts.indent, Indent::Spaces4, Indent::Spaces4.name());
                ui.radio_value(&mut opts.indent, Indent::Tabs, Indent::Tabs.name());
                if self.fmt == Fmt::Yaml && opts.indent == Indent::Tabs {
                    if opts.minify {
                        ui.small(
                            "YAML erlaubt keine Tabs – im Minify-Modus wird mit einem Leerzeichen eingerückt.",
                        );
                    } else {
                        ui.small(
                            "YAML erlaubt keine Tabs zur Einrückung – es gilt 2 Leerzeichen.",
                        );
                    }
                }
                ui.add_space(4.0);
                ui.checkbox(&mut opts.minify, "Kompakte Darstellung (Minify)");
                ui.checkbox(
                    &mut opts.sort_json_keys,
                    "JSON-Schlüssel alphabetisch sortieren",
                );
                if opts != before {
                    self.profiles.set_options(opts);
                    self.persist_profiles();
                }

                ui.separator();
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_profile_name)
                            .hint_text("Neuer Profilname")
                            .desired_width(160.0),
                    );
                    if ui.button("Als Profil speichern").clicked() {
                        let name = self.new_profile_name.trim().to_string();
                        let options = self.profiles.options();
                        if self.profiles.add(&name, options) {
                            self.new_profile_name.clear();
                            self.persist_profiles();
                            self.profile_msg = format!("Profil „{name}“ angelegt.");
                        } else {
                            self.profile_msg =
                                "Name ist leer oder bereits vergeben.".into();
                        }
                    }
                    let can_remove = self.profiles.profiles.len() > 1;
                    if ui
                        .add_enabled(can_remove, egui::Button::new("Profil löschen"))
                        .on_hover_text("Löscht das aktive Profil")
                        .clicked()
                    {
                        self.profiles.remove_active();
                        self.persist_profiles();
                    }
                });
                if !self.profile_msg.is_empty() {
                    ui.label(&self.profile_msg);
                }
            });
        self.format_window = open;
    }

    /// Dialog für die Formatkonvertierung (JSON, XML, YAML).
    fn convert_window_ui(&mut self, ctx: &egui::Context) {
        if !self.convert_window {
            return;
        }
        let mut open = self.convert_window;
        egui::Window::new("Konvertieren")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Quelldokument:");
                    ui.label(egui::RichText::new(self.fmt.name()).strong());
                });
                ui.add_space(4.0);
                ui.label("Zielformat:");
                ui.radio_value(&mut self.convert_target, Fmt::Json, "JSON");
                ui.radio_value(&mut self.convert_target, Fmt::Xml, "XML");
                ui.radio_value(&mut self.convert_target, Fmt::Yaml, "YAML");
                ui.add_space(4.0);
                ui.checkbox(
                    &mut self.convert_compare,
                    "Vorher-/Nachher-Vergleich anzeigen",
                );
                ui.small("Einrückung und Minify folgen dem aktiven Formatierungsprofil.");
                if let Some(e) = &self.convert_error {
                    ui.colored_label(
                        Color32::from_rgb(200, 30, 30),
                        format!("Fehler: {e}"),
                    );
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("In Editor übernehmen").clicked() {
                        let target = self.convert_target;
                        let compare = self.convert_compare;
                        self.apply_conversion(target, compare);
                        if self.convert_error.is_none() {
                            self.convert_window = false;
                        }
                    }
                    if ui.button("Als Datei speichern…").clicked() {
                        self.save_conversion(self.convert_target);
                    }
                    if ui.button("Schließen").clicked() {
                        self.convert_window = false;
                    }
                });
            });
        self.convert_window = open;
    }

    /// Zeichnet das Ribbon am oberen Fensterrand.
    fn ribbon_ui(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("ribbon").show(ctx, |ui| {
            // Dezente Abhebung vom Editor; die Farbe folgt dem aktiven Theme.
            let bg = ui.visuals().faint_bg_color;
            ui.painter().rect_filled(ui.max_rect(), 0.0, bg);
            ui.add_space(2.0);
            self.ribbon_tabs(ui);
            if !self.ribbon_collapsed {
                ui.separator();
                ui.add_space(2.0);
                self.ribbon_content(ui);
                ui.add_space(2.0);
            }
        });
    }

    /// Registerkartenleiste inklusive Ein-/Ausklappen.
    fn ribbon_tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Der Pfeil wird zuerst rechts platziert, damit ihn die
            // Registerkarten bei schmalen Fenstern nicht überdecken.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (icon, tip) = if self.ribbon_collapsed {
                    (Icon::ChevronDown, "Ribbon einblenden")
                } else {
                    (Icon::ChevronUp, "Ribbon einklappen")
                };
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(24.0, 20.0), egui::Sense::click());
                if response.hovered() {
                    ui.painter()
                        .rect_filled(rect, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
                }
                let color = if response.hovered() {
                    ui.visuals().strong_text_color()
                } else {
                    ui.visuals().text_color()
                };
                icons::draw(
                    ui.painter(),
                    egui::Rect::from_center_size(rect.center(), egui::vec2(14.0, 14.0)),
                    icon,
                    color,
                );
                if response.on_hover_text(tip).clicked() {
                    self.ribbon_collapsed = !self.ribbon_collapsed;
                }

                // Registerkarten im verbleibenden Platz, bei Bedarf umbrechend.
                ui.with_layout(
                    egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for tab in RibbonTab::ALL {
                            let selected = self.ribbon_tab == tab;
                            let text = egui::RichText::new(tab.name()).size(13.0);
                            if ui.selectable_label(selected, text).clicked() {
                                if selected {
                                    // Erneuter Klick auf die aktive Registerkarte klappt das Ribbon ein.
                                    self.ribbon_collapsed = !self.ribbon_collapsed;
                                } else {
                                    self.ribbon_tab = tab;
                                    self.ribbon_collapsed = false;
                                }
                            }
                        }
                    },
                );
            });
        });
    }

    /// Inhalt der aktiven Registerkarte.
    fn ribbon_content(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 2.0);
            match self.ribbon_tab {
                RibbonTab::Datei => {
                    ribbon_group(ui, "Datei", 3, |ui| {
                        if ribbon_button(ui, Icon::Open, "Öffnen", true, "Datei öffnen (Strg+O)")
                        {
                            self.open();
                        }
                        if ribbon_button(ui, Icon::Save, "Speichern", true, "Speichern (Strg+S)") {
                            self.save();
                        }
                        let can_reload = self.path.is_some();
                        if ribbon_button(
                            ui,
                            Icon::Reload,
                            "Neu laden",
                            can_reload,
                            "Datei erneut vom Datenträger lesen",
                        ) {
                            self.reload();
                        }
                    });
                    ribbon_group(ui, "Zuletzt", 1, |ui| {
                        if ribbon_button(
                            ui,
                            Icon::Clock,
                            "Zuletzt",
                            true,
                            "Zuletzt verwendete Dateien anzeigen",
                        ) {
                            self.recent_open = true;
                        }
                    });
                }
                RibbonTab::Bearbeiten => {
                    ribbon_group(ui, "Rückgängig", 2, |ui| {
                        let can_undo = self.history.can_undo();
                        if ribbon_button(ui, Icon::Undo, "Zurück", can_undo, "Rückgängig (Strg+Z)")
                        {
                            self.undo();
                        }
                        let can_redo = self.history.can_redo();
                        if ribbon_button(
                            ui,
                            Icon::Redo,
                            "Vorwärts",
                            can_redo,
                            "Wiederholen (Strg+Umschalt+Z)",
                        ) {
                            self.redo();
                        }
                    });
                    ribbon_group(ui, "Suchen", 2, |ui| {
                        if ribbon_button(
                            ui,
                            Icon::Find,
                            "Suchen",
                            true,
                            "Suchen und Ersetzen (Strg+F)",
                        ) {
                            self.find_open = true;
                            self.focus_find = true;
                            self.focus_replace = false;
                        }
                        if ribbon_button(
                            ui,
                            Icon::Replace,
                            "Ersetzen",
                            true,
                            "Suchleiste mit Fokus im Ersetzen-Feld öffnen",
                        ) {
                            self.find_open = true;
                            self.focus_find = false;
                            self.focus_replace = true;
                        }
                    });
                }
                RibbonTab::Formatieren => {
                    ribbon_group(ui, "Format", 3, |ui| {
                        if ribbon_toggle(
                            ui,
                            Icon::Braces,
                            "JSON",
                            self.fmt == Fmt::Json,
                            "JSON als Zielformat",
                        ) {
                            self.fmt = Fmt::Json;
                        }
                        if ribbon_toggle(
                            ui,
                            Icon::Tags,
                            "XML",
                            self.fmt == Fmt::Xml,
                            "XML als Zielformat",
                        ) {
                            self.fmt = Fmt::Xml;
                        }
                        if ribbon_toggle(
                            ui,
                            Icon::List,
                            "YAML",
                            self.fmt == Fmt::Yaml,
                            "YAML als Zielformat",
                        ) {
                            self.fmt = Fmt::Yaml;
                        }
                    });
                    ribbon_group(ui, "Aktionen", 2, |ui| {
                        if ribbon_button(
                            ui,
                            Icon::Format,
                            "Formatieren",
                            true,
                            "Inhalt formatieren",
                        ) {
                            self.format();
                        }
                        let minify = self.profiles.options().minify;
                        if ribbon_toggle(
                            ui,
                            Icon::Minify,
                            "Minify",
                            minify,
                            "Kompakte Darstellung umschalten",
                        ) {
                            let mut opts = self.profiles.options();
                            opts.minify = !opts.minify;
                            self.profiles.set_options(opts);
                            self.persist_profiles();
                        }
                    });
                    ribbon_group(ui, "Konvertieren", 1, |ui| {
                        if ribbon_button(
                            ui,
                            Icon::Convert,
                            "Konvertieren",
                            true,
                            "Dokument in ein anderes Format (JSON, XML, YAML) umwandeln",
                        ) {
                            // Das Zielformat vorauswählen, das nicht dem
                            // Quellformat entspricht.
                            self.convert_target = match self.fmt {
                                Fmt::Json => Fmt::Yaml,
                                _ => Fmt::Json,
                            };
                            self.convert_compare = false;
                            self.convert_error = None;
                            self.convert_window = true;
                        }
                    });
                    ribbon_group(ui, "Einstellungen", 1, |ui| {
                        if ribbon_button(
                            ui,
                            Icon::Settings,
                            "Profile…",
                            true,
                            "Einrückung, Minify und Profile einstellen",
                        ) {
                            self.format_window = true;
                        }
                    });
                    // Kontextabhängig: Die Schlüsselsortierung gibt es nur für JSON.
                    if self.fmt == Fmt::Json {
                        ribbon_group(ui, "Sortierung", 1, |ui| {
                            let sorted = self.profiles.options().sort_json_keys;
                            if ribbon_toggle(
                                ui,
                                Icon::Sort,
                                "Sortieren",
                                sorted,
                                "JSON-Schlüssel alphabetisch sortieren",
                            ) {
                                let mut opts = self.profiles.options();
                                opts.sort_json_keys = !opts.sort_json_keys;
                                self.profiles.set_options(opts);
                                self.persist_profiles();
                            }
                        });
                    }
                }
                RibbonTab::Vergleichen => {
                    ribbon_group(ui, "Vergleich", 2, |ui| {
                        if ribbon_button(
                            ui,
                            Icon::Compare,
                            "Vergleich",
                            true,
                            "Aktuellen Inhalt formatieren und als Vorher/Nachher vergleichen",
                        ) {
                            self.start_compare_format();
                        }
                        let can_saved = self.path.is_some() || !self.saved_text.is_empty();
                        if ribbon_button(
                            ui,
                            Icon::Reload,
                            "Dateistand",
                            can_saved,
                            "Aktuellen Inhalt mit dem gespeicherten Dateistand vergleichen",
                        ) {
                            self.start_compare_saved();
                        }
                    });
                    ribbon_group(ui, "Teilung", 2, |ui| {
                        let side = self
                            .compare
                            .as_ref()
                            .map(|c| c.side_by_side)
                            .unwrap_or(self.settings.compare_side_by_side);
                        if ribbon_toggle(
                            ui,
                            Icon::SplitH,
                            "Nebeneinander",
                            side,
                            "Alt und Neu nebeneinander anzeigen",
                        ) {
                            self.set_compare_layout(true);
                        }
                        if ribbon_toggle(
                            ui,
                            Icon::SplitV,
                            "Untereinander",
                            !side,
                            "Alt und Neu untereinander anzeigen",
                        ) {
                            self.set_compare_layout(false);
                        }
                    });
                    // Kontextabhängig: Navigation und Beenden nur im Vergleich.
                    if self.compare.is_some() {
                        let has_changes = self
                            .compare
                            .as_ref()
                            .is_some_and(|c| !c.diff.changes.is_empty());
                        ribbon_group(ui, "Änderungen", 2, |ui| {
                            if ribbon_button(
                                ui,
                                Icon::ChevronLeft,
                                "Zurück",
                                has_changes,
                                "Vorheriger Änderungsblock",
                            ) {
                                self.compare_step(false);
                            }
                            if ribbon_button(
                                ui,
                                Icon::ChevronRight,
                                "Weiter",
                                has_changes,
                                "Nächster Änderungsblock",
                            ) {
                                self.compare_step(true);
                            }
                        });
                        ribbon_group(ui, "Beenden", 1, |ui| {
                            if ribbon_button(
                                ui,
                                Icon::Close,
                                "Beenden",
                                true,
                                "Vergleichsansicht schließen",
                            ) {
                                self.close_compare();
                            }
                        });
                    }
                }
                RibbonTab::Ansicht => {
                    ribbon_group(ui, "Ribbon", 1, |ui| {
                        if ribbon_button(
                            ui,
                            Icon::ChevronUp,
                            "Einklappen",
                            true,
                            "Ribbon einklappen und mehr Platz für den Editor",
                        ) {
                            self.ribbon_collapsed = true;
                        }
                    });
                    ribbon_group(ui, "Statusleiste", 1, |ui| {
                        if ribbon_toggle(
                            ui,
                            Icon::List,
                            "Anzeigen",
                            !self.hide_status,
                            "Statusleiste ein- oder ausblenden",
                        ) {
                            self.hide_status = !self.hide_status;
                        }
                    });
                    ribbon_group(ui, "Zoom", 3, |ui| {
                        if ribbon_button(ui, Icon::Minus, "Kleiner", true, "Ansicht verkleinern") {
                            let z = ui.ctx().zoom_factor();
                            ui.ctx().set_zoom_factor((z / 1.1).max(0.6));
                        }
                        if ribbon_button(ui, Icon::Plus, "Größer", true, "Ansicht vergrößern") {
                            let z = ui.ctx().zoom_factor();
                            ui.ctx().set_zoom_factor((z * 1.1).min(3.0));
                        }
                        if ribbon_button(ui, Icon::Reload, "100 %", true, "Zoom zurücksetzen") {
                            ui.ctx().set_zoom_factor(1.0);
                        }
                    });
                    ribbon_group(ui, "Einstellungen", 1, |ui| {
                        if ribbon_button(
                            ui,
                            Icon::Settings,
                            "Darstellung",
                            true,
                            "Farbschema, Schriftgröße und Editor-Schriftart einstellen",
                        ) {
                            self.settings_window = true;
                        }
                    });
                }
            }
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _f: &mut eframe::Frame) {
        let now = ctx.input(|i| i.time);
        // Drag & Drop: die erste abgelegte Datei öffnen.
        let dropped = ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone()));
        if let Some(path) = dropped {
            self.request_open(path);
        }
        self.check_disk(ctx, now);
        self.update_title(ctx);
        self.handle_close_request(ctx);
        let _ = self.auto_indent(ctx, now);
        let _ = self.handle_typing(ctx, now);
        let _ = self.handle_line_ops(ctx, now);
        let sc_save = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
        let sc_find = KeyboardShortcut::new(Modifiers::COMMAND, Key::F);
        let sc_open = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
        let sc_undo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
        let sc_redo = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        let sc_redo_y = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
        // Vor dem TextEdit abfangen, damit egui nicht seine eigene Undo-Historie nutzt.
        if ctx.input_mut(|i| i.consume_shortcut(&sc_redo) || i.consume_shortcut(&sc_redo_y)) {
            self.redo();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc_undo)) {
            self.undo();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc_save)) {
            self.save();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc_open)) {
            self.open();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc_find)) {
            self.find_open = true;
            self.focus_find = true;
            self.focus_replace = false;
        }
        if self.find_open && ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.find_open = false;
            self.matches.clear();
            self.match_idx = None;
            self.match_range = None;
        }

        // Ribbon und Statusleiste vor dem Editor anlegen, damit sie Platz reservieren.
        self.ribbon_ui(ctx);
        if !self.hide_status {
            egui::TopBottomPanel::bottom("statusleiste").show(ctx, |ui| {
                self.status_bar(ui);
            });
        }
        let cursor_before = self.cursor;

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.external_change {
                ui.horizontal_wrapped(|ui| {
                    let hint = if self.is_modified() {
                        "⚠ Die Datei wurde außerhalb von Parsix geändert – Neu laden verwirft die ungespeicherten Änderungen."
                    } else {
                        "⚠ Die Datei wurde außerhalb von Parsix geändert."
                    };
                    ui.colored_label(ui.visuals().warn_fg_color, hint);
                    if ui
                        .button("Neu laden")
                        .on_hover_text("Datei erneut vom Datenträger lesen")
                        .clicked()
                    {
                        self.reload();
                    }
                    if ui
                        .button("Behalten")
                        .on_hover_text("Lokalen Stand weiter bearbeiten")
                        .clicked()
                    {
                        self.external_change = false;
                    }
                    if ui
                        .button("Trotzdem speichern")
                        .on_hover_text("Datei mit dem lokalen Stand überschreiben")
                        .clicked()
                    {
                        self.overwrite();
                    }
                });
            }
            if self.find_open { self.find_bar(ui); }
            ui.label(&self.msg);

            if self.compare.is_some() {
                self.compare_ui(ui, now);
            } else {
            egui::ScrollArea::both().show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    // Zeilennummern
                    let lines = self.text.split('\n').count().max(1);
                    let nums = (1..=lines).map(|n| n.to_string()).collect::<Vec<_>>().join("\n");
                    ui.label(egui::RichText::new(nums).monospace().color(ui.visuals().weak_text_color()));

                    let mut out = {
                        self.update_highlight(ui);
                        let cached = self.hl.clone();
                        let hl_fmt = self.fmt;
                        let hl_dark = ui.visuals().dark_mode;
                        let hl_font = egui::TextStyle::Monospace.resolve(ui.style());
                        let mut layouter = move |ui: &egui::Ui, text: &str, wrap_width: f32| {
                            let mut job = match &cached {
                                Some(job) if job.text.as_str() == text => job.clone(),
                                // Der Text wurde in diesem Frame geändert.
                                _ => highlight::layout_job(
                                    hl_fmt,
                                    text,
                                    hl_font.clone(),
                                    hl_dark,
                                    f32::INFINITY,
                                ),
                            };
                            job.wrap.max_width = wrap_width;
                            ui.fonts(|f| f.layout_job(job))
                        };
                        egui::TextEdit::multiline(&mut self.text)
                            .code_editor()
                            .desired_width(f32::INFINITY)
                            .desired_rows(30)
                            .layouter(&mut layouter)
                            .show(ui)
                    };
                    self.editor_id = Some(out.response.id);

                    if out.response.changed() {
                        self.err_pos = None;
                        self.match_range = None;
                        self.search_dirty = true;
                        self.last_edit = now;
                    }

                    // Nach Undo/Redo, Zeilenaktionen usw. die Auswahl wiederherstellen.
                    if let Some((s, e)) = self.restore_sel.take() {
                        let chars = self.text.chars().count();
                        let s = s.min(chars);
                        let e = e.min(chars).max(s);
                        self.cursor = e;
                        out.state.cursor.set_char_range(Some(CCursorRange::two(
                            CCursor::new(s),
                            CCursor::new(e),
                        )));
                        out.state.store(ui.ctx(), out.response.id);
                        self.restore_cursor = false;
                    } else if self.restore_cursor {
                        self.restore_cursor = false;
                        let chars = self.text.chars().count();
                        let idx = self.cursor.min(chars);
                        self.cursor = idx;
                        out.state
                            .cursor
                            .set_char_range(Some(CCursorRange::one(CCursor::new(idx))));
                        out.state.store(ui.ctx(), out.response.id);
                    } else if let Some(range) = out.cursor_range {
                        self.cursor = range.primary.ccursor.index;
                    }

                    let rows = out.galley.rows.len().max(1);
                    let lh = out.galley.size().y / rows as f32;
                    let line_rect = |line: usize| {
                        let line = line.clamp(1, rows);
                        let y = out.galley_pos.y + (line - 1) as f32 * lh;
                        egui::Rect::from_x_y_ranges(ui.clip_rect().x_range(), y..=y + lh)
                    };

                    // Fehlerzeile und exakte Fehlerstelle
                    if let Some(p) = self.err_pos {
                        let r = line_rect(p.line);
                        ui.painter().rect_filled(
                            r,
                            0.0,
                            Color32::from_rgba_unmultiplied(255, 60, 60, 70),
                        );
                        // Senkrechte Markierung an der Fehlerspalte.
                        let idx = char_index_at(&self.text, p);
                        let caret = out.galley.pos_from_ccursor(egui::text::CCursor::new(idx));
                        let col_rect = egui::Rect::from_min_size(
                            egui::pos2(
                                out.galley_pos.x + caret.min.x,
                                out.galley_pos.y + caret.min.y,
                            ),
                            egui::vec2(2.0, lh),
                        );
                        ui.painter().rect_filled(col_rect, 0.0, Color32::from_rgb(255, 80, 80));
                        if self.jump {
                            ui.scroll_to_rect(r, Some(egui::Align::Center));
                            self.jump = false;
                        }
                    }

                    // Alle Suchtreffer hervorheben, den aktuellen kräftiger.
                    let mut last_b = 0;
                    let mut last_c = 0;
                    for (i, &(s, e)) in self.matches.iter().enumerate() {
                        let cs = last_c + self.text[last_b..s].chars().count();
                        let ce = cs + self.text[s..e].chars().count();
                        last_b = e;
                        last_c = ce;
                        let a = out.galley.pos_from_ccursor(egui::text::CCursor::new(cs));
                        let b = out.galley.pos_from_ccursor(egui::text::CCursor::new(ce));
                        let rect = egui::Rect::from_min_max(
                            out.galley_pos + a.min.to_vec2(),
                            out.galley_pos + egui::vec2(b.max.x, a.max.y),
                        );
                        let color = if Some(i) == self.match_idx {
                            Color32::from_rgba_unmultiplied(255, 190, 0, 150)
                        } else {
                            Color32::from_rgba_unmultiplied(255, 225, 120, 90)
                        };
                        ui.painter().rect_filled(rect, 2.0, color);
                    }

                    if let Some(l) = self.scroll_line.take() {
                        ui.scroll_to_rect(line_rect(l), Some(egui::Align::Center));
                    }
                });
            });
            }
        });

        if self.confirm_close {
            egui::Window::new("Ungespeicherte Änderungen")
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("Die Datei enthält ungespeicherte Änderungen.");
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Speichern und schließen").clicked() {
                            self.save();
                            if !self.is_modified() {
                                self.allow_close = true;
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        }
                        if ui.button("Ohne Speichern schließen").clicked() {
                            self.allow_close = true;
                            self.discard_recovery();
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button("Abbrechen").clicked() {
                            self.confirm_close = false;
                        }
                    });
                });
        }

        // Tipp-Eingaben nach einer kurzen Pause als einen Undo-Schritt sichern.
        if self.text != self.history.current().text {
            let waited = now - self.last_edit;
            if waited >= UNDO_DEBOUNCE_SECS {
                self.commit_undo();
            } else {
                ctx.request_repaint_after(Duration::from_secs_f64(UNDO_DEBOUNCE_SECS - waited));
            }
        }

        self.format_window_ui(ctx);
        self.convert_window_ui(ctx);
        self.settings_window_ui(ctx);
        self.recent_window_ui(ctx);
        self.pending_open_ui(ctx);
        self.recovery_window_ui(ctx);
        self.autosave(now);

        // Die Statusleiste wurde vor dem Editor gezeichnet; hat sich die
        // Cursorposition geändert, einmal neu zeichnen, damit sie aktuell ist.
        if self.cursor != cursor_before {
            ctx.request_repaint();
        }
    }
}

fn main() -> eframe::Result {
    let mut app = App::default();
    app.load_config();
    eframe::run_native(
        "Parsix",
        eframe::NativeOptions::default(),
        Box::new(|cc| {
            app.apply_appearance(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Standardoptionen für Tests, die nur die Grundformatierung prüfen.
    fn def_opts() -> FormatOptions {
        FormatOptions::default()
    }

    #[test]
    fn undo_und_redo_stellen_zustaende_wieder_her() {
        let mut h = History::new("a", 1);
        assert!(!h.can_undo());
        assert!(!h.can_redo());
        assert!(h.record("ab", 2));
        assert!(h.can_undo());
        assert!(!h.can_redo());

        let s = h.undo("ab", 2).unwrap();
        assert_eq!(s.text, "a");
        assert_eq!(s.cursor, 1);
        assert!(h.can_redo());

        let s = h.redo("a", 1).unwrap();
        assert_eq!(s.text, "ab");
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn gleicher_text_erzeugt_keinen_schritt() {
        let mut h = History::new("x", 0);
        assert!(!h.record("x", 3));
        assert!(!h.can_undo());
        assert_eq!(h.current().cursor, 3);
    }

    #[test]
    fn undo_sichert_noch_offene_eingaben() {
        let mut h = History::new("", 0);
        h.record("hallo", 5);
        // Eingabe " welt" wurde noch nicht aufgenommen.
        let s = h.undo("hallo welt", 10).unwrap();
        assert_eq!(s.text, "hallo");

        let s = h.redo("hallo", 5).unwrap();
        assert_eq!(s.text, "hallo welt");
    }

    #[test]
    fn neuer_schritt_verwirft_den_redo_zweig() {
        let mut h = History::new("a", 1);
        h.record("ab", 2);
        h.undo("ab", 2).unwrap();
        assert!(h.can_redo());
        h.record("ax", 2);
        assert!(!h.can_redo());
    }

    #[test]
    fn undo_bei_leerer_historie_liefert_nichts() {
        let mut h = History::new("", 0);
        assert!(h.undo("", 0).is_none());
        assert!(h.redo("", 0).is_none());
    }

    #[test]
    fn historie_wird_begrenzt() {
        let mut h = History::new("0", 0);
        for i in 1..=(MAX_UNDO_STATES * 2) {
            let t = i.to_string();
            h.record(&t, t.len());
        }
        assert_eq!(h.states.len(), MAX_UNDO_STATES);
        assert_eq!(h.pos, MAX_UNDO_STATES - 1);
        assert_eq!(h.current().text, (MAX_UNDO_STATES * 2).to_string());
        assert!(h.can_undo());
    }

    // --- automatische Einrückung ---

    fn nl(fmt: Fmt, text: &str) -> String {
        newline_with_indent(fmt, text, text.len())
    }

    #[test]
    fn json_rueckt_nach_klammer_ein() {
        assert_eq!(nl(Fmt::Json, "{\n  \"a\": {"), "\n    ");
        assert_eq!(nl(Fmt::Json, "["), "\n  ");
        assert_eq!(nl(Fmt::Json, "  \"a\": 1,"), "\n  ");
        assert_eq!(nl(Fmt::Json, "\t\"a\": ["), "\n\t\t");
    }

    #[test]
    fn xml_rueckt_nach_offenem_tag_ein() {
        assert_eq!(nl(Fmt::Xml, "<root>"), "\n  ");
        assert_eq!(nl(Fmt::Xml, "  <a x=\"1\">"), "\n    ");
        assert_eq!(nl(Fmt::Xml, "  <a x=\"1\" />"), "\n  ");
        assert_eq!(nl(Fmt::Xml, "  </a>"), "\n  ");
        assert_eq!(nl(Fmt::Xml, "  <a>text</a>"), "\n  ");
        assert_eq!(nl(Fmt::Xml, "<a>text"), "\n");
    }

    #[test]
    fn yaml_rueckt_nach_doppelpunkt_und_sequenz_ein() {
        assert_eq!(nl(Fmt::Yaml, "eintrag:"), "\n  ");
        assert_eq!(nl(Fmt::Yaml, "- eintrag:"), "\n  ");
        assert_eq!(nl(Fmt::Yaml, "  -"), "\n    ");
        assert_eq!(nl(Fmt::Yaml, "text: |"), "\n  ");
        assert_eq!(nl(Fmt::Yaml, "- wert"), "\n");
        assert_eq!(nl(Fmt::Yaml, "name: Parsix"), "\n");
    }

    #[test]
    fn einrueckung_nutzt_die_zeile_an_der_cursorposition() {
        let text = "{\n    \"a\": 1,\n}";
        let cursor = text.find(",\n").unwrap() + 1; // direkt nach dem Komma
        assert_eq!(newline_with_indent(Fmt::Json, text, cursor), "\n    ");
    }

    #[test]
    fn einrueckung_bei_leerem_text() {
        assert_eq!(nl(Fmt::Json, ""), "\n");
    }

    #[test]
    fn byte_index_ist_unicode_sicher() {
        let text = "äöü";
        assert_eq!(byte_index(text, 0), 0);
        assert_eq!(byte_index(text, 1), 2);
        assert_eq!(byte_index(text, 3), 6);
        assert_eq!(byte_index(text, 4), 6); // hinter dem Ende
    }

    // --- Klammer- und Tag-Vervollständigung ---

    #[test]
    fn klammern_und_anfuehrungszeichen_werden_ergaenzt() {
        assert_eq!(auto_pair(Fmt::Json, "", 0, '{'), Some('}'));
        assert_eq!(auto_pair(Fmt::Json, "", 0, '['), Some(']'));
        assert_eq!(auto_pair(Fmt::Json, "", 0, '"'), Some('"'));
        assert_eq!(auto_pair(Fmt::Json, "x", 1, '}'), None);
        assert_eq!(auto_pair(Fmt::Yaml, "", 0, '\''), Some('\''));
        // In XML nur innerhalb eines Tags.
        assert_eq!(auto_pair(Fmt::Xml, "<a ", 3, '"'), Some('"'));
        assert_eq!(auto_pair(Fmt::Xml, "text ", 5, '"'), None);
    }

    #[test]
    fn schliessende_zeichen_werden_erkannt() {
        assert!(is_closing_char(Fmt::Json, '}'));
        assert!(is_closing_char(Fmt::Json, '"'));
        assert!(!is_closing_char(Fmt::Json, '{'));
        assert!(is_closing_char(Fmt::Yaml, '\''));
        assert!(is_closing_char(Fmt::Xml, '"'));
        assert!(!is_closing_char(Fmt::Xml, '>'));
    }

    #[test]
    fn tag_erkennung_fuer_vervollstaendigung() {
        let tag = |t: &str| open_tag_name(t, t.len()).map(str::to_owned);
        assert_eq!(tag("<a>"), Some("a".to_string()));
        assert_eq!(tag("<a x=\"1\">"), Some("a".to_string()));
        assert_eq!(tag("<ns:a>"), Some("ns:a".to_string()));
        assert_eq!(tag("<br/>"), None);
        assert_eq!(tag("</a>"), None);
        assert_eq!(tag("<!-- c -->"), None);
        assert_eq!(tag("<?xml?>"), None);
        assert_eq!(tag("<a>text<b>"), Some("b".to_string()));
        assert_eq!(tag("<a>text</b>"), None);
        assert_eq!(tag("<a> x >"), None);
        assert_eq!(tag("<a>text"), None);
    }

    #[test]
    fn inside_tag_erkennt_tag_bereich() {
        assert!(inside_tag("<a ", 3));
        assert!(inside_tag("<a x=", 5));
        assert!(!inside_tag("<a>", 3));
        assert!(!inside_tag("text", 4));
        assert!(inside_tag("text <a", 7));
    }

    /// Simuliert das Tippen von Zeichen wie im Editor (ohne egui).
    fn type_seq(fmt: Fmt, mut text: String, mut cursor: usize, keys: &str) -> (String, usize) {
        for ch in keys.chars() {
            match apply_char(fmt, &text, cursor, cursor, ch) {
                Some(t) => {
                    text = t.text;
                    cursor = t.cursor;
                }
                None => {
                    let b = byte_index(&text, cursor);
                    text.insert(b, ch);
                    cursor += 1;
                }
            }
        }
        (text, cursor)
    }

    #[test]
    fn json_tippen_erzeugt_vollstaendiges_dokument() {
        let (text, cursor) = type_seq(Fmt::Json, String::new(), 0, "{\"a\": 1}");
        assert_eq!(text, "{\"a\": 1}");
        assert_eq!(cursor, text.chars().count());

        let (text, cursor) = type_seq(Fmt::Json, String::new(), 0, "[1, 2]");
        assert_eq!(text, "[1, 2]");
        assert_eq!(cursor, 6);
    }

    #[test]
    fn json_anfuehrungszeichen_ueberspringt_vorhandenes() {
        // Erstes " ergänzt das Paar, das zweite überspringt es.
        let (text, _) = type_seq(Fmt::Json, String::new(), 0, "\"");
        assert_eq!(text, "\"\"");
        let (text, cursor) = type_seq(Fmt::Json, "\"\"".into(), 1, "\"");
        assert_eq!(text, "\"\"");
        assert_eq!(cursor, 2);
    }

    #[test]
    fn xml_tag_wird_vervollstaendigt() {
        let (text, cursor) = type_seq(Fmt::Xml, String::new(), 0, "<a>");
        assert_eq!(text, "<a></a>");
        assert_eq!(cursor, 3);
        // Text zwischen den Tags landet innerhalb.
        let (text, cursor) = type_seq(Fmt::Xml, text, cursor, "hallo");
        assert_eq!(text, "<a>hallo</a>");
        assert_eq!(cursor, 8);
        // Selbstschließende Tags und schließende Tags bleiben unverändert.
        let (text, _) = type_seq(Fmt::Xml, "<br/>".into(), 5, "");
        assert_eq!(text, "<br/>");
        let (text, _) = type_seq(Fmt::Xml, String::new(), 0, "</a>");
        assert_eq!(text, "</a>");
    }

    #[test]
    fn xml_anfuehrungszeichen_nur_im_tag() {
        let (text, _) = type_seq(Fmt::Xml, "<a ".into(), 3, "\"");
        assert_eq!(text, "<a \"\"");
        // Im Textinhalt wird kein Paar ergänzt.
        let (text, cursor) = type_seq(Fmt::Xml, "<a>".into(), 3, "\"x");
        assert_eq!(text, "<a>\"x");
        assert_eq!(cursor, 5);
    }

    #[test]
    fn yaml_flow_klammern_werden_ergaenzt() {
        let (text, cursor) = type_seq(Fmt::Yaml, String::new(), 0, "a: [1, 2]");
        assert_eq!(text, "a: [1, 2]");
        assert_eq!(cursor, text.chars().count());
        let (text, _) = type_seq(Fmt::Yaml, String::new(), 0, "'x'");
        assert_eq!(text, "'x'");
    }

    #[test]
    fn loeschen_entfernt_leeres_paar() {
        let backspace = |text: &str, cursor: usize| {
            apply_backspace(Fmt::Json, text, cursor, cursor).map(|t| (t.text, t.cursor))
        };
        assert_eq!(backspace("{}", 1), Some((String::new(), 0)));
        assert_eq!(backspace("\"\"", 1), Some((String::new(), 0)));
        // Nur bei wirklich leeren Paaren.
        assert_eq!(backspace("{a}", 1), None);
        assert_eq!(backspace("{}", 0), None);
        // Auswahl wird nicht angetastet.
        assert_eq!(apply_backspace(Fmt::Json, "{}", 0, 2), None);
    }

    #[test]
    fn auswahl_wird_umschlossen() {
        let t = apply_char(Fmt::Json, "abc", 0, 3, '{').unwrap();
        assert_eq!(t.text, "{abc}");
        assert_eq!(t.cursor, 4);
        let t = apply_char(Fmt::Json, "abc", 0, 3, '"').unwrap();
        assert_eq!(t.text, "\"abc\"");
    }

    /// Baut einen App-Zustand mit fokussiertem Editor und einem Eingabe-Ereignis.
    fn app_with_event(event: egui::Event) -> (egui::Context, App) {
        let ctx = egui::Context::default();
        let id = egui::Id::new("test-editor");
        let mut app = App::default();
        app.editor_id = Some(id);
        ctx.memory_mut(|m| m.request_focus(id));
        ctx.begin_pass(egui::RawInput {
            events: vec![event],
            ..Default::default()
        });
        (ctx, app)
    }

    fn key_event(key: Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn getippte_klammer_wird_vor_dem_editor_abgefangen() {
        let (ctx, mut app) = app_with_event(egui::Event::Text("{".into()));
        assert!(app.handle_typing(&ctx, 0.0));
        assert_eq!(app.text, "{}");
        assert_eq!(app.cursor, 1);
        assert!(app.restore_cursor);
        // Das Ereignis darf den Editor nicht mehr erreichen.
        assert_eq!(ctx.input(|i| i.events.len()), 0);
        let _ = ctx.end_pass();
    }

    #[test]
    fn normales_zeichen_bleibt_fuer_den_editor() {
        let (ctx, mut app) = app_with_event(egui::Event::Text("x".into()));
        assert!(!app.handle_typing(&ctx, 0.0));
        assert_eq!(app.text, "");
        assert_eq!(ctx.input(|i| i.events.len()), 1);
        let _ = ctx.end_pass();
    }

    #[test]
    fn enter_loest_einrueckung_aus() {
        let (ctx, mut app) = app_with_event(key_event(Key::Enter));
        app.text = "{\n  \"a\": {".into();
        app.cursor = app.text.chars().count();
        assert!(app.auto_indent(&ctx, 0.0));
        assert_eq!(app.text, "{\n  \"a\": {\n    ");
        assert_eq!(ctx.input(|i| i.events.len()), 0);
        let _ = ctx.end_pass();
    }

    #[test]
    fn backspace_entfernt_leeres_paar_im_editor() {
        let (ctx, mut app) = app_with_event(key_event(Key::Backspace));
        app.text = "{}".into();
        app.cursor = 1;
        assert!(app.handle_typing(&ctx, 0.0));
        assert_eq!(app.text, "");
        assert_eq!(app.cursor, 0);
        let _ = ctx.end_pass();
    }

    // --- Erkennung geänderter Dateien ---

    /// Legt eine temporäre Datei an und liefert Pfad und Aufräumfunktion.
    fn temp_file(name: &str, content: &str) -> (PathBuf, impl FnOnce()) {
        let dir = std::env::temp_dir().join(format!("parsix-test-{}-{}", std::process::id(), name));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("datei.json");
        std::fs::write(&path, content).unwrap();
        (path.clone(), move || {
            let _ = std::fs::remove_dir_all(&dir);
        })
    }

    #[test]
    fn disk_state_erkennt_aenderungen() {
        let (path, cleanup) = temp_file("disk", "{}");
        let before = DiskState::of(&path).unwrap();
        std::fs::write(&path, "{\"a\": 1}").unwrap();
        let after = DiskState::of(&path).unwrap();
        assert_ne!(before, after);
        assert_eq!(DiskState::of(&path), Some(after));
        cleanup();
    }

    #[test]
    fn externe_aenderung_setzt_hinweis() {
        let (path, cleanup) = temp_file("extern", "1");
        let mut app = App::default();
        app.set_loaded(path.clone(), "1".into());
        assert!(!app.external_change);
        assert!(!app.is_modified());

        // Inhalt außerhalb von Parsix ändern und Prüfung auslösen.
        std::fs::write(&path, "22").unwrap();
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        app.last_poll = 0.0;
        app.check_disk(&ctx, 10.0);
        assert!(app.external_change);

        // Eine erneute Prüfung meldet dieselbe Änderung nicht noch einmal.
        app.external_change = false;
        app.check_disk(&ctx, 20.0);
        assert!(!app.external_change);
        let _ = ctx.end_pass();
        cleanup();
    }

    #[test]
    fn speichern_bei_externer_aenderung_wird_blockiert() {
        let (path, cleanup) = temp_file("block", "1");
        let mut app = App::default();
        app.set_loaded(path.clone(), "1".into());
        app.text = "lokal".into();
        app.external_change = true;

        app.save();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "1");
        assert!(app.msg.contains("außerhalb"));

        // Bewusstes Überschreiben funktioniert danach.
        app.overwrite();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "lokal");
        assert!(!app.external_change);
        assert!(!app.is_modified());
        cleanup();
    }

    #[test]
    fn neu_laden_uebernimmt_den_dateiinhalt() {
        let (path, cleanup) = temp_file("reload", "alt");
        let mut app = App::default();
        app.set_loaded(path.clone(), "alt".into());
        app.text = "ungespeichert".into();
        assert!(app.is_modified());

        std::fs::write(&path, "neu").unwrap();
        app.reload();
        assert_eq!(app.text, "neu");
        assert!(!app.is_modified());
        assert!(!app.external_change);
        assert!(!app.history.can_undo());
        cleanup();
    }

    #[test]
    fn fenstertitel_zeigt_ungespeicherte_aenderungen() {
        let mut app = App::default();
        assert_eq!(app.window_title(), "Parsix – Unbenannt");
        app.path = Some(PathBuf::from("C:/tmp/daten.json"));
        assert_eq!(app.window_title(), "Parsix – daten.json");
        app.text = "x".into();
        assert_eq!(app.window_title(), "Parsix – daten.json *");
    }

    #[test]
    fn format_wird_aus_der_endung_erkannt() {
        assert_eq!(App::fmt_for_path(Path::new("a.JSON"), Fmt::Yaml), Fmt::Json);
        assert_eq!(App::fmt_for_path(Path::new("a.yml"), Fmt::Json), Fmt::Yaml);
        assert_eq!(App::fmt_for_path(Path::new("a.xml"), Fmt::Json), Fmt::Xml);
        assert_eq!(App::fmt_for_path(Path::new("a.txt"), Fmt::Yaml), Fmt::Yaml);
    }

    // --- Fehlerdiagnose mit Zeilen- und Spaltenangaben ---

    #[test]
    fn json_fehler_hat_zeile_und_spalte() {
        let e = format_json("{\"a\": }", &def_opts()).unwrap_err();
        assert_eq!(e.msg, "expected value");
        assert_eq!(e.pos, Some(Pos { line: 1, col: 7 }));
    }

    #[test]
    fn json_fehler_mehrzeilig_hat_korrekte_position() {
        let e = format_json("{\n  \"a\": 1,\n  \"b\": \n}", &def_opts()).unwrap_err();
        assert_eq!(e.pos, Some(Pos { line: 4, col: 1 }));
    }

    #[test]
    fn yaml_fehler_hat_position_ohne_doppelte_angabe() {
        let e = format_yaml("a: 1\nb: [1, 2\n", &def_opts()).unwrap_err();
        assert_eq!(e.pos, Some(Pos { line: 3, col: 1 }));
        assert!(!e.msg.contains(" at line"));
        assert!(!e.msg.contains(" at position"));
        assert!(e.msg.contains("expected"));
    }

    #[test]
    fn xml_fehler_hat_zeile_und_spalte() {
        let e = format_xml("<a>\n  <b></c>\n</a>", &def_opts()).unwrap_err();
        assert_eq!(e.pos, Some(Pos { line: 2, col: 6 }));
        assert!(!e.msg.contains(" at line"));
    }

    #[test]
    fn pos_at_byte_ist_unicode_sicher() {
        let text = "ä\nöü";
        assert_eq!(pos_at_byte(text, 0), Pos { line: 1, col: 1 });
        assert_eq!(pos_at_byte(text, 2), Pos { line: 1, col: 2 });
        assert_eq!(pos_at_byte(text, 3), Pos { line: 2, col: 1 });
        assert_eq!(pos_at_byte(text, 5), Pos { line: 2, col: 2 });
        // Mitten in einem Zeichen: auf die Zeichengrenze zurückgehen.
        assert_eq!(pos_at_byte(text, 6), Pos { line: 2, col: 2 });
        assert_eq!(pos_at_byte(text, 99), Pos { line: 2, col: 3 });
    }

    #[test]
    fn char_index_at_liefert_zeichenindex() {
        let text = "ab\ncd";
        assert_eq!(char_index_at(text, Pos { line: 1, col: 1 }), 0);
        assert_eq!(char_index_at(text, Pos { line: 1, col: 3 }), 2);
        assert_eq!(char_index_at(text, Pos { line: 2, col: 1 }), 3);
        assert_eq!(char_index_at(text, Pos { line: 2, col: 3 }), 5);
        // Spalte hinter dem Zeilenende wird begrenzt.
        assert_eq!(char_index_at(text, Pos { line: 2, col: 10 }), 5);
        // Zeile hinter dem Textende liefert das Textende.
        assert_eq!(char_index_at(text, Pos { line: 9, col: 1 }), 5);
    }

    #[test]
    fn strip_positions_entfernt_alle_angaben() {
        assert_eq!(strip_positions("kaputt"), "kaputt");
        assert_eq!(strip_positions("kaputt at line 1 column 2"), "kaputt");
        assert_eq!(
            strip_positions("kaputt at line 1 column 2, Kontext at line 3 column 4"),
            "kaputt, Kontext"
        );
        assert_eq!(strip_positions("kaputt at position 12"), "kaputt");
        // Text mit „ at “ ohne Positionsangabe bleibt erhalten.
        assert_eq!(strip_positions("look at this"), "look at this");
    }

    #[test]
    fn formatieren_setzt_cursor_an_die_fehlerstelle() {
        let mut app = App::default();
        app.fmt = Fmt::Json;
        app.text = "{\"a\": }".into();
        app.format();
        assert_eq!(app.err_pos, Some(Pos { line: 1, col: 7 }));
        assert_eq!(app.cursor, 6);
        assert!(app.restore_cursor);
        assert!(app.jump);
        assert!(app.msg.contains("Zeile 1, Spalte 7"));

        // Erfolgreiches Formatieren räumt die Fehleranzeige ab.
        app.text = "{\"a\": 1}".into();
        app.format();
        assert_eq!(app.err_pos, None);
        assert_eq!(app.msg, "OK");
    }

    // --- Zeilen markieren, kopieren und verschieben ---

    #[test]
    fn select_line_umfasst_die_ganze_zeile() {
        let text = "ab\ncd\nef";
        assert_eq!(select_line(text, 0), (0, 3));
        assert_eq!(select_line(text, 1), (0, 3));
        assert_eq!(select_line(text, 3), (3, 6));
        assert_eq!(select_line(text, 5), (3, 6));
        assert_eq!(select_line(text, 8), (6, 8));
        assert_eq!(select_line("", 0), (0, 0));
        // Abschließender Umbruch: die letzte Zeile ist leer.
        assert_eq!(select_line("ab\n", 3), (3, 3));
        // Unicode: Zeichen-, nicht Byte-Indizes.
        assert_eq!(select_line("ä\nöü", 2), (2, 4));
    }

    #[test]
    fn aktuelle_zeile_als_text() {
        assert_eq!(current_line_text("ab\ncd", 1), "ab\n");
        assert_eq!(current_line_text("ab\ncd", 4), "cd");
        assert_eq!(current_line_text("ab\n", 3), "");
    }

    #[test]
    fn zeile_nach_oben_verschieben() {
        let m = move_lines("A\nB\nC", 3, 3, true).unwrap();
        assert_eq!(m.text, "B\nA\nC");
        // Cursor bleibt in derselben Spalte.
        assert_eq!(m.sel, (1, 1));
        // Die erste Zeile kann nicht weiter nach oben.
        assert!(move_lines("A\nB\nC", 0, 0, true).is_none());
    }

    #[test]
    fn zeile_nach_unten_verschieben() {
        let m = move_lines("A\nB\nC", 2, 2, false).unwrap();
        assert_eq!(m.text, "A\nC\nB");
        assert_eq!(m.sel, (4, 4));
        // Die letzte Zeile kann nicht weiter nach unten.
        assert!(move_lines("A\nB\nC", 4, 4, false).is_none());
    }

    #[test]
    fn mehrere_zeilen_verschieben_behaelt_die_auswahl() {
        // Auswahl umfasst B und C.
        let m = move_lines("A\nB\nC\nD", 2, 6, true).unwrap();
        assert_eq!(m.text, "B\nC\nA\nD");
        assert_eq!(m.sel, (0, 4));

        let m = move_lines("A\nB\nC\nD", 2, 6, false).unwrap();
        assert_eq!(m.text, "A\nD\nB\nC");
        assert_eq!(m.sel, (4, 7));
    }

    #[test]
    fn auswahl_bis_zeilenanfang_schliesst_die_naechste_zeile_nicht_ein() {
        // Auswahl "B\n" (2..4): nur B wird verschoben, nicht C.
        let m = move_lines("A\nB\nC\nD\nE", 2, 4, false).unwrap();
        assert_eq!(m.text, "A\nC\nB\nD\nE");
        assert_eq!(m.sel, (4, 6));
    }

    #[test]
    fn verschieben_erhaelt_abschliessenden_umbruch() {
        let m = move_lines("A\nB\n", 3, 3, true).unwrap();
        assert_eq!(m.text, "B\nA\n");
        assert_eq!(m.sel, (1, 1));
    }

    #[test]
    fn verschieben_im_leeren_dokument_aendert_nichts() {
        assert!(move_lines("", 0, 0, true).is_none());
        assert!(move_lines("", 0, 0, false).is_none());
        assert!(move_lines("A", 0, 0, true).is_none());
        assert!(move_lines("A", 0, 0, false).is_none());
    }

    fn key_event_mod(key: Key, modifiers: Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    /// Baut einen App-Zustand mit gespeicherter Auswahl und einem Ereignis.
    fn app_with_selection(
        event: egui::Event,
        text: &str,
        start: usize,
        end: usize,
    ) -> (egui::Context, App) {
        let ctx = egui::Context::default();
        let id = egui::Id::new("test-editor");
        let mut app = App::default();
        app.editor_id = Some(id);
        app.text = text.to_owned();
        app.cursor = end;
        ctx.memory_mut(|m| m.request_focus(id));
        let mut state = egui::text_edit::TextEditState::default();
        state.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(start),
            CCursor::new(end),
        )));
        state.store(&ctx, id);
        ctx.begin_pass(egui::RawInput {
            events: vec![event],
            ..Default::default()
        });
        (ctx, app)
    }

    #[test]
    fn strg_l_markiert_die_aktuelle_zeile() {
        let (ctx, mut app) =
            app_with_selection(key_event_mod(Key::L, Modifiers::COMMAND), "ab\ncd", 4, 4);
        assert!(app.handle_line_ops(&ctx, 0.0));
        assert_eq!(app.restore_sel, Some((3, 5)));
        assert_eq!(app.cursor, 5);
        let _ = ctx.end_pass();
    }

    #[test]
    fn alt_oben_verschiebt_die_zeile_im_editor() {
        let (ctx, mut app) =
            app_with_selection(key_event_mod(Key::ArrowUp, Modifiers::ALT), "A\nB\nC", 3, 3);
        assert!(app.handle_line_ops(&ctx, 0.0));
        assert_eq!(app.text, "B\nA\nC");
        assert_eq!(app.restore_sel, Some((1, 1)));
        assert!(app.history.can_undo());
        let _ = ctx.end_pass();
    }

    #[test]
    fn alt_oben_in_der_ersten_zeile_aendert_nichts() {
        let (ctx, mut app) =
            app_with_selection(key_event_mod(Key::ArrowUp, Modifiers::ALT), "A\nB", 0, 0);
        assert!(app.handle_line_ops(&ctx, 0.0));
        assert_eq!(app.text, "A\nB");
        assert_eq!(app.restore_sel, None);
        let _ = ctx.end_pass();
    }

    #[test]
    fn strg_c_ohne_auswahl_kopiert_die_zeile() {
        let (ctx, mut app) = app_with_selection(egui::Event::Copy, "ab\ncd", 4, 4);
        assert!(app.handle_line_ops(&ctx, 0.0));
        assert!(app.msg.contains("kopiert"));
        // Das Ereignis wurde entfernt, damit der Editor nicht zusätzlich reagiert.
        assert_eq!(ctx.input(|i| i.events.len()), 0);
        let copied = ctx.output(|o| {
            o.commands.iter().find_map(|c| match c {
                egui::OutputCommand::CopyText(t) => Some(t.clone()),
                _ => None,
            })
        });
        assert_eq!(copied.as_deref(), Some("cd"));
        let _ = ctx.end_pass();
    }

    #[test]
    fn strg_c_mit_auswahl_bleibt_fuer_den_editor() {
        let (ctx, mut app) = app_with_selection(egui::Event::Copy, "ab\ncd", 0, 2);
        assert!(!app.handle_line_ops(&ctx, 0.0));
        assert_eq!(ctx.input(|i| i.events.len()), 1);
        let _ = ctx.end_pass();
    }

    // --- Statusleiste mit Zeilen- und Spaltenposition ---

    #[test]
    fn line_col_liefert_zeile_und_spalte() {
        let text = "ab\ncd";
        assert_eq!(line_col(text, 0), Pos { line: 1, col: 1 });
        assert_eq!(line_col(text, 1), Pos { line: 1, col: 2 });
        assert_eq!(line_col(text, 2), Pos { line: 1, col: 3 });
        assert_eq!(line_col(text, 3), Pos { line: 2, col: 1 });
        assert_eq!(line_col(text, 4), Pos { line: 2, col: 2 });
        assert_eq!(line_col(text, 5), Pos { line: 2, col: 3 });
        assert_eq!(line_col("", 0), Pos { line: 1, col: 1 });
        assert_eq!(line_col("ab\n", 3), Pos { line: 2, col: 1 });
        // Unicode zählt Zeichen, nicht Bytes.
        assert_eq!(line_col("ä\nöü", 3), Pos { line: 2, col: 2 });
        // Positionen hinter dem Textende werden begrenzt.
        assert_eq!(line_col(text, 99), Pos { line: 2, col: 3 });
    }

    #[test]
    fn line_col_und_char_index_sind_zueinander_invers() {
        let text = "{\n  \"a\": 1,\n\n  \"b\": [2, 3]\n}";
        for cursor in 0..=text.chars().count() {
            assert_eq!(char_index_at(text, line_col(text, cursor)), cursor);
        }
    }

    #[test]
    fn formatnamen_fuer_die_statusleiste() {
        assert_eq!(Fmt::Json.name(), "JSON");
        assert_eq!(Fmt::Xml.name(), "XML");
        assert_eq!(Fmt::Yaml.name(), "YAML");
    }

    // --- YAML-Kommentare und Quelltexterhaltung ---

    #[test]
    fn yaml_scan_findet_kommentare_ausserhalb_von_zeichenketten() {
        let (c, s) = scan_yaml_line("a: 1 # x", ScanState::default());
        assert_eq!(c, Some(5));
        assert_eq!(s.flow, 0);
        assert_eq!(
            scan_yaml_line("a: \"# kein\"", ScanState::default()).0,
            None
        );
        assert_eq!(scan_yaml_line("a: '# kein'", ScanState::default()).0, None);
        let (c, s) = scan_yaml_line("a: [1, # x", ScanState::default());
        assert_eq!(c, Some(7));
        assert_eq!(s.flow, 1);
        // Eine Raute ohne vorangehendes Leerzeichen beginnt keinen Kommentar.
        assert_eq!(scan_yaml_line("a: b#c", ScanState::default()).0, None);
    }

    #[test]
    fn yaml_erkennt_block_skalare() {
        assert!(starts_block_scalar("key: |"));
        assert!(starts_block_scalar("key: >-"));
        assert!(starts_block_scalar("key: |2"));
        assert!(starts_block_scalar("key: |+"));
        assert!(starts_block_scalar("- |"));
        assert!(starts_block_scalar("? >"));
        assert!(starts_block_scalar("key: &a |"));
        assert!(!starts_block_scalar("key: a |"));
        assert!(!starts_block_scalar("key: \"a |\""));
        assert!(!starts_block_scalar("key: value"));
    }

    #[test]
    fn yaml_shift_spaces_verschiebt_die_einrueckung() {
        assert_eq!(shift_spaces("    a", 2), "      a");
        assert_eq!(shift_spaces("    a", -2), "  a");
        assert_eq!(shift_spaces("  a", -5), "a");
        assert_eq!(shift_spaces("a", 0), "a");
    }

    #[test]
    fn yaml_behaelt_inline_und_eigene_kommentare() {
        let src = "# Kopfkommentar\na: 1  # inline\n# zu b\nb: 2\n";
        assert_eq!(format_yaml_source(src, &def_opts()), src);
    }

    #[test]
    fn yaml_behaelt_leerzeilen() {
        let src = "a: 1\n\n# Abschnitt\n\nb: 2\n";
        assert_eq!(format_yaml_source(src, &def_opts()), src);
    }

    #[test]
    fn yaml_normalisiert_einrueckung_auf_zwei_leerzeichen() {
        let src = "root:\n    child:\n        leaf: 1\n    sibling: 2\n";
        let out = format_yaml_source(src, &def_opts());
        assert_eq!(out, "root:\n  child:\n    leaf: 1\n  sibling: 2\n");
    }

    #[test]
    fn yaml_normalisiert_sequenzen() {
        let src = "items:\n    - a\n    - b\n    -\n        - c\n";
        let out = format_yaml_source(src, &def_opts());
        assert_eq!(out, "items:\n  - a\n  - b\n  -\n    - c\n");
    }

    #[test]
    fn yaml_block_skalar_inhalt_bleibt_unveraendert() {
        let src = "key: |\n    line one\n      indented\n    line two\nnext: 1\n";
        assert_eq!(format_yaml_source(src, &def_opts()), src);
    }

    #[test]
    fn yaml_block_skalar_wird_mit_der_indikatorzeile_verschoben() {
        let src = "root:\n    text: |\n        line\n        more\n";
        let out = format_yaml_source(src, &def_opts());
        assert_eq!(out, "root:\n  text: |\n      line\n      more\n");
    }

    #[test]
    fn yaml_mehrzeilige_zeichenkette_bleibt_unveraendert() {
        let src = "key: \"line one\n    line two\"\nnext: 1\n";
        assert_eq!(format_yaml_source(src, &def_opts()), src);
    }

    #[test]
    fn yaml_anker_und_aliase_bleiben_erhalten() {
        let src = "base: &b\n    x: 1\nother: *b\n";
        let out = format_yaml_source(src, &def_opts());
        assert_eq!(out, "base: &b\n  x: 1\nother: *b\n");
    }

    #[test]
    fn yaml_pipe_im_klartext_ist_kein_block_skalar() {
        // "a |" ist Teil des Klartextwerts, kein Block-Skalar.
        let src = "key: a |\n    text\n";
        let out = format_yaml_source(src, &def_opts());
        assert_eq!(out, "key: a |\n  text\n");
    }

    #[test]
    fn yaml_raute_in_zeichenketten_ist_kein_kommentar() {
        let src = "a: \"x # y\"\nb: 'z # w'\n";
        assert_eq!(format_yaml_source(src, &def_opts()), src);
    }

    #[test]
    fn yaml_crlf_wird_normalisiert_und_abschluss_ergaenzt() {
        assert_eq!(
            format_yaml_source("a: 1\r\nb: 2", &def_opts()),
            "a: 1\nb: 2\n"
        );
    }

    #[test]
    fn yaml_mehrzeilige_flow_sammlung_bleibt_erhalten() {
        let src = "a: [\n    1,\n    2\n]\n";
        assert_eq!(format_yaml_source(src, &def_opts()), src);
    }

    #[test]
    fn yaml_kommentar_vor_erstem_inhalt_bleibt_links() {
        let src = "  # Kopf\n    a: 1\n";
        assert_eq!(format_yaml_source(src, &def_opts()), "# Kopf\na: 1\n");
    }

    #[test]
    fn yaml_kommentar_richtet_sich_an_der_ebene_aus() {
        let src = "root:\n    child: 1\n    # Notiz\n";
        let out = format_yaml_source(src, &def_opts());
        assert_eq!(out, "root:\n  child: 1\n  # Notiz\n");
    }

    #[test]
    fn yaml_formatierung_erhaelt_die_daten() {
        let src = "\
# Konfiguration
name: demo  # Name
liste:
    - eins
    - zwei
nested:
    a: 1
    b:
        - x: 1
          y: 2
text: |
    Zeile 1
      eingerueckt
    Zeile 3
anker: &a
    k: v
alias: *a
";
        let out = format_yaml_source(src, &def_opts());
        let orig: serde_yaml_ng::Value = serde_yaml_ng::from_str(src).unwrap();
        let neu: serde_yaml_ng::Value = serde_yaml_ng::from_str(&out).unwrap();
        assert_eq!(orig, neu);
        assert!(out.contains("# Konfiguration"));
        assert!(out.contains("# Name"));
        assert!(out.contains("Zeile 1"));
        assert!(out.contains("eingerueckt"));
        assert!(out.contains("&a"));
        assert!(out.contains("*a"));
        // Die Einrückung wurde tatsächlich normalisiert.
        assert!(out.contains("liste:\n  - eins"));
        assert!(out.contains("  b:\n    - x: 1"));
        assert!(!out.contains("    - eins"));
    }

    // --- Erweiterte Suche ---

    fn opts(case_sensitive: bool, whole_word: bool, use_regex: bool) -> SearchOpts {
        SearchOpts {
            case_sensitive,
            whole_word,
            use_regex,
        }
    }

    #[test]
    fn suche_ignoriert_standardmaessig_die_schreibweise() {
        let m = find_all("Foo foo FOO", "foo", SearchOpts::default()).unwrap();
        assert_eq!(m, vec![(0, 3), (4, 7), (8, 11)]);
    }

    #[test]
    fn suche_mit_schreibweise_unterscheidet_gross_und_klein() {
        let m = find_all("Foo foo FOO", "foo", opts(true, false, false)).unwrap();
        assert_eq!(m, vec![(4, 7)]);
        let m = find_all("Foo foo FOO", "Foo", opts(true, false, false)).unwrap();
        assert_eq!(m, vec![(0, 3)]);
    }

    #[test]
    fn suche_ignoriert_sonderzeichen_im_literalmodus() {
        // "." ist im Literalmodus ein Punkt und kein Platzhalter.
        let m = find_all("a.b axb", "a.b", SearchOpts::default()).unwrap();
        assert_eq!(m, vec![(0, 3)]);
        // "[" ist kein kaputter regulärer Ausdruck.
        assert!(find_all("[x]", "[x]", SearchOpts::default()).is_ok());
    }

    #[test]
    fn suche_nach_ganzen_woertern() {
        let m = find_all("cat scatter cat.", "cat", opts(false, true, false)).unwrap();
        assert_eq!(m, vec![(0, 3), (12, 15)]);
        // Unterstriche zählen als Wortzeichen.
        let m = find_all("_cat cat_ cat", "cat", opts(false, true, false)).unwrap();
        assert_eq!(m, vec![(10, 13)]);
    }

    #[test]
    fn ganze_woerter_bei_nichtwort_mustern() {
        // Ein Bindestrich ist kein Wortzeichen, daher bleiben beide Treffer gültig.
        let m = find_all("a-b - c", "-", opts(false, true, false)).unwrap();
        assert_eq!(m, vec![(1, 2), (4, 5)]);
    }

    #[test]
    fn suche_mit_regulaerem_ausdruck() {
        let m = find_all("a1 b22", r"\d+", opts(false, false, true)).unwrap();
        assert_eq!(m, vec![(1, 2), (4, 6)]);
        let m = find_all("a1 b22", r"[a-z]\d+", opts(false, false, true)).unwrap();
        assert_eq!(m, vec![(0, 2), (3, 6)]);
    }

    #[test]
    fn regulaerer_ausdruck_ist_standardmaessig_gross_klein_neutral() {
        let m = find_all("Foo foo", "foo", opts(false, false, true)).unwrap();
        assert_eq!(m.len(), 2);
        let m = find_all("Foo foo", "foo", opts(true, false, true)).unwrap();
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn ungueltiger_regulaerer_ausdruck_liefert_fehler() {
        assert!(find_all("abc", "(", opts(false, false, true)).is_err());
    }

    #[test]
    fn leere_treffer_werden_uebersprungen() {
        // "^" und "a*" erzeugen leere Treffer, die nicht navigierbar sind.
        assert!(find_all("abc", "^", opts(false, false, true))
            .unwrap()
            .is_empty());
        assert_eq!(
            find_all("abc", "a*", opts(false, false, true)).unwrap(),
            vec![(0, 1)]
        );
    }

    #[test]
    fn unicode_suche_ist_gross_klein_neutral() {
        let m = find_all("Ärger ärger", "ärger", SearchOpts::default()).unwrap();
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn leeres_muster_findet_nichts() {
        assert!(find_all("abc", "", SearchOpts::default())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn ersetzen_im_literalmodus_expandiert_keine_rueckverweise() {
        let out = replace_all_matches("a", "a", "$1", SearchOpts::default()).unwrap();
        assert_eq!(out, "$1");
        // Der Punkt bleibt ein Punkt.
        let out = replace_all_matches("a.b axb", "a.b", "X", SearchOpts::default()).unwrap();
        assert_eq!(out, "X axb");
    }

    #[test]
    fn ersetzen_mit_regulaerem_ausdruck_und_rueckverweisen() {
        let out =
            replace_all_matches("a1 b2", r"([a-z])(\d)", "$2$1", opts(false, false, true)).unwrap();
        assert_eq!(out, "1a 2b");
    }

    #[test]
    fn ersetzen_nach_ganzen_woertern() {
        let out =
            replace_all_matches("cat scatter cat", "cat", "dog", opts(false, true, false)).unwrap();
        assert_eq!(out, "dog scatter dog");
    }

    #[test]
    fn find_next_springt_zum_naechsten_treffer_und_beginnt_von_vorn() {
        let mut app = App::default();
        app.text = "a a a".into();
        app.find = "a".into();
        app.search_dirty = true;

        app.find_next();
        assert_eq!(app.match_range, Some((0, 1)));
        assert_eq!(app.match_idx, Some(0));
        app.find_next();
        assert_eq!(app.match_range, Some((2, 3)));
        app.find_next();
        assert_eq!(app.match_range, Some((4, 5)));
        // Am Ende wird von vorn begonnen.
        app.find_next();
        assert_eq!(app.match_range, Some((0, 1)));
    }

    #[test]
    fn find_prev_springt_zurueck_und_beginnt_hinten() {
        let mut app = App::default();
        app.text = "a a a".into();
        app.find = "a".into();
        app.search_dirty = true;

        // Ohne aktuellen Treffer wird der letzte gewählt.
        app.find_prev();
        assert_eq!(app.match_range, Some((4, 5)));
        app.find_prev();
        assert_eq!(app.match_range, Some((2, 3)));
        app.find_prev();
        assert_eq!(app.match_range, Some((0, 1)));
        // Am Anfang wird hinten begonnen.
        app.find_prev();
        assert_eq!(app.match_range, Some((4, 5)));
    }

    #[test]
    fn schreibweise_aendert_die_trefferliste() {
        let mut app = App::default();
        app.text = "Foo foo".into();
        app.find = "foo".into();
        app.search_dirty = true;
        app.refresh_matches();
        assert_eq!(app.matches.len(), 2);

        app.case_sensitive = true;
        app.search_dirty = true;
        app.refresh_matches();
        assert_eq!(app.matches, vec![(4, 7)]);
    }

    #[test]
    fn textaenderung_verwirft_die_treffer() {
        let mut app = App::default();
        app.text = "a".into();
        app.find = "a".into();
        app.search_dirty = true;
        app.refresh_matches();
        assert_eq!(app.matches.len(), 1);

        app.text = "bb".into();
        app.after_edit(0.0);
        app.refresh_matches();
        assert!(app.matches.is_empty());
        assert_eq!(app.match_range, None);
    }

    #[test]
    fn ersetzen_ersetzt_den_aktuellen_treffer_und_geht_weiter() {
        let mut app = App::default();
        app.text = "a a".into();
        app.find = "a".into();
        app.replace = "bb".into();
        app.search_dirty = true;

        app.find_next();
        app.replace_current();
        assert_eq!(app.text, "bb a");
        // Der nächste Treffer ist bereits ausgewählt.
        assert_eq!(app.match_range, Some((3, 4)));
    }

    #[test]
    fn ersetzen_mit_regex_und_rueckverweis_im_editor() {
        let mut app = App::default();
        app.text = "a1 b2".into();
        app.find = r"([a-z])(\d)".into();
        app.use_regex = true;
        app.replace = "$2$1".into();
        app.search_dirty = true;

        app.find_next();
        app.replace_current();
        assert_eq!(app.text, "1a b2");

        app.replace_all();
        assert_eq!(app.text, "1a 2b");
    }

    #[test]
    fn ersetzen_alle_beachtet_ganze_woerter() {
        let mut app = App::default();
        app.text = "cat scatter cat".into();
        app.find = "cat".into();
        app.replace = "dog".into();
        app.whole_word = true;
        app.search_dirty = true;

        app.replace_all();
        assert_eq!(app.text, "dog scatter dog");
    }

    #[test]
    fn ungueltiger_ausdruck_setzt_fehlermeldung() {
        let mut app = App::default();
        app.text = "abc".into();
        app.find = "(".into();
        app.use_regex = true;
        app.search_dirty = true;

        app.refresh_matches();
        assert!(app.search_error.is_some());
        assert!(app.matches.is_empty());
        assert_eq!(app.match_range, None);
    }

    // --- Erweiterte Formatierung ---

    fn fmt_opts(indent: Indent, minify: bool, sort_json_keys: bool) -> FormatOptions {
        FormatOptions {
            indent,
            minify,
            sort_json_keys,
        }
    }

    #[test]
    fn json_einrueckung_ist_konfigurierbar() {
        let src = r#"{"a":{"b":1},"c":[1,2]}"#;
        let two = format_json(src, &fmt_opts(Indent::Spaces2, false, false)).unwrap();
        assert_eq!(
            two,
            "{\n  \"a\": {\n    \"b\": 1\n  },\n  \"c\": [\n    1,\n    2\n  ]\n}\n"
        );

        let four = format_json(src, &fmt_opts(Indent::Spaces4, false, false)).unwrap();
        assert_eq!(
            four,
            "{\n    \"a\": {\n        \"b\": 1\n    },\n    \"c\": [\n        1,\n        2\n    ]\n}\n"
        );

        let tabs = format_json(src, &fmt_opts(Indent::Tabs, false, false)).unwrap();
        assert_eq!(
            tabs,
            "{\n\t\"a\": {\n\t\t\"b\": 1\n\t},\n\t\"c\": [\n\t\t1,\n\t\t2\n\t]\n}\n"
        );
    }

    #[test]
    fn json_minify_entfernt_leerraum() {
        let src = "{\n  \"a\": 1,\n  \"b\": [1, 2],\n  \"c\": {}\n}";
        let out = format_json(src, &fmt_opts(Indent::Spaces2, true, false)).unwrap();
        assert_eq!(out, "{\"a\":1,\"b\":[1,2],\"c\":{}}\n");
    }

    #[test]
    fn json_schluesselreihenfolge_bleibt_standardmaessig_erhalten() {
        let src = r#"{"b":1,"a":2}"#;
        let out = format_json(src, &def_opts()).unwrap();
        assert!(out.starts_with("{\n  \"b\": 1,\n  \"a\": 2\n}"));
        let out = format_json(src, &fmt_opts(Indent::Spaces2, true, false)).unwrap();
        assert_eq!(out, "{\"b\":1,\"a\":2}\n");
    }

    #[test]
    fn json_sortierung_ist_rekursiv_und_erhaelt_werte() {
        let src = r#"{"b":{"z":1,"a":2},"a":[{"d":4,"c":3}]}"#;
        let opts = fmt_opts(Indent::Spaces2, false, true);
        let out = format_json(src, &opts).unwrap();
        assert_eq!(
            out,
            "{\n  \"a\": [\n    {\n      \"c\": 3,\n      \"d\": 4\n    }\n  ],\n  \"b\": {\n    \"a\": 2,\n    \"z\": 1\n  }\n}\n"
        );
        // Die Werte bleiben unverändert.
        let orig: serde_json::Value = serde_json::from_str(src).unwrap();
        let neu: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(orig, neu);
    }

    #[test]
    fn json_ausgabe_endet_mit_genau_einem_umbruch() {
        let out = format_json("{\"a\": 1}\n\n", &def_opts()).unwrap();
        assert!(out.ends_with("}\n"));
        assert!(!out.ends_with("\n\n"));
    }

    #[test]
    fn xml_einrueckung_ist_konfigurierbar() {
        let src = "<root><a x=\"1\"><b/></a></root>";
        let four = format_xml(src, &fmt_opts(Indent::Spaces4, false, false)).unwrap();
        assert_eq!(
            four,
            "<root>\n    <a x=\"1\">\n        <b/>\n    </a>\n</root>\n"
        );

        let tabs = format_xml(src, &fmt_opts(Indent::Tabs, false, false)).unwrap();
        assert_eq!(tabs, "<root>\n\t<a x=\"1\">\n\t\t<b/>\n\t</a>\n</root>\n");
    }

    #[test]
    fn xml_minify_entfernt_leerraum() {
        let src = "<root>\n  <a x=\"1\">\n    <b/>\n  </a>\n</root>";
        let out = format_xml(src, &fmt_opts(Indent::Spaces2, true, false)).unwrap();
        assert_eq!(out, "<root><a x=\"1\"><b/></a></root>\n");
    }

    #[test]
    fn yaml_einrueckung_ist_konfigurierbar() {
        let src = "root:\n  child:\n    leaf: 1\n";
        let four = format_yaml_source(src, &fmt_opts(Indent::Spaces4, false, false));
        assert_eq!(four, "root:\n    child:\n        leaf: 1\n");
    }

    #[test]
    fn yaml_tabs_fallen_auf_zwei_leerzeichen_zurueck() {
        let src = "root:\n  child:\n    leaf: 1\n";
        let out = format_yaml_source(src, &fmt_opts(Indent::Tabs, false, false));
        assert_eq!(out, "root:\n  child:\n    leaf: 1\n");
    }

    #[test]
    fn yaml_minify_entfernt_kommentare_und_leerzeilen() {
        let src = "\
# Kopf
a: 1  # inline

# Abschnitt
root:
    child: 1
text: |
    # bleibt Inhalt
    Zeile
";
        let out = format_yaml_source(src, &fmt_opts(Indent::Spaces2, true, false));
        assert_eq!(
            out,
            "a: 1\nroot:\n child: 1\ntext: |\n    # bleibt Inhalt\n    Zeile\n"
        );
        // Die Daten bleiben trotz entfernter Kommentare identisch.
        let orig: serde_yaml_ng::Value = serde_yaml_ng::from_str(src).unwrap();
        let neu: serde_yaml_ng::Value = serde_yaml_ng::from_str(&out).unwrap();
        assert_eq!(orig, neu);
    }

    #[test]
    fn yaml_minify_mit_tabs_und_sequenzen() {
        let src = "root:\n    child:\n        leaf: 1\nliste:\n    - a\n    - b\n";
        let out = format_yaml_source(src, &fmt_opts(Indent::Tabs, true, false));
        assert_eq!(out, "root:\n child:\n  leaf: 1\nliste:\n - a\n - b\n");
        let orig: serde_yaml_ng::Value = serde_yaml_ng::from_str(src).unwrap();
        let neu: serde_yaml_ng::Value = serde_yaml_ng::from_str(&out).unwrap();
        assert_eq!(orig, neu);
    }

    #[test]
    fn yaml_minify_behaelt_block_skalar_leerzeilen() {
        let src = "text: |\n    Zeile 1\n\n    Zeile 2\n";
        let out = format_yaml_source(src, &fmt_opts(Indent::Spaces2, true, false));
        assert_eq!(out, "text: |\n    Zeile 1\n\n    Zeile 2\n");
    }

    #[test]
    fn alle_ausgaben_enden_mit_genau_einem_umbruch() {
        for out in [
            format_json("{\"a\": 1}", &def_opts()).unwrap(),
            format_xml("<a/>", &def_opts()).unwrap(),
            format_yaml("a: 1", &def_opts()).unwrap(),
        ] {
            assert!(out.ends_with('\n'), "{out:?}");
            assert!(!out.ends_with("\n\n"), "{out:?}");
            assert!(!out.contains('\r'), "{out:?}");
        }
    }

    #[test]
    fn standardprofile_sind_vorhanden() {
        let p = Profiles::default();
        assert_eq!(p.profiles.len(), 5);
        assert_eq!(p.active_index(), 0);
        assert_eq!(p.options(), FormatOptions::default());
        let names: Vec<&str> = p.profiles.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"Breit (4 Leerzeichen)"));
        assert!(names.contains(&"Kompakt"));
    }

    #[test]
    fn profil_hinzufuegen_lehnt_leere_und_doppelte_namen_ab() {
        let mut p = Profiles::default();
        assert!(!p.add("   ", FormatOptions::default()));
        assert!(!p.add("Tabs", FormatOptions::default()));
        assert!(p.add(
            "  Meins  ",
            FormatOptions {
                indent: Indent::Spaces4,
                ..Default::default()
            }
        ));
        assert_eq!(p.active().name, "Meins");
        assert_eq!(p.options().indent, Indent::Spaces4);
        assert_eq!(p.profiles.len(), 6);
    }

    #[test]
    fn profil_loeschen_haelt_mindestens_ein_profil() {
        let mut p = Profiles::default();
        let n = p.profiles.len();
        assert!(p.remove_active());
        assert_eq!(p.profiles.len(), n - 1);
        while p.profiles.len() > 1 {
            assert!(p.remove_active());
        }
        assert!(!p.remove_active());
        assert_eq!(p.profiles.len(), 1);
    }

    #[test]
    fn profile_normalisieren_ungueltige_indizes() {
        let mut p = Profiles::default();
        p.active = 99;
        p.normalize();
        assert!(p.active < p.profiles.len());
        p.profiles.clear();
        p.normalize();
        assert_eq!(p.profiles.len(), 5);
    }

    #[test]
    fn profile_serialisierung_erhaelt_alle_optionen() {
        let mut p = Profiles::default();
        p.set_options(fmt_opts(Indent::Tabs, true, true));
        let json = serde_json::to_string(&p).unwrap();
        let back: Profiles = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
        assert_eq!(back.options(), fmt_opts(Indent::Tabs, true, true));
    }

    #[test]
    fn profile_werden_gespeichert_und_geladen() {
        let (path, cleanup) = temp_file("profiles", "");
        let mut p = Profiles::default();
        assert!(p.add("Eigenes", fmt_opts(Indent::Spaces4, true, true)));
        save_profiles(&path, &p).unwrap();

        let loaded = load_profiles(&path);
        assert_eq!(loaded, p);
        assert_eq!(loaded.active().name, "Eigenes");
        assert_eq!(loaded.options(), fmt_opts(Indent::Spaces4, true, true));
        cleanup();
    }

    #[test]
    fn fehlende_profildatei_liefert_standardprofile() {
        let missing = std::env::temp_dir().join("parsix-gibt-es-nicht-4711.json");
        let loaded = load_profiles(&missing);
        assert_eq!(loaded, Profiles::default());
    }

    #[test]
    fn app_formatiert_mit_aktivem_profil() {
        let mut app = App::default();
        app.text = r#"{"a":{"b":1}}"#.into();
        app.profiles
            .set_options(fmt_opts(Indent::Spaces4, false, false));
        app.format();
        assert_eq!(app.msg, "OK");
        assert_eq!(app.text, "{\n    \"a\": {\n        \"b\": 1\n    }\n}\n");

        app.text = r#"{"a": 1}"#.into();
        app.profiles
            .set_options(fmt_opts(Indent::Spaces2, true, false));
        app.format();
        assert_eq!(app.text, "{\"a\":1}\n");
    }

    // --- Ribbon ---

    #[test]
    fn ribbon_startet_mit_der_datei_registerkarte() {
        let app = App::default();
        assert_eq!(app.ribbon_tab, RibbonTab::Datei);
        assert!(!app.ribbon_collapsed);
        assert!(!app.hide_status);
        assert!(!app.focus_replace);
    }

    #[test]
    fn ribbon_registerkarten_haben_feste_reihenfolge() {
        let names: Vec<&str> = RibbonTab::ALL.iter().map(|t| t.name()).collect();
        assert_eq!(
            names,
            [
                "Datei",
                "Bearbeiten",
                "Formatieren",
                "Vergleichen",
                "Ansicht"
            ]
        );
    }

    #[test]
    fn gruppenbreite_waechst_mit_der_knopfzahl() {
        let one = ribbon_group_width(1);
        assert_eq!(one, RIBBON_BUTTON_SIZE.x);

        let three = ribbon_group_width(3);
        assert_eq!(
            three,
            3.0 * RIBBON_BUTTON_SIZE.x + 2.0 * RIBBON_BUTTON_SPACING
        );
        assert!(three > ribbon_group_width(2));

        // Auch ohne Knöpfe bleibt eine Mindestbreite für die Beschriftung.
        assert_eq!(ribbon_group_width(0), one);
    }

    #[test]
    fn schmale_fenster_brechen_gruppen_um() {
        // Zwei Dreiergruppen passen bei 300 px nicht mehr nebeneinander.
        let verfuegbar = 300.0 - 16.0; // Innenabstand des Panels
        let zwei_gruppen = 2.0 * ribbon_group_width(3) + RIBBON_GROUP_SPACING + 2.0 * 4.0;
        assert!(zwei_gruppen > verfuegbar);
        // Eine einzelne Gruppe passt weiterhin vollständig.
        assert!(ribbon_group_width(3) <= verfuegbar);
    }

    // --- Einstellungen und Darstellung ---

    #[test]
    fn einstellungen_starten_mit_system_theme_und_standardschrift() {
        let app = App::default();
        assert_eq!(app.settings, Settings::default());
        assert_eq!(app.settings.theme, ThemeMode::System);
        assert_eq!(app.settings.editor_font_size, EDITOR_FONT_DEFAULT);
        assert!(app.settings.editor_font.is_empty());
        assert!(!app.settings_window);
        assert!(app.applied_font.is_empty());
    }

    #[test]
    fn einstellungen_begrenzen_die_schriftgroesse() {
        let mut s = Settings {
            editor_font_size: 99.0,
            ..Settings::default()
        };
        s.normalize();
        assert_eq!(s.editor_font_size, EDITOR_FONT_MAX);

        s.editor_font_size = 1.0;
        s.normalize();
        assert_eq!(s.editor_font_size, EDITOR_FONT_MIN);

        s.editor_font_size = f32::NAN;
        s.normalize();
        assert_eq!(s.editor_font_size, EDITOR_FONT_DEFAULT);
    }

    #[test]
    fn einstellungen_werden_gespeichert_und_geladen() {
        let (path, cleanup) = temp_file("settings", "");
        let settings = Settings {
            theme: ThemeMode::Dark,
            editor_font_size: 18.0,
            editor_font: "Consolas".into(),
            recent: vec!["C:/tmp/a.json".into()],
            compare_side_by_side: false,
        };
        save_settings(&path, &settings).unwrap();
        assert_eq!(load_settings(&path), settings);
        cleanup();
    }

    #[test]
    fn fehlende_einstellungsdatei_liefert_standardwerte() {
        let missing = std::env::temp_dir().join("parsix-einstellungen-4711.json");
        assert_eq!(load_settings(&missing), Settings::default());
    }

    #[test]
    fn theme_modi_liefern_die_passende_vorgabe() {
        assert_eq!(
            ThemeMode::System.preference(),
            egui::ThemePreference::System
        );
        assert_eq!(ThemeMode::Light.preference(), egui::ThemePreference::Light);
        assert_eq!(ThemeMode::Dark.preference(), egui::ThemePreference::Dark);
        assert_eq!(
            ThemeMode::ALL.map(|m| m.name()),
            ["System", "Hell", "Dunkel"]
        );
    }

    #[test]
    fn schriftdateien_werden_am_dateikopf_geprueft() {
        let dir = std::env::temp_dir().join(format!("parsix-schrift-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let kaputt = dir.join("kaputt.ttf");
        std::fs::write(&kaputt, b"keine Schrift").unwrap();
        assert!(read_font_file(&kaputt).is_none());

        let gueltig = dir.join("gueltig.ttf");
        std::fs::write(&gueltig, [0x00, 0x01, 0x00, 0x00, 0x00, 0x00]).unwrap();
        assert!(read_font_file(&gueltig).is_some());

        let fehlt = dir.join("fehlt.ttf");
        assert!(read_font_file(&fehlt).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Zuletzt verwendet, Drag & Drop, Wiederherstellung ---

    #[test]
    fn zuletzt_liste_ist_begrenzt_und_ohne_doppelungen() {
        let mut s = Settings::default();
        for i in 0..(MAX_RECENT + 5) {
            s.remember_file(Path::new(&format!("C:/tmp/datei{i}.json")));
        }
        assert_eq!(s.recent.len(), MAX_RECENT);
        assert_eq!(s.recent[0], format!("C:/tmp/datei{}.json", MAX_RECENT + 4));

        // Erneutes Merken verschiebt den Eintrag nach vorn, ohne Duplikat.
        let letzte = format!("C:/tmp/datei{}.json", MAX_RECENT + 4);
        s.remember_file(Path::new(&letzte));
        assert_eq!(s.recent.len(), MAX_RECENT);
        assert_eq!(s.recent[0], letzte);
        assert_eq!(s.recent.iter().filter(|p| **p == letzte).count(), 1);
    }

    #[test]
    fn zwischenstand_wird_gespeichert_und_geladen() {
        let dir = std::env::temp_dir().join(format!("parsix-recovery-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("recovery.json");
        let rec = Recovery {
            path: Some("C:/tmp/a.json".into()),
            fmt: Fmt::Yaml,
            text: "a: 1".into(),
        };
        save_recovery(&path, &rec).unwrap();
        assert_eq!(load_recovery(&path), Some(rec));

        // Leerer Inhalt gilt als kein Zwischenstand.
        let leer = Recovery {
            path: None,
            fmt: Fmt::Json,
            text: String::new(),
        };
        save_recovery(&path, &leer).unwrap();
        assert!(load_recovery(&path).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn geoeffnete_datei_landet_in_der_zuletzt_liste() {
        let (path, cleanup) = temp_file("recent", "{\"a\": 1}");
        let mut app = App::default();
        app.open_path(path.clone());
        assert_eq!(app.text, "{\"a\": 1}");
        assert_eq!(
            app.settings.recent.first().map(String::as_str),
            Some(path.to_string_lossy().as_ref())
        );

        // Dieselbe Datei erzeugt keinen doppelten Eintrag.
        app.open_path(path.clone());
        assert_eq!(app.settings.recent.len(), 1);
        cleanup();
    }

    #[test]
    fn ungespeicherte_anderungen_verhindern_direktes_oeffnen() {
        let (path, cleanup) = temp_file("pending", "{\"b\": 2}");
        let mut app = App::default();
        app.text = "{\"a\": 1}".into();
        app.request_open(path.clone());
        assert_eq!(app.pending_open.as_deref(), Some(path.as_path()));
        assert_eq!(app.text, "{\"a\": 1}");

        app.saved_text = app.text.clone();
        app.request_open(path.clone());
        assert!(app.pending_open.is_none());
        assert_eq!(app.text, "{\"b\": 2}");
        cleanup();
    }

    #[test]
    fn wiederherstellung_uebernimmt_den_zwischenstand() {
        let (path, cleanup) = temp_file("restore", "{\"alt\": true}");
        let mut app = App::default();
        app.recovery = Some(Recovery {
            path: Some(path.to_string_lossy().to_string()),
            fmt: Fmt::Json,
            text: "{\"neu\": true}".into(),
        });
        app.restore_recovery();
        assert_eq!(app.text, "{\"neu\": true}");
        assert_eq!(app.saved_text, "{\"alt\": true}");
        assert_eq!(app.path.as_deref(), Some(path.as_path()));
        assert!(app.is_modified());
        assert!(app.recovery.is_none());
        cleanup();
    }

    #[test]
    fn neues_dokument_verwirft_alten_zwischenstand() {
        let dir = std::env::temp_dir().join(format!("parsix-recovery-neu-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rec_path = dir.join("recovery.json");
        std::fs::write(&rec_path, "{}").unwrap();
        let file = dir.join("datei.json");
        std::fs::write(&file, "{}").unwrap();

        let mut app = App::default();
        app.recovery = Some(Recovery {
            path: None,
            fmt: Fmt::Json,
            text: "x".into(),
        });
        app.recovery_path = Some(rec_path.clone());
        app.set_loaded(file, "{}".into());
        assert!(app.recovery.is_none());
        assert!(!rec_path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn autosave_schreibt_und_loescht_zwischenstand() {
        let dir = std::env::temp_dir().join(format!("parsix-autosave-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rec_path = dir.join("recovery.json");

        let mut app = App::default();
        app.recovery_path = Some(rec_path.clone());
        app.text = "{\"a\":1}".into();
        app.autosave(10.0);
        let rec = load_recovery(&rec_path).expect("Zwischenstand fehlt");
        assert_eq!(rec.text, "{\"a\":1}");
        assert_eq!(rec.fmt, Fmt::Json);

        // Ohne Änderungen wird der Zwischenstand entfernt.
        app.saved_text = app.text.clone();
        app.autosave(20.0);
        assert!(!rec_path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Vorher-/Nachher-Vergleich ---

    #[test]
    fn vergleich_identischer_inhalte() {
        let d = diff_lines("a\nb\nc", "a\nb\nc");
        assert!(d.is_empty());
        assert!(d.left.iter().all(|c| *c == LineChange::Equal));
        assert!(d.right.iter().all(|c| *c == LineChange::Equal));
    }

    #[test]
    fn vergleich_erkennt_eingefuegte_zeile() {
        let d = diff_lines("a\nb", "a\nx\nb");
        assert_eq!(d.left, vec![LineChange::Equal, LineChange::Equal]);
        assert_eq!(
            d.right,
            vec![LineChange::Equal, LineChange::Added, LineChange::Equal]
        );
        assert_eq!(d.changes.len(), 1);
        let c = d.changes[0];
        assert_eq!(
            (c.left_start, c.left_len, c.right_start, c.right_len),
            (1, 0, 1, 1)
        );
    }

    #[test]
    fn vergleich_erkennt_entfernte_zeile() {
        let d = diff_lines("a\nx\nb", "a\nb");
        assert_eq!(
            d.left,
            vec![LineChange::Equal, LineChange::Removed, LineChange::Equal]
        );
        assert_eq!(d.right, vec![LineChange::Equal, LineChange::Equal]);
        let c = d.changes[0];
        assert_eq!(
            (c.left_start, c.left_len, c.right_start, c.right_len),
            (1, 1, 1, 0)
        );
    }

    #[test]
    fn vergleich_erkennt_geaenderte_zeile() {
        let d = diff_lines("a\nb\nc", "a\nx\nc");
        assert_eq!(
            d.left,
            vec![LineChange::Equal, LineChange::Changed, LineChange::Equal]
        );
        assert_eq!(
            d.right,
            vec![LineChange::Equal, LineChange::Changed, LineChange::Equal]
        );
        let c = d.changes[0];
        assert_eq!(
            (c.left_start, c.left_len, c.right_start, c.right_len),
            (1, 1, 1, 1)
        );
    }

    #[test]
    fn vergleich_mehrerer_bloecke() {
        let d = diff_lines("a\nb\nc\nd", "a\nX\nc\nY");
        assert_eq!(d.changes.len(), 2);
        assert_eq!(d.changes[0].left_start, 1);
        assert_eq!(d.changes[1].left_start, 3);
        assert_eq!(d.left[1], LineChange::Changed);
        assert_eq!(d.left[2], LineChange::Equal);
        assert_eq!(d.left[3], LineChange::Changed);
    }

    #[test]
    fn vergleich_leerer_und_kurzer_seiten() {
        let d = diff_lines("", "");
        assert!(d.is_empty());
        assert_eq!(d.left.len(), 1);
        assert_eq!(d.right.len(), 1);

        let d = diff_lines("", "x");
        assert_eq!(d.left, vec![LineChange::Changed]);
        assert_eq!(d.right, vec![LineChange::Changed]);
    }

    #[test]
    fn vergleich_mit_abschliessendem_umbruch() {
        let d = diff_lines("a\n", "a\nb\n");
        assert_eq!(d.left.len(), 2);
        assert_eq!(d.right.len(), 3);
        assert_eq!(d.left[1], LineChange::Equal);
        assert_eq!(d.right[1], LineChange::Added);
        assert_eq!(d.right[2], LineChange::Equal);
    }

    #[test]
    fn vergleich_operationen_ergeben_beide_seiten() {
        let cases = [
            ("", ""),
            ("a", ""),
            ("", "a"),
            ("a\nb\nc", "a\nx\nc"),
            ("a\nb\nc\nd\ne", "x\nb\ny\nd\nz"),
            ("one\ntwo\nthree", "one\nthree\nfour"),
            ("a\nb\nc", "a\nb\nc\nd"),
        ];
        for (a, b) in cases {
            let al: Vec<&str> = a.split('\n').collect();
            let bl: Vec<&str> = b.split('\n').collect();
            let ops = diff_ops(&al, &bl);
            let mut la = Vec::new();
            let mut lb = Vec::new();
            for op in &ops {
                match *op {
                    DiffOp::Equal(i, j) => {
                        assert_eq!(al[i], bl[j], "gleiche Zeile für {a:?} / {b:?}");
                        la.push(al[i]);
                        lb.push(bl[j]);
                    }
                    DiffOp::Delete(i) => la.push(al[i]),
                    DiffOp::Insert(j) => lb.push(bl[j]),
                }
            }
            assert_eq!(la, al, "linke Seite für {a:?} / {b:?}");
            assert_eq!(lb, bl, "rechte Seite für {a:?} / {b:?}");
        }
    }

    #[test]
    fn vergleich_sehr_grosser_dokumente() {
        let left: String = (0..5000).map(|i| format!("links {i}\n")).collect();
        let right: String = (0..5000).map(|i| format!("rechts {i}\n")).collect();
        let d = diff_lines(&left, &right);
        assert_eq!(d.left.len(), 5001);
        assert_eq!(d.right.len(), 5001);
        assert!(!d.is_empty());
    }

    #[test]
    fn vergleich_startet_mit_formatiertem_ergebnis() {
        let mut app = App::default();
        app.fmt = Fmt::Json;
        app.text = "{\"b\":1,\"a\":2}".into();
        app.saved_text = app.text.clone();
        app.start_compare_format();
        let cmp = app.compare.as_ref().expect("Vergleich fehlt");
        assert_eq!(cmp.left, "{\"b\":1,\"a\":2}");
        assert!(app.text.contains('\n'));
        assert!(!cmp.diff.is_empty());
        assert_eq!(app.ribbon_tab, RibbonTab::Vergleichen);
    }

    #[test]
    fn vergleich_navigation_springt_zum_naechsten_block() {
        let mut app = App::default();
        app.text = "a\nb\nc\nd".into();
        app.saved_text = app.text.clone();
        app.open_compare("a\nX\nc\nY".into());
        assert_eq!(app.compare.as_ref().unwrap().change_idx, Some(0));
        app.compare_step(true);
        assert_eq!(app.compare.as_ref().unwrap().change_idx, Some(1));
        app.compare_step(true);
        assert_eq!(app.compare.as_ref().unwrap().change_idx, Some(0));
        app.compare_step(false);
        assert_eq!(app.compare.as_ref().unwrap().change_idx, Some(1));
    }

    #[test]
    fn vergleich_beenden_behaelt_den_rechten_inhalt() {
        let mut app = App::default();
        app.text = "neu".into();
        app.open_compare("alt".into());
        app.close_compare();
        assert!(app.compare.is_none());
        assert_eq!(app.text, "neu");
    }
}
