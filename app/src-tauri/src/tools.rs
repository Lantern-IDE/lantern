//! 에이전트 도구. 읽기 도구는 바로 실행하고, 파일 변경과 목록 밖 명령은 사용자 승인을 받는다.

use crate::config::Config;
use crate::llm::ToolSpec;
use crate::state::{rel_path, resolve_in_root, Project};
use anyhow::{bail, Context, Result};
use globset::{Glob, GlobSetBuilder};
use lantern_context::assemble::ContextRequest;
use serde_json::{json, Value};
use similar::TextDiff;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const READ_LIMIT_LINES: usize = 2000;
const OUTPUT_LIMIT: usize = 20_000;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

pub fn specs(allowed: &[String]) -> Vec<ToolSpec> {
    let all = vec![
        ("get_context", "Assemble the project code most relevant to a question (ranked, within a token budget, with the reason each item was included). Use this first when you need to find where something is implemented.",
            json!({"type":"object","properties":{"query":{"type":"string"},"file":{"type":"string","description":"optional focus file"},"line":{"type":"integer"}},"required":["query"]})),
        ("search_symbols", "Full-text search over symbol names, signatures and bodies.",
            json!({"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer"}},"required":["query"]})),
        ("get_symbol", "Full definition of a symbol by exact name, plus what uses it and what it uses.",
            json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"]})),
        ("find_references", "All places that reference a name.",
            json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"]})),
        ("read_file", "Read a file with line numbers (`N<TAB>text`). Optionally a 1-based inclusive line range.",
            json!({"type":"object","properties":{"path":{"type":"string"},"start_line":{"type":"integer"},"end_line":{"type":"integer"}},"required":["path"]})),
        ("list_dir", "List a directory (respects .gitignore). Use \".\" for the project root.",
            json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]})),
        ("write_file", "Create or overwrite a file with the full content. The user reviews a diff before it is applied. Prefer edit_file for small changes to existing files.",
            json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]})),
        ("edit_file", "Replace an exact string in a file. old_string must match exactly once (including whitespace) unless replace_all is true. Do not include the line-number prefix from read_file. The user reviews a diff before it is applied.",
            json!({"type":"object","properties":{"path":{"type":"string"},"old_string":{"type":"string"},"new_string":{"type":"string"},"replace_all":{"type":"boolean"}},"required":["path","old_string","new_string"]})),
        ("run_command", "Run a shell command in the project root and return its output (120s timeout). Commands outside the allow-list need user approval.",
            json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"]})),
        ("remember", "Save a durable project fact (convention, decision, recurring explanation) to .lantern/memory/<topic>.md so future requests include it. The user approves the change.",
            json!({"type":"object","properties":{"topic":{"type":"string","description":"file name without extension, e.g. conventions"},"note":{"type":"string"}},"required":["topic","note"]})),
    ];
    all.into_iter()
        .filter(|(n, _, _)| allowed.iter().any(|a| a == n))
        .map(|(name, desc, schema)| ToolSpec { name: name.into(), description: desc.into(), input_schema: schema })
        .collect()
}

pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// 사용자와의 접점. 테스트에서는 가짜 구현으로 바꾼다.
pub trait Approver: Send + Sync {
    /// 승인 요청. 사용자가 거절하거나 중단하면 false.
    fn ask<'a>(&'a self, kind: &'a str, title: &'a str, detail: &'a str) -> BoxFuture<'a, bool>;
    /// 명령 실행 출력 등을 출력 패널로 보낸다.
    fn output(&self, text: &str);
}

pub struct ToolCtx<'a> {
    pub project: Arc<Project>,
    pub config: &'a Config,
    pub approver: &'a dyn Approver,
    /// 이번 실행에서 바뀐 파일
    pub changed: std::sync::Mutex<Vec<String>>,
    /// 되돌리기용: 이번 실행에서 처음 바꾸기 직전의 원본 (None이면 새로 만든 파일)
    pub originals: std::sync::Mutex<crate::state::Originals>,
    /// 격리된 작업이면 그 작업 공간(git worktree). 파일 읽기·쓰기·명령이 여기서 일어난다.
    pub workdir: Option<std::path::PathBuf>,
}

pub struct ToolOutcome {
    pub content: String,
    pub is_error: bool,
}

fn s<'v>(input: &'v Value, key: &str) -> Result<&'v str> {
    input.get(key).and_then(Value::as_str).with_context(|| format!("'{key}' 인자가 필요합니다"))
}

