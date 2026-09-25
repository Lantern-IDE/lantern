//! 소스 제어 (git 명령줄을 감싼다): 상태, Diff, 스테이징, 되돌리기, 커밋.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::Path;
use std::process::Command;

pub(crate) fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = lantern_context::git::no_window(&mut Command::new("git"))
        .arg("-C")
        .arg(root)
        .args(["-c", "core.quotepath=off", "-c", "color.ui=false"])
        .args(args)
        .output()
        .context("git을 실행하지 못했습니다. Git이 설치되어 있는지 확인하세요")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("{}", err.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[derive(Debug, Serialize, PartialEq)]
pub struct GitFile {
    pub path: String,
    /// 이름이 바뀐 경우 원래 경로
    pub orig: Option<String>,
    /// 스테이징 영역 상태 (M, A, D, R, ' ')
    pub index: String,
    /// 작업 트리 상태 (M, D, ?, ' ')
    pub worktree: String,
}

#[derive(Debug, Serialize)]
pub struct GitStatus {
    pub repo: bool,
    pub branch: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<GitFile>,
}

/// `git status --porcelain=v1 -z -b` 출력 해석
pub fn parse_status(out: &str) -> (Option<String>, u32, u32, Vec<GitFile>) {
    let mut branch = None;
    let (mut ahead, mut behind) = (0, 0);
    let mut files = Vec::new();
    let mut parts = out.split('\0').filter(|s| !s.is_empty());
    while let Some(entry) = parts.next() {
        if let Some(head) = entry.strip_prefix("## ") {
            let name = head.split("...").next().unwrap_or(head);
            branch = Some(name.trim_start_matches("No commits yet on ").to_string());
            for (key, slot) in [("ahead ", &mut ahead), ("behind ", &mut behind)] {
                if let Some(i) = head.find(key) {
                    *slot = head[i + key.len()..].chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0);
                }
            }
            continue;
        }
        if entry.len() < 4 {
            continue;
        }
        let (x, y, path) = (&entry[0..1], &entry[1..2], entry[3..].to_string());
        let orig = if x == "R" || x == "C" { parts.next().map(str::to_string) } else { None };
        files.push(GitFile { path, orig, index: x.into(), worktree: y.into() });
    }
    (branch, ahead, behind, files)
}

pub fn status(root: &Path) -> Result<GitStatus> {
    if git(root, &["rev-parse", "--is-inside-work-tree"]).is_err() {
        return Ok(GitStatus { repo: false, branch: None, ahead: 0, behind: 0, files: vec![] });
    }
    let out = git(root, &["status", "--porcelain=v1", "-z", "-b", "--untracked-files=all"])?;
    let (branch, ahead, behind, files) = parse_status(&out);
    Ok(GitStatus { repo: true, branch, ahead, behind, files })
}

/// 파일 하나의 Diff. 추적하지 않는 새 파일은 전체를 추가로 보여준다.
pub fn diff(root: &Path, path: &str, staged: bool) -> Result<String> {
    let tracked = git(root, &["ls-files", "--error-unmatch", "--", path]).is_ok();
    if !tracked && !staged {
        let text = std::fs::read_to_string(root.join(path)).unwrap_or_default();
        let n = text.lines().count();
        let mut out = format!("--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{n} @@\n");
        for l in text.lines() {
            out.push('+');
            out.push_str(l);
            out.push('\n');
        }
        return Ok(out);
    }
    let mut args = vec!["diff", "--no-ext-diff"];
    if staged {
        args.push("--cached");
    }
    args.extend(["--", path]);
    git(root, &args)
}

pub fn stage(root: &Path, paths: &[String]) -> Result<()> {
    let mut args = vec!["add", "--"];
    args.extend(paths.iter().map(String::as_str));
    git(root, &args).map(|_| ())
}

pub fn unstage(root: &Path, paths: &[String]) -> Result<()> {
    let mut args = vec!["restore", "--staged", "--"];
    args.extend(paths.iter().map(String::as_str));
    // 첫 커밋 전에는 restore --staged가 안 되므로 rm --cached로 대신한다.
    git(root, &args).or_else(|_| {
        let mut alt = vec!["rm", "--cached", "-r", "-q", "--"];
        alt.extend(paths.iter().map(String::as_str));
        git(root, &alt)
    })?;
    Ok(())
}

/// 작업 트리 변경 취소. 추적하는 파일은 마지막 커밋으로 되돌리고, 새 파일은 휴지통으로 보낸다.
pub fn discard(root: &Path, paths: &[String]) -> Result<()> {
    for p in paths {
        if git(root, &["ls-files", "--error-unmatch", "--", p]).is_ok() {
            git(root, &["checkout", "--", p])?;
        } else {
            trash::delete(root.join(p)).with_context(|| format!("{p}을(를) 휴지통으로 옮기지 못했습니다"))?;
        }
    }
    Ok(())
}

pub fn commit(root: &Path, message: &str) -> Result<String> {
    if message.trim().is_empty() {
        bail!("커밋 메시지를 입력하세요");
    }
    git(root, &["commit", "-m", message])
}

pub fn init(root: &Path) -> Result<()> {
    git(root, &["init"]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain() {
        let out = "## main...origin/main [ahead 2, behind 1]\0 M src/a.rs\0A  new.rs\0R  b2.rs\0b.rs\0?? 한글.txt\0";
        let (branch, ahead, behind, files) = parse_status(out);
        assert_eq!(branch.as_deref(), Some("main"));
        assert_eq!((ahead, behind), (2, 1));
        assert_eq!(files.len(), 4);
        assert_eq!(files[0], GitFile { path: "src/a.rs".into(), orig: None, index: " ".into(), worktree: "M".into() });
        assert_eq!(files[2].orig.as_deref(), Some("b.rs"));
        assert_eq!(files[3].worktree, "?");
        assert_eq!(files[3].path, "한글.txt");
    }

    #[test]
    fn real_repo_flow() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        init(root).unwrap();
        git(root, &["config", "user.email", "t@example.com"]).unwrap();
        git(root, &["config", "user.name", "t"]).unwrap();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        let s = status(root).unwrap();
        assert!(s.repo);
        assert_eq!(s.files[0].worktree, "?");
        assert!(diff(root, "a.txt", false).unwrap().contains("+one"));
        stage(root, &["a.txt".into()]).unwrap();
        assert_eq!(status(root).unwrap().files[0].index, "A");
        unstage(root, &["a.txt".into()]).unwrap();
        assert_eq!(status(root).unwrap().files[0].worktree, "?");
        stage(root, &["a.txt".into()]).unwrap();
        commit(root, "첫 커밋").unwrap();
        assert!(status(root).unwrap().files.is_empty());
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        assert!(diff(root, "a.txt", false).unwrap().contains("-one"));
        discard(root, &["a.txt".into()]).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap().trim(), "one");
        assert!(commit(root, "  ").is_err());
    }
}
