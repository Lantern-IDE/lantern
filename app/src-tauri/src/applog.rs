//! 앱 로그와 충돌 보고서. 모두 데이터 폴더의 logs/ 아래 로컬 파일로만 남고, 어디에도 보내지 않는다.
//! 사용자가 "문제 보고"를 누르면 비밀 값을 가린 보고서를 만들어 직접 복사해 붙이게 한다.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
const REPORT_LOG_LINES: usize = 200;

pub fn log_dir() -> Option<PathBuf> {
    crate::state::data_dir().map(|d| d.join("logs"))
}

/// UTC 기준 "2026-09-24 12:34:56"
fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // 1970-01-01부터의 일수 → 그레고리력 날짜 (Howard Hinnant 알고리즘)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// 한 줄 기록. 로그 쓰기가 실패해도 앱은 계속 돈다.
pub fn write(level: &str, msg: &str) {
    if let Some(dir) = log_dir() {
        write_in(&dir, level, msg);
    }
}

fn write_in(dir: &Path, level: &str, msg: &str) {
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join("lantern.log");
    if path.metadata().map(|m| m.len() > MAX_LOG_BYTES).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join("lantern.old.log"));
    }
    let (clean, _) = lantern_context::secrets::redact(msg);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{} [{level}] {}", timestamp(), clean.replace('\n', "\n    "));
    }
}

pub fn info(msg: &str) {
    write("INFO", msg);
}

pub fn error(msg: &str) {
    write("ERROR", msg);
}

fn system_line() -> String {
    format!("Lantern {} · {} {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH)
}

/// 패닉이 나면 충돌 보고서를 남긴다. 다음 실행 때 사용자에게 알린다.
pub fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let text = format!("{}\n{}\n\n{info}\n\n{bt}", system_line(), timestamp());
        error(&format!("충돌: {info}"));
        if let Some(dir) = log_dir() {
            let _ = std::fs::create_dir_all(&dir);
            let name = format!("crash-{}.txt", SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
            let _ = std::fs::write(dir.join(name), lantern_context::secrets::redact(&text).0);
        }
        prev(info);
    }));
}

fn crash_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt") && !n.ends_with(".seen.txt")))
        .collect();
    v.sort();
    v
}

/// 아직 알리지 않은 충돌 보고서가 있으면 가장 최근 것의 첫 줄들을 돌려주고, 알린 것으로 표시한다.
pub fn take_pending_crash() -> Option<String> {
    take_pending_crash_in(&log_dir()?)
}

fn take_pending_crash_in(dir: &Path) -> Option<String> {
    let files = crash_files(dir);
    let last = files.last()?.clone();
    let text = std::fs::read_to_string(&last).ok()?;
    for f in files {
        let _ = std::fs::rename(&f, f.with_extension("seen.txt"));
    }
    Some(text.lines().take(6).collect::<Vec<_>>().join("\n"))
}

/// 문제 보고용 텍스트: 버전, OS, 최근 로그, 최근 충돌. 비밀 값은 가린다.
pub fn report() -> String {
    report_in(log_dir().as_deref())
}

fn report_in(dir: Option<&Path>) -> String {
    let mut out = format!("## 환경\n{}\n\n", system_line());
    if let Some(dir) = dir {
        let log = std::fs::read_to_string(dir.join("lantern.log")).unwrap_or_default();
        let lines: Vec<&str> = log.lines().collect();
        let tail = lines[lines.len().saturating_sub(REPORT_LOG_LINES)..].join("\n");
        out.push_str(&format!("## 최근 로그 ({}줄)\n```\n{}\n```\n", lines.len().min(REPORT_LOG_LINES), tail));
        let crashes: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("crash-")))
            .collect();
        if let Some(latest) = crashes.iter().max() {
            let text = std::fs::read_to_string(latest).unwrap_or_default();
            let short: String = text.lines().take(60).collect::<Vec<_>>().join("\n");
            out.push_str(&format!("\n## 최근 충돌\n```\n{short}\n```\n"));
        }
    }
    lantern_context::secrets::redact(&out).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_format() {
        let t = timestamp();
        assert_eq!(t.len(), 19);
        assert!(t.starts_with("20"));
    }

    #[test]
    fn log_report_and_crash() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write_in(dir, "INFO", "시작");
        write_in(dir, "ERROR", "키 sk-ant-api03-abcdefghijklmnopqrstuvwx 로 실패");
        let r = report_in(Some(dir));
        assert!(r.contains("시작") && r.contains("실패"));
        assert!(!r.contains("sk-ant-api03"), "비밀은 가린다");
        assert!(take_pending_crash_in(dir).is_none());
        std::fs::write(dir.join("crash-1.txt"), "Lantern\n패닉").unwrap();
        assert!(take_pending_crash_in(dir).unwrap().contains("패닉"));
        assert!(take_pending_crash_in(dir).is_none(), "한 번만 알린다");
        assert!(report_in(Some(dir)).contains("최근 충돌"));
    }
}
