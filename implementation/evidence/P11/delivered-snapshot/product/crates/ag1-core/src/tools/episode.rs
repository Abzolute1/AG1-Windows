//! Episode tool presentation and completion gates. Index prose is never evidence.
use crate::{
    DomainError,
    context::{
        ContextArtifactPort,
        episode::{self, EpisodeRecord},
        episode_retrieval,
    },
    evidence::Freshness,
    task::{TaskState, passing_check},
};
use serde_json::{Value, json};
fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn rows(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn join(v: &Value) -> String {
    rows(v)
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}
fn result(text: String, details: Value) -> Value {
    json!({"content":[{"type":"text","text":text}],"details":details})
}
pub fn completion_gate(
    entries: &[Value],
    args: &Value,
    artifacts: &impl ContextArtifactPort,
    mut validate: impl FnMut(&str) -> Freshness,
) -> Result<(), DomainError> {
    if args["outcome"] != "completed" {
        return Ok(());
    }
    let task = entries
        .iter()
        .rev()
        .find(|e| e["type"] == "custom" && e["customType"] == "task_state")
        .map(|e| TaskState::parse(&e["data"]))
        .transpose()?;
    let Some(task) = task else {
        return Ok(());
    };
    if task.value()["blockerId"]
        .as_str()
        .is_some_and(|s| !s.is_empty())
    {
        return Err(DomainError::InvalidInput("Resolve the current task blocker before closing a completed episode; use partial for unfinished work.".into()));
    }
    let (_, start) = episode::find_open(entries)
        .ok_or_else(|| DomainError::InvalidInput("no open episode".into()))?;
    let supported = rows(&args["verificationSources"]).iter().any(|source| {
        let Some(reference) = source.as_str() else {
            return false;
        };
        if reference.starts_with("file:") {
            validate(reference) == Freshness::Current
        } else if reference
            .strip_prefix("artifact:")
            .unwrap_or(reference)
            .starts_with("art_")
        {
            artifacts
                .get(reference.strip_prefix("artifact:").unwrap_or(reference))
                .is_ok()
        } else {
            passing_check(std::slice::from_ref(source), &entries[start..])
        }
    });
    if !supported {
        return Err(DomainError::InvalidInput("Completed episode requires retrievable verificationSources (current file revision, exact artifact, or successful check result); otherwise close as partial.".into()));
    }
    Ok(())
}
pub fn format_record(record: &EpisodeRecord, health: &Value) -> String {
    let r = record.value();
    let mut lines = vec![
        format!(
            "EPISODE {} · {} · source_status={}",
            s(r, "id"),
            s(r, "outcome"),
            s(health, "status")
        ),
        format!("goal: {}", s(r, "goal")),
    ];
    if let Some(title) = r["title"].as_str() {
        lines.push(format!("title: {title}"));
    }
    lines.extend([
        format!(
            "synopsis (MODEL-AUTHORED INDEX; NOT EVIDENCE): {}",
            s(r, "synopsis")
        ),
        format!(
            "trajectory: {} ({}, {} session entries)",
            s(r, "trajectoryArtifactId"),
            s(health, "trajectoryArtifact"),
            r["entryCount"]
        ),
        format!("range: {}..{}", s(r, "startEntryId"), s(r, "endEntryId")),
    ]);
    for (key, label) in [
        ("fileRevisions", "file_revisions:"),
        ("verificationSources", "verification_sources:"),
    ] {
        if !rows(&r[key]).is_empty() {
            lines.push(label.into());
            lines.extend(
                rows(&r[key])
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|s| format!("- {s}")),
            );
        }
    }
    if !rows(&r["claims"]).is_empty() {
        lines.push("claims:".into());
        for claim in rows(&r["claims"]) {
            let confidence = claim["confidence"]
                .as_str()
                .map(|s| format!(" confidence={s}"))
                .unwrap_or_default();
            let sources = join(&claim["sources"]);
            lines.extend([
                format!(
                    "- [{}{}] {}",
                    s(claim, "kind"),
                    confidence,
                    s(claim, "statement")
                ),
                format!(
                    "  sources: {}",
                    if sources.is_empty() {
                        "(none; normative/uncertain claim only)"
                    } else {
                        &sources
                    }
                ),
            ]);
        }
    }
    if rows(&health["sources"])
        .iter()
        .any(|row| row["status"] != "current")
    {
        lines.push("source_validation:".into());
        lines.extend(
            rows(&health["sources"])
                .iter()
                .map(|row| format!("- {}: {}", s(row, "status"), s(row, "source"))),
        );
    }
    lines.push("Rehydrate exact evidence with artifact grep/read before relying on this synopsis for a material code change.".into());
    lines.join("\n")
}
pub fn closed_result(record: &EpisodeRecord, health: &Value) -> Value {
    let r = record.value();
    result(
        format_record(record, health),
        json!({"action":"close","id":r["id"],"status":r["outcome"],"artifactId":r["trajectoryArtifactId"]}),
    )
}
pub fn begin_result(record: &EpisodeRecord) -> Value {
    let r = record.value();
    result(
        format!(
            "Opened {}: {}\nExact active-branch entries will be preserved when this episode is closed.",
            s(r, "id"),
            s(r, "goal")
        ),
        json!({"action":"begin","id":r["id"],"status":"open"}),
    )
}
pub fn query(
    entries: &[Value],
    args: &Value,
    artifacts: &impl ContextArtifactPort,
    mut validate: impl FnMut(&str) -> Freshness,
) -> Result<Value, DomainError> {
    let limit = args["limit"].as_u64().unwrap_or(20) as usize;
    match s(args, "action") {
        "list" => {
            let open = episode::find_open(entries);
            let closed = episode::list_closed(entries, limit);
            let mut lines = vec![];
            let mut details = json!({"action":"list","count":closed.len()});
            if let Some((record, _)) = open {
                let r = record.value();
                lines.push(format!("OPEN {} · {}", s(r, "id"), s(r, "goal")));
                details["id"] = r["id"].clone();
            }
            for record in closed {
                let r = record.value();
                lines.push(format!(
                    "{} · {} · {} entries · {} · {}",
                    s(r, "id"),
                    s(r, "outcome"),
                    r["entryCount"],
                    s(r, "trajectoryArtifactId"),
                    r["title"].as_str().unwrap_or(s(r, "goal"))
                ));
            }
            if lines.is_empty() {
                lines.push("No episodes on the active branch.".into());
            }
            Ok(result(lines.join("\n"), details))
        }
        "show" => {
            let id = s(args, "id");
            if id.is_empty() {
                return Err(DomainError::InvalidInput(
                    "episode show requires id.".into(),
                ));
            }
            if let Some(record) = entries
                .iter()
                .rev()
                .filter_map(episode::entry_record)
                .find(|r| r.value()["event"] == "closed" && r.value()["id"] == id)
            {
                let health = episode::health(&record, artifacts, &mut validate);
                return Ok(result(
                    format_record(&record, &health),
                    json!({"action":"show","id":id,"status":health["status"],"artifactId":record.value()["trajectoryArtifactId"]}),
                ));
            }
            if let Some((record, start)) =
                episode::find_open(entries).filter(|(r, _)| r.value()["id"] == id)
            {
                let r = record.value();
                let mut lines = vec![
                    format!("OPEN EPISODE {id}"),
                    format!("goal: {}", s(r, "goal")),
                ];
                if let Some(title) = r["title"].as_str() {
                    lines.push(format!("title: {title}"));
                }
                lines.extend([format!("opened: {}",s(r,"createdAt")),format!("active_branch_entries: {}",entries.len()-start),format!("start_entry: {}",s(&entries[start],"id")),"This episode is still collecting the active-branch trajectory. Close it before treating it as completed memory.".into()]);
                return Ok(result(
                    lines.join("\n"),
                    json!({"action":"show","id":id,"status":"open"}),
                ));
            }
            Err(DomainError::InvalidInput(format!("Unknown episode: {id}")))
        }
        "search" => {
            let found = episode_retrieval::search(
                entries,
                s(args, "query"),
                args["limit"].as_u64().unwrap_or(5) as usize,
                artifacts,
                validate,
            )?;
            let matches = rows(&found["matches"]);
            let conflicts = rows(&found["conflicts"]);
            let mut lines = vec![
                "EPISODE SEARCH RESULTS (MODEL-AUTHORED INDEXES; NOT EVIDENCE)".into(),
                format!("query: {}", s(&found, "query")),
                format!(
                    "scanned: {}; matches: {}; conflicts: {}",
                    found["scanned"],
                    matches.len(),
                    conflicts.len()
                ),
            ];
            for m in matches {
                let r = &m["record"];
                lines.extend([
                    format!(
                        "\n{} · score={:.2} · outcome={} · source_status={}",
                        s(r, "id"),
                        m["score"].as_f64().unwrap_or(0.),
                        s(r, "outcome"),
                        s(&m["health"], "status")
                    ),
                    format!("goal: {}", s(r, "goal")),
                    format!("synopsis (index only): {}", s(r, "synopsis")),
                    format!("trajectory: {}", s(r, "trajectoryArtifactId")),
                    format!("reasons: {}", join(&m["reasons"])),
                ]);
                if !rows(&m["matchingClaims"]).is_empty() {
                    lines.push("matching claims (not evidence):".into());
                }
                for claim in rows(&m["matchingClaims"]) {
                    let refs = join(&claim["sources"]);
                    lines.push(format!(
                        "- [{}] {} · sources={}",
                        s(claim, "kind"),
                        s(claim, "statement"),
                        if refs.is_empty() { "none" } else { &refs }
                    ));
                }
            }
            if !conflicts.is_empty() {
                lines.push(
                    "\nCONFLICTS: do not choose a claim from recency or synopsis alone.".into(),
                );
            }
            for conflict in conflicts {
                lines.push(format!(
                    "- {}: {}",
                    s(conflict, "key"),
                    join(&conflict["episodes"])
                ));
                for item in rows(&conflict["statements"]) {
                    let refs = join(&item["sources"]);
                    lines.push(format!(
                        "  - {}: {} · sources={}",
                        s(item, "episodeId"),
                        s(item, "statement"),
                        if refs.is_empty() { "none" } else { &refs }
                    ));
                }
            }
            lines.push("Rehydrate exact artifacts and inspect current source before using an episode claim for a material code change.".into());
            Ok(result(
                lines.join("\n"),
                json!({"action":"search","count":matches.len(),"status":if conflicts.is_empty(){"indexed".into()}else{format!("conflicts:{}",conflicts.len())}}),
            ))
        }
        _ => Err(DomainError::InvalidInput("episode query action".into())),
    }
}
fn number(n: &Value) -> String {
    let raw = n.as_u64().unwrap_or(0).to_string();
    raw.chars()
        .enumerate()
        .flat_map(|(i, c)| {
            let mut s = String::new();
            if i > 0 && (raw.len() - i).is_multiple_of(3) {
                s.push(',');
            }
            s.push(c);
            s.chars().collect::<Vec<_>>()
        })
        .collect()
}
pub fn draft_result(candidate: &Value, input: u64, output: u64) -> Value {
    let mut candidate = candidate.clone();
    candidate["inputTokens"] = json!(input);
    candidate["outputTokens"] = json!(output);
    let text = ["EPISODE CANDIDATE (MODEL-AUTHORED; NOT PERSISTED MEMORY; NOT EVIDENCE)".into(),
        format!("exact_trajectory: {} ({} entries)",s(&candidate,"trajectoryArtifactId"),candidate["entryCount"]),
        format!("analysis_coverage: {} ({} selected, {} omitted)",s(&candidate,"analysisCoverage"),candidate["selectedEntryCount"],candidate["omittedEntryCount"]),
        format!("candidate_output: {}",s(&candidate,"candidateArtifactId")),
        format!("analysis: {} chars, {} input tokens, {} output tokens, {} large blocks externalized",number(&candidate["analysisChars"]),number(&candidate["inputTokens"]),number(&candidate["outputTokens"]),candidate["externalizedBlocks"]),
        serde_json::to_string_pretty(&candidate["candidate"]).unwrap(),
        "Review this candidate against exact evidence, then explicitly call episode close with only the fields you accept. Nothing above was appended as durable episode memory.".into()].join("\n");
    result(
        text,
        json!({"action":"draft","status":"candidate-only","artifactId":candidate["trajectoryArtifactId"],"candidateArtifactId":candidate["candidateArtifactId"],"inputTokens":input,"outputTokens":output}),
    )
}
