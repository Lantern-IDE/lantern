//! `lantern autopsy`를 합성 세션 기록과 진짜 git 저장소, 진짜 실행 파일로 돌린다.
//! 홈, 데이터, 임시 폴더는 모두 시험 폴더 안으로 돌려 사용자 폴더를 건드리지 않는다.

use lantern_context::sessions::claude::slug;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::SystemTime;

const LIB_BEFORE: &str = "pub fn old_helper(a: i32) -> i32 {\n    let doubled = a * 2;\n    doubled + 40\n}\n\npub fn resize_it(w: u32) -> u32 {\n    w + 1\n}\n\npub fn keep_me(a: i32) -> i32 {\n    a + 1\n}\n";
const LIB_AFTER: &str = "pub fn fresh_helper(a: i32) -> i32 {\n    let doubled = a * 2;\n    doubled + 40\n}\n\npub fn resize_it(w: u32, h: u32) -> u32 {\n    w + h\n}\n\npub fn keep_me(a: i32) -> i32 {\n    a + 2\n}\n";
const DANGLING_USER: &str = "use crate::old_helper;\n\nfn use_old() -> i32 {\n    old_helper(1)\n}\n";
const STALE_USER: &str = "use crate::resize_it;\n\nfn draw_box() -> u32 {\n    resize_it(3)\n}\n";
const STALE_USER_FIXED: &str = "use crate::resize_it;\n\nfn draw_box() -> u32 {\n    resize_it(3, 4)\n}\n";
const LOST: &str = "pub fn lost_fn(a: i32) -> i32 {\n    a\n}\n\npub fn kept_fn() -> i32 {\n    2\n}\n";
const LOST_AFTER: &str = "// PRIVATE-CODE\npub fn kept_fn() -> i32 {\n    2\n}\n";
const LOST_USER: &str = "use crate::lost_fn;\n\nfn use_lost() -> i32 {\n    lost_fn(1)\n}\n";
const KEEP_TEST: &str = "use crate::keep_me;\n\n#[test]\nfn keep_works() {\n    let v = keep_me(1);\n    assert_eq!(v, 2);\n}\n";
const PROMPT: &str = "PRIVATE-PROMPT please change resize_it in the lib to take a height and rename the old helper";

const BASE_DATE: &str = "2026-01-01T00:00:00Z";
const LANDING_DATE: &str = "2026-01-10T11:00:00Z";
const LATER_DATE: &str = "2026-01-15T00:00:00Z";

struct Sandbox {
    _tmp: tempfile::TempDir,
    repo: PathBuf,
    home: PathBuf,
    data: PathBuf,
    temp: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let (repo, home, data, temp) = (tmp.path().join("repo"), tmp.path().join("home"), tmp.path().join("data"), tmp.path().join("temp"));
        for dir in [&repo, &home, &data, &temp] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let s = Self { _tmp: tmp, repo, home, data, temp };
        s.git(&["init", "-q"]);
        s
    }

    fn isolate<'c>(&self, cmd: &'c mut Command) -> &'c mut Command {
        cmd.env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.home.join("gitconfig"))
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CLAUDE_CONFIG_DIR", self.home.join(".claude"))
            .env("CODEX_HOME", self.home.join(".codex"))
            .env("LANTERN_DATA_DIR", &self.data)
            .env("TEMP", &self.temp)
            .env("TMP", &self.temp)
            .env("TMPDIR", &self.temp)
            .env_remove("LANTERN_INDEX_DIR")
    }

    fn git_in(&self, dir: &Path, args: &[&str]) -> String {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(dir).args(["-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "core.autocrlf=false", "-c", "commit.gpgsign=false"]);
        let out = self.isolate(&mut cmd).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn git(&self, args: &[&str]) -> String {
        self.git_in(&self.repo, args)
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.repo.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// 이 경로들만 정한 시각으로 커밋한다
    fn commit_at(&self, date: &str, paths: &[&str]) {
        let mut add = vec!["add", "--"];
        add.extend_from_slice(paths);
        self.git(&add);
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(&self.repo).args(["-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false", "commit", "-q", "-m", "c"]);
        let out = self.isolate(&mut cmd).env("GIT_COMMITTER_DATE", date).env("GIT_AUTHOR_DATE", date).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }

    fn root(&self) -> String {
        self.git(&["rev-parse", "--show-toplevel"]).trim().to_string()
    }

    fn base_repo(&self) {
        for (rel, text) in [
            ("src/lib.rs", LIB_BEFORE),
            ("src/dangling_user.rs", DANGLING_USER),
            ("src/stale_user.rs", STALE_USER),
            ("src/lost.rs", LOST),
            ("src/lost_user.rs", LOST_USER),
            ("tests/keep_test.rs", KEEP_TEST),
        ] {
            self.write(rel, text);
        }
        self.commit_at(BASE_DATE, &["."]);
    }

    /// 세션이 고친 것 중 `src/lib.rs`만 커밋된다. `src/lost.rs`의 변경은 작업 트리에도 남지 않는다
    fn land_lib(&self) {
        self.write("src/lib.rs", LIB_AFTER);
        self.commit_at(LANDING_DATE, &["src/lib.rs"]);
    }

    fn lantern(&self, args: &[&str]) -> Output {
        self.lantern_in(&self.repo, args)
    }

    fn lantern_in(&self, dir: &Path, args: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_lantern"));
        cmd.arg("-C").arg(dir).arg("autopsy").args(args).stdin(Stdio::null());
        // 디버그 빌드가 바쁜 PC에서 시간 상한에 걸리면 시험이 되지 않는다
        if !args.contains(&"--max-seconds") {
            cmd.args(["--max-seconds", "86400"]);
        }
        self.isolate(&mut cmd).output().unwrap()
    }

    fn write_session(&self, id: &str, log: &Log) {
        let dir = self.home.join(".claude/projects").join(slug(Path::new(&self.root())));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{id}.jsonl")), log.lines.join("\n") + "\n").unwrap();
    }

    fn json(&self, args: &[&str]) -> Value {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        let out = self.lantern(&all);
        assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)))
    }
}

