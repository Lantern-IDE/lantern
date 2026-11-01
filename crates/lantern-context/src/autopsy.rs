//! 지난 에이전트 세션을 되짚어 놓쳤을 수 있는 곳을 찾는다 (`lantern autopsy`).
//!
//! 세션마다 첫 편집 직전 커밋을 임시 클론에 꺼내 그때의 인덱스로 [`check`]의 판단을 다시 돌리고, 그 뒤의 git 이력으로
//! 확인한다. 반영되지 않은 변경과 반영 때 이미 해결된 참조는 빼서 남은 발견이 근거를 갖게 한다.
//!
//! 원본 저장소는 읽기만 한다. 모델과 네트워크를 쓰지 않는다. 임시 폴더는 끝나면 지운다.
//! 메시지, 명령, 코드는 출력하지 않는다. 첫 질문은 메모리 안에서 맥락 조립에만 쓰고 버린다.

use crate::assemble::{self, ContextRequest};
use crate::check::{self, blobs_with, collect_findings, file_changes, same_family, Confidence, FileChanges, Finding, FindingKind, SymbolChange};
use crate::git::{git, no_window, refresh_cochange, NO_LAZY_FETCH};
use crate::graph;
use crate::indexer::{self, is_unindexable, MAX_FILE_BYTES, SKIP_DIRS};
use crate::lang::Lang;
use crate::parse::{Parsed, Parser};
use crate::sessions::{self, path_key, Agent, AgentEvent, DiscoverOpts, FileChange, Homes, Op, Scope, Seen, SessionSummary, Usage};
use crate::store::Store;
use crate::tokenize;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 편집 뒤 이 안의 커밋을 반영 지점으로, 그 뒤 이 안의 변경을 고친 증거로 본다
const WINDOW_SECS: i64 = 14 * 86_400;
/// 편집 사이가 이만큼 벌어지면 다른 작업 구간으로 본다
const PERIOD_GAP_SECS: i64 = 3 * 3600;
/// 기본 보고서에 보여 줄 발견 수
const TOP_FINDINGS: usize = 5;
/// 첫 맥락에서 볼 파일 수
const FIRST_FILES: usize = 3;
const MIN_PROMPT_CHARS: usize = 12;
const MAX_PROMPT_CHARS: usize = 4000;
/// 세션 하나에서 이력으로 확인할 발견의 상한. 믿음이 큰 순서로 자른다
const MAX_VERIFY: usize = 200;
/// 이보다 많은 파일에 나오는 이름은 흔한 단어라 정의를 찾아 확인하지 않는다
const MAX_GREP_FILES: usize = 300;
const MAX_TEST_FILES: usize = 30;
/// 시간 상한의 이 비율을 넘기면 재현을 건너뛴다
const REPLAY_TIME_SHARE: f64 = 0.8;
/// 중간에 끊겨 남은 임시 폴더를 지우는 기준
const STALE_TEMP: Duration = Duration::from_secs(86_400);

/// 가격표를 확인한 날
pub const PRICE_TABLE_DATE: &str = "2026-11-02";
pub const PRICE_NOTE: &str = "List price estimate per 1M tokens, not your bill. Subscriptions and long-prompt tiers differ. Unknown models are not priced.";
pub const REPLAY_NOTE: &str = "Replay of the session's first prompt against Lantern's context at the base commit, not an agent run. Left out of the text report: when measured, it did not beat a random pick of 3 files.";

struct Price {
    model: &'static str,
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

const fn price(model: &'static str, input: f64, output: f64, cache_read: f64, cache_write: f64) -> Price {
    Price { model, input, output, cache_read, cache_write }
}

/// 달러, 100만 토큰당. 캐시 쓰기는 Claude의 5분 값(입력의 1.25배), 따로 받지 않는 곳은 입력 값이다.
/// 여기 없는 모델은 짐작하지 않고 "모르는 모델"로 센다
const PRICES: &[Price] = &[
    price("claude-fable-5-1", 10.0, 50.0, 0.25, 12.5),
    price("claude-mythos-5-1", 10.0, 50.0, 0.25, 12.5),
    price("claude-fable-5", 10.0, 50.0, 1.0, 12.5),
    price("claude-opus-5-5", 4.0, 20.0, 0.2, 5.0),
    price("claude-opus-5", 5.0, 25.0, 0.5, 6.25),
    price("claude-opus-4-8", 5.0, 25.0, 0.5, 6.25),
    price("claude-opus-4-7", 5.0, 25.0, 0.5, 6.25),
    price("claude-opus-4-6", 5.0, 25.0, 0.5, 6.25),
    price("claude-sonnet-5-5", 2.0, 10.0, 0.2, 2.5),
    price("claude-sonnet-5", 2.0, 10.0, 0.2, 2.5),
    price("claude-sonnet-4-6", 3.0, 15.0, 0.3, 3.75),
    price("claude-haiku-5-5", 0.1, 0.5, 0.01, 0.125),
    price("claude-haiku-4-5", 1.0, 5.0, 0.1, 1.25),
    price("gpt-5.5", 5.0, 30.0, 0.5, 5.0),
];

/// 표기 차이만 지운다: `[1m]`, 뒤의 `-YYYYMMDD`
fn normalize_model(name: &str) -> &str {
    let name = name.split('[').next().unwrap_or(name);
    match name.rsplit_once('-') {
        Some((head, tail)) if tail.len() == 8 && tail.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => name,
    }
}

fn price_of(model: &str) -> Option<&'static Price> {
    let model = normalize_model(model);
    PRICES.iter().find(|p| p.model == model)
}

fn cost_of(p: &Price, u: &Usage) -> f64 {
    let m = |n: u64, per: f64| n as f64 * per / 1_000_000.0;
    m(u.input, p.input) + m(u.output, p.output) + m(u.cache_read, p.cache_read) + m(u.cache_write, p.cache_write)
}

fn total_tokens(u: &Usage) -> u64 {
    u.input + u.output + u.cache_read + u.cache_write
}

#[derive(Debug, Clone)]
pub struct AutopsyOpts {
    /// 이 일수 안에 바뀐 세션만. `None`이면 전부
    pub days: Option<u64>,
    /// 조건에 맞는 세션 중 최근 이만큼만. `None`이면 전부
    pub max_sessions: Option<usize>,
    pub agent: Option<Agent>,
    pub deep: bool,
    pub replay: bool,
    pub max_seconds: u64,
}

impl Default for AutopsyOpts {
    fn default() -> Self {
        Self { days: Some(30), max_sessions: Some(40), agent: None, deep: false, replay: false, max_seconds: 120 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportLang {
    En,
    Ko,
}

fn lang_of_locale(locale: &str) -> ReportLang {
    if locale.to_ascii_lowercase().starts_with("ko") {
        ReportLang::Ko
    } else {
        ReportLang::En
    }
}

/// 환경 변수, 없으면 Windows 사용자 설정
pub fn system_lang() -> ReportLang {
    for k in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(v) = std::env::var(k).ok().filter(|v| !v.is_empty()) {
            return lang_of_locale(&v);
        }
    }
    windows_locale().map_or(ReportLang::En, |l| lang_of_locale(&l))
}

#[cfg(windows)]
fn windows_locale() -> Option<String> {
    extern "system" {
        fn GetUserDefaultLocaleName(name: *mut u16, len: i32) -> i32;
    }
    let mut buf = [0u16; 85];
    // SAFETY: 크기를 함께 넘긴 버퍼에 이름만 받는다. 실패하면 0이다
    let n = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    (n > 1).then(|| String::from_utf16_lossy(&buf[..n as usize - 1]))
}

#[cfg(not(windows))]
fn windows_locale() -> Option<String> {
    None
}

// ── 시각 ─────────────────────────────────────────────────

/// 1970-01-01부터 며칠째인가 (그레고리력)
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// RFC 3339 → 유닉스 초 (소수점 아래는 버린다)
pub(crate) fn parse_ts(ts: &str) -> Option<i64> {
    let b = ts.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |from: usize, to: usize| ts.get(from..to).and_then(|s| s.parse::<i64>().ok());
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hour, minute, second) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 24 || minute > 59 || second > 60 {
        return None;
    }
    let mut rest = ts.get(19..)?;
    if let Some(r) = rest.strip_prefix('.') {
        rest = r.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match rest.as_bytes().first() {
        None | Some(b'Z' | b'z') => 0,
        Some(sign @ (b'+' | b'-')) => {
            let digits: String = rest[1..].chars().filter(char::is_ascii_digit).collect();
            let hh = digits.get(0..2)?.parse::<i64>().ok()?;
            let mm = digits.get(2..4).map_or(Some(0), |s| s.parse::<i64>().ok())?;
            let secs = hh * 3600 + mm * 60;
            if *sign == b'-' {
                -secs
            } else {
                secs
            }
        }
        _ => return None,
    };
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

fn date_of(ts: &str) -> String {
    ts.chars().take(10).collect()
}

// ── 임시 클론 ────────────────────────────────────────────

/// 사라질 때 지우는 임시 폴더
struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Result<Self> {
        let base = std::env::temp_dir();
        forget_stale_temps(&base);
        for n in 0..100u32 {
            let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
            let dir = base.join(format!("{TEMP_PREFIX}{}-{nanos}-{n}", std::process::id()));
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok(Self(dir)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e).context("임시 폴더 만들기"),
            }
        }
        bail!("임시 폴더를 만들지 못했습니다")
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        // 바이러스 검사 등이 잠깐 잡고 있을 수 있어 몇 번 더 해 본다
        for attempt in 1..=5u64 {
            if std::fs::remove_dir_all(&self.0).is_ok() || !self.0.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100 * attempt));
        }
    }
}

const TEMP_PREFIX: &str = "lantern-autopsy-";

