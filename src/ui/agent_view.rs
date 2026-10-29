//! Drawing of the agent panel: transcript, recommendation cards and input.
//!
//! Like `render.rs`, every interactive region is recorded in the
//! [`LayoutMap`](super::view::LayoutMap) so mouse input uses the exact layout
//! of the last frame.

use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::agent::recommend::{Recommendation, Source, Tier};
use crate::agent::{Lang, topics};
use crate::format::get_countdown_color;

use super::agent::Entry;
use super::agent_quick::QuickAction;
use super::app::{App, Focus};
use super::line_input::{char_width, text_width};
use super::theme;
use super::view::ViewState;

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

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

/// Cut `text` to `width` cells, ending with `…` when shortened.
fn truncate(text: &str, width: usize) -> String {
    if text_width(text) <= width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = char_width(c);
        if used + w + 1 > width {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push('…');
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

pub(crate) fn tier_style(tier: Tier) -> Style {
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
                    theme::SEARCH_STYLE.add_modifier(Modifier::BOLD),
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
    expanded: bool,
    highlighted: bool,
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
    if selected && highlighted {
        header = header
            .into_iter()
            .map(|span| {
                let style = span.style.bg(theme::SELECTED_BG);
                span.style(style)
            })
            .collect();
    }
    let mut lines = vec![Line::from(header)];
    if !expanded {
        // Compact: one line of reasoning under the title line.
        let reason = item.reason.replace('\n', " ");
        let text = truncate(&reason, width.saturating_sub(4));
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(text, theme::DIM),
        ]));
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
    let cards_focused = app.focus == Focus::AgentCards;
    let panel_focused = focused || cards_focused;
    let status = if app.agent.is_busy() {
        lang.pick(" working ", " 运行中 ")
    } else {
        ""
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(if panel_focused {
            Style::new().fg(theme::PRIMARY)
        } else {
            theme::BORDER_STYLE
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
        let expanded = selected && app.agent.card_details;
        for line in card_lines(app, item, selected, expanded, cards_focused, width, now) {
            cards.push((index, line));
        }
    }
    let conditions = app
        .agent
        .recommendations
        .as_ref()
        .map(|set| set.conditions.clone())
        .unwrap_or_default();
    let header_rows = if conditions.is_empty() { 1 } else { 2 };
    let cards_height = if cards.is_empty() {
        0
    } else {
        // Compact cards are two lines each; leave the transcript at least
        // four rows above them.
        ((cards.len() + header_rows) as u16)
            .min(inner.height.saturating_sub(6))
            .max(3)
    };
    let quick = app.quick_actions_visible();
    let quick_height = if quick {
        (QuickAction::ALL.len() as u16 + 1).min(inner.height.saturating_sub(4))
    } else {
        0
    };
    let [transcript_area, quick_area, cards_area, input_area] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(quick_height),
        Constraint::Length(cards_height),
        Constraint::Length(2),
    ])
    .areas(inner);
    if quick_height > 0 {
        render_quick_actions(frame, app, view, quick_area, lang);
    }
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
        let mut title = match lang {
            Lang::Zh => format!("推荐 {} 个 · {source}", items.len()),
            Lang::En => format!("{} picks · {source}", items.len()),
        };
        if let Some(round) = app.round_label() {
            title = format!("{title} · {round}");
        }
        if !cards_focused {
            title.push_str(lang.pick(" · Tab to the cards", " · Tab 进入卡片区"));
        }
        let title_style = if cards_focused {
            theme::TITLE_STYLE
        } else {
            theme::SECTION_STYLE
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("── {title} "),
                title_style,
            ))),
            Rect::new(cards_area.x, cards_area.y, cards_area.width, 1),
        );
        if header_rows == 2 && cards_area.height > 2 {
            let label = lang.pick("Conditions: ", "条件：");
            let hint = lang.pick("  (/reset clears)", "  （/reset 清空）");
            let text = truncate(
                &format!("{label}{}", conditions.join(" · ")),
                width.saturating_sub(text_width(hint)),
            );
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(text, Style::new().fg(theme::ACCENT)),
                    Span::styled(hint, theme::DIM),
                ])),
                Rect::new(cards_area.x, cards_area.y + 1, cards_area.width, 1),
            );
        }
        let skip = if header_rows == 2 && cards_area.height > 2 {
            2
        } else {
            1
        };
        let body = Rect::new(
            cards_area.x,
            cards_area.y + skip,
            cards_area.width,
            cards_area.height.saturating_sub(skip),
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
            theme::BORDER_STYLE
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
    const PROMPT: &str = "› ";
    let prompt = Span::styled(PROMPT, theme::TITLE_STYLE);
    let prompt_width = text_width(PROMPT) as u16;
    let available = field.width.saturating_sub(prompt_width) as usize;
    let (line, cursor_x) = if app.agent.input.is_empty() {
        let placeholder = if app.agent.input_masked() {
            lang.pick("Paste the key (hidden)", "粘贴 Key（输入会被隐藏）")
        } else if app.agent.wizard.is_some() || app.agent.setup.is_some() {
            lang.pick("Answer the question above", "回答上面的问题")
        } else if focused {
            lang.pick(
                "Describe your research, or type / for commands",
                "描述你的研究方向，或输入 / 查看命令",
            )
        } else if cards_focused {
            lang.pick("Press i to type", "按 i 回到输入")
        } else {
            lang.pick("Press a to ask the agent", "按 a 向 agent 提问")
        };
        (
            Line::from(vec![prompt, Span::styled(placeholder, theme::DIM)]),
            0,
        )
    } else {
        // Scrolled so the cursor stays visible; an API key is drawn as
        // bullets only.
        let viewport = app
            .agent
            .input
            .viewport(available, app.agent.input_masked());
        (
            Line::from(vec![
                prompt,
                Span::styled(viewport.text, theme::SEARCH_STYLE),
            ]),
            viewport.cursor_x,
        )
    };
    frame.render_widget(Paragraph::new(line), field);
    if focused && field.height > 0 {
        // The real terminal cursor, so input methods (Chinese IME) place
        // their candidate window at the insertion point.
        let x = (field.x + prompt_width + cursor_x).min(field.right().saturating_sub(1));
        frame.set_cursor_position(Position::new(x, field.y));
    }

    if focused {
        render_completions(frame, app, view, inner, input_area, lang);
    }
}

