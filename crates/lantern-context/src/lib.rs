//! Lantern 맥락 엔진.
//!
//! 프로젝트 코드를 로컬에서 인덱싱하고, 질문에 필요한 코드만 골라 토큰 예산 안에서 조립한다.
//! 설계: `docs/Phase0_맥락엔진_설계.md`

pub mod assemble;
pub mod git;
pub mod graph;
pub mod indexer;
pub mod lang;
pub mod mcp;
pub mod parse;
pub mod secrets;
pub mod store;
pub mod tokenize;

use anyhow::{Context, Result};
use assemble::{ContextRequest, ContextResult};
use indexer::IndexStats;
use parse::Parser;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use store::{Store, SymbolRow};

const SYMBOL_BODY_LIMIT: usize = 6000;

/// 프로젝트 설정 폴더 이름
pub const PROJECT_DIR: &str = ".lantern";
/// 예전 이름 (KHALA 시절). 있으면 한 번 옮긴다.
pub const LEGACY_PROJECT_DIR: &str = ".khala";

/// 예전 설정 폴더(`.khala`)만 있으면 `.lantern`으로 옮긴다. 옮겼으면 true.
/// 새 폴더가 이미 있으면 아무것도 하지 않는다 (덮어쓰지 않음).
pub fn migrate_legacy_dir(root: &Path) -> bool {
    let (old, new) = (root.join(LEGACY_PROJECT_DIR), root.join(PROJECT_DIR));
    old.is_dir() && !new.exists() && std::fs::rename(&old, &new).is_ok()
}

/// 인덱스 파일 위치.
/// 기본은 `<root>/.lantern/index.db`. 환경변수 `LANTERN_INDEX_DIR`이 있으면 프로젝트 폴더를 건드리지 않고
/// `<LANTERN_INDEX_DIR>/<루트 경로 해시>.db`에 둔다.
pub fn index_path(root: &Path) -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("LANTERN_INDEX_DIR").filter(|d| !d.is_empty()) {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir)?;
        let key = blake3::hash(root.to_string_lossy().as_bytes()).to_hex();
        return Ok(dir.join(format!("{}.db", &key[..16])));
    }
    let dir = root.join(PROJECT_DIR);
    std::fs::create_dir_all(&dir)?;
    let gitignore = dir.join(".gitignore");
    if !gitignore.exists() {
        std::fs::write(&gitignore, "index.db*\n")?;
    }
    Ok(dir.join("index.db"))
}

pub struct Engine {
    pub root: PathBuf,
    pub store: Store,
    parser: Parser,
}

impl Engine {
    /// 인덱스를 열거나 만든다. 위치는 [`index_path`] 참고.
    pub fn open(root: &Path) -> Result<Self> {
        let root = std::path::absolute(root)?;
        anyhow::ensure!(root.is_dir(), "프로젝트 폴더가 아닙니다: {}", root.display());
        migrate_legacy_dir(&root);
        let db = index_path(&root)?;
        let store = Store::open(&db).context("인덱스 열기")?;
        Ok(Self { root, store, parser: Parser::new()? })
    }

    /// 인덱스를 지우고 새로 만든다.
    pub fn open_fresh(root: &Path) -> Result<Self> {
        let db = index_path(&std::path::absolute(root)?)?;
        for suffix in ["", "-wal", "-shm"] {
            let mut p = db.clone().into_os_string();
            p.push(suffix);
            let _ = std::fs::remove_file(p);
        }
        Self::open(root)
    }

    /// 증분 인덱싱 + git 동시 변경 갱신. 바뀐 파일이 없으면 파일 stat 비용만 든다.
    pub fn refresh(&mut self) -> Result<IndexStats> {
        let mut stats = indexer::index_project(&self.root, &self.store, &mut self.parser)?;
        stats.cochange_pairs = git::refresh_cochange(&self.root, &self.store).unwrap_or_else(|e| {
            eprintln!("lantern: git 이력 분석 건너뜀: {e}");
            None
        });
        Ok(stats)
    }

