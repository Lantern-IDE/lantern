//! 에이전트가 끝났다고 한 변경에서 놓친 곳을 찾는다.
//!
//! 편집 전후 본문에서 심볼 변화를 뽑고([`symbol_changes`]), 옛 이름과 바뀐 시그니처가 손대지 않은 파일에
//! 남았는지 코드 그래프로 본다([`left_behind`]). 저장소를 바꾸지 않고 모델도 네트워크도 쓰지 않는다.
//!
//! 옛 이름은 편집 전 본문에서 얻고 참조는 이름으로 찾는다. 갱신된 인덱스만 보면 이름을 바꾼 함수의 호출자가
//! 0개로 나오기 때문이다. 이름 기반이라 결과는 늘 "그럴 수 있다"로만 말한다.

use crate::git::{git, no_window};
use crate::graph::is_test_path;
use crate::indexer::{self, is_unindexable, MAX_FILE_BYTES};
use crate::lang::Lang;
use crate::parse::{NewSymbol, Parser};
use crate::sessions::path_key;
use crate::store::Store;
use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// 이보다 많이 정의된 이름은 누구를 가리키는지 몰라 건너뛴다
pub const MAX_DEFS: i64 = 5;
/// 한 번에 내는 발견의 상한
pub const MAX_FINDINGS: usize = 20;
/// 이름 하나에서 살펴볼 참조 수
const MAX_REFS: usize = 500;
/// 본문이 이보다 짧으면 같아도 이름을 바꾼 증거로 치지 않는다 (공백 뺀 바이트)
const MIN_BODY_FOR_RENAME: usize = 24;
const MIN_SIGNATURE_FOR_RENAME: usize = 16;
const SIMILAR_SIGNATURE: f64 = 0.85;

/// 정밀도 기준은 실제 측정 전에 정해 뒀다 (Dangling 70%, Stale caller 60%).
/// Stale caller는 기준을 넘긴 것을 확인하기 전까지 막지 않는다.
pub const BLOCK_DANGLING: bool = true;
pub const BLOCK_STALE: bool = false;

/// 저장소 밖으로 나가지 않는 설명. 찾는 방법과 못 보는 것
pub const METHOD: &str = "Name based: compares symbols before and after the edit, then looks up the old name in the code graph. No model and no network.";
pub const NOT_COVERED: &str = "Not covered: reads through the shell, dynamic dispatch, dependency injection, name collisions.";

/// 흔한 이름. 표준 라이브러리 메서드와 겹쳐 지워져도 참조가 남아 보인다 (소문자로 비교)
const GENERIC_NAMES: &[&str] = &[
    "add", "all", "any", "apply", "bind", "build", "call", "clear", "clone", "close", "copy", "count", "create", "data", "debug", "default",
    "delete", "drop", "emit", "equals", "error", "exec", "execute", "fetch", "filter", "find", "first", "flush", "format", "from", "get",
    "handle", "hash", "index", "info", "init", "insert", "into", "item", "items", "iter", "join", "key", "keys", "last", "len", "length", "list",
    "load", "log", "main", "map", "merge", "name", "new", "next", "open", "parse", "pop", "print", "process", "push", "put", "read", "reduce",
    "remove", "render", "replace", "reset", "resolve", "run", "save", "send", "set", "setup", "size", "sort", "split", "start", "stop", "test",
    "then", "to_string", "tostring", "trim", "update", "value", "values", "warn", "write", "__init__", "constructor",
];

/// 파일 하나에서 일어난 심볼 변화. 시그니처 글은 메모리에만 두고 출력하지 않는다
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolChange {
    /// 이름이 파일에서 사라졌다
    Removed { name: String, kind: String, signature: String, line: u32, renamed_to: Option<String> },
    /// 호출하는 쪽에 보이는 모양이 달라졌다
    Signature { name: String, old: String, new: String, line: u32 },
    /// 본문만 바뀌었다. 놓친 호출자로 보고하지 않는다
    Body { name: String },
}

/// 호출하는 쪽에서 본 정의의 모양
#[derive(Debug, Clone, PartialEq, Eq)]
struct CallShape {
    /// 이름 앞뒤 (반환 타입, `async`, 제네릭). 공개 범위 키워드는 뺐다
    head: String,
    params: Vec<Param>,
    /// 닫는 괄호 뒤 (반환 타입, `throws`, `where`)
    ret: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Param {
    /// 이름이 호출에 상관없는 언어는 타입만
    shape: String,
    default: Option<String>,
}

impl Param {
    /// 끝에 새로 붙어도 원래 호출이 그대로 되는가
    fn optional(&self) -> bool {
        self.default.is_some() || self.shape.starts_with('?') || self.shape.starts_with('*') || self.shape.contains("...")
    }
}

struct Shape {
    /// 매개변수 목록이 있는 정의만
    call: Option<CallShape>,
    /// 머리를 뺀 본문의 해시. 공백과 주석, 자기 이름은 지웠다
    body: blake3::Hash,
    body_len: usize,
    /// 이름을 지운 머리 글자
    header: String,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_callable(kind: &str) -> bool {
    matches!(kind, "function" | "method")
}

/// `i`가 문자열 시작이면 끝난 다음 위치. Rust의 `'`는 수명 표기라 문자열로 보지 않는다
fn skip_string(lang: Lang, b: &[u8], i: usize) -> Option<usize> {
    let q = b[i];
    if !(q == b'"' || q == b'`' || (q == b'\'' && lang != Lang::Rust)) {
        return None;
    }
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            c if c == q => return Some(j + 1),
            b'\n' if q != b'`' => return Some(j),
            _ => j += 1,
        }
    }
    Some(b.len())
}