/// First-screen shortcuts, triggered with 1-4 or a click.
fn render_quick_actions(
    frame: &mut Frame,
    app: &App,
    view: &mut ViewState,
    area: Rect,
    lang: Lang,
) {
    if area.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            lang.pick("── Shortcuts (press a number)", "── 快捷入口（按数字键）"),
            theme::SECTION_STYLE,
        ))),
        Rect::new(area.x, area.y, area.width, 1),
    );
    for (index, action) in QuickAction::ALL
        .iter()
        .enumerate()
        .take(area.height.saturating_sub(1) as usize)
    {
        let row = Rect::new(area.x, area.y + 1 + index as u16, area.width, 1);
        let hint = action.hint(app);
        let mut spans = vec![
            Span::styled(format!("  {} ", index + 1), theme::TITLE_STYLE),
            Span::styled(action.label(app), Style::new().fg(theme::TEXT)),
        ];
        if !hint.is_empty() {
            spans.push(Span::styled(format!("  {hint}"), theme::DIM));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), row);
        view.layout.agent_quick.push((row, index));
    }
}

/// The `/command` popup, drawn just above the input over the transcript.
fn render_completions(
    frame: &mut Frame,
    app: &App,
    view: &mut ViewState,
    panel: Rect,
    input_area: Rect,
    lang: Lang,
) {
    let matches = app.agent.completions();
    if matches.is_empty() {
        return;
    }
    const MAX_ROWS: usize = 8;
    let selected = app.agent.completion_cursor.min(matches.len() - 1);
    let rows = matches.len().min(MAX_ROWS);
    let height = (rows as u16).min(input_area.y.saturating_sub(panel.y));
    if height == 0 {
        return;
    }
    // Keep the selection visible when there are more matches than rows.
    let offset = (selected + 1).saturating_sub(height as usize);
    let area = Rect::new(panel.x, input_area.y - height, panel.width, height);
    frame.render_widget(Clear, area);
    let usage_width = matches
        .iter()
        .map(|c| text_width(&c.usage()))
        .max()
        .unwrap_or(0)
        .min(panel.width as usize / 2);
    for (row, (index, command)) in matches
        .iter()
        .enumerate()
        .skip(offset)
        .take(height as usize)
        .enumerate()
    {
        let row_area = Rect::new(area.x, area.y + row as u16, area.width, 1);
        let style = if index == selected {
            theme::SELECTED_STYLE
        } else {
            Style::new().bg(theme::SURFACE)
        };
        let usage = command.usage();
        let pad = usage_width.saturating_sub(text_width(&usage));
        let line = Line::from(vec![
            Span::styled(format!(" {usage}{} ", " ".repeat(pad)), theme::TITLE_STYLE),
            Span::styled(command.description(lang), Style::new().fg(theme::TEXT)),
        ])
        .style(style);
        frame.render_widget(Paragraph::new(line).style(style), row_area);
        view.layout.agent_completions.push((row_area, index));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::view::ViewState;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

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
        app.agent.input.set("/setup");
        app.submit_agent_input();
        app.agent.input.set("1");
        app.submit_agent_input();
        app.agent.input.set("sk-visible-secret");
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

    fn draw_app(app: &mut App, width: u16, height: u16) -> (Terminal<TestBackend>, ViewState) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut view = ViewState::new();
        terminal
            .draw(|frame| crate::ui::render::draw_with_view(frame, app, &mut view, Utc::now()))
            .unwrap();
        (terminal, view)
    }

    fn screen(terminal: &Terminal<TestBackend>) -> Vec<String> {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn terminal_cursor_follows_the_insertion_point() {
        use crate::ui::agent::tests::app_with;

        let (mut app, _dir) = app_with(None);
        app.open_agent();
        app.agent.input.set("会议ab");
        let (mut terminal, view) = draw_app(&mut app, 140, 30);
        let field = view.layout.agent_input;
        // Block top border, then "› " (2 cells) and 会议ab (6 cells).
        terminal
            .backend_mut()
            .assert_cursor_position((field.x + 2 + 6, field.y + 1));

        app.agent.input.home();
        app.agent.input.right();
        let (mut terminal, _) = draw_app(&mut app, 140, 30);
        terminal
            .backend_mut()
            .assert_cursor_position((field.x + 2 + 2, field.y + 1));
    }

    #[test]
    fn slash_popup_lists_matching_commands_above_the_input() {
        use crate::ui::agent::tests::app_with;

        let (mut app, _dir) = app_with(None);
        app.open_agent();
        app.agent.input.set("/");
        app.agent.input_edited();
        let (terminal, view) = draw_app(&mut app, 140, 30);
        let lines = screen(&terminal);
        let rows = &view.layout.agent_completions;
        assert_eq!(rows.len(), 8, "capped at eight rows");
        assert!(
            rows.iter()
                .all(|(area, _)| area.bottom() <= view.layout.agent_input.y)
        );
        assert!(
            lines.iter().any(|l| l.contains("/recommend [note]")),
            "{lines:?}"
        );

        app.agent.input.set("/exp");
        app.agent.input_edited();
        let (terminal, view) = draw_app(&mut app, 140, 30);
        assert_eq!(view.layout.agent_completions.len(), 1);
        let lines = screen(&terminal);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("/export [path]") && l.contains("Save recommendations")),
            "{lines:?}"
        );

        // Once dismissed nothing is drawn.
        app.agent.dismiss_completion();
        let (_, view) = draw_app(&mut app, 140, 30);
        assert!(view.layout.agent_completions.is_empty());
    }

    #[test]
    fn long_input_scrolls_instead_of_overflowing() {
        use crate::ui::agent::tests::app_with;

        let (mut app, _dir) = app_with(None);
        app.open_agent();
        let long = format!("{}END", "x".repeat(300));
        app.agent.input.set(&long);
        let (terminal, view) = draw_app(&mut app, 140, 30);
        let field_row = view.layout.agent_input.y + 1;
        let lines = screen(&terminal);
        assert!(
            lines[field_row as usize].contains("xEND"),
            "{:?}",
            lines[field_row as usize]
        );
    }

    #[test]
    fn shortcuts_render_and_respond_to_clicks() {
        use crate::ui::agent::tests::app_with;
        use crate::ui::input::handle_mouse;
        use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

        let (mut app, _dir) = app_with(None);
        app.open_agent();
        let (terminal, mut view) = draw_app(&mut app, 140, 30);
        let lines = screen(&terminal);
        assert_eq!(view.layout.agent_quick.len(), 4);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("1 Recommend for my profile")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("Connect a model  /setup")),
            "{lines:?}"
        );
        assert!(
            lines.last().unwrap().contains("Shortcut"),
            "footer: {:?}",
            lines.last()
        );

        let (area, _) = view.layout.agent_quick[2];
        handle_mouse(
            &mut app,
            &mut view,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: area.x + 3,
                row: area.y,
                modifiers: KeyModifiers::NONE,
            },
        );
        assert!(app.agent.wizard.is_some(), "clicking 3 starts /init");
        let (_, view) = draw_app(&mut app, 140, 30);
        assert!(view.layout.agent_quick.is_empty());
    }

    #[test]
    fn card_focus_changes_the_header_and_footer() {
        use crate::ui::agent::tests::app_with;

        let (mut app, _dir) = app_with(None);
        app.open_agent();
        app.agent.profile.horizon_days = 100_000;
        app.agent.input.set("computer vision database");
        app.submit_agent_input();
        while app.agent.is_busy() {
            app.poll_agent();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let (terminal, _) = draw_app(&mut app, 140, 30);
        let lines = screen(&terminal);
        assert!(
            lines.iter().any(|l| l.contains("Tab to the cards")),
            "{lines:?}"
        );

        app.focus = Focus::AgentCards;
        let (terminal, _) = draw_app(&mut app, 140, 30);
        let lines = screen(&terminal);
        assert!(!lines.iter().any(|l| l.contains("Tab to the cards")));
        let footer = lines.last().unwrap();
        assert!(
            footer.contains("Show in table") && footer.contains("Details"),
            "{footer}"
        );
        // Compact by default; the detail lines follow the `d` toggle.
        assert!(!lines.iter().any(|l| l.contains(" Due ")), "{lines:?}");
        app.toggle_card_details();
        let (terminal, _) = draw_app(&mut app, 140, 30);
        assert!(screen(&terminal).iter().any(|l| l.contains(" Due ")));
    }

    #[test]
    fn eight_compact_cards_fit_a_100_by_30_terminal() {
        use crate::agent::recommend::Source;
        use crate::ui::agent::tests::app_with;

        let (mut app, _dir) = app_with(None);
        app.open_agent();
        app.agent.profile.horizon_days = 100_000;
        app.agent.input.set("computer vision database");
        app.submit_agent_input();
        while app.agent.is_busy() {
            app.poll_agent();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // Eight cards, as the recommender produces by default.
        let mut set = app.agent.recommendations.clone().unwrap();
        let one = set.items[0].clone();
        set.items = (0..8).map(|_| one.clone()).collect();
        set.source = Source::Rules;
        app.agent.push_recommendations(set);
        app.focus = Focus::AgentCards;

        let (terminal, view) = draw_app(&mut app, 100, 30);
        let shown: std::collections::BTreeSet<usize> =
            view.layout.agent_cards.iter().map(|(_, i)| *i).collect();
        assert_eq!(shown.len(), 8, "{:?}", screen(&terminal));
        // Two lines per card: the title line and one line of reasoning.
        assert_eq!(view.layout.agent_cards.len(), 16);
    }

    #[test]
    fn full_size_panel_hides_the_table_while_focused() {
        use crate::ui::agent::tests::app_with;

        let (mut app, _dir) = app_with(None);
        app.open_agent();
        let (_, view) = draw_app(&mut app, 160, 30);
        let half = view.layout.agent.unwrap();
        assert!(half.width < 100, "{half:?}");
        assert!(view.layout.table.width > 0);

        app.cycle_panel_size();
        let (_, view) = draw_app(&mut app, 160, 30);
        assert_eq!(view.layout.agent.unwrap().width, 160);
        assert_eq!(view.layout.table.width, 0);

        // The table comes back when it takes the focus.
        app.focus = Focus::Table;
        let (_, view) = draw_app(&mut app, 160, 30);
        assert!(view.layout.table.width > 0);
    }

    #[test]
    fn truncate_marks_cut_text() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("abcdefghij", 5), "abcd…");
        assert_eq!(truncate("会议推荐理由", 5), "会议…");
    }

    #[test]
    fn widths_count_cjk_as_two_cells() {
        assert_eq!(text_width("ab"), 2);
        assert_eq!(text_width("会议"), 4);
    }
}
