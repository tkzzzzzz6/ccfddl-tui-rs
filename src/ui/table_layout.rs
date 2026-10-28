//! Column geometry of the conference table: default and user-resized widths,
//! horizontal scrolling with frozen leading columns, and `~/.ccfddl/layout.yaml`.
//!
//! Positions are computed here rather than by ratatui's `Table`, so rendering,
//! sort clicks, resize handles and scrollbars all use the same numbers.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use serde_yaml_ng::{Mapping, Value};

use super::app::Language;
use super::render::TableColumn;
use crate::data::favorites::app_dir;

/// Width of the `>` selection marker column.
pub const SELECTION_WIDTH: u16 = 1;
/// Gap between columns; it also hosts the resize handle.
pub const SPACING: u16 = 1;
/// Columns moved by one horizontal scroll step (keys and wheel).
pub const HSCROLL_STEP: u16 = 4;
const MIN_COLUMN_WIDTH: u16 = 3;

/// User-chosen widths, by column.
pub type ColumnWidths = BTreeMap<TableColumn, u16>;

/// One column placed in table-content coordinates (0 = selection marker).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacedColumn {
    pub column: TableColumn,
    pub x: u16,
    pub width: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableGeometry {
    pub columns: Vec<PlacedColumn>,
    /// Width of the full content, selection marker included.
    pub content_width: u16,
    /// Width of the visible table area.
    pub viewport: u16,
    /// Content before this x never scrolls (marker, ★ and title).
    pub pinned: u16,
}

impl TableGeometry {
    pub fn overflows(&self) -> bool {
        self.content_width > self.viewport
    }

    pub fn max_offset(&self) -> u16 {
        self.content_width.saturating_sub(self.viewport)
    }

    /// Content x -> viewport x for a horizontal offset, `None` when hidden.
    pub fn to_screen(&self, x: u16, offset: u16) -> Option<u16> {
        let screen = if x < self.pinned {
            x
        } else if x >= self.pinned + offset {
            x - offset
        } else {
            return None;
        };
        (screen < self.viewport).then_some(screen)
    }

    /// Viewport x -> content x.
    pub fn to_content(&self, x: u16, offset: u16) -> u16 {
        if x < self.pinned { x } else { x + offset }
    }

    /// Visible part of `[x, x + width)` in viewport coordinates.
    pub fn visible_span(&self, x: u16, width: u16, offset: u16) -> Option<(u16, u16)> {
        let (start, end) = if x < self.pinned {
            (x, (x + width).min(self.pinned))
        } else {
            let start = x.max(self.pinned + offset);
            (start - offset, (x + width).saturating_sub(offset))
        };
        let end = end.min(self.viewport);
        (end > start).then(|| (start, end - start))
    }

    pub fn width_of(&self, column: TableColumn) -> Option<u16> {
        self.columns
            .iter()
            .find(|c| c.column == column)
            .map(|c| c.width)
    }
}

/// Smallest width a column can be dragged to: its header stays readable.
pub fn min_width(column: TableColumn, language: Language) -> u16 {
    if column == TableColumn::Favorite {
        return 1;
    }
    (Line::from(column.label(language)).width() as u16).max(MIN_COLUMN_WIDTH)
}

fn natural(constraint: Constraint) -> u16 {
    match constraint {
        Constraint::Length(n) | Constraint::Max(n) | Constraint::Min(n) => n,
        _ => MIN_COLUMN_WIDTH,
    }
}

fn split(constraints: &[Constraint], width: u16) -> Vec<u16> {
    Layout::horizontal(constraints.to_vec())
        .spacing(SPACING)
        .split(Rect::new(0, 0, width, 1))
        .iter()
        .map(|r| r.width)
        .collect()
}

/// Place `columns` in a table `viewport` columns wide.
///
/// Without custom widths the columns share the space as before. Custom
/// widths are honoured exactly; when they do not fit, every column takes its
/// natural width and the table scrolls horizontally.
pub fn layout(
    columns: &[TableColumn],
    language: Language,
    custom: &ColumnWidths,
    viewport: u16,
) -> TableGeometry {
    let available = viewport.saturating_sub(SELECTION_WIDTH);
    let defaults: Vec<Constraint> = columns.iter().map(|c| c.width(language)).collect();
    let requested: Vec<Option<u16>> = columns
        .iter()
        .map(|c| custom.get(c).map(|w| (*w).max(min_width(*c, language))))
        .collect();

    let widths = if requested.iter().all(Option::is_none) {
        split(&defaults, available)
    } else {
        let constraints: Vec<Constraint> = defaults
            .iter()
            .zip(&requested)
            .map(|(d, r)| r.map_or(*d, Constraint::Length))
            .collect();
        let fitted = split(&constraints, available);
        let honoured =
            fitted.len() == columns.len()
                && fitted.iter().zip(&requested).zip(columns).all(
                    |((got, want), column)| match want {
                        Some(w) => got == w,
                        None => *got >= min_width(*column, language),
                    },
                );
        if honoured {
            fitted
        } else {
            defaults
                .iter()
                .zip(&requested)
                .map(|(d, r)| r.unwrap_or_else(|| natural(*d)))
                .collect()
        }
    };

    let mut x = SELECTION_WIDTH;
    let mut placed = Vec::with_capacity(columns.len());
    for (column, width) in columns.iter().zip(widths) {
        placed.push(PlacedColumn {
            column: *column,
            x,
            width,
        });
        x += width + SPACING;
    }
    let content_width = x.saturating_sub(SPACING).max(SELECTION_WIDTH);

    let mut geometry = TableGeometry {
        columns: placed,
        content_width,
        viewport,
        pinned: SELECTION_WIDTH,
    };
    if geometry.overflows()
        && let Some(title) = geometry
            .columns
            .iter()
            .take(2)
            .find(|c| c.column == TableColumn::Title)
    {
        let pinned = title.x + title.width + SPACING;
        // Freeze ★ and the title only while enough room is left to scroll.
        if pinned + 12 <= viewport {
            geometry.pinned = pinned;
        }
    }
    geometry
}

