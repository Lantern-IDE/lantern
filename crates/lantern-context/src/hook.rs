//! Claude Code 훅: Stop 때 놓친 곳을 확인하고, 설정 파일에 훅을 넣고 뺀다.
//!
//! Stop 훅은 에이전트를 세울 수 있어서 조심한다. 어떤 오류든 종료 코드 0에 출력 없음으로 끝나고(fail open),
//! 세션마다 한 번만 막고, 예산 안에 못 끝내면 그냥 지나간다. 기록에는 개수와 해시만 남기고 이름과 경로는 남기지 않는다.
//! 검사에 쓰는 인덱스는 저장소에 없으면 데이터 폴더에 두는데, 거기에는 심볼 이름과 경로만 있고 코드는 없다.
//! 막는 것은 측정으로 정밀도 기준을 넘긴 종류뿐이다([`BlockPolicy::MEASURED`]). 지금은 그런 종류가 없어 세우지 않는다.

use crate::check::{self, plural, BlockPolicy, Finding, FindingKind};
use crate::git::git;
use crate::lang::Lang;
use crate::parse::Parser;
use crate::sessions::{self, path_key, Agent, Homes, Op, Scope, Seen, SessionRef, SessionSummary};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::io::{Read, Write as _};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 이 안에 못 끝내면 지나간다. 훅 전체 3초 약속에서 프로세스 시작·종료 몫을 뺐다.
/// 2.5초로 두면 PC가 바쁠 때 전체가 3.2초까지 나왔다. 평소 실행은 0.5초 안쪽이라 여유가 있다
pub const STOP_HOOK_BUDGET: Duration = Duration::from_millis(2000);
/// 훅 입력 크기 상한
const MAX_INPUT: u64 = 1024 * 1024;
/// 이보다 큰 세션 기록(하위 에이전트 포함)은 예산 안에 못 읽는다. 실제 로그에서 50~200MB/s가 나왔다
const MAX_TRANSCRIPT_BYTES: u64 = 64 * 1024 * 1024;
const HOOK_TIMEOUT_SECS: u64 = 10;
const HOOK_MARK: &str = "check --stop-hook";
/// 다른 프로세스가 인덱스를 쓰는 동안 기다리는 시간. 넘기면 저장된 인덱스로 본다. 예산 안에 끝나야 한다
const INDEX_WAIT: Duration = Duration::from_millis(500);
/// 막는 이유에 넣을 줄 수
const MAX_REASON_LINES: usize = 8;
/// 이보다 오래된 세션 상태는 지운다
const STATE_KEEP: Duration = Duration::from_secs(30 * 86_400);

/// 앱과 같은 데이터 폴더. `LANTERN_DATA_DIR`로 바꿀 수 있다 (테스트 격리용)
pub fn data_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("LANTERN_DATA_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    dirs::data_dir().map(|d| d.join("lantern"))
}

/// Claude Code가 Stop 훅에 넘기는 값. 모두 선택으로 본다
#[derive(Deserialize, Default)]
struct StopInput {
    session_id: Option<String>,
    transcript_path: Option<String>,
    cwd: Option<String>,
    #[serde(default)]
    stop_hook_active: bool,
}

/// 세션마다 남기는 상태. 파일 해시만 있다
#[derive(Serialize, Deserialize, Default)]
struct SessionState {
    blocked: bool,
    flagged: Vec<String>,
    edited: Vec<String>,
}

fn short_hash(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex()[..16].to_string()
}

fn state_path(data: &Path, session: &str) -> PathBuf {
    data.join("check-sessions").join(format!("{}.json", short_hash(session)))
}