impl ToolCtx<'_> {
    pub async fn run(&self, name: &str, input: &Value) -> ToolOutcome {
        match self.dispatch(name, input).await {
            // 모델로 가기 전에 키·토큰으로 보이는 값을 가린다.
            Ok(content) => ToolOutcome { content: lantern_context::secrets::redact(&truncate(content)).0, is_error: false },
            Err(e) => ToolOutcome { content: lantern_context::secrets::redact(&format!("{e:#}")).0, is_error: true },
        }
    }

    /// 파일 도구가 일하는 폴더 (격리 작업이면 작업 공간)
    pub fn root(&self) -> &Path {
        self.workdir.as_deref().unwrap_or(&self.project.root)
    }

    async fn dispatch(&self, name: &str, input: &Value) -> Result<String> {
        let root = self.root().to_path_buf();
        // 에이전트 쪽에서도 거르지만, 제한 모드의 쓰기·실행 도구는 여기서 한 번 더 막는다.
        if !self.project.is_trusted() && matches!(name, "write_file" | "edit_file" | "run_command" | "remember") {
            bail!("제한 모드라 '{name}'을(를) 쓸 수 없습니다. 사용자가 이 폴더를 신뢰해야 합니다");
        }
        match name {
            "get_context" | "search_symbols" | "get_symbol" | "find_references" => {
                let engine = self.project.engine.clone();
                let budget = self.config.context.budget_tokens;
                let (name, input) = (name.to_string(), input.clone());
                tokio::task::spawn_blocking(move || -> Result<String> {
                    let mut e = engine.lock().unwrap();
                    e.refresh()?;
                    match name.as_str() {
                        "get_context" => Ok(e
                            .context(&ContextRequest {
                                query: s(&input, "query")?.to_string(),
                                file: input.get("file").and_then(Value::as_str).map(str::to_string),
                                line: input.get("line").and_then(Value::as_u64).map(|l| l as u32),
                                budget_tokens: budget,
                            })?
                            .to_markdown()),
                        "search_symbols" => {
                            let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
                            e.search_report(s(&input, "query")?, limit)
                        }
                        "get_symbol" => e.symbol_report(s(&input, "name")?),
                        _ => e.references_report(s(&input, "name")?),
                    }
                })
                .await?
            }
            "read_file" => {
                let path = resolve_in_root(&root, s(input, "path")?)?;
                let rel = rel_path(&root, &path);
                if lantern_context::secrets::is_secret_path(&rel)
                    && !self
                        .approver
                        .ask("secret", &format!("비밀 파일 읽기: {rel}"), "비밀 정보가 들어 있을 수 있는 파일입니다. 허용해도 키·토큰으로 보이는 값은 가려서 보냅니다.")
                        .await
                {
                    bail!("사용자가 {rel} 읽기를 거절했습니다");
                }
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("{} 읽기 실패", rel_path(&root, &path)))?;
                let start = input.get("start_line").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
                let end = input.get("end_line").and_then(Value::as_u64).map(|e| e as usize);
                let total = text.lines().count();
                let requested_end = end.unwrap_or(total).min(total);
                let end = requested_end.min(start + READ_LIMIT_LINES - 1);
                let mut out: String = text
                    .lines()
                    .enumerate()
                    .skip(start - 1)
                    .take(end.saturating_sub(start - 1))
                    .map(|(i, l)| format!("{}\t{}\n", i + 1, l))
                    .collect();
                if end < requested_end {
                    out.push_str(&format!("…({}줄 중 {}-{}줄 표시. start_line으로 이어서 읽으세요)\n", total, start, end));
                }
                if out.is_empty() {
                    out = "(빈 파일)".into();
                }
                Ok(out)
            }
            "list_dir" => {
                let path = resolve_in_root(&root, s(input, "path").unwrap_or("."))?;
                let entries = crate::workspace::list_dir(&root, &path)?;
                Ok(entries
                    .iter()
                    .map(|e| if e.is_dir { format!("{}/", e.path) } else { e.path.clone() })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "write_file" => {
                let rel = s(input, "path")?;
                let content = s(input, "content")?;
                let path = resolve_in_root(&root, rel)?;
                let old = std::fs::read_to_string(&path).unwrap_or_default();
                self.apply_change(&path, &old, content, if old.is_empty() && !path.exists() { "새 파일" } else { "파일 덮어쓰기" }).await
            }
            "edit_file" => {
                let path = resolve_in_root(&root, s(input, "path")?)?;
                let old_s = s(input, "old_string")?;
                let new_s = s(input, "new_string")?;
                let all = input.get("replace_all").and_then(Value::as_bool).unwrap_or(false);
                let old = std::fs::read_to_string(&path)
                    .with_context(|| format!("{} 읽기 실패", rel_path(&root, &path)))?;
                if old_s.is_empty() {
                    bail!("old_string이 비어 있습니다. 새 파일은 write_file을 쓰세요");
                }
                let count = old.matches(old_s).count();
                let new = match (count, all) {
                    (0, _) => bail!("old_string을 찾지 못했습니다. read_file로 정확한 내용을 다시 확인하세요"),
                    (1, _) | (_, true) => old.replace(old_s, new_s),
                    (n, false) => bail!("old_string이 {n}군데에 있습니다. 앞뒤 줄을 더 넣어 한 곳만 가리키게 하거나 replace_all을 쓰세요"),
                };
                self.apply_change(&path, &old, &new, "파일 수정").await
            }
            "remember" => {
                let topic: String = s(input, "topic")?
                    .chars()
                    .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
                    .collect();
                let note = s(input, "note")?.trim();
                let path = root.join(".lantern").join("memory").join(format!("{topic}.md"));
                let old = std::fs::read_to_string(&path).unwrap_or_default();
                let new = if old.trim().is_empty() {
                    format!("# {topic}\n\n- {note}\n")
                } else {
                    format!("{}\n- {note}\n", old.trim_end())
                };
                self.apply_change(&path, &old, &new, "프로젝트 메모리 추가").await
            }
            "run_command" => {
                let cmd = s(input, "command")?.trim().to_string();
                let allowed = self
                    .config
                    .agent
                    .allowed_commands
                    .iter()
                    .any(|a| cmd == *a || cmd.starts_with(&format!("{a} ")))
                    && !cmd.contains(['&', '|', ';', '>', '<', '`', '$', '\n']);
                if !allowed && !self.approver.ask("command", "명령 실행", &cmd).await {
                    bail!("사용자가 명령 실행을 거절했습니다");
                }
                self.approver.output(&format!("$ {cmd}\n"));
                let out = run_shell(&root, &cmd, COMMAND_TIMEOUT).await?;
                self.approver.output(&out);
                Ok(out)
            }
            other => bail!("알 수 없는 도구: {other}"),
        }
    }

    async fn apply_change(&self, path: &Path, old: &str, new: &str, what: &str) -> Result<String> {
        let root = self.root();
        let rel = rel_path(root, path);
        if old == new {
            return Ok(format!("{rel}: 바뀐 내용이 없습니다"));
        }
        let diff = unified_diff(&rel, old, new);
        if !self.auto_approved(&rel) && !self.approver.ask("edit", &format!("{what}: {rel}"), &diff).await {
            bail!("사용자가 {rel} 변경을 거절했습니다. 이유를 묻거나 다른 방법을 제안하세요");
        }
        {
            let mut orig = self.originals.lock().unwrap();
            if !orig.iter().any(|(p, _)| p == path) {
                orig.push((path.to_path_buf(), path.exists().then(|| old.to_string())));
            }
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, new).with_context(|| format!("{rel} 쓰기 실패"))?;
        self.changed.lock().unwrap().push(rel.clone());
        let (add, del) = diff_stats(&diff);
        Ok(format!("{rel} 저장됨 (+{add} -{del})"))
    }

    fn auto_approved(&self, rel: &str) -> bool {
        let mut b = GlobSetBuilder::new();
        for g in &self.config.agent.auto_approve {
            if let Ok(g) = Glob::new(g) {
                b.add(g);
            }
        }
        b.build().map(|set| set.is_match(rel)).unwrap_or(false)
    }
}

