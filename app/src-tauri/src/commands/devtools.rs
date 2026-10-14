//! Tauri 명령: 터미널과 언어 서버

use crate::state::{file_uri, AppState};
use crate::{applog, config, lsp, lspinstall, terminal};
use crate::{anyhow_err, err, CmdResult};
use tauri::{AppHandle, State};

#[tauri::command]
pub(crate) async fn term_spawn(app: AppHandle, state: State<'_, AppState>, cols: u16, rows: u16) -> CmdResult<u32> {
    let root = state
        .project()
        .map(|p| p.root.clone())
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default());
    let (id, term) = terminal::spawn(app, &root, cols, rows).map_err(anyhow_err)?;
    state.terminals.lock().unwrap().insert(id, term);
    Ok(id)
}

#[tauri::command]
pub(crate) async fn term_write(state: State<'_, AppState>, id: u32, data: String) -> CmdResult<()> {
    if let Some(t) = state.terminals.lock().unwrap().get_mut(&id) {
        t.write(&data).map_err(anyhow_err)?;
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn term_resize(state: State<'_, AppState>, id: u32, cols: u16, rows: u16) -> CmdResult<()> {
    if let Some(t) = state.terminals.lock().unwrap().get(&id) {
        t.resize(cols, rows).map_err(anyhow_err)?;
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn term_kill(state: State<'_, AppState>, id: u32) -> CmdResult<()> {
    if let Some(mut t) = state.terminals.lock().unwrap().remove(&id) {
        t.kill();
    }
    Ok(())
}

/// 언어 서버 시작. 이미 떠 있으면 그대로 둔다. 설정이 없거나 설치되지 않았으면 오류.
#[tauri::command]
pub(crate) async fn lsp_start(app: AppHandle, state: State<'_, AppState>, lang: String) -> CmdResult<()> {
    if state.lsps.lock().unwrap().contains_key(&lang) {
        return Ok(());
    }
    let p = state.project().map_err(anyhow_err)?;
    let cfg = config::load(p.config_root()).map_err(anyhow_err)?;
    let l = cfg.lsp.get(&lang).ok_or_else(|| format!("[lsp.{lang}] 설정이 없습니다"))?;
    let server = lsp::start(app, &lang, l, &p.root, &file_uri(&p.root)).map_err(anyhow_err)?;
    state.lsps.lock().unwrap().insert(lang, server);
    Ok(())
}

/// 언어 서버를 찾지 못했을 때 설치 방법. 이미 있으면 None (ignore_found면 있어도 계획을 준다: rustup 셈처럼 껍데기만 있는 경우)
#[tauri::command]
pub(crate) async fn lsp_install_plan(state: State<'_, AppState>, lang: String, ignore_found: bool) -> CmdResult<Option<lspinstall::InstallPlan>> {
    let p = state.project().map_err(anyhow_err)?;
    let cfg = config::load(p.config_root()).map_err(anyhow_err)?;
    let Some(l) = cfg.lsp.get(&lang) else { return Ok(None) };
    let found = which::which(&l.command).is_ok() || lspinstall::installed_bin(&l.command).is_some();
    if found && !ignore_found {
        return Ok(None);
    }
    Ok(lspinstall::plan(&l.command))
}

#[tauri::command]
pub(crate) async fn lsp_install(state: State<'_, AppState>, lang: String) -> CmdResult<()> {
    let p = state.project().map_err(anyhow_err)?;
    let cfg = config::load(p.config_root()).map_err(anyhow_err)?;
    let command = cfg.lsp.get(&lang).ok_or_else(|| format!("[lsp.{lang}] 설정이 없습니다"))?.command.clone();
    tauri::async_runtime::spawn_blocking(move || lspinstall::install(&command))
        .await
        .map_err(err)?
        .map_err(|e| {
            applog::error(&format!("언어 서버 설치 실패: {e:#}"));
            anyhow_err(e)
        })
}

#[tauri::command]
pub(crate) async fn lsp_send(state: State<'_, AppState>, lang: String, msg: String) -> CmdResult<()> {
    let mut lsps = state.lsps.lock().unwrap();
    let l = lsps.get_mut(&lang).ok_or("언어 서버가 실행 중이 아닙니다")?;
    l.send(&msg).map_err(anyhow_err)
}
