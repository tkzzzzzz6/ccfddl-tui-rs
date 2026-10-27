//! Drawing of the agent panel: transcript, recommendation cards and input.
//!
//! Like `render.rs`, every interactive region is recorded in the
//! [`LayoutMap`](super::view::LayoutMap) so mouse input uses the exact layout
//! of the last frame.

use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use crate::agent::recommend::{Recommendation, Source, Tier};
use crate::agent::{Lang, topics};
use crate::format::get_countdown_color;

use super::agent::Entry;
use super::app::{App, Focus};
use super::theme;
use super::view::ViewState;

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn char_width(c: char) -> usize {
    if c.is_ascii() {
        usize::from(!c.is_ascii_control())
    } else {
        let mut buf = [0u8; 4];
        Span::raw(&*c.encode_utf8(&mut buf)).width()
    }
}

pub fn text_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// Greedy wrap by display width. ASCII words are kept whole when possible;
/// CJK text breaks between any two characters.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        let mut line_width = 0;
        for c in paragraph.chars() {
            let w = char_width(c);
            if line_width + w > width && !line.is_empty() {
                // Prefer breaking at the last space of an ASCII word.
                let split = if c.is_ascii_alphanumeric() {
                    line.rfind(' ').filter(|i| *i > 0)
                } else {
                    None
                };
                match split {
                    Some(index) => {
                        let rest = line[index + 1..].to_string();
                        line.truncate(index);
                        out.push(std::mem::take(&mut line));
                        line_width = text_width(&rest);
                        line = rest;
                    }
                    None => {
                        out.push(std::mem::take(&mut line));
                        line_width = 0;
                    }
                }
                if c == ' ' && line.is_empty() {
                    continue;
                }
            }
            line.push(c);
            line_width += w;
        }
        out.push(line);
    }
    out
}

/// Wrap `text` with `first` as the prefix of the first line and spaces of
/// the same width on following lines.
fn prefixed(
    lines: &mut Vec<Line<'static>>,
    first: Span<'static>,
    text: &str,
    style: Style,
    width: usize,
) {
    let indent = text_width(&first.content);
    for (index, chunk) in wrap(text, width.saturating_sub(indent))
        .into_iter()
        .enumerate()
    {
        let lead = if index == 0 {
            first.clone()
        } else {
            Span::raw(" ".repeat(indent))
        };
        lines.push(Line::from(vec![lead, Span::styled(chunk, style)]));
    }
}

fn tier_style(tier: Tier) -> Style {
    let color = match tier {
        Tier::Stretch => theme::ERROR,
        Tier::Main => theme::WARNING,
        Tier::Safe => theme::SUCCESS,
    };
    Style::new().fg(color).add_modifier(Modifier::BOLD)
}

fn transcript_lines(app: &App, width: usize, now: DateTime<Utc>) -> Vec<Line<'static>> {
    let lang: Lang = app.language.into();
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (index, entry) in app.agent.entries.iter().enumerate() {
        match entry {
            Entry::User(text) => {
                if index > 0 {
                    lines.push(Line::default());
                }
                prefixed(
                    &mut lines,
                    Span::styled("› ", theme::TITLE_STYLE),
                    text,
                    Style::new()
                        .fg(ratatui::style::Color::White)
                        .add_modifier(Modifier::BOLD),
                    width,
                );
            }
            Entry::Assistant(text) => prefixed(
                &mut lines,
                Span::styled("● ", Style::new().fg(theme::PRIMARY)),
                text.trim_start_matches('\n'),
                Style::new().fg(theme::TEXT),
                width,
            ),
            Entry::Tool { name, summary } => {
                let label = tool_label(name, lang);
                lines.push(Line::from(vec![
                    Span::styled("⏺ ", Style::new().fg(theme::SUCCESS)),
                    Span::styled(label, theme::SECTION_STYLE),
                ]));
                let detail = summary
                    .clone()
                    .unwrap_or_else(|| lang.pick("running…", "执行中…").to_string());
                prefixed(
                    &mut lines,
                    Span::styled("  ⎿ ", theme::DIM),
                    &detail,
                    theme::DIM,
                    width,
                );
            }
            Entry::Info(text) => prefixed(
                &mut lines,
                Span::raw("  "),
                text,
                Style::new().fg(theme::MUTED),
                width,
            ),
            Entry::Error(text) => prefixed(
                &mut lines,
                Span::styled("✗ ", Style::new().fg(theme::ERROR)),
                text,
                Style::new().fg(theme::ERROR),
                width,
            ),
        }
    }
    if app.agent.is_busy() {
        let frame = (now.timestamp_subsec_millis() / 100) as usize % SPINNER.len();
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", SPINNER[frame]), theme::TITLE_STYLE),
            Span::styled(
                lang.pick("Thinking… (Esc to stop)", "思考中…（Esc 中断）"),
                theme::DIM,
            ),
        ]));
    }
    lines
}

