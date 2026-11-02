//! git 이력으로 "함께 자주 바뀌는 파일" 쌍을 계산한다.

use crate::store::Store;
use anyhow::{bail, Result};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

const MAX_COMMITS: usize = 500;
/// 이보다 많은 파일을 건드린 커밋은 대량 변경(포매팅, 이름 변경 등)으로 보고 제외한다.
const MAX_FILES_PER_COMMIT: usize = 30;
const MIN_PAIR_COUNT: u32 = 2;

/// GUI 앱에서 호출할 때 Windows 콘솔 창이 뜨지 않게 한다.
pub fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// 일부만 받은 클론에서 빠진 객체를 받으러 네트워크에 나가지 않게 한다 (`core.sshCommand` 실행과 `.git` 쓰기도 막는다).
/// 이 설정을 모르는 오래된 git에서는 효과가 없다
pub(crate) const NO_LAZY_FETCH: &str = "GIT_NO_LAZY_FETCH";

pub(crate) fn git(root: &Path, args: &[&str]) -> Result<String> {
    run(root, args, true)
}

fn run(root: &Path, args: &[&str], offline: bool) -> Result<String> {
    let mut cmd = Command::new("git");
    no_window(&mut cmd).arg("-C").arg(root).args(["-c", "core.quotepath=off"]).args(args);
    if offline {
        cmd.env(NO_LAZY_FETCH, "1");
    }
    let out = cmd.output()?;
    if !out.status.success() {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// HEAD가 바뀌었을 때만 다시 계산한다. git 저장소가 아니면 `None`.
pub fn refresh_cochange(root: &Path, store: &Store) -> Result<Option<usize>> {
    let Ok(head) = git(root, &["rev-parse", "HEAD"]) else {
        return Ok(None);
    };
    let head = head.trim();
    if store.meta("git_head")?.as_deref() == Some(head) {
        return Ok(None);
    }

    let pairs = cochange_pairs(&cochange_log(root)?);
    store.begin()?;
    store.replace_cochange(&pairs)?;
    store.set_meta("git_head", head)?;
    store.commit()?;
    Ok(Some(pairs.len()))
}

/// 이름 바뀜을 찾으려면 파일 내용이 필요하다. 일부만 받은 클론에서 받지 못하게 하면
/// 그동안 체크아웃해 둔 내용에 따라 결과가 달라지므로 여기서는 받게 둔다
fn cochange_log(root: &Path) -> Result<String> {
    let n = MAX_COMMITS.to_string();
    run(root, &["log", "--no-merges", "--relative", "--name-only", "--format=%x00", "-n", &n], false)
}

/// `git log --name-only --format=%x00` 출력에서 파일 쌍별 동시 변경 횟수를 센다.
fn cochange_pairs(log: &str) -> Vec<(String, String, u32)> {
    let mut counts: HashMap<(String, String), u32> = HashMap::new();
    for commit in log.split('\0') {
        let mut files: Vec<&str> = commit.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        if files.len() < 2 || files.len() > MAX_FILES_PER_COMMIT {
            continue;
        }
        files.sort_unstable();
        files.dedup();
        for i in 0..files.len() {
            for j in i + 1..files.len() {
                *counts.entry((files[i].to_string(), files[j].to_string())).or_default() += 1;
            }
        }
    }
    let mut pairs: Vec<_> = counts
        .into_iter()
        .filter(|(_, n)| *n >= MIN_PAIR_COUNT)
        .map(|((a, b), n)| (a, b, n))
        .collect();
    pairs.sort();
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_pairs() {
        let log = "\0\n\nsrc/a.rs\nsrc/b.rs\n\0\n\nsrc/b.rs\nsrc/a.rs\nsrc/c.rs\n\0\n\nsrc/c.rs\n";
        let pairs = cochange_pairs(log);
        assert_eq!(pairs, vec![("src/a.rs".to_string(), "src/b.rs".to_string(), 2)]);
    }

    /// 내용 없이 받은 클론도 전체 저장소와 같은 쌍을 낸다 (#170)
    #[test]
    fn partial_clone_gives_same_pairs() {
        let g = |dir: &Path, args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        let src = tempfile::tempdir().unwrap();
        let s = src.path();
        g(s, &["init", "-q"]);
        g(s, &["config", "uploadpack.allowfilter", "true"]);
        let body: String = (0..40).map(|i| format!("line {i}\n")).collect();
        std::fs::write(s.join("old.rs"), &body).unwrap();
        std::fs::write(s.join("other.rs"), "a\n").unwrap();
        g(s, &["add", "."]);
        g(s, &["commit", "-q", "-m", "1"]);
        // 내용을 조금 바꾸며 이름을 바꾼다. 이름 바뀜을 찾으려면 두 파일의 내용이 필요하다
        std::fs::remove_file(s.join("old.rs")).unwrap();
        std::fs::write(s.join("new.rs"), format!("{body}line 40\n")).unwrap();
        std::fs::write(s.join("other.rs"), "b\n").unwrap();
        g(s, &["add", "-A"]);
        g(s, &["commit", "-q", "-m", "2"]);
        std::fs::write(s.join("new.rs"), format!("{body}line 41\n")).unwrap();
        std::fs::write(s.join("other.rs"), "c\n").unwrap();
        g(s, &["commit", "-q", "-a", "-m", "3"]);

        let url = format!("file:///{}", s.to_string_lossy().replace('\\', "/").trim_start_matches('/'));
        let dst = tempfile::tempdir().unwrap();
        g(dst.path(), &["clone", "-q", "--filter=blob:none", "--no-checkout", &url, "c"]);

        let full = cochange_pairs(&cochange_log(s).unwrap());
        assert_eq!(full, vec![("new.rs".to_string(), "other.rs".to_string(), 2)]);
        assert_eq!(cochange_pairs(&cochange_log(&dst.path().join("c")).unwrap()), full);
    }
}
