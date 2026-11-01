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

pub(crate) const MAX_FILE_BYTES: u64 = 1024 * 1024;

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
pub(crate) fn is_unindexable(bytes: &[u8]) -> bool {
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return true;
    }
    let lines = bytes.iter().filter(|b| **b == b'\n').count() + 1;
    bytes.len() > 20_000 && bytes.len() / lines > 500
}

/// 파싱할 파일 한 묶음의 크기. 묶음마다 병렬로 파싱하고, 결과는 경로 순서대로 저장한다
/// (같은 입력이면 늘 같은 심볼 id가 나와 순위의 동점 처리가 흔들리지 않게).
const PARSE_BATCH: usize = 256;
/// 이보다 적게 바뀌었으면 스레드를 띄우지 않고 그냥 파싱한다 (평소 증분 갱신)
const PARALLEL_MIN: usize = 32;

/// 읽고 파싱한 결과
enum Work {
    /// 내용이 같다 (시간만 바뀜)
    Touch { id: i64 },
    Parsed { hash: String, parsed: Parsed },
    Skip,
    Error,
}

struct Job {
    file: SourceFile,
    mtime: i64,
    size: i64,
    prev: Option<(i64, String)>,
}

fn work(job: &Job, parser: &mut Parser) -> Work {
    let Ok(bytes) = fs::read(&job.file.abs) else { return Work::Error };
    if is_unindexable(&bytes) {
        return Work::Skip;
    }
    let hash = blake3::hash(&bytes).to_hex().to_string();
    if let Some((id, _)) = job.prev.as_ref().filter(|(_, h)| *h == hash) {
        return Work::Touch { id: *id };
    }
    match parser.parse(job.file.lang, &bytes) {
        Ok(parsed) => Work::Parsed { hash, parsed },
        Err(e) => {
            eprintln!("lantern: {} 파싱 실패: {e}", job.file.rel);
            Work::Parsed { hash, parsed: Parsed::default() }
        }
    }
}

/// 묶음 하나를 스레드 여러 개로 나눠 파싱한다 (스레드마다 파서 하나)
fn work_parallel(jobs: &[Job], parsers: &mut [Parser]) -> Vec<Work> {
    let per = jobs.len().div_ceil(parsers.len()).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .chunks(per)
            .zip(parsers.iter_mut())
            .map(|(chunk, parser)| scope.spawn(move || chunk.iter().map(|j| work(j, parser)).collect::<Vec<_>>()))
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    })
}

pub fn index_project(root: &Path, store: &Store, parser: &mut Parser) -> Result<IndexStats> {
    let started = Instant::now();
    let mut stats = IndexStats::default();
    let existing = store.files()?;
    let mut seen = HashSet::new();

    // 1. 순회: 크기·시간이 그대로인 파일은 건너뛰고, 나머지는 읽을 일로 모은다
    let mut jobs: Vec<Job> = Vec::new();
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
        jobs.push(Job { prev: prev.map(|p| (p.id, p.hash.clone())), file, mtime, size });
    }

    // 2. 읽고 파싱해서 저장 (바뀐 파일이 많으면 묶음마다 병렬로)
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8);
    let mut extra: Vec<Parser> = Vec::new();
    if jobs.len() >= PARALLEL_MIN && threads > 1 {
        for _ in 1..threads {
            extra.push(Parser::new()?);
        }
    }
    store.begin()?;
    let result = (|| -> Result<()> {
        for batch in jobs.chunks(PARSE_BATCH) {
            let results: Vec<Work> = if extra.is_empty() {
                batch.iter().map(|j| work(j, parser)).collect()
            } else {
                let mut all: Vec<Parser> = Vec::with_capacity(threads);
                all.push(std::mem::replace(parser, Parser::new()?));
                all.append(&mut extra);
                let r = work_parallel(batch, &mut all);
                *parser = all.remove(0);
                extra = all;
                r
            };
            for (job, w) in batch.iter().zip(results) {
                match w {
                    Work::Error => stats.errors += 1,
                    Work::Skip => stats.skipped += 1,
                    Work::Touch { id } => {
                        seen.insert(job.file.rel.clone());
                        store.touch_file(id, job.mtime, job.size)?;
                        stats.unchanged += 1;
                    }
                    Work::Parsed { hash, parsed } => {
                        seen.insert(job.file.rel.clone());
                        store.replace_file(&job.file.rel, job.file.lang.name(), &hash, job.mtime, job.size, &parsed)?;
                        stats.indexed += 1;
                    }
                }
            }
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