/// 합성 Claude Code 세션 기록
struct Log {
    root: String,
    lines: Vec<String>,
    n: u32,
    /// 시각의 앞부분. 초는 차례로 붙인다
    day: String,
}

impl Log {
    fn new(s: &Sandbox, day: &str) -> Self {
        Self { root: s.root(), lines: Vec::new(), n: 0, day: day.to_string() }
    }

    fn ts(&self) -> String {
        format!("{}T10:{:02}:{:02}Z", self.day, self.n / 60, self.n % 60)
    }

    fn prompt(&mut self, text: &str) {
        self.n += 1;
        self.lines.push(json!({"type": "user", "uuid": format!("p{}", self.n), "timestamp": self.ts(), "cwd": self.root, "message": {"role": "user", "content": text}}).to_string());
    }

    fn tool(&mut self, model: &str, name: &str, input: Value, result: Value) {
        self.n += 1;
        let (ts, n, root) = (self.ts(), self.n, self.root.clone());
        let usage = json!({"input_tokens": 100, "output_tokens": 10, "cache_read_input_tokens": 1000, "cache_creation_input_tokens": 50});
        self.lines.push(json!({"type": "assistant", "uuid": format!("a{n}"), "timestamp": ts, "cwd": root,
            "message": {"id": format!("m{n}"), "model": model, "usage": usage, "content": [{"type": "tool_use", "id": format!("t{n}"), "name": name, "input": input}]}}).to_string());
        self.lines.push(json!({"type": "user", "uuid": format!("u{n}"), "timestamp": ts, "cwd": root,
            "message": {"content": [{"type": "tool_result", "tool_use_id": format!("t{n}"), "content": "PRIVATE-TOOL-OUTPUT"}]}, "toolUseResult": result}).to_string());
    }

    fn read(&mut self, rel: &str) {
        let path = format!("{}/{rel}", self.root);
        self.tool("claude-opus-5-5", "Read", json!({"file_path": path}), json!({"type": "text"}));
    }

    /// `originalFile`이 없는 편집. 기준 커밋 본문 위에 적용해야 한다
    fn edit(&mut self, rel: &str, old: &str, new: &str) {
        let path = format!("{}/{rel}", self.root);
        self.tool(
            "claude-opus-5-5",
            "Edit",
            json!({"file_path": path, "old_string": old, "new_string": new}),
            json!({"filePath": path, "oldString": old, "newString": new, "originalFile": null, "structuredPatch": [], "replaceAll": false}),
        );
    }

