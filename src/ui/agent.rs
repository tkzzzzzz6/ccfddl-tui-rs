//! Agent panel behaviour: conversation state, slash commands, the `/init`
//! questionnaire and the link between recommendation cards and the table.
//!
//! Like `app.rs` this module is pure logic; `agent_view.rs` draws it and
//! `input.rs` routes keys and clicks to the methods here.

use std::path::{Path, PathBuf};

use chrono::Utc;

use std::sync::Arc;
use std::sync::mpsc::Receiver;

use crate::agent::config::Overrides;
use crate::agent::llm::{ChatMessage, LlmClient, LlmConfig, LlmError, OpenAiClient, list_models};
use crate::agent::profile::Profile;
use crate::agent::providers;
use crate::agent::recommend::RecommendationSet;
use crate::agent::refine::Constraints;
use crate::agent::report::export_markdown;
use crate::agent::session::AgentEvent;
use crate::agent::session_store::{self, SavedSession};
use crate::agent::tools::ToolContext;
use crate::agent::worker::{self, AgentHandle, AgentJob};
use crate::agent::{Lang, Recommendation};
use crate::data::ConferenceRow;

use super::agent_input::SLASH_COMMANDS;
use super::agent_results::{CardsView, PanelSize};
use super::agent_setup::{SetupEvent, SetupWizard};
use super::app::{App, Focus, Language};
use super::browser;
use super::line_input::LineInput;
use super::profile_form::ProfileForm;

impl From<Language> for Lang {
    fn from(language: Language) -> Self {
        match language {
            Language::En => Lang::En,
            Language::Zh => Lang::Zh,
        }
    }
}

/// Copies text somewhere the user can paste it from.
pub type Clipboard = Arc<dyn Fn(&str) -> std::io::Result<()> + Send + Sync>;

/// One block of the transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    User(String),
    Assistant(String),
    Tool {
        name: String,
        summary: Option<String>,
    },
    Info(String),
    Error(String),
}

pub type ClientFactory = Box<dyn Fn(&LlmConfig) -> Box<dyn LlmClient>>;
/// Fetches `GET /models`; runs on a worker thread.
pub type ModelsFetcher = Arc<dyn Fn(&LlmConfig) -> Result<Vec<String>, LlmError> + Send + Sync>;
/// Environment lookup, injectable so tests never read real variables.
pub type EnvLookup = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

pub struct AgentPanel {
    pub open: bool,
    pub input: LineInput,
    /// Sent inputs for Up/Down (never API keys or questionnaire answers).
    pub input_history: Vec<String>,
    /// Position while browsing `input_history`; `None` when editing.
    pub history_index: Option<usize>,
    /// The unsent text saved when history browsing started.
    pub history_draft: String,
    /// Selected row of the slash-command popup.
    pub completion_cursor: usize,
    /// Esc closed the popup; it re-opens on the next edit.
    pub completion_dismissed: bool,
    pub entries: Vec<Entry>,
    /// Conversation sent to the model on the next turn.
    pub history: Vec<ChatMessage>,
    pub handle: Option<AgentHandle>,
    pub recommendations: Option<RecommendationSet>,
    pub card_cursor: usize,
    /// Show the details of the selected card (`d` toggles).
    pub card_details: bool,
    /// The last few recommendation rounds, oldest first.
    pub rounds: Vec<RecommendationSet>,
    /// Which round `recommendations` shows.
    pub round_cursor: usize,
    pub panel_size: PanelSize,
    /// How the cards are shown now, and the user's last choice.
    pub cards_view: CardsView,
    pub cards_view_pref: CardsView,
    /// The running turn produced cards (otherwise its text answer gets room).
    pub turn_cards: bool,
    /// Offline follow-up conditions accumulated over turns (`/reset`).
    pub constraints: Constraints,
    /// Width chosen by dragging the divider or with `<` / `>`.
    pub panel_width: Option<u16>,
    /// Allowed panel widths in the last frame (side-by-side layout only).
    pub panel_bounds: Option<(u16, u16)>,
    /// Width of the panel in the last side-by-side frame.
    pub panel_shown_width: u16,
    /// Where `y` copies a card; OSC 52 to the terminal by default.
    pub clipboard: Clipboard,
    /// `agent-session.json`; `None` disables saving.
    pub session_path: Option<PathBuf>,
    /// The saved session was read; only then is it written back.
    pub session_loaded: bool,
    /// Transcript lines scrolled up from the bottom.
    pub scroll: usize,
    pub profile: Profile,
    pub profile_path: PathBuf,
    profile_loaded: bool,
    /// `profile.yaml` exists; without it, shortcut 1 opens the form first.
    pub profile_exists: bool,
    pub llm: Option<LlmConfig>,
    pub client_factory: ClientFactory,
    /// The research-profile form (`/init`, `/profile`, `p`).
    pub form: Option<ProfileForm>,
    /// Cached preview of the form's draft.
    pub form_preview: Option<(Profile, super::agent_profile::Preview)>,
    /// `/setup` questionnaire for the model connection.
    pub setup: Option<SetupWizard>,
    /// Background model-list or connection-test request.
    pub setup_rx: Option<Receiver<SetupEvent>>,
    /// Models from the last `/model list`, for `/model set <number>`.
    pub last_models: Vec<String>,
    pub config_path: PathBuf,
    /// CLI overrides, reapplied when switching profiles.
    pub overrides: Overrides,
    /// Shown in the welcome message, e.g. a config error at startup.
    pub notice: Option<String>,
    pub env: EnvLookup,
    pub models_fetcher: ModelsFetcher,
}