fn strip_comments(lang: Lang, src: &str) -> String {
    let b = src.as_bytes();
    let python = lang == Lang::Python;
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if let Some(end) = skip_string(lang, b, i) {
            out.extend_from_slice(&b[i..end.min(b.len())]);
            i = end;
            continue;
        }
        let line_comment = if python { b[i] == b'#' } else { b[i..].starts_with(b"//") };
        if line_comment {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if !python && b[i..].starts_with(b"/*") {
            let len = b[i + 2..].windows(2).position(|w| w == b"*/").map_or(b.len() - i, |p| p + 4);
            i += len;
            out.push(b' ');
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 여는 괄호 `open`의 짝 다음 위치
fn balanced_end(lang: Lang, b: &[u8], open: usize) -> Option<usize> {
    let (o, c) = match b[open] {
        b'(' => (b'(', b')'),
        b'[' => (b'[', b']'),
        b'{' => (b'{', b'}'),
        _ => return None,
    };
    let (mut depth, mut i) = (0, open);
    while i < b.len() {
        if let Some(end) = skip_string(lang, b, i) {
            i = end;
            continue;
        }
        if b[i] == o {
            depth += 1;
        } else if b[i] == c {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
        i += 1;
    }
    None
}

/// 애너테이션·속성·데코레이터를 걷어낸다 (`@Override`, `@Get("/x")`, `#[x]`, C#의 `[x]`)
fn strip_marks(lang: Lang, s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let (mut i, mut start, mut depth) = (0, 0, 0i32);
    if lang == Lang::CSharp {
        loop {
            let skip = s[i..].len() - s[i..].trim_start().len();
            if b.get(i + skip) != Some(&b'[') {
                break;
            }
            let Some(end) = balanced_end(lang, b, i + skip) else { break };
            i = end;
            start = end;
        }
    }
    while i < b.len() {
        if let Some(end) = skip_string(lang, b, i) {
            i = end;
            continue;
        }
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'@' if depth == 0 => {
                out.push_str(&s[start..i]);
                i += 1;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'.')) {
                    i += 1;
                }
                if b.get(i) == Some(&b'(') {
                    i = balanced_end(lang, b, i).unwrap_or(b.len());
                }
                start = i;
                continue;
            }
            b'#' if depth == 0 && b.get(i + 1) == Some(&b'[') => {
                out.push_str(&s[start..i]);
                i = balanced_end(lang, b, i + 1).unwrap_or(b.len());
                start = i;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out.push_str(&s[start.min(s.len())..]);
    out
}

/// 공백을 지운다. 단어 둘 사이에서만 한 칸 남긴다
fn squeeze(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut gap = false;
    for c in s.chars() {
        if c.is_whitespace() {
            gap = true;
            continue;
        }
        if gap && is_word(c) && out.chars().next_back().is_some_and(is_word) {
            out.push(' ');
        }
        gap = false;
        out.push(c);
    }
    out
}

/// 머리의 끝. 본문 시작(`{`, Python의 `:`), `;`, 화살표, 값을 주는 `=` 앞까지.
/// 꺾쇠가 닫히지 않으면(`operator<` 등) 꺾쇠를 세지 않고 다시 본다.
fn header_end(lang: Lang, kind: &str, s: &str) -> usize {
    scan_header(lang, kind, s, true).or_else(|| scan_header(lang, kind, s, false)).unwrap_or(s.len())
}

fn scan_header(lang: Lang, kind: &str, s: &str, track_angle: bool) -> Option<usize> {
    let b = s.as_bytes();
    let python = lang == Lang::Python;
    let (mut i, mut depth, mut angle) = (0, 0i32, 0i32);
    let mut seen_params = false;
    while i < b.len() {
        if let Some(end) = skip_string(lang, b, i) {
            i = end;
            continue;
        }
        let c = b[i];
        let arrow_tail = i > 0 && matches!(b[i - 1], b'-' | b'=');
        match c {
            b'(' | b'[' => depth += 1,
            b')' | b']' => {
                depth = (depth - 1).max(0);
                seen_params |= depth == 0 && c == b')';
            }
            _ if depth > 0 => {}
            b'<' if track_angle => angle += 1,
            b'>' if track_angle && !arrow_tail => angle = (angle - 1).max(0),
            _ if angle > 0 => {}
            b'{' | b';' if !python => return Some(i),
            b':' if python => return Some(i),
            b'=' if b.get(i + 1) == Some(&b'>') => return Some(i),
            // `const f = (a) => …`의 `=`는 머리 안이다. 매개변수 뒤나 값 정의의 `=`는 머리의 끝
            b'=' if seen_params || !is_callable(kind) => return Some(i),
            _ => {}
        }
        i += 1;
    }
    (!track_angle || angle == 0).then_some(b.len())
}

/// 괄호·꺾쇠·따옴표 밖의 글자마다 `f`를 부른다. `Some`이 나오면 멈춘다
fn at_top<T>(lang: Lang, s: &str, mut f: impl FnMut(usize, u8) -> Option<T>) -> Option<T> {
    let b = s.as_bytes();
    let (mut i, mut depth, mut angle) = (0, 0i32, 0i32);
    while i < b.len() {
        if let Some(end) = skip_string(lang, b, i) {
            i = end;
            continue;
        }
        let c = b[i];
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'<' if depth == 0 => angle += 1,
            b'>' if depth == 0 && angle > 0 && !matches!(b[i.saturating_sub(1)], b'-' | b'=') => angle -= 1,
            _ if depth == 0 && angle == 0 => {
                if let Some(v) = f(i, c) {
                    return Some(v);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn find_word(lang: Lang, s: &str, word: &str) -> Option<usize> {
    let b = s.as_bytes();
    at_top(lang, s, |i, _| {
        let hit = b[i..].starts_with(word.as_bytes())
            && !s[..i].chars().next_back().is_some_and(is_word)
            && !s[i + word.len()..].chars().next().is_some_and(is_word);
        hit.then_some(i)
    })
}

/// 매개변수를 여는 괄호. 함수가 아니면 이름 바로 뒤(제네릭 다음)에 있어야 한다:
/// Kotlin의 `class A : B(x)`는 생성자 인자가 아니다.
fn open_paren(b: &[u8], from: usize, callable: bool) -> Option<usize> {
    let mut angle = 0i32;
    for i in from..b.len() {
        match b[i] {
            b'(' if angle == 0 => return Some(i),
            b'<' => angle += 1,
            b'>' if !matches!(b[i - 1], b'-' | b'=') => angle -= 1,
            c if callable || angle > 0 || c.is_ascii_whitespace() => {}
            _ => return None,
        }
    }
    None
}

fn split_commas(lang: Lang, s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    at_top(lang, s, |i, c| {
        if c == b',' {
            parts.push(&s[start..i]);
            start = i + 1;
        }
        None::<()>
    });
    parts.push(&s[start..]);
    parts.retain(|p| !p.trim().is_empty());
    parts
}

/// (선언, 기본값)
fn split_default(lang: Lang, p: &str) -> (&str, Option<&str>) {
    let b = p.as_bytes();
    let eq = at_top(lang, p, |i, c| {
        let simple = c == b'='
            && !matches!(b.get(i + 1), Some(b'=' | b'>'))
            && !matches!(b.get(i.wrapping_sub(1)), Some(b'=' | b'!' | b'<' | b'>'));
        simple.then_some(i)
    });
    match eq {
        Some(i) => (&p[..i], Some(&p[i + 1..])),
        None => (p, None),
    }
}

fn top_colon(lang: Lang, s: &str) -> Option<usize> {
    let b = s.as_bytes();
    at_top(lang, s, |i, c| (c == b':' && b.get(i + 1) != Some(&b':') && b.get(i.wrapping_sub(1)) != Some(&b':')).then_some(i))
}

/// `이름: 타입` 언어. 이름은 호출에 상관없으니 타입만 남긴다
fn typed_shape(lang: Lang, decl: &str) -> String {
    let dots = if decl.starts_with("...") { "..." } else { "" };
    match top_colon(lang, decl) {
        Some(i) => {
            let mark = if decl[..i].ends_with('?') { "?" } else { "" };
            format!("{dots}{mark}:{}", &decl[i + 1..])
        }
        None if matches!(lang, Lang::JavaScript | Lang::TypeScript | Lang::Tsx) && decl.trim_start_matches('.').chars().all(is_word) => {
            format!("{dots}_")
        }
        None => decl.to_string(),
    }
}

/// `타입 이름` 언어: 끝의 이름을 뗀다 (`char*s` → `char*`)
fn drop_trailing_name(decl: &str) -> String {
    let decl = decl.strip_prefix("final ").unwrap_or(decl).trim_end();
    let cut = decl.rfind(|c: char| !is_word(c)).map_or(0, |i| i + decl[i..].chars().next().map_or(1, char::len_utf8));
    if cut == 0 {
        decl.to_string()
    } else {
        decl[..cut].trim_end().to_string()
    }
}

/// Go: 전부 이름이 있거나 전부 없다. `a, b int`에서 `a`는 뒤의 타입을 물려받는다
fn go_params(parts: &[&str]) -> Vec<Param> {
    let typed = |p: &str| p.trim().split_once(char::is_whitespace).map(|(_, t)| squeeze(t));
    if !parts.iter().any(|p| typed(p).is_some()) {
        return parts.iter().map(|p| Param { shape: squeeze(p), default: None }).collect();
    }
    let mut ty = String::new();
    let mut out: Vec<Param> = parts
        .iter()
        .rev()
        .map(|p| {
            if let Some(t) = typed(p) {
                ty = t;
            }
            Param { shape: ty.clone(), default: None }
        })
        .collect();
    out.reverse();
    out
}

fn param(lang: Lang, raw: &str) -> Param {
    let raw = strip_marks(lang, raw);
    let (decl, default) = split_default(lang, &raw);
    let decl = squeeze(decl);
    let shape = match lang {
        // 키워드 인자로 부를 수 있어 이름이 중요하다. 타입 힌트는 실행에 상관없다
        Lang::Python => top_colon(lang, &decl).map_or(decl.as_str(), |i| &decl[..i]).to_string(),
        Lang::Java | Lang::CSharp | Lang::C | Lang::Cpp => drop_trailing_name(&decl),
        _ => typed_shape(lang, &decl),
    };
    Param { shape, default: default.map(squeeze) }
}

fn visibility_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"\bpub\b(?:\s*\([^)]*\))?|\b(?:public|private|protected|internal|export|default|override|final|abstract|virtual|sealed|open|synchronized|inline|declare|native)\b",
        )
        .expect("정규식")
    })
}

fn call_shape(lang: Lang, kind: &str, name: &str, header: &str) -> Option<CallShape> {
    // Python의 `class A(B)`는 상위 클래스다
    if lang == Lang::Python && kind == "class" {
        return None;
    }
    let header = strip_marks(lang, header);
    let b = header.as_bytes();
    let at = find_word(lang, &header, name)?;
    let after = at + name.len();
    let open = open_paren(b, after, is_callable(kind))?;
    let close = balanced_end(lang, b, open)?;
    let head = format!("{} {}", &header[..at], &header[after..open]);
    let head = squeeze(&visibility_re().replace_all(&head, " "));
    let mut ret = squeeze(&header[close..]);
    // 생성자 초기화 목록은 호출하는 쪽과 상관없다
    if matches!(lang, Lang::Cpp | Lang::CSharp) && ret.starts_with(':') {
        ret.clear();
    }
    let parts = split_commas(lang, &header[open + 1..close - 1]);
    let params = if lang == Lang::Go { go_params(&parts) } else { parts.iter().map(|p| param(lang, p)).collect() };
    Some(CallShape { head, params, ret })
}

fn shape_of(lang: Lang, kind: &str, name: &str, raw: &str) -> Shape {
    let text = strip_comments(lang, raw);
    let (header, body) = text.split_at(header_end(lang, kind, &text));
    let body: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    let body = body.replace(name, "\u{0}");
    Shape {
        call: call_shape(lang, kind, name, header),
        body: blake3::hash(body.as_bytes()),
        body_len: body.len(),
        header: squeeze(header).replace(name, ""),
    }
}

/// 원래 호출이 그대로 되는 변화인가: 끝에 선택 매개변수가 붙거나 기존 매개변수에 기본값이 생긴 정도
fn compatible(lang: Lang, old: &CallShape, new: &CallShape) -> bool {
    // Python 반환 힌트는 새로 달아도 호출에 상관없다
    let ret_same = old.ret == new.ret || (lang == Lang::Python && old.ret.is_empty());
    old.head == new.head
        && ret_same
        && new.params.len() >= old.params.len()
        && old.params.iter().zip(&new.params).all(|(o, n)| o.shape == n.shape && (o.default.is_none() || o.default == n.default))
        && new.params[old.params.len()..].iter().all(Param::optional)
}

/// 글자 두 개씩 묶어 비교한 유사도 (0~1)
fn dice(a: &str, b: &str) -> f64 {
    let grams = |s: &str| -> HashMap<(char, char), i32> {
        let chars: Vec<char> = s.chars().collect();
        let mut m = HashMap::new();
        for w in chars.windows(2) {
            *m.entry((w[0], w[1])).or_default() += 1;
        }
        m
    };
    let (x, y) = (grams(a), grams(b));
    let total: i32 = x.values().sum::<i32>() + y.values().sum::<i32>();
    if total == 0 {
        return 0.0;
    }
    let both: i32 = x.iter().map(|(k, n)| (*n).min(y.get(k).copied().unwrap_or(0))).sum();
    2.0 * both as f64 / total as f64
}

fn signature_similarity(a: &Shape, b: &Shape) -> f64 {
    if let (Some(x), Some(y)) = (&a.call, &b.call) {
        if x == y && !x.params.is_empty() {
            return 1.0;
        }
    }
    if a.header.len() < MIN_SIGNATURE_FOR_RENAME || b.header.len() < MIN_SIGNATURE_FOR_RENAME {
        return 0.0;
    }
    dice(&a.header, &b.header)
}

struct Item<'a> {
    sym: &'a NewSymbol,
    shape: Shape,
    paired: bool,
}

fn items<'a>(lang: Lang, src: &[u8], syms: &'a [NewSymbol]) -> Vec<Item<'a>> {
    syms.iter()
        .map(|s| {
            let text = String::from_utf8_lossy(src.get(s.start_byte..s.end_byte).unwrap_or(&[]));
            Item { sym: s, shape: shape_of(lang, &s.kind, &s.name, &text), paired: false }
        })
        .collect()
}

/// 사라진 심볼의 새 이름 후보: 옛 파일에 없던 같은 종류의 심볼 중 본문이 같거나 시그니처가 매우 비슷한 것
fn find_rename(old: &Item<'_>, new: &[Item<'_>], old_names: &HashSet<&str>, claimed: &mut HashSet<usize>) -> Option<String> {
    let candidates = || {
        new.iter()
            .enumerate()
            .filter(|(i, n)| !claimed.contains(i) && n.sym.kind == old.sym.kind && !old_names.contains(n.sym.name.as_str()))
    };
    let same_body = candidates().find(|(_, n)| old.shape.body_len >= MIN_BODY_FOR_RENAME && n.shape.body == old.shape.body).map(|(i, _)| i);
    let hit = same_body.or_else(|| {
        let scored: Vec<(usize, f64)> =
            candidates().map(|(i, n)| (i, signature_similarity(&old.shape, &n.shape))).filter(|(_, s)| *s >= SIMILAR_SIGNATURE).collect();
        let best = scored.iter().map(|(_, s)| *s).fold(0.0, f64::max);
        // 똑같이 비슷한 후보가 여럿이면 어느 쪽인지 모른다
        let mut top = scored.iter().filter(|(_, s)| *s == best);
        match (top.next(), top.next()) {
            (Some((i, _)), None) => Some(*i),
            _ => None,
        }
    })?;
    claimed.insert(hit);
    Some(new[hit].sym.name.clone())
}