    fn write_file(&mut self, rel: &str, original: &str, content: &str) {
        let path = format!("{}/{rel}", self.root);
        self.tool(
            "claude-opus-5-5",
            "Write",
            json!({"file_path": path, "content": content}),
            json!({"type": "update", "filePath": path, "content": content, "originalFile": original}),
        );
    }

    fn bash(&mut self, command: &str, result: Value) {
        self.tool("claude-opus-5-5", "Bash", json!({"command": command}), result);
    }
}

/// `src/lib.rs`를 고치고 `src/lost.rs`에서 함수를 지우는 세션
fn main_session(s: &Sandbox) -> Log {
    let mut log = Log::new(s, "2026-01-10");
    log.prompt(PROMPT);
    log.read("src/lib.rs");
    log.edit("src/lib.rs", "pub fn old_helper(a: i32)", "pub fn fresh_helper(a: i32)");
    log.edit("src/lib.rs", "pub fn resize_it(w: u32) -> u32 {\n    w + 1", "pub fn resize_it(w: u32, h: u32) -> u32 {\n    w + h");
    log.edit("src/lib.rs", "    a + 1\n", "    a + 2\n");
    log.write_file("src/lost.rs", LOST, LOST_AFTER);
    // 가격표에 없는 모델
    log.tool("mystery-model", "Read", json!({"file_path": format!("{}/src/lib.rs", log.root)}), json!({"type": "text"}));
    log
}

/// 손댄 파일과 시각을 그대로 옮긴 목록
fn snapshot(dir: &Path) -> BTreeMap<String, (u64, SystemTime)> {
    fn walk(dir: &Path, out: &mut BTreeMap<String, (u64, SystemTime)>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let path = e.path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                let m = std::fs::metadata(&path).unwrap();
                out.insert(path.to_string_lossy().into_owned(), (m.len(), m.modified().unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, &mut out);
    out
}

fn finding<'a>(v: &'a Value, symbol: &str) -> &'a Value {
    v["findings"].as_array().unwrap().iter().find(|f| f["symbol"] == symbol).unwrap_or_else(|| panic!("{symbol}: {v:#}"))
}

#[test]
fn end_to_end_finds_one_dangling_one_stale_and_one_not_landed() {
    let s = Sandbox::new();
    s.base_repo();
    s.write_session("session-main", &main_session(&s));
    s.land_lib();
    // 놓친 호출자를 닷새 뒤에 고쳤다
    s.write("src/stale_user.rs", STALE_USER_FIXED);
    s.commit_at(LATER_DATE, &["src/stale_user.rs"]);

    let v = s.json(&[]);
    assert_eq!(v["sessions"]["with_code_edits"], 1, "{v:#}");
    assert_eq!(v["sessions"]["analyzed"], 1, "{v:#}");
    assert_eq!(v["sessions"]["agents"]["claude"], 1);
    assert_eq!(v["sessions"]["period_from"], "2026-01-10");
    assert_eq!(v["timed_out"], false);

    let m = &v["missed"];
    assert_eq!((m["dangling"].as_u64(), m["stale_caller"].as_u64(), m["not_landed"].as_u64()), (Some(1), Some(1), Some(1)), "{v:#}");
    assert_eq!(m["already_resolved"], 0, "{v:#}");
    assert_eq!(m["stale_caller_later_fixed"], 1, "{v:#}");
    assert_eq!(m["dangling_later_fixed"], 0, "{v:#}");

    let dangling = finding(&v, "old_helper");
    assert_eq!(dangling["kind"], "dangling");
    assert_eq!(dangling["confidence"], "high");
    assert_eq!(dangling["renamed_to"], "fresh_helper");
    assert_eq!(dangling["path"], "src/dangling_user.rs");
    assert_eq!(dangling["line"], 4);
    assert_eq!(dangling["caller"], "use_old");
    assert_eq!(dangling["later_fixed"], false);

    let stale = finding(&v, "resize_it");
    assert_eq!(stale["kind"], "stale_caller");
    assert_eq!(stale["confidence"], "medium");
    assert_eq!(stale["path"], "src/stale_user.rs");
    assert_eq!(stale["caller"], "draw_box");
    assert_eq!(stale["later_fixed"], true);
    assert_eq!(v["findings"].as_array().unwrap().len(), 2, "{v:#}");

    // 바뀐 심볼에 이어진 테스트가 있는데 마지막 편집 뒤 돌리지 않았다
    assert_eq!(v["tests"]["periods_with_related_tests"], 1, "{v:#}");
    assert_eq!(v["tests"]["tests_not_run"], 1, "{v:#}");

    // 첫 편집 앞에 읽기 하나
    assert_eq!(v["search_cost"]["sessions"], 1);
    assert_eq!(v["search_cost"]["median_tool_calls"], 1, "{v:#}");

    // 첫 질문은 src/lib.rs의 심볼을 말한다
    assert_eq!(v["replay"]["ran"], 1, "{v:#}");
    assert_eq!(v["replay"]["edited_file_in_first_3"], 1, "{v:#}");
    assert!(v["replay"]["note"].as_str().unwrap().contains("not an agent run"));

    // 토큰: 알려진 모델 줄 5개와 모르는 모델 줄 1개
    let tok = &v["tokens"];
    assert_eq!((tok["input"].as_u64(), tok["output"].as_u64(), tok["cache_read"].as_u64(), tok["cache_write"].as_u64()), (Some(600), Some(60), Some(6000), Some(300)), "{v:#}");
    let c = &v["cost"];
    assert_eq!(c["unknown_models"], 1, "{v:#}");
    assert_eq!(c["unknown_model_tokens"], 1160, "{v:#}");
    assert_eq!(c["table_date"], "2026-11-02");
    let expected = 5.0 * (100.0 * 4.0 + 10.0 * 20.0 + 1000.0 * 0.2 + 50.0 * 5.0) / 1_000_000.0;
    assert!((c["estimated_usd"].as_f64().unwrap() - expected).abs() < 1e-9, "{c:#}");
    let row = &v["per_period"][0];
    assert_eq!(row["files_changed"], 2, "{row:#}");
    assert_eq!(row["files_from_git"], 0, "{row:#}");
}

