//! 코딩 에이전트가 디스크에 남긴 세션 기록을 읽는다 (Claude Code, Codex).
//!
//! 읽기 전용이고 네트워크를 쓰지 않는다. 메시지 본문과 명령 문자열은 분류만 하고 버린다.
//! 파일 전문(`EditDetail`)과 첫 질문은 메모리에만 두고 파일로 쓰거나 출력하지 않는다.
//! 로그 형식은 언제든 바뀔 수 있어 모든 필드를 선택으로 보고, 모르는 줄은 세기만 한다.

pub mod claude;
pub mod codex;

use regex::Regex;
use ruzstd::decoding::{FrameDecoder, StreamingDecoder};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

/// 이보다 긴 줄은 건너뛴다 (메모리 보호)
pub const MAX_LINE: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Agent {
    ClaudeCode,
    Codex,
}

impl Agent {
    pub fn name(self) -> &'static str {
        match self {
            Agent::ClaudeCode => "claude",
            Agent::Codex => "codex",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionRef {
    pub agent: Agent,
    pub id: String,
    /// 본 로그가 먼저, 그다음 하위 에이전트 로그
    pub files: Vec<PathBuf>,
    pub modified: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Read,
    Edit,
    Shell,
    Search,
}

#[derive(Debug, Clone)]
pub struct AgentEvent {
    pub ts: Option<String>,
    pub op: Op,
    /// 슬래시로 통일한 절대 경로. Shell은 작업 폴더, Search는 찾은 폴더
    pub path: Option<PathBuf>,
    pub ok: Option<bool>,
    /// 편집의 텍스트. 결과를 모르거나 실패한 편집은 `None`
    pub edit: Option<EditDetail>,
    /// 부분 읽기의 줄 범위 (1부터, 양끝 포함). 전체를 읽었으면 `None`
    pub range: Option<(u32, u32)>,
    pub test_run: bool,
}

impl AgentEvent {
    pub(crate) fn new(op: Op, ts: Option<String>, path: Option<PathBuf>) -> Self {
        Self { ts, op, path, ok: None, edit: None, range: None, test_run: false }
    }
}

/// 편집 전후의 파일 전문. 절대 저장하거나 출력하지 않는다.
/// 메모리를 아끼려고 파일마다 첫 `before`와 마지막 `after`, 그 뒤의 `patch`만 남긴다.
#[derive(Clone, Default)]
pub struct EditDetail {
    pub before: Option<String>,
    pub after: Option<String>,
    /// `after`를 만들 본문이 로그에 없을 때 나중에 기준 커밋 본문에 다시 적용할 변경
    pub patch: Option<Patch>,
}

impl fmt::Debug for EditDetail {
    // 실수로 찍혀도 본문이 나가지 않게 길이만 보여 준다
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EditDetail")
            .field("before_len", &self.before.as_ref().map(String::len))
            .field("after_len", &self.after.as_ref().map(String::len))
            .field("has_patch", &self.patch.is_some())
            .finish()
    }
}

#[derive(Clone, Default)]
pub struct Patch {
    pub replace: Vec<Replace>,
    pub hunks: Vec<Hunk>,
}

#[derive(Clone)]
pub struct Replace {
    pub old: String,
    pub new: String,
    pub all: bool,
}

#[derive(Clone)]
pub struct Hunk {
    /// 옛 파일에서 시작하는 줄 (1부터). 줄 번호가 없는 형식은 `None`이고 내용으로 찾는다
    pub old_start: Option<u32>,
    /// 앞 글자가 ` `(문맥), `-`, `+`인 줄들
    pub lines: Vec<String>,
}

/// 문자열 치환을 먼저, 안 되면 줄 단위 조각을 적용한다. 둘 다 안 되면 `None`
pub fn apply_patch(text: &str, patch: &Patch) -> Option<String> {
    if !patch.replace.is_empty() {
        let replaced = patch.replace.iter().try_fold(text.to_string(), |t, r| apply_replace(&t, r));
        if replaced.is_some() {
            return replaced;
        }
    }
    if patch.hunks.is_empty() {
        return None;
    }
    apply_hunks(text, &patch.hunks)
}

fn apply_replace(text: &str, r: &Replace) -> Option<String> {
    if r.old.is_empty() {
        // Edit의 빈 old_string은 새 파일 만들기다
        return text.is_empty().then(|| r.new.clone());
    }
    let run = |t: &str, old: &str, new: &str| {
        if !t.contains(old) {
            None
        } else if r.all {
            Some(t.replace(old, new))
        } else {
            Some(t.replacen(old, new, 1))
        }
    };
    run(text, &r.old, &r.new).or_else(|| {
        // 로그의 문자열은 LF인데 기준 커밋 본문은 CRLF일 수 있다
        if !text.contains('\r') {
            return None;
        }
        run(&text.replace("\r\n", "\n"), &r.old.replace("\r\n", "\n"), &r.new.replace("\r\n", "\n"))
    })
}

fn apply_hunks(text: &str, hunks: &[Hunk]) -> Option<String> {
    let lines: Vec<&str> = if text.is_empty() { Vec::new() } else { text.split('\n').collect() };
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut at = 0;
    for h in hunks {
        let old: Vec<&str> = h
            .lines
            .iter()
            .filter_map(|l| match l.as_bytes().first() {
                Some(b' ' | b'-') => Some(&l[1..]),
                None => Some(""),
                _ => None,
            })
            .collect();
        let pos = locate_hunk(&lines, &old, at, h.old_start)?;
        out.extend_from_slice(&lines[at..pos]);
        let mut i = pos;
        for l in &h.lines {
            match l.as_bytes().first() {
                Some(b'+') => out.push(&l[1..]),
                Some(b'-') => i += 1,
                Some(b'\\') => {} // "\ No newline at end of file"
                _ => {
                    out.push(lines[i]);
                    i += 1;
                }
            }
        }
        at = i;
    }
    out.extend_from_slice(&lines[at..]);
    Some(out.join("\n"))
}

/// 조각이 들어갈 자리. 줄 번호가 맞으면 거기, 아니면 내용으로 찾는다
fn locate_hunk(lines: &[&str], old: &[&str], from: usize, start: Option<u32>) -> Option<usize> {
    let same = |at: usize| {
        at + old.len() <= lines.len()
            && old.iter().zip(&lines[at..]).all(|(a, b)| a.trim_end_matches('\r') == b.trim_end_matches('\r'))
    };
    if let Some(s) = start {
        let at = (s as usize).saturating_sub(1);
        if at >= from && same(at) {
            return Some(at);
        }
    }
    if old.is_empty() {
        // 문맥 없이 끼워 넣기: 파일 끝 (마지막 줄바꿈 앞)
        let end = if lines.last() == Some(&"") { lines.len() - 1 } else { lines.len() };
        return Some(end.max(from));
    }
    (from..lines.len()).find(|&at| same(at))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// 캐시를 뺀 입력
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Usage {
    fn add(&mut self, o: &Usage) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
    }

    /// 같은 메시지가 여러 줄에 나오면 뒤로 갈수록 커진다
    fn merge_max(&mut self, o: &Usage) {
        self.input = self.input.max(o.input);
        self.output = self.output.max(o.output);
        self.cache_read = self.cache_read.max(o.cache_read);
        self.cache_write = self.cache_write.max(o.cache_write);
    }
}

/// 세션 하나를 읽은 결과. 본문이 담길 수 있어 `Debug`를 만들지 않는다.
/// 토큰은 세션 전체 기준이다 (한 세션이 여러 저장소를 건드려도 나누지 않는다).
pub struct SessionSummary {
    pub agent: Agent,
    pub id: String,
    /// 시각 순. `Scope` 안의 이벤트만
    pub events: Vec<AgentEvent>,
    pub models: Vec<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub usage_by_model: BTreeMap<String, Usage>,
    /// 메모리에만 둔다
    pub first_prompt: Option<String>,
    pub started: Option<String>,
    pub ended: Option<String>,
    /// 첫 편집 앞에 있던 이벤트 수 (`Scope` 안)
    pub tool_calls_before_first_edit: Option<u32>,
    /// 첫 편집까지의 입력(캐시 제외)+캐시 쓰기+출력
    pub tokens_before_first_edit: Option<u64>,
    /// 첫 편집까지 다시 읽은 캐시. 위와 섞으면 대화 길이에 따라 부풀어 나눠 둔다
    pub cache_read_before_first_edit: Option<u64>,
    pub unknown_lines: u64,
    pub parsed_lines: u64,
    pub oversized_lines: u64,
    pub unreadable_files: u32,
    /// 있는 본문에 변경을 적용하지 못한 횟수
    pub after_failed: u32,
    /// false면 무엇을 읽었는지 모른다 (Codex). "안 읽었다"고 말하면 안 된다
    pub reads_known: bool,
    /// 알아본 줄이 절반 아래인 파일: 형식이 바뀌었을 수 있다
    pub format_warnings: Vec<PathBuf>,
}

/// 한 세션에서 파일 하나가 바뀐 모습
pub struct FileChange<'a> {
    pub path: &'a Path,
    /// 세션 시작 때 본문. 로그에 없으면 `None`
    pub before: Option<&'a str>,
    /// 마지막으로 알 수 있는 본문
    pub after: Option<&'a str>,
    /// `after`(없으면 시작 본문) 위에 차례로 적용할 변경
    pub pending: Vec<&'a Patch>,
    /// 내용을 모르는 변경이 끼었다 (셸 등). 끝 상태는 git에서 봐야 한다
    pub opaque: bool,
    pub edits: u32,
    /// 이 파일의 첫 편집 시각. 세션이 몇 주씩 이어지기도 해서 기준 커밋은 세션 시작보다 이 시각으로 잡는 편이 맞다
    pub first_ts: Option<&'a str>,
    pub last_ts: Option<&'a str>,
}

