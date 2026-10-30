//! Colour depth of the terminal: 24-bit, 256, 16 colours or none.
//!
//! The theme uses fixed RGB colours. Terminals without truecolor support
//! may show those wrongly, so each finished frame is converted to the
//! nearest colours the terminal can show, without touching any widget code.
//! The mode comes from `--color`, `CCFDDL_COLOR`, `NO_COLOR` or detection.

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    #[default]
    TrueColor,
    Ansi256,
    Ansi16,
    None,
}

impl ColorMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::TrueColor => "truecolor",
            Self::Ansi256 => "256",
            Self::Ansi16 => "16",
            Self::None => "none",
        }
    }
}

/// `Ok(None)` means `auto`.
pub fn parse(text: &str) -> Result<Option<ColorMode>, String> {
    match text.trim().to_lowercase().as_str() {
        "auto" | "" => Ok(None),
        "truecolor" | "24bit" | "24-bit" | "rgb" => Ok(Some(ColorMode::TrueColor)),
        "256" | "ansi256" => Ok(Some(ColorMode::Ansi256)),
        "16" | "ansi" | "basic" => Ok(Some(ColorMode::Ansi16)),
        "none" | "no" | "off" | "0" => Ok(Some(ColorMode::None)),
        other => Err(format!(
            "unknown colour mode '{other}' (use auto, truecolor, 256, 16 or none)"
        )),
    }
}

/// What the environment says about the terminal.
pub fn detect(env: &dyn Fn(&str) -> Option<String>) -> ColorMode {
    let var = |key: &str| env(key).filter(|v| !v.is_empty());
    // https://no-color.org: any non-empty value disables colour.
    if var("NO_COLOR").is_some() {
        return ColorMode::None;
    }
    let colorterm = var("COLORTERM").unwrap_or_default().to_lowercase();
    if colorterm == "truecolor" || colorterm == "24bit" {
        return ColorMode::TrueColor;
    }
    let program = var("TERM_PROGRAM").unwrap_or_default();
    if [
        "iTerm.app",
        "WezTerm",
        "vscode",
        "ghostty",
        "Hyper",
        "Tabby",
    ]
    .iter()
    .any(|p| program.eq_ignore_ascii_case(p))
        || var("WT_SESSION").is_some()
        || var("KITTY_WINDOW_ID").is_some()
        || var("WEZTERM_EXECUTABLE").is_some()
    {
        return ColorMode::TrueColor;
    }
    let term = var("TERM").unwrap_or_default().to_lowercase();
    if ["truecolor", "24bit", "direct"]
        .iter()
        .any(|t| term.contains(t))
    {
        return ColorMode::TrueColor;
    }
    if term.contains("256color") {
        return ColorMode::Ansi256;
    }
    if term == "dumb" {
        return ColorMode::None;
    }
    // Windows consoles without the variables above still render ANSI
    // colours; 256 is the safe choice there and everywhere else unknown
    // terminals usually manage 256 colours too, but 16 never fails.
    if cfg!(windows) {
        ColorMode::Ansi256
    } else {
        ColorMode::Ansi16
    }
}

/// `--color` > `CCFDDL_COLOR` > detection.
pub fn resolve(
    cli: Option<&str>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<ColorMode, String> {
    if let Some(text) = cli
        && let Some(mode) = parse(text)?
    {
        return Ok(mode);
    }
    if let Some(text) = env("CCFDDL_COLOR")
        && let Some(mode) = parse(&text).map_err(|e| format!("CCFDDL_COLOR: {e}"))?
    {
        return Ok(mode);
    }
    Ok(detect(env))
}

const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// The 16 basic colours as xterm draws them, in ANSI order.
const BASIC: [(Color, (u8, u8, u8)); 16] = [
    (Color::Black, (0, 0, 0)),
    (Color::Red, (205, 0, 0)),
    (Color::Green, (0, 205, 0)),
    (Color::Yellow, (205, 205, 0)),
    (Color::Blue, (0, 0, 238)),
    (Color::Magenta, (205, 0, 205)),
    (Color::Cyan, (0, 205, 205)),
    (Color::Gray, (229, 229, 229)),
    (Color::DarkGray, (127, 127, 127)),
    (Color::LightRed, (255, 0, 0)),
    (Color::LightGreen, (0, 255, 0)),
    (Color::LightYellow, (255, 255, 0)),
    (Color::LightBlue, (92, 92, 255)),
    (Color::LightMagenta, (255, 0, 255)),
    (Color::LightCyan, (0, 255, 255)),
    (Color::White, (255, 255, 255)),
];

/// RGB of an xterm 256-colour index.
pub fn index_rgb(index: u8) -> (u8, u8, u8) {
    match index {
        0..=15 => BASIC[index as usize].1,
        16..=231 => {
            let i = index - 16;
            (
                CUBE[(i / 36) as usize],
                CUBE[(i / 6 % 6) as usize],
                CUBE[(i % 6) as usize],
            )
        }
        _ => {
            let v = 8 + 10 * (index - 232);
            (v, v, v)
        }
    }
}

fn distance(a: (u8, u8, u8), b: (u8, u8, u8)) -> u32 {
    // Weighted for the eye's sensitivity (a cheap "redmean" variant).
    let (dr, dg, db) = (
        a.0 as i32 - b.0 as i32,
        a.1 as i32 - b.1 as i32,
        a.2 as i32 - b.2 as i32,
    );
    (2 * dr * dr + 4 * dg * dg + 3 * db * db) as u32
}

/// Nearest xterm colour among the 6×6×6 cube and the grey ramp (16-255;
/// the first 16 vary between terminals and are not used).
pub fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    (16..=255u8)
        .min_by_key(|i| distance(index_rgb(*i), (r, g, b)))
        .unwrap_or(16)
}