#[test]
fn original_repository_data_and_temp_stay_untouched() {
    let s = Sandbox::new();
    s.base_repo();
    s.write_session("session-main", &main_session(&s));
    s.land_lib();
    // 커밋하지 않은 변경과 새 파일이 있어도 그대로여야 한다
    s.write("src/stale_user.rs", STALE_USER_FIXED);
    s.write("src/untracked.rs", "fn fresh_one() {}\n");

    let head = s.git(&["rev-parse", "HEAD"]);
    let config = std::fs::read(s.repo.join(".git/config")).unwrap();
    let head_file = std::fs::read(s.repo.join(".git/HEAD")).unwrap();
    let index_mtime = std::fs::metadata(s.repo.join(".git/index")).unwrap().modified().unwrap();
    let files = snapshot(&s.repo);

    let v = s.json(&[]);
    assert_eq!(v["sessions"]["analyzed"], 1, "{v:#}");

    assert_eq!(std::fs::read(s.repo.join(".git/config")).unwrap(), config);
    assert_eq!(std::fs::read(s.repo.join(".git/HEAD")).unwrap(), head_file);
    assert_eq!(std::fs::metadata(s.repo.join(".git/index")).unwrap().modified().unwrap(), index_mtime, "인덱스 시각이 바뀌었다");
    // `.git` 안까지 파일 하나 시각 하나 바뀌지 않는다. 인덱스 폴더도 만들지 않는다
    assert_eq!(snapshot(&s.repo), files);
    assert!(!s.repo.join(".lantern").exists());
    assert_eq!(s.git(&["rev-parse", "HEAD"]), head);

    // 임시 클론과 임시 인덱스를 지웠고, 데이터 폴더에 아무것도 남기지 않았다
    assert_eq!(std::fs::read_dir(&s.temp).unwrap().count(), 0, "임시 폴더가 남았다");
    assert_eq!(std::fs::read_dir(&s.data).unwrap().count(), 0, "데이터 폴더에 썼다");
    assert!(!s.home.join(".lantern").exists());
}

#[test]
fn no_message_command_or_code_text_leaves_the_process() {
    let s = Sandbox::new();
    s.base_repo();
    let mut log = main_session(&s);
    log.bash("PRIVATE-COMMAND --flag", json!({"stdout": "PRIVATE-STDOUT"}));
    s.write_session("session-main", &log);
    s.land_lib();
    for args in [&["--json"][..], &[][..], &["--details"][..], &["--lang", "ko", "--details"][..]] {
        let out = s.lantern(args);
        assert_eq!(out.status.code(), Some(0));
        let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
        for secret in ["PRIVATE-PROMPT", "PRIVATE-CODE", "PRIVATE-COMMAND", "PRIVATE-STDOUT", "PRIVATE-TOOL-OUTPUT", "a * 2", "please change"] {
            assert!(!text.contains(secret), "{args:?} {secret}:\n{text}");
        }
    }
}