/// 강제 종료로 남은 사본을 지운다. 저장소 코드가 들어 있어 오래 두지 않는다
fn forget_stale_temps(base: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else { return };
    let now = SystemTime::now();
    for e in entries.flatten() {
        if !e.file_name().to_string_lossy().starts_with(TEMP_PREFIX) {
            continue;
        }
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| now.duration_since(t).ok()).is_some_and(|age| age > STALE_TEMP);
        if old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// 임시 클론. 원본의 객체를 빌려 쓰므로 복사가 없고, 원본에는 아무것도 쓰지 않는다
struct Sandbox {
    /// 클론 폴더
    dir: PathBuf,
    hooks: PathBuf,
    temp: TempRoot,
}

impl Sandbox {
    /// 사용자의 전역 설정이 끼어들지 못하게 하고 네트워크를 막은 git 명령
    fn git_cmd(&self, cwd: &Path) -> Command {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(cwd);
        let hooks = format!("core.hooksPath={}", self.hooks.to_string_lossy().replace('\\', "/"));
        cmd.args(["-c", &hooks]);
        for c in [
            "core.autocrlf=false",
            "core.safecrlf=false",
            "core.fsmonitor=false",
            "core.longpaths=true",
            // Git for Windows는 시스템 설정으로 켜 둔다. 아래에서 시스템 설정을 끄니 빠른 순회를 여기서 켠다
            "core.fscache=true",
            // 사용자 설정을 끄면 그 안의 safe.directory도 사라진다. 이 저장소는 이미 사용자의 설정으로 열어 본 것이다
            "safe.directory=*",
            // 저장소의 심볼릭 링크가 임시 폴더 밖을 가리켜도 따라가지 않게 글 파일로 꺼낸다
            "core.symlinks=false",
            "core.quotepath=off",
            "submodule.recurse=false",
            "advice.detachedHead=false",
            "gc.auto=0",
            "maintenance.auto=false",
            "protocol.allow=never",
            "protocol.file.allow=always",
        ] {
            cmd.args(["-c", c]);
        }
        for k in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES", "GIT_CONFIG_PARAMETERS", "GIT_CONFIG_COUNT"] {
            cmd.env_remove(k);
        }
        // 전역 설정의 `filter.*.smudge`는 저장소에 커밋된 `.gitattributes`만으로 꺼내기 때 돌아간다
        cmd.env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null");
        cmd.env("GIT_LFS_SKIP_SMUDGE", "1").env("GIT_TERMINAL_PROMPT", "0").env(NO_LAZY_FETCH, "1").env("GIT_OPTIONAL_LOCKS", "0");
        no_window(&mut cmd);
        cmd
    }

    fn output(&self, cwd: &Path, args: &[&str]) -> Result<Output> {
        Ok(self.git_cmd(cwd).args(args).output()?)
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Result<String> {
        let out = self.output(cwd, args)?;
        if !out.status.success() {
            bail!("git {}: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn create(root: &Path) -> Result<Self> {
        let temp = TempRoot::new()?;
        let dir = temp.0.join("repo");
        let hooks = temp.0.join("nohooks");
        std::fs::create_dir_all(&hooks)?;
        // 작업 트리면 객체는 공통 폴더에 있다
        let common = git(root, &["rev-parse", "--git-common-dir"])?;
        let common = root.join(common.trim());
        let sandbox = Self { dir, hooks, temp };
        let source = common.to_string_lossy().into_owned();
        sandbox.run(&sandbox.temp.0, &["clone", "--quiet", "--shared", "--no-checkout", "--no-tags", "--template=", "--", &source, "repo"]).context("임시 클론")?;
        // 커밋하지 않은 인덱싱 제외 목록은 그대로 따른다. 코드가 아니라 설정이다
        if let Ok(ignore) = std::fs::read(root.join(crate::PROJECT_DIR).join("ignore")) {
            let dir = sandbox.dir.join(crate::PROJECT_DIR);
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join("ignore"), ignore)?;
        }
        Ok(sandbox)
    }

    fn checkout(&self, commit: &str) -> Result<()> {
        self.run(&self.dir, &["checkout", "--quiet", "--force", "--detach", commit]).map(|_| ())
    }

    fn blobs(&self, commit: &str, paths: &[String]) -> Result<HashMap<String, Vec<u8>>> {
        blobs_with(self.git_cmd(&self.dir), commit, paths)
    }

    /// 이 이름이 낱말로 들어 있는 파일. 인덱스에 들지 않는 폴더는 뺀다
    fn grep_files(&self, root: &Path, state: &State, name: &str) -> Result<Vec<String>> {
        const FLAGS: [&str; 7] = ["grep", "-I", "-l", "-z", "-w", "-F", "--full-name"];
        let out = match state {
            State::Commit { hash, .. } => self.output(&self.dir, &[&FLAGS[..], &["-e", name, hash.as_str()]].concat())?,
            // 인덱스 파일을 읽기만 한다. 추적하지 않는 새 파일도 본다
            State::WorkTree => self.output(root, &[&FLAGS[..], &["--untracked", "-e", name]].concat())?,
        };
        // 1은 찾지 못했다는 뜻이다
        if !out.status.success() && out.status.code() != Some(1) {
            bail!("git grep: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let prefix = match state {
            State::Commit { hash, .. } => format!("{hash}:"),
            State::WorkTree => String::new(),
        };
        Ok(text.split('\0').filter_map(|e| e.strip_prefix(prefix.as_str())).filter(|p| !p.is_empty() && !outside_index(p)).map(str::to_string).collect())
    }
}

/// 숨은 폴더와 빌드 결과처럼 인덱스가 보지 않는 곳인가
fn outside_index(rel: &str) -> bool {
    rel.split('/').any(|seg| seg.starts_with('.') || SKIP_DIRS.contains(&seg))
}

fn key(path: &str) -> String {
    path_key(Path::new(path))
}

fn too_big(b: &[u8]) -> bool {
    b.len() as u64 > MAX_FILE_BYTES || is_unindexable(b)
}

// ── 이력 ─────────────────────────────────────────────────

/// 어느 시점에서 확인하는가
#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    /// 편집 뒤 이 파일을 처음 건드린 커밋. `pos`는 [`History::commits`]의 자리
    Commit { hash: String, pos: usize },
    /// 그런 커밋이 없으면 지금의 작업 트리
    WorkTree,
}

impl State {
    fn label(&self) -> &str {
        match self {
            State::Commit { hash, .. } => hash,
            State::WorkTree => "worktree",
        }
    }
}

struct Commit {
    hash: String,
    ct: i64,
    /// `path_key` 기준
    files: HashSet<String>,
}

#[derive(Default)]
struct History {
    /// (커밋 시각, 해시). 오래된 순
    timeline: Vec<(i64, String)>,
    /// 가장 이른 세션 이후 커밋. 오래된 순
    commits: Vec<Commit>,
}

impl History {
    fn load(sandbox: &Sandbox, head: &str, since: Option<i64>) -> Result<Self> {
        let list = sandbox.run(&sandbox.dir, &["rev-list", "--timestamp", head])?;
        Ok(Self::parse(&list, &Self::log(sandbox, head, since)?))
    }

    fn log(sandbox: &Sandbox, head: &str, since: Option<i64>) -> Result<String> {
        let since = since.map(|t| format!("--since={t}"));
        let mut args = vec!["log", "--no-renames", "--name-only", "--format=%x00%H %ct"];
        if let Some(s) = since.as_deref() {
            args.push(s);
        }
        args.push(head);
        sandbox.run(&sandbox.dir, &args)
    }

    /// `git rev-list --timestamp`와 `git log --name-only --format=%x00%H %ct`의 출력
    fn parse(list: &str, log: &str) -> Self {
        let mut timeline: Vec<(i64, String)> = list
            .lines()
            .filter_map(|l| {
                let (ct, hash) = l.trim().split_once(' ')?;
                Some((ct.parse().ok()?, hash.to_string()))
            })
            .collect();
        // git은 최신이 먼저다. 뒤집어야 시각이 같을 때 자식이 뒤에 온다
        timeline.reverse();
        timeline.sort_by_key(|c| c.0);
        let mut commits: Vec<Commit> = log
            .split('\0')
            .filter_map(|chunk| {
                let mut lines = chunk.lines();
                let (hash, ct) = lines.next()?.trim().split_once(' ')?;
                let files = lines.map(str::trim).filter(|l| !l.is_empty()).map(key).collect();
                Some(Commit { hash: hash.to_string(), ct: ct.parse().ok()?, files })
            })
            .collect();
        commits.reverse();
        commits.sort_by_key(|c| c.ct);
        Self { timeline, commits }
    }

    /// 이 시각 이전의 마지막 커밋 (`rev-list -1 --before`)
    fn before(&self, ts: i64) -> Option<&str> {
        let i = self.timeline.partition_point(|c| c.0 <= ts);
        i.checked_sub(1).map(|i| self.timeline[i].1.as_str())
    }

    /// 파일의 마지막 편집 뒤 그 파일을 처음 건드린 커밋. 기한 안에 없으면 작업 트리
    fn landing(&self, file_key: &str, last_edit: i64) -> State {
        let hit = self.commits.iter().position(|c| c.ct >= last_edit && c.ct <= last_edit + WINDOW_SECS && c.files.contains(file_key));
        match hit {
            Some(pos) => State::Commit { hash: self.commits[pos].hash.clone(), pos },
            None => State::WorkTree,
        }
    }

    /// 반영 지점 뒤 기한 안에 이 파일을 건드린 마지막 커밋
    fn later(&self, file_key: &str, pos: usize) -> Option<State> {
        let limit = self.commits[pos].ct + WINDOW_SECS;
        (pos + 1..self.commits.len())
            .rev()
            .find(|&i| self.commits[i].ct <= limit && self.commits[i].files.contains(file_key))
            .map(|i| State::Commit { hash: self.commits[i].hash.clone(), pos: i })
    }

    /// 이 시각 뒤에 이 파일을 건드린 커밋이 있는가. 있으면 작업 트리는 그 커밋까지 담고 있다
    fn touched_after(&self, file_key: &str, t: i64) -> bool {
        self.commits.iter().any(|c| c.ct >= t && c.files.contains(file_key))
    }
}

// ── 세션 하나 ────────────────────────────────────────────

/// 범위 안에서 편집한 코드 파일 하나
struct Edited<'a> {
    key: String,
    fc: FileChange<'a>,
}

/// 인덱스가 보는 코드 파일의 편집만. 같은 파일을 작업 트리 여럿에서 고쳤으면 나중 것을 쓴다
fn edited_files<'a>(s: &'a SessionSummary, scope: &Scope) -> Vec<Edited<'a>> {
    let mut out: Vec<Edited<'a>> = Vec::new();
    for fc in s.file_changes() {
        let Some(rel) = scope.relative(fc.path) else { continue };
        if rel.is_empty() || outside_index(&rel) || Lang::from_path(Path::new(&rel)).is_none() {
            continue;
        }
        let file_key = key(&rel);
        match out.iter_mut().find(|e| e.key == file_key) {
            Some(prev) if fc.last_ts >= prev.fc.last_ts => prev.fc = fc,
            Some(_) => {}
            None => out.push(Edited { key: file_key, fc }),
        }
    }
    out
}

fn ts_of(e: &AgentEvent) -> Option<i64> {
    e.ts.as_deref().and_then(parse_ts)
}

/// 작업 구간 안에서 고친 코드 파일
struct PeriodFile {
    rel: String,
    key: String,
    lang: Lang,
    first: i64,
    last: i64,
}

/// 한 번에 이어서 고친 묶음. 한 세션이 몇 주씩 이어지기도 해서 처음과 끝만 비교하면 그 사이의 일이 보이지 않는다.
/// 그래서 세션을 쉬는 시간으로 나눠 구간마다 기준 커밋과 손댄 파일을 따로 본다
struct Period {
    first_edit: i64,
    last_edit: i64,
    /// 앞 구간의 마지막 편집 (없으면 맨 앞). 이 뒤의 읽기만 이 구간의 것이다
    from: i64,
    /// 다음 구간의 첫 편집 (없으면 맨 뒤). 이 앞의 읽기와 테스트까지 이 구간의 것이다
    until: i64,
    files: Vec<PeriodFile>,
}

impl Period {
    /// 이 구간에서 고치거나 읽은 파일 (루트 기준)
    fn paths(&self, s: &SessionSummary, scope: &Scope, op: Op) -> Vec<String> {
        s.events
            .iter()
            .filter(|e| e.op == op && e.ok != Some(false))
            .filter(|e| ts_of(e).is_some_and(|t| t > self.from && t < self.until))
            .filter_map(|e| scope.relative(e.path.as_deref()?))
            .filter(|r| !r.is_empty())
            .collect()
    }

