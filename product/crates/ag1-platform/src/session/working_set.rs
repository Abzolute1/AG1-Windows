//! Request-local compiler history, published only with successful admission.
//! Durable journals and freshness witnesses remain independent authorities.
use ag1_core::runtime::boundary::Stamp;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct WorkingSet {
    stamp: Stamp,
    key: Value,
    branch: Prefix,
    original: Prefix,
    compiled: Arc<Vec<Value>>,
}

impl WorkingSet {
    pub(super) fn new(
        stamp: Stamp,
        key: Value,
        branch: Vec<Value>,
        original: Vec<Value>,
        compiled: Vec<Value>,
    ) -> Self {
        Self {
            stamp,
            key,
            branch: Prefix::of(&branch),
            original: Prefix::of(&original),
            compiled: Arc::new(compiled),
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