#[test]
fn history_drops_what_was_already_fixed_when_it_landed() {
    let s = Sandbox::new();
    s.base_repo();
    s.write_session("session-main", &main_session(&s));
    // 세션이 모르는 사이 같은 커밋에서 호출자들이 함께 고쳐졌다
    s.write("src/lib.rs", LIB_AFTER);
    s.write("src/dangling_user.rs", "use crate::fresh_helper;\n\nfn use_old() -> i32 {\n    fresh_helper(1)\n}\n");
    s.write("src/stale_user.rs", STALE_USER_FIXED);
    s.commit_at(LANDING_DATE, &["src/lib.rs", "src/dangling_user.rs", "src/stale_user.rs"]);

    let v = s.json(&[]);
    assert_eq!(v["findings"].as_array().unwrap().len(), 0, "{v:#}");
    assert_eq!(v["missed"]["already_resolved"], 2, "{v:#}");
    assert_eq!(v["missed"]["not_landed"], 1, "{v:#}");
    assert_eq!(v["missed"]["raw"], 3, "{v:#}");
}

#[test]
fn dangling_is_dropped_when_the_name_is_defined_again_at_landing() {
    let s = Sandbox::new();
    s.base_repo();
    s.write_session("session-main", &main_session(&s));
    s.write("src/lib.rs", LIB_AFTER);
    s.write("src/moved.rs", "pub fn old_helper(a: i32) -> i32 {\n    a\n}\n");
    s.commit_at(LANDING_DATE, &["src/lib.rs", "src/moved.rs"]);
    let v = s.json(&[]);
    assert_eq!(v["missed"]["dangling"], 0, "{v:#}");
    assert_eq!(v["missed"]["already_resolved"], 1, "{v:#}");
}

#[test]
fn signature_that_did_not_land_is_not_a_finding() {
    let s = Sandbox::new();
    s.base_repo();
    s.write_session("session-main", &main_session(&s));
    // 커밋된 쪽의 시그니처는 세션이 쓴 것이 아니다
    s.write("src/lib.rs", "pub fn fresh_helper(a: i32) -> i32 {\n    let doubled = a * 2;\n    doubled + 40\n}\n\npub fn resize_it(w: u32) -> u32 {\n    w + 1\n}\n\npub fn keep_me(a: i32) -> i32 {\n    a + 2\n}\n");
    s.commit_at(LANDING_DATE, &["src/lib.rs"]);
    let v = s.json(&[]);
    assert_eq!(v["missed"]["stale_caller"], 0, "{v:#}");
    assert_eq!(v["missed"]["not_landed"], 2, "{v:#}");
    assert_eq!(v["missed"]["dangling"], 1, "{v:#}");
}

#[test]
fn test_command_after_the_last_edit_counts_and_earlier_one_does_not() {
    let s = Sandbox::new();
    s.base_repo();
    let mut log = Log::new(&s, "2026-01-10");
    log.prompt(PROMPT);
    log.bash("cargo test -p something", json!({"stdout": ""}));
    log.edit("src/lib.rs", "    a + 1\n", "    a + 2\n");
    s.write_session("session-early", &log);
    let v = s.json(&[]);
    assert_eq!((v["tests"]["periods_with_related_tests"].as_u64(), v["tests"]["tests_not_run"].as_u64()), (Some(1), Some(1)), "{v:#}");

    log.bash("cargo test -p something", json!({"stdout": ""}));
    s.write_session("session-early", &log);
    let v = s.json(&[]);
    assert_eq!((v["tests"]["periods_with_related_tests"].as_u64(), v["tests"]["tests_not_run"].as_u64()), (Some(1), Some(0)), "{v:#}");
    // 본문만 바뀌었으니 놓친 호출자로 보고하지 않는다
    assert_eq!(v["missed"]["raw"], 0, "{v:#}");
}

