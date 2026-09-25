//! OpenAI 호환 Chat Completions API (OpenAI, Ollama, LM Studio, vLLM, OpenRouter 등).

use super::sse::SseParser;
use super::{cancelled, http_error, Block, ChatRequest, ChatResponse, Role, StopReason, StreamEvent, Usage};
use crate::config::ModelConfig;
use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const DEFAULT_BASE: &str = "https://api.openai.com/v1";

pub fn build_body(m: &ModelConfig, req: &ChatRequest<'_>) -> Value {
    let mut messages = vec![json!({ "role": "system", "content": req.system })];
    for msg in req.messages {
        match msg.role {
            Role::User => {
                let mut text = Vec::new();
                for b in &msg.blocks {
                    match b {
                        Block::Text { text: t } => text.push(t.clone()),
                        Block::ToolResult { tool_use_id, content, is_error } => {
                            let content = if *is_error { format!("[오류] {content}") } else { content.clone() };
                            messages.push(json!({ "role": "tool", "tool_call_id": tool_use_id, "content": content }));
                        }
                        _ => {}
                    }
                }
                if !text.is_empty() {
                    messages.push(json!({ "role": "user", "content": text.join("\n\n") }));
                }
            }
            Role::Assistant => {
                let text: String = msg
                    .blocks
                    .iter()
                    .filter_map(|b| if let Block::Text { text } = b { Some(text.as_str()) } else { None })
                    .collect();
                let calls: Vec<Value> = msg
                    .blocks
                    .iter()
                    .filter_map(|b| match b {
                        Block::ToolUse { id, name, input, .. } => Some(json!({
                            "id": id, "type": "function",
                            "function": { "name": name, "arguments": input.to_string() }
                        })),
                        _ => None,
                    })
                    .collect();
                let mut a = json!({ "role": "assistant", "content": if text.is_empty() { Value::Null } else { json!(text) } });
                if !calls.is_empty() {
                    a["tool_calls"] = Value::Array(calls);
                }
                messages.push(a);
            }
        }
    }
    let mut body = json!({
        "model": m.model,
        "messages": messages,
        "stream": true,
        "stream_options": { "include_usage": true },
        "max_tokens": m.max_tokens,
    });
    if !req.tools.is_empty() {
        body["tools"] = req
            .tools
            .iter()
            .map(|t| json!({ "type": "function", "function": {
                "name": t.name, "description": t.description, "parameters": t.input_schema
            }}))
            .collect();
    }
    body
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    args: String,
}

