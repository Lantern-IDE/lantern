//! 고친 뒤 확인: 에이전트가 파일을 바꾸고 끝내려 할 때, 영향 반경이 찾은 관련 테스트만 골라 돌린다.
//! 실패하면 출력을 모델에 돌려줘 다시 고치게 하고, 같은 실패가 반복되면 멈추고 사람에게 넘긴다.

use crate::state::{rel_path, Project};
use lantern_context::graph;
use similar::{DiffOp, TextDiff};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 다시 고치게 하는 최대 횟수
pub const MAX_ROUNDS: usize = 2;
/// 모델에 돌려줄 테스트 출력 (끝부분이 실패 원인인 경우가 많다)
const OUTPUT_TAIL: usize = 6000;

/// 한 작업에서 앞서 확인한 것
#[derive(Default)]
pub struct Checked {
    /// 지금까지 관련 테스트로 찾은 파일
    pub tests: BTreeSet<String>,
    /// 마지막 실패의 지문 (같은 실패가 반복되면 멈춘다)
    pub last_failure: Option<String>,
}

/// 바뀐 줄 범위 (1부터). `new_side`면 새 내용 기준, 아니면 원래 내용 기준
pub fn changed_ranges(old: &str, new: &str, new_side: bool) -> Vec<(u32, u32)> {
    let diff = TextDiff::from_lines(old, new);
    let mut out = Vec::new();
    for op in diff.ops() {
        // 지운 줄은 그 자리(새 내용의 다음 줄)를, 넣은 줄은 원래 내용의 그 자리를 바뀐 곳으로 본다
        let (from, len) = match (*op, new_side) {
            (DiffOp::Equal { .. }, _) => continue,
            (DiffOp::Delete { new_index, .. }, true) => (new_index, 1),
            (DiffOp::Delete { old_index, old_len, .. }, false) => (old_index, old_len),
            (DiffOp::Insert { new_index, new_len, .. }, true) => (new_index, new_len),
            (DiffOp::Insert { old_index, .. }, false) => (old_index, 1),
            (DiffOp::Replace { new_index, new_len, .. }, true) => (new_index, new_len),
            (DiffOp::Replace { old_index, old_len, .. }, false) => (old_index, old_len),
        };
        out.push((from as u32 + 1, (from + len.max(1)) as u32));
    }
    out
}

/// 바뀐 파일과 연결된 테스트 파일. 에이전트가 직접 고친 테스트 파일도 넣는다.
/// `isolated`면 인덱스가 원래 코드 기준이라 원래 줄 번호로 찾는다.
pub fn related_tests(project: &Project, root: &Path, changed: &[String], originals: &[(PathBuf, Option<String>)], isolated: bool) -> Vec<String> {
    let mut tests = BTreeSet::new();
    let mut engine = project.engine.lock().unwrap();
    let _ = engine.refresh();
    for rel in changed {
        if graph::is_test_path(rel) {
            if root.join(rel).exists() {
                tests.insert(rel.clone());
            }
            continue;
        }
        let new = std::fs::read_to_string(root.join(rel)).unwrap_or_default();
        let old = originals.iter().find(|(p, _)| rel_path(root, p) == *rel).and_then(|(_, o)| o.clone()).unwrap_or_default();
        let ranges = changed_ranges(&old, &new, !isolated);
        if ranges.is_empty() {
            continue;
        }
        if let Ok(i) = graph::impact(&engine.store, rel, &ranges) {
            tests.extend(i.tests);
        }
    }
    tests.into_iter().filter(|t| root.join(t).exists()).collect()
}

fn quote(p: &str) -> String {
    if p.contains(' ') {
        format!("\"{p}\"")
    } else {
        p.to_string()
    }
}

