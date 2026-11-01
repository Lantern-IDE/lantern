//! Codex 세션 로그: `<설정 폴더>/sessions/**/rollout-*.jsonl[.zst]`.
//!
//! 읽기 도구가 없어 무엇을 읽었는지 모른다 (`reads_known=false`). 바뀐 파일은 `FileChange` 항목이 가장 정확하고,
//! 수정은 줄 번호가 있는 조각만 남아 시작 본문이 없다. 그래서 변경을 들고 있다가 기준 커밋 본문 위에 적용한다.
//! `FileChange`가 없는 판은 `apply_patch` 입력 글을 읽는다.

use super::{
    is_test_command, modified_at, open_log, resolve_path, shell_cwd, walk_files, Agent, AgentEvent, Collector, DiscoverOpts, EditDetail,
    Hunk, Line, LineReader, Op, Patch, SessionRef, Usage,
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// 아는 줄 종류. 안쪽 `payload.type`은 자주 늘어나므로 겉 종류만 본다
const KNOWN: &[&str] = &["session_meta", "turn_context", "event_msg", "response_item", "compacted", "world_state", "token_usage_record"];

fn is_rollout(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    name.starts_with("rollout-") && (name.ends_with(".jsonl") || name.ends_with(".jsonl.zst"))
}

/// 첫 줄의 `session_meta`에서 세션 묶음 id. 하위 스레드는 `session_id`가 부모를 가리킨다
fn session_group(path: &Path) -> Option<String> {
    let mut lines = LineReader::new(open_log(path).ok()?);
    let Line::Text(first) = lines.next_line()? else { return None };
    let v: Value = serde_json::from_slice(first).ok()?;
    if v.get("type").and_then(Value::as_str) != Some("session_meta") {
        return None;
    }
    let meta = v.get("payload")?;
    meta.get("session_id").or(meta.get("id")).and_then(Value::as_str).map(str::to_string)
}

pub(super) fn discover(home: &Path, opts: &DiscoverOpts) -> Vec<SessionRef> {
    let mut files = Vec::new();
    walk_files(&home.join("sessions"), &mut files);
    files.retain(|p| is_rollout(p));
    // 압축 중이면 같은 이름이 둘 있을 수 있다. 압축 안 된 쪽이 최신이다
    let plain: HashSet<PathBuf> = files.iter().filter(|p| p.extension().is_some_and(|x| x == "jsonl")).cloned().collect();
    files.retain(|p| p.extension().is_none_or(|x| x != "zst") || !plain.contains(&p.with_extension("")));
    files.sort();

    let mut sessions: BTreeMap<String, SessionRef> = BTreeMap::new();
    for path in files {
        let modified = modified_at(&path).unwrap_or(SystemTime::UNIX_EPOCH);
        if opts.since.is_some_and(|t| modified < t) {
            continue;
        }
        let id = session_group(&path).unwrap_or_else(|| path.file_stem().unwrap_or_default().to_string_lossy().into_owned());
        let parent = path.file_name().unwrap_or_default().to_string_lossy().contains(id.as_str());
        let s = sessions
            .entry(id.clone())
            .or_insert_with(|| SessionRef { agent: Agent::Codex, id, files: Vec::new(), modified });
        s.modified = s.modified.max(modified);
        // 부모 로그가 맨 앞
        if parent {
            s.files.insert(0, path);
        } else {
            s.files.push(path);
        }
    }
    sessions.into_values().collect()
}

#[derive(Default)]
struct FileState {
    cwd: Option<String>,
    model: Option<String>,
    /// 누적 토큰. 줄마다 차이만 더한다
    total: Usage,
    /// 결과를 기다리는 셸 호출: call_id → 이벤트
    shells: HashMap<String, usize>,
    /// `FileChange`가 없을 때 쓸 apply_patch 호출
    patch_calls: HashMap<String, PatchCall>,
    /// 이미 `FileChange`로 처리한 호출
    changed: HashSet<String>,
}

struct PatchCall {
    ts: Option<String>,
    cwd: Option<String>,
    ops: Vec<PatchOp>,
    ok: Option<bool>,
}

pub(super) fn parse_file(path: &Path, c: &mut Collector<'_>) {
    let Ok(reader) = open_log(path) else {
        c.out.unreadable_files += 1;
        return;
    };
    c.begin_file();
    let mut st = FileState::default();
    let mut lines = LineReader::new(reader);
    while let Some(line) = lines.next_line() {
        match line {
            Line::TooLong => c.out.oversized_lines += 1,
            Line::Text(b) => on_line(b, &mut st, c),
        }
    }
    // `FileChange`가 끝내 없었던 호출 중 성공한 것만 글에서 읽는다
    let mut calls: Vec<PatchCall> = st.patch_calls.into_values().filter(|p| p.ok == Some(true)).collect();
    calls.sort_by(|a, b| a.ts.cmp(&b.ts));
    for call in calls {
        for op in call.ops {
            emit_patch_op(op, call.ts.as_deref(), call.cwd.as_deref(), c);
        }
    }
    c.end_file(path);
}

fn on_line(b: &[u8], st: &mut FileState, c: &mut Collector<'_>) {
    let Ok(mut v) = serde_json::from_slice::<Value>(b) else {
        c.unknown();
        return;
    };
    let kind = v.get("type").and_then(Value::as_str).map(str::to_string).unwrap_or_default();
    if !KNOWN.contains(&kind.as_str()) {
        c.unknown();
        return;
    }
    c.parsed();
    let ts = v.get("timestamp").and_then(Value::as_str).map(str::to_string);
    if let Some(t) = ts.as_deref() {
        c.note_ts(t);
    }
    let Some(payload) = v.get_mut("payload").filter(|p| p.is_object()) else { return };
    match kind.as_str() {
        "session_meta" => {
            if st.cwd.is_none() {
                st.cwd = payload.get("cwd").and_then(Value::as_str).map(str::to_string);
            }
        }
        "turn_context" => {
            if let Some(d) = payload.get("cwd").and_then(Value::as_str) {
                st.cwd = Some(d.to_string());
            }
            if let Some(m) = payload.get("model").and_then(Value::as_str) {
                st.model = Some(m.to_string());
                c.model(m);
            }
        }
        "event_msg" => on_event(payload, ts.as_deref(), st, c),
        "response_item" => on_item(payload, ts.as_deref(), st, c),
        _ => {}
    }
}

fn on_event(p: &mut Value, ts: Option<&str>, st: &mut FileState, c: &mut Collector<'_>) {
    match p.get("type").and_then(Value::as_str) {
        Some("token_count") => {
            let Some(t) = p.get("info").and_then(|i| i.get("total_token_usage")) else { return };
            let n = |k: &str| t.get(k).and_then(Value::as_u64).unwrap_or(0);
            // input_tokens에 캐시 읽기가 들어 있다
            let now = Usage {
                input: n("input_tokens").saturating_sub(n("cached_input_tokens")),
                output: n("output_tokens"),
                cache_read: n("cached_input_tokens"),
                cache_write: n("cache_write_input_tokens"),
            };
            let delta = Usage {
                input: now.input.saturating_sub(st.total.input),
                output: now.output.saturating_sub(st.total.output),
                cache_read: now.cache_read.saturating_sub(st.total.cache_read),
                cache_write: now.cache_write.saturating_sub(st.total.cache_write),
            };
            st.total = now;
            let model = st.model.clone();
            c.add_usage(None, ts, model.as_deref(), delta);
        }
        Some("user_message") => {
            if let Some(m) = p.get("message").and_then(Value::as_str) {
                c.prompt(m);
            }
        }
        Some("item_completed") => {
            let Some(item) = p.get_mut("item") else { return };
            match item.get("type").and_then(Value::as_str) {
                Some("UserMessage") => {
                    for t in item.get("content").and_then(Value::as_array).into_iter().flatten() {
                        if let Some(text) = t.get("text").and_then(Value::as_str) {
                            c.prompt(text);
                        }
                    }
                }
                Some("FileChange") => on_file_change(item, ts, st, c),
                _ => {}
            }
        }
        _ => {}
    }
}

fn on_file_change(item: &mut Value, ts: Option<&str>, st: &mut FileState, c: &mut Collector<'_>) {
    if let Some(id) = item.get("id").and_then(Value::as_str) {
        st.patch_calls.remove(id);
        st.changed.insert(id.to_string());
    }
    let applied = item.get("status").and_then(Value::as_str).is_none_or(|s| s == "completed");
    let Some(changes) = item.get_mut("changes").and_then(Value::as_object_mut) else { return };
    let mut changes: Vec<_> = std::mem::take(changes).into_iter().collect();
    changes.sort_by(|a, b| a.0.cmp(&b.0));
    for (raw, mut ch) in changes {
        let path = resolve_path(&raw, st.cwd.as_deref());
        if !c.scope.contains(&path) {
            continue;
        }
        if !applied {
            let mut e = AgentEvent::new(Op::Edit, ts.map(str::to_string), Some(path));
            e.ok = Some(false);
            c.push(e);
            continue;
        }
        let kind = ch.get("type").and_then(Value::as_str).unwrap_or("").to_string();
        let mut content = match ch.get_mut("content").map(Value::take) {
            Some(Value::String(s)) => Some(s),
            _ => None,
        };
        let hunks = ch.get("unified_diff").and_then(Value::as_str).map(parse_unified).unwrap_or_default();
        let moved = ch.get("move_path").and_then(Value::as_str).map(|m| resolve_path(m, st.cwd.as_deref()));
        let op = match (kind.as_str(), moved) {
            ("add", _) => PatchOp::Add { path, content: content.take().unwrap_or_default() },
            ("delete", _) => PatchOp::Delete { path },
            (_, Some(to)) => PatchOp::Move { path, to },
            _ => PatchOp::Update { path, hunks, content },
        };
        emit_resolved(op, ts, c);
    }
}

fn on_item(p: &mut Value, ts: Option<&str>, st: &mut FileState, c: &mut Collector<'_>) {
    let kind = p.get("type").and_then(Value::as_str).unwrap_or("").to_string();
    let call_id = p.get("call_id").and_then(Value::as_str).map(str::to_string);
    match kind.as_str() {
        "custom_tool_call" | "function_call" => {
            let name = p.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            let text = p.get("input").or(p.get("arguments")).and_then(Value::as_str).unwrap_or("");
            match name.as_str() {
                "apply_patch" => {
                    let args = serde_json::from_str::<Value>(text).ok();
                    let body = args.as_ref().and_then(|a| a.get("input").or(a.get("patch"))).and_then(Value::as_str).unwrap_or(text);
                    if let Some(id) = call_id.filter(|id| !st.changed.contains(id)) {
                        let call = PatchCall { ts: ts.map(str::to_string), cwd: st.cwd.clone(), ops: parse_apply_patch(body), ok: None };
                        st.patch_calls.insert(id, call);
                    }
                }
                "exec" => {
                    // 코드 안에서 셸을 부르는 도구. 글은 분류만 하고 버린다
                    push_shell(text, None, ts, call_id, st, c);
                }
                "shell_command" | "exec_command" | "shell" | "container.exec" | "local_shell" => {
                    let Ok(args) = serde_json::from_str::<Value>(text) else { return };
                    let cmd = args.get("command").or(args.get("cmd")).map(command_text).unwrap_or_default();
                    let dir = args.get("workdir").or(args.get("cwd")).and_then(Value::as_str).map(str::to_string);
                    push_shell(&cmd, dir.as_deref(), ts, call_id, st, c);
                }
                _ => {}
            }
        }
        "local_shell_call" => {
            let cmd = p.get("action").and_then(|a| a.get("command")).map(command_text).unwrap_or_default();
            let dir = p.get("action").and_then(|a| a.get("working_directory")).and_then(Value::as_str).map(str::to_string);
            push_shell(&cmd, dir.as_deref(), ts, call_id, st, c);
        }
        "custom_tool_call_output" | "function_call_output" => {
            let Some(id) = call_id else { return };
            let Some(output) = p.get("output") else { return };
            if let Some(call) = st.patch_calls.get_mut(&id) {
                call.ok = Some(patch_succeeded(output));
            } else if let Some(i) = st.shells.remove(&id) {
                c.out.events[i].ok = exit_status(output);
            }
        }
        _ => {}
    }
}

fn command_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
        _ => String::new(),
    }
}