impl AgentPanel {
    /// `config.yaml` defaults to the profile's directory (`~/.ccfddl`).
    pub fn new(profile_path: PathBuf, llm: Option<LlmConfig>) -> Self {
        let config_path = profile_path
            .parent()
            .map(|dir| dir.join("config.yaml"))
            .unwrap_or_else(|| PathBuf::from("config.yaml"));
        let session_path = profile_path
            .parent()
            .map(|dir| dir.join("agent-session.json"));
        Self {
            open: false,
            input: LineInput::new(),
            input_history: Vec::new(),
            history_index: None,
            history_draft: String::new(),
            completion_cursor: 0,
            completion_dismissed: false,
            entries: Vec::new(),
            history: Vec::new(),
            handle: None,
            recommendations: None,
            card_cursor: 0,
            card_details: false,
            rounds: Vec::new(),
            round_cursor: 0,
            panel_size: PanelSize::Half,
            constraints: Constraints::default(),
            cards_view: CardsView::default(),
            cards_view_pref: CardsView::default(),
            turn_cards: false,
            session_path,
            session_loaded: false,
            panel_width: None,
            panel_bounds: None,
            panel_shown_width: 0,
            clipboard: Arc::new(super::clipboard::copy_to_terminal),
            scroll: 0,
            profile: Profile::default(),
            profile_path,
            profile_loaded: false,
            profile_exists: false,
            llm,
            client_factory: Box::new(|config| Box::new(OpenAiClient::new(config.clone()))),
            form: None,
            form_preview: None,
            setup: None,
            setup_rx: None,
            last_models: Vec::new(),
            config_path,
            overrides: Overrides::default(),
            notice: None,
            env: Arc::new(|key| std::env::var(key).ok()),
            models_fetcher: Arc::new(list_models),
        }
    }

    pub fn with_config_path(mut self, path: PathBuf) -> Self {
        self.config_path = path;
        self
    }

    pub fn with_overrides(mut self, overrides: Overrides) -> Self {
        self.overrides = overrides;
        self
    }

    pub fn with_notice(mut self, notice: Option<String>) -> Self {
        self.notice = notice;
        self
    }

    /// A request or a `/setup` check is in flight.
    pub fn is_busy(&self) -> bool {
        self.handle.is_some() || self.setup_rx.is_some()
    }

    /// Mask the input while `/setup` asks for an API key.
    pub fn input_masked(&self) -> bool {
        self.setup.as_ref().is_some_and(SetupWizard::masks_input)
    }

    pub fn is_running(&self) -> bool {
        self.handle.is_some()
    }

    pub fn model_label(&self) -> String {
        match &self.llm {
            Some(config) => config.label(),
            None => "offline".to_string(),
        }
    }

    pub fn selected_card(&self) -> Option<&Recommendation> {
        self.recommendations
            .as_ref()
            .and_then(|set| set.items.get(self.card_cursor))
    }

    pub(super) fn info(&mut self, text: impl Into<String>) {
        self.entries.push(Entry::Info(text.into()));
        self.scroll = 0;
    }

    pub(super) fn error(&mut self, text: impl Into<String>) {
        self.entries.push(Entry::Error(text.into()));
        self.scroll = 0;
    }

