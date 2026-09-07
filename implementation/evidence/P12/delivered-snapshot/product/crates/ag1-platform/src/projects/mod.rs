//! Host-admitted project operations. Model tools enter through target_tools::Service.
//! No target command receives the host environment or an AG1 command capability.
pub mod checks;
mod display;
pub mod godot;
pub mod preview;
mod runtime;
mod target;
pub use runtime::*;
pub use target::{CheckPort, ProjectPlan, ProjectPort, SharedCheck, SharedProject, VisualPort};
