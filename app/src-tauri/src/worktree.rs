//! 작업 격리. 작업마다 git worktree(별도 폴더 + 브랜치 `lantern/<id>`)에서 에이전트가 일하고,
//! 끝나면 사용자가 변경을 보고 프로젝트에 적용하거나 버린다.
//! 여러 작업이 같은 파일을 동시에 고쳐도 서로 덮어쓰지 않는다.
//!
//! 시작점은 지금 프로젝트 상태 그대로다: HEAD + 커밋하지 않은 변경 + 추적하지 않는 파일.

use crate::gitops::git;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

const MAX_COPY_BYTES: u64 = 5 * 1024 * 1024;
const COMMITTER: [&str; 4] = ["-c", "user.name=Lantern", "-c", "user.email=lantern@localhost"];

#[derive(Debug, Clone, Serialize)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: String,
}

fn base_dir(root: &Path) -> Result<PathBuf> {
    Ok(crate::state::data_dir().context("데이터 폴더를 찾지 못했습니다")?.join("worktrees").join(crate::tasks::project_key(root)))
}

fn short(id: &str) -> String {
    id.chars().filter(|c| c.is_ascii_alphanumeric()).take(8).collect()
}

pub fn create(root: &Path, task_id: &str) -> Result<Worktree> {
    create_in(root, &base_dir(root)?, task_id)
}

