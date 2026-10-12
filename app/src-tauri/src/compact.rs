//! 긴 작업의 대화 줄이기: 도구 결과가 쌓여 모델 입력 한도를 넘기 전에 대화 기록을 줄인다.
//!
//! 1단계: 최근 메시지를 빼고, 긴 도구 결과는 앞부분만, 지난 질문의 맥락 카드와 이미지는 뺀다.
//! 2단계: 그래도 넘으면 오래된 단계(모델 응답 + 그 결과)를 짝으로 지우고, 무엇을 했는지 첫 요청에 짧게 남긴다.
//! 도구 호출과 결과의 짝, 사용자·모델이 번갈아 오는 순서는 깨지 않는다.

use crate::llm::{Block, Message, Role, ToolSpec};
use lantern_context::tokenize::estimate_tokens;

/// 줄이지 않고 그대로 두는 최근 메시지 수
const KEEP_RECENT: usize = 6;
/// 1단계에서 오래된 도구 결과를 이 글자 수까지 줄인다
const OLD_RESULT_CHARS: usize = 1500;
const OLD_RESULT_HEAD: usize = 600;
/// 그래도 넘으면 최근 도구 결과도 이만큼까지
const RECENT_RESULT_CHARS: usize = 6000;
/// 지운 단계 요약의 최대 길이
const SUMMARY_CHARS: usize = 4000;
const SUMMARY_OPEN: &str = "<earlier_steps>";
const SUMMARY_CLOSE: &str = "</earlier_steps>";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Compaction {
    pub before: usize,
    pub after: usize,
    /// 지운 단계 수 (모델 응답 기준)
    pub dropped: usize,
}

fn block_tokens(b: &Block) -> usize {
    match b {
        Block::Text { text } => estimate_tokens(text),
        Block::ToolUse { input, name, .. } => estimate_tokens(&input.to_string()) + estimate_tokens(name) + 10,
        Block::ToolResult { content, .. } => estimate_tokens(content) + 10,
        // 이미지는 공급자마다 다르지만 대략 이 정도
        Block::Image { .. } => 1600,
        Block::Raw { value } => estimate_tokens(&value.to_string()),
    }
}

pub fn estimate(system: &str, tools: &[ToolSpec], history: &[Message]) -> usize {
    let fixed = estimate_tokens(system) + tools.iter().map(|t| estimate_tokens(&t.description) + estimate_tokens(&t.input_schema.to_string()) + 10).sum::<usize>();
    fixed + history.iter().map(|m| m.blocks.iter().map(block_tokens).sum::<usize>() + 4).sum::<usize>()
}

fn shorten(content: &str, head: usize) -> String {
    let total = content.chars().count();
    let kept: String = content.chars().take(head).collect();
    format!("{kept}\n…(Lantern이 맥락 한도에 맞추려고 줄였습니다: 원래 {total}자. 필요하면 다시 읽으세요)")
}

/// `<project_context>…</project_context>`를 짧은 표시로
fn drop_context(text: &str) -> Option<String> {
    let start = text.find("<project_context>")?;
    let end = text.find("</project_context>").map(|e| e + "</project_context>".len()).unwrap_or(text.len());
    if end - start < 200 {
        return None;
    }
    Some(format!("{}<project_context>(이 질문에 붙였던 코드 맥락은 줄였습니다)</project_context>{}", &text[..start], &text[end..]))
}

/// 1단계: 메시지 [0, upto)의 긴 도구 결과·맥락 카드·이미지를 줄인다. 마지막 사용자 질문의 맥락은 둔다.
fn shrink(history: &mut [Message], upto: usize, result_chars: usize, result_head: usize) {
    let last_user_text = history.iter().rposition(|m| m.role == Role::User && m.blocks.iter().any(|b| matches!(b, Block::Text { .. })));
    for (i, m) in history.iter_mut().enumerate().take(upto) {
        for b in m.blocks.iter_mut() {
            match b {
                Block::ToolResult { content, .. } if content.chars().count() > result_chars => *content = shorten(content, result_head),
                Block::Text { text } if m.role == Role::User && Some(i) != last_user_text => {
                    if let Some(t) = drop_context(text) {
                        *text = t;
                    }
                }
                Block::Image { .. } => *b = Block::Text { text: "[이미지는 줄였습니다]".into() },
                _ => {}
            }
        }
    }
}

