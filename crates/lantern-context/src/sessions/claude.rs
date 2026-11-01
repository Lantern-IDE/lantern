//! Claude Code 세션 로그: `<설정 폴더>/projects/<슬러그>/<세션>.jsonl`과
//! `<세션>/subagents/**/*.jsonl`.
//!
//! 하위 에이전트 로그에는 `toolUseResult`가 없어 도구 입력(`old_string` 등)만 있다.
//! 본 로그도 `originalFile`이 비어 있는 편집이 흔해서, 그때는 변경만 들고 있다가 앞 결과나 기준 커밋 위에 적용한다.

use super::{
    apply_patch, is_test_command, modified_at, open_log, resolve_path, shell_cwd, vcs_only, walk_files, Agent, AgentEvent, Collector,
    DiscoverOpts, EditDetail, Hunk, Line, LineReader, Op, Patch, Replace, Scope, SessionRef, Usage,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// 쓰지는 않지만 형식을 아는 줄. 형식 변경 경고가 괜히 뜨지 않게 알아본 것으로 친다
const KNOWN_OTHER: &[&str] = &[
    "agent-name",
    "ai-title",
    "api-request",
    "api-request-blob",
    "api-request-shape",
    "artifact-autoreact-ledger",
    "artifact-comment-monitor",
    "atis-latch",
    "attachment",
    "bridge-session",
    "cost-state",
    "custom-title",
    "file-history-delta",
    "file-history-snapshot",
    "frame-link",
    "last-prompt",
    "mode",
    "permission-mode",
    "pr-link",
    "progress",
    "queue-operation",
    "summary",
    "system",
    "tag",
    "tools-used",
    "worktree-state",
    // subagents/workflows 아래 작업 기록
    "launched",
    "started",
    "result",
];

/// 작업 폴더를 프로젝트 폴더 이름으로: 영문·숫자가 아닌 글자마다 `-` (UTF-16 단위)
pub fn slug(path: &Path) -> String {
    let mut out = String::new();
    for c in path.to_string_lossy().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            out.extend(std::iter::repeat_n('-', c.len_utf16()));
        }
    }
    out
}

fn slug_matches(dir: &str, slugs: &[String]) -> bool {
    let dir = if cfg!(windows) { dir.to_lowercase() } else { dir.to_string() };
    slugs.iter().any(|s| dir.starts_with(s.as_str()))
}

pub(super) fn discover(home: &Path, scope: &Scope, opts: &DiscoverOpts) -> Vec<SessionRef> {
    let slugs: Vec<String> = scope
        .roots()
        .map(|r| slug(Path::new(r)))
        .map(|s| if cfg!(windows) { s.to_lowercase() } else { s })
        .collect();
    let Ok(dirs) = std::fs::read_dir(home.join("projects")) else { return Vec::new() };
    let mut sessions: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for dir in dirs.flatten() {
        let name = dir.file_name().to_string_lossy().into_owned();
        if !dir.path().is_dir() || !(opts.deep || slug_matches(&name, &slugs)) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(dir.path()) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_file() {
                if path.extension().is_some_and(|x| x == "jsonl") {
                    let id = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                    // 본 로그가 맨 앞
                    sessions.entry(id).or_default().insert(0, path);
                }
            } else if path.is_dir() {
                let mut logs = Vec::new();
                walk_files(&path.join("subagents"), &mut logs);
                logs.retain(|p| p.extension().is_some_and(|x| x == "jsonl"));
                logs.sort();
                if !logs.is_empty() {
                    sessions.entry(e.file_name().to_string_lossy().into_owned()).or_default().extend(logs);
                }
            }
        }
    }
    sessions
        .into_iter()
        .filter_map(|(id, files)| {
            let modified = files.iter().filter_map(|f| modified_at(f)).max().unwrap_or(SystemTime::UNIX_EPOCH);
            let fresh = opts.since.is_none_or(|t| modified >= t);
            fresh.then_some(SessionRef { agent: Agent::ClaudeCode, id, files, modified })
        })
        .collect()
}

/// 줄 종류만 빨리 본다. 대부분의 바이트는 쓰지 않는 줄에 있어서 전체를 트리로 풀지 않는다
#[derive(Deserialize)]
struct Head {
    #[serde(rename = "type", default)]
    kind: Option<String>,
}

/// 결과를 기다리는 도구 호출
struct Pending {
    event: Option<usize>,
    /// 셸이 바꾼 파일을 따져 볼 작업 폴더. 셸이 아니면 `None`
    shell_cwd: Option<String>,
    edits_count: bool,
}

pub(super) fn parse_file(path: &Path, c: &mut Collector<'_>) {
    let Ok(reader) = open_log(path) else {
        c.out.unreadable_files += 1;
        return;
    };
    c.begin_file();
    let mut pending: HashMap<String, Pending> = HashMap::new();
    let mut lines = LineReader::new(reader);
    while let Some(line) = lines.next_line() {
        match line {
            Line::TooLong => c.out.oversized_lines += 1,
            Line::Text(b) => on_line(b, &mut pending, c),
        }
    }
    // 결과가 없는 편집은 적용됐는지 모른다. 건드렸다고는 치되 본문은 믿지 않는다.
    // `None`이면 앞 편집의 본문이 끝 상태로 남으니, 내용을 모르는 변경으로 남겨 git에서 보게 한다
    for p in pending.values() {
        if let Some(i) = p.event {
            if c.out.events[i].op == Op::Edit {
                c.out.events[i].edit = Some(EditDetail::default());
            }
        }
    }
    c.end_file(path);
}

fn on_line(b: &[u8], pending: &mut HashMap<String, Pending>, c: &mut Collector<'_>) {
    let Ok(head) = serde_json::from_slice::<Head>(b) else {
        c.unknown();
        return;
    };
    let kind = head.kind.as_deref().unwrap_or("");
    if KNOWN_OTHER.contains(&kind) {
        c.parsed();
        return;
    }
    if kind != "user" && kind != "assistant" {
        c.unknown();
        return;
    }
    let Ok(mut v) = serde_json::from_slice::<Value>(b) else {
        c.unknown();
        return;
    };
    if !v.get("message").is_some_and(Value::is_object) {
        c.unknown();
        return;
    }
    c.parsed();
    if let Some(uuid) = v.get("uuid").and_then(Value::as_str) {
        if !c.first_line(uuid) {
            return;
        }
    }
    let ts = v.get("timestamp").and_then(Value::as_str).map(str::to_string);
    if let Some(t) = ts.as_deref() {
        c.note_ts(t);
    }
    let cwd = v.get("cwd").and_then(Value::as_str).map(str::to_string);
    if kind == "assistant" {
        on_assistant(&mut v, ts.as_deref(), cwd.as_deref(), pending, c);
    } else {
        on_user(&mut v, ts.as_deref(), cwd.as_deref(), pending, c);
    }
}

