//! Minimal OpenAI-compatible chat client with tool calling and streaming.
//!
//! One wire format covers DeepSeek, Qwen (DashScope compatible mode), Kimi,
//! OpenAI, Ollama and most gateways. The client is synchronous (`ureq`) and is
//! meant to run on a worker thread; [`LlmClient`] is a trait so tests can
//! inject a scripted model and never touch the network.

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use super::Lang;
use super::config::{DEFAULT_TEMPERATURE, DEFAULT_TIMEOUT_SECS, Overrides, mask_key, resolve};
use super::providers;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Raw JSON arguments, exactly as produced by the model.
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::new(Role::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new(Role::User, content)
    }

    pub fn assistant(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            tool_calls,
            ..Self::new(Role::Assistant, content)
        }
    }

    pub fn tool_result(id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            tool_call_id: Some(id.into()),
            ..Self::new(Role::Tool, content)
        }
    }

    pub fn to_json(&self) -> Value {
        let mut message = json!({ "role": self.role.as_str(), "content": self.content });
        if !self.tool_calls.is_empty() {
            message["tool_calls"] = self
                .tool_calls
                .iter()
                .map(|call| {
                    json!({
                        "id": call.id,
                        "type": "function",
                        "function": { "name": call.name, "arguments": call.arguments },
                    })
                })
                .collect();
        }
        if let Some(id) = &self.tool_call_id {
            message["tool_call_id"] = json!(id);
        }
        message
    }
}

#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

impl ToolSpec {
    fn to_json(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": self.parameters,
            }
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Completion {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("request failed: {0}")]
    Http(String),
    #[error("HTTP {code}: {body}")]
    Status { code: u16, body: String },
    #[error("unexpected response: {0}")]
    Parse(String),
    #[error("cancelled")]
    Cancelled,
}

impl LlmError {
    /// The model or endpoint rejected the `tools` parameter.
    pub fn is_tool_unsupported(&self) -> bool {
        match self {
            Self::Status {
                code: 400 | 404 | 422,
                body,
            } => {
                let body = body.to_lowercase();
                body.contains("tool") || body.contains("function call")
            }
            _ => false,
        }
    }

    /// What the user can do about it.
    pub fn hint(&self, lang: Lang) -> Option<&'static str> {
        let hint = match self {
            _ if self.is_tool_unsupported() => lang.pick(
                "This model does not support tool calling; pick another one with /model list.",
                "该模型不支持工具调用，请用 /model list 换一个模型。",
            ),
            Self::Status {
                code: 401 | 403, ..
            } => lang.pick(
                "The API key is invalid or has no access to this model. Run /setup again.",
                "API Key 无效或没有该模型的权限，请重新运行 /setup。",
            ),
            Self::Status { code: 404, .. } => lang.pick(
                "Wrong model name or base URL. /model list shows the available models.",
                "模型名或 Base URL 不正确，可用 /model list 查看可用模型。",
            ),
            Self::Status {
                code: 402 | 429, ..
            } => lang.pick(
                "Rate limited or out of credit. Wait a moment or check your balance.",
                "触发限流或余额不足，请稍后重试或检查账户余额。",
            ),
            Self::Status { code, .. } if *code >= 500 => lang.pick(
                "The provider had a server error; try again later.",
                "服务商出现服务器错误，请稍后再试。",
            ),
            Self::Http(_) => lang.pick(
                "Network error or timeout. Check your connection, or set `proxy` in config.yaml.",
                "网络错误或超时。请检查网络，或在 config.yaml 中设置 proxy。",
            ),
            _ => return None,
        };
        Some(hint)
    }

    /// Error text followed by the hint, if any.
    pub fn explain(&self, lang: Lang) -> String {
        match self.hint(lang) {
            Some(hint) => format!("{self}\n{hint}"),
            None => self.to_string(),
        }
    }
}

/// A chat model that may call tools.
pub trait LlmClient: Send {
    fn model(&self) -> &str;

