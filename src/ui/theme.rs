//! Colours and styles ported from the Textual `styles.tcss` of the Python app.
//!
//! The Python side uses Textual theme variables (`$accent`, `$error`, ...) plus
//! a few literal hex values for the per-category accents. Ratatui has no CSS,
//! so the mapping is spelled out here once and reused by every widget.

use ratatui::style::{Color, Modifier, Style};

use crate::format::CountdownColor;

// Colours are fixed RGB values rather than the terminal's 16-colour palette:
// the backgrounds below are fixed too, and palette colours such as DarkGray
// or Blue range from readable to invisible depending on the terminal. Every
// text colour keeps a WCAG contrast of at least 4.5:1 on the panel
// backgrounds (see the tests). `Modifier::DIM` is not used for text because
// terminals render it inconsistently (some halve the brightness).

/// Header/footer accent (`$accent`).
pub const ACCENT: Color = Color::Rgb(0x56, 0xc8, 0xd8);
/// Primary highlight (`$primary`).
pub const PRIMARY: Color = Color::Rgb(0x6e, 0xa8, 0xf5);
pub const ERROR: Color = Color::Rgb(0xf2, 0x78, 0x78);
pub const WARNING: Color = Color::Rgb(0xe8, 0xc5, 0x47);
pub const SUCCESS: Color = Color::Rgb(0x6d, 0xcb, 0x7a);
/// Secondary text (`$text-muted`).
pub const MUTED: Color = Color::Rgb(0xa9, 0xb1, 0xbd);
/// Hints, placeholders and de-emphasised rows; still readable.
pub const HINT: Color = Color::Rgb(0x8f, 0x97, 0xa3);
/// Unfocused borders, separators and scrollbar tracks (decoration only).
pub const BORDER: Color = Color::Rgb(0x5c, 0x63, 0x6e);
pub const TEXT: Color = Color::Rgb(0xd0, 0xd4, 0xda);

pub const SURFACE: Color = Color::Rgb(31, 31, 31);
pub const SURFACE_DARK: Color = Color::Rgb(24, 24, 24);
pub const HEADER_BG: Color = Color::Rgb(25, 40, 60);
pub const FOOTER_BG: Color = Color::Rgb(20, 20, 20);
pub const SELECTED_BG: Color = Color::Rgb(36, 54, 86);

pub const BASE: Style = Style::new();
pub const HEADER_STYLE: Style = Style::new().fg(TEXT).bg(HEADER_BG);
pub const FOOTER_STYLE: Style = Style::new().fg(MUTED).bg(FOOTER_BG);
pub const SIDEBAR_STYLE: Style = Style::new().fg(TEXT).bg(SURFACE);
pub const TITLE_STYLE: Style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
pub const SECTION_STYLE: Style = Style::new().fg(MUTED).add_modifier(Modifier::BOLD);
pub const SELECTED_STYLE: Style = Style::new().bg(SELECTED_BG).add_modifier(Modifier::BOLD);
/// Secondary text: hints, finished conferences, placeholders.
pub const DIM: Style = Style::new().fg(HINT);
/// Unfocused borders and scrollbar tracks.
pub const BORDER_STYLE: Style = Style::new().fg(BORDER);
pub const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);
pub const SEARCH_STYLE: Style = Style::new().fg(Color::Rgb(0xf0, 0xf2, 0xf5));
pub const PLACE_STYLE: Style = Style::new().fg(MUTED);

/// Per-category accent colours, from the `.sub-*` rules in `styles.tcss`.
pub fn sub_color(sub: &str) -> Color {
    match sub {
        "DS" => Color::Rgb(0x65, 0x8c, 0xb2),
        "NW" => Color::Rgb(0x87, 0xaf, 0xaf),
        "SC" => Color::Rgb(0xba, 0x75, 0x75),
        "SE" => Color::Rgb(0xaf, 0x87, 0x5f),
        "DB" => Color::Rgb(0x5f, 0xaf, 0x87),
        "CT" => Color::Rgb(0x9f, 0x7f, 0x9f),
        "CG" => Color::Rgb(0xb7, 0x6f, 0xb7),
        "AI" => Color::Rgb(0x5f, 0xaf, 0xaf),
        "HI" => Color::Rgb(0xaf, 0x87, 0x87),
        "MX" => Color::Rgb(0x8c, 0x8c, 0x65),
        _ => TEXT,
    }
}

/// CCF rank colour (`CCF_RANK_COLORS` in `conference_table.py`).
pub fn ccf_rank_color(rank: &str) -> Color {
    match rank {
        "A" => ERROR,
        "B" => WARNING,
        "C" => SUCCESS,
        "N" => MUTED,
        _ => TEXT,
    }
}

/// CORE rank colour (`CORE_RANK_COLORS`).
pub fn core_rank_color(rank: &str) -> Color {
    match rank {
        "A*" | "A" => ERROR,
        "B" => WARNING,
        "C" => SUCCESS,
        "N" => MUTED,
        _ => TEXT,
    }
}

