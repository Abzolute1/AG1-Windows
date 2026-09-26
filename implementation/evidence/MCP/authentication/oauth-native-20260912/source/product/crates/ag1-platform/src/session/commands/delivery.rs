//! Slow readers hold bounded transient progress, never a writer or model slot.
//! Durable operation events are replayed before progress; a poll must not move
//! past unseen events merely because a transient frame is also available.
use super::{Book, SessionCommands, domain, w};
use crate::session::{Listener, SessionEvent};
use ag1_core::{
    DomainError,
    provider::{BlockKind, ProviderEvent},
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Mutex},
};

const MAX_SUBSCRIBERS: usize = 64;
struct Subscriber {
    cursor: w::Cursor,
    progress: VecDeque<(Arc<w::Delivery>, usize)>,
    bytes: usize,
    gap: bool,
}
#[derive(Default)]
pub(super) struct State {
    serial: u64,
    tick: u64,
    subscribers: BTreeMap<w::SubscriptionId, Subscriber>,
}
impl SessionCommands {
    pub fn subscribe(&self) -> Result<w::Subscription, w::ProtocolError> {
        let _activity = self.runtime().enter().map_err(domain)?;
        let mut book = self.book().map_err(domain)?;
        if book.delivery.subscribers.len() == MAX_SUBSCRIBERS {
            return Err(w::ProtocolError::Capacity {
                resource: "subscribers".into(),
            });
        }
        let serial = book
            .delivery
            .serial
            .checked_add(1)
            .ok_or(w::ProtocolError::Capacity {
                resource: "subscription_ids".into(),
            })?;
        let identity = ag1_storage::digest(
            serde_json::json!([self.scope, serial])
                .to_string()
                .as_bytes(),
        );
        let id: w::SubscriptionId = format!("native-sub-{identity}").try_into().map_err(|_| {
            w::ProtocolError::Capacity {
                resource: "subscription_ids".into(),
            }
        })?;
        let snapshot = self.snapshot_at(&book).map_err(domain)?;
        let cursor = w::Cursor {
            scope: self.scope.clone(),
            after: snapshot.watermark.clone(),
        };
        book.delivery.subscribers.insert(
            id.clone(),
            Subscriber {
                cursor,
                progress: VecDeque::new(),
                bytes: 0,
                gap: false,
            },
        );
        book.delivery.serial = serial;
        Ok(w::Subscription { id, snapshot })
    }
    pub fn unsubscribe(&self, id: &w::SubscriptionId) -> Result<(), DomainError> {
        self.book()?.delivery.subscribers.remove(id);
        Ok(())
    }
    pub fn queued_progress_bytes(&self, id: &w::SubscriptionId) -> Result<usize, w::ProtocolError> {
        self.book()
            .map_err(domain)?
            .delivery
            .subscribers
            .get(id)
            .map(|s| s.bytes)
            .ok_or(w::ProtocolError::NotFound)
    }
    pub fn poll(&self, id: &w::SubscriptionId) -> Result<w::Delivery, w::ProtocolError> {
        let mut book = self.book().map_err(domain)?;
        let subscriber = book
            .delivery
            .subscribers
            .get(id)
            .ok_or(w::ProtocolError::NotFound)?;
        let replay = if subscriber.gap {
            w::Delivery::Gap {
                snapshot: self.snapshot_at(&book).map_err(domain)?,
            }
        } else {
            self.replay_at(&book, &subscriber.cursor).map_err(domain)?
        };
        let live = book
            .active
            .as_ref()
            .filter(|a| !a.recovery)
            .map(|a| a.operation.clone());
        let subscriber = book
            .delivery
            .subscribers
            .get_mut(id)
            .expect("registered under book lock");
        match &replay {
            w::Delivery::Gap { snapshot } => {
                subscriber.cursor.after = snapshot.watermark.clone();
                subscriber.progress.clear();
                subscriber.bytes = 0;
                subscriber.gap = false;
            }
            w::Delivery::Events { events, cursor, .. } => {
                subscriber.cursor = cursor.clone();
                if !events.is_empty() {
                    return Ok(replay);
                }
                while let Some((frame, bytes)) = subscriber.progress.pop_front() {
                    subscriber.bytes -= bytes;
                    if let w::Delivery::Progress { operation, .. } = frame.as_ref()
                        && live.as_ref() == Some(operation)
                    {
                        return Ok(Arc::unwrap_or_clone(frame));
                    }
                }
            }
            _ => unreachable!("replay returns events or gap"),
        }
        Ok(replay)
    }
    /// Host-only transient progress. It carries no receipt or permission to
    /// execute, and is discarded after terminal settlement or consumer loss.
    pub fn progress(
        &self,
        operation: &w::OperationId,
        text: String,
    ) -> Result<(), w::ProtocolError> {
        let mut book = self.book().map_err(domain)?;
        if !book
            .active
            .as_ref()
            .is_some_and(|active| active.operation == *operation && !active.recovery)
        {
            return Err(w::ProtocolError::NotFound);
        }
        if text.len() > w::MAX_ENVELOPE {
            return Err(w::ProtocolError::Oversize {
                limit: w::MAX_ENVELOPE as u32,
                use_artifact: true,
            });
        }
        let tick = book
            .delivery
            .tick
            .checked_add(1)
            .ok_or(w::ProtocolError::Capacity {
                resource: "progress_ticks".into(),
            })?;
        let frame = w::Delivery::Progress {
            scope: self.scope.clone(),
            operation: operation.clone(),
            tick: w::Sequence::new(tick),
            text,
        };
        let bytes = w::encode(&w::WireMessage::Delivery(frame.clone()))?.len();
        let frame = Arc::new(frame);
        book.delivery.tick = tick;
        for subscriber in book.delivery.subscribers.values_mut() {
            if subscriber.bytes + bytes > w::MAX_SUBSCRIBER_BYTES {
                subscriber.progress.clear();
                subscriber.bytes = 0;
                subscriber.gap = true;
            }
            subscriber.progress.push_back((frame.clone(), bytes));
            subscriber.bytes += bytes;
        }
        Ok(())
    }
    fn progress_gap(&self) {
        if let Ok(mut book) = self.book() {
            for subscriber in book.delivery.subscribers.values_mut() {
                subscriber.progress.clear();
                subscriber.bytes = 0;
                subscriber.gap = true;
            }
        }
    }
}
pub(super) fn terminal(book: &mut Book, operation: &w::OperationId) {
    for subscriber in book.delivery.subscribers.values_mut() {
        subscriber.progress.retain(|(frame, _)| !matches!(frame.as_ref(), w::Delivery::Progress { operation: found, .. } if found == operation));
        subscriber.bytes = subscriber.progress.iter().map(|(_, bytes)| bytes).sum();
    }
}
pub(super) struct Relay<'a> {
    commands: &'a SessionCommands,
    operation: &'a w::OperationId,
    listener: &'a dyn Listener,
    text_keys: Mutex<BTreeSet<String>>,
}
impl<'a> Relay<'a> {
    pub(super) fn new(
        commands: &'a SessionCommands,
        operation: &'a w::OperationId,
        listener: &'a dyn Listener,
    ) -> Self {
        Self {
            commands,
            operation,
            listener,
            text_keys: Mutex::new(BTreeSet::new()),
        }
    }
}
impl Listener for Relay<'_> {
    fn on_event(&self, event: &SessionEvent) {
        if let SessionEvent::Provider(provider) = event {
            let mut keys = self
                .text_keys
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match provider {
                ProviderEvent::Begin {
                    key,
                    kind,
                    redacted,
                    ..
                } => {
                    keys.remove(key);
                    if *kind == BlockKind::Text && !redacted && keys.len() < 128 {
                        keys.insert(key.clone());
                    }
                }
                ProviderEvent::Delta { key, text } if keys.contains(key) => {
                    if self
                        .commands
                        .progress(self.operation, text.clone())
                        .is_err()
                    {
                        self.commands.progress_gap();
                    }
                }
                // v1 progress has no replace-content operation. Explicitly
                // resync instead of appending a replacement as another delta.
                ProviderEvent::Replace {
                    key,
                    signature: false,
                    ..
                } if keys.remove(key) => self.commands.progress_gap(),
                ProviderEvent::End { key } => {
                    keys.remove(key);
                }
                ProviderEvent::Stop(_) | ProviderEvent::Fail(_) => keys.clear(),
                _ => (),
            }
        }
        // NativeSession's listener boundary catches client panics. Publication
        // above happens first, so a bad client cannot discard native progress.
        self.listener.on_event(event);
    }
}
