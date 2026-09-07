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
                    self.generation_end = None;
                    self.output = 0;
                }
                if self.keys.len() < 256 {
                    self.keys.insert(key.clone(), *kind);
                }
                self.phase = format!("{kind:?}").to_lowercase();
            }
            SessionEvent::Provider(ProviderEvent::Delta { key, text }) => {
                self.first_delta.get_or_insert(now);
                self.last_delta = Some(now);
                if self.keys.get(key) == Some(&BlockKind::Text) {
                    return Some(text.clone());
                }
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
            SessionEvent::Recovery(value) => {
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
        json!({"latest_provider_output_tokens_per_second":throughput,"throughput_interval_ms":duration,"throughput_measurement":"Reported output tokens divided by native time from first provider block to stop; unavailable until usage and stop are observed", "phase":self.phase,"provider_stops":self.generations,"latest_reported_output_tokens":self.output,"first_delta_ms":self.first_delta.map(|first|first.saturating_sub(self.started)),"last_delta_elapsed_ms":elapsed,"measurement":"Wall time from native operation start; provider-reported usage, not estimated tokens or live-model benchmarks","events":self.events})
    }
}
