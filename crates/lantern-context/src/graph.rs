//! 코드 지식그래프. 인덱스(파일·심볼·참조·함께 바뀐 이력)와 프로젝트 메모리를 노드와 간선으로 엮는다.
//!
//! - [`overview`]: 파일 단위 전체 지도
//! - [`neighborhood`]: 파일이나 심볼 하나를 중심으로 한 주변
//! - [`impact`]: 바꾸려는 줄이 닿는 호출자·함께 바뀌던 파일·테스트 (변경 영향 반경)
//! - [`memory`]: `.lantern/memory/*.md`의 메모와 거기서 언급한 코드
//!
//! 참조는 이름 기반이라, 같은 이름이 여러 곳에 정의된 흔한 이름은 잡음으로 보고 뺀다.

use crate::lang::Lang;
use crate::store::{Store, SymbolRow};
use anyhow::Result;
use serde::Serialize;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

/// 이 수보다 많이 정의된 이름은 연결하지 않는다 (new, get, render 같은 이름)
const MAX_DEFS: i64 = 3;
const CONTAINER_DIRS: &[&str] = &["src", "app", "apps", "packages", "crates", "lib", "libs", "modules", "services", "internal", "pkg"];

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// 폴더 (큰 프로젝트의 전체 지도)
    Dir,
    File,
    Symbol,
    Memory,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// 파일 A가 파일 B의 이름을 참조
    Ref,
    /// 심볼 A가 심볼 B를 호출(참조)
    Calls,
    /// 파일이 심볼을 정의
    Defines,
    /// git 이력에서 함께 바뀜
    Cochange,
    /// 메모가 코드를 언급
    Mentions,
}

#[derive(Debug, Clone, Serialize)]
pub struct Node {
    pub id: String,
    pub kind: NodeKind,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// 색 구분용 모듈 (상위 폴더)
    pub group: String,
    /// 크기: 파일은 심볼 수, 심볼은 호출자 수
    pub weight: f64,
    /// 심볼 종류, 또는 메모 본문
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// 영향 반경에서의 거리 (0 = 바뀌는 곳)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub kind: EdgeKind,
    pub weight: f64,
}

#[derive(Debug, Default, Serialize)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// 한도 때문에 일부를 뺐는지
    pub truncated: bool,
}

pub fn file_node_id(path: &str) -> String {
    format!("f:{path}")
}

pub fn dir_node_id(dir: &str) -> String {
    format!("d:{dir}")
}

/// 파일이 속한 폴더를 앞에서 `depth`단계까지 (루트 파일은 빈 문자열)
pub fn dir_at(path: &str, depth: usize) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    let dirs = &parts[..parts.len().saturating_sub(1)];
    dirs[..dirs.len().min(depth)].join("/")
}

/// 이 수보다 파일이 많으면 전체 지도를 폴더 단위로 묶는다
pub const DIR_OVERVIEW_THRESHOLD: usize = 150;

pub fn symbol_node_id(id: i64) -> String {
    format!("s:{id}")
}

/// 모듈 이름: 첫 폴더. `src/`, `crates/` 같은 담는 폴더면 한 단계 더 (`crates/lantern-context`)
pub fn group_of(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.len() {
        0 | 1 => "(root)".into(),
        2 => parts[0].into(),
        _ if CONTAINER_DIRS.contains(&parts[0]) => format!("{}/{}", parts[0], parts[1]),
        _ => parts[0].into(),
    }
}

pub fn is_test_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.contains("/test/") || p.contains("/tests/") || p.contains("__tests__") || p.starts_with("test/") || p.starts_with("tests/")
        || p.contains(".test.") || p.contains(".spec.") || p.contains("_test.") || p.rsplit('/').next().is_some_and(|f| f.starts_with("test_"))
        // C# 테스트 프로젝트 (MyApp.Tests/)
        || p.contains(".tests/")
        // Java·C#: UserServiceTest.java, UserServiceTests.cs (대문자 T로 구분해 Contest.java는 빼기)
        || Path::new(path).file_stem().and_then(|s| s.to_str()).is_some_and(|s| s.len() > 4 && (s.ends_with("Test") || s.ends_with("Tests")))
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

struct Builder {
    graph: Graph,
    seen: HashSet<String>,
    edges: HashSet<(String, String, u8)>,
}

impl Builder {
    fn new() -> Self {
        Self { graph: Graph::default(), seen: HashSet::new(), edges: HashSet::new() }
    }

    fn file(&mut self, path: &str, weight: f64) -> String {
        let id = file_node_id(path);
        if self.seen.insert(id.clone()) {
            self.graph.nodes.push(Node {
                id: id.clone(),
                kind: NodeKind::File,
                label: basename(path).into(),
                path: Some(path.into()),
                line: None,
                group: group_of(path),
                weight,
                detail: None,
                depth: None,
            });
        }
        id
    }

    fn dir(&mut self, dir: &str, files: usize) -> String {
        let id = dir_node_id(dir);
        if self.seen.insert(id.clone()) {
            // src, scripts, test처럼 흔한 이름끼리 구분되게 마지막 두 단계를 보인다
            let parts: Vec<&str> = dir.split('/').collect();
            let label = if dir.is_empty() { "(root)".to_string() } else { parts[parts.len().saturating_sub(2)..].join("/") };
            self.graph.nodes.push(Node {
                id: id.clone(),
                kind: NodeKind::Dir,
                label,
                path: Some(dir.into()),
                line: None,
                group: group_of(&format!("{dir}/_/_")),
                weight: files as f64,
                detail: Some(files.to_string()),
                depth: None,
            });
        }
        id
    }