impl SessionState {
    /// 없으면 빈 상태. 읽을 수 없으면 오류: 한 번만 막는다는 약속을 지킬 수 없으니 막지 않는다
    fn load(data: &Path, session: &str) -> Result<Self> {
        match std::fs::read(state_path(data, session)) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    fn save(&self, data: &Path, session: &str) -> Result<()> {
        let path = state_path(data, session);
        let dir = path.parent().context("상태 폴더")?;
        std::fs::create_dir_all(dir)?;
        let fresh = !path.exists();
        write_atomic(&path, &serde_json::to_vec(self)?)?;
        if fresh {
            forget_old_states(dir);
        }
        Ok(())
    }
}

fn forget_old_states(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = SystemTime::now();
    for e in entries.flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| now.duration_since(t).ok()).is_some_and(|age| age > STATE_KEEP);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// 같은 폴더의 임시 파일에 쓰고 바꿔 치운다. 도중에 죽어도 원본이 잘리지 않는다
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

/// `check-log.jsonl`의 한 줄. 개수만 있고 이름도 경로도 없다
#[derive(Serialize, Deserialize, Default)]
struct LogLine {
    v: u32,
    ts: u64,
    session: String,
    #[serde(default)]
    code_files: u32,
    #[serde(default)]
    dangling: u32,
    #[serde(default)]
    stale: u32,
    #[serde(default)]
    flagged_new: u32,
    #[serde(default)]
    edited_new: u32,
    #[serde(default)]
    blocked: bool,
    #[serde(default)]
    elapsed_ms: u64,
}

fn log_path(data: &Path) -> PathBuf {
    data.join("check-log.jsonl")
}

/// 기록을 못 남겨도 훅을 망치지 않는다
fn append_log(data: &Path, line: &LogLine) {
    let write = || -> Result<()> {
        std::fs::create_dir_all(data)?;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(log_path(data))?;
        writeln!(f, "{}", serde_json::to_string(line)?)?;
        Ok(())
    };
    let _ = write();
}

/// 시간이 다 됐을 때 인덱싱 중이었으면 끝낼 수 있게 뒤에서 돌릴 수 있도록 알려 둔다
#[derive(Default)]
pub struct Progress {
    indexing: Mutex<Option<PathBuf>>,
}

impl Progress {
    fn set(&self, root: Option<PathBuf>) {
        if let Ok(mut g) = self.indexing.lock() {
            *g = root;
        }
    }