fn tool_label(name: &str, lang: Lang) -> String {
    let friendly = match name {
        "get_profile" => lang.pick("read profile", "读取画像"),
        "rank_candidates" => lang.pick("score candidates", "规则打分"),
        "search_conferences" => lang.pick("search conferences", "检索会议"),
        "get_conference" => lang.pick("conference details", "查看会议详情"),
        "submit_recommendations" => lang.pick("submit recommendations", "提交推荐"),
        _ => "",
    };
    if friendly.is_empty() {
        name.to_string()
    } else {
        format!("{name} · {friendly}")
    }
}

/// Lines for one card; the selected card is expanded.
fn card_lines(
    app: &App,
    item: &Recommendation,
    selected: bool,
    width: usize,
    now: DateTime<Utc>,
) -> Vec<Line<'static>> {
    let lang: Lang = app.language.into();
    let facts = &item.facts;
    let (countdown, countdown_style) = match facts.days_left(now) {
        Some(days) => (
            match lang {
                Lang::Zh => format!("{days}天"),
                Lang::En => format!("{days}d"),
            },
            theme::countdown_style(get_countdown_color(days)),
        ),
        None => (lang.pick("TBD", "待定").to_string(), theme::DIM),
    };
    let favorite = if app.is_card_favorite(item) {
        Span::styled(" ★", Style::new().fg(theme::WARNING))
    } else {
        Span::raw("")
    };
    let marker = if selected { "▸ " } else { "  " };
    let mut header = vec![
        Span::styled(marker, theme::TITLE_STYLE),
        Span::styled(format!("{} ", item.tier.label(lang)), tier_style(item.tier)),
        Span::styled(format!("{} {}", facts.title, facts.year), theme::BOLD),
        Span::styled("  CCF-", theme::DIM),
        Span::styled(
            facts.ccf.clone(),
            Style::new().fg(theme::ccf_rank_color(&facts.ccf)),
        ),
        Span::styled(
            format!("  {}", facts.sub),
            Style::new().fg(theme::sub_color(&facts.sub)),
        ),
        Span::raw("  "),
        Span::styled(countdown, countdown_style),
        favorite,
    ];
    if selected {
        header = header
            .into_iter()
            .map(|span| {
                let style = span.style.bg(theme::SELECTED_BG);
                span.style(style)
            })
            .collect();
    }
    let mut lines = vec![Line::from(header)];
    if !selected {
        return lines;
    }

    let label = |text: &'static str| Span::styled(format!("    {text} "), theme::TITLE_STYLE);
    let mut deadline = facts.deadline_text.clone();
    if !facts.place.is_empty() {
        deadline.push_str(&format!(" · {}", facts.place));
    }
    prefixed(
        &mut lines,
        label(lang.pick("Due", "截稿")),
        &deadline,
        theme::BOLD,
        width,
    );
    if let Some(core) = &facts.core {
        let mut ranks = format!("CORE {core}");
        if let Some(thcpl) = &facts.thcpl {
            ranks.push_str(&format!(" · THCPL {thcpl}"));
        }
        prefixed(
            &mut lines,
            label(lang.pick("Rank", "等级")),
            &ranks,
            theme::DIM,
            width,
        );
    }
    prefixed(
        &mut lines,
        label(lang.pick("Why", "理由")),
        &item.reason,
        Style::new().fg(theme::TEXT),
        width,
    );
    prefixed(
        &mut lines,
        label(lang.pick("Idea", "方向")),
        &item.direction,
        Style::new().fg(theme::TEXT),
        width,
    );
    if !item.risks.is_empty() {
        prefixed(
            &mut lines,
            label(lang.pick("Risk", "风险")),
            &item.risks.join(lang.pick("; ", "；")),
            Style::new().fg(theme::WARNING),
            width,
        );
    }
    let venue = topics::venue_tags(&facts.title, &facts.description);
    if !venue.is_empty() {
        let names: Vec<&str> = venue
            .iter()
            .map(|t| topics::tag_name(t, lang.is_zh()))
            .collect();
        prefixed(
            &mut lines,
            label(lang.pick("Topic", "主题")),
            &names.join(lang.pick(", ", "、")),
            theme::DIM,
            width,
        );
    }
    lines
}