    fn symbol(&mut self, s: &SymbolRow, weight: f64, depth: Option<u8>) -> String {
        let id = symbol_node_id(s.id);
        if self.seen.insert(id.clone()) {
            self.graph.nodes.push(Node {
                id: id.clone(),
                kind: NodeKind::Symbol,
                label: s.name.clone(),
                path: Some(s.path.clone()),
                line: Some(s.start_line),
                group: group_of(&s.path),
                weight,
                detail: Some(s.kind.clone()),
                depth,
            });
        } else if let Some(d) = depth {
            // 더 가까운 거리로 다시 만나면 갱신
            if let Some(n) = self.graph.nodes.iter_mut().find(|n| n.id == id) {
                n.depth = Some(n.depth.map_or(d, |old| old.min(d)));
            }
        }
        id
    }

    fn edge(&mut self, source: &str, target: &str, kind: EdgeKind, weight: f64) {
        if source == target {
            return;
        }
        if self.edges.insert((source.into(), target.into(), kind as u8)) {
            self.graph.edges.push(Edge { source: source.into(), target: target.into(), kind, weight });
        }
    }

    fn finish(self) -> Graph {
        self.graph
    }
}

/// 이름이 흔하지 않은 경우에만 정의를 돌려준다
fn defs(store: &Store, name: &str, limit: usize) -> Result<Vec<SymbolRow>> {
    if store.count_defs(name)? > MAX_DEFS {
        return Ok(vec![]);
    }
    store.symbols_by_name(name, limit)
}

fn callers(store: &Store, s: &SymbolRow, limit: usize) -> Result<Vec<SymbolRow>> {
    if store.count_defs(&s.name)? > MAX_DEFS {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    for id in store.caller_ids(&s.name, limit)? {
        if id == s.id {
            continue;
        }
        if let Some(c) = store.symbol(id)? {
            // 자기 안에 든 심볼(메서드가 클래스를 참조 등)은 호출자로 치지 않는다.
            // 다른 언어 계열에서 같은 이름을 부른 것도 호출이 아니다
            if !s.contains(&c) && Lang::family(&c.lang) == Lang::family(&s.lang) {
                out.push(c);
            }
        }
    }
    Ok(out)
}

// ── 전체 지도 ──────────────────────────────────────────────

/// 파일 단위 전체 지도. 파일이 많으면 연결이 많은 순으로 `max_nodes`개만.
pub fn overview(store: &Store, max_nodes: usize) -> Result<Graph> {
    let files = store.file_summaries()?;
    let refs = store.file_ref_edges(MAX_DEFS)?;
    let cochange = store.cochange_pairs(2)?;
    if files.len() > DIR_OVERVIEW_THRESHOLD {
        return Ok(dir_overview(&files, &refs, &cochange));
    }

    let mut degree: HashMap<&str, f64> = HashMap::new();
    for (a, b, n) in &refs {
        *degree.entry(a).or_default() += *n as f64;
        *degree.entry(b).or_default() += *n as f64;
    }
    for (a, b, n) in &cochange {
        *degree.entry(a).or_default() += *n as f64;
        *degree.entry(b).or_default() += *n as f64;
    }
    let mut ranked: Vec<&(String, String, i64)> = files.iter().collect();
    ranked.sort_by(|x, y| {
        let score = |f: &(String, String, i64)| degree.get(f.0.as_str()).copied().unwrap_or(0.0) + f.2 as f64;
        score(y).total_cmp(&score(x)).then(x.0.cmp(&y.0))
    });
    let truncated = ranked.len() > max_nodes;
    let keep: HashSet<&str> = ranked.iter().take(max_nodes).map(|f| f.0.as_str()).collect();

    let mut b = Builder::new();
    for (path, lang, symbols) in &files {
        if keep.contains(path.as_str()) {
            let id = b.file(path, (*symbols).max(1) as f64);
            if let Some(n) = b.graph.nodes.iter_mut().find(|n| n.id == id) {
                n.detail = Some(lang.clone());
            }
        }
    }
    for (a, c, n) in &refs {
        if keep.contains(a.as_str()) && keep.contains(c.as_str()) {
            b.edge(&file_node_id(a), &file_node_id(c), EdgeKind::Ref, *n as f64);
        }
    }
    for (a, c, n) in &cochange {
        if keep.contains(a.as_str()) && keep.contains(c.as_str()) {
            b.edge(&file_node_id(a), &file_node_id(c), EdgeKind::Cochange, *n as f64);
        }
    }
    let mut g = b.finish();
    g.truncated = truncated;
    Ok(g)
}

/// 폴더 깊이를 골라, 폴더가 14~90개쯤 되게 묶는다
fn choose_depth(files: &[(String, String, i64)]) -> usize {
    let mut chosen = 1;
    for d in 1..=8 {
        let n = files.iter().map(|f| dir_at(&f.0, d)).collect::<HashSet<_>>().len();
        if n > 90 && d > 1 {
            break;
        }
        chosen = d;
        if n >= 14 {
            break;
        }
    }
    chosen
}

fn dir_overview(files: &[(String, String, i64)], refs: &[(String, String, i64)], cochange: &[(String, String, u32)]) -> Graph {
    let depth = choose_depth(files);
    let mut count: HashMap<String, usize> = HashMap::new();
    for f in files {
        *count.entry(dir_at(&f.0, depth)).or_default() += 1;
    }
    let mut b = Builder::new();
    let mut dirs: Vec<_> = count.iter().collect();
    dirs.sort();
    for (d, n) in dirs {
        b.dir(d, *n);
    }
    let mut agg: HashMap<(String, String, u8), f64> = HashMap::new();
    for (a, c, n) in refs {
        let (da, dc) = (dir_at(a, depth), dir_at(c, depth));
        if da != dc {
            *agg.entry((da, dc, 0)).or_default() += *n as f64;
        }
    }
    for (a, c, n) in cochange {
        let (da, dc) = (dir_at(a, depth), dir_at(c, depth));
        if da != dc {
            *agg.entry((da, dc, 1)).or_default() += *n as f64;
        }
    }
    let mut edges: Vec<_> = agg.into_iter().collect();
    edges.sort_by(|x, y| x.0.cmp(&y.0));
    for ((a, c, k), w) in edges {
        let kind = if k == 0 { EdgeKind::Ref } else { EdgeKind::Cochange };
        b.edge(&dir_node_id(&a), &dir_node_id(&c), kind, w);
    }
    b.finish()
}

/// 폴더 하나를 펼친다: 안의 파일과 그 사이 참조, 바깥은 같은 깊이의 폴더로 묶는다
fn expand_dir(store: &Store, dir: &str, limit: usize) -> Result<Graph> {
    let files = store.file_summaries()?;
    let prefix = format!("{dir}/");
    let inside = |p: &str| if dir.is_empty() { !p.contains('/') } else { p.starts_with(&prefix) };
    let depth = if dir.is_empty() { 1 } else { dir.split('/').count() };
    let mut b = Builder::new();
    let mut mine: Vec<&(String, String, i64)> = files.iter().filter(|f| inside(&f.0)).collect();
    // 파일이 많으면 연결이 많은 것부터
    let refs = store.file_ref_edges(MAX_DEFS)?;
    let mut degree: HashMap<&str, f64> = HashMap::new();
    for (a, c, n) in &refs {
        *degree.entry(a).or_default() += *n as f64;
        *degree.entry(c).or_default() += *n as f64;
    }
    mine.sort_by(|x, y| {
        let dx = degree.get(x.0.as_str()).copied().unwrap_or(0.0);
        let dy = degree.get(y.0.as_str()).copied().unwrap_or(0.0);
        dy.total_cmp(&dx).then(x.0.cmp(&y.0))
    });
    let cap = limit * 2;
    if mine.len() > cap {
        b.graph.truncated = true;
    }
    let keep: HashSet<&str> = mine.iter().take(cap).map(|f| f.0.as_str()).collect();
    for f in mine.iter().take(cap) {
        b.file(&f.0, f.2.max(1) as f64);
    }
    let mut outside: HashMap<(String, String, bool), f64> = HashMap::new();
    for (a, c, n) in &refs {
        match (keep.contains(a.as_str()), keep.contains(c.as_str())) {
            (true, true) => b.edge(&file_node_id(a), &file_node_id(c), EdgeKind::Ref, *n as f64),
            (true, false) if !inside(c) => *outside.entry((a.clone(), dir_at(c, depth), true)).or_default() += *n as f64,
            (false, true) if !inside(a) => *outside.entry((c.clone(), dir_at(a, depth), false)).or_default() += *n as f64,
            _ => {}
        }
    }
    // 바깥 폴더는 연결이 많은 것 몇 개만
    let mut by_dir: HashMap<String, f64> = HashMap::new();
    for ((_, d, _), w) in &outside {
        *by_dir.entry(d.clone()).or_default() += w;
    }
    let mut top: Vec<_> = by_dir.into_iter().collect();
    top.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(&y.0)));
    let top: HashSet<String> = top.into_iter().take(12).map(|x| x.0).collect();
    let file_count = |d: &str| files.iter().filter(|f| dir_at(&f.0, depth) == d).count();
    let mut outs: Vec<_> = outside.into_iter().filter(|((_, d, _), _)| top.contains(d)).collect();
    outs.sort_by(|x, y| x.0.cmp(&y.0));
    for ((f, d, from_me), w) in outs {
        let id = b.dir(&d, file_count(&d));
        if from_me {
            b.edge(&file_node_id(&f), &id, EdgeKind::Ref, w);
        } else {
            b.edge(&id, &file_node_id(&f), EdgeKind::Ref, w);
        }
    }
    for (a, c, n) in store.cochange_pairs(2)? {
        if keep.contains(a.as_str()) && keep.contains(c.as_str()) {
            b.edge(&file_node_id(&a), &file_node_id(&c), EdgeKind::Cochange, n as f64);
        }
    }
    Ok(b.finish())
}