    fn take(&self) -> Option<PathBuf> {
        self.indexing.try_lock().ok().and_then(|mut g| g.take())
    }
}

/// 세션 기록을 읽는다. 없거나 예산 안에 못 읽을 만큼 크면 `None`: 그때는 작업 트리만 본다
fn read_transcript(path: &Path, session: &str, scope: &Scope, max_bytes: u64) -> Option<SessionSummary> {
    if !path.is_file() {
        return None;
    }
    let mut logs = Vec::new();
    sessions::walk_files(&path.with_extension("").join("subagents"), &mut logs);
    logs.retain(|p| p.extension().is_some_and(|x| x == "jsonl"));
    logs.sort();
    logs.insert(0, path.to_path_buf());
    let total: u64 = logs.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
    if total > max_bytes {
        return None;
    }
    let session = SessionRef { agent: Agent::ClaudeCode, id: session.to_string(), files: logs, modified: SystemTime::now() };
    Some(sessions::parse_session(&session, scope, &mut Seen::default()))
}

fn file_hash(session: &str, rel: &str) -> String {
    short_hash(&format!("{session}\0{}", path_key(Path::new(rel))))
}

fn reason_for(blocking: &[&Finding]) -> String {
    let mut out = String::from("Lantern check found references this change may have left behind (name based, may be wrong):\n");
    for f in blocking.iter().take(MAX_REASON_LINES) {
        let caller = f.caller.as_deref().map(|c| format!(" in `{c}`")).unwrap_or_default();
        let what = match (f.kind, f.renamed_to.as_deref()) {
            (FindingKind::Dangling, Some(new)) => format!("`{}` was removed from {} (looks renamed to `{new}`)", f.symbol, f.changed_file),
            (FindingKind::Dangling, None) => format!("`{}` was removed from {}", f.symbol, f.changed_file),
            (FindingKind::StaleCaller, _) => format!("the signature of `{}` changed in {}", f.symbol, f.changed_file),
        };
        let _ = writeln!(out, "- {what}, but {}:{}{caller} still uses that name", f.path, f.line);
    }
    if blocking.len() > MAX_REASON_LINES {
        let _ = writeln!(out, "- and {} more", blocking.len() - MAX_REASON_LINES);
    }
    out.push_str("Check these before finishing. If they are fine, say so and stop. This check will not stop you again in this session.");
    out
}

/// Stop 훅 본체. 측정으로 기준을 넘긴 종류만 막는다. 지금은 넘긴 종류가 없어 개수만 남기고 막지 않는다.
/// `base`를 주면 시작 본문을 전부 그 커밋에서 읽는다.
pub fn stop_hook(input: &str, data: &Path, fallback_root: &Path, base: Option<&str>, progress: &Progress) -> Result<Option<String>> {
    stop_hook_with(input, data, fallback_root, base, progress, BlockPolicy::MEASURED)
}

/// 막아야 하면 stdout에 쓸 JSON을 돌려준다. 정책을 넘기는 것은 기준을 넘긴 뒤의 동작을 시험하기 위해서다
pub fn stop_hook_with(input: &str, data: &Path, fallback_root: &Path, base: Option<&str>, progress: &Progress, policy: BlockPolicy) -> Result<Option<String>> {
    let started = Instant::now();
    let input: StopInput = serde_json::from_str(input).context("훅 입력")?;
    let session = input.session_id.as_deref().filter(|s| !s.is_empty()).context("session_id가 없습니다")?;
    let mut state = SessionState::load(data, session)?;
    let done = input.stop_hook_active || state.blocked;
    // 이미 막았거나 막은 뒤 이어지는 중이면 할 일은 뒷정리뿐이다. 그것도 없으면 바로 끝낸다
    if done && state.flagged.is_empty() {
        return Ok(None);
    }

    let root = check::repo_root(input.cwd.as_deref().map_or(fallback_root, Path::new))?;
    let scope = Scope::new([root.clone()]);
    let rel = |p: &Path| scope.relative(p).filter(|r| !r.is_empty());
    let summary = input.transcript_path.as_deref().and_then(|p| read_transcript(Path::new(p), session, &scope, MAX_TRANSCRIPT_BYTES));
    // 형식이 바뀐 로그는 믿지 않고 작업 트리만 본다
    let log = summary.as_ref().filter(|s| s.format_warnings.is_empty() && s.parsed_lines > 0);
    let session_files: Option<Vec<String>> = log.map(|s| s.touched_paths().into_iter().filter_map(rel).collect());

    let mut line = LogLine { v: 1, ts: unix_now(), session: short_hash(session), ..Default::default() };
    for f in session_files.iter().flatten() {
        let h = file_hash(session, f);
        if state.flagged.contains(&h) && !state.edited.contains(&h) {
            state.edited.push(h);
            line.edited_new += 1;
        }
    }
    if done {
        if line.edited_new > 0 {
            state.save(data, session)?;
            line.elapsed_ms = started.elapsed().as_millis() as u64;
            append_log(data, &line);
        }
        return Ok(None);
    }

    // 이 세션이 코드를 고치지 않았으면 볼 것이 없다
    let is_code = |f: &String| Lang::from_path(Path::new(f)).is_some();
    if session_files.as_ref().is_some_and(|files| !files.iter().any(is_code)) {
        return Ok(None);
    }

    let mut before: HashMap<String, String> = HashMap::new();
    let mut earliest: Option<&str> = None;
    if let (Some(s), None) = (log, base) {
        for fc in s.file_changes() {
            if let (Some(r), Some(text)) = (rel(fc.path), fc.before) {
                before.insert(path_key(Path::new(&r)), text.to_string());
            }
        }
        // 로그에 시작 본문이 없는 파일은 첫 편집 직전 커밋에서 읽는다 (세션 중에 커밋했을 수 있다)
        earliest = s.events.iter().filter(|e| e.op == Op::Edit).find_map(|e| {
            let known = rel(e.path.as_deref()?).is_some_and(|r| before.contains_key(&path_key(Path::new(&r))));
            if known {
                None
            } else {
                e.ts.as_deref()
            }
        });
    }
    let commit = match (base, earliest) {
        (Some(b), _) => check::resolve_commit(&root, b)?,
        (None, Some(ts)) => {
            let at = format!("--before={ts}");
            let found = git(&root, &["rev-list", "-1", &at, "HEAD"]).unwrap_or_default();
            match found.trim() {
                "" => check::resolve_commit(&root, "HEAD")?,
                hash => hash.to_string(),
            }
        }
        (None, None) => check::resolve_commit(&root, "HEAD")?,
    };

    let diff = check::changed_files(&root, &commit)?;
    let files = match session_files {
        Some(f) => check::git_spelling(&root, &commit, f),
        None => diff.clone(),
    };
    let mut parser = Parser::new()?;
    let changes = check::analyze_files(&root, &commit, &files, &before, &mut parser)?.changes;
    line.code_files = changes.len() as u32;

    let findings = if check::may_leave_behind(&changes) {
        let touched: Vec<String> = diff.into_iter().chain(files).collect();
        let read: Vec<String> = log.map(|s| s.read_paths().into_iter().filter_map(rel).collect()).unwrap_or_default();
        let reads_known = log.is_some_and(|s| s.reads_known);
        progress.set(Some(root.clone()));
        let index = check::refreshed_index(&root, data, &mut parser, INDEX_WAIT)?;
        progress.set(None);
        check::left_behind(&index.store, &check::read_under(&root), &changes, &touched, &read, reads_known)?
    } else {
        Vec::new()
    };

    line.dangling = findings.iter().filter(|f| f.kind == FindingKind::Dangling).count() as u32;
    line.stale = findings.iter().filter(|f| f.kind == FindingKind::StaleCaller).count() as u32;
    let flagged: HashSet<&str> = findings.iter().map(|f| f.path.as_str()).collect();
    for path in flagged {
        let h = file_hash(session, path);
        if !state.flagged.contains(&h) {
            state.flagged.push(h);
            line.flagged_new += 1;
        }
    }
    let blocking: Vec<&Finding> = findings.iter().filter(|f| policy.blocks(f)).collect();
    line.blocked = !blocking.is_empty();
    state.blocked = line.blocked;
    if line.blocked || line.flagged_new > 0 || line.edited_new > 0 {
        // 막기 전에 먼저 남긴다. 못 남기면 오류로 끝나 막지 않는다 (한 번만 막는다는 약속이 먼저다)
        state.save(data, session)?;
    }
    line.elapsed_ms = started.elapsed().as_millis() as u64;
    if line.code_files > 0 {
        append_log(data, &line);
    }
    if blocking.is_empty() {
        return Ok(None);
    }
    Ok(Some(json!({ "decision": "block", "reason": reason_for(&blocking) }).to_string()))
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// 다른 스레드에서 돌려 예산 안에 끝난 답만 받는다. 패닉과 시간 초과는 `None`
pub fn guarded<T: Send + 'static>(budget: Duration, f: impl FnOnce() -> Option<T> + Send + 'static) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .spawn(move || {
            let out = catch_unwind(AssertUnwindSafe(f)).ok().flatten();
            let _ = tx.send(out);
        })
        .ok()?;
    rx.recv_timeout(budget).ok().flatten()
}