    fn export_dir(&self) -> PathBuf {
        self.profile_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    pub(super) fn ensure_profile(&mut self, lang: Lang) {
        if self.profile_loaded {
            return;
        }
        self.profile_loaded = true;
        match Profile::load(&self.profile_path) {
            Ok(Some(profile)) => {
                self.profile = profile;
                self.profile_exists = true;
            }
            Ok(None) => {}
            Err(error) => self.error(match lang {
                Lang::Zh => format!("读取画像失败：{error}"),
                Lang::En => format!("Could not read profile: {error}"),
            }),
        }
    }

    /// Write the input history, the newest round and the follow-up
    /// conditions. Best effort; nothing is written before the saved session
    /// was read, so an unopened panel never clears it.
    pub fn save_session(&self) {
        let Some(path) = self.session_path.as_deref().filter(|_| self.session_loaded) else {
            return;
        };
        let _ = session_store::save(
            path,
            &SavedSession {
                input_history: self.input_history.clone(),
                round: self.rounds.last().cloned(),
                constraints: self.constraints.clone(),
                cards_view: Some(self.cards_view_pref.key().to_string()),
            },
        );
    }

    fn restore_session(&mut self, conferences: &[crate::model::Conference], lang: Lang) {
        if self.session_loaded {
            return;
        }
        self.session_loaded = true;
        let Some(path) = self.session_path.clone() else {
            return;
        };
        match session_store::load(&path, conferences, Utc::now()) {
            Ok(None) => {}
            Ok(Some(restored)) => {
                let SavedSession {
                    input_history,
                    round,
                    constraints,
                    cards_view,
                } = restored.session;
                self.input_history = input_history;
                self.constraints = constraints;
                if let Some(view) = cards_view.as_deref().and_then(CardsView::from_key) {
                    self.cards_view_pref = view;
                    self.cards_view = view;
                }
                if let Some(round) = round {
                    self.push_recommendations(round);
                }
                if restored.saved > 0 {
                    let closed = restored.saved - restored.kept;
                    self.info(match (lang, closed) {
                        (Lang::Zh, 0) => format!(
                            "已恢复上次的推荐（{} 个）。/forget 可清除保存的会话。",
                            restored.kept
                        ),
                        (Lang::Zh, _) => format!(
                            "已恢复上次的推荐（{}/{}，{closed} 个已截止或不在数据中）。",
                            restored.kept, restored.saved
                        ),
                        (Lang::En, 0) => format!(
                            "Restored the last recommendations ({}). /forget deletes the saved session.",
                            restored.kept
                        ),
                        (Lang::En, _) => format!(
                            "Restored the last recommendations ({}/{}; {closed} closed or no longer in the data).",
                            restored.kept, restored.saved
                        ),
                    });
                }
            }
            Err(error) => self.error(match lang {
                Lang::Zh => format!("上次的会话无法读取，已忽略：{error}"),
                Lang::En => format!("Ignored an unreadable saved session: {error}"),
            }),
        }
    }

    fn welcome(&mut self, lang: Lang) {
        self.info(lang.pick(
            "Tell me what you work on and when the paper will be ready, e.g. \"multimodal LLM, ready 2027-03-01\", or pick a shortcut below.",
            "说说你的研究方向和论文完成时间，例如「多模态大模型，2027-03-01 写完」，或选择下方的快捷入口。",
        ));
        match &self.llm {
            Some(config) => {
                let text = match lang {
                    Lang::Zh => {
                        format!("模型：{}（/model 切换，/config 查看配置）", config.label())
                    }
                    Lang::En => format!(
                        "Model: {} (/model to switch, /config for details)",
                        config.label()
                    ),
                };
                self.info(text);
            }
            None => {
                if let Some(notice) = self.notice.clone() {
                    self.error(match lang {
                        Lang::Zh => format!("模型配置有误，暂用离线模式：{notice}"),
                        Lang::En => {
                            format!("Model configuration problem, staying offline: {notice}")
                        }
                    });
                }
                self.info(lang.pick(
                    "Offline mode: rule-based recommendations.",
                    "离线模式：使用规则推荐。",
                ));
                let detected = providers::detect_env_keys(self.env.as_ref());
                if let Some((preset, var)) = detected.first() {
                    let text = match lang {
                        Lang::Zh => format!(
                            "检测到 {var}：选择快捷入口 4 并连按回车即可启用 {}。",
                            preset.name_zh
                        ),
                        Lang::En => format!(
                            "Found {var}: shortcut 4, then Enter through the defaults, enables {}.",
                            preset.name
                        ),
                    };
                    self.info(text);
                }
            }
        }
    }
}

impl App {
    fn lang(&self) -> Lang {
        self.language.into()
    }

    /// Open the panel (or move focus to it) from the table or sidebar.
    pub fn open_agent(&mut self) {
        let lang = self.lang();
        let first = !self.agent.open && self.agent.entries.is_empty();
        self.agent.open = true;
        self.focus = Focus::Agent;
        self.agent.ensure_profile(lang);
        if first {
            self.agent.welcome(lang);
        }
        if !self.agent.session_loaded {
            let conferences = self.service.conferences().to_vec();
            self.agent.restore_session(&conferences, lang);
            self.recommendations_changed();
        }
    }

    pub fn close_agent(&mut self) {
        self.agent.open = false;
        self.focus = Focus::Table;
    }

    pub fn has_cards(&self) -> bool {
        self.agent
            .recommendations
            .as_ref()
            .is_some_and(|set| !set.items.is_empty())
    }

    /// Tab from the input: the cards when there are any, else the table.
    pub fn focus_next_from_input(&mut self) {
        if self.has_cards() {
            self.focus_cards();
        } else {
            self.focus = Focus::Table;
        }
    }

    pub fn focus_agent_input(&mut self) {
        self.agent.open = true;
        self.focus = Focus::Agent;
    }

    pub fn toggle_card_details(&mut self) {
        self.agent.card_details = !self.agent.card_details;
    }

    /// Enter in the agent input.
    pub fn submit_agent_input(&mut self) {
        let text = self.agent.input.take().trim().to_string();
        self.agent.input_edited();
        // Keys and questionnaire answers never enter the history.
        if self.agent.setup.is_none() {
            self.agent.record_history(&text);
            self.agent.save_session();
        }
        if self.agent.setup.is_some() {
            self.answer_setup(&text);
            return;
        }
        if text.is_empty() {
            return;
        }
        if let Some(command) = text.strip_prefix('/') {
            self.run_command(command);
        } else {
            self.start_agent_turn(&text);
        }
    }

