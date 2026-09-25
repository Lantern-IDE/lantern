//! 파일 트리, 빠른 열기용 파일 목록, 파일 읽기·쓰기.

use crate::state::rel_path;
use anyhow::{bail, Context, Result};
use ignore::WalkBuilder;
use serde::Serialize;
use std::path::Path;

const MAX_OPEN_BYTES: u64 = 5 * 1024 * 1024;
const MAX_LIST_FILES: usize = 50_000;
const ALWAYS_HIDDEN: &[&str] = &[".git", "node_modules", "target", "__pycache__", ".venv", "venv"];

#[derive(Debug, Serialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

fn walker(root: &Path, dir: &Path) -> WalkBuilder {
    let mut b = WalkBuilder::new(dir);
    b.hidden(false)
        .git_ignore(true)
        .require_git(false)
        .filter_entry(|e| !e.file_name().to_str().is_some_and(|n| ALWAYS_HIDDEN.contains(&n)));
    let extra = root.join(".lantern").join("ignore");
    if extra.exists() {
        b.add_ignore(extra);
    }
    b
}

/// 폴더 한 단계. 폴더 먼저, 이름순.
pub fn list_dir(root: &Path, dir: &Path) -> Result<Vec<Entry>> {
    if !dir.is_dir() {
        bail!("폴더가 아닙니다: {}", rel_path(root, dir));
    }
    let mut out: Vec<Entry> = walker(root, dir)
        .max_depth(Some(1))
        .build()
        .filter_map(|e| e.ok())
        .filter(|e| e.depth() == 1)
        .map(|e| Entry {
            name: e.file_name().to_string_lossy().into_owned(),
            path: rel_path(root, e.path()),
            is_dir: e.file_type().is_some_and(|t| t.is_dir()),
        })
        .collect();
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(out)
}

/// 빠른 열기(Ctrl+P)용 전체 파일 목록
pub fn list_files(root: &Path) -> Vec<String> {
    let mut out: Vec<String> = walker(root, root)
        .build()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .take(MAX_LIST_FILES)
        .map(|e| rel_path(root, e.path()))
        .collect();
    out.sort();
    out
}

