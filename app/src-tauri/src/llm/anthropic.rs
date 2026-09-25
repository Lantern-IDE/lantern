//! Anthropic Messages API (`POST /v1/messages`, SSE 스트리밍, 도구 사용).
//! Rust용 공식 SDK가 없어 HTTP로 직접 호출한다.

use super::sse::SseParser;
use super::{cancelled, http_error, Block, ChatRequest, ChatResponse, Role, StopReason, StreamEvent, Usage};
use crate::config::ModelConfig;
use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const DEFAULT_BASE: &str = "https://api.anthropic.com";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

pub fn build_body(m: &ModelConfig, req: &ChatRequest<'_>) -> Value {
    let messages: Vec<Value> = req
        .messages
        .iter()
        .map(|msg| {
            let content: Vec<Value> = msg
                .blocks
                .iter()
                .map(|b| match b {
                    Block::Text { text } => json!({ "type": "text", "text": text }),
                    Block::ToolUse { id, name, input, .. } => {
                        json!({ "type": "tool_use", "id": id, "name": name, "input": input })
                    }
                    Block::ToolResult { tool_use_id, content, is_error } => json!({
                        "type": "tool_result", "tool_use_id": tool_use_id,
                        "content": content, "is_error": is_error
                    }),
                    Block::Raw { value } => value.clone(),
                })
                .collect();
            json!({ "role": if msg.role == Role::User { "user" } else { "assistant" }, "content": content })
        })
        .collect();

    let tools: Vec<Value> = req
        .tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
                // 파일 내용 같은 큰 입력을 한 번에 몰아 받지 않고 흘려 받는다.
                "eager_input_streaming": true,
            })
        })
        .collect();

    let mut body = json!({
        "model": m.model,
        "max_tokens": m.max_tokens,
        "stream": true,
        "system": [{ "type": "text", "text": req.system }],
        "messages": messages,
        // 마지막 캐시 가능 블록에 자동으로 캐시 지점을 둔다 (시스템 프롬프트와 이전 대화 재사용).
        "cache_control": { "type": "ephemeral" },
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }
    if let Some(effort) = &m.effort {
        body["output_config"] = json!({ "effort": effort });
    }
    if let Some(f) = &m.fallbacks {
        body["fallbacks"] = json!(f);
    }
    body
}

enum Building {
    Text(String),
    Tool { id: String, name: String, json: String },
    Raw(Value),
}