    /// Esc in the agent input: interrupt, leave the questionnaire, clear the
    /// input, or close the panel, in that order.
    pub fn agent_escape(&mut self) {
        if let Some(handle) = &self.agent.handle {
            handle.cancel();
        } else if self.agent.setup.is_some() {
            self.cancel_setup();
        } else if self.agent.form.is_some() {
            self.cancel_profile_form();
        } else if !self.agent.input.is_empty() {
            self.agent.input.clear();
        } else {
            self.close_agent();
        }
    }

    pub fn start_agent_turn(&mut self, text: &str) {
        let lang = self.lang();
        if self.agent.is_running() {
            self.agent.info(lang.pick(
                "Still working on the previous request (Esc to stop).",
                "上一个请求还在处理中（按 Esc 中断）。",
            ));
            return;
        }
        self.agent.ensure_profile(lang);
        self.agent.entries.push(Entry::User(text.to_string()));
        self.agent.scroll = 0;
        // Offline, a question about one conference is answered from the
        // rules right away instead of producing a new set of cards.
        if self.agent.llm.is_none() {
            let answer = crate::agent::explain::answer(
                &crate::agent::recommend::Request {
                    conferences: self.service.conferences(),
                    favorites: self.service.favorites(),
                    profile: &self.agent.profile,
                    now: Utc::now(),
                    lang,
                },
                text,
            );
            if let Some(answer) = answer {
                self.agent.entries.push(Entry::Assistant(answer));
                if self.has_cards() {
                    self.agent.cards_view = CardsView::Collapsed;
                }
                return;
            }
        }
        // Offline, follow-ups refine the conditions of earlier turns.
        let constraints = self.agent.llm.is_none().then(|| {
            let current: Vec<String> = self
                .agent
                .recommendations
                .iter()
                .flat_map(|set| set.items.iter().map(|item| item.facts.title.clone()))
                .collect();
            self.agent.constraints =
                self.agent
                    .constraints
                    .refine(text, self.service.conferences(), &current);
            self.agent.constraints.clone()
        });
        let ctx = ToolContext::new(
            self.service.conferences().to_vec(),
            self.service.favorites().clone(),
            self.agent.profile.clone(),
            Utc::now(),
            lang,
        );
        let client = self
            .agent
            .llm
            .as_ref()
            .map(|config| (self.agent.client_factory)(config));
        self.agent.turn_cards = false;
        self.agent.handle = Some(worker::spawn(AgentJob {
            ctx,
            history: self.agent.history.clone(),
            user_text: text.to_string(),
            client,
            constraints,
        }));
    }

    /// Apply events from the worker. Returns `true` when something changed.
    pub fn poll_agent(&mut self) -> bool {
        let mut events = Vec::new();
        if let Some(handle) = &self.agent.handle {
            while let Ok(event) = handle.receiver.try_recv() {
                events.push(event);
            }
            if events.is_empty()
                && matches!(
                    handle.receiver.try_recv(),
                    Err(std::sync::mpsc::TryRecvError::Disconnected)
                )
            {
                events.push(AgentEvent::Failed("agent worker stopped".to_string()));
            }
        }
        let mut changed = !events.is_empty();
        for event in events {
            self.apply_agent_event(event);
        }
        changed |= self.poll_setup();
        changed
    }

    pub fn apply_agent_event(&mut self, event: AgentEvent) {
        let lang = self.lang();
        let panel = &mut self.agent;
        panel.scroll = 0;
        match event {
            AgentEvent::Delta(text) => match panel.entries.last_mut() {
                Some(Entry::Assistant(current)) => current.push_str(&text),
                _ => panel.entries.push(Entry::Assistant(text)),
            },
            AgentEvent::ToolStarted { name, .. } => {
                panel.entries.push(Entry::Tool {
                    name,
                    summary: None,
                });
            }
            AgentEvent::ToolFinished { name, summary } => {
                let open = panel
                    .entries
                    .iter_mut()
                    .rev()
                    .find_map(|entry| match entry {
                        Entry::Tool {
                            name: n,
                            summary: s @ None,
                        } if *n == name => Some(s),
                        _ => None,
                    });
                if let Some(slot) = open {
                    *slot = Some(summary);
                }
            }
            AgentEvent::Recommendations(set) => {
                let summary = if set.summary.is_empty() {
                    match lang {
                        Lang::Zh => format!("已生成 {} 条推荐。", set.items.len()),
                        Lang::En => format!("{} recommendations ready.", set.items.len()),
                    }
                } else {
                    set.summary.clone()
                };
                panel.entries.push(Entry::Assistant(summary));
                panel.push_recommendations(set);
                self.recommendations_changed();
            }
            AgentEvent::Finished { history } => {
                if !history.is_empty() {
                    panel.history = history;
                }
                panel.handle = None;
                // A text-only answer gets the room the cards were using.
                if !panel.turn_cards && panel.recommendations.is_some() {
                    panel.cards_view = CardsView::Collapsed;
                }
                panel.save_session();
            }
            AgentEvent::Failed(error) => {
                panel.entries.push(Entry::Error(match lang {
                    Lang::Zh => format!("出错了：{error}"),
                    Lang::En => format!("Error: {error}"),
                }));
                panel.handle = None;
            }
            AgentEvent::Cancelled => {
                panel.entries.push(Entry::Info(
                    lang.pick("Interrupted.", "已中断。").to_string(),
                ));
                panel.handle = None;
            }
        }
    }