/// `lantern check --stop-hook`: stdin을 읽어 돌리고 stdout에 쓸 글을 돌려준다.
/// 큰 저장소의 첫 인덱싱은 예산에 못 들어와 늘 되돌려지므로, 그때는 뒤에서 끝내 두어 다음 번에 쓴다.
pub fn stop_hook_process(fallback_root: &Path, base: Option<&str>) -> Option<String> {
    keep_stdio_from_children();
    let data = data_dir()?;
    let progress = Arc::new(Progress::default());
    let worker = {
        let (data, progress) = (data.clone(), progress.clone());
        let (root, base) = (fallback_root.to_path_buf(), base.map(str::to_string));
        move || {
            let mut text = String::new();
            std::io::stdin().take(MAX_INPUT).read_to_string(&mut text).ok()?;
            stop_hook(&text, &data, &root, base.as_deref(), &progress).ok().flatten()
        }
    };
    // 디버그 빌드는 태그 질의를 만드는 데만 1초 넘게 걸려 테스트에서 늘린다
    let budget = std::env::var("LANTERN_CHECK_BUDGET_MS").ok().and_then(|s| s.parse().ok()).map_or(STOP_HOOK_BUDGET, Duration::from_millis);
    let out = guarded(budget, worker);
    if out.is_none() {
        if let Some(root) = progress.take() {
            index_in_background(&root, &data);
        }
    }
    out
}

