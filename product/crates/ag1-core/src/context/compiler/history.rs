//! Final working-budget fallback for accumulated small conversation frames.
//! Archives are custody, not new evidence occurrences: request text and copied
//! tool results must not recursively become global artifact-search matches.
use super::*;
const MARKER: &str = "ag1-history-custody";

pub(super) fn archive(
    messages: Vec<ContextMessage>,
    recent: usize,
    lease_budget: u64,
    fixed: u64,
    working_target: u64,
    target: u64,
    store: &mut impl ContextArtifactPort,
) -> Result<(Vec<ContextMessage>, usize), DomainError> {
    // Previous pruning changed positions. Recompute protection against these
    // exact frames, including both endpoints of leased tool evidence.
    let mut keep = protected(&messages, recent, false);
    leased(&messages, store, &mut keep, lease_budget)?;
    let mut calls: BTreeMap<String, (Vec<usize>, Vec<usize>)> = BTreeMap::new();
    for (i, message) in messages.iter().enumerate() {
        if message["role"] == "assistant" {
            for block in blocks(&message["content"]) {
                if block["type"] == "toolCall" {
                    calls
                        .entry(string(block, "id").into())
                        .or_default()
                        .0
                        .push(i);
                }
            }
        } else if message["role"] == "toolResult" {
            calls
                .entry(string(message, "toolCallId").into())
                .or_default()
                .1
                .push(i);
        }
    }
    let mut linked = vec![None::<(usize, usize)>; messages.len()];
    for (id, (call, result)) in calls {
        if id.is_empty() || call.len() != 1 || result.len() != 1 {
            // Pending or ambiguous protocol identities are never discarded.
            keep.extend(call.into_iter().chain(result));
            continue;
        }
        let (a, b) = (call[0].min(result[0]), call[0].max(result[0]));
        for index in [call[0], result[0]] {
            linked[index] = Some(linked[index].map_or((a, b), |(lo, hi)| (lo.min(a), hi.max(b))));
        }
    }
    let users: Vec<_> = messages
        .iter()
        .enumerate()
        .filter_map(|(i, m)| (m["role"] == "user").then_some(i))
        .collect();
    let latest_user = users.last().copied();
    // Prefer whole completed user turns, avoiding disconnected answers whose
    // original instruction has vanished. A long current task can additionally
    // externalize its old, closed tool rounds while keeping its instruction.
    let mut starts = vec![0];
    starts.extend(users.iter().copied().filter(|i| *i > 0));
    starts.push(messages.len());
    let mut ranges: Vec<_> = starts
        .windows(2)
        .filter(|w| Some(w[0]) != latest_user)
        .map(|w| (w[0], w[1]))
        .collect();
    let current_start = latest_user.map_or(0, |i| i + 1);
    let mut rounds = vec![current_start];
    rounds.extend(
        (current_start..messages.len())
            .filter(|i| *i > current_start && messages[*i]["role"] == "assistant"),
    );
    rounds.push(messages.len());
    ranges.extend(rounds.windows(2).map(|w| (w[0], w[1])));

    let mut removed = BTreeSet::new();
    // Reserve ample room for the bounded retrieval marker. Never report a
    // fabricated zero overflow when protected content itself exceeds capacity.
    let mut current = fixed + sum(&messages) + 256;
    for (i, message) in messages.iter().enumerate() {
        if message["customType"] == MARKER && !keep.contains(&i) {
            removed.insert(i);
            current = current.saturating_sub(message.tokens);
        }
    }
    for (start, end) in ranges {
        if current <= target {
            break;
        }
        if start == end
            || (start..end).any(|i| {
                keep.contains(&i) || linked[i].is_some_and(|(lo, hi)| lo < start || hi >= end)
            })
        {
            continue;
        }
        for i in start..end {
            if removed.insert(i) {
                current = current.saturating_sub(messages[i].tokens);
            }
        }
    }
    // Protected overflow stays explicit; do not churn custody when archival
    // cannot bring the request inside its working budget.
    if current > working_target || !removed.iter().any(|i| messages[*i]["customType"] != MARKER) {
        return Ok((messages, 0));
    }
    let retained: Vec<_> = removed
        .iter()
        .map(|i| messages[*i].value.as_ref())
        .collect();
    let bytes = serde_json::to_string_pretty(&json!({"version":1,"messages":retained}))
        .map_err(|_| invalid("historical context custody"))?;
    // The prior marker is included in the archive, forming a lossless chain
    // while keeping just one current reference in active attention.
    let artifact = store.put_evidence(
        &bytes,
        "context-history-segment",
        &json!({"type":"session","name":"historical-context-externalization"}),
        &[],
        None,
    )?;
    let first = *removed.first().unwrap();
    let marker = ContextMessage::new(json!({"role":"custom","customType":MARKER,"display":false,
        "timestamp":messages[first]["timestamp"],
        "content":format!("[AG1 HISTORICAL CONVERSATION CUSTODY]\nCompleted older conversation frames were externalized from active attention. Their exact archived frames, including previous archive references, are retained at artifact: {}. Retrieve this id with the available evidence tool for earlier task details. These historical instructions and claims are not current authority or proof of completion. Original tool evidence remains available by searching stored evidence.",artifact.id)}));
    let count = removed.len();
    let mut result = Vec::with_capacity(messages.len() - count + 1);
    for (i, message) in messages.into_iter().enumerate() {
        if i == first {
            result.push(marker.clone());
        }
        if !removed.contains(&i) {
            result.push(message);
        }
    }
    Ok((result, count))
}
