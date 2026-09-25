//! 맥락 조립기: 후보 수집 → 그래프 확장 → 동시 변경 가산 → 순위 → 예산 내 압축.

use crate::lang::Lang;
use crate::store::{Store, SymbolRow};
use crate::tokenize::{self, estimate_tokens};
use anyhow::Result;
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

// 점수 가중치 (설계 문서 5장). 평가 결과로 조정한다.
const W_SEARCH: f64 = 1.0;
/// 한국어 개발 용어에서 옮긴 영어 단어로 찾은 결과
const W_SEARCH_EXPANDED: f64 = 0.5;
const W_NAME: f64 = 2.0;
const W_FOCUS: f64 = 3.0;
const W_FOCUS_FILE: f64 = 0.8;
const W_CALLEE: f64 = 0.5;
const W_CALLER: f64 = 0.4;
const W_COCHANGE: f64 = 0.3;

const SEARCH_LIMIT: usize = 40;
const SEED_COUNT: usize = 8;
/// 정의가 이보다 많은 이름(`new`, `get` 등)은 이름만으로 해석하면 오탐이 많아 그래프 확장에서 뺀다.
const MAX_AMBIGUOUS_DEFS: i64 = 5;
/// 최고점 대비 이 비율보다 낮은 후보는 버린다.
const MIN_RELATIVE_SCORE: f64 = 0.12;
const MEMORY_SHARE: usize = 15; // %
const SINGLE_ITEM_SHARE: usize = 35; // %
const ITEM_OVERHEAD_TOKENS: usize = 25;

pub const DEFAULT_BUDGET: usize = 8000;

#[derive(Debug, Clone)]
pub struct ContextRequest {
    pub query: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub budget_tokens: usize,
}

impl ContextRequest {
    pub fn new(query: impl Into<String>) -> Self {
        Self { query: query.into(), file: None, line: None, budget_tokens: DEFAULT_BUDGET }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Full,
    Signature,
}

#[derive(Debug, Serialize)]
pub struct ContextItem {
    pub path: String,
    pub lang: String,
    pub name: String,
    pub kind: String,
    pub start_line: u32,
    pub end_line: u32,
    pub score: f64,
    pub reasons: Vec<String>,
    pub mode: Mode,
    pub doc: Option<String>,
    pub text: String,
    pub tokens: usize,
}

#[derive(Debug, Serialize)]
pub struct MemoryItem {
    pub path: String,
    pub text: String,
    pub tokens: usize,
}

#[derive(Debug, Serialize)]
pub struct ContextResult {
    pub query: String,
    pub query_terms: Vec<String>,
    pub budget_tokens: usize,
    pub used_tokens: usize,
    pub candidates: usize,
    pub memory: Vec<MemoryItem>,
    pub items: Vec<ContextItem>,
    pub elapsed_ms: f64,
    /// 비밀로 보여 가린 값의 개수
    pub redacted: usize,
}

struct Candidate {
    sym: SymbolRow,
    score: f64,
    reasons: Vec<String>,
}

#[derive(Default)]
struct Candidates(HashMap<i64, Candidate>);

impl Candidates {
    fn add(&mut self, sym: SymbolRow, score: f64, reason: String) {
        let c = self.0.entry(sym.id).or_insert(Candidate { sym, score: 0.0, reasons: Vec::new() });
        c.score += score;
        if !c.reasons.contains(&reason) {
            c.reasons.push(reason);
        }
    }

    fn ranked(&self) -> Vec<&Candidate> {
        let mut v: Vec<_> = self.0.values().collect();
        v.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.sym.id.cmp(&b.sym.id)));
        v
    }
}

/// 검색으로 고른 이유: 질문의 어떤 단어가 이름·시그니처·설명에서 일치했는지.
/// 거기에 없으면 본문에서 일치한 것이다.
fn search_reason(sym: &SymbolRow, terms: &[tokenize::QueryTerm]) -> String {
    let head = format!(
        "{} {} {} {}",
        sym.name,
        tokenize::split_identifier(&sym.name).join(" "),
        sym.signature,
        sym.doc.as_deref().unwrap_or("")
    )
    .to_lowercase();
    let matched: Vec<&str> = terms.iter().map(|t| t.text.as_str()).filter(|t| head.contains(t)).take(3).collect();
    if matched.is_empty() {
        "본문에서 일치".into()
    } else {
        format!("'{}' 일치", matched.join("', '"))
    }
}

