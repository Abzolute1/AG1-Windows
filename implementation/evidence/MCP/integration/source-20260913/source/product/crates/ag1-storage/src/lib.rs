//! Shadow-only persistence. No model, process-control, renderer or live TS authority.
mod artifacts;
pub mod context;
mod import;
mod journal;
mod persistence;
mod ports;
pub use artifacts::{ArtifactInput, ArtifactPage, DecodedCursor, Lease, SearchHit};
pub use import::{
    FileKind, FrozenFile, FrozenManifest, ImportStatus, MemoryView, normalize_journal,
};
pub use journal::{Ack, CommandAdmission, CommandResult, JournalEntry, JournalSnapshot};
pub use persistence::{
    Boundary, FaultInjector, Health, StorageError, StorageResult, Store, digest,
};

mod artifact_catalog;
pub use artifact_catalog::{CatalogPosition, CatalogRecord};

mod artifact_search;
pub use artifact_search::{ArtifactSearchMatch, ArtifactSearchResult, ArtifactSearchSnapshot};

mod session_lifecycle;
pub use session_lifecycle::{SessionBootstrap, SessionSeed};
mod runtime_recovery;
mod file_changes;
mod runtime_selection;
pub use runtime_recovery::InterruptedCommand;
pub use runtime_selection::{RuntimeSelection, SelectionAck, SelectionChange, StoredSelection};
mod client;

pub mod mcp;
pub mod mcp_private;