/// Windows의 자식 프로세스는 상속 가능한 핸들을 모두 물려받는다. git이나 뒤의 인덱싱이 훅의 출력 파이프를 들고 있으면
/// 훅이 끝나도 읽는 쪽(Claude Code)이 파이프 끝을 보지 못해 그만큼 기다린다.
#[cfg(windows)]
fn keep_stdio_from_children() {
    use std::os::windows::io::AsRawHandle;
    extern "system" {
        fn SetHandleInformation(handle: *mut std::ffi::c_void, mask: u32, flags: u32) -> i32;
    }
    const HANDLE_FLAG_INHERIT: u32 = 1;
    for handle in [std::io::stdout().as_raw_handle(), std::io::stderr().as_raw_handle()] {
        // SAFETY: 표준 입출력 핸들의 상속 표시만 끈다. 없는 핸들이면 실패하고 아무 일도 없다
        unsafe {
            SetHandleInformation(handle.cast(), HANDLE_FLAG_INHERIT, 0);
        }
    }
}

#[cfg(not(windows))]
fn keep_stdio_from_children() {}

/// 훅이 기다리지 않는 낮은 우선순위 인덱싱. 훅과 같은 인덱스를 같은 규칙으로 만든다 (데이터 폴더면 이름만)
fn index_in_background(root: &Path, data: &Path) {
    let Ok(exe) = std::env::current_exe() else { return };
    let mut cmd = Command::new(exe);
    cmd.arg("-C").arg(root).args(["check", "--index-only"]).env("LANTERN_DATA_DIR", data).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
        cmd.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
    }
    let _ = cmd.spawn();
}

/// `lantern check --stats`의 합계
#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct HookStats {
    pub runs: u64,
    pub sessions: u64,
    pub blocked: u64,
    pub dangling: u64,
    pub stale: u64,
    pub flagged_files: u64,
    pub later_edited: u64,
}

pub fn stats(data: &Path) -> HookStats {
    let mut out = HookStats::default();
    let Ok(text) = std::fs::read_to_string(log_path(data)) else { return out };
    let mut sessions = HashSet::new();
    for line in text.lines().filter_map(|l| serde_json::from_str::<LogLine>(l).ok()) {
        out.runs += 1;
        out.blocked += u64::from(line.blocked);
        out.dangling += u64::from(line.dangling);
        out.stale += u64::from(line.stale);
        out.flagged_files += u64::from(line.flagged_new);
        out.later_edited += u64::from(line.edited_new);
        sessions.insert(line.session);
    }
    out.sessions = sessions.len() as u64;
    out
}

impl HookStats {
    pub fn to_text(&self) -> String {
        if self.runs == 0 {
            return "lantern check: no stop hook runs logged yet.\n".to_string();
        }
        format!(
            "lantern check, stop hook log: {} in {}, blocked {}.\nFlagged {}, later edited {}.\nName based. Later edited means the agent edited a flagged file afterwards in the same session, not that the flag was right.\n",
            plural(self.runs as usize, "run"),
            plural(self.sessions as usize, "session"),
            plural(self.blocked as usize, "time"),
            plural(self.flagged_files as usize, "file"),
            self.later_edited
        )
    }
}

// ── 설정 파일 ────────────────────────────────────────────

/// 설정 파일 위치: 기본은 커밋하지 않는 `.claude/settings.local.json`, `user`면 사용자 설정
pub fn claude_settings_path(root: &Path, user: bool, homes: &Homes) -> PathBuf {
    if user {
        homes.claude.join("settings.json")
    } else {
        root.join(".claude").join("settings.local.json")
    }
}

/// 훅 명령. Windows에서 Claude Code는 훅을 Git Bash로 돌려 역슬래시가 사라지므로 슬래시로 쓴다
pub fn stop_hook_command(exe: &Path) -> String {
    format!("\"{}\" {HOOK_MARK}", exe.to_string_lossy().replace('\\', "/"))
}

fn is_our_command(cmd: &str) -> bool {
    cmd.contains(HOOK_MARK) && cmd.to_lowercase().contains("lantern")
}

/// 프로젝트 설정 파일이 git에 잡힐 수 있으면 알려 줄 말. 기계마다 다른 경로가 커밋되면 곤란하다
pub fn commit_warning(root: &Path, settings: &Path) -> Option<String> {
    git(root, &["rev-parse", "--git-dir"]).ok()?;
    let rel = settings.strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/");
    if git(root, &["check-ignore", "-q", "--", &rel]).is_ok() {
        return None;
    }
    Some(format!("Note: {rel} is not ignored by git. It holds a path from this machine, so keep it out of commits."))
}

