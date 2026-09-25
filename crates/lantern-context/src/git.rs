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

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = no_window(&mut Command::new("git"))
        .arg("-C")
        .arg(root)
        .args(["-c", "core.quotepath=off"])
        .args(args)
        .output()?;
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

    let log = git(
        root,
        &[
            "log",
            "--no-merges",
            "--relative",
            "--name-only",
            "--format=%x00",
            "-n",
            &MAX_COMMITS.to_string(),
        ],
    )?;
    let pairs = cochange_pairs(&log);
    store.begin()?;
    store.replace_cochange(&pairs)?;
    store.set_meta("git_head", head)?;
    store.commit()?;
    Ok(Some(pairs.len()))
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
}
