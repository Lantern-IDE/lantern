//! 소스 파일에서 심볼 정의와 참조를 추출한다.

use crate::lang::{Lang, TagConfigs};
use crate::tokenize;
use anyhow::Result;
use std::ops::Range;
use tree_sitter_tags::TagsContext;

const BODY_LIMIT: usize = 2000;
const SIGNATURE_LIMIT: usize = 300;

#[derive(Debug, Clone)]
pub struct NewSymbol {
    pub name: String,
    pub kind: String,
    pub start_line: u32,
    pub end_line: u32,
    pub start_byte: usize,
    pub end_byte: usize,
    pub signature: String,
    pub doc: Option<String>,
    /// 검색용: 이름·시그니처·본문 식별자를 분해한 단어
    pub terms: String,
    /// 검색용: 본문 앞부분
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct NewRef {
    pub name: String,
    pub kind: String,
    pub line: u32,
    /// 이 참조를 감싸는 심볼 (`Parsed::symbols`의 인덱스)
    pub enclosing: Option<usize>,
}

#[derive(Debug, Default)]
pub struct Parsed {
    pub symbols: Vec<NewSymbol>,
    pub refs: Vec<NewRef>,
}

pub struct Parser {
    configs: TagConfigs,
    ctx: TagsContext,
}

struct RawDef {
    name: String,
    kind: String,
    range: Range<usize>,
    docs: Option<String>,
}

impl Parser {
    pub fn new() -> Result<Self> {
        Ok(Self { configs: TagConfigs::new()?, ctx: TagsContext::new() })
    }

