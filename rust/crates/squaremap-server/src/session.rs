#![allow(dead_code)]
use rand::RngCore;
use squaremap_protocol::wire::{Ack, AckStatus, Envelope, ProtocolError, ProtocolErrorCode};
use squaremap_state::{ChunkCoordinate, Repository, WorldId};
use std::fmt;
use std::future::Future;
use std::panic::{self, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

const SESSION_ID_BYTES: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    New,
    Duplicate,
    Gap { expected: u64, actual: u64 },
}

#[derive(Clone, Debug)]
pub struct SessionCursor {
    expected: u64,
    last_accepted: Option<u64>,
    latched_gap: Option<(u64, u64)>,
}

impl SessionCursor {
    pub fn new() -> Self {
        Self {
            expected: 1,
            last_accepted: None,
            latched_gap: None,
        }
    }

    pub fn accept(&mut self, sequence: u64) -> Decision {
        let decision = self.evaluate(sequence);
        match decision {
            Decision::New => self.commit_new(sequence),
            Decision::Gap { expected, actual } => {
                self.latch_gap(expected, actual);
            }
            Decision::Duplicate => {}
        }
        decision
    }

    fn evaluate(&self, sequence: u64) -> Decision {
        if let Some((expected, _)) = self.latched_gap {
            return Decision::Gap {
                expected,
                actual: sequence,
            };
        }
        if self.last_accepted.is_some_and(|last| sequence <= last) {
            return Decision::Duplicate;
        }
        if sequence != self.expected {
            return Decision::Gap {
                expected: self.expected,
                actual: sequence,
            };
        }
        Decision::New
    }

    fn commit_new(&mut self, sequence: u64) {
        self.last_accepted = Some(sequence);
        self.expected = sequence.checked_add(1).unwrap_or(u64::MAX);
    }

    fn latch_gap(&mut self, expected: u64, actual: u64) -> Decision {
        self.latched_gap = Some((expected, actual));
        Decision::Gap { expected, actual }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    InvalidSessionId,
    HandlerFailed(String),
    HandlerPanicked(String),
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSessionId => {
                formatter.write_str("session ID must contain exactly 16 bytes")
            }
            Self::HandlerFailed(message) => write!(formatter, "durable handler failed: {message}"),
            Self::HandlerPanicked(message) => {
                write!(formatter, "durable handler panicked: {message}")
            }
        }
    }
}

impl std::error::Error for SessionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionOutcome {
    ack: Option<Ack>,
    protocol_error: Option<ProtocolError>,
}

impl SessionOutcome {
    pub fn ack(&self) -> Option<&Ack> {
        self.ack.as_ref()
    }
    pub fn protocol_error(&self) -> Option<&ProtocolError> {
        self.protocol_error.as_ref()
    }
}

pub struct Session {
    bridge_id: [u8; SESSION_ID_BYTES],
    session_id: [u8; SESSION_ID_BYTES],
    cursor: SessionCursor,
}

impl Session {
    pub fn new(bridge_id: [u8; SESSION_ID_BYTES], session_id: [u8; SESSION_ID_BYTES]) -> Self {
        Self {
            bridge_id,
            session_id,
            cursor: SessionCursor::new(),
        }
    }

    #[cfg(test)]
    pub fn authenticated(bridge_id: [u8; SESSION_ID_BYTES]) -> Self {
        Self::new(bridge_id, random_session_id())
    }

