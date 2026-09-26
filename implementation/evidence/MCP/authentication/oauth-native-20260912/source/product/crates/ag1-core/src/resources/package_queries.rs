//! Read-only package update protocol. The host grants filesystem, Git and registry
//! access separately; constructing a query never authorizes a remote helper.
use super::{
    package_lifecycle::Scope,
    packages::{self, Source},
};
use crate::{DomainError, PortFuture};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    future::{Future, poll_fn},
    task::Poll,
};

pub const CONCURRENCY: usize = 4;
pub const TIMEOUT_MS: u64 = 10_000;
pub struct Query {
    source: String,
    scope: Scope,
    installed: String,
    parsed: Source,
}
impl Query {
    pub fn new(source: String, scope: Scope, installed: String) -> Result<Self, DomainError> {
        let parsed = packages::parse(&source)?;
        if scope == Scope::Temporary
            || !installed.starts_with('/')
            || installed.len() > 8192
            || installed.contains('\0')
            || !std::path::Path::new(&installed).components().all(|c| {
                matches!(
                    c,
                    std::path::Component::RootDir | std::path::Component::Normal(_)
                )
            })
            || !matches!(
                parsed,
                Source::Npm { pinned: false, .. } | Source::Git { pinned: false, .. }
            )
        {
            return Err(DomainError::InvalidInput("package update query".into()));
        }
        Ok(Self {
            source,
            scope,
            installed,
            parsed,
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    pub source: String,
    pub display_name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub scope: Scope,
}
pub trait Port: Sync {
    fn checkpoint(&self) -> Result<(), DomainError>;
    fn exists<'a>(&'a self, installed: &'a str) -> PortFuture<'a, bool>;
    fn manifest<'a>(&'a self, installed: &'a str) -> PortFuture<'a, Option<Value>>;
    fn latest_npm<'a>(&'a self, name: &'a str, timeout_ms: u64) -> PortFuture<'a, Option<Value>>;
    fn git<'a>(
        &'a self,
        installed: &'a str,
        args: Vec<String>,
        remote: bool,
        timeout_ms: u64,
    ) -> PortFuture<'a, String>;
}
fn fatal(error: &DomainError) -> bool {
    // SDK suppresses update-check failures. Cancellation, expiry, permissions and
    // resource bounds must still reach the native owner, rather than look current.
    matches!(
        error,
        DomainError::DurabilityUnknown
            | DomainError::Conflict
            | DomainError::QueueFull
            | DomainError::Cancelled
            | DomainError::TimedOut
            | DomainError::PermissionDenied
            | DomainError::LimitExceeded(_)
    )
}
fn bounded(value: &Value) -> Result<(), DomainError> {
    packages::validate_settings_document(value)
        .map_err(|_| DomainError::LimitExceeded("package query JSON".into()))
}
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::Number(v) => v.as_f64() != Some(0.0),
        Value::String(v) => !v.is_empty(),
        _ => true,
    }
}
fn strictly_equal(left: &Value, right: &Value) -> bool {
    !left.is_array() && !left.is_object() && left == right
}
async fn git(
    port: &impl Port,
    query: &Query,
    args: &[&str],
    remote: bool,
) -> Result<String, DomainError> {
    port.checkpoint()?;
    let value = port
        .git(
            &query.installed,
            args.iter().map(|v| (*v).into()).collect(),
            remote,
            TIMEOUT_MS,
        )
        .await?;
    if value.len() > 65536 {
        return Err(DomainError::LimitExceeded(
            "package query Git output".into(),
        ));
    }
    Ok(value)
}
async fn changed(port: &impl Port, query: &Query) -> Result<bool, DomainError> {
    match &query.parsed {
        Source::Npm { name, .. } => {
            let Some(manifest) = port.manifest(&query.installed).await? else {
                return Ok(false);
            };
            bounded(&manifest)?;
            let Some(installed) = manifest.get("version").filter(|v| truthy(v)) else {
                return Ok(false);
            };
            port.checkpoint()?;
            let latest = port.latest_npm(name, TIMEOUT_MS).await?;
            if let Some(latest) = &latest {
                bounded(latest)?;
            }
            Ok(latest
                .as_ref()
                .is_none_or(|latest| !strictly_equal(installed, latest)))
        }
        Source::Git { .. } => {
            let local = git(port, query, &["rev-parse", "HEAD"], false).await?;
            let upstream = match git(
                port,
                query,
                &["rev-parse", "--abbrev-ref", "@{upstream}"],
                false,
            )
            .await
            {
                Ok(value) => value
                    .trim()
                    .strip_prefix("origin/")
                    .filter(|v| !v.is_empty())
                    .map(|v| format!("refs/heads/{v}")),
                Err(error) if fatal(&error) => return Err(error),
                Err(_) => None,
            };
            if let Some(upstream) = upstream {
                let remote = git(port, query, &["ls-remote", "origin", &upstream], true).await?;
                if let Some(captures) =
                    crate::tools::research::rx(r"(?m)^([0-9a-f]{40})\s+").captures(&remote)
                {
                    return Ok(local.trim() != &captures[1]);
                }
            }
            let remote = git(port, query, &["ls-remote", "origin", "HEAD"], true).await?;
            let captures = crate::tools::research::rx(r"(?m)^([0-9a-f]{40})\s+HEAD$")
                .captures(&remote)
                .ok_or_else(|| {
                    DomainError::Unavailable("Failed to determine remote HEAD".into())
                })?;
            Ok(local.trim() != &captures[1])
        }
        _ => Err(DomainError::InvalidInput("package query source".into())),
    }
}
async fn check(port: &impl Port, query: &Query) -> Result<Option<Update>, DomainError> {
    port.checkpoint()?;
    if !port.exists(&query.installed).await? {
        return Ok(None);
    }
    let changed = match changed(port, query).await {
        Ok(value) => value,
        Err(error) if fatal(&error) => return Err(error),
        Err(_) => false,
    };
    port.checkpoint()?;
    if !changed {
        return Ok(None);
    }
    let (kind, display_name) = match &query.parsed {
        Source::Npm { name, .. } => ("npm", name.clone()),
        Source::Git { host, path, .. } => ("git", format!("{host}/{path}")),
        _ => unreachable!(),
    };
    Ok(Some(Update {
        source: query.source.clone(),
        scope: query.scope,
        display_name,
        kind: kind.into(),
    }))
}
/// Four live checks at most, with results retained in configuration order.
/// Dropping this future drops every active port request; ports must bind network
/// requests and owned subprocesses to that cancellation lifetime.
pub async fn check_all(port: &impl Port, queries: &[Query]) -> Result<Vec<Update>, DomainError> {
    if queries.len() > 4096
        || queries
            .iter()
            .map(|q| q.source.len() + q.installed.len())
            .sum::<usize>()
            > 8 * 1024 * 1024
    {
        return Err(DomainError::LimitExceeded("package update queries".into()));
    }
    let mut next = 0;
    let mut active = Vec::new();
    let mut results = vec![None; queries.len()];
    while next < queries.len() || !active.is_empty() {
        port.checkpoint()?;
        while next < queries.len() && active.len() < CONCURRENCY {
            active.push((next, Box::pin(check(port, &queries[next]))));
            next += 1;
        }
        let (slot, value) = poll_fn(|cx| {
            for (slot, (_, future)) in active.iter_mut().enumerate() {
                if let Poll::Ready(value) = future.as_mut().poll(cx) {
                    return Poll::Ready((slot, value));
                }
            }
            Poll::Pending
        })
        .await;
        let (index, _) = active.swap_remove(slot);
        results[index] = value?;
    }
    Ok(results.into_iter().flatten().collect())
}
