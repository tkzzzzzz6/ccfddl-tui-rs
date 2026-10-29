//! Draws the research-profile form inside the agent panel.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::agent::{Lang, Tier};

use super::agent_profile::today;
use super::agent_view::tier_style;
use super::app::App;
use super::line_input::text_width;
use super::profile_form::{Field, InvalidDate};
use super::theme;
use super::view::ViewState;

/// A piece of a row; `hit` makes it clickable.
struct Segment {
    text: String,
    style: Style,
    hit: Option<(Field, Option<usize>)>,
}

impl Segment {
    fn new(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
            hit: None,
        }
    }

    fn hit(mut self, field: Field, option: Option<usize>) -> Self {
        self.hit = Some((field, option));
        self
    }
}

#[derive(Default)]
struct Row {
    segments: Vec<Segment>,
    field: Option<Field>,
    /// Cursor column within the row (text fields under edit).
    cursor: Option<u16>,
}

fn tier_label(tier: Tier, lang: Lang) -> &'static str {
    tier.label(lang)
}

/// Push `segments` as one or more rows no wider than `width`, breaking
/// between segments and inside long ones; continuation rows start with
/// `indent`.
fn push_wrapped(
    rows: &mut Vec<Row>,
    segments: Vec<Segment>,
    width: usize,
    indent: &str,
    field: Option<Field>,
) {
    let indent_width = text_width(indent);
    let mut row = Row {
        field,
        ..Default::default()
    };
    let mut used = 0;
    for segment in segments {
        let mut text = segment.text.as_str();
        loop {
            let room = width.saturating_sub(used);
            if text_width(text) <= room {
                used += text_width(text);
                row.segments.push(Segment {
                    text: text.to_string(),
                    style: segment.style,
                    hit: segment.hit,
                });
                break;
            }
            // Take what fits, then continue on a new row.
            let mut taken = 0;
            let mut cut = 0;
            for (i, c) in text.char_indices() {
                let w = super::line_input::char_width(c);
                if taken + w > room {
                    break;
                }
                taken += w;
                cut = i + c.len_utf8();
            }
            let fresh = used == 0 || used == indent_width;
            if cut == 0 && fresh {
                // Too narrow for even one character: take it anyway.
                cut = text.chars().next().map_or(0, char::len_utf8);
            }
            if cut > 0 {
                row.segments.push(Segment {
                    text: text[..cut].to_string(),
                    style: segment.style,
                    hit: segment.hit,
                });
                text = &text[cut..];
            }
            rows.push(std::mem::replace(
                &mut row,
                Row {
                    field,
                    ..Default::default()
                },
            ));
            if text.is_empty() {
                break;
            }
            row.segments.push(Segment::new(indent, theme::DIM));
            used = indent_width;
        }
    }
    rows.push(row);
}

