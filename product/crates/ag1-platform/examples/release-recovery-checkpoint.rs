//! Disposable P14 recovery driver using native archive/restore and OS authority.
//! No inference, production-state mutation, or authority namespace substitution.
use ag1_platform::cutover::{Owner, Registry, Stage, restore_native_archive};
use serde::Deserialize;
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    workspace: PathBuf,
    archive: PathBuf,
    #[serde(default)]
    restore_sha256: Option<String>,
    #[serde(default)]
    retained_corrupt_state: Option<PathBuf>,
    #[serde(default)]
    restored_staging: Option<PathBuf>,
}
fn checked_new(path: &Path, root: &Path) -> Result<PathBuf, String> {
    if path.exists() || !path.is_absolute() {
        return Err("New absolute destination required".into());
    }
    let parent = path
        .parent()
        .ok_or("Destination parent")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !parent.starts_with(root) {
        return Err("Recovery destination outside disposable workload".into());
    }
    Ok(parent.join(path.file_name().ok_or("Destination filename")?))
}
fn main() -> Result<(), String> {
    let path = std::env::args()
        .nth(1)
        .ok_or("Private recovery request required")?;
    let input: Input = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join(".local/p14-long-horizon")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let workspace = input.workspace.canonicalize().map_err(|e| e.to_string())?;
    if !workspace.starts_with(&base) {
        return Err("Disposable P14 workspace required".into());
    }
    let root = workspace.parent().ok_or("Workload root")?;
    let registry = Registry::for_user().map_err(|e| format!("{e:?}"))?;
    let before = registry
        .inspect(&workspace)
        .map_err(|e| format!("{e:?}"))?
        .ok_or("Native authority absent")?;
    if !before.isolated
        || before.owner != Owner::Native
        || before.stage != Stage::Active
        || before.transfer.is_some()
        || before.rollback.is_some()
    {
        return Err("Isolated native authority required".into());
    }
    let state = before
        .state
        .as_ref()
        .ok_or("Native state is not initialized")?;
    if !state.starts_with(root) || state.starts_with(&workspace) {
        return Err("Disposable state boundary mismatch".into());
    }
    if let Some(sha) = input.restore_sha256 {
        let archive = input.archive.canonicalize().map_err(|e| e.to_string())?;
        if !archive.starts_with(root) {
            return Err("Archive outside disposable workload".into());
        }
        let retained = checked_new(
            input
                .retained_corrupt_state
                .as_deref()
                .ok_or("Retained corrupt destination required")?,
            root,
        )?;
        let staged = checked_new(
            input
                .restored_staging
                .as_deref()
                .ok_or("Fresh restoration destination required")?,
            root,
        )?;
        // This actual workspace lock excludes any running CLI/desktop owner.
        // No stored PID or archived authority is used to acquire execution.
        let mut lease = registry
            .open_native(&workspace, state)
            .map_err(|e| format!("{e:?}"))?;
        if lease.record().map_err(|e| format!("{e:?}"))?.state.as_ref() != Some(state) {
            return Err("State binding changed".into());
        }
        let restored =
            restore_native_archive(&archive, &staged, &sha).map_err(|e| format!("{e:?}"))?;
        if restored.authority.workspace != workspace
            || restored.authority.state.as_ref() != Some(state)
        {
            return Err("Archive belongs to another state/workspace".into());
        }
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            state,
            rustix::fs::CWD,
            &retained,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(|e| e.to_string())?;
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            &staged,
            rustix::fs::CWD,
            state,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(|e| e.to_string())?;
        fs::File::open(root)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        lease.quiesced().map_err(|e| format!("{e:?}"))?;
        println!(
            "{}",
            json!({"status":"restored","workspace":workspace,"state":state,"retained_corrupt_state":retained,"archive_sha256":sha,"authority_transferred":false,"runtime_actions":0})
        );
    } else {
        if !before.quiescent
            || input.retained_corrupt_state.is_some()
            || input.restored_staging.is_some()
        {
            return Err(
                "Checkpoint requires a previously drained owner and no restore arguments".into(),
            );
        }
        let archive = checked_new(&input.archive, root)?;
        let mut lease = registry
            .open_native(&workspace, state)
            .map_err(|e| format!("{e:?}"))?;
        lease.begin_drain().map_err(|e| format!("{e:?}"))?;
        // The prior owner was already quiescent, and this locked administrative
        // operation has created no runtime, tool, or generation work.
        lease.quiesced().map_err(|e| format!("{e:?}"))?;
        let archived = lease
            .rollback_archive(&archive)
            .map_err(|e| format!("{e:?}"));
        let cancelled = lease.cancel_transfer().map_err(|e| format!("{e:?}"));
        let receipt = archived?;
        cancelled?;
        println!(
            "{}",
            json!({"status":"archived","workspace":workspace,"state":state,"archive":archive,"receipt":receipt,"authority_transferred":false,"runtime_actions":0})
        );
    }
    Ok(())
}
