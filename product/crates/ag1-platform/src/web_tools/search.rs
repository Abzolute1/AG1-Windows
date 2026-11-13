//! Native search providers share fetch's DNS/TLS boundaries and scoped hint memory.
use super::*;
mod ddg_web;
use crate::web_http::WebPurpose;
use ag1_core::tools::websearch::{self, Attempt, ProviderResponse};
use std::time::Duration;
// Serialize public DuckDuckGo searches across native services in this process.
// No fixed success delay: DDG publishes no verified HTML/lite request quota.
static DUCKDUCKGO_SEARCH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn search_slot<'a>(
    gate: &'a tokio::sync::Mutex<()>,
    deadline: Instant,
    cancel: &mut CancelToken,
) -> Result<tokio::sync::MutexGuard<'a, ()>, DomainError> {
    tokio::select! {
        biased;
        _=cancel.wait()=>Err(DomainError::Cancelled),
        _=tokio::time::sleep_until(deadline)=>Err(DomainError::TimedOut),
        value=gate.lock()=>Ok(value),
    }
}
#[derive(Clone)]
pub struct SearchEndpoints {
    pub web: Option<(String, String)>,
    pub html: String,
    pub lite: String,
    pub brave: String,
}
impl Default for SearchEndpoints {
    fn default() -> Self {
        Self {
            web: Some((
                "https://noai.duckduckgo.com/".into(),
                "https://links.duckduckgo.com/d.js".into(),
            )),
            html: "https://html.duckduckgo.com/html/".into(),
            lite: "https://lite.duckduckgo.com/lite/".into(),
            brave: "https://api.search.brave.com/res/v1/web/search".into(),
        }
    }
}
pub struct SearchConfig {
    pub mode: String,
    pub endpoints: SearchEndpoints,
    pub brave: Option<WebCredential>,
}
impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            mode: "auto".into(),
            endpoints: SearchEndpoints::default(),
            brave: None,
        }
    }
}
struct SearchCache {
    key: (String, u32),
    at: u64,
    ttl: u64,
    response: ProviderResponse,
    evidence: Vec<String>,
    cost: usize,
}
pub struct SearchService<R> {
    web: Arc<WebService<R>>,
    config: SearchConfig,
    target: String,
    cache: Mutex<VecDeque<SearchCache>>,
    pending: tokio::sync::Mutex<()>,
    // Provider-wide rather than per query: rephrasing cannot fix a challenge.
    blocked_until: Mutex<Option<Instant>>,
}
impl<R: WebResolver> SearchService<R> {
    pub fn new(
        web: Arc<WebService<R>>,
        config: SearchConfig,
        incarnation: &str,
    ) -> Result<Self, DomainError> {
        ag1_core::OperationId::new(incarnation)?;
        if !matches!(
            config.mode.as_str(),
            "auto" | "duckduckgo" | "brave" | "serpapi" | "tavily" | "exa"
        ) {
            return Err(DomainError::InvalidInput("web search provider".into()));
        }
        let mut endpoints = vec![
            &config.endpoints.html,
            &config.endpoints.lite,
            &config.endpoints.brave,
        ];
        if let Some((landing, data)) = &config.endpoints.web {
            endpoints.extend([landing, data]);
        }
        for endpoint in endpoints {
            let url = reqwest::Url::parse(endpoint)
                .map_err(|_| DomainError::InvalidInput("search endpoint".into()))?;
            if endpoint.len() > 4096
                || !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(DomainError::InvalidInput("search endpoint".into()));
            }
        }
        let target = format!("{}:search:{incarnation}", web.target_revision());
        Ok(Self {
            web,
            config,
            target,
            cache: Mutex::new(VecDeque::new()),
            pending: tokio::sync::Mutex::new(()),
            blocked_until: Mutex::new(None),
        })
    }
    pub fn target_revision(&self) -> &str {
        &self.target
    }
    fn order(&self) -> Vec<String> {
        let mode = self.config.mode.as_str();
        if mode == "duckduckgo" || (mode == "auto" && self.config.brave.is_none()) {
            vec!["duckduckgo".into()]
        } else {
            vec![
                if mode == "auto" { "brave" } else { mode }.into(),
                "duckduckgo".into(),
            ]
        }
    }
    pub async fn execute<F: Fn() -> u64>(
        &self,
        ticket: AuthorizedCall,
        execution: WebExecution<'_, F>,
        year: u32,
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
        self.web.scope(scope)?;
        if ticket.call().name() != "webSearch" || ticket.call().origin() != Origin::BuiltIn {
            return Err(DomainError::PermissionDenied);
        }
        if cancel.cancelled() {
            return Err(DomainError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(DomainError::TimedOut);
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
        let query = search_query(call.args())?;
        let maximum = call.args()["maxResults"].as_f64().unwrap_or(3.);
        if !maximum.is_finite() || maximum < 1. || maximum.fract() != 0. {
            return Err(DomainError::InvalidInput(
                "maxResults must be a positive integer".into(),
            ));
        }
        let max = maximum.min(4.) as usize;
        let plan = websearch::plan(query, year)?;
        // Cache the bounded candidate pool, independent of presentation count.
        // Waiting callers recheck the cache after the owner completes, so an
        // identical concurrent search does not spend another provider request.
        let _pending = match search_slot(&self.pending, deadline, cancel).await {
            Ok(slot) => slot,
            Err(DomainError::Cancelled) => return Ok(search_cancelled(query, true)),
            Err(DomainError::TimedOut) => return Ok(search_cancelled(query, false)),
            Err(error) => return Err(error),
        };
        let key = (plan.effective_query.clone(), year);
        let cached = {
            let mut cache = self
                .cache
                .lock()
                .map_err(|_| DomainError::Unavailable("search cache".into()))?;
            cache.retain(|v| now() >= v.at && now() - v.at < v.ttl);
            cache
                .iter()
                .find(|v| v.key == key)
                .map(|v| (v.response.clone(), v.evidence.clone(), now() - v.at))
        };
        let (response, evidence, cache_age) = if let Some((mut response, evidence, age)) = cached {
            response.cache_hit = true;
            for id in &evidence {
                artifacts.get(id)?;
            }
            (response, evidence, age)
        } else {
            let fetched = self
                .providers(
                    &plan.effective_query,
                    24,
                    deadline,
                    cancel,
                    artifacts,
                    call.call_id(),
                )
                .await;
            let (response, evidence) = match fetched {
                Ok(value) => value,
                Err(DomainError::Cancelled) => return Ok(search_cancelled(query, true)),
                Err(DomainError::TimedOut) => return Ok(search_cancelled(query, false)),
                Err(error) => return Err(error),
            };
            if cancel.cancelled() {
                return Ok(search_cancelled(query, true));
            }
            if Instant::now() >= deadline {
                return Ok(search_cancelled(query, false));
            }
            let cost = serde_json::to_vec(&response)
                .map_err(|_| DomainError::InvalidInput("search cache serialization".into()))?
                .len()
                + evidence.iter().map(String::len).sum::<usize>()
                + key.0.len();
            let successful = response.attempts.iter().any(|a| a.status == "ok")
                && !response.attempts.iter().any(|a| a.status == "blocked");
            if successful && cost <= 2 * 1024 * 1024 {
                let mut cache = self
                    .cache
                    .lock()
                    .map_err(|_| DomainError::Unavailable("search cache".into()))?;
                while cache.len() >= 32
                    || cache.iter().map(|v| v.cost).sum::<usize>() + cost > 2 * 1024 * 1024
                {
                    cache.pop_front();
                }
                cache.push_back(SearchCache {
                    key,
                    at: now(),
                    ttl: if response.results.is_empty()
                        || ag1_core::tools::research::intent(query) == "current_events"
                        || query.split_whitespace().any(|t| {
                            matches!(
                                t.to_ascii_lowercase().as_str(),
                                "latest" | "current" | "today" | "recent"
                            )
                        }) {
                        60_000
                    } else {
                        TTL
                    },
                    response: response.clone(),
                    evidence: evidence.clone(),
                    cost,
                });
            }
            (response, evidence, 0)
        };
        drop(_pending);
        let outcomes = {
            let mut memory = self
                .web
                .memory
                .lock()
                .map_err(|_| DomainError::Unavailable("web memory".into()))?;
            memory.prune(now());
            memory
                .outcomes
                .iter()
                .map(|(url, (_, v))| (url.clone(), v.clone()))
                .collect()
        };
        let projection = websearch::project(query, year, max, response, profile, &outcomes)?;
        if cancel.cancelled() {
            return Ok(search_cancelled(query, true));
        }
        if Instant::now() >= deadline {
            return Ok(search_cancelled(query, false));
        }
        for (url, hint) in &projection.hints {
            self.web
                .remember_hints(scope, std::slice::from_ref(url), hint, now())?;
        }
        let mut result = projection.result;
        result["details"]["cacheAgeMs"] = json!(cache_age);
        result["details"]["networkEvidence"] =
            json!({"trust":"untrusted","artifactIds":evidence,"fallbackUrlsFetched":false});
        Ok(AdapterOutcome {
            result,
            effect: Effect::None,
            artifact_page: false,
        })
    }
    async fn providers(
        &self,
        query: &str,
        max: usize,
        deadline: Instant,
        cancel: &mut CancelToken,
        artifacts: &mut impl ContextArtifactPort,
        call_id: &str,
    ) -> Result<(ProviderResponse, Vec<String>), DomainError> {
        let order = self.order();
        let mut attempts = vec![];
        let mut evidence = vec![];
        for name in &order {
            if cancel.cancelled() {
                return Err(DomainError::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(DomainError::TimedOut);
            }
            if name == "duckduckgo"
                && self
                    .blocked_until
                    .lock()
                    .map_err(|_| DomainError::Unavailable("search cooldown".into()))?
                    .is_some_and(|until| Instant::now() < until)
            {
                attempts.push(Attempt { provider: name.clone(), url: "provider:duckduckgo".into(),
                    status: "blocked".into(), result_count: 0,
                    error: Some("DuckDuckGo blocked or rate-limited automated access. Search is paused for up to five minutes; changing the query will not resolve this.".into()) });
                continue;
            }
            if matches!(name.as_str(), "serpapi" | "tavily" | "exa") {
                attempts.push(Attempt{provider:name.clone(),url:format!("provider:{name}:{query}"),status:"skipped".into(),result_count:0,error:Some(format!("{name} provider is not implemented yet; AG1_{}_API_KEY is optional and DuckDuckGo fallback remains available",name.to_uppercase()))});
                continue;
            }
            // Trusted local fixture endpoints exercise the same transport
            // without participating in the public-service serialization.
            let public_ddg = name == "duckduckgo"
                && reqwest::Url::parse(&self.config.endpoints.html)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_owned))
                    .is_some_and(|host| {
                        host == "duckduckgo.com" || host.ends_with(".duckduckgo.com")
                    });
            let _search_slot = if public_ddg {
                Some(search_slot(&DUCKDUCKGO_SEARCH, deadline, cancel).await?)
            } else {
                None
            };
            // Another queued call may have encountered a challenge meanwhile.
            if public_ddg
                && self
                    .blocked_until
                    .lock()
                    .map_err(|_| DomainError::Unavailable("search cooldown".into()))?
                    .is_some_and(|until| Instant::now() < until)
            {
                attempts.push(Attempt {
                    provider: name.clone(),
                    url: "provider:duckduckgo".into(),
                    status: "blocked".into(),
                    result_count: 0,
                    error: Some("DuckDuckGo is paused after a blocked request.".into()),
                });
                continue;
            }
            if name == "duckduckgo"
                && let Some((landing, data)) = &self.config.endpoints.web
            {
                let result = self
                    .duckduckgo_web(
                        landing,
                        data,
                        max,
                        &mut ddg_web::WebAttempt {
                            query,
                            deadline: deadline.min(Instant::now() + Duration::from_secs(12)),
                            cancel,
                            artifacts,
                            evidence: &mut evidence,
                            call_id,
                        },
                    )
                    .await;
                match result {
                    Ok(results) => {
                        attempts.push(Attempt {
                            provider: "duckduckgo-web".into(),
                            url: search_url(landing, query, None),
                            status: "ok".into(),
                            result_count: results.len(),
                            error: None,
                        });
                        return Ok((
                            ProviderResponse {
                                results,
                                attempts,
                                provider: "duckduckgo-web".into(),
                                provider_mode: self.config.mode.clone(),
                                provider_order: order,
                                cache_hit: false,
                            },
                            evidence,
                        ));
                    }
                    Err(DomainError::Cancelled) => return Err(DomainError::Cancelled),
                    Err(DomainError::PermissionDenied) => {
                        return Err(DomainError::PermissionDenied);
                    }
                    Err(DomainError::TimedOut) if Instant::now() >= deadline => {
                        return Err(DomainError::TimedOut);
                    }
                    Err(error) => {
                        let message = error_message(error);
                        let blocked = message == "DuckDuckGo web blocked"
                            || message == "web HTTP status 403"
                            || message == "web HTTP status 429";
                        attempts.push(Attempt {
                            provider: "duckduckgo-web".into(),
                            url: search_url(landing, query, None),
                            status: if blocked { "blocked" } else { "error" }.into(),
                            result_count: 0,
                            error: Some(message),
                        });
                        if blocked {
                            *self.blocked_until.lock().map_err(|_| {
                                DomainError::Unavailable("search cooldown".into())
                            })? = Some(Instant::now() + Duration::from_secs(300));
                            continue;
                        }
                    }
                }
            }
            let endpoints = if name == "brave" {
                vec![(
                    "brave",
                    &self.config.endpoints.brave,
                    WebPurpose::SearchJson,
                )]
            } else {
                vec![
                    (
                        "duckduckgo-html",
                        &self.config.endpoints.html,
                        WebPurpose::SearchHtml,
                    ),
                    (
                        "duckduckgo-lite",
                        &self.config.endpoints.lite,
                        WebPurpose::SearchHtml,
                    ),
                ]
            };
            for (provider, endpoint, purpose) in endpoints {
                let url = search_url(
                    endpoint,
                    query,
                    if name == "brave" {
                        Some(max.min(20))
                    } else {
                        None
                    },
                );
                if name == "brave" && self.config.brave.is_none() {
                    attempts.push(Attempt {
                        provider: provider.into(),
                        url,
                        status: "skipped".into(),
                        result_count: 0,
                        error: Some("AG1_BRAVE_SEARCH_API_KEY is not configured".into()),
                    });
                    continue;
                }
                let credential = if name == "brave" {
                    self.config.brave.as_ref()
                } else {
                    None
                };
                let limit = deadline.min(Instant::now() + Duration::from_secs(8));
                let fetched = self
                    .web
                    .http
                    .fetch_for(&url, credential, purpose, limit, cancel)
                    .await;
                match fetched {
                    Ok(response) => {
                        let raw = json!({"version":1,"initialRequestMethod":if name=="duckduckgo"{"POST"}else{"GET"},"initialRequestTarget":if name=="duckduckgo"{endpoint.as_str()}else{url.as_str()},"searchQuery":query,"requestedUrl":url,"finalUrl":response.final_url,"redirectChain":response.redirect_chain,"statusCode":response.status,"contentType":response.content_type,"bodyBase64":STANDARD.encode(&response.bytes),"bodySha256":format!("{:x}",Sha256::digest(&response.bytes)),"bodyRepresentation":"decoded-entity","trust":"untrusted"});
                        let artifact = artifacts.put_evidence(
                            &raw.to_string(),
                            "web-response",
                            &json!({"type":"tool","name":"webSearch","callId":call_id}),
                            &[],
                            None,
                        )?;
                        evidence.push(artifact.id);
                        if name == "duckduckgo" {
                            let text = String::from_utf8_lossy(&response.bytes);
                            let status = websearch::duckduckgo_page_status(&text);
                            if status == "blocked"
                                || status == "parse_error"
                                || response.status != 200
                            {
                                let blocked = status == "blocked";
                                attempts.push(Attempt { provider: provider.into(), url,
                                    status: if blocked { "blocked" } else { "parse_error" }.into(),
                                    result_count: 0, error: Some(if blocked {
                                        "DuckDuckGo requires human verification; no search results were received.".into()
                                    } else { format!("Unrecognized DuckDuckGo response (HTTP {}). No usable search results were received.", response.status) }) });
                                if blocked {
                                    *self.blocked_until.lock().map_err(|_| {
                                        DomainError::Unavailable("search cooldown".into())
                                    })? = Some(Instant::now() + Duration::from_secs(300));
                                    break;
                                }
                                continue;
                            }
                        }
                        let parsed = if name == "brave" {
                            serde_json::from_slice::<Value>(&response.bytes)
                                .map_err(|_| {
                                    DomainError::InvalidInput("search provider JSON".into())
                                })
                                .and_then(|data| websearch::parse_brave(&data, max))
                        } else {
                            let text = String::from_utf8_lossy(&response.bytes);
                            websearch::parse_duckduckgo(
                                text.strip_prefix('\u{feff}').unwrap_or(&text),
                                max,
                            )
                            .map(|rows| {
                                let n = rows.len();
                                (rows, n)
                            })
                        };
                        match parsed {
                            Ok((results, count)) => {
                                if name == "duckduckgo"
                                    && results.is_empty()
                                    && websearch::duckduckgo_page_status(&String::from_utf8_lossy(
                                        &response.bytes,
                                    )) == "results"
                                {
                                    attempts.push(Attempt { provider: provider.into(), url,
                                        status: "parse_error".into(), result_count: 0,
                                        error: Some("DuckDuckGo result markup contained no usable result links.".into()) });
                                    continue;
                                }
                                attempts.push(Attempt {
                                    provider: provider.into(),
                                    url,
                                    status: "ok".into(),
                                    result_count: count,
                                    error: None,
                                });
                                if !results.is_empty() {
                                    return Ok((
                                        ProviderResponse {
                                            results,
                                            attempts,
                                            provider: provider.into(),
                                            provider_mode: self.config.mode.clone(),
                                            provider_order: order,
                                            cache_hit: false,
                                        },
                                        evidence,
                                    ));
                                }
                            }
                            Err(e) => attempts.push(Attempt {
                                provider: provider.into(),
                                url,
                                status: "error".into(),
                                result_count: 0,
                                error: Some(error_message(e)),
                            }),
                        }
                    }
                    Err(DomainError::Cancelled) => return Err(DomainError::Cancelled),
                    Err(DomainError::TimedOut) if Instant::now() >= deadline => {
                        return Err(DomainError::TimedOut);
                    }
                    Err(e) => {
                        let message = error_message(e);
                        let code = message
                            .strip_prefix("web HTTP status ")
                            .and_then(|v| v.parse::<u16>().ok());
                        let blocked = name == "duckduckgo" && matches!(code, Some(403 | 429));
                        if blocked {
                            *self.blocked_until.lock().map_err(|_| {
                                DomainError::Unavailable("search cooldown".into())
                            })? = Some(Instant::now() + Duration::from_secs(300));
                        }
                        let status = if blocked {
                            "blocked"
                        } else if code.is_some() {
                            "http_error"
                        } else {
                            "error"
                        };
                        let error = if let Some(code) = code {
                            format!(
                                "{code} {}",
                                reqwest::StatusCode::from_u16(code)
                                    .ok()
                                    .and_then(|s| s.canonical_reason())
                                    .unwrap_or_default()
                            )
                            .trim()
                            .to_string()
                        } else {
                            message
                        };
                        attempts.push(Attempt {
                            provider: provider.into(),
                            url,
                            status: status.into(),
                            result_count: 0,
                            error: Some(error),
                        });
                        if blocked {
                            break;
                        }
                    }
                }
            }
        }
        let provider = attempts
            .last()
            .map(|v| v.provider.clone())
            .or_else(|| order.last().cloned())
            .unwrap_or_else(|| "duckduckgo".into());
        Ok((
            ProviderResponse {
                results: vec![],
                attempts,
                provider,
                provider_mode: self.config.mode.clone(),
                provider_order: order,
                cache_hit: false,
            },
            evidence,
        ))
    }
}

