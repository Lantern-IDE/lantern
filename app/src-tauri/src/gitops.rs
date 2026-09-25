//! 소스 제어 (git 명령줄을 감싼다): 저장소 찾기, 상태, Diff, 스테이징, 되돌리기, 커밋,
//! 원격 동기화(fetch/pull/push), 커밋 이력, 브랜치.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn git_command(root: &Path) -> Command {
    let mut cmd = Command::new("git");
    lantern_context::git::no_window(&mut cmd)
        .arg("-C")
        .arg(root)
        .args(["-c", "core.quotepath=off", "-c", "color.ui=false"])
        // 인증이 필요할 때 터미널 입력을 기다리며 멈추지 않게 한다. 자격 증명 관리자(창)는 그대로 쓴다.
        .env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

pub(crate) fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = git_command(root).args(args).output().context("git을 실행하지 못했습니다. Git이 설치되어 있는지 확인하세요")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("{}", err.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// 네트워크를 쓰는 명령(fetch/pull/push). 원격이 응답하지 않아도 앱이 영원히 기다리지 않게 시간 제한을 둔다.
fn git_timed(root: &Path, args: &[&str], limit: Duration) -> Result<String> {
    let mut child = git_command(root)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("git을 실행하지 못했습니다. Git이 설치되어 있는지 확인하세요")?;
    let started = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if started.elapsed() > limit {
            let _ = child.kill();
            bail!("git이 {}초 안에 끝나지 않아 멈췄습니다. 원격 주소와 네트워크, 인증을 확인하세요", limit.as_secs());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let out = child.wait_with_output()?;
    // git은 진행 상황과 결과 요약을 stderr에 쓴다
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        bail!("{}", redact_urls(text.trim()));
    }
    Ok(redact_urls(text.trim()))
}

// ── 저장소 찾기 ─────────────────────────────────────────

/// 찾지 않을 폴더 (의존성·빌드 결과·숨김 폴더)
const SKIP_DIRS: &[&str] = &["node_modules", "target", "dist", "build", "out", "vendor", "venv", ".venv", "__pycache__", "bin", "obj"];
/// 연 폴더 아래로 몇 단계까지 찾을지 (VS Code의 repositoryScanMaxDepth와 같은 역할)
const SCAN_DEPTH: usize = 2;
const MAX_REPOS: usize = 30;

/// 연 폴더의 git 저장소들: 연 폴더가 속한 저장소와, 그 아래(깊이 2까지)에 있는 저장소.
/// 저장소 안의 저장소(서브모듈 등)는 따로 찾지 않는다.
pub fn discover(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Ok(top) = git(root, &["rev-parse", "--show-toplevel"]) {
        found.push(PathBuf::from(top.trim()));
        // 연 폴더 자체가 저장소면(또는 그 안이면) 하위 폴더는 그 저장소에 속한다
        return found;
    }
    fn walk(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
        if depth > SCAN_DEPTH || found.len() >= MAX_REPOS {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str())
            })
            .map(|e| e.path())
            .collect();
        dirs.sort_by_key(|d| d.file_name().map(|n| n.to_string_lossy().to_lowercase()));
        for d in dirs {
            if found.len() >= MAX_REPOS {
                return;
            }
            if d.join(".git").exists() {
                found.push(d);
            } else {
                walk(&d, depth + 1, found);
            }
        }
    }
    walk(root, 1, &mut found);
    found
}

/// 원격 주소에서 사용자 이름·토큰을 뺀다 (`https://user:token@host/x` → `https://host/x`)
pub fn redact_url(url: &str) -> String {
    if let Some((scheme, rest)) = url.split_once("://") {
        if let Some((cred, host)) = rest.split_once('@') {
            if !cred.contains('/') {
                return format!("{scheme}://{host}");
            }
        }
    }
    url.to_string()
}

/// 여러 줄 출력 안의 원격 주소에서 인증 정보를 뺀다
fn redact_urls(text: &str) -> String {
    text.split(' ').map(|w| if w.contains("://") && w.contains('@') { redact_url(w) } else { w.to_string() }).collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Serialize)]