#[test]
fn shell_edit_takes_the_end_state_from_the_landing_commit() {
    let s = Sandbox::new();
    s.base_repo();
    let mut log = Log::new(&s, "2026-01-10");
    log.prompt(PROMPT);
    let root = log.root.clone();
    log.bash("python rewrite.py", json!({"stdout": "", "bashEditDiff": {"files": [], "changedFiles": [format!("{root}/src/lib.rs")]}}));
    s.write_session("session-shell", &log);
    s.land_lib();
    let v = s.json(&[]);
    assert_eq!(v["per_period"][0]["files_from_git"], 1, "{v:#}");
    assert_eq!(finding(&v, "old_helper")["path"], "src/dangling_user.rs");
    assert_eq!(v["missed"]["not_landed"], 0, "{v:#}");
}

#[test]
fn session_older_than_the_first_commit_starts_from_the_first_commit() {
    // 첫 커밋이 세션의 결과를 이미 담고 있다. 시작 본문은 기록에만 있다
    let s = Sandbox::new();
    s.write("src/lib.rs", LIB_AFTER);
    s.write("src/dangling_user.rs", DANGLING_USER);
    s.commit_at(BASE_DATE, &["."]);
    let mut log = Log::new(&s, "2025-12-30");
    log.prompt(PROMPT);
    log.write_file("src/lib.rs", LIB_BEFORE, LIB_AFTER);
    s.write_session("session-old", &log);
    let v = s.json(&[]);
    assert_eq!((v["sessions"]["analyzed"].as_u64(), v["sessions"]["skipped"]["no_base"].as_u64()), (Some(1), Some(0)), "{v:#}");
    assert_eq!(v["per_period"][0]["base_after_first_edit"], true, "{v:#}");
    assert_eq!(finding(&v, "old_helper")["path"], "src/dangling_user.rs");
}

#[test]
fn long_session_is_split_and_a_miss_fixed_in_a_later_period_still_shows() {
    let s = Sandbox::new();
    s.base_repo();
    // 첫 구간: 이름을 바꾸고 호출자를 놓친 채 커밋
    let mut log = Log::new(&s, "2026-01-10");
    log.prompt(PROMPT);
    log.edit("src/lib.rs", "pub fn old_helper(a: i32)", "pub fn fresh_helper(a: i32)");
    let renamed = LIB_BEFORE.replace("pub fn old_helper", "pub fn fresh_helper");
    s.write("src/lib.rs", &renamed);
    s.commit_at(LANDING_DATE, &["src/lib.rs"]);
    // 이틀 뒤 같은 세션: 놓친 호출자를 고치고 시그니처를 바꾼다
    log.day = "2026-01-12".to_string();
    let (old_sig, new_sig) = ("pub fn resize_it(w: u32) -> u32 {\n    w + 1", "pub fn resize_it(w: u32, h: u32) -> u32 {\n    w + h");
    log.edit("src/dangling_user.rs", "old_helper", "fresh_helper");
    log.edit("src/lib.rs", old_sig, new_sig);
    s.write_session("session-long", &log);
    s.write("src/dangling_user.rs", &DANGLING_USER.replace("old_helper", "fresh_helper"));
    s.write("src/lib.rs", &renamed.replace(old_sig, new_sig));
    s.commit_at("2026-01-12T11:00:00Z", &["src/dangling_user.rs", "src/lib.rs"]);

    let v = s.json(&[]);
    let c = &v["sessions"];
    assert_eq!((c["analyzed"].as_u64(), c["periods_total"].as_u64(), c["periods_analyzed"].as_u64()), (Some(1), Some(2), Some(2)), "{v:#}");
    // 처음과 끝만 비교했다면 호출자를 손댄 것으로 쳐서 보이지 않았을 곳이다
    let dangling = finding(&v, "old_helper");
    assert_eq!(dangling["path"], "src/dangling_user.rs");
    assert_eq!(dangling["later_fixed"], true, "{v:#}");
    let stale = finding(&v, "resize_it");
    assert_eq!(stale["path"], "src/stale_user.rs");
    assert_eq!(stale["later_fixed"], false);
    assert_eq!(v["findings"].as_array().unwrap().len(), 2, "{v:#}");
    // 여러 구간에 걸쳐 고친 파일은 git에서, 한 구간만 고친 파일은 기록에서 본다
    let rows = v["per_period"].as_array().unwrap();
    assert_eq!((rows[0]["files_from_git"].as_u64(), rows[1]["files_from_git"].as_u64()), (Some(1), Some(1)), "{v:#}");
    assert_eq!((rows[0]["files_changed"].as_u64(), rows[1]["files_changed"].as_u64()), (Some(1), Some(2)), "{v:#}");
    let text = String::from_utf8_lossy(&s.lantern(&["--lang", "en"]).stdout).into_owned();
    assert!(text.contains("Analyzed 1 of 1 sessions (2 of 2 work periods)"), "{text}");
}

