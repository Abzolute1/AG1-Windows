//! Session orchestration. Domain ownership is independent of desktop/TUI types.
//! Platform composition supplies real provider/tool/storage work; this module
//! never creates a process or changes a working directory.
pub mod boundary;
pub mod completion;
pub mod context;
pub mod health;
pub mod messages;
pub mod planning;
pub mod policy;
pub mod prompt;
pub mod recovery;
pub mod repetition;
pub mod retry;
pub mod session;
pub mod subagent;
pub mod tree;
pub mod verification;
pub mod xml;
pub use session::SessionRuntime;
