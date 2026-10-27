//! `ccfddl-tui --setup` and `--check-llm`: the same model setup as the TUI's
//! `/setup`, as plain terminal prompts. Input, output, the hidden key prompt
//! and all network calls are injected so the flow is testable offline.

use std::io::{self, BufRead, Write};
use std::path::Path;

use super::Lang;
use super::config::{LlmSettings, Overrides, ProfileEntry, load_and_resolve, report};
use super::llm::{LlmClient, LlmConfig, LlmError, ping};
use super::providers::{self, PRESETS};

/// Reads a line without echoing it, writing any feedback to the output.
pub type SecretReader<'a> = dyn FnMut(&mut dyn Write) -> io::Result<String> + 'a;

/// Everything the prompts need from the outside world.
pub struct Io<'a> {
    pub input: &'a mut dyn BufRead,
    pub out: &'a mut dyn Write,
    /// Read a secret without echoing it; `None` reads a plain line from
    /// `input` (piped stdin).
    pub read_secret: Option<&'a mut SecretReader<'a>>,
    pub env: &'a dyn Fn(&str) -> Option<String>,
    pub fetch_models: &'a dyn Fn(&LlmConfig) -> Result<Vec<String>, LlmError>,
    pub make_client: &'a dyn Fn(&LlmConfig) -> Box<dyn LlmClient>,
}

fn ask(io: &mut Io, prompt: &str) -> io::Result<String> {
    write!(io.out, "{prompt} ")?;
    io.out.flush()?;
    let mut line = String::new();
    if io.input.read_line(&mut line)? == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "input closed"));
    }
    Ok(line.trim().to_string())
}