pub struct RepoSummary {
    /// 연 폴더 기준 경로 ("" = 연 폴더 자체이거나 연 폴더를 품은 저장소)
    pub path: String,
    pub name: String,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub changes: u32,
    pub remote: Option<String>,
    /// 연 폴더가 이 저장소의 하위 폴더일 때, 저장소 기준으로 본 연 폴더 경로 (`a/b/`). 파일 경로를 바꿀 때 쓴다.
    pub prefix: String,
}

/// 저장소의 연 폴더 기준 경로 (폴더 이름의 대소문자는 그대로). 연 폴더를 품은 저장소는 ""
pub fn rel_path(root: &Path, repo: &Path) -> String {
    // Windows는 경로의 대소문자를 가리지 않고, git은 `C:/a/b`처럼 `/`로 준다. 구성 요소 단위로 비교한다.
    let same = |a: &std::path::Component, b: &std::path::Component| {
        let (a, b) = (a.as_os_str().to_string_lossy(), b.as_os_str().to_string_lossy());
        if cfg!(windows) { a.to_lowercase() == b.to_lowercase() } else { a == b }
    };
    let (r, p): (Vec<_>, Vec<_>) = (root.components().collect(), repo.components().collect());
    if p.len() <= r.len() || !r.iter().zip(&p).all(|(a, b)| same(a, b)) {
        return String::new();
    }
    p[r.len()..].iter().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

pub fn summary(root: &Path, repo: &Path) -> RepoSummary {
    let path = rel_path(root, repo);
    let name = if path.is_empty() {
        repo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    } else {
        path.clone()
    };
    let (branch, ahead, behind, files) =
        git(repo, &["status", "--porcelain=v1", "-z", "-b", "--untracked-files=normal"]).map(|o| parse_status(&o)).unwrap_or_default();
    let upstream = git(repo, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let remote = default_remote(repo).and_then(|r| git(repo, &["remote", "get-url", &r]).ok()).map(|u| redact_url(u.trim()));
    let prefix = if path.is_empty() {
        git(root, &["rev-parse", "--show-prefix"]).map(|s| s.trim().to_string()).unwrap_or_default()
    } else {
        String::new()
    };
    RepoSummary { path, name, branch, upstream, ahead, behind, changes: files.len() as u32, remote, prefix }
}

/// 연 폴더의 저장소 목록과 요약. 저장소마다 git을 여러 번 부르므로 나눠서 동시에 돈다.
pub fn repos(root: &Path) -> Vec<RepoSummary> {
    let found = discover(root);
    std::thread::scope(|s| {
        let handles: Vec<_> = found.iter().map(|r| s.spawn(move || summary(root, r))).collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    })
}

/// 화면이 준 저장소 경로를 실제 폴더로 바꾼다. 연 폴더 밖이나 저장소가 아닌 곳은 거부한다.
pub fn resolve(root: &Path, rel: &str) -> Result<PathBuf> {
    if rel.is_empty() {
        let top = git(root, &["rev-parse", "--show-toplevel"]).context("이 폴더는 git 저장소가 아닙니다")?;
        return Ok(PathBuf::from(top.trim()));
    }
    if rel.split(['/', '\\']).any(|c| c == ".." || c.is_empty()) || Path::new(rel).is_absolute() {
        bail!("잘못된 저장소 경로: {rel}");
    }
    let dir = root.join(rel);
    if !dir.join(".git").exists() {
        bail!("{rel}은(는) git 저장소가 아닙니다");
    }
    Ok(dir)
}

fn default_remote(repo: &Path) -> Option<String> {
    let remotes = git(repo, &["remote"]).ok()?;
    let list: Vec<&str> = remotes.lines().map(str::trim).filter(|s| !s.is_empty()).collect();
    list.iter().find(|r| **r == "origin").or(list.first()).map(|s| s.to_string())
}

// ── 원격 동기화 ─────────────────────────────────────────

const NETWORK_LIMIT: Duration = Duration::from_secs(120);

pub fn fetch(repo: &Path) -> Result<String> {
    if default_remote(repo).is_none() {
        bail!("연결된 원격 저장소가 없습니다");
    }
    git_timed(repo, &["fetch", "--all", "--prune"], NETWORK_LIMIT)
}

/// 앞으로 감기(fast-forward)만 한다. 갈라졌으면 병합하지 않고 알린다(병합·리베이스는 사용자가 고른다).
pub fn pull(repo: &Path) -> Result<String> {
    if git(repo, &["rev-parse", "--abbrev-ref", "@{u}"]).is_err() {
        bail!("이 브랜치에 연결된 원격 브랜치(업스트림)가 없습니다. 먼저 푸시하세요");
    }
    git_timed(repo, &["pull", "--ff-only"], NETWORK_LIMIT).map_err(|e| {
        let msg = e.to_string();
        if msg.contains("Not possible to fast-forward") || msg.contains("diverging") || msg.contains("fast-forward") {
            anyhow::anyhow!("로컬과 원격이 갈라져 앞으로 감기로 받을 수 없습니다. 터미널에서 병합(git merge)이나 리베이스(git rebase)를 고르세요.\n\n{msg}")
        } else {
            e
        }
    })
}

/// 업스트림이 없으면 기본 원격에 같은 이름으로 올리고 연결한다.
pub fn push(repo: &Path) -> Result<String> {
    if git(repo, &["rev-parse", "--abbrev-ref", "@{u}"]).is_ok() {
        return git_timed(repo, &["push"], NETWORK_LIMIT);
    }
    let remote = default_remote(repo).context("연결된 원격 저장소가 없습니다")?;
    git_timed(repo, &["push", "-u", &remote, "HEAD"], NETWORK_LIMIT)
}

// ── 커밋 이력 ───────────────────────────────────────────

#[derive(Debug, Serialize, PartialEq)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub author: String,
    /// ISO 8601
    pub date: String,
    pub refs: Vec<String>,
    pub subject: String,
}

const FIELD: char = '\u{1f}';
const RECORD: char = '\u{1e}';

pub fn parse_log(out: &str) -> Vec<Commit> {
    out.split(RECORD)
        .filter_map(|rec| {
            let f: Vec<&str> = rec.trim_start_matches('\n').split(FIELD).collect();
            if f.len() < 6 {
                return None;
            }
            Some(Commit {
                hash: f[0].into(),
                short: f[1].into(),
                author: f[2].into(),
                date: f[3].into(),
                refs: f[4].split(", ").map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect(),
                subject: f[5].into(),
            })
        })
        .collect()
}

pub fn log(repo: &Path, skip: u32, limit: u32) -> Result<Vec<Commit>> {
    if git(repo, &["rev-parse", "--verify", "-q", "HEAD"]).is_err() {
        return Ok(vec![]); // 아직 커밋이 없음
    }
    let (skip, limit) = (skip.to_string(), limit.clamp(1, 200).to_string());
    let out = git(repo, &["log", "--format=%H%x1f%h%x1f%an%x1f%aI%x1f%D%x1f%s%x1e", "-n", &limit, "--skip", &skip])?;
    Ok(parse_log(&out))
}

fn check_hash(hash: &str) -> Result<()> {
    if hash.len() < 4 || hash.len() > 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("잘못된 커밋: {hash}");
    }
    Ok(())
}

