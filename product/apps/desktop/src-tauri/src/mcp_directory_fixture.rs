use std::path::PathBuf;
/// Owned offline registry responses only in the diagnostic native candidate.
/// Production does not compile this filesystem/reader composition.
pub fn mcp_directory(
    service: std::sync::Arc<ag1_platform::mcp::service::Service>,
) -> Result<std::sync::Arc<ag1_platform::mcp::service::Service>, String> {
    use std::{
        os::unix::fs::{MetadataExt, OpenOptionsExt},
        sync::Arc,
    };
    let Some(socket) = std::env::var_os("AG1_VISUAL_SOCKET") else {
        return Ok(service);
    };
    let parent = PathBuf::from(socket)
        .parent()
        .ok_or("Visual socket parent")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let path = parent.join("owned-mcp-directory.json");
    if !path.exists() {
        return Ok(service);
    }
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../.local/p14-live-fixes")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if std::env::var("AG1_FIXTURE_PROVIDER").as_deref() != Ok("1")
        || !parent.starts_with(&base)
        || !parent
            .parent()
            .and_then(|p| p.file_name())
            .is_some_and(|p| p.to_string_lossy().starts_with("native-candidate-mcp-c3-"))
    {
        return Err("Owned directory requires an isolated C3 fixture run".into());
    }
    let directory = ag1_platform::mcp::registry::Directory::with_reader(move |url| {
        use ag1_core::DomainError;
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
            .map_err(|_| DomainError::NotFound)?;
        let meta = file.metadata().map_err(|_| DomainError::Conflict)?;
        if !meta.is_file()
            || meta.len() > 64000
            || meta.mode() & 0o077 != 0
            || meta.uid() != unsafe { libc::geteuid() }
        {
            return Err(DomainError::PermissionDenied);
        }
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut std::io::Read::take(&mut file, 64001), &mut bytes)
            .map_err(|_| DomainError::Conflict)?;
        if bytes.len() > 64000 {
            return Err(DomainError::LimitExceeded("Owned directory".into()));
        }
        let row: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|_| DomainError::InvalidInput("Owned directory JSON".into()))?;
        if row["unavailable"] == true {
            return Err(DomainError::Unavailable("Owned registry offline".into()));
        }
        let response = if url.path().contains("/versions/") {
            row
        } else {
            serde_json::json!({"servers":[row],"metadata":{}})
        };
        Ok((response.to_string().into_bytes(), 0))
    });
    Ok(service.with_fixture_directory(Arc::new(directory)))
}