fn diff_symbols(lang: Lang, before: &[u8], old_syms: &[NewSymbol], after: &[u8], new_syms: &[NewSymbol]) -> Vec<SymbolChange> {
    let mut old = items(lang, before, old_syms);
    let mut new = items(lang, after, new_syms);
    let old_names: HashSet<&str> = old_syms.iter().map(|s| s.name.as_str()).collect();
    let new_names: HashSet<&str> = new_syms.iter().map(|s| s.name.as_str()).collect();
    let mut by_key: HashMap<(&str, &str), Vec<usize>> = HashMap::new();
    let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
    for (j, s) in new_syms.iter().enumerate() {
        by_key.entry((&s.name, &s.kind)).or_default().push(j);
        by_name.entry(&s.name).or_default().push(j);
    }

    // 이름·종류가 같은 것끼리 짝짓는다. 오버로드 순서가 바뀌어도 흔들리지 않게 모양이 같은 쌍부터
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for pass in 0..4 {
        for (i, o) in old.iter_mut().enumerate() {
            if o.paired {
                continue;
            }
            let s = o.sym;
            // 마지막은 종류만 바뀐 것 (함수 ↔ 메서드)
            let cands = if pass == 3 { by_name.get(s.name.as_str()) } else { by_key.get(&(s.name.as_str(), s.kind.as_str())) };
            let hit = cands.into_iter().flatten().copied().find(|&j| {
                !new[j].paired
                    && match pass {
                        0 => o.shape.call == new[j].shape.call && o.shape.body == new[j].shape.body,
                        1 => o.shape.call == new[j].shape.call,
                        _ => true,
                    }
            });
            if let Some(j) = hit {
                o.paired = true;
                new[j].paired = true;
                pairs.push((i, j));
            }
        }
    }

    let mut signature = Vec::new();
    let mut bodies: Vec<(&str, usize, usize)> = Vec::new();
    let mut changed: Vec<(usize, usize)> = Vec::new();
    for &(i, j) in &pairs {
        let (o, n) = (&old[i], &new[j]);
        let differs = match (&o.shape.call, &n.shape.call) {
            (Some(a), Some(b)) => !compatible(lang, a, b),
            (None, None) => false,
            // 구조체 ↔ 튜플 구조체는 모양이 바뀐 것이다. 함수인데 괄호를 못 찾았으면 모른다
            _ => !is_callable(&n.sym.kind),
        };
        if differs {
            signature.push(SymbolChange::Signature {
                name: n.sym.name.clone(),
                old: o.sym.signature.clone(),
                new: n.sym.signature.clone(),
                line: n.sym.start_line,
            });
        }
        if differs || o.shape.body != n.shape.body {
            changed.push((n.sym.start_byte, n.sym.end_byte));
            if !differs {
                bodies.push((&n.sym.name, n.sym.start_byte, n.sym.end_byte));
            }
        }
    }

    let mut out = Vec::new();
    let mut claimed = HashSet::new();
    let mut seen = HashSet::new();
    for o in old.iter().filter(|o| !o.paired && !new_names.contains(o.sym.name.as_str())) {
        if seen.insert((o.sym.name.as_str(), o.sym.kind.as_str())) {
            out.push(SymbolChange::Removed {
                name: o.sym.name.clone(),
                kind: o.sym.kind.clone(),
                signature: o.sym.signature.clone(),
                line: o.sym.start_line,
                renamed_to: find_rename(o, &new, &old_names, &mut claimed),
            });
        }
    }
    out.append(&mut signature);
    // 메서드가 바뀌면 감싼 클래스도 바뀐 것으로 나오니 가장 안쪽만 남긴다
    for (name, start, end) in bodies {
        let inner = changed.iter().any(|&(s, e)| (s, e) != (start, end) && start <= s && e <= end);
        if !inner {
            out.push(SymbolChange::Body { name: name.to_string() });
        }
    }
    out
}