    /// 마지막 편집 뒤, 다음 구간 전에 테스트 명령이 돌았는가
    fn tested(&self, s: &SessionSummary) -> bool {
        s.events.iter().any(|e| e.test_run && ts_of(e).is_some_and(|t| t >= self.last_edit && t < self.until))
    }
}

/// 시각이 있는 코드 편집을 쉬는 시간으로 나눈다
fn periods_of(s: &SessionSummary, scope: &Scope) -> Vec<Period> {
    let mut edits: Vec<(i64, String, Lang)> = s
        .events
        .iter()
        .filter(|e| e.op == Op::Edit && e.ok != Some(false))
        .filter_map(|e| {
            let rel = scope.relative(e.path.as_deref()?).filter(|r| !r.is_empty() && !outside_index(r))?;
            let lang = Lang::from_path(Path::new(&rel))?;
            Some((ts_of(e)?, rel, lang))
        })
        .collect();
    edits.sort_by_key(|e| e.0);
    let mut out: Vec<Period> = Vec::new();
    for (ts, rel, lang) in edits {
        let from = match out.last() {
            Some(p) if ts - p.last_edit < PERIOD_GAP_SECS => None,
            Some(p) => Some(p.last_edit),
            None => Some(i64::MIN),
        };
        if let Some(from) = from {
            if let Some(p) = out.last_mut() {
                p.until = ts;
            }
            out.push(Period { first_edit: ts, last_edit: ts, from, until: i64::MAX, files: Vec::new() });
        }
        let Some(p) = out.last_mut() else { continue };
        p.last_edit = ts;
        let file_key = key(&rel);
        match p.files.iter_mut().find(|f| f.key == file_key) {
            Some(f) => f.last = ts,
            None => p.files.push(PeriodFile { rel, key: file_key, lang, first: ts, last: ts }),
        }
    }
    out
}

/// 한 세션을 나눠 볼 때 함께 쓰는 것들
struct SessionView<'a> {
    s: &'a SessionSummary,
    /// 로그에 남은 파일별 처음과 끝
    edited: HashMap<String, Edited<'a>>,
    periods: Vec<Period>,
    /// 파일별 고친 구간 수와 마지막 구간
    file_periods: HashMap<String, (u32, usize)>,
}

impl<'a> SessionView<'a> {
    fn new(s: &'a SessionSummary, scope: &Scope) -> Self {
        let periods = periods_of(s, scope);
        let mut file_periods: HashMap<String, (u32, usize)> = HashMap::new();
        for (i, p) in periods.iter().enumerate() {
            for f in &p.files {
                let e = file_periods.entry(f.key.clone()).or_default();
                e.0 += 1;
                e.1 = i;
            }
        }
        let edited = edited_files(s, scope).into_iter().map(|e| (e.key.clone(), e)).collect();
        Self { s, edited, periods, file_periods }
    }

    /// 로그의 처음과 끝을 그대로 쓸 수 있는 파일. 로그는 파일마다 첫 본문과 마지막 본문만 남기므로 한 구간에서만 고친 파일에 맞다
    fn log_text(&self, file_key: &str) -> Option<&FileChange<'a>> {
        (self.file_periods.get(file_key).map(|p| p.0) == Some(1)).then(|| self.edited.get(file_key).map(|e| &e.fc)).flatten()
    }
}

/// 작업 요청으로 볼 만한 첫 질문인가. 재현은 질문이 말이 될 때만 뜻이 있다
fn is_task_prompt(prompt: &str) -> bool {
    let t = prompt.trim();
    if t.chars().count() < MIN_PROMPT_CHARS || t.starts_with(['/', '<', '[', '!']) {
        return false;
    }
    let low = t.to_lowercase();
    const NOT_TASKS: &[&str] = &["continue", "resume", "go on", "keep going", "this session is being continued", "caveat:", "이어서", "계속"];
    if NOT_TASKS.iter().any(|p| low.starts_with(p)) {
        return false;
    }
    tokenize::query_terms(t).len() >= 2
}

fn change_name(c: &SymbolChange) -> &str {
    match c {
        SymbolChange::Removed { name, .. } | SymbolChange::Signature { name, .. } | SymbolChange::Body { name } => name,
    }
}

