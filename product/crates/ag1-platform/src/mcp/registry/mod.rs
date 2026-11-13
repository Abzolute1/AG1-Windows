//! Cached public metadata discovery. Neither a cache row nor a setup draft is
//! execution authority. This module never launches/connects an advertised server.
use super::{
    definitions::{Configuration, Draft, Provenance},
    invalid,
    wire::{Limits, decode_json},
};
use ag1_core::DomainError;
use ag1_storage::Store;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
mod icons;
mod network;
const SOURCE: &str = "https://api.mcp.github.com";
const MAX_PAGE_BYTES: usize = 1024 * 1024;
const TTL: u64 = 300;
fn now() -> Result<u64, DomainError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DomainError::Conflict)?
        .as_secs())
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Entry {
    key: String,
    server: Value,
    registry_meta: Value,
    digest: String,
    #[serde(default)]
    icon_url: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Page {
    #[serde(default)]
    icon_version: u8,
    query: String,
    cursor: String,
    fetched_at: u64,
    entries: Vec<Entry>,
    next_cursor: Option<String>,
    excluded: usize,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cache {
    pages: VecDeque<Page>,
    retry_at: u64,
}
struct State {
    cache: Cache,
    revision: u64,
    requests: VecDeque<u64>,
}
pub struct Directory {
    icons: Mutex<icons::Icons>,
    icon_ready: std::sync::Condvar,
    store: Option<Store>,
    state: Mutex<State>,
    reader: Box<dyn Fn(reqwest::Url) -> Result<(Vec<u8>, u64), DomainError> + Send + Sync>,
}
impl Directory {
    pub fn open(store: Store) -> Result<Self, DomainError> {
        let saved = store.mcp_state()?;
        let (cache, revision) = if let Some(saved) = saved {
            (
                serde_json::from_value(decode_json(&saved.bytes, Limits::default())?)
                    .map_err(|_| invalid("registry cache"))?,
                saved.revision,
            )
        } else {
            (Cache::default(), 0)
        };
        let cache: Cache = cache;
        if cache.pages.len() > 32 || cache.pages.iter().any(|p| p.entries.len() > 10) {
            return Err(invalid("registry cache bounds"));
        }
        Ok(Self {
            icons: Mutex::new(icons::Icons::default()),
            icon_ready: std::sync::Condvar::new(),
            store: Some(store),
            state: Mutex::new(State {
                cache,
                revision,
                requests: VecDeque::new(),
            }),
            reader: Box::new(network::get),
        })
    }
    pub fn memory() -> Self {
        Self {
            icons: Mutex::new(icons::Icons::default()),
            icon_ready: std::sync::Condvar::new(),
            store: None,
            state: Mutex::new(State {
                cache: Cache::default(),
                revision: 0,
                requests: VecDeque::new(),
            }),
            reader: Box::new(network::get),
        }
    }
    /// Host-only reader injection for owned metadata fixtures; production uses
    /// the fixed HTTPS client. No renderer configuration can replace this port.
    pub fn with_reader(
        reader: impl Fn(reqwest::Url) -> Result<(Vec<u8>, u64), DomainError> + Send + Sync + 'static,
    ) -> Self {
        let mut directory = Self::memory();
        directory.reader = Box::new(reader);
        directory
    }
    fn save(&self, state: &mut State) -> Result<(), DomainError> {
        loop {
            let bytes = serde_json::to_vec(&state.cache).map_err(|_| invalid("registry cache"))?;
            if bytes.len() <= 900 * 1024 && state.cache.pages.len() <= 32 {
                if let Some(store) = &self.store {
                    state.revision = store.commit_mcp_state(state.revision, &bytes)?;
                }
                return Ok(());
            }
            if state.cache.pages.pop_front().is_none() {
                return Err(DomainError::LimitExceeded("registry cache".into()));
            }
        }
    }
    /// Opening Discover reads cache only. Explicit Search/Refresh may fetch the
    /// fixed public registry; query text is typed by the user, never chat/source.
    pub fn search(
        &self,
        query: &str,
        cursor: &str,
        fetch: bool,
        refresh: bool,
    ) -> Result<Value, DomainError> {
        if query.len() > 128
            || query.chars().any(char::is_control)
            || cursor.len() > 4096
            || cursor.chars().any(char::is_control)
        {
            return Err(invalid("registry search bounds"));
        }
        let mut state = self.state.lock().map_err(|_| DomainError::Conflict)?;
        let current = now()?;
        let cached = state
            .cache
            .pages
            .iter()
            .find(|p| p.query == query && p.cursor == cursor)
            .cloned();
        let mut page = cached.clone();
        let mut error = None;
        if fetch
            && (refresh
                || cached
                    .as_ref()
                    .is_none_or(|p| current.saturating_sub(p.fetched_at) >= TTL))
        {
            state.requests.retain(|t| current.saturating_sub(*t) < 60);
            if current < state.cache.retry_at || state.requests.len() >= 5 {
                error = Some(
                    "Discovery request limit reached. Use cached results and try again later."
                        .to_owned(),
                );
            } else {
                state.requests.push_back(current);
                let mut url = reqwest::Url::parse(&format!("{SOURCE}/v0.1/servers"))
                    .map_err(|_| invalid("registry source"))?;
                url.query_pairs_mut()
                    .append_pair("limit", "10")
                    .append_pair("version", "latest");
                if !query.is_empty() {
                    url.query_pairs_mut().append_pair("search", query);
                }
                if !cursor.is_empty() {
                    url.query_pairs_mut().append_pair("cursor", cursor);
                }
                match (self.reader)(url).and_then(|(bytes, retry)| {
                    state.cache.retry_at = retry;
                    if bytes.is_empty() {
                        return Err(DomainError::Unavailable(
                            "Discovery source rate limit reached. Try again later.".into(),
                        ));
                    }
                    parse_page(&bytes, query, cursor, current)
                }) {
                    Ok(new) => {
                        state
                            .cache
                            .pages
                            .retain(|p| p.query != query || p.cursor != cursor);
                        state.cache.pages.push_back(new.clone());
                        page = Some(new);
                    }
                    Err(e) => {
                        error=Some(match e{DomainError::Unavailable(s)=>s,DomainError::TimedOut=>"Discovery source timed out. Cached results remain available.".into(),_=>"Discovery source response could not be used safely. Cached results remain available.".into()});
                    }
                }
                self.save(&mut state)?;
            }
        }
        let stale = page
            .as_ref()
            .is_some_and(|p| current.saturating_sub(p.fetched_at) >= TTL);
        Ok(
            json!({"source":SOURCE,"sourceName":"GitHub MCP Registry","persistentCache":self.store.is_some(),"query":query,"page":page.as_ref().map(listing_page),"stale":stale,"error":error,"retryAt":state.cache.retry_at,"network":"Only explicit searches fetch public registry metadata. Server endpoints are not contacted.","coverage":"A curated, partial directory; search matches the upstream index. Registry metadata and popularity do not establish trust."}),
        )
    }
    pub fn detail(&self, key: &str) -> Result<Value, DomainError> {
        let state = self.state.lock().map_err(|_| DomainError::Conflict)?;
        let (entry, fetched) = find(&state, key)?;
        Ok(
            json!({"key":entry.key,"iconUrl":entry.icon_url,"server":entry.server,"registryMeta":entry.registry_meta,"sha256":entry.digest,"source":SOURCE,"fetchedAt":fetched,"stale":now()?.saturating_sub(fetched)>=TTL,"setup":recipes(&entry.server),"capabilities":"Unverified registry claims only. Connect after review to discover actual tools, resources and prompts."}),
        )
    }
    /// Revalidate a registry-derived launch against this exact version before
    /// presenting activation. An outage or changed/withdrawn entry cannot grant
    /// new execution through a stale cache. Existing manual records still work.
    pub fn revalidate(&self, provenance: &Provenance) -> Result<(), DomainError> {
        let Provenance::Registry {
            registry,
            name,
            version,
            sha256,
            ..
        } = provenance
        else {
            return Ok(());
        };
        if registry != SOURCE {
            return Err(DomainError::PermissionDenied);
        }
        let mut state = self.state.lock().map_err(|_| DomainError::Conflict)?;
        let current = now()?;
        state.requests.retain(|t| current.saturating_sub(*t) < 60);
        if current < state.cache.retry_at || state.requests.len() >= 5 {
            return Err(DomainError::Unavailable(
                "Discovery source is rate limited. Retry the reviewed setup or connection later."
                    .into(),
            ));
        }
        state.requests.push_back(current);
        let mut url = reqwest::Url::parse(SOURCE).map_err(|_| invalid("registry source"))?;
        url.path_segments_mut()
            .map_err(|_| DomainError::Conflict)?
            .extend(["v0.1", "servers", name, "versions", version]);
        let (bytes, retry) = (self.reader)(url)?;
        state.cache.retry_at = retry;
        self.save(&mut state)?;
        if bytes.is_empty() {
            return Err(DomainError::Unavailable(
                "Discovery source is rate limited. Retry later.".into(),
            ));
        }
        let value = decode_json(&bytes, Limits::default())?;
        let page = parse_page(
            &serde_json::to_vec(&json!({"servers":[value],"metadata":{}}))
                .map_err(|_| invalid("registry record"))?,
            "",
            "",
            current,
        )?;
        let entry = page.entries.first().ok_or(DomainError::NotFound)?;
        if &entry.digest != sha256
            || entry.server["name"] != name.as_str()
            || entry.server["version"] != version.as_str()
            || entry.registry_meta["io.modelcontextprotocol.registry/official"]["status"]
                != "active"
        {
            return Err(DomainError::Unavailable("The registry listing changed or is no longer active. Refresh Discover and review a new configuration.".into()));
        }
        Ok(())
    }
    pub fn local_draft(
        &self,
        key: &str,
        digest: &str,
        index: usize,
        configuration: Configuration,
    ) -> Result<Draft, DomainError> {
        let state = self.state.lock().map_err(|_| DomainError::Conflict)?;
        let (entry, fetched) = find(&state, key)?;
        if entry.digest != digest || now()?.saturating_sub(fetched) >= TTL {
            return Err(DomainError::Conflict);
        }
        if entry.registry_meta["io.modelcontextprotocol.registry/official"]["status"] != "active" {
            return Err(DomainError::Unavailable(
                "This listing is not active.".into(),
            ));
        }
        let package = entry
            .server
            .get("packages")
            .and_then(Value::as_array)
            .and_then(|v| v.get(index))
            .ok_or(DomainError::NotFound)?;
        if package
            .get("transport")
            .and_then(|v| v.get("type"))
            .and_then(Value::as_str)
            != Some("stdio")
        {
            return Err(DomainError::Unavailable(
                "Use manual setup for this package transport.".into(),
            ));
        }
        let Configuration::Local {
            executable,
            arguments,
            cwd,
            public_environment,
            ..
        } = configuration
        else {
            return Err(DomainError::PermissionDenied);
        };
        let mut secret_environment = std::collections::BTreeSet::new();
        let variables = package
            .get("environmentVariables")
            .and_then(Value::as_array);
        if variables.is_some_and(|v| v.len() > 32) {
            return Err(invalid("package environment bounds"));
        }
        for variable in variables.into_iter().flatten() {
            let name = variable
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("package environment name"))?;
            if variable.get("isSecret").and_then(Value::as_bool) == Some(true) {
                secret_environment.insert(name.into());
            } else if variable.get("isRequired").and_then(Value::as_bool) == Some(true)
                && !public_environment.contains_key(name)
            {
                return Err(DomainError::Unavailable(format!(
                    "Supply the required public environment value {name} in the setup form."
                )));
            }
        }
        let draft = Draft {
            name: entry
                .server
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or(entry.server["name"].as_str().unwrap_or("MCP server"))
                .into(),
            configuration: Configuration::Local {
                executable,
                arguments,
                cwd,
                public_environment,
                secret_environment,
            },
            protocol: super::Revision::Current20260728.as_str().into(),
            auth_profile: None,
            provenance: Provenance::Registry {
                registry: SOURCE.into(),
                name: entry.server["name"].as_str().unwrap_or_default().into(),
                version: entry.server["version"].as_str().unwrap_or_default().into(),
                sha256: entry.digest.clone(),
                retrieved_at: fetched,
            },
        };
        draft.validate()?;
        super::definitions::LocalAccess::ReadOnlyDirectory.validate(&draft.configuration)?;
        Ok(draft)
    }
    pub fn remote_draft(
        &self,
        key: &str,
        digest: &str,
        index: usize,
    ) -> Result<Draft, DomainError> {
        let state = self.state.lock().map_err(|_| DomainError::Conflict)?;
        let (entry, fetched) = find(&state, key)?;
        if entry.digest != digest || now()?.saturating_sub(fetched) >= TTL {
            return Err(DomainError::Conflict);
        }
        let official = entry
            .registry_meta
            .get("io.modelcontextprotocol.registry/official");
        if official
            .and_then(|v| v.get("status"))
            .and_then(Value::as_str)
            != Some("active")
        {
            return Err(DomainError::Unavailable("This listing is unavailable, withdrawn or has unknown status. Review the publisher documentation.".into()));
        }
        let remote = entry
            .server
            .get("remotes")
            .and_then(Value::as_array)
            .and_then(|v| v.get(index))
            .ok_or(DomainError::NotFound)?;
        let url = remote_url(remote)?;
        let draft = Draft {
            name: entry
                .server
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or(entry.server["name"].as_str().unwrap_or("MCP server"))
                .into(),
            configuration: Configuration::Http {
                url,
                loopback_development: false,
            },
            protocol: super::Revision::Current20260728.as_str().into(),
            auth_profile: None,
            provenance: Provenance::Registry {
                registry: SOURCE.into(),
                name: entry.server["name"].as_str().unwrap_or_default().into(),
                version: entry.server["version"].as_str().unwrap_or_default().into(),
                sha256: entry.digest.clone(),
                retrieved_at: fetched,
            },
        };
        draft.validate()?;
        Ok(draft)
    }
}
// Directory rows cross the 48 KB client bridge; full configuration stays in
// the native cache for detail and version-bound setup. Bound by UTF-8 bytes,
// including hostile JSON escaping, rather than by the upstream page count.
fn preview(value: &Value, max: usize) -> String {
    let text = value.as_str().unwrap_or_default();
    let mut out = String::new();
    let mut bytes = 0;
    for ch in text.chars() {
        let size = match ch {
            '"' | '\\' => 2,
            '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
            c if c.is_control() && (c as u32) < 32 => 6,
            c => c.len_utf8(),
        };
        if bytes + size > max.saturating_sub(3) {
            out.push('…');
            break;
        }
        out.push(ch);
        bytes += size;
    }
    out
}
fn listing_page(page: &Page) -> Value {
    let entries:Vec<_>=page.entries.iter().map(|e|{
        let remotes=e.server["remotes"].as_array();let packages=e.server["packages"].as_array();
        let mut transports=std::collections::BTreeSet::new();
        for r in remotes.into_iter().flatten(){transports.insert(preview(&r["type"],32));}
        for p in packages.into_iter().flatten(){transports.insert(preview(&p["transport"]["type"],32));}
        let credentials=remotes.into_iter().flatten().flat_map(|r|r["headers"].as_array().into_iter().flatten())
            .chain(packages.into_iter().flatten().flat_map(|p|p["environmentVariables"].as_array().into_iter().flatten())).any(|v|v["isSecret"]==true);
        let official=&e.registry_meta["io.modelcontextprotocol.registry/official"];
        json!({"key":e.key,"iconUrl":e.icon_url,"server":{"name":preview(&e.server["name"],256),"title":preview(&e.server["title"],256),"version":preview(&e.server["version"],256),"description":preview(&e.server["description"],1024)},
            "summary":{"local":packages.is_some_and(|p|!p.is_empty()),"remote":remotes.is_some_and(|r|!r.is_empty()),"credentials":credentials,"transport":transports.into_iter().filter(|s|!s.is_empty()).take(4).collect::<Vec<_>>().join(" · ")},
            "registryMeta":{"io.modelcontextprotocol.registry/official":{"status":preview(&official["status"],32),"publishedAt":preview(&official["publishedAt"],64)}}})
    }).collect();
    json!({"iconVersion":page.icon_version,"query":page.query,"cursor":page.cursor,"fetchedAt":page.fetched_at,"entries":entries,"nextCursor":page.next_cursor,"excluded":page.excluded})
}
fn find<'a>(state: &'a State, key: &str) -> Result<(&'a Entry, u64), DomainError> {
    if key.len() != 64 {
        return Err(DomainError::NotFound);
    }
    state
        .cache
        .pages
        .iter()
        .rev()
        .find_map(|p| {
            p.entries
                .iter()
                .find(|e| e.key == key)
                .map(|e| (e, p.fetched_at))
        })
        .ok_or(DomainError::NotFound)
}
fn parse_page(
    bytes: &[u8],
    query: &str,
    cursor: &str,
    fetched_at: u64,
) -> Result<Page, DomainError> {
    let value = decode_json(bytes, Limits::default())?;
    let rows = value
        .get("servers")
        .and_then(Value::as_array)
        .filter(|r| r.len() <= 10)
        .ok_or_else(|| invalid("registry page rows"))?;
    let mut entries = Vec::new();
    let mut excluded = 0;
    for row in rows {
        let Some(server) = row.get("server").and_then(Value::as_object) else {
            excluded += 1;
            continue;
        };
        if !["name", "version"].iter().all(|k| {
            server
                .get(*k)
                .and_then(Value::as_str)
                .is_some_and(|v| super::definitions::label(v, 256))
        }) {
            excluded += 1;
            continue;
        }
        if server.get("title").is_some_and(|v| {
            !v.as_str()
                .is_some_and(|v| super::definitions::label(v, 256))
        }) || server
            .get("description")
            .is_some_and(|v| !v.as_str().is_some_and(|v| v.len() <= 8192))
        {
            excluded += 1;
            continue;
        }
        // Retain configuration/provenance fields exactly; bulky publisher README,
        // icons and executable HTML are never fetched or displayed as active content.
        let icon_url = icons::source(server);
        let mut server = server.clone();
        server.remove("_meta");
        server.remove("icons");
        let registry_meta = row.get("_meta").cloned().unwrap_or(json!({}));
        let raw = serde_json::to_vec(&json!({"server":server,"registryMeta":registry_meta}))
            .map_err(|_| invalid("registry entry"))?;
        if raw.len() > 64 * 1024 {
            excluded += 1;
            continue;
        }
        let digest = ag1_storage::digest(&raw);
        entries.push(Entry {
            key: ag1_storage::digest(
                format!("{SOURCE}:{}:{}", server["name"], server["version"]).as_bytes(),
            ),
            server: Value::Object(server),
            registry_meta,
            digest,
            icon_url,
        });
    }
    let next_cursor = match value.get("metadata").and_then(|v| v.get("nextCursor")) {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.is_empty() => None,
        Some(Value::String(s))
            if s.len() <= 4096 && !s.chars().any(char::is_control) && s != cursor =>
        {
            Some(s.clone())
        }
        _ => return Err(invalid("registry cursor")),
    };
    Ok(Page {
        icon_version: 1,
        query: query.into(),
        cursor: cursor.into(),
        fetched_at,
        entries,
        next_cursor,
        excluded,
    })
}
fn remote_url(remote: &Value) -> Result<String, DomainError> {
    if remote.get("type").and_then(Value::as_str) != Some("streamable-http")
        || remote
            .get("headers")
            .is_some_and(|v| v.as_array().is_none_or(|a| !a.is_empty()))
        || remote
            .get("variables")
            .is_some_and(|v| v.as_object().is_none_or(|o| !o.is_empty()))
    {
        return Err(DomainError::Unavailable("This remote needs unsupported transport, custom headers or URL variables. Use manual setup and native private entry.".into()));
    }
    let url = remote
        .get("url")
        .and_then(Value::as_str)
        .filter(|s| !s.contains(['{', '}']))
        .ok_or_else(|| invalid("registry remote URL"))?;
    super::http::Endpoint::public_https(url)?;
    Ok(url.into())
}
fn recipes(server: &Value) -> Value {
    let remote:Vec<_>=server.get("remotes").and_then(Value::as_array).into_iter().flatten().take(16).enumerate().map(|(index,r)|{let supported=remote_url(r).is_ok();json!({"kind":"remote","index":index,"target":r,"supported":supported,"guidance":if supported{"Prefill this exact HTTPS endpoint. Save inactive, then configure authentication, review connection and enable in the desired scope."}else{"Use manual setup: this transport, custom header or URL template is not supported by the guided recipe."}})}).collect();
    let local:Vec<_>=server.get("packages").and_then(Value::as_array).into_iter().flatten().take(16).enumerate().map(|(index,p)|json!({"kind":"local","index":index,"package":p,"supported":p.get("transport").and_then(|v|v.get("type")).and_then(Value::as_str)==Some("stdio"),"guidance":"Package acquisition is not automatic. Install this exact package/version using its publisher instructions, then choose Add server → Local with the absolute installed executable, literal arguments and reviewed working directory. Enter private values through native private entry."})).collect();
    json!({"remote":remote,"local":local})
}