    pub fn context(&self, req: &ContextRequest) -> Result<ContextResult> {
        assemble::assemble(&self.store, &self.root, req)
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<(SymbolRow, f64)>> {
        let terms = tokenize::query_terms(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        self.store.search(&tokenize::fts_query(&terms), limit)
    }

    pub fn search_report(&self, query: &str, limit: usize) -> Result<String> {
        let hits = self.search(query, limit)?;
        if hits.is_empty() {
            return Ok(format!("'{query}'에 해당하는 심볼이 없습니다."));
        }
        let mut out = String::new();
        for (s, score) in hits {
            let _ = writeln!(
                out,
                "- `{}` ({}) {}:{} · {:.2}\n  {}",
                s.name, s.kind, s.path, s.start_line, score, s.signature
            );
        }
        Ok(out)
    }

    /// 심볼 정의 본문과 호출자·호출 대상
    pub fn symbol_report(&self, name: &str) -> Result<String> {
        let defs = self.store.symbols_by_name(name, 5)?;
        if defs.is_empty() {
            let similar = self.search_report(name, 5)?;
            return Ok(format!("`{name}` 정의를 찾지 못했습니다. 비슷한 후보:\n{similar}"));
        }
        let mut out = String::new();
        let mut sources: HashMap<String, Option<Vec<u8>>> = HashMap::new();
        for d in &defs {
            let _ = writeln!(out, "## `{}` ({}) — {}:{}-{}", d.name, d.kind, d.path, d.start_line, d.end_line);
            if let Some(doc) = &d.doc {
                let _ = writeln!(out, "> {}", doc.replace('\n', "\n> "));
            }
            let body = secrets::redact(&self.read_symbol(&mut sources, d).unwrap_or_else(|| d.signature.clone())).0;
            let body = if body.chars().count() > SYMBOL_BODY_LIMIT {
                body.chars().take(SYMBOL_BODY_LIMIT).collect::<String>() + "\n…(잘림)"
            } else {
                body
            };
            let _ = writeln!(out, "```{}\n{}\n```", lang::Lang::fence(&d.lang), body.trim_end());

            let callers: Vec<String> = self
                .store
                .caller_ids(&d.name, 20)?
                .into_iter()
                .filter_map(|id| self.store.symbol(id).ok().flatten())
                .filter(|s| s.id != d.id)
                .map(|s| format!("`{}` {}:{}", s.name, s.path, s.start_line))
                .collect();
            if !callers.is_empty() {
                let _ = writeln!(out, "**사용하는 곳:** {}", callers.join(", "));
            }

            let mut callees = Vec::new();
            for n in self.store.callee_names(d.id)? {
                let found = self.store.symbols_by_name(&n, 2)?;
                if let Some(s) = found.first() {
                    callees.push(format!("`{}` {}:{}", s.name, s.path, s.start_line));
                }
            }
            if !callees.is_empty() {
                let _ = writeln!(out, "**사용하는 심볼:** {}", callees.join(", "));
            }
            out.push('\n');
        }
        Ok(out)
    }

    pub fn references_report(&self, name: &str) -> Result<String> {
        let refs = self.store.references(name, 100)?;
        if refs.is_empty() {
            return Ok(format!("`{name}` 참조를 찾지 못했습니다."));
        }
        let mut lines_cache: HashMap<String, Vec<String>> = HashMap::new();
        let mut out = format!("`{name}` 참조 {}건\n", refs.len());
        for r in refs {
            let lines = lines_cache.entry(r.path.clone()).or_insert_with(|| {
                std::fs::read(self.root.join(&r.path))
                    .map(|b| String::from_utf8_lossy(&b).lines().map(str::to_string).collect())
                    .unwrap_or_default()
            });
            let text = secrets::redact(lines.get(r.line as usize - 1).map(|l| l.trim()).unwrap_or("")).0;
            let within = r.enclosing.map(|e| format!(" (`{e}` 안)")).unwrap_or_default();
            let _ = writeln!(out, "- {}:{}{} {}", r.path, r.line, within, text);
        }
        Ok(out)
    }

    fn read_symbol(&self, cache: &mut HashMap<String, Option<Vec<u8>>>, s: &SymbolRow) -> Option<String> {
        let src = cache
            .entry(s.path.clone())
            .or_insert_with(|| std::fs::read(self.root.join(&s.path)).ok())
            .as_ref()?;
        src.get(s.start_byte..s.end_byte).map(|b| String::from_utf8_lossy(b).into_owned())
    }
}