    pub fn move_card(&mut self, delta: isize) {
        let Some(set) = &self.agent.recommendations else {
            return;
        };
        if set.items.is_empty() {
            return;
        }
        let last = set.items.len() as isize - 1;
        self.agent.card_cursor = (self.agent.card_cursor as isize + delta).clamp(0, last) as usize;
    }

    pub fn scroll_transcript(&mut self, delta: isize) {
        self.agent.scroll = (self.agent.scroll as isize + delta).max(0) as usize;
    }

    /// Show the selected card's conference in the table.
    pub fn jump_to_card(&mut self) {
        let lang = self.lang();
        let Some((title, year)) = self
            .agent
            .selected_card()
            .map(|card| (card.facts.title.clone(), card.facts.year))
        else {
            return;
        };
        let visible = |app: &App| {
            app.visible
                .iter()
                .any(|row| row.title == title && row.year == year)
        };
        if !visible(self) {
            // Filters hide it: start from a clean slate rather than guess.
            self.reset_filters();
        }
        if visible(self) {
            self.select_conference(&title, year);
            self.focus = Focus::Table;
            self.status = Some(match lang {
                Lang::Zh => format!("已定位到 {title} {year}"),
                Lang::En => format!("Showing {title} {year}"),
            });
        } else {
            self.status = Some(match lang {
                Lang::Zh => format!("{title} {year} 不在当前列表中"),
                Lang::En => format!("{title} {year} is not in the table"),
            });
        }
    }

    pub fn toggle_card_favorite(&mut self) {
        let Some(card) = self.agent.selected_card() else {
            return;
        };
        let (title, year) = (card.facts.title.clone(), card.facts.year);
        self.toggle_favorite_of(&title, year);
    }

    pub fn open_card_url(&mut self) -> bool {
        let Some(link) = self.agent.selected_card().map(|c| c.facts.link.clone()) else {
            return false;
        };
        if !browser::is_openable_url(&link) {
            self.status = Some(format!("Invalid URL: {link}"));
            return false;
        }
        match browser::open_url(&link) {
            Ok(()) => true,
            Err(error) => {
                self.status = Some(format!("Failed to open URL: {error}"));
                false
            }
        }
    }

    /// Favorite state of a card, read live so table toggles show up too.
    pub fn is_card_favorite(&self, card: &Recommendation) -> bool {
        self.service.is_favorite(&card.facts.title, card.facts.year)
    }

    // --- Slash commands -------------------------------------------------

    pub fn run_command(&mut self, command: &str) {
        let lang = self.lang();
        let (name, arg) = match command.trim().split_once(char::is_whitespace) {
            Some((name, arg)) => (name, arg.trim()),
            None => (command.trim(), ""),
        };
        self.agent.entries.push(Entry::User(format!("/{command}")));
        match name.to_lowercase().as_str() {
            "help" | "h" | "?" => self.agent.info(command_help(lang)),
            "init" => {
                self.agent.entries.pop();
                self.open_profile_form();
            }
            "setup" => self.start_setup(),
            "config" => self.config_command(),
            "recommend" | "rec" => {
                // Drop the echoed command; the turn echoes its own prompt.
                self.agent.entries.pop();
                let prompt = lang.pick(
                    "Recommend conferences for my profile.",
                    "根据我的研究画像推荐会议。",
                );
                let prompt = if arg.is_empty() {
                    prompt.to_string()
                } else {
                    format!("{prompt} {arg}")
                };
                self.start_agent_turn(&prompt);
            }
            "profile" if arg.eq_ignore_ascii_case("save") => {
                self.agent.entries.pop();
                self.save_conditions_as_profile();
            }
            "profile" => {
                self.agent.entries.pop();
                self.open_profile_form();
            }
            "model" => self.model_command(arg),
            "only" => {
                self.agent.entries.pop();
                self.toggle_only_recommended();
                if let Some(status) = self.status.clone() {
                    self.agent.info(status);
                }
            }
            "forget" => {
                if let Some(path) = &self.agent.session_path
                    && let Err(error) = session_store::forget(path)
                {
                    self.agent.error(error.to_string());
                    return;
                }
                self.agent.input_history.clear();
                self.agent.constraints = Constraints::default();
                self.agent.rounds.clear();
                self.agent.recommendations = None;
                self.agent.round_cursor = 0;
                self.agent.card_cursor = 0;
                self.only_recommended = false;
                self.refresh_visible();
                self.agent.info(lang.pick(
                    "Saved session deleted: input history, recommendations and conditions.",
                    "已删除保存的会话：输入历史、推荐结果和追问条件。",
                ));
            }
            "reset" => {
                self.agent.constraints = Constraints::default();
                self.agent.save_session();
                self.agent.info(lang.pick(
                    "Follow-up conditions cleared; the next request starts from the profile.",
                    "已清空追问条件，下次推荐从画像重新开始。",
                ));
            }
            "clear" => {
                self.agent.constraints = Constraints::default();
                self.agent.entries.clear();
                self.agent.history.clear();
                self.agent.recommendations = None;
                self.agent.rounds.clear();
                self.agent.round_cursor = 0;
                self.only_recommended = false;
                self.refresh_visible();
                self.agent.card_cursor = 0;
                self.agent.save_session();
                self.agent
                    .info(lang.pick("Conversation cleared.", "对话已清空。"));
            }
            "export" => self.export_command(arg),
            "lang" => {
                self.toggle_language();
                let lang = self.lang();
                self.agent
                    .info(lang.pick("Language: English", "语言：中文"));
            }
            other => self.agent.error(match lang {
                Lang::Zh => format!("未知命令 /{other}，输入 /help 查看"),
                Lang::En => format!("Unknown command /{other}; try /help"),
            }),
        }
    }