#[derive(Debug, Serialize, PartialEq)]
pub struct CommitFile {
    /// A, M, D, R, C, T
    pub status: String,
    pub path: String,
    pub orig: Option<String>,
}

/// `git show --name-status -z` 출력 해석
pub fn parse_name_status(out: &str) -> Vec<CommitFile> {
    let mut files = Vec::new();
    let mut parts = out.split('\0').map(|s| s.trim_start_matches('\n')).filter(|s| !s.is_empty());
    while let Some(code) = parts.next() {
        let status: String = code.chars().take(1).collect();
        if status == "R" || status == "C" {
            let (Some(orig), Some(path)) = (parts.next(), parts.next()) else { break };
            files.push(CommitFile { status, path: path.into(), orig: Some(orig.into()) });
        } else if let Some(path) = parts.next() {
            files.push(CommitFile { status, path: path.into(), orig: None });
        }
    }
    files
}

pub fn commit_files(repo: &Path, hash: &str) -> Result<Vec<CommitFile>> {
    check_hash(hash)?;
    // 병합 커밋은 첫 부모 기준으로 보여준다
    Ok(parse_name_status(&git(repo, &["show", "--format=", "--name-status", "-z", "-M", "--first-parent", "-m", hash])?))
}

pub fn commit_diff(repo: &Path, hash: &str, path: &str) -> Result<String> {
    check_hash(hash)?;
    git(repo, &["show", "--format=", "--no-ext-diff", "-M", "--first-parent", "-m", hash, "--", path])
}

