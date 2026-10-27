//! LLM settings: `~/.ccfddl/config.yaml`, environment variables and CLI
//! overrides, merged into one [`LlmConfig`] with the origin of every value.
//!
//! Precedence, highest first: CLI flags, `CCFDDL_*` environment variables,
//! the active profile in `config.yaml`, global values in `config.yaml`, the
//! provider preset. With no configuration at all the agent stays offline:
//! standard variables such as `DEEPSEEK_API_KEY` are only *detected* and
//! offered in `/setup`, never used behind the user's back. Once a profile
//! names a provider, that provider's standard variable is a key fallback.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_yaml_ng::Value;

use super::Lang;
use super::llm::LlmConfig;
use super::providers::{self, Preset};
use crate::data::favorites::app_dir;

pub const DEFAULT_TEMPERATURE: f64 = 0.3;
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;

pub fn default_config_path() -> PathBuf {
    app_dir().join("config.yaml")
}

/// One named model configuration.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProfileEntry {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub stream: Option<bool>,
    pub temperature: Option<f64>,
    pub timeout_secs: Option<u64>,
    pub proxy: Option<String>,
}

/// The `llm:` section of `config.yaml`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LlmSettings {
    pub active: Option<String>,
    pub profiles: BTreeMap<String, ProfileEntry>,
    /// Global defaults; a profile can override each of them.
    pub defaults: ProfileEntry,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("{path}: {message}")]
    Read { path: String, message: String },
    #[error("{path}: invalid YAML: {message}")]
    Parse { path: String, message: String },
    #[error("no model profile called \"{0}\"")]
    UnknownProfile(String),
    #[error("unknown provider \"{0}\" (set base_url, or use one of: {1})")]
    UnknownProvider(String, String),
    #[error("provider \"{0}\" needs a base_url")]
    MissingBaseUrl(String),
    #[error("no model chosen for provider \"{0}\"; run /setup or set `model`")]
    MissingModel(String),
    #[error("no API key for \"{provider}\"; run /setup, or set {hint}")]
    MissingKey { provider: String, hint: String },
}

fn opt_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

fn opt_bool(value: Option<&Value>) -> Option<bool> {
    match value {
        Some(Value::Bool(b)) => Some(*b),
        Some(Value::String(s)) => parse_bool(s),
        _ => None,
    }
}