impl FileChange<'_> {
    /// (시작 본문, 끝 본문). `base`는 로그에 시작 본문이 없을 때 쓸 기준 커밋 본문.
    /// 파일이 기준 커밋에 없었으면 빈 문자열을 넘긴다. 알 수 없으면 `None`
    pub fn resolve(&self, base: Option<&str>) -> Option<(String, String)> {
        let start = self.before.or(base)?;
        if self.opaque {
            return None;
        }
        let from = self.after.unwrap_or(start);
        let end = self.pending.iter().try_fold(from.to_string(), |t, p| apply_patch(&t, p))?;
        Some((start.to_string(), end))
    }
}

impl SessionSummary {
    /// 편집한 파일별 시작과 끝. 처음 편집한 순서대로
    pub fn file_changes(&self) -> Vec<FileChange<'_>> {
        let mut out: Vec<FileChange<'_>> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for e in &self.events {
            let (Op::Edit, Some(path), Some(d)) = (e.op, e.path.as_deref(), e.edit.as_ref()) else { continue };
            let i = *index.entry(path_key(path)).or_insert_with(|| {
                out.push(FileChange {
                    path,
                    before: d.before.as_deref(),
                    after: None,
                    pending: Vec::new(),
                    opaque: false,
                    edits: 0,
                    first_ts: e.ts.as_deref(),
                    last_ts: None,
                });
                out.len() - 1
            });
            let fc = &mut out[i];
            fc.edits += 1;
            fc.last_ts = e.ts.as_deref();
            if let Some(a) = d.after.as_deref() {
                fc.after = Some(a);
                fc.pending.clear();
                fc.opaque = false;
            } else if let Some(p) = d.patch.as_ref() {
                fc.pending.push(p);
            } else {
                fc.after = None;
                fc.pending.clear();
                fc.opaque = true;
            }
        }
        out
    }

    /// 편집한 경로 (실패가 확실한 것 제외, 중복 제거)
    pub fn touched_paths(&self) -> Vec<&Path> {
        self.paths_of(Op::Edit)
    }

    /// 읽은 경로. `reads_known`이 false면 비어 있어도 안 읽었다는 뜻이 아니다
    pub fn read_paths(&self) -> Vec<&Path> {
        self.paths_of(Op::Read)
    }

    fn paths_of(&self, op: Op) -> Vec<&Path> {
        let mut seen = HashSet::new();
        self.events
            .iter()
            .filter(|e| e.op == op && e.ok != Some(false))
            .filter_map(|e| e.path.as_deref())
            .filter(|p| seen.insert(path_key(p)))
            .collect()
    }

    /// 점검용 요약: 개수와 경로만. 메시지·명령·코드는 없다
    pub fn counts_json(&self, scope: &Scope) -> Value {
        let rel = |p: &&Path| scope.relative(p).unwrap_or_else(|| p.display().to_string());
        let ops = |op: Op| self.events.iter().filter(|e| e.op == op).count();
        let failed = self.events.iter().filter(|e| e.ok == Some(false)).count();
        let changes = self.file_changes();
        let both = changes.iter().filter(|c| c.before.is_some() && c.after.is_some() && c.pending.is_empty()).count();
        let pending = changes.iter().filter(|c| !c.pending.is_empty()).count();
        let opaque = changes.iter().filter(|c| c.opaque).count();
        json!({
            "agent": self.agent.name(),
            "id": self.id,
            "lines": {
                "parsed": self.parsed_lines,
                "unknown": self.unknown_lines,
                "oversized": self.oversized_lines,
                "unreadable_files": self.unreadable_files,
                "format_warnings": self.format_warnings.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            },
            "events": {
                "read": ops(Op::Read),
                "edit": ops(Op::Edit),
                "shell": ops(Op::Shell),
                "search": ops(Op::Search),
                "failed": failed,
                "test_runs": self.events.iter().filter(|e| e.test_run).count(),
            },
            "files": {
                "changed": changes.len(),
                "before_and_after_in_log": both,
                "needs_base_replay": pending,
                "opaque": opaque,
                "after_failed": self.after_failed,
                "touched": self.touched_paths().iter().map(rel).collect::<Vec<_>>(),
                "read": self.read_paths().iter().map(rel).collect::<Vec<_>>(),
            },
            "reads_known": self.reads_known,
            "models": self.models,
            "tokens": {
                "input": self.input_tokens,
                "output": self.output_tokens,
                "cache_read": self.cache_read,
                "cache_write": self.cache_write,
            },
            "started": self.started,
            "ended": self.ended,
            "tool_calls_before_first_edit": self.tool_calls_before_first_edit,
            "tokens_before_first_edit": self.tokens_before_first_edit,
            "cache_read_before_first_edit": self.cache_read_before_first_edit,
            "has_first_prompt": self.first_prompt.is_some(),
        })
    }
}

