// 릴리스 빌드에서 콘솔 창을 띄우지 않는다.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod acp;
mod agent;
mod agents;
mod config;
mod applog;
mod inline;
mod review;
mod semantic;
mod mcp_client;
mod verify;
mod webfetch;
mod compact;
mod mentions;
mod tasks;
mod worktree;
mod commands;
mod completion;
mod fswatch;
mod lspinstall;
mod gitops;
mod hooks;
mod llm;
mod lsp;
mod settings;
mod state;
mod terminal;
mod tools;
mod workspace;

#[cfg(test)]
mod e2e_tests;

use lantern_context::Engine;
use state::{file_uri, AppState};
use std::time::Duration;
use tauri::{Manager, State};

pub(crate) type CmdResult<T> = Result<T, String>;

pub(crate) fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub(crate) fn anyhow_err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

/// 소스 제어가 쓸 저장소 폴더. git은 저장소 설정(훅, fsmonitor 등)으로 명령을 실행할 수 있어서
/// 신뢰하지 않은 폴더(제한 모드)에서는 쓰지 않는다 (VS Code와 같음).
pub(crate) fn git_repo(state: &AppState, repo: &str) -> anyhow::Result<std::path::PathBuf> {
    let p = state.project()?;
    if !p.is_trusted() {
        anyhow::bail!("제한 모드에서는 소스 제어를 쓸 수 없습니다. 이 폴더를 신뢰하면 켜집니다");
    }
    gitops::resolve(&p.root, repo)
}

/// git 작업을 백그라운드 스레드에서 돈다 (네트워크 작업은 오래 걸릴 수 있다)
pub(crate) async fn git_blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(err)?.map_err(anyhow_err)
}

/// 프로젝트 설정을 읽어도 되는 루트. 제한 모드면 None (전역 설정만 쓴다).
pub(crate) fn project_root(state: &State<'_, AppState>) -> Option<std::path::PathBuf> {
    state.project().ok().and_then(|p| p.config_root().map(|r| r.to_path_buf()))
}

pub(crate) fn key_name_for(state: &State<'_, AppState>, model_key: &str) -> Result<String, String> {
    let cfg = config::load(project_root(state).as_deref()).map_err(anyhow_err)?;
    let m = cfg.models.get(model_key).ok_or_else(|| format!("모델 '{model_key}'이 없습니다"))?;
    m.api_key_env.clone().ok_or_else(|| "이 모델은 API 키 이름(api_key_env)이 없습니다".to_string())
}

pub(crate) fn assemble_rel(root: &std::path::Path, path: &str) -> String {
    lantern_context::assemble::normalize_path(root, path)
}

// ── 지식그래프 ──────────────────────────────────────────────

/// 그래프 질의를 색인 잠금 안에서 실행한다 (먼저 증분 갱신)
pub(crate) async fn with_engine<T: Send + 'static>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&lantern_context::Engine, &std::path::Path) -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    let p = state.project().map_err(anyhow_err)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut e = p.engine.lock().unwrap();
        let _ = p.refresh_index(&mut e);
        f(&e, &p.root)
    })
    .await
    .map_err(err)?
    .map_err(anyhow_err)
}

/// `lantern-app --mcp <폴더>`: 창 없이 맥락 엔진을 MCP 서버(stdio)로 돌린다.
/// 외부 에이전트(Gemini CLI, Codex 등)에게 Lantern 맥락 엔진을 넘길 때 쓴다.
fn mcp_mode() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    let i = args.iter().position(|a| a == "--mcp")?;
    let root = args.get(i + 1).map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from("."));
    let run = || -> anyhow::Result<()> {
        let mut engine = Engine::open(&root)?;
        engine.refresh()?;
        lantern_context::mcp::serve(&mut engine)
    };
    Some(match run() {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("lantern mcp: {e:#}");
            1
        }
    })
}

