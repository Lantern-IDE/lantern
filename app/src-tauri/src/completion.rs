//! 인라인 자동 완성 (고스트 텍스트). 기본은 꺼져 있고, `[routing] completion`에 모델을 정하면 켜진다.
//! 타이핑이 멈출 때마다 요청하므로 빠르고 싼 모델(로컬 모델 등)을 권한다.

use crate::config::{Config, ModelConfig};
use crate::llm::{self, ChatRequest, Message};
use anyhow::Result;
use lantern_context::secrets::redact;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const PREFIX_CHARS: usize = 4000;
const SUFFIX_CHARS: usize = 1200;

const SYSTEM: &str = "You are a code completion engine inside an editor. \
Given the text before and after the cursor, output ONLY the text to insert at the cursor. \
No explanations, no markdown fences, no repetition of the text before the cursor. \
Prefer completing the current line or statement; at most 6 lines. \
If nothing sensible can be inserted, output nothing.";

/// 자동 완성에 쓸 모델. 설정이 비어 있거나 없는 모델이면 None (= 꺼짐)
pub fn model(config: &Config) -> Option<ModelConfig> {
    let key = config.routing.completion.trim();
    if key.is_empty() {
        return None;
    }
    let mut m = config.models.get(key)?.clone();
    // 짧고 빠르게: 출력은 조금만, 생각은 최소로, 거절 시 다른 모델로 넘기지 않는다
    m.max_tokens = m.max_tokens.min(256);
    if m.provider == "anthropic" {
        m.effort = Some("low".into());
    }
    m.fallbacks = None;
    Some(m)
}

fn tail(s: &str, n: usize) -> &str {
    let count = s.chars().count();
    if count <= n {
        return s;
    }
    let skip = s.char_indices().nth(count - n).map(|(i, _)| i).unwrap_or(0);
    &s[skip..]
}

fn head(s: &str, n: usize) -> &str {
    s.char_indices().nth(n).map(|(i, _)| &s[..i]).unwrap_or(s)
}

pub fn prompt(path: &str, prefix: &str, suffix: &str) -> String {
    let (before, _) = redact(tail(prefix, PREFIX_CHARS));
    let (after, _) = redact(head(suffix, SUFFIX_CHARS));
    format!("File: {path}\n<before_cursor>{before}</before_cursor><after_cursor>{after}</after_cursor>")
}

/// 모델 출력 정리: 코드 울타리 제거, 커서 뒤 내용과 겹치는 끝부분 제거
pub fn clean(raw: &str, suffix: &str) -> String {
    let mut s = raw.trim_end_matches(['\n', '\r']).to_string();
    if let Some(rest) = s.trim_start().strip_prefix("```") {
        let body = rest.split_once('\n').map(|(_, b)| b).unwrap_or("");
        s = body.trim_end().trim_end_matches("```").trim_end().to_string();
    }
    let next_line = suffix.lines().next().unwrap_or("").trim();
    if !next_line.is_empty() {
        if let Some(stripped) = s.strip_suffix(next_line) {
            s = stripped.to_string();
        }
    }
    s
}

pub async fn complete(
    http: &reqwest::Client,
    m: &ModelConfig,
    path: &str,
    prefix: &str,
    suffix: &str,
    cancel: &Arc<AtomicBool>,
) -> Result<(String, llm::Usage)> {
    let messages = [Message::user_text(prompt(path, prefix, suffix))];
    let req = ChatRequest { system: SYSTEM, messages: &messages, tools: &[] };
    let resp = llm::stream(http, m, &req, cancel, &mut |_| {}).await?;
    let text: String = resp
        .blocks
        .iter()
        .filter_map(|b| match b {
            llm::Block::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    Ok((clean(&text, suffix), resp.usage))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_context_by_chars() {
        assert_eq!(tail("가나다라", 2), "다라");
        assert_eq!(head("가나다라", 3), "가나다");
        assert_eq!(tail("ab", 5), "ab");
    }

    #[test]
    fn prompt_redacts_secrets() {
        let p = prompt("a.ts", "const key = \"sk-ant-api03-abcdefghijklmnopqrstuvwx\";\nfoo(", ")");
        assert!(!p.contains("sk-ant-api03") && p.contains("foo("));
    }

    #[test]
    fn cleans_output() {
        assert_eq!(clean("```ts\nreturn a + b;\n```", ""), "return a + b;");
        assert_eq!(clean("a, b)", ")"), "a, b", "커서 뒤에 이미 있는 닫는 괄호는 뺀다");
        assert_eq!(clean("x\n", ""), "x");
    }

    #[test]
    fn disabled_by_default() {
        let cfg: Config = toml::from_str(crate::config::DEFAULT_CONFIG).unwrap();
        assert!(model(&cfg).is_none());
    }
}
