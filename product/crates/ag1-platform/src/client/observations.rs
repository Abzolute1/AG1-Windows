//! Advisory measured progress. Completion and evidence remain native receipts.
use super::*;
use ag1_core::provider::{BlockKind, ProviderEvent};
use std::collections::{BTreeMap, VecDeque};
#[derive(Default)]
pub(super) struct Observations {
    scope: Option<w::Scope>,
    started: u64,
    first_delta: Option<u64>,
    last_delta: Option<u64>,
    output: u64,
    generation_start: Option<u64>,
    generation_end: Option<u64>,
    generations: u64,
    keys: BTreeMap<String, BlockKind>,
    events: VecDeque<Value>,
    phase: String,
    live_blocks: Vec<Value>,
    live_bytes: usize,
    committed: bool,
    generation: u64,
}
impl Observations {
    pub fn begin(&mut self, scope: w::Scope, started: u64) {
        *self = Self {
            scope: Some(scope),
            started,
            ..Default::default()
        };
    }
    pub fn event(&mut self, event: &SessionEvent) -> Option<String> {
        let now = WallClock.now_ms();
        match event {
            SessionEvent::Provider(ProviderEvent::Begin { key, kind, .. }) => {
                if self.keys.is_empty() {
                    self.generation_start = Some(now);
                    self.generation += 1;
                    self.live_blocks.clear();
                    self.live_bytes = 0;
                    self.committed = false;
                    self.generation_end = None;
                    self.output = 0;
                }
                if self.keys.len() < 256 {
                    self.keys.insert(key.clone(), *kind);
                    if matches!(kind, BlockKind::Text | BlockKind::Thinking) {
                        self.live_blocks.push(json!({"key":key,"kind":if *kind == BlockKind::Thinking {"thinking"} else {"text"},"text":"","revision":0}));
                    }
                }
                self.phase = format!("{kind:?}").to_lowercase();
            }
            SessionEvent::Provider(ProviderEvent::Delta { key, text }) => {
                self.first_delta.get_or_insert(now);
                self.last_delta = Some(now);
                if let Some(block) = self.live_blocks.iter_mut().find(|b| b["key"] == *key) {
                    let room = (4 * 1024 * 1024usize).saturating_sub(self.live_bytes);
                    let end = utf8_end(text, room);
                    if let Value::String(contents) = &mut block["text"] {
                        contents.push_str(&text[..end]);
                    }
                    self.live_bytes += end;
                    if end < text.len() {
                        block["limited"] = json!(true);
                    }
                    self.phase = if self.keys.get(key) == Some(&BlockKind::Thinking) {
                        "thinking"
                    } else {
                        "generating"
                    }
                    .into();
                }
                if self.keys.get(key) == Some(&BlockKind::Text) {
                    return Some(text.clone());
                }
            }
            SessionEvent::Provider(ProviderEvent::Replace {
                key,
                text,
                signature: false,
            }) => {
                if let Some(block) = self.live_blocks.iter_mut().find(|b| b["key"] == *key) {
                    self.live_bytes = self
                        .live_bytes
                        .saturating_sub(block["text"].as_str().unwrap_or("").len());
                    block["text"] = json!("");
                    block["limited"] = json!(false);
                    block["revision"] = json!(block["revision"].as_u64().unwrap_or(0) + 1);
                }
                let _ = self.event(&SessionEvent::Provider(ProviderEvent::Delta {
                    key: key.clone(),
                    text: text.clone(),
                }));
            }
            SessionEvent::Provider(ProviderEvent::Usage(usage)) => {
                if let Some(output) = usage.output {
                    self.output = output;
                }
            }
            SessionEvent::Provider(ProviderEvent::Stop(_)) => {
                self.generation_end = Some(now);
                self.generations += 1;
                self.phase = "provider stopped".into();
                self.keys.clear();
            }
            SessionEvent::Committed { .. } => {
                if self.generation_end.is_some() {
                    self.committed = true;
                }
                if self.generation == 0 {
                    self.phase = "processing".into();
                }
            }
            SessionEvent::Recovery(value) => {
                if value["type"] == "request_phase" {
                    if let Some(phase @ ("preparing" | "waiting" | "executing")) =
                        value["phase"].as_str()
                    {
                        self.phase = phase.into();
                    }
                    return None;
                }
                let value = if value.to_string().len() > 4096 {
                    json!({"type":value["type"],"display":"Event display limited; inspect the operation and its artifacts"})
                } else {
                    value.clone()
                };
                self.events.push_back(value);
                while self.events.len() > 12
                    || self
                        .events
                        .iter()
                        .map(|e| e.to_string().len())
                        .sum::<usize>()
                        > 12000
                {
                    self.events.pop_front();
                }
            }
            _ => {}
        }
        None
    }
    pub fn page(&self, scope: &w::Scope, args: &Value) -> Result<Value> {
        use base64::{Engine, engine::general_purpose::STANDARD};
        if self.scope.as_ref() != Some(scope)
            || args["id"] != format!("{}-{}", self.started, self.generation)
        {
            return Err(DomainError::Conflict);
        }
        let block = self
            .live_blocks
            .iter()
            .find(|b| b["key"] == args["key"])
            .ok_or(DomainError::NotFound)?;
        if args["revision"].as_u64().unwrap_or(0) != block["revision"].as_u64().unwrap_or(0) {
            return Err(DomainError::Conflict);
        }
        let text = block["text"].as_str().unwrap_or("").as_bytes();
        let offset = args["offset"].as_u64().unwrap_or(0) as usize;
        if offset > text.len() {
            return Err(DomainError::InvalidInput("Live block offset".into()));
        }
        let end = (offset + 16000).min(text.len());
        Ok(
            json!({"base64":STANDARD.encode(&text[offset..end]),"offset":end,"total":text.len(),"limited":block["limited"]==true}),
        )
    }
    pub fn view(&self, scope: &w::Scope) -> Value {
        if self.scope.as_ref() != Some(scope) {
            return Value::Null;
        }
        let elapsed = self
            .last_delta
            .map(|last| last.saturating_sub(self.started));
        let duration = self
            .generation_start
            .zip(self.generation_end)
            .map(|(start, end)| end.saturating_sub(start))
            .filter(|n| *n > 0);
        let throughput = duration
            .filter(|_| self.output > 0)
            .map(|ms| self.output as f64 * 1000.0 / ms as f64);
        // Reserve envelope space for durable messages, approvals and other observations.
        let mut room = 8000;
        let blocks = self.live_blocks.iter().map(|block| {
            let contents = block["text"].as_str().unwrap_or("");
            let end = utf8_end(contents, room.min(4000)); room -= end;
            json!({"key":block["key"],"kind":block["kind"],"revision":block["revision"],"text":&contents[..end],"bytes":contents.len(),"truncated":end < contents.len(),"limited":block["limited"]==true})
        }).collect::<Vec<_>>();
        json!({"started_ms":self.started,"live":{"id":format!("{}-{}",self.started,self.generation),"blocks":blocks,"committed":self.committed},"latest_provider_output_tokens_per_second":throughput,"throughput_interval_ms":duration,"throughput_measurement":"Reported output tokens divided by native time from first provider block to stop; unavailable until usage and stop are observed", "phase":self.phase,"provider_stops":self.generations,"latest_reported_output_tokens":self.output,"first_delta_ms":self.first_delta.map(|first|first.saturating_sub(self.started)),"last_delta_elapsed_ms":elapsed,"measurement":"Wall time from native operation start; provider-reported usage, not estimated tokens or live-model benchmarks","events":self.events})
    }
}