// ── 주변 ──────────────────────────────────────────────────

/// 중심을 찾는다: `f:<경로>`, `s:<id>`, 또는 심볼 이름 / 파일 경로 일부
fn resolve_center(store: &Store, center: &str) -> Result<Option<Center>> {
    if let Some(path) = center.strip_prefix("f:") {
        return Ok(Some(Center::File(path.to_string())));
    }
    if let Some(id) = center.strip_prefix("s:").and_then(|s| s.parse::<i64>().ok()) {
        return Ok(store.symbol(id)?.map(Center::Symbol));
    }
    if let Some(s) = store.symbols_by_name(center, 1)?.into_iter().next() {
        return Ok(Some(Center::Symbol(s)));
    }
    let needle = center.to_ascii_lowercase();
    let found = store.file_summaries()?.into_iter().map(|f| f.0).find(|p| p.to_ascii_lowercase().ends_with(&needle));
    Ok(found.map(Center::File))
}

enum Center {
    File(String),
    Symbol(SymbolRow),
}

/// 파일이나 심볼 하나의 주변: 정의, 호출자, 호출하는 것, 함께 바뀌던 파일
pub fn neighborhood(store: &Store, center: &str, limit: usize) -> Result<Graph> {
    if let Some(dir) = center.strip_prefix("d:") {
        return expand_dir(store, dir, limit);
    }
    let mut b = Builder::new();
    match resolve_center(store, center)? {
        None => {}
        Some(Center::Symbol(s)) => {
            let ups = callers(store, &s, limit)?;
            let me = b.symbol(&s, ups.len().max(1) as f64, None);
            let file = b.file(&s.path, 1.0);
            b.edge(&file, &me, EdgeKind::Defines, 1.0);
            for c in ups.iter().take(limit) {
                let id = b.symbol(c, 1.0, None);
                b.edge(&id, &me, EdgeKind::Calls, 1.0);
            }
            for name in store.callee_names(s.id)? {
                if b.graph.nodes.len() >= limit * 2 {
                    b.graph.truncated = true;
                    break;
                }
                for d in defs(store, &name, 2)? {
                    if d.id == s.id || s.contains(&d) {
                        continue;
                    }
                    let id = b.symbol(&d, 1.0, None);
                    b.edge(&me, &id, EdgeKind::Calls, 1.0);
                }
            }
            for (other, n) in store.cochanged(&s.path, 5)? {
                let id = b.file(&other, 1.0);
                b.edge(&file, &id, EdgeKind::Cochange, n as f64);
            }
        }
        Some(Center::File(path)) => {
            let symbols = store.symbols_in_file(&path)?;
            let me = b.file(&path, symbols.len().max(1) as f64);
            let mut in_refs: HashMap<String, f64> = HashMap::new();
            let mut out_refs: HashMap<String, f64> = HashMap::new();
            for (i, s) in symbols.iter().enumerate() {
                // 파일 안 심볼은 바깥 것만 (메서드는 클래스에 묶여 있다)
                let nested = symbols.iter().enumerate().any(|(j, o)| j != i && o.contains(s) && !s.contains(o));
                if !nested && b.graph.nodes.len() < limit {
                    let id = b.symbol(s, 1.0, None);
                    b.edge(&me, &id, EdgeKind::Defines, 1.0);
                } else if !nested {
                    b.graph.truncated = true;
                }
                for c in callers(store, s, 20)? {
                    if c.path != path {
                        *in_refs.entry(c.path).or_default() += 1.0;
                    }
                }
                for name in store.callee_names(s.id)? {
                    for d in defs(store, &name, 2)? {
                        if d.path != path {
                            *out_refs.entry(d.path).or_default() += 1.0;
                        }
                    }
                }
            }
            let add_files = |b: &mut Builder, refs: HashMap<String, f64>, incoming: bool| {
                let mut v: Vec<_> = refs.into_iter().collect();
                v.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(&y.0)));
                if v.len() > limit {
                    b.graph.truncated = true;
                }
                for (p, n) in v.into_iter().take(limit) {
                    let id = b.file(&p, 1.0);
                    if incoming {
                        b.edge(&id, &me, EdgeKind::Ref, n);
                    } else {
                        b.edge(&me, &id, EdgeKind::Ref, n);
                    }
                }
            };
            add_files(&mut b, in_refs, true);
            add_files(&mut b, out_refs, false);
            for (other, n) in store.cochanged(&path, 8)? {
                let id = b.file(&other, 1.0);
                b.edge(&me, &id, EdgeKind::Cochange, n as f64);
            }
        }
    }
    Ok(b.finish())
}

