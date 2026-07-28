use crate::event::Event;
use async_trait::async_trait;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct OperatorError {
    pub operator_id: String,
    pub message: String,
}

impl std::fmt::Display for OperatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "operator {}: {}", self.operator_id, self.message)
    }
}

/// A single parsing/transform step. Implementations never lose `event.raw`;
/// they read from and write to `body`/`attributes`/`resource` via `FieldRef`.
#[async_trait]
pub trait Operator: Send + Sync {
    fn id(&self) -> &str;

    async fn process(&self, event: Event) -> Result<Event, (Event, OperatorError)>;
}

/// What happens when a step's `process` returns an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnError {
    /// Discard the event entirely (only for genuinely disposable garbage).
    Drop,
    /// Default: record the error on `event.meta.errors` and continue the
    /// chain with the event unchanged by this step. Never silently loses
    /// data.
    Pass,
    /// Mark `event.meta.dead_letter` and stop the chain; the runner is
    /// expected to append the event to a dead-letter sink.
    DeadLetter,
}

#[derive(Debug)]
pub struct OperatorChain {
    steps: Vec<(Arc<dyn Operator>, OnError)>,
}

impl std::fmt::Debug for dyn Operator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Operator({})", self.id())
    }
}

impl OperatorChain {
    pub fn new(steps: Vec<(Arc<dyn Operator>, OnError)>) -> Self {
        Self { steps }
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Runs the chain. Returns `None` if the event was dropped or
    /// dead-lettered (in the dead-letter case, `event.meta.dead_letter`
    /// is left `true` on the returned-via-callback event so callers doing
    /// their own dead-letter write can distinguish it; the simple `run`
    /// API here just signals "stop" via `None`).
    pub async fn run(&self, mut event: Event) -> Option<Event> {
        for (op, on_error) in &self.steps {
            match op.process(event).await {
                Ok(ev) => event = ev,
                Err((mut ev, err)) => match on_error {
                    OnError::Drop => {
                        tracing::debug!(operator = %op.id(), %err, "dropping event");
                        return None;
                    }
                    OnError::Pass => {
                        tracing::debug!(operator = %op.id(), %err, "operator error, passing event through");
                        ev.meta.errors.push(err.to_string());
                        event = ev;
                    }
                    OnError::DeadLetter => {
                        ev.meta.dead_letter = true;
                        ev.meta.errors.push(err.to_string());
                        return Some(ev);
                    }
                },
            }
        }
        Some(event)
    }
}