#[test]
fn edit_with_a_much_later_commit_is_not_read_from_the_worktree() {
    // 셸로 고친 파일이 기한 안에 커밋되지 않았고 한참 뒤의 커밋이 그 파일을 바꿨다. 작업 트리를 끝으로 보면 남의 변경이 세션 탓이 된다
    let s = Sandbox::new();
    s.base_repo();
    let mut log = Log::new(&s, "2026-01-10");
    log.prompt(PROMPT);
    let root = log.root.clone();
    log.bash("python rewrite.py", json!({"stdout": "", "bashEditDiff": {"files": [], "changedFiles": [format!("{root}/src/lib.rs")]}}));
    s.write_session("session-shell", &log);
    s.write("src/lib.rs", LIB_AFTER);
    s.commit_at("2026-03-01T00:00:00Z", &["src/lib.rs"]);
    let v = s.json(&[]);
    assert_eq!(v["missed"]["raw"], 0, "{v:#}");
    assert_eq!(v["sessions"]["skipped"]["no_text"], 1, "{v:#}");
}

#[test]
fn time_cap_reports_k_of_n() {
    let s = Sandbox::new();
    s.base_repo();
    s.write_session("session-main", &main_session(&s));
    s.land_lib();
    let v = s.json(&["--max-seconds", "0"]);
    assert_eq!((v["sessions"]["analyzed"].as_u64(), v["sessions"]["with_code_edits"].as_u64()), (Some(0), Some(1)), "{v:#}");
    assert_eq!(v["sessions"]["skipped"]["time_cap"], 1);
    assert_eq!(v["timed_out"], true);
    // 토큰은 이미 읽었으니 센다
    assert_eq!(v["tokens"]["sessions"], 1);

    let text = String::from_utf8_lossy(&s.lantern(&["--max-seconds", "0", "--lang", "en"]).stdout).into_owned();
    assert!(text.contains("Analyzed 0 of 1 sessions") && text.contains("time cap"), "{text}");
}

#[test]
fn replay_can_be_turned_off_and_short_prompts_are_skipped() {
    let s = Sandbox::new();
    s.base_repo();
    s.write_session("session-main", &main_session(&s));
    s.land_lib();
    let v = s.json(&["--no-replay"]);
    assert_eq!((v["replay"]["enabled"].as_bool(), v["replay"]["ran"].as_u64()), (Some(false), Some(0)), "{v:#}");

    let mut log = Log::new(&s, "2026-01-10");
    log.prompt("go");
    log.edit("src/lib.rs", "    a + 1\n", "    a + 2\n");
    s.write_session("session-main", &log);
    let v = s.json(&[]);
    assert_eq!((v["replay"]["ran"].as_u64(), v["replay"]["skipped_prompt"].as_u64()), (Some(0), Some(1)), "{v:#}");
}

#[test]
fn text_reports_fit_a_screen_in_both_languages() {
    let s = Sandbox::new();
    s.base_repo();
    s.write_session("session-main", &main_session(&s));
    s.land_lib();
    let en = String::from_utf8_lossy(&s.lantern(&["--lang", "en"]).stdout).into_owned();
    assert!(en.lines().count() <= 24, "{en}");
    assert!(en.contains("Analyzed 1 of 1 sessions") && en.contains("src/dangling_user.rs:4") && en.contains("old_helper"), "{en}");
    assert!(en.contains("name based") && en.contains("lantern hook install claude") && en.contains("--details"), "{en}");
    assert!(en.contains("1 of 1 work periods with related tests") && en.contains("not an agent run") && en.contains("Unknown model: 1"), "{en}");

    let ko = String::from_utf8_lossy(&s.lantern(&["--lang", "ko"]).stdout).into_owned();
    assert!(ko.lines().count() <= 24, "{ko}");
    assert!(ko.contains("세션 1개 중 1개 분석") && ko.contains("src/dangling_user.rs:4") && ko.contains("이름 기반"), "{ko}");

    let details = String::from_utf8_lossy(&s.lantern(&["--lang", "en", "--details"]).stdout).into_owned();
    assert!(details.contains("session session-") && details.contains("use_old") && details.contains("Skipped sessions"), "{details}");
}