pub fn default_layout_path() -> PathBuf {
    app_dir().join("layout.yaml")
}

/// Read saved widths. A missing or broken file yields no custom widths.
pub fn load_widths(path: &Path) -> ColumnWidths {
    let Ok(text) = fs::read_to_string(path) else {
        return ColumnWidths::new();
    };
    let Ok(Value::Mapping(root)) = serde_yaml_ng::from_str::<Value>(&text) else {
        return ColumnWidths::new();
    };
    let Some(Value::Mapping(columns)) = root.get("columns") else {
        return ColumnWidths::new();
    };
    columns
        .iter()
        .filter_map(|(key, value)| {
            let column = TableColumn::from_key(key.as_str()?)?;
            let width = value.as_u64()?.clamp(1, u16::MAX as u64) as u16;
            Some((column, width))
        })
        .collect()
}

pub fn save_widths(path: &Path, widths: &ColumnWidths) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut columns = Mapping::new();
    for (column, width) in widths {
        columns.insert(column.key().into(), u64::from(*width).into());
    }
    let mut root = Mapping::new();
    root.insert("columns".into(), Value::Mapping(columns));
    let text = serde_yaml_ng::to_string(&Value::Mapping(root)).map_err(io::Error::other)?;
    fs::write(
        path,
        format!("# Conference table column widths (drag the header separators to change)\n{text}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use TableColumn::*;

    fn wide() -> Vec<TableColumn> {
        vec![Favorite, Title, Sub, CcfRank, Countdown, Date]
    }

    #[test]
    fn default_layout_fills_the_viewport() {
        let g = layout(&wide(), Language::En, &ColumnWidths::new(), 100);
        assert!(!g.overflows());
        assert_eq!(g.columns[0].x, SELECTION_WIDTH);
        let last = g.columns.last().unwrap();
        assert_eq!(last.x + last.width, 100);
        assert_eq!(g.pinned, SELECTION_WIDTH);
    }

    #[test]
    fn custom_widths_that_fit_are_exact() {
        let custom = ColumnWidths::from([(Title, 20)]);
        let g = layout(&wide(), Language::En, &custom, 100);
        assert_eq!(g.width_of(Title), Some(20));
        assert!(!g.overflows());
    }

    #[test]
    fn wide_custom_widths_overflow_and_freeze_the_title() {
        let custom = ColumnWidths::from([(Date, 80)]);
        let g = layout(&wide(), Language::En, &custom, 100);
        assert_eq!(g.width_of(Date), Some(80));
        assert!(g.overflows());
        let title = g.columns[1];
        assert_eq!(g.pinned, title.x + title.width + SPACING);
        assert_eq!(g.max_offset(), g.content_width - 100);
        // The frozen title stays, the next column scrolls away.
        let sub = g.columns[2];
        assert_eq!(g.to_screen(title.x, 5), Some(title.x));
        assert_eq!(g.to_screen(sub.x, 5), None);
        assert_eq!(g.to_screen(sub.x + 5, 5), Some(sub.x));
        assert_eq!(g.to_content(sub.x, 5), sub.x + 5);
    }

    #[test]
    fn widths_never_go_below_the_header() {
        let custom = ColumnWidths::from([(Countdown, 1)]);
        let g = layout(&wide(), Language::En, &custom, 100);
        assert_eq!(g.width_of(Countdown), Some(9)); // "Countdown"
        assert_eq!(min_width(Countdown, Language::Zh), 6); // 倒计时
    }

    #[test]
    fn visible_span_clips_scrolled_columns() {
        let custom = ColumnWidths::from([(Date, 80)]);
        let g = layout(&wide(), Language::En, &custom, 100);
        let date = g.columns.iter().find(|c| c.column == Date).unwrap();
        let (start, width) = g.visible_span(date.x, date.width, g.max_offset()).unwrap();
        assert_eq!(start + width, 100);
        let sub = g.columns[2];
        assert_eq!(g.visible_span(sub.x, sub.width, g.max_offset()), None);
    }

    #[test]
    fn widths_round_trip_and_broken_files_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/layout.yaml");
        let widths = ColumnWidths::from([(Title, 30), (Place, 25)]);
        save_widths(&path, &widths).unwrap();
        assert_eq!(load_widths(&path), widths);

        fs::write(&path, "columns: [nope").unwrap();
        assert!(load_widths(&path).is_empty());
        fs::write(&path, "columns: {title: 12, bogus: 3, date: x}").unwrap();
        assert_eq!(load_widths(&path), ColumnWidths::from([(Title, 12)]));
        assert!(load_widths(&dir.path().join("missing.yaml")).is_empty());
    }
}
