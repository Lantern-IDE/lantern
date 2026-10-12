//! 채팅에서 @로 집어 넣은 파일·심볼: 맥락 엔진이 고른 것과 별도로 꼭 붙인다.
//! 맥락 카드가 그대로 읽도록 엔진의 Markdown 형식(`## 경로` / `### \`이름\` (종류, L시작-끝) — 이유`)을 따른다.

use crate::state::{rel_path, resolve_in_root};
use lantern_context::lang::Lang;
use lantern_context::tokenize::estimate_tokens;
use lantern_context::Engine;
use serde::Deserialize;
use std::fmt::Write as _;
use std::path::Path;

pub const REASON: &str = "사용자가 지정";
/// 지정한 코드가 맥락 예산에서 차지할 수 있는 몫
const SHARE: usize = 40; // %

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Mention {
    File { path: String },
    Symbol { name: String, path: String, line: u32 },
}

fn fence(path: &str) -> &'static str {
    Lang::from_path(Path::new(path)).map(|l| Lang::fence(l.name())).unwrap_or("")
}

/// 지정한 코드를 Markdown으로 (토큰 추정치와 함께). 없으면 빈 글
pub fn pinned(engine: &Engine, root: &Path, mentions: &[Mention], budget_tokens: usize) -> (String, usize) {
    if mentions.is_empty() {
        return (String::new(), 0);
    }
    let limit = budget_tokens * SHARE / 100;
    let per_item = (limit / mentions.len()).max(300);
    let mut out = String::from("# 사용자가 지정한 코드\n\n");
    let mut used = 0usize;
    for m in mentions {
        let section = match m {
            Mention::File { path } => file_section(root, path, per_item),
            Mention::Symbol { name, path, line } => symbol_section(engine, root, name, path, *line, per_item),
        };
        if let Some(s) = section {
            used += estimate_tokens(&s);
            out.push_str(&s);
        }
    }
    let (text, _) = lantern_context::secrets::redact(&out);
    (text, used)
}

fn file_section(root: &Path, path: &str, max_tokens: usize) -> Option<String> {
    let abs = resolve_in_root(root, path).ok()?;
    let rel = rel_path(root, &abs);
    let text = String::from_utf8_lossy(&std::fs::read(&abs).ok()?).into_owned();
    let lines: Vec<&str> = text.lines().collect();
    // 예산 안에 드는 앞부분만
    let mut kept = 0usize;
    let mut tokens = 0usize;
    for l in &lines {
        let t = estimate_tokens(l) + 1;
        if tokens + t > max_tokens {
            break;
        }
        tokens += t;
        kept += 1;
    }
    let why = if kept < lines.len() { format!("{REASON} (앞 {kept}줄 / {}줄)", lines.len()) } else { REASON.to_string() };
    let name = Path::new(&rel).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| rel.clone());
    let mut s = String::new();
    let _ = writeln!(s, "## {rel}\n\n### `{name}` (file, L1-{}) — {why}", kept.max(1));
    let _ = writeln!(s, "```{}\n{}\n```\n", fence(&rel), lines[..kept].join("\n"));
    Some(s)
}

fn symbol_section(engine: &Engine, root: &Path, name: &str, path: &str, line: u32, max_tokens: usize) -> Option<String> {
    let syms = engine.store.symbols_in_file(path).ok()?;
    let sym = syms.iter().filter(|s| s.name == name).min_by_key(|s| s.start_line.abs_diff(line))?;
    let src = std::fs::read(root.join(&sym.path)).ok()?;
    let body = String::from_utf8_lossy(src.get(sym.start_byte..sym.end_byte)?).into_owned();
    let (text, why) = if estimate_tokens(&body) > max_tokens {
        (sym.signature.clone(), format!("{REASON} · 길어서 시그니처만"))
    } else {
        (body, REASON.to_string())
    };
    let mut s = String::new();
    let _ = writeln!(s, "## {}\n\n### `{}` ({}, L{}-{}) — {why}", sym.path, sym.name, sym.kind, sym.start_line, sym.end_line);
    let _ = writeln!(s, "```{}\n{}\n```\n", fence(&sym.path), text.trim_end());
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_files_and_symbols_in_context_card_format() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::create_dir_all(r.join("src")).unwrap();
        std::fs::write(r.join("src/a.ts"), "export function hello() {\n  return 'API_KEY=sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789';\n}\n\nexport function bye() {\n  return 2;\n}\n").unwrap();
        let mut e = Engine::open(r).unwrap();
        e.refresh().unwrap();
        let (md, used) = pinned(
            &e,
            r,
            &[Mention::File { path: "src/a.ts".into() }, Mention::Symbol { name: "bye".into(), path: "src/a.ts".into(), line: 5 }],
            8000,
        );
        assert!(used > 0);
        assert!(md.contains("## src/a.ts\n\n### `a.ts` (file, L1-7) — 사용자가 지정"), "{md}");
        assert!(md.contains("### `bye` (function, L5-7) — 사용자가 지정"), "{md}");
        assert!(!md.contains("abcdefghijklmnop"), "비밀 값은 가린다");
        // 밖으로 나가는 경로는 무시
        let (none, _) = pinned(&e, r, &[Mention::File { path: "../x".into() }], 8000);
        assert!(!none.contains("## "));
    }

    #[test]
    fn long_files_keep_the_head() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::write(r.join("big.py"), (0..2000).map(|i| format!("x{i} = {i}\n")).collect::<String>()).unwrap();
        let e = Engine::open(r).unwrap();
        let (md, _) = pinned(&e, r, &[Mention::File { path: "big.py".into() }], 1000);
        assert!(md.contains("사용자가 지정 (앞 "), "{}", &md[..200]);
    }
}
