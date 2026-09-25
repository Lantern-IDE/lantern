//! 통합 터미널: 의사 터미널(PTY)에 셸을 띄우고 입출력을 화면과 주고받는다.

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde_json::json;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use tauri::{AppHandle, Emitter};

pub struct Term {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
}

static NEXT: AtomicU32 = AtomicU32::new(1);

fn default_shell() -> String {
    if cfg!(windows) {
        "powershell.exe".into()
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into())
    }
}

pub fn spawn(app: AppHandle, root: &Path, cols: u16, rows: u16) -> Result<(u32, Term)> {
    let pair = native_pty_system()
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .context("터미널 생성 실패")?;
    let mut cmd = CommandBuilder::new(default_shell());
    cmd.cwd(root);
    let child = pair.slave.spawn_command(cmd).context("셸 실행 실패")?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    let id = NEXT.fetch_add(1, Ordering::Relaxed);

    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        let mut pending: Vec<u8> = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    pending.extend_from_slice(&buf[..n]);
                    // 청크 경계에서 잘린 UTF-8 문자는 다음 청크와 합쳐서 보낸다.
                    let valid = match std::str::from_utf8(&pending) {
                        Ok(_) => pending.len(),
                        Err(e) if e.error_len().is_none() => e.valid_up_to(),
                        Err(_) => pending.len(),
                    };
                    let text = String::from_utf8_lossy(&pending[..valid]).into_owned();
                    pending.drain(..valid);
                    let _ = app.emit("term-data", json!({ "id": id, "data": text }));
                }
            }
        }
        let _ = app.emit("term-exit", json!({ "id": id }));
    });

    Ok((id, Term { master: pair.master, writer, child }))
}

impl Term {
    pub fn write(&mut self, data: &str) -> Result<()> {
        self.writer.write_all(data.as_bytes())?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        Ok(())
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }
}
