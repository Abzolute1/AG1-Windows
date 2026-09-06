//! Native search providers share fetch's DNS/TLS boundaries and scoped hint memory.
use super::*;
use crate::web_http::WebPurpose;
use ag1_core::tools::websearch::{self, Attempt, ProviderResponse};
use std::time::Duration;
#[derive(Clone)]
pub struct SearchEndpoints {
    pub html: String,
    pub lite: String,
    pub brave: String,
}
impl Default for SearchEndpoints {
    fn default() -> Self {
        Self {
            html: "https://html.duckduckgo.com/html/".into(),
            lite: "https://duckduckgo.com/html/".into(),
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
    key: (String, usize, u32),
    at: u64,
    response: ProviderResponse,
    evidence: Vec<String>,
    cost: usize,
}
pub struct SearchService<R> {
    web: Arc<WebService<R>>,
    config: SearchConfig,
    target: String,
    cache: Mutex<VecDeque<SearchCache>>,
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
        for endpoint in [
            &config.endpoints.html,
            &config.endpoints.lite,
            &config.endpoints.brave,
        ] {
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
        let query = call.args()["query"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("search query".into()))?;
        if query.is_empty() {
            return Ok(AdapterOutcome {
                result: json!({"details":{},"content":[{"type":"text","text":"Error: 'query' parameter is required."}]}),
                effect: Effect::None,
                artifact_page: false,
            });
        }
        let maximum = call.args()["maxResults"].as_f64().unwrap_or(3.);
        if !maximum.is_finite() || maximum < 1. || maximum.fract() != 0. {
            return Err(DomainError::InvalidInput(
                "maxResults must be a positive integer".into(),
            ));
        }
        let max = maximum.min(4.) as usize;
        let plan = websearch::plan(query, year)?;
        let key = (query.to_string(), max, year);
        let cached = {
            let mut cache = self
                .cache
                .lock()
                .map_err(|_| DomainError::Unavailable("search cache".into()))?;
            cache.retain(|v| {
                now() >= v.at
                    && now() - v.at
                        < if v.response.results.is_empty() {
                            60000
                        } else {
                            TTL
                        }
            });
            cache
                .iter()
                .find(|v| v.key == key)
                .map(|v| (v.response.clone(), v.evidence.clone()))
        };
        let (mut response, evidence) = if let Some((mut response, evidence)) = cached {
            response.cache_hit = true;
            for id in &evidence {
                artifacts.get(id)?;
            }
            (response, evidence)
        } else {
            let fetched = self
                .providers(
                    &plan.effective_query,
                    max,
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
            if cost <= 2 * 1024 * 1024 {
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
                    response: response.clone(),
                    evidence: evidence.clone(),
                    cost,
                });
            }
            (response, evidence)
        };
        response.results.truncate(max);
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
            if matches!(name.as_str(), "serpapi" | "tavily" | "exa") {
                attempts.push(Attempt{provider:name.clone(),url:format!("provider:{name}:{query}"),status:"skipped".into(),result_count:0,error:Some(format!("{name} provider is not implemented yet; AG1_{}_API_KEY is optional and DuckDuckGo fallback remains available",name.to_uppercase()))});
                continue;
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
                    if name == "brave" { Some(max) } else { None },
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
                        let raw = json!({"version":1,"requestedUrl":url,"finalUrl":response.final_url,"redirectChain":response.redirect_chain,"statusCode":response.status,"contentType":response.content_type,"bodyBase64":STANDARD.encode(&response.bytes),"bodySha256":format!("{:x}",Sha256::digest(&response.bytes)),"bodyRepresentation":"decoded-entity","trust":"untrusted"});
                        let artifact = artifacts.put_evidence(
                            &raw.to_string(),
                            "web-response",
                            &json!({"type":"tool","name":"webSearch","callId":call_id}),
                            &[],
                            None,
                        )?;
                        evidence.push(artifact.id);
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
                        let status = if code.is_some() {
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
