//! Tool definitions and execution contracts are independent of renderers and host adapters.
pub mod approval;
pub mod arguments;
pub mod diff;
pub mod edit;
pub mod policy;
pub mod registry;
pub mod result;
pub mod schema;
pub mod truncate;

pub mod executor;
pub mod file;
pub mod git_diff;
pub mod jobs;
pub mod list;
pub mod path;
pub mod preview;
pub mod read;
pub mod recovery;
pub mod research;
pub mod search;
pub mod shell;
pub mod web_presentation;
pub mod websearch;

pub mod targets;

pub mod memory;
pub mod repo;
pub mod task_state;

pub mod artifact_scan;

pub mod submit_plan;
pub mod todo;

pub mod observation;
/// Derived journal records and the tool presentation published after their acknowledgement.
pub struct JournalToolPlan {
    pub records: Vec<serde_json::Value>,
    pub result: serde_json::Value,
}

pub mod episode;
