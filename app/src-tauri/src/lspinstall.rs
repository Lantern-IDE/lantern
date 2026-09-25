//! 언어 서버 설치. 사용자가 동의했을 때만 실행한다.
//! npm 패키지는 전역이 아니라 Lantern 데이터 폴더(servers/)에 설치해 시스템을 건드리지 않는다.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct InstallPlan {
    /// 사용자에게 보여줄 설명. 예: "npm으로 typescript-language-server 설치 (약 30MB)"
    pub summary: String,
    /// 필요한 도구가 없으면 안내 문구 (이때는 설치 버튼 대신 안내만)
    pub missing: Option<String>,
}

pub fn servers_dir() -> Option<PathBuf> {
    crate::state::data_dir().map(|d| d.join("servers"))
}

/// Lantern이 설치한 서버 실행 파일 (없으면 None)
pub fn installed_bin(command: &str) -> Option<PathBuf> {
    installed_bin_in(&servers_dir()?, command)
}

fn installed_bin_in(dir: &Path, command: &str) -> Option<PathBuf> {
    let bin = dir.join("node_modules").join(".bin");
    let names: &[String] = &if cfg!(windows) { vec![format!("{command}.cmd"), format!("{command}.exe")] } else { vec![command.to_string()] };
    names.iter().map(|n| bin.join(n)).find(|p| p.is_file())
}

/// 명령 이름으로 어떤 방법으로 설치할지 정한다. 모르는 서버면 None.
fn npm_packages(command: &str) -> Option<&'static [&'static str]> {
    match command {
        // TypeScript 7(네이티브)에는 tsserver.js가 없어 typescript-language-server가 쓸 수 없다. 6으로 고정.
        "typescript-language-server" => Some(&["typescript-language-server", "typescript@6"]),
        "pyright-langserver" => Some(&["pyright"]),
        "vscode-css-language-server" | "vscode-html-language-server" | "vscode-json-language-server" => {
            Some(&["vscode-langservers-extracted"])
        }
        "bash-language-server" => Some(&["bash-language-server"]),
        "yaml-language-server" => Some(&["yaml-language-server"]),
        _ => None,
    }
}

/// 서버를 초기화할 때 덧붙일 옵션.
/// typescript-language-server는 TypeScript 본체를 프로젝트의 node_modules에서 찾는다.
/// 프로젝트에 없으면(순수 JS 프로젝트, npm install 전, tsserver.js가 없는 TypeScript 7 등) Lantern이 함께 설치한 것을 쓰게 한다.
pub fn init_options(command: &str, root: &Path) -> Option<serde_json::Value> {
    init_options_in(&servers_dir()?, command, root)
}

fn init_options_in(dir: &Path, command: &str, root: &Path) -> Option<serde_json::Value> {
    if command != "typescript-language-server" {
        return None;
    }
    let rel = Path::new("node_modules").join("typescript").join("lib").join("tsserver.js");
    if root.join(&rel).is_file() {
        return None;
    }
    let bundled = dir.join(&rel);
    bundled.is_file().then(|| serde_json::json!({ "tsserver": { "path": bundled.to_string_lossy() } }))
}

pub fn plan(command: &str) -> Option<InstallPlan> {
    if let Some(pkgs) = npm_packages(command) {
        let missing = which::which("npm").is_err().then(|| "Node.js(npm)가 필요합니다. https://nodejs.org 에서 설치한 뒤 다시 시도하세요.".to_string());
        return Some(InstallPlan { summary: format!("npm으로 {}을(를) Lantern 전용 폴더에 설치합니다", pkgs.join(", ")), missing });
    }
    if command == "rust-analyzer" {
        let missing = which::which("rustup").is_err().then(|| "rustup이 필요합니다. https://rustup.rs 에서 Rust를 설치하세요.".to_string());
        return Some(InstallPlan { summary: "rustup으로 rust-analyzer 구성 요소를 추가합니다 (rustup component add rust-analyzer)".into(), missing });
    }
    None
}

fn run(mut cmd: Command, what: &str) -> Result<()> {
    lantern_context::git::no_window(&mut cmd);
    let out = cmd.output().with_context(|| format!("{what} 실행 실패"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = err.lines().rev().take(8).collect();
        bail!("{what} 실패:\n{}", tail.into_iter().rev().collect::<Vec<_>>().join("\n"));
    }
    Ok(())
}

pub fn install(command: &str) -> Result<()> {
    let plan = plan(command).with_context(|| format!("'{command}'는 자동 설치를 지원하지 않습니다. 직접 설치한 뒤 PATH에 넣어 주세요"))?;
    if let Some(m) = plan.missing {
        bail!(m);
    }
    crate::applog::info(&format!("언어 서버 설치: {}", plan.summary));
    if let Some(pkgs) = npm_packages(command) {
        let dir = servers_dir().context("데이터 폴더를 찾지 못했습니다")?;
        std::fs::create_dir_all(&dir)?;
        if !dir.join("package.json").exists() {
            std::fs::write(dir.join("package.json"), "{\n  \"name\": \"lantern-servers\",\n  \"private\": true\n}\n")?;
        }
        let npm = which::which("npm")?;
        let mut cmd = Command::new(npm);
        cmd.current_dir(&dir).args(["install", "--no-audit", "--no-fund", "--loglevel=error", "--prefix"]).arg(&dir).args(pkgs);
        run(cmd, "npm install")?;
        if installed_bin_in(&dir, command).is_none() {
            bail!("설치는 끝났지만 {command} 실행 파일을 찾지 못했습니다");
        }
    } else {
        let mut cmd = Command::new(which::which("rustup")?);
        cmd.args(["component", "add", "rust-analyzer"]);
        run(cmd, "rustup")?;
    }
    crate::applog::info(&format!("언어 서버 설치 완료: {command}"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_known_servers() {
        assert!(plan("typescript-language-server").unwrap().summary.contains("typescript"));
        assert!(plan("pyright-langserver").is_some());
        assert!(plan("rust-analyzer").unwrap().summary.contains("rustup"));
        assert!(plan("my-custom-ls").is_none());
    }

    #[test]
    fn typescript_falls_back_to_bundled_tsserver() {
        let servers = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let rel = "node_modules/typescript/lib/tsserver.js";
        assert!(init_options_in(servers.path(), "typescript-language-server", project.path()).is_none(), "설치본도 없으면 없음");
        std::fs::create_dir_all(servers.path().join("node_modules/typescript/lib")).unwrap();
        std::fs::write(servers.path().join(rel), "").unwrap();
        let o = init_options_in(servers.path(), "typescript-language-server", project.path()).unwrap();
        assert!(o["tsserver"]["path"].as_str().unwrap().contains("tsserver.js"));
        assert!(init_options_in(servers.path(), "pyright-langserver", project.path()).is_none());
        std::fs::create_dir_all(project.path().join("node_modules/typescript/lib")).unwrap();
        std::fs::write(project.path().join(rel), "").unwrap();
        assert!(init_options_in(servers.path(), "typescript-language-server", project.path()).is_none(), "프로젝트 것이 우선");
    }

    #[test]
    fn finds_installed_bin() {
        let dir = tempfile::tempdir().unwrap();
        assert!(installed_bin_in(dir.path(), "pyright-langserver").is_none());
        let bin = dir.path().join("node_modules/.bin");
        std::fs::create_dir_all(&bin).unwrap();
        let name = if cfg!(windows) { "pyright-langserver.cmd" } else { "pyright-langserver" };
        std::fs::write(bin.join(name), "").unwrap();
        assert_eq!(installed_bin_in(dir.path(), "pyright-langserver"), Some(bin.join(name)));
    }
}