    fn export_command(&mut self, arg: &str) {
        let lang = self.lang();
        let Some(set) = &self.agent.recommendations else {
            self.agent.error(lang.pick(
                "Nothing to export yet; ask for recommendations first.",
                "还没有推荐结果，请先获取推荐。",
            ));
            return;
        };
        let target = if arg.is_empty() {
            self.agent.export_dir()
        } else {
            PathBuf::from(arg)
        };
        match export_markdown(set, &target, Utc::now(), lang) {
            Ok(path) => {
                let text = match lang {
                    Lang::Zh => format!("已导出到 {}", path.display()),
                    Lang::En => format!("Exported to {}", path.display()),
                };
                self.agent.info(text);
            }
            Err(error) => self.agent.error(format!("{error}")),
        }
    }

    /// Toggle a favorite by identity, used by both the table and the cards.
    pub fn toggle_favorite_of(&mut self, title: &str, year: i32) {
        let row = self
            .rows
            .iter()
            .find(|row| row.title == title && row.year == year)
            .cloned()
            .unwrap_or_else(|| ConferenceRow {
                title: title.to_string(),
                year,
                ..ConferenceRow::default()
            });
        let selected = self.selected_row().map(|r| (r.title.clone(), r.year));
        let outcome = self.service.toggle_favorite(&row);
        for candidate in &mut self.rows {
            if candidate.title == row.title && candidate.year == row.year {
                candidate.is_favorite = outcome.is_favorite;
            }
        }
        self.status = match outcome.save_error {
            Some(error) => Some(format!("Favorites not saved: {error}")),
            None if outcome.is_favorite => Some(format!("{} added to favorites", row.title)),
            None => Some(format!("{} removed from favorites", row.title)),
        };
        self.refresh_visible();
        if let Some((title, year)) = selected {
            self.select_conference(&title, year);
        }
    }
}

pub fn command_help(lang: Lang) -> String {
    let mut text = String::from(lang.pick("Commands:", "命令："));
    for command in SLASH_COMMANDS {
        text.push_str(&format!(
            "\n  {:<30} {}",
            command.usage(),
            command.description(lang)
        ));
    }
    text.push_str(lang.pick(
        "\nInput: Enter send · ↑↓ history · / commands (Tab completes) · ←→ Home End move · Ctrl+W delete word · Ctrl+U clear · PgUp/PgDn scroll · Tab cards/table · Ctrl+P or F3 profile · Ctrl+O fold the cards · Esc stop/close\nCards: j/k move · Enter show in table · f favorite · o open link · d details · ? ask about it · v table shows only these · z fold · y copy as Markdown · s save the conditions as the profile · p edit the profile · < > panel width · [ ] earlier rounds · i or Esc back to input · Tab table\nF2 or Ctrl+\\ panel size · A in the table asks about a conference",
        "\n输入区：Enter 发送 · ↑↓ 历史 · / 命令（Tab 补全）· ←→ Home End 移动光标 · Ctrl+W 删除一个词 · Ctrl+U 清空 · PgUp/PgDn 滚动 · Tab 卡片/表格 · Ctrl+P 或 F3 画像 · Ctrl+O 收放卡片 · Esc 中断/关闭\n卡片区：j/k 移动 · Enter 跳到表格 · f 收藏 · o 打开官网 · d 详情 · ? 追问 · v 表格只看推荐 · z 收放 · y 复制为 Markdown · s 把条件存为画像 · p 编辑画像 · < > 调整面板宽度 · [ ] 历史轮次 · i 或 Esc 回到输入 · Tab 表格\nF2 或 Ctrl+\\ 切换面板尺寸 · 在表格中按 A 让助手评估当前会议",
    ));
    text
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::agent::llm::Completion;
    use crate::agent::llm::tests::{ScriptedClient, call};
    use crate::agent::tools::SUBMIT;
    use crate::data::{DataService, Favorites};
    use serde_json::json;
    use std::time::{Duration, Instant};

    const YAML: &str = "
- title: CVPR
  description: IEEE/CVF Conference on Computer Vision and Pattern Recognition
  sub: AI
  rank: {ccf: A, core: 'A*'}
  confs:
  - year: 2099
    timezone: UTC
    link: https://cvpr.example
    timeline:
    - deadline: '2099-06-01 00:00:00'
- title: ICDE
  description: IEEE International Conference on Data Engineering
  sub: DB
  rank: {ccf: B}
  confs:
  - year: 2099
    timezone: UTC
    timeline:
    - deadline: '2099-07-01 00:00:00'
";

    pub(crate) fn app_with(llm: Option<LlmConfig>) -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut service = DataService::with_paths(
            None,
            dir.path().join("cache.yml"),
            Favorites::empty_at(dir.path().join("favorites.json")),
        );
        service
            .load_conferences_with(|| Ok(YAML.to_string()))
            .unwrap();
        let mut app = App::new(service);
        app.agent = AgentPanel::new(dir.path().join("profile.yaml"), llm);
        app.agent.env = Arc::new(|_| None);
        app.agent.models_fetcher = Arc::new(|_| Ok(vec!["m1".into(), "m2".into()]));
        // Horizon must cover the far-future fixture dates.
        app.agent.profile.horizon_days = 100_000;
        app.agent.profile_loaded = true;
        app.agent.profile_exists = true;
        (app, dir)
    }