#[derive(Debug, Serialize)]
pub struct TextMatch {
    pub line: u32,
    /// 줄 안에서 일치 시작 위치 (문자 단위)
    pub col: u32,
    pub len: u32,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct FileMatches {
    pub path: String,
    pub matches: Vec<TextMatch>,
}

const SEARCH_MAX_TOTAL: usize = 2000;
const SEARCH_MAX_PER_FILE: usize = 100;
const SEARCH_MAX_FILE_BYTES: u64 = 1024 * 1024;

/// 프로젝트 전체 텍스트 검색 (VS Code 검색 뷰). 대소문자 구분은 선택.
pub fn search_text(root: &Path, query: &str, case_sensitive: bool) -> Vec<FileMatches> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle = if case_sensitive { query.to_string() } else { query.to_lowercase() };
    let mut out = Vec::new();
    let mut total = 0;
    for e in walker(root, root).build().filter_map(|e| e.ok()) {
        if total >= SEARCH_MAX_TOTAL {
            break;
        }
        if !e.file_type().is_some_and(|t| t.is_file())
            || e.metadata().map(|m| m.len() > SEARCH_MAX_FILE_BYTES).unwrap_or(true)
        {
            continue;
        }
        let Ok(bytes) = std::fs::read(e.path()) else { continue };
        if bytes[..bytes.len().min(8192)].contains(&0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        let mut matches = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let hay = if case_sensitive { line.to_string() } else { line.to_lowercase() };
            if let Some(byte) = hay.find(&needle) {
                // 소문자 변환으로 바이트 위치가 달라질 수 있어 문자 단위로 센다.
                let col = hay[..byte].chars().count();
                let preview: String = line.chars().take(400).collect();
                matches.push(TextMatch {
                    line: i as u32 + 1,
                    col: col as u32,
                    len: needle.chars().count() as u32,
                    text: preview,
                });
                if matches.len() >= SEARCH_MAX_PER_FILE {
                    break;
                }
            }
        }
        if !matches.is_empty() {
            total += matches.len();
            out.push(FileMatches { path: rel_path(root, e.path()), matches });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}


// ── 파일 작업 (탐색기) ─────────────────────────────────

fn check_name(name: &str) -> Result<()> {
    if name.is_empty() || name.contains(['\\', ':', '*', '?', '"', '<', '>', '|']) || name.ends_with(' ') || name.ends_with('.') {
        bail!("쓸 수 없는 이름입니다: {name}");
    }
    Ok(())
}

pub fn create_file(path: &Path) -> Result<()> {
    check_name(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())?;
    if path.exists() {
        bail!("이미 있습니다: {}", path.display());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::File::create_new(path)?;
    Ok(())
}

pub fn create_dir(path: &Path) -> Result<()> {
    check_name(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())?;
    if path.exists() {
        bail!("이미 있습니다: {}", path.display());
    }
    std::fs::create_dir_all(path)?;
    Ok(())
}

pub fn rename(from: &Path, to: &Path) -> Result<()> {
    check_name(&to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())?;
    if !from.exists() {
        bail!("없는 파일입니다: {}", from.display());
    }
    // 대소문자만 바꾸는 경우(Windows)는 같은 파일로 보이므로 존재 검사를 건너뛴다.
    let case_only = from.to_string_lossy().to_lowercase() == to.to_string_lossy().to_lowercase();
    if to.exists() && !case_only {
        bail!("같은 이름이 이미 있습니다: {}", to.display());
    }
    std::fs::rename(from, to)?;
    Ok(())
}

/// 지우지 않고 휴지통으로 보낸다 (되살릴 수 있게).
pub fn delete_to_trash(path: &Path) -> Result<()> {
    if !path.exists() {
        bail!("없는 파일입니다: {}", path.display());
    }
    trash::delete(path).context("휴지통으로 옮기지 못했습니다")
}

/// OS 파일 탐색기에서 보여준다.
pub fn reveal_in_os(path: &Path) -> Result<()> {
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("explorer");
        c.arg(format!("/select,{}", path.display()));
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg("-R").arg(path);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(path.parent().unwrap_or(path));
        c
    };
    cmd.spawn().context("파일 탐색기를 열지 못했습니다")?;
    Ok(())
}

pub fn read_text(path: &Path) -> Result<String> {
    let meta = std::fs::metadata(path).with_context(|| format!("{} 없음", path.display()))?;
    if meta.len() > MAX_OPEN_BYTES {
        bail!("파일이 너무 큽니다 ({}MB). 5MB 이하만 열 수 있습니다", meta.len() / 1024 / 1024);
    }
    let bytes = std::fs::read(path)?;
    if bytes[..bytes.len().min(8192)].contains(&0) {
        bail!("바이너리 파일은 열 수 없습니다");
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// 파일들에서 `query`를 모두 `replacement`로 바꾼다 (정규식 아님, 글자 그대로).
/// 되돌리기용 원본과 바꾼 개수를 돌려준다.
pub fn replace_text(
    root: &Path,
    paths: &[String],
    query: &str,
    replacement: &str,
    case_sensitive: bool,
) -> Result<(usize, crate::state::Originals)> {
    if query.is_empty() {
        bail!("찾을 내용을 입력하세요");
    }
    let re = regex::RegexBuilder::new(&regex::escape(query)).case_insensitive(!case_sensitive).build()?;
    let mut count = 0;
    let mut originals = Vec::new();
    for rel in paths {
        let path = crate::state::resolve_in_root(root, rel)?;
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let n = re.find_iter(&text).count();
        if n == 0 {
            continue;
        }
        let new = re.replace_all(&text, regex::NoExpand(replacement));
        std::fs::write(&path, new.as_bytes()).with_context(|| format!("{rel} 쓰기 실패"))?;
        originals.push((path, Some(text)));
        count += n;
    }
    Ok((count, originals))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searches_text_and_reads_branch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::write(root.join("src/a.rs"), "fn main() {\n    let 한글 = Estimate();\n}\n").unwrap();
        std::fs::write(root.join("node_modules/x/b.js"), "estimate()").unwrap();

        let r = search_text(root, "estimate", false);
        assert_eq!(r.len(), 1, "node_modules는 제외");
        assert_eq!(r[0].path, "src/a.rs");
        let m = &r[0].matches[0];
        assert_eq!((m.line, m.col, m.len), (2, 13, 8), "열은 문자 단위 (한글 포함)");
        assert!(search_text(root, "estimate", true).is_empty(), "대소문자 구분");

        // 바꾸기: 대소문자 무시, $1 같은 문자는 그대로
        let (n, orig) = replace_text(root, &["src/a.rs".into()], "estimate", "$1추정", false).unwrap();
        assert_eq!(n, 1);
        assert!(std::fs::read_to_string(root.join("src/a.rs")).unwrap().contains("$1추정"));
        crate::state::restore(&orig).unwrap();
        assert!(std::fs::read_to_string(root.join("src/a.rs")).unwrap().contains("Estimate"));
        assert!(replace_text(root, &["../x".into()], "a", "b", false).is_err(), "프로젝트 밖은 거부");

        // 파일 작업
        create_file(&root.join("src/new.rs")).unwrap();
        assert!(create_file(&root.join("src/new.rs")).is_err(), "이미 있으면 실패");
        assert!(create_file(&root.join("src/bad:name.rs")).is_err());
        create_dir(&root.join("docs/guide")).unwrap();
        rename(&root.join("src/new.rs"), &root.join("src/renamed.rs")).unwrap();
        assert!(root.join("src/renamed.rs").exists() && !root.join("src/new.rs").exists());
        assert!(rename(&root.join("src/renamed.rs"), &root.join("src/a.rs")).is_err(), "덮어쓰지 않음");
    }
}