fn parse_bool(text: &str) -> Option<bool> {
    match text.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn opt_f64(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
}

fn opt_u64(value: Option<&Value>) -> Option<u64> {
    match value {
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
}

impl ProfileEntry {
    fn from_value(value: &Value) -> Self {
        Self {
            provider: opt_string(value.get("provider")).map(|p| p.to_lowercase()),
            model: opt_string(value.get("model")),
            base_url: opt_string(value.get("base_url")),
            api_key: opt_string(value.get("api_key")),
            api_key_env: opt_string(value.get("api_key_env")),
            stream: opt_bool(value.get("stream")),
            temperature: opt_f64(value.get("temperature")),
            timeout_secs: opt_u64(value.get("timeout_secs")),
            proxy: opt_string(value.get("proxy")),
        }
    }

    fn write_yaml(&self, out: &mut String, indent: &str) {
        let mut line = |key: &str, value: String| {
            out.push_str(&format!("{indent}{key}: {value}\n"));
        };
        if let Some(v) = &self.provider {
            line("provider", quote(v));
        }
        if let Some(v) = &self.model {
            line("model", quote(v));
        }
        if let Some(v) = &self.base_url {
            line("base_url", quote(v));
        }
        if let Some(v) = &self.api_key_env {
            line("api_key_env", quote(v));
        }
        if let Some(v) = &self.api_key {
            line("api_key", quote(v));
        }
        if let Some(v) = self.stream {
            line("stream", v.to_string());
        }
        if let Some(v) = self.temperature {
            line("temperature", v.to_string());
        }
        if let Some(v) = self.timeout_secs {
            line("timeout_secs", v.to_string());
        }
        if let Some(v) = &self.proxy {
            line("proxy", quote(v));
        }
    }
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

impl LlmSettings {
    pub fn parse(content: &str, origin: &str) -> Result<Self, ConfigError> {
        let value: Value = serde_yaml_ng::from_str(content).map_err(|e| ConfigError::Parse {
            path: origin.to_string(),
            message: e.to_string(),
        })?;
        let Some(llm) = value.get("llm") else {
            return Ok(Self::default());
        };
        let mut profiles = BTreeMap::new();
        if let Some(Value::Mapping(map)) = llm.get("profiles") {
            for (name, entry) in map {
                let name = match name {
                    Value::String(s) => s.trim().to_string(),
                    Value::Number(n) => n.to_string(),
                    _ => continue,
                };
                if !name.is_empty() {
                    profiles.insert(name, ProfileEntry::from_value(entry));
                }
            }
        }
        Ok(Self {
            active: opt_string(llm.get("active")),
            profiles,
            defaults: ProfileEntry::from_value(llm),
        })
    }

    /// Load the settings; a missing file yields `Ok(None)`.
    pub fn load(path: &Path) -> Result<Option<Self>, ConfigError> {
        match fs::read_to_string(path) {
            Ok(content) => Self::parse(&content, &path.display().to_string()).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(ConfigError::Read {
                path: path.display().to_string(),
                message: e.to_string(),
            }),
        }
    }

    /// Write the file; on Unix it is readable by the owner only, because it
    /// may hold an API key.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("yaml.tmp");
        fs::write(&tmp, self.to_yaml())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
        }
        fs::rename(&tmp, path)
    }

    pub fn to_yaml(&self) -> String {
        let mut out = String::from(
            "# ccfddl-tui settings, written by `/setup` or `ccfddl-tui --setup`.\n\
             # Providers: deepseek, qwen, kimi, zhipu, siliconflow, openrouter, openai, ollama, custom.\n\
             # Prefer `api_key_env` (the name of an environment variable) over `api_key`.\n\
             llm:\n",
        );
        if let Some(active) = &self.active {
            out.push_str(&format!("  active: {}\n", quote(active)));
        }
        self.defaults_without_identity().write_yaml(&mut out, "  ");
        out.push_str("  profiles:\n");
        if self.profiles.is_empty() {
            out.push_str("    {}\n");
        }
        for (name, entry) in &self.profiles {
            out.push_str(&format!("    {}:\n", quote(name)));
            entry.write_yaml(&mut out, "      ");
        }
        out
    }

    /// Global defaults only carry tuning values, never a provider or key.
    fn defaults_without_identity(&self) -> ProfileEntry {
        ProfileEntry {
            stream: self.defaults.stream,
            temperature: self.defaults.temperature,
            timeout_secs: self.defaults.timeout_secs,
            proxy: self.defaults.proxy.clone(),
            ..ProfileEntry::default()
        }
    }

    /// Insert or replace a profile, optionally making it the active one.
    pub fn set_profile(&mut self, name: &str, entry: ProfileEntry, activate: bool) {
        self.profiles.insert(name.to_string(), entry);
        if activate {
            self.active = Some(name.to_string());
        }
    }

    pub fn profile_names(&self) -> Vec<String> {
        self.profiles.keys().cloned().collect()
    }
}

/// One-off overrides from the command line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overrides {
    pub profile: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
}

/// Where a value came from, for `/config` and `--check-llm`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Cli,
    Env(String),
    Profile(String),
    Global,
    Preset(&'static str),
    Default,
}

impl Origin {
    pub fn describe(&self, lang: Lang) -> String {
        match (self, lang) {
            (Self::Cli, Lang::Zh) => "命令行参数".into(),
            (Self::Cli, Lang::En) => "command line".into(),
            (Self::Env(var), Lang::Zh) => format!("环境变量 {var}"),
            (Self::Env(var), Lang::En) => format!("env {var}"),
            (Self::Profile(name), Lang::Zh) => format!("config.yaml 配置 \"{name}\""),
            (Self::Profile(name), Lang::En) => format!("config.yaml profile \"{name}\""),
            (Self::Global, Lang::Zh) => "config.yaml 全局设置".into(),
            (Self::Global, Lang::En) => "config.yaml global".into(),
            (Self::Preset(id), Lang::Zh) => format!("{id} 预设"),
            (Self::Preset(id), Lang::En) => format!("{id} preset"),
            (Self::Default, Lang::Zh) => "默认值".into(),
            (Self::Default, Lang::En) => "default".into(),
        }
    }
}

