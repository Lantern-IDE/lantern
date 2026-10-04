//! 모델 라우터. 공급자와 무관한 대화 형식과 스트리밍 인터페이스.

mod anthropic;
mod openai;
mod sse;

use crate::config::ModelConfig;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
        /// 스트리밍된 입력 JSON이 깨졌을 때 원문. 이 경우 도구를 실행하지 않는다.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        invalid_json: Option<String>,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    /// 사용자가 붙인 이미지 (base64)
    Image {
        media_type: String,
        data: String,
    },
    /// 공급자 고유 블록 (thinking 등). 같은 공급자에 그대로 되돌려 보낸다.
    Raw {
        value: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub blocks: Vec<Block>,
}

impl Message {
    pub fn user_text(text: impl Into<String>) -> Self {
        Self { role: Role::User, blocks: vec![Block::Text { text: text.into() }] }
    }
}

/// 채팅에 붙인 이미지 (화면에서 받는다)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageInput {
    pub media_type: String,
    pub data: String,
}

pub const MAX_IMAGES: usize = 4;
/// base64 길이 기준 (원본 약 3.7MB). 공급자 한도(5MB)보다 작게 둔다
pub const MAX_IMAGE_B64: usize = 5_000_000;

/// 공급자가 받는 형식·크기인지 확인한다
pub fn check_images(images: &[ImageInput]) -> anyhow::Result<()> {
    if images.len() > MAX_IMAGES {
        anyhow::bail!("이미지는 한 번에 {MAX_IMAGES}장까지 보낼 수 있습니다");
    }
    for i in images {
        if !matches!(i.media_type.as_str(), "image/png" | "image/jpeg" | "image/gif" | "image/webp") {
            anyhow::bail!("PNG, JPEG, GIF, WebP 이미지만 보낼 수 있습니다 ({})", i.media_type);
        }
        if i.data.len() > MAX_IMAGE_B64 {
            anyhow::bail!("이미지가 너무 큽니다 (한 장에 약 3.5MB까지)");
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

pub struct ChatRequest<'a> {
    pub system: &'a str,
    pub messages: &'a [Message],
    pub tools: &'a [ToolSpec],
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Refusal,
    Other,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

impl Usage {
    /// 가격이 설정된 경우의 비용 (USD). 캐시 읽기 0.1배, 캐시 쓰기 1.25배.
    pub fn cost(&self, m: &ModelConfig) -> Option<f64> {
        let (pi, po) = (m.price_input?, m.price_output?);
        let input = self.input_tokens as f64
            + self.cache_read_tokens as f64 * 0.1
            + self.cache_write_tokens as f64 * 1.25;
        Some((input * pi + self.output_tokens as f64 * po) / 1_000_000.0)
    }
}

#[derive(Debug)]
pub struct ChatResponse {
    pub blocks: Vec<Block>,
    pub stop_reason: StopReason,
    pub usage: Usage,
    pub model: String,
}

/// 스트리밍 중 화면에 보낼 이벤트
pub enum StreamEvent<'a> {
    Text(&'a str),
    ToolStart { name: &'a str },
}

pub async fn stream(
    http: &reqwest::Client,
    m: &ModelConfig,
    req: &ChatRequest<'_>,
    cancel: &Arc<AtomicBool>,
    on_event: &mut (dyn FnMut(StreamEvent<'_>) + Send),
) -> Result<ChatResponse> {
    match m.provider.as_str() {
        "anthropic" => anthropic::stream(http, m, req, cancel, on_event).await,
        "openai" => openai::stream(http, m, req, cancel, on_event).await,
        other => bail!("알 수 없는 provider: {other} (anthropic 또는 openai)"),
    }
}

pub(crate) fn cancelled(cancel: &Arc<AtomicBool>) -> bool {
    cancel.load(Ordering::Relaxed)
}

/// HTTP 오류 응답에서 사람이 읽을 메시지를 뽑는다.
pub(crate) async fn http_error(resp: reqwest::Response) -> anyhow::Error {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    let msg = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message").and_then(Value::as_str).map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(500).collect());
    let hint = match status.as_u16() {
        401 | 403 => " (API 키를 확인하세요)",
        429 => " (요청 한도 초과, 잠시 후 다시 시도하세요)",
        _ => "",
    };
    anyhow::anyhow!("모델 API 오류 {status}: {msg}{hint}")
}

#[cfg(test)]
mod image_tests {
    use super::*;

    #[test]
    fn checks_image_count_type_and_size() {
        let png = |n: usize| ImageInput { media_type: "image/png".into(), data: "A".repeat(n) };
        assert!(check_images(&[png(10), png(10)]).is_ok());
        assert!(check_images(&vec![png(1); MAX_IMAGES + 1]).is_err());
        assert!(check_images(&[ImageInput { media_type: "image/svg+xml".into(), data: "x".into() }]).is_err());
        assert!(check_images(&[png(MAX_IMAGE_B64 + 1)]).is_err());
    }
}
