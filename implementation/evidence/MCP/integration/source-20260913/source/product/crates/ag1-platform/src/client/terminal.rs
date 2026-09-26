//! Explicit workspace terminal, owned and drained independently of React.
use super::*;
use ag1_core::{PtyPort, PtySize};
pub(super) struct Terminal {
    pty: crate::process::OwnedPty,
    text: String,
    sequence: u64,
}
impl Host {
    pub(super) fn terminal_command(&self, action: &str, args: &Value) -> Result<Value> {
        let mut slot = self.terminal.lock().map_err(|_| DomainError::Conflict)?;
        match action {
            "start" => {
                if slot.is_some() {
                    return Err(DomainError::Conflict);
                }
                let tooling = self.config.tooling.as_ref().ok_or_else(|| {
                    DomainError::Unavailable(
                        "Configure the native process helper to open a terminal".into(),
                    )
                })?;
                let workspace = self.working_folder(false)?;
                workspace.verify_root()?;
                let state = self.state.join("terminal");
                fs::create_dir_all(&state).map_err(crate::io_error)?;
                let home = self.state.join("home");
                let pty = crate::process::ProcessSupervisor::new(&tooling.process_host)?
                    .spawn_pty(
                        crate::process::ProcessSpec {
                            executable: "/bin/bash".into(),
                            argv: vec!["--noprofile".into(), "--norc".into(), "-i".into()],
                            cwd: workspace.root().into(),
                            env: std::collections::BTreeMap::from([
                                ("PATH".into(), "/usr/bin:/bin".into()),
                                ("HOME".into(), home.display().to_string()),
                                ("TERM".into(), "dumb".into()),
                                ("PS1".into(), "ag1> ".into()),
                            ]),
                            timeout_ms: 86400000,
                            grace_ms: 500,
                            resident: false,
                        },
                        &state.join(format!("{}.log", nonce()?)),
                        PtySize {
                            rows: 24,
                            columns: 100,
                        },
                    )?;
                *slot = Some(Terminal {
                    pty,
                    text: String::new(),
                    sequence: 0,
                });
            }
            "input" => {
                let text = args["text"].as_str().ok_or(DomainError::NotFound)?;
                let written = slot
                    .as_mut()
                    .ok_or(DomainError::NotFound)?
                    .pty
                    .write_input(text.as_bytes())?;
                return Ok(json!({"written":written}));
            }
            "resize" => {
                let rows = args["rows"]
                    .as_u64()
                    .filter(|v| *v <= 1000)
                    .ok_or(DomainError::NotFound)? as u16;
                let columns = args["columns"]
                    .as_u64()
                    .filter(|v| *v <= 1000)
                    .ok_or(DomainError::NotFound)? as u16;
                slot.as_mut()
                    .ok_or(DomainError::NotFound)?
                    .pty
                    .resize(PtySize { rows, columns })?;
            }
            "stop" => {
                if let Some(terminal) = slot.as_mut() {
                    terminal.pty.disconnect()?;
                }
                *slot = None;
            }
            "status" => {}
            _ => return Err(DomainError::NotFound),
        }
        Ok(match slot.as_ref() {
            Some(t) => json!({"open":true,"text":t.text,"sequence":t.sequence}),
            None => json!({"open":false,"text":"","sequence":0}),
        })
    }
    pub(super) fn poll_terminal(&self) -> Result<()> {
        let mut slot = self.terminal.lock().map_err(|_| DomainError::Conflict)?;
        if let Some(t) = slot.as_mut() {
            let bytes = t.pty.read_output(32768)?;
            if !bytes.is_empty() {
                t.sequence += 1;
                t.text.push_str(&String::from_utf8_lossy(&bytes));
                while t.text.len() > 16384 {
                    let end = t
                        .text
                        .char_indices()
                        .nth(1024)
                        .map_or(t.text.len(), |(i, _)| i);
                    t.text.drain(..end);
                }
            }
        }
        Ok(())
    }
}