/// 에이전트 설정 폴더. 테스트는 임시 폴더를 넘긴다
#[derive(Debug, Clone)]
pub struct Homes {
    pub claude: PathBuf,
    pub codex: PathBuf,
}

impl Homes {
    /// `<home>/.claude`, `<home>/.codex`
    pub fn under(home: &Path) -> Self {
        Self { claude: home.join(".claude"), codex: home.join(".codex") }
    }

    /// `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, 없으면 사용자 홈 아래
    pub fn detect() -> Self {
        let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let home = env("USERPROFILE").or_else(|| env("HOME")).unwrap_or_default();
        Self {
            claude: env("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude")),
            codex: env("CODEX_HOME").unwrap_or_else(|| home.join(".codex")),
        }
    }
}

/// 어느 경로의 이벤트를 남길지. Windows에서는 대소문자를 가리지 않는다.
pub struct Scope {
    /// 슬래시로 통일. 긴 것이 먼저라 저장소 안 작업 트리가 먼저 잡힌다
    roots: Vec<String>,
    everything: bool,
}

impl Scope {
    /// 거르지 않는다
    pub fn all() -> Self {
        Self { roots: Vec::new(), everything: true }
    }

    pub fn new(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut clean: Vec<String> = roots
            .into_iter()
            .map(|r| {
                let c = clean_path(&r.to_string_lossy());
                if is_absolute(&c) {
                    c
                } else {
                    clean_path(&std::path::absolute(&r).unwrap_or(r).to_string_lossy())
                }
            })
            .collect();
        clean.sort_by_key(|r| std::cmp::Reverse(r.len()));
        let mut seen = HashSet::new();
        clean.retain(|r| seen.insert(fold_case(r)));
        Self { roots: clean, everything: false }
    }

    /// 저장소 루트와 git이 아는 작업 트리들. git이 없거나 저장소가 아니면 루트만
    pub fn for_repo(root: &Path) -> Self {
        let mut roots = vec![root.to_path_buf()];
        if let Ok(list) = crate::git::git(root, &["worktree", "list", "--porcelain"]) {
            roots.extend(list.lines().filter_map(|l| l.strip_prefix("worktree ")).map(PathBuf::from));
        }
        Self::new(roots)
    }

    pub fn roots(&self) -> impl Iterator<Item = &str> {
        self.roots.iter().map(String::as_str)
    }

    /// 경로가 범위 안이면 그 루트 기준 상대 경로 (슬래시). 루트 자신은 빈 문자열
    pub fn relative(&self, path: &Path) -> Option<String> {
        let clean = clean_path(&path.to_string_lossy());
        let segs: Vec<&str> = clean.split('/').collect();
        self.roots.iter().find_map(|root| {
            let want: Vec<&str> = root.split('/').collect();
            let matched = segs.len() >= want.len() && want.iter().zip(&segs).all(|(a, b)| same_segment(a, b));
            matched.then(|| segs[want.len()..].join("/"))
        })
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.everything || self.relative(path).is_some()
    }
}

fn same_segment(a: &str, b: &str) -> bool {
    a == b || (cfg!(windows) && a.to_lowercase() == b.to_lowercase())
}