/// 바뀐 심볼에 이어진 테스트가 있는가. 영향 계산은 IDE 승인 카드와 같은 [`graph::impact`]다
fn has_related_tests(store: &Store, changes: &[FileChanges]) -> Result<bool> {
    for fc in changes.iter().take(MAX_TEST_FILES) {
        let names: HashSet<&str> = fc.changes.iter().map(change_name).collect();
        if names.is_empty() {
            continue;
        }
        let ranges: Vec<(u32, u32)> = store.symbols_in_file(&fc.path)?.iter().filter(|s| names.contains(s.name.as_str())).map(|s| (s.start_line, s.end_line)).collect();
        if !ranges.is_empty() && !graph::impact(store, &fc.path, &ranges)?.tests.is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// 시작과 끝을 알아낸 파일
struct Resolved {
    path: String,
    lang: Lang,
    /// 세션이 끝났을 때 본문
    after: String,
    state: State,
}

struct Verified {
    finding: Finding,
    later_fixed: bool,
    checked_at: String,
}

#[derive(Default)]
struct Verification {
    kept: Vec<Verified>,
    not_landed: u32,
    already_resolved: u32,
    unverified: u32,
}

struct RefsNow {
    line: u32,
    count: u32,
}

/// 첫 맥락 재현의 결과
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReplayResult {
    #[default]
    Off,
    Hit,
    Miss,
    SkippedPrompt,
    SkippedNoFile,
    SkippedTime,
    Failed,
}

enum Outcome {
    NoBase,
    NoText,
    Done(Box<Done>),
}

struct Done {
    row: PeriodRow,
    kept: Vec<Verified>,
}

/// 세션 분석 동안 쓰는 것들. 임시 폴더를 지우기 전에 인덱스가 닫히도록 `store`가 `sandbox`보다 먼저다
struct Ctx<'a> {
    root: &'a Path,
    scope: &'a Scope,
    store: Store,
    sandbox: Sandbox,
    parser: Parser,
    history: History,
    bytes: HashMap<(String, String), Option<Rc<Vec<u8>>>>,
    parsed: HashMap<(String, String), Rc<Parsed>>,
    timing: Timing,
}

impl Ctx<'_> {
    fn cache_key(state: &State, path: &str) -> (String, String) {
        (state.label().to_string(), key(path))
    }

    /// 한 시점의 파일 여럿을 프로세스 하나로 읽어 둔다
    fn preload(&mut self, state: &State, paths: &BTreeSet<String>) {
        let State::Commit { hash, .. } = state else { return };
        let want: Vec<String> = paths.iter().filter(|p| !self.bytes.contains_key(&Self::cache_key(state, p))).cloned().collect();
        let Ok(mut blobs) = self.sandbox.blobs(hash, &want) else { return };
        for p in want {
            let body = blobs.remove(&p).map(Rc::new);
            self.bytes.insert(Self::cache_key(state, &p), body);
        }
    }

    /// 그 시점의 파일 본문. 없으면 `None`
    fn read_at(&mut self, state: &State, path: &str) -> Option<Rc<Vec<u8>>> {
        let k = Self::cache_key(state, path);
        if let Some(hit) = self.bytes.get(&k) {
            return hit.clone();
        }
        let body = match state {
            State::Commit { hash, .. } => self.sandbox.blobs(hash, std::slice::from_ref(&path.to_string())).ok().and_then(|mut b| b.remove(path)),
            State::WorkTree => check::read_capped(&self.root.join(path)).ok().flatten(),
        };
        let body = body.map(Rc::new);
        self.bytes.insert(k, body.clone());
        body
    }

    fn parsed_at(&mut self, state: &State, path: &str) -> Option<Rc<Parsed>> {
        let k = Self::cache_key(state, path);
        if let Some(hit) = self.parsed.get(&k) {
            return Some(hit.clone());
        }
        let lang = Lang::from_path(Path::new(path))?;
        let body = self.read_at(state, path)?;
        if too_big(&body) {
            return None;
        }
        let parsed = Rc::new(self.parser.parse(lang, &body).ok()?);
        self.parsed.insert(k, parsed.clone());
        Some(parsed)
    }

    /// 이 시점의 같은 언어 계열에 이 이름의 정의가 있는가
    fn defined_at(&mut self, state: &State, name: &str, family: &str) -> Result<bool> {
        let files: BTreeSet<String> = self.sandbox.grep_files(self.root, state, name)?.into_iter().filter(|p| same_family(p, family)).collect();
        if files.len() > MAX_GREP_FILES {
            bail!("너무 흔한 이름");
        }
        self.preload(state, &files);
        for p in &files {
            if self.parsed_at(state, p).is_some_and(|x| x.symbols.iter().any(|s| s.name == name)) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn refs_at(&mut self, state: &State, path: &str, name: &str, calls_only: bool) -> Option<RefsNow> {
        let parsed = self.parsed_at(state, path)?;
        let hits = parsed.refs.iter().filter(|r| r.name == name && (!calls_only || matches!(r.kind.as_str(), "call" | "class")));
        let (mut line, mut count) = (u32::MAX, 0);
        for r in hits {
            line = line.min(r.line);
            count += 1;
        }
        (count > 0).then_some(RefsNow { line, count })
    }

    /// 세션의 변경이 그 시점에 들어가 있는가
    fn landed(&mut self, f: &Finding, x: &Resolved) -> bool {
        let Some(at) = self.read_at(&x.state, &x.path) else {
            // 파일이 사라졌으면 지운 이름은 지워졌지만 새 이름이나 새 시그니처는 없다
            return f.kind == FindingKind::Dangling && f.renamed_to.is_none();
        };
        match f.kind {
            FindingKind::Dangling => {
                let Some(parsed) = self.parsed_at(&x.state, &x.path) else { return true };
                let has = |n: &str| parsed.symbols.iter().any(|s| s.name == n);
                !has(&f.symbol) && f.renamed_to.as_deref().is_none_or(has)
            }
            FindingKind::StaleCaller => match file_changes(&mut self.parser, &x.path, x.lang, x.after.as_bytes(), &at) {
                Ok(fc) => !fc.changes.iter().any(|c| matches!(c, SymbolChange::Removed { name, .. } | SymbolChange::Signature { name, .. } if *name == f.symbol)),
                Err(_) => true,
            },
        }
    }

    /// 참조를 감싼 심볼이 두 본문 사이에 바뀌었는가. 공백과 주석만 바뀐 것은 치지 않는다.
    /// 감싼 심볼이 없는 맨 위 코드는 참조 수로 본다
    fn caller_changed(&mut self, f: &Finding, then: &[u8], now: &[u8]) -> bool {
        let Some(lang) = Lang::from_path(Path::new(&f.path)) else { return false };
        match f.caller.as_deref() {
            Some(caller) => {
                let Ok(fc) = file_changes(&mut self.parser, &f.path, lang, then, now) else { return false };
                fc.changes.iter().any(|c| change_name(c) == caller)
            }
            None => {
                let mut count = |src: &[u8]| self.parser.parse(lang, src).map(|p| p.refs.iter().filter(|r| r.name == f.symbol).count());
                match (count(then), count(now)) {
                    (Ok(a), Ok(b)) => a != b,
                    _ => false,
                }
            }
        }
    }

    /// 그 시점에 이 이름을 부르는 줄들. 공백은 지우고 줄 번호는 보지 않아 위아래로 밀린 것은 같게 나온다
    fn ref_lines(&mut self, state: &State, path: &str, name: &str) -> Option<Vec<String>> {
        let body = self.read_at(state, path)?;
        let parsed = self.parsed_at(state, path)?;
        let lines: Vec<&[u8]> = body.split(|&b| b == b'\n').collect();
        let mut out: Vec<String> = parsed
            .refs
            .iter()
            .filter(|r| r.name == name)
            .filter_map(|r| lines.get((r.line as usize).checked_sub(1)?))
            .map(|l| String::from_utf8_lossy(l).split_whitespace().collect())
            .collect();
        out.sort_unstable();
        Some(out)
    }

    /// 반영 뒤 기한 안에 그 참조 줄이 바뀌었는가. 고쳤다는 증거는 참조 줄의 변화뿐이다:
    /// 감싼 함수의 다른 곳이 바뀐 것이나 파일을 지운 것은 이 참조를 고친 것이 아니다
    fn later_fixed(&mut self, f: &Finding, state: &State) -> bool {
        let State::Commit { pos, .. } = state else { return false };
        let Some(later) = self.history.later(&key(&f.path), *pos) else { return false };
        match (self.ref_lines(state, &f.path, &f.symbol), self.ref_lines(&later, &f.path, &f.symbol)) {
            (Some(then), Some(now)) => then != now,
            _ => false,
        }
    }

    /// 이력으로 확인한다. 이것이 발견을 믿을 만하게 만든다
    fn verify(&mut self, raw: Vec<Finding>, files: &HashMap<String, Resolved>) -> Verification {
        let mut out = Verification::default();
        let mut want: BTreeMap<String, (State, BTreeSet<String>)> = BTreeMap::new();
        for f in &raw {
            let Some(x) = files.get(&key(&f.changed_file)) else { continue };
            let entry = want.entry(x.state.label().to_string()).or_insert_with(|| (x.state.clone(), BTreeSet::new()));
            entry.1.insert(x.path.clone());
            entry.1.insert(f.path.clone());
        }
        for (state, paths) in want.values() {
            self.preload(state, paths);
        }
        let mut defined: HashMap<(String, String, String), Option<bool>> = HashMap::new();
        for mut f in raw {
            let Some(x) = files.get(&key(&f.changed_file)) else {
                out.unverified += 1;
                continue;
            };
            if !self.landed(&f, x) {
                out.not_landed += 1;
                continue;
            }
            let stale = f.kind == FindingKind::StaleCaller;
            let Some(now) = self.refs_at(&x.state, &f.path, &f.symbol, stale) else {
                out.already_resolved += 1;
                continue;
            };
            if stale {
                // 클론은 기준 커밋에 있다
                let base = std::fs::read(self.sandbox.dir.join(&f.path)).ok();
                let at = self.read_at(&x.state, &f.path);
                if let (Some(base), Some(at)) = (base, at) {
                    if self.caller_changed(&f, &base, &at) {
                        out.already_resolved += 1;
                        continue;
                    }
                }
            } else {
                let family = Lang::family(x.lang.name()).to_string();
                let k = (x.state.label().to_string(), f.symbol.clone(), family.clone());
                let known = match defined.get(&k) {
                    Some(v) => *v,
                    None => {
                        let v = self.defined_at(&x.state, &f.symbol, &family).ok();
                        defined.insert(k, v);
                        v
                    }
                };
                match known {
                    Some(true) => {
                        out.already_resolved += 1;
                        continue;
                    }
                    Some(false) => {}
                    None => {
                        out.unverified += 1;
                        continue;
                    }
                }
            }
            let later_fixed = self.later_fixed(&f, &x.state);
            f.line = now.line;
            f.refs_in_file = now.count;
            out.kept.push(Verified { finding: f, later_fixed, checked_at: x.state.label().to_string() });
        }
        out
    }

    /// 구간의 파일별 시작과 끝을 정하고 심볼 변화를 뽑는다
    fn resolve_files(&mut self, v: &SessionView<'_>, index: usize, base: &str, row: &mut PeriodRow) -> Result<(HashMap<String, Resolved>, Vec<FileChanges>)> {
        let p = &v.periods[index];
        let canon: HashMap<String, String> = self.store.files()?.into_keys().map(|p| (key(&p), p)).collect();
        let path_of = |f: &PeriodFile| canon.get(&f.key).cloned().unwrap_or_else(|| f.rel.clone());
        // 시작 본문이 로그에 없으면 그 파일의 이 구간 첫 편집 직전 커밋에서 읽는다
        let mut want: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for f in &p.files {
            if v.log_text(&f.key).and_then(|fc| fc.before).is_none() {
                let commit = self.history.before(f.first).unwrap_or(base);
                want.entry(commit.to_string()).or_default().push(path_of(f));
            }
        }
        let mut starts: HashMap<(String, String), Vec<u8>> = HashMap::new();
        for (commit, paths) in &want {
            for (path, body) in self.sandbox.blobs(commit, paths)? {
                starts.insert((commit.clone(), path), body);
            }
        }

        let mut files = HashMap::new();
        let mut changes = Vec::new();
        for f in &p.files {
            let path = path_of(f);
            let state = self.history.landing(&f.key, f.last);
            let log = v.log_text(&f.key);
            let log_start = log.and_then(|fc| fc.before);
            // 그 커밋에 없으면 새 파일이다
            let git_start = log_start.is_none().then(|| {
                let commit = self.history.before(f.first).unwrap_or(base);
                starts.get(&(commit.to_string(), path.clone())).map_or(String::new(), |b| String::from_utf8_lossy(b).into_owned())
            });
            let (before, after) = match log.and_then(|fc| fc.resolve(git_start.as_deref())) {
                Some(pair) => pair,
                None => {
                    // 셸로 고쳤거나, 여러 구간에 걸쳐 고쳤거나, 변경을 적용하지 못했다. 끝은 반영된 본문에서 본다.
                    // 작업 트리는 이 파일을 마지막으로 고친 구간이고 그 뒤 커밋이 없을 때만 이 구간의 끝이다
                    let last_here = v.file_periods.get(&f.key).is_some_and(|x| x.1 == index);
                    if state == State::WorkTree && !(last_here && !self.history.touched_after(&f.key, f.last)) {
                        row.files_skipped += 1;
                        continue;
                    }
                    let start = log_start.map(str::to_string).or(git_start).unwrap_or_default();
                    let end = self.read_at(&state, &path).map_or(String::new(), |b| String::from_utf8_lossy(&b).into_owned());
                    row.files_from_git += 1;
                    (start, end)
                }
            };
            if before == after {
                continue;
            }
            if too_big(before.as_bytes()) || too_big(after.as_bytes()) {
                row.files_skipped += 1;
                continue;
            }
            let Ok(fc) = file_changes(&mut self.parser, &path, f.lang, before.as_bytes(), after.as_bytes()) else {
                row.files_skipped += 1;
                continue;
            };
            files.insert(f.key.clone(), Resolved { path, lang: f.lang, after, state });
            changes.push(fc);
        }
        Ok((files, changes))
    }

    fn replay(&self, prompt: &str, edited: &HashSet<String>) -> Result<bool> {
        let query: String = prompt.chars().take(MAX_PROMPT_CHARS).collect();
        let result = assemble::assemble(&self.store, &self.sandbox.dir, &ContextRequest::new(query))?;
        let mut first: Vec<&str> = Vec::new();
        for item in &result.items {
            if !first.contains(&item.path.as_str()) {
                first.push(&item.path);
                if first.len() == FIRST_FILES {
                    break;
                }
            }
        }
        Ok(first.iter().any(|p| edited.contains(&key(p))))
    }

    /// 작업 구간 하나. `replay`는 재현을 켠 경우 시간이 남았는지이고, 세션의 첫 구간에서만 준다
    fn analyze_period(&mut self, v: &SessionView<'_>, index: usize, replay: Option<bool>) -> Result<Outcome> {
        self.bytes.clear();
        self.parsed.clear();
        let s = v.s;
        let p = &v.periods[index];
        // 저장소의 첫 커밋보다 먼저 고친 구간은 그때의 코드가 git에 없다. 첫 커밋부터 본다
        let (base, base_after_first_edit) = match (self.history.before(p.first_edit), self.history.timeline.first()) {
            (Some(b), _) => (b.to_string(), false),
            (None, Some((_, oldest))) => (oldest.clone(), true),
            (None, None) => return Ok(Outcome::NoBase),
        };

        let t = Instant::now();
        self.sandbox.checkout(&base)?;
        indexer::index_project(&self.sandbox.dir, &self.store, &mut self.parser)?;
        // 동시 변경은 순위와 테스트 찾기의 보조 재료다. 없어도 분석은 된다
        let _ = refresh_cochange(&self.sandbox.dir, &self.store);
        self.timing.checkout_and_index += t.elapsed().as_millis() as u64;

        let mut row = PeriodRow {
            session: s.id.clone(),
            agent: s.agent.name().to_string(),
            period: index as u32,
            base: base.clone(),
            base_after_first_edit,
            reads_known: s.reads_known,
            ..Default::default()
        };
        let t = Instant::now();
        let (files, changes) = self.resolve_files(v, index, &base, &mut row)?;
        self.timing.resolve_files += t.elapsed().as_millis() as u64;
        if files.is_empty() {
            return Ok(Outcome::NoText);
        }
        row.files_changed = files.len() as u32;

        let t = Instant::now();
        let touched = p.paths(s, self.scope, Op::Edit);
        let read = p.paths(s, self.scope, Op::Read);
        // 인덱스도 클론도 기준 커밋에 있다
        let mut raw = if check::may_leave_behind(&changes) {
            collect_findings(&self.store, &check::read_under(&self.sandbox.dir), &changes, &touched, &read, s.reads_known)?
        } else {
            Vec::new()
        };
        row.findings_raw = raw.len() as u32;
        let cut = raw.len().saturating_sub(MAX_VERIFY);
        raw.truncate(MAX_VERIFY);
        let verified = self.verify(raw, &files);
        row.not_landed = verified.not_landed;
        row.already_resolved = verified.already_resolved;
        row.unverified = verified.unverified + cut as u32;
        row.findings_kept = verified.kept.len() as u32;

        row.related_tests = has_related_tests(&self.store, &changes)?;
        row.test_after_last_edit = p.tested(s);
        self.timing.verify += t.elapsed().as_millis() as u64;

        let t = Instant::now();
        row.replay = match replay {
            None => ReplayResult::Off,
            Some(false) => ReplayResult::SkippedTime,
            Some(true) => {
                // 기준 커밋에 없던 파일은 맥락에 나올 수 없으니 재현의 답이 될 수 없다.
                // 첫 질문은 첫 구간의 일을 말하므로 답도 첫 구간에서 고친 파일이다
                let at_base: HashSet<String> = self.store.files()?.keys().map(|p| key(p)).collect();
                let targets: HashSet<String> = p.files.iter().map(|f| f.key.clone()).filter(|k| at_base.contains(k)).collect();
                match s.first_prompt.as_deref().filter(|q| is_task_prompt(q)) {
                    None => ReplayResult::SkippedPrompt,
                    Some(_) if targets.is_empty() => ReplayResult::SkippedNoFile,
                    Some(prompt) => match self.replay(prompt, &targets) {
                        Ok(true) => ReplayResult::Hit,
                        Ok(false) => ReplayResult::Miss,
                        Err(_) => ReplayResult::Failed,
                    },
                }
            }
        };
        self.timing.replay += t.elapsed().as_millis() as u64;
        Ok(Outcome::Done(Box::new(Done { row, kept: verified.kept })))
    }
}

// ── 보고서 ───────────────────────────────────────────────

#[derive(Debug, Default, Serialize)]
pub struct OptionsEcho {
    pub days: Option<u64>,
    pub max_sessions: Option<usize>,
    pub agent: Option<String>,
    pub deep: bool,
    pub replay: bool,
    pub max_seconds: u64,
}

#[derive(Debug, Default, Serialize)]
pub struct Skipped {
    /// 최근 세션만 보느라 뺀 세션
    pub over_max_sessions: u32,
    pub time_cap: u32,
    /// 그때의 커밋이 없다
    pub no_base: u32,
    /// 시작과 끝 본문을 알 수 없거나 차이가 없다
    pub no_text: u32,
    pub errors: u32,
}

#[derive(Debug, Default, Serialize)]
pub struct SessionCounts {
    /// 찾은 세션 (다른 저장소만 만진 것 포함)
    pub found: u32,
    /// 이 저장소의 코드를 고친 세션: `k of n`의 n
    pub with_code_edits: u32,
    /// `k of n`의 k. 구간을 하나라도 분석한 세션
    pub analyzed: u32,
    /// 쉬는 시간으로 나눈 작업 구간
    pub periods_total: u32,
    pub periods_analyzed: u32,
    pub period_gap_hours: u32,
    pub skipped: Skipped,
    pub agents: BTreeMap<String, u32>,
    pub models: Vec<String>,
    pub period_from: Option<String>,
    pub period_to: Option<String>,
    pub parsed_lines: u64,
    pub unknown_lines: u64,
    pub oversized_lines: u64,
    pub unreadable_files: u32,
    /// 알아본 줄이 절반 아래인 로그. 형식이 바뀌었을 수 있다
    pub format_warnings: u32,
}

#[derive(Debug, Default, Serialize)]
pub struct MissedCounts {
    pub dangling: u32,
    pub dangling_later_fixed: u32,
    pub stale_caller: u32,
    pub stale_caller_later_fixed: u32,
    /// 이력 확인 전 (세션별로 센 수)
    pub raw: u32,
    pub not_landed: u32,
    pub already_resolved: u32,
    pub unverified: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct AutopsyFinding {
    #[serde(flatten)]
    pub finding: Finding,
    /// 반영 뒤 14일 안에 그 참조나 감싼 심볼이 바뀌었다
    pub later_fixed: bool,
    /// 확인한 시점: 커밋 해시 또는 `worktree`
    pub checked_at: String,
    pub session: String,
    pub agent: String,
    /// 같은 곳을 짚은 세션 수
    pub sessions: u32,
}

#[derive(Debug, Default, Serialize)]
pub struct TestCounts {
    /// 바뀐 심볼에 이어진 테스트가 있는 작업 구간
    pub periods_with_related_tests: u32,
    /// 그중 마지막 편집 뒤 테스트 명령이 없던 구간
    pub tests_not_run: u32,
}

#[derive(Debug, Default, Serialize)]
pub struct SearchCost {
    pub sessions: u32,
    pub median_tool_calls: Option<u64>,
    pub median_tokens: Option<u64>,
    pub median_cache_read: Option<u64>,
}

#[derive(Debug, Default, Serialize)]
pub struct ReplayCounts {
    pub enabled: bool,
    /// 재현한 세션
    pub ran: u32,
    /// 그중 편집한 파일이 첫 맥락의 앞 3개 파일에 있던 세션
    pub edited_file_in_first_3: u32,
    pub skipped_prompt: u32,
    pub skipped_no_file: u32,
    pub skipped_time: u32,
    pub failed: u32,
    pub note: &'static str,
}

#[derive(Debug, Default, Serialize)]
pub struct TokenCounts {
    pub sessions: u32,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub note: &'static str,
}

#[derive(Debug, Default, Serialize)]
pub struct ModelCost {
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// 가격표에 없으면 `null`
    pub usd: Option<f64>,
}

#[derive(Debug, Default, Serialize)]
pub struct CostEstimate {
    pub estimated_usd: f64,
    pub priced_tokens: u64,
    pub unknown_models: u32,
    pub unknown_model_tokens: u64,
    pub table_date: &'static str,
    pub note: &'static str,
    pub by_model: Vec<ModelCost>,
}

#[derive(Debug, Default, Serialize)]
pub struct PeriodRow {
    pub session: String,
    pub agent: String,
    /// 세션 안에서 몇 번째 구간인가 (0부터)
    pub period: u32,
    pub base: String,
    /// 첫 편집 앞에 커밋이 없어 저장소의 첫 커밋을 기준으로 썼다
    pub base_after_first_edit: bool,
    pub reads_known: bool,
    pub files_changed: u32,
    /// 끝 본문을 git에서 본 파일
    pub files_from_git: u32,
    pub files_skipped: u32,
    pub findings_raw: u32,
    pub findings_kept: u32,
    pub not_landed: u32,
    pub already_resolved: u32,
    pub unverified: u32,
    pub related_tests: bool,
    pub test_after_last_edit: bool,
    /// 세션의 첫 구간에서만 돌린다
    pub replay: ReplayResult,
}

#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct Timing {
    pub total: u64,
    pub parse: u64,
    pub clone_and_history: u64,
    pub checkout_and_index: u64,
    pub resolve_files: u64,
    pub verify: u64,
    pub replay: u64,
}

#[derive(Debug, Default, Serialize)]
pub struct AutopsyReport {
    pub version: u32,
    pub head: String,
    pub options: OptionsEcho,
    pub sessions: SessionCounts,
    pub missed: MissedCounts,
    /// 전부 (같은 곳은 하나로)
    pub findings: Vec<AutopsyFinding>,
    pub tests: TestCounts,
    pub search_cost: SearchCost,
    pub replay: ReplayCounts,
    pub tokens: TokenCounts,
    pub cost: CostEstimate,
    pub per_period: Vec<PeriodRow>,
    pub method: &'static str,
    pub not_covered: &'static str,
    pub timing_ms: Timing,
    pub timed_out: bool,
}

fn median(mut v: Vec<u64>) -> Option<u64> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    let mid = v.len() / 2;
    Some(if v.len() % 2 == 1 { v[mid] } else { (v[mid - 1] + v[mid]) / 2 })
}

fn add_usage(sum: &mut Usage, u: &Usage) {
    sum.input += u.input;
    sum.output += u.output;
    sum.cache_read += u.cache_read;
    sum.cache_write += u.cache_write;
}

/// 토큰과 정가 추정. 세션 전체 값이라 한 세션이 다른 저장소를 만졌어도 나누지 않는다
fn tally_cost(by_model: &BTreeMap<String, Usage>) -> CostEstimate {
    let mut out = CostEstimate { table_date: PRICE_TABLE_DATE, note: PRICE_NOTE, ..Default::default() };
    for (model, u) in by_model {
        let tokens = total_tokens(u);
        if tokens == 0 {
            continue;
        }
        let usd = price_of(model).map(|p| cost_of(p, u));
        match usd {
            Some(v) => {
                out.estimated_usd += v;
                out.priced_tokens += tokens;
            }
            None => {
                out.unknown_models += 1;
                out.unknown_model_tokens += tokens;
            }
        }
        out.by_model.push(ModelCost { model: model.clone(), input: u.input, output: u.output, cache_read: u.cache_read, cache_write: u.cache_write, usd });
    }
    out
}

fn compact(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}k", n as f64 / 1e3),
        1_000_000..=999_999_999 => format!("{:.1}M", n as f64 / 1e6),
        _ => format!("{:.1}B", n as f64 / 1e9),
    }
}

