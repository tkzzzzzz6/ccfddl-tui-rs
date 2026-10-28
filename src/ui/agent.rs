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
use crate::agent::profile::{Profile, normalize_ranks, parse_date, split_list};
use crate::agent::providers;
use crate::agent::recommend::RecommendationSet;
use crate::agent::report::export_markdown;
use crate::agent::session::AgentEvent;
use crate::agent::tools::ToolContext;
use crate::agent::worker::{self, AgentHandle, AgentJob};
use crate::agent::{Lang, Recommendation};
use crate::data::ConferenceRow;

use super::agent_input::SLASH_COMMANDS;
use super::agent_results::PanelSize;
use super::agent_setup::{SetupEvent, SetupWizard};
use super::app::{App, Focus, Language};
use super::browser;
use super::line_input::LineInput;

impl From<Language> for Lang {
    fn from(language: Language) -> Self {
        match language {
            Language::En => Lang::En,
            Language::Zh => Lang::Zh,
        }
    }
}

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

/// Steps of the `/init` questionnaire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardStep {
    Interests,
    TargetRanks,
    ReadyDate,
    PreferredPlaces,
    AvoidedPlaces,
}

impl WizardStep {
    pub const ALL: [WizardStep; 5] = [
        Self::Interests,
        Self::TargetRanks,
        Self::ReadyDate,
        Self::PreferredPlaces,
        Self::AvoidedPlaces,
    ];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    fn next(self) -> Option<Self> {
        Self::ALL.get(self.index() + 1).copied()
    }

    fn question(self, lang: Lang, profile: &Profile) -> String {
        let current = |items: &[String]| {
            if items.is_empty() {
                lang.pick("none", "无").to_string()
            } else {
                items.join(", ")
            }
        };
        let (en, zh, value) = match self {
            Self::Interests => (
                "Research interests, comma separated (e.g. multimodal LLM, program analysis)",
                "研究方向，用逗号分隔（如：多模态大模型, 程序分析）",
                current(&profile.interests),
            ),
            Self::TargetRanks => (
                "Target CCF ranks (e.g. A,B)",
                "目标 CCF 等级（如：A,B）",
                current(&profile.target_ranks),
            ),
            Self::ReadyDate => (
                "When will the paper be ready? YYYY-MM-DD",
                "论文预计何时完成？格式 YYYY-MM-DD",
                profile
                    .ready_date
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| lang.pick("none", "无").to_string()),
            ),
            Self::PreferredPlaces => (
                "Preferred places (e.g. Asia, China)",
                "偏好的举办地（如：China, Singapore）",
                current(&profile.preferred_places),
            ),
            Self::AvoidedPlaces => (
                "Places to avoid",
                "希望避开的举办地",
                current(&profile.avoided_places),
            ),
        };
        format!(
            "[{}/{}] {}  ({}: {}; {})",
            self.index() + 1,
            Self::ALL.len(),
            lang.pick(en, zh),
            lang.pick("current", "当前"),
            value,
            lang.pick("Enter keeps it, - clears", "回车保留，输入 - 清空"),
        )
    }
}