fn fold_case(s: &str) -> String {
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s.to_string()
    }
}

/// 슬래시로 통일하고 `.`, `..`를 접는다. 대소문자는 그대로 둔다
pub(crate) fn clean_path(raw: &str) -> String {
    let raw = match raw.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => raw.strip_prefix(r"\\?\").unwrap_or(raw).to_string(),
    };
    let mut s = raw.replace('\\', "/");
    // Git Bash 형태 `/c/Users/...`
    if cfg!(windows) {
        let b = s.as_bytes();
        if b.len() >= 2 && b[0] == b'/' && b[1].is_ascii_alphabetic() && (b.len() == 2 || b[2] == b'/') {
            s = format!("{}:{}", &s[1..2], &s[2..]);
        }
    }
    let (prefix, rest) = match s.strip_prefix("//") {
        Some(r) => ("//", r),
        None => match s.strip_prefix('/') {
            Some(r) => ("/", r),
            None => ("", s.as_str()),
        },
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in rest.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            x => parts.push(x),
        }
    }
    format!("{prefix}{}", parts.join("/"))
}

/// 경로를 비교할 때 쓰는 키
pub fn path_key(path: &Path) -> String {
    fold_case(&clean_path(&path.to_string_lossy()))
}

fn is_absolute(clean: &str) -> bool {
    let b = clean.as_bytes();
    clean.starts_with('/') || (b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic())
}

/// 로그에 적힌 경로를 절대 경로로. `cwd`가 없으면 상대 경로로 남는다 (범위 밖으로 걸러진다)
pub(crate) fn resolve_path(raw: &str, cwd: Option<&str>) -> PathBuf {
    let clean = clean_path(raw);
    match cwd {
        Some(dir) if !is_absolute(&clean) => PathBuf::from(clean_path(&format!("{dir}/{raw}"))),
        _ => PathBuf::from(clean),
    }
}

fn split_segments(cmd: &str) -> impl Iterator<Item = &str> {
    cmd.split(['\n', ';', '|', '&']).map(str::trim).filter(|s| !s.is_empty())
}

fn first_word(segment: &str) -> String {
    let word = segment.split_whitespace().next().unwrap_or("");
    word.trim_matches(['(', ')', '"', '\'', '`']).to_lowercase()
}

const NOT_A_RUN: &[&str] = &[
    "git", "gh", "grep", "rg", "egrep", "findstr", "echo", "printf", "cat", "type", "ls", "dir", "find", "sed", "awk", "head", "tail",
    "wc", "which", "where", "diff", "select-string", "get-content", "write-host", "write-output", "curl", "wget", "rm", "cp", "mv",
    "mkdir", "touch",
];

fn test_runner_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?ix)
              \bcargo(?:\.exe)?["']?(?:\s+\S+)*?\s+(?:test|nextest|tarpaulin|llvm-cov)(?:\s|$|["'])
            | \b(?:npm|pnpm|yarn|bun)(?:\.cmd|\.exe|\.ps1)?["']?\s+(?:(?:--filter|--prefix|--dir|--cwd|-C|-F|workspace)\s+\S+\s+|-\S+\s+)*(?:(?:run|run-script)\s+\S*(?:test|check|verify)\S*|test\b|t\b|check\b|verify\b)
            | \b(?:go|dotnet|swift|deno|flutter|dart|mix|rails|artisan|forge)(?:\.exe|\.bat|\.cmd)?["']?\s+test\b
            | \b(?:pytest|py\.test|unittest|nose2|tox|nox|vitest|jest|mocha|rspec|phpunit|ctest)\b
            | \bplaywright(?:\.cmd)?["']?\s+test\b
            | \bcypress(?:\.cmd)?["']?\s+run\b
            | \bnode(?:\.exe)?["']?\s+--test\b
            | \b(?:mvn|mvnw|gradle|gradlew)(?:\.cmd|\.bat)?["']?(?:\s+\S+)*?\s+(?:\S*:)?(?:(?:[^\s\-]\S*)?test\S*|check|verify|build|install|package)(?:\s|$|["'])
            | \b(?:make|just|rake)\s+(?:-\S+\s+)*\S*test\S*
            | \bmanage\.py["']?\s+(?:test|check)\b
            | \bzig\s+build\s+test\b
            | \bsbt\s+\S*test\S*
            | \brun[-_]tests?\b
            | \b(?:python\d*(?:\.\d+)?|py|node|bash|sh|pwsh|powershell)(?:\.exe)?\s+(?:-\S+\s+)*\S*test\S*\.(?:py|m?js|sh|ps1)\b
            | (?:^|[\s/\\])tests?\.(?:sh|ps1|bat|cmd)\b
            "#,
        )
        .expect("정규식")
    })
}

/// 테스트를 돌리는 명령인가. 일부러 넉넉하게 잡는다: 잘못 true면 "테스트 안 돌림" 지적이 줄 뿐이지만
/// 잘못 false면 없는 잘못을 지적하게 된다. `git commit -m "cargo test"`처럼 실행이 아닌 것만 뺀다.
pub fn is_test_command(cmd: &str) -> bool {
    split_segments(cmd).any(|seg| !NOT_A_RUN.contains(&first_word(seg).as_str()) && test_runner_re().is_match(seg))
}

/// git만 부르는 명령인가. checkout·pull로 바뀐 파일은 에이전트의 편집으로 치지 않는다
pub(crate) fn vcs_only(cmd: &str) -> bool {
    let cmd = cmd.replace("2>&1", "");
    let mut any_git = false;
    for seg in split_segments(&cmd) {
        match first_word(seg).as_str() {
            "git" | "gh" => any_git = true,
            "cd" | "pushd" | "popd" | "set-location" | "sl" | "echo" | "head" | "tail" | "cat" | "grep" => {}
            _ => return false,
        }
    }
    any_git
}

/// 명령 앞머리의 `cd 경로 &&`를 반영한 작업 폴더
pub(crate) fn shell_cwd(cmd: &str, cwd: Option<&str>) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r#"(?i)^\s*(?:cd|pushd|set-location|sl)\s+(?:/d\s+)?(?:-path\s+)?(?:"([^"]+)"|'([^']+)'|([^\s;&|]+))\s*(?:&&|;|\n|$)"#,
        )
        .expect("정규식")
    });
    let target = re.captures(cmd).and_then(|c| c.get(1).or(c.get(2)).or(c.get(3))).map(|m| m.as_str());
    match target {
        // 셸이 풀어 줘야 아는 경로는 모른다고 본다
        Some(t) if !t.starts_with(['~', '$', '%']) => Some(resolve_path(t, cwd).to_string_lossy().into_owned()),
        _ => cwd.map(str::to_string),
    }
}