fn dollars(v: f64) -> String {
    if v >= 100.0 {
        format!("${v:.0}")
    } else {
        format!("${v:.2}")
    }
}

type SeenSites = HashMap<(bool, String, String), usize>;

/// 여러 세션이 같은 곳을 짚으면 하나로 센다
fn merge_finding(all: &mut Vec<AutopsyFinding>, index: &mut SeenSites, v: Verified, session: &str, agent: &str) {
    let k = (v.finding.kind == FindingKind::Dangling, v.finding.symbol.clone(), key(&v.finding.path));
    if let Some(&i) = index.get(&k) {
        let f = &mut all[i];
        f.sessions += 1;
        f.later_fixed |= v.later_fixed;
        return;
    }
    index.insert(k, all.len());
    all.push(AutopsyFinding { finding: v.finding, later_fixed: v.later_fixed, checked_at: v.checked_at, session: session.to_string(), agent: agent.to_string(), sessions: 1 });
}

/// `lantern autopsy`: `dir`의 저장소를 읽기만 한다
pub fn run(dir: &Path, homes: &Homes, opts: &AutopsyOpts) -> Result<AutopsyReport> {
    let started = Instant::now();
    let cap = Duration::from_secs(opts.max_seconds);
    let root = check::repo_root(dir)?;
    let head = check::resolve_commit(&root, "HEAD")?;
    let scope = Scope::for_repo(&root);

    let mut report = AutopsyReport {
        version: 1,
        head: head.clone(),
        options: OptionsEcho {
            days: opts.days,
            max_sessions: opts.max_sessions,
            agent: opts.agent.map(|a| a.name().to_string()),
            deep: opts.deep,
            replay: opts.replay,
            max_seconds: opts.max_seconds,
        },
        method: check::METHOD,
        not_covered: check::NOT_COVERED,
        ..Default::default()
    };
    report.replay = ReplayCounts { enabled: opts.replay, note: REPLAY_NOTE, ..Default::default() };

    // 1. 찾아서 읽는다. 다른 저장소만 만진 세션이 상한을 먹지 않게 읽은 뒤에 자른다
    let since = opts.days.and_then(|d| SystemTime::now().checked_sub(Duration::from_secs(d.saturating_mul(86_400))));
    let found = sessions::discover(homes, &scope, &DiscoverOpts { deep: opts.deep, since, agent: opts.agent });
    report.sessions.found = found.len() as u32;
    let mut seen = Seen::default();
    let mut eligible: VecDeque<SessionSummary> = VecDeque::new();
    for s in &found {
        let summary = sessions::parse_session(s, &scope, &mut seen);
        let c = &mut report.sessions;
        c.parsed_lines += summary.parsed_lines;
        c.unknown_lines += summary.unknown_lines;
        c.oversized_lines += summary.oversized_lines;
        c.unreadable_files += summary.unreadable_files;
        c.format_warnings += summary.format_warnings.len() as u32;
        if edited_files(&summary, &scope).is_empty() {
            continue;
        }
        eligible.push_back(summary);
        if opts.max_sessions.is_some_and(|m| eligible.len() > m) {
            eligible.pop_front();
            c.skipped.over_max_sessions += 1;
        }
    }
    drop(seen);
    report.timing_ms.parse = started.elapsed().as_millis() as u64;
    report.sessions.with_code_edits = eligible.len() as u32;

    // 토큰은 시간 상한과 상관없이 읽은 세션 전체로 센다
    let mut by_model: BTreeMap<String, Usage> = BTreeMap::new();
    for s in &eligible {
        for (model, u) in &s.usage_by_model {
            add_usage(by_model.entry(model.clone()).or_default(), u);
        }
    }
    let mut sum = Usage::default();
    by_model.values().for_each(|u| add_usage(&mut sum, u));
    report.tokens = TokenCounts {
        sessions: eligible.len() as u32,
        input: sum.input,
        output: sum.output,
        cache_read: sum.cache_read,
        cache_write: sum.cache_write,
        note: "Whole sessions: one session can touch other repositories too.",
    };
    report.cost = tally_cost(&by_model);

    if !eligible.is_empty() {
        analyze_all(&root, &scope, &head, eligible, opts, started, cap, &mut report)?;
    }

    report.findings.sort_by(|a, b| {
        (a.finding.confidence, a.finding.in_test, !a.later_fixed, &a.finding.symbol, &a.finding.path, a.finding.line)
            .cmp(&(b.finding.confidence, b.finding.in_test, !b.later_fixed, &b.finding.symbol, &b.finding.path, b.finding.line))
    });
    let m = &mut report.missed;
    for f in &report.findings {
        match f.finding.kind {
            FindingKind::Dangling => {
                m.dangling += 1;
                m.dangling_later_fixed += u32::from(f.later_fixed);
            }
            FindingKind::StaleCaller => {
                m.stale_caller += 1;
                m.stale_caller_later_fixed += u32::from(f.later_fixed);
            }
        }
    }
    report.timing_ms.total = started.elapsed().as_millis() as u64;
    Ok(report)
}