/// THCPL rank colour (`THCPL_RANK_COLORS`).
pub fn thcpl_rank_color(rank: &str) -> Color {
    match rank {
        "A" => ERROR,
        "B" => WARNING,
        "N" => MUTED,
        _ => TEXT,
    }
}

/// Countdown colour, reusing the thresholds from `format::get_countdown_color`.
pub fn countdown_color(color: CountdownColor) -> Color {
    match color {
        CountdownColor::Red => ERROR,
        CountdownColor::Yellow => WARNING,
        CountdownColor::Blue => PRIMARY,
        CountdownColor::Green => SUCCESS,
        CountdownColor::Dim => MUTED,
    }
}

/// Urgent countdowns blink in the Python app; a bold style keeps the same
/// emphasis without depending on terminal blink support.
pub fn countdown_style(color: CountdownColor) -> Style {
    match color {
        CountdownColor::Red => Style::new().fg(ERROR).add_modifier(Modifier::BOLD),
        CountdownColor::Dim => DIM,
        other => Style::new().fg(countdown_color(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_palettes_match_python_maps() {
        assert_eq!(ccf_rank_color("A"), ERROR);
        assert_eq!(ccf_rank_color("B"), WARNING);
        assert_eq!(ccf_rank_color("C"), SUCCESS);
        assert_eq!(ccf_rank_color("N"), MUTED);
        assert_eq!(ccf_rank_color("Z"), TEXT);

        // CORE has an extra A* tier; THCPL has no C.
        assert_eq!(core_rank_color("A*"), ERROR);
        assert_eq!(thcpl_rank_color("C"), TEXT);
        assert_eq!(thcpl_rank_color("A"), ERROR);
    }

    #[test]
    fn every_category_has_a_distinct_colour() {
        let mut seen = std::collections::HashSet::new();
        for cat in crate::model::CATEGORIES {
            assert!(
                seen.insert(sub_color(cat.sub)),
                "duplicate colour for {}",
                cat.sub
            );
        }
        assert_eq!(sub_color("ZZ"), TEXT);
    }

    fn rgb(color: Color) -> (f64, f64, f64) {
        match color {
            Color::Rgb(r, g, b) => (r as f64, g as f64, b as f64),
            other => panic!("{other:?} is not a fixed RGB colour"),
        }
    }

    /// WCAG 2 contrast ratio.
    fn contrast(a: Color, b: Color) -> f64 {
        let luminance = |c: Color| {
            let (r, g, b) = rgb(c);
            let lin = |v: f64| {
                let v = v / 255.0;
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
        };
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    const BACKGROUNDS: [Color; 4] = [SURFACE, SURFACE_DARK, HEADER_BG, FOOTER_BG];

    #[test]
    fn text_colours_are_readable_on_every_background() {
        let base = [TEXT, MUTED, HINT, ACCENT, PRIMARY, ERROR, WARNING, SUCCESS];
        for fg in base {
            for bg in BACKGROUNDS {
                let ratio = contrast(fg, bg);
                assert!(ratio >= 4.5, "{fg:?} on {bg:?}: {ratio:.2}");
            }
        }
        // Category colours only appear in the table and the sidebar.
        let categories: Vec<Color> = crate::model::CATEGORIES
            .iter()
            .map(|c| sub_color(c.sub))
            .collect();
        for fg in &categories {
            for bg in [SURFACE, SURFACE_DARK] {
                let ratio = contrast(*fg, bg);
                assert!(ratio >= 4.5, "{fg:?} on {bg:?}: {ratio:.2}");
            }
        }
        let text: Vec<Color> = base.into_iter().chain(categories).collect();
        // The selected row is a lighter background: body text stays at
        // 4.5:1, coloured accents at 3:1 (they are also bold there).
        for fg in [TEXT, MUTED, ACCENT, PRIMARY, WARNING, SUCCESS] {
            let ratio = contrast(fg, SELECTED_BG);
            assert!(ratio >= 4.5, "{fg:?} on the selection: {ratio:.2}");
        }
        for fg in &text {
            let ratio = contrast(*fg, SELECTED_BG);
            assert!(ratio >= 3.0, "{fg:?} on the selection: {ratio:.2}");
        }
    }

    #[test]
    fn text_styles_do_not_use_the_dim_modifier() {
        for style in [
            DIM,
            PLACE_STYLE,
            SECTION_STYLE,
            FOOTER_STYLE,
            countdown_style(CountdownColor::Dim),
        ] {
            assert!(!style.add_modifier.contains(Modifier::DIM), "{style:?}");
        }
        // Borders stay visible without competing with text.
        assert!(contrast(BORDER, SURFACE) >= 2.0);
        assert!(contrast(BORDER, SURFACE) < contrast(HINT, SURFACE));
    }

    #[test]
    fn urgent_countdown_is_bold() {
        assert!(
            countdown_style(CountdownColor::Red)
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            !countdown_style(CountdownColor::Green)
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }
}
