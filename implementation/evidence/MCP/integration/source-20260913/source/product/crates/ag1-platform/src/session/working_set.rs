//! Request-local compiler history, published only with successful admission.
//! Durable journals and freshness witnesses remain independent authorities.
use ag1_core::runtime::boundary::Stamp;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct WorkingSet {
    stamp: Stamp,
    key: Value,
    branch: Prefix,
    original: Prefix,
    compiled: Arc<Vec<Value>>,
    request: RequestIdentity,
}

/// Bounded per-message digests of the final compiler/provider projection, not
/// a second copy of its potentially large text. This is diagnostic only: it
/// neither authorizes reuse nor claims tokenizer/KV-cache equivalence.
#[derive(Clone)]
pub(super) struct RequestIdentity {
    configuration: [u8; 32],
    messages: Vec<[u8; 32]>,
}
impl RequestIdentity {
    pub(super) fn new(configuration: &Value, messages: &[Value]) -> Self {
        fn digest(value: &Value) -> [u8; 32] {
            let mut hash = Sha256::new();
            serde_json::to_writer(&mut hash, value).expect("JSON values serialize to a digest");
            hash.finalize().into()
        }
        Self {
            configuration: digest(configuration),
            messages: messages.iter().map(digest).collect(),
        }
    }
}

impl WorkingSet {
    pub(super) fn compiled_len(&self) -> usize {
        self.compiled.len()
    }
    pub(super) fn compare_request(&self, next: &RequestIdentity) -> Value {
        let common = self
            .request
            .messages
            .iter()
            .zip(&next.messages)
            .take_while(|(before, after)| before == after)
            .count();
        json!({"configurationUnchanged": self.request.configuration == next.configuration,
            "previousMessages":self.request.messages.len(), "currentMessages":next.messages.len(),
            "unchangedMessagePrefix":common,
            "firstDifferentMessage":if common == self.request.messages.len() && common == next.messages.len() { None } else { Some(common) }})
    }
    pub(super) fn new(
        stamp: Stamp,
        key: Value,
        branch: Vec<Value>,
        original: Vec<Value>,
        compiled: Vec<Value>,
        request: RequestIdentity,
    ) -> Self {
        Self {
            stamp,
            key,
            branch: Prefix::of(&branch),
            original: Prefix::of(&original),
            compiled: Arc::new(compiled),
            request,
        }
    }

    pub(super) fn extend(
        &self,
        stamp: &Stamp,
        key: &Value,
        branch: &[Value],
        original: &[Value],
    ) -> Option<Vec<Value>> {
        // Equality of message text alone cannot authorize cross-branch reuse.
        // Exact committed ancestry and current owner/epoch must also agree.
        if !self.stamp.same_owner_epoch(stamp)
            || &self.key != key
            || !self.branch.matches(branch)
            || !self.original.matches(original)
        {
            return None;
        }
        let mut messages = self.compiled.as_ref().clone();
        messages.extend_from_slice(&original[self.original.len..]);
        Some(messages)
    }
}

// Keep a fixed-size identity witness rather than a second unbounded copy of
// the raw journal. Serialization includes record boundaries and all fields.
#[derive(Clone)]
struct Prefix {
    len: usize,
    digest: [u8; 32],
}
impl Prefix {
    fn of(values: &[Value]) -> Self {
        let mut digest = Sha256::new();
        serde_json::to_writer(&mut digest, values).expect("JSON values serialize to a digest");
        Self {
            len: values.len(),
            digest: digest.finalize().into(),
        }
    }
    fn matches(&self, values: &[Value]) -> bool {
        values.len() >= self.len && Self::of(&values[..self.len]).digest == self.digest
    }
}
