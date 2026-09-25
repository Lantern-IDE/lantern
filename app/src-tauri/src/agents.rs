//! 에이전트 정의: 프롬프트 + 허용 도구 + 모델. `.lantern/agents/*.md` 파일 하나가 에이전트 하나다.
//!
//! ```text
//! ---
//! name: 리뷰어
//! description: 변경 사항을 검토한다
//! model: smart
//! tools: [get_context, read_file, search_symbols]
//! ---
//! 프롬프트 본문 (Markdown)
//! ```

use serde::Serialize;
use std::path::Path;

pub const READ_TOOLS: &[&str] =
    &["get_context", "search_symbols", "get_symbol", "find_references", "read_file", "list_dir"];
pub const ALL_TOOLS: &[&str] = &[
    "get_context",
    "search_symbols",
    "get_symbol",
    "find_references",
    "read_file",
    "list_dir",
    "write_file",
    "edit_file",
    "run_command",
    "remember",
];

pub const EXAMPLE_AGENT: &str = r#"---
name: 리뷰어
description: 코드를 읽기만 하고 문제점과 개선안을 정리한다
tools: [get_context, search_symbols, get_symbol, find_references, read_file, list_dir]
---
당신은 꼼꼼한 코드 리뷰어입니다.
- 버그 가능성, 보안 문제, 읽기 어려운 부분을 우선순위대로 정리하세요.
- 지적마다 파일 경로와 줄 번호를 붙이세요.
- 코드를 직접 고치지 말고, 고칠 방법을 제안만 하세요.
"#;

#[derive(Debug, Clone, Serialize)]
pub struct AgentDef {
    pub id: String,
    pub name: String,
    pub description: String,
    /// 비어 있으면 `routing.default`
    pub model: String,
    pub tools: Vec<String>,
    pub prompt: String,
    /// 정의 파일 경로 (내장 에이전트는 없음)
    pub path: Option<String>,
}

fn builtin() -> Vec<AgentDef> {
    vec![
        AgentDef {
            id: "ask".into(),
            name: "질문".into(),
            description: "코드를 읽고 답한다. 파일을 바꾸지 않는다".into(),
            model: String::new(),
            tools: READ_TOOLS.iter().map(|s| s.to_string()).collect(),
            prompt: "사용자의 질문에 이 프로젝트의 실제 코드를 근거로 답하세요. \
                     근거가 된 파일과 줄 번호를 밝히세요."
                .into(),
            path: None,
        },
        AgentDef {
            id: "code".into(),
            name: "코드 작성".into(),
            description: "파일을 수정하고 명령을 실행한다. 변경은 승인 후 적용".into(),
            model: String::new(),
            tools: ALL_TOOLS.iter().map(|s| s.to_string()).collect(),
            prompt: "사용자의 요청을 코드로 구현하세요. 수정 전에 관련 코드를 먼저 읽고, \
                     기존 스타일을 따르세요. 가능하면 테스트나 빌드 명령으로 확인하세요. \
                     끝나면 무엇을 바꿨는지 짧게 정리하세요."
                .into(),
            path: None,
        },
    ]
}

/// `---`로 감싼 머리말과 본문을 나눈다. 머리말은 `key: value` 줄만 지원한다.
fn parse(id: &str, text: &str) -> AgentDef {
    let mut def = AgentDef {
        id: id.to_string(),
        name: id.to_string(),
        description: String::new(),
        model: String::new(),
        tools: READ_TOOLS.iter().map(|s| s.to_string()).collect(),
        prompt: text.trim().to_string(),
        path: None,
    };
    let text = text.trim_start_matches('\u{feff}');
    let Some(rest) = text.strip_prefix("---") else { return def };
    let Some(end) = rest.find("\n---") else { return def };
    let head = &rest[..end];
    def.prompt = rest[end + 4..].trim().to_string();
    for line in head.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        let v = v.trim().trim_matches('"');
        match k.trim() {
            "name" => def.name = v.to_string(),
            "description" => def.description = v.to_string(),
            "model" => def.model = v.to_string(),
            "tools" => {
                def.tools = v
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .split(',')
                    .map(|t| t.trim().trim_matches('"').to_string())
                    .filter(|t| ALL_TOOLS.contains(&t.as_str()))
                    .collect();
            }
            _ => {}
        }
    }
    def
}

/// 내장 에이전트 + 프로젝트 에이전트. 같은 id면 프로젝트 쪽이 이긴다.
pub fn load(root: Option<&Path>) -> Vec<AgentDef> {
    let mut out = builtin();
    let Some(root) = root else { return out };
    let dir = root.join(".lantern").join("agents");
    let Ok(entries) = std::fs::read_dir(&dir) else { return out };
    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    files.sort();
    for p in files {
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        let id = p.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let mut def = parse(&id, &text);
        def.path = Some(p.to_string_lossy().to_string());
        out.retain(|a| a.id != def.id);
        out.push(def);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_front_matter() {
        let d = parse("reviewer", EXAMPLE_AGENT);
        assert_eq!(d.name, "리뷰어");
        assert_eq!(d.tools.len(), 6);
        assert!(!d.tools.contains(&"write_file".to_string()));
        assert!(d.prompt.starts_with("당신은"));
    }

    #[test]
    fn unknown_tools_are_dropped_and_no_front_matter_is_prompt() {
        let d = parse("x", "---\ntools: [read_file, rm_rf]\nmodel: local\n---\nhi");
        assert_eq!(d.tools, ["read_file"]);
        assert_eq!(d.model, "local");
        let plain = parse("y", "그냥 프롬프트");
        assert_eq!(plain.prompt, "그냥 프롬프트");
    }
}
