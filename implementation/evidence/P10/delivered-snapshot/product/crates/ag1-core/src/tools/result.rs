//! B01: one aggregate text budget, including framing, with original content retained first.
use crate::{
    DomainError,
    context::{ContextArtifact, ContextArtifactPort, salient, slice_utf16, tail_utf16},
    task::units,
};
use serde_json::{Value, json};
pub trait ResultArtifacts: ContextArtifactPort {
    fn read_call(&self, id: &str, offset: u64, max_bytes: u32) -> Result<Value, DomainError>;
    /// Authenticate exact producer text in this requester's store. Raw result
    /// metadata cannot grant access to an arbitrary artifact.
    fn producer_record(
        &self,
        _id: &str,
        _tool: &str,
        _call: &str,
    ) -> Result<ContextArtifact, DomainError> {
        Err(DomainError::PermissionDenied)
    }
    fn text_page(
        &self,
        _record: &ContextArtifact,
        _offset: u64,
        _max_bytes: u32,
    ) -> Result<(u64, String), DomainError> {
        Err(DomainError::PermissionDenied)
    }
}
/// Native-only grant, not deserializable from extension or model JSON.
pub struct ProducerArtifact {
    record: ContextArtifact,
    tool: String,
    call: String,
}
impl ProducerArtifact {
    pub fn authorize(
        store: &impl ResultArtifacts,
        id: &str,
        tool: &str,
        call: &str,
    ) -> Result<Self, DomainError> {
        let record = store.producer_record(id, tool, call)?;
        if record.kind != kind(tool) {
            return Err(DomainError::PermissionDenied);
        }
        Ok(Self {
            record,
            tool: tool.into(),
            call: call.into(),
        })
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ResultBudget {
    pub text_units: usize,
    pub images: usize,
    pub image_bytes: usize,
    /// Host-owned override used for bounded tool-specific projections.
    pub threshold_bytes: Option<usize>,
}
impl Default for ResultBudget {
    fn default() -> Self {
        Self {
            text_units: 64000,
            images: 8,
            image_bytes: 8 * 1024 * 1024,
            threshold_bytes: None,
        }
    }
}
pub struct ResultProjection {
    pub original: ContextArtifact,
    pub text_artifacts: Vec<ContextArtifact>,
    pub result: Value,
    pub projected_text_units: usize,
    pub image_bytes: usize,
    pub exempt_artifact_page: bool,
}
fn error(detail: &str) -> DomainError {
    DomainError::InvalidInput(format!("tool result: {detail}"))
}
fn kind(tool: &str) -> &'static str {
    let t = tool.to_lowercase();
    match t.as_str() {
        "bash" | "job" => "terminal-output",
        "check" => "test-output",
        "webfetch" | "websearch" => "web-response",
        "write" | "diff" => "diff",
        _ if t.contains("test") => "test-output",
        _ if t.contains("edit") => "diff",
        _ => "tool-output",
    }
}
/// Validate an untrusted hook replacement before allowing it to affect a completed native result.
pub(crate) fn validate(raw: &Value, budget: ResultBudget) -> Result<(), DomainError> {
    validate_content(raw, budget).map(|_| ())
}
fn validate_content(
    raw: &Value,
    budget: ResultBudget,
) -> Result<(usize, usize, Vec<&str>), DomainError> {
    if budget.text_units < 128
        || budget.text_units > 1_000_000
        || budget
            .threshold_bytes
            .is_some_and(|n| n == 0 || n > 128 * 1024 * 1024)
        || budget.images > 32
        || budget.image_bytes > 64 * 1024 * 1024
    {
        return Err(error("invalid projection budget"));
    }
    super::schema::bounded(raw, 128 * 1024 * 1024, 64)?;
    let content = raw["content"]
        .as_array()
        .ok_or_else(|| error("content array required"))?;
    if content.len() > 10000 {
        return Err(error("content block limit"));
    }
    let mut text_units = 0usize;
    let mut image_bytes = 0usize;
    let mut images = 0usize;
    let mut texts = vec![];
    for b in content {
        match b["type"].as_str() {
            Some("text") => {
                let text = b["text"].as_str().ok_or_else(|| error("text required"))?;
                text_units = text_units.saturating_add(units(text));
                texts.push(text);
            }
            Some("image") => {
                let data = b["data"]
                    .as_str()
                    .ok_or_else(|| error("image data required"))?;
                if !b["mimeType"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("image/"))
                {
                    return Err(error("image MIME required"));
                }
                images += 1;
                image_bytes = image_bytes.saturating_add(data.len());
            }
            _ => return Err(error("unsupported content block")),
        }
    }
    if images > budget.images || image_bytes > budget.image_bytes {
        return Err(DomainError::LimitExceeded("tool image budget".into()));
    }
    Ok((text_units, image_bytes, texts))
}
/// Original and text artifacts are acknowledged before any shortened result is returned.
/// Only the built-in artifact read dispatcher may set `artifact_page` after cursor authorization.
pub fn project(
    store: &mut impl ResultArtifacts,
    tool: &str,
    call: &str,
    raw: &Value,
    budget: ResultBudget,
    artifact_page: bool,
) -> Result<ResultProjection, DomainError> {
    project_with_producer(store, tool, call, raw, budget, artifact_page, None)
}
pub fn project_with_producer(
    store: &mut impl ResultArtifacts,
    tool: &str,
    call: &str,
    raw: &Value,
    budget: ResultBudget,
    artifact_page: bool,
    producer: Option<&ProducerArtifact>,
) -> Result<ResultProjection, DomainError> {
    if let Some(producer) = producer
        && (producer.tool != tool
            || producer.call != call
            || store.producer_record(&producer.record.id, tool, call)? != producer.record)
    {
        return Err(DomainError::PermissionDenied);
    }
    let (text_units, image_bytes, texts) = validate_content(raw, budget)?;
    let content = raw["content"].as_array().expect("validated content");
    let bytes = serde_json::to_string(raw).map_err(|_| error("JSON"))?;
    if bytes.len() > 128 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded(
            "original tool result bytes".into(),
        ));
    }
    let original = store.put_evidence(
        &bytes,
        "tool-original-result",
        &json!({"type":"tool","name":tool,"callId":call}),
        &[],
        None,
    )?;
    if artifact_page && tool != "artifact" {
        return Err(error(
            "artifact exemption requires the built-in artifact dispatcher",
        ));
    }
    let text_bytes: usize = texts.iter().map(|t| t.len()).sum();
    let threshold = budget
        .threshold_bytes
        .unwrap_or_else(|| match tool.to_lowercase().as_str() {
            "write" => 8192,
            "read" | "edit" | "diff" => 56 * 1024,
            _ => 24 * 1024,
        });
    if artifact_page || text_units <= budget.text_units && text_bytes < threshold {
        return Ok(ResultProjection {
            original,
            text_artifacts: vec![],
            result: raw.clone(),
            projected_text_units: text_units,
            image_bytes,
            exempt_artifact_page: artifact_page,
        });
    }
    let large_count = texts.iter().filter(|s| s.len() >= threshold).count();
    let small_units: usize = texts
        .iter()
        .filter(|s| s.len() < threshold)
        .map(|s| units(s))
        .sum();
    // Preserve small acknowledgements and block positions when each preview can
    // retain a useful pointer. Otherwise externalize aggregate text once: many
    // individually small blocks and repeated framing must share the same budget.
    let individual =
        large_count > 0 && small_units.saturating_add(large_count * 128) <= budget.text_units;
    let mut evidence = vec![];
    let mut references = vec![];
    let mut blocks = vec![];
    let mut producer = producer;
    if individual {
        let preview_budget = (budget.text_units - small_units) / large_count;
        for block in content {
            if let Some(text) = block["text"].as_str().filter(|_| block["type"] == "text")
                && text.len() >= threshold
            {
                let (record, next, preview) =
                    externalize(store, tool, call, text, preview_budget, producer.take())?;
                references.push(reference(&record, next));
                evidence.push(record);
                blocks.push(json!({"type":"text","text":preview}));
            } else {
                blocks.push(block.clone());
            }
        }
    } else {
        let joined = texts.join("\n");
        let (record, next, preview) = externalize(
            store,
            tool,
            call,
            &joined,
            budget.text_units,
            producer.take(),
        )?;
        references.push(reference(&record, next));
        evidence.push(record);
        let mut inserted = false;
        for block in content {
            if block["type"] == "text" {
                if !inserted {
                    blocks.push(json!({"type":"text","text":preview}));
                    inserted = true;
                }
            } else {
                blocks.push(block.clone());
            }
        }
    }
    let projected_text_units = blocks
        .iter()
        .filter_map(|b| b["text"].as_str())
        .map(units)
        .sum();
    if projected_text_units > budget.text_units {
        return Err(error("aggregate projection exceeded its budget"));
    }
    let mut result = raw.clone();
    result["content"] = json!(blocks);
    // Unknown/non-object details remain verbatim alongside explicit projection metadata.
    let projection = json!({"version":1,"originalResultArtifact":original.id,"artifacts":references,"aggregateTextBudget":budget.text_units,"fullResultRetained":true});
    if result["details"].is_object() {
        let details = result["details"].as_object_mut().unwrap();
        details.insert("toolResultProjection".into(), projection);
    } else {
        result["details"] = json!({"originalDetails":raw.get("details").cloned().unwrap_or(Value::Null),"toolResultProjection":projection});
    }
    Ok(ResultProjection {
        original,
        text_artifacts: evidence,
        result,
        projected_text_units,
        image_bytes,
        exempt_artifact_page: false,
    })
}
fn reference(record: &ContextArtifact, next: Value) -> Value {
    json!({"id":record.id,"kind":record.kind,"bytes":record.bytes,"sha256":record.sha256,"nextCall":next})
}
fn externalize(
    store: &mut impl ResultArtifacts,
    tool: &str,
    call: &str,
    text: &str,
    cap: usize,
    producer: Option<&ProducerArtifact>,
) -> Result<(ContextArtifact, Value, String), DomainError> {
    let record = match producer {
        Some(source) => source.record.clone(),
        None => store.put_text(text, kind(tool), tool, call, &salient::select(text))?,
    };
    let next = store.read_call(&record.id, 0, 32768)?;
    let preview = if producer.is_some() {
        let (head_limit, tail_limit) = if tool.eq_ignore_ascii_case("write") {
            (1000, 500)
        } else {
            (6000, 2000)
        };
        let (head_offset, head) = store.text_page(&record, 0, head_limit)?;
        let requested_tail =
            (head.len() as u64).max(record.bytes.saturating_sub(tail_limit as u64));
        let (tail_offset, tail) = store.text_page(&record, requested_tail, tail_limit)?;
        if head_offset != 0
            || head.len() > head_limit as usize + 3
            || tail.len() > tail_limit as usize + 3
            || tail_offset < head.len() as u64
            || tail_offset.checked_add(tail.len() as u64) != Some(record.bytes)
        {
            return Err(error("producer artifact page bounds"));
        }
        preview_segments(tool, &record, &next, cap, &head, &tail, tail_offset)
    } else {
        preview(text, tool, &record, &next, cap)
    };
    Ok((record, next, preview))
}
fn byte_head(text: &str, limit: usize) -> &str {
    let mut end = limit.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
fn byte_tail(text: &str, limit: usize, after: usize) -> &str {
    let mut start = after.max(text.len().saturating_sub(limit));
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}
fn preview(text: &str, tool: &str, record: &ContextArtifact, next: &Value, cap: usize) -> String {
    let is_write = tool.eq_ignore_ascii_case("write");
    let head = byte_head(text, if is_write { 1000 } else { 6000 });
    let tail = byte_tail(text, if is_write { 500 } else { 2000 }, head.len());
    preview_segments(
        tool,
        record,
        next,
        cap,
        head,
        tail,
        (text.len() - tail.len()) as u64,
    )
}
fn preview_segments(
    tool: &str,
    record: &ContextArtifact,
    next: &Value,
    cap: usize,
    head: &str,
    tail: &str,
    omitted_end: u64,
) -> String {
    let omitted_start = head.len() as u64;
    let header = format!(
        "{}\n[Large {tool} result externalized]\nartifact: {}\nkind: {}\nsize: {} bytes, {} lines\nfull_result: exact artifact, paged by UTF-8 byte offset\n\n",
        json!({"externalizedToolResult":{"version":1,"artifactId":record.id,"kind":record.kind,"totalBytes":record.bytes,"exact":true,"omittedRange":{"startByte":omitted_start,"endByte":omitted_end},"nextCall":next}}),
        record.id,
        record.kind,
        crate::context::commas(record.bytes),
        record
            .line_count
            .map(|n| n.to_string())
            .unwrap_or_else(|| "?".into())
    );
    let marker = if omitted_end > omitted_start {
        format!(
            "\n[... {} bytes omitted from active context; exact artifact bytes {omitted_start}-{omitted_end} ...]\n",
            crate::context::commas(omitted_end - omitted_start)
        )
    } else {
        String::new()
    };
    let footer = if omitted_end > omitted_start {
        "\nExact omitted content remains available through externalizedToolResult.nextCall above."
    } else {
        "\nThe complete result is resident above; the artifact is its immutable verification copy."
    };
    let full = format!("{header}{head}{marker}{tail}{footer}");
    if units(&full) <= cap {
        return full;
    }
    // The bounded fallback counts framing too. Cursor details remain available in
    // projection metadata even when the text budget cannot fit a typed envelope.
    let header = format!(
        "artifact: {}\nfull_result: exact artifact, paged by UTF-8 byte offset\n",
        record.id
    );
    let marker = "\n[omitted; read artifact]";
    let available = cap.saturating_sub(units(&header) + units(marker));
    let head = slice_utf16(head, available * 3 / 4);
    let tail = tail_utf16(tail, available.saturating_sub(units(&head)));
    slice_utf16(&format!("{header}{head}{marker}{tail}"), cap)
}