fn push_shell(cmd: &str, workdir: Option<&str>, ts: Option<&str>, call_id: Option<String>, st: &mut FileState, c: &mut Collector<'_>) {
    let base = workdir.map(|d| resolve_path(d, st.cwd.as_deref()).to_string_lossy().into_owned()).or_else(|| st.cwd.clone());
    let dir = shell_cwd(cmd, base.as_deref());
    let Some(path) = dir.as_deref().map(|d| resolve_path(d, None)).filter(|p| c.scope.contains(p)) else { return };
    let mut e = AgentEvent::new(Op::Shell, ts.map(str::to_string), Some(path));
    e.test_run = is_test_command(cmd);
    let i = c.push(e);
    if let Some(id) = call_id {
        st.shells.insert(id, i);
    }
}

fn output_text(output: &Value) -> Option<&str> {
    match output {
        Value::String(s) => Some(s.trim_start()),
        _ => None,
    }
}

/// 셸 결과의 종료 코드. 글 형식이 `Exit code: N`이거나 JSON의 `metadata.exit_code`다
fn exit_status(output: &Value) -> Option<bool> {
    let text = output_text(output)?;
    if let Some(rest) = text.strip_prefix("Exit code: ") {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        return digits.parse::<i64>().ok().map(|n| n == 0);
    }
    if text.starts_with('{') {
        let json: Value = serde_json::from_str(text).ok()?;
        return json.get("metadata")?.get("exit_code")?.as_i64().map(|n| n == 0);
    }
    None
}