// ── 변경 영향 반경 ──────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct Impact {
    pub path: String,
    /// 바뀌는 줄에 걸친 심볼 (가장 안쪽)
    pub touched: Vec<String>,
    /// 직접 호출하는 심볼 수 / 그 호출자를 부르는 심볼 수
    pub callers: usize,
    pub callers2: usize,
    /// 영향이 닿는 파일 수와 모듈 수
    pub files: usize,
    pub modules: usize,
    /// 함께 바뀌곤 하던 파일
    pub cochanged: Vec<(String, u32)>,
    /// 영향 범위에 있는 테스트 파일
    pub tests: Vec<String>,
    /// 이름이 흔해 호출자를 셀 수 없는 심볼
    pub ambiguous: Vec<String>,
    /// 바뀌는 심볼의 이름이 모두 프로젝트에서 한 곳에만 정의됨 (이름 기준 호출자가 믿을 만함)
    pub unique: bool,
    /// 엔진이 이 파일의 언어를 분석함. 아니면 호출자 0은 '없음'이 아니라 '모름'
    pub supported: bool,
    /// 바뀌는 심볼에 붙은, 프레임워크가 부른다는 표시 (`@GetMapping`, `[HttpGet]`, `#[tauri::command]`).
    /// 이런 코드는 프레임워크·라우팅·DI가 부르기 때문에 이름으로 찾은 호출자와 테스트에 나타나지 않는다.
    pub framework: Vec<String>,
    /// low | medium | high
    pub risk: String,
    pub graph: Graph,
}

/// 코드에 붙어 있어도 누가 부르는지와 상관없는 표시 (언어 기능·검사·코드 생성·테스트)
const NOT_FRAMEWORK: &[&str] = &[
    "override", "deprecated", "suppresswarnings", "safevarargs", "functionalinterface", "nullable", "nonnull",
    "notnull", "obsolete", "staticmethod", "classmethod", "abstractmethod", "property", "dataclass", "derive",
    "allow", "deny", "warn", "expect", "cfg", "cfg_attr", "inline", "must_use", "doc", "test", "transactional",
    "data", "getter", "setter", "builder", "tostring", "equalsandhashcode", "value", "serializable",
];

/// 시그니처 앞의 애너테이션·데코레이터·속성 중 프레임워크가 부른다는 표시.
/// Java·TypeScript `@Name(...)`, Python `@app.get(...)`, C# `[HttpGet(...)]`, Rust `#[tauri::command]`
pub fn framework_marks(signature: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut s = signature.trim_start();
    loop {
        let (mark, rest) = if let Some(r) = s.strip_prefix("#[") {
            let end = r.find(']').unwrap_or(r.len());
            (format!("#[{}]", r[..end].split('(').next().unwrap_or("").trim()), &r[(end + 1).min(r.len())..])
        } else if let Some(r) = s.strip_prefix('[') {
            let end = r.find(']').unwrap_or(r.len());
            (format!("[{}]", r[..end].split('(').next().unwrap_or("").trim()), &r[(end + 1).min(r.len())..])
        } else if let Some(r) = s.strip_prefix('@') {
            let name_end = r.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.')).unwrap_or(r.len());
            let mut rest = &r[name_end..];
            if rest.starts_with('(') {
                let mut depth = 0;
                let close = rest.char_indices().find(|&(_, c)| {
                    match c {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        _ => {}
                    }
                    depth == 0
                });
                rest = close.map(|(i, _)| &rest[i + 1..]).unwrap_or("");
            }
            (format!("@{}", &r[..name_end]), rest)
        } else {
            break;
        };
        let last = mark.trim_start_matches(['#', '[', '@']).trim_end_matches(']').rsplit(['.', ':']).next().unwrap_or("").to_ascii_lowercase();
        if !last.is_empty() && !NOT_FRAMEWORK.contains(&last.as_str()) && !out.contains(&mark) {
            out.push(mark);
        }
        s = rest.trim_start();
    }
    out
}