/// Interactive setup. Returns the saved profile name.
pub fn run_setup(io: &mut Io, config_path: &Path, lang: Lang) -> anyhow::Result<String> {
    let mut settings = LlmSettings::load(config_path)?.unwrap_or_default();
    let detected = providers::detect_env_keys(io.env);
    let default = detected
        .first()
        .map(|(p, _)| *p)
        .or_else(|| providers::preset(providers::DEFAULT_PROVIDER))
        .unwrap_or(&PRESETS[0]);

    writeln!(io.out, "{}", lang.pick("Providers:", "可选厂商："))?;
    for (index, preset) in PRESETS.iter().enumerate() {
        let found = detected
            .iter()
            .find(|(p, _)| p.id == preset.id)
            .map(|(_, var)| format!("  ✓ {var}"))
            .unwrap_or_default();
        writeln!(
            io.out,
            "{:>4}. {}{found}",
            index + 1,
            lang.pick(preset.name, preset.name_zh)
        )?;
    }

    // 1. Provider
    let preset = loop {
        let answer = ask(
            io,
            &match lang {
                Lang::Zh => format!("[1/5] 选择厂商（回车默认 {}）:", default.name_zh),
                Lang::En => format!("[1/5] Provider (Enter for {}):", default.name),
            },
        )?;
        if answer.is_empty() {
            break default;
        }
        match providers::pick(&answer) {
            Some(preset) => break preset,
            None => writeln!(io.out, "{}", lang.pick("Unknown provider.", "未知厂商。"))?,
        }
    };
    let mut base_url = preset.base_url.to_string();
    if preset.id == "custom" {
        base_url = loop {
            let url = ask(io, lang.pick("Base URL:", "Base URL："))?;
            if url.starts_with("http://") || url.starts_with("https://") {
                break url.trim_end_matches('/').to_string();
            }
            writeln!(
                io.out,
                "{}",
                lang.pick("Must start with http(s)://", "需以 http(s):// 开头")
            )?;
        };
    }

    // 2. Key
    let mut key: Option<String> = None;
    let mut key_env: Option<String> = None;
    let mut save_key = false;
    if preset.needs_key || preset.id == "custom" {
        let found = preset.key_env.filter(|var| (io.env)(var).is_some());
        let prompt = match (found, lang) {
            (Some(var), Lang::Zh) => {
                format!("[2/5] API Key（输入隐藏；回车使用 {var}；或 env:变量名）:")
            }
            (Some(var), Lang::En) => {
                format!("[2/5] API key (hidden; Enter uses {var}; or env:VAR):")
            }
            (None, Lang::Zh) => "[2/5] API Key（输入隐藏；或 env:变量名）:".to_string(),
            (None, Lang::En) => "[2/5] API key (hidden; or env:VAR):".to_string(),
        };
        loop {
            let answer = match io.read_secret.as_mut() {
                Some(read_secret) => {
                    write!(io.out, "{prompt} ")?;
                    io.out.flush()?;
                    read_secret(io.out)?.trim().to_string()
                }
                None => ask(io, &prompt)?,
            };
            if let Some(var) = answer.strip_prefix("env:") {
                if (io.env)(var.trim()).is_some() {
                    key_env = Some(var.trim().to_string());
                    break;
                }
                writeln!(io.out, "{} {var}", lang.pick("Not set:", "未设置："))?;
            } else if answer.is_empty() {
                if let Some(var) = found {
                    key_env = Some(var.to_string());
                    break;
                }
                if !preset.needs_key {
                    break;
                }
                writeln!(
                    io.out,
                    "{}",
                    lang.pick("A key is required.", "需要 API Key。")
                )?;
            } else {
                key = Some(answer);
                let choice = ask(
                    io,
                    lang.pick(
                        "Save the key in config.yaml (plain text, owner-only)? [y/N]",
                        "把 Key 明文写入 config.yaml（仅本人可读）吗？[y/N]",
                    ),
                )?;
                save_key = matches!(choice.to_lowercase().as_str(), "y" | "yes" | "是");
                break;
            }
        }
    }
    let api_key = key.clone().or_else(|| key_env.as_deref().and_then(io.env));
    let mut config = LlmConfig::new(preset.id, base_url.clone(), "", api_key);

    // 3. Model
    writeln!(
        io.out,
        "{}",
        lang.pick("Fetching models…", "正在获取模型列表…")
    )?;
    let models = match (io.fetch_models)(&config) {
        Ok(models) => models,
        Err(error) => {
            writeln!(io.out, "{}", error.explain(lang))?;
            Vec::new()
        }
    };
    for (index, model) in models.iter().take(40).enumerate() {
        writeln!(io.out, "{:>4}. {model}", index + 1)?;
    }
    let default_model = preset
        .default_model
        .map(str::to_string)
        .filter(|m| models.is_empty() || models.contains(m))
        .or_else(|| models.first().cloned());
    config.model = loop {
        let prompt = match (&default_model, lang) {
            (Some(m), Lang::Zh) => format!("[3/5] 模型（编号或名称，回车默认 {m}）:"),
            (Some(m), Lang::En) => format!("[3/5] Model (number or name, Enter for {m}):"),
            (None, Lang::Zh) => "[3/5] 模型名称:".to_string(),
            (None, Lang::En) => "[3/5] Model name:".to_string(),
        };
        let answer = ask(io, &prompt)?;
        let choice = if answer.is_empty() {
            default_model.clone()
        } else if let Ok(n) = answer.parse::<usize>() {
            n.checked_sub(1).and_then(|i| models.get(i)).cloned()
        } else {
            Some(answer)
        };
        if let Some(model) = choice {
            break model;
        }
    };

    // 4. Test
    writeln!(
        io.out,
        "{} {} …",
        lang.pick("[4/5] Testing", "[4/5] 正在测试"),
        config.label()
    )?;
    let mut client = (io.make_client)(&config);
    match ping(client.as_mut()) {
        Ok(()) => writeln!(io.out, "{}", lang.pick("✓ Connected.", "✓ 连接成功。"))?,
        Err(error) => {
            writeln!(io.out, "{}", error.explain(lang))?;
            let again = ask(io, lang.pick("Save anyway? [y/N]", "仍然保存吗？[y/N]"))?;
            if !matches!(again.to_lowercase().as_str(), "y" | "yes" | "是") {
                anyhow::bail!("{}", lang.pick("setup cancelled", "已取消配置"));
            }
        }
    }

    // 5. Save
    let name = ask(
        io,
        &match lang {
            Lang::Zh => format!("[5/5] 配置名称（回车默认 {}）:", preset.id),
            Lang::En => format!("[5/5] Name (Enter for {}):", preset.id),
        },
    )?;
    let name = if name.is_empty() {
        preset.id.to_string()
    } else {
        name
    };
    settings.set_profile(
        &name,
        ProfileEntry {
            provider: Some(preset.id.to_string()),
            model: Some(config.model.clone()),
            base_url: (base_url != preset.base_url).then_some(base_url),
            api_key: if save_key { key.clone() } else { None },
            api_key_env: key_env,
            ..ProfileEntry::default()
        },
        true,
    );
    settings.save(config_path)?;
    writeln!(
        io.out,
        "✓ {} {}",
        lang.pick("Saved to", "已保存到"),
        config_path.display()
    )?;
    if key.is_some() && !save_key {
        let var = preset.key_env.unwrap_or("CCFDDL_API_KEY");
        writeln!(
            io.out,
            "{}",
            match lang {
                Lang::Zh => format!("Key 未写入文件，请设置环境变量 {var} 后使用。"),
                Lang::En => format!("The key was not saved; set {var} before using the agent."),
            }
        )?;
    }
    Ok(name)
}

