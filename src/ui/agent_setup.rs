//! Connecting the agent to a model: the `/setup` questionnaire, `/model`
//! and `/config`. Network checks (model list, connection test) run on a
//! worker thread and report back through [`SetupEvent`].

use std::sync::mpsc;
use std::thread;

use crate::agent::Lang;
use crate::agent::config::{
    LlmSettings, Overrides, ProfileEntry, load_and_resolve, report, resolve,
};
use crate::agent::llm::{LlmConfig, LlmError, ping};
use crate::agent::providers::{self, PRESETS, Preset};

use super::agent::Entry;
use super::app::App;

/// Shown at most this many models in a numbered list.
const MAX_LISTED_MODELS: usize = 40;

pub enum SetupEvent {
    Models {
        for_setup: bool,
        result: Result<Vec<String>, LlmError>,
    },
    Tested(Result<(), LlmError>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupStep {
    Provider,
    BaseUrl,
    Key,
    KeyStorage,
    FetchingModels,
    Model,
    Testing,
    SaveAnyway,
    Name,
}

impl SetupStep {
    /// Position in the five visible stages.
    fn stage(self) -> usize {
        match self {
            Self::Provider => 1,
            Self::BaseUrl | Self::Key | Self::KeyStorage => 2,
            Self::FetchingModels | Self::Model => 3,
            Self::Testing | Self::SaveAnyway => 4,
            Self::Name => 5,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SetupWizard {
    pub step: SetupStep,
    pub preset: &'static Preset,
    /// Suggested when the provider answer is empty.
    pub default_preset: &'static Preset,
    pub base_url: String,
    /// A pasted key (never shown again, never echoed).
    key: Option<String>,
    /// Name of the variable holding the key.
    pub key_env: Option<String>,
    /// Write the pasted key into `config.yaml`.
    pub save_key: bool,
    pub models: Vec<String>,
    pub model: Option<String>,
}

impl SetupWizard {
    pub fn masks_input(&self) -> bool {
        self.step == SetupStep::Key
    }

    pub fn title(&self) -> String {
        format!(" /setup {}/5 · {} ", self.step.stage(), self.preset.id)
    }

    fn config(&self, env: &dyn Fn(&str) -> Option<String>) -> LlmConfig {
        let key = self
            .key
            .clone()
            .or_else(|| self.key_env.as_deref().and_then(env));
        LlmConfig::new(
            self.preset.id,
            self.base_url.clone(),
            self.model.clone().unwrap_or_default(),
            key,
        )
    }

    fn default_model(&self) -> Option<String> {
        let preset_default = self.preset.default_model.map(str::to_string);
        if self.models.is_empty() {
            return preset_default;
        }
        preset_default
            .filter(|m| self.models.contains(m))
            .or_else(|| self.models.first().cloned())
    }
}

fn numbered(models: &[String], lang: Lang) -> String {
    let mut text = String::new();
    for (index, model) in models.iter().take(MAX_LISTED_MODELS).enumerate() {
        text.push_str(&format!("{:>4}. {model}\n", index + 1));
    }
    if models.len() > MAX_LISTED_MODELS {
        text.push_str(&match lang {
            Lang::Zh => format!("  …共 {} 个，未列出的可直接输入名称\n", models.len()),
            Lang::En => format!(
                "  …{} in total; type any other name directly\n",
                models.len()
            ),
        });
    }
    text.pop();
    text
}

impl App {
    fn setup_lang(&self) -> Lang {
        self.language.into()
    }

    fn env_value(&self, key: &str) -> Option<String> {
        (self.agent.env)(key).filter(|v| !v.trim().is_empty())
    }

    // --- /setup ----------------------------------------------------------

    pub fn start_setup(&mut self) {
        let lang = self.setup_lang();
        if self.agent.handle.is_some() {
            self.agent.info(lang.pick(
                "Wait for the current request to finish (or press Esc).",
                "请等待当前请求结束（或按 Esc 中断）。",
            ));
            return;
        }
        let detected = providers::detect_env_keys(self.agent.env.as_ref());
        let default_preset = detected
            .first()
            .map(|(preset, _)| *preset)
            .or_else(|| providers::preset(providers::DEFAULT_PROVIDER))
            .unwrap_or(&PRESETS[0]);

        let mut list = String::from(lang.pick(
            "Connect a model (Esc cancels). Providers:",
            "配置模型（按 Esc 取消）。可选厂商：",
        ));
        for (index, preset) in PRESETS.iter().enumerate() {
            let name = lang.pick(preset.name, preset.name_zh);
            list.push_str(&format!("\n{:>4}. {name}", index + 1));
            if let Some((_, var)) = detected.iter().find(|(p, _)| p.id == preset.id) {
                list.push_str(&match lang {
                    Lang::Zh => format!("  ✓ 已检测到 {var}"),
                    Lang::En => format!("  ✓ {var} found"),
                });
            }
        }
        self.agent.info(list);
        self.agent.setup = Some(SetupWizard {
            step: SetupStep::Provider,
            preset: default_preset,
            default_preset,
            base_url: default_preset.base_url.to_string(),
            key: None,
            key_env: None,
            save_key: false,
            models: Vec::new(),
            model: None,
        });
        self.ask_provider();
    }

    fn ask_provider(&mut self) {
        let lang = self.setup_lang();
        let Some(wizard) = &self.agent.setup else {
            return;
        };
        let name = lang.pick(wizard.default_preset.name, wizard.default_preset.name_zh);
        let text = match lang {
            Lang::Zh => format!("[1/5] 选择厂商：输入编号或名称（回车默认 {name}）"),
            Lang::En => format!("[1/5] Provider: number or name (Enter for {name})"),
        };
        self.agent.info(text);
    }

    pub fn cancel_setup(&mut self) {
        let lang = self.setup_lang();
        self.agent.setup = None;
        self.agent.setup_rx = None;
        self.agent.info(lang.pick(
            "Model setup cancelled; nothing was saved.",
            "已取消模型配置，未保存任何内容。",
        ));
    }

    pub fn answer_setup(&mut self, answer: &str) {
        let lang = self.setup_lang();
        let Some(step) = self.agent.setup.as_ref().map(|w| w.step) else {
            return;
        };
        // Never echo a pasted key into the transcript.
        if !answer.is_empty() {
            let shown = if step == SetupStep::Key && !answer.starts_with("env:") {
                "••••••".to_string()
            } else {
                answer.to_string()
            };
            self.agent.entries.push(Entry::User(shown));
        }
        match step {
            SetupStep::Provider => {
                let picked = if answer.is_empty() {
                    self.agent.setup.as_ref().map(|w| w.default_preset)
                } else {
                    providers::pick(answer)
                };
                let Some(preset) = picked else {
                    self.agent.error(lang.pick(
                        "Unknown provider; use a number from the list.",
                        "未知厂商，请输入列表中的编号。",
                    ));
                    self.ask_provider();
                    return;
                };
                if let Some(wizard) = &mut self.agent.setup {
                    wizard.preset = preset;
                    wizard.base_url = preset.base_url.to_string();
                }
                if preset.id == "custom" {
                    self.set_step(SetupStep::BaseUrl);
                    self.agent.info(lang.pick(
                        "[2/5] Base URL of the OpenAI-compatible API, e.g. https://example.com/v1",
                        "[2/5] 输入 OpenAI 兼容接口的 Base URL，例如 https://example.com/v1",
                    ));
                } else {
                    self.ask_key();
                }
            }
            SetupStep::BaseUrl => {
                let url = answer.trim().trim_end_matches('/');
                if !(url.starts_with("http://") || url.starts_with("https://")) {
                    self.agent.error(lang.pick(
                        "The URL must start with http:// or https://",
                        "URL 需要以 http:// 或 https:// 开头",
                    ));
                    return;
                }
                if let Some(wizard) = &mut self.agent.setup {
                    wizard.base_url = url.to_string();
                }
                self.ask_key();
            }
            SetupStep::Key => self.answer_key(answer),
            SetupStep::KeyStorage => {
                if let Some(wizard) = &mut self.agent.setup {
                    wizard.save_key = matches!(answer.trim(), "2" | "y" | "yes" | "是");
                }
                self.fetch_models(true);
            }
            SetupStep::FetchingModels | SetupStep::Testing => self.agent.info(lang.pick(
                "Still checking, please wait (Esc cancels).",
                "正在检查，请稍候（Esc 取消）。",
            )),
            SetupStep::Model => {
                let Some(wizard) = &mut self.agent.setup else {
                    return;
                };
                let choice = if answer.is_empty() {
                    wizard.default_model()
                } else if let Ok(n) = answer.parse::<usize>() {
                    n.checked_sub(1).and_then(|i| wizard.models.get(i)).cloned()
                } else {
                    Some(answer.to_string())
                };
                match choice {
                    Some(model) => {
                        wizard.model = Some(model);
                        self.start_test();
                    }
                    None => self.agent.error(lang.pick(
                        "Choose a model by number or type its name.",
                        "请输入模型编号或模型名称。",
                    )),
                }
            }
            SetupStep::SaveAnyway => {
                if matches!(answer.trim().to_lowercase().as_str(), "y" | "yes" | "是") {
                    self.ask_name();
                } else {
                    self.cancel_setup();
                }
            }
            SetupStep::Name => {
                let name = if answer.trim().is_empty() {
                    self.agent
                        .setup
                        .as_ref()
                        .map(|w| w.preset.id.to_string())
                        .unwrap_or_default()
                } else {
                    answer.trim().to_string()
                };
                self.finish_setup(&name);
            }
        }
    }

    fn set_step(&mut self, step: SetupStep) {
        if let Some(wizard) = &mut self.agent.setup {
            wizard.step = step;
        }
    }

    fn ask_key(&mut self) {
        let lang = self.setup_lang();
        let Some(wizard) = &self.agent.setup else {
            return;
        };
        let preset = wizard.preset;
        if !preset.needs_key && preset.id != "custom" {
            self.fetch_models(true);
            return;
        }
        let detected = preset.key_env.filter(|var| self.env_value(var).is_some());
        let text = match (detected, preset.id == "custom", lang) {
            (Some(var), _, Lang::Zh) => format!(
                "[2/5] 粘贴 API Key（输入会被隐藏）；回车直接使用已检测到的 {var}；也可输入 env:变量名"
            ),
            (Some(var), _, Lang::En) => format!(
                "[2/5] Paste the API key (hidden); Enter uses the detected {var}; or type env:VAR_NAME"
            ),
            (None, true, Lang::Zh) => {
                "[2/5] 粘贴 API Key（输入会被隐藏）；不需要 Key 时直接回车；也可输入 env:变量名"
                    .to_string()
            }
            (None, true, Lang::En) => {
                "[2/5] Paste the API key (hidden); Enter if none is needed; or type env:VAR_NAME"
                    .to_string()
            }
            (None, false, Lang::Zh) => format!(
                "[2/5] 粘贴 {} 的 API Key（输入会被隐藏），或输入 env:变量名 从环境变量读取",
                preset.name_zh
            ),
            (None, false, Lang::En) => format!(
                "[2/5] Paste your {} API key (hidden), or type env:VAR_NAME to read it from a variable",
                preset.name
            ),
        };
        self.set_step(SetupStep::Key);
        self.agent.info(text);
    }

    fn answer_key(&mut self, answer: &str) {
        let lang = self.setup_lang();
        let Some(preset) = self.agent.setup.as_ref().map(|w| w.preset) else {
            return;
        };
        let answer = answer.trim();
        if let Some(var) = answer.strip_prefix("env:") {
            let var = var.trim();
            if self.env_value(var).is_none() {
                self.agent.error(match lang {
                    Lang::Zh => format!("环境变量 {var} 未设置，请重新输入。"),
                    Lang::En => format!("{var} is not set; try again."),
                });
                return;
            }
            if let Some(wizard) = &mut self.agent.setup {
                wizard.key_env = Some(var.to_string());
                wizard.key = None;
            }
            self.fetch_models(true);
        } else if answer.is_empty() {
            let detected = preset.key_env.filter(|var| self.env_value(var).is_some());
            match detected {
                Some(var) => {
                    if let Some(wizard) = &mut self.agent.setup {
                        wizard.key_env = Some(var.to_string());
                    }
                    self.fetch_models(true);
                }
                None if preset.id == "custom" => self.fetch_models(true),
                None => self
                    .agent
                    .error(lang.pick("This provider needs an API key.", "该厂商需要 API Key。")),
            }
        } else {
            if let Some(wizard) = &mut self.agent.setup {
                wizard.key = Some(answer.to_string());
                wizard.key_env = None;
                wizard.step = SetupStep::KeyStorage;
            }
            self.agent.info(lang.pick(
                "Where should the key live?\n  1. This session only (default; set an environment variable for next time)\n  2. Save it in config.yaml (plain text, owner-only permissions)",
                "Key 如何保存？\n  1. 仅本次会话使用（默认；下次请设置环境变量）\n  2. 写入 config.yaml（明文保存，仅本人可读）",
            ));
        }
    }

    /// Start `GET /models` in the background.
    fn fetch_models(&mut self, for_setup: bool) {
        let lang = self.setup_lang();
        let config = if for_setup {
            let Some(wizard) = &mut self.agent.setup else {
                return;
            };
            wizard.step = SetupStep::FetchingModels;
            wizard.config(self.agent.env.as_ref())
        } else {
            match &self.agent.llm {
                Some(config) => config.clone(),
                None => {
                    self.agent.error(lang.pick(
                        "No model configured yet; run /setup first.",
                        "尚未配置模型，请先运行 /setup。",
                    ));
                    return;
                }
            }
        };
        let fetcher = self.agent.models_fetcher.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(SetupEvent::Models {
                for_setup,
                result: fetcher(&config),
            });
        });
        self.agent.setup_rx = Some(receiver);
        self.agent
            .info(lang.pick("Fetching the model list…", "正在获取模型列表…"));
    }

    fn start_test(&mut self) {
        let lang = self.setup_lang();
        let Some(wizard) = &mut self.agent.setup else {
            return;
        };
        wizard.step = SetupStep::Testing;
        let config = wizard.config(self.agent.env.as_ref());
        let mut client = (self.agent.client_factory)(&config);
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(SetupEvent::Tested(ping(client.as_mut())));
        });
        self.agent.setup_rx = Some(receiver);
        self.agent.info(match lang {
            Lang::Zh => format!("正在测试 {} …", config.label()),
            Lang::En => format!("Testing {} …", config.label()),
        });
    }

    fn ask_name(&mut self) {
        let lang = self.setup_lang();
        let Some(id) = self.agent.setup.as_ref().map(|w| w.preset.id) else {
            return;
        };
        self.set_step(SetupStep::Name);
        self.agent.info(match lang {
            Lang::Zh => format!("[5/5] 给这个配置起个名字（回车默认 {id}）"),
            Lang::En => format!("[5/5] Name for this configuration (Enter for {id})"),
        });
    }

    /// Apply a finished background check. Returns `true` if one arrived.
    pub fn poll_setup(&mut self) -> bool {
        let Some(receiver) = &self.agent.setup_rx else {
            return false;
        };
        let event = match receiver.try_recv() {
            Ok(event) => event,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.agent.setup_rx = None;
                return true;
            }
        };
        self.agent.setup_rx = None;
        let lang = self.setup_lang();
        match event {
            SetupEvent::Models {
                for_setup: true,
                result,
            } => self.setup_models_arrived(result),
            SetupEvent::Models {
                for_setup: false,
                result: Ok(models),
            } => {
                let text = if models.is_empty() {
                    lang.pick("The provider returned no models.", "厂商没有返回任何模型。")
                        .to_string()
                } else {
                    format!(
                        "{}\n{}",
                        numbered(&models, lang),
                        lang.pick(
                            "/model set <number|name> switches and saves.",
                            "用 /model set <编号|名称> 切换并保存。"
                        )
                    )
                };
                self.agent.last_models = models;
                self.agent.info(text);
            }
            SetupEvent::Models {
                for_setup: false,
                result: Err(error),
            } => self.agent.error(error.explain(lang)),
            SetupEvent::Tested(Ok(())) => {
                self.agent.info(lang.pick(
                    "✓ Connected; the model accepts tool calls.",
                    "✓ 连接成功，模型支持工具调用。",
                ));
                self.ask_name();
            }
            SetupEvent::Tested(Err(error)) => {
                self.agent.error(error.explain(lang));
                self.set_step(SetupStep::SaveAnyway);
                self.agent.info(lang.pick(
                    "[4/5] Save this configuration anyway? (y/N)",
                    "[4/5] 仍然保存这个配置吗？(y/N)",
                ));
            }
        }
        true
    }

