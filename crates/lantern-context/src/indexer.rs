//! 프로젝트 순회와 증분 인덱싱.

use crate::lang::Lang;
use crate::parse::{Parsed, Parser};
use crate::store::Store;
use anyhow::Result;
use ignore::WalkBuilder;
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};

const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// .gitignore가 없어도 항상 건너뛰는 폴더
const SKIP_DIRS: &[&str] = &[
    "node_modules", "target", "dist", "build", "out", ".next", ".nuxt", "vendor", "__pycache__",
    ".venv", "venv", ".git", ".lantern", "coverage",
];

#[derive(Debug, Default, Serialize)]
pub struct IndexStats {
    pub scanned: usize,
    pub indexed: usize,
    pub unchanged: usize,
    pub removed: usize,
    pub skipped: usize,
    pub errors: usize,
    pub cochange_pairs: Option<usize>,
    pub elapsed_ms: u128,
}

pub struct SourceFile {
    pub abs: PathBuf,
    pub rel: String,
    pub lang: Lang,
}

pub fn walk(root: &Path) -> Vec<SourceFile> {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(true)
        .git_ignore(true)
        .require_git(false)
        .filter_entry(|e| {
            let is_dir = e.file_type().is_some_and(|t| t.is_dir());
            !(is_dir && e.file_name().to_str().is_some_and(|n| SKIP_DIRS.contains(&n)))
        });
    let extra_ignore = root.join(".lantern").join("ignore");
    if extra_ignore.exists() {
        builder.add_ignore(extra_ignore);
    }

    let mut files: Vec<SourceFile> = builder
        .build()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|e| {
            let lang = Lang::from_path(e.path())?;
            let rel = e.path().strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/");
            Some(SourceFile { abs: e.path().to_path_buf(), rel, lang })
        })
        .collect();
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    files
}

/// 바이너리이거나 번들·압축된 파일은 인덱싱하지 않는다.
fn is_unindexable(bytes: &[u8]) -> bool {
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return true;
    }
    let lines = bytes.iter().filter(|b| **b == b'\n').count() + 1;
    bytes.len() > 20_000 && bytes.len() / lines > 500
}

pub fn index_project(root: &Path, store: &Store, parser: &mut Parser) -> Result<IndexStats> {
    let started = Instant::now();
    let mut stats = IndexStats::default();
    let existing = store.files()?;
    let mut seen = HashSet::new();

    store.begin()?;
    let result = (|| -> Result<()> {
        for file in walk(root) {
            stats.scanned += 1;
            let Ok(meta) = fs::metadata(&file.abs) else {
                stats.errors += 1;
                continue;
            };
            if meta.len() > MAX_FILE_BYTES {
                stats.skipped += 1;
                continue;
            }
            let size = meta.len() as i64;
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as i64)
                .unwrap_or(0);

            let prev = existing.get(&file.rel);
            if prev.is_some_and(|p| p.mtime == mtime && p.size == size) {
                seen.insert(file.rel);
                stats.unchanged += 1;
                continue;
            }

            let Ok(bytes) = fs::read(&file.abs) else {
                stats.errors += 1;
                continue;
            };
            if is_unindexable(&bytes) {
                stats.skipped += 1;
                continue;
            }
            seen.insert(file.rel.clone());

            let hash = blake3::hash(&bytes).to_hex().to_string();
            if let Some(p) = prev.filter(|p| p.hash == hash) {
                store.touch_file(p.id, mtime, size)?;
                stats.unchanged += 1;
                continue;
            }

            let parsed = parser.parse(file.lang, &bytes).unwrap_or_else(|e| {
                eprintln!("lantern: {} 파싱 실패: {e}", file.rel);
                stats.errors += 1;
                Parsed::default()
            });
            store.replace_file(&file.rel, file.lang.name(), &hash, mtime, size, &parsed)?;
            stats.indexed += 1;
        }

        for (path, row) in &existing {
            if !seen.contains(path) {
                store.remove_file(row.id)?;
                stats.removed += 1;
            }
        }
        Ok(())
    })();

    match result {
        Ok(()) => store.commit()?,
        Err(e) => {
            store.rollback();
            return Err(e);
        }
    }
    stats.elapsed_ms = started.elapsed().as_millis();
    Ok(stats)
}