/// 편집 전후 본문에서 심볼 변화를 뽑는다. 새로 문법이 깨진 파일은 심볼이 사라져 보이므로 비워 돌려준다
pub fn symbol_changes(parser: &mut Parser, lang: Lang, before: &[u8], after: &[u8]) -> Result<Vec<SymbolChange>> {
    Ok(file_changes(parser, "", lang, before, after)?.changes)
}

/// 파일 하나의 변화
#[derive(Debug, Clone)]
pub struct FileChanges {
    /// 루트 기준 상대 경로
    pub path: String,
    pub lang: Lang,
    pub changes: Vec<SymbolChange>,
    /// 끝 상태에서 이 파일에 정의된 이름
    pub after_defs: HashSet<String>,
}

impl FileChanges {
    fn may_leave_behind(&self) -> bool {
        self.changes.iter().any(|c| !matches!(c, SymbolChange::Body { .. }))
    }
}

pub fn file_changes(parser: &mut Parser, path: &str, lang: Lang, before: &[u8], after: &[u8]) -> Result<FileChanges> {
    let old = parser.parse(lang, before)?;
    let new = parser.parse(lang, after)?;
    let mut after_defs: HashSet<String> = new.symbols.iter().map(|s| s.name.clone()).collect();
    let changes = if new.has_error && !old.has_error {
        // 믿을 수 없으니 옛 이름도 남았다고 본다
        after_defs.extend(old.symbols.iter().map(|s| s.name.clone()));
        Vec::new()
    } else {
        diff_symbols(lang, before, &old.symbols, after, &new.symbols)
    };
    Ok(FileChanges { path: path.to_string(), lang, changes, after_defs })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    High,
    Medium,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    Dangling,
    StaleCaller,
}

/// 놓쳤을 수 있는 곳 하나. 참조 파일마다 하나씩
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub kind: FindingKind,
    pub confidence: Confidence,
    /// 바뀐 심볼
    pub symbol: String,
    pub renamed_to: Option<String>,
    /// 심볼이 바뀐 파일
    pub changed_file: String,
    /// 참조가 남은 파일과 줄 (그 파일의 첫 참조)
    pub path: String,
    pub line: u32,
    /// 참조를 감싼 심볼. 파일 맨 위 코드면 없다
    pub caller: Option<String>,
    /// 그 파일 안의 참조 수
    pub refs_in_file: u32,
    pub in_test: bool,
    pub why: String,
}

impl Finding {
    /// 에이전트를 멈춰 세울 만큼 믿는가
    pub fn blocks(&self) -> bool {
        match self.kind {
            FindingKind::Dangling => BLOCK_DANGLING,
            FindingKind::StaleCaller => BLOCK_STALE,
        }
    }
}

/// 너무 짧거나 흔한 이름은 이름만으로 잇지 않는다
pub fn skip_name(name: &str) -> bool {
    name.chars().count() < 3 || GENERIC_NAMES.contains(&name.to_lowercase().as_str())
}

fn key(path: &str) -> String {
    path_key(Path::new(path))
}

pub(crate) fn same_family(path: &str, family: &str) -> bool {
    Lang::from_path(Path::new(path)).is_some_and(|l| Lang::family(l.name()) == family)
}

/// 같은 언어 계열에 이 이름의 정의가 끝 상태에도 있는가.
/// 인덱스가 기준 커밋이어도 현재여도 되게, 손댄 파일은 인덱스 대신 끝 상태(`after_defs`)로 본다.
fn definition_remains(store: &Store, name: &str, family: &str, changes: &[FileChanges], touched: &HashSet<String>) -> Result<bool> {
    if changes.iter().any(|f| Lang::family(f.lang.name()) == family && f.after_defs.contains(name)) {
        return Ok(true);
    }
    let known: HashSet<String> = changes.iter().map(|f| key(&f.path)).collect();
    for d in store.symbols_by_name(name, MAX_DEFS as usize + 1)? {
        let k = key(&d.path);
        // 끝 상태를 모르는 파일(파싱 실패 등)의 정의는 남았다고 친다
        if Lang::family(&d.lang) == family && (!touched.contains(&k) || !known.contains(&k)) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn why_dangling(name: &str, renamed_to: Option<&str>, file: &str) -> String {
    let rename = renamed_to.map(|n| format!(" (looks renamed to `{n}`)")).unwrap_or_default();
    format!("`{name}` was removed from {file}{rename} and no definition of that name is left, but this file still refers to it. Name based, may be a different `{name}`.")
}

fn why_stale(name: &str, file: &str, reads_known: bool) -> String {
    let seen = if reads_known { "was not edited or read in this session" } else { "was not edited" };
    format!("The signature of `{name}` changed in {file} and this caller {seen}. Name based, may call a different `{name}`.")
}

/// 전부 모은다 (상한 전)
pub(crate) fn collect_findings(store: &Store, changes: &[FileChanges], touched: &[String], read: &[String], reads_known: bool) -> Result<Vec<Finding>> {
    let mut touched: HashSet<String> = touched.iter().map(|p| key(p)).collect();
    touched.extend(changes.iter().map(|f| key(&f.path)));
    let read: HashSet<String> = read.iter().map(|p| key(p)).collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for fc in changes {
        let family = Lang::family(fc.lang.name());
        for change in &fc.changes {
            let (name, renamed_to, removed) = match change {
                // 모듈과 impl 블록은 호출 참조로 이어지지 않는다
                SymbolChange::Removed { kind, .. } if matches!(kind.as_str(), "module" | "impl") => continue,
                SymbolChange::Removed { name, renamed_to, .. } => (name, renamed_to.as_deref(), true),
                SymbolChange::Signature { name, .. } => (name, None, false),
                SymbolChange::Body { .. } => continue,
            };
            if skip_name(name) || !seen.insert((removed, family, name.as_str())) || store.count_defs(name)? > MAX_DEFS {
                continue;
            }
            if removed && definition_remains(store, name, family, changes, &touched)? {
                continue;
            }
            // 자기 파일에 같은 이름의 정의가 있으면 그 참조는 십중팔구 자기 것을 가리킨다
            let own_defs: HashSet<String> = store.symbols_by_name(name, MAX_DEFS as usize + 1)?.iter().map(|d| key(&d.path)).collect();
            let mut by_file: Vec<Finding> = Vec::new();
            for r in store.references(name, MAX_REFS)? {
                let k = key(&r.path);
                if touched.contains(&k) || own_defs.contains(&k) || !same_family(&r.path, family) {
                    continue;
                }
                // 타입 표기는 호출이 아니라서 시그니처가 바뀌어도 고칠 일이 없다
                if !removed && (!matches!(r.kind.as_str(), "call" | "class") || (reads_known && read.contains(&k))) {
                    continue;
                }
                if let Some(last) = by_file.last_mut().filter(|f| f.path == r.path) {
                    last.refs_in_file += 1;
                    continue;
                }
                by_file.push(Finding {
                    kind: if removed { FindingKind::Dangling } else { FindingKind::StaleCaller },
                    confidence: if removed { Confidence::High } else { Confidence::Medium },
                    symbol: name.clone(),
                    renamed_to: renamed_to.map(str::to_string),
                    changed_file: fc.path.clone(),
                    in_test: is_test_path(&r.path),
                    why: if removed { why_dangling(name, renamed_to, &fc.path) } else { why_stale(name, &fc.path, reads_known) },
                    path: r.path,
                    line: r.line,
                    caller: r.enclosing,
                    refs_in_file: 1,
                });
            }
            out.append(&mut by_file);
        }
    }
    out.sort_by(|a, b| {
        (a.confidence, a.in_test, &a.symbol, &a.path, a.line).cmp(&(b.confidence, b.in_test, &b.symbol, &b.path, b.line))
    });
    Ok(out)
}

/// 바뀐 심볼의 참조가 손대지 않은 파일에 남았는지 본다. 믿음이 큰 순서로 최대 [`MAX_FINDINGS`]개.
///
/// - `touched`, `read`: 루트 기준 상대 경로. Windows에서는 대소문자를 가리지 않는다
/// - `reads_known`: false면 읽은 파일을 모른다 (Codex). 이때 `read`는 보지 않는다
pub fn left_behind(store: &Store, changes: &[FileChanges], touched: &[String], read: &[String], reads_known: bool) -> Result<Vec<Finding>> {
    let mut all = collect_findings(store, changes, touched, read, reads_known)?;
    all.truncate(MAX_FINDINGS);
    Ok(all)
}

// ── 저장소 읽기 ──────────────────────────────────────────

/// git 저장소 맨 위 폴더 (절대 경로)
pub fn repo_root(dir: &Path) -> Result<PathBuf> {
    let top = git(dir, &["rev-parse", "--show-toplevel"]).context("git 저장소가 아닙니다")?;
    Ok(std::path::absolute(top.trim())?)
}

/// 리비전을 커밋 해시로
pub fn resolve_commit(root: &Path, rev: &str) -> Result<String> {
    if rev.is_empty() || rev.starts_with('-') || rev.contains(char::is_whitespace) {
        bail!("기준 커밋이 이상합니다: {rev}");
    }
    let spec = format!("{rev}^{{commit}}");
    let hash = git(root, &["rev-parse", "--verify", "-q", &spec]).with_context(|| format!("기준 커밋을 찾지 못했습니다: {rev}"))?;
    Ok(hash.trim().to_string())
}

/// `commit` 대비 바뀐 파일 (스테이징, 작업 트리, 추적하지 않는 새 파일).
/// 저장소를 바꾸지 않게 인덱스 잠금과 fsmonitor를 쓰지 않는다.
pub fn changed_files(root: &Path, commit: &str) -> Result<Vec<String>> {
    const READ_ONLY: [&str; 3] = ["--no-optional-locks", "-c", "core.fsmonitor=false"];
    let run = |args: &[&str]| git(root, &[&READ_ONLY[..], args].concat());
    let diff = run(&["diff", "--name-only", "--no-renames", "--no-ext-diff", "--ignore-submodules", "-z", commit, "--"])?;
    let new = run(&["ls-files", "--others", "--exclude-standard", "-z"])?;
    let mut files: Vec<String> = diff.split('\0').chain(new.split('\0')).filter(|p| !p.is_empty()).map(str::to_string).collect();
    files.sort();
    files.dedup();
    Ok(files)
}

/// 커밋의 파일 본문 여러 개를 프로세스 하나로 읽는다. 없는 파일은 결과에 없다
fn read_blobs(root: &Path, commit: &str, paths: &[String]) -> Result<HashMap<String, Vec<u8>>> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(root).arg("--no-optional-locks");
    blobs_with(cmd, commit, paths)
}

/// `cmd`는 하위 명령 앞까지 채운 git. 부검의 임시 클론처럼 환경을 따로 잡아야 할 때 넘긴다
pub(crate) fn blobs_with(mut cmd: Command, commit: &str, paths: &[String]) -> Result<HashMap<String, Vec<u8>>> {
    if paths.is_empty() {
        return Ok(HashMap::new());
    }
    cmd.args(["cat-file", "--batch"]);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = no_window(&mut cmd).spawn().context("git 실행")?;
    let mut stdin = child.stdin.take().context("git 입력")?;
    let requests: String = paths.iter().map(|p| format!("{commit}:{p}\n")).collect();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(requests.as_bytes());
    });
    let mut out = Vec::new();
    child.stdout.take().context("git 출력")?.read_to_end(&mut out)?;
    let _ = writer.join();
    child.wait()?;

    let mut blobs = HashMap::new();
    let mut pos = 0;
    for p in paths {
        let Some(nl) = out[pos.min(out.len())..].iter().position(|&b| b == b'\n') else { break };
        let head = String::from_utf8_lossy(&out[pos..pos + nl]).into_owned();
        pos += nl + 1;
        let mut words = head.rsplit(' ');
        let (size, ty) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
        // `<이름> missing`은 크기가 없다
        let Ok(size) = size.parse::<usize>() else { continue };
        let Some(body) = out.get(pos..pos + size) else { break };
        if ty == "blob" {
            blobs.insert(p.clone(), body.to_vec());
        }
        pos += size + 1;
    }
    Ok(blobs)
}

