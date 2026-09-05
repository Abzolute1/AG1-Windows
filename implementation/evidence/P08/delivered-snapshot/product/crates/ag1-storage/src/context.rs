//! Durable shadow compilation. Admission and epoch commits belong to the session runtime.
use crate::{
    ArtifactInput, Store, digest,
    persistence::{domain_error, valid_id},
};
use ag1_core::{
    DomainError, SessionId,
    context::{
        ContextArtifact, ContextArtifactPort,
        compiler::{Compilation, CompileOptions, compile_request},
        features::Features,
        validate_history,
    },
    storage::StoragePort,
};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

pub struct ContextStore<'a> {
    store: &'a Store,
    session: SessionId,
    origin: String,
    operation: String,
    timestamp: String,
    next: u64,
    features: Features,
}
pub struct ShadowCompilation {
    pub projection: Compilation,
    pub original_journal: ContextArtifact,
    pub original_request: ContextArtifact,
    pub revision: u64,
}
impl<'a> ContextStore<'a> {
    pub fn new(
        store: &'a Store,
        session: SessionId,
        origin: &str,
        operation: &str,
        timestamp: &str,
        features: Features,
    ) -> Result<Self, DomainError> {
        valid_id(operation).map_err(domain_error)?;
        valid_id(origin).map_err(domain_error)?;
        // This is an owned output origin. Inherited evidence is resolved separately below.
        let db = store.conn().map_err(domain_error)?;
        Store::ready(&db).map_err(domain_error)?;
        let owner: Option<String> = db
            .query_row("SELECT session FROM origins WHERE id=?1", [origin], |r| {
                r.get(0)
            })
            .optional()
            .map_err(crate::StorageError::from)
            .map_err(domain_error)?;
        if owner.as_deref() != Some(session.as_str())
            || timestamp.is_empty()
            || timestamp.len() > 64
        {
            return Err(DomainError::InvalidInput(
                "context origin or timestamp".into(),
            ));
        }
        Ok(Self {
            store,
            session,
            origin: origin.into(),
            operation: operation.into(),
            timestamp: timestamp.into(),
            next: 0,
            features,
        })
    }
    /// Captures exact journal record bytes and the explicit projection input before any reductions.
    /// A changed revision rejects the shadow result; P10 must still fence final model admission.
    pub fn compile(
        &mut self,
        messages: &[Value],
        options: &CompileOptions,
        expected: u64,
    ) -> Result<ShadowCompilation, DomainError> {
        self.project(messages, expected, |messages, store| {
            compile_request(messages, options, store)
        })
    }
    /// Controlled shadow transaction for compiler, emergency hygiene and summary projections.
    /// The reduction is invoked only after both custody artifacts acknowledge durable bytes.
    pub fn project(
        &mut self,
        messages: &[Value],
        expected: u64,
        reduce: impl FnOnce(&[Value], &mut Self) -> Result<Compilation, DomainError>,
    ) -> Result<ShadowCompilation, DomainError> {
        validate_history(messages)?;
        let journal = self.store.journal_snapshot(&self.session)?;
        if journal.revision != expected {
            return Err(DomainError::Conflict);
        }
        // Base64 retains exact bytes without expanding each source byte into a JSON number.
        let original = json!({"version":1,"session":self.session.as_str(),"revision":journal.revision,"leaf":journal.leaf,"headerBase64":B64.encode(&journal.header),"entries":journal.entries.iter().map(|e| json!({"id":e.id,"parent":e.parent,"physicalOrder":e.physical_order,"rawBase64":B64.encode(&e.raw)})).collect::<Vec<_>>()});
        let original_journal = self.put_text(
            &original.to_string(),
            "context-original-journal",
            "contextCompiler",
            "journal",
            &[],
        )?;
        let original_request = self.put_text(
            &serde_json::to_string(messages)
                .map_err(|_| DomainError::InvalidInput("request JSON".into()))?,
            "context-original-request",
            "contextCompiler",
            "request",
            &[],
        )?;
        let projection = reduce(messages, self)?;
        if !ag1_core::context::retry::complete_pairs(&projection.messages) {
            return Err(DomainError::InvalidInput(
                "context projection has an orphan tool result".into(),
            ));
        }
        if self.store.journal_snapshot(&self.session)?.revision != expected {
            return Err(DomainError::Conflict);
        }
        Ok(ShadowCompilation {
            projection,
            original_journal,
            original_request,
            revision: expected,
        })
    }
    pub fn origin_for(&self, id: &str) -> Result<String, DomainError> {
        let db = self.store.conn().map_err(domain_error)?;
        Store::ready(&db).map_err(domain_error)?;
        let mut query = db.prepare("SELECT origin FROM access WHERE session=?1 AND artifact=?2 ORDER BY origin LIMIT 2").map_err(crate::StorageError::from).map_err(domain_error)?;
        let origins = query
            .query_map(params![self.session.as_str(), id], |r| {
                r.get::<_, String>(0)
            })
            .map_err(crate::StorageError::from)
            .map_err(domain_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(crate::StorageError::from)
            .map_err(domain_error)?;
        if origins.len() != 1 {
            return Err(DomainError::InvalidInput(
                "missing or ambiguous context artifact".into(),
            ));
        }
        Ok(origins[0].clone())
    }
}
impl ContextArtifactPort for ContextStore<'_> {
    fn put_text(
        &mut self,
        text: &str,
        kind: &str,
        tool: &str,
        call_id: &str,
        salient: &[String],
    ) -> Result<ContextArtifact, DomainError> {
        self.put_evidence(
            text,
            kind,
            &json!({"type":"tool","name":tool,"callId":call_id}),
            salient,
            None,
        )
    }
    fn put_evidence(
        &mut self,
        text: &str,
        kind: &str,
        producer: &Value,
        salient: &[String],
        parent: Option<&str>,
    ) -> Result<ContextArtifact, DomainError> {
        if !producer["name"].is_string()
            || !["tool", "system", "session"].contains(&producer["type"].as_str().unwrap_or(""))
        {
            return Err(DomainError::InvalidInput("artifact producer".into()));
        }
        let sha256 = digest(text.as_bytes());
        // Stable per-operation occurrence identity makes a retry idempotent without merging producers.
        let identity = json!([
            self.session.as_str(),
            self.origin,
            self.operation,
            self.next,
            kind,
            producer,
            parent,
            sha256,
            salient
        ])
        .to_string();
        let id = format!("art_{}", &digest(identity.as_bytes())[..32]);
        let line_count = if text.is_empty() {
            0
        } else {
            text.bytes().filter(|b| *b == b'\n').count() + 1 - usize::from(text.ends_with('\n'))
        } as u64;
        let mut record = json!({"version":1,"id":id,"sessionId":self.session.as_str(),"kind":kind,"mimeType":"text/plain","encoding":"utf8","createdAt":self.timestamp,"sha256":sha256,"bytes":text.len(),"lineCount":line_count,"salientLines":salient,"producer":producer});
        if let Some(parent) = parent {
            record["parentEntryId"] = parent.into();
        }
        self.store
            .put_artifact(&ArtifactInput {
                origin: self.origin.clone(),
                session: self.session.as_str().into(),
                record: record.to_string().into_bytes(),
                bytes: text.as_bytes().to_vec(),
            })
            .map_err(domain_error)?;
        self.next += 1;
        Ok(ContextArtifact {
            id,
            kind: kind.into(),
            bytes: text.len() as u64,
            sha256,
            line_count: Some(line_count),
            salient_lines: salient.to_vec(),
        })
    }
    fn get(&self, id: &str) -> Result<ContextArtifact, DomainError> {
        let origin = self.origin_for(id)?;
        let raw = self
            .store
            .artifact_record(self.session.as_str(), &origin, id)
            .map_err(domain_error)?;
        let value: Value = serde_json::from_slice(&raw)
            .map_err(|_| DomainError::InvalidInput("context artifact".into()))?;
        // Reading a page also verifies the payload hash; metadata alone cannot prove recoverability.
        self.store
            .page(self.session.as_str(), &origin, id, 0, 1)
            .map_err(domain_error)?;
        serde_json::from_value(value)
            .map_err(|_| DomainError::InvalidInput("context text artifact".into()))
    }
    fn leased_ids(&self) -> Result<Vec<String>, DomainError> {
        if !self.features.evidence_leases {
            return Ok(vec![]);
        }
        Ok(self
            .store
            .leases(self.session.as_str())
            .map_err(domain_error)?
            .into_iter()
            .map(|l| l.artifact)
            .collect())
    }
}