    fn wait(app: &mut App) {
        let start = Instant::now();
        while app.agent.is_busy() {
            app.poll_agent();
            assert!(start.elapsed() < Duration::from_secs(10), "agent hung");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn type_and_enter(app: &mut App, text: &str) {
        app.agent.input.set(text);
        app.submit_agent_input();
    }

    #[test]
    fn opening_shows_a_welcome_and_focuses_input() {
        let (mut app, _dir) = app_with(None);
        app.open_agent();
        assert!(app.agent.open);
        assert_eq!(app.focus, Focus::Agent);
        assert!(
            app.agent
                .entries
                .iter()
                .any(|e| matches!(e, Entry::Info(t) if t.contains("Offline")))
        );
        // Esc on an empty input closes the panel.
        app.agent_escape();
        assert!(!app.agent.open);
        assert_eq!(app.focus, Focus::Table);
    }

    #[test]
    fn the_session_is_restored_after_a_restart() {
        let (mut app, dir) = app_with(None);
        app.open_agent();
        type_and_enter(&mut app, "computer vision and databases");
        wait(&mut app);
        type_and_enter(&mut app, "只要 A 类");
        wait(&mut app);
        assert!(dir.path().join("agent-session.json").exists());

        // A new app with the same home directory.
        let (mut next, _other) = app_with(None);
        next.agent = AgentPanel::new(dir.path().join("profile.yaml"), None);
        next.agent.profile.horizon_days = 100_000;
        next.agent.profile_loaded = true;
        assert!(next.agent.input_history.is_empty());
        next.open_agent();
        assert_eq!(
            next.agent.input_history,
            ["computer vision and databases", "只要 A 类"]
        );
        assert_eq!(next.agent.constraints.only_ranks, ["A"]);
        let set = next.agent.recommendations.clone().expect("restored cards");
        assert_eq!(set.items.len(), 1);
        assert_eq!(set.items[0].facts.title, "CVPR");
        assert!(
            next.agent
                .entries
                .iter()
                .any(|e| matches!(e, Entry::Info(t) if t.contains("Restored")))
        );

        // /forget deletes it.
        next.run_command("forget");
        assert!(!dir.path().join("agent-session.json").exists());
        assert!(next.agent.input_history.is_empty());
        assert!(next.agent.recommendations.is_none());
    }

    #[test]
    fn an_unopened_panel_never_overwrites_the_saved_session() {
        let (mut app, dir) = app_with(None);
        let path = dir.path().join("agent-session.json");
        std::fs::write(&path, "{\"input_history\": [\"keep me\"]}").unwrap();
        app.agent.save_session();
        assert!(std::fs::read_to_string(&path).unwrap().contains("keep me"));
        std::fs::write(&path, "{broken").unwrap();
        app.open_agent();
        assert!(
            app.agent
                .entries
                .iter()
                .any(|e| matches!(e, Entry::Error(t) if t.contains("unreadable")))
        );
    }

    #[test]
    fn offline_follow_ups_refine_the_previous_request() {
        let (mut app, _dir) = app_with(None);
        app.open_agent();
        type_and_enter(&mut app, "computer vision and databases");
        wait(&mut app);
        let first = app.agent.recommendations.clone().expect("cards");
        assert_eq!(first.items.len(), 2);
        assert!(first.conditions.len() <= 1);

        type_and_enter(&mut app, "只要 A 类");
        wait(&mut app);
        let set = app.agent.recommendations.clone().expect("cards");
        assert!(set.items.iter().all(|item| item.facts.ccf == "A"));
        assert_eq!(set.items.len(), 1);
        assert!(set.conditions.iter().any(|c| c.contains("CCF-A")));
        // The research interests from the first turn are still in force.
        assert!(app.agent.constraints.interests.is_some());

        type_and_enter(&mut app, "换一批");
        wait(&mut app);
        let set = app.agent.recommendations.clone().expect("cards");
        assert!(set.items.is_empty(), "{set:?}");

        app.run_command("reset");
        assert!(app.agent.constraints.is_empty());
        type_and_enter(&mut app, "computer vision and databases");
        wait(&mut app);
        assert_eq!(app.agent.recommendations.as_ref().unwrap().items.len(), 2);
    }

    #[test]
    fn offline_turn_produces_cards_that_link_to_the_table() {
        let (mut app, _dir) = app_with(None);
        app.open_agent();
        type_and_enter(&mut app, "computer vision");
        wait(&mut app);
        let set = app.agent.recommendations.clone().expect("cards");
        assert_eq!(set.items[0].facts.title, "CVPR");

        // Hide CVPR with a filter, then jump: filters reset and the row is
        // selected.
        app.toggle_category("AI");
        // An empty Enter in the input no longer jumps.
        app.focus = Focus::Agent;
        type_and_enter(&mut app, "");
        assert_eq!(app.focus, Focus::Agent);
        app.focus = Focus::AgentCards;
        app.jump_to_card();
        assert_eq!(app.focus, Focus::Table);
        assert_eq!(app.selected_row().unwrap().title, "CVPR");
        assert!(app.is_category_enabled("AI"));

        app.toggle_card_favorite();
        assert!(app.service.is_favorite("CVPR", 2099));
        assert!(app.is_card_favorite(&set.items[0]));
    }

    #[test]
    fn llm_turn_streams_text_and_tools() {
        let config = LlmConfig::new("custom", "http://localhost", "scripted", None);
        let (mut app, _dir) = app_with(Some(config));
        app.agent.client_factory = Box::new(|_| {
            Box::new(ScriptedClient::new(vec![
                Completion {
                    content: "Checking".into(),
                    tool_calls: vec![call("rank_candidates", json!({}))],
                },
                Completion {
                    content: String::new(),
                    tool_calls: vec![call(
                        SUBMIT,
                        json!({ "summary": "Try ICDE", "items": [{ "id": "ICDE", "tier": "main", "reason": "data", "direction": "systems" }] }),
                    )],
                },
            ]))
        });
        app.open_agent();
        type_and_enter(&mut app, "data engineering");
        wait(&mut app);
        assert!(
            app.agent
                .entries
                .contains(&Entry::Assistant("Checking".into()))
        );
        assert!(app.agent.entries.iter().any(
            |e| matches!(e, Entry::Tool { name, summary: Some(_) } if name == "rank_candidates")
        ));
        assert!(
            app.agent
                .entries
                .contains(&Entry::Assistant("Try ICDE".into()))
        );
        assert_eq!(app.agent.history.len(), 5);
        assert_eq!(app.agent.selected_card().unwrap().facts.title, "ICDE");
    }

    #[test]
    fn llm_errors_are_shown() {
        let config = LlmConfig::new("custom", "http://localhost", "scripted", None);
        let (mut app, _dir) = app_with(Some(config));
        app.agent.client_factory = Box::new(|_| Box::new(ScriptedClient::new(vec![])));
        app.open_agent();
        type_and_enter(&mut app, "hi");
        wait(&mut app);
        assert!(matches!(app.agent.entries.last(), Some(Entry::Error(_))));
    }

    #[test]
    fn commands_cover_help_profile_model_clear_and_export() {
        let (mut app, dir) = app_with(None);
        app.open_agent();
        type_and_enter(&mut app, "/help");
        assert!(matches!(app.agent.entries.last(), Some(Entry::Info(t)) if t.contains("/export")));
        type_and_enter(&mut app, "/profile");
        assert!(app.agent.form.is_some(), "/profile opens the form");
        app.agent_escape();
        assert!(app.agent.form.is_none());
        type_and_enter(&mut app, "/model");
        assert!(matches!(app.agent.entries.last(), Some(Entry::Info(t)) if t.contains("/setup")));
        type_and_enter(&mut app, "/export");
        assert!(matches!(app.agent.entries.last(), Some(Entry::Error(_))));
        type_and_enter(&mut app, "/nope");
        assert!(matches!(app.agent.entries.last(), Some(Entry::Error(t)) if t.contains("/nope")));

        type_and_enter(&mut app, "/recommend vision");
        wait(&mut app);
        assert!(app.agent.recommendations.is_some());
        type_and_enter(&mut app, "/export");
        let exported = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("recommendations-")
            });
        assert!(exported);

        type_and_enter(&mut app, "/lang");
        assert_eq!(app.language, Language::Zh);
        type_and_enter(&mut app, "/clear");
        assert!(app.agent.recommendations.is_none());
        assert_eq!(app.agent.entries.len(), 1);
    }

    #[test]
    fn model_can_be_switched_for_the_session() {
        let config = LlmConfig::new("custom", "http://localhost", "a", None);
        let (mut app, _dir) = app_with(Some(config));
        app.open_agent();
        type_and_enter(&mut app, "/model b");
        assert_eq!(app.agent.model_label(), "custom · b");
    }

    #[test]
    fn card_cursor_is_clamped() {
        let (mut app, _dir) = app_with(None);
        app.open_agent();
        type_and_enter(&mut app, "vision database");
        wait(&mut app);
        let count = app.agent.recommendations.as_ref().unwrap().items.len();
        app.move_card(-5);
        assert_eq!(app.agent.card_cursor, 0);
        app.move_card(50);
        assert_eq!(app.agent.card_cursor, count - 1);
    }
}