fn on_assistant(v: &mut Value, ts: Option<&str>, cwd: Option<&str>, pending: &mut HashMap<String, Pending>, c: &mut Collector<'_>) {
    let message = &mut v["message"];
    let model = message.get("model").and_then(Value::as_str).filter(|m| !m.starts_with('<')).map(str::to_string);
    if let Some(m) = model.as_deref() {
        c.model(m);
    }
    if let Some(u) = message.get("usage") {
        let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
        let usage = Usage {
            input: n("input_tokens"),
            output: n("output_tokens"),
            cache_read: n("cache_read_input_tokens"),
            cache_write: n("cache_creation_input_tokens"),
        };
        let id = message.get("id").and_then(Value::as_str);
        c.add_usage(id, ts, model.as_deref(), usage);
    }
    let Some(blocks) = message.get_mut("content").and_then(Value::as_array_mut) else { return };
    for b in blocks {
        if b.get("type").and_then(Value::as_str) == Some("tool_use") {
            on_tool_use(b, ts, cwd, pending, c);
        }
    }
}

fn take_str(o: &mut Value, key: &str) -> Option<String> {
    match o.get_mut(key).map(Value::take) {
        Some(Value::String(s)) => Some(s),
        _ => None,
    }
}

fn replace_from(o: &mut Value, old_key: &str, new_key: &str, all_key: &str) -> Option<Replace> {
    let all = o.get(all_key).and_then(Value::as_bool).unwrap_or(false);
    Some(Replace { old: take_str(o, old_key)?, new: take_str(o, new_key)?, all })
}

fn on_tool_use(b: &mut Value, ts: Option<&str>, cwd: Option<&str>, pending: &mut HashMap<String, Pending>, c: &mut Collector<'_>) {
    let Some(name) = b.get("name").and_then(Value::as_str).map(str::to_string) else { return };
    let Some(id) = b.get("id").and_then(Value::as_str).map(str::to_string) else { return };
    let mut empty = Value::Null;
    let input = b.get_mut("input").unwrap_or(&mut empty);
    let ts = ts.map(str::to_string);
    let file_arg = |input: &Value| {
        ["file_path", "notebook_path", "filePath"].iter().find_map(|k| input.get(*k).and_then(Value::as_str)).map(|p| resolve_path(p, cwd))
    };
    let event = match name.as_str() {
        "Read" | "NotebookRead" => file_arg(&*input).filter(|p| c.scope.contains(p)).map(|path| {
            let mut e = AgentEvent::new(Op::Read, ts, Some(path));
            e.range = requested_range(input);
            e
        }),
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => file_arg(&*input).filter(|p| c.scope.contains(p)).map(|path| {
            let mut e = AgentEvent::new(Op::Edit, ts, Some(path));
            e.edit = Some(edit_from_input(&name, input));
            e
        }),
        "Grep" | "Glob" | "LS" => {
            let dir = input.get("path").and_then(Value::as_str).map(|p| resolve_path(p, cwd)).or_else(|| cwd.map(|d| resolve_path(d, None)));
            dir.filter(|p| c.scope.contains(p)).map(|path| AgentEvent::new(Op::Search, ts, Some(path)))
        }
        "Bash" | "PowerShell" => {
            // 명령 글은 여기서 분류만 하고 버린다
            let cmd = input.get("command").and_then(Value::as_str).unwrap_or("");
            let dir = shell_cwd(cmd, cwd);
            let edits_count = !vcs_only(cmd);
            let test_run = is_test_command(cmd);
            let path = dir.as_deref().map(|d| resolve_path(d, None));
            let event = path.filter(|p| c.scope.contains(p)).map(|p| {
                let mut e = AgentEvent::new(Op::Shell, ts, Some(p));
                e.test_run = test_run;
                c.push(e)
            });
            pending.insert(id, Pending { event, shell_cwd: dir, edits_count });
            return;
        }
        _ => None,
    };
    if let Some(e) = event {
        let i = c.push(e);
        pending.insert(id, Pending { event: Some(i), shell_cwd: None, edits_count: false });
    }
}

fn requested_range(input: &Value) -> Option<(u32, u32)> {
    let n = |k: &str| input.get(k).and_then(Value::as_u64).map(|v| v.min(u32::MAX as u64) as u32);
    let (offset, limit) = (n("offset"), n("limit"));
    if offset.is_none() && limit.is_none() {
        return None;
    }
    let start = offset.unwrap_or(1).max(1);
    Some((start, limit.map_or(u32::MAX, |l| start.saturating_add(l.max(1) - 1))))
}

/// 도구 입력만으로 아는 것. 하위 에이전트 로그에는 이것밖에 없다
fn edit_from_input(tool: &str, input: &mut Value) -> EditDetail {
    let mut d = EditDetail::default();
    match tool {
        "Write" => d.after = take_str(input, "content"),
        "Edit" => {
            if let Some(r) = replace_from(input, "old_string", "new_string", "replace_all") {
                d.patch = Some(Patch { replace: vec![r], hunks: Vec::new() });
            }
        }
        "MultiEdit" => {
            let edits: Vec<Replace> = input
                .get_mut("edits")
                .and_then(Value::as_array_mut)
                .map(|list| list.iter_mut().filter_map(|e| replace_from(e, "old_string", "new_string", "replace_all")).collect())
                .unwrap_or_default();
            if !edits.is_empty() {
                d.patch = Some(Patch { replace: edits, hunks: Vec::new() });
            }
        }
        _ => {}
    }
    // 빈 old_string은 새 파일을 만든다
    let creating = d.patch.as_ref().is_some_and(|p| p.replace.first().is_some_and(|r| r.old.is_empty()));
    if creating {
        d.before = Some(String::new());
    }
    d
}

