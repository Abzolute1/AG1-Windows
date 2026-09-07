//! Project memory commands and deterministic bounded selection. Storage commits with CAS.
use crate::{
    DomainError,
    storage::StoragePort,
    task::{bounded_json, invalid, text, units},
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
const KINDS: &[&str] = &["decision", "style", "breakage", "fact"];
#[derive(Clone, Debug, PartialEq)]
pub struct MemoryStore(Value);
#[derive(Clone, Debug)]
pub struct MemoryChange {
    pub store: MemoryStore,
    pub entry: Value,
    pub archived: Vec<Value>,
    pub duplicate: bool,
    pub near_limit: bool,
}
fn string<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn priority(v: &Value) -> &str {
    v["priority"].as_str().unwrap_or("normal")
}
fn recency(v: &Value) -> i64 {
    v["updatedAt"]
        .as_i64()
        .or_else(|| v["createdAt"].as_i64())
        .unwrap_or(0)
}
fn normalized_sources(v: &Value) -> Result<Vec<String>, DomainError> {
    let empty = vec![];
    let values = if v.is_null() {
        &empty
    } else {
        v.as_array().ok_or_else(|| invalid("memory sources"))?
    };
    if values.len() > 8 {
        return Err(invalid("memory source cap"));
    }
    let mut out = vec![];
    for s in values {
        let s = s.as_str().ok_or_else(|| invalid("memory source"))?.trim();
        if units(s) > 500 || s.contains(['\r', '\n', '\0']) {
            return Err(invalid("single-line memory source"));
        }
        if !s.is_empty() && !out.iter().any(|v| v == s) {
            out.push(s.into());
        }
    }
    Ok(out)
}
fn validate_entry(e: &Value) -> Result<(), DomainError> {
    if !e.is_object()
        || !e["id"].is_string()
        || !e["text"].is_string()
        || !e["createdAt"].is_i64()
        || !KINDS.contains(&string(e, "kind"))
        || !["core", "normal"].contains(&priority(e))
    {
        return Err(invalid("memory entry"));
    }
    if e.get("updatedAt").is_some() && !e["updatedAt"].is_i64() {
        return Err(invalid("memory timestamp"));
    }
    if let Some(c) = e.get("component") {
        let c = c.as_str().ok_or_else(|| invalid("component"))?;
        if c.trim().is_empty() || units(c) > 100 {
            return Err(invalid("component"));
        }
    }
    let sources = normalized_sources(&e["sources"])?;
    if e.get("component").is_some()
        && ["fact", "breakage"].contains(&string(e, "kind"))
        && sources.is_empty()
    {
        return Err(invalid("component claim needs provenance"));
    }
    Ok(())
}
impl Default for MemoryStore {
    fn default() -> Self {
        Self(json!({"version":3,"entries":[]}))
    }
}
impl MemoryStore {
    pub fn value(&self) -> &Value {
        &self.0
    }
    pub fn parse(v: &Value) -> Result<Self, DomainError> {
        bounded_json(v)?;
        if ![json!(1), json!(2), json!(3)].contains(&v["version"]) {
            return Err(invalid("memory version"));
        }
        let active = v["entries"]
            .as_array()
            .ok_or_else(|| invalid("memory entries"))?;
        let empty = vec![];
        let archived = if let Some(a) = v.get("archived") {
            a.as_array().ok_or_else(|| invalid("memory archive"))?
        } else {
            &empty
        };
        if active.len() + archived.len() > 10000 {
            return Err(DomainError::LimitExceeded("memory catalog".into()));
        }
        let mut ids = BTreeSet::new();
        for e in active {
            validate_entry(e)?;
            if !ids.insert(string(e, "id")) {
                return Err(invalid("duplicate memory identity"));
            }
        }
        let mut cold = vec![];
        for e in archived {
            validate_entry(e)?;
            if ids.contains(string(e, "id")) {
                if active.iter().any(|a| a["id"] == e["id"]) {
                    continue;
                }
                return Err(invalid("duplicate archived memory"));
            }
            ids.insert(string(e, "id"));
            cold.push(e.clone());
        }
        // Unknown source fields survive migration. Invalid input is never replaced by an empty store.
        let mut out = v.clone();
        out["version"] = json!(3);
        out["entries"] = json!(active);
        if cold.is_empty() {
            out.as_object_mut().unwrap().remove("archived");
        } else {
            out["archived"] = json!(cold);
        }
        Ok(Self(out))
    }
    fn hot(&self) -> &Vec<Value> {
        self.0["entries"].as_array().unwrap()
    }
    fn cold(&self) -> Vec<Value> {
        self.0["archived"].as_array().cloned().unwrap_or_default()
    }
    pub fn all(&self) -> Vec<&Value> {
        self.hot()
            .iter()
            .chain(self.0["archived"].as_array().into_iter().flatten())
            .collect()
    }
    pub fn add(
        &self,
        kind: &str,
        raw: &str,
        now: i64,
        priority: &str,
        metadata: &Value,
    ) -> Result<MemoryChange, DomainError> {
        bounded_json(metadata)?;
        let normalized = text(raw, usize::MAX);
        if normalized.is_empty() || units(&normalized) > 400 {
            return Err(invalid("memory text must contain 1..400 UTF-16 units"));
        }
        let mut e = json!({"kind":kind,"text":normalized,"createdAt":now,"priority":priority});
        if let Some(c) = metadata.get("component") {
            let c = text(c.as_str().ok_or_else(|| invalid("component"))?, usize::MAX);
            if c.is_empty() || units(&c) > 100 {
                return Err(invalid("component"));
            }
            e["component"] = json!(c);
        }
        let sources = normalized_sources(&metadata["sources"])?;
        if !sources.is_empty() {
            e["sources"] = json!(sources);
        }
        let all = self.all();
        let mut n = all.len() + 1;
        while all.iter().any(|e| string(e, "id") == format!("m{n}")) {
            n += 1;
        }
        e["id"] = json!(format!("m{n}"));
        validate_entry(&e)?;
        if let Some(old) = all.iter().find(|old| {
            old["text"] == e["text"] && string(old, "component") == string(&e, "component")
        }) {
            return Ok(MemoryChange {
                store: self.clone(),
                entry: (*old).clone(),
                archived: vec![],
                duplicate: true,
                near_limit: false,
            });
        }
        if all.len() >= 10000 {
            return Err(DomainError::LimitExceeded("memory archive".into()));
        }
        let mut hot = self.hot().clone();
        let mut moved = vec![];
        while hot.len() >= 40 {
            let i = hot
                .iter()
                .position(|e| self::priority(e) != "core")
                .ok_or_else(|| invalid("memory full of core entries"))?;
            moved.push(hot.remove(i));
        }
        hot.push(e.clone());
        let near_limit = 40usize.saturating_sub(hot.len()) < 5;
        let mut cold = self.cold();
        cold.extend(moved.clone());
        let mut out = self.0.clone();
        out["entries"] = json!(hot);
        if !cold.is_empty() {
            out["archived"] = json!(cold);
        }
        Ok(MemoryChange {
            store: Self(out),
            entry: e,
            archived: moved,
            duplicate: false,
            near_limit,
        })
    }
    pub fn update(&self, id: &str, patch: &Value, now: i64) -> Result<MemoryChange, DomainError> {
        bounded_json(patch)?;
        let keys = patch.as_object().ok_or_else(|| invalid("memory patch"))?;
        if keys.is_empty()
            || keys.keys().any(|k| {
                !["text", "kind", "priority", "component", "sources"].contains(&k.as_str())
            })
        {
            return Err(invalid("memory mutation fields"));
        }
        let all = self.all();
        let current = all
            .iter()
            .find(|e| e["id"] == id)
            .ok_or(DomainError::NotFound)?;
        let mut e = (*current).clone();
        e["priority"] = json!(priority(&e));
        for (k, v) in keys {
            match k.as_str() {
                "text" => {
                    let t = text(
                        v.as_str().ok_or_else(|| invalid("memory text"))?,
                        usize::MAX,
                    );
                    if t.is_empty() || units(&t) > 400 {
                        return Err(invalid("memory text cap"));
                    }
                    e[k] = json!(t);
                }
                "component" => {
                    let c = text(v.as_str().ok_or_else(|| invalid("component"))?, usize::MAX);
                    if units(&c) > 100 {
                        return Err(invalid("component cap"));
                    }
                    if c.is_empty() {
                        e.as_object_mut().unwrap().remove(k);
                    } else {
                        e[k] = json!(c);
                    }
                }
                "sources" => {
                    let s = normalized_sources(v)?;
                    if s.is_empty() {
                        e.as_object_mut().unwrap().remove(k);
                    } else {
                        e[k] = json!(s);
                    }
                }
                _ => e[k] = v.clone(),
            }
        }
        validate_entry(&e)?;
        if patch.get("text").is_some()
            && all.iter().any(|a| {
                a["id"] != id
                    && a["text"] == e["text"]
                    && string(a, "component") == string(&e, "component")
            })
        {
            return Err(invalid("duplicate memory text"));
        }
        e["updatedAt"] = json!(now);
        let mut out = self.0.clone();
        for key in ["entries", "archived"] {
            if let Some(rows) = out.get_mut(key).and_then(Value::as_array_mut) {
                for row in rows {
                    if row["id"] == id {
                        *row = e.clone();
                    }
                }
            }
        }
        Ok(MemoryChange {
            store: Self(out),
            entry: e,
            archived: vec![],
            duplicate: false,
            near_limit: false,
        })
    }
    pub fn remove(&self, id: &str) -> Result<Self, DomainError> {
        if !self.all().iter().any(|e| e["id"] == id) {
            return Err(DomainError::NotFound);
        }
        let mut out = self.0.clone();
        for key in ["entries", "archived"] {
            if let Some(rows) = out.get_mut(key).and_then(Value::as_array_mut) {
                rows.retain(|e| e["id"] != id);
            }
        }
        if out["archived"].as_array().is_some_and(|a| a.is_empty()) {
            out.as_object_mut().unwrap().remove("archived");
        }
        Ok(Self(out))
    }
    pub fn select(
        &self,
        query: &str,
        max_chars: usize,
        max_entries: usize,
        validate: impl Fn(&Value) -> crate::evidence::Freshness,
    ) -> MemorySelection {
        let all = self.all();
        let mut selected = vec![];
        let mut statuses = BTreeMap::new();
        let mut used = 0;
        let scores: BTreeMap<_, _> = all
            .iter()
            .map(|e| (string(e, "id"), relevance(e, query)))
            .collect();
        let mut relevant: Vec<_> = all
            .iter()
            .copied()
            .filter(|e| priority(e) != "core" && scores[string(e, "id")] > 0)
            .collect();
        relevant.sort_by(|a, b| {
            scores[string(b, "id")]
                .cmp(&scores[string(a, "id")])
                .then_with(|| recency(b).cmp(&recency(a)))
                .then_with(|| string(a, "id").cmp(string(b, "id")))
        });
        let mut recent: Vec<_> = self
            .hot()
            .iter()
            .filter(|e| priority(e) != "core")
            .collect();
        recent.sort_by(|a, b| {
            recency(b)
                .cmp(&recency(a))
                .then_with(|| string(a, "id").cmp(string(b, "id")))
        });
        let mut add = |e: &Value| {
            let id = string(e, "id");
            if statuses.contains_key(id) || selected.len() >= max_entries.min(16) {
                return false;
            }
            let status = validate(e);
            let cost = units(&context_line(e, status)) + 2;
            // One bounded entry may exceed the soft reference character budget. Entry cap stays hard.
            if used + cost > max_chars.min(1600) && !selected.is_empty() {
                return false;
            }
            used += cost;
            selected.push(e.clone());
            statuses.insert(id.to_owned(), status);
            true
        };
        for e in all.iter().filter(|e| priority(e) == "core") {
            if !add(e) {
                break;
            }
        }
        for e in relevant {
            add(e);
        }
        for e in recent {
            if !validate(e).historical() {
                add(e);
            }
        }
        selected.sort_by(|a, b| natural_id(string(a, "id"), string(b, "id")));
        MemorySelection {
            entries: selected,
            stored_count: all.len(),
            source_statuses: statuses,
        }
    }
}
fn natural_id(a: &str, b: &str) -> std::cmp::Ordering {
    match (
        a.strip_prefix('m').and_then(|s| s.parse::<u64>().ok()),
        b.strip_prefix('m').and_then(|s| s.parse::<u64>().ok()),
    ) {
        (Some(a), Some(b)) => a.cmp(&b),
        _ => a.cmp(b),
    }
}
fn tokens(text: &str) -> Vec<String> {
    let stop = [
        "the", "and", "for", "with", "this", "that", "from", "into", "does", "did", "was", "were",
        "are", "you", "can", "could", "should", "would", "what", "when", "where", "which", "how",
        "why", "who", "please", "make", "use", "using", "used",
    ];
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric() && !"_./-".contains(c))
        .filter(|s| s.len() >= 3 && !stop.contains(s))
        .map(str::to_owned)
        .collect()
}
pub fn relevance(e: &Value, query: &str) -> usize {
    let query: BTreeSet<_> = tokens(query).into_iter().collect();
    tokens(&format!(
        "{} {} {}",
        string(e, "kind"),
        string(e, "component"),
        string(e, "text")
    ))
    .iter()
    .filter(|t| query.contains(*t))
    .count()
}
fn date(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "invalid-date".into())
}
pub fn context_line(e: &Value, status: crate::evidence::Freshness) -> String {
    let mut label = string(e, "kind").to_owned();
    if priority(e) == "core" {
        label.push_str(" core");
    }
    if let Some(c) = e["component"].as_str() {
        label += &format!(" component={c}");
    }
    let sources = e["sources"].as_array();
    if sources.is_some_and(|a| !a.is_empty()) {
        label += &format!(" source={status}");
    } else if e.get("component").is_some() {
        label += " source=unverified";
    }
    if let Some(s) = sources.and_then(|s| s.first()).and_then(Value::as_str) {
        label += &format!(" provenance={s}");
    }
    let historical = if status.historical() {
        format!(
            "HISTORICAL {} — verify original/current source before use: ",
            status.to_string().to_uppercase()
        )
    } else {
        String::new()
    };
    format!(
        "- [{label}] {} {historical}{}",
        date(recency(e)),
        string(e, "text")
    )
}
#[derive(Clone, Debug)]
pub struct MemorySelection {
    pub entries: Vec<Value>,
    pub stored_count: usize,
    pub source_statuses: BTreeMap<String, crate::evidence::Freshness>,
}
impl MemorySelection {
    pub fn value(&self) -> Value {
        if self.stored_count == 0 {
            json!({"entries":[],"storedCount":0})
        } else {
            json!({"entries":self.entries,"storedCount":self.stored_count,"sourceStatuses":self.source_statuses})
        }
    }
}
pub struct MemoryService;
impl MemoryService {
    pub fn load(
        store: &impl StoragePort,
        id: &str,
    ) -> Result<(MemoryStore, Option<String>), DomainError> {
        let Some(doc) = store.load_memory(id)? else {
            return Ok((MemoryStore::default(), None));
        };
        if doc.diagnostic.is_some() {
            return Err(invalid("corrupt memory retained; explicit repair required"));
        }
        let v = serde_json::from_slice(&doc.bytes).map_err(|_| invalid("memory JSON"))?;
        Ok((MemoryStore::parse(&v)?, Some(doc.sha256)))
    }
    pub fn mutate(
        store: &impl StoragePort,
        id: &str,
        expected: Option<&str>,
        reduce: impl FnOnce(MemoryStore) -> Result<MemoryStore, DomainError>,
    ) -> Result<String, DomainError> {
        let (memory, version) = Self::load(store, id)?;
        if version.as_deref() != expected {
            return Err(DomainError::Conflict);
        }
        let next = reduce(memory)?;
        let bytes =
            serde_json::to_vec(next.value()).map_err(|_| invalid("memory serialization"))?;
        store.commit_memory(id, expected, &bytes)
    }
}
