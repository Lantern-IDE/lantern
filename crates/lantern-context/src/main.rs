use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use lantern_context::assemble::{ContextRequest, DEFAULT_BUDGET};
use lantern_context::tokenize::estimate_tokens;
use lantern_context::Engine;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

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
    let cli = Cli::parse();
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
            let result = engine.context(&ContextRequest { query, file, line, budget_tokens: budget })?;
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
