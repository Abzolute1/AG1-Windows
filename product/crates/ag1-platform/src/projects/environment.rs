//! Host-configured tool capabilities shared by project checks and shell/jobs.
use super::Toolchains;
use ag1_core::DomainError;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

impl Toolchains {
    pub fn environment(&self, workspace: &Path) -> Result<BTreeMap<String, String>, DomainError> {
        let mut paths = Vec::new();
        for executable in self
            .node
            .iter()
            .chain(self.npm.iter())
            .chain(self.cargo.iter())
            .chain(self.rustc.iter())
            .chain(self.godot.iter())
        {
            let resolved = executable.canonicalize().map_err(|_| {
                DomainError::Unavailable(format!(
                    "Configured toolchain executable is missing: {}",
                    executable.display()
                ))
            })?;
            if !executable.is_absolute() || !resolved.is_file() || resolved.starts_with(workspace) {
                return Err(DomainError::PermissionDenied);
            }
            if let Some(parent) = executable.parent() {
                paths.push(parent.to_owned());
            }
        }
        #[cfg(unix)]
        paths.extend([PathBuf::from("/usr/bin"), PathBuf::from("/bin")]);
        #[cfg(windows)]
        if let Some(root) = std::env::var_os("SystemRoot") {
            paths.push(PathBuf::from(root).join("System32"));
        }
        let path = std::env::join_paths(paths)
            .map_err(|_| DomainError::InvalidInput("toolchain PATH".into()))?;
        let mut env = BTreeMap::from([
            ("PATH".into(), path.to_string_lossy().into_owned()),
            (
                "CARGO_HOME".into(),
                workspace.join("target/ag1-cargo").display().to_string(),
            ),
            (
                "CARGO_TARGET_DIR".into(),
                workspace.join("target").display().to_string(),
            ),
            ("RUSTUP_AUTO_INSTALL".into(), "0".into()),
            ("RUSTUP_NO_UPDATE_CHECK".into(), "1".into()),
        ]);
        if let Some(rustc) = &self.rustc {
            env.insert("RUSTC".into(), rustc.display().to_string());
        }
        if let Some(root) = self.rustup_home() {
            env.insert("RUSTUP_HOME".into(), root.display().to_string());
        }
        Ok(env)
    }

    pub(super) fn rustup_home(&self) -> Option<PathBuf> {
        // Conventional configured rustup shims, including Windows .exe shims.
        // Never inherit arbitrary host environment or change sandbox HOME.
        self.mounts
            .iter()
            .find(|root| root.join("settings.toml").is_file() && root.join("toolchains").is_dir())
            .cloned()
            .or_else(|| {
                self.cargo
                    .iter()
                    .chain(self.rustc.iter())
                    .find_map(|executable| {
                        let bin = executable.parent()?;
                        let cargo = bin.parent()?;
                        if bin.file_name()? != "bin" || cargo.file_name()? != ".cargo" {
                            return None;
                        }
                        let root = cargo.parent()?.join(".rustup");
                        root.is_dir().then_some(root)
                    })
            })
    }
}

/// Resolve individual trust files, including distro symlinks out of /etc/ssl.
/// canonicalize is OS-bounded (symlink loops fail); cap metadata before mounting.
pub fn trust_bundles(candidates: &[&Path]) -> Result<Vec<PathBuf>, DomainError> {
    let mut bundles = Vec::new();
    for candidate in candidates {
        match candidate.symlink_metadata() {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                return Err(DomainError::Unavailable(
                    "Cannot inspect TLS trust bundle".into(),
                ));
            }
            Ok(_) => {}
        }
        let resolved = candidate.canonicalize().map_err(|_| {
            DomainError::Unavailable(format!(
                "Cannot resolve TLS trust bundle {}",
                candidate.display()
            ))
        })?;
        let metadata = resolved
            .metadata()
            .map_err(|_| DomainError::Unavailable("Cannot read TLS trust bundle".into()))?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 16 * 1024 * 1024 {
            return Err(DomainError::Unavailable(format!(
                "Invalid TLS trust bundle {}",
                candidate.display()
            )));
        }
        if !bundles.contains(&resolved) {
            bundles.push(resolved);
        }
    }
    Ok(bundles)
}

pub(super) fn system_trust_bundles() -> Result<Vec<PathBuf>, DomainError> {
    trust_bundles(&[
        Path::new("/etc/ssl/cert.pem"),
        Path::new("/etc/ssl/certs/ca-certificates.crt"),
        Path::new("/etc/pki/tls/certs/ca-bundle.crt"),
        Path::new("/etc/ssl/ca-bundle.pem"),
    ])
}