fn record_period(report: &mut AutopsyReport, seen_sites: &mut SeenSites, done: Done) {
    let Done { row, kept } = done;
    report.sessions.periods_analyzed += 1;
    let m = &mut report.missed;
    m.raw += row.findings_raw;
    m.not_landed += row.not_landed;
    m.already_resolved += row.already_resolved;
    m.unverified += row.unverified;
    for v in kept {
        merge_finding(&mut report.findings, seen_sites, v, &row.session, &row.agent);
    }
    if row.related_tests {
        report.tests.periods_with_related_tests += 1;
        report.tests.tests_not_run += u32::from(!row.test_after_last_edit);
    }
    let r = &mut report.replay;
    match row.replay {
        ReplayResult::Off => {}
        ReplayResult::Hit => {
            r.ran += 1;
            r.edited_file_in_first_3 += 1;
        }
        ReplayResult::Miss => r.ran += 1,
        ReplayResult::SkippedPrompt => r.skipped_prompt += 1,
        ReplayResult::SkippedNoFile => r.skipped_no_file += 1,
        ReplayResult::SkippedTime => r.skipped_time += 1,
        ReplayResult::Failed => r.failed += 1,
    }
    report.per_period.push(row);
}

#[allow(clippy::too_many_arguments)]
fn analyze_all(root: &Path, scope: &Scope, head: &str, eligible: VecDeque<SessionSummary>, opts: &AutopsyOpts, started: Instant, cap: Duration, report: &mut AutopsyReport) -> Result<()> {
    report.sessions.period_gap_hours = (PERIOD_GAP_SECS / 3600) as u32;
    report.sessions.periods_total = eligible.iter().map(|s| periods_of(s, scope).len() as u32).sum();
    // 읽는 데 시간을 다 썼으면 클론을 만들 필요도 없다
    if started.elapsed() >= cap {
        report.sessions.skipped.time_cap = eligible.len() as u32;
        report.timed_out = true;
        return Ok(());
    }
    let t = Instant::now();
    let sandbox = Sandbox::create(root)?;
    let earliest = eligible.iter().filter_map(|s| s.started.as_deref().and_then(parse_ts)).min();
    let history = History::load(&sandbox, head, earliest)?;
    let store = Store::open(&sandbox.temp.0.join("index.db")).context("임시 인덱스 열기")?;
    report.timing_ms.clone_and_history = t.elapsed().as_millis() as u64;
    let mut ctx = Ctx { root, scope, store, sandbox, parser: Parser::new()?, history, bytes: HashMap::new(), parsed: HashMap::new(), timing: Timing::default() };

    let total = eligible.len();
    let mut models: BTreeSet<String> = BTreeSet::new();
    let (mut tool_calls, mut tokens, mut cache_read) = (Vec::new(), Vec::new(), Vec::new());
    let mut seen_sites: SeenSites = HashMap::new();
    let replay_until = cap.as_secs_f64() * REPLAY_TIME_SHARE;
    for (i, s) in eligible.into_iter().enumerate() {
        if started.elapsed() >= cap {
            report.sessions.skipped.time_cap += (total - i) as u32;
            report.timed_out = true;
            break;
        }
        let view = SessionView::new(&s, scope);
        let (mut done, mut no_base, mut errors, mut cut) = (0u32, 0u32, 0u32, false);
        for index in 0..view.periods.len() {
            if started.elapsed() >= cap {
                report.timed_out = true;
                cut = true;
                break;
            }
            let replay = (opts.replay && index == 0).then(|| started.elapsed().as_secs_f64() < replay_until);
            match ctx.analyze_period(&view, index, replay) {
                Ok(Outcome::Done(d)) => {
                    done += 1;
                    record_period(report, &mut seen_sites, *d);
                }
                Ok(Outcome::NoBase) => no_base += 1,
                Ok(Outcome::NoText) => {}
                Err(e) => {
                    eprintln!("lantern autopsy: skipped a work period: {e:#}");
                    errors += 1;
                }
            }
        }
        let c = &mut report.sessions;
        if done == 0 {
            // 어느 구간도 분석하지 못한 까닭 하나만 센다
            if cut {
                c.skipped.time_cap += 1;
            } else if errors > 0 {
                c.skipped.errors += 1;
            } else if no_base > 0 || view.periods.is_empty() {
                c.skipped.no_base += 1;
            } else {
                c.skipped.no_text += 1;
            }
            continue;
        }
        c.analyzed += 1;
        *c.agents.entry(s.agent.name().to_string()).or_default() += 1;
        models.extend(s.models.iter().cloned());
        if let Some(from) = s.started.as_deref().map(date_of) {
            if c.period_from.as_ref().is_none_or(|p| from < *p) {
                c.period_from = Some(from);
            }
        }
        if let Some(to) = s.ended.as_deref().map(date_of) {
            if c.period_to.as_ref().is_none_or(|p| to > *p) {
                c.period_to = Some(to);
            }
        }
        tool_calls.extend(s.tool_calls_before_first_edit.map(u64::from));
        tokens.extend(s.tokens_before_first_edit);
        cache_read.extend(s.cache_read_before_first_edit);
    }
    report.sessions.models = models.into_iter().collect();
    report.search_cost = SearchCost {
        sessions: tool_calls.len() as u32,
        median_tool_calls: median(tool_calls),
        median_tokens: median(tokens),
        median_cache_read: median(cache_read),
    };
    let t = ctx.timing;
    report.timing_ms.checkout_and_index = t.checkout_and_index;
    report.timing_ms.resolve_files = t.resolve_files;
    report.timing_ms.verify = t.verify;
    report.timing_ms.replay = t.replay;
    Ok(())
}

// ── 글 ───────────────────────────────────────────────────

fn finding_line(a: &AutopsyFinding, ko: bool, details: bool) -> String {
    let f = &a.finding;
    let level = match (f.confidence, ko) {
        (Confidence::High, false) => "HIGH",
        (Confidence::Medium, false) => "MED ",
        (Confidence::High, true) => "높음",
        (Confidence::Medium, true) => "보통",
    };
    let mut line = format!("  {level}  {}:{}  {}", f.path, f.line, f.symbol);
    if let Some(new) = &f.renamed_to {
        let _ = write!(line, " -> {new}");
    }
    if a.later_fixed {
        line.push_str(if ko { "  (14일 안에 바뀜)" } else { "  (changed within 14 days)" });
    }
    if details {
        let caller = f.caller.as_deref().unwrap_or("-");
        let session: String = a.session.chars().take(8).collect();
        let at: String = a.checked_at.chars().take(8).collect();
        if ko {
            let _ = write!(line, "  | 감싼 심볼 {caller} | 바뀐 파일 {} | 파일 안 참조 {} | 세션 {session} ({}) | 확인 시점 {at}", f.changed_file, f.refs_in_file, a.agent);
        } else {
            let _ = write!(line, "  | in {caller} | changed {} | {} refs in file | session {session} ({}) | checked at {at}", f.changed_file, f.refs_in_file, a.agent);
        }
    }
    line
}