    fn setup_models_arrived(&mut self, result: Result<Vec<String>, LlmError>) {
        let lang = self.setup_lang();
        match result {
            Err(
                error @ LlmError::Status {
                    code: 401 | 403, ..
                },
            ) => {
                // The key is wrong: ask for it again instead of a model.
                self.agent.error(error.explain(lang));
                self.ask_key();
                return;
            }
            Err(error) => {
                self.agent.error(error.explain(lang));
                self.agent.info(lang.pick(
                    "Could not list models; type the model name instead.",
                    "无法获取模型列表，请直接输入模型名称。",
                ));
            }
            Ok(models) => {
                if let Some(wizard) = &mut self.agent.setup {
                    wizard.models = models;
                }
            }
        }
        let Some(wizard) = &mut self.agent.setup else {
            return;
        };
        wizard.step = SetupStep::Model;
        let default = wizard.default_model();
        let list = numbered(&wizard.models, lang);
        let prompt = match (default, lang) {
            (Some(model), Lang::Zh) => {
                format!("[3/5] 选择模型：输入编号或名称（回车默认 {model}）")
            }
            (Some(model), Lang::En) => {
                format!("[3/5] Model: number or name (Enter for {model})")
            }
            (None, Lang::Zh) => "[3/5] 输入模型名称".to_string(),
            (None, Lang::En) => "[3/5] Type the model name".to_string(),
        };
        if list.is_empty() {
            self.agent.info(prompt);
        } else {
            self.agent.info(format!("{list}\n{prompt}"));
        }
    }

