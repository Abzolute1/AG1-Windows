use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QueueMode {
    All,
    #[default]
    OneAtATime,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Prompt,
    Steering,
    FollowUp,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    input: w::PromptInput,
}
pub(super) fn input(command: &w::Command) -> Result<(Kind, w::PromptInput), w::ProtocolError> {
    match command {
        w::Command::Prompt { input } => Ok((Kind::Prompt, input.clone())),
        w::Command::Session { action, args } if action == "steer" || action == "follow_up" => {
            let input: Input =
                serde_json::from_value(args.clone()).map_err(|_| w::ProtocolError::Malformed {
                    detail: "queued message input".into(),
                })?;
            Ok((
                if action == "steer" {
                    Kind::Steering
                } else {
                    Kind::FollowUp
                },
                input.input,
            ))
        }
        _ => Err(unavailable("native_session_action")),
    }
}

impl SessionCommands {
    /// The native loop calls this only at a safe message boundary, after all
    /// current tool outcomes acknowledge. Follow-ups are polled only at stop.
    pub(crate) async fn deliver_messages(
        self: &Arc<Self>,
        follow_up: bool,
        cancel: &AtomicBool,
        listener: &dyn Listener,
    ) -> Result<usize, DomainError> {
        let kind = if follow_up {
            Kind::FollowUp
        } else {
            Kind::Steering
        };
        let mode = if follow_up {
            self.settings.follow_up_mode
        } else {
            self.settings.steering_mode
        };
        let queued: Vec<_> = {
            let book = self.book()?;
            if book.queue.iter().any(|q| q.claimed) {
                return Err(DomainError::DurabilityUnknown);
            }
            book.queue
                .iter()
                .filter(|q| q.kind == kind)
                .map(|q| q.key.clone())
                .collect()
        };
        let mut delivered = 0;
        for key in queued {
            if mode == QueueMode::OneAtATime && delivered > 0 {
                break;
            }
            if cancel.load(Ordering::Acquire) {
                return Err(DomainError::Cancelled);
            }
            let queued = {
                let mut book = self.book()?;
                let queued = book
                    .queue
                    .iter_mut()
                    .find(|q| q.key == key)
                    .ok_or(DomainError::Conflict)?;
                if queued.claimed {
                    return Err(DomainError::Conflict);
                }
                queued.claimed = true;
                queued.clone()
            };
            let now = self.clock.now_ms();
            let timestamp = i64::try_from(now)
                .ok()
                .and_then(chrono::DateTime::from_timestamp_millis)
                .ok_or_else(|| DomainError::InvalidInput("queue clock".into()))?
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            let status = if queued.cancelled.load(Ordering::Acquire) {
                w::OperationStatus::Cancelled
            } else {
                let owner = self.clone();
                let input = queued.input.clone();
                let message = self
                    .runtime()
                    .pool
                    .submit(move || owner.input_message(&input, now))
                    .await?;
                let text = ag1_core::context::hygiene::content(&message, "\n");
                let id = format!("queued-{}", ag1_storage::digest(key.as_bytes()));
                let action = loop {
                    self.runtime().wait_for_commit().await?;
                    let mut state = self.runtime().state()?;
                    if state.pending_commit() {
                        continue;
                    }
                    break state.append_message(&id, &id, &timestamp, message.clone())?;
                };
                self.runtime().commit(action, listener).await?;
                let task_id = format!("{id}-task");
                let action = loop {
                    self.runtime().wait_for_commit().await?;
                    let mut state = self.runtime().state()?;
                    if state.pending_commit() {
                        continue;
                    }
                    break state.capture_instruction(&task_id, &task_id, &timestamp, &text)?;
                };
                if let Some(action) = action {
                    self.runtime().commit(action, listener).await?;
                }
                self.runtime().repetition.reset()?;
                delivered += 1;
                w::OperationStatus::Completed
            };
            // Completion of a steering/follow-up command means acknowledged
            // delivery. The parent operation still owns subsequent generation.
            let done = w::OperationView {
                operation: queued.operation.clone(),
                status,
            };
            let evidence = {
                let state = self.runtime().state()?;
                serde_json::to_vec(&Terminal {
                    version: 1,
                    operation: done.clone(),
                    journal_revision: state.journal_revision(),
                    context_epoch: state.stamp().epoch(),
                    execution_settled: true,
                    error: None,
                    generations: None,
                })
                .map_err(|_| DomainError::Conflict)?
            };
            let owner = self.clone();
            let activity = self.runtime().retain_work()?;
            self.administrative
                .submit(move || {
                    let _activity = activity;
                    owner
                        .runtime()
                        .store
                        .settle_intent(&queued.key, &queued.intent, &evidence)?;
                    let mut book = owner.book()?;
                    owner.publish(&mut book, done.clone())?;
                    book.last_terminal = Some(done);
                    book.queue.retain(|q| q.key != queued.key);
                    Ok(())
                })
                .await?;
        }
        Ok(delivered)
    }
}
