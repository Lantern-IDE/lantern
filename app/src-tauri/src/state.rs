//! 앱 전역 상태, 프로젝트 경로 검사, 월별 사용량 기록.

use crate::llm::Message;
use anyhow::{bail, Context, Result};
use lantern_context::Engine;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

pub struct Project {
    pub root: PathBuf,
    /// rusqlite 연결은 Sync가 아니라 Mutex로 감싼다. 블로킹 작업에서만 잠근다.
    pub engine: Arc<Mutex<Engine>>,
    /// 사용자가 이 폴더를 신뢰했는지. 신뢰하지 않으면 제한 모드:
    /// 프로젝트 설정·에이전트를 읽지 않고(모델 주소를 바꿔 키를 빼돌리는 공격 방지),
    /// 파일 수정·명령 실행·훅을 막는다.
    pub trusted: AtomicBool,
}

impl Project {
    /// 프로젝트 설정(.lantern/)을 읽어도 되면 루트, 제한 모드면 None
    pub fn config_root(&self) -> Option<&Path> {
        self.is_trusted().then_some(self.root.as_path())
    }

    pub fn is_trusted(&self) -> bool {
        self.trusted.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[derive(Default)]
pub struct AppState {
    pub project: Mutex<Option<Arc<Project>>>,
    pub sessions: Mutex<HashMap<String, Vec<Message>>>,
    pub cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub approvals: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    pub terminals: Mutex<HashMap<u32, crate::terminal::Term>>,
    pub lsps: Mutex<HashMap<String, crate::lsp::Lsp>>,
    /// 격리된 작업: 세션 → 작업 공간
    pub worktrees: Mutex<HashMap<String, crate::worktree::Worktree>>,
    /// 진행 중인 자동 완성 요청 취소 신호 (새 요청이 오면 이전 것을 끊는다)
    pub completion_cancel: Mutex<Option<Arc<std::sync::atomic::AtomicBool>>>,
    /// AI 작업 되돌리기: 체크포인트 id → 원본 파일들
    pub checkpoints: Mutex<HashMap<String, Originals>>,
    /// 외부 파일 변경 감시 (프로젝트를 바꾸면 교체)
    pub watcher: Mutex<Option<notify::RecommendedWatcher>>,
    pub http: reqwest::Client,
}

impl AppState {
    pub fn project(&self) -> Result<Arc<Project>> {
        self.project.lock().unwrap().clone().context("먼저 프로젝트 폴더를 여세요")
    }
}

/// 사용자·모델이 준 상대 경로를 프로젝트 안의 절대 경로로 바꾼다. 밖으로 나가는 경로는 거부한다.
pub fn resolve_in_root(root: &Path, rel: &str) -> Result<PathBuf> {
    let rel = rel.trim().replace('\\', "/");
    let p = Path::new(&rel);
    let candidate = if p.is_absolute() { p.to_path_buf() } else { root.join(p) };
    let mut out = PathBuf::new();
    for c in candidate.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    if !out.starts_with(root) {
        bail!("프로젝트 밖의 경로는 쓸 수 없습니다: {rel}");
    }
    let inner = out.strip_prefix(root).unwrap_or(&out);
    if inner.components().next().is_some_and(|c| c.as_os_str() == ".git") {
        bail!(".git 폴더는 건드릴 수 없습니다");
    }
    Ok(out)
}

pub fn rel_path(root: &Path, abs: &Path) -> String {
    abs.strip_prefix(root).unwrap_or(abs).to_string_lossy().replace('\\', "/")
}

/// `file:///C:/a/b` 형식. 한글 등은 퍼센트 인코딩.
pub fn file_uri(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !s.starts_with('/') {
        out.push('/');
    }
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ── 사용량 ─────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MonthUsage {
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub requests: u64,
}

/// 앱 데이터 폴더 (사용량, 신뢰 목록, 로그). `LANTERN_DATA_DIR`로 바꿀 수 있다 (테스트 격리용).
pub fn data_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("LANTERN_DATA_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    dirs::data_dir().map(|d| d.join("lantern"))
}

/// 예전 이름(KHALA)으로 남은 앱 데이터·전역 설정·WebView 데이터를 새 자리로 옮긴다 (시작할 때 한 번).
/// 새 자리가 이미 있거나 테스트용 환경변수로 자리를 바꿨으면 건드리지 않는다. 옮긴 항목을 돌려준다.
pub fn migrate_legacy_data() -> Vec<String> {
    let mut moved = Vec::new();
    let mut mv = |old: Option<PathBuf>, new: Option<PathBuf>, what: &str| {
        if let (Some(old), Some(new)) = (old, new) {
            if old.is_dir() && !new.exists() && std::fs::rename(&old, &new).is_ok() {
                moved.push(format!("{what}: {} → {}", old.display(), new.display()));
            }
        }
    };
    if std::env::var_os("LANTERN_DATA_DIR").is_none() {
        mv(dirs::data_dir().map(|d| d.join("khala")), dirs::data_dir().map(|d| d.join("lantern")), "앱 데이터");
    }
    if std::env::var_os("LANTERN_HOME").is_none() {
        mv(dirs::home_dir().map(|d| d.join(".khala")), dirs::home_dir().map(|d| d.join(".lantern")), "전역 설정");
    }
    if std::env::var_os("LANTERN_WEBVIEW_DATA_DIR").is_none() {
        // Tauri는 앱 식별자 이름의 폴더에 WebView 데이터(화면 설정, 최근 폴더 등)를 둔다
        mv(dirs::data_local_dir().map(|d| d.join("dev.khala.ide")), dirs::data_local_dir().map(|d| d.join("dev.lantern.ide")), "화면 데이터");
    }
    moved
}

fn usage_file() -> Option<PathBuf> {
    data_dir().map(|d| d.join("usage.json"))
}

// ── AI 작업 되돌리기 ───────────────────────────────────

/// 되돌리기용 원본: (절대 경로, 바꾸기 전 내용). None이면 새로 만든 파일
pub type Originals = Vec<(PathBuf, Option<String>)>;

/// 체크포인트의 원본으로 되돌린다. 새로 만들어졌던 파일은 휴지통으로 보낸다. 되돌린 파일 수를 돌려준다.
pub fn restore(originals: &[(PathBuf, Option<String>)]) -> Result<usize> {
    let mut n = 0;
    for (path, content) in originals {
        match content {
            Some(text) => {
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                std::fs::write(path, text)?;
            }
            None if path.exists() => trash::delete(path).with_context(|| format!("{} 삭제 실패", path.display()))?,
            None => continue,
        }
        n += 1;
    }
    Ok(n)
}

// ── 폴더 신뢰 ─────────────────────────────────────────

fn trust_file() -> Option<PathBuf> {
    data_dir().map(|d| d.join("trusted.json"))
}

fn trust_key(root: &Path) -> String {
    root.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_lowercase()
}

fn load_trusted() -> Vec<String> {
    trust_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// 이 폴더나 상위 폴더를 신뢰했는지
pub fn is_trusted(root: &Path) -> bool {
    let key = trust_key(root);
    load_trusted().iter().any(|t| key == *t || key.starts_with(&format!("{t}/")))
}

pub fn set_trusted(root: &Path, trusted: bool) -> Result<()> {
    let key = trust_key(root);
    let mut list = load_trusted();
    list.retain(|t| *t != key);
    if trusted {
        list.push(key);
    }
    let p = trust_file().context("앱 데이터 폴더를 찾을 수 없습니다")?;
    std::fs::create_dir_all(p.parent().unwrap())?;
    std::fs::write(p, serde_json::to_string_pretty(&list)?)?;
    Ok(())
}

/// 유닉스 시간 → "YYYY-MM" (UTC)
pub fn month_key(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    // Howard Hinnant의 civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}")
}

pub fn current_month() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    month_key(now)
}

fn load_usage() -> BTreeMap<String, MonthUsage> {
    usage_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn this_month_usage() -> MonthUsage {
    load_usage().remove(&current_month()).unwrap_or_default()
}

pub fn record_usage(cost: f64, input: u64, output: u64) -> MonthUsage {
    let mut all = load_usage();
    let entry = all.entry(current_month()).or_default();
    entry.cost_usd += cost;
    entry.input_tokens += input;
    entry.output_tokens += output;
    entry.requests += 1;
    let snapshot = entry.clone();
    if let Some(p) = usage_file() {
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = std::fs::write(p, serde_json::to_string_pretty(&all).unwrap_or_default());
    }
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn month_keys() {
        assert_eq!(month_key(0), "1970-01");
        assert_eq!(month_key(1_790_208_000), "2026-09"); // 2026-09-24
        assert_eq!(month_key(951_782_400), "2000-02"); // 2000-02-29
    }

    #[test]
    fn rejects_escaping_paths() {
        let root = Path::new(if cfg!(windows) { r"C:\proj" } else { "/proj" });
        assert!(resolve_in_root(root, "src/a.rs").is_ok());
        assert!(resolve_in_root(root, "src/../b.rs").unwrap().ends_with("b.rs"));
        assert!(resolve_in_root(root, "../etc/passwd").is_err());
        assert!(resolve_in_root(root, ".git/config").is_err());
    }

    #[test]
    fn trust_list_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("LANTERN_DATA_DIR", dir.path());
        let proj = Path::new(if cfg!(windows) { r"C:\work\Proj" } else { "/work/Proj" });
        let child = proj.join("sub");
        let other = Path::new(if cfg!(windows) { r"C:\work\ProjX" } else { "/work/ProjX" });
        assert!(!is_trusted(proj));
        set_trusted(proj, true).unwrap();
        assert!(is_trusted(proj));
        assert!(is_trusted(&child), "하위 폴더도 신뢰");
        assert!(!is_trusted(other), "이름이 비슷한 다른 폴더는 아님");
        set_trusted(proj, false).unwrap();
        assert!(!is_trusted(proj));
        std::env::remove_var("LANTERN_DATA_DIR");
    }

    #[test]
    fn uri_encoding() {
        let p = Path::new(r"C:\Users\로지\a b.rs");
        let u = file_uri(p);
        assert!(u.starts_with("file:///C:/Users/"));
        assert!(u.ends_with("/a%20b.rs"));
        assert!(!u.contains('로'));
    }
}