/// 프로젝트의 테스트 명령. `template`(config의 agent.test_command)이 있으면 `{files}`에 테스트 파일을 넣어 쓴다.
pub fn command(root: &Path, tests: &[String], template: Option<&str>) -> Option<String> {
    let files = tests.iter().map(|t| quote(t)).collect::<Vec<_>>().join(" ");
    if let Some(t) = template.filter(|t| !t.trim().is_empty()) {
        return Some(t.replace("{files}", &files));
    }
    let first = tests.first()?;
    let ext = Path::new(first).extension().and_then(|e| e.to_str()).unwrap_or("");
    let has = |f: &str| root.join(f).exists();
    match ext {
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => {
            let pkg = std::fs::read_to_string(root.join("package.json")).ok()?;
            let deps = |name: &str| pkg.contains(&format!("\"{name}\""));
            if deps("vitest") {
                Some(format!("npx vitest run {files}"))
            } else if deps("jest") {
                Some(format!("npx jest {files}"))
            } else if pkg.contains("\"test\"") && !pkg.contains("no test specified") {
                Some("npm test".into())
            } else {
                None
            }
        }
        "py" => Some(format!("python -m pytest {files}")),
        "go" => {
            let mut pkgs: Vec<String> = tests.iter().map(|t| format!("./{}", Path::new(t).parent().map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default())).collect();
            pkgs.dedup();
            has("go.mod").then(|| format!("go test {}", pkgs.join(" ")))
        }
        "rs" => has("Cargo.toml").then(|| "cargo test".into()),
        "java" => {
            let classes: Vec<String> = tests.iter().filter_map(|t| Path::new(t).file_stem().map(|s| s.to_string_lossy().into_owned())).collect();
            if has("pom.xml") {
                Some(format!("mvn -q test -Dtest={} -Dsurefire.failIfNoSpecifiedTests=false", classes.join(",")))
            } else if has("build.gradle") || has("build.gradle.kts") {
                let gradlew = if cfg!(windows) { "gradlew.bat" } else { "./gradlew" };
                let runner = if has(if cfg!(windows) { "gradlew.bat" } else { "gradlew" }) { gradlew } else { "gradle" };
                Some(format!("{runner} test {}", classes.iter().map(|c| format!("--tests {c}")).collect::<Vec<_>>().join(" ")))
            } else {
                None
            }
        }
        "cs" => Some("dotnet test".into()),
        _ => None,
    }
}

/// run_shell 출력의 종료 코드
pub fn exit_code(output: &str) -> Option<i32> {
    let i = output.rfind("(종료 코드 ")?;
    output[i + "(종료 코드 ".len()..].split(')').next()?.trim().parse().ok()
}

/// 같은 실패인지 비교할 지문: 숫자(시간·줄 번호 외 변하는 값)를 지운 마지막 줄들
pub fn signature(output: &str) -> String {
    let tail: Vec<&str> = output.lines().rev().filter(|l| !l.trim().is_empty()).take(15).collect();
    tail.iter().map(|l| l.chars().filter(|c| !c.is_ascii_digit()).collect::<String>()).collect::<Vec<_>>().join("\n")
}

/// 실패 출력을 모델에 돌려줄 글
pub fn failure_message(cmd: &str, output: &str) -> String {
    let tail: String = {
        let chars: Vec<char> = output.chars().collect();
        chars[chars.len().saturating_sub(OUTPUT_TAIL)..].iter().collect()
    };
    format!(
        "Lantern ran the tests related to your changes and they failed.\n\
         Command: `{cmd}`\n```\n{tail}\n```\n\
         Fix the code so these tests pass. Change a test only if the test itself is wrong, and say so. \
         When done, briefly say what you fixed."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_changed_ranges_on_both_sides() {
        let old = "a\nb\nc\nd\n";
        let new = "a\nB\nc\nx\ny\nd\n";
        assert_eq!(changed_ranges(old, new, true), [(2, 2), (4, 5)]);
        assert_eq!(changed_ranges(old, new, false), [(2, 2), (4, 4)]);
        assert!(changed_ranges(old, old, true).is_empty());
    }

    #[test]
    fn picks_test_commands() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::write(r.join("package.json"), r#"{"devDependencies":{"vitest":"^3"}}"#).unwrap();
        assert_eq!(command(r, &["tests/a.test.ts".into(), "tests/b c.test.ts".into()], None).unwrap(), "npx vitest run tests/a.test.ts \"tests/b c.test.ts\"");
        assert_eq!(command(r, &["test_x.py".into()], None).unwrap(), "python -m pytest test_x.py");
        assert_eq!(command(r, &["x.test.ts".into()], Some("pnpm test -- {files}")).unwrap(), "pnpm test -- x.test.ts");
        std::fs::write(r.join("pom.xml"), "<project/>").unwrap();
        assert!(command(r, &["src/test/java/a/FooTest.java".into()], None).unwrap().starts_with("mvn -q test -Dtest=FooTest"));
        assert!(command(r, &["x.unknown".into()], None).is_none());
    }

    #[test]
    fn reads_exit_codes_and_signatures() {
        assert_eq!(exit_code("FAIL\n(종료 코드 1)\n"), Some(1));
        assert_eq!(exit_code("ok\n(종료 코드 0)\n"), Some(0));
        assert_eq!(exit_code("timeout"), None);
        assert_eq!(signature("x\nTest failed in 12ms\n(종료 코드 1)"), signature("x\nTest failed in 340ms\n(종료 코드 1)"));
    }
}