// ── 브랜치 ─────────────────────────────────────────────

#[derive(Debug, Serialize, PartialEq)]
pub struct Branch {
    /// `main`, 원격이면 `origin/main`
    pub name: String,
    pub remote: bool,
    pub current: bool,
    pub upstream: Option<String>,
    pub date: String,
}

pub fn parse_branches(out: &str) -> Vec<Branch> {
    let mut list: Vec<Branch> = out
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(FIELD).collect();
            if f.len() < 5 || f[0].ends_with("/HEAD") {
                return None;
            }
            Some(Branch {
                remote: f[0].starts_with("refs/remotes/"),
                name: f[1].into(),
                current: f[2] == "*",
                upstream: Some(f[3].to_string()).filter(|s| !s.is_empty()),
                date: f[4].into(),
            })
        })
        .collect();
    // 지금 브랜치, 로컬, 원격 순. 같은 종류끼리는 최근에 바뀐 것부터
    list.sort_by(|a, b| (!a.current, a.remote).cmp(&(!b.current, b.remote)).then(b.date.cmp(&a.date)));
    list
}

pub fn branches(repo: &Path) -> Result<Vec<Branch>> {
    let out = git(repo, &[
        "for-each-ref",
        "--format=%(refname)%1f%(refname:short)%1f%(HEAD)%1f%(upstream:short)%1f%(committerdate:iso-strict)",
        "refs/heads",
        "refs/remotes",
    ])?;
    Ok(parse_branches(&out))
}

/// 브랜치 이름 검사. `-`로 시작하는 이름(옵션으로 해석됨)과 git이 허용하지 않는 이름을 막는다.
fn check_branch(repo: &Path, name: &str) -> Result<()> {
    if name.is_empty() || name.starts_with('-') || git(repo, &["check-ref-format", "--branch", name]).is_err() {
        bail!("쓸 수 없는 브랜치 이름: {name}");
    }
    Ok(())
}