    /// Run one completion. Text deltas are reported through `on_delta` as
    /// they arrive; `cancel` is checked between chunks.
    fn complete(
        &mut self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_delta: &mut dyn FnMut(&str),
        cancel: &AtomicBool,
    ) -> Result<Completion, LlmError>;
}

/// Effective connection settings, produced by [`super::config::resolve`].
#[derive(Clone, PartialEq)]
pub struct LlmConfig {
    /// Preset id (`deepseek`, `qwen`, ..., `custom`).
    pub provider: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub stream: bool,
    pub temperature: f64,
    pub timeout_secs: u64,
    pub proxy: Option<String>,
}

/// Never print the key, even in debug output.
impl std::fmt::Debug for LlmConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmConfig")
            .field("provider", &self.provider)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_deref().map(mask_key))
            .field("model", &self.model)
            .field("stream", &self.stream)
            .field("temperature", &self.temperature)
            .field("timeout_secs", &self.timeout_secs)
            .field("proxy", &self.proxy)
            .finish()
    }
}

impl LlmConfig {
    /// A config with default tuning values.
    pub fn new(
        provider: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: Option<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key,
            model: model.into(),
            stream: true,
            temperature: DEFAULT_TEMPERATURE,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            proxy: None,
        }
    }

    /// Only the `CCFDDL_*` variables, ignoring `config.yaml` (kept for
    /// callers that predate the config file).
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Option<Self> {
        resolve(None, &Overrides::default(), &lookup)
            .ok()
            .flatten()
            .map(|resolved| resolved.config)
    }

    pub fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    /// `provider · model`, for titles.
    pub fn label(&self) -> String {
        format!("{} · {}", self.provider, self.model)
    }

    fn agent(&self) -> Result<ureq::Agent, LlmError> {
        let mut builder = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(self.timeout_secs)))
            .timeout_recv_body(Some(Duration::from_secs(self.timeout_secs.max(300))))
            .http_status_as_error(false);
        if let Some(proxy) = &self.proxy {
            let proxy = ureq::Proxy::new(proxy)
                .map_err(|e| LlmError::Http(format!("invalid proxy {proxy}: {e}")))?;
            builder = builder.proxy(Some(proxy));
        }
        Ok(builder.build().into())
    }
}

/// `GET {base_url}/models`, for `/setup` and `/model list`.
pub fn list_models(config: &LlmConfig) -> Result<Vec<String>, LlmError> {
    let agent = config.agent()?;
    let mut request = agent.get(&format!("{}/models", config.base_url));
    if let Some(key) = &config.api_key {
        request = request.header("Authorization", &format!("Bearer {key}"));
    }
    let response = request.call().map_err(|e| LlmError::Http(e.to_string()))?;
    let status = response.status().as_u16();
    let text = response
        .into_body()
        .read_to_string()
        .map_err(|e| LlmError::Http(e.to_string()))?;
    if !(200..300).contains(&status) {
        return Err(LlmError::Status {
            code: status,
            body: text.chars().take(300).collect(),
        });
    }
    providers::parse_models(&text).map_err(LlmError::Parse)
}

/// A tiny request that exercises the key, the model and tool calling.
pub fn ping(client: &mut dyn LlmClient) -> Result<(), LlmError> {
    let tools = [ToolSpec {
        name: "noop",
        description: "Does nothing.",
        parameters: json!({ "type": "object", "properties": {} }),
    }];
    let messages = [
        ChatMessage::system("Connection test. Reply with the single word OK."),
        ChatMessage::user("ping"),
    ];
    client
        .complete(&messages, &tools, &mut |_| {}, &AtomicBool::new(false))
        .map(|_| ())
}

pub struct OpenAiClient {
    config: LlmConfig,
    agent: Result<ureq::Agent, String>,
}