// Treat a missing or blank query as a failed tool call. Returning a normal
// result here made the model believe that `webSearch {}` had succeeded, so it
// could repeat the same useless call instead of correcting its arguments.
fn search_query(arguments: &Value) -> Result<&str, DomainError> {
    let query = arguments["query"]
        .as_str()
        .ok_or_else(|| DomainError::InvalidInput("webSearch needs a non-empty query.".into()))?
        .trim();
    if query.is_empty() {
        return Err(DomainError::InvalidInput(
            "webSearch needs a non-empty query.".into(),
        ));
    }
    Ok(query)
}

fn search_url(endpoint: &str, query: &str, max: Option<usize>) -> String {
    let encoded = query
        .as_bytes()
        .iter()
        .map(|&b| {
            if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect::<String>();
    format!(
        "{endpoint}?q={encoded}{}",
        max.map(|n| format!("&count={n}")).unwrap_or_default()
    )
}

fn search_cancelled(query: &str, cancelled: bool) -> AdapterOutcome {
    let reason = if cancelled {
        "Web search cancelled."
    } else {
        "Web search timed out."
    };
    AdapterOutcome {
        result: json!({"isError":true,"details":{"query":query,"error":reason,"termination":if cancelled{"cancelled"}else{"timeout"}},"content":[{"type":"text","text":format!("Search Error: {reason}")}]}),
        effect: Effect::None,
        artifact_page: false,
    }
}

#[cfg(test)]
mod serialization_tests {
    use super::*;
    use crate::provider::http::cancellation;

    #[test]
    fn search_rejects_missing_or_blank_queries() {
        for arguments in [json!({}), json!({"query":"   "})] {
            let error = search_query(&arguments).unwrap_err();
            assert!(
                matches!(error, DomainError::InvalidInput(message) if message == "webSearch needs a non-empty query.")
            );
        }
        assert_eq!(
            search_query(&json!({"query":"  Rust release notes  "})).unwrap(),
            "Rust release notes"
        );
    }

    #[tokio::test]
    async fn search_serialization_has_no_padding_and_honors_cancellation_and_deadline() {
        let gate = tokio::sync::Mutex::new(());
        let (_source, mut cancel) = cancellation();
        for _ in 0..2 {
            let mut slot = Box::pin(search_slot(
                &gate,
                Instant::now() + Duration::from_secs(1),
                &mut cancel,
            ));
            // An uncontended call must resolve on its first poll, without a timer.
            std::future::poll_fn(|cx| {
                assert!(slot.as_mut().poll(cx).is_ready());
                std::task::Poll::Ready(())
            })
            .await;
        }
        let held = gate.lock().await;
        assert!(matches!(
            search_slot(
                &gate,
                Instant::now() + Duration::from_millis(10),
                &mut cancel
            )
            .await,
            Err(DomainError::TimedOut)
        ));
        let (source, mut cancel) = cancellation();
        source.cancel();
        assert!(matches!(
            search_slot(&gate, Instant::now() + Duration::from_secs(1), &mut cancel).await,
            Err(DomainError::Cancelled)
        ));
        drop(held);
    }
}