    pub fn parse(&mut self, lang: Lang, src: &[u8]) -> Result<Parsed> {
        let cfg = self.configs.get(lang);
        let lines = LineIndex::new(src);
        let mut defs: Vec<RawDef> = Vec::new();
        let mut raw_refs: Vec<(String, String, usize)> = Vec::new();

        let (tags, _) = self.ctx.generate_tags(cfg, src, None)?;
        for tag in tags {
            let Ok(tag) = tag else { continue };
            let name = String::from_utf8_lossy(&src[tag.name_range.clone()]).trim().to_string();
            if name.is_empty() || name.len() > 200 {
                continue;
            }
            let kind = cfg.syntax_type_name(tag.syntax_type_id).to_string();
            if tag.is_definition {
                defs.push(RawDef { name, kind, range: tag.range, docs: tag.docs });
            } else {
                raw_refs.push((name, kind, tag.name_range.start));
            }
        }

        // 같은 노드가 여러 패턴에 걸리면 (예: Rust 메서드는 method와 function 둘 다) 하나만 남긴다.
        defs.sort_by(|a, b| a.range.start.cmp(&b.range.start).then(b.range.end.cmp(&a.range.end)));
        defs.dedup_by(|later, kept| {
            let same = later.range == kept.range && later.name == kept.name;
            if same && later.kind == "method" {
                kept.kind = later.kind.clone();
            }
            same
        });

        let symbols = defs
            .iter()
            .map(|d| {
                let full = String::from_utf8_lossy(&src[d.range.clone()]);
                let signature = signature(lang, &full);
                let doc = d
                    .docs
                    .as_deref()
                    .and_then(clean_doc)
                    .or_else(|| leading_comment(src, &lines, d.range.start))
                    .or_else(|| if lang == Lang::Python { python_docstring(&full) } else { None });
                let body: String = full.chars().take(BODY_LIMIT).collect();
                let terms_src = format!("{} {} {} {}", d.name, signature, doc.as_deref().unwrap_or(""), body);
                NewSymbol {
                    name: d.name.clone(),
                    kind: d.kind.clone(),
                    start_line: lines.line_of(d.range.start),
                    end_line: lines.line_of(d.range.end.saturating_sub(1).max(d.range.start)),
                    start_byte: d.range.start,
                    end_byte: d.range.end,
                    signature,
                    doc,
                    terms: tokenize::terms_of(&terms_src).join(" "),
                    body,
                }
            })
            .collect();

        let refs = raw_refs
            .into_iter()
            .map(|(name, kind, pos)| NewRef {
                enclosing: innermost(&defs, pos),
                line: lines.line_of(pos),
                name,
                kind,
            })
            .collect();

        Ok(Parsed { symbols, refs })
    }
}

/// `pos`를 감싸는 가장 안쪽 정의
fn innermost(defs: &[RawDef], pos: usize) -> Option<usize> {
    defs.iter()
        .enumerate()
        .filter(|(_, d)| d.range.start <= pos && pos < d.range.end)
        .min_by_key(|(_, d)| d.range.end - d.range.start)
        .map(|(i, _)| i)
}

/// 정의의 머리 부분. 본문 시작(`{`, Python은 줄 끝의 `:`) 전까지 공백을 정리해 자른다.
fn signature(lang: Lang, full: &str) -> String {
    let mut out = String::new();
    let mut depth = 0i32;
    for c in full.chars() {
        match c {
            '(' | '[' | '<' => depth += 1,
            // `->`, `=>`의 `>`로 음수가 되지 않게 0에서 멈춘다.
            ')' | ']' | '>' => depth = (depth - 1).max(0),
            _ => {}
        }
        if depth <= 0 {
            let stop = match lang {
                Lang::Python => c == '\n',
                _ => c == '{' || c == ';',
            };
            if stop {
                break;
            }
        }
        out.push(c);
        if out.len() > SIGNATURE_LIMIT * 2 {
            break;
        }
    }
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_end_matches([':', ' ', '=']).to_string();
    if trimmed.chars().count() > SIGNATURE_LIMIT {
        trimmed.chars().take(SIGNATURE_LIMIT).collect::<String>() + "…"
    } else {
        trimmed
    }
}

/// JSDoc 등에서 남은 주석 기호(`/**`, `*`, `*/`)를 줄마다 걷어낸다.
fn clean_doc(raw: &str) -> Option<String> {
    let lines: Vec<&str> = raw
        .lines()
        .map(|l| l.trim().trim_start_matches(['/', '*']).trim_end_matches("*/").trim())
        .filter(|l| !l.is_empty())
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// 정의 바로 위의 주석 줄 (속성·데코레이터는 건너뛴다)
fn leading_comment(src: &[u8], lines: &LineIndex, start_byte: usize) -> Option<String> {
    let line = lines.line_of(start_byte) as usize; // 1-based
    let mut collected = Vec::new();
    let mut idx = line.checked_sub(1)?; // 정의 줄의 0-based 인덱스
    for _ in 0..12 {
        idx = idx.checked_sub(1)?;
        let text = String::from_utf8_lossy(lines.text(src, idx)).trim().to_string();
        if text.starts_with("#[") || text.starts_with('@') {
            continue;
        }
        let is_comment = ["///", "//!", "//", "#", "*", "/*", "*/"].iter().any(|p| text.starts_with(p));
        if !is_comment {
            break;
        }
        let stripped = text
            .trim_start_matches(['/', '*', '#', '!'])
            .trim_end_matches("*/")
            .trim()
            .to_string();
        if !stripped.is_empty() {
            collected.push(stripped);
        }
        if collected.len() >= 8 {
            break;
        }
    }
    if collected.is_empty() {
        return None;
    }
    collected.reverse();
    Some(collected.join("\n"))
}

fn python_docstring(full: &str) -> Option<String> {
    let mut lines = full.lines().skip_while(|l| !l.trim_end().ends_with(':')).skip(1);
    let first = lines.by_ref().find(|l| !l.trim().is_empty())?.trim();
    let quote = ["\"\"\"", "'''"].into_iter().find(|q| first.starts_with(q))?;
    let rest = &first[3..];
    if let Some(end) = rest.find(quote) {
        return Some(rest[..end].trim().to_string()).filter(|s| !s.is_empty());
    }
    let mut doc = vec![rest.trim().to_string()];
    for l in lines.take(8) {
        if let Some(end) = l.find(quote) {
            doc.push(l[..end].trim().to_string());
            break;
        }
        doc.push(l.trim().to_string());
    }
    let joined = doc.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
    Some(joined).filter(|s| !s.is_empty())
}

pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(src: &[u8]) -> Self {
        let mut starts = vec![0];
        starts.extend(src.iter().enumerate().filter(|(_, b)| **b == b'\n').map(|(i, _)| i + 1));
        Self { starts }
    }

    /// 바이트 위치의 줄 번호 (1부터)
    pub fn line_of(&self, byte: usize) -> u32 {
        self.starts.partition_point(|&s| s <= byte) as u32
    }

    /// 0-based 줄의 내용
    pub fn text<'a>(&self, src: &'a [u8], idx: usize) -> &'a [u8] {
        let start = self.starts[idx.min(self.starts.len() - 1)];
        let end = self.starts.get(idx + 1).copied().unwrap_or(src.len());
        &src[start..end.max(start)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(p: &Parsed) -> Vec<(String, String)> {
        p.symbols.iter().map(|s| (s.name.clone(), s.kind.clone())).collect()
    }

    #[test]
    fn parses_rust() {
        let src = r#"
/// 설정을 읽는다
pub fn load_config(path: &str) -> Config {
    let raw = read_file(path);
    Config::parse(&raw)
}

pub struct Config { debug: bool }

impl Config {
    pub fn parse(raw: &str) -> Self { Config { debug: raw.is_empty() } }
}
"#;
        let mut p = Parser::new().unwrap();
        let out = p.parse(Lang::Rust, src.as_bytes()).unwrap();
        let n = names(&out);
        assert!(n.contains(&("load_config".into(), "function".into())));
        assert!(n.contains(&("Config".into(), "class".into())));
        assert!(n.contains(&("Config".into(), "impl".into())));
        assert!(n.contains(&("parse".into(), "method".into())));
        assert_eq!(n.iter().filter(|(name, _)| name == "parse").count(), 1);

        let load = out.symbols.iter().find(|s| s.name == "load_config").unwrap();
        assert_eq!(load.doc.as_deref(), Some("설정을 읽는다"));
        assert_eq!(load.signature, "pub fn load_config(path: &str) -> Config");
        assert_eq!(load.start_line, 3);

        let load_idx = out.symbols.iter().position(|s| s.name == "load_config").unwrap();
        let calls: Vec<_> = out
            .refs
            .iter()
            .filter(|r| r.enclosing == Some(load_idx))
            .map(|r| r.name.as_str())
            .collect();
        assert!(calls.contains(&"read_file"));
        assert!(calls.contains(&"parse"));
    }

    #[test]
    fn parses_python() {
        let src = r#"
class UserService:
    def login(self, name, password):
        """사용자 로그인"""
        user = find_user(name)
        return check_password(user, password)
"#;
        let mut p = Parser::new().unwrap();
        let out = p.parse(Lang::Python, src.as_bytes()).unwrap();
        let login = out.symbols.iter().find(|s| s.name == "login").unwrap();
        assert_eq!(login.doc.as_deref(), Some("사용자 로그인"));
        assert_eq!(login.signature, "def login(self, name, password)");
        assert!(out.refs.iter().any(|r| r.name == "find_user"));
    }

    #[test]
    fn parses_typescript_and_tsx() {
        let src = r#"
export interface User { id: string }
export type Role = "admin" | "user";
export enum Level { Low, High }
/** 토큰을 갱신한다 */
export async function refreshToken(user: User): Promise<string> {
  return fetchToken(user.id);
}
export const logout = (user: User) => { clearSession(user); };
class AuthService {
  login(name: string) { return new Session(name); }
}
"#;
        let mut p = Parser::new().unwrap();
        for lang in [Lang::TypeScript, Lang::Tsx] {
            let out = p.parse(lang, src.as_bytes()).unwrap();
            let n: Vec<_> = out.symbols.iter().map(|s| s.name.as_str()).collect();
            for expected in ["User", "Role", "Level", "refreshToken", "logout", "AuthService", "login"] {
                assert!(n.contains(&expected), "{lang:?}: {expected} 없음, {n:?}");
            }
            let refresh = out.symbols.iter().find(|s| s.name == "refreshToken").unwrap();
            assert_eq!(refresh.doc.as_deref(), Some("토큰을 갱신한다"));
            assert!(out.refs.iter().any(|r| r.name == "fetchToken"));
        }
    }

    #[test]
    fn parses_javascript() {
        let src = b"function add(a, b) { return sum([a, b]); }\nconst mul = (a, b) => a * b;\n";
        let mut p = Parser::new().unwrap();
        let out = p.parse(Lang::JavaScript, src).unwrap();
        let n: Vec<_> = out.symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(n, ["add", "mul"]);
        assert!(out.refs.iter().any(|r| r.name == "sum" && r.enclosing == Some(0)));
    }
}
