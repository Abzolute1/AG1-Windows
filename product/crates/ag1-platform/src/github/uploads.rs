//! Exact granted-file snapshots for native release uploads. Source bytes are
//! retained privately and never serialized into previews or tool arguments.
use super::{
    downloads::{MAX_BYTES, Payload},
    operations::{Operation, Request, Risk},
    service::{ApiError, Result, check},
};
use crate::{folder_access::FolderAccess, resource_fs::ScanControl, workspace::Workspace};
use ag1_core::FileRevision;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Default)]
pub(super) struct Budget(Arc<AtomicUsize>);
struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
impl Budget {
    fn acquire(&self) -> Result<Permit> {
        self.0.fetch_update(Ordering::AcqRel,Ordering::Acquire,|n|(n<2).then_some(n+1)).map_err(|_|ApiError::new("upload_quota","Two file uploads are already retained. Cancel an upload preview before preparing another."))?;
        Ok(Permit(self.0.clone()))
    }
}
pub struct Upload {
    root: Arc<Workspace>,
    grant: Arc<FolderAccess>,
    grant_revision: u64,
    path: String,
    revision: FileRevision,
    pub(crate) payload: Payload,
    pub(crate) content_type: String,
    _permit: Permit,
}
fn invalid() -> String {
    "Upload requires repo, positive release_id, relative file path, a simple asset name, and media type; no body or arbitrary URL.".into()
}
pub(super) fn compile(request: &Request) -> std::result::Result<Option<Operation>, String> {
    if request.operation != "releases.upload_asset" {
        return Ok(None);
    }
    let p = &request.parameters;
    let repo = request.repo.as_deref().ok_or_else(invalid)?;
    let components = repo.split('/').collect::<Vec<_>>();
    if components.len() != 2
        || components.iter().any(|s| {
            s.is_empty()
                || s.len() > 128
                || *s == "."
                || *s == ".."
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        })
        || !request.body.is_null()
        || p.keys().any(|k| {
            !["release_id", "folder", "path", "name", "content_type"].contains(&k.as_str())
        })
    {
        return Err(invalid());
    }
    let id = p
        .get("release_id")
        .and_then(Value::as_u64)
        .filter(|n| *n > 0 && *n <= 9_007_199_254_740_991)
        .ok_or_else(invalid)?;
    let name = p
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 255
                && !s.starts_with('.')
                && !s.ends_with('.')
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
        .ok_or_else(invalid)?;
    let path = p
        .get("path")
        .and_then(Value::as_str)
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 4096
                && !s.contains('\0')
                && !s.starts_with('/')
                && s.split('/')
                    .all(|part| !part.is_empty() && part != "." && part != "..")
        })
        .ok_or_else(invalid)?;
    if path.is_empty()
        || p.get("folder").is_some_and(|v| {
            !v.as_str()
                .is_some_and(|s| !s.is_empty() && s.len() <= 4096 && !s.contains('\0'))
        })
    {
        return Err(invalid());
    }
    let mime = p
        .get("content_type")
        .and_then(Value::as_str)
        .unwrap_or("application/octet-stream");
    let parts = mime.split('/').collect::<Vec<_>>();
    if p.get("content_type").is_some_and(|v| !v.is_string())
        || mime.len() > 100
        || parts.len() != 2
        || parts.iter().any(|s| {
            s.is_empty()
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&b))
        })
    {
        return Err(invalid());
    }
    Ok(Some(Operation {
        id: request.operation.clone(),
        method: "POST".into(),
        endpoint: format!("repos/{repo}/releases/{id}/assets?name={name}"),
        body: None,
        risk: Risk::Consequential,
        repo: Some(repo.into()),
    }))
}
pub(super) fn schema(id: &str) -> Option<Value> {
    (id=="releases.upload_asset").then(||json!({"id":id,"repo":"owner/name","parameters":{"release_id":"Positive release ID","folder":"An already granted folder; default current folder","path":"Exact relative regular-file path inside that folder","name":"Asset filename, 1–255 ASCII letters/digits/dots/underscores/hyphens; no leading/trailing dot","content_type":"Media type; default application/octet-stream"},"body":"Omit. Bytes come only from the native granted-file snapshot.","limits":"64 MiB per upload, two retained upload plans per service. Exact native confirmation required; no replacement/deletion of existing assets, redirects or retries."}))
}
impl Upload {
    pub(super) fn capture(
        request: &Request,
        root: Arc<Workspace>,
        grant: Arc<FolderAccess>,
        grant_revision: u64,
        budget: &Budget,
        control: &ScanControl<'_>,
    ) -> Result<Self> {
        check(control)?;
        let permit = budget.acquire()?;
        grant.check(grant_revision).map_err(|_| source_error())?;
        let admitted = grant
            .snapshot()
            .and_then(|s| {
                s.resolve(
                    root.root()
                        .to_str()
                        .ok_or(ag1_core::DomainError::PermissionDenied)?,
                    false,
                )
            })
            .map_err(|_| source_error())?;
        if admitted.root() != root.root() {
            return Err(source_error());
        }
        let path = request.parameters["path"]
            .as_str()
            .ok_or_else(source_error)?
            .to_owned();
        let mut payload = Payload { bytes: Vec::new() };
        let revision = root
            .stream(&path, MAX_BYTES as u64, |chunk| {
                control.check()?;
                grant.check(grant_revision)?;
                payload.bytes.extend_from_slice(chunk);
                Ok(())
            })
            .map_err(|_| check(control).err().unwrap_or_else(source_error))?;
        grant.check(grant_revision).map_err(|_| source_error())?;
        check(control)?;
        Ok(Self {
            root,
            grant,
            grant_revision,
            path,
            revision,
            payload,
            content_type: request
                .parameters
                .get("content_type")
                .and_then(Value::as_str)
                .unwrap_or("application/octet-stream")
                .into(),
            _permit: permit,
        })
    }
    pub fn bytes(&self) -> &[u8] {
        &self.payload.bytes
    }
    pub fn verify(&self, control: &ScanControl<'_>) -> Result<()> {
        check(control)?;
        self.grant
            .check(self.grant_revision)
            .map_err(|_| source_error())?;
        let revision = self
            .root
            .stream(&self.path, MAX_BYTES as u64, |_| {
                control.check()?;
                self.grant.check(self.grant_revision)?;
                Ok(())
            })
            .map_err(|_| check(control).err().unwrap_or_else(source_error))?;
        if revision != self.revision {
            return Err(ApiError::new(
                "stale_upload",
                "The selected file changed after review. Prepare a new upload.",
            ));
        }
        check(control)?;
        self.grant
            .check(self.grant_revision)
            .map_err(|_| source_error())?;
        Ok(())
    }
    pub fn preview(&self) -> Value {
        json!({"folder":self.root.root(),"path":self.path,"bytes":self.payload.bytes.len(),"sha256":self.revision.sha256,"content_type":self.content_type})
    }
    pub(super) fn validate_reply(&self, operation: &Operation, data: &Value) -> Result<()> {
        let expected_name = operation
            .endpoint
            .split_once("?name=")
            .map(|(_, s)| s)
            .unwrap_or("");
        let expected_digest = format!("sha256:{}", self.revision.sha256);
        if !data["id"].as_u64().is_some_and(|id| id > 0)
            || data["name"] != expected_name
            || data["size"] != self.payload.bytes.len()
            || data["state"] != "uploaded"
            || data
                .get("digest")
                .is_some_and(|v| !v.is_null() && v != &json!(expected_digest))
        {
            return Err(ApiError::new("upload_uncertain","GitHub returned an incomplete or mismatched upload receipt. Inspect release assets before trying again.").uncertain(operation));
        }
        Ok(())
    }
}
fn source_error() -> ApiError {
    ApiError::new(
        "upload_source",
        "Upload source is unavailable, changed, exceeds 64 MiB, or is outside the current folder grant. Review the file and folder access again.",
    )
}