/// 경로를 인덱스 형식(루트 기준, `/` 구분)으로 맞춘다.
pub fn normalize_path(root: &Path, path: &str) -> String {
    let p = Path::new(path);
    let rel = if p.is_absolute() {
        p.strip_prefix(root).map(|r| r.to_path_buf()).unwrap_or_else(|_| p.to_path_buf())
    } else {
        p.to_path_buf()
    };
    rel.to_string_lossy().replace('\\', "/").trim_start_matches("./").to_string()
}

pub fn assemble(store: &Store, root: &Path, req: &ContextRequest) -> Result<ContextResult> {
    let started = Instant::now();
    let mut cands = Candidates::default();
    let terms = tokenize::query_terms(&req.query);

    // 1. 시드: 전문 검색. 질문의 단어와, 한국어 개발 용어에서 옮긴 영어 단어를 따로 찾고
    //    옮긴 단어 쪽은 약하게 친다 (validate, check 같은 흔한 단어가 질문의 핵심어를 누르지 않게).
    let (asked, expanded): (Vec<_>, Vec<_>) = terms.iter().cloned().partition(|t| !t.expanded);
    for (group, weight) in [(&asked, W_SEARCH), (&expanded, W_SEARCH_EXPANDED)] {
        if group.is_empty() {
            continue;
        }
        let hits = store.search(&tokenize::fts_query(group), SEARCH_LIMIT)?;
        let max = hits.first().map(|(_, s)| *s).unwrap_or(1.0).max(f64::EPSILON);
        for (sym, s) in hits {
            let reason = search_reason(&sym, &terms);
            cands.add(sym, weight * s / max, reason);
        }
    }

    // 1-2. 질문에 심볼 이름이 그대로 나오면 가산
    for ident in tokenize::identifiers(&req.query).filter(|i| i.len() >= 3) {
        let defs = store.symbols_by_name(ident, MAX_AMBIGUOUS_DEFS as usize)?;
        let n = defs.len().max(1) as f64;
        for d in defs {
            cands.add(d, W_NAME / n, "질문에 이름 등장".into());
        }
    }

    // 1-3. 포커스 위치
    if let Some(file) = &req.file {
        let path = normalize_path(root, file);
        let syms = store.symbols_in_file(&path)?;
        match req.line {
            Some(line) => {
                if let Some(s) = syms
                    .iter()
                    .filter(|s| s.start_line <= line && line <= s.end_line)
                    .min_by_key(|s| s.end_line - s.start_line)
                {
                    cands.add(s.clone(), W_FOCUS, format!("포커스 위치 (L{line})"));
                }
            }
            None => {
                for s in syms.into_iter().take(30) {
                    cands.add(s, W_FOCUS_FILE, "포커스 파일".into());
                }
            }
        }
    }

    // 2. 그래프 확장 (1홉)
    let seeds: Vec<(SymbolRow, f64)> =
        cands.ranked().into_iter().take(SEED_COUNT).map(|c| (c.sym.clone(), c.score)).collect();
    for (seed, score) in &seeds {
        for name in store.callee_names(seed.id)?.into_iter().take(30) {
            if name == seed.name {
                continue;
            }
            let n = store.count_defs(&name)?;
            if n == 0 || n > MAX_AMBIGUOUS_DEFS {
                continue;
            }
            for d in store.symbols_by_name(&name, n as usize)? {
                if d.id == seed.id || seed.contains(&d) {
                    continue;
                }
                let locality = if d.path == seed.path { 1.5 } else { 1.0 };
                cands.add(d, W_CALLEE * score * locality / n as f64, format!("`{}`가 사용", seed.name));
            }
        }
        if store.count_defs(&seed.name)? <= 3 {
            for id in store.caller_ids(&seed.name, 12)? {
                if id == seed.id {
                    continue;
                }
                if let Some(s) = store.symbol(id)? {
                    if !s.contains(seed) {
                        cands.add(s, W_CALLER * score, format!("`{}`를 사용", seed.name));
                    }
                }
            }
        }
    }

    // 3. 동시 변경 가산: 시드 파일과 자주 함께 바뀐 파일의 후보를 조금 올린다.
    let mut partner: HashMap<String, (f64, String)> = HashMap::new();
    let mut seed_files: Vec<&str> = seeds.iter().map(|(s, _)| s.path.as_str()).collect();
    seed_files.dedup();
    for f in seed_files.into_iter().take(5) {
        let co = store.cochanged(f, 10)?;
        let max = co.first().map(|(_, n)| *n).unwrap_or(1).max(1) as f64;
        for (p, n) in co {
            let w = n as f64 / max;
            let e = partner.entry(p).or_insert((0.0, f.to_string()));
            if w > e.0 {
                *e = (w, f.to_string());
            }
        }
    }
    for c in cands.0.values_mut() {
        if let Some((w, with)) = partner.get(&c.sym.path) {
            c.score += W_COCHANGE * w;
            c.reasons.push(format!("`{with}`와 자주 함께 변경"));
        }
    }

    // 4~5. 순위와 예산 내 압축
    let budget = req.budget_tokens.max(500);
    let mut used = 0usize;
    let mut memory = load_memory(root, budget * MEMORY_SHARE / 100, &mut used);

    let mut redacted = 0usize;
    for m in &mut memory {
        let (t, n) = crate::secrets::redact(&m.text);
        m.text = t;
        redacted += n;
    }
    let ranked = cands.ranked();
    let top = ranked.first().map(|c| c.score).unwrap_or(0.0);
    let mut sources: HashMap<String, Option<Vec<u8>>> = HashMap::new();
    let mut items: Vec<ContextItem> = Vec::new();
    let mut included: Vec<(SymbolRow, Mode)> = Vec::new();

    for c in ranked.iter().filter(|c| c.score >= top * MIN_RELATIVE_SCORE).take(80) {
        let remaining = budget.saturating_sub(used);
        if remaining < ITEM_OVERHEAD_TOKENS * 2 {
            break;
        }
        // 이미 본문 전체가 들어간 심볼 안쪽이면 중복이다.
        if included.iter().any(|(s, m)| *m == Mode::Full && s.contains(&c.sym)) {
            continue;
        }
        let src = sources
            .entry(c.sym.path.clone())
            .or_insert_with(|| std::fs::read(root.join(&c.sym.path)).ok());
        let Some(src) = src else { continue };
        if c.sym.end_byte > src.len() {
            continue; // 인덱스 이후 파일이 바뀜
        }

        let full = String::from_utf8_lossy(&src[c.sym.start_byte..c.sym.end_byte]).into_owned();
        let full_tokens = estimate_tokens(&full) + ITEM_OVERHEAD_TOKENS;
        let wraps_included = included.iter().any(|(s, _)| c.sym.contains(s));
        let mode = if !wraps_included
            && full_tokens <= remaining
            && full_tokens <= budget * SINGLE_ITEM_SHARE / 100
        {
            Mode::Full
        } else {
            Mode::Signature
        };
        let raw = match mode {
            Mode::Full => full,
            Mode::Signature => c.sym.signature.clone(),
        };
        let (text, hidden) = crate::secrets::redact(&raw);
        redacted += hidden;
        let tokens = estimate_tokens(&text) + ITEM_OVERHEAD_TOKENS;
        if tokens > remaining {
            continue;
        }
        used += tokens;
        included.push((c.sym.clone(), mode));
        items.push(ContextItem {
            path: c.sym.path.clone(),
            lang: c.sym.lang.clone(),
            name: c.sym.name.clone(),
            kind: c.sym.kind.clone(),
            start_line: c.sym.start_line,
            end_line: c.sym.end_line,
            score: (c.score * 1000.0).round() / 1000.0,
            reasons: c.reasons.clone(),
            mode,
            doc: c.sym.doc.clone(),
            text,
            tokens,
        });
    }

    Ok(ContextResult {
        query: req.query.clone(),
        query_terms: terms.into_iter().map(|t| t.text).collect(),
        budget_tokens: budget,
        used_tokens: used,
        candidates: cands.0.len(),
        memory,
        items,
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        redacted,
    })
}

