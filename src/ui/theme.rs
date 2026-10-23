//! Colours and styles ported from the Textual `styles.tcss` of the Python app.
//!
//! The Python side uses Textual theme variables (`$accent`, `$error`, ...) plus
//! a few literal hex values for the per-category accents. Ratatui has no CSS,
//! so the mapping is spelled out here once and reused by every widget.

use ratatui::style::{Color, Modifier, Style};

use crate::format::CountdownColor;

/// Header/footer accent (`$accent`).
pub const ACCENT: Color = Color::Cyan;
/// Primary highlight (`$primary`).
pub const PRIMARY: Color = Color::Blue;
pub const ERROR: Color = Color::Red;
pub const WARNING: Color = Color::Yellow;
pub const SUCCESS: Color = Color::Green;
/// Muted body text (`$text-muted`).
pub const MUTED: Color = Color::DarkGray;
pub const TEXT: Color = Color::Gray;

pub const SURFACE: Color = Color::Rgb(31, 31, 31);
pub const SURFACE_DARK: Color = Color::Rgb(24, 24, 24);
pub const HEADER_BG: Color = Color::Rgb(25, 40, 60);
pub const FOOTER_BG: Color = Color::Rgb(20, 20, 20);
pub const SELECTED_BG: Color = Color::Rgb(45, 70, 105);

pub const BASE: Style = Style::new();
pub const HEADER_STYLE: Style = Style::new().fg(TEXT).bg(HEADER_BG);
pub const FOOTER_STYLE: Style = Style::new().fg(MUTED).bg(FOOTER_BG);
pub const SIDEBAR_STYLE: Style = Style::new().fg(TEXT).bg(SURFACE);
pub const TITLE_STYLE: Style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
pub const SECTION_STYLE: Style = Style::new().fg(MUTED).add_modifier(Modifier::BOLD);
pub const SELECTED_STYLE: Style = Style::new().bg(SELECTED_BG).add_modifier(Modifier::BOLD);
pub const DIM: Style = Style::new().fg(MUTED).add_modifier(Modifier::DIM);
pub const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);
pub const SEARCH_STYLE: Style = Style::new().fg(Color::White);
pub const PLACE_STYLE: Style = Style::new().fg(TEXT).add_modifier(Modifier::DIM);

/// Per-category accent colours, from the `.sub-*` rules in `styles.tcss`.
pub fn sub_color(sub: &str) -> Color {
    match sub {
        "DS" => Color::Rgb(0x5f, 0x87, 0xaf),
        "NW" => Color::Rgb(0x87, 0xaf, 0xaf),
        "SC" => Color::Rgb(0xaf, 0x5f, 0x5f),
        "SE" => Color::Rgb(0xaf, 0x87, 0x5f),
        "DB" => Color::Rgb(0x5f, 0xaf, 0x87),
        "CT" => Color::Rgb(0x87, 0x5f, 0x87),
        "CG" => Color::Rgb(0xaf, 0x5f, 0xaf),
        "AI" => Color::Rgb(0x5f, 0xaf, 0xaf),
        "HI" => Color::Rgb(0xaf, 0x87, 0x87),
        "MX" => Color::Rgb(0x87, 0x87, 0x5f),
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
            assert!(seen.insert(sub_color(cat.sub)), "duplicate colour for {}", cat.sub);
        }
        assert_eq!(sub_color("ZZ"), TEXT);
    }

    #[test]
    fn urgent_countdown_is_bold() {
        assert!(countdown_style(CountdownColor::Red).add_modifier.contains(Modifier::BOLD));
        assert!(!countdown_style(CountdownColor::Green).add_modifier.contains(Modifier::BOLD));
    }
}