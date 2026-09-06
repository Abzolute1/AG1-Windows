use super::{
    ContextArtifactPort,
    episode::{EpisodeRecord, closed_records, health},
    slice_utf16, string, tail_utf16,
};
use crate::{
    DomainError,
    evidence::Freshness,
    task::{invalid, text, units},
};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    sync::LazyLock,
};
static KEYS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?-u:\b)(?:DEC|POLICY|INV|CONSTRAINT|VERIFY|HYP|FAIL|RESULT)-[A-Z0-9_-]+(?-u:\b)",
    )
    .unwrap()
});
fn normalize(s: &str) -> String {
    text(&s.to_lowercase(), usize::MAX)
}
fn stable_keys(s: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    KEYS.find_iter(s)
        .map(|m| slice_utf16(&m.as_str().to_uppercase(), 80))
        .filter(|s| seen.insert(s.clone()))
        .collect()
}
fn terms(s: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let all: Vec<_> = normalize(s)
        .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || "_.:/@-".contains(c)))
        .filter(|s| s.len() >= 2)
        .map(str::to_string)
        .filter(|s| seen.insert(s.clone()))
        .collect();
    if all.len() <= 48 {
        all
    } else {
        all[..24]
            .iter()
            .chain(&all[all.len() - 24..])
            .cloned()
            .collect()
    }
}
fn bounded_query(raw: &str) -> String {
    if units(raw) <= 600 {
        return raw.into();
    }
    let stable: Vec<_> = stable_keys(raw).into_iter().take(4).collect();
    let reserved = if stable.is_empty() {
        String::new()
    } else {
        format!(" [stable_ids: {}]", stable.join(","))
    };
    let available = 600 - units(&reserved) - 5;
    let head = (available / 2).max(1);
    slice_utf16(
        &format!(
            "{} ... {}{}",
            slice_utf16(raw, head),
            tail_utf16(raw, (available - head).max(1)),
            reserved
        ),
        600,
    )
}
fn grams(s: &str) -> HashSet<Vec<u16>> {
    let chars: Vec<_> = s.encode_utf16().collect();
    if chars.is_empty() {
        return HashSet::new();
    }
    chars
        .windows(chars.len().min(3))
        .map(<[u16]>::to_vec)
        .collect()
}
fn score(record: &EpisodeRecord, query: &str, terms: &[String]) -> (f64, Vec<String>, Vec<Value>) {
    let r = record.value();
    let phrase = normalize(query);
    let goal = normalize(&format!("{} {}", string(r, "title"), string(r, "goal")));
    let synopsis = normalize(string(r, "synopsis"));
    let claims = r["claims"].as_array().unwrap();
    let claim_text: Vec<_> = claims
        .iter()
        .map(|c| normalize(string(c, "statement")))
        .collect();
    let mut score = 0.0;
    let mut reasons = vec![];
    for (present, weight, reason) in [
        (
            units(&phrase) >= 3 && goal.contains(&phrase),
            40.,
            "goal-phrase",
        ),
        (
            units(&phrase) >= 3 && claim_text.iter().any(|c| c.contains(&phrase)),
            30.,
            "claim-phrase",
        ),
        (
            units(&phrase) >= 3 && synopsis.contains(&phrase),
            12.,
            "synopsis-phrase",
        ),
    ] {
        if present {
            score += weight;
            reasons.push(reason.into());
        }
    }
    for (count, weight, label) in [
        (
            terms.iter().filter(|t| goal.contains(t.as_str())).count(),
            7.,
            "goal-terms",
        ),
        (
            terms
                .iter()
                .filter(|t| claim_text.iter().any(|c| c.contains(t.as_str())))
                .count(),
            5.,
            "claim-terms",
        ),
        (
            terms
                .iter()
                .filter(|t| synopsis.contains(t.as_str()))
                .count(),
            2.,
            "synopsis-terms",
        ),
    ] {
        if count > 0 {
            score += count as f64 * weight;
            reasons.push(format!("{label}:{count}"));
        }
    }
    let matching = claims
        .iter()
        .zip(claim_text)
        .filter(|(_, c)| {
            (units(&phrase) >= 3 && c.contains(&phrase)) || terms.iter().any(|t| c.contains(t))
        })
        .map(|(c, _)| c.clone())
        .collect();
    (score, reasons, matching)
}
fn conflicts(matches: &[Value]) -> Vec<Value> {
    let mut by_key: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for m in matches {
        for claim in m["record"]["claims"].as_array().unwrap() {
            for key in stable_keys(string(claim, "statement")) {
                by_key.entry(key).or_default().push(json!({"episodeId":m["record"]["id"],"statement":claim["statement"],"sources":claim["sources"]}));
            }
        }
    }
    by_key
        .into_iter()
        .filter_map(|(key, values)| {
            let statements: BTreeSet<_> = values
                .iter()
                .map(|v| normalize(string(v, "statement")))
                .collect();
            let mut seen = BTreeSet::new();
            let episodes: Vec<_> = values
                .iter()
                .map(|v| string(v, "episodeId"))
                .filter(|s| seen.insert(*s))
                .collect();
            if statements.len() < 2 || episodes.len() < 2 {
                None
            } else {
                Some(json!({"key":key,"episodes":episodes,"statements":values}))
            }
        })
        .collect()
}
fn rank(a: &Value, b: &Value) -> std::cmp::Ordering {
    b["score"]
        .as_f64()
        .unwrap()
        .total_cmp(&a["score"].as_f64().unwrap())
        .then_with(|| string(&b["record"], "closedAt").cmp(string(&a["record"], "closedAt")))
}
pub fn search(
    entries: &[Value],
    query: &str,
    limit: usize,
    store: &impl ContextArtifactPort,
    mut validate: impl FnMut(&str) -> Freshness,
) -> Result<Value, DomainError> {
    if entries.len() > 100000 || query.len() > 1024 * 1024 {
        return Err(invalid("episode search input limit"));
    }
    let raw = text(query, usize::MAX);
    if raw.is_empty() {
        return Err(invalid("empty episode query"));
    }
    let query = bounded_query(&raw);
    let terms = terms(&format!("{} {}", query, stable_keys(&raw).join(" ")));
    if terms.is_empty() {
        return Err(invalid("episode query has no searchable terms"));
    }
    let limit = limit.clamp(1, 10);
    let records = closed_records(entries);
    let scanned = records.len();
    let query_grams: Vec<_> = terms.iter().map(|t| grams(t)).collect();
    let mut pool = vec![];
    for (recency, record) in records.into_iter().enumerate() {
        let r = record.value();
        let content = normalize(&format!(
            "{} {} {} {}",
            string(r, "title"),
            string(r, "goal"),
            string(r, "synopsis"),
            r["claims"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| string(c, "statement"))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        let index = grams(&content);
        if !query_grams
            .iter()
            .any(|g| !g.is_empty() && g.is_subset(&index))
        {
            continue;
        }
        let (base, reasons, matching) = score(&record, &query, &terms);
        if base > 0. && !reasons.is_empty() {
            pool.push(json!({"record":r,"score":base+(2.-recency as f64*0.01).max(0.),"reasons":reasons,"matchingClaims":matching}));
        }
    }
    pool.sort_by(rank);
    pool.truncate(10.max(limit * 3));
    for m in &mut pool {
        let record = EpisodeRecord::parse(&m["record"])?;
        let health = health(&record, store, &mut validate);
        let adjustment = match string(&health, "status") {
            "current" => 3.,
            "unverified" => 0.,
            _ => -3.,
        };
        let score = m["score"].as_f64().unwrap() + adjustment;
        m["score"] = if score.fract() == 0.0 {
            json!(score as i64)
        } else {
            json!(score)
        };
        m["reasons"]
            .as_array_mut()
            .unwrap()
            .push(format!("source-status:{}", string(&health, "status")).into());
        m["health"] = health;
    }
    pool.sort_by(rank);
    pool.truncate(limit);
    Ok(json!({"query":query,"conflicts":conflicts(&pool),"matches":pool,"scanned":scanned}))
}
/// Automatic injection always uses the contracted maximum of three pointers and 2,400 UTF-16 units.
pub fn addendum(
    entries: &[Value],
    query: &str,
    store: &impl ContextArtifactPort,
    validate: impl FnMut(&str) -> Freshness,
) -> Result<Option<String>, DomainError> {
    let result = search(entries, query, 3, store, validate)?;
    let query = string(&result, "query");
    let stable: BTreeSet<_> = stable_keys(query).into_iter().collect();
    if stable.is_empty() && terms(query).len() < 2 {
        return Ok(None);
    }
    let selected: Vec<_> = result["matches"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| {
            m["score"].as_f64().unwrap() >= 20.
                || m["record"]["claims"].as_array().unwrap().iter().any(|c| {
                    stable_keys(string(c, "statement"))
                        .iter()
                        .any(|k| stable.contains(k))
                })
        })
        .collect();
    if selected.is_empty() {
        return Ok(None);
    }
    let selected_ids: BTreeSet<_> = selected
        .iter()
        .map(|m| string(&m["record"], "id"))
        .collect();
    let conflicts: Vec<_> = result["conflicts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| {
            c["episodes"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .any(|id| selected_ids.contains(id))
        })
        .collect();
    let mut lines = vec![
        "AUTO-RETRIEVED EPISODE POINTERS (MODEL-AUTHORED INDEXES; NOT EVIDENCE)".into(),
        format!("query: {query}"),
        "Reason loaded: strong lexical or stable-identifier overlap with the current user request."
            .into(),
    ];
    for m in selected {
        let r = &m["record"];
        let mut block = vec![
            format!(
                "- {} outcome={} source_status={} score={:.2}",
                string(r, "id"),
                string(r, "outcome"),
                string(&m["health"], "status"),
                m["score"].as_f64().unwrap()
            ),
            format!("  goal: {}", slice_utf16(string(r, "goal"), 300)),
            format!("  exact_trajectory: {}", string(r, "trajectoryArtifactId")),
        ];
        for c in m["matchingClaims"].as_array().unwrap().iter().take(2) {
            let sources = c["sources"]
                .as_array()
                .unwrap()
                .iter()
                .take(3)
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(",");
            block.push(format!(
                "  index_claim_not_evidence: [{}] {} sources={}",
                string(c, "kind"),
                slice_utf16(string(c, "statement"), 300),
                if sources.is_empty() { "none" } else { &sources }
            ));
        }
        if units(
            &lines
                .iter()
                .chain(&block)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"),
        ) > 2160
        {
            break;
        }
        lines.extend(block);
    }
    if lines.len() == 3 {
        return Ok(None);
    }
    if !conflicts.is_empty() {
        lines.push(
            "CONFLICT: matched episode indexes disagree; do not choose by recency or synopsis."
                .into(),
        );
        for c in conflicts.into_iter().take(3) {
            let line = format!(
                "- {}: {}",
                string(c, "key"),
                c["episodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(",")
            );
            if units(&format!("{}\n{line}", lines.join("\n"))) > 2220 {
                break;
            }
            lines.push(line);
        }
    }
    lines.push(
        "Rehydrate exact artifacts and inspect current source before a material code change."
            .into(),
    );
    Ok(Some(lines.join("\n")))
}