/// Draw the panel and record its hit regions.
pub fn render_agent(
    frame: &mut Frame,
    app: &mut App,
    view: &mut ViewState,
    area: Rect,
    now: DateTime<Utc>,
) {
    let lang: Lang = app.language.into();
    view.layout.agent = Some(area);
    let focused = app.focus == Focus::Agent;
    let status = if app.agent.is_busy() {
        lang.pick(" working ", " 运行中 ")
    } else {
        ""
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(if focused {
            Style::new().fg(theme::PRIMARY)
        } else {
            theme::DIM
        })
        .title(Span::styled(
            format!(" Agent · {} ", app.agent.model_label()),
            theme::TITLE_STYLE,
        ))
        .title_bottom(Line::from(Span::styled(status, theme::TITLE_STYLE)).right_aligned())
        .style(theme::SIDEBAR_STYLE);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 4 || inner.width < 10 {
        return;
    }
    let width = inner.width as usize;

    // Cards first, so the transcript gets whatever is left.
    let items = app
        .agent
        .recommendations
        .as_ref()
        .map(|set| set.items.clone())
        .unwrap_or_default();
    let mut cards: Vec<(usize, Line<'static>)> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let selected = index == app.agent.card_cursor;
        for line in card_lines(app, item, selected, width, now) {
            cards.push((index, line));
        }
    }
    let cards_height = if cards.is_empty() {
        0
    } else {
        ((cards.len() + 1) as u16)
            .min(inner.height.saturating_sub(4) * 3 / 5)
            .max(3)
    };
    let [transcript_area, cards_area, input_area] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(cards_height),
        Constraint::Length(2),
    ])
    .areas(inner);
    view.layout.agent_transcript = transcript_area;
    view.layout.agent_cards_area = cards_area;
    view.layout.agent_input = input_area;

    // --- Transcript -----------------------------------------------------
    let lines = transcript_lines(app, width, now);
    let height = transcript_area.height as usize;
    let max_scroll = lines.len().saturating_sub(height);
    app.agent.scroll = app.agent.scroll.min(max_scroll);
    let start = max_scroll - app.agent.scroll;
    let visible: Vec<Line> = lines.into_iter().skip(start).take(height).collect();
    frame.render_widget(Paragraph::new(visible), transcript_area);
    if app.agent.scroll > 0 {
        let hint = lang.pick(" ↓ more (PgDn) ", " ↓ 更多 (PgDn) ");
        let w = text_width(hint) as u16;
        if transcript_area.width > w {
            frame.render_widget(
                Paragraph::new(Span::styled(hint, theme::SELECTED_STYLE)),
                Rect::new(
                    transcript_area.right() - w,
                    transcript_area.bottom().saturating_sub(1),
                    w,
                    1,
                ),
            );
        }
    }

    // --- Cards ----------------------------------------------------------
    if cards_height > 0 {
        let source = match app.agent.recommendations.as_ref().map(|s| &s.source) {
            Some(Source::Llm(model)) => model.clone(),
            _ => lang.pick("offline rules", "离线规则").to_string(),
        };
        let title = match lang {
            Lang::Zh => format!("推荐 {} 个 · {source} · ↑↓ 选择 · 回车跳转", items.len()),
            Lang::En => format!("{} picks · {source} · ↑↓ select · Enter jump", items.len()),
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("── {title} "),
                theme::SECTION_STYLE,
            ))),
            Rect::new(cards_area.x, cards_area.y, cards_area.width, 1),
        );
        let body = Rect::new(
            cards_area.x,
            cards_area.y + 1,
            cards_area.width,
            cards_area.height.saturating_sub(1),
        );
        let page = body.height as usize;
        // Keep the selected card (all of its lines) in view.
        let first = cards
            .iter()
            .position(|(i, _)| *i == app.agent.card_cursor)
            .unwrap_or(0);
        let last = cards
            .iter()
            .rposition(|(i, _)| *i == app.agent.card_cursor)
            .unwrap_or(0);
        let offset = if last + 1 > page {
            (last + 1 - page).min(first)
        } else {
            0
        };
        for (row, (index, line)) in cards.into_iter().skip(offset).take(page).enumerate() {
            let line_area = Rect::new(body.x, body.y + row as u16, body.width, 1);
            frame.render_widget(Paragraph::new(line), line_area);
            view.layout.agent_cards.push((line_area, index));
        }
    }

    // --- Input ----------------------------------------------------------
    let input_block = Block::default()
        .borders(Borders::TOP)
        .border_style(if focused {
            Style::new().fg(theme::PRIMARY)
        } else {
            theme::DIM
        })
        .title(Span::styled(
            match (&app.agent.setup, &app.agent.wizard) {
                (Some(setup), _) => setup.title(),
                (None, Some(wizard)) => format!(" /init {}/5 ", wizard.step.index() + 1),
                (None, None) => String::new(),
            },
            theme::TITLE_STYLE,
        ));
    let field = input_block.inner(input_area);
    frame.render_widget(input_block, input_area);
    let prompt = Span::styled("› ", theme::TITLE_STYLE);
    let available = (field.width as usize).saturating_sub(3);
    let line = if app.agent.input.is_empty() {
        let placeholder = if app.agent.input_masked() {
            lang.pick("Paste the key (hidden)", "粘贴 Key（输入会被隐藏）")
        } else if app.agent.wizard.is_some() || app.agent.setup.is_some() {
            lang.pick("Answer the question above", "回答上面的问题")
        } else if focused {
            lang.pick(
                "Describe your research, or /help",
                "描述你的研究方向，或输入 /help",
            )
        } else {
            lang.pick("Press a to ask the agent", "按 a 向 agent 提问")
        };
        let mut spans = vec![prompt];
        if focused {
            spans.push(Span::styled("▏", theme::TITLE_STYLE));
        }
        spans.push(Span::styled(placeholder, theme::DIM));
        Line::from(spans)
    } else {
        // Show the end of long input, like a single-line text field. An API
        // key is drawn as bullets only.
        let masked;
        let input: &str = if app.agent.input_masked() {
            masked = "•".repeat(app.agent.input.chars().count());
            &masked
        } else {
            &app.agent.input
        };
        let mut shown: Vec<char> = Vec::new();
        let mut used = 0;
        for c in input.chars().rev() {
            let w = char_width(c);
            if used + w > available {
                break;
            }
            used += w;
            shown.push(c);
        }
        let text: String = shown.into_iter().rev().collect();
        Line::from(vec![
            prompt,
            Span::styled(text, theme::SEARCH_STYLE),
            Span::styled(if focused { "▏" } else { "" }, theme::TITLE_STYLE),
        ])
    };
    frame.render_widget(Paragraph::new(line), field);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_respects_display_width() {
        assert_eq!(wrap("hello world foo", 11), ["hello world", "foo"]);
        assert_eq!(wrap("会议推荐理由", 4), ["会议", "推荐", "理由"]);
        assert_eq!(wrap("a\nb", 10), ["a", "b"]);
        assert_eq!(wrap("", 5), [""]);
        // A word longer than the line is split rather than looping.
        assert_eq!(wrap("abcdefgh", 3), ["abc", "def", "gh"]);
    }

    #[test]
    fn api_keys_are_drawn_as_bullets() {
        use crate::ui::agent::tests::app_with;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let (mut app, _dir) = app_with(None);
        app.open_agent();
        app.agent.input = "/setup".into();
        app.submit_agent_input();
        app.agent.input = "1".into();
        app.submit_agent_input();
        app.agent.input = "sk-visible-secret".into();
        let mut terminal = Terminal::new(TestBackend::new(140, 30)).unwrap();
        terminal
            .draw(|frame| crate::ui::render::draw(frame, &mut app, Utc::now()))
            .unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(!screen.contains("sk-visible"), "key leaked to the screen");
        assert!(screen.contains("•••••"));
        assert!(screen.contains("/setup 2/5"));
    }

    #[test]
    fn widths_count_cjk_as_two_cells() {
        assert_eq!(text_width("ab"), 2);
        assert_eq!(text_width("会议"), 4);
    }
}
