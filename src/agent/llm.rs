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

pub const DEFAULT_BASE_URL: &str = "https://api.deepseek.com/v1";
pub const DEFAULT_MODEL: &str = "deepseek-chat";

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

/// Connection settings, read from `CCFDDL_*` environment variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub stream: bool,
}

impl LlmConfig {
    /// `CCFDDL_API_KEY`, `CCFDDL_BASE_URL`, `CCFDDL_MODEL`, `CCFDDL_STREAM`.
    /// Returns `None` (offline mode) without an API key, unless the base URL
    /// points at a local server such as Ollama.
    pub fn from_env() -> Option<Self> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let get = |key: &str| {
            lookup(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let api_key = get("CCFDDL_API_KEY");
        let base_url = get("CCFDDL_BASE_URL");
        let is_local = base_url
            .as_deref()
            .is_some_and(|url| url.contains("localhost") || url.contains("127.0.0.1"));
        if api_key.is_none() && !is_local {
            return None;
        }
        Some(Self {
            base_url: base_url
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
                .trim_end_matches('/')
                .to_string(),
            api_key,
            model: get("CCFDDL_MODEL").unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            stream: get("CCFDDL_STREAM").is_none_or(|v| v != "0" && v != "false"),
        })
    }

    pub fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }
}

pub struct OpenAiClient {
    config: LlmConfig,
    agent: ureq::Agent,
}

impl OpenAiClient {
    pub fn new(config: LlmConfig) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(120)))
            .timeout_recv_body(Some(Duration::from_secs(300)))
            .http_status_as_error(false)
            .build()
            .into();
        Self { config, agent }
    }
}

pub fn request_body(
    model: &str,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
    stream: bool,
) -> Value {
    let mut body = json!({
        "model": model,
        "messages": messages.iter().map(ChatMessage::to_json).collect::<Vec<_>>(),
        "temperature": 0.3,
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
        let body = request_body(&self.config.model, messages, tools, self.config.stream);
        let mut request = self.agent.post(&self.config.endpoint()).header(
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
    fn config_requires_a_key_unless_local() {
        assert_eq!(LlmConfig::from_lookup(|_| None), None);
        let cfg = LlmConfig::from_lookup(|k| (k == "CCFDDL_API_KEY").then(|| "sk".into())).unwrap();
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.model, DEFAULT_MODEL);
        assert!(cfg.stream);
        let local = LlmConfig::from_lookup(|k| match k {
            "CCFDDL_BASE_URL" => Some("http://localhost:11434/v1/".into()),
            "CCFDDL_MODEL" => Some("qwen2.5".into()),
            "CCFDDL_STREAM" => Some("0".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(
            local.endpoint(),
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(local.api_key, None);
        assert!(!local.stream);
    }

    #[test]
    fn messages_serialize_to_the_openai_shape() {
        let assistant = ChatMessage::assistant("", vec![call("get_profile", json!({}))]);
        let value = assistant.to_json();
        assert_eq!(value["tool_calls"][0]["function"]["name"], "get_profile");
        let tool = ChatMessage::tool_result("id_x", "{}").to_json();
        assert_eq!(tool["role"], "tool");
        assert_eq!(tool["tool_call_id"], "id_x");
        let body = request_body("m", &[ChatMessage::user("hi")], &[], true);
        assert!(body.get("tools").is_none());
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
