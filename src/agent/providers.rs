//! Built-in presets for OpenAI-compatible providers.
//!
//! Model names change quickly, so a preset only carries a default model when
//! the provider's documentation lists a current one; `/setup` and
//! `/model list` fetch the live list from `GET {base_url}/models` instead.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub name_zh: &'static str,
    pub base_url: &'static str,
    /// Environment variable the provider's own SDKs use for the key.
    pub key_env: Option<&'static str>,
    pub default_model: Option<&'static str>,
    pub needs_key: bool,
}

pub const DEFAULT_PROVIDER: &str = "deepseek";

pub const PRESETS: &[Preset] = &[
    Preset {
        id: "deepseek",
        name: "DeepSeek",
        name_zh: "DeepSeek 深度求索",
        base_url: "https://api.deepseek.com",
        key_env: Some("DEEPSEEK_API_KEY"),
        default_model: Some("deepseek-flash"),
        needs_key: true,
    },
    Preset {
        id: "qwen",
        name: "Qwen (DashScope)",
        name_zh: "通义千问（阿里云百炼）",
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        key_env: Some("DASHSCOPE_API_KEY"),
        default_model: Some("qwen3.8-flash"),
        needs_key: true,
    },
    Preset {
        id: "kimi",
        name: "Kimi (Moonshot)",
        name_zh: "Kimi 月之暗面",
        base_url: "https://api.moonshot.cn/v1",
        key_env: Some("MOONSHOT_API_KEY"),
        default_model: Some("kimi-k2.6"),
        needs_key: true,
    },
    Preset {
        id: "zhipu",
        name: "Zhipu GLM",
        name_zh: "智谱 GLM",
        base_url: "https://open.bigmodel.cn/api/paas/v4",
        key_env: Some("ZHIPUAI_API_KEY"),
        default_model: Some("glm-5.2"),
        needs_key: true,
    },
    Preset {
        id: "siliconflow",
        name: "SiliconFlow",
        name_zh: "硅基流动",
        base_url: "https://api.siliconflow.cn/v1",
        key_env: Some("SILICONFLOW_API_KEY"),
        default_model: Some("deepseek-ai/DeepSeek-V3.2"),
        needs_key: true,
    },
    Preset {
        id: "openrouter",
        name: "OpenRouter",
        name_zh: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        key_env: Some("OPENROUTER_API_KEY"),
        default_model: None,
        needs_key: true,
    },
    Preset {
        id: "openai",
        name: "OpenAI",
        name_zh: "OpenAI",
        base_url: "https://api.openai.com/v1",
        key_env: Some("OPENAI_API_KEY"),
        default_model: None,
        needs_key: true,
    },
    Preset {
        id: "ollama",
        name: "Ollama (local)",
        name_zh: "Ollama（本地）",
        base_url: "http://localhost:11434/v1",
        key_env: None,
        default_model: None,
        needs_key: false,
    },
    Preset {
        id: "custom",
        name: "Custom OpenAI-compatible",
        name_zh: "自定义 OpenAI 兼容接口",
        base_url: "",
        key_env: None,
        default_model: None,
        needs_key: false,
    },
];

pub fn preset(id: &str) -> Option<&'static Preset> {
    let id = id.trim();
    PRESETS.iter().find(|p| p.id.eq_ignore_ascii_case(id))
}

/// Match a `/setup` answer: a 1-based number, an id or a display name.
pub fn pick(answer: &str) -> Option<&'static Preset> {
    let answer = answer.trim();
    if let Ok(n) = answer.parse::<usize>() {
        return n.checked_sub(1).and_then(|i| PRESETS.get(i));
    }
    preset(answer).or_else(|| {
        let lower = answer.to_lowercase();
        PRESETS
            .iter()
            .find(|p| p.name.to_lowercase().contains(&lower) || p.name_zh.contains(answer))
    })
}

/// Presets whose standard key variable is set, e.g. `DEEPSEEK_API_KEY`.
pub fn detect_env_keys(
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Vec<(&'static Preset, &'static str)> {
    PRESETS
        .iter()
        .filter_map(|p| {
            let var = p.key_env?;
            lookup(var)
                .filter(|v| !v.trim().is_empty())
                .map(|_| (p, var))
        })
        .collect()
}

/// `true` for URLs of a server on this machine (no key needed).
pub fn is_local_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.contains("://localhost") || lower.contains("://127.0.0.1") || lower.contains("://[::1]")
}

/// Parse an OpenAI-style `GET /models` response into sorted model ids.
pub fn parse_models(text: &str) -> Result<Vec<String>, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let list = value
        .get("data")
        .or_else(|| value.get("models"))
        .and_then(Value::as_array)
        .ok_or_else(|| "response has no model list".to_string())?;
    let mut models: Vec<String> = list
        .iter()
        .filter_map(|m| {
            m.get("id")
                .or_else(|| m.get("name"))
                .or_else(|| m.get("model"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    models.sort();
    models.dedup();
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_unique_and_deepseek_is_default() {
        let mut ids: Vec<&str> = PRESETS.iter().map(|p| p.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), PRESETS.len());
        assert_eq!(preset(DEFAULT_PROVIDER).unwrap().name, "DeepSeek");
        assert_eq!(PRESETS[0].id, DEFAULT_PROVIDER);
        for p in PRESETS {
            assert!(
                p.id == "custom" || p.base_url.starts_with("http"),
                "{}",
                p.id
            );
            assert_eq!(p.needs_key, p.key_env.is_some(), "{}", p.id);
        }
    }

    #[test]
    fn pick_accepts_numbers_ids_and_names() {
        assert_eq!(pick("1").unwrap().id, "deepseek");
        assert_eq!(pick("QWEN").unwrap().id, "qwen");
        assert_eq!(pick("智谱").unwrap().id, "zhipu");
        assert_eq!(pick("moonshot").unwrap().id, "kimi");
        assert!(pick("0").is_none());
        assert!(pick("99").is_none());
        assert!(pick("nope").is_none());
    }

    #[test]
    fn detects_standard_key_variables() {
        let found = detect_env_keys(&|k| (k == "DASHSCOPE_API_KEY").then(|| "sk".to_string()));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0.id, "qwen");
        assert!(detect_env_keys(&|_| Some(" ".into())).is_empty());
    }

    #[test]
    fn parses_model_lists() {
        let text = r#"{"object":"list","data":[{"id":"b"},{"id":"a"},{"id":"a"},{"x":1}]}"#;
        assert_eq!(parse_models(text).unwrap(), ["a", "b"]);
        // Ollama's native shape.
        assert_eq!(
            parse_models(r#"{"models":[{"name":"qwen2.5"}]}"#).unwrap(),
            ["qwen2.5"]
        );
        assert!(parse_models("{}").is_err());
        assert!(parse_models("nope").is_err());
    }

    #[test]
    fn local_urls_are_recognised() {
        assert!(is_local_url("http://localhost:11434/v1"));
        assert!(is_local_url("http://127.0.0.1:8000"));
        assert!(!is_local_url("https://api.deepseek.com"));
    }
}