fn main() {
    if let Some(code) = mcp_mode() {
        std::process::exit(code);
    }
    applog::install_panic_hook();
    // 예전 이름(KHALA)의 데이터를 먼저 옮긴다 (로그도 새 자리에 쌓이도록)
    let moved = state::migrate_legacy_data();
    tasks::prune_checkpoints();
    applog::info(&format!("Lantern {} 시작", env!("CARGO_PKG_VERSION")));
    for m in moved {
        applog::info(&format!("예전 데이터 옮김 — {m}"));
    }
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .build()
        .expect("HTTP 클라이언트");
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState { http, ..Default::default() })
        .setup(|app| {
            // 창은 코드에서 만든다. LANTERN_WEBVIEW_DATA_DIR을 주면 WebView 데이터(localStorage 등)를
            // 별도 폴더에 두어, 이미 실행 중인 Lantern과 충돌 없이 두 번째 인스턴스를 띄울 수 있다.
            let mut builder = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
                .title("Lantern")
                .inner_size(1440.0, 900.0)
                .min_inner_size(900.0, 560.0)
                .shadow(true)
                .background_color(tauri::window::Color(0x18, 0x18, 0x18, 0xff))
                // 파일 끌어 놓기를 웹 화면이 받는다 (채팅에 이미지 붙이기)
                .disable_drag_drop_handler();
            // Windows·Linux는 제목 표시줄을 직접 그린다 (창 조작 버튼 포함).
            // macOS는 시스템 신호등 버튼을 제목 표시줄 위에 겹쳐 두고, 그 자리만 비운다 (styles.css의 .mac).
            #[cfg(not(target_os = "macos"))]
            {
                builder = builder.decorations(false);
            }
            #[cfg(target_os = "macos")]
            {
                builder = builder.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true);
            }
            if let Some(dir) = std::env::var_os("LANTERN_WEBVIEW_DATA_DIR") {
                builder = builder.data_directory(dir.into());
            }
            // E2E 전용: 원격 디버깅 포트를 연다. WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS 환경변수는
            // WebView2 런타임 버전에 따라 Tauri가 넘기는 인자에 밀려 무시돼서(CI) 인자로 직접 준다.
            // Tauri 기본 인자를 대신하므로 그 값(--disable-features=...)도 함께 넘긴다.
            #[cfg(windows)]
            if let Some(port) = std::env::var("LANTERN_E2E_CDP_PORT").ok().filter(|p| p.parse::<u16>().is_ok()) {
                builder = builder.additional_browser_args(&format!(
                    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --remote-debugging-port={port}"
                ));
            }
            builder.build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::agent::acp_authenticate,
            commands::scm::review_changes,
            commands::scm::review_target,
            commands::project::config_ignored,
            commands::scm::commit_message,
            commands::agent::inline_edit,
            commands::agent::test_examples,
            commands::settings::test_embeddings,
            commands::settings::semantic_status,
            commands::settings::set_embeddings_key,
            commands::settings::mcp_status,
            commands::project::open_project,
            commands::project::set_trust,
            commands::project::startup_path,
            commands::files::list_dir,
            commands::files::list_files,
            commands::files::create_path,
            commands::files::rename_path,
            commands::files::delete_path,
            commands::files::reveal_path,
            commands::files::search_text,
            commands::files::replace_text,
            commands::scm::git_status,
            commands::scm::git_diff,
            commands::scm::git_action,
            commands::scm::git_repos,
            commands::scm::git_log,
            commands::scm::git_commit_files,
            commands::scm::git_commit_diff,
            commands::scm::git_branches,
            commands::scm::git_checkout,
            commands::files::read_file,
            commands::files::write_file,
            commands::settings::ensure_config,
            commands::settings::get_settings,
            commands::settings::set_settings,
            commands::settings::set_api_key,
            commands::settings::delete_api_key,
            commands::settings::test_model,
            commands::settings::probe_local,
            commands::agent::list_agents,
            commands::settings::model_info,
            commands::agent::agent_send,
            commands::agent::agent_cancel,
            commands::agent::agent_reset,
            commands::agent::resolve_approval,
            commands::tasks::revert_checkpoint,
            commands::tasks::task_list,
            commands::tasks::task_save,
            commands::tasks::task_delete,
            commands::tasks::worktree_create,
            commands::tasks::worktree_changes,
            commands::tasks::worktree_apply,
            commands::tasks::worktree_discard,
            commands::graph::graph_overview,
            commands::graph::graph_neighborhood,
            commands::graph::graph_impact,
            commands::graph::graph_memory,
            commands::graph::graph_find,
            commands::graph::graph_enclosing,
            commands::agent::inline_complete,
            commands::settings::get_keybindings,
            commands::system::log_frontend,
            commands::system::check_update,
            commands::system::install_update,
            commands::system::problem_report,
            commands::system::pending_crash,
            commands::system::open_log_dir,
            commands::settings::set_keybindings,
            commands::graph::context_preview,
            commands::devtools::term_spawn,
            commands::devtools::term_write,
            commands::devtools::term_resize,
            commands::devtools::term_kill,
            commands::devtools::lsp_start,
            commands::devtools::lsp_send,
            commands::devtools::lsp_install_plan,
            commands::devtools::lsp_install,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let st = window.app_handle().state::<AppState>();
                for (_, mut t) in st.terminals.lock().unwrap().drain() {
                    t.kill();
                }
                for (_, mut l) in st.lsps.lock().unwrap().drain() {
                    l.stop();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("Lantern 실행 실패");
}