/// 없으면 빈 객체. 객체가 아니거나 읽을 수 없으면 건드리지 않게 오류로 낸다
fn load_settings(path: &Path) -> Result<Option<Map<String, Value>>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("{} 읽기", path.display())),
    };
    if text.trim().is_empty() {
        return Ok(Some(Map::new()));
    }
    match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
        Ok(Value::Object(m)) => Ok(Some(m)),
        Ok(_) => bail!("{}가 JSON 객체가 아니라 건드리지 않았습니다", path.display()),
        Err(e) => bail!("{}를 JSON으로 읽지 못해 건드리지 않았습니다: {e}", path.display()),
    }
}

/// 원본을 지우지 않게 비어 있는 이름을 찾아 복사한다
fn backup_settings(path: &Path) -> Result<PathBuf> {
    for n in 0..1000 {
        let suffix = if n == 0 { ".lantern-backup".to_string() } else { format!(".lantern-backup.{n}") };
        let mut name = path.as_os_str().to_owned();
        name.push(&suffix);
        let backup = PathBuf::from(name);
        if !backup.exists() {
            std::fs::copy(path, &backup).with_context(|| format!("{} 백업", path.display()))?;
            return Ok(backup);
        }
    }
    bail!("{} 백업 이름이 다 찼습니다", path.display())
}

fn save_settings(path: &Path, settings: &Map<String, Value>) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut text = serde_json::to_string_pretty(settings)?;
    text.push('\n');
    write_atomic(path, text.as_bytes()).with_context(|| format!("{} 쓰기", path.display()))
}

/// 설정에 Stop 훅을 넣는다. 다른 키는 그대로 두고, 쓰기 전에 백업한다. 무엇을 바꿨는지 사람이 읽을 줄들을 돌려준다
pub fn install_claude(path: &Path, command: &str) -> Result<Vec<String>> {
    let existing = load_settings(path)?;
    let existed = existing.is_some();
    let mut settings = existing.unwrap_or_default();
    let hooks = settings.entry("hooks").or_insert_with(|| json!({})).as_object_mut();
    let hooks = hooks.with_context(|| format!("{}의 hooks가 객체가 아니라 건드리지 않았습니다", path.display()))?;
    let stop = hooks.entry("Stop").or_insert_with(|| json!([])).as_array_mut();
    let stop = stop.with_context(|| format!("{}의 hooks.Stop이 목록이 아니라 건드리지 않았습니다", path.display()))?;

    let mut replaced = None;
    let mut present = false;
    for hook in stop.iter_mut().filter_map(|g| g.get_mut("hooks")).filter_map(Value::as_array_mut).flatten() {
        let Some(cmd) = hook.get("command").and_then(Value::as_str).map(str::to_string) else { continue };
        if !is_our_command(&cmd) {
            continue;
        }
        present = true;
        if cmd != command {
            hook["command"] = json!(command);
            replaced = Some(cmd);
        }
    }
    if present && replaced.is_none() {
        return Ok(vec![format!("Already installed in {}. Nothing changed.", path.display())]);
    }
    if !present {
        stop.push(json!({ "hooks": [{ "type": "command", "command": command, "timeout": HOOK_TIMEOUT_SECS }] }));
    }

    let mut lines = Vec::new();
    if existed {
        lines.push(format!("Backup: {}", backup_settings(path)?.display()));
    } else {
        lines.push(format!("Created {}", path.display()));
    }
    save_settings(path, &settings)?;
    match replaced {
        Some(old) => lines.push(format!("Changed hooks.Stop in {}: {old} -> {command}", path.display())),
        None => lines.push(format!("Added to hooks.Stop in {}: {command}", path.display())),
    }
    Ok(lines)
}