pub fn unified_diff(rel: &str, old: &str, new: &str) -> String {
    TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{rel}"), &format!("b/{rel}"))
        .to_string()
}

fn diff_stats(diff: &str) -> (usize, usize) {
    let add = diff.lines().filter(|l| l.starts_with('+') && !l.starts_with("+++")).count();
    let del = diff.lines().filter(|l| l.starts_with('-') && !l.starts_with("---")).count();
    (add, del)
}

fn truncate(s: String) -> String {
    if s.len() <= OUTPUT_LIMIT {
        return s;
    }
    let mut cut = OUTPUT_LIMIT;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n…(출력 {}바이트 중 앞부분만 표시)", &s[..cut], s.len())
}

/// 셸 명령 실행. 표준 출력과 오류를 합쳐 돌려준다.
pub async fn run_shell(root: &Path, cmd: &str, timeout: Duration) -> Result<String> {
    let mut c = if cfg!(windows) {
        let mut c = tokio::process::Command::new("cmd");
        c.args(["/C", cmd]);
        c
    } else {
        let mut c = tokio::process::Command::new("sh");
        c.args(["-c", cmd]);
        c
    };
    #[cfg(windows)]
    c.creation_flags(0x0800_0000);
    c.current_dir(root).kill_on_drop(true).stdin(std::process::Stdio::null());
    let out = tokio::time::timeout(timeout, c.output())
        .await
        .map_err(|_| anyhow::anyhow!("{}초 안에 끝나지 않아 중단했습니다", timeout.as_secs()))??;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.trim().is_empty() {
        text.push_str(&err);
    }
    let code = out.status.code().map(|c| c.to_string()).unwrap_or_else(|| "?".into());
    Ok(format!("{}\n(종료 코드 {code})\n", truncate(text).trim_end()))
}