    fn finish_setup(&mut self, name: &str) {
        let lang = self.setup_lang();
        let Some(wizard) = self.agent.setup.take() else {
            return;
        };
        let mut settings = match LlmSettings::load(&self.agent.config_path) {
            Ok(settings) => settings.unwrap_or_default(),
            Err(error) => {
                // Never overwrite a file we could not read.
                self.agent.error(format!("{error}"));
                return;
            }
        };
        let preset = wizard.preset;
        let entry = ProfileEntry {
            provider: Some(preset.id.to_string()),
            model: wizard.model.clone(),
            base_url: (wizard.base_url != preset.base_url).then(|| wizard.base_url.clone()),
            api_key: if wizard.save_key {
                wizard.key.clone()
            } else {
                None
            },
            api_key_env: wizard.key_env.clone(),
            ..ProfileEntry::default()
        };
        settings.set_profile(name, entry, true);
        if let Err(error) = settings.save(&self.agent.config_path) {
            self.agent.error(format!("{error}"));
            return;
        }
        let config = wizard.config(self.agent.env.as_ref());
        let mut text = match lang {
            Lang::Zh => format!(
                "✓ 已保存配置「{name}」到 {}，当前模型：{}",
                self.agent.config_path.display(),
                config.label()
            ),
            Lang::En => format!(
                "✓ Saved \"{name}\" to {}; now using {}",
                self.agent.config_path.display(),
                config.label()
            ),
        };
        if wizard.key.is_some() && !wizard.save_key {
            let var = preset.key_env.unwrap_or("CCFDDL_API_KEY");
            text.push_str(&match lang {
                Lang::Zh => format!(
                    "\nKey 只在本次会话有效。下次启动前请设置环境变量 {var}，或重新 /setup 选择写入文件。"
                ),
                Lang::En => format!(
                    "\nThe key is kept for this session only. Set {var} before the next start, or run /setup again and choose to save it."
                ),
            });
        }
        self.agent.llm = Some(config);
        self.agent.info(text);
    }