/// Draw the form in `area` (the panel's inner area).
pub fn render_form(frame: &mut Frame, app: &mut App, view: &mut ViewState, area: Rect) {
    let lang: Lang = app.language.into();
    let preview = app.profile_preview();
    let Some(form) = app.agent.form.as_ref() else {
        return;
    };
    let width = area.width as usize;
    let label_width = Field::ALL
        .iter()
        .map(|f| text_width(f.label(lang)))
        .max()
        .unwrap_or(8)
        + 3;
    let indent = " ".repeat(label_width);
    let mut rows: Vec<Row> = Vec::new();

    let title = lang.pick("Research profile", "研究画像");
    let keys = lang.pick("   Ctrl+S save · Esc cancel", "   Ctrl+S 保存 · Esc 取消");
    let mut head = vec![Segment::new(title, theme::TITLE_STYLE)];
    if text_width(title) + text_width(keys) <= width {
        head.push(Segment::new(keys, theme::DIM));
    }
    rows.push(Row {
        segments: head,
        ..Default::default()
    });
    if let Some(notice) = &form.notice {
        for line in notice.lines() {
            push_wrapped(
                &mut rows,
                vec![Segment::new(line, Style::new().fg(theme::WARNING))],
                width,
                "  ",
                None,
            );
        }
    }
    if let Some(error) = &form.error {
        push_wrapped(
            &mut rows,
            vec![Segment::new(
                format!("✗ {error}"),
                Style::new().fg(theme::ERROR),
            )],
            width,
            "  ",
            None,
        );
    }
    rows.push(Row::default());

    for field in Field::ALL {
        let focused = form.field == field;
        let marker = if focused { "▸ " } else { "  " };
        let label = field.label(lang);
        let pad = label_width.saturating_sub(text_width(label) + 2);
        let label_style = if focused {
            theme::TITLE_STYLE
        } else {
            Style::new().fg(theme::MUTED)
        };
        let lead = vec![
            Segment::new(marker, theme::TITLE_STYLE),
            Segment::new(format!("{label}{}", " ".repeat(pad)), label_style).hit(field, None),
        ];

        if field.is_text() {
            let input = if field == Field::Interests {
                &form.interests
            } else {
                &form.ready
            };
            let available = width.saturating_sub(label_width).max(1);
            let viewport = input.viewport(available, false);
            let mut segments = lead;
            let cursor = focused.then_some(label_width as u16 + viewport.cursor_x);
            if input.is_empty() {
                let placeholder = match field {
                    Field::Interests => lang.pick(
                        "e.g. multimodal LLM, program analysis",
                        "如：多模态大模型, 程序分析",
                    ),
                    _ => lang.pick(
                        "e.g. 2027-03-01, 3/1, in 3 months (empty: not set)",
                        "如：2027-03-01、3月1日、3个月后（留空表示未定）",
                    ),
                };
                segments.push(Segment::new(placeholder, theme::DIM).hit(field, None));
            } else {
                segments.push(Segment::new(viewport.text, theme::SEARCH_STYLE).hit(field, None));
            }
            rows.push(Row {
                segments,
                field: Some(field),
                cursor,
            });
            // The hint under the field.
            let hint = match field {
                Field::Interests => {
                    let names = form.recognised(lang);
                    if form.interests.is_empty() {
                        None
                    } else if names.is_empty() {
                        Some((
                            lang.pick(
                                "No topic recognised; try other words (e.g. computer vision, 数据库)",
                                "未识别出研究方向，换个说法试试（如：计算机视觉、数据库）",
                            )
                            .to_string(),
                            Style::new().fg(theme::WARNING),
                        ))
                    } else {
                        Some((
                            format!(
                                "{}{}",
                                lang.pick("Recognised: ", "识别为："),
                                names.join(" · ")
                            ),
                            Style::new().fg(theme::SUCCESS),
                        ))
                    }
                }
                _ => match form.ready_date(today()) {
                    Ok(Some(_)) => form
                        .date_hint(today(), lang)
                        .map(|h| (h, Style::new().fg(theme::SUCCESS))),
                    Ok(None) => None,
                    Err(InvalidDate) => Some((
                        lang.pick(
                            "Not a date yet: 2027-03-01, 3/1, 3月1日, 3个月后, in 3 months",
                            "还不是日期：可写 2027-03-01、3/1、3月1日、3个月后",
                        )
                        .to_string(),
                        Style::new().fg(theme::WARNING),
                    )),
                },
            };
            if let Some((text, style)) = hint {
                push_wrapped(
                    &mut rows,
                    vec![
                        Segment::new(indent.clone(), theme::DIM),
                        Segment::new(text, style),
                    ],
                    width,
                    &indent,
                    Some(field),
                );
            }
            continue;
        }

        // Choices, wrapped onto continuation lines.
        let radio = matches!(field, Field::Horizon);
        let mut current = Row {
            segments: lead,
            field: Some(field),
            cursor: None,
        };
        let mut used = label_width;
        for (index, choice) in form.choices(field, lang).iter().enumerate() {
            let mark = match (
                radio || (field == Field::Categories && index == 0),
                choice.checked,
            ) {
                _ if choice.implied => "[·]",
                (true, true) => "(•)",
                (true, false) => "( )",
                (false, true) => "[x]",
                (false, false) => "[ ]",
            };
            let text = format!("{mark} {}", choice.label);
            let w = text_width(&text) + 2;
            if used + w > width && used > label_width {
                rows.push(std::mem::take(&mut current));
                current.field = Some(field);
                current
                    .segments
                    .push(Segment::new(indent.clone(), theme::DIM));
                used = label_width;
            }
            let mut style = if choice.implied {
                theme::DIM
            } else if choice.checked {
                Style::new().fg(theme::TEXT).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme::MUTED)
            };
            if focused && form.option == index {
                style = style.patch(theme::SELECTED_STYLE);
            }
            current
                .segments
                .push(Segment::new(text, style).hit(field, Some(index)));
            current.segments.push(Segment::new("  ", theme::DIM));
            used += w;
        }
        let custom = form.custom_places(field);
        if !custom.is_empty() {
            let text = format!("+ {}", custom.join(", "));
            if used + text_width(&text) > width && used > label_width {
                rows.push(std::mem::take(&mut current));
                current.field = Some(field);
                current
                    .segments
                    .push(Segment::new(indent.clone(), theme::DIM));
            }
            current.segments.push(Segment::new(text, theme::DIM));
        }
        rows.push(current);
    }

    // Preview of what this profile recommends.
    rows.push(Row::default());
    let mut segments = vec![Segment::new(
        lang.pick("Preview: ", "预览："),
        theme::TITLE_STYLE,
    )];
    match &preview {
        Some(p) if p.candidates > 0 => {
            segments.push(Segment::new(
                match lang {
                    Lang::Zh => format!("{} 个候选", p.candidates),
                    Lang::En => format!("{} candidates", p.candidates),
                },
                Style::new().fg(theme::TEXT),
            ));
            for (tier, title) in &p.firsts {
                segments.push(Segment::new(" · ", theme::DIM));
                segments.push(Segment::new(
                    format!("{} ", tier_label(*tier, lang)),
                    tier_style(*tier),
                ));
                segments.push(Segment::new(title.clone(), Style::new().fg(theme::TEXT)));
            }
        }
        _ => segments.push(Segment::new(
            lang.pick(
                "no open conference matches yet; widen the ranks or the horizon",
                "暂无符合条件的会议，可放宽等级或截稿范围",
            ),
            Style::new().fg(theme::WARNING),
        )),
    }
    push_wrapped(&mut rows, segments, width, "  ", None);

    // Scroll so the focused field stays visible.
    let height = area.height as usize;
    let first = rows
        .iter()
        .position(|r| r.field == Some(form.field))
        .unwrap_or(0);
    let last = rows
        .iter()
        .rposition(|r| r.field == Some(form.field))
        .unwrap_or(0);
    let offset = if last + 1 > height {
        (last + 1 - height).min(first)
    } else {
        0
    };

    let mut cursor = None;
    for (index, row) in rows.into_iter().skip(offset).take(height).enumerate() {
        let y = area.y + index as u16;
        let mut x = area.x;
        let mut spans = Vec::new();
        for segment in row.segments {
            let w = text_width(&segment.text) as u16;
            if let Some((field, option)) = segment.hit {
                let w = w.min(area.right().saturating_sub(x));
                if w > 0 {
                    view.layout
                        .agent_form
                        .push((Rect::new(x, y, w, 1), field, option));
                }
            }
            x = x.saturating_add(w);
            spans.push(Span::styled(segment.text, segment.style));
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(area.x, y, area.width, 1),
        );
        if let Some(cx) = row.cursor {
            cursor = Some(Position::new(
                (area.x + cx).min(area.right().saturating_sub(1)),
                y,
            ));
        }
    }
    if let Some(position) = cursor {
        frame.set_cursor_position(position);
    }
}