fn hunks_from(list: Option<&Value>) -> Vec<Hunk> {
    let Some(list) = list.and_then(Value::as_array) else { return Vec::new() };
    let mut out = Vec::with_capacity(list.len());
    for h in list {
        let (Some(start), Some(lines)) = (h.get("oldStart").and_then(Value::as_u64), h.get("lines").and_then(Value::as_array)) else {
            return Vec::new();
        };
        out.push(Hunk {
            old_start: Some(start.min(u32::MAX as u64) as u32),
            lines: lines.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        });
    }
    out
}

/// 도구 결과에 있는 본문·변경을 합치고 `after`를 만든다
fn merge_result(d: &mut EditDetail, tr: &mut Value, after_failed: &mut u32) {
    if tr.is_object() {
        if let Some(r) = replace_from(tr, "oldString", "newString", "replaceAll") {
            d.patch.get_or_insert_with(Patch::default).replace = vec![r];
        }
        let hunks = hunks_from(tr.get("structuredPatch"));
        if !hunks.is_empty() {
            d.patch.get_or_insert_with(Patch::default).hunks = hunks;
        }
        let created = tr.get("type").and_then(Value::as_str) == Some("create");
        match take_str(tr, "originalFile").or_else(|| take_str(tr, "originalFileContents")) {
            Some(original) => d.before = Some(original),
            None if created => d.before = Some(String::new()),
            None => {}
        }
        if let Some(content) = take_str(tr, "content") {
            d.after = Some(content);
        }
    }
    if d.after.is_some() {
        return;
    }
    if let (Some(before), Some(patch)) = (d.before.as_deref(), d.patch.as_ref()) {
        d.after = apply_patch(before, patch);
        if d.after.is_none() {
            *after_failed += 1;
        }
    }
}

fn on_user(v: &mut Value, ts: Option<&str>, cwd: Option<&str>, pending: &mut HashMap<String, Pending>, c: &mut Collector<'_>) {
    let flag = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
    let human = !flag("isMeta")
        && !flag("isSidechain")
        && !flag("isCompactSummary")
        && v.get("origin").and_then(|o| o.get("kind")).and_then(Value::as_str).is_none_or(|k| k == "human");
    if human && c.out.first_prompt.is_none() {
        match v["message"].get("content") {
            Some(Value::String(s)) => c.prompt(s),
            Some(Value::Array(blocks)) if !blocks.iter().any(|b| block_is(b, "tool_result")) => {
                for t in blocks.iter().filter(|b| block_is(b, "text")).filter_map(|b| b.get("text").and_then(Value::as_str)) {
                    c.prompt(t);
                }
            }
            _ => {}
        }
    }
    let results: Vec<(String, bool)> = v["message"]
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| block_is(b, "tool_result"))
                .filter_map(|b| {
                    let id = b.get("tool_use_id").and_then(Value::as_str)?;
                    Some((id.to_string(), b.get("is_error").and_then(Value::as_bool).unwrap_or(false)))
                })
                .collect()
        })
        .unwrap_or_default();
    // 구조화된 결과는 줄 하나에 하나뿐이다. 결과가 여럿인 줄에서는 누구 것인지 몰라 쓰지 않는다
    let mut structured = if results.len() == 1 { v.get_mut("toolUseResult").map(Value::take) } else { None };
    for (id, is_error) in results {
        let Some(p) = pending.remove(&id) else { continue };
        if let Some(i) = p.event {
            on_result(i, is_error, structured.as_mut(), c);
        }
        if let (Some(dir), true, Some(tr)) = (p.shell_cwd.as_deref().or(cwd), p.edits_count, structured.as_mut()) {
            on_shell_edits(tr, dir, ts, c);
        }
    }
}

fn block_is(b: &Value, kind: &str) -> bool {
    b.get("type").and_then(Value::as_str) == Some(kind)
}

fn on_result(i: usize, is_error: bool, tr: Option<&mut Value>, c: &mut Collector<'_>) {
    let op = c.out.events[i].op;
    c.out.events[i].ok = Some(!is_error);
    if op == Op::Read {
        if let Some(file) = tr.and_then(|t| t.get("file")) {
            let n = |k: &str| file.get(k).and_then(Value::as_u64);
            if let (Some(start), Some(lines), Some(total)) = (n("startLine"), n("numLines"), n("totalLines")) {
                let whole = start <= 1 && lines >= total;
                let end = start.saturating_add(lines.saturating_sub(1)).min(u32::MAX as u64) as u32;
                c.out.events[i].range = (!whole).then_some((start.min(u32::MAX as u64) as u32, end));
            }
        }
        return;
    }
    if op != Op::Edit {
        return;
    }
    if is_error {
        c.out.events[i].edit = None;
        return;
    }
    let mut failed = 0;
    let mut empty = Value::Null;
    if let Some(d) = c.out.events[i].edit.as_mut() {
        merge_result(d, tr.unwrap_or(&mut empty), &mut failed);
    }
    c.out.after_failed += failed;
    c.settle_edit(i);
}