#[derive(Debug, Clone)]
pub struct Wizard {
    pub step: WizardStep,
    pub draft: Profile,
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
    /// Transcript lines scrolled up from the bottom.
    pub scroll: usize,
    pub profile: Profile,
    pub profile_path: PathBuf,
    profile_loaded: bool,
    pub llm: Option<LlmConfig>,
    pub client_factory: ClientFactory,
    pub wizard: Option<Wizard>,
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
            scroll: 0,
            profile: Profile::default(),
            profile_path,
            profile_loaded: false,
            llm,
            client_factory: Box::new(|config| Box::new(OpenAiClient::new(config.clone()))),
            wizard: None,
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
            Ok(Some(profile)) => self.profile = profile,
            Ok(None) => {}
            Err(error) => self.error(match lang {
                Lang::Zh => format!("读取画像失败：{error}"),
                Lang::En => format!("Could not read profile: {error}"),
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
        self.focus = if self.has_cards() {
            Focus::AgentCards
        } else {
            Focus::Table
        };
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
        if self.agent.setup.is_none() && self.agent.wizard.is_none() {
            self.agent.record_history(&text);
        }
        if self.agent.setup.is_some() {
            self.answer_setup(&text);
            return;
        }
        if self.agent.wizard.is_some() {
            self.answer_wizard(&text);
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
        let lang = self.lang();
        if let Some(handle) = &self.agent.handle {
            handle.cancel();
        } else if self.agent.setup.is_some() {
            self.cancel_setup();
        } else if self.agent.wizard.take().is_some() {
            self.agent
                .info(lang.pick("Profile setup cancelled.", "已取消画像设置。"));
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
                return;
            }
        }
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
        self.agent.handle = Some(worker::spawn(AgentJob {
            ctx,
            history: self.agent.history.clone(),
            user_text: text.to_string(),
            client,
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
            }
            AgentEvent::Finished { history } => {
                if !history.is_empty() {
                    panel.history = history;
                }
                panel.handle = None;
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
            "init" => self.start_wizard(),
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
            "profile" => {
                self.agent.ensure_profile(lang);
                let summary = self.agent.profile.summary(lang.is_zh());
                let path = self.agent.profile_path.display().to_string();
                self.agent.info(format!("{summary}\n{path}"));
            }
            "model" => self.model_command(arg),
            "clear" => {
                self.agent.entries.clear();
                self.agent.history.clear();
                self.agent.recommendations = None;
                self.agent.rounds.clear();
                self.agent.round_cursor = 0;
                self.agent.card_cursor = 0;
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

    // --- /init questionnaire --------------------------------------------

    pub fn start_wizard(&mut self) {
        let lang = self.lang();
        self.agent.ensure_profile(lang);
        let draft = self.agent.profile.clone();
        let question = WizardStep::Interests.question(lang, &draft);
        self.agent.wizard = Some(Wizard {
            step: WizardStep::Interests,
            draft,
        });
        self.agent.info(lang.pick(
            "Let's set up your research profile (Esc to cancel).",
            "开始设置研究画像（按 Esc 取消）。",
        ));
        self.agent.info(question);
    }

    fn answer_wizard(&mut self, answer: &str) {
        let lang = self.lang();
        let Some(mut wizard) = self.agent.wizard.take() else {
            return;
        };
        if !answer.is_empty() {
            self.agent.entries.push(Entry::User(answer.to_string()));
        }
        let clear = answer == "-";
        let draft = &mut wizard.draft;
        let mut invalid = None;
        match wizard.step {
            WizardStep::Interests if clear => draft.interests.clear(),
            WizardStep::Interests if !answer.is_empty() => draft.interests = split_list(answer),
            WizardStep::TargetRanks if !answer.is_empty() && !clear => {
                let ranks = normalize_ranks(&split_list(&answer.replace(' ', ",")));
                if ranks.is_empty() {
                    invalid = Some(lang.pick("Use A, B, C or N.", "请输入 A、B、C 或 N。"));
                } else {
                    draft.target_ranks = ranks;
                }
            }
            WizardStep::ReadyDate if clear => draft.ready_date = None,
            WizardStep::ReadyDate if !answer.is_empty() => match parse_date(answer) {
                Some(date) => draft.ready_date = Some(date),
                None => invalid = Some(lang.pick("Use YYYY-MM-DD.", "日期格式应为 YYYY-MM-DD。")),
            },
            WizardStep::PreferredPlaces if clear => draft.preferred_places.clear(),
            WizardStep::PreferredPlaces if !answer.is_empty() => {
                draft.preferred_places = split_list(answer)
            }
            WizardStep::AvoidedPlaces if clear => draft.avoided_places.clear(),
            WizardStep::AvoidedPlaces if !answer.is_empty() => {
                draft.avoided_places = split_list(answer)
            }
            _ => {}
        }
        if let Some(message) = invalid {
            self.agent.error(message);
            let question = wizard.step.question(lang, &wizard.draft);
            self.agent.info(question);
            self.agent.wizard = Some(wizard);
            return;
        }

        match wizard.step.next() {
            Some(step) => {
                wizard.step = step;
                let question = step.question(lang, &wizard.draft);
                self.agent.info(question);
                self.agent.wizard = Some(wizard);
            }
            None => {
                let profile = wizard.draft;
                match profile.save(&self.agent.profile_path) {
                    Ok(()) => {
                        let text = match lang {
                            Lang::Zh => format!(
                                "画像已保存到 {}：\n{}\n输入 /recommend 获取推荐。",
                                self.agent.profile_path.display(),
                                profile.summary(true)
                            ),
                            Lang::En => format!(
                                "Profile saved to {}:\n{}\nType /recommend for suggestions.",
                                self.agent.profile_path.display(),
                                profile.summary(false)
                            ),
                        };
                        self.agent.info(text);
                    }
                    Err(error) => self.agent.error(format!("{error}")),
                }
                self.agent.profile = profile;
            }
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
        "\nInput: Enter send · ↑↓ history · / commands (Tab completes) · ←→ Home End move · Ctrl+W delete word · Ctrl+U clear · PgUp/PgDn scroll · Tab cards/table · Esc stop/close\nCards: j/k move · Enter show in table · f favorite · o open link · d details · ? ask about it · [ ] earlier rounds · i or Esc back to input · Tab table\nF2 or Ctrl+\\ panel size · A in the table asks about a conference",
        "\n输入区：Enter 发送 · ↑↓ 历史 · / 命令（Tab 补全）· ←→ Home End 移动光标 · Ctrl+W 删除一个词 · Ctrl+U 清空 · PgUp/PgDn 滚动 · Tab 卡片/表格 · Esc 中断/关闭\n卡片区：j/k 移动 · Enter 跳到表格 · f 收藏 · o 打开官网 · d 详情 · ? 追问 · [ ] 历史轮次 · i 或 Esc 回到输入 · Tab 表格\nF2 或 Ctrl+\\ 切换面板尺寸 · 在表格中按 A 让助手评估当前会议",
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
    fn wizard_builds_and_saves_a_profile() {
        let (mut app, dir) = app_with(None);
        app.open_agent();
        type_and_enter(&mut app, "/init");
        assert!(app.agent.wizard.is_some());
        type_and_enter(&mut app, "多模态, program analysis");
        type_and_enter(&mut app, "x"); // invalid rank: asked again
        assert_eq!(
            app.agent.wizard.as_ref().unwrap().step,
            WizardStep::TargetRanks
        );
        type_and_enter(&mut app, "a b");
        type_and_enter(&mut app, "2099-01-31");
        type_and_enter(&mut app, ""); // keep places
        type_and_enter(&mut app, "-");
        assert!(app.agent.wizard.is_none());

        let saved = Profile::load(&dir.path().join("profile.yaml"))
            .unwrap()
            .unwrap();
        assert_eq!(saved.interests, ["多模态", "program analysis"]);
        assert_eq!(saved.target_ranks, ["A", "B"]);
        assert_eq!(saved.ready_date, parse_date("2099-01-31"));
        assert_eq!(app.agent.profile, saved);
    }

    #[test]
    fn escape_cancels_the_wizard_first() {
        let (mut app, _dir) = app_with(None);
        app.open_agent();
        type_and_enter(&mut app, "/init");
        app.agent_escape();
        assert!(app.agent.wizard.is_none());
        assert!(app.agent.open);
    }

    #[test]
    fn commands_cover_help_profile_model_clear_and_export() {
        let (mut app, dir) = app_with(None);
        app.open_agent();
        type_and_enter(&mut app, "/help");
        assert!(matches!(app.agent.entries.last(), Some(Entry::Info(t)) if t.contains("/export")));
        type_and_enter(&mut app, "/profile");
        assert!(
            matches!(app.agent.entries.last(), Some(Entry::Info(t)) if t.contains("profile.yaml"))
        );
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