/// `path`의 `ranges`(1부터, 양끝 포함) 줄을 바꿀 때 닿는 범위
pub fn impact(store: &Store, path: &str, ranges: &[(u32, u32)]) -> Result<Impact> {
    let symbols = store.symbols_in_file(path)?;
    let overlaps = |s: &SymbolRow| ranges.iter().any(|&(a, z)| s.start_line <= z && a <= s.end_line);
    let hit: Vec<&SymbolRow> = symbols.iter().filter(|s| overlaps(s)).collect();
    // 가장 안쪽만: 바뀐 줄이 메서드 안이면 클래스 전체가 아니라 메서드
    let touched: Vec<&SymbolRow> = hit.iter().copied().filter(|s| !hit.iter().any(|o| o.id != s.id && s.contains(o))).collect();

    let mut b = Builder::new();
    let file = b.file(path, symbols.len().max(1) as f64);
    let mut ambiguous = Vec::new();
    let mut d1: Vec<SymbolRow> = Vec::new();
    let mut d2: Vec<SymbolRow> = Vec::new();
    for s in &touched {
        let id = b.symbol(s, 1.0, Some(0));
        b.edge(&file, &id, EdgeKind::Defines, 1.0);
        if store.count_defs(&s.name)? > MAX_DEFS {
            ambiguous.push(s.name.clone());
            continue;
        }
        for c in callers(store, s, 60)? {
            let cid = b.symbol(&c, 1.0, Some(1));
            b.edge(&cid, &id, EdgeKind::Calls, 1.0);
            if !d1.iter().any(|x| x.id == c.id) {
                d1.push(c);
            }
        }
    }
    let touched_ids: HashSet<i64> = touched.iter().map(|s| s.id).collect();
    d1.retain(|c| !touched_ids.contains(&c.id));
    for c in d1.clone() {
        if d2.len() >= 60 {
            b.graph.truncated = true;
            break;
        }
        for cc in callers(store, &c, 20)? {
            if touched_ids.contains(&cc.id) || d1.iter().any(|x| x.id == cc.id) {
                continue;
            }
            let id = b.symbol(&cc, 1.0, Some(2));
            b.edge(&id, &symbol_node_id(c.id), EdgeKind::Calls, 1.0);
            if !d2.iter().any(|x| x.id == cc.id) {
                d2.push(cc);
            }
        }
    }
    let cochanged = store.cochanged(path, 8)?;
    for (other, n) in &cochanged {
        let id = b.file(other, 1.0);
        b.edge(&file, &id, EdgeKind::Cochange, *n as f64);
    }

    let mut affected: BTreeSet<&str> = d1.iter().chain(d2.iter()).map(|s| s.path.as_str()).collect();
    affected.remove(path);
    let modules: BTreeSet<String> = affected.iter().map(|p| group_of(p)).filter(|g| *g != group_of(path)).collect();
    let mut tests: BTreeSet<String> = affected.iter().filter(|p| is_test_path(p)).map(|p| p.to_string()).collect();
    for (p, _) in &cochanged {
        if is_test_path(p) {
            tests.insert(p.clone());
        }
    }
    for s in &touched {
        if store.count_defs(&s.name)? <= MAX_DEFS {
            for r in store.references(&s.name, 200)? {
                let same_family = Lang::from_path(Path::new(&r.path)).is_some_and(|l| Lang::family(l.name()) == Lang::family(&s.lang));
                if same_family && is_test_path(&r.path) {
                    tests.insert(r.path);
                }
            }
        }
    }

    let mut unique = !touched.is_empty();
    for s in &touched {
        unique &= store.count_defs(&s.name)? == 1;
    }
    let mut framework: Vec<String> = Vec::new();
    for s in &touched {
        for m in framework_marks(&s.signature) {
            if !framework.contains(&m) {
                framework.push(m);
            }
        }
    }
    let callers = d1.len();
    let risk = if callers >= 10 || modules.len() >= 2 || (callers + d2.len()) >= 25 {
        "high"
    } else if callers >= 3
        || cochanged.len() >= 3
        || (callers > 0 && tests.is_empty())
        // 밖에서 불리는 코드(엔드포인트, 이벤트 처리)는 호출자가 안 보여도 낮다고 하지 않는다
        || !framework.is_empty()
    {
        "medium"
    } else {
        "low"
    };
    Ok(Impact {
        path: path.into(),
        touched: touched.iter().map(|s| s.name.clone()).collect(),
        callers,
        callers2: d2.len(),
        files: affected.len(),
        modules: modules.len(),
        cochanged,
        tests: tests.into_iter().collect(),
        ambiguous,
        unique,
        supported: Lang::from_path(Path::new(path)).is_some(),
        framework,
        risk: risk.into(),
        graph: b.finish(),
    })
}

// ── 프로젝트 기억 ──────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct MemoryNote {
    pub id: String,
    pub topic: String,
    /// 프로젝트 기준 경로 (`.lantern/memory/<topic>.md`)
    pub file: String,
    /// 파일 안 줄 번호 (1부터)
    pub line: u32,
    pub text: String,
    /// 연결된 코드 노드 id
    pub links: Vec<String>,
}