/// 셸 명령이 바꾼 파일. `files`에 없고 `changedFiles`에만 있으면 내용을 모르는 변경이다.
/// 명령이 실패해도 파일은 바뀌었으므로 성공으로 친다.
fn on_shell_edits(tr: &mut Value, cwd: &str, ts: Option<&str>, c: &mut Collector<'_>) {
    let Some(diff) = tr.get("bashEditDiff") else { return };
    let mut done: HashSet<String> = HashSet::new();
    let mut emit = |raw: &str, detail: EditDetail, c: &mut Collector<'_>| {
        let path = resolve_path(raw, Some(cwd));
        if !c.scope.contains(&path) || !done.insert(super::path_key(&path)) {
            return;
        }
        let mut e = AgentEvent::new(Op::Edit, ts.map(str::to_string), Some(path));
        e.ok = Some(true);
        e.edit = Some(detail);
        let i = c.push(e);
        c.settle_edit(i);
    };
    for f in diff.get("files").and_then(Value::as_array).into_iter().flatten() {
        let Some(raw) = f.get("filePath").and_then(Value::as_str) else { continue };
        let flag = |k: &str| f.get(k).and_then(Value::as_bool).unwrap_or(false);
        let hunks = hunks_from(f.get("hunks"));
        let mut d = EditDetail::default();
        if !hunks.is_empty() {
            d.patch = Some(Patch { replace: Vec::new(), hunks });
        }
        if flag("created") {
            d.before = Some(String::new());
            d.after = d.patch.as_ref().and_then(|p| apply_patch("", p));
        }
        if flag("deleted") {
            d.after = Some(String::new());
        }
        emit(raw, d, c);
    }
    for raw in diff.get("changedFiles").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        emit(raw, EditDetail::default(), c);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{discover as find_sessions, parse_session, Homes, Seen, SessionSummary, MAX_LINE};
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::io::Write as _;

    const REPO: &str = "/work/repo";

    fn scope() -> Scope {
        Scope::new([PathBuf::from(REPO)])
    }

    fn write_log(path: &Path, lines: &[Value]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut f = fs::File::create(path).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
    }

    fn session(files: Vec<PathBuf>) -> SessionRef {
        SessionRef { agent: Agent::ClaudeCode, id: "s".into(), files, modified: SystemTime::UNIX_EPOCH }
    }

    /// 줄 목록을 로그 파일 하나로 읽는다
    fn run(lines: &[Value]) -> SessionSummary {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        write_log(&path, lines);
        parse_session(&session(vec![path]), &scope(), &mut Seen::default())
    }

    fn tool_use(n: u32, name: &str, input: Value) -> Value {
        json!({
            "type": "assistant",
            "uuid": format!("a{n}"),
            "timestamp": format!("2026-10-01T00:00:{n:02}.000Z"),
            "cwd": REPO,
            "message": {"id": format!("m{n}"), "model": "m-1", "content": [{"type": "tool_use", "id": format!("t{n}"), "name": name, "input": input}]},
        })
    }

    fn tool_result(n: u32, is_error: bool, structured: Option<Value>) -> Value {
        let mut line = json!({
            "type": "user",
            "uuid": format!("u{n}"),
            "timestamp": format!("2026-10-01T00:00:{n:02}.500Z"),
            "cwd": REPO,
            "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": format!("t{n}"), "is_error": is_error, "content": "private tool output"}]},
        });
        if let Some(s) = structured {
            line["toolUseResult"] = s;
        }
        line
    }

    fn after_of(s: &SessionSummary) -> Option<String> {
        s.events.iter().rev().find(|e| e.op == Op::Edit).and_then(|e| e.edit.as_ref()).and_then(|d| d.after.clone())
    }

    #[test]
    fn slug_replaces_every_non_alphanumeric() {
        assert_eq!(slug(Path::new(r"C:\Users\me\study\my.repo")), "C--Users-me-study-my-repo");
        assert_eq!(slug(Path::new("/a_b/c")), "-a-b-c");
        assert_eq!(slug(Path::new("/한글")), "---");
    }

    #[test]
    fn edit_applies_old_and_new_string_to_original() {
        let s = run(&[
            tool_use(1, "Edit", json!({"file_path": "/work/repo/a.rs", "old_string": "x", "new_string": "y"})),
            tool_result(
                1,
                false,
                Some(json!({"filePath": "/work/repo/a.rs", "originalFile": "fn x() {}\nfn x2() {}\n", "oldString": "fn x()", "newString": "fn y()", "replaceAll": false})),
            ),
        ]);
        assert_eq!(s.events.len(), 1);
        let e = &s.events[0];
        assert_eq!((e.op, e.ok), (Op::Edit, Some(true)));
        let d = e.edit.as_ref().unwrap();
        assert_eq!(d.before.as_deref(), Some("fn x() {}\nfn x2() {}\n"));
        assert_eq!(d.after.as_deref(), Some("fn y() {}\nfn x2() {}\n"));
        assert_eq!(s.after_failed, 0);
    }

    #[test]
    fn edit_respects_replace_all() {
        let s = run(&[
            tool_use(1, "Edit", json!({"file_path": "/work/repo/a.rs", "old_string": "a", "new_string": "b", "replace_all": true})),
            tool_result(1, false, Some(json!({"originalFile": "a a a", "oldString": "a", "newString": "b", "replaceAll": true}))),
        ]);
        assert_eq!(after_of(&s).as_deref(), Some("b b b"));
    }

    #[test]
    fn write_uses_content_and_marks_new_files() {
        let s = run(&[
            tool_use(1, "Write", json!({"file_path": "/work/repo/new.rs", "content": "input text"})),
            tool_result(1, false, Some(json!({"type": "create", "filePath": "/work/repo/new.rs", "content": "fn new() {}\n", "originalFile": null}))),
            tool_use(2, "Write", json!({"file_path": "/work/repo/old.rs", "content": "fn b() {}\n"})),
            tool_result(2, false, Some(json!({"type": "update", "filePath": "/work/repo/old.rs", "content": "fn b() {}\n", "originalFile": "fn a() {}\n"}))),
        ]);
        let new = s.events[0].edit.as_ref().unwrap();
        assert_eq!((new.before.as_deref(), new.after.as_deref()), (Some(""), Some("fn new() {}\n")));
        let old = s.events[1].edit.as_ref().unwrap();
        assert_eq!((old.before.as_deref(), old.after.as_deref()), (Some("fn a() {}\n"), Some("fn b() {}\n")));
    }

    #[test]
    fn structured_patch_is_the_fallback() {
        let patch = json!([{"oldStart": 2, "oldLines": 2, "newStart": 2, "newLines": 2, "lines": [" b", "-c", "+C"]}]);
        // 문자열이 본문에 없을 때
        let s = run(&[
            tool_use(1, "Edit", json!({"file_path": "/work/repo/a.rs", "old_string": "gone", "new_string": "z"})),
            tool_result(1, false, Some(json!({"originalFile": "a\nb\nc\nd\n", "oldString": "gone", "newString": "z", "structuredPatch": patch}))),
        ]);
        assert_eq!(after_of(&s).as_deref(), Some("a\nb\nC\nd\n"));
        assert_eq!(s.after_failed, 0);
        // 문자열이 아예 없을 때
        let s = run(&[
            tool_use(1, "NotebookEdit", json!({"file_path": "/work/repo/a.rs"})),
            tool_result(1, false, Some(json!({"originalFile": "a\nb\nc\nd\n", "structuredPatch": patch}))),
        ]);
        assert_eq!(after_of(&s).as_deref(), Some("a\nb\nC\nd\n"));
    }

    #[test]
    fn after_is_none_and_counted_when_nothing_applies() {
        let s = run(&[
            tool_use(1, "Edit", json!({"file_path": "/work/repo/a.rs", "old_string": "gone", "new_string": "z"})),
            tool_result(
                1,
                false,
                Some(json!({"originalFile": "a\n", "oldString": "gone", "newString": "z", "structuredPatch": [{"oldStart": 1, "lines": ["-nope", "+z"]}]})),
            ),
        ]);
        let d = s.events[0].edit.as_ref().unwrap();
        assert_eq!(d.after, None);
        assert_eq!(s.after_failed, 1);
        assert_eq!(s.events[0].ok, Some(true));
    }

    #[test]
    fn tool_errors_are_not_edits() {
        let s = run(&[
            tool_use(1, "Edit", json!({"file_path": "/work/repo/a.rs", "old_string": "x", "new_string": "y"})),
            tool_result(1, true, Some(json!("Error: string not found"))),
            tool_use(2, "Read", json!({"file_path": "/work/repo/gone.rs"})),
            tool_result(2, true, None),
        ]);
        let e = &s.events[0];
        assert_eq!((e.ok, e.edit.is_none()), (Some(false), true));
        assert!(s.touched_paths().is_empty());
        assert!(s.read_paths().is_empty());
        assert_eq!(s.tool_calls_before_first_edit, None);
    }

    #[test]
    fn edit_without_result_keeps_no_text() {
        let s = run(&[tool_use(1, "Write", json!({"file_path": "/work/repo/a.rs", "content": "fn a() {}"}))]);
        let d = s.events[0].edit.as_ref().unwrap();
        assert_eq!((s.events[0].ok, d.before.is_none(), d.after.is_none(), d.patch.is_none()), (None, true, true, true));
        assert_eq!(s.touched_paths().len(), 1);
        assert!(s.file_changes()[0].opaque);
    }

    #[test]
    fn lost_result_does_not_leave_the_earlier_text_as_the_end() {
        let s = run(&[
            tool_use(1, "Write", json!({"file_path": "/work/repo/a.rs", "content": "fn second() {}\n"})),
            tool_result(1, false, Some(json!({"type": "update", "filePath": "/work/repo/a.rs", "content": "fn second() {}\n", "originalFile": "fn first() {}\n"}))),
            // 결과 줄이 잘렸다
            tool_use(2, "Write", json!({"file_path": "/work/repo/a.rs", "content": "fn third() {}\n"})),
        ]);
        let changes = s.file_changes();
        let fc = &changes[0];
        assert_eq!((fc.before, fc.after, fc.opaque, fc.edits), (Some("fn first() {}\n"), None, true, 2));
        assert_eq!(fc.resolve(None), None, "끝 상태는 git에서 봐야 한다");

        // 그 뒤에 온전한 편집이 있으면 다시 안다
        let s = run(&[
            tool_use(1, "Write", json!({"file_path": "/work/repo/a.rs", "content": "lost\n"})),
            tool_use(2, "Write", json!({"file_path": "/work/repo/a.rs", "content": "known\n"})),
            tool_result(2, false, Some(json!({"type": "update", "filePath": "/work/repo/a.rs", "content": "known\n", "originalFile": "lost\n"}))),
        ]);
        let changes = s.file_changes();
        assert_eq!((changes[0].after, changes[0].opaque), (Some("known\n"), false));
    }

    #[test]
    fn hunk_with_an_unknown_prefix_is_counted_not_fatal() {
        for (original, lines) in [("", vec!["zzz"]), ("a\nb", vec![" a", "-b", "+c", "EOF"])] {
            let s = run(&[
                tool_use(1, "Edit", json!({"file_path": "/work/repo/a.rs", "old_string": "gone", "new_string": "z"})),
                tool_result(1, false, Some(json!({"originalFile": original, "oldString": "gone", "newString": "z", "structuredPatch": [{"oldStart": 1, "lines": lines}]}))),
            ]);
            assert_eq!(s.events[0].edit.as_ref().unwrap().after, None);
            assert_eq!(s.after_failed, 1);
        }
    }

    #[test]
    fn doubling_replace_all_stops_at_the_line_limit() {
        let path = "/work/repo/a.rs";
        let mut lines = vec![
            tool_use(1, "Write", json!({"file_path": path, "content": "x"})),
            tool_result(1, false, Some(json!({"type": "update", "filePath": path, "content": "x", "originalFile": ""}))),
        ];
        // 한 번에 배가 되는 치환 28번. 끝까지 하면 268MB다. 이 시험이 망가져도 PC가 버티게 글자 하나에서 시작한다
        for n in 2..30 {
            lines.push(tool_use(n, "Edit", json!({"file_path": path, "old_string": "x", "new_string": "xx", "replace_all": true})));
            lines.push(tool_result(n, false, Some(json!({"filePath": path, "oldString": "x", "newString": "xx", "originalFile": null, "replaceAll": true}))));
        }
        let s = run(&lines);
        let longest = s.events.iter().filter_map(|e| e.edit.as_ref()?.after.as_ref()).map(String::len).max().unwrap_or(0);
        assert!(longest <= MAX_LINE, "{longest}");
        assert!(s.after_failed > 0);
    }

    #[test]
    fn subagent_folder_is_found_and_merged_in_time_order() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("code").join("my.repo");
        let scope = Scope::new([root.clone()]);
        let root_str = root.to_string_lossy().replace('\\', "/");
        let file = format!("{root_str}/a.rs");
        let project = home.path().join(".claude/projects").join(slug(&root));
        let line = |n: u32, name: &str, input: Value| {
            let mut l = tool_use(n, name, input);
            l["cwd"] = json!(root_str);
            l
        };
        write_log(
            &project.join("sid.jsonl"),
            &[
                json!({"type": "user", "uuid": "p", "timestamp": "2026-10-01T00:00:00.000Z", "message": {"content": "fix the thing"}}),
                line(1, "Write", json!({"file_path": file, "content": "one\n"})),
                tool_result(1, false, Some(json!({"type": "create", "content": "one\n", "originalFile": null}))),
                line(5, "Edit", json!({"file_path": file, "old_string": "two", "new_string": "three"})),
                tool_result(5, false, Some(json!({"originalFile": null, "oldString": "two", "newString": "three"}))),
            ],
        );
        // 하위 에이전트 로그에는 toolUseResult가 없다
        write_log(
            &project.join("sid/subagents/agent-a.jsonl"),
            &[
                json!({"type": "user", "uuid": "sp", "isSidechain": true, "timestamp": "2026-10-01T00:00:02.000Z", "message": {"content": "subagent task"}}),
                line(3, "Edit", json!({"file_path": file, "old_string": "one", "new_string": "two"})),
                tool_result(3, false, None),
            ],
        );
        write_log(&project.join("sid/subagents/workflows/wf/agent-b.jsonl"), &[line(4, "Read", json!({"file_path": file}))]);
        fs::write(project.join("sid/subagents/agent-a.meta.json"), "{}").unwrap();
        // 슬러그가 다른 프로젝트는 기본으로 보지 않는다
        write_log(&home.path().join(".claude/projects/other-project/x.jsonl"), &[]);

        let homes = Homes::under(home.path());
        let found = find_sessions(&homes, &scope, &DiscoverOpts::default());
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].id.as_str(), found[0].files.len()), ("sid", 3));
        assert!(found[0].files[0].ends_with("sid.jsonl"));
        assert_eq!(find_sessions(&homes, &scope, &DiscoverOpts { deep: true, ..Default::default() }).len(), 2);

        let s = parse_session(&found[0], &scope, &mut Seen::default());
        assert_eq!(s.first_prompt.as_deref(), Some("fix the thing"));
        assert_eq!(s.events.iter().map(|e| e.op).collect::<Vec<_>>(), [Op::Edit, Op::Edit, Op::Read, Op::Edit]);
        let changes = s.file_changes();
        assert_eq!(changes.len(), 1);
        // 본 로그(1) → 하위 에이전트(3) → 본 로그(5)가 시각 순으로 이어진다
        assert_eq!(changes[0].edits, 3);
        assert_eq!(changes[0].before, Some(""));
        assert_eq!(changes[0].after, Some("three\n"));
        assert!(changes[0].pending.is_empty());
    }

    #[test]
    fn usage_is_deduped_by_message_id_and_lines_by_uuid() {
        let usage = |output: u64| json!({"input_tokens": 10, "output_tokens": output, "cache_read_input_tokens": 100, "cache_creation_input_tokens": 5});
        let msg = |uuid: &str, id: &str, u: Value| json!({"type": "assistant", "uuid": uuid, "timestamp": "2026-10-01T00:00:01.000Z", "message": {"id": id, "model": "m-1", "usage": u}});
        let s = run(&[
            msg("a", "m1", usage(1)),
            // 같은 메시지가 블록마다 줄을 만든다. 뒤로 갈수록 출력이 커진다
            msg("b", "m1", usage(7)),
            msg("c", "m2", usage(3)),
            // 같은 uuid는 다시 세지 않는다
            msg("c", "m3", usage(100)),
            json!({"type": "assistant", "uuid": "d", "message": {"id": "m9", "model": "<synthetic>", "usage": usage(2)}}),
        ]);
        assert_eq!(s.parsed_lines, 5);
        assert_eq!(s.input_tokens, 30);
        assert_eq!(s.output_tokens, 12);
        assert_eq!(s.cache_read, 300);
        assert_eq!(s.cache_write, 15);
        assert_eq!(s.models, ["m-1"]);
        assert_eq!(s.usage_by_model["m-1"].output, 10);
        assert_eq!(s.usage_by_model["unknown"].output, 2);
    }

    #[test]
    fn repeated_lines_across_sessions_count_once() {
        let dir = tempfile::tempdir().unwrap();
        let lines = [
            tool_use(1, "Read", json!({"file_path": "/work/repo/a.rs"})),
            json!({"type": "assistant", "uuid": "z1", "message": {"id": "mz", "usage": {"output_tokens": 4}}}),
        ];
        let (a, b) = (dir.path().join("a.jsonl"), dir.path().join("b.jsonl"));
        write_log(&a, &lines);
        write_log(&b, &lines);
        let mut seen = Seen::default();
        let first = parse_session(&session(vec![a]), &scope(), &mut seen);
        let second = parse_session(&session(vec![b]), &scope(), &mut seen);
        assert_eq!((first.events.len(), first.output_tokens), (1, 4));
        assert_eq!((second.events.len(), second.output_tokens), (0, 0));
    }

    #[test]
    fn truncated_last_line_and_unknown_types_are_counted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        write_log(
            &path,
            &[
                tool_use(1, "Read", json!({"file_path": "/work/repo/a.rs"})),
                tool_result(1, false, None),
                json!({"type": "summary", "summary": "x"}),
                json!({"type": "brand-new-thing", "x": 1}),
                json!({"type": "assistant", "message": "not an object"}),
                json!([1, 2]),
            ],
        );
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(br#"{"type":"assistant","message":{"id":"m"#).unwrap();
        drop(f);
        let s = parse_session(&session(vec![path]), &scope(), &mut Seen::default());
        assert_eq!(s.events.len(), 1);
        assert_eq!(s.parsed_lines, 3);
        assert_eq!(s.unknown_lines, 4);
        // 4 > 3: 절반 아래면 형식이 바뀌었을 수 있다고 알린다
        assert_eq!(s.format_warnings.len(), 1);
    }

    #[test]
    fn mostly_known_lines_raise_no_format_warning() {
        let s = run(&[json!({"type": "summary"}), json!({"type": "attachment"}), json!({"type": "brand-new-thing"})]);
        assert_eq!((s.parsed_lines, s.unknown_lines), (2, 1));
        assert!(s.format_warnings.is_empty());
    }

    #[test]
    fn missing_file_is_counted_not_fatal() {
        let s = parse_session(&session(vec![PathBuf::from("/no/such/file.jsonl")]), &scope(), &mut Seen::default());
        assert_eq!((s.unreadable_files, s.events.len()), (1, 0));
    }

    #[test]
    fn oversized_line_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        let huge = json!({"type": "user", "message": {"content": "x".repeat(MAX_LINE)}});
        writeln!(f, "{huge}").unwrap();
        writeln!(f, "{}", tool_use(1, "Read", json!({"file_path": "/work/repo/a.rs"}))).unwrap();
        drop(f);
        let s = parse_session(&session(vec![path]), &scope(), &mut Seen::default());
        assert_eq!((s.oversized_lines, s.parsed_lines, s.events.len()), (1, 1, 1));
        assert_eq!(s.first_prompt, None);
    }

    #[test]
    fn events_are_filtered_by_path_not_by_session_cwd() {
        let s = run(&[
            tool_use(1, "Write", json!({"file_path": "/elsewhere/b.rs", "content": "private"})),
            tool_result(1, false, Some(json!({"type": "create", "content": "private"}))),
            tool_use(2, "Read", json!({"file_path": "/work/repo2/c.rs"})),
            tool_use(3, "Read", json!({"file_path": "src/rel.rs"})),
            tool_use(4, "Grep", json!({"pattern": "secret", "path": "/elsewhere"})),
            tool_use(5, "Glob", json!({"pattern": "**/*.rs"})),
        ]);
        let ops: Vec<Op> = s.events.iter().map(|e| e.op).collect();
        assert_eq!(ops, [Op::Read, Op::Search]);
        assert_eq!(s.events[0].path.as_deref(), Some(Path::new("/work/repo/src/rel.rs")));
        assert!(s.touched_paths().is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn path_filter_ignores_case_and_backslashes_on_windows() {
        let scope = Scope::new([PathBuf::from(r"C:\Users\Me\Repo")]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        write_log(
            &path,
            &[
                tool_use(1, "Write", json!({"file_path": r"c:\users\me\REPO\src\A.rs", "content": "one"})),
                tool_result(1, false, None),
                tool_use(2, "Edit", json!({"file_path": "C:/Users/Me/Repo/src/a.RS", "old_string": "one", "new_string": "two"})),
                tool_result(2, false, None),
                tool_use(3, "Read", json!({"file_path": r"C:\Users\Me\Repo2\a.rs"})),
            ],
        );
        let s = parse_session(&session(vec![path]), &scope, &mut Seen::default());
        assert_eq!(s.events.len(), 2);
        assert_eq!(s.touched_paths().len(), 1);
        let changes = s.file_changes();
        assert_eq!((changes.len(), changes[0].after), (1, Some("two")));
        assert_eq!(scope.relative(changes[0].path).as_deref(), Some("src/A.rs"));
    }

    #[cfg(windows)]
    #[test]
    fn discovery_ignores_drive_letter_case_on_windows() {
        let home = tempfile::tempdir().unwrap();
        write_log(&home.path().join(".claude/projects/c--Users-Me-Repo/lower.jsonl"), &[]);
        write_log(&home.path().join(".claude/projects/C--Users-Me-Repo-sub/upper.jsonl"), &[]);
        let scope = Scope::new([PathBuf::from(r"C:\Users\Me\Repo")]);
        let found = find_sessions(&Homes::under(home.path()), &scope, &DiscoverOpts::default());
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn keeps_only_first_before_and_last_after() {
        let s = run(&[
            tool_use(1, "Edit", json!({"file_path": "/work/repo/a.rs", "old_string": "1", "new_string": "2"})),
            tool_result(1, false, Some(json!({"originalFile": "1", "oldString": "1", "newString": "2"}))),
            tool_use(2, "Edit", json!({"file_path": "/work/repo/a.rs", "old_string": "2", "new_string": "3"})),
            tool_result(2, false, Some(json!({"originalFile": "2", "oldString": "2", "newString": "3"}))),
            tool_use(3, "Edit", json!({"file_path": "/work/repo/a.rs", "old_string": "3", "new_string": "4"})),
            tool_result(3, false, Some(json!({"originalFile": "3", "oldString": "3", "newString": "4"}))),
        ]);
        let texts: Vec<(Option<&str>, Option<&str>)> =
            s.events.iter().map(|e| e.edit.as_ref().unwrap()).map(|d| (d.before.as_deref(), d.after.as_deref())).collect();
        assert_eq!(texts, [(Some("1"), None), (None, None), (None, Some("4"))]);
        let changes = s.file_changes();
        assert_eq!((changes[0].before, changes[0].after, changes[0].edits), (Some("1"), Some("4"), 3));
        assert_eq!(changes[0].resolve(None).unwrap(), ("1".to_string(), "4".to_string()));
        assert_eq!(changes[0].first_ts, Some("2026-10-01T00:00:01.000Z"));
        assert_eq!(changes[0].last_ts, Some("2026-10-01T00:00:03.000Z"));
    }

    #[test]
    fn edits_without_text_replay_on_a_base_blob() {
        // originalFile이 비어 있는 편집 (실제 로그에서 흔하다)
        let s = run(&[
            tool_use(1, "Edit", json!({"file_path": "/work/repo/a.rs", "old_string": "one", "new_string": "two"})),
            tool_result(1, false, Some(json!({"originalFile": null, "oldString": "one", "newString": "two", "structuredPatch": []}))),
            tool_use(2, "Edit", json!({"file_path": "/work/repo/a.rs", "old_string": "two", "new_string": "three"})),
            tool_result(2, false, Some(json!({"originalFile": null, "oldString": "two", "newString": "three"}))),
        ]);
        let changes = s.file_changes();
        assert_eq!((changes[0].before, changes[0].after, changes[0].pending.len()), (None, None, 2));
        assert_eq!(s.after_failed, 0);
        assert_eq!(changes[0].resolve(None), None);
        assert_eq!(changes[0].resolve(Some("one\r\nrest\r\n")).unwrap().1, "three\r\nrest\r\n");
        assert_eq!(changes[0].resolve(Some("unrelated")), None);
    }

    #[test]
    fn shell_commands_are_classified_and_dropped() {
        let s = run(&[
            tool_use(1, "Bash", json!({"command": "cargo test -p secret-crate-name"})),
            tool_result(1, true, Some(json!({"stdout": "private output"}))),
            tool_use(2, "PowerShell", json!({"command": "git log --grep=pytest"})),
            tool_result(2, false, None),
            tool_use(3, "Bash", json!({"command": "cd /elsewhere && cargo test"})),
            tool_use(4, "Bash", json!({"command": "cd /work/repo/sub && npm test"})),
        ]);
        assert_eq!(s.events.iter().map(|e| e.test_run).collect::<Vec<_>>(), [true, false, true]);
        assert_eq!(s.events[0].ok, Some(false));
        assert_eq!(s.events[2].path.as_deref(), Some(Path::new("/work/repo/sub")));
    }

    #[test]
    fn shell_edits_count_as_touched_with_what_is_known() {
        let hunks = json!([{"oldStart": 1, "oldLines": 1, "newStart": 1, "newLines": 1, "lines": ["-a", "+b"]}]);
        let s = run(&[
            tool_use(1, "Bash", json!({"command": "python patch.py"})),
            tool_result(
                1,
                false,
                Some(json!({"stdout": "", "bashEditDiff": {
                    "files": [
                        {"filePath": "/work/repo/edited.rs", "hunks": hunks},
                        {"filePath": "/work/repo/made.rs", "created": true, "hunks": [{"oldStart": 1, "lines": ["+x"]}]},
                        {"filePath": "/work/repo/gone.rs", "deleted": true, "hunks": []},
                        {"filePath": "/elsewhere/z.rs", "hunks": hunks},
                    ],
                    "moreFiles": 1,
                    "changedFiles": ["/work/repo/edited.rs", "/work/repo/more.rs", "/elsewhere/y.rs"],
                }})),
            ),
            // git이 바꾼 파일은 에이전트의 편집이 아니다
            tool_use(2, "Bash", json!({"command": "git pull"})),
            tool_result(2, false, Some(json!({"bashEditDiff": {"files": [], "moreFiles": 0, "changedFiles": ["/work/repo/pulled.rs"]}}))),
        ]);
        let touched: Vec<_> = s.touched_paths().iter().map(|p| p.display().to_string()).collect();
        assert_eq!(touched, ["/work/repo/edited.rs", "/work/repo/made.rs", "/work/repo/gone.rs", "/work/repo/more.rs"]);
        let by_name = |n: &str| s.file_changes().into_iter().find(|c| c.path.ends_with(n)).unwrap().resolve(Some("a\n"));
        assert_eq!(by_name("edited.rs").unwrap().1, "b\n");
        assert_eq!(by_name("made.rs").unwrap(), (String::new(), "x".to_string()));
        assert_eq!(by_name("gone.rs").unwrap().1, "");
        assert_eq!(by_name("more.rs"), None);
    }

    #[test]
    fn read_ranges_come_from_input_or_result() {
        let s = run(&[
            tool_use(1, "Read", json!({"file_path": "/work/repo/a.rs", "offset": 10, "limit": 5})),
            tool_use(2, "Read", json!({"file_path": "/work/repo/b.rs", "offset": 10, "limit": 5})),
            tool_result(2, false, Some(json!({"type": "text", "file": {"startLine": 10, "numLines": 3, "totalLines": 12, "content": "x"}}))),
            tool_use(3, "Read", json!({"file_path": "/work/repo/c.rs", "limit": 500})),
            tool_result(3, false, Some(json!({"type": "text", "file": {"startLine": 1, "numLines": 12, "totalLines": 12, "content": "x"}}))),
            tool_use(4, "Read", json!({"file_path": "/work/repo/d.rs"})),
        ]);
        assert_eq!(s.events.iter().map(|e| e.range).collect::<Vec<_>>(), [Some((10, 14)), Some((10, 12)), None, None]);
    }

    #[test]
    fn first_prompt_skips_injected_and_meta_lines() {
        let user = |extra: Value, content: Value| {
            let mut l = json!({"type": "user", "message": {"role": "user", "content": content}});
            for (k, v) in extra.as_object().unwrap() {
                l[k] = v.clone();
            }
            l
        };
        let s = run(&[
            user(json!({"isMeta": true}), json!("meta text")),
            user(json!({}), json!("<command-name>/clear</command-name>")),
            user(json!({"origin": {"kind": "task-notification"}}), json!("notification")),
            user(json!({"isCompactSummary": true}), json!("summary")),
            user(json!({}), json!([{"type": "text", "text": "<ide_opened_file>x</ide_opened_file>"}, {"type": "text", "text": " real task "}])),
            user(json!({}), json!("second task")),
        ]);
        assert_eq!(s.first_prompt.as_deref(), Some("real task"));
    }

    #[test]
    fn search_cost_counts_up_to_the_first_edit() {
        let usage = |n: u32, output: u64| {
            json!({"type": "assistant", "uuid": format!("x{n}"), "timestamp": format!("2026-10-01T00:00:{n:02}.000Z"),
                "message": {"id": format!("x{n}"), "model": "m-1", "usage": {"input_tokens": 1, "output_tokens": output, "cache_read_input_tokens": 50}}})
        };
        let s = run(&[
            usage(1, 10),
            tool_use(2, "Read", json!({"file_path": "/work/repo/a.rs"})),
            tool_result(2, false, None),
            tool_use(3, "Grep", json!({"pattern": "x"})),
            tool_result(3, false, None),
            // 실패한 편집은 첫 편집이 아니다
            tool_use(4, "Edit", json!({"file_path": "/work/repo/a.rs", "old_string": "x", "new_string": "y"})),
            tool_result(4, true, None),
            usage(5, 20),
            tool_use(6, "Write", json!({"file_path": "/work/repo/a.rs", "content": "y"})),
            tool_result(6, false, None),
            usage(7, 1000),
        ]);
        assert_eq!(s.tool_calls_before_first_edit, Some(3));
        assert_eq!(s.tokens_before_first_edit, Some(32));
        assert_eq!(s.cache_read_before_first_edit, Some(100));
        assert_eq!(s.output_tokens, 1030);
        assert_eq!(s.started.as_deref(), Some("2026-10-01T00:00:01.000Z"));
        assert_eq!(s.ended.as_deref(), Some("2026-10-01T00:00:07.000Z"));
    }

    #[test]
    fn report_has_counts_and_paths_but_no_text() {
        let s = run(&[
            json!({"type": "user", "uuid": "p", "message": {"content": "PRIVATE-PROMPT"}}),
            tool_use(1, "Bash", json!({"command": "PRIVATE-COMMAND && cargo test"})),
            tool_result(1, false, Some(json!({"stdout": "PRIVATE-OUTPUT"}))),
            tool_use(2, "Grep", json!({"pattern": "PRIVATE-PATTERN"})),
            tool_use(3, "Write", json!({"file_path": "/work/repo/a.rs", "content": "PRIVATE-CODE"})),
            tool_result(3, false, Some(json!({"type": "create", "content": "PRIVATE-CODE"}))),
        ]);
        assert_eq!(s.first_prompt.as_deref(), Some("PRIVATE-PROMPT"));
        let report = s.counts_json(&scope()).to_string();
        assert!(!report.contains("PRIVATE"), "{report}");
        assert!(report.contains("a.rs"));
        assert_eq!(s.events.iter().filter(|e| e.test_run).count(), 1);
        assert!(!format!("{:?}", s.events).contains("PRIVATE"));
    }
}