/// 세션을 찾을 때의 조건
#[derive(Debug, Clone, Default)]
pub struct DiscoverOpts {
    /// Claude: 슬러그와 상관없이 모든 프로젝트 폴더를 본다
    pub deep: bool,
    /// 이보다 오래 전에 마지막으로 바뀐 세션은 뺀다
    pub since: Option<SystemTime>,
    /// 한 에이전트만
    pub agent: Option<Agent>,
}

/// 세션 목록 (오래된 순). 폴더가 없으면 비어 있다
pub fn discover(homes: &Homes, scope: &Scope, opts: &DiscoverOpts) -> Vec<SessionRef> {
    let mut out = Vec::new();
    if opts.agent != Some(Agent::Codex) {
        out.extend(claude::discover(&homes.claude, scope, opts));
    }
    if opts.agent != Some(Agent::ClaudeCode) {
        out.extend(codex::discover(&homes.codex, opts));
    }
    out.sort_by(|a, b| a.modified.cmp(&b.modified).then_with(|| a.id.cmp(&b.id)));
    out
}

/// 폴더 아래 모든 파일 (링크는 따라가지 않는다)
pub(crate) fn walk_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let Ok(kind) = e.file_type() else { continue };
        if kind.is_dir() {
            walk_files(&e.path(), out);
        } else if kind.is_file() {
            out.push(e.path());
        }
    }
}

pub(crate) fn modified_at(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// 이어 붙은 zstd 프레임까지 읽는다 (세션을 이어 쓰면 프레임이 늘어난다)
struct ZstFrames<R: Read> {
    dec: Option<StreamingDecoder<R, FrameDecoder>>,
}

impl<R: Read> Read for ZstFrames<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while let Some(dec) = self.dec.as_mut() {
            let n = dec.read(out)?;
            if n > 0 || out.is_empty() {
                return Ok(n);
            }
            self.dec = self.dec.take().and_then(|d| StreamingDecoder::new(d.into_inner()).ok());
        }
        Ok(0)
    }
}

/// 로그 파일을 스트림으로 연다. `.zst`는 풀면서 읽는다
pub(crate) fn open_log(path: &Path) -> io::Result<Box<dyn Read>> {
    let file = BufReader::new(File::open(path)?);
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zst")) {
        let dec = StreamingDecoder::new(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        return Ok(Box::new(ZstFrames { dec: Some(dec) }));
    }
    Ok(Box::new(file))
}

pub(crate) enum Line<'a> {
    Text(&'a [u8]),
    TooLong,
}

/// 줄 단위로 읽되 `MAX_LINE`을 넘는 줄은 메모리에 올리지 않는다.
/// 읽기 오류(잘린 압축 등)는 파일 끝으로 친다.
pub(crate) struct LineReader<R: Read> {
    r: BufReader<R>,
    buf: Vec<u8>,
    done: bool,
}

impl<R: Read> LineReader<R> {
    pub(crate) fn new(r: R) -> Self {
        Self { r: BufReader::with_capacity(1 << 20, r), buf: Vec::new(), done: false }
    }

    pub(crate) fn next_line(&mut self) -> Option<Line<'_>> {
        loop {
            if self.done {
                return None;
            }
            self.buf.clear();
            let mut too_long = false;
            loop {
                let chunk = match self.r.fill_buf() {
                    Ok(c) => c,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => &[],
                };
                if chunk.is_empty() {
                    self.done = true;
                    break;
                }
                let newline = chunk.iter().position(|&b| b == b'\n');
                let part = &chunk[..newline.unwrap_or(chunk.len())];
                if too_long || self.buf.len() + part.len() > MAX_LINE {
                    too_long = true;
                    self.buf.clear();
                } else {
                    self.buf.extend_from_slice(part);
                }
                let used = newline.map_or(chunk.len(), |p| p + 1);
                self.r.consume(used);
                if newline.is_some() {
                    break;
                }
            }
            if too_long {
                return Some(Line::TooLong);
            }
            if !self.buf.iter().all(u8::is_ascii_whitespace) {
                let text = self.buf.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&self.buf);
                return Some(Line::Text(text));
            }
        }
    }
}

/// 여러 세션을 읽는 동안 이미 본 줄과 메시지. 이어 하기로 복사된 기록을 두 번 세지 않는다
#[derive(Default)]
pub struct Seen {
    lines: HashSet<[u8; 16]>,
    messages: HashSet<[u8; 16]>,
}

fn short_hash(s: &str) -> [u8; 16] {
    let mut out = [0u8; 16];
    out.copy_from_slice(&blake3::hash(s.as_bytes()).as_bytes()[..16]);
    out
}

struct MsgUsage {
    ts: Option<String>,
    model: Option<String>,
    usage: Usage,
}

#[derive(Default)]
struct FileTrack {
    seen: bool,
    last_after: Option<usize>,
    pending: Vec<usize>,
    opaque: bool,
}

/// 파서 공용 상태
pub(crate) struct Collector<'a> {
    pub(crate) scope: &'a Scope,
    pub(crate) out: SessionSummary,
    seen: &'a mut Seen,
    tracks: HashMap<String, FileTrack>,
    usages: Vec<MsgUsage>,
    usage_index: HashMap<String, usize>,
    file_parsed: u64,
    file_unknown: u64,
}