impl AutopsyReport {
    /// 기본은 한 화면, `details`면 전부
    pub fn to_text(&self, lang: ReportLang, details: bool) -> String {
        let ko = lang == ReportLang::Ko;
        let mut out = String::new();
        let s = &self.sessions;
        out.push_str(if ko {
            "Lantern 부검: 에이전트 세션이 놓쳤을 수 있는 곳 (이름 기반, 틀릴 수 있음)\n"
        } else {
            "Lantern autopsy: what agent sessions may have missed (name based, may be wrong)\n"
        });
        if s.with_code_edits == 0 {
            let days = self.options.days.map_or(String::new(), |d| if ko { format!(" (최근 {d}일)") } else { format!(" in the last {d} days") });
            let _ = writeln!(
                out,
                "{}",
                if ko { format!("이 저장소의 코드를 고친 세션을 찾지 못했습니다{days}. --all 이나 --deep 으로 넓혀 보세요.") } else { format!("No session edited code in this repository{days}. Try --all or --deep.") }
            );
            self.push_warnings(&mut out, ko);
            return out;
        }

        let agents: Vec<String> = s.agents.iter().map(|(a, n)| format!("{a} {n}")).collect();
        let mut head = if ko { format!("세션 {}개 중 {}개 분석", s.with_code_edits, s.analyzed) } else { format!("Analyzed {} of {} sessions", s.analyzed, s.with_code_edits) };
        // 세션과 작업 구간이 같으면 굳이 말하지 않는다
        if s.periods_total != s.with_code_edits || s.periods_analyzed != s.analyzed {
            let _ = if ko { write!(head, " (작업 구간 {}개 중 {}개)", s.periods_total, s.periods_analyzed) } else { write!(head, " ({} of {} work periods)", s.periods_analyzed, s.periods_total) };
        }
        let mut parts = vec![head];
        if !agents.is_empty() {
            parts.push(agents.join(", "));
        }
        if let (Some(a), Some(b)) = (&s.period_from, &s.period_to) {
            parts.push(if ko { format!("{a} ~ {b}") } else { format!("{a} to {b}") });
        }
        if !s.models.is_empty() {
            let more = s.models.len().saturating_sub(2);
            let shown = s.models.iter().take(2).cloned().collect::<Vec<_>>().join(", ");
            parts.push(if more > 0 { format!("{shown} +{more}") } else { shown });
        }
        let _ = writeln!(out, "{}", parts.join(" | "));
        self.push_warnings(&mut out, ko);
        out.push('\n');

        // 기준을 못 넘긴 종류는 --details에서만 줄을 보여 준다
        let m = &self.missed;
        let shown: Vec<&AutopsyFinding> = self.findings.iter().filter(|f| details || !f.finding.details_only).collect();
        let hidden = self.findings.len() - shown.len();
        let _ = match (ko, details, shown.is_empty()) {
            (true, false, true) => writeln!(out, "놓쳤을 수 있는 호출 위치: 찾지 못함. 이름 기반이라 없다는 증거는 아닙니다."),
            (false, false, true) => writeln!(out, "Missed call sites: none found. Name based, so this is not proof."),
            (true, false, false) => writeln!(out, "놓쳤을 수 있는 호출 위치: 사라진 이름(Dangling) {}곳 (그중 {}곳은 14일 안에 그 줄이 바뀜)", m.dangling, m.dangling_later_fixed),
            (false, false, false) => writeln!(out, "Missed call sites: {} removed names still referenced (Dangling), {} of them changed on that line within 14 days", m.dangling, m.dangling_later_fixed),
            (true, true, _) => writeln!(
                out,
                "놓쳤을 수 있는 호출 위치: 사라진 이름(Dangling) {} (그중 {}곳은 14일 안에 그 줄이 바뀜), 옛 호출(Stale caller) {} (그중 {}곳은 14일 안에 그 줄이 바뀜). 옛 호출은 정밀도 기준을 넘지 못했습니다.",
                m.dangling, m.dangling_later_fixed, m.stale_caller, m.stale_caller_later_fixed
            ),
            (false, true, _) => writeln!(
                out,
                "Missed call sites: Dangling {} ({} changed on that line within 14 days), Stale caller {} ({} changed on that line within 14 days). Stale caller is below its precision bar.",
                m.dangling, m.dangling_later_fixed, m.stale_caller, m.stale_caller_later_fixed
            ),
        };
        let top = if details { shown.len() } else { TOP_FINDINGS.min(shown.len()) };
        for f in &shown[..top] {
            let _ = writeln!(out, "{}", finding_line(f, ko, details));
        }
        if shown.len() > top {
            let more = shown.len() - top;
            let _ = writeln!(out, "{}", if ko { format!("  그 밖에 {more}곳은 --details 로 볼 수 있습니다") } else { format!("  {more} more with --details") });
        }
        if hidden > 0 {
            let _ = writeln!(
                out,
                "{}",
                if ko { format!("  시그니처 경고 {hidden}건은 정밀도 기준을 넘지 못해 숨겼습니다. --details 로 볼 수 있습니다") } else { format!("  {hidden} signature warnings are below the precision bar, hidden. --details shows them") }
            );
        }
        let dropped = m.not_landed + m.already_resolved + m.unverified;
        if dropped > 0 {
            let _ = if ko {
                writeln!(out, "  이력 확인으로 {dropped}건 뺌: 반영되지 않음 {}, 반영 때 이미 해결 {}, 확인 못 함 {}", m.not_landed, m.already_resolved, m.unverified)
            } else {
                writeln!(out, "  History check dropped {dropped}: {} did not land, {} already resolved at landing, {} not verified", m.not_landed, m.already_resolved, m.unverified)
            };
        }
        out.push('\n');

        let t = &self.tests;
        if t.periods_with_related_tests > 0 {
            // 테스트 명령은 모양으로 알아보니 놓칠 수 있다. 그래서 상한이다
            let _ = if ko {
                writeln!(out, "테스트: 관련 테스트가 있는 작업 구간 {}개 중 최대 {}개는 마지막 편집 뒤 테스트 명령이 보이지 않음 (명령 모양으로 판별, 상한)", t.periods_with_related_tests, t.tests_not_run)
            } else {
                writeln!(out, "Tests: at most {} of {} work periods with related tests ran no test command after the last edit (commands matched by pattern, an upper bound)", t.tests_not_run, t.periods_with_related_tests)
            };
        }
        // 도구 호출은 이 저장소만, 토큰은 세션 전체다. 한 세션이 다른 저장소를 먼저 만졌으면 호출 0번에 토큰이 많을 수 있다
        let c = &self.search_cost;
        if let Some(calls) = c.median_tool_calls {
            let tok = c.median_tokens.map_or_else(|| "-".to_string(), compact);
            let read = c.median_cache_read.map_or_else(|| "-".to_string(), compact);
            let _ = if ko {
                writeln!(out, "이 저장소 첫 편집까지 (세션 {}개 중앙값): 여기서 도구 호출 {calls}번. 세션 전체로는 토큰 {tok}, 캐시 읽기 {read}", c.sessions)
            } else {
                writeln!(out, "Before the first edit here (median of {} sessions): {calls} tool calls here. Whole session so far: {tok} tokens, {read} cache read", c.sessions)
            };
        }
        out.push('\n');

        let k = &self.tokens;
        let _ = if ko {
            writeln!(out, "토큰 (세션 {}개 전체, 다른 저장소 포함): 입력 {}, 출력 {}, 캐시 읽기 {}, 캐시 쓰기 {}", k.sessions, compact(k.input), compact(k.output), compact(k.cache_read), compact(k.cache_write))
        } else {
            writeln!(out, "Tokens ({} whole sessions, other repositories included): input {}, output {}, cache read {}, cache write {}", k.sessions, compact(k.input), compact(k.output), compact(k.cache_read), compact(k.cache_write))
        };
        let g = &self.cost;
        let mut cost = if ko {
            format!("그 세션 전체의 추정 비용: 약 {} (정가표 {} 기준 추정, 실제 청구액 아님)", dollars(g.estimated_usd), g.table_date)
        } else {
            format!("Estimated cost of those whole sessions: about {} (list price table {}, an estimate, not your bill)", dollars(g.estimated_usd), g.table_date)
        };
        if g.unknown_models > 0 {
            let _ = if ko {
                write!(cost, ". 모르는 모델 {}개 ({} 토큰)는 뺌", g.unknown_models, compact(g.unknown_model_tokens))
            } else {
                write!(cost, ". Unknown model: {} ({} tokens, not priced)", g.unknown_models, compact(g.unknown_model_tokens))
            };
        }
        let _ = writeln!(out, "{cost}\n");

        if details {
            self.push_details(&mut out, ko);
        }
        if ko {
            out.push_str("방법: 편집 전후 심볼을 비교해 옛 이름을 코드 그래프에서 찾고 git 이력으로 확인합니다. 모델과 네트워크를 쓰지 않습니다.\n");
            out.push_str("못 보는 것: 셸로 읽은 파일, 동적 디스패치, 의존성 주입, 이름 충돌, 정의 파일이나 타입 이름을 부르지 않는 호출.\n");
            out.push_str("더 보기: --details, --json. 앞으로 개수를 모으려면: lantern hook install claude (에이전트를 멈추지는 않습니다)\n");
        } else {
            out.push_str("Method: symbols before and after each edit, old names looked up in the code graph, checked against git history. No model, no network.\n");
            out.push_str("Not covered: shell reads, dynamic dispatch, dependency injection, name collisions, callers that never name the changed file or type.\n");
            out.push_str("More: --details, --json. To collect counts from now on: lantern hook install claude (it does not stop the agent)\n");
        }
        out
    }

    /// 알아 둘 것들. 한 화면에 들어가게 한 줄로 잇는다
    fn push_warnings(&self, out: &mut String, ko: bool) {
        let s = &self.sessions;
        let mut notes = Vec::new();
        if self.timed_out {
            notes.push(if ko {
                format!("시간 상한 {}초에 멈췄습니다 (--max-seconds).", self.options.max_seconds)
            } else {
                format!("Stopped at the {} s time cap (--max-seconds).", self.options.max_seconds)
            });
        }
        if s.format_warnings > 0 {
            notes.push(if ko {
                format!("로그 {}개는 형식이 바뀌었을 수 있어 숫자가 적을 수 있습니다.", s.format_warnings)
            } else {
                format!("{} log files: format may have changed, counts can be low.", s.format_warnings)
            });
        }
        if !notes.is_empty() {
            let _ = writeln!(out, "{}", notes.join(" "));
        }
    }