fn patch_succeeded(output: &Value) -> bool {
    match output_text(output) {
        Some(t) => t.starts_with("Success") || exit_status(output) == Some(true),
        None => false,
    }
}

pub(super) enum PatchOp {
    Add { path: PathBuf, content: String },
    Update { path: PathBuf, hunks: Vec<Hunk>, content: Option<String> },
    Delete { path: PathBuf },
    Move { path: PathBuf, to: PathBuf },
}

fn emit_patch_op(op: PatchOp, ts: Option<&str>, cwd: Option<&str>, c: &mut Collector<'_>) {
    let fix = |p: PathBuf| resolve_path(&p.to_string_lossy(), cwd);
    let op = match op {
        PatchOp::Add { path, content } => PatchOp::Add { path: fix(path), content },
        PatchOp::Update { path, hunks, content } => PatchOp::Update { path: fix(path), hunks, content },
        PatchOp::Delete { path } => PatchOp::Delete { path: fix(path) },
        PatchOp::Move { path, to } => PatchOp::Move { path: fix(path), to: fix(to) },
    };
    emit_resolved(op, ts, c);
}

fn emit_resolved(op: PatchOp, ts: Option<&str>, c: &mut Collector<'_>) {
    let emit = |path: PathBuf, detail: EditDetail, c: &mut Collector<'_>| {
        if !c.scope.contains(&path) {
            return;
        }
        let mut e = AgentEvent::new(Op::Edit, ts.map(str::to_string), Some(path));
        e.ok = Some(true);
        e.edit = Some(detail);
        let i = c.push(e);
        c.settle_edit(i);
    };
    match op {
        PatchOp::Add { path, content } => {
            emit(path, EditDetail { before: Some(String::new()), after: Some(content), patch: None }, c);
        }
        PatchOp::Update { path, hunks, content } => {
            let patch = (!hunks.is_empty()).then_some(Patch { replace: Vec::new(), hunks });
            emit(path, EditDetail { before: None, after: content, patch }, c);
        }
        PatchOp::Delete { path } => {
            emit(path, EditDetail { before: None, after: Some(String::new()), patch: None }, c);
        }
        PatchOp::Move { path, to } => {
            // 옛 경로는 사라지고, 새 경로의 내용은 여기서 알 수 없다
            emit(path, EditDetail { before: None, after: Some(String::new()), patch: None }, c);
            emit(to, EditDetail::default(), c);
        }
    }
}