impl<'a> Collector<'a> {
    fn new(session: &SessionRef, scope: &'a Scope, seen: &'a mut Seen) -> Self {
        let out = SessionSummary {
            agent: session.agent,
            id: session.id.clone(),
            events: Vec::new(),
            models: Vec::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read: 0,
            cache_write: 0,
            usage_by_model: BTreeMap::new(),
            first_prompt: None,
            started: None,
            ended: None,
            tool_calls_before_first_edit: None,
            tokens_before_first_edit: None,
            cache_read_before_first_edit: None,
            unknown_lines: 0,
            parsed_lines: 0,
            oversized_lines: 0,
            unreadable_files: 0,
            after_failed: 0,
            reads_known: session.agent == Agent::ClaudeCode,
            format_warnings: Vec::new(),
        };
        Self {
            scope,
            out,
            seen,
            tracks: HashMap::new(),
            usages: Vec::new(),
            usage_index: HashMap::new(),
            file_parsed: 0,
            file_unknown: 0,
        }
    }

    pub(crate) fn begin_file(&mut self) {
        self.tracks.clear();
        self.file_parsed = 0;
        self.file_unknown = 0;
    }

    pub(crate) fn end_file(&mut self, path: &Path) {
        if self.file_parsed < self.file_unknown {
            self.out.format_warnings.push(path.to_path_buf());
        }
    }

    pub(crate) fn parsed(&mut self) {
        self.out.parsed_lines += 1;
        self.file_parsed += 1;
    }

    pub(crate) fn unknown(&mut self) {
        self.out.unknown_lines += 1;
        self.file_unknown += 1;
    }

    /// 처음 보는 줄이면 true
    pub(crate) fn first_line(&mut self, uuid: &str) -> bool {
        self.seen.lines.insert(short_hash(uuid))
    }

    pub(crate) fn note_ts(&mut self, ts: &str) {
        if self.out.started.as_deref().is_none_or(|s| ts < s) {
            self.out.started = Some(ts.to_string());
        }
        if self.out.ended.as_deref().is_none_or(|s| ts > s) {
            self.out.ended = Some(ts.to_string());
        }
    }

    pub(crate) fn prompt(&mut self, text: &str) {
        let text = text.trim();
        // `<command-name>`, `<environment_context>`처럼 도구가 넣은 글은 질문이 아니다
        if self.out.first_prompt.is_none() && !text.is_empty() && !text.starts_with('<') {
            self.out.first_prompt = Some(text.to_string());
        }
    }

    pub(crate) fn model(&mut self, name: &str) {
        if !name.starts_with('<') && !self.out.models.iter().any(|m| m == name) {
            self.out.models.push(name.to_string());
        }
    }

    /// 사용량 하나. `id`가 같으면 합치지 않고 큰 값을 남긴다
    pub(crate) fn add_usage(&mut self, id: Option<&str>, ts: Option<&str>, model: Option<&str>, usage: Usage) {
        let Some(id) = id else {
            self.usages.push(MsgUsage { ts: ts.map(str::to_string), model: model.map(str::to_string), usage });
            return;
        };
        if self.seen.messages.contains(&short_hash(id)) {
            return;
        }
        match self.usage_index.get(id) {
            Some(&i) => self.usages[i].usage.merge_max(&usage),
            None => {
                self.usage_index.insert(id.to_string(), self.usages.len());
                self.usages.push(MsgUsage { ts: ts.map(str::to_string), model: model.map(str::to_string), usage });
            }
        }
    }

    pub(crate) fn push(&mut self, event: AgentEvent) -> usize {
        self.out.events.push(event);
        self.out.events.len() - 1
    }

    /// 결과까지 받은 편집을 반영한다: 앞 결과에 이어 붙이고, 더 필요 없는 본문을 버린다
    pub(crate) fn settle_edit(&mut self, i: usize) {
        track_edit(&mut self.tracks, &mut self.out.events, i, None);
    }

