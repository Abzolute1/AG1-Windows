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
pub use journal::{Ack, CommandResult, JournalEntry, JournalSnapshot};
pub use persistence::{
    Boundary, FaultInjector, Health, StorageError, StorageResult, Store, digest,
};