    // --- /model and /config ---------------------------------------------

    pub fn model_command(&mut self, arg: &str) {
        let lang = self.setup_lang();
        let (sub, rest) = match arg.split_once(char::is_whitespace) {
            Some((sub, rest)) => (sub, rest.trim()),
            None => (arg, ""),
        };
        match sub {
            "" => self.show_models(),
            "list" | "ls" => self.fetch_models(false),
            "set" if !rest.is_empty() => self.set_model(rest),
            "set" => self.agent.error(lang.pick(
                "Usage: /model set <number|name>",
                "用法：/model set <编号|名称>",
            )),
            name => self.switch_or_override(name),
        }
    }

    fn show_models(&mut self) {
        let lang = self.setup_lang();
        let mut text = match &self.agent.llm {
            Some(config) => match lang {
                Lang::Zh => format!("当前：{} @ {}", config.label(), config.base_url),
                Lang::En => format!("Current: {} @ {}", config.label(), config.base_url),
            },
            None => lang
                .pick("Current: offline rules", "当前：离线规则")
                .to_string(),
        };
        if let Ok(Some(settings)) = LlmSettings::load(&self.agent.config_path)
            && !settings.profiles.is_empty()
        {
            text.push_str(lang.pick("\nSaved configurations:", "\n已保存的配置："));
            for (name, entry) in &settings.profiles {
                let mark = if settings.active.as_deref() == Some(name) {
                    "*"
                } else {
                    " "
                };
                text.push_str(&format!(
                    "\n  {mark} {name}  {} · {}",
                    entry.provider.as_deref().unwrap_or("?"),
                    entry.model.as_deref().unwrap_or("-")
                ));
            }
        }
        text.push_str(lang.pick(
            "\n/model <name> switches · /model list shows provider models · /setup adds one",
            "\n/model <名称> 切换 · /model list 查看厂商模型 · /setup 新增配置",
        ));
        self.agent.info(text);
    }

