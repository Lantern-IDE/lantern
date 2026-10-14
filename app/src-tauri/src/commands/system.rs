//! Tauri 명령: 업데이트·로그·문제 보고

use crate::{applog, workspace};
use crate::{anyhow_err, err, CmdResult};
use tauri::AppHandle;

#[derive(serde::Serialize)]
pub(crate) struct UpdateInfo {
    version: String,
    notes: Option<String>,
}

/// 새 버전이 있으면 알려준다. 배포 주소가 아직 없거나 오프라인이면 오류 대신 None.
#[tauri::command]
pub(crate) async fn check_update(app: AppHandle) -> CmdResult<Option<UpdateInfo>> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(err)?;
    match updater.check().await {
        Ok(Some(u)) => Ok(Some(UpdateInfo { version: u.version.clone(), notes: u.body.clone() })),
        Ok(None) => Ok(None),
        Err(e) => {
            applog::info(&format!("업데이트 확인 실패: {e}"));
            Err(format!("업데이트 서버에 연결하지 못했습니다: {e}"))
        }
    }
}

/// 내려받아 설치하고 다시 시작한다 (서명이 맞지 않으면 설치하지 않는다).
#[tauri::command]
pub(crate) async fn install_update(app: AppHandle) -> CmdResult<()> {
    use tauri_plugin_updater::UpdaterExt;
    let update = app.updater().map_err(err)?.check().await.map_err(err)?.ok_or("설치할 새 버전이 없습니다")?;
    applog::info(&format!("업데이트 설치: {}", update.version));
    update.download_and_install(|_, _| {}, || {}).await.map_err(err)?;
    app.restart();
}

/// 프런트엔드 오류·경고를 앱 로그에 남긴다
#[tauri::command]
pub(crate) fn log_frontend(level: String, message: String) {
    let level = if level == "error" { "UI-ERROR" } else { "UI" };
    applog::write(level, &message.chars().take(4000).collect::<String>());
}

#[tauri::command]
pub(crate) fn problem_report() -> String {
    applog::report()
}

#[tauri::command]
pub(crate) fn pending_crash() -> Option<String> {
    applog::take_pending_crash()
}

#[tauri::command]
pub(crate) fn open_log_dir() -> CmdResult<()> {
    let dir = applog::log_dir().ok_or("데이터 폴더를 찾지 못했습니다")?;
    std::fs::create_dir_all(&dir).map_err(err)?;
    let target = if dir.join("lantern.log").exists() { dir.join("lantern.log") } else { dir };
    workspace::reveal_in_os(&target).map_err(anyhow_err)
}