pub async fn stream(
    http: &reqwest::Client,
    m: &ModelConfig,
    req: &ChatRequest<'_>,
    cancel: &Arc<AtomicBool>,
    on_event: &mut (dyn FnMut(StreamEvent<'_>) + Send),
) -> Result<ChatResponse> {
    let key = m.resolve_api_key().with_context(|| {
        format!(
            "API 키가 없습니다. 환경변수 {}를 설정하세요",
            m.api_key_env.as_deref().unwrap_or("ANTHROPIC_API_KEY")
        )
    })?;
    let base = m.base_url.as_deref().unwrap_or(DEFAULT_BASE).trim_end_matches('/');
    let mut rb = http
        .post(format!("{base}/v1/messages"))
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json");
    if m.fallbacks.is_some() {
        rb = rb.header("anthropic-beta", FALLBACK_BETA);
    }
    let resp = rb.json(&build_body(m, req)).send().await.context("모델 API 연결 실패")?;
    if !resp.status().is_success() {
        return Err(http_error(resp).await);
    }

    let mut parser = SseParser::default();
    let mut building: Vec<Option<Building>> = Vec::new();
    let mut blocks: Vec<(usize, Block)> = Vec::new();
    let mut usage = Usage::default();
    let mut stop = StopReason::Other;
    let mut model = m.model.clone();
    let mut body = resp.bytes_stream();

    'outer: while let Some(chunk) = body.next().await {
        if cancelled(cancel) {
            bail!("사용자가 중단했습니다");
        }
        let chunk = chunk.context("스트림 수신 오류")?;
        for ev in parser.push(&chunk) {
            let Ok(v) = serde_json::from_str::<Value>(&ev.data) else { continue };
            match v["type"].as_str().unwrap_or("") {
                "message_start" => {
                    let u = &v["message"]["usage"];
                    usage.input_tokens = u["input_tokens"].as_u64().unwrap_or(0);
                    usage.cache_read_tokens = u["cache_read_input_tokens"].as_u64().unwrap_or(0);
                    usage.cache_write_tokens = u["cache_creation_input_tokens"].as_u64().unwrap_or(0);
                    if let Some(mm) = v["message"]["model"].as_str() {
                        model = mm.to_string();
                    }
                }
                "content_block_start" => {
                    let idx = v["index"].as_u64().unwrap_or(0) as usize;
                    let cb = &v["content_block"];
                    let b = match cb["type"].as_str() {
                        Some("text") => Building::Text(cb["text"].as_str().unwrap_or("").to_string()),
                        Some("tool_use") => {
                            let name = cb["name"].as_str().unwrap_or("").to_string();
                            on_event(StreamEvent::ToolStart { name: &name });
                            Building::Tool { id: cb["id"].as_str().unwrap_or("").to_string(), name, json: String::new() }
                        }
                        _ => Building::Raw(cb.clone()),
                    };
                    if building.len() <= idx {
                        building.resize_with(idx + 1, || None);
                    }
                    building[idx] = Some(b);
                }
                "content_block_delta" => {
                    let idx = v["index"].as_u64().unwrap_or(0) as usize;
                    let d = &v["delta"];
                    let Some(Some(b)) = building.get_mut(idx) else { continue };
                    match (b, d["type"].as_str().unwrap_or("")) {
                        (Building::Text(t), "text_delta") => {
                            let s = d["text"].as_str().unwrap_or("");
                            t.push_str(s);
                            on_event(StreamEvent::Text(s));
                        }
                        (Building::Tool { json, .. }, "input_json_delta") => {
                            json.push_str(d["partial_json"].as_str().unwrap_or(""));
                        }
                        (Building::Raw(raw), "thinking_delta") => append(raw, "thinking", &d["thinking"]),
                        (Building::Raw(raw), "signature_delta") => {
                            raw["signature"] = d["signature"].clone();
                        }
                        (Building::Raw(raw), "text_delta") => append(raw, "text", &d["text"]),
                        _ => {}
                    }
                }
                "content_block_stop" => {
                    let idx = v["index"].as_u64().unwrap_or(0) as usize;
                    if let Some(b) = building.get_mut(idx).and_then(Option::take) {
                        blocks.push((idx, finish(b)));
                    }
                }
                "message_delta" => {
                    if let Some(s) = v["delta"]["stop_reason"].as_str() {
                        stop = match s {
                            "end_turn" | "stop_sequence" => StopReason::EndTurn,
                            "tool_use" => StopReason::ToolUse,
                            "max_tokens" => StopReason::MaxTokens,
                            "refusal" => StopReason::Refusal,
                            _ => StopReason::Other,
                        };
                    }
                    if let Some(o) = v["usage"]["output_tokens"].as_u64() {
                        usage.output_tokens = o;
                    }
                }
                "message_stop" => break 'outer,
                "error" => {
                    let msg = v["error"]["message"].as_str().unwrap_or("알 수 없는 오류");
                    return Err(anyhow!("모델 스트림 오류: {msg}"));
                }
                _ => {}
            }
        }
    }

    // 스트림이 중간에 끊겨 닫히지 않은 블록도 살린다.
    for (idx, b) in building.into_iter().enumerate() {
        if let Some(b) = b {
            blocks.push((idx, finish(b)));
        }
    }
    blocks.sort_by_key(|(i, _)| *i);
    Ok(ChatResponse { blocks: blocks.into_iter().map(|(_, b)| b).collect(), stop_reason: stop, usage, model })
}

fn append(raw: &mut Value, field: &str, delta: &Value) {
    let cur = raw[field].as_str().unwrap_or("").to_string();
    raw[field] = Value::String(cur + delta.as_str().unwrap_or(""));
}

fn finish(b: Building) -> Block {
    match b {
        Building::Text(text) => Block::Text { text },
        Building::Raw(value) => Block::Raw { value },
        Building::Tool { id, name, json } => {
            let src = if json.trim().is_empty() { "{}" } else { json.as_str() };
            match serde_json::from_str::<Value>(src) {
                Ok(input) if input.is_object() => Block::ToolUse { id, name, input, invalid_json: None },
                _ => Block::ToolUse { id, name, input: json!({}), invalid_json: Some(json) },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{Message, ToolSpec};

    fn model() -> ModelConfig {
        crate::config::load(None).unwrap().models["smart"].clone()
    }

    #[test]
    fn body_shape() {
        let msgs = vec![
            Message::user_text("hi"),
            Message {
                role: Role::Assistant,
                blocks: vec![
                    Block::Raw { value: json!({"type":"thinking","thinking":"","signature":"s"}) },
                    Block::ToolUse { id: "t1".into(), name: "read_file".into(), input: json!({"path":"a"}), invalid_json: None },
                ],
            },
            Message {
                role: Role::User,
                blocks: vec![Block::ToolResult { tool_use_id: "t1".into(), content: "ok".into(), is_error: false }],
            },
        ];
        let tools = vec![ToolSpec { name: "read_file".into(), description: "d".into(), input_schema: json!({"type":"object"}) }];
        let b = build_body(&model(), &ChatRequest { system: "sys", messages: &msgs, tools: &tools });
        assert_eq!(b["model"], "claude-opus-5");
        assert_eq!(b["stream"], true);
        assert_eq!(b["fallbacks"], "default");
        assert_eq!(b["output_config"]["effort"], "high");
        assert_eq!(b["tools"][0]["eager_input_streaming"], true);
        assert_eq!(b["messages"][1]["content"][0]["type"], "thinking");
        assert_eq!(b["messages"][1]["content"][1]["type"], "tool_use");
        assert_eq!(b["messages"][2]["content"][0]["type"], "tool_result");
    }

    #[test]
    fn invalid_tool_json_is_flagged() {
        let b = finish(Building::Tool { id: "x".into(), name: "write_file".into(), json: "{\"path\": \"a".into() });
        assert!(matches!(b, Block::ToolUse { invalid_json: Some(_), .. }));
    }
}
