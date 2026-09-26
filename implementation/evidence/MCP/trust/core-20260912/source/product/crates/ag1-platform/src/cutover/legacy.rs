//! Admission of an explicitly selected legacy state with no running owner.
//! This observes processes; it neither signals them nor resumes legacy work.
use super::*;

fn stopped(workspace: &Path, state: &Path) -> Result<()> {
    let uid = unsafe { libc::geteuid() };
    for entry in fs::read_dir("/proc").map_err(io)? {
        let entry = entry.map_err(io)?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|v| v.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == std::process::id() {
            continue;
        }
        let path = entry.path();
        let name = match fs::read_to_string(path.join("comm")) {
            Ok(name) => name,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(io(e)),
        };
        let legacy_engine = name.trim().contains("ag1") || matches!(name.trim(), "node" | "bun");
        let check = (|| -> std::io::Result<bool> {
            if fs::metadata(&path)?.uid() != uid {
                return Ok(false);
            }
            let exe = fs::read_link(path.join("exe"))?;
            let name = exe.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if fs::read_link(path.join("cwd"))? == workspace
                && (name.contains("ag1") || name == "node" || name == "bun")
            {
                return Ok(true);
            }
            for fd in fs::read_dir(path.join("fd"))? {
                match fs::read_link(fd?.path()) {
                    Ok(target) if target == state || target.starts_with(state) => return Ok(true),
                    Ok(_) => (),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                    Err(e) => return Err(e),
                }
            }
            Ok(false)
        })();
        match check {
            Ok(false) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            // Privileged desktop services are outside the supported legacy
            // engine domain. Inaccessible AG1/Node/Bun processes still block.
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && !legacy_engine => (),
            _ => {
                return Err(DomainError::Unavailable(
                    "Legacy owner is active or cannot be verified stopped; no authority registered"
                        .into(),
                ));
            }
        }
    }
    Ok(())
}
impl Registry {
    /// Register a complete frozen legacy input only after two stopped-owner
    /// observations and source validation. This writes host authority metadata
    /// only; no legacy file is opened for writing or recovered in place.
    pub fn register_stopped_legacy(
        &self,
        workspace: &Path,
        state: &Path,
        manifest: &FrozenManifest,
    ) -> Result<Record> {
        let mut lease = self.locked(workspace)?;
        if lease.record.is_some() {
            return Err(DomainError::Conflict);
        }
        let state = fs::canonicalize(state).map_err(io)?;
        if state.starts_with(&lease.workspace) || lease.workspace.starts_with(&state) {
            return Err(DomainError::PermissionDenied);
        }
        stopped(&lease.workspace, &state)?;
        manifest.preflight(&state)?;
        stopped(&lease.workspace, &state)?;
        let record = Record {
            version: 1,
            workspace: lease.workspace.clone(),
            device: lease.device,
            inode: lease.inode,
            epoch: 1,
            owner: Owner::TypeScript,
            stage: Stage::Active,
            state: Some(state),
            isolated: false,
            quiescent: true,
            transfer: None,
            rollback: None,
        };
        lease.save(record.clone())?;
        Ok(record)
    }
}
