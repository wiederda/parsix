// src/icons.rs
//! Kleine, mit dem Painter gezeichnete Symbole für das Ribbon.
//!
//! Die Symbole werden relativ zu einem Rechteck gezeichnet, damit sie in jeder
//! Größe scharf aussehen und keine externen Bilddateien benötigt werden.

use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

/// Verfügbare Funktionssymbole.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Open,
    Save,
    Reload,
    Undo,
    Redo,
    Find,
    Replace,
    Format,
    Minify,
    Settings,
    Braces,
    Tags,
    List,
    Sort,
    Clock,
    ChevronUp,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    Compare,
    SplitH,
    SplitV,
    Convert,
    Close,
    Plus,
    Minus,
}

/// Wandelt relative Koordinaten (0..1) in Bildschirmkoordinaten um.
fn pt(rect: Rect, x: f32, y: f32) -> Pos2 {
    rect.min + Vec2::new(rect.width() * x, rect.height() * y)
}

/// Zeichnet einen offenen Linienzug.
fn line(painter: &Painter, rect: Rect, pts: &[(f32, f32)], stroke: Stroke) {
    let points: Vec<Pos2> = pts.iter().map(|&(x, y)| pt(rect, x, y)).collect();
    painter.add(Shape::line(points, stroke));
}

/// Zeichnet einen geschlossenen Linienzug.
fn closed(painter: &Painter, rect: Rect, pts: &[(f32, f32)], stroke: Stroke) {
    let points: Vec<Pos2> = pts.iter().map(|&(x, y)| pt(rect, x, y)).collect();
    painter.add(Shape::closed_line(points, stroke));
}

/// Zeichnet einen Kreis.
fn circle(painter: &Painter, rect: Rect, x: f32, y: f32, r: f32, stroke: Stroke) {
    painter.circle_stroke(pt(rect, x, y), rect.width() * r, stroke);
}

/// Zeichnet ein gefülltes Kreissegment (Aufzählungspunkt).
fn dot(painter: &Painter, rect: Rect, x: f32, y: f32, r: f32, color: Color32) {
    painter.circle_filled(pt(rect, x, y), rect.width() * r, color);
}

/// Zeichnet eine Spitze (Pfeilkopf) an `tip` mit den beiden Flügeln `a` und `b`.
fn head(
    painter: &Painter,
    rect: Rect,
    tip: (f32, f32),
    a: (f32, f32),
    b: (f32, f32),
    stroke: Stroke,
) {
    line(painter, rect, &[a, tip, b], stroke);
}