/// `FileChange.unified_diff`: `@@ -a,b +c,d @@`로 시작하는 조각들
pub(super) fn parse_unified(diff: &str) -> Vec<Hunk> {
    let mut out: Vec<Hunk> = Vec::new();
    for line in diff.split('\n') {
        if let Some(rest) = line.strip_prefix("@@ -") {
            let range = rest.split(' ').next().unwrap_or("");
            let (start, count) = range.split_once(',').unwrap_or((range, "1"));
            let start = start.parse::<u32>().unwrap_or(0);
            // `-n,0`은 n번째 줄 뒤에 끼워 넣는다는 뜻이다
            let old_start = if count == "0" { start + 1 } else { start };
            out.push(Hunk { old_start: Some(old_start), lines: Vec::new() });
        } else if let Some(h) = out.last_mut() {
            h.lines.push(line.to_string());
        }
    }
    // 마지막 줄바꿈이 만든 빈 줄
    if let Some(h) = out.last_mut() {
        if h.lines.last().is_some_and(String::is_empty) {
            h.lines.pop();
        }
    }
    out
}

/// apply_patch 입력 글. 경로는 적힌 그대로다 (상대 경로일 수 있다)
pub(super) fn parse_apply_patch(text: &str) -> Vec<PatchOp> {
    let mut ops: Vec<PatchOp> = Vec::new();
    for line in text.lines() {
        if let Some(p) = line.strip_prefix("*** Add File: ") {
            ops.push(PatchOp::Add { path: PathBuf::from(p.trim()), content: String::new() });
        } else if let Some(p) = line.strip_prefix("*** Update File: ") {
            ops.push(PatchOp::Update { path: PathBuf::from(p.trim()), hunks: Vec::new(), content: None });
        } else if let Some(p) = line.strip_prefix("*** Delete File: ") {
            ops.push(PatchOp::Delete { path: PathBuf::from(p.trim()) });
        } else if let Some(p) = line.strip_prefix("*** Move to: ") {
            if let Some(PatchOp::Update { path, .. }) = ops.last() {
                let from = path.clone();
                ops.pop();
                ops.push(PatchOp::Move { path: from, to: PathBuf::from(p.trim()) });
            }
        } else if line.starts_with("*** ") {
            // Begin Patch, End Patch, End of File
        } else {
            match ops.last_mut() {
                Some(PatchOp::Add { content, .. }) => {
                    if let Some(body) = line.strip_prefix('+') {
                        content.push_str(body);
                        content.push('\n');
                    }
                }
                Some(PatchOp::Update { hunks, .. }) => {
                    if line.starts_with("@@") {
                        hunks.push(Hunk { old_start: None, lines: Vec::new() });
                    } else {
                        if hunks.is_empty() {
                            hunks.push(Hunk { old_start: None, lines: Vec::new() });
                        }
                        if let Some(h) = hunks.last_mut() {
                            h.lines.push(line.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::super::{discover as find_sessions, parse_session, Homes, Scope, Seen, SessionSummary};
    use super::*;
    use ruzstd::encoding::{compress_to_vec, CompressionLevel};
    use serde_json::json;
    use std::fs;

    const REPO: &str = "/work/repo";

    fn jsonl(lines: &[Value]) -> String {
        lines.iter().map(|l| format!("{l}\n")).collect()
    }

    fn scope() -> Scope {
        Scope::new([PathBuf::from(REPO)])
    }

    fn session(files: Vec<PathBuf>) -> SessionRef {
        SessionRef { agent: Agent::Codex, id: "c".into(), files, modified: SystemTime::UNIX_EPOCH }
    }

    fn run(lines: &[Value]) -> SessionSummary {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-a.jsonl");
        fs::write(&path, jsonl(lines)).unwrap();
        parse_session(&session(vec![path]), &scope(), &mut Seen::default())
    }

    fn at(n: u32, kind: &str, payload: Value) -> Value {
        json!({"timestamp": format!("2026-10-01T00:00:{n:02}.000Z"), "type": kind, "payload": payload})
    }

    fn meta() -> Value {
        at(0, "session_meta", json!({"id": "c", "cwd": REPO, "originator": "x", "base_instructions": {"text": "long"}}))
    }

    fn change(n: u32, id: &str, changes: Value) -> Value {
        at(n, "event_msg", json!({"type": "item_completed", "item": {"type": "FileChange", "id": id, "status": "completed", "stdout": "", "changes": changes}}))
    }

    #[test]
    fn file_changes_chain_add_and_update() {
        let s = run(&[
            meta(),
            change(1, "c1", json!({"/work/repo/a.rs": {"type": "add", "content": "one\ntwo\n"}, "/elsewhere/b.rs": {"type": "add", "content": "private"}})),
            change(2, "c2", json!({"/work/repo/a.rs": {"type": "update", "unified_diff": "@@ -1,2 +1,2 @@\n one\n-two\n+three\n"}})),
        ]);
        assert_eq!(s.events.len(), 2);
        assert!(!s.reads_known);
        let changes = s.file_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!((changes[0].before, changes[0].after), (Some(""), Some("one\nthree\n")));
    }

    #[test]
    fn updates_without_text_replay_on_a_base_blob() {
        let s = run(&[
            meta(),
            change(1, "c1", json!({"/work/repo/a.rs": {"type": "update", "unified_diff": "@@ -2,2 +2,2 @@\n b\n-c\n+C\n"}})),
            change(2, "c2", json!({"/work/repo/a.rs": {"type": "update", "unified_diff": "@@ -1 +1 @@\n-a\r\n+A\r\n"}})),
        ]);
        let changes = s.file_changes();
        assert_eq!((changes[0].before, changes[0].after, changes[0].pending.len()), (None, None, 2));
        assert_eq!(s.touched_paths().len(), 1);
        let (before, after) = changes[0].resolve(Some("a\nb\nc\n")).unwrap();
        assert_eq!(before, "a\nb\nc\n");
        assert_eq!(after, "A\r\nb\nC\n");
        assert_eq!(changes[0].resolve(None), None);
    }

    #[test]
    fn apply_patch_text_is_read_when_there_is_no_file_change() {
        let patch = "*** Begin Patch\n*** Add File: src/new.rs\n+fn new() {}\n*** Update File: /work/repo/a.rs\n@@ fn a\n fn a() {\n-  old();\n+  new();\n }\n*** Delete File: gone.rs\n*** End Patch\n";
        let s = run(&[
            meta(),
            at(1, "turn_context", json!({"cwd": "/work/repo", "model": "gpt-x"})),
            at(2, "response_item", json!({"type": "custom_tool_call", "name": "apply_patch", "call_id": "p1", "input": patch, "status": "completed"})),
            at(3, "response_item", json!({"type": "custom_tool_call_output", "call_id": "p1", "output": "Exit code: 0\nWall time: 0.1 seconds\nOutput:\n"})),
            // 실패한 호출과 결과가 없는 호출은 편집이 아니다
            at(4, "response_item", json!({"type": "custom_tool_call", "name": "apply_patch", "call_id": "p2", "input": "*** Begin Patch\n*** Add File: bad.rs\n+x\n*** End Patch"})),
            at(5, "response_item", json!({"type": "custom_tool_call_output", "call_id": "p2", "output": "apply_patch verification failed: no match"})),
            at(6, "response_item", json!({"type": "custom_tool_call", "name": "apply_patch", "call_id": "p3", "input": "*** Begin Patch\n*** Add File: none.rs\n+x\n*** End Patch"})),
        ]);
        let touched: Vec<_> = s.touched_paths().iter().map(|p| p.display().to_string()).collect();
        assert_eq!(touched, ["/work/repo/src/new.rs", "/work/repo/a.rs", "/work/repo/gone.rs"]);
        let changes = s.file_changes();
        assert_eq!((changes[0].before, changes[0].after), (Some(""), Some("fn new() {}\n")));
        let (_, after) = changes[1].resolve(Some("// top\nfn a() {\n  old();\n}\n")).unwrap();
        assert_eq!(after, "// top\nfn a() {\n  new();\n}\n");
        assert_eq!(changes[2].after, Some(""));
    }

    #[test]
    fn file_change_wins_over_the_same_apply_patch_call() {
        let patch = "*** Begin Patch\n*** Add File: a.rs\n+from text\n*** End Patch";
        let s = run(&[
            meta(),
            at(1, "response_item", json!({"type": "custom_tool_call", "name": "apply_patch", "call_id": "p1", "input": patch})),
            change(2, "p1", json!({"/work/repo/a.rs": {"type": "add", "content": "from change\n"}})),
            at(3, "response_item", json!({"type": "custom_tool_call_output", "call_id": "p1", "output": "Exit code: 0\n"})),
        ]);
        assert_eq!(s.events.len(), 1);
        assert_eq!(s.file_changes()[0].after, Some("from change\n"));
    }

    #[test]
    fn failed_file_change_and_moves() {
        let s = run(&[
            meta(),
            at(1, "event_msg", json!({"type": "item_completed", "item": {"type": "FileChange", "id": "x", "status": "failed", "changes": {"/work/repo/f.rs": {"type": "add", "content": "x"}}}})),
            change(2, "m", json!({"/work/repo/old.rs": {"type": "update", "unified_diff": "@@ -1 +1 @@\n-a\n+b\n", "move_path": "/work/repo/new.rs"}})),
        ]);
        assert_eq!(s.events[0].ok, Some(false));
        let touched: Vec<_> = s.touched_paths().iter().map(|p| p.display().to_string()).collect();
        assert_eq!(touched, ["/work/repo/old.rs", "/work/repo/new.rs"]);
        let changes = s.file_changes();
        assert_eq!(changes[0].after, Some(""));
        assert!(changes[1].opaque);
    }

    #[test]
    fn shell_calls_are_classified_with_exit_status() {
        let call = |n: u32, id: &str, args: Value| at(n, "response_item", json!({"type": "function_call", "name": "shell_command", "call_id": id, "arguments": args.to_string()}));
        let out = |n: u32, id: &str, output: &str| at(n, "response_item", json!({"type": "function_call_output", "call_id": id, "output": output}));
        let s = run(&[
            meta(),
            call(1, "a", json!({"command": "cargo test -p secret", "workdir": "/work/repo/sub"})),
            out(2, "a", "Exit code: 101\nOutput:\nprivate"),
            call(3, "b", json!({"command": "ls"})),
            out(4, "b", "Exit code: 0\n"),
            at(5, "response_item", json!({"type": "function_call", "name": "exec_command", "call_id": "c", "arguments": json!({"cmd": "pytest -q", "workdir": "/elsewhere"}).to_string()})),
            at(6, "response_item", json!({"type": "custom_tool_call", "name": "exec", "call_id": "d", "input": "const r = await tools.shell_command({command: \"npm test\"});"})),
            at(7, "response_item", json!({"type": "function_call", "name": "shell", "call_id": "e", "arguments": json!({"command": ["bash", "-lc", "go test ./..."]}).to_string()})),
            out(8, "e", "{\"output\":\"x\",\"metadata\":{\"exit_code\":0}}"),
        ]);
        let summary: Vec<_> = s.events.iter().map(|e| (e.op, e.test_run, e.ok)).collect();
        assert_eq!(
            summary,
            [(Op::Shell, true, Some(false)), (Op::Shell, false, Some(true)), (Op::Shell, true, None), (Op::Shell, true, Some(true))]
        );
        assert_eq!(s.events[0].path.as_deref(), Some(Path::new("/work/repo/sub")));
        let report = s.counts_json(&scope()).to_string();
        assert!(!report.contains("secret") && !report.contains("private"), "{report}");
    }

    #[test]
    fn tokens_are_cumulative_and_first_prompt_comes_from_user_items() {
        let tokens = |n: u32, input: u64, cached: u64, output: u64| {
            at(n, "event_msg", json!({"type": "token_count", "info": {"total_token_usage": {"input_tokens": input, "cached_input_tokens": cached, "output_tokens": output, "cache_write_input_tokens": 0, "total_tokens": input + output}}}))
        };
        let user = |n: u32, text: &str| at(n, "event_msg", json!({"type": "item_completed", "item": {"type": "UserMessage", "id": "u", "content": [{"type": "text", "text": text, "text_elements": []}]}}));
        let s = run(&[
            meta(),
            at(1, "turn_context", json!({"cwd": REPO, "model": "gpt-x"})),
            user(2, "<environment_context>x</environment_context>"),
            user(3, "do the task"),
            tokens(4, 100, 40, 10),
            at(5, "event_msg", json!({"type": "token_count", "info": null})),
            tokens(6, 150, 60, 15),
            change(7, "c", json!({"/work/repo/a.rs": {"type": "add", "content": "x"}})),
            tokens(8, 300, 100, 20),
        ]);
        assert_eq!(s.first_prompt.as_deref(), Some("do the task"));
        assert_eq!((s.input_tokens, s.cache_read, s.output_tokens), (200, 100, 20));
        assert_eq!(s.models, ["gpt-x"]);
        assert_eq!(s.usage_by_model["gpt-x"].input, 200);
        assert_eq!(s.tool_calls_before_first_edit, Some(0));
        assert_eq!(s.tokens_before_first_edit, Some(105));
        assert_eq!(s.cache_read_before_first_edit, Some(60));
    }

    #[test]
    fn unknown_and_truncated_lines_are_counted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-a.jsonl");
        let mut text = jsonl(&[meta(), json!({"type": "brand-new", "payload": {}}), json!({"type": "event_msg", "payload": {"type": "brand-new-inner"}})]);
        text.push_str("{\"type\":\"event_msg\",\"pay");
        fs::write(&path, text).unwrap();
        let s = parse_session(&session(vec![path]), &scope(), &mut Seen::default());
        assert_eq!((s.parsed_lines, s.unknown_lines), (2, 2));
        assert!(s.format_warnings.is_empty());
    }

    fn zst(text: &str) -> Vec<u8> {
        compress_to_vec(text.as_bytes(), CompressionLevel::Fastest)
    }

    #[test]
    fn zst_files_are_read_including_concatenated_frames() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout-a.jsonl.zst");
        let mut bytes = zst(&jsonl(&[meta(), change(1, "c", json!({"/work/repo/a.rs": {"type": "add", "content": "one\n"}}))]));
        bytes.extend(zst(&jsonl(&[change(2, "d", json!({"/work/repo/a.rs": {"type": "update", "unified_diff": "@@ -1 +1 @@\n-one\n+two\n"}}))])));
        fs::write(&path, bytes).unwrap();
        let s = parse_session(&session(vec![path]), &scope(), &mut Seen::default());
        assert_eq!(s.parsed_lines, 3);
        assert_eq!(s.file_changes()[0].after, Some("two\n"));
    }

    #[test]
    fn broken_zst_is_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let garbage = dir.path().join("rollout-g.jsonl.zst");
        fs::write(&garbage, b"not zstd at all").unwrap();
        let s = parse_session(&session(vec![garbage]), &scope(), &mut Seen::default());
        assert_eq!((s.unreadable_files, s.parsed_lines), (1, 0));

        // 쓰는 도중 잘린 파일: 읽은 데까지만 쓴다
        let cut = dir.path().join("rollout-c.jsonl.zst");
        let filler: Vec<Value> = (0..2000).map(|n| json!({"type": "world_state", "n": n, "s": format!("{}", n * 7919)})).collect();
        let bytes = zst(&jsonl(&filler));
        fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
        let s = parse_session(&session(vec![cut]), &scope(), &mut Seen::default());
        assert!(s.parsed_lines < 2000);
        assert_eq!(s.unreadable_files, 0);
    }

    #[test]
    fn discovery_groups_child_threads_under_the_root_session() {
        let home = tempfile::tempdir().unwrap();
        let day = home.path().join(".codex/sessions/2026/10/01");
        fs::create_dir_all(&day).unwrap();
        let meta_of = |id: &str, root: &str| json!({"type": "session_meta", "payload": {"id": id, "session_id": root, "parent_thread_id": root}});
        fs::write(day.join("rollout-2026-T2-child.jsonl.zst"), zst(&jsonl(&[meta_of("child", "root")]))).unwrap();
        fs::write(day.join("rollout-2026-T1-root.jsonl.zst"), zst(&jsonl(&[meta_of("root", "root")]))).unwrap();
        // 압축 전 파일이 함께 있으면 그쪽을 읽는다
        fs::write(day.join("rollout-2026-T3-solo.jsonl"), jsonl(&[json!({"type": "session_meta", "payload": {"id": "solo"}})])).unwrap();
        fs::write(day.join("rollout-2026-T3-solo.jsonl.zst"), b"old").unwrap();
        fs::write(day.join("not-a-rollout.jsonl"), "{}").unwrap();

        let homes = Homes::under(home.path());
        let opts = DiscoverOpts { agent: Some(Agent::Codex), ..Default::default() };
        let found = find_sessions(&homes, &scope(), &opts);
        assert_eq!(found.len(), 2);
        let root = found.iter().find(|s| s.id == "root").unwrap();
        assert_eq!(root.files.len(), 2);
        assert!(root.files[0].to_string_lossy().contains("root"));
        let solo = found.iter().find(|s| s.id == "solo").unwrap();
        assert_eq!(solo.files.len(), 1);
        assert!(solo.files[0].extension().is_some_and(|x| x == "jsonl"));
        // 빈 집이어도 괜찮다
        let empty = tempfile::tempdir().unwrap();
        assert!(find_sessions(&Homes::under(empty.path()), &scope(), &DiscoverOpts::default()).is_empty());
    }

    #[test]
    fn unified_diff_parser_handles_empty_old_side() {
        let hunks = parse_unified("--- a\n+++ b\n@@ -0,0 +1,2 @@\n+x\n+y\n");
        assert_eq!((hunks.len(), hunks[0].old_start, hunks[0].lines.len()), (1, Some(1), 2));
    }
}