fn one_line(s: &str, n: usize) -> String {
    let s = s.split("<project_context>").next().unwrap_or(s).split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s
    }
}

/// 지운 메시지에서 무엇을 했는지 한 줄씩
fn summarize(dropped: &[Message]) -> Vec<String> {
    let mut out = Vec::new();
    for m in dropped {
        for b in &m.blocks {
            match (m.role, b) {
                (Role::Assistant, Block::Text { text }) if !text.trim().is_empty() => out.push(format!("- assistant: {}", one_line(text, 160))),
                (Role::Assistant, Block::ToolUse { name, input, .. }) => {
                    let arg = ["path", "query", "name", "command", "url"].iter().find_map(|k| input.get(*k).and_then(|v| v.as_str())).unwrap_or("");
                    out.push(format!("- {name} {}", one_line(arg, 120)));
                }
                (Role::User, Block::Text { text }) if !text.trim().is_empty() => out.push(format!("- user: {}", one_line(text, 200))),
                (Role::User, Block::ToolResult { is_error: true, content, .. }) => out.push(format!("  (error: {})", one_line(content, 120))),
                _ => {}
            }
        }
    }
    out
}

/// 첫 요청에 붙은 지난 요약을 새 줄과 합친다 (오래된 줄부터 버려 길이를 맞춘다)
fn attach_summary(first: &mut Message, lines: Vec<String>) {
    let mut all: Vec<String> = Vec::new();
    first.blocks.retain(|b| match b {
        Block::Text { text } if text.starts_with(SUMMARY_OPEN) => {
            all.extend(text.lines().skip(2).filter(|l| *l != SUMMARY_CLOSE).map(str::to_string));
            false
        }
        _ => true,
    });
    all.extend(lines);
    while all.iter().map(|l| l.len() + 1).sum::<usize>() > SUMMARY_CHARS && all.len() > 1 {
        all.remove(0);
    }
    first.blocks.push(Block::Text {
        text: format!("{SUMMARY_OPEN}\nLantern removed earlier steps of this task to fit the model's context window. What they did (oldest first):\n{}\n{SUMMARY_CLOSE}", all.join("\n")),
    });
}

/// 대화가 `budget` 토큰(시스템·도구 설명 포함)을 넘으면 줄인다. 줄였으면 얼마나 줄였는지 돌려준다.
pub fn compact(history: &mut Vec<Message>, system: &str, tools: &[ToolSpec], budget: usize) -> Option<Compaction> {
    let before = estimate(system, tools, history);
    if before <= budget || history.len() < 2 {
        return None;
    }
    let recent_from = history.len().saturating_sub(KEEP_RECENT);
    shrink(history, recent_from, OLD_RESULT_CHARS, OLD_RESULT_HEAD);

    // 2단계: 첫 요청 뒤의 (모델 응답, 그다음 사용자 메시지) 짝을 오래된 것부터 지운다
    let mut dropped_msgs: Vec<Message> = Vec::new();
    while estimate(system, tools, history) > budget && history.len() > KEEP_RECENT + 1 && history.len() >= 3 && history[1].role == Role::Assistant {
        dropped_msgs.extend(history.drain(1..3));
    }
    let dropped = dropped_msgs.iter().filter(|m| m.role == Role::Assistant).count();
    if dropped > 0 {
        attach_summary(&mut history[0], summarize(&dropped_msgs));
    }
    // 그래도 넘으면 최근 도구 결과도 줄인다
    if estimate(system, tools, history) > budget {
        let n = history.len();
        shrink(history, n, RECENT_RESULT_CHARS, RECENT_RESULT_CHARS / 2);
    }
    Some(Compaction { before, after: estimate(system, tools, history), dropped })
}