/// 메모 한 줄에서 코드 이름·경로를 찾아 노드로 잇는다
fn link_note(store: &Store, text: &str, files: &[String], b: &mut Builder) -> Result<Vec<String>> {
    let mut links = Vec::new();
    let word = regex::Regex::new(r"[A-Za-z_][A-Za-z0-9_]{2,}").expect("정규식");
    let pathlike = regex::Regex::new(r"[\w./-]+\.[A-Za-z0-9]{1,5}").expect("정규식");
    for m in pathlike.find_iter(text) {
        let t = m.as_str().trim_start_matches("./");
        if let Some(p) = files.iter().find(|p| p.as_str() == t || p.ends_with(&format!("/{t}"))) {
            links.push(b.file(p, 1.0));
        }
    }
    let mut seen = HashSet::new();
    for m in word.find_iter(text) {
        let w = m.as_str();
        if !seen.insert(w) || links.len() >= 8 {
            continue;
        }
        let d = store.count_defs(w)?;
        if (1..=MAX_DEFS).contains(&d) {
            if let Some(s) = store.symbols_by_name(w, 1)?.into_iter().next() {
                links.push(b.symbol(&s, 1.0, None));
            }
        }
    }
    links.dedup();
    Ok(links)
}

/// `.lantern/memory/*.md`의 목록 줄(`- …`) 하나가 메모 하나
pub fn memory(store: &Store, root: &Path) -> Result<(Vec<MemoryNote>, Graph)> {
    let dir = root.join(".lantern").join("memory");
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    paths.sort();
    let files: Vec<String> = store.file_summaries()?.into_iter().map(|f| f.0).collect();
    let mut b = Builder::new();
    let mut notes = Vec::new();
    for p in paths {
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        let topic = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        for (i, line) in text.lines().enumerate() {
            let Some(note) = line.trim_start().strip_prefix("- ").or_else(|| line.trim_start().strip_prefix("* ")) else { continue };
            let note = note.trim();
            if note.is_empty() {
                continue;
            }
            let id = format!("m:{topic}#{}", i + 1);
            let (clean, _) = crate::secrets::redact(note);
            b.seen.insert(id.clone());
            b.graph.nodes.push(Node {
                id: id.clone(),
                kind: NodeKind::Memory,
                label: topic.clone(),
                path: Some(format!(".lantern/memory/{topic}.md")),
                line: Some(i as u32 + 1),
                group: "memory".into(),
                weight: 1.0,
                detail: Some(clean.clone()),
                depth: None,
            });
            let links = link_note(store, note, &files, &mut b)?;
            for l in &links {
                b.edge(&id, l, EdgeKind::Mentions, 1.0);
            }
            notes.push(MemoryNote { id, topic: topic.clone(), file: format!(".lantern/memory/{topic}.md"), line: i as u32 + 1, text: clean, links });
        }
    }
    Ok((notes, b.finish()))
}

/// 파일의 한 줄(1부터)을 감싸는 가장 안쪽 심볼. 언어 서버가 준 참조 위치를 호출자로 바꿀 때 쓴다.
pub fn enclosing(store: &Store, path: &str, line: u32) -> Result<Option<Node>> {
    let symbols = store.symbols_in_file(path)?;
    let hit: Vec<&SymbolRow> = symbols.iter().filter(|s| s.start_line <= line && line <= s.end_line).collect();
    let inner = hit.iter().copied().find(|s| !hit.iter().any(|o| o.id != s.id && s.contains(o)));
    let mut b = Builder::new();
    Ok(inner.map(|s| {
        b.symbol(s, 1.0, None);
        b.finish().nodes.remove(0)
    }))
}