/// 브랜치 전환. `create`면 지금 위치에서 새로 만든다. 원격 브랜치(`origin/x`)를 고르면 같은 이름의 로컬 브랜치로 받아 연결한다.
pub fn checkout(repo: &Path, name: &str, create: bool, remote: bool) -> Result<String> {
    if create {
        check_branch(repo, name)?;
        return git(repo, &["switch", "-c", name]);
    }
    if remote {
        let local = name.split_once('/').map(|(_, b)| b).unwrap_or(name);
        check_branch(repo, local)?;
        if git(repo, &["rev-parse", "--verify", "-q", &format!("refs/heads/{local}")]).is_ok() {
            return git(repo, &["switch", local]);
        }
        if name.starts_with('-') || git(repo, &["rev-parse", "--verify", "-q", &format!("refs/remotes/{name}")]).is_err() {
            bail!("원격 브랜치를 찾을 수 없습니다: {name}");
        }
        return git(repo, &["switch", "-c", local, "--track", name]);
    }
    check_branch(repo, name)?;
    git(repo, &["switch", name])
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

    fn have_git() -> bool {
        Command::new("git").arg("--version").output().is_ok()
    }

    /// 사용자 전역 설정에 기대지 않는 저장소 (이름·메일, 기본 브랜치)
    fn new_repo(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        git(dir, &["init", "-q", "-b", "main"]).unwrap();
        git(dir, &["config", "user.email", "t@example.com"]).unwrap();
        git(dir, &["config", "user.name", "t"]).unwrap();
        git(dir, &["config", "core.autocrlf", "false"]).unwrap();
    }

    fn commit_file(dir: &Path, name: &str, text: &str, msg: &str) {
        std::fs::write(dir.join(name), text).unwrap();
        git(dir, &["add", name]).unwrap();
        git(dir, &["commit", "-qm", msg]).unwrap();
    }

    #[test]
    fn redacts_credentials_in_urls() {
        assert_eq!(redact_url("https://user:ghp_secret@github.com/a/b.git"), "https://github.com/a/b.git");
        assert_eq!(redact_url("http://work.example:8070/WX/GIS.git"), "http://work.example:8070/WX/GIS.git");
        assert_eq!(redact_url("git@github.com:a/b.git"), "git@github.com:a/b.git");
        assert!(!redact_urls("To https://me:tok@host/x.git\n   abc..def  main -> main").contains("tok"));
    }

    #[test]
    fn parses_log_name_status_and_branches() {
        let log = "aaaa1111\u{1f}aaaa111\u{1f}오태훈\u{1f}2026-09-26T01:00:00+09:00\u{1f}HEAD -> main, origin/main\u{1f}FEAT: 추가\u{1e}\nbbbb2222\u{1f}bbbb222\u{1f}t\u{1f}2026-09-25T01:00:00+09:00\u{1f}\u{1f}첫 커밋\u{1e}\n";
        let c = parse_log(log);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].refs, vec!["HEAD -> main", "origin/main"]);
        assert_eq!(c[0].author, "오태훈");
        assert!(c[1].refs.is_empty());

        let ns = parse_name_status("M\0src/a.rs\0R087\0old.rs\0new.rs\0A\0한글.md\0");
        assert_eq!(ns[0], CommitFile { status: "M".into(), path: "src/a.rs".into(), orig: None });
        assert_eq!(ns[1], CommitFile { status: "R".into(), path: "new.rs".into(), orig: Some("old.rs".into()) });
        assert_eq!(ns[2].path, "한글.md");

        let refs = "refs/heads/dev\u{1f}dev\u{1f} \u{1f}\u{1f}2026-09-20T00:00:00+09:00\nrefs/heads/main\u{1f}main\u{1f}*\u{1f}origin/main\u{1f}2026-09-10T00:00:00+09:00\nrefs/remotes/origin/HEAD\u{1f}origin\u{1f} \u{1f}\u{1f}x\nrefs/remotes/origin/main\u{1f}origin/main\u{1f} \u{1f}\u{1f}2026-09-10T00:00:00+09:00\n";
        let b = parse_branches(refs);
        assert_eq!(b.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), vec!["main", "dev", "origin/main"], "지금 브랜치, 로컬, 원격 순. origin/HEAD는 뺌");
        assert_eq!(b[0].upstream.as_deref(), Some("origin/main"));
        assert!(b[2].remote);
    }

    #[test]
    fn discovers_nested_repositories() {
        if !have_git() {
            return;
        }
        let ws = tempfile::tempdir().unwrap();
        let root = ws.path().join("workspace");
        for r in ["api", "Web", "libs/shared"] {
            new_repo(&root.join(r));
        }
        std::fs::create_dir_all(root.join("node_modules/pkg/.git")).unwrap(); // 의존성 폴더는 찾지 않음
        new_repo(&root.join("a/b/c/deep")); // 깊이 4: 찾지 않음
        let found: Vec<String> = discover(&root).iter().map(|p| rel_path(&root, p)).collect();
        assert_eq!(found, vec!["api", "libs/shared", "Web"], "대소문자는 그대로, 정렬은 대소문자 무시");
        // git은 `/`로 된 경로를 준다
        let git_style = PathBuf::from(root.join("Web").to_string_lossy().replace('\\', "/"));
        assert_eq!(rel_path(&root, &git_style), "Web");
        assert_eq!(rel_path(&root, &root), "");

        let s = repos(&root);
        assert_eq!(s.len(), 3);
        assert!(s.iter().all(|r| r.branch.as_deref() == Some("main") && r.remote.is_none()));

        assert!(resolve(&root, "api").is_ok());
        assert!(resolve(&root, "../x").is_err(), "연 폴더 밖");
        assert!(resolve(&root, "a").is_err(), "저장소가 아님");

        // 저장소 안의 하위 폴더를 열면 그 저장소 하나로 본다
        std::fs::create_dir_all(root.join("api/src")).unwrap();
        let inner = discover(&root.join("api/src"));
        assert_eq!(inner.len(), 1);
        assert_eq!(summary(&root.join("api/src"), &inner[0]).prefix, "src/");
    }

    #[test]
    fn syncs_with_remote_and_switches_branches() {
        if !have_git() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let bare = tmp.path().join("remote.git");
        git(tmp.path(), &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()]).unwrap();
        let a = tmp.path().join("a");
        new_repo(&a);
        commit_file(&a, "x.txt", "1\n", "첫 커밋");
        assert!(fetch(&a).is_err(), "원격이 없으면 알림");
        git(&a, &["remote", "add", "origin", bare.to_str().unwrap()]).unwrap();
        assert!(pull(&a).is_err(), "업스트림이 없으면 알림");
        push(&a).unwrap(); // 업스트림이 없으면 -u origin HEAD
        assert_eq!(summary(&a, &a).upstream.as_deref(), Some("origin/main"));

        // 다른 사본에서 올린 커밋을 fetch → pull(앞으로 감기)
        let b = tmp.path().join("b");
        git(tmp.path(), &["clone", "-q", bare.to_str().unwrap(), b.to_str().unwrap()]).unwrap();
        git(&b, &["config", "user.email", "t@example.com"]).unwrap();
        git(&b, &["config", "user.name", "t"]).unwrap();
        commit_file(&b, "y.txt", "2\n", "원격에서 추가");
        git(&b, &["push", "-q"]).unwrap();
        fetch(&a).unwrap();
        assert_eq!(summary(&a, &a).behind, 1);
        pull(&a).unwrap();
        assert!(a.join("y.txt").exists());
        assert_eq!(log(&a, 0, 10).unwrap().len(), 2);

        // 갈라지면 병합하지 않고 알린다
        commit_file(&b, "z.txt", "3\n", "원격 쪽");
        git(&b, &["push", "-q"]).unwrap();
        commit_file(&a, "w.txt", "4\n", "로컬 쪽");
        fetch(&a).unwrap();
        let e = pull(&a).unwrap_err().to_string();
        assert!(e.contains("갈라져"), "{e}");
        git(&a, &["reset", "-q", "--hard", "origin/main"]).unwrap();

        // 커밋 이력과 바뀐 파일, diff
        let head = &log(&a, 0, 1).unwrap()[0];
        assert_eq!(head.subject, "원격 쪽");
        assert!(head.refs.iter().any(|r| r.contains("main")));
        let files = commit_files(&a, &head.hash).unwrap();
        assert_eq!(files, vec![CommitFile { status: "A".into(), path: "z.txt".into(), orig: None }]);
        assert!(commit_diff(&a, &head.hash, "z.txt").unwrap().contains("+3"));
        assert!(commit_files(&a, "--output=x").is_err(), "해시가 아니면 거부");

        // 브랜치: 새로 만들기, 원격 브랜치 받기, 잘못된 이름
        checkout(&a, "feat/x", true, false).unwrap();
        assert_eq!(summary(&a, &a).branch.as_deref(), Some("feat/x"));
        git(&b, &["switch", "-q", "-c", "dev"]).unwrap();
        git(&b, &["push", "-q", "-u", "origin", "dev"]).unwrap();
        fetch(&a).unwrap();
        assert!(branches(&a).unwrap().iter().any(|x| x.name == "origin/dev" && x.remote));
        checkout(&a, "origin/dev", false, true).unwrap();
        assert_eq!(summary(&a, &a).branch.as_deref(), Some("dev"));
        assert_eq!(summary(&a, &a).upstream.as_deref(), Some("origin/dev"));
        checkout(&a, "main", false, false).unwrap();
        assert!(checkout(&a, "-f", false, false).is_err());
        assert!(checkout(&a, "bad..name", true, false).is_err());
    }
}
