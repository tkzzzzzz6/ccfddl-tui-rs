//! Draws the `/history` list inside the agent panel.

use chrono::Utc;
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::agent::{Lang, Tier};

use super::agent_view::tier_style;
use super::app::App;
use super::line_input::text_width;
use super::theme;
use super::view::ViewState;

fn fit(text: &str, width: usize) -> String {
    if text_width(text) <= width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = super::line_input::char_width(c);
        if used + w + 1 > width {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push('…');
    out
}

pub fn render_history(frame: &mut Frame, app: &App, view: &mut ViewState, area: Rect) {
    let lang: Lang = app.language.into();
    let Some(list) = app.agent.history_list.as_ref() else {
        return;
    };
    let history = &app.agent.round_history;
    let visible = list.visible(history);
    let width = area.width as usize;
    let now = Utc::now();
    let mut rows: Vec<Line<'static>> = Vec::new();

    // Title and keys.
    let title = match lang {
        Lang::Zh => format!("推荐历史（{}）", history.rounds.len()),
        Lang::En => format!("Saved recommendations ({})", history.rounds.len()),
    };
    let keys = lang.pick(
        "  Enter open · p pin · r rename · d delete · e export · / filter · c compare · Esc back",
        "  Enter 打开 · p 置顶 · r 改名 · d 删除 · e 导出 · / 筛选 · c 对比 · Esc 返回",
    );
    rows.push(Line::from(vec![
        Span::styled(title.clone(), theme::TITLE_STYLE),
        Span::styled(
            fit(keys, width.saturating_sub(text_width(&title))),
            theme::DIM,
        ),
    ]));

    // Filter line.
    let filter_text = list.filter.text();
    let mut cursor = None;
    if list.filtering || !filter_text.is_empty() {
        let label = lang.pick("Filter: ", "筛选：");
        let shown = fit(&filter_text, width.saturating_sub(text_width(label)));
        if list.filtering {
            cursor = Some((
                rows.len(),
                text_width(label) + list.filter.viewport(width, false).cursor_x as usize,
            ));
        }
        rows.push(Line::from(vec![
            Span::styled(label, Style::new().fg(theme::ACCENT)),
            Span::styled(shown, theme::SEARCH_STYLE),
        ]));
    }
    rows.push(Line::default());

    // Leave room for the preview / comparison below the list.
    let footer_rows = 4usize;
    let first_row = rows.len();
    let room = (area.height as usize)
        .saturating_sub(first_row + footer_rows)
        .max(1);
    let offset = list.cursor.saturating_sub(room.saturating_sub(1));
    if visible.is_empty() {
        rows.push(Line::from(Span::styled(
            lang.pick("No round matches the filter.", "没有匹配的推荐。"),
            Style::new().fg(theme::WARNING),
        )));
    }
    for (position, index) in visible.iter().enumerate().skip(offset).take(room) {
        let round = &history.rounds[*index];
        let selected = position == list.cursor;
        let shown = app.current_round_id() == Some(round.id);
        let marker = if selected { "▸ " } else { "  " };
        let pin = if round.pinned { "⚑" } else { " " };
        let time = round
            .set
            .generated_at
            .with_timezone(&chrono::Local)
            .format("%m-%d %H:%M")
            .to_string();
        let closed = round.closed(now);
        let count = match (lang, closed) {
            (Lang::Zh, 0) => format!("{} 个", round.set.items.len()),
            (Lang::Zh, _) => format!("{} 个（{closed} 个已截止）", round.set.items.len()),
            (Lang::En, 0) => format!("{} picks", round.set.items.len()),
            (Lang::En, _) => format!("{} picks ({closed} closed)", round.set.items.len()),
        };
        let renaming = selected && list.renaming.is_some();
        let name = if renaming {
            list.renaming.as_ref().map(|n| n.text()).unwrap_or_default()
        } else {
            round.name.clone()
        };
        let fixed =
            text_width(marker) + text_width(pin) + 1 + time.len() + 2 + text_width(&count) + 2;
        let name_width = width.saturating_sub(fixed).max(4);
        let name = fit(&name, name_width);
        let pad = name_width.saturating_sub(text_width(&name));
        let base = if selected {
            theme::SELECTED_STYLE
        } else {
            Style::new()
        };
        let name_style = if renaming {
            theme::SEARCH_STYLE.add_modifier(Modifier::UNDERLINED)
        } else if shown {
            base.fg(theme::ACCENT)
        } else {
            base.fg(theme::TEXT)
        };
        if renaming {
            let prefix = text_width(marker) + text_width(pin) + 1 + time.len() + 2;
            let cx = list
                .renaming
                .as_ref()
                .map_or(0, |n| n.viewport(name_width, false).cursor_x as usize);
            cursor = Some((rows.len(), prefix + cx));
        }
        let mut spans = vec![
            Span::styled(marker, theme::TITLE_STYLE),
            Span::styled(pin, base.fg(theme::WARNING)),
            Span::styled(" ", base),
            Span::styled(time, base.fg(theme::MUTED)),
            Span::styled("  ", base),
            Span::styled(name, name_style),
            Span::styled(" ".repeat(pad + 2), base),
            Span::styled(
                count,
                base.fg(if closed > 0 {
                    theme::WARNING
                } else {
                    theme::MUTED
                }),
            ),
        ];
        if list.confirm_delete == Some(round.id) {
            spans.push(Span::styled(
                lang.pick("  d again to delete", "  再按 d 删除"),
                Style::new().fg(theme::ERROR).add_modifier(Modifier::BOLD),
            ));
        }
        let y = area.y + rows.len() as u16;
        if y < area.bottom() {
            view.layout
                .agent_history_rows
                .push((Rect::new(area.x, y, area.width, 1), position));
        }
        rows.push(Line::from(spans));
    }

    // Preview of the selected round, or the comparison.
    rows.push(Line::from(Span::styled(
        "─".repeat(width),
        theme::BORDER_STYLE,
    )));
    if let Some((_, cmp)) = &list.comparison {
        let label = |text: &str, items: &[String], style: Style| {
            let body = if items.is_empty() {
                lang.pick("none", "无").to_string()
            } else {
                items.join(", ")
            };
            Line::from(vec![
                Span::styled(text.to_string(), style.add_modifier(Modifier::BOLD)),
                Span::styled(
                    fit(&body, width.saturating_sub(text_width(text))),
                    Style::new().fg(theme::TEXT),
                ),
            ])
        };
        rows.push(label(
            lang.pick("Only here: ", "仅这一轮有："),
            &cmp.added,
            Style::new().fg(theme::SUCCESS),
        ));
        rows.push(label(
            lang.pick("Only on screen: ", "仅当前显示的有："),
            &cmp.removed,
            Style::new().fg(theme::ERROR),
        ));
        rows.push(Line::from(Span::styled(
            match lang {
                Lang::Zh => format!("两轮共有 {} 个 · Esc 关闭对比", cmp.common),
                Lang::En => format!("{} in both · Esc closes the comparison", cmp.common),
            },
            theme::DIM,
        )));
    } else if let Some(round) = list.selected(history).and_then(|id| history.get(id)) {
        let mut spans = vec![Span::styled(
            lang.pick("Preview: ", "预览："),
            theme::TITLE_STYLE,
        )];
        for tier in Tier::ALL {
            if let Some(item) = round.set.items.iter().find(|i| i.tier == tier) {
                spans.push(Span::styled(
                    format!("{} ", tier.label(lang)),
                    tier_style(tier),
                ));
                spans.push(Span::styled(
                    format!("{} {}  ", item.facts.title, item.facts.year),
                    Style::new().fg(theme::TEXT),
                ));
            }
        }
        rows.push(Line::from(spans));
        if !round.set.conditions.is_empty() {
            rows.push(Line::from(Span::styled(
                fit(
                    &format!(
                        "{}{}",
                        lang.pick("Conditions: ", "条件："),
                        round.set.conditions.join(" · ")
                    ),
                    width,
                ),
                Style::new().fg(theme::ACCENT),
            )));
        }
    }

    for (i, line) in rows.into_iter().enumerate().take(area.height as usize) {
        frame.render_widget(
            Paragraph::new(line),
            Rect::new(area.x, area.y + i as u16, area.width, 1),
        );
    }
    if let Some((row, x)) = cursor {
        let y = area.y + row as u16;
        if y < area.bottom() {
            frame.set_cursor_position(Position::new(
                (area.x + x as u16).min(area.right().saturating_sub(1)),
                y,
            ));
        }
    }
}