    fn push_details(&self, out: &mut String, ko: bool) {
        let x = &self.sessions.skipped;
        let _ = if ko {
            writeln!(out, "건너뛴 세션: 상한 초과 {}, 시간 상한 {}, 그때 커밋 없음 {}, 본문 없음 {}, 오류 {}", x.over_max_sessions, x.time_cap, x.no_base, x.no_text, x.errors)
        } else {
            writeln!(out, "Skipped sessions: over max {}, time cap {}, no base commit {}, no text {}, errors {}", x.over_max_sessions, x.time_cap, x.no_base, x.no_text, x.errors)
        };
        for m in &self.cost.by_model {
            let usd = m.usd.map_or_else(|| if ko { "모르는 모델".to_string() } else { "unknown model".to_string() }, dollars);
            let _ = writeln!(out, "  {}: {} / {} / {} / {}  {usd}", m.model, compact(m.input), compact(m.output), compact(m.cache_read), compact(m.cache_write));
        }
        for row in &self.per_period {
            let id: String = row.session.chars().take(8).collect();
            let base: String = row.base.chars().take(8).collect();
            let _ = writeln!(
                out,
                "  {id}#{} {} base {base}: files {} (git {}, skipped {}), findings {}/{}, not landed {}, resolved {}",
                row.period, row.agent, row.files_changed, row.files_from_git, row.files_skipped, row.findings_kept, row.findings_raw, row.not_landed, row.already_resolved
            );
        }
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_parse_with_fractions_and_offsets() {
        assert_eq!(parse_ts("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_ts("1970-01-02T00:00:01.999Z"), Some(86_401));
        assert_eq!(parse_ts("2026-01-10T10:00:00Z"), Some(1_768_039_200));
        assert_eq!(parse_ts("2026-01-10T19:00:00+09:00"), Some(1_768_039_200));
        assert_eq!(parse_ts("2026-01-10T05:00:00-05:00"), Some(1_768_039_200));
        assert_eq!(parse_ts("2024-02-29T00:00:00Z"), Some(1_709_164_800));
        for bad in ["", "not a date", "2026-13-01T00:00:00Z", "2026-01-10", "2026-01-10T10:00:00 nonsense"] {
            assert_eq!(parse_ts(bad), None, "{bad}");
        }
    }

    fn history() -> History {
        let list = "300 ccc\n200 bbb\n100 aaa\n";
        let log = "\u{0}ccc 300\n\nsrc/A.rs\n\u{0}bbb 200\n\nsrc/a.rs\nsrc/b.rs\n\u{0}aaa 100\n\nsrc/a.rs\n";
        History::parse(list, log)
    }

    #[test]
    fn history_finds_base_landing_and_later_commit() {
        let h = history();
        assert_eq!(h.before(99), None);
        assert_eq!(h.before(100), Some("aaa"));
        assert_eq!(h.before(250), Some("bbb"));
        assert_eq!(h.before(1000), Some("ccc"));

        let k = key("src/b.rs");
        assert_eq!(h.landing(&k, 150), State::Commit { hash: "bbb".into(), pos: 1 });
        // 편집 뒤에 그 파일을 건드린 커밋이 없다
        assert_eq!(h.landing(&k, 201), State::WorkTree);
        // 기한이 지난 커밋은 반영 지점으로 치지 않는다
        assert_eq!(h.landing(&k, 200 - WINDOW_SECS - 1), State::WorkTree);

        assert_eq!(h.later(&k, 1), None);
        if cfg!(windows) {
            assert_eq!(h.later(&key("src/a.rs"), 1), Some(State::Commit { hash: "ccc".into(), pos: 2 }));
        }
    }

    #[test]
    fn later_commit_must_be_inside_the_window() {
        let list = format!("{} late\n100 first\n", 100 + WINDOW_SECS + 1);
        let log = format!("\u{0}late {}\n\nsrc/a.rs\n\u{0}first 100\n\nsrc/a.rs\n", 100 + WINDOW_SECS + 1);
        let h = History::parse(&list, &log);
        assert_eq!(h.later(&key("src/a.rs"), 0), None);
    }

    #[test]
    fn same_second_commits_keep_the_child_last() {
        let h = History::parse("100 child\n100 parent\n", "");
        assert_eq!(h.before(100), Some("child"));
    }

    #[test]
    fn prompts_that_are_not_tasks_are_skipped() {
        for p in ["hi", "/compact", "<command-name>x</command-name>", "continue where you left off", "이어서 계속 진행해 주세요 부탁합니다", "[Request interrupted by user]", "!!!!!!!!!!!!!!"] {
            assert!(!is_task_prompt(p), "{p}");
        }
        for p in ["fix the login retry in the session store", "로그인 재시도 처리를 세션 저장소에서 고쳐 줘"] {
            assert!(is_task_prompt(p), "{p}");
        }
    }

    #[test]
    fn prices_are_exact_and_unknown_models_are_not_guessed() {
        assert_eq!(normalize_model("claude-opus-5-5[1m]"), "claude-opus-5-5");
        assert_eq!(normalize_model("claude-opus-4-8-20260101"), "claude-opus-4-8");
        assert_eq!(normalize_model("gpt-5.5"), "gpt-5.5");
        assert!(price_of("claude-opus-5-5").is_some());
        for unknown in ["claude-opus-9", "gpt-6-astra", "unknown", "opus"] {
            assert!(price_of(unknown).is_none(), "{unknown}");
        }

        let mut m = BTreeMap::new();
        m.insert("claude-opus-5-5".to_string(), Usage { input: 1_000_000, output: 1_000_000, cache_read: 1_000_000, cache_write: 1_000_000 });
        m.insert("mystery".to_string(), Usage { input: 5, output: 5, cache_read: 0, cache_write: 0 });
        m.insert("idle".to_string(), Usage::default());
        let c = tally_cost(&m);
        assert!((c.estimated_usd - (4.0 + 20.0 + 0.2 + 5.0)).abs() < 1e-9, "{}", c.estimated_usd);
        assert_eq!((c.unknown_models, c.unknown_model_tokens, c.priced_tokens), (1, 10, 4_000_000));
        // 쓰지 않은 모델은 표에 올리지 않는다
        assert_eq!(c.by_model.len(), 2);
        assert_eq!(c.table_date, PRICE_TABLE_DATE);
    }

    #[test]
    fn small_numbers_and_medians() {
        assert_eq!(median(vec![]), None);
        assert_eq!(median(vec![3, 1, 2]), Some(2));
        assert_eq!(median(vec![1, 2, 3, 10]), Some(2));
        assert_eq!(compact(999), "999");
        assert_eq!(compact(1_500), "1.5k");
        assert_eq!(compact(2_500_000), "2.5M");
        assert_eq!(compact(3_000_000_000), "3.0B");
        assert_eq!(dollars(1.5), "$1.50");
        assert_eq!(dollars(1234.0), "$1234");
    }

    #[test]
    fn locale_picks_the_report_language() {
        assert_eq!(lang_of_locale("ko-KR"), ReportLang::Ko);
        assert_eq!(lang_of_locale("ko_KR.UTF-8"), ReportLang::Ko);
        assert_eq!(lang_of_locale("en_US.UTF-8"), ReportLang::En);
        assert_eq!(lang_of_locale("C"), ReportLang::En);
    }

    #[test]
    fn index_blind_paths_are_recognized() {
        for p in [".claude/worktrees/x/src/a.rs", "node_modules/a/b.js", "target/debug/x.rs", "web/dist/a.js"] {
            assert!(outside_index(p), "{p}");
        }
        for p in ["src/a.rs", "app/src-tauri/src/x.rs", "builds/a.rs"] {
            assert!(!outside_index(p), "{p}");
        }
    }

    fn finding(i: u32, kind: FindingKind) -> AutopsyFinding {
        AutopsyFinding {
            finding: Finding {
                kind,
                confidence: if kind == FindingKind::Dangling { Confidence::High } else { Confidence::Medium },
                symbol: format!("sym_{i}"),
                renamed_to: None,
                changed_file: "src/lib.rs".into(),
                path: format!("src/user_{i}.rs"),
                line: i + 1,
                caller: None,
                refs_in_file: 1,
                in_test: false,
                details_only: kind.details_only(),
                why: String::new(),
            },
            later_fixed: i == 0,
            checked_at: "worktree".into(),
            session: "abcdef0123".into(),
            agent: "claude".into(),
            sessions: 1,
        }
    }

    /// 모든 줄이 나오는 가장 긴 보고서
    fn busy_report() -> AutopsyReport {
        let mut r = AutopsyReport { timed_out: true, ..Default::default() };
        r.options.max_seconds = 120;
        r.sessions.with_code_edits = 40;
        r.sessions.analyzed = 12;
        r.sessions.format_warnings = 1;
        r.sessions.periods_total = 88;
        r.sessions.periods_analyzed = 30;
        r.sessions.agents.insert("claude".into(), 10);
        r.sessions.agents.insert("codex".into(), 2);
        r.sessions.models = vec!["claude-opus-5-5".into(), "gpt-5.5".into(), "other".into()];
        r.sessions.period_from = Some("2026-10-01".into());
        r.sessions.period_to = Some("2026-10-30".into());
        r.findings = (0..12).map(|i| finding(i, if i < 7 { FindingKind::Dangling } else { FindingKind::StaleCaller })).collect();
        r.missed = MissedCounts { dangling: 7, dangling_later_fixed: 1, stale_caller: 5, stale_caller_later_fixed: 0, raw: 23, not_landed: 3, already_resolved: 7, unverified: 1 };
        r.tests = TestCounts { periods_with_related_tests: 9, tests_not_run: 3 };
        r.search_cost = SearchCost { sessions: 12, median_tool_calls: Some(14), median_tokens: Some(5200), median_cache_read: Some(90_000) };
        r.replay = ReplayCounts { enabled: true, ran: 8, edited_file_in_first_3: 5, note: REPLAY_NOTE, ..Default::default() };
        r.tokens = TokenCounts { sessions: 12, input: 1, output: 2, cache_read: 3, cache_write: 4, note: "" };
        r.cost = CostEstimate { estimated_usd: 12.5, unknown_models: 2, unknown_model_tokens: 2000, table_date: PRICE_TABLE_DATE, ..Default::default() };
        r
    }

    #[test]
    fn default_report_fits_one_screen_in_both_languages() {
        let r = busy_report();
        for lang in [ReportLang::En, ReportLang::Ko] {
            let text = r.to_text(lang, false);
            assert!(text.lines().count() <= 24, "{lang:?} {} lines:\n{text}", text.lines().count());
            assert!(text.contains("src/user_0.rs:1") && text.contains("sym_0"), "{text}");
            assert!(!text.contains("src/user_5.rs"), "상위 5개만 보인다:\n{text}");
            assert!(text.contains("lantern hook install claude"), "{text}");
            assert!(text.contains("$12.50") && text.contains(PRICE_TABLE_DATE), "{text}");
            // 옛 호출은 기준을 못 넘겨 줄이 보이지 않고, 재현 수치는 믿을 수 없어 나오지 않는다
            assert!(!text.contains("src/user_7.rs") && !text.contains("sym_11"), "{text}");
            assert!(!text.contains("first 3") && !text.contains("앞 3개"), "{text}");
        }
        let en = r.to_text(ReportLang::En, false);
        assert!(en.contains("Analyzed 12 of 40 sessions (30 of 88 work periods)") && en.contains("Unknown model: 2"), "{en}");
        assert!(en.contains("time cap") && en.contains("format may have changed"), "{en}");
        assert!(en.contains("5 signature warnings are below the precision bar") && en.contains("7 removed names still referenced") && en.contains("2 more with --details"), "{en}");
        assert!(en.contains("at most 3 of 9 work periods") && en.contains("whole sessions") && en.contains("does not stop the agent"), "{en}");
        let ko = r.to_text(ReportLang::Ko, false);
        assert!(ko.contains("세션 40개 중 12개 분석 (작업 구간 88개 중 30개)") && ko.contains("모르는 모델 2개"), "{ko}");
        assert!(ko.contains("시간 상한") && ko.contains("시그니처 경고 5건") && ko.contains("최대 3개"), "{ko}");

        // 전부 보기에는 옛 호출까지 모두 나온다. 재현 수치는 여기에도 없다
        let all = r.to_text(ReportLang::En, true);
        assert!(all.contains("src/user_11.rs") && all.contains("Skipped sessions") && all.contains("below its precision bar"), "{all}");
        assert!(!all.contains("hidden") && !all.contains("first 3") && !all.to_lowercase().contains("replay"), "{all}");
    }

    #[test]
    fn nothing_found_says_so_and_that_it_is_not_proof() {
        let mut r = busy_report();
        r.findings.clear();
        r.missed = MissedCounts::default();
        let en = r.to_text(ReportLang::En, false);
        assert!(en.contains("Missed call sites: none found") && en.contains("not proof") && !en.contains("hidden"), "{en}");
        assert!(r.to_text(ReportLang::Ko, false).contains("찾지 못함"));
    }

    #[test]
    fn measures_without_data_are_not_printed() {
        let mut r = busy_report();
        r.tests = TestCounts::default();
        r.search_cost = SearchCost::default();
        r.replay = ReplayCounts::default();
        let text = r.to_text(ReportLang::En, false);
        assert!(!text.contains("Tests:") && !text.contains("Before the first edit") && !text.to_lowercase().contains("replay"), "{text}");
    }

    #[test]
    fn report_without_sessions_says_so() {
        let mut r = AutopsyReport::default();
        r.options.days = Some(30);
        let en = r.to_text(ReportLang::En, false);
        assert!(en.contains("No session edited code") && en.contains("30 days"), "{en}");
        assert!(r.to_text(ReportLang::Ko, false).contains("찾지 못했습니다"));
    }
}