/// 검색창용: 이름·경로가 맞는 노드 (심볼 먼저)
pub fn find(store: &Store, query: &str, limit: usize) -> Result<Vec<Node>> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(vec![]);
    }
    let mut b = Builder::new();
    for s in store.symbols_by_name(q, limit)? {
        b.symbol(&s, 1.0, None);
    }
    let lower = q.to_ascii_lowercase();
    for (path, _, n) in store.file_summaries()? {
        if b.graph.nodes.len() >= limit {
            break;
        }
        if path.to_ascii_lowercase().contains(&lower) {
            b.file(&path, n.max(1) as f64);
        }
    }
    let terms = crate::tokenize::query_terms(q);
    if !terms.is_empty() && b.graph.nodes.len() < limit {
        for (s, _) in store.search(&crate::tokenize::fts_query(&terms), limit)? {
            if b.graph.nodes.len() >= limit {
                break;
            }
            b.symbol(&s, 1.0, None);
        }
    }
    Ok(b.finish().nodes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Engine;

    fn project() -> (tempfile::TempDir, Engine) {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        std::fs::create_dir_all(r.join("src/auth")).unwrap();
        std::fs::create_dir_all(r.join("tests")).unwrap();
        std::fs::write(r.join("src/auth/session.ts"), "export function issueSession(user: string) {\n  return signCookie(user);\n}\n\nexport function signCookie(v: string) {\n  return v + '.sig';\n}\n").unwrap();
        std::fs::write(r.join("src/auth/login.ts"), "import { issueSession } from './session';\n\nexport function handleLogin(name: string) {\n  const s = issueSession(name);\n  return s;\n}\n").unwrap();
        std::fs::write(r.join("src/api.ts"), "import { handleLogin } from './auth/login';\n\nexport function route(path: string) {\n  if (path === '/login') return handleLogin('x');\n}\n").unwrap();
        std::fs::write(r.join("tests/session.test.ts"), "import { signCookie } from '../src/auth/session';\n\nexport function testSign() {\n  return signCookie('a');\n}\n").unwrap();
        let mut e = Engine::open(r).unwrap();
        e.refresh().unwrap();
        (dir, e)
    }

    #[test]
    fn migrates_legacy_project_dir() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".khala/memory")).unwrap();
        std::fs::write(d.path().join(".khala/memory/a.md"), "- x\n").unwrap();
        assert!(crate::migrate_legacy_dir(d.path()));
        assert!(d.path().join(".lantern/memory/a.md").exists() && !d.path().join(".khala").exists());
        // 새 폴더가 이미 있으면 건드리지 않는다
        std::fs::create_dir_all(d.path().join(".khala")).unwrap();
        assert!(!crate::migrate_legacy_dir(d.path()));
        assert!(d.path().join(".khala").exists());
    }

    #[test]
    fn groups_and_tests() {
        assert_eq!(group_of("src/auth/login.ts"), "src/auth");
        assert_eq!(group_of("crates/lantern-context/src/lib.rs"), "crates/lantern-context");
        assert_eq!(group_of("docs/a.md"), "docs");
        assert_eq!(group_of("main.rs"), "(root)");
        assert!(is_test_path("tests/a.rs") && is_test_path("src/a.test.ts") && is_test_path("pkg/foo_test.go"));
        assert!(!is_test_path("src/contest.ts"));
        assert!(is_test_path("src/test/java/com/x/UserServiceTest.java") && is_test_path("UserServiceTests.java"));
        assert!(is_test_path("Shop.Tests/OrderTests.cs") && is_test_path("src/OrderServiceTest.cs"));
        assert!(!is_test_path("src/main/java/com/x/Contest.java") && !is_test_path("src/Test.java"));
    }

    #[test]
    fn framework_marks_by_language() {
        assert_eq!(framework_marks("@GetMapping(\"/owners\") public String processFindForm(@RequestParam int page)"), ["@GetMapping"]);
        assert_eq!(framework_marks("@Override @EventListener(OrderPlaced.class) public void on(OrderPlaced e)"), ["@EventListener"]);
        assert_eq!(framework_marks("[HttpGet(\"{id}\")] [Authorize] public IActionResult Get(int id)"), ["[HttpGet]", "[Authorize]"]);
        assert_eq!(framework_marks("#[tauri::command] #[allow(dead_code)] fn git_repos(state: State)"), ["#[tauri::command]"]);
        assert_eq!(framework_marks("@app.get(\"/items/{id}\") async def read_item(id: int)"), ["@app.get"]);
        assert!(framework_marks("@Override public String toString()").is_empty());
        assert!(framework_marks("@staticmethod def parse(raw)").is_empty());
        assert!(framework_marks("pub fn load(path: &str) -> Config").is_empty());
    }

    #[test]
    fn impact_flags_framework_entry_points_and_unknown_languages() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        std::fs::create_dir_all(r.join("src")).unwrap();
        std::fs::write(r.join("src/OwnerController.java"), "package app;\n\npublic class OwnerController {\n    @GetMapping(\"/owners\")\n    public String list(Model model) {\n        return \"owners\";\n    }\n}\n").unwrap();
        std::fs::write(r.join("src/api.py"), "@app.get(\"/items\")\ndef list_items():\n    return []\n").unwrap();
        std::fs::write(r.join("src/Owner.kt"), "class Owner(val name: String)\n").unwrap();
        let mut e = Engine::open(r).unwrap();
        e.refresh().unwrap();

        // 호출자가 안 보여도 엔드포인트는 '낮음'이 아니다
        let i = impact(&e.store, "src/OwnerController.java", &[(6, 6)]).unwrap();
        assert_eq!((i.callers, i.framework.clone(), i.risk.as_str(), i.supported), (0, vec!["@GetMapping".to_string()], "medium", true));
        let p = impact(&e.store, "src/api.py", &[(3, 3)]).unwrap();
        assert_eq!(p.framework, ["@app.get"]);
        // 분석하지 않는 언어: 0은 '없음'이 아니라 '모름'
        let k = impact(&e.store, "src/Owner.kt", &[(1, 1)]).unwrap();
        assert!(!k.supported && k.touched.is_empty());
    }

    #[test]
    fn impact_stays_within_a_language_family() {
        // Spring 백엔드와 React 화면에 같은 이름 getUser가 있어도 서로 호출로 잇지 않는다
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        std::fs::create_dir_all(r.join("api/src/main/java/app")).unwrap();
        std::fs::create_dir_all(r.join("api/src/test/java/app")).unwrap();
        std::fs::create_dir_all(r.join("web/src")).unwrap();
        std::fs::write(r.join("api/src/main/java/app/UserService.java"), "package app;\n\npublic class UserService {\n    public User getUser(long id) {\n        return repo.findById(id);\n    }\n}\n").unwrap();
        std::fs::write(r.join("api/src/main/java/app/UserController.java"), "package app;\n\npublic class UserController {\n    public User show(long id) {\n        return service.getUser(id);\n    }\n}\n").unwrap();
        std::fs::write(r.join("api/src/test/java/app/UserServiceTest.java"), "package app;\n\nclass UserServiceTest {\n    void gets() {\n        service.getUser(1);\n    }\n}\n").unwrap();
        std::fs::write(r.join("web/src/user.ts"), "export async function loadProfile(id: number) {\n  return getUser(id);\n}\n").unwrap();
        std::fs::write(r.join("web/src/user.test.ts"), "export function testProfile() {\n  return getUser(1);\n}\n").unwrap();
        let mut e = Engine::open(r).unwrap();
        e.refresh().unwrap();

        let i = impact(&e.store, "api/src/main/java/app/UserService.java", &[(5, 5)]).unwrap();
        assert_eq!(i.touched, vec!["getUser"]);
        let callers: Vec<&str> = i.graph.nodes.iter().filter(|n| n.depth == Some(1)).map(|n| n.label.as_str()).collect();
        assert!(callers.contains(&"show") && callers.contains(&"gets"), "{callers:?}");
        assert!(!callers.contains(&"loadProfile") && !callers.contains(&"testProfile"), "다른 언어의 호출: {callers:?}");
        assert_eq!(i.tests, vec!["api/src/test/java/app/UserServiceTest.java"]);

        let g = overview(&e.store, 100).unwrap();
        let web_to_api = g.edges.iter().any(|x| x.source.contains("web/") && x.target.contains("api/"));
        assert!(!web_to_api, "{:?}", g.edges);
    }

    #[test]
    fn overview_links_files() {
        let (_d, e) = project();
        let g = overview(&e.store, 100).unwrap();
        assert_eq!(g.nodes.len(), 4);
        let has = |a: &str, b: &str| g.edges.iter().any(|x| x.source == file_node_id(a) && x.target == file_node_id(b) && x.kind == EdgeKind::Ref);
        assert!(has("src/auth/login.ts", "src/auth/session.ts"), "{:?}", g.edges);
        assert!(has("src/api.ts", "src/auth/login.ts"));
        let small = overview(&e.store, 2).unwrap();
        assert!(small.truncated && small.nodes.len() == 2);
    }

    #[test]
    fn big_projects_group_by_folder() {
        assert_eq!(dir_at("src/auth/login.ts", 1), "src");
        assert_eq!(dir_at("src/auth/login.ts", 5), "src/auth");
        assert_eq!(dir_at("main.rs", 2), "");
        // 폴더 20개 × 파일 10개: 폴더 단위 지도
        let files: Vec<(String, String, i64)> = (0..200).map(|i| (format!("app/m{}/f{i}.ts", i % 20), "ts".into(), 1)).collect();
        assert_eq!(choose_depth(&files), 2);
        let refs = vec![("app/m0/f0.ts".to_string(), "app/m1/f1.ts".to_string(), 3), ("app/m0/f20.ts".to_string(), "app/m1/f21.ts".to_string(), 2)];
        let g = dir_overview(&files, &refs, &[]);
        assert_eq!(g.nodes.len(), 20);
        assert!(g.nodes.iter().all(|n| n.kind == NodeKind::Dir));
        let e = g.edges.iter().find(|e| e.source == "d:app/m0").unwrap();
        assert_eq!((e.target.as_str(), e.weight), ("d:app/m1", 5.0), "파일 간 참조를 폴더로 합친다");
    }

    #[test]
    fn expands_a_folder() {
        let (_d, e) = project();
        let g = neighborhood(&e.store, "d:src/auth", 20).unwrap();
        let files: Vec<&str> = g.nodes.iter().filter(|n| n.kind == NodeKind::File).filter_map(|n| n.path.as_deref()).collect();
        assert!(files.contains(&"src/auth/login.ts") && files.contains(&"src/auth/session.ts"), "{files:?}");
        assert!(!files.contains(&"src/api.ts"), "바깥 파일은 폴더로 묶는다");
        assert!(g.edges.iter().any(|x| x.source == file_node_id("src/auth/login.ts") && x.target == file_node_id("src/auth/session.ts")));
    }

    #[test]
    fn neighborhood_of_symbol_and_file() {
        let (_d, e) = project();
        let g = neighborhood(&e.store, "issueSession", 20).unwrap();
        let labels: Vec<&str> = g.nodes.iter().map(|n| n.label.as_str()).collect();
        assert!(labels.contains(&"handleLogin"), "호출자: {labels:?}");
        assert!(labels.contains(&"signCookie"), "호출하는 것: {labels:?}");
        let f = neighborhood(&e.store, "f:src/auth/session.ts", 20).unwrap();
        assert!(f.nodes.iter().any(|n| n.path.as_deref() == Some("src/auth/login.ts") && n.kind == NodeKind::File));
        assert!(neighborhood(&e.store, "없는이름", 20).unwrap().nodes.is_empty());
    }

    #[test]
    fn impact_reaches_callers_and_tests() {
        let (_d, e) = project();
        // signCookie 본문(6줄)을 바꾸면: 호출자 issueSession(1단계), testSign(1단계), handleLogin(2단계)
        let i = impact(&e.store, "src/auth/session.ts", &[(6, 6)]).unwrap();
        assert_eq!(i.touched, vec!["signCookie"]);
        assert!(i.unique, "signCookie는 한 곳에만 정의됨");
        assert_eq!(i.callers, 2, "{i:?}");
        assert_eq!(i.callers2, 1);
        assert!(i.tests.iter().any(|t| t == "tests/session.test.ts"));
        assert!(i.graph.nodes.iter().any(|n| n.label == "handleLogin" && n.depth == Some(2)));
        let none = impact(&e.store, "src/auth/session.ts", &[(4, 4)]).unwrap();
        assert!(none.touched.is_empty() && none.callers == 0 && none.risk == "low");
    }

    #[test]
    fn memory_links_code() {
        let (d, e) = project();
        std::fs::create_dir_all(d.path().join(".lantern/memory")).unwrap();
        std::fs::write(d.path().join(".lantern/memory/conventions.md"), "# conventions\n\n- 세션은 항상 `issueSession`으로 만든다 (src/auth/session.ts)\n- 토큰 sk-ant-api03-abcdefghijklmnopqrstuvwx 는 쓰지 않는다\n").unwrap();
        let (notes, g) = memory(&e.store, d.path()).unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].line, 3);
        assert!(notes[0].links.iter().any(|l| l.starts_with("s:")) && notes[0].links.contains(&file_node_id("src/auth/session.ts")));
        assert!(!notes[1].text.contains("sk-ant-api03"), "비밀은 가린다");
        assert!(g.edges.iter().all(|x| x.kind == EdgeKind::Mentions));
    }

    #[test]
    fn enclosing_symbol_of_a_line() {
        let (_d, e) = project();
        assert_eq!(enclosing(&e.store, "src/auth/login.ts", 4).unwrap().unwrap().label, "handleLogin");
        assert!(enclosing(&e.store, "src/auth/login.ts", 1).unwrap().is_none(), "import 줄은 어떤 심볼 안도 아니다");
    }

    #[test]
    fn find_symbols_and_files() {
        let (_d, e) = project();
        let r = find(&e.store, "login", 10).unwrap();
        assert!(r.iter().any(|n| n.path.as_deref() == Some("src/auth/login.ts")));
        assert!(find(&e.store, "handleLogin", 10).unwrap()[0].kind == NodeKind::Symbol);
    }
}
