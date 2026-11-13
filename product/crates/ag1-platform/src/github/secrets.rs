//! Typed secret targets and native-only value entry. No value is serializable.
use super::{
    operations::{Operation, Request, Risk},
    service::{Account, ApiError, Reply, Result},
};
use crate::resource_fs::ScanControl;
use base64::Engine;
use serde_json::{Value, json};
pub const MAX_BYTES: usize = 48 * 1024;
pub const TARGETS: &[&str] = &[
    "actions_repository",
    "actions_environment",
    "actions_organization",
    "dependabot_repository",
    "dependabot_organization",
    "codespaces_repository",
    "codespaces_organization",
    "codespaces_user",
    "agents_repository",
    "agents_organization",
];
#[link(name = "sodium")]
unsafe extern "C" {
    fn sodium_init() -> i32;
    fn sodium_memzero(p: *mut libc::c_void, len: usize);
    fn crypto_box_publickeybytes() -> usize;
    fn crypto_box_sealbytes() -> usize;
    fn crypto_box_seal(c: *mut u8, m: *const u8, len: u64, pk: *const u8) -> i32;
}
pub struct Secret(Vec<u8>);
impl Drop for Secret {
    fn drop(&mut self) {
        unsafe {
            sodium_memzero(self.0.as_mut_ptr().cast(), self.0.len());
        }
    }
}
impl Secret {
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        let secret = Self(bytes);
        if secret.0.is_empty() || secret.0.len() > MAX_BYTES {
            return Err(ApiError::new(
                "secret_size",
                "Enter a nonempty secret of at most 48 KiB.",
            ));
        }
        Ok(secret)
    }
    pub fn seal(&self, public_key: &str) -> Result<String> {
        let key = base64::engine::general_purpose::STANDARD
            .decode(public_key)
            .map_err(|_| invalid_key())?;
        unsafe {
            if sodium_init() < 0
                || crypto_box_publickeybytes() != 32
                || crypto_box_sealbytes() != 48
                || key.len() != 32
            {
                return Err(invalid_key());
            }
            let mut encrypted = vec![0; self.0.len() + 48];
            if crypto_box_seal(
                encrypted.as_mut_ptr(),
                self.0.as_ptr(),
                self.0.len() as u64,
                key.as_ptr(),
            ) != 0
            {
                return Err(invalid_key());
            }
            Ok(base64::engine::general_purpose::STANDARD.encode(encrypted))
        }
    }
}
fn invalid_key() -> ApiError {
    ApiError::new(
        "secret_key",
        "GitHub returned an unusable secret encryption key.",
    )
}
/// Implemented by the native desktop composition, never by renderer arguments.
/// Cancellation must close entry and discard any late reply.
pub trait PrivateEntry: Send + Sync {
    fn enter(
        &self,
        account: &Account,
        operation: &Operation,
        control: &ScanControl<'_>,
    ) -> Result<Secret>;
}
fn bad() -> String {
    "Invalid secret target or metadata. Never pass a secret value in operation arguments.".into()
}
fn component(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && !s.starts_with('-')
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
fn name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && !s.starts_with("GITHUB_")
        && !s.as_bytes()[0].is_ascii_digit()
        && s.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}