    pub fn from_ids(bridge_id: &[u8], session_id: &[u8]) -> Result<Self, SessionError> {
        let bridge_id: [u8; SESSION_ID_BYTES] = bridge_id
            .try_into()
            .map_err(|_| SessionError::InvalidSessionId)?;
        let session_id: [u8; SESSION_ID_BYTES] = session_id
            .try_into()
            .map_err(|_| SessionError::InvalidSessionId)?;
        Ok(Self::new(bridge_id, session_id))
    }
    pub fn replace_bridge_id(
        &mut self,
        bridge_id: [u8; SESSION_ID_BYTES],
    ) -> Result<(), SessionError> {
        if bridge_id.iter().all(|byte| *byte == 0) {
            return Err(SessionError::InvalidSessionId);
        }
        self.bridge_id = bridge_id;
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn expected_sequence(&self) -> u64 {
        self.cursor.expected
    }
    pub fn bridge_id(&self) -> &[u8] {
        &self.bridge_id
    }

    pub fn session_id(&self) -> &[u8] {
        &self.session_id
    }
    pub fn cursor_mut(&mut self) -> &mut SessionCursor {
        &mut self.cursor
    }
    pub async fn process<F, Fut>(
        &mut self,
        envelope: Envelope,
        handler: F,
    ) -> Result<SessionOutcome, SessionError>
    where
        F: FnOnce(&Envelope) -> Fut,
        Fut: Future<Output = Result<(), SessionError>>,
    {
        self.process_with_disposition(envelope, handler, |_| None)
            .await
    }

    pub async fn process_with_disposition<F, Fut, R>(
        &mut self,
        envelope: Envelope,
        handler: F,
        rejected: R,
    ) -> Result<SessionOutcome, SessionError>
    where
        F: FnOnce(&Envelope) -> Fut,
        Fut: Future<Output = Result<(), SessionError>>,
        R: FnOnce(&Envelope) -> Option<ProtocolError>,
    {
        if envelope.session_id.as_slice() != self.session_id {
            return Ok(SessionOutcome {
                ack: None,
                protocol_error: Some(ProtocolError {
                    code: ProtocolErrorCode::InvalidMessage as i32,
                    message: "authenticated session ID does not match; full bootstrap replacement required".to_string(),
                    fatal: true,
                    offending_sequence: envelope.sequence,
                    config_revision: 0,
                }),
            });
        }
        match self.cursor.evaluate(envelope.sequence) {
            Decision::Duplicate => Ok(SessionOutcome {
                ack: Some(Ack {
                    acknowledged_sequence: envelope.sequence,
                    status: AckStatus::Duplicate as i32,
                    message: "duplicate sequence already durably applied".to_string(),
                }),
                protocol_error: None,
            }),
            Decision::Gap { expected, actual } => {
                self.cursor.latch_gap(expected, actual);
                Ok(SessionOutcome {
                    ack: None,
                    protocol_error: Some(ProtocolError {
                        code: ProtocolErrorCode::InvalidMessage as i32,
                        message: format!(
                            "sequence gap: expected {expected}, actual {actual}; full bootstrap replacement required"
                        ),
                        fatal: true,
                        offending_sequence: actual,
                        config_revision: 0,
                    }),
                })
            }
            Decision::New => {
                if let Some(error) = rejected(&envelope) {
                    self.cursor.commit_new(envelope.sequence);
                    return Ok(SessionOutcome {
                        ack: Some(Ack {
                            acknowledged_sequence: envelope.sequence,
                            status: AckStatus::Accepted as i32,
                            message: "durably consumed after semantic rejection".to_string(),
                        }),
                        protocol_error: Some(error),
                    });
                }
                match PanicSafe::new(async { handler(&envelope).await }).await {
                    Ok(Ok(())) => {
                        self.cursor.commit_new(envelope.sequence);
                        Ok(SessionOutcome {
                            ack: Some(Ack {
                                acknowledged_sequence: envelope.sequence,
                                status: AckStatus::Accepted as i32,
                                message: "durably applied".to_string(),
                            }),
                            protocol_error: None,
                        })
                    }
                    Ok(Err(error)) => Err(error),
                    Err(panic) => Err(SessionError::HandlerPanicked(panic_message(panic))),
                }
            }
        }
    }
    pub async fn process_with_repository(
        &mut self,
        envelope: Envelope,
        repository: Arc<Repository>,
    ) -> Result<SessionOutcome, SessionError> {
        let payload = envelope.payload.clone();
        let bridge_id = self.bridge_id;
        let session_id = envelope.session_id.clone();
        let sequence = envelope.sequence;
        self.process(envelope, move |_| {
            let repository = repository.clone();
            async move {
                let Some(squaremap_protocol::wire::envelope::Payload::ChunkDirty(dirty)) = payload
                else {
                    return Err(SessionError::HandlerFailed(
                        "new payload is not a durable dirty event".into(),
                    ));
                };
                let world = dirty.world.ok_or_else(|| {
                    SessionError::HandlerFailed("chunk dirty event has no world".into())
                })?;
                let coordinate = dirty.coordinate.ok_or_else(|| {
                    SessionError::HandlerFailed("chunk dirty event has no coordinate".into())
                })?;
                repository
                    .mark_dirty(
                        &WorldId::new(world.namespace, world.value, world.epoch),
                        ChunkCoordinate {
                            x: coordinate.x,
                            z: coordinate.z,
                        },
                        dirty.revision,
                        &bridge_id,
                        &session_id,
                        sequence,
                    )
                    .await
                    .map_err(|error| SessionError::HandlerFailed(error.to_string()))?;
                Ok(())
            }
        })
        .await
    }
}

fn random_session_id() -> [u8; SESSION_ID_BYTES] {
    let mut session_id = [0_u8; SESSION_ID_BYTES];
    rand::rng().fill_bytes(&mut session_id);
    session_id[6] = (session_id[6] & 0x0f) | 0x40;
    session_id[8] = (session_id[8] & 0x3f) | 0x80;
    session_id
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    panic.downcast_ref::<&str>().map_or_else(
        || {
            panic
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_else(|| "unknown panic".to_string())
        },
        |message| (*message).to_string(),
    )
}

struct PanicSafe<F: Future> {
    future: Pin<Box<F>>,
}

impl<F: Future> PanicSafe<F> {
    fn new(future: F) -> Self {
        Self {
            future: Box::pin(future),
        }
    }
}
impl<F: Future> Unpin for PanicSafe<F> {}

impl<F: Future> Future for PanicSafe<F> {
    type Output = Result<F::Output, Box<dyn std::any::Any + Send>>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let future = self.get_mut().future.as_mut();
        match panic::catch_unwind(AssertUnwindSafe(|| future.poll(context))) {
            Ok(Poll::Ready(output)) => Poll::Ready(Ok(output)),
            Ok(Poll::Pending) => Poll::Pending,
            Err(panic) => Poll::Ready(Err(panic)),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use squaremap_protocol::wire::{ControlKind, ControlRequest, envelope};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn control(sequence: u64) -> Envelope {
        Envelope {
            protocol_major: 1,
            protocol_minor: 0,
            session_id: vec![1; SESSION_ID_BYTES],
            sequence,
            correlation_id: sequence,
            payload: Some(envelope::Payload::ControlRequest(ControlRequest {
                kind: ControlKind::Health as i32,
                ..Default::default()
            })),
        }
    }

    #[tokio::test]
    async fn duplicate_sequence_does_not_run_mutating_handler() {
        let mut session = Session::new([1; SESSION_ID_BYTES], [1; SESSION_ID_BYTES]);
        let calls = Arc::new(AtomicUsize::new(0));
        let first_calls = calls.clone();
        let first = session
            .process(control(1), move |_| {
                first_calls.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            })
            .await
            .unwrap();
        assert_eq!(
            first.ack().unwrap().status,
            squaremap_protocol::wire::AckStatus::Accepted as i32
        );
        let duplicate_calls = calls.clone();
        let duplicate = session
            .process(control(1), move |_| {
                duplicate_calls.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            })
            .await
            .unwrap();
        assert_eq!(
            duplicate.ack().unwrap().status,
            squaremap_protocol::wire::AckStatus::Duplicate as i32
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn sequence_gap_rejects_without_running_handler() {
        let mut session = Session::new([1; SESSION_ID_BYTES], [1; SESSION_ID_BYTES]);
        let calls = Arc::new(AtomicUsize::new(0));
        let gap_calls = calls.clone();
        let outcome = session
            .process(control(2), move |_| {
                gap_calls.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            })
            .await
            .unwrap();
        assert!(outcome.protocol_error().is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