/// `.lantern/memory/*.md`를 예산 안에서 읽는다.
fn load_memory(root: &Path, limit: usize, used: &mut usize) -> Vec<MemoryItem> {
    let dir = root.join(".lantern").join("memory");
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut paths: Vec<_> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    paths.sort();

    let mut out = Vec::new();
    let mut spent = 0usize;
    for p in paths {
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        let left = limit.saturating_sub(spent);
        if left < 50 {
            break;
        }
        let mut text = text.trim().to_string();
        let mut tokens = estimate_tokens(&text) + 10;
        if tokens > left {
            // 앞에서부터 예산만큼만 넣는다.
            let ratio = left as f64 / tokens as f64;
            let keep = (text.chars().count() as f64 * ratio * 0.95) as usize;
            text = text.chars().take(keep).collect::<String>() + "\n…(잘림)";
            tokens = estimate_tokens(&text) + 10;
        }
        spent += tokens;
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        out.push(MemoryItem { path: format!(".lantern/memory/{name}"), text, tokens });
    }
    *used += spent;
    out
}

impl ContextResult {
    /// 모델에 넣기 좋은 Markdown. 파일별로 묶고, 파일 안에서는 줄 순서로 둔다.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        let full = self.items.iter().filter(|i| i.mode == Mode::Full).count();
        let _ = writeln!(out, "# 코드 맥락 (Lantern)");
        let _ = writeln!(out, "질문: {}", self.query);
        let _ = writeln!(
            out,
            "토큰 약 {}/{} · 후보 {}개 중 {}개 포함 (본문 {}, 시그니처 {}) · {:.0}ms",
            self.used_tokens,
            self.budget_tokens,
            self.candidates,
            self.items.len(),
            full,
            self.items.len() - full,
            self.elapsed_ms
        );
        if self.redacted > 0 {
            let _ = writeln!(out, "비밀로 보이는 값 {}개를 «가려진 비밀»로 바꿨습니다.", self.redacted);
        }
        let _ = writeln!(out, "시그니처만 있는 항목은 필요하면 get_symbol로 본문을 요청하세요.\n");