fn ids(v: &Value) -> bool {
    v.as_array().is_some_and(|a| {
        a.len() <= 100
            && a.iter().all(|v| {
                v.as_u64()
                    .is_some_and(|n| n > 0 && n <= 9_007_199_254_740_991)
            })
            && a.iter()
                .map(Value::to_string)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == a.len()
    })
}
pub(super) fn compile(r: &Request) -> std::result::Result<Option<Operation>, String> {
    if !r.operation.starts_with("secrets.") {
        return Ok(None);
    }
    if ![
        "secrets.set",
        "secrets.list",
        "secrets.get",
        "secrets.delete",
        "secrets.repositories",
        "secrets.set_repositories",
    ]
    .contains(&r.operation.as_str())
        || !r.body.is_null()
    {
        return Err(bad());
    }
    let p = &r.parameters;
    let target = p
        .get("target")
        .and_then(Value::as_str)
        .filter(|t| TARGETS.contains(t))
        .ok_or_else(bad)?;
    let (service, kind) = target.split_once('_').ok_or_else(bad)?;
    let mut allowed = vec!["target"];
    let repo = if ["repository", "environment"].contains(&kind) {
        Some(
            r.repo
                .as_ref()
                .filter(|s| s.split('/').count() == 2 && s.split('/').all(component))
                .ok_or_else(bad)?
                .clone(),
        )
    } else {
        if r.repo.is_some() {
            return Err(bad());
        }
        None
    };
    let base = match kind {
        "repository" => format!("repos/{}/{service}/secrets", repo.as_ref().unwrap()),
        "environment" => {
            allowed.push("environment");
            let env = p
                .get("environment")
                .and_then(Value::as_str)
                .filter(|s| {
                    !s.is_empty()
                        && s.len() <= 255
                        && *s != "."
                        && *s != ".."
                        && !s.chars().any(char::is_control)
                })
                .ok_or_else(bad)?;
            let mut url = reqwest::Url::parse("https://api.github.com/").unwrap();
            url.path_segments_mut().unwrap().push(env);
            format!(
                "repos/{}/environments/{}/secrets",
                repo.as_ref().unwrap(),
                url.path().trim_start_matches('/')
            )
        }
        "organization" => {
            allowed.push("organization");
            let org = p
                .get("organization")
                .and_then(Value::as_str)
                .filter(|s| component(s))
                .ok_or_else(bad)?;
            format!("orgs/{org}/{service}/secrets")
        }
        "user" => "user/codespaces/secrets".into(),
        _ => return Err(bad()),
    };
    let mut endpoint = base;
    let mut body = None;
    if r.operation != "secrets.list" {
        allowed.push("name");
        let n = p
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| name(s))
            .ok_or_else(bad)?;
        endpoint.push('/');
        endpoint.push_str(n);
    }
    let (method, risk) = match r.operation.as_str() {
        "secrets.list" | "secrets.repositories" => {
            if r.operation == "secrets.repositories" {
                if !["organization", "user"].contains(&kind) {
                    return Err(bad());
                }
                endpoint.push_str("/repositories");
            }
            allowed.extend(["page", "per_page"]);
            let page = p
                .get("page")
                .map_or(Some(1), Value::as_u64)
                .filter(|v| (1..=1000).contains(v))
                .ok_or_else(bad)?;
            let size = p
                .get("per_page")
                .map_or(Some(30), Value::as_u64)
                .filter(|v| (1..=100).contains(v))
                .ok_or_else(bad)?;
            endpoint.push_str(&format!("?page={page}&per_page={size}"));
            ("GET", Risk::Read)
        }
        "secrets.get" => ("GET", Risk::Read),
        "secrets.delete" => ("DELETE", Risk::Consequential),
        "secrets.set" | "secrets.set_repositories" => {
            let mut fields = serde_json::Map::new();
            let selected = if r.operation == "secrets.set_repositories" {
                if !["organization", "user"].contains(&kind) {
                    return Err(bad());
                }
                endpoint.push_str("/repositories");
                true
            } else if kind == "organization" {
                allowed.push("visibility");
                let visibility = p
                    .get("visibility")
                    .and_then(Value::as_str)
                    .filter(|v| ["all", "private", "selected"].contains(v))
                    .ok_or_else(bad)?;
                fields.insert("visibility".into(), json!(visibility));
                visibility == "selected"
            } else {
                kind == "user"
            };
            if selected {
                allowed.push("selected_repository_ids");
                let selected = p
                    .get("selected_repository_ids")
                    .filter(|v| ids(v))
                    .ok_or_else(bad)?;
                fields.insert("selected_repository_ids".into(), selected.clone());
            }
            body = Some(Value::Object(fields));
            ("PUT", Risk::Consequential)
        }
        _ => return Err(bad()),
    };
    if p.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(bad());
    }
    Ok(Some(Operation {
        id: r.operation.clone(),
        method: method.into(),
        endpoint,
        body,
        repo,
        risk,
    }))
}
pub(super) fn validate_schema(op: &Operation) -> std::result::Result<(), String> {
    let mut native = op.clone();
    native.id = "native.secret.schema".into();
    if op.id == "secrets.set" {
        native.body.as_mut().ok_or_else(bad)?["encrypted_value"] = json!("YQ==");
        native.body.as_mut().unwrap()["key_id"] = json!("native-key");
    }
    super::schemas::validate(&native)
}
pub fn public_key_operation(op: &Operation) -> Result<Operation> {
    if op.id != "secrets.set" || op.method != "PUT" {
        return Err(ApiError::new(
            "arguments",
            "Invalid private secret operation.",
        ));
    }
    let base = op.endpoint.rsplit_once('/').ok_or_else(invalid_key)?.0;
    Ok(Operation {
        id: "native.secret.public_key".into(),
        method: "GET".into(),
        endpoint: format!("{base}/public-key"),
        body: None,
        repo: op.repo.clone(),
        risk: Risk::Read,
    })
}
pub fn validate_public_key(key: &Value) -> Result<()> {
    key["key_id"]
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 256
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        })
        .ok_or_else(invalid_key)?;
    let public = key["key"]
        .as_str()
        .filter(|s| s.len() <= 100)
        .ok_or_else(invalid_key)?;
    if base64::engine::general_purpose::STANDARD
        .decode(public)
        .map_err(|_| invalid_key())?
        .len()
        != 32
    {
        return Err(invalid_key());
    }
    Ok(())
}
pub fn encrypted_body(op: &Operation, secret: &Secret, key: &Value) -> Result<Value> {
    let id = key["key_id"]
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 256
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        })
        .ok_or_else(invalid_key)?;
    let public = key["key"]
        .as_str()
        .filter(|s| s.len() <= 100)
        .ok_or_else(invalid_key)?;
    let mut body = op
        .body
        .clone()
        .filter(Value::is_object)
        .ok_or_else(invalid_key)?;
    body["key_id"] = json!(id);
    body["encrypted_value"] = json!(secret.seal(public)?);
    Ok(body)
}
/// Empty write receipts never expose a server's echoed private body.
pub fn set_receipt(op: &Operation, status: u16) -> Result<Reply> {
    if ![201, 204].contains(&status) {
        return Err(ApiError::new(
            "secret_response",
            "GitHub did not confirm saving the secret. Inspect its metadata before trying again.",
        )
        .uncertain(op));
    }
    Ok(Reply {
        data: json!({"secret_saved":true,"target":op.endpoint,"value_returned":false}),
        status,
        next: None,
        rate_remaining: None,
        rate_reset: None,
        rate_resource: None,
        rate_limit: None,
        outcome: super::service::Outcome::Applied,
    })
}
pub(super) fn validate(op: &Operation, reply: &Reply) -> Result<()> {
    if !op.id.starts_with("secrets.") {
        return Ok(());
    }
    let invalid = || {
        ApiError::new("secret_metadata","GitHub returned invalid secret metadata. Refresh the destination before another change.").uncertain(op)
    };
    let item = |v: &Value| v["name"].as_str().is_some_and(name);
    let okay = match op.id.as_str() {
        "secrets.set" => {
            [201, 204].contains(&reply.status)
                && reply.data["secret_saved"] == true
                && reply.data["target"] == op.endpoint
        }
        "secrets.delete" | "secrets.set_repositories" => {
            reply.status == 204 && reply.data.is_null()
        }
        "secrets.get" => {
            reply.status == 200
                && item(&reply.data)
                && op.endpoint.rsplit('/').next() == reply.data["name"].as_str()
        }
        "secrets.list" => {
            reply.status == 200
                && reply.data["total_count"].as_u64().is_some()
                && reply.data["secrets"]
                    .as_array()
                    .is_some_and(|a| a.len() <= 100 && a.iter().all(item))
        }
        "secrets.repositories" => {
            reply.status == 200
                && reply.data["total_count"].as_u64().is_some()
                && reply.data["repositories"].as_array().is_some_and(|a| {
                    a.len() <= 100
                        && a.iter().all(|r| {
                            r["id"].as_u64().is_some_and(|id| id > 0)
                                && r["full_name"].as_str().is_some_and(|s| {
                                    s.split('/').count() == 2 && s.split('/').all(component)
                                })
                        })
                })
        }
        _ => false,
    };
    if okay { Ok(()) } else { Err(invalid()) }
}
pub fn schema(id: &str) -> Option<Value> {
    if ![
        "secrets.set",
        "secrets.list",
        "secrets.get",
        "secrets.delete",
        "secrets.repositories",
        "secrets.set_repositories",
    ]
    .contains(&id)
    {
        return None;
    }
    Some(
        json!({"id":id,"note":"Secret values can only be entered in the native desktop dialog after exact approval. Body is forbidden. Repository/environment targets require repo; organization/user targets forbid repo. Names use uppercase ASCII letters, digits and underscores. Organization set requires explicit visibility; selected visibility and user set require exact repository IDs (empty means none).","parameters":{"target":TARGETS,"organization":"required for organization target","environment":"required for environment target","name":"required except list","visibility":["all","private","selected"],"selected_repository_ids":"0-100 unique positive numeric IDs when selected/user or set_repositories","page":"1-1000 for list/repositories","per_page":"1-100 for list/repositories"}}),
    )
}
#[cfg(test)]
mod tests {
    use super::super::service::{Outcome, Service, Transport};
    use super::*;
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };
    fn request(target: &str, operation: &str) -> Request {
        let kind = target.split_once('_').unwrap().1;
        let mut p = json!({"target":target});
        if kind == "organization" {
            p["organization"] = json!("owned");
        }
        if kind == "environment" {
            p["environment"] = json!("production / blue");
        }
        if operation != "secrets.list" {
            p["name"] = json!("OWNED_SECRET");
        }
        if operation == "secrets.set" && kind == "organization" {
            p["visibility"] = json!("selected");
        }
        if operation == "secrets.set" && ["organization", "user"].contains(&kind)
            || operation == "secrets.set_repositories"
        {
            p["selected_repository_ids"] = json!([42]);
        }
        serde_json::from_value(json!({"operation":operation,"repo":if ["environment","repository"].contains(&kind){Some("owned/repo")}else{None},"parameters":p})).unwrap()
    }
    #[test]
    fn secret_targets_and_access_match_pinned_schemas_and_reject_value_arguments() {
        for target in TARGETS {
            for action in [
                "secrets.set",
                "secrets.list",
                "secrets.get",
                "secrets.delete",
                "secrets.repositories",
                "secrets.set_repositories",
            ] {
                let r = request(target, action);
                if ["secrets.repositories", "secrets.set_repositories"].contains(&action)
                    && !target.ends_with("organization")
                    && !target.ends_with("user")
                {
                    assert!(r.compile().is_err());
                    continue;
                }
                let op = r.compile().unwrap();
                validate_schema(&op).unwrap_or_else(|e| panic!("{target} {action}: {e}"));
                if action == "secrets.set" {
                    super::super::schemas::validate(&public_key_operation(&op).unwrap()).unwrap();
                }
                assert!(schema(action).is_some());
            }
        }
        for field in [
            "value",
            "secret",
            "encrypted_value",
            "key_id",
            "token",
            "endpoint",
            "extra",
        ] {
            let mut r = request("actions_repository", "secrets.set");
            r.parameters.insert(field.into(), json!("private"));
            assert!(r.compile().is_err());
        }
        for n in [
            "",
            "lowercase",
            "GITHUB_TOKEN",
            "1SECRET",
            "../ESCAPE",
            "SECRET/NAME",
        ] {
            let mut r = request("actions_repository", "secrets.set");
            r.parameters.insert("name".into(), json!(n));
            assert!(r.compile().is_err());
        }
        let mut r = request("actions_organization", "secrets.set");
        r.parameters
            .insert("selected_repository_ids".into(), json!([1, 1]));
        assert!(r.compile().is_err());
        r.parameters.insert("visibility".into(), json!("all"));
        assert!(r.compile().is_err());
        let mut r = request("actions_repository", "secrets.set");
        r.body = json!({"value":"private"});
        assert!(r.compile().is_err());
        assert!(Secret::new(vec![]).is_err());
        assert!(Secret::new(vec![1; MAX_BYTES + 1]).is_err());
        let s = Secret::new(vec![1; MAX_BYTES]).unwrap();
        assert!(s.seal("bad").is_err());
    }
    struct Fixture {
        account: Mutex<Account>,
        writes: AtomicUsize,
        uncertain: bool,
    }
    impl Transport for Fixture {
        fn account(&self, c: &ScanControl<'_>) -> Result<Account> {
            super::super::service::check(c)?;
            Ok(self.account.lock().unwrap().clone())
        }
        fn execute(&self, _: &Account, _: &Operation, _: &ScanControl<'_>) -> Result<Reply> {
            panic!("No ordinary secret write channel")
        }
        fn set_secret(
            &self,
            a: &Account,
            op: &Operation,
            e: &dyn PrivateEntry,
            c: &ScanControl<'_>,
        ) -> Result<Reply> {
            if self.account(c)? != *a {
                return Err(ApiError::new("stale_account", "Account changed."));
            }
            let _secret = e.enter(a, op, c)?;
            super::super::service::check(c)?;
            if self.account(c)? != *a {
                return Err(ApiError::new(
                    "stale_account",
                    "Account changed during entry.",
                ));
            }
            self.writes.fetch_add(1, Ordering::AcqRel);
            if self.uncertain {
                return Err(ApiError::new("network", "Interrupted.").uncertain(op));
            }
            set_receipt(op, 204)
        }
    }
    struct Entry {
        calls: AtomicUsize,
        fixture: Arc<Fixture>,
        scenario: &'static str,
    }
    impl PrivateEntry for Entry {
        fn enter(&self, _: &Account, op: &Operation, _: &ScanControl<'_>) -> Result<Secret> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            assert!(!serde_json::to_string(op).unwrap().contains("private-value"));
            match self.scenario {
                "entry_cancel" => return Err(ApiError::new("cancelled", "Cancelled.")),
                "late_account" => {
                    self.fixture.account.lock().unwrap().credential_revision = "new".into()
                }
                _ => {}
            }
            Secret::new(b"private-value".to_vec())
        }
    }
    #[test]
    fn secret_entry_waits_for_one_use_approval_and_rechecks_native_lifecycle() {
        for scenario in [
            "success",
            "cancel",
            "entry_cancel",
            "scope",
            "account",
            "late_account",
            "disconnect",
            "uncertain",
            "missing_entry",
        ] {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("github-connected"), b"generation").unwrap();
            let fixture = Arc::new(Fixture {
                account: Mutex::new(Account::bound(1, "owned".into(), "cred".into()).unwrap()),
                writes: AtomicUsize::new(0),
                uncertain: scenario == "uncertain",
            });
            let service = Service::new(dir.path().into(), fixture.clone());
            let entry = Arc::new(Entry {
                calls: AtomicUsize::new(0),
                fixture: fixture.clone(),
                scenario,
            });
            if scenario != "missing_entry" {
                service.set_private_entry(entry.clone()).unwrap();
            }
            let cancel = AtomicBool::new(false);
            let c = ScanControl {
                cancel: &cancel,
                deadline: Instant::now() + Duration::from_secs(5),
            };
            let r = request("actions_repository", "secrets.set");
            assert!(service.read(vec![r.clone()], "s", &c).is_err());
            let p = service.preview(r, "s", &c).unwrap();
            assert_eq!(entry.calls.load(Ordering::Acquire), 0);
            assert!(!p.to_string().contains("private-value"));
            match scenario {
                "cancel" => cancel.store(true, Ordering::Release),
                "account" => fixture.account.lock().unwrap().credential_revision = "new".into(),
                "disconnect" => {
                    std::fs::remove_file(dir.path().join("github-connected")).unwrap();
                }
                _ => {}
            }
            let id = p["confirmation"].as_str().unwrap();
            let result = service.confirm(id, if scenario == "scope" { "other" } else { "s" }, &c);
            if scenario == "success" {
                assert_eq!(result.unwrap().data["secret_saved"], true);
            } else {
                let err = result.unwrap_err();
                assert_eq!(
                    err.outcome,
                    if scenario == "uncertain" {
                        Outcome::Unknown
                    } else {
                        Outcome::NotSent
                    }
                );
            }
            assert_eq!(
                fixture.writes.load(Ordering::Acquire),
                usize::from(["success", "uncertain"].contains(&scenario))
            );
            assert!(service.confirm(id, "s", &c).is_err());
        }
    }
    #[test]
    fn malformed_secret_metadata_and_key_responses_are_rejected() {
        let mut reply = Reply {
            data: json!({"name":"OTHER_SECRET"}),
            status: 200,
            next: None,
            rate_remaining: None,
            rate_reset: None,
            rate_resource: None,
            rate_limit: None,
            outcome: Outcome::Read,
        };
        let get = request("actions_repository", "secrets.get")
            .compile()
            .unwrap();
        assert!(validate(&get, &reply).is_err());
        reply.data = json!({"name":"OWNED_SECRET"});
        assert!(validate(&get, &reply).is_ok());
        let list = request("actions_repository", "secrets.list")
            .compile()
            .unwrap();
        for data in [
            json!({}),
            json!({"total_count":1,"secrets":[{}]}),
            json!({"total_count":1,"secrets":[{"name":"../TOKEN"}]}),
        ] {
            reply.data = data;
            assert!(validate(&list, &reply).is_err());
        }
        for key in [
            json!({}),
            json!({"key_id":"owned","key":"bad"}),
            json!({"key_id":"bad/key","key":"YQ=="}),
        ] {
            assert!(validate_public_key(&key).is_err());
        }
        let delete = request("actions_repository", "secrets.delete")
            .compile()
            .unwrap();
        reply.status = 200;
        reply.data = Value::Null;
        assert_eq!(
            validate(&delete, &reply).unwrap_err().outcome,
            Outcome::Unknown
        );
        reply.status = 204;
        assert!(validate(&delete, &reply).is_ok());
    }
}
