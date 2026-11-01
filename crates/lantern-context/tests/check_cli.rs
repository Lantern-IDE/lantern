//! `lantern check`와 `lantern hook`을 진짜 git 저장소와 진짜 실행 파일로 돌린다.
//! 홈, 데이터, 인덱스 폴더는 모두 임시 폴더로 돌려 사용자 폴더를 건드리지 않는다.

use serde_json::{json, Value};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const LIB: &str = "pub fn compute_total(a: i32) -> i32 {\n    let doubled = a * 2;\n    doubled + 40\n}\n";
const SHOW: &str = "use crate::lib::compute_total;\n\nfn show_it() -> i32 {\n    compute_total(1)\n}\n";
const RENAMED: &str = "pub fn add_all_up(a: i32) -> i32 {\n    let doubled = a * 2;\n    doubled + 40\n}\n";
const WIDER: &str = "pub fn compute_total(a: i32, b: i32) -> i32 {\n    let doubled = a * 2;\n    doubled + 40 + b\n}\n";

struct Sandbox {
    _tmp: tempfile::TempDir,
    repo: PathBuf,
    home: PathBuf,
    data: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, home, data) = (tmp.path().join("repo"), tmp.path().join("home"), tmp.path().join("data"));
        for dir in [&repo, &home, &data] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let sandbox = Self { _tmp: tmp, repo, home, data };
        sandbox.git(&["init", "-q"]);
        sandbox
    }

    /// 사용자 git 설정의 영향을 받지 않게 한다
    fn isolate<'c>(&self, cmd: &'c mut Command) -> &'c mut Command {
        cmd.env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.home.join("gitconfig"))
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CLAUDE_CONFIG_DIR", self.home.join(".claude"))
            .env("CODEX_HOME", self.home.join(".codex"))
            .env("LANTERN_DATA_DIR", &self.data)
            // 디버그 빌드가 병렬 테스트 중에 예산을 넘겨 조용히 지나가면 시험이 되지 않는다
            .env("LANTERN_CHECK_BUDGET_MS", "120000")
            .env_remove("LANTERN_INDEX_DIR")
    }

    fn git(&self, args: &[&str]) -> String {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(&self.repo).args(["-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "core.autocrlf=false", "-c", "commit.gpgsign=false"]);
        let out = self.isolate(&mut cmd).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.repo.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn commit_all(&self) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", "c"]);
    }

    /// 커밋 시각을 정해 커밋한다
    fn commit_at(&self, date: &str) {
        self.git(&["add", "-A"]);
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(&self.repo).args(["-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false", "commit", "-q", "-m", "c"]);
        let out = self.isolate(&mut cmd).env("GIT_COMMITTER_DATE", date).env("GIT_AUTHOR_DATE", date).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }

    /// 호출하는 파일 하나, 불리는 파일 하나
    fn with_caller() -> Self {
        let s = Self::new();
        s.write("src/lib.rs", LIB);
        s.write("src/show.rs", SHOW);
        s.commit_all();
        s
    }

    fn lantern(&self, args: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_lantern"));
        cmd.arg("-C").arg(&self.repo).args(args).stdin(Stdio::null());
        self.isolate(&mut cmd).output().unwrap()
    }

    fn stop_hook(&self, input: &str) -> Output {
        self.stop_hook_args(input, &[])
    }

    fn stop_hook_args(&self, input: &str, extra: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_lantern"));
        cmd.current_dir(&self.repo).args(["check", "--stop-hook"]).args(extra);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = self.isolate(&mut cmd).spawn().unwrap();
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    }

    fn transcript_path(&self) -> PathBuf {
        self.home.join("session.jsonl")
    }

    /// 합성 세션 기록을 쓴다. 파일을 고치고 읽은 것만 있다
    fn transcript(&self, edits: &[&str], reads: &[&str]) -> PathBuf {
        let root = self.git(&["rev-parse", "--show-toplevel"]);
        let root = root.trim();
        let mut lines = Vec::new();
        let mut n = 0;
        let mut tool = |name: &str, input: Value, result: Value| {
            n += 1;
            let ts = format!("2026-01-01T00:00:{n:02}Z");
            lines.push(json!({"type": "assistant", "uuid": format!("a{n}"), "timestamp": ts, "cwd": root,
                "message": {"id": format!("m{n}"), "model": "test-model", "content": [{"type": "tool_use", "id": format!("t{n}"), "name": name, "input": input}]}}).to_string());
            lines.push(json!({"type": "user", "uuid": format!("u{n}"), "timestamp": ts, "cwd": root,
                "message": {"content": [{"type": "tool_result", "tool_use_id": format!("t{n}")}]}, "toolUseResult": result}).to_string());
        };
        for r in reads {
            tool("Read", json!({"file_path": format!("{root}/{r}")}), json!({"type": "text"}));
        }
        for e in edits {
            tool("Write", json!({"file_path": format!("{root}/{e}"), "content": "x"}), json!({"type": "update", "filePath": format!("{root}/{e}")}));
        }
        let path = self.transcript_path();
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        path
    }

    fn input(&self, session: &str, transcript: &Path, active: bool) -> String {
        json!({"session_id": session, "transcript_path": transcript, "cwd": self.repo, "hook_event_name": "Stop", "stop_hook_active": active}).to_string()
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.data.join("check-log.jsonl")).unwrap_or_default()
    }
}