/// The effective configuration and where each part came from.
#[derive(Clone, PartialEq)]
pub struct Resolved {
    pub config: LlmConfig,
    /// Active `config.yaml` profile, if any.
    pub profile: Option<String>,
    /// `(field, origin)` in display order.
    pub sources: Vec<(&'static str, Origin)>,
}

impl fmt::Debug for Resolved {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Resolved")
            .field("config", &self.config)
            .field("profile", &self.profile)
            .field("sources", &self.sources)
            .finish()
    }
}

/// `sk-abc…wxyz`: enough to recognise a key, not enough to use it.
pub fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.trim().chars().collect();
    if chars.len() <= 8 {
        return "••••".to_string();
    }
    let head: String = chars[..3].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

type Lookup<'a> = &'a dyn Fn(&str) -> Option<String>;

fn env_value(env: Lookup, key: &str) -> Option<String> {
    env(key)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Merge settings, environment and CLI flags. `Ok(None)` means offline: no
/// provider was chosen anywhere.
pub fn resolve(
    settings: Option<&LlmSettings>,
    overrides: &Overrides,
    env: Lookup,
) -> Result<Option<Resolved>, ConfigError> {
    let empty = LlmSettings::default();
    let settings = settings.unwrap_or(&empty);
    let mut sources: Vec<(&'static str, Origin)> = Vec::new();

    // --- Profile ---------------------------------------------------------
    let profile_name = match &overrides.profile {
        Some(name) => {
            if !settings.profiles.contains_key(name) {
                return Err(ConfigError::UnknownProfile(name.clone()));
            }
            Some(name.clone())
        }
        None => settings
            .active
            .clone()
            .filter(|name| settings.profiles.contains_key(name)),
    };
    let blank = ProfileEntry::default();
    let entry = profile_name
        .as_ref()
        .and_then(|name| settings.profiles.get(name))
        .unwrap_or(&blank);
    let from_profile = || Origin::Profile(profile_name.clone().unwrap_or_default());

    let env_key = env_value(env, "CCFDDL_API_KEY");
    let env_base = env_value(env, "CCFDDL_BASE_URL");
    let env_model = env_value(env, "CCFDDL_MODEL");

    // --- Provider --------------------------------------------------------
    let (provider, provider_origin) = if let Some(p) = &overrides.provider {
        (p.to_lowercase(), Origin::Cli)
    } else if let Some(p) = &entry.provider {
        (p.clone(), from_profile())
    } else if overrides.base_url.is_some() || env_base.is_some() || entry.base_url.is_some() {
        ("custom".to_string(), Origin::Default)
    } else if env_key.is_some() || profile_name.is_some() {
        // Backwards compatible: CCFDDL_API_KEY alone meant DeepSeek.
        (providers::DEFAULT_PROVIDER.to_string(), Origin::Default)
    } else {
        return Ok(None);
    };
    let preset: &Preset = match providers::preset(&provider) {
        Some(preset) => preset,
        None => {
            let ids: Vec<&str> = providers::PRESETS.iter().map(|p| p.id).collect();
            return Err(ConfigError::UnknownProvider(provider, ids.join(", ")));
        }
    };
    sources.push(("provider", provider_origin));

    // --- Base URL ----------------------------------------------------------
    let (base_url, origin) = if let Some(url) = &overrides.base_url {
        (url.clone(), Origin::Cli)
    } else if let Some(url) = &env_base {
        (url.clone(), Origin::Env("CCFDDL_BASE_URL".into()))
    } else if let Some(url) = &entry.base_url {
        (url.clone(), from_profile())
    } else if !preset.base_url.is_empty() {
        (preset.base_url.to_string(), Origin::Preset(preset.id))
    } else {
        return Err(ConfigError::MissingBaseUrl(preset.id.to_string()));
    };
    let base_url = base_url.trim().trim_end_matches('/').to_string();
    sources.push(("base_url", origin));

    // --- Model -------------------------------------------------------------
    let (model, origin) = if let Some(m) = &overrides.model {
        (m.clone(), Origin::Cli)
    } else if let Some(m) = &env_model {
        (m.clone(), Origin::Env("CCFDDL_MODEL".into()))
    } else if let Some(m) = &entry.model {
        (m.clone(), from_profile())
    } else if let Some(m) = preset.default_model {
        (m.to_string(), Origin::Preset(preset.id))
    } else {
        return Err(ConfigError::MissingModel(preset.id.to_string()));
    };
    sources.push(("model", origin));

    // --- API key -----------------------------------------------------------
    let mut key: Option<(String, Origin)> = None;
    if let Some(k) = env_key {
        key = Some((k, Origin::Env("CCFDDL_API_KEY".into())));
    } else if let Some(k) = &entry.api_key {
        key = Some((k.clone(), from_profile()));
    } else if let Some(var) = &entry.api_key_env {
        if let Some(k) = env_value(env, var) {
            key = Some((k, Origin::Env(var.clone())));
        }
    } else if entry.provider.is_some() || overrides.provider.is_some() {
        // The user picked this provider, so its standard variable is fair game.
        if let Some(var) = preset.key_env
            && let Some(k) = env_value(env, var)
        {
            key = Some((k, Origin::Env(var.to_string())));
        }
    }
    let needs_key = preset.needs_key && !providers::is_local_url(&base_url);
    if key.is_none() && needs_key {
        let hint = entry
            .api_key_env
            .clone()
            .or(preset.key_env.map(str::to_string))
            .unwrap_or_else(|| "CCFDDL_API_KEY".to_string());
        return Err(ConfigError::MissingKey {
            provider: preset.id.to_string(),
            hint,
        });
    }
    if let Some((_, origin)) = &key {
        sources.push(("api_key", origin.clone()));
    }

    // --- Tuning ------------------------------------------------------------
    let defaults = &settings.defaults;
    let (stream, origin) =
        if let Some(s) = env_value(env, "CCFDDL_STREAM").and_then(|v| parse_bool(&v)) {
            (s, Origin::Env("CCFDDL_STREAM".into()))
        } else if let Some(s) = entry.stream {
            (s, from_profile())
        } else if let Some(s) = defaults.stream {
            (s, Origin::Global)
        } else {
            (true, Origin::Default)
        };
    sources.push(("stream", origin));
    let pick_f64 = |p: Option<f64>, g: Option<f64>, d: f64| match (p, g) {
        (Some(v), _) => (v, from_profile()),
        (None, Some(v)) => (v, Origin::Global),
        (None, None) => (d, Origin::Default),
    };
    let (temperature, origin) =
        pick_f64(entry.temperature, defaults.temperature, DEFAULT_TEMPERATURE);
    sources.push(("temperature", origin));
    let (timeout_secs, origin) = match (entry.timeout_secs, defaults.timeout_secs) {
        (Some(v), _) => (v, from_profile()),
        (None, Some(v)) => (v, Origin::Global),
        (None, None) => (DEFAULT_TIMEOUT_SECS, Origin::Default),
    };
    sources.push(("timeout_secs", origin));
    let proxy = match (&entry.proxy, &defaults.proxy) {
        (Some(p), _) => {
            sources.push(("proxy", from_profile()));
            Some(p.clone())
        }
        (None, Some(p)) => {
            sources.push(("proxy", Origin::Global));
            Some(p.clone())
        }
        _ => None,
    };

    Ok(Some(Resolved {
        config: LlmConfig {
            provider: preset.id.to_string(),
            base_url,
            api_key: key.map(|(k, _)| k),
            model,
            stream,
            temperature,
            timeout_secs: timeout_secs.max(5),
            proxy,
        },
        profile: profile_name,
        sources,
    }))
}

/// Load `path` and resolve. Errors are returned as text for the UI.
pub fn load_and_resolve(
    path: &Path,
    overrides: &Overrides,
    env: Lookup,
) -> Result<Option<Resolved>, ConfigError> {
    let settings = LlmSettings::load(path)?;
    resolve(settings.as_ref(), overrides, env)
}

/// Human-readable summary for `/config` and `--check-llm`.
pub fn report(
    result: &Result<Option<Resolved>, ConfigError>,
    config_path: &Path,
    lang: Lang,
) -> String {
    let mut out = format!(
        "{}: {}\n",
        lang.pick("Config file", "配置文件"),
        config_path.display()
    );
    match result {
        Err(error) => out.push_str(&format!("{}: {error}", lang.pick("Error", "错误"))),
        Ok(None) => out.push_str(lang.pick(
            "No model configured: offline rule-based mode. Run /setup (or ccfddl-tui --setup).",
            "尚未配置模型：使用离线规则模式。运行 /setup（或 ccfddl-tui --setup）进行配置。",
        )),
        Ok(Some(resolved)) => {
            let config = &resolved.config;
            if let Some(profile) = &resolved.profile {
                out.push_str(&format!(
                    "{}: {profile}\n",
                    lang.pick("Profile", "当前配置")
                ));
            }
            for (field, origin) in &resolved.sources {
                let value = match *field {
                    "provider" => config.provider.clone(),
                    "base_url" => config.base_url.clone(),
                    "model" => config.model.clone(),
                    "api_key" => config.api_key.as_deref().map(mask_key).unwrap_or_default(),
                    "stream" => config.stream.to_string(),
                    "temperature" => config.temperature.to_string(),
                    "timeout_secs" => config.timeout_secs.to_string(),
                    "proxy" => config.proxy.clone().unwrap_or_default(),
                    _ => String::new(),
                };
                out.push_str(&format!(
                    "  {field:<12} {value}  ({})\n",
                    origin.describe(lang)
                ));
            }
            if config.api_key.is_none() {
                out.push_str(&format!(
                    "  {:<12} {}\n",
                    "api_key",
                    lang.pick("(none, local server)", "（无，本地服务）")
                ));
            }
            out.pop();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        }
    }

    const FILE: &str = "
llm:
  active: work
  temperature: 0.7
  proxy: http://127.0.0.1:7890
  profiles:
    work:
      provider: qwen
      model: qwen-max
      api_key_env: MY_QWEN_KEY
    inline:
      provider: kimi
      api_key: sk-inline-123456789
      stream: false
      timeout_secs: 30
    local:
      provider: ollama
      model: qwen2.5:7b
    bad:
      provider: nope
";

    fn settings() -> LlmSettings {
        LlmSettings::parse(FILE, "test").unwrap()
    }

    #[test]
    fn no_configuration_means_offline_even_with_standard_keys() {
        let result = resolve(
            None,
            &Overrides::default(),
            &env(&[("DEEPSEEK_API_KEY", "sk-x")]),
        );
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn legacy_ccfddl_variables_still_work() {
        let r = resolve(
            None,
            &Overrides::default(),
            &env(&[("CCFDDL_API_KEY", "sk-legacy-12345")]),
        )
        .unwrap()
        .unwrap();
        assert_eq!(r.config.provider, "deepseek");
        assert_eq!(r.config.base_url, "https://api.deepseek.com");
        assert_eq!(r.config.model, "deepseek-flash");
        assert_eq!(r.config.api_key.as_deref(), Some("sk-legacy-12345"));

        let local = resolve(
            None,
            &Overrides::default(),
            &env(&[
                ("CCFDDL_BASE_URL", "http://localhost:11434/v1/"),
                ("CCFDDL_MODEL", "qwen2.5"),
                ("CCFDDL_STREAM", "0"),
            ]),
        )
        .unwrap()
        .unwrap();
        assert_eq!(local.config.provider, "custom");
        assert_eq!(local.config.base_url, "http://localhost:11434/v1");
        assert_eq!(local.config.api_key, None);
        assert!(!local.config.stream);
    }

    #[test]
    fn active_profile_with_key_from_named_variable() {
        let s = settings();
        let r = resolve(
            Some(&s),
            &Overrides::default(),
            &env(&[("MY_QWEN_KEY", "sk-qwen-abcdef")]),
        )
        .unwrap()
        .unwrap();
        assert_eq!(r.profile.as_deref(), Some("work"));
        assert_eq!(r.config.provider, "qwen");
        assert_eq!(r.config.model, "qwen-max");
        assert_eq!(
            r.config.base_url,
            "https://dashscope.aliyuncs.com/compatible-mode/v1"
        );
        assert_eq!(r.config.temperature, 0.7);
        assert_eq!(r.config.proxy.as_deref(), Some("http://127.0.0.1:7890"));
        assert!(
            r.sources
                .contains(&("api_key", Origin::Env("MY_QWEN_KEY".into())))
        );
        assert!(r.sources.contains(&("base_url", Origin::Preset("qwen"))));
        assert!(r.sources.contains(&("temperature", Origin::Global)));
    }

    #[test]
    fn missing_key_names_the_variable_to_set() {
        let s = settings();
        let err = resolve(Some(&s), &Overrides::default(), &env(&[])).unwrap_err();
        assert_eq!(
            err,
            ConfigError::MissingKey {
                provider: "qwen".into(),
                hint: "MY_QWEN_KEY".into()
            }
        );
    }

    #[test]
    fn chosen_provider_falls_back_to_its_standard_variable() {
        let mut s = LlmSettings::default();
        s.set_profile(
            "ds",
            ProfileEntry {
                provider: Some("deepseek".into()),
                ..ProfileEntry::default()
            },
            true,
        );
        let r = resolve(
            Some(&s),
            &Overrides::default(),
            &env(&[("DEEPSEEK_API_KEY", "sk-ds-123456789")]),
        )
        .unwrap()
        .unwrap();
        assert!(
            r.sources
                .contains(&("api_key", Origin::Env("DEEPSEEK_API_KEY".into())))
        );
    }

    #[test]
    fn cli_and_env_override_the_file() {
        let s = settings();
        let overrides = Overrides {
            profile: Some("inline".into()),
            model: Some("kimi-k3".into()),
            ..Overrides::default()
        };
        let r = resolve(Some(&s), &overrides, &env(&[("CCFDDL_STREAM", "true")]))
            .unwrap()
            .unwrap();
        assert_eq!(r.config.provider, "kimi");
        assert_eq!(r.config.model, "kimi-k3");
        assert!(r.config.stream, "env beats profile");
        assert_eq!(r.config.timeout_secs, 30);
        assert!(r.sources.contains(&("model", Origin::Cli)));
        assert!(
            r.sources
                .contains(&("api_key", Origin::Profile("inline".into())))
        );

        let r = resolve(Some(&s), &overrides, &env(&[("CCFDDL_MODEL", "x")]))
            .unwrap()
            .unwrap();
        assert_eq!(r.config.model, "kimi-k3", "CLI beats env");
    }

    #[test]
    fn local_providers_need_no_key_and_errors_are_specific() {
        let s = settings();
        let ov = |p: &str| Overrides {
            profile: Some(p.into()),
            ..Overrides::default()
        };
        let r = resolve(Some(&s), &ov("local"), &env(&[])).unwrap().unwrap();
        assert_eq!(r.config.base_url, "http://localhost:11434/v1");
        assert!(matches!(
            resolve(Some(&s), &ov("bad"), &env(&[])),
            Err(ConfigError::UnknownProvider(..))
        ));
        assert!(matches!(
            resolve(Some(&s), &ov("missing"), &env(&[])),
            Err(ConfigError::UnknownProfile(_))
        ));
        let openai = Overrides {
            provider: Some("openai".into()),
            ..Overrides::default()
        };
        assert!(matches!(
            resolve(None, &openai, &env(&[("OPENAI_API_KEY", "sk")])),
            Err(ConfigError::MissingModel(_))
        ));
        let custom = Overrides {
            provider: Some("custom".into()),
            ..Overrides::default()
        };
        assert!(matches!(
            resolve(None, &custom, &env(&[])),
            Err(ConfigError::MissingBaseUrl(_))
        ));
    }

    #[test]
    fn settings_round_trip_through_yaml() {
        let mut s = settings();
        s.profiles.remove("bad");
        let text = s.to_yaml();
        assert_eq!(LlmSettings::parse(&text, "test").unwrap(), s, "{text}");
        assert_eq!(LlmSettings::parse("", "t").unwrap(), LlmSettings::default());
        let empty = LlmSettings::default().to_yaml();
        assert_eq!(
            LlmSettings::parse(&empty, "t").unwrap(),
            LlmSettings::default()
        );
    }

    #[test]
    fn save_and_load_use_private_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/config.yaml");
        assert_eq!(LlmSettings::load(&path), Ok(None));
        let s = settings();
        s.save(&path).unwrap();
        assert_eq!(LlmSettings::load(&path).unwrap().unwrap(), s);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn keys_are_masked_everywhere() {
        assert_eq!(mask_key("sk-1234567890abcd"), "sk-…abcd");
        assert_eq!(mask_key("short"), "••••");
        let s = settings();
        let result = resolve(
            Some(&s),
            &Overrides {
                profile: Some("inline".into()),
                ..Overrides::default()
            },
            &env(&[]),
        );
        let text = report(&result, Path::new("/x/config.yaml"), Lang::En);
        assert!(text.contains("sk-…6789"), "{text}");
        assert!(!text.contains("sk-inline-123456789"), "{text}");
        assert!(!format!("{:?}", result.unwrap().unwrap()).contains("sk-inline-123456789"));
        let offline = report(&Ok(None), Path::new("c"), Lang::Zh);
        assert!(offline.contains("/setup"));
    }
}