    fn finish(self) -> SessionSummary {
        let Self { mut out, seen, usages, usage_index, .. } = self;
        // 본 로그와 하위 에이전트 로그를 시각으로 합친다. 시각이 없으면 뒤로
        out.events.sort_by(|a, b| match (&a.ts, &b.ts) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
        let mut tracks = HashMap::new();
        let mut failed = 0;
        for i in 0..out.events.len() {
            if out.events[i].op == Op::Edit && out.events[i].edit.is_some() {
                track_edit(&mut tracks, &mut out.events, i, Some(&mut failed));
            }
        }
        out.after_failed += failed;

        for u in &usages {
            out.input_tokens += u.usage.input;
            out.output_tokens += u.usage.output;
            out.cache_read += u.usage.cache_read;
            out.cache_write += u.usage.cache_write;
            let model = u.model.clone().unwrap_or_else(|| "unknown".to_string());
            out.usage_by_model.entry(model).or_default().add(&u.usage);
        }
        if let Some(pos) = out.events.iter().position(|e| e.op == Op::Edit && e.ok == Some(true)) {
            out.tool_calls_before_first_edit = Some(pos as u32);
            if let Some(ts) = out.events[pos].ts.clone() {
                let mut sum = Usage::default();
                for u in usages.iter().filter(|u| u.ts.as_deref().is_some_and(|t| t <= ts.as_str())) {
                    sum.add(&u.usage);
                }
                out.tokens_before_first_edit = Some(sum.input + sum.cache_write + sum.output);
                out.cache_read_before_first_edit = Some(sum.cache_read);
            }
        }
        out.models.sort();
        for id in usage_index.keys() {
            seen.messages.insert(short_hash(id));
        }
        out
    }
}

/// `i`번째 편집을 파일별 추적에 반영한다. `failed`는 마지막 합치기에서만 센다
/// (파일 안에서 실패해도 다른 로그의 편집과 합치면 풀릴 수 있다).
fn track_edit(tracks: &mut HashMap<String, FileTrack>, events: &mut [AgentEvent], i: usize, failed: Option<&mut u32>) {
    let Some(key) = events[i].path.as_deref().map(path_key) else { return };
    let t = tracks.entry(key).or_default();
    // 앞 편집의 결과가 있으면 그 위에 적용한다. 로그에 본문이 없는 편집(하위 에이전트, Codex)을 살린다
    let chained = match (t.last_after, t.pending.is_empty() && !t.opaque, events[i].edit.as_ref()) {
        (Some(prev), true, Some(EditDetail { after: None, patch: Some(patch), .. })) => {
            let base = events[prev].edit.as_ref().and_then(|d| d.after.as_deref()).unwrap_or("");
            let result = apply_patch(base, patch);
            if result.is_none() {
                if let Some(f) = failed {
                    *f += 1;
                }
            }
            result
        }
        _ => None,
    };
    let first = !std::mem::replace(&mut t.seen, true);
    let Some(d) = events[i].edit.as_mut() else { return };
    if chained.is_some() {
        d.after = chained;
    }
    if !first {
        d.before = None;
    }
    let known = d.after.is_some();
    if !known && d.patch.is_some() {
        t.pending.push(i);
        return;
    }
    // 본문을 알았거나(앞은 더 필요 없다) 내용을 모르는 변경이다(앞은 쓸모없어졌다)
    d.patch = None;
    if let Some(prev) = t.last_after.take() {
        if let Some(x) = events[prev].edit.as_mut() {
            x.after = None;
        }
    }
    for j in t.pending.drain(..) {
        if let Some(x) = events[j].edit.as_mut() {
            x.patch = None;
        }
    }
    t.opaque = !known;
    if known {
        t.last_after = Some(i);
    }
}

/// 세션 하나를 읽는다. 내용 때문에 실패하지 않는다: 모르는 줄은 세고, 못 여는 파일도 센다
pub fn parse_session(session: &SessionRef, scope: &Scope, seen: &mut Seen) -> SessionSummary {
    let mut c = Collector::new(session, scope, seen);
    for f in &session.files {
        match session.agent {
            Agent::ClaudeCode => claude::parse_file(f, &mut c),
            Agent::Codex => codex::parse_file(f, &mut c),
        }
    }
    c.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replace(old: &str, new: &str, all: bool) -> Patch {
        Patch { replace: vec![Replace { old: old.into(), new: new.into(), all }], hunks: Vec::new() }
    }

    fn hunk(start: Option<u32>, lines: &[&str]) -> Hunk {
        Hunk { old_start: start, lines: lines.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn replace_respects_first_and_all() {
        assert_eq!(apply_patch("a a", &replace("a", "b", false)).unwrap(), "b a");
        assert_eq!(apply_patch("a a", &replace("a", "b", true)).unwrap(), "b b");
        assert!(apply_patch("zzz", &replace("a", "b", false)).is_none());
    }

    #[test]
    fn replace_handles_crlf_base_and_empty_old() {
        assert_eq!(apply_patch("x\r\ny\r\n", &replace("x\ny", "z", false)).unwrap(), "z\n");
        assert_eq!(apply_patch("", &replace("", "new", false)).unwrap(), "new");
        assert!(apply_patch("text", &replace("", "new", false)).is_none());
    }

    #[test]
    fn hunks_apply_by_line_number_then_by_content() {
        let text = "a\nb\nc\nd\n";
        let numbered = Patch { hunks: vec![hunk(Some(2), &[" b", "-c", "+C"])], ..Patch::default() };
        assert_eq!(apply_patch(text, &numbered).unwrap(), "a\nb\nC\nd\n");
        let shifted = Patch { hunks: vec![hunk(Some(99), &[" b", "-c", "+C"])], ..Patch::default() };
        assert_eq!(apply_patch(text, &shifted).unwrap(), "a\nb\nC\nd\n");
        let seek = Patch { hunks: vec![hunk(None, &["-a", "+A"]), hunk(None, &["-d", "+D", "+E"])], ..Patch::default() };
        assert_eq!(apply_patch(text, &seek).unwrap(), "A\nb\nc\nD\nE\n");
        let wrong = Patch { hunks: vec![hunk(Some(1), &["-nope", "+x"])], ..Patch::default() };
        assert!(apply_patch(text, &wrong).is_none());
    }

    #[test]
    fn hunks_tolerate_crlf_and_no_newline_marker() {
        let patch = Patch { hunks: vec![hunk(Some(1), &[" a", "-b", "+B", "\\ No newline at end of file"])], ..Patch::default() };
        assert_eq!(apply_patch("a\r\nb\r\n", &patch).unwrap(), "a\r\nB\n");
        let insert = Patch { hunks: vec![hunk(None, &["+x", "+y"])], ..Patch::default() };
        assert_eq!(apply_patch("", &insert).unwrap(), "x\ny");
        assert_eq!(apply_patch("a\n", &insert).unwrap(), "a\nx\ny\n");
    }

    #[test]
    fn scope_compares_segments_with_slashes_normalized() {
        let scope = Scope::new([PathBuf::from("/work/repo")]);
        assert_eq!(scope.relative(Path::new("/work/repo/src/a.rs")).as_deref(), Some("src/a.rs"));
        assert_eq!(scope.relative(Path::new(r"\work\repo\src\a.rs")).as_deref(), Some("src/a.rs"));
        assert_eq!(scope.relative(Path::new("/work/repo/x/../src/./a.rs")).as_deref(), Some("src/a.rs"));
        assert_eq!(scope.relative(Path::new("/work/repo")).as_deref(), Some(""));
        assert!(scope.relative(Path::new("/work/repo2/src/a.rs")).is_none());
        assert!(!scope.contains(Path::new("/work/other/a.rs")));
        assert!(Scope::all().contains(Path::new("/anything")));
    }

    #[test]
    fn scope_prefers_the_deepest_root() {
        let scope = Scope::new([PathBuf::from("/rr"), PathBuf::from("/rr/.claude/worktrees/w")]);
        assert_eq!(scope.relative(Path::new("/rr/.claude/worktrees/w/a.rs")).as_deref(), Some("a.rs"));
        assert_eq!(scope.relative(Path::new("/rr/b.rs")).as_deref(), Some("b.rs"));
    }

    #[cfg(windows)]
    #[test]
    fn scope_ignores_case_and_backslashes_on_windows() {
        let scope = Scope::new([PathBuf::from(r"C:\Users\Me\Repo")]);
        for p in [r"c:\users\me\repo\Src\A.rs", "C:/USERS/ME/REPO/Src/A.rs", r"\\?\C:\Users\Me\Repo\Src\A.rs", "/c/Users/me/repo/Src/A.rs"] {
            assert_eq!(scope.relative(Path::new(p)).as_deref(), Some("Src/A.rs"), "{p}");
        }
        assert!(scope.relative(Path::new(r"C:\Users\Me\Repo2\a.rs")).is_none());
        assert_eq!(path_key(Path::new(r"C:\A\b.rs")), path_key(Path::new("c:/a/B.RS")));
    }

    #[test]
    fn resolve_joins_relative_paths() {
        assert_eq!(resolve_path("src/a.rs", Some("/rr")), PathBuf::from("/rr/src/a.rs"));
        assert_eq!(resolve_path("/xx/a.rs", Some("/rr")), PathBuf::from("/xx/a.rs"));
        assert_eq!(resolve_path("C:\\x\\a.rs", Some("/rr")), PathBuf::from("C:/x/a.rs"));
        assert_eq!(resolve_path("../a.rs", Some("/rr/s")), PathBuf::from("/rr/a.rs"));
    }

    #[test]
    fn classifies_test_commands() {
        for c in [
            "cargo test",
            "cargo test -p lantern-context 2>&1 | tail -5",
            "cd x && cargo +nightly test",
            "cargo nextest run",
            "RUST_LOG=1 cargo test --workspace",
            "bash lowpri.sh \"cargo clippy; cargo test -p x\"",
            "npm test",
            "npm run test:unit",
            "pnpm -r test",
            "yarn workspace web test",
            "npx vitest run",
            "pytest -q tests/",
            "python -m pytest",
            "python -m unittest discover",
            "python tests/test_a.py",
            "go test ./...",
            "dotnet test",
            ".\\gradlew.bat check",
            "mvn -q verify",
            "make unit-test",
            "./scripts/run-tests.sh",
            "./test.sh",
            // 빌드가 테스트까지 돌리는 도구, 모듈 작업 이름, Windows 실행 파일
            "./gradlew build",
            "./gradlew :core:test",
            ".\\gradlew.bat :app:testDebugUnitTest --tests Foo",
            "\"C:\\tools\\gradle\\gradlew.bat\" build",
            "mvn package",
            "mvn -pl api install",
            "pnpm check",
            "pnpm -C web check",
            "npm run type-check",
            "npm.cmd test",
            "pnpm.cmd --filter web test",
            "flutter.bat test",
            "\"C:\\flutter\\bin\\flutter.bat\" test test/a_test.dart",
            "go.exe test ./...",
            "cargo.exe test -p a",
            "python manage.py test app.tests",
            "python.exe manage.py check",
        ] {
            assert!(is_test_command(c), "{c}");
        }
        for c in [
            "cargo build",
            "cargo clippy --all-targets",
            "cargo add test-case",
            "git commit -m \"cargo test passes\"",
            "git log --grep=pytest",
            "grep -rn pytest .",
            "echo cargo test",
            "cat tests/test_a.py",
            "ls tests",
            "npm install",
            "pnpm build",
            "npm run dev",
            "./gradlew assembleDebug",
            ".\\gradlew.bat :app:compileKotlin",
            "mvn compile -DskipTests",
            "flutter analyze",
            "flutter.bat pub get",
            "python manage.py migrate",
        ] {
            assert!(!is_test_command(c), "{c}");
        }
    }

    #[test]
    fn detects_git_only_commands() {
        assert!(vcs_only("git pull"));
        assert!(vcs_only("cd repo && git checkout main 2>&1 | tail -3"));
        assert!(!vcs_only("python patch.py && git add -A"));
        assert!(!vcs_only("sed -i s/a/b/ x.txt"));
        assert!(!vcs_only("cd repo"));
    }

    #[test]
    fn leading_cd_changes_the_working_folder() {
        assert_eq!(shell_cwd("cd /other/repo && cargo test", Some("/rr")).as_deref(), Some("/other/repo"));
        assert_eq!(shell_cwd("cd sub; ls", Some("/rr")).as_deref(), Some("/rr/sub"));
        assert_eq!(shell_cwd("cd \"/aa b/c\" && ls", Some("/rr")).as_deref(), Some("/aa b/c"));
        assert_eq!(shell_cwd("cd ~/x && ls", Some("/rr")).as_deref(), Some("/rr"));
        assert_eq!(shell_cwd("cargo test", Some("/rr")).as_deref(), Some("/rr"));
        assert_eq!(shell_cwd("cargo test", None), None);
    }

    #[test]
    fn line_reader_skips_long_lines_and_keeps_going() {
        let mut data = vec![b'x'; MAX_LINE + 10];
        data.extend_from_slice(b"\n\n  \nok\nlast");
        let mut r = LineReader::new(&data[..]);
        assert!(matches!(r.next_line(), Some(Line::TooLong)));
        assert!(matches!(r.next_line(), Some(Line::Text(b"ok"))));
        assert!(matches!(r.next_line(), Some(Line::Text(b"last"))));
        assert!(r.next_line().is_none());
        assert!(r.next_line().is_none());
    }

    #[test]
    fn debug_output_never_shows_file_text() {
        let d = EditDetail { before: Some("secret-before".into()), after: Some("secret-after".into()), patch: None };
        let shown = format!("{d:?}");
        assert!(!shown.contains("secret"), "{shown}");
    }
}