pub async fn stream(
    http: &reqwest::Client,
    m: &ModelConfig,
    req: &ChatRequest<'_>,
    cancel: &Arc<AtomicBool>,
    on_event: &mut (dyn FnMut(StreamEvent<'_>) + Send),
) -> Result<ChatResponse> {
    let base = m.base_url.as_deref().unwrap_or(DEFAULT_BASE).trim_end_matches('/');
    let mut rb = http.post(format!("{base}/chat/completions")).header("content-type", "application/json");
    if let Some(key) = m.resolve_api_key() {
        rb = rb.bearer_auth(key);
    }
    let resp = rb
        .json(&build_body(m, req))
        .send()
        .await
        .with_context(|| format!("{base} 연결 실패 (서버가 실행 중인지 확인하세요)"))?;
    if !resp.status().is_success() {
        return Err(http_error(resp).await);
    }

    let mut parser = SseParser::default();
    let mut text = String::new();
    let mut calls: BTreeMap<u64, PartialCall> = BTreeMap::new();
    let mut usage = Usage::default();
    let mut stop = StopReason::Other;
    let mut model = m.model.clone();
    let mut body = resp.bytes_stream();

    'outer: while let Some(chunk) = body.next().await {
        if cancelled(cancel) {
            bail!("사용자가 중단했습니다");
        }
        for ev in parser.push(&chunk.context("스트림 수신 오류")?) {
            if ev.data.trim() == "[DONE]" {
                break 'outer;
            }
            let Ok(v) = serde_json::from_str::<Value>(&ev.data) else { continue };
            if let Some(err) = v.get("error") {
                bail!("모델 스트림 오류: {}", err["message"].as_str().unwrap_or(&err.to_string()));
            }
            if let Some(mm) = v["model"].as_str() {
                model = mm.to_string();
            }
            if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
                usage.input_tokens = u["prompt_tokens"].as_u64().unwrap_or(0);
                usage.output_tokens = u["completion_tokens"].as_u64().unwrap_or(0);
                usage.cache_read_tokens =
                    u["prompt_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0);
                usage.input_tokens = usage.input_tokens.saturating_sub(usage.cache_read_tokens);
            }
            let Some(choice) = v["choices"].get(0) else { continue };
            let d = &choice["delta"];
            if let Some(s) = d["content"].as_str() {
                text.push_str(s);
                on_event(StreamEvent::Text(s));
            }
            if let Some(tcs) = d["tool_calls"].as_array() {
                for tc in tcs {
                    let idx = tc["index"].as_u64().unwrap_or(0);
                    let c = calls.entry(idx).or_default();
                    if let Some(id) = tc["id"].as_str() {
                        c.id = id.to_string();
                    }
                    if let Some(n) = tc["function"]["name"].as_str() {
                        if c.name.is_empty() {
                            on_event(StreamEvent::ToolStart { name: n });
                        }
                        c.name.push_str(n);
                    }
                    if let Some(a) = tc["function"]["arguments"].as_str() {
                        c.args.push_str(a);
                    }
                }
            }
            if let Some(f) = choice["finish_reason"].as_str() {
                stop = match f {
                    "stop" => StopReason::EndTurn,
                    "tool_calls" | "function_call" => StopReason::ToolUse,
                    "length" => StopReason::MaxTokens,
                    "content_filter" => StopReason::Refusal,
                    _ => StopReason::Other,
                };
            }
        }
    }

    let mut blocks = Vec::new();
    if !text.is_empty() {
        blocks.push(Block::Text { text });
    }
    for (i, c) in calls {
        let id = if c.id.is_empty() { format!("call_{i}") } else { c.id };
        let src = if c.args.trim().is_empty() { "{}" } else { c.args.as_str() };
        let block = match serde_json::from_str::<Value>(src) {
            Ok(input) if input.is_object() => Block::ToolUse { id, name: c.name, input, invalid_json: None },
            _ => Block::ToolUse { id, name: c.name, input: json!({}), invalid_json: Some(c.args) },
        };
        blocks.push(block);
    }
    // 일부 서버는 도구 호출이 있어도 finish_reason을 "stop"으로 보낸다.
    if stop != StopReason::MaxTokens && blocks.iter().any(|b| matches!(b, Block::ToolUse { .. })) {
        stop = StopReason::ToolUse;
    }
    Ok(ChatResponse { blocks, stop_reason: stop, usage, model })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::Message;

    #[test]
    fn converts_tool_round_trip() {
        let m = crate::config::load(None).unwrap().models["local"].clone();
        let msgs = vec![
            Message::user_text("hi"),
            Message {
                role: Role::Assistant,
                blocks: vec![
                    Block::Raw { value: json!({"type":"thinking"}) },
                    Block::ToolUse { id: "c1".into(), name: "read_file".into(), input: json!({"path":"a"}), invalid_json: None },
                ],
            },
            Message {
                role: Role::User,
                blocks: vec![Block::ToolResult { tool_use_id: "c1".into(), content: "x".into(), is_error: false }],
            },
        ];
        let b = build_body(&m, &ChatRequest { system: "s", messages: &msgs, tools: &[] });
        let ms = b["messages"].as_array().unwrap();
        assert_eq!(ms.len(), 4);
        assert_eq!(ms[0]["role"], "system");
        assert_eq!(ms[2]["tool_calls"][0]["function"]["arguments"], "{\"path\":\"a\"}");
        assert_eq!(ms[3]["role"], "tool");
        assert!(b.get("tools").is_none());
    }
}