/// The closest of the 16 basic colours by hue rather than by distance, so
/// a soft red stays red: greys by brightness, other colours by which
/// channels stand out, bright variants for bright colours.
pub fn rgb_to_16(r: u8, g: u8, b: u8) -> Color {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    if max - min < 40 {
        let level = (r as u32 * 3 + g as u32 * 6 + b as u32) / 10;
        return match level {
            0..50 => Color::Black,
            50..110 => Color::DarkGray,
            110..220 => Color::Gray,
            _ => Color::White,
        };
    }
    let mid = min as u32 + (max - min) as u32 / 2;
    let on = |c: u8| c as u32 > mid;
    let bright = max >= 0xc8;
    match (on(r), on(g), on(b), bright) {
        (true, false, false, false) => Color::Red,
        (true, false, false, true) => Color::LightRed,
        (false, true, false, false) => Color::Green,
        (false, true, false, true) => Color::LightGreen,
        (false, false, true, false) => Color::Blue,
        (false, false, true, true) => Color::LightBlue,
        (true, true, false, false) => Color::Yellow,
        (true, true, false, true) => Color::LightYellow,
        (true, false, true, false) => Color::Magenta,
        (true, false, true, true) => Color::LightMagenta,
        (false, true, true, false) => Color::Cyan,
        (false, true, true, true) => Color::LightCyan,
        (_, _, _, false) => Color::Gray,
        (_, _, _, true) => Color::White,
    }
}

fn convert(color: Color, mode: ColorMode) -> Color {
    match (mode, color) {
        (ColorMode::TrueColor, c) => c,
        (ColorMode::None, _) => Color::Reset,
        (ColorMode::Ansi256, Color::Rgb(r, g, b)) => Color::Indexed(rgb_to_256(r, g, b)),
        (ColorMode::Ansi16, Color::Rgb(r, g, b)) => rgb_to_16(r, g, b),
        (ColorMode::Ansi16, Color::Indexed(i)) if i >= 16 => {
            let (r, g, b) = index_rgb(i);
            rgb_to_16(r, g, b)
        }
        (_, c) => c,
    }
}