    fn set_model(&mut self, choice: &str) {
        let lang = self.setup_lang();
        let model = match choice.parse::<usize>() {
            Ok(n) => match n.checked_sub(1).and_then(|i| self.agent.last_models.get(i)) {
                Some(model) => model.clone(),
                None => {
                    self.agent.error(lang.pick(
                        "No such number; run /model list first.",
                        "编号无效，请先运行 /model list。",
                    ));
                    return;
                }
            },
            Err(_) => choice.to_string(),
        };
        let Some(config) = &mut self.agent.llm else {
            self.agent.error(lang.pick(
                "No model configured yet; run /setup first.",
                "尚未配置模型，请先运行 /setup。",
            ));
            return;
        };
        config.model = model.clone();
        let saved = match LlmSettings::load(&self.agent.config_path) {
            Ok(Some(mut settings)) => match settings.active.clone() {
                Some(active) if settings.profiles.contains_key(&active) => {
                    if let Some(entry) = settings.profiles.get_mut(&active) {
                        entry.model = Some(model.clone());
                    }
                    settings.save(&self.agent.config_path).ok().map(|_| active)
                }
                _ => None,
            },
            _ => None,
        };
        self.agent.info(match (saved, lang) {
            (Some(profile), Lang::Zh) => format!("已切换到 {model}，并保存到配置「{profile}」。"),
            (Some(profile), Lang::En) => format!("Using {model}, saved to \"{profile}\"."),
            (None, Lang::Zh) => {
                format!("本次会话改用 {model}（没有可保存的配置，运行 /setup 可保存）。")
            }
            (None, Lang::En) => format!("Using {model} for this session (run /setup to save it)."),
        });
    }