/// 파일들의 시작 본문과 지금 디스크 본문을 비교한다.
/// `known`은 로그에 있던 시작 본문 (`path_key` 기준). 없으면 `commit`에서 읽고, 거기도 없으면 새 파일이다.
pub fn analyze_files(root: &Path, commit: &str, files: &[String], known: &HashMap<String, String>, parser: &mut Parser) -> Result<Vec<FileChanges>> {
    let code: Vec<&String> = files.iter().filter(|f| Lang::from_path(Path::new(f)).is_some()).collect();
    let from_git: Vec<String> = code.iter().filter(|f| !known.contains_key(&key(f))).map(|f| (*f).clone()).collect();
    let blobs = read_blobs(root, commit, &from_git)?;
    let mut out = Vec::new();
    for f in code {
        let Some(lang) = Lang::from_path(Path::new(f)) else { continue };
        let before: &[u8] = match known.get(&key(f)) {
            Some(text) => text.as_bytes(),
            None => blobs.get(f).map_or(&[], Vec::as_slice),
        };
        let after = match std::fs::read(root.join(f)) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => continue,
        };
        let too_big = |b: &[u8]| b.len() as u64 > MAX_FILE_BYTES || is_unindexable(b);
        if before == after.as_slice() || too_big(before) || too_big(&after) {
            continue;
        }
        // 파싱하지 못한 파일은 건너뛴다. 손댄 파일로는 남는다
        if let Ok(fc) = file_changes(parser, f, lang, before, &after) {
            out.push(fc);
        }
    }
    Ok(out)
}

/// 이 검사가 쓸 인덱스. 저장소에 `.lantern` 인덱스가 있으면 그것을, 없으면 저장소를 건드리지 않게 데이터 폴더에 둔다.
pub fn index_db(root: &Path, data: &Path) -> Result<PathBuf> {
    if std::env::var_os("LANTERN_INDEX_DIR").is_some_and(|d| !d.is_empty()) {
        return crate::index_path(root);
    }
    let own = root.join(crate::PROJECT_DIR).join("index.db");
    if own.exists() {
        return Ok(own);
    }
    let dir = data.join("index");
    std::fs::create_dir_all(&dir)?;
    Ok(crate::keyed_index(&dir, root))
}

/// 증분 갱신한 인덱스를 연다
pub fn refreshed_index(root: &Path, data: &Path, parser: &mut Parser) -> Result<Store> {
    let store = Store::open(&index_db(root, data)?).context("인덱스 열기")?;
    indexer::index_project(root, &store, parser)?;
    Ok(store)
}

#[derive(Debug, Serialize)]
pub struct CheckReport {
    pub base: String,
    pub commit: String,
    pub changed_files: usize,
    pub changed_code_files: usize,
    pub removed: usize,
    pub signature: usize,
    pub body: usize,
    /// 상한 전 개수
    pub total_findings: usize,
    pub findings: Vec<Finding>,
    pub method: &'static str,
    pub not_covered: &'static str,
}

/// `1 file`, `2 files`
pub fn plural(n: usize, word: &str) -> String {
    match (n, word.ends_with('s')) {
        (1, _) => format!("1 {word}"),
        (_, true) => format!("{n} {word}es"),
        _ => format!("{n} {word}s"),
    }
}

impl CheckReport {
    pub fn blocking(&self) -> usize {
        self.findings.iter().filter(|f| f.blocks()).count()
    }

    pub fn to_text(&self) -> String {
        let files = plural(self.changed_code_files, "changed code file");
        let short = &self.commit[..self.commit.len().min(10)];
        if self.findings.is_empty() {
            return format!("lantern check: nothing left behind in {files} since {} ({short}). Name based.\n", self.base);
        }
        let mut out = format!(
            "lantern check: {} in {files} since {} ({short}). Name based, may be wrong.\n\n",
            plural(self.total_findings, "possible miss"),
            self.base
        );
        for f in &self.findings {
            let level = if f.confidence == Confidence::High { "HIGH" } else { "MEDIUM" };
            let caller = f.caller.as_deref().map(|c| format!(" in {c}")).unwrap_or_default();
            let more = if f.refs_in_file > 1 { format!(" (+{} more in this file)", f.refs_in_file - 1) } else { String::new() };
            let _ = writeln!(out, "{level:<6}  {}:{}{caller}{more}\n        {}", f.path, f.line, f.why);
        }
        if self.total_findings > self.findings.len() {
            let _ = writeln!(out, "\nShowing {} of {}.", self.findings.len(), self.total_findings);
        }
        let _ = writeln!(out, "\n{METHOD}\n{NOT_COVERED}");
        out
    }
}