#[cfg(test)]
pub(crate) fn fixture_source() -> (
    tempfile::TempDir,
    Arc<Workspace>,
    Arc<FolderAccess>,
    Request,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source");
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("owned.bin"), [0, 255, 13, 10, 65]).unwrap();
    let root = crate::workspace::WorkspaceService::new()
        .unwrap()
        .register(&path)
        .unwrap();
    let grant =
        FolderAccess::open(root.clone(), &dir.path().join("state"), "owned-upload").unwrap();
    let request=serde_json::from_value(json!({"operation":"releases.upload_asset","repo":"owned/repo","parameters":{"release_id":12,"path":"owned.bin","name":"owned.bin"}})).unwrap();
    (dir, root, grant, request)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::service::{Account, Outcome, Reply, Service, Transport};
    use std::{
        sync::{Mutex, atomic::AtomicBool},
        time::{Duration, Instant},
    };
    struct Fixture {
        account: Mutex<Account>,
        writes: AtomicUsize,
        fail: bool,
        changed: Option<std::path::PathBuf>,
    }
    impl Transport for Fixture {
        fn account(&self, c: &ScanControl<'_>) -> Result<Account> {
            check(c)?;
            Ok(self.account.lock().unwrap().clone())
        }
        fn execute(&self, _: &Account, _: &Operation, _: &ScanControl<'_>) -> Result<Reply> {
            panic!("Upload cannot use the JSON channel")
        }
        fn upload(
            &self,
            a: &Account,
            op: &Operation,
            u: &Upload,
            c: &ScanControl<'_>,
        ) -> Result<Reply> {
            if &self.account(c)? != a {
                return Err(ApiError::new("stale_account", "Changed account"));
            }
            if let Some(path) = &self.changed {
                std::fs::write(path, b"later writer").unwrap();
            }
            u.verify(c)?;
            self.writes.fetch_add(1, Ordering::AcqRel);
            assert_eq!(u.bytes(), [0, 255, 13, 10, 65]);
            if self.fail {
                return Err(ApiError::new("network", "Interrupted").uncertain(op));
            }
            Ok(Reply {
                data: json!({"id":99,"name":"owned.bin","size":5,"state":"uploaded","digest":format!("sha256:{}",u.revision.sha256)}),
                status: 201,
                next: None,
                rate_remaining: None,
                rate_reset: None,
                rate_resource: None,
                rate_limit: None,
                outcome: Outcome::Applied,
            })
        }
    }
    #[test]
    fn upload_approval_binds_file_account_scope_and_grant_with_no_replay() {
        for scenario in [
            "success",
            "changed_file",
            "late_file",
            "scope",
            "account",
            "grant",
            "cancel",
            "uncertain",
        ] {
            let (dir, root, grant, request) = fixture_source();
            std::fs::write(dir.path().join("github-connected"), b"generation").unwrap();
            let fixture = Arc::new(Fixture {
                account: Mutex::new(Account::bound(1, "owned".into(), "cred".into()).unwrap()),
                writes: AtomicUsize::new(0),
                fail: scenario == "uncertain",
                changed: (scenario == "late_file").then(|| root.root().join("owned.bin")),
            });
            let service = Service::new(dir.path().into(), fixture.clone());
            let cancel = AtomicBool::new(false);
            let c = ScanControl {
                cancel: &cancel,
                deadline: Instant::now() + Duration::from_secs(10),
            };
            assert!(service.prepare(request.clone(), "s", &c).is_err());
            assert!(service.read(vec![request.clone()], "s", &c).is_err());
            let p = service
                .preview_upload(request, root.clone(), grant.clone(), 1, "s", &c)
                .unwrap();
            assert_eq!(fixture.writes.load(Ordering::Acquire), 0);
            assert_eq!(p["upload_source"]["bytes"], 5);
            assert!(!p.to_string().contains("base64"));
            match scenario {
                "changed_file" => std::fs::write(root.root().join("owned.bin"), b"later").unwrap(),
                "account" => fixture.account.lock().unwrap().credential_revision = "new".into(),
                "grant" => {
                    grant
                        .change(crate::folder_access::Change {
                            revision: 1,
                            action: "mode".into(),
                            path: root.root().into(),
                            mode: Some(crate::folder_access::AccessMode::ReadOnly),
                        })
                        .unwrap();
                }
                "cancel" => cancel.store(true, Ordering::Release),
                _ => {}
            }
            let id = p["confirmation"].as_str().unwrap();
            let result = service.confirm(id, if scenario == "scope" { "other" } else { "s" }, &c);
            if scenario == "success" {
                assert_eq!(result.unwrap().outcome, Outcome::Applied);
            } else if scenario == "uncertain" {
                assert_eq!(result.unwrap_err().outcome, Outcome::Unknown);
            } else {
                assert_eq!(result.unwrap_err().outcome, Outcome::NotSent);
            }
            assert_eq!(
                fixture.writes.load(Ordering::Acquire),
                usize::from(["success", "uncertain"].contains(&scenario))
            );
            assert!(service.confirm(id, "s", &c).is_err());
        }
    }
    #[test]
    fn source_links_quota_invalid_fields_and_uncertain_receipts_are_refused() {
        let (_dir, root, grant, request) = fixture_source();
        let cancel = AtomicBool::new(false);
        let c = ScanControl {
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_secs(5),
        };
        let budget = Budget::default();
        let first = Upload::capture(&request, root.clone(), grant.clone(), 1, &budget, &c).unwrap();
        let second =
            Upload::capture(&request, root.clone(), grant.clone(), 1, &budget, &c).unwrap();
        assert!(Upload::capture(&request, root.clone(), grant.clone(), 1, &budget, &c).is_err());
        drop(second);
        let op = request.compile().unwrap();
        for bad in [
            json!({}),
            json!({"id":99,"name":"other.bin","size":5,"state":"uploaded"}),
            json!({"id":99,"name":"owned.bin","size":4,"state":"uploaded"}),
            json!({"id":99,"name":"owned.bin","size":5,"state":"starter"}),
            json!({"id":99,"name":"owned.bin","size":5,"state":"uploaded","digest":"sha256:wrong"}),
        ] {
            assert_eq!(
                first.validate_reply(&op, &bad).unwrap_err().outcome,
                Outcome::Unknown
            );
        }
        for (key, value) in [
            ("name", json!("../x")),
            ("path", json!("../outside")),
            ("path", json!("/outside")),
            ("release_id", json!(0)),
            ("content_type", json!("text/plain\r\nAuthorization: evil")),
            ("url", json!("https://evil.test")),
        ] {
            let mut r = request.clone();
            r.parameters.insert(key.into(), value);
            assert!(r.compile().is_err());
        }
        std::fs::File::create(root.root().join("large.bin"))
            .unwrap()
            .set_len(MAX_BYTES as u64 + 1)
            .unwrap();
        let mut large = request.clone();
        large.parameters.insert("path".into(), json!("large.bin"));
        assert!(Upload::capture(&large, root.clone(), grant.clone(), 1, &budget, &c).is_err());
        #[cfg(target_os = "linux")]
        {
            std::fs::remove_file(root.root().join("owned.bin")).unwrap();
            std::os::unix::fs::symlink("outside", root.root().join("owned.bin")).unwrap();
            assert!(Upload::capture(&request, root, grant, 1, &budget, &c).is_err());
        }
    }
}