/// Convert a finished frame for `mode`. Without colours, cells that had a
/// highlighted background are drawn reversed so the selection stays visible.
pub fn apply(buffer: &mut Buffer, mode: ColorMode, highlight: Color) {
    if mode == ColorMode::TrueColor {
        return;
    }
    for cell in buffer.content.iter_mut() {
        if mode == ColorMode::None && cell.bg == highlight {
            cell.modifier.insert(Modifier::REVERSED);
        }
        cell.fg = convert(cell.fg, mode);
        cell.bg = convert(cell.bg, mode);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn modes_are_parsed() {
        assert_eq!(parse("auto"), Ok(None));
        assert_eq!(parse("TrueColor"), Ok(Some(ColorMode::TrueColor)));
        assert_eq!(parse("256"), Ok(Some(ColorMode::Ansi256)));
        assert_eq!(parse("16"), Ok(Some(ColorMode::Ansi16)));
        assert_eq!(parse("none"), Ok(Some(ColorMode::None)));
        assert!(parse("pink").is_err());
    }

    #[test]
    fn detection_follows_the_environment() {
        let d = |pairs: &[(&str, &str)]| detect(&env(pairs));
        assert_eq!(d(&[("COLORTERM", "truecolor")]), ColorMode::TrueColor);
        assert_eq!(
            d(&[("COLORTERM", "24bit"), ("TERM", "xterm")]),
            ColorMode::TrueColor
        );
        assert_eq!(d(&[("TERM_PROGRAM", "iTerm.app")]), ColorMode::TrueColor);
        assert_eq!(d(&[("TERM_PROGRAM", "vscode")]), ColorMode::TrueColor);
        assert_eq!(d(&[("WT_SESSION", "x")]), ColorMode::TrueColor);
        assert_eq!(d(&[("KITTY_WINDOW_ID", "1")]), ColorMode::TrueColor);
        assert_eq!(d(&[("TERM", "xterm-direct")]), ColorMode::TrueColor);
        // macOS Terminal.app sets TERM_PROGRAM=Apple_Terminal and no COLORTERM.
        assert_eq!(
            d(&[
                ("TERM_PROGRAM", "Apple_Terminal"),
                ("TERM", "xterm-256color")
            ]),
            ColorMode::Ansi256
        );
        assert_eq!(d(&[("TERM", "screen-256color")]), ColorMode::Ansi256);
        assert_eq!(d(&[("TERM", "dumb")]), ColorMode::None);
        assert_eq!(
            d(&[("NO_COLOR", "1"), ("COLORTERM", "truecolor")]),
            ColorMode::None
        );
        // An empty NO_COLOR does not count.
        assert_eq!(
            d(&[("NO_COLOR", ""), ("COLORTERM", "truecolor")]),
            ColorMode::TrueColor
        );
    }

    #[test]
    fn flag_beats_variable_beats_detection() {
        let e = env(&[("CCFDDL_COLOR", "16"), ("COLORTERM", "truecolor")]);
        assert_eq!(resolve(Some("256"), &e), Ok(ColorMode::Ansi256));
        assert_eq!(resolve(None, &e), Ok(ColorMode::Ansi16));
        assert_eq!(resolve(Some("auto"), &e), Ok(ColorMode::Ansi16));
        let e = env(&[("CCFDDL_COLOR", "auto"), ("COLORTERM", "truecolor")]);
        assert_eq!(resolve(None, &e), Ok(ColorMode::TrueColor));
        let bad = env(&[("CCFDDL_COLOR", "pink")]);
        assert!(resolve(None, &bad).unwrap_err().starts_with("CCFDDL_COLOR"));
    }

    #[test]
    fn nearest_colours() {
        assert_eq!(rgb_to_256(0, 0, 0), 16);
        assert_eq!(rgb_to_256(255, 255, 255), 231);
        assert_eq!(index_rgb(rgb_to_256(95, 135, 175)), (95, 135, 175));
        // Dark greys go to the grey ramp, not the cube.
        assert!(rgb_to_256(31, 31, 31) >= 232);
        assert_eq!(rgb_to_16(240, 120, 120), Color::LightRed);
        assert_eq!(rgb_to_16(20, 20, 20), Color::Black);
        // The theme keeps its meaning in 16 colours.
        use crate::ui::theme::*;
        let named = |c: Color| match c {
            Color::Rgb(r, g, b) => rgb_to_16(r, g, b),
            other => other,
        };
        assert_eq!(named(ERROR), Color::LightRed);
        assert_eq!(named(WARNING), Color::LightYellow);
        assert_eq!(named(SUCCESS), Color::LightGreen);
        assert_eq!(named(PRIMARY), Color::LightBlue);
        assert_eq!(named(ACCENT), Color::LightCyan);
        assert_eq!(named(MUTED), Color::Gray);
        assert_eq!(named(HINT), Color::Gray);
        assert_eq!(named(BORDER), Color::DarkGray);
        assert_eq!(named(SURFACE), Color::Black);
        assert_eq!(named(SELECTED_BG), Color::Blue);
    }

    #[test]
    fn frames_are_converted() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 2, 1));
        buffer[(0, 0)]
            .set_fg(Color::Rgb(0xa9, 0xb1, 0xbd))
            .set_bg(Color::Rgb(36, 54, 86));
        buffer[(1, 0)].set_fg(Color::Red);
        let highlight = Color::Rgb(36, 54, 86);

        let mut b256 = buffer.clone();
        apply(&mut b256, ColorMode::Ansi256, highlight);
        assert!(matches!(b256[(0, 0)].fg, Color::Indexed(_)));
        assert!(matches!(b256[(0, 0)].bg, Color::Indexed(_)));
        assert_eq!(b256[(1, 0)].fg, Color::Red);

        let mut b16 = buffer.clone();
        apply(&mut b16, ColorMode::Ansi16, highlight);
        assert!(!matches!(
            b16[(0, 0)].fg,
            Color::Rgb(..) | Color::Indexed(_)
        ));

        let mut none = buffer.clone();
        apply(&mut none, ColorMode::None, highlight);
        assert_eq!(none[(0, 0)].fg, Color::Reset);
        assert!(none[(0, 0)].modifier.contains(Modifier::REVERSED));
        assert_eq!(none[(1, 0)].fg, Color::Reset);

        let mut same = buffer.clone();
        apply(&mut same, ColorMode::TrueColor, highlight);
        assert_eq!(same, buffer);
    }

    #[test]
    fn a_whole_frame_has_no_rgb_left_in_256_and_16_colours() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let (mut app, _dir) = crate::ui::agent::tests::app_with(None);
        app.open_agent();
        for mode in [ColorMode::Ansi256, ColorMode::Ansi16, ColorMode::None] {
            let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
            terminal
                .draw(|frame| {
                    crate::ui::render::draw(frame, &mut app, chrono::Utc::now());
                    apply(frame.buffer_mut(), mode, crate::ui::theme::SELECTED_BG);
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            for cell in buffer.content.iter() {
                for color in [cell.fg, cell.bg] {
                    assert!(!matches!(color, Color::Rgb(..)), "{mode:?}: {color:?}");
                    if mode != ColorMode::Ansi256 {
                        assert!(!matches!(color, Color::Indexed(_)), "{mode:?}: {color:?}");
                    }
                }
            }
        }
    }
}
