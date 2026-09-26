//! Approved web fetch with scoped bounded caches and durable untrusted evidence.
//! The P10 host owns this service and retires it on epoch/configuration changes.
use crate::{
    provider::http::CancelToken,
    web_http::{WebCredential, WebHttp, WebResolver, WebResponse},
};
use ag1_core::{
    DomainError,
    context::ContextArtifactPort,
    tools::{
        approval::{AuthorizedCall, ExecutionFence},
        executor::{AdapterOutcome, Effect},
        registry::{Origin, Registry, Scope},
        research::{self, Profile, UrlHint},
    },
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};
use tokio::time::Instant;
const TTL: u64 = 15 * 60 * 1000;
const CACHE_BYTES: usize = 8 * 1024 * 1024;
struct Cached {
    key: (String, String),
    at: u64,
    response: Arc<WebResponse>,
    cost: usize,
}
struct Memory {
    cache: VecDeque<Cached>,
    bytes: usize,
    hints: BTreeMap<String, (u64, UrlHint)>,
    outcomes: BTreeMap<String, (u64, String)>,
}
impl Memory {
    fn prune(&mut self, now: u64) {
        self.cache.retain(|v| now >= v.at && now - v.at < TTL);
        self.bytes = self.cache.iter().map(|v| v.cost).sum();
        self.hints
            .retain(|_, (at, _)| now >= *at && now - *at < TTL);
        self.outcomes
            .retain(|_, (at, _)| now >= *at && now - *at < TTL);
    }
    fn insert_cache(&mut self, key: (String, String), at: u64, response: Arc<WebResponse>) {
        let cost = response.bytes.len()
            + response.requested_url.len()
            + response.final_url.len()
            + response.content_type.len()
            + response
                .redirect_chain
                .iter()
                .map(String::len)
                .sum::<usize>()
            + key.0.len()
            + key.1.len();
        self.cache.retain(|v| v.key != key);
        self.bytes = self.cache.iter().map(|v| v.cost).sum();
        while self.cache.len() >= 32 || self.bytes + cost > CACHE_BYTES {
            let Some(old) = self.cache.pop_front() else {
                return;
            };
            self.bytes -= old.cost;
        }
        self.bytes += cost;
        self.cache.push_back(Cached {
            key,
            at,
            response,
            cost,
        });
    }
}
pub struct WebService<R> {
    http: WebHttp<R>,
    scope: Scope,
    target: String,
    credentials: BTreeMap<String, WebCredential>,
    memory: Mutex<Memory>,
}
pub struct WebExecution<'a, F> {
    pub registry: &'a Registry,
    pub scope: &'a Scope,
    pub policy_revision: u64,
    pub now: F,
    pub deadline: Instant,
    pub cancel: &'a mut CancelToken,
    pub profile: Profile,
}
impl<R: WebResolver> WebService<R> {
    /// `incarnation` must be freshly allocated by the host, including after credential
    /// or policy reload. Secrets cannot be replaced under an existing approval stamp.
    pub fn new(
        http: WebHttp<R>,
        scope: Scope,
        incarnation: &str,
        credentials: BTreeMap<String, WebCredential>,
    ) -> Result<Self, DomainError> {
        ag1_core::OperationId::new(incarnation)?;
        if credentials.len() > 64 || credentials.keys().any(|k| k.is_empty() || k.len() > 128) {
            return Err(DomainError::LimitExceeded("web credential names".into()));
        }
        let target = format!(
            "web:{}:{}:{}:{incarnation}",
            scope.workspace, scope.session, scope.epoch
        );
        Ok(Self {
            http,
            scope,
            target,
            credentials,
            memory: Mutex::new(Memory {
                cache: VecDeque::new(),
                bytes: 0,
                hints: BTreeMap::new(),
                outcomes: BTreeMap::new(),
            }),
        })
    }
    pub fn target_revision(&self) -> &str {
        &self.target
    }
    fn scope(&self, scope: &Scope) -> Result<(), DomainError> {
        if scope.workspace != self.scope.workspace
            || scope.session != self.scope.session
            || scope.epoch != self.scope.epoch
        {
            Err(DomainError::PermissionDenied)
        } else {
            Ok(())
        }
    }
    /// Called with core-produced search hints only after the search result is acknowledged.
    pub fn remember_hints(
        &self,
        scope: &Scope,
        urls: &[String],
        hint: &UrlHint,
        now: u64,
    ) -> Result<(), DomainError> {
        self.scope(scope)?;
        if urls.len() > 32
            || urls.iter().any(|u| u.len() > 8192)
            || hint.focus_terms.len() > 64
            || hint.focus_terms.iter().any(|t| t.len() > 256)
            || !matches!(
                hint.intent.as_str(),
                "general" | "current_events" | "docs_reference" | "code_lookup"
            )
        {
            return Err(DomainError::LimitExceeded("research hints".into()));
        }
        let mut memory = self
            .memory
            .lock()
            .map_err(|_| DomainError::Unavailable("web memory".into()))?;
        memory.prune(now);
        for url in urls {
            if memory.hints.len() >= 256 && !memory.hints.contains_key(url) {
                let key = memory
                    .hints
                    .iter()
                    .min_by_key(|(_, v)| v.0)
                    .map(|(k, _)| k.clone())
                    .unwrap();
                memory.hints.remove(&key);
            }
            memory.hints.insert(url.clone(), (now, hint.clone()));
        }
        Ok(())
    }
    pub fn outcome(
        &self,
        scope: &Scope,
        url: &str,
        now: u64,
    ) -> Result<Option<String>, DomainError> {
        self.scope(scope)?;
        let mut memory = self
            .memory
            .lock()
            .map_err(|_| DomainError::Unavailable("web memory".into()))?;
        memory.prune(now);
        Ok(memory.outcomes.get(url).map(|(_, v)| v.clone()))
    }
    pub fn cache_usage(&self) -> Result<(usize, usize), DomainError> {
        let memory = self
            .memory
            .lock()
            .map_err(|_| DomainError::Unavailable("web memory".into()))?;
        Ok((memory.cache.len(), memory.bytes))
    }
    pub async fn execute<F: Fn() -> u64>(
        &self,
        ticket: AuthorizedCall,
        execution: WebExecution<'_, F>,
        artifacts: &mut impl ContextArtifactPort,
    ) -> Result<AdapterOutcome, DomainError> {
        let WebExecution {
            registry,
            scope,
            policy_revision,
            now,
            deadline,
            cancel,
            profile,
        } = execution;
        self.scope(scope)?;
        if ticket.call().name() != "webFetch" || ticket.call().origin() != Origin::BuiltIn {
            return Err(DomainError::PermissionDenied);
        }
        if cancel.cancelled() {
            return Err(DomainError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(DomainError::TimedOut);
        }
        if profile.fetch_transcript_facts > 6
            || profile.fetch_transcript_preview_lines > 6
            || profile.fetch_transcript_line_chars > 220
        {
            return Err(DomainError::InvalidInput(
                "research compression profile".into(),
            ));
        }
        let call = ticket.consume(
            registry,
            ExecutionFence {
                scope,
                policy_revision,
                target_revision: &self.target,
                now: now(),
            },
        )?;
        let url = call.args()["url"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("web URL".into()))?;
        let credential_name = call
            .args()
            .get("credential")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let credential = if credential_name.is_empty() {
            None
        } else {
            match self.credentials.get(credential_name) {
                Some(value) => Some(value),
                None => {
                    return Ok(error(
                        url,
                        &format!(
                            "Unknown webFetch credential name. Available: {}",
                            self.credentials
                                .keys()
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    ));
                }
            }
        };
        if credential.is_some_and(|value| !value.permits(url)) {
            let host = reqwest::Url::parse(url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .unwrap_or_else(|| "invalid URL".into());
            return Ok(error(
                url,
                &format!("Credential is not allowed for host {host}."),
            ));
        }
        let key = (credential_name.to_string(), url.to_string());
        let (cached, hint) = {
            let mut memory = self
                .memory
                .lock()
                .map_err(|_| DomainError::Unavailable("web memory".into()))?;
            memory.prune(now());
            (
                memory
                    .cache
                    .iter()
                    .find(|v| v.key == key)
                    .map(|v| v.response.clone()),
                memory.hints.get(url).map(|(_, v)| v.clone()),
            )
        };
        let cache_hit = cached.is_some();
        let response = if let Some(cached) = cached {
            cached
        } else {
            match self.http.fetch(url, credential, deadline, cancel).await {
                Ok(value) => Arc::new(value),
                Err(e) => return Ok(error(url, &error_message(e))),
            }
        };
        if cancel.cancelled() {
            return Ok(error(url, "Fetch cancelled."));
        }
        if Instant::now() >= deadline {
            return Ok(error(url, "Fetch timed out."));
        }
        let body_sha256 = format!("{:x}", Sha256::digest(&response.bytes));
        let original = json!({"version":1,"requestedUrl":url,"finalUrl":response.final_url,"redirectChain":response.redirect_chain,"statusCode":response.status,"contentType":response.content_type,"bodyBase64":STANDARD.encode(&response.bytes),"bodySha256":body_sha256,"trust":"untrusted","cacheHit":cache_hit});
        let artifact = artifacts.put_evidence(
            &original.to_string(),
            "web-response",
            &json!({"type":"tool","name":"webFetch","callId":call.call_id()}),
            &[],
            None,
        )?;
        let projection = research::fetch_projection(
            research::FetchedDocument {
                url,
                final_url: &response.final_url,
                status_code: response.status,
                content_type: if response.content_type.is_empty() {
                    None
                } else {
                    Some(&response.content_type)
                },
                cache_hit,
                bytes: &response.bytes,
            },
            profile,
            hint.as_ref(),
        )?;
        if cancel.cancelled() {
            return Ok(error(url, "Fetch cancelled after evidence capture."));
        }
        if Instant::now() >= deadline {
            return Ok(error(url, "Fetch timed out after evidence capture."));
        }
        let mut result = projection.result;
        result["details"]["networkEvidence"] = json!({"trust":"untrusted","artifactId":artifact.id,"bodySha256":body_sha256,"redirectChain":response.redirect_chain});
        let mut memory = self
            .memory
            .lock()
            .map_err(|_| DomainError::Unavailable("web memory".into()))?;
        memory.prune(now());
        if !cache_hit {
            memory.insert_cache(key, now(), response);
        }
        if let Some(status) = projection.status {
            if memory.outcomes.len() >= 256 && !memory.outcomes.contains_key(url) {
                let key = memory
                    .outcomes
                    .iter()
                    .min_by_key(|(_, v)| v.0)
                    .map(|(k, _)| k.clone())
                    .unwrap();
                memory.outcomes.remove(&key);
            }
            memory.outcomes.insert(url.into(), (now(), status));
        }
        Ok(AdapterOutcome {
            result,
            effect: Effect::None,
            artifact_page: false,
        })
    }
}
fn error(url: &str, message: &str) -> AdapterOutcome {
    AdapterOutcome {
        result: json!({"isError":true,"details":{"url":url,"error":message,"status":"error"},"content":[{"type":"text","text":format!("Fetch Error: {message}")}]}),
        effect: Effect::None,
        artifact_page: false,
    }
}
fn error_message(error: DomainError) -> String {
    match error {
        DomainError::PermissionDenied => {
            "URL or credential is outside the permitted web destinations.".into()
        }
        DomainError::Cancelled => "Fetch cancelled.".into(),
        DomainError::TimedOut => "Fetch timed out.".into(),
        DomainError::LimitExceeded(reason)
        | DomainError::InvalidInput(reason)
        | DomainError::Unavailable(reason) => reason,
        DomainError::QueueFull => "Web fetch capacity is full.".into(),
        _ => "Web fetch failed.".into(),
    }
}
mod search;
pub use search::{SearchConfig, SearchEndpoints, SearchService};