fn create_in(root: &Path, base: &Path, task_id: &str) -> Result<Worktree> {
    if git(root, &["rev-parse", "--verify", "HEAD"]).is_err() {
        bail!("격리하려면 git 저장소에 커밋이 하나 이상 있어야 합니다");
    }
    let s = short(task_id);
    let path = base.join(&s);
    let branch = format!("lantern/{s}");
    std::fs::create_dir_all(base)?;
    let p = path.to_string_lossy().into_owned();
    git(root, &["worktree", "add", "-q", "-b", &branch, &p, "HEAD"]).context("작업 공간을 만들지 못했습니다")?;

    // 커밋하지 않은 변경도 가져간다
    let patch = git(root, &["diff", "HEAD", "--binary"])?;
    if !patch.trim().is_empty() {
        let file = base.join(format!("{s}.start.patch"));
        std::fs::write(&file, &patch)?;
        let r = git(&path, &["apply", "--binary", "--whitespace=nowarn", &file.to_string_lossy()]);
        let _ = std::fs::remove_file(&file);
        r.context("커밋하지 않은 변경을 작업 공간에 옮기지 못했습니다")?;
    }
    // 추적하지 않는 파일
    let untracked = git(root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    for rel in untracked.split('\0').filter(|s| !s.is_empty()) {
        let from = root.join(rel);
        if from.metadata().map(|m| m.len() > MAX_COPY_BYTES).unwrap_or(true) {
            continue;
        }
        let to = path.join(rel);
        if let Some(d) = to.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::copy(&from, &to)?;
    }
    commit_all(&path, "lantern: 작업 시작점")?;
    Ok(Worktree { path, branch })
}

fn commit_all(wt: &Path, message: &str) -> Result<()> {
    git(wt, &["add", "-A"])?;
    let mut args: Vec<&str> = COMMITTER.to_vec();
    args.extend(["commit", "-q", "--allow-empty", "--no-verify", "-m", message]);
    git(wt, &args)?;
    Ok(())
}

/// 작업 공간에서 바뀐 것 (시작점 또는 마지막 적용 이후)
pub fn diff(wt: &Path) -> Result<String> {
    git(wt, &["add", "-A"])?;
    git(wt, &["diff", "--cached", "--binary", "HEAD"])
}

pub fn changed_files(wt: &Path) -> Result<Vec<String>> {
    git(wt, &["add", "-A"])?;
    Ok(git(wt, &["diff", "--cached", "--name-only", "-z", "HEAD"])?.split('\0').filter(|s| !s.is_empty()).map(str::to_string).collect())
}

/// 프로젝트에 적용한다. 프로젝트 쪽이 그사이 같은 곳을 바꿨으면 아무것도 바꾸지 않고 알려 준다.
pub fn apply(root: &Path, wt: &Path) -> Result<Vec<String>> {
    let files = changed_files(wt)?;
    if files.is_empty() {
        return Ok(files);
    }
    let patch = diff(wt)?;
    let file = wt.with_extension("apply.patch");
    std::fs::write(&file, &patch)?;
    let f = file.to_string_lossy().into_owned();
    let check = git(root, &["apply", "--check", "--binary", "--whitespace=nowarn", &f]);
    if let Err(e) = check {
        let _ = std::fs::remove_file(&file);
        bail!("프로젝트의 같은 곳이 그사이 바뀌어 적용하지 못했습니다. 변경 보기에서 확인하고 직접 옮기거나, 프로젝트 쪽 변경을 되돌린 뒤 다시 시도하세요.\n{e}");
    }
    let r = git(root, &["apply", "--binary", "--whitespace=nowarn", &f]);
    let _ = std::fs::remove_file(&file);
    r?;
    // 다음 적용은 여기서부터
    commit_all(wt, "lantern: 프로젝트에 적용됨")?;
    Ok(files)
}

/// 작업 공간과 브랜치를 지운다. 이미 없으면 조용히 넘어간다.
pub fn remove(root: &Path, wt: &Path, branch: &str) {
    let p = wt.to_string_lossy().into_owned();
    let _ = git(root, &["worktree", "remove", "--force", &p]);
    let _ = git(root, &["branch", "-D", branch]);
    let _ = std::fs::remove_dir_all(wt);
    let _ = git(root, &["worktree", "prune"]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn repo() -> Option<tempfile::TempDir> {
        Command::new("git").arg("--version").output().ok()?;
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        git(r, &["init", "-q"]).unwrap();
        git(r, &["config", "user.email", "t@example.com"]).unwrap();
        git(r, &["config", "user.name", "t"]).unwrap();
        git(r, &["config", "core.autocrlf", "false"]).unwrap();
        std::fs::write(r.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(r.join("b.txt"), "b\n").unwrap();
        git(r, &["add", "-A"]).unwrap();
        git(r, &["commit", "-qm", "init"]).unwrap();
        Some(d)
    }

    #[test]
    fn isolate_apply_and_remove() {
        let Some(d) = repo() else { return };
        let root = d.path();
        let base = tempfile::tempdir().unwrap();
        // 커밋하지 않은 변경과 새 파일도 작업 공간에 있어야 한다
        std::fs::write(root.join("b.txt"), "b changed\n").unwrap();
        std::fs::write(root.join("new.txt"), "new\n").unwrap();
        let w = create_in(root, base.path(), "task-1234abcd-xyz").unwrap();
        assert_eq!(w.branch, "lantern/task1234");
        assert_eq!(std::fs::read_to_string(w.path.join("b.txt")).unwrap(), "b changed\n");
        assert!(w.path.join("new.txt").exists());
        assert!(changed_files(&w.path).unwrap().is_empty(), "시작점은 변경 없음");

        // 작업 공간에서 고쳐도 프로젝트는 그대로
        std::fs::write(w.path.join("a.txt"), "one\nTWO\nthree\n").unwrap();
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one\ntwo\nthree\n");
        assert_eq!(changed_files(&w.path).unwrap(), vec!["a.txt"]);
        assert!(diff(&w.path).unwrap().contains("+TWO"));

        // 적용
        assert_eq!(apply(root, &w.path).unwrap(), vec!["a.txt"]);
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one\nTWO\nthree\n");
        assert!(changed_files(&w.path).unwrap().is_empty(), "적용 후 다시 시작점");

        // 충돌: 프로젝트와 작업 공간이 같은 줄을 다르게 바꿈
        std::fs::write(w.path.join("a.txt"), "one\nTWO\nTHREE-wt\n").unwrap();
        std::fs::write(root.join("a.txt"), "one\nTWO\nTHREE-main\n").unwrap();
        assert!(apply(root, &w.path).is_err());
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "one\nTWO\nTHREE-main\n", "실패하면 아무것도 바꾸지 않는다");

        remove(root, &w.path, &w.branch);
        assert!(!w.path.exists());
        assert!(git(root, &["rev-parse", "--verify", "lantern/task1234"]).is_err(), "브랜치도 지운다");
    }

    #[test]
    fn needs_a_commit() {
        let Some(_) = repo() else { return };
        let empty = tempfile::tempdir().unwrap();
        git(empty.path(), &["init", "-q"]).unwrap();
        let base = tempfile::tempdir().unwrap();
        assert!(create_in(empty.path(), base.path(), "t1").is_err());
    }
}