/// 공급자가 입력이 너무 길다고 거절한 오류인지
pub fn is_context_overflow(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    ["prompt is too long", "context_length_exceeded", "maximum context length", "context window", "too many tokens", "input is too long", "reduce the length"].iter().any(|p| e.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool_step(i: usize, result: &str) -> [Message; 2] {
        [
            Message { role: Role::Assistant, blocks: vec![Block::ToolUse { id: format!("t{i}"), name: "read_file".into(), input: json!({ "path": format!("src/f{i}.rs") }), invalid_json: None }] },
            Message { role: Role::User, blocks: vec![Block::ToolResult { tool_use_id: format!("t{i}"), content: result.into(), is_error: false }] },
        ]
    }

    fn task(steps: usize, result_len: usize) -> Vec<Message> {
        let mut h = vec![Message::user_text(format!("세션 쿠키를 고쳐줘\n\n<project_context>\n{}\n</project_context>", "ctx ".repeat(500)))];
        for i in 0..steps {
            h.extend(tool_step(i, &"x".repeat(result_len)));
        }
        h
    }

    fn well_formed(h: &[Message]) {
        assert_eq!(h[0].role, Role::User);
        for w in h.windows(2) {
            assert_ne!(w[0].role, w[1].role, "사용자·모델이 번갈아 와야 한다");
        }
        // 모든 도구 결과에는 바로 앞 모델 응답에 같은 id의 호출이 있다
        for (i, m) in h.iter().enumerate() {
            for b in &m.blocks {
                if let Block::ToolResult { tool_use_id, .. } = b {
                    assert!(h[i - 1].blocks.iter().any(|x| matches!(x, Block::ToolUse { id, .. } if id == tool_use_id)), "짝 없는 결과 {tool_use_id}");
                }
            }
        }
    }

    #[test]
    fn leaves_short_conversations_alone() {
        let mut h = task(3, 100);
        assert!(compact(&mut h, "sys", &[], 100_000).is_none());
        assert_eq!(h.len(), 7);
    }

    #[test]
    fn shrinks_old_results_first() {
        let mut h = task(6, 8000);
        let before = estimate("sys", &[], &h);
        let c = compact(&mut h, "sys", &[], before - 3000).unwrap();
        assert_eq!(c.dropped, 0, "결과를 줄이는 것만으로 맞으면 단계를 지우지 않는다");
        assert!(c.after <= before - 3000);
        assert_eq!(h.len(), 13);
        // 최근 결과는 그대로, 오래된 결과는 줄어듦
        assert!(matches!(&h[12].blocks[0], Block::ToolResult { content, .. } if content.len() == 8000));
        assert!(matches!(&h[2].blocks[0], Block::ToolResult { content, .. } if content.contains("줄였습니다")));
        well_formed(&h);
    }

    #[test]
    fn drops_old_steps_and_keeps_a_summary() {
        let mut h = task(30, 3000);
        let c = compact(&mut h, "sys", &[], 6000).unwrap();
        assert!(c.dropped > 0 && c.after < c.before);
        well_formed(&h);
        // 첫 요청은 남고, 지운 단계는 요약으로
        let first: String = h[0].blocks.iter().filter_map(|b| if let Block::Text { text } = b { Some(text.as_str()) } else { None }).collect();
        assert!(first.contains("세션 쿠키를 고쳐줘"));
        assert!(first.contains("<earlier_steps>") && first.contains("read_file src/f0.rs"));
        // 다시 줄이면 요약이 하나로 합쳐진다
        h.extend(tool_step(99, &"y".repeat(20000)));
        h.extend(tool_step(100, &"y".repeat(20000)));
        compact(&mut h, "sys", &[], 6000).unwrap();
        let n = h[0].blocks.iter().filter(|b| matches!(b, Block::Text { text } if text.starts_with("<earlier_steps>"))).count();
        assert_eq!(n, 1);
        well_formed(&h);
    }

    #[test]
    fn recognizes_overflow_errors() {
        assert!(is_context_overflow("모델 API 오류 400: prompt is too long: 210000 tokens > 200000 maximum"));
        assert!(is_context_overflow("This model's maximum context length is 128000 tokens"));
        assert!(!is_context_overflow("모델 API 오류 401: invalid x-api-key"));
    }
}