impl OpenAiClient {
    pub fn new(config: LlmConfig) -> Self {
        let agent = config.agent().map_err(|e| e.to_string());
        Self { config, agent }
    }
}

pub fn request_body(
    model: &str,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
    stream: bool,
    temperature: f64,
) -> Value {
    let mut body = json!({
        "model": model,
        "messages": messages.iter().map(ChatMessage::to_json).collect::<Vec<_>>(),
        "temperature": temperature,
        "stream": stream,
    });
    if !tools.is_empty() {
        body["tools"] = tools.iter().map(ToolSpec::to_json).collect();
        body["tool_choice"] = json!("auto");
    }
    body
}

impl LlmClient for OpenAiClient {
    fn model(&self) -> &str {
        &self.config.model
    }

    fn complete(
        &mut self,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_delta: &mut dyn FnMut(&str),
        cancel: &AtomicBool,
    ) -> Result<Completion, LlmError> {
        let body = request_body(
            &self.config.model,
            messages,
            tools,
            self.config.stream,
            self.config.temperature,
        );
        let agent = self.agent.as_ref().map_err(|e| LlmError::Http(e.clone()))?;
        let mut request = agent.post(&self.config.endpoint()).header(
            "Accept",
            if self.config.stream {
                "text/event-stream"
            } else {
                "application/json"
            },
        );
        if let Some(key) = &self.config.api_key {
            request = request.header("Authorization", &format!("Bearer {key}"));
        }
        let response = request
            .content_type("application/json")
            .send(body.to_string())
            .map_err(|e| LlmError::Http(e.to_string()))?;
        let status = response.status().as_u16();
        let mut reader = BufReader::new(response.into_body().into_reader());
        if !(200..300).contains(&status) {
            let mut text = String::new();
            let _ = std::io::Read::read_to_string(&mut reader, &mut text);
            return Err(LlmError::Status {
                code: status,
                body: text.chars().take(500).collect(),
            });
        }
        if self.config.stream {
            parse_sse(reader, on_delta, cancel)
        } else {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut reader, &mut text)
                .map_err(|e| LlmError::Http(e.to_string()))?;
            let completion = parse_completion(&text)?;
            if !completion.content.is_empty() {
                on_delta(&completion.content);
            }
            Ok(completion)
        }
    }
}

fn tool_call_from(value: &Value) -> Option<ToolCall> {
    let function = value.get("function")?;
    Some(ToolCall {
        id: value
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        name: function.get("name").and_then(Value::as_str)?.to_string(),
        arguments: match function.get("arguments") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => "{}".to_string(),
        },
    })
}

/// Parse a non-streaming `chat/completions` response.
pub fn parse_completion(text: &str) -> Result<Completion, LlmError> {
    let value: Value = serde_json::from_str(text).map_err(|e| LlmError::Parse(e.to_string()))?;
    let message = value
        .pointer("/choices/0/message")
        .ok_or_else(|| LlmError::Parse(text.chars().take(200).collect()))?;
    let mut completion = Completion {
        content: message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        tool_calls: message
            .get("tool_calls")
            .and_then(Value::as_array)
            .map(|calls| calls.iter().filter_map(tool_call_from).collect())
            .unwrap_or_default(),
    };
    fill_missing_ids(&mut completion);
    Ok(completion)
}

fn fill_missing_ids(completion: &mut Completion) {
    for (index, call) in completion.tool_calls.iter_mut().enumerate() {
        if call.id.is_empty() {
            call.id = format!("call_{index}");
        }
        if call.arguments.trim().is_empty() {
            call.arguments = "{}".to_string();
        }
    }
}