/// 훅을 돌리는 셸. Windows의 PATH에 있는 `bash`는 WSL일 수 있어 git 옆의 Git Bash를 찾는다
fn hook_shell() -> Option<PathBuf> {
    if !cfg!(windows) {
        return Some(PathBuf::from("bash"));
    }
    let out = Command::new("git").arg("--exec-path").output().ok()?;
    let exec = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let top = exec.ancestors().nth(3)?;
    [top.join("bin/bash.exe"), top.join("usr/bin/bash.exe")].into_iter().find(|p| p.exists())
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn json_of(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", stdout(out)))
}

fn assert_silent_ok(out: &Output, what: &str) {
    assert_eq!(out.status.code(), Some(0), "{what}");
    assert!(out.stdout.is_empty(), "{what} stdout: {}", stdout(out));
    assert!(out.stderr.is_empty(), "{what} stderr: {}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn check_finds_renamed_function_with_missed_caller() {
    let s = Sandbox::with_caller();
    s.write("src/lib.rs", RENAMED);
    let out = s.lantern(&["check", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let v = json_of(&out);
    assert_eq!(v["removed"], 1, "{v}");
    assert_eq!(v["changed_code_files"], 1);
    let findings = v["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{v}");
    let f = &findings[0];
    assert_eq!(f["kind"], "dangling");
    assert_eq!(f["confidence"], "high");
    assert_eq!(f["symbol"], "compute_total");
    assert_eq!(f["renamed_to"], "add_all_up");
    assert_eq!(f["path"], "src/show.rs");
    assert_eq!(f["line"], 4);
    assert_eq!(f["caller"], "show_it");

    // 글로 볼 때도 근거와 이름 기반이라는 말이 있다. 코드는 없다
    let text = stdout(&s.lantern(&["check"]));
    assert!(text.contains("HIGH") && text.contains("src/show.rs:4") && text.contains("Name based"), "{text}");
    assert!(!text.contains("a * 2"), "{text}");

    assert_eq!(s.lantern(&["check", "--strict"]).status.code(), Some(1));
}

#[test]
fn check_leaves_the_repository_alone() {
    let s = Sandbox::with_caller();
    s.write("src/lib.rs", RENAMED);
    let head = s.git(&["rev-parse", "HEAD"]);
    let status = s.git(&["status", "--porcelain"]);
    let config = std::fs::read(s.repo.join(".git/config")).unwrap();
    let index_mtime = std::fs::metadata(s.repo.join(".git/index")).unwrap().modified().unwrap();
    s.lantern(&["check"]);
    assert_eq!(s.git(&["rev-parse", "HEAD"]), head);
    assert_eq!(std::fs::read(s.repo.join(".git/config")).unwrap(), config);
    assert_eq!(std::fs::metadata(s.repo.join(".git/index")).unwrap().modified().unwrap(), index_mtime);
    // 저장소에 인덱스 폴더도 만들지 않는다
    assert!(!s.repo.join(".lantern").exists());
    assert_eq!(s.git(&["status", "--porcelain"]), status);
    assert!(s.data.join("index").is_dir());
}

#[test]
fn check_reuses_the_index_the_repository_already_has() {
    let s = Sandbox::with_caller();
    assert_eq!(s.lantern(&["index"]).status.code(), Some(0));
    let db = s.repo.join(".lantern/index.db");
    assert!(db.exists());
    s.write("src/lib.rs", RENAMED);
    let v = json_of(&s.lantern(&["check", "--json"]));
    assert_eq!(v["findings"].as_array().unwrap().len(), 1, "{v}");
    assert!(!s.data.join("index").exists());
}

#[test]
fn check_is_quiet_when_the_caller_was_updated_too() {
    let s = Sandbox::with_caller();
    s.write("src/lib.rs", RENAMED);
    s.write("src/show.rs", &SHOW.replace("compute_total", "add_all_up"));
    let v = json_of(&s.lantern(&["check", "--json"]));
    assert_eq!(v["findings"].as_array().unwrap().len(), 0, "{v}");
    let text = stdout(&s.lantern(&["check"]));
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(text.contains("nothing left behind"), "{text}");
    assert_eq!(s.lantern(&["check", "--strict"]).status.code(), Some(0));
}

#[test]
fn check_stale_caller_is_shown_but_does_not_fail_strict() {
    let s = Sandbox::with_caller();
    s.write("src/lib.rs", WIDER);
    let v = json_of(&s.lantern(&["check", "--json"]));
    assert_eq!(v["signature"], 1, "{v}");
    let f = &v["findings"][0];
    assert_eq!(f["kind"], "stale_caller");
    assert_eq!(f["confidence"], "medium");
    assert_eq!(f["path"], "src/show.rs");
    assert_eq!(s.lantern(&["check", "--strict"]).status.code(), Some(0));
}

#[test]
fn check_cuts_findings_at_twenty_unless_all() {
    let s = Sandbox::with_caller();
    for i in 0..25 {
        s.write(&format!("src/caller{i}.rs"), "fn call_it() -> i32 {\n    compute_total(1)\n}\n");
    }
    s.commit_all();
    s.write("src/lib.rs", RENAMED);

    let v = json_of(&s.lantern(&["check", "--json"]));
    assert_eq!(v["total_findings"], 26, "{v}");
    assert_eq!(v["findings"].as_array().unwrap().len(), 20, "{v}");
    assert!(stdout(&s.lantern(&["check"])).contains("Showing 20 of 26. --all shows the rest."));

    let all = json_of(&s.lantern(&["check", "--json", "--all"]));
    assert_eq!(all["total_findings"], 26, "{all}");
    assert_eq!(all["findings"].as_array().unwrap().len(), 26, "{all}");
    assert_eq!(all["findings"].as_array().unwrap()[..20], v["findings"].as_array().unwrap()[..], "앞 20개는 기본 출력과 같다");
    assert!(!stdout(&s.lantern(&["check", "--all"])).contains("Showing"));
}

#[test]
fn check_sees_untracked_moves_deleted_files_and_other_bases() {
    // 정의를 새 파일로 옮기면 놓친 것이 아니다
    let s = Sandbox::with_caller();
    s.write("src/lib.rs", "pub fn other_fn() {}\n");
    s.write("src/moved.rs", LIB);
    let v = json_of(&s.lantern(&["check", "--json"]));
    assert_eq!(v["findings"].as_array().unwrap().len(), 0, "{v}");

    // 파일을 지우면 그 안의 정의도 사라진다
    let s = Sandbox::with_caller();
    std::fs::remove_file(s.repo.join("src/lib.rs")).unwrap();
    let v = json_of(&s.lantern(&["check", "--json"]));
    assert_eq!(v["findings"][0]["symbol"], "compute_total", "{v}");

    // 스테이징만 해도, 이미 커밋해도 --base로 볼 수 있다
    let s = Sandbox::with_caller();
    s.write("src/lib.rs", RENAMED);
    s.git(&["add", "-A"]);
    assert_eq!(json_of(&s.lantern(&["check", "--json"]))["findings"].as_array().unwrap().len(), 1);
    s.commit_all();
    assert_eq!(json_of(&s.lantern(&["check", "--json"]))["findings"].as_array().unwrap().len(), 0);
    let v = json_of(&s.lantern(&["check", "--json", "--base", "HEAD~1"]));
    assert_eq!(v["findings"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v["base"], "HEAD~1");
}

#[test]
fn check_errors_exit_zero_unless_strict() {
    let s = Sandbox::with_caller();
    let out = s.lantern(&["check", "--base", "no-such-commit"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(!out.stderr.is_empty());
    assert_eq!(s.lantern(&["check", "--strict", "--base", "no-such-commit"]).status.code(), Some(2));
    assert_eq!(s.lantern(&["check", "--strict", "--base", "--upload-pack=x"]).status.code(), Some(2));
}

#[test]
fn stop_hook_blocks_once_with_a_short_actionable_reason() {
    let s = Sandbox::with_caller();
    s.write("src/lib.rs", RENAMED);
    let t = s.transcript(&["src/lib.rs"], &[]);

    let out = s.stop_hook(&s.input("s1", &t, false));
    assert_eq!(out.status.code(), Some(0));
    let v = json_of(&out);
    assert_eq!(v["decision"], "block");
    let reason = v["reason"].as_str().unwrap();
    assert!(reason.contains("compute_total") && reason.contains("src/show.rs:4") && reason.contains("add_all_up"), "{reason}");
    assert!(reason.contains("name based"), "{reason}");
    assert!(!reason.contains("a * 2"), "{reason}");

    // 같은 세션은 다시 막지 않는다
    let out = s.stop_hook(&s.input("s1", &t, false));
    assert_silent_ok(&out, "두 번째");
    // 다른 세션은 막는다
    let out = s.stop_hook(&s.input("s2", &t, false));
    assert_eq!(json_of(&out)["decision"], "block");
}

#[test]
fn stop_hook_active_is_honored() {
    let s = Sandbox::with_caller();
    s.write("src/lib.rs", RENAMED);
    let t = s.transcript(&["src/lib.rs"], &[]);
    assert_silent_ok(&s.stop_hook(&s.input("s1", &t, true)), "활성");
    assert!(s.log().is_empty(), "{}", s.log());
    // 활성 중에 그냥 지나갔어도 나중의 Stop은 막을 수 있다
    assert_eq!(json_of(&s.stop_hook(&s.input("s1", &t, false)))["decision"], "block");
}

#[test]
fn stop_hook_only_blocks_for_this_sessions_code_edits() {
    let s = Sandbox::with_caller();
    s.write("src/lib.rs", RENAMED);
    // 이 세션은 다른 파일만 고쳤다. 작업 트리의 다른 변경은 이 세션 탓이 아니다
    s.write("src/note.rs", "fn note_it() {}\n");
    let t = s.transcript(&["src/note.rs"], &[]);
    assert_silent_ok(&s.stop_hook(&s.input("s1", &t, false)), "남의 변경");
    // 코드가 아닌 파일만 고친 세션
    s.write("README.md", "# hi\n");
    let t = s.transcript(&["README.md"], &[]);
    assert_silent_ok(&s.stop_hook(&s.input("s2", &t, false)), "문서만");
    // 아무것도 고치지 않은 세션
    let t = s.transcript(&[], &["src/lib.rs"]);
    assert_silent_ok(&s.stop_hook(&s.input("s3", &t, false)), "읽기만");
}

#[test]
fn stop_hook_without_a_transcript_falls_back_to_the_working_tree() {
    let s = Sandbox::with_caller();
    s.write("src/lib.rs", RENAMED);
    let out = s.stop_hook(&json!({"session_id": "s1", "cwd": s.repo}).to_string());
    assert_eq!(json_of(&out)["decision"], "block");
}

#[test]
fn stop_hook_with_a_transcript_too_big_to_read_in_time_uses_the_working_tree() {
    let s = Sandbox::with_caller();
    s.write("src/lib.rs", RENAMED);
    let t = s.transcript_path();
    std::fs::File::create(&t).unwrap().set_len(65 * 1024 * 1024).unwrap();
    let out = s.stop_hook(&s.input("s1", &t, false));
    assert_eq!(json_of(&out)["decision"], "block");
    // 하위 에이전트 기록까지 합쳐 잰다
    let s = Sandbox::with_caller();
    s.write("src/lib.rs", RENAMED);
    let t = s.transcript(&[], &["src/lib.rs"]);
    assert_silent_ok(&s.stop_hook(&s.input("s1", &t, false)), "읽기만 한 세션");
    let sub = t.with_extension("").join("subagents");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::File::create(sub.join("agent-1.jsonl")).unwrap().set_len(65 * 1024 * 1024).unwrap();
    assert_eq!(json_of(&s.stop_hook(&s.input("s2", &t, false)))["decision"], "block");
}

#[test]
fn stop_hook_does_not_block_on_medium_and_logs_read_callers_as_filtered() {
    let s = Sandbox::with_caller();
    s.write("src/lib.rs", WIDER);
    let t = s.transcript(&["src/lib.rs"], &[]);
    assert_silent_ok(&s.stop_hook(&s.input("s1", &t, false)), "중간 믿음");
    assert!(s.log().contains(r#""stale":1"#), "{}", s.log());

    // 호출자를 읽었으면 발견이 아니다
    let t = s.transcript(&["src/lib.rs"], &["src/show.rs"]);
    assert_silent_ok(&s.stop_hook(&s.input("s2", &t, false)), "읽음");
    let last = s.log().lines().last().unwrap().to_string();
    assert!(last.contains(r#""stale":0"#), "{last}");
}

#[test]
fn stop_hook_uses_the_session_start_text_when_the_session_already_committed() {
    let s = Sandbox::with_caller();
    s.write("src/lib.rs", RENAMED);
    s.commit_all();
    let root = s.git(&["rev-parse", "--show-toplevel"]);
    let file = format!("{}/src/lib.rs", root.trim());
    let lines = [
        json!({"type": "assistant", "uuid": "a1", "timestamp": "2026-01-01T00:00:01Z", "cwd": root.trim(),
            "message": {"id": "m1", "content": [{"type": "tool_use", "id": "t1", "name": "Edit", "input": {"file_path": file, "old_string": "compute_total", "new_string": "add_all_up"}}]}})
        .to_string(),
        json!({"type": "user", "uuid": "u1", "timestamp": "2026-01-01T00:00:02Z", "cwd": root.trim(),
            "message": {"content": [{"type": "tool_result", "tool_use_id": "t1"}]},
            "toolUseResult": {"filePath": file, "originalFile": LIB, "oldString": "compute_total", "newString": "add_all_up"}})
        .to_string(),
    ];
    std::fs::write(s.transcript_path(), lines.join("\n") + "\n").unwrap();
    assert_eq!(json_of(&s.stop_hook(&s.input("s1", &s.transcript_path(), false)))["decision"], "block");
}

#[test]
fn stop_hook_reads_the_commit_before_the_first_edit_when_the_log_has_no_start_text() {
    let s = Sandbox::new();
    s.write("src/lib.rs", LIB);
    s.write("src/show.rs", SHOW);
    s.commit_at("2026-01-01T00:00:00Z");
    s.write("src/lib.rs", RENAMED);
    s.commit_at("2026-03-01T00:00:00Z");
    // 2월의 편집이 3월에 커밋됐다. 로그에는 시작 본문이 없다
    let root = s.git(&["rev-parse", "--show-toplevel"]);
    let file = format!("{}/src/lib.rs", root.trim());
    let lines = [
        json!({"type": "assistant", "uuid": "a1", "timestamp": "2026-02-01T00:00:01Z", "cwd": root.trim(),
            "message": {"id": "m1", "content": [{"type": "tool_use", "id": "t1", "name": "Edit", "input": {"file_path": file, "old_string": "compute_total", "new_string": "add_all_up"}}]}})
        .to_string(),
        json!({"type": "user", "uuid": "u1", "timestamp": "2026-02-01T00:00:02Z", "cwd": root.trim(),
            "message": {"content": [{"type": "tool_result", "tool_use_id": "t1"}]}, "toolUseResult": {"filePath": file}})
        .to_string(),
    ];
    std::fs::write(s.transcript_path(), lines.join("\n") + "\n").unwrap();
    let v = json_of(&s.stop_hook(&s.input("s1", &s.transcript_path(), false)));
    assert_eq!(v["decision"], "block", "{v}");
}

#[test]
fn other_commands_may_still_receive_the_flag_as_text() {
    let s = Sandbox::with_caller();
    let out = s.lantern(&["search", "--stop-hook"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).contains("--stop-hook"), "{}", stdout(&out));
    assert!(s.repo.join(".lantern").exists());
}

#[test]
fn stop_hook_internal_errors_mean_exit_zero_and_no_output() {
    let s = Sandbox::with_caller();
    s.write("src/lib.rs", RENAMED);
    let t = s.transcript(&["src/lib.rs"], &[]);
    let outside = s.home.join("not-a-repo");
    std::fs::create_dir_all(&outside).unwrap();
    for (what, input) in [
        ("빈 입력", String::new()),
        ("JSON 아님", "{ not json".to_string()),
        ("형식 다름", "[1,2]".to_string()),
        ("세션 없음", json!({"transcript_path": t, "cwd": s.repo}).to_string()),
        ("없는 폴더", json!({"session_id": "e1", "transcript_path": t, "cwd": s.repo.join("no-such")}).to_string()),
        ("저장소 아님", json!({"session_id": "e2", "transcript_path": t, "cwd": outside}).to_string()),
    ] {
        assert_silent_ok(&s.stop_hook(&input), what);
    }
    // 없는 기준 커밋
    assert_silent_ok(&s.stop_hook_args(&s.input("e3", &t, false), &["--base", "no-such-commit"]), "기준 없음");
    // 잘못된 인자도 clap의 종료 코드 2로 끝나면 막기가 된다
    assert_silent_ok(&s.stop_hook_args(&s.input("e4", &t, false), &["--no-such-flag"]), "인자 오류");
    // 데이터 폴더를 쓸 수 없으면 한 번만 막는다고 약속할 수 없으니 막지 않는다
    let blocked_dir = s.home.join("file-not-dir");
    std::fs::write(&blocked_dir, "x").unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_lantern"));
    cmd.current_dir(&s.repo).args(["check", "--stop-hook"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    s.isolate(&mut cmd).env("LANTERN_DATA_DIR", &blocked_dir);
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(s.input("e5", &t, false).as_bytes()).unwrap();
    assert_silent_ok(&child.wait_with_output().unwrap(), "데이터 폴더 없음");
}

#[test]
fn stop_hook_gives_up_quietly_when_stdin_never_ends() {
    let s = Sandbox::with_caller();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_lantern"));
    cmd.current_dir(&s.repo).args(["check", "--stop-hook"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = s.isolate(&mut cmd).env("LANTERN_CHECK_BUDGET_MS", "300").spawn().unwrap();
    let keep_open = child.stdin.take().unwrap();
    let started = Instant::now();
    let out = child.wait_with_output().unwrap();
    drop(keep_open);
    assert_silent_ok(&out, "입력이 끝나지 않음");
    assert!(started.elapsed() < Duration::from_secs(20), "{:?}", started.elapsed());
}

#[test]
fn stop_hook_default_budget_is_under_three_seconds() {
    assert!(lantern_context::hook::STOP_HOOK_BUDGET < Duration::from_secs(3));
}

#[test]
fn stats_count_flagged_and_later_edited_without_names() {
    let s = Sandbox::with_caller();
    assert!(stdout(&s.lantern(&["check", "--stats"])).contains("no stop hook runs"));
    s.write("src/lib.rs", RENAMED);
    let t = s.transcript(&["src/lib.rs"], &[]);
    assert_eq!(json_of(&s.stop_hook(&s.input("s1", &t, false)))["decision"], "block");
    let v = json_of(&s.lantern(&["check", "--stats", "--json"]));
    assert_eq!((v["runs"].as_u64(), v["blocked"].as_u64(), v["flagged_files"].as_u64(), v["later_edited"].as_u64()), (Some(1), Some(1), Some(1), Some(0)), "{v}");

    // 막힌 뒤 호출자를 고치고 다시 Stop
    let t = s.transcript(&["src/lib.rs", "src/show.rs"], &[]);
    assert_silent_ok(&s.stop_hook(&s.input("s1", &t, true)), "뒤이은 Stop");
    let v = json_of(&s.lantern(&["check", "--stats", "--json"]));
    assert_eq!((v["flagged_files"].as_u64(), v["later_edited"].as_u64()), (Some(1), Some(1)), "{v}");
    // 같은 세션의 다음 Stop에서 두 번 세지 않는다
    assert_silent_ok(&s.stop_hook(&s.input("s1", &t, false)), "또 Stop");
    let v = json_of(&s.lantern(&["check", "--stats", "--json"]));
    assert_eq!(v["later_edited"].as_u64(), Some(1), "{v}");
    let text = stdout(&s.lantern(&["check", "--stats"]));
    assert!(text.contains("Flagged 1 file, later edited 1"), "{text}");

    // 기록과 상태 파일에는 이름도 경로도 없다
    let mut stored = s.log();
    for e in std::fs::read_dir(s.data.join("check-sessions")).unwrap().flatten() {
        stored.push_str(&std::fs::read_to_string(e.path()).unwrap());
    }
    assert!(!stored.is_empty());
    for secret in ["compute_total", "add_all_up", "show", "src/", "repo", "s1"] {
        assert!(!stored.contains(secret), "{secret}: {stored}");
    }
}

#[test]
fn hook_install_and_uninstall_through_the_command_line() {
    let s = Sandbox::with_caller();
    let settings = s.repo.join(".claude/settings.local.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(&settings, r#"{"other": {"keep": 1}, "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "mine"}]}]}}"#).unwrap();

    let out = s.lantern(&["hook", "install", "claude"]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let text = stdout(&out);
    assert!(text.contains("Backup:") && text.contains("Added to hooks.Stop") && text.contains("--stop-hook"), "{text}");
    // 임시 저장소에는 무시 규칙이 없으니 커밋되지 않게 알려 준다
    assert!(text.contains("not ignored by git"), "{text}");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(v["other"]["keep"], 1);
    assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], "mine");
    let cmd = v["hooks"]["Stop"][1]["hooks"][0]["command"].as_str().unwrap();
    assert!(cmd.contains("lantern") && cmd.ends_with("check --stop-hook") && !cmd.contains('\\'), "{cmd}");
    assert!(s.repo.join(".claude/settings.local.json.lantern-backup").exists());

    let text = stdout(&s.lantern(&["hook", "uninstall", "claude"]));
    assert!(text.contains("Removed from hooks.Stop"), "{text}");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1);

    // 사용자 설정은 CLAUDE_CONFIG_DIR 아래로 간다
    let user = s.home.join(".claude/settings.json");
    assert_eq!(s.lantern(&["hook", "install", "claude", "--user"]).status.code(), Some(0));
    assert!(user.exists());
    assert_eq!(s.lantern(&["hook", "uninstall", "claude", "--user"]).status.code(), Some(0));

    // 깨진 설정은 건드리지 않고 실패한다
    std::fs::write(&settings, "{ broken").unwrap();
    assert_ne!(s.lantern(&["hook", "install", "claude"]).status.code(), Some(0));
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{ broken");

    assert_ne!(s.lantern(&["hook", "install", "claude", "--user", "--project"]).status.code(), Some(0));
    assert_ne!(s.lantern(&["hook", "install", "codex"]).status.code(), Some(0));
}

#[test]
fn installed_hook_command_actually_runs() {
    let s = Sandbox::with_caller();
    s.write("src/lib.rs", RENAMED);
    s.lantern(&["hook", "install", "claude", "--user"]);
    let v: Value = serde_json::from_str(&std::fs::read_to_string(s.home.join(".claude/settings.json")).unwrap()).unwrap();
    let command = v["hooks"]["Stop"][0]["hooks"][0]["command"].as_str().unwrap();
    let t = s.transcript(&["src/lib.rs"], &[]);
    // Windows에서 Claude Code는 Git Bash로 훅을 돌린다
    let Some(shell) = hook_shell() else { return };
    let mut cmd = Command::new(shell);
    cmd.args(["-c", command]).current_dir(&s.repo).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = s.isolate(&mut cmd).spawn().unwrap();
    child.stdin.take().unwrap().write_all(s.input("s1", &t, false).as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(json_of(&out)["decision"], "block", "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn each_language_family_works_end_to_end() {
    let cases = [
        ("py/lib.py", "def compute_total(a):\n    return a\n", "py/show.py", "from lib import compute_total\n\ndef show_it():\n    return compute_total(1)\n"),
        ("ts/lib.ts", "export function computeTotal(a: number): number { return a; }\n", "ts/show.ts", "function showIt() {\n  return computeTotal(1);\n}\n"),
        ("js/lib.js", "function computeTotal(a) { return a; }\n", "js/show.js", "function showIt() {\n  return computeTotal(1);\n}\n"),
        ("java/Lib.java", "class Lib {\n  int computeTotal(int a) { return a; }\n}\n", "java/Show.java", "class Show {\n  int showIt(Lib l) {\n    return l.computeTotal(1);\n  }\n}\n"),
        ("go/lib.go", "package lib\n\nfunc ComputeTotal(a int) int { return a }\n", "go/show.go", "package lib\n\nfunc ShowIt() int {\n\treturn ComputeTotal(1)\n}\n"),
    ];
    let s = Sandbox::new();
    for (lib, lib_text, show, show_text) in cases {
        s.write(lib, lib_text);
        s.write(show, show_text);
    }
    s.commit_all();
    for (lib, _, _, _) in cases {
        s.write(lib, "");
    }
    let v = json_of(&s.lantern(&["check", "--json"]));
    let found: Vec<&str> = v["findings"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap()).collect();
    for (_, _, show, _) in cases {
        assert!(found.contains(&show), "{show} 없음: {v}");
    }
    assert_eq!(found.len(), cases.len(), "{v}");
}