/// `lantern check`: 작업 트리를 `base`와 비교한다
pub fn run_check(dir: &Path, data: &Path, base: &str) -> Result<CheckReport> {
    let root = repo_root(dir)?;
    let commit = resolve_commit(&root, base)?;
    let files = changed_files(&root, &commit)?;
    let mut parser = Parser::new()?;
    let changes = analyze_files(&root, &commit, &files, &HashMap::new(), &mut parser)?;
    let mut all = if changes.iter().any(FileChanges::may_leave_behind) {
        let store = refreshed_index(&root, data, &mut parser)?;
        collect_findings(&store, &changes, &files, &[], false)?
    } else {
        Vec::new()
    };
    let count = |pick: fn(&SymbolChange) -> bool| changes.iter().flat_map(|f| &f.changes).filter(|c| pick(c)).count();
    let total_findings = all.len();
    all.truncate(MAX_FINDINGS);
    Ok(CheckReport {
        base: base.to_string(),
        commit,
        changed_files: files.len(),
        changed_code_files: files.iter().filter(|f| Lang::from_path(Path::new(f)).is_some()).count(),
        removed: count(|c| matches!(c, SymbolChange::Removed { .. })),
        signature: count(|c| matches!(c, SymbolChange::Signature { .. })),
        body: count(|c| matches!(c, SymbolChange::Body { .. })),
        total_findings,
        findings: all,
        method: METHOD,
        not_covered: NOT_COVERED,
    })
}