/// Zeichnet das Symbol `icon` zentriert in `rect`.
pub fn draw(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let stroke = Stroke::new(1.6_f32, color);
    match icon {
        Icon::Open => {
            closed(
                painter,
                rect,
                &[
                    (0.12, 0.32),
                    (0.44, 0.32),
                    (0.52, 0.42),
                    (0.88, 0.42),
                    (0.88, 0.80),
                    (0.12, 0.80),
                ],
                stroke,
            );
            line(painter, rect, &[(0.12, 0.42), (0.44, 0.42)], stroke);
        }
        Icon::Save => {
            closed(
                painter,
                rect,
                &[
                    (0.18, 0.18),
                    (0.74, 0.18),
                    (0.82, 0.26),
                    (0.82, 0.82),
                    (0.18, 0.82),
                ],
                stroke,
            );
            line(
                painter,
                rect,
                &[(0.34, 0.18), (0.34, 0.42), (0.66, 0.42), (0.66, 0.18)],
                stroke,
            );
            closed(
                painter,
                rect,
                &[(0.30, 0.56), (0.70, 0.56), (0.70, 0.82), (0.30, 0.82)],
                stroke,
            );
        }
        Icon::Reload => {
            line(
                painter,
                rect,
                &[
                    (0.74, 0.30),
                    (0.56, 0.20),
                    (0.36, 0.26),
                    (0.22, 0.44),
                    (0.24, 0.64),
                    (0.42, 0.80),
                    (0.64, 0.80),
                ],
                stroke,
            );
            head(
                painter,
                rect,
                (0.74, 0.30),
                (0.58, 0.26),
                (0.70, 0.44),
                stroke,
            );
        }
        Icon::Undo => {
            line(
                painter,
                rect,
                &[
                    (0.80, 0.72),
                    (0.76, 0.54),
                    (0.62, 0.38),
                    (0.42, 0.32),
                    (0.26, 0.38),
                ],
                stroke,
            );
            head(
                painter,
                rect,
                (0.26, 0.38),
                (0.40, 0.26),
                (0.40, 0.52),
                stroke,
            );
        }
        Icon::Redo => {
            line(
                painter,
                rect,
                &[
                    (0.20, 0.72),
                    (0.24, 0.54),
                    (0.38, 0.38),
                    (0.58, 0.32),
                    (0.74, 0.38),
                ],
                stroke,
            );
            head(
                painter,
                rect,
                (0.74, 0.38),
                (0.60, 0.26),
                (0.60, 0.52),
                stroke,
            );
        }
        Icon::Find => {
            circle(painter, rect, 0.42, 0.40, 0.20, stroke);
            line(painter, rect, &[(0.57, 0.55), (0.82, 0.80)], stroke);
        }
        Icon::Replace => {
            line(painter, rect, &[(0.20, 0.34), (0.74, 0.34)], stroke);
            head(
                painter,
                rect,
                (0.74, 0.34),
                (0.60, 0.24),
                (0.60, 0.44),
                stroke,
            );
            line(painter, rect, &[(0.80, 0.66), (0.26, 0.66)], stroke);
            head(
                painter,
                rect,
                (0.26, 0.66),
                (0.40, 0.56),
                (0.40, 0.76),
                stroke,
            );
        }
        Icon::Format => {
            line(painter, rect, &[(0.20, 0.26), (0.80, 0.26)], stroke);
            line(painter, rect, &[(0.20, 0.50), (0.66, 0.50)], stroke);
            line(painter, rect, &[(0.20, 0.74), (0.74, 0.74)], stroke);
            line(painter, rect, &[(0.86, 0.20), (0.86, 0.32)], stroke);
        }
        Icon::Minify => {
            // Zwei Pfeile, die aufeinander zu zeigen: zusammendrücken.
            line(painter, rect, &[(0.50, 0.14), (0.50, 0.42)], stroke);
            head(
                painter,
                rect,
                (0.50, 0.42),
                (0.40, 0.32),
                (0.60, 0.32),
                stroke,
            );
            line(painter, rect, &[(0.50, 0.86), (0.50, 0.58)], stroke);
            head(
                painter,
                rect,
                (0.50, 0.58),
                (0.40, 0.68),
                (0.60, 0.68),
                stroke,
            );
        }
        Icon::Settings => {
            circle(painter, rect, 0.50, 0.50, 0.15, stroke);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                let (s, c) = a.sin_cos();
                line(
                    painter,
                    rect,
                    &[
                        (0.50 + 0.24 * c, 0.50 + 0.24 * s),
                        (0.50 + 0.34 * c, 0.50 + 0.34 * s),
                    ],
                    stroke,
                );
            }
        }
        Icon::Braces => {
            line(
                painter,
                rect,
                &[
                    (0.44, 0.16),
                    (0.34, 0.16),
                    (0.34, 0.40),
                    (0.24, 0.50),
                    (0.34, 0.60),
                    (0.34, 0.84),
                    (0.44, 0.84),
                ],
                stroke,
            );
            line(
                painter,
                rect,
                &[
                    (0.56, 0.16),
                    (0.66, 0.16),
                    (0.66, 0.40),
                    (0.76, 0.50),
                    (0.66, 0.60),
                    (0.66, 0.84),
                    (0.56, 0.84),
                ],
                stroke,
            );
        }
        Icon::Tags => {
            line(
                painter,
                rect,
                &[(0.38, 0.26), (0.20, 0.50), (0.38, 0.74)],
                stroke,
            );
            line(
                painter,
                rect,
                &[(0.62, 0.26), (0.80, 0.50), (0.62, 0.74)],
                stroke,
            );
        }
        Icon::List => {
            for y in [0.28, 0.50, 0.72] {
                dot(painter, rect, 0.20, y, 0.045, color);
                line(painter, rect, &[(0.32, y), (0.82, y)], stroke);
            }
        }
        Icon::Sort => {
            // Absteigend kürzer werdende Zeilen mit einem Pfeil nach unten.
            line(painter, rect, &[(0.16, 0.26), (0.52, 0.26)], stroke);
            line(painter, rect, &[(0.16, 0.48), (0.42, 0.48)], stroke);
            line(painter, rect, &[(0.16, 0.70), (0.32, 0.70)], stroke);
            line(painter, rect, &[(0.74, 0.24), (0.74, 0.74)], stroke);
            head(
                painter,
                rect,
                (0.74, 0.76),
                (0.64, 0.64),
                (0.84, 0.64),
                stroke,
            );
        }
        Icon::Clock => {
            // Uhr für die zuletzt verwendeten Dateien.
            circle(painter, rect, 0.50, 0.50, 0.30, stroke);
            line(
                painter,
                rect,
                &[(0.50, 0.28), (0.50, 0.52), (0.66, 0.60)],
                stroke,
            );
        }
        Icon::ChevronUp => {
            line(
                painter,
                rect,
                &[(0.24, 0.62), (0.50, 0.36), (0.76, 0.62)],
                stroke,
            );
        }
        Icon::ChevronDown => {
            line(
                painter,
                rect,
                &[(0.24, 0.38), (0.50, 0.64), (0.76, 0.38)],
                stroke,
            );
        }
        Icon::ChevronLeft => {
            line(
                painter,
                rect,
                &[(0.62, 0.24), (0.36, 0.50), (0.62, 0.76)],
                stroke,
            );
        }
        Icon::ChevronRight => {
            line(
                painter,
                rect,
                &[(0.38, 0.24), (0.64, 0.50), (0.38, 0.76)],
                stroke,
            );
        }
        Icon::Compare => {
            // Zwei Dokumente nebeneinander; kurze Zeilen deuten den Inhalt an.
            closed(
                painter,
                rect,
                &[(0.10, 0.20), (0.42, 0.20), (0.42, 0.80), (0.10, 0.80)],
                stroke,
            );
            closed(
                painter,
                rect,
                &[(0.58, 0.20), (0.90, 0.20), (0.90, 0.80), (0.58, 0.80)],
                stroke,
            );
            line(painter, rect, &[(0.16, 0.38), (0.36, 0.38)], stroke);
            line(painter, rect, &[(0.16, 0.56), (0.36, 0.56)], stroke);
            line(painter, rect, &[(0.64, 0.38), (0.84, 0.38)], stroke);
            line(painter, rect, &[(0.64, 0.56), (0.84, 0.56)], stroke);
        }
        Icon::SplitH => {
            // Senkrechte Trennlinie: Ansichten nebeneinander.
            closed(
                painter,
                rect,
                &[(0.12, 0.20), (0.88, 0.20), (0.88, 0.80), (0.12, 0.80)],
                stroke,
            );
            line(painter, rect, &[(0.50, 0.20), (0.50, 0.80)], stroke);
        }
        Icon::SplitV => {
            // Waagerechte Trennlinie: Ansichten untereinander.
            closed(
                painter,
                rect,
                &[(0.12, 0.20), (0.88, 0.20), (0.88, 0.80), (0.12, 0.80)],
                stroke,
            );
            line(painter, rect, &[(0.12, 0.50), (0.88, 0.50)], stroke);
        }
        Icon::Convert => {
            // Zwei Striche mit Pfeil dazwischen: Inhalt wechselt das Format.
            line(painter, rect, &[(0.16, 0.28), (0.16, 0.72)], stroke);
            line(painter, rect, &[(0.84, 0.28), (0.84, 0.72)], stroke);
            line(painter, rect, &[(0.30, 0.50), (0.60, 0.50)], stroke);
            head(
                painter,
                rect,
                (0.68, 0.50),
                (0.56, 0.40),
                (0.56, 0.60),
                stroke,
            );
        }
        Icon::Close => {
            line(painter, rect, &[(0.28, 0.28), (0.72, 0.72)], stroke);
            line(painter, rect, &[(0.72, 0.28), (0.28, 0.72)], stroke);
        }
        Icon::Plus => {
            line(painter, rect, &[(0.22, 0.50), (0.78, 0.50)], stroke);
            line(painter, rect, &[(0.50, 0.22), (0.50, 0.78)], stroke);
        }
        Icon::Minus => {
            line(painter, rect, &[(0.22, 0.50), (0.78, 0.50)], stroke);
        }
    }
}
