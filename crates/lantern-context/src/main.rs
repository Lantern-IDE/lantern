use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use lantern_context::assemble::{ContextRequest, DEFAULT_BUDGET};
use lantern_context::{autopsy, check, hook, sessions};
use lantern_context::tokenize::estimate_tokens;
use lantern_context::Engine;
use std::collections::HashSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

#[derive(Parser)]
#[command(name = "lantern", version, about = "Lantern 맥락 엔진: 로컬 코드 인덱싱과 LLM용 맥락 조립")]
struct Cli {
    /// 프로젝트 루트
    #[arg(long, short = 'C', global = true, default_value = ".")]
    root: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 증분 인덱싱
    Index {
        /// 기존 인덱스를 지우고 처음부터
        #[arg(long)]
        full: bool,
    },
    /// 심볼 검색
    Search {
        /// '-'로 시작해도 질문으로 받음 (예: "--seq 모드에서 …")
        #[arg(allow_hyphen_values = true)]
        query: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// 심볼 정의, 사용하는 곳, 사용하는 심볼
    Symbol { name: String },
    /// 이름을 참조하는 위치
    Refs { name: String },
    /// 질문에 맞는 맥락 조립
    Context {
        /// '-'로 시작해도 질문으로 받음 (예: "--seq 모드에서 …")
        #[arg(allow_hyphen_values = true)]
        query: String,
        #[arg(long)]
        file: Option<String>,
        #[arg(long)]
        line: Option<u32>,
        #[arg(long, default_value_t = DEFAULT_BUDGET)]
        budget: usize,
        /// JSON으로 출력
        #[arg(long)]
        json: bool,
    },
    /// 인덱스 통계
    Stats,
    /// 성능과 토큰 절감 측정 (질문 파일: 한 줄에 질문 하나, #은 주석)
    Bench {
        questions: PathBuf,
        #[arg(long, default_value_t = DEFAULT_BUDGET)]
        budget: usize,
        /// 인덱스를 지우고 초기 인덱싱 시간부터 측정
        #[arg(long)]
        fresh: bool,
    },
    /// MCP 서버 실행 (stdio). 루트는 인자 또는 --root
    Mcp { path: Option<PathBuf> },
    /// 코드 지도와 변경 영향 반경 (JSON)
    Graph {
        #[command(subcommand)]
        view: GraphView,
    },
    /// 에이전트가 고친 뒤 놓친 곳 찾기 (이름 기반, 저장소를 바꾸지 않음)
    Check {
        /// 비교할 기준 커밋 (기본 HEAD)
        #[arg(long)]
        base: Option<String>,
        /// JSON으로 출력
        #[arg(long)]
        json: bool,
        /// 20개로 자르지 않는다
        #[arg(long)]
        all: bool,
        /// 정밀도 기준을 못 넘겨 숨긴 시그니처 경고(Stale caller)도 보여 준다
        #[arg(long)]
        details: bool,
        /// 사라진 이름의 참조(HIGH)가 남아 있으면 종료 코드 1, 검사를 못 했으면 2
        #[arg(long)]
        strict: bool,
        /// Claude Code Stop 훅으로 실행. stdin의 JSON을 읽고, 어떤 오류든 출력 없이 0으로 끝난다
        #[arg(long)]
        stop_hook: bool,
        /// Stop 훅 기록 합계
        #[arg(long)]
        stats: bool,
        /// 다음 검사가 쓸 인덱스만 만든다. 훅이 예산을 넘겼을 때 뒤에서 부른다
        #[arg(long, hide = true)]
        index_only: bool,
    },
    /// 에이전트 훅 넣기와 빼기
    Hook {
        #[command(subcommand)]
        action: HookAction,
    },
    /// 지난 에이전트 세션이 놓쳤을 수 있는 곳 되짚기 (이름 기반, 저장소를 바꾸지 않음)
    Autopsy {
        /// 이 일수 안에 바뀐 세션만
        #[arg(long, default_value_t = 30)]
        days: u64,
        /// 이 저장소의 코드를 고친 세션 중 최근 이만큼만
        #[arg(long, default_value_t = 40)]
        max_sessions: usize,
        /// 한 에이전트만
        #[arg(long, value_enum)]
        agent: Option<SessionAgent>,
        /// 슬러그와 상관없이 모든 Claude 프로젝트 폴더를 본다
        #[arg(long)]
        deep: bool,
        /// 일수와 세션 수 제한 없이 전부
        #[arg(long)]
        all: bool,
        /// JSON으로 출력. 메시지·명령·코드는 없다
        #[arg(long)]
        json: bool,
        /// 보고서 언어. 없으면 시스템 언어
        #[arg(long, value_enum)]
        lang: Option<ReportLangArg>,
        /// 발견과 세션을 전부 보여 준다
        #[arg(long)]
        details: bool,
        /// 세션 분석을 이 시간에서 멈추고 몇 개를 봤는지 알린다
        #[arg(long, default_value_t = 120)]
        max_seconds: u64,
        /// 첫 질문으로 맥락을 다시 조립해 JSON에 넣는다. 무작위 3파일보다 낫지 않아 글 보고서에는 나오지 않는다
        #[arg(long)]
        replay: bool,
    },
    /// 세션 로그 점검용. 개수와 경로만 내고 메시지·명령·코드는 내지 않는다
    #[command(hide = true)]
    Sessions {
        /// 세션별 내용을 JSON으로. 없으면 합계 한 줄
        #[arg(long)]
        json: bool,
        /// 슬러그와 상관없이 모든 Claude 프로젝트 폴더를 본다
        #[arg(long)]
        deep: bool,
        /// 이 일수 안에 바뀐 세션만
        #[arg(long)]
        days: Option<u64>,
        /// claude 또는 codex
        #[arg(long)]
        agent: Option<String>,
        /// 최근 세션 이만큼만
        #[arg(long)]
        max_sessions: Option<usize>,
    },
}

#[derive(Subcommand)]
enum GraphView {
    /// 전체 구조. 파일이 많으면 폴더 단위로 묶는다
    Overview {
        #[arg(long, default_value_t = 400)]
        max_nodes: usize,
    },
    /// 한 곳의 주변: `f:경로`, `s:심볼id`, 또는 이름
    Neighborhood {
        center: String,
        #[arg(long, default_value_t = 30)]
        limit: usize,
    },
    /// 파일의 줄을 바꿀 때 닿는 범위 (IDE 승인 카드와 같은 계산)
    Impact {
        path: String,
        /// 바뀌는 줄 범위 `10-20` 또는 `15` (여러 번 줄 수 있다)
        #[arg(long = "lines", required = true)]
        lines: Vec<String>,
    },
}

#[derive(Subcommand)]
enum HookAction {
    /// Stop 훅 넣기. 다른 설정은 그대로 두고 백업을 먼저 쓴다
    Install(HookTarget),
    /// Stop 훅 빼기
    Uninstall(HookTarget),
}

#[derive(clap::Args)]
struct HookTarget {
    /// 대상 에이전트
    agent: HookAgent,
    /// `.claude/settings.local.json` (기본, 커밋하지 않는 파일)
    #[arg(long, conflicts_with = "user")]
    project: bool,
    /// 사용자 설정 `~/.claude/settings.json`
    #[arg(long)]
    user: bool,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum HookAgent {
    Claude,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum SessionAgent {
    Claude,
    Codex,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum ReportLangArg {
    Ko,
    En,
}

/// `10-20` 또는 `15` → (시작, 끝). 1부터, 양끝 포함
fn parse_range(s: &str) -> Result<(u32, u32)> {
    let bad = || anyhow::anyhow!("줄 범위는 `10-20`이나 `15`처럼 쓰세요: {s}");
    let (a, z) = match s.split_once('-') {
        Some((a, z)) => (a.trim().parse::<u32>().map_err(|_| bad())?, z.trim().parse::<u32>().map_err(|_| bad())?),
        None => {
            let n = s.trim().parse::<u32>().map_err(|_| bad())?;
            (n, n)
        }
    };
    if a == 0 || z < a {
        return Err(bad());
    }
    Ok((a, z))
}

fn main() -> Result<()> {
    // 탐색기에서 더블클릭하면 인자 없이 실행되어 창이 바로 닫힌다. 안내를 보여주고 기다린다.
    if std::env::args().len() == 1 {
        println!("lantern: Lantern 맥락 엔진 명령줄 도구입니다. IDE 앱이 아닙니다.\n");
        println!("  IDE를 쓰려면 같은 폴더의 lantern-app.exe 를 실행하세요.\n");
        println!("  명령줄 사용 예:");
        println!("    lantern -C <프로젝트 폴더> context \"로그인 처리 흐름\"");
        println!("    lantern -C <프로젝트 폴더> mcp      (다른 AI 도구에 MCP 서버로 연결)");
        println!("    lantern --help                       (전체 명령)\n");
        println!("Enter를 누르면 닫힙니다.");
        let _ = std::io::stdin().read_line(&mut String::new());
        return Ok(());
    }
    let cli = if std::env::args().any(|a| a == "--stop-hook") { parse_for_stop_hook() } else { Cli::parse() };
    match cli.cmd {
        Cmd::Index { full } => {
            let mut engine = if full { Engine::open_fresh(&cli.root)? } else { Engine::open(&cli.root)? };
            let s = engine.refresh()?;
            println!(
                "인덱싱 완료 {}ms · 파일 {}개 확인, {}개 갱신, {}개 변경 없음, {}개 삭제, {}개 건너뜀, 오류 {}",
                s.elapsed_ms, s.scanned, s.indexed, s.unchanged, s.removed, s.skipped, s.errors
            );
            if let Some(n) = s.cochange_pairs {
                println!("git 동시 변경 쌍 {n}개 갱신");
            }
        }
        Cmd::Search { query, limit } => {
            let mut engine = Engine::open(&cli.root)?;
            engine.refresh()?;
            print!("{}", engine.search_report(&query, limit)?);
        }
        Cmd::Symbol { name } => {
            let mut engine = Engine::open(&cli.root)?;
            engine.refresh()?;
            print!("{}", engine.symbol_report(&name)?);
        }
        Cmd::Refs { name } => {
            let mut engine = Engine::open(&cli.root)?;
            engine.refresh()?;
            print!("{}", engine.references_report(&name)?);
        }
        Cmd::Context { query, file, line, budget, json } => {
            let mut engine = Engine::open(&cli.root)?;
            engine.refresh()?;
            let req = ContextRequest { query, file, line, budget_tokens: budget };
            // 의미 검색: LANTERN_EMBED_URL·LANTERN_EMBED_MODEL이 있으면 빠진 벡터를 다 만든 뒤 함께 쓴다
            let result = match lantern_context::semantic::Embedder::from_env() {
                Some(emb) => {
                    let (done, total) = engine.embed_pending(&emb, usize::MAX)?;
                    eprintln!("lantern: 의미 검색 조각 {done}/{total}");
                    engine.context_semantic(&req, &emb)?
                }
                None => engine.context(&req)?,
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print!("{}", result.to_markdown());
            }
        }
        Cmd::Stats => {
            let mut engine = Engine::open(&cli.root)?;
            engine.refresh()?;
            let s = engine.store.stats()?;
            println!("파일 {} · 심볼 {} · 참조 {} · 동시 변경 쌍 {}", s.files, s.symbols, s.refs, s.cochange_pairs);
            for (lang, n) in s.by_lang {
                println!("  {lang}: {n}");
            }
        }
        Cmd::Bench { questions, budget, fresh } => bench(&cli.root, &questions, budget, fresh)?,
        Cmd::Mcp { path } => {
            let mut engine = Engine::open(path.as_ref().unwrap_or(&cli.root))?;
            lantern_context::mcp::serve(&mut engine)?;
        }
        Cmd::Graph { view } => {
            use lantern_context::graph;
            let mut engine = Engine::open(&cli.root)?;
            engine.refresh()?;
            let json = match view {
                GraphView::Overview { max_nodes } => serde_json::to_string_pretty(&graph::overview(&engine.store, max_nodes)?)?,
                GraphView::Neighborhood { center, limit } => serde_json::to_string_pretty(&graph::neighborhood(&engine.store, &center, limit)?)?,
                GraphView::Impact { path, lines } => {
                    let ranges = lines.iter().map(|s| parse_range(s)).collect::<Result<Vec<_>>>()?;
                    serde_json::to_string_pretty(&graph::impact(&engine.store, &path.replace('\\', "/"), &ranges)?)?
                }
            };
            println!("{json}");
        }
        Cmd::Check { index_only: true, .. } => {
            let data = hook::data_dir().context("데이터 폴더를 찾지 못했습니다")?;
            check::refresh_index_only(&cli.root, &data)?;
        }
        Cmd::Check { base, json, all, details, strict, stop_hook: _, stats, index_only: _ } => check_command(&cli.root, base.as_deref(), json, all, details, strict, stats)?,
        Cmd::Hook { action } => hook_command(&cli.root, action)?,
        Cmd::Autopsy { days, max_sessions, agent, deep, all, json, lang, details, max_seconds, replay } => {
            let opts = autopsy::AutopsyOpts {
                days: (!all).then_some(days),
                max_sessions: (!all).then_some(max_sessions),
                agent: agent.map(|a| match a {
                    SessionAgent::Claude => sessions::Agent::ClaudeCode,
                    SessionAgent::Codex => sessions::Agent::Codex,
                }),
                deep,
                replay,
                max_seconds,
            };
            let report = autopsy::run(&cli.root, &sessions::Homes::detect(), &opts)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                let lang = match lang {
                    Some(ReportLangArg::Ko) => autopsy::ReportLang::Ko,
                    Some(ReportLangArg::En) => autopsy::ReportLang::En,
                    None => autopsy::system_lang(),
                };
                print!("{}", report.to_text(lang, details));
            }
        }
        Cmd::Sessions { json, deep, days, agent, max_sessions } => {
            sessions_report(&cli.root, json, deep, days, agent.as_deref(), max_sessions)?;
        }
    }
    Ok(())
}

/// `--stop-hook`이 보이면 인자 해석부터 조심한다: clap의 오류 종료 코드 2를 Claude Code는 "막기"로 읽는다.
/// 훅이면 돌리고 어떤 실패든 출력 없이 0으로 끝낸다. 다른 명령의 인자일 뿐이면(`search --stop-hook`) 그대로 돌려준다.
fn parse_for_stop_hook() -> Cli {
    match Cli::try_parse() {
        Ok(Cli { root, cmd: Cmd::Check { stop_hook: true, base, .. } }) => {
            std::panic::set_hook(Box::new(|_| {}));
            if let Some(text) = hook::stop_hook_process(&root, base.as_deref()) {
                let mut stdout = std::io::stdout();
                let _ = stdout.write_all(text.as_bytes()).and_then(|_| stdout.flush());
            }
            std::process::exit(0)
        }
        Ok(other) => other,
        Err(_) => std::process::exit(0),
    }
}

/// 평소에는 결과와 상관없이 0. `--strict`일 때만 사라진 이름의 참조(HIGH)가 있으면 1, 검사를 못 하면 2
fn check_command(root: &Path, base: Option<&str>, json: bool, all: bool, details: bool, strict: bool, stats: bool) -> Result<()> {
    if stats {
        let data = hook::data_dir().context("데이터 폴더를 찾지 못했습니다")?;
        let s = hook::stats(&data);
        if json {
            println!("{}", serde_json::to_string_pretty(&s)?);
        } else {
            print!("{}", s.to_text());
        }
        return Ok(());
    }
    let run = || -> Result<check::CheckReport> {
        let data = hook::data_dir().context("데이터 폴더를 찾지 못했습니다")?;
        check::run_check(root, &data, base.unwrap_or("HEAD"), all)
    };
    let report = match run() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("lantern check: {e:#}");
            std::process::exit(if strict { 2 } else { 0 });
        }
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", report.to_text(details));
    }
    if strict && report.high_findings() > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn hook_command(root: &Path, action: HookAction) -> Result<()> {
    let (install, target) = match action {
        HookAction::Install(t) => (true, t),
        HookAction::Uninstall(t) => (false, t),
    };
    let HookAgent::Claude = target.agent;
    let user = target.user && !target.project;
    let dir = check::repo_root(root).unwrap_or_else(|_| root.to_path_buf());
    let path = hook::claude_settings_path(&dir, user, &sessions::Homes::detect());
    let data = hook::data_dir().context("데이터 폴더를 찾지 못했습니다")?;
    let backups = hook::backup_dir(&data);
    let exe = std::env::current_exe().context("실행 파일 경로")?;
    let command = hook::stop_hook_command(&exe);
    let mut lines = if install { hook::install_claude(&path, &command, &backups)? } else { hook::uninstall_claude(&path, &command, &backups)? };
    if !install {
        // 사용자 설정의 훅은 모든 저장소에서 돌았으니 인덱스도 전부 지운다
        lines.extend(check::forget_index((!user).then_some(dir.as_path()), &data));
    }
    for line in lines {
        println!("{line}");
    }
    if install {
        println!("The hook does not stop the agent: no finding kind has met its precision bar. It logs counts (lantern check --stats).");
        println!("Unless this repository has its own index, it also keeps symbol names and file paths, no code, under {}. lantern hook uninstall claude deletes them.", data.join("index").display());
    }
    if install && !user {
        if let Some(note) = hook::commit_warning(&dir, &path) {
            println!("{note}");
        }
    }
    Ok(())
}

/// 숨은 점검 명령: 이 저장소의 세션 로그를 읽고 개수와 경로만 낸다
fn sessions_report(root: &Path, json: bool, deep: bool, days: Option<u64>, agent: Option<&str>, max_sessions: Option<usize>) -> Result<()> {
    let agent = match agent {
        None => None,
        Some("claude") => Some(sessions::Agent::ClaudeCode),
        Some("codex") => Some(sessions::Agent::Codex),
        Some(other) => anyhow::bail!("--agent는 claude나 codex입니다: {other}"),
    };
    let root = std::path::absolute(root)?;
    let scope = sessions::Scope::for_repo(&root);
    let since = days.and_then(|d| SystemTime::now().checked_sub(Duration::from_secs(d * 86_400)));
    let opts = sessions::DiscoverOpts { deep, since, agent };
    let mut found = sessions::discover(&sessions::Homes::detect(), &scope, &opts);
    if let Some(n) = max_sessions {
        found.drain(..found.len().saturating_sub(n));
    }

    let started = Instant::now();
    let mut seen = sessions::Seen::default();
    let mut rows = Vec::new();
    let mut touching = 0;
    let (mut events, mut parsed, mut unknown, mut oversized, mut warned, mut files) = (0, 0, 0, 0, 0, 0);
    for s in &found {
        let summary = sessions::parse_session(s, &scope, &mut seen);
        files += s.files.len();
        events += summary.events.len();
        parsed += summary.parsed_lines;
        unknown += summary.unknown_lines;
        oversized += summary.oversized_lines;
        warned += summary.format_warnings.len();
        touching += usize::from(!summary.touched_paths().is_empty());
        if json {
            rows.push(summary.counts_json(&scope));
        }
    }
    let elapsed_ms = started.elapsed().as_millis() as u64;
    if json {
        let roots: Vec<&str> = scope.roots().collect();
        let report = serde_json::json!({
            "roots": roots,
            "elapsed_ms": elapsed_ms,
            "log_files": files,
            "sessions": rows,
        });
        println!("{report}");
    } else {
        println!(
            "세션 {}개 (파일 {files}개), 이 저장소를 편집한 세션 {touching}개, 이벤트 {events}개, 줄 {parsed}개 읽음 · 모름 {unknown} · 너무 김 {oversized}, 형식 경고 {warned}개, {elapsed_ms}ms",
            found.len()
        );
    }
    Ok(())
}

fn bench(root: &Path, questions: &Path, budget: usize, fresh: bool) -> Result<()> {
    let text = std::fs::read_to_string(questions).with_context(|| format!("{} 읽기", questions.display()))?;
    let qs: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();

    let mut engine = if fresh { Engine::open_fresh(root)? } else { Engine::open(root)? };
    let t = Instant::now();
    let s = engine.refresh()?;
    let first_ms = t.elapsed().as_secs_f64() * 1000.0;
    let t = Instant::now();
    engine.refresh()?;
    let noop_ms = t.elapsed().as_secs_f64() * 1000.0;
    let stats = engine.store.stats()?;

    println!("## 인덱싱");
    println!(
        "- {}: {:.0}ms (파일 {}개 갱신 / 전체 {}개, 심볼 {}개)",
        if fresh { "초기 인덱싱" } else { "증분 인덱싱" },
        first_ms,
        s.indexed,
        stats.files,
        stats.symbols
    );
    println!("- 변경 없을 때 갱신 확인: {noop_ms:.1}ms\n");

    println!("## 질문별 결과 (예산 {budget} 토큰)");
    println!("| # | 조립 ms | Lantern 토큰 | 기준 토큰 | 항목 | 질문 |");
    println!("|---|---|---|---|---|---|");
    let mut latencies = Vec::new();
    let (mut total_k, mut total_b) = (0usize, 0usize);
    for (i, q) in qs.iter().enumerate() {
        let r = engine.context(&ContextRequest { budget_tokens: budget, ..ContextRequest::new(*q) })?;
        latencies.push(r.elapsed_ms);

        // 기준 방식: 검색 상위 10개 결과가 속한 파일을 통째로 넣기
        let mut files = HashSet::new();
        for (sym, _) in engine.search(q, 10)? {
            files.insert(sym.path);
        }
        let baseline: usize = files
            .iter()
            .filter_map(|p| std::fs::read(engine.root.join(p)).ok())
            .map(|b| estimate_tokens(&String::from_utf8_lossy(&b)))
            .sum();

        total_k += r.used_tokens;
        total_b += baseline;
        println!(
            "| {} | {:.1} | {} | {} | {} | {} |",
            i + 1,
            r.elapsed_ms,
            r.used_tokens,
            baseline,
            r.items.len(),
            q
        );
    }

    latencies.sort_by(|a, b| a.total_cmp(b));
    let pct = |p: f64| latencies.get(((latencies.len() as f64 * p).ceil() as usize).saturating_sub(1)).copied().unwrap_or(0.0);
    println!("\n## 요약");
    println!("- 맥락 조립 p50 {:.1}ms · p95 {:.1}ms", pct(0.5), pct(0.95));
    if total_b > 0 {
        let saving = 100.0 * (1.0 - total_k as f64 / total_b as f64);
        println!("- 토큰: Lantern {total_k} vs 기준 {total_b} → {saving:.0}% 절감");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn parses_line_ranges() {
        assert_eq!(parse_range("10-20").unwrap(), (10, 20));
        assert_eq!(parse_range("15").unwrap(), (15, 15));
        assert_eq!(parse_range(" 3 - 4 ").unwrap(), (3, 4));
        for bad in ["0", "20-10", "a-b", "", "5-"] {
            assert!(parse_range(bad).is_err(), "{bad}");
        }
    }
}