fn utf8_end(text: &str, maximum: usize) -> usize {
    let mut end = maximum.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope() -> w::Scope {
        serde_json::from_value(json!({"workspace":"ws","session":"s","instance":"i","epoch":"1"}))
            .unwrap()
    }
    fn begin(key: &str, kind: BlockKind) -> SessionEvent {
        SessionEvent::Provider(ProviderEvent::Begin {
            key: key.into(),
            kind,
            id: String::new(),
            name: String::new(),
            arguments: None,
            redacted: false,
        })
    }
    fn delta(key: &str, text: &str) -> SessionEvent {
        SessionEvent::Provider(ProviderEvent::Delta {
            key: key.into(),
            text: text.into(),
        })
    }
    #[test]
    fn thinking_and_answer_are_visible_before_stop_and_commit_is_distinct() {
        let mut o = Observations::default();
        let scope = scope();
        o.begin(scope.clone(), 1);
        o.event(&begin("thought", BlockKind::Thinking));
        assert_eq!(o.event(&delta("thought", "Inspect first")), None);
        o.event(&begin("answer", BlockKind::Text));
        assert_eq!(o.event(&delta("answer", "Here is")), Some("Here is".into()));
        let v = o.view(&scope);
        assert_eq!(v["live"]["blocks"][0]["text"], "Inspect first");
        assert_eq!(v["live"]["blocks"][1]["text"], "Here is");
        assert_eq!(v["live"]["committed"], false);
        o.event(&SessionEvent::Provider(ProviderEvent::Stop(
            ag1_core::provider::StopReason::Stop,
        )));
        assert_eq!(o.view(&scope)["live"]["committed"], false);
        o.event(&SessionEvent::Committed {
            revision: 1,
            sequence: 1,
        });
        assert_eq!(o.view(&scope)["live"]["committed"], true);
        o.event(&begin("next", BlockKind::Text));
        assert_ne!(v["live"]["id"], o.view(&scope)["live"]["id"]);
        assert_eq!(
            o.view(&scope)["live"]["blocks"].as_array().unwrap().len(),
            1
        );
    }
    #[test]
    fn replaced_provider_text_invalidates_old_live_cursors() {
        let mut o = Observations::default();
        let scope = scope();
        o.begin(scope.clone(), 1);
        o.event(&begin("answer", BlockKind::Text));
        o.event(&delta("answer", "earlier"));
        let id = o.view(&scope)["live"]["id"].clone();
        o.event(&SessionEvent::Provider(ProviderEvent::Replace {
            key: "answer".into(),
            text: "replacement".into(),
            signature: false,
        }));
        assert_eq!(o.view(&scope)["live"]["blocks"][0]["text"], "replacement");
        assert!(
            o.page(&scope, &json!({"id":id,"key":"answer","revision":0}))
                .is_err()
        );
        assert!(
            o.page(&scope, &json!({"id":id,"key":"answer","revision":1}))
                .is_ok()
        );
    }
    #[test]
    fn long_unicode_stream_pages_without_consuming_answer_preview_or_crossing_scope() {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let mut o = Observations::default();
        let scope = scope();
        o.begin(scope.clone(), 1);
        o.event(&begin("thinking", BlockKind::Thinking));
        let input = "😀thinking ".repeat(3000);
        o.event(&delta("thinking", &input));
        let view = o.view(&scope);
        assert_eq!(view["live"]["blocks"][0]["truncated"], true);
        assert!(view.to_string().len() < 12000);
        let mut bytes = Vec::new();
        let mut offset = 0;
        while offset < input.len() {
            let page = o
                .page(
                    &scope,
                    &json!({"id":view["live"]["id"],"key":"thinking","offset":offset}),
                )
                .unwrap();
            bytes.extend(STANDARD.decode(page["base64"].as_str().unwrap()).unwrap());
            offset = page["offset"].as_u64().unwrap() as usize;
        }
        assert_eq!(String::from_utf8(bytes).unwrap(), input);
        assert!(
            o.page(&scope, &json!({"id":"stale","key":"thinking"}))
                .is_err()
        );
        let mut other = scope.clone();
        other.session = serde_json::from_value(json!("other")).unwrap();
        assert!(o.view(&other).is_null());
        assert!(
            o.page(&other, &json!({"id":view["live"]["id"],"key":"thinking"}))
                .is_err()
        );
    }
}
