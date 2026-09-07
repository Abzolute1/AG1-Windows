//! Native ownership for active/preserved autonomous workspaces. The composition
//! owner admits a child of its dedicated workspace storage root; model/client
//! arguments cannot select a deletion path. P10 owns session persistence and
//! quiesces all jobs before preserving; P12 supplies command routing/presentation.
use crate::{
    resource_fs::ScanControl,
    workspace::{PinnedDirectory, Workspace},
};
use ag1_core::DomainError;
use std::{
    path::{Component, Path},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Empty,
    Active,
    Preserved,
    Uncertain,
}

pub struct View {
    pub state: State,
    pub root: Option<String>,
    pub output: String,
    pub warning: Option<String>,
    pub dim: bool,
}
struct Owned {
    parent: Arc<Workspace>,
    pin: Option<PinnedDirectory>,
    root: String,
}
pub struct Lifecycle {
    state: State,
    owned: Option<Owned>,
}
impl Default for Lifecycle {
    fn default() -> Self {
        Self {
            state: State::Empty,
            owned: None,
        }
    }
}
impl Lifecycle {
    /// Host-only capability admission; this does not copy, execute or delete.
    /// The parent must be AG1-owned workspace storage, never the source project.
    pub fn admit(
        &mut self,
        parent: Arc<Workspace>,
        child: &str,
        active: bool,
    ) -> Result<(), DomainError> {
        if self.state != State::Empty {
            return Err(DomainError::Conflict);
        }
        if child.is_empty()
            || Path::new(child).components().count() != 1
            || !matches!(
                Path::new(child).components().next(),
                Some(Component::Normal(_))
            )
        {
            return Err(DomainError::PermissionDenied);
        }
        let pin = parent.pin_directory(child)?;
        let root = parent.root().join(child).to_string_lossy().into_owned();
        self.owned = Some(Owned {
            parent,
            pin: Some(pin),
            root,
        });
        self.state = if active {
            State::Active
        } else {
            State::Preserved
        };
        Ok(())
    }
    /// Call only after the session owner has stopped/joined every workspace job.
    pub fn preserve_quiescent(&mut self) -> Result<(), DomainError> {
        if self.state != State::Active {
            return Err(DomainError::Conflict);
        }
        self.owned
            .as_ref()
            .and_then(|o| o.pin.as_ref())
            .ok_or(DomainError::Conflict)?
            .verify_identity()?;
        self.state = State::Preserved;
        Ok(())
    }
    pub fn view(&self) -> View {
        let output = match self.state {
            State::Empty => {
                "No autonomous workspace is active or preserved in this session.".into()
            }
            State::Active => "Autonomous workspace is active.".into(),
            State::Preserved => "Autonomous workspace is preserved.".into(),
            State::Uncertain => "Autonomous workspace cleanup requires recovery.".into(),
        };
        View {
            state: self.state,
            root: self.owned.as_ref().map(|o| o.root.clone()),
            output,
            warning: None,
            dim: self.state == State::Empty,
        }
    }
    /// Invoked by an authorized host command. P03 pins the exact admitted inode,
    /// detaches atomically and never follows descendants' links. Once cleanup has
    /// started, a failure retains uncertainty instead of re-adopting the pathname.
    pub fn discard(&mut self, control: &ScanControl<'_>) -> Result<View, DomainError> {
        control.check()?;
        let warning = match self.state {
            State::Active => Some("Cannot discard the autonomous workspace while it is active."),
            State::Empty => Some("No preserved autonomous workspace to discard."),
            State::Uncertain => return Err(DomainError::DurabilityUnknown),
            State::Preserved => None,
        };
        if let Some(warning) = warning {
            let mut view = self.view();
            view.warning = Some(warning.into());
            return Ok(view);
        }
        let owned = self.owned.as_mut().ok_or(DomainError::Conflict)?;
        owned
            .pin
            .as_ref()
            .ok_or(DomainError::Conflict)?
            .verify_identity()?;
        control.check()?;
        let pin = owned.pin.take().ok_or(DomainError::Conflict)?;
        self.state = State::Uncertain;
        owned
            .parent
            .remove_directory_tree_checked(pin, || control.check())?;
        let root = owned.root.clone();
        self.owned = None;
        self.state = State::Empty;
        Ok(View {
            output: format!("Discarded preserved autonomous workspace: {root}"),
            dim: true,
            ..self.view()
        })
    }
}