/// 설정에서 우리 Stop 훅만 뺀다. 비게 된 그룹과 키만 치운다
pub fn uninstall_claude(path: &Path) -> Result<Vec<String>> {
    let nothing = || vec![format!("Not installed in {}. Nothing changed.", path.display())];
    let Some(mut settings) = load_settings(path)? else { return Ok(nothing()) };
    let mut removed = Vec::new();
    if let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) {
        if let Some(stop) = hooks.get_mut("Stop").and_then(Value::as_array_mut) {
            for group in stop.iter_mut() {
                let Some(list) = group.get_mut("hooks").and_then(Value::as_array_mut) else { continue };
                list.retain(|h| match h.get("command").and_then(Value::as_str) {
                    Some(cmd) if is_our_command(cmd) => {
                        removed.push(cmd.to_string());
                        false
                    }
                    _ => true,
                });
            }
            if !removed.is_empty() {
                stop.retain(|g| g.get("hooks").and_then(Value::as_array).is_none_or(|l| !l.is_empty()));
            }
            if stop.is_empty() && !removed.is_empty() {
                hooks.remove("Stop");
            }
        }
        if hooks.is_empty() && !removed.is_empty() {
            settings.remove("hooks");
        }
    }
    if removed.is_empty() {
        return Ok(nothing());
    }
    let backup = backup_settings(path)?;
    save_settings(path, &settings)?;
    let mut lines = vec![format!("Backup: {}", backup.display())];
    lines.extend(removed.iter().map(|cmd| format!("Removed from hooks.Stop in {}: {cmd}", path.display())));
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CMD: &str = "\"C:/tools/lantern.exe\" check --stop-hook";

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn install_merges_without_clobbering_and_backs_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let original = r#"{
  "zeta": 1,
  "permissions": { "allow": ["Bash(ls)"] },
  "hooks": {
    "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "other-tool" }] }],
    "Stop": [{ "hooks": [{ "type": "command", "command": "say done" }] }]
  },
  "alpha": { "x": [1, 2] }
}
"#;
        std::fs::write(&path, original).unwrap();
        let lines = install_claude(&path, CMD).unwrap();
        assert!(lines.iter().any(|l| l.contains("Added") && l.contains(CMD)), "{lines:?}");
        assert!(lines.iter().any(|l| l.starts_with("Backup:")), "{lines:?}");

        // 백업은 원본 그대로
        let backup = dir.path().join("settings.json.lantern-backup");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), original);

        let v = read_json(&path);
        assert_eq!(v["zeta"], 1);
        assert_eq!(v["permissions"]["allow"][0], "Bash(ls)");
        assert_eq!(v["alpha"]["x"][1], 2);
        assert_eq!(v["hooks"]["PreToolUse"][0]["matcher"], "Bash");
        let stop = v["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[0]["hooks"][0]["command"], "say done");
        assert_eq!(stop[1]["hooks"][0]["command"], CMD);
        assert_eq!(stop[1]["hooks"][0]["type"], "command");

        // 키 순서도 그대로
        let text = std::fs::read_to_string(&path).unwrap();
        let at = |k: &str| text.find(k).unwrap();
        assert!(at("\"zeta\"") < at("\"permissions\"") && at("\"permissions\"") < at("\"hooks\"") && at("\"hooks\"") < at("\"alpha\""), "{text}");
    }

    #[test]
    fn install_twice_changes_nothing_and_new_path_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("settings.local.json");
        let lines = install_claude(&path, CMD).unwrap();
        assert!(lines[0].starts_with("Created"), "{lines:?}");
        let first = std::fs::read(&path).unwrap();

        let lines = install_claude(&path, CMD).unwrap();
        assert!(lines[0].starts_with("Already installed"), "{lines:?}");
        assert_eq!(std::fs::read(&path).unwrap(), first);
        assert!(!dir.path().join("nested").join("settings.local.json.lantern-backup").exists());

        // 실행 파일이 옮겨 갔으면 줄을 늘리지 않고 고친다
        let moved = "\"D:/new/lantern.exe\" check --stop-hook";
        let lines = install_claude(&path, moved).unwrap();
        assert!(lines.iter().any(|l| l.contains("Changed")), "{lines:?}");
        let v = read_json(&path);
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], moved);
    }

    #[test]
    fn broken_or_odd_settings_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        for text in ["{ not json", "[1, 2]", r#"{"hooks": 1}"#, r#"{"hooks": {"Stop": {}}}"#] {
            std::fs::write(&path, text).unwrap();
            assert!(install_claude(&path, CMD).is_err(), "{text}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        }
        assert!(!dir.path().join("settings.json.lantern-backup").exists());
    }

    #[test]
    fn uninstall_removes_only_ours() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"keep": true, "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}, {"type": "command", "command": "\"C:/x/lantern.exe\" check --stop-hook"}]}, {"hooks": [{"type": "command", "command": "lantern check --stop-hook"}]}], "SessionStart": [{"hooks": []}]}}"#,
        )
        .unwrap();
        let lines = uninstall_claude(&path).unwrap();
        assert_eq!(lines.iter().filter(|l| l.starts_with("Removed")).count(), 2, "{lines:?}");
        assert!(lines[0].starts_with("Backup:"));
        let v = read_json(&path);
        assert_eq!(v["keep"], true);
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert_eq!(v["hooks"]["Stop"][0]["hooks"].as_array().unwrap().len(), 1);
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], "say done");
        assert!(v["hooks"]["SessionStart"].is_array());

        // 두 번째는 할 일이 없다
        let before = std::fs::read(&path).unwrap();
        assert!(uninstall_claude(&path).unwrap()[0].starts_with("Not installed"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn uninstall_cleans_empty_keys_and_handles_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert!(uninstall_claude(&path).unwrap()[0].starts_with("Not installed"));
        assert!(!path.exists());
        install_claude(&path, CMD).unwrap();
        uninstall_claude(&path).unwrap();
        assert_eq!(read_json(&path), json!({}));
    }

    #[test]
    fn command_uses_forward_slashes_and_is_recognized() {
        let cmd = stop_hook_command(Path::new(r"C:\Program Files\lantern\lantern.exe"));
        assert_eq!(cmd, "\"C:/Program Files/lantern/lantern.exe\" check --stop-hook");
        assert!(is_our_command(&cmd));
        assert!(!is_our_command("other check --stop-hook"));
        assert!(!is_our_command("lantern check"));
    }

    #[test]
    fn settings_paths_are_injected() {
        let homes = Homes::under(Path::new("/home/x"));
        assert_eq!(claude_settings_path(Path::new("/r"), false, &homes), Path::new("/r/.claude/settings.local.json"));
        assert_eq!(claude_settings_path(Path::new("/r"), true, &homes), Path::new("/home/x/.claude/settings.json"));
    }

    #[test]
    fn guard_turns_panic_and_timeout_into_nothing() {
        assert_eq!(guarded(Duration::from_secs(5), || Some(7)), Some(7));
        assert_eq!(guarded::<i32>(Duration::from_secs(5), || panic!("망가짐")), None);
        let slow = guarded(Duration::from_millis(50), || {
            std::thread::sleep(Duration::from_secs(5));
            Some(1)
        });
        assert_eq!(slow, None);
    }

    #[test]
    fn broken_session_state_means_no_block() {
        let dir = tempfile::tempdir().unwrap();
        let path = state_path(dir.path(), "s1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ 깨진").unwrap();
        assert!(SessionState::load(dir.path(), "s1").is_err());
        let input = json!({ "session_id": "s1", "cwd": dir.path() }).to_string();
        assert!(stop_hook(&input, dir.path(), dir.path(), None, &Progress::default()).is_err());
    }

    #[test]
    fn stats_sum_lines_and_skip_broken_ones() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(stats(dir.path()), HookStats::default());
        assert!(stats(dir.path()).to_text().contains("no stop hook runs"));
        let line = |session: &str, flagged, edited, blocked| LogLine {
            v: 1,
            session: session.to_string(),
            flagged_new: flagged,
            edited_new: edited,
            dangling: flagged,
            blocked,
            ..Default::default()
        };
        append_log(dir.path(), &line("a", 3, 0, true));
        append_log(dir.path(), &line("a", 0, 2, false));
        append_log(dir.path(), &line("b", 1, 0, false));
        let mut f = std::fs::OpenOptions::new().append(true).open(log_path(dir.path())).unwrap();
        writeln!(f, "깨진 줄").unwrap();
        let s = stats(dir.path());
        assert_eq!((s.runs, s.sessions, s.blocked, s.flagged_files, s.later_edited), (3, 2, 1, 4, 2));
        assert!(s.to_text().contains("Flagged 4 files, later edited 2"), "{}", s.to_text());
    }
}