/// `--check-llm`: effective settings, model list and a test request.
/// Returns `false` when something is broken.
pub fn check(
    io: &mut Io,
    config_path: &Path,
    overrides: &Overrides,
    lang: Lang,
) -> io::Result<bool> {
    let result = load_and_resolve(config_path, overrides, io.env);
    writeln!(io.out, "{}", report(&result, config_path, lang))?;
    let config = match result {
        Ok(Some(resolved)) => resolved.config,
        Ok(None) => return Ok(true),
        Err(_) => return Ok(false),
    };
    match (io.fetch_models)(&config) {
        Ok(models) => {
            let listed = models.contains(&config.model);
            writeln!(
                io.out,
                "{}: {} ({})",
                lang.pick("Models", "模型列表"),
                models.len(),
                match (listed, lang) {
                    (true, Lang::Zh) => "包含当前模型",
                    (true, Lang::En) => "includes the configured model",
                    (false, Lang::Zh) => "未包含当前模型，请确认模型名",
                    (false, Lang::En) => "configured model not listed; check the name",
                }
            )?;
        }
        Err(error) => writeln!(
            io.out,
            "{}: {}",
            lang.pick("Models", "模型列表"),
            error.explain(lang)
        )?,
    }
    let mut client = (io.make_client)(&config);
    match ping(client.as_mut()) {
        Ok(()) => {
            writeln!(
                io.out,
                "{}",
                lang.pick("✓ Test request succeeded.", "✓ 测试请求成功。")
            )?;
            Ok(true)
        }
        Err(error) => {
            writeln!(io.out, "✗ {}", error.explain(lang))?;
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::llm::Completion;
    use crate::agent::llm::tests::ScriptedClient;

    struct Harness {
        input: io::Cursor<Vec<u8>>,
        out: Vec<u8>,
        secrets: Vec<String>,
    }

    impl Harness {
        fn new(lines: &[&str], secrets: &[&str]) -> Self {
            Self {
                input: io::Cursor::new(format!("{}\n", lines.join("\n")).into_bytes()),
                out: Vec::new(),
                secrets: secrets.iter().rev().map(|s| s.to_string()).collect(),
            }
        }
    }

    fn run(
        harness: &mut Harness,
        env: &dyn Fn(&str) -> Option<String>,
        ping_ok: bool,
        path: &Path,
    ) -> anyhow::Result<String> {
        let mut secrets = std::mem::take(&mut harness.secrets);
        let mut read_secret = |_: &mut dyn Write| Ok(secrets.pop().unwrap_or_default());
        let fetch = |_: &LlmConfig| Ok(vec!["m1".to_string(), "m2".to_string()]);
        let make = move |_: &LlmConfig| -> Box<dyn LlmClient> {
            Box::new(ScriptedClient::new(if ping_ok {
                vec![Completion::default()]
            } else {
                vec![]
            }))
        };
        let mut io = Io {
            input: &mut harness.input,
            out: &mut harness.out,
            read_secret: Some(&mut read_secret),
            env,
            fetch_models: &fetch,
            make_client: &make,
        };
        run_setup(&mut io, path, Lang::En)
    }

    #[test]
    fn terminal_setup_saves_a_profile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let mut harness = Harness::new(&["2", "", "2", "work"], &["sk-secret-123456"]);
        let name = run(&mut harness, &|_| None, true, &path).unwrap();
        assert_eq!(name, "work");
        let out = String::from_utf8(harness.out).unwrap();
        assert!(!out.contains("sk-secret"), "{out}");
        assert!(out.contains("DASHSCOPE_API_KEY"), "{out}");
        let settings = LlmSettings::load(&path).unwrap().unwrap();
        assert_eq!(settings.active.as_deref(), Some("work"));
        assert_eq!(settings.profiles["work"].model.as_deref(), Some("m2"));
        assert_eq!(settings.profiles["work"].api_key, None);
    }

    #[test]
    fn detected_variable_is_used_on_enter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let env = |k: &str| (k == "DEEPSEEK_API_KEY").then(|| "sk-ds".to_string());
        let mut harness = Harness::new(&["", "1", ""], &[""]);
        run(&mut harness, &env, true, &path).unwrap();
        let settings = LlmSettings::load(&path).unwrap().unwrap();
        let entry = &settings.profiles["deepseek"];
        assert_eq!(entry.api_key_env.as_deref(), Some("DEEPSEEK_API_KEY"));
        assert_eq!(entry.model.as_deref(), Some("m1"));
    }

    #[test]
    fn failed_test_can_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let mut harness = Harness::new(&["ollama", "m1", "n"], &[]);
        assert!(run(&mut harness, &|_| None, false, &path).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn check_reports_offline_and_failures() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let mut input = io::Cursor::new(Vec::new());
        let mut out = Vec::new();
        let mut read_secret = |_: &mut dyn Write| Ok(String::new());
        let fetch = |_: &LlmConfig| Ok(vec!["llama".to_string()]);
        let make = |_: &LlmConfig| -> Box<dyn LlmClient> { Box::new(ScriptedClient::new(vec![])) };
        let mut io = Io {
            input: &mut input,
            out: &mut out,
            read_secret: Some(&mut read_secret),
            env: &|_| None,
            fetch_models: &fetch,
            make_client: &make,
        };
        assert!(check(&mut io, &path, &Overrides::default(), Lang::En).unwrap());
        let ollama = Overrides {
            provider: Some("ollama".into()),
            model: Some("llama".into()),
            ..Overrides::default()
        };
        assert!(!check(&mut io, &path, &ollama, Lang::En).unwrap());
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("offline"), "{text}");
        assert!(text.contains("includes the configured model"), "{text}");
    }
}