#[test]
fn no_sessions_needs_no_clone_and_says_so() {
    let s = Sandbox::new();
    s.base_repo();
    let v = s.json(&[]);
    assert_eq!(v["sessions"]["with_code_edits"], 0);
    assert_eq!(v["findings"].as_array().unwrap().len(), 0);
    let text = String::from_utf8_lossy(&s.lantern(&["--lang", "en"]).stdout).into_owned();
    assert!(text.contains("No session edited code"), "{text}");
    assert_eq!(std::fs::read_dir(&s.temp).unwrap().count(), 0);
}

#[test]
fn not_a_repository_is_an_error_not_a_crash() {
    let s = Sandbox::new();
    let plain = s.home.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let out = s.lantern_in(&plain, &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("git"), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn linked_worktree_uses_its_own_head_and_stays_untouched() {
    let s = Sandbox::new();
    s.base_repo();
    s.write_session("session-main", &main_session(&s));
    s.land_lib();
    let wt = s.home.join("wt");
    s.git(&["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "side"]);
    let before = snapshot(&s.repo);
    let out = s.lantern_in(&wt, &["--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["sessions"]["analyzed"], 1, "{v:#}");
    assert_eq!(v["head"].as_str().unwrap(), s.git_in(&wt, &["rev-parse", "HEAD"]).trim());
    assert_eq!(finding(&v, "old_helper")["path"], "src/dangling_user.rs");
    assert_eq!(snapshot(&s.repo), before);
    assert_eq!(std::fs::read_dir(&s.temp).unwrap().count(), 0);
}

/// 합성 Codex 세션 기록. 읽기를 모르고 시작 본문이 없다
#[test]
fn codex_session_uses_the_base_blob_and_does_not_claim_unread() {
    let s = Sandbox::new();
    s.base_repo();
    let root = s.root();
    let at = |sec: u32, kind: &str, payload: Value| json!({"timestamp": format!("2026-01-10T10:00:{sec:02}Z"), "type": kind, "payload": payload}).to_string();
    let path = format!("{root}/src/lib.rs");
    let diff = "@@ -6,3 +6,3 @@\n-pub fn resize_it(w: u32) -> u32 {\n-    w + 1\n+pub fn resize_it(w: u32, h: u32) -> u32 {\n+    w + h\n }\n";
    let mut changes = serde_json::Map::new();
    changes.insert(path, json!({"type": "update", "unified_diff": diff}));
    let lines = [
        at(0, "session_meta", json!({"id": "codex-one", "cwd": root})),
        at(1, "turn_context", json!({"cwd": root, "model": "gpt-5.5"})),
        at(2, "event_msg", json!({"type": "user_message", "message": PROMPT})),
        at(3, "event_msg", json!({"type": "item_completed", "item": {"type": "FileChange", "id": "c1", "status": "completed", "changes": changes}})),
        at(4, "event_msg", json!({"type": "token_count", "info": {"total_token_usage": {"input_tokens": 300, "cached_input_tokens": 100, "output_tokens": 20}}})),
    ];
    let dir = s.home.join(".codex/sessions/2026/01/10");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("rollout-2026-01-10-codex-one.jsonl"), lines.join("\n") + "\n").unwrap();
    s.land_lib();

    let v = s.json(&["--agent", "codex"]);
    assert_eq!(v["sessions"]["agents"]["codex"], 1, "{v:#}");
    let stale = finding(&v, "resize_it");
    assert_eq!(stale["kind"], "stale_caller");
    assert!(!stale["why"].as_str().unwrap().contains("read"), "{stale:#}");
    assert_eq!(v["per_period"][0]["reads_known"], false);
    assert_eq!(v["tokens"]["input"], 200);
    assert_eq!(v["tokens"]["cache_read"], 100);
    let expected = (200.0 * 5.0 + 20.0 * 30.0 + 100.0 * 0.5) / 1_000_000.0;
    assert!((v["cost"]["estimated_usd"].as_f64().unwrap() - expected).abs() < 1e-9, "{:#}", v["cost"]);
}