    /// `/model <name>`: a saved profile, otherwise a model for this session.
    fn switch_or_override(&mut self, name: &str) {
        let lang = self.setup_lang();
        let settings = match LlmSettings::load(&self.agent.config_path) {
            Ok(settings) => settings,
            Err(error) => {
                self.agent.error(format!("{error}"));
                return;
            }
        };
        if let Some(mut settings) = settings.filter(|s| s.profiles.contains_key(name)) {
            settings.active = Some(name.to_string());
            let overrides = Overrides {
                profile: None,
                ..self.agent.overrides.clone()
            };
            match resolve(Some(&settings), &overrides, self.agent.env.as_ref()) {
                Ok(Some(resolved)) => {
                    if let Err(error) = settings.save(&self.agent.config_path) {
                        self.agent.error(format!("{error}"));
                        return;
                    }
                    let label = resolved.config.label();
                    self.agent.llm = Some(resolved.config);
                    self.agent.info(match lang {
                        Lang::Zh => format!("已切换到配置「{name}」：{label}"),
                        Lang::En => format!("Switched to \"{name}\": {label}"),
                    });
                }
                Ok(None) => {}
                Err(error) => self.agent.error(match lang {
                    Lang::Zh => format!("无法切换到「{name}」：{error}"),
                    Lang::En => format!("Cannot switch to \"{name}\": {error}"),
                }),
            }
            return;
        }
        match &mut self.agent.llm {
            Some(config) => {
                config.model = name.to_string();
                self.agent.info(match lang {
                    Lang::Zh => format!("本次会话改用模型 {name}（/model set {name} 可保存）"),
                    Lang::En => {
                        format!("Using model {name} for this session (/model set {name} saves it)")
                    }
                });
            }
            None => self.agent.info(lang.pick(
                "Offline mode. Run /setup to connect a model (no environment variables needed).",
                "当前为离线模式。运行 /setup 即可配置模型，无需设置环境变量。",
            )),
        }
    }

    pub fn config_command(&mut self) {
        let lang = self.setup_lang();
        let result = load_and_resolve(
            &self.agent.config_path,
            &self.agent.overrides,
            self.agent.env.as_ref(),
        );
        let mut text = report(&result, &self.agent.config_path, lang);
        let effective = result
            .as_ref()
            .ok()
            .and_then(|r| r.as_ref())
            .map(|r| &r.config);
        if let Some(session) = &self.agent.llm
            && effective != Some(session)
        {
            text.push_str(&match lang {
                Lang::Zh => format!("\n本次会话实际使用：{}", session.label()),
                Lang::En => format!("\nThis session uses: {}", session.label()),
            });
        }
        self.agent.info(text);
    }
}