/// Parse a server-sent-event stream, forwarding text deltas as they arrive
/// and assembling tool calls that are split across chunks.
pub fn parse_sse(
    reader: impl BufRead,
    on_delta: &mut dyn FnMut(&str),
    cancel: &AtomicBool,
) -> Result<Completion, LlmError> {
    let mut completion = Completion::default();
    // Partial tool calls keyed by their `index`.
    let mut partial: Vec<(String, String, String)> = Vec::new();
    for line in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            return Err(LlmError::Cancelled);
        }
        let line = line.map_err(|e| LlmError::Http(e.to_string()))?;
        let Some(data) = line.trim().strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" {
            break;
        }
        let Ok(chunk) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if let Some(error) = chunk.get("error") {
            return Err(LlmError::Parse(error.to_string()));
        }
        let Some(delta) = chunk.pointer("/choices/0/delta") else {
            continue;
        };
        if let Some(text) = delta.get("content").and_then(Value::as_str)
            && !text.is_empty()
        {
            completion.content.push_str(text);
            on_delta(text);
        }
        for call in delta
            .get("tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            if partial.len() <= index {
                partial.resize(index + 1, Default::default());
            }
            let slot = &mut partial[index];
            if let Some(id) = call.get("id").and_then(Value::as_str) {
                slot.0.push_str(id);
            }
            if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                slot.1.push_str(name);
            }
            if let Some(args) = call.pointer("/function/arguments").and_then(Value::as_str) {
                slot.2.push_str(args);
            }
        }
    }
    completion.tool_calls = partial
        .into_iter()
        .filter(|(_, name, _)| !name.is_empty())
        .map(|(id, name, arguments)| ToolCall {
            id,
            name,
            arguments,
        })
        .collect();
    fill_missing_ids(&mut completion);
    Ok(completion)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A model that replays canned completions and records what it was sent.
    pub(crate) struct ScriptedClient {
        pub replies: VecDeque<Completion>,
        pub seen: std::sync::Arc<std::sync::Mutex<Vec<Vec<ChatMessage>>>>,
    }

    impl ScriptedClient {
        pub(crate) fn new(replies: Vec<Completion>) -> Self {
            Self {
                replies: replies.into(),
                seen: Default::default(),
            }
        }
    }

    impl LlmClient for ScriptedClient {
        fn model(&self) -> &str {
            "scripted"
        }

        fn complete(
            &mut self,
            messages: &[ChatMessage],
            _tools: &[ToolSpec],
            on_delta: &mut dyn FnMut(&str),
            cancel: &AtomicBool,
        ) -> Result<Completion, LlmError> {
            if cancel.load(Ordering::Relaxed) {
                return Err(LlmError::Cancelled);
            }
            self.seen.lock().unwrap().push(messages.to_vec());
            let reply = self
                .replies
                .pop_front()
                .ok_or_else(|| LlmError::Parse("script exhausted".into()))?;
            // Stream the text in two pieces to exercise delta handling.
            let mid = reply
                .content
                .char_indices()
                .nth(reply.content.chars().count() / 2)
                .map_or(0, |(i, _)| i);
            for piece in [&reply.content[..mid], &reply.content[mid..]] {
                if !piece.is_empty() {
                    on_delta(piece);
                }
            }
            Ok(reply)
        }
    }

    pub(crate) fn call(name: &str, args: Value) -> ToolCall {
        ToolCall {
            id: format!("id_{name}"),
            name: name.to_string(),
            arguments: args.to_string(),
        }
    }

    #[test]
    fn env_only_config_still_works() {
        assert_eq!(LlmConfig::from_lookup(|_| None), None);
        let cfg =
            LlmConfig::from_lookup(|k| (k == "CCFDDL_API_KEY").then(|| "sk-123456789".into()))
                .unwrap();
        assert_eq!(cfg.provider, "deepseek");
        assert_eq!(cfg.endpoint(), "https://api.deepseek.com/chat/completions");
        assert!(cfg.stream);
        assert!(!format!("{cfg:?}").contains("sk-123456789"));
        assert_eq!(cfg.label(), "deepseek · deepseek-flash");
    }

    #[test]
    fn errors_carry_actionable_hints() {
        let status = |code: u16, body: &str| LlmError::Status {
            code,
            body: body.into(),
        };
        assert!(status(401, "").explain(Lang::En).contains("/setup"));
        assert!(status(404, "").explain(Lang::Zh).contains("/model list"));
        assert!(status(429, "").hint(Lang::En).is_some());
        assert!(status(400, "this model does not support tools").is_tool_unsupported());
        assert!(!status(400, "bad request").is_tool_unsupported());
        assert!(
            LlmError::Http("timeout".into())
                .explain(Lang::En)
                .contains("proxy")
        );
        assert_eq!(LlmError::Cancelled.hint(Lang::En), None);
    }

    #[test]
    fn invalid_proxy_is_reported_on_use() {
        let mut config = LlmConfig::new("custom", "http://localhost:1/v1", "m", None);
        config.proxy = Some("::not a proxy::".into());
        let mut client = OpenAiClient::new(config);
        let result = client.complete(&[], &[], &mut |_| {}, &AtomicBool::new(false));
        assert!(matches!(result, Err(LlmError::Http(message)) if message.contains("proxy")));
    }

    #[test]
    fn ping_sends_a_tool_and_reports_failures() {
        let mut ok = ScriptedClient::new(vec![Completion::default()]);
        assert!(ping(&mut ok).is_ok());
        let mut failing = ScriptedClient::new(vec![]);
        assert!(ping(&mut failing).is_err());
    }

    #[test]
    fn messages_serialize_to_the_openai_shape() {
        let assistant = ChatMessage::assistant("", vec![call("get_profile", json!({}))]);
        let value = assistant.to_json();
        assert_eq!(value["tool_calls"][0]["function"]["name"], "get_profile");
        let tool = ChatMessage::tool_result("id_x", "{}").to_json();
        assert_eq!(tool["role"], "tool");
        assert_eq!(tool["tool_call_id"], "id_x");
        let body = request_body("m", &[ChatMessage::user("hi")], &[], true, 0.5);
        assert!(body.get("tools").is_none());
        assert_eq!(body["temperature"], 0.5);
    }

    #[test]
    fn parses_a_plain_completion() {
        let text = r#"{"choices":[{"message":{"content":"hi","tool_calls":[{"function":{"name":"get_profile","arguments":""}}]}}]}"#;
        let completion = parse_completion(text).unwrap();
        assert_eq!(completion.content, "hi");
        assert_eq!(completion.tool_calls[0].id, "call_0");
        assert_eq!(completion.tool_calls[0].arguments, "{}");
        assert!(parse_completion("{}").is_err());
    }

    #[test]
    fn streams_text_and_assembles_split_tool_calls() {
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
            ": keep-alive\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"search_conferences\",\"arguments\":\"{\\\"query\\\"\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\":\\\"cvpr\\\"}\"}}]}}]}\n\n",
            "data: [DONE]\n\n",
        );
        let mut seen = String::new();
        let completion = parse_sse(
            stream.as_bytes(),
            &mut |delta| seen.push_str(delta),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(seen, "Hello");
        assert_eq!(completion.content, "Hello");
        assert_eq!(completion.tool_calls.len(), 1);
        assert_eq!(completion.tool_calls[0].id, "c1");
        assert_eq!(completion.tool_calls[0].arguments, "{\"query\":\"cvpr\"}");
    }

    #[test]
    fn stream_stops_when_cancelled() {
        let stream = "data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n";
        let result = parse_sse(stream.as_bytes(), &mut |_| {}, &AtomicBool::new(true));
        assert!(matches!(result, Err(LlmError::Cancelled)));
    }

    #[test]
    fn stream_errors_are_reported() {
        let stream = "data: {\"error\":{\"message\":\"bad key\"}}\n";
        let result = parse_sse(stream.as_bytes(), &mut |_| {}, &AtomicBool::new(false));
        assert!(matches!(result, Err(LlmError::Parse(message)) if message.contains("bad key")));
    }
}