/// 참조를 볼 만한 변화가 있는가. 없으면 인덱스를 열지 않아도 된다
pub fn may_leave_behind(changes: &[FileChanges]) -> bool {
    changes.iter().any(FileChanges::may_leave_behind)
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        // 태그 질의를 만드는 데 시간이 들어 스레드마다 하나만 만든다
        static PARSER: std::cell::RefCell<Parser> = std::cell::RefCell::new(Parser::new().unwrap());
    }

    fn with_parser<T>(f: impl FnOnce(&mut Parser) -> T) -> T {
        PARSER.with(|p| f(&mut p.borrow_mut()))
    }

    fn changes(lang: Lang, before: &str, after: &str) -> Vec<SymbolChange> {
        with_parser(|p| symbol_changes(p, lang, before.as_bytes(), after.as_bytes())).unwrap()
    }

    fn removed(c: &[SymbolChange]) -> Vec<(&str, Option<&str>)> {
        c.iter()
            .filter_map(|x| match x {
                SymbolChange::Removed { name, renamed_to, .. } => Some((name.as_str(), renamed_to.as_deref())),
                _ => None,
            })
            .collect()
    }

    fn signatures(c: &[SymbolChange]) -> Vec<&str> {
        c.iter()
            .filter_map(|x| match x {
                SymbolChange::Signature { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }

    fn bodies(c: &[SymbolChange]) -> Vec<&str> {
        c.iter()
            .filter_map(|x| match x {
                SymbolChange::Body { name } => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }

    /// 제거, 시그니처, 본문 변화를 언어마다 한 번에 본다
    fn all_three(lang: Lang, before: &str, after: &str, gone: &str, sig: &str, body: &str) {
        let c = changes(lang, before, after);
        assert!(removed(&c).iter().any(|(n, _)| *n == gone), "{lang:?} 제거 {gone}: {c:?}");
        assert_eq!(signatures(&c), [sig], "{lang:?} 시그니처: {c:?}");
        assert!(bodies(&c).contains(&body), "{lang:?} 본문 {body}: {c:?}");
    }

    #[test]
    fn rust_family() {
        all_three(
            Lang::Rust,
            "pub fn dropped_one(a: i32) -> i32 { a + 1 }\nfn resize_it(w: u32) -> u32 { w }\nfn body_only(a: i32) -> i32 { a }\n",
            "fn resize_it(w: u32, h: u32) -> u32 { w * h }\nfn body_only(a: i32) -> i32 { a + 2 }\n",
            "dropped_one",
            "resize_it",
            "body_only",
        );
    }

    #[test]
    fn python_family() {
        all_three(
            Lang::Python,
            "def dropped_one(a):\n    return a + 1\n\ndef resize_it(w):\n    return w\n\ndef body_only(a):\n    return a\n",
            "def resize_it(w, h):\n    return w * h\n\ndef body_only(a):\n    return a + 2\n",
            "dropped_one",
            "resize_it",
            "body_only",
        );
    }

    #[test]
    fn typescript_and_javascript_family() {
        let before = "export function droppedOne(a: number): number { return a + 1; }\nfunction resizeIt(w: number): number { return w; }\nconst bodyOnly = (a: number): number => { return a; };\n";
        let after = "function resizeIt(w: number, h: number): number { return w * h; }\nconst bodyOnly = (a: number): number => { return a + 2; };\n";
        for lang in [Lang::TypeScript, Lang::Tsx] {
            all_three(lang, before, after, "droppedOne", "resizeIt", "bodyOnly");
        }
        all_three(
            Lang::JavaScript,
            "function droppedOne(a) { return a + 1; }\nfunction resizeIt(w) { return w; }\nconst bodyOnly = (a) => { return a; };\n",
            "function resizeIt(w, h) { return w * h; }\nconst bodyOnly = (a) => { return a + 2; };\n",
            "droppedOne",
            "resizeIt",
            "bodyOnly",
        );
    }

    #[test]
    fn java_family() {
        all_three(
            Lang::Java,
            "class A {\n  public int droppedOne(int a) { return a + 1; }\n  @Override\n  public int resizeIt(int w) { return w; }\n  int bodyOnly(int a) { return a; }\n}\n",
            "class A {\n  @Override\n  public int resizeIt(int w, int h) { return w * h; }\n  int bodyOnly(int a) { return a + 2; }\n}\n",
            "droppedOne",
            "resizeIt",
            "bodyOnly",
        );
    }

    #[test]
    fn go_family() {
        all_three(
            Lang::Go,
            "package a\n\nfunc DroppedOne(a int) int { return a + 1 }\nfunc ResizeIt(w int) int { return w }\nfunc (u *User) BodyOnly(a int) int { return a }\n",
            "package a\n\nfunc ResizeIt(w, h int) int { return w * h }\nfunc (u *User) BodyOnly(a int) int { return a + 2 }\n",
            "DroppedOne",
            "ResizeIt",
            "BodyOnly",
        );
    }

    #[test]
    fn other_languages_see_signature_changes() {
        let cases = [
            (Lang::CSharp, "class A { public int Resize(int w) { return w; } }", "class A { public int Resize(int w, int h) { return w; } }", "Resize"),
            (Lang::Kotlin, "fun resizeIt(w: Int): Int { return w }\n", "fun resizeIt(w: Int, h: Int): Int { return w }\n", "resizeIt"),
            (Lang::C, "int resize_it(int w) { return w; }\n", "int resize_it(int w, int h) { return w; }\n", "resize_it"),
            (Lang::Cpp, "int resize_it(int w) { return w; }\n", "long resize_it(int w) { return w; }\n", "resize_it"),
        ];
        for (lang, before, after, name) in cases {
            assert_eq!(signatures(&changes(lang, before, after)), [name], "{lang:?}");
        }
    }

    #[test]
    fn whitespace_comments_and_visibility_are_not_signature_changes() {
        for (lang, before, after) in [
            (Lang::Rust, "fn tidy_it(a: i32) -> i32 { a }\n", "pub(crate) fn tidy_it(\n    a: i32, // 설명\n) -> i32 { a }\n"),
            (Lang::Java, "class A { int tidyIt(int a) { return a; } }", "class A { public final int tidyIt(int /* x */ a) { return a; } }"),
            (Lang::TypeScript, "function tidyIt(a: number): void {}\n", "export function tidyIt(\n  a: number,\n): void {}\n"),
            (Lang::Python, "def tidy_it(a, b):\n    pass\n", "def tidy_it(a,\n            b):  # 설명\n    pass\n"),
            (Lang::Go, "package a\n\nfunc TidyIt(a int, b int) int { return a }\n", "package a\n\nfunc TidyIt(a int, b int) int {\n\treturn a\n}\n"),
        ] {
            let c = changes(lang, before, after);
            assert!(signatures(&c).is_empty() && removed(&c).is_empty(), "{lang:?}: {c:?}");
        }
        // 주석과 공백만 바뀌면 본문 변화도 아니다
        assert!(changes(Lang::Rust, "fn tidy_it() -> i32 { 1 }\n", "// 설명\nfn tidy_it() -> i32 {\n    // 안쪽 설명\n    1\n}\n").is_empty());
    }

    #[test]
    fn added_defaults_and_optional_params_keep_old_calls_working() {
        for (lang, before, after) in [
            (Lang::Python, "def go_on(a):\n    pass\n", "def go_on(a, b=1):\n    pass\n"),
            (Lang::Python, "def go_on(a):\n    pass\n", "def go_on(a=1, *rest, **more):\n    pass\n"),
            (Lang::TypeScript, "function goOn(a: number) {}\n", "function goOn(a: number, b?: string, c = 1) {}\n"),
            (Lang::Kotlin, "fun goOn(a: Int) { }\n", "fun goOn(a: Int, b: Int = 1) { }\n"),
            (Lang::Cpp, "int go_on(int a) { return a; }\n", "int go_on(int a, int b = 1) { return a; }\n"),
        ] {
            let c = changes(lang, before, after);
            assert!(signatures(&c).is_empty(), "{lang:?} {after}: {c:?}");
        }
        // 기본값을 없애면 원래 호출이 깨진다
        assert_eq!(signatures(&changes(Lang::Python, "def go_on(a=1):\n    pass\n", "def go_on(a):\n    pass\n")), ["go_on"]);
    }

    #[test]
    fn parameter_names_only_matter_where_calls_can_use_them() {
        for (lang, before, after) in [
            (Lang::Rust, "fn same_call(a: i32) -> i32 { 1 }\n", "fn same_call(b: i32) -> i32 { 1 }\n"),
            (Lang::Java, "class A { int sameCall(String a) { return 1; } }", "class A { int sameCall(String b) { return 1; } }"),
            (Lang::Go, "package a\n\nfunc SameCall(a, b int) int { return 1 }\n", "package a\n\nfunc SameCall(c int, d int) int { return 1 }\n"),
            (Lang::C, "int same_call(const char *a) { return 1; }\n", "int same_call(const char *b) { return 1; }\n"),
            (Lang::JavaScript, "function sameCall(a) { return 1; }\n", "function sameCall(b) { return 1; }\n"),
        ] {
            let c = changes(lang, before, after);
            assert!(signatures(&c).is_empty(), "{lang:?}: {c:?}");
        }
        assert_eq!(signatures(&changes(Lang::Python, "def same_call(a):\n    pass\n", "def same_call(b):\n    pass\n")), ["same_call"]);
    }

    #[test]
    fn return_type_counts_and_values_do_not() {
        assert_eq!(signatures(&changes(Lang::Rust, "fn ret_kind() -> i32 { 1 }\n", "fn ret_kind() -> i64 { 1 }\n")), ["ret_kind"]);
        assert_eq!(signatures(&changes(Lang::Java, "class A { int retKind() { return 1; } }", "class A { long retKind() { return 1; } }")), ["retKind"]);
        // 값이 바뀐 상수와 필드가 바뀐 구조체는 호출 모양이 그대로다
        assert!(signatures(&changes(Lang::Rust, "const LIMIT_N: u32 = 5;\nstruct Conf { a: i32 }\n", "const LIMIT_N: u32 = 6;\nstruct Conf { a: i64 }\n")).is_empty());
        // Python 상위 클래스는 매개변수가 아니다
        assert!(signatures(&changes(Lang::Python, "class Foo(Bar):\n    pass\n", "class Foo(Baz):\n    pass\n")).is_empty());
        // 튜플 구조체는 생성자 호출 모양이 바뀐다
        assert_eq!(signatures(&changes(Lang::Rust, "struct Pair(i32);\n", "struct Pair(i32, i32);\n")), ["Pair"]);
    }

    #[test]
    fn rename_is_found_by_body_or_by_signature() {
        let c = changes(
            Lang::Rust,
            "fn old_name(a: i32) -> i32 { let total = a * 2 + 40; total + old_name(total) }\nfn keep_it() -> i32 { 1 }\n",
            "fn new_name(a: i32, b: i32) -> i32 { let total = a * 2 + 40; total + new_name(total) }\nfn keep_it() -> i32 { 1 }\n",
        );
        assert_eq!(removed(&c), [("old_name", Some("new_name"))]);

        let c = changes(
            Lang::Java,
            "class A { public User findUserByName(String name, int limit) { return lookup(name); } }",
            "class A { public User findUser(String name, int limit) { return store.get(name, limit); } }",
        );
        assert_eq!(removed(&c), [("findUserByName", Some("findUser"))]);

        // 비슷한 후보가 여럿이면 고르지 않는다
        let c = changes(
            Lang::Rust,
            "fn old_name(a: i32, b: i32) -> i32 { a }\n",
            "fn left_one(a: i32, b: i32) -> i32 { b }\nfn right_one(a: i32, b: i32) -> i32 { a + b }\n",
        );
        assert_eq!(removed(&c), [("old_name", None)]);

        // 짧은 본문이 같은 것만으로는 이름을 바꿨다고 하지 않는다
        let c = changes(Lang::Rust, "fn old_name() {}\n", "fn other_thing() {}\n");
        assert_eq!(removed(&c), [("old_name", None)]);
    }

    #[test]
    fn overloads_and_kind_changes_are_not_removed() {
        // 오버로드 하나만 없어져도 이름은 남아 있다
        let c = changes(Lang::Java, "class A { int foo(int a) { return 1; } int foo(int a, int b) { return 2; } }", "class A { int foo(int a) { return 1; } }");
        assert!(removed(&c).is_empty(), "{c:?}");
        // 순서가 바뀌어도 그대로다
        let c = changes(Lang::Java, "class A { int foo(int a) { return 1; } int foo(String b) { return 2; } }", "class A { int foo(String b) { return 2; } int foo(int a) { return 1; } }");
        assert!(signatures(&c).is_empty() && removed(&c).is_empty(), "{c:?}");
        // 함수가 메서드가 되어도 이름은 남아 있다
        let c = changes(Lang::Rust, "fn move_me(a: i32) -> i32 { a }\n", "struct S;\nimpl S { fn move_me(a: i32) -> i32 { a } }\n");
        assert!(removed(&c).is_empty() && signatures(&c).is_empty(), "{c:?}");
    }

    #[test]
    fn body_change_reports_only_the_innermost_symbol() {
        let c = changes(Lang::Python, "class Box:\n    def fill(self):\n        return 1\n", "class Box:\n    def fill(self):\n        return 2\n");
        assert_eq!(bodies(&c), ["fill"], "{c:?}");
    }

    #[test]
    fn newly_broken_syntax_reports_nothing() {
        let c = changes(Lang::Rust, "fn keep_me() -> i32 { 1 }\nfn other_one() {}\n", "fn keep_me( -> i32 { 1 \n");
        assert!(c.is_empty(), "{c:?}");
    }

    // ── left_behind ──────────────────────────────────────

    fn store_with(files: &[(&str, &str)]) -> Store {
        let store = Store::open_in_memory().unwrap();
        for (path, src) in files {
            let lang = Lang::from_path(Path::new(path)).unwrap();
            let parsed = with_parser(|p| p.parse(lang, src.as_bytes())).unwrap();
            store.replace_file(path, lang.name(), "h", 0, 0, &parsed).unwrap();
        }
        store
    }

    fn change(path: &str, before: &str, after: &str) -> FileChanges {
        let lang = Lang::from_path(Path::new(path)).unwrap();
        with_parser(|p| file_changes(p, path, lang, before.as_bytes(), after.as_bytes())).unwrap()
    }

    const OLD_LIB: &str = "pub fn compute_total(a: i32) -> i32 { a + 1 }\n";
    const CALLER: &str = "fn show_it() -> i32 {\n    compute_total(1)\n}\n";

    #[test]
    fn dangling_across_files_with_line_and_caller() {
        // 인덱스가 끝 상태든 기준 상태든 같은 답이어야 한다
        for lib_in_index in ["pub fn other_fn() {}\n", OLD_LIB] {
            let store = store_with(&[("src/lib.rs", lib_in_index), ("src/show.rs", CALLER)]);
            let fc = change("src/lib.rs", OLD_LIB, "pub fn other_fn() {}\n");
            let got = left_behind(&store, &[fc], &[], &[], true).unwrap();
            assert_eq!(got.len(), 1, "{got:?}");
            let f = &got[0];
            assert_eq!((f.kind, f.confidence), (FindingKind::Dangling, Confidence::High));
            assert_eq!((f.path.as_str(), f.line, f.caller.as_deref()), ("src/show.rs", 2, Some("show_it")));
            assert_eq!(f.symbol, "compute_total");
            assert!(f.why.contains("Name based"), "{}", f.why);
            assert!(f.blocks());
        }
    }

    #[test]
    fn rename_with_missed_caller_is_dangling_and_says_the_new_name() {
        let after = "pub fn add_up(a: i32) -> i32 { a + 1 }\n";
        let store = store_with(&[("src/lib.rs", after), ("src/show.rs", CALLER)]);
        let got = left_behind(&store, &[change("src/lib.rs", OLD_LIB, after)], &[], &[], false).unwrap();
        assert_eq!(got[0].renamed_to.as_deref(), Some("add_up"), "{got:?}");
        assert!(got[0].why.contains("add_up"));
    }

    #[test]
    fn not_dangling_when_touched_or_defined_elsewhere_or_other_family() {
        let after = "pub fn other_fn() {}\n";
        let fc = || change("src/lib.rs", OLD_LIB, after);
        // 호출자도 손댔다
        let store = store_with(&[("src/lib.rs", after), ("src/show.rs", CALLER)]);
        assert!(left_behind(&store, &[fc()], &["src/show.rs".into()], &[], true).unwrap().is_empty());
        // 다른 파일에 정의가 남았다
        let store = store_with(&[("src/lib.rs", after), ("src/show.rs", CALLER), ("src/moved.rs", OLD_LIB)]);
        assert!(left_behind(&store, &[fc()], &[], &[], true).unwrap().is_empty());
        // 손댄 다른 파일로 옮겼다
        let moved = change("src/moved.rs", "", OLD_LIB);
        let store = store_with(&[("src/lib.rs", after), ("src/show.rs", CALLER)]);
        assert!(left_behind(&store, &[fc(), moved], &[], &[], true).unwrap().is_empty());
        // 다른 언어의 같은 이름은 상관없다
        let store = store_with(&[("src/lib.rs", after), ("web/show.ts", "function showIt() { return compute_total(1); }\n")]);
        assert!(left_behind(&store, &[fc()], &[], &[], true).unwrap().is_empty());
        // 다른 언어에만 정의가 남은 것은 남은 게 아니다
        let store = store_with(&[("src/lib.rs", after), ("src/show.rs", CALLER), ("web/a.ts", "function compute_total() {}\n")]);
        assert_eq!(left_behind(&store, &[fc()], &[], &[], true).unwrap().len(), 1);
    }

    #[test]
    fn test_paths_are_flagged_and_sorted_after_code() {
        let store = store_with(&[("src/lib.rs", OLD_LIB), ("tests/a.rs", CALLER), ("zzz/show.rs", CALLER)]);
        let got = left_behind(&store, &[change("src/lib.rs", OLD_LIB, "pub fn other_fn() {}\n")], &[], &[], true).unwrap();
        let order: Vec<(&str, bool)> = got.iter().map(|f| (f.path.as_str(), f.in_test)).collect();
        assert_eq!(order, [("zzz/show.rs", false), ("tests/a.rs", true)]);
    }

    fn signature_change() -> FileChanges {
        change("src/lib.rs", OLD_LIB, "pub fn compute_total(a: i32, b: i32) -> i32 { a + b }\n")
    }

    #[test]
    fn stale_caller_is_medium_and_read_callers_are_filtered() {
        let store = store_with(&[("src/lib.rs", OLD_LIB), ("src/show.rs", CALLER)]);
        let got = left_behind(&store, &[signature_change()], &[], &[], true).unwrap();
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!((got[0].kind, got[0].confidence), (FindingKind::StaleCaller, Confidence::Medium));
        assert!(!got[0].blocks());
        assert!(got[0].why.contains("not edited or read"), "{}", got[0].why);

        // 세션에서 읽은 호출자는 뺀다
        let read = ["src/show.rs".to_string()];
        assert!(left_behind(&store, &[signature_change()], &[], &read, true).unwrap().is_empty());
        // 읽은 것을 모르면(Codex) 뺄 수 없고, 안 읽었다고 말하지도 않는다
        let got = left_behind(&store, &[signature_change()], &[], &read, false).unwrap();
        assert_eq!(got.len(), 1);
        assert!(!got[0].why.contains("read"), "{}", got[0].why);
        // 손댄 호출자도 뺀다
        assert!(left_behind(&store, &[signature_change()], &read, &[], true).unwrap().is_empty());
    }

    #[test]
    fn caller_with_its_own_definition_of_the_name_is_not_stale() {
        let own = "fn compute_total() -> i32 { 1 }\nfn show_it() -> i32 {\n    compute_total()\n}\n";
        let store = store_with(&[("src/lib.rs", OLD_LIB), ("src/own.rs", own)]);
        assert!(left_behind(&store, &[signature_change()], &[], &[], true).unwrap().is_empty());
    }

    #[test]
    fn body_change_is_never_a_finding() {
        let store = store_with(&[("src/lib.rs", OLD_LIB), ("src/show.rs", CALLER)]);
        let fc = change("src/lib.rs", OLD_LIB, "pub fn compute_total(a: i32) -> i32 { a + 2 }\n");
        assert_eq!(bodies(&fc.changes), ["compute_total"]);
        assert!(left_behind(&store, &[fc], &[], &[], true).unwrap().is_empty());
    }

    #[test]
    fn generic_and_short_names_are_skipped() {
        for name in ["new", "get", "set", "init", "main", "test", "run", "Run", "ab"] {
            let lib = format!("pub fn {name}(a: i32) -> i32 {{ a }}\n");
            let caller = format!("fn show_it() {{ {name}(1); }}\n");
            let store = store_with(&[("src/lib.rs", "pub fn other_fn() {}\n"), ("src/show.rs", &caller)]);
            let fc = change("src/lib.rs", &lib, "pub fn other_fn() {}\n");
            assert_eq!(removed(&fc.changes).len(), 1, "{name}");
            assert!(left_behind(&store, &[fc], &[], &[], true).unwrap().is_empty(), "{name}");
        }
    }

    #[test]
    fn names_with_too_many_definitions_are_skipped() {
        let lib = "pub fn shared_helper(a: i32) -> i32 { a }\n";
        let caller = "fn show_it() { shared_helper(1); }\n";
        let after = "pub fn other_fn() {}\n";
        // 정의 6개: 어느 것을 가리키는지 모른다
        let many: Vec<(String, &str)> = (0..6).map(|i| (format!("web/f{i}.ts"), "function shared_helper() {}\n")).collect();
        let mut files: Vec<(&str, &str)> = vec![("src/lib.rs", after), ("src/show.rs", caller)];
        files.extend(many.iter().map(|(p, s)| (p.as_str(), *s)));
        let store = store_with(&files);
        assert_eq!(store.count_defs("shared_helper").unwrap(), 6);
        assert!(left_behind(&store, &[change("src/lib.rs", lib, after)], &[], &[], true).unwrap().is_empty());
        // 5개까지는 본다
        files.pop();
        let store = store_with(&files);
        assert_eq!(left_behind(&store, &[change("src/lib.rs", lib, after)], &[], &[], true).unwrap().len(), 1);
    }

    #[test]
    fn one_finding_per_file_capped_and_high_first() {
        let lib = "pub fn gone_helper() -> i32 { 1 }\npub fn wide_helper(a: i32) -> i32 { a }\n";
        let after = "pub fn wide_helper(a: i32, b: i32) -> i32 { a + b }\n";
        let mut owned: Vec<(String, String)> = (0..25).map(|i| (format!("src/c{i:02}.rs"), "fn stale_user() { wide_helper(1); wide_helper(2); }\n".to_string())).collect();
        owned.push(("src/z_last.rs".into(), "fn old_user() {\n    gone_helper();\n    gone_helper();\n}\n".into()));
        let mut files: Vec<(&str, &str)> = vec![("src/lib.rs", after)];
        files.extend(owned.iter().map(|(p, s)| (p.as_str(), s.as_str())));
        let store = store_with(&files);
        let got = left_behind(&store, &[change("src/lib.rs", lib, after)], &[], &[], true).unwrap();
        assert_eq!(got.len(), MAX_FINDINGS);
        assert_eq!(got[0].kind, FindingKind::Dangling);
        assert_eq!((got[0].refs_in_file, got[0].line), (2, 2));
        assert!(got[1..].iter().all(|f| f.kind == FindingKind::StaleCaller && f.refs_in_file == 2));
    }

    #[test]
    #[cfg(windows)]
    fn paths_compare_without_case_on_windows() {
        let store = store_with(&[("src/lib.rs", OLD_LIB), ("src/show.rs", CALLER)]);
        let read = ["SRC\\Show.rs".to_string()];
        assert!(left_behind(&store, &[signature_change()], &[], &read, true).unwrap().is_empty());
        assert!(left_behind(&store, &[signature_change()], &read, &[], true).unwrap().is_empty());
    }
}