#[cfg(test)]
mod tests {
    use super::super::agent::tests::app_with;
    use super::*;
    use crate::agent::llm::Completion;
    use crate::agent::llm::tests::ScriptedClient;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn wait(app: &mut App) {
        let start = Instant::now();
        while app.agent.is_busy() {
            app.poll_agent();
            assert!(start.elapsed() < Duration::from_secs(10), "setup hung");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn say(app: &mut App, text: &str) {
        app.agent.input.set(text);
        app.submit_agent_input();
        wait(app);
    }

    fn step(app: &App) -> Option<SetupStep> {
        app.agent.setup.as_ref().map(|w| w.step)
    }

    fn last_text(app: &App) -> String {
        match app.agent.entries.last() {
            Some(Entry::Info(t) | Entry::Error(t) | Entry::User(t) | Entry::Assistant(t)) => {
                t.clone()
            }
            other => format!("{other:?}"),
        }
    }

    fn ok_client(app: &mut App) {
        app.agent.client_factory =
            Box::new(|_| Box::new(ScriptedClient::new(vec![Completion::default()])));
    }

    #[test]
    fn pasted_key_flow_saves_a_profile_without_the_key() {
        let (mut app, dir) = app_with(None);
        ok_client(&mut app);
        app.open_agent();
        say(&mut app, "/setup");
        assert_eq!(step(&app), Some(SetupStep::Provider));
        assert!(
            app.agent
                .entries
                .iter()
                .any(|e| matches!(e, Entry::Info(t) if t.contains("DeepSeek")))
        );

        say(&mut app, "");
        assert_eq!(step(&app), Some(SetupStep::Key));
        assert!(app.agent.input_masked());
        say(&mut app, "sk-secret-123456789");
        assert_eq!(step(&app), Some(SetupStep::KeyStorage));
        // The key is never echoed.
        assert!(
            !app.agent
                .entries
                .iter()
                .any(|e| format!("{e:?}").contains("sk-secret"))
        );
        say(&mut app, "");
        assert_eq!(step(&app), Some(SetupStep::Model));
        assert!(last_text(&app).contains("1. m1"), "{}", last_text(&app));
        say(&mut app, "2");
        assert_eq!(step(&app), Some(SetupStep::Name));
        say(&mut app, "");
        assert!(app.agent.setup.is_none());

        let llm = app.agent.llm.clone().unwrap();
        assert_eq!(llm.label(), "deepseek · m2");
        assert_eq!(llm.api_key.as_deref(), Some("sk-secret-123456789"));
        let text = std::fs::read_to_string(dir.path().join("config.yaml")).unwrap();
        assert!(!text.contains("sk-secret"), "{text}");
        assert!(text.contains("active: \"deepseek\""), "{text}");
        assert!(last_text(&app).contains("DEEPSEEK_API_KEY"));
    }

    #[test]
    fn saving_the_key_and_detected_variables() {
        let (mut app, dir) = app_with(None);
        ok_client(&mut app);
        app.agent.env = Arc::new(|k| (k == "MOONSHOT_API_KEY").then(|| "sk-moon".into()));
        app.open_agent();
        assert!(
            app.agent
                .entries
                .iter()
                .any(|e| matches!(e, Entry::Info(t) if t.contains("MOONSHOT_API_KEY")))
        );
        say(&mut app, "/setup");
        // The detected provider is the default and Enter uses its variable.
        say(&mut app, "");
        assert_eq!(app.agent.setup.as_ref().unwrap().preset.id, "kimi");
        say(&mut app, "");
        assert_eq!(step(&app), Some(SetupStep::Model));
        say(&mut app, "m1");
        say(&mut app, "moon");
        let settings = LlmSettings::load(&dir.path().join("config.yaml"))
            .unwrap()
            .unwrap();
        let entry = &settings.profiles["moon"];
        assert_eq!(entry.api_key_env.as_deref(), Some("MOONSHOT_API_KEY"));
        assert_eq!(entry.api_key, None);
        assert_eq!(
            app.agent.llm.as_ref().unwrap().api_key.as_deref(),
            Some("sk-moon")
        );

        // Choosing "2" writes the pasted key.
        say(&mut app, "/setup");
        say(&mut app, "qwen");
        say(&mut app, "sk-qwen-123456789");
        say(&mut app, "2");
        say(&mut app, "");
        say(&mut app, "");
        let settings = LlmSettings::load(&dir.path().join("config.yaml"))
            .unwrap()
            .unwrap();
        assert_eq!(settings.active.as_deref(), Some("qwen"));
        assert_eq!(
            settings.profiles["qwen"].api_key.as_deref(),
            Some("sk-qwen-123456789")
        );
        assert_eq!(settings.profiles.len(), 2);
    }

    #[test]
    fn local_and_custom_providers() {
        let (mut app, _dir) = app_with(None);
        ok_client(&mut app);
        app.open_agent();
        say(&mut app, "/setup");
        say(&mut app, "ollama");
        // No key step for a local server.
        assert_eq!(step(&app), Some(SetupStep::Model));
        app.agent_escape();
        assert!(app.agent.setup.is_none());

        say(&mut app, "/setup");
        say(&mut app, "custom");
        assert_eq!(step(&app), Some(SetupStep::BaseUrl));
        say(&mut app, "ftp://nope");
        assert_eq!(step(&app), Some(SetupStep::BaseUrl));
        say(&mut app, "https://llm.example.com/v1/");
        say(&mut app, "");
        say(&mut app, "m1");
        say(&mut app, "mine");
        let llm = app.agent.llm.clone().unwrap();
        assert_eq!(llm.base_url, "https://llm.example.com/v1");
        assert_eq!(llm.provider, "custom");
    }

    #[test]
    fn errors_lead_to_useful_next_steps() {
        let (mut app, _dir) = app_with(None);
        app.agent.client_factory = Box::new(|_| Box::new(ScriptedClient::new(vec![])));
        app.agent.models_fetcher = Arc::new(|_| {
            Err(LlmError::Status {
                code: 500,
                body: String::new(),
            })
        });
        app.open_agent();
        say(&mut app, "/setup");
        say(&mut app, "nonsense");
        assert_eq!(step(&app), Some(SetupStep::Provider));
        say(&mut app, "1");
        say(&mut app, "env:MISSING_VAR");
        assert_eq!(step(&app), Some(SetupStep::Key));
        say(&mut app, "sk-123456789");
        say(&mut app, "1");
        // Listing failed: type the model by hand, the preset default is offered.
        assert_eq!(step(&app), Some(SetupStep::Model));
        assert!(last_text(&app).contains("deepseek-flash"));
        say(&mut app, "");
        // The connection test failed: confirm before saving.
        assert_eq!(step(&app), Some(SetupStep::SaveAnyway));
        say(&mut app, "n");
        assert!(app.agent.setup.is_none());
        assert!(app.agent.llm.is_none());
    }

    #[test]
    fn wrong_key_is_asked_again() {
        let (mut app, _dir) = app_with(None);
        app.agent.models_fetcher = Arc::new(|_| {
            Err(LlmError::Status {
                code: 401,
                body: "bad key".into(),
            })
        });
        app.open_agent();
        say(&mut app, "/setup");
        say(&mut app, "");
        say(&mut app, "sk-wrong-123456");
        say(&mut app, "");
        assert_eq!(step(&app), Some(SetupStep::Key));
    }

    #[test]
    fn model_commands_list_set_and_switch() {
        let (mut app, dir) = app_with(None);
        ok_client(&mut app);
        app.open_agent();
        say(&mut app, "/model list");
        assert!(matches!(app.agent.entries.last(), Some(Entry::Error(t)) if t.contains("/setup")));

        let mut settings = LlmSettings::default();
        settings.set_profile(
            "a",
            ProfileEntry {
                provider: Some("ollama".into()),
                model: Some("llama".into()),
                ..ProfileEntry::default()
            },
            false,
        );
        settings.set_profile(
            "b",
            ProfileEntry {
                provider: Some("openai".into()),
                model: Some("gpt".into()),
                ..ProfileEntry::default()
            },
            false,
        );
        settings.save(&dir.path().join("config.yaml")).unwrap();

        say(&mut app, "/model a");
        assert_eq!(app.agent.model_label(), "ollama · llama");
        // "b" has no key: the error explains, nothing changes.
        say(&mut app, "/model b");
        assert!(
            matches!(app.agent.entries.last(), Some(Entry::Error(t)) if t.contains("OPENAI_API_KEY"))
        );
        assert_eq!(app.agent.model_label(), "ollama · llama");

        say(&mut app, "/model list");
        assert_eq!(app.agent.last_models, ["m1", "m2"]);
        say(&mut app, "/model set 2");
        assert_eq!(app.agent.model_label(), "ollama · m2");
        let saved = LlmSettings::load(&dir.path().join("config.yaml"))
            .unwrap()
            .unwrap();
        assert_eq!(saved.active.as_deref(), Some("a"));
        assert_eq!(saved.profiles["a"].model.as_deref(), Some("m2"));

        say(&mut app, "/model");
        assert!(last_text(&app).contains("* a"), "{}", last_text(&app));
        say(&mut app, "/model temp-model");
        assert_eq!(app.agent.model_label(), "ollama · temp-model");
        say(&mut app, "/config");
        assert!(
            last_text(&app).contains("temp-model"),
            "{}",
            last_text(&app)
        );
    }

    #[test]
    fn config_reports_offline_and_errors() {
        let (mut app, dir) = app_with(None);
        app.open_agent();
        say(&mut app, "/config");
        assert!(last_text(&app).contains("/setup"));
        std::fs::write(dir.path().join("config.yaml"), "llm: [").unwrap();
        say(&mut app, "/config");
        assert!(last_text(&app).contains("invalid YAML"));
        // A broken file is never overwritten by /setup.
        ok_client(&mut app);
        say(&mut app, "/setup");
        for answer in ["ollama", "m1", ""] {
            say(&mut app, answer);
        }
        assert_eq!(
            std::fs::read_to_string(dir.path().join("config.yaml")).unwrap(),
            "llm: ["
        );
    }
}