        if !self.memory.is_empty() {
            let _ = writeln!(out, "## 프로젝트 메모리\n");
            for m in &self.memory {
                let _ = writeln!(out, "### {}\n{}\n", m.path, m.text);
            }
        }

        let mut order: Vec<&str> = Vec::new();
        for i in &self.items {
            if !order.contains(&i.path.as_str()) {
                order.push(&i.path);
            }
        }
        for path in order {
            let _ = writeln!(out, "## {path}\n");
            let mut in_file: Vec<&ContextItem> = self.items.iter().filter(|i| i.path == path).collect();
            in_file.sort_by_key(|i| i.start_line);
            for i in in_file {
                let lines = if i.start_line == i.end_line {
                    format!("L{}", i.start_line)
                } else {
                    format!("L{}-{}", i.start_line, i.end_line)
                };
                let mode = if i.mode == Mode::Signature { " · 시그니처만" } else { "" };
                let _ = writeln!(
                    out,
                    "### `{}` ({}, {}){} — {}",
                    i.name,
                    i.kind,
                    lines,
                    mode,
                    i.reasons.join(", ")
                );
                if let Some(doc) = &i.doc {
                    for l in doc.lines().take(3) {
                        let _ = writeln!(out, "> {l}");
                    }
                }
                let _ = writeln!(out, "```{}\n{}\n```\n", Lang::fence(&i.lang), i.text.trim_end());
            }
        }
        out
    }
}
