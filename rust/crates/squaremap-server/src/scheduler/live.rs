use super::{BridgeError, SnapshotBridge, SnapshotReply, SnapshotRequest};
use crate::dirty_resync::{
    MAX_ENUMERATION_ITEMS, WorldEnumerationError, validate_enumeration_completion,
    validate_enumeration_item, validate_enumeration_page, validate_enumeration_request,
};
use crate::snapshot_client::{SnapshotClient, SnapshotClientError, SnapshotOutcome};
use async_trait::async_trait;
use squaremap_protocol::wire::{Envelope, WorldEnumerationRequest, WorldIdentity, envelope};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc, oneshot};

pub struct OutboundSnapshotRequest {
    pub request: SnapshotRequest,
    pub response: oneshot::Sender<Result<SnapshotReply, BridgeError>>,
    pub cancellation: oneshot::Receiver<()>,
}

pub struct OutboundEnumerationRequest {
    pub request: WorldEnumerationRequest,
    pub response: oneshot::Sender<Result<Vec<squaremap_state::ChunkCoordinate>, BridgeError>>,
    pub cancellation: oneshot::Receiver<()>,
}

#[derive(Clone)]
pub struct LiveSnapshotBridge {
    sender: mpsc::Sender<OutboundSnapshotRequest>,
    enumeration_sender: mpsc::Sender<OutboundEnumerationRequest>,
}
impl LiveSnapshotBridge {
    pub fn channel(capacity: usize) -> (Arc<Self>, mpsc::Receiver<OutboundSnapshotRequest>) {
        let (sender, receiver) = mpsc::channel(capacity);
        let (enumeration_sender, _) = mpsc::channel(capacity);
        (
            Arc::new(Self {
                sender,
                enumeration_sender,
            }),
            receiver,
        )
    }
    pub fn channel_with_enumeration(
        capacity: usize,
    ) -> (
        Arc<Self>,
        mpsc::Receiver<OutboundSnapshotRequest>,
        mpsc::Receiver<OutboundEnumerationRequest>,
    ) {
        let (sender, receiver) = mpsc::channel(capacity);
        let (enumeration_sender, enumeration_receiver) = mpsc::channel(capacity);
        (
            Arc::new(Self {
                sender,
                enumeration_sender,
            }),
            receiver,
            enumeration_receiver,
        )
    }
}

struct CancellationGuard {
    sender: Option<oneshot::Sender<()>>,
}
impl Drop for CancellationGuard {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(());
        }
    }
}

#[async_trait]
impl SnapshotBridge for LiveSnapshotBridge {
    async fn request(&self, request: SnapshotRequest) -> Result<SnapshotReply, BridgeError> {
        let (response, result) = oneshot::channel();
        let (cancel_sender, cancellation) = oneshot::channel();
        let _guard = CancellationGuard {
            sender: Some(cancel_sender),
        };
        self.sender
            .send(OutboundSnapshotRequest {
                request,
                response,
                cancellation,
            })
            .await
            .map_err(|_| BridgeError::Transient("bridge connection closed".into()))?;
        result
            .await
            .map_err(|_| BridgeError::Transient("bridge connection closed".into()))?
    }
    async fn enumerate_world(
        &self,
        world: &squaremap_state::WorldId,
    ) -> Result<Vec<squaremap_state::ChunkCoordinate>, BridgeError> {
        let request = WorldEnumerationRequest {
            session_id: Vec::new(),
            config_revision: 0,
            enumeration_id: 0,
            world: Some(WorldIdentity {
                namespace: world.namespace.clone(),
                value: world.value.clone(),
                epoch: world.epoch,
            }),
            max_items: MAX_ENUMERATION_ITEMS,
            bridge_id: Vec::new(),
            page_index: 0,
        };
        let (response, result) = oneshot::channel();
        let (cancel_sender, cancellation) = oneshot::channel();
        let _guard = CancellationGuard {
            sender: Some(cancel_sender),
        };
        self.enumeration_sender
            .send(OutboundEnumerationRequest {
                request,
                response,
                cancellation,
            })
            .await
            .map_err(|_| BridgeError::Transient("bridge connection closed".into()))?;
        result
            .await
            .map_err(|_| BridgeError::Transient("bridge connection closed".into()))?
    }
}

pub struct EnumerationDispatcher {
    state: Arc<Mutex<EnumerationState>>,
    activation: Arc<tokio::sync::RwLock<Option<(Vec<u8>, Vec<u8>, u64)>>>,
}
struct EnumerationState {
    pending:
        HashMap<u64, oneshot::Sender<Result<Vec<squaremap_state::ChunkCoordinate>, BridgeError>>>,
    requests: HashMap<
        u64,
        (
            WorldEnumerationRequest,
            Vec<squaremap_state::ChunkCoordinate>,
            u32,
        ),
    >,
    outbound: Option<mpsc::Sender<Envelope>>,
    aborted: bool,
    cancellations: HashMap<u64, tokio::task::JoinHandle<()>>,
}
impl EnumerationDispatcher {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(EnumerationState {
                pending: HashMap::new(),
                requests: HashMap::new(),
                outbound: None,
                aborted: false,
                cancellations: HashMap::new(),
            })),
            activation: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }
    pub async fn activate(&self, bridge_id: Vec<u8>, session_id: Vec<u8>, config_revision: u64) {
        *self.activation.write().await = Some((bridge_id, session_id, config_revision));
    }
    pub async fn deactivate(&self) {
        *self.activation.write().await = None;
        self.abort("bridge connection closed").await;
    }
    pub async fn set_outbound(&self, outbound: mpsc::Sender<Envelope>) {
        self.state.lock().await.outbound = Some(outbound);
    }
    pub async fn begin(
        &self,
        command: OutboundEnumerationRequest,
        session_id: &[u8],
        bridge_id: &[u8],
        config_revision: u64,
        next_id: u64,
    ) -> Result<Envelope, WorldEnumerationError> {
        let OutboundEnumerationRequest {
            mut request,
            response,
            cancellation,
        } = command;
        if let Some((active_bridge, active_session, active_revision)) =
            self.activation.read().await.clone()
        {
            if active_bridge != bridge_id
                || active_session != session_id
                || active_revision != config_revision
            {
                let _ = response.send(Err(BridgeError::Transient(
                    "bridge activation changed".into(),
                )));
                return Err(WorldEnumerationError::InvalidSession);
            }
        }
        request.session_id = session_id.to_vec();
        request.bridge_id = bridge_id.to_vec();
        request.config_revision = config_revision;
        request.enumeration_id = next_id;
        if let Err(error) = validate_enumeration_page(&request, 0).and_then(|_| {
            validate_enumeration_request(
                &request,
                bridge_id,
                session_id,
                config_revision,
                MAX_ENUMERATION_ITEMS,
            )
        }) {
            let _ = response.send(Err(BridgeError::Permanent(format!("{error:?}"))));
            return Err(error);
        }
        let cancellation_state = Arc::clone(&self.state);
        let mut state = self.state.lock().await;
        if state.aborted {
            let _ = response.send(Err(BridgeError::Transient(
                "bridge connection closed".into(),
            )));
            return Err(WorldEnumerationError::InvalidSession);
        }
        state.pending.insert(next_id, response);
        state
            .requests
            .insert(next_id, (request.clone(), Vec::new(), 0));
        let cancellation_task = tokio::spawn(async move {
            if cancellation.await.is_ok() {
                let mut state = cancellation_state.lock().await;
                state.requests.remove(&next_id);
                if let Some(response) = state.pending.remove(&next_id) {
                    let _ = response.send(Err(BridgeError::Transient(
                        "enumeration request cancelled".into(),
                    )));
                }
                state.cancellations.remove(&next_id);
            }
        });
        state.cancellations.insert(next_id, cancellation_task);
        Ok(Envelope {
            protocol_major: 1,
            protocol_minor: 0,
            session_id: session_id.to_vec(),
            correlation_id: next_id,
            payload: Some(envelope::Payload::WorldEnumerationRequest(request)),
            ..Default::default()
        })
    }
    pub async fn accept(&self, envelope: &Envelope) -> Result<bool, WorldEnumerationError> {
        let mut state = self.state.lock().await;
        let id = envelope.correlation_id;
        if let Some(envelope::Payload::WorldEnumerationItem(item)) = envelope.payload.as_ref() {
            let Some((request, coordinates, page_count)) = state.requests.get_mut(&id) else {
                return Ok(false);
            };
            if let Err(error) = validate_enumeration_item(item, request, *page_count) {
                state.requests.remove(&id);
                if let Some(response) = state.pending.remove(&id) {
                    let _ = response.send(Err(BridgeError::Permanent(format!("{error:?}"))));
                }
                if let Some(task) = state.cancellations.remove(&id) {
                    task.abort();
                }
                return Err(error);
            }
            if coordinates.len() >= MAX_ENUMERATION_ITEMS as usize * 16 {
                let error = WorldEnumerationError::InvalidIndex {
                    expected: MAX_ENUMERATION_ITEMS * 16,
                    actual: coordinates.len() as u32,
                };
                state.requests.remove(&id);
                if let Some(response) = state.pending.remove(&id) {
                    let _ = response.send(Err(BridgeError::Permanent(format!("{error:?}"))));
                }
                if let Some(task) = state.cancellations.remove(&id) {
                    task.abort();
                }
                return Err(error);
            }
            let coordinate = item.coordinate.as_ref().unwrap();
            coordinates.push(squaremap_state::ChunkCoordinate {
                x: coordinate.x,
                z: coordinate.z,
            });
            *page_count = page_count.saturating_add(1);
            return Ok(true);
        }
        if let Some(envelope::Payload::WorldEnumerationComplete(completion)) =
            envelope.payload.as_ref()
        {
            let Some((mut request, coordinates, page_count)) = state.requests.remove(&id) else {
                return Ok(false);
            };
            if let Err(error) = validate_enumeration_completion(completion, &request, page_count) {
                if let Some(response) = state.pending.remove(&id) {
                    let _ = response.send(Err(BridgeError::Permanent(format!("{error:?}"))));
                }
                if let Some(task) = state.cancellations.remove(&id) {
                    task.abort();
                }
                return Err(error);
            }
            if completion.status != 1 {
                if let Some(response) = state.pending.remove(&id) {
                    let _ = response.send(Err(BridgeError::Permanent(
                        completion.failure_reason.clone(),
                    )));
                }
            } else if completion.has_more {
                request.page_index = request
                    .page_index
                    .checked_add(1)
                    .ok_or(WorldEnumerationError::InvalidCompletion)?;
                validate_enumeration_page(&request, request.page_index)?;
                let Some(outbound) = state.outbound.clone() else {
                    if let Some(response) = state.pending.remove(&id) {
                        let _ = response.send(Err(BridgeError::Transient(
                            "enumeration outbound closed".into(),
                        )));
                    }
                    if let Some(task) = state.cancellations.remove(&id) {
                        task.abort();
                    }
                    return Err(WorldEnumerationError::InvalidSession);
                };
                let next = Envelope {
                    protocol_major: 1,
                    protocol_minor: 0,
                    session_id: request.session_id.clone(),
                    correlation_id: id,
                    payload: Some(envelope::Payload::WorldEnumerationRequest(request.clone())),
                    ..Default::default()
                };
                state.requests.insert(id, (request, coordinates, 0));
                drop(state);
                if outbound.send(next).await.is_err() {
                    let mut state = self.state.lock().await;
                    state.requests.remove(&id);
                    if let Some(response) = state.pending.remove(&id) {
                        let _ = response.send(Err(BridgeError::Transient(
                            "enumeration outbound closed".into(),
                        )));
                    }
                    if let Some(task) = state.cancellations.remove(&id) {
                        task.abort();
                    }
                    return Err(WorldEnumerationError::InvalidSession);
                }
                return Ok(true);
            } else if let Some(response) = state.pending.remove(&id) {
                let mut unique = std::collections::HashSet::with_capacity(coordinates.len());
                if coordinates
                    .iter()
                    .any(|coordinate| !unique.insert((coordinate.x, coordinate.z)))
                {
                    let _ = response.send(Err(BridgeError::Permanent(
                        "duplicate enumeration coordinate".into(),
                    )));
                } else {
                    let _ = response.send(Ok(coordinates));
                }
            }
            if completion.status != 1 || !completion.has_more {
                if let Some(task) = state.cancellations.remove(&id) {
                    task.abort();
                }
            }
            return Ok(true);
        }
        Ok(false)
    }
    pub async fn abort_request(&self, correlation_id: u64, message: impl Into<String>) {
        let message = message.into();
        let mut state = self.state.lock().await;
        state.requests.remove(&correlation_id);
        if let Some(task) = state.cancellations.remove(&correlation_id) {
            task.abort();
        }
        if let Some(response) = state.pending.remove(&correlation_id) {
            let _ = response.send(Err(BridgeError::Transient(message)));
        }
    }
    pub async fn abort(&self, message: impl Into<String>) {
        let message = message.into();
        let mut state = self.state.lock().await;
        state.aborted = true;
        state.requests.clear();
        for (_, task) in state.cancellations.drain() {
            task.abort();
        }
        for response in state.pending.drain().map(|(_, response)| response) {
            let _ = response.send(Err(BridgeError::Transient(message.clone())));
        }
    }
    pub async fn in_flight(&self) -> usize {
        self.state.lock().await.pending.len()
    }
}

pub struct SnapshotDispatcher {
    state: Arc<Mutex<DispatcherState>>,
}

struct DispatcherState {
    client: SnapshotClient,
    pending: HashMap<u64, oneshot::Sender<Result<SnapshotReply, BridgeError>>>,
    aborted: bool,
}

impl SnapshotDispatcher {
    pub fn new(client: SnapshotClient) -> Self {
        Self {
            state: Arc::new(Mutex::new(DispatcherState {
                client,
                pending: HashMap::new(),
                aborted: false,
            })),
        }
    }
    pub async fn begin(
        &self,
        command: OutboundSnapshotRequest,
    ) -> Result<Envelope, SnapshotClientError> {
        let OutboundSnapshotRequest {
            request,
            response,
            cancellation,
        } = command;
        let world = WorldIdentity {
            namespace: request.world.namespace,
            value: request.world.value,
            epoch: request.world.epoch,
        };
        let mut state = self.state.lock().await;
        if state.aborted {
            let _ = response.send(Err(BridgeError::Transient(
                "bridge connection closed".into(),
            )));
            return Err(SnapshotClientError::InvalidResponse(
                "bridge connection closed",
            ));
        }
        let envelope = match state.client.request_once(
            world,
            request.coordinate.x,
            request.coordinate.z,
            request.revision,
        ) {
            Ok(envelope) => envelope,
            Err(error) => {
                let _ = response.send(Err(BridgeError::Transient(error.to_string())));
                return Err(error);
            }
        };
        let correlation_id = envelope.correlation_id;
        state.pending.insert(correlation_id, response);
        let cancellation_state = Arc::clone(&self.state);
        drop(state);
        tokio::spawn(async move {
            if cancellation.await.is_ok() {
                let mut state = cancellation_state.lock().await;
                state.client.cancel(correlation_id);
                if let Some(response) = state.pending.remove(&correlation_id) {
                    let _ = response.send(Err(BridgeError::Transient(
                        "snapshot request cancelled".into(),
                    )));
                }
            }
        });
        Ok(envelope)
    }

    pub async fn accept(&self, envelope: &Envelope) -> Result<bool, SnapshotClientError> {
        let mut state = self.state.lock().await;
        let outcome = match state.client.accept(envelope) {
            Ok(outcome) => outcome,
            Err(SnapshotClientError::InvalidResponse("snapshot registry unavailable")) => {
                if let Some(response) = state.pending.remove(&envelope.correlation_id) {
                    let _ = response.send(Err(BridgeError::Transient(
                        "snapshot registry unavailable".into(),
                    )));
                }
                state.client.cancel(envelope.correlation_id);
                return Ok(true);
            }
            Err(error) => return Err(error),
        };
        let Some(outcome) = outcome else {
            return Ok(false);
        };
        if let Some(response) = state.pending.remove(&envelope.correlation_id) {
            let reply = match outcome {
                SnapshotOutcome::Snapshot(snapshot) => SnapshotReply::Snapshot(Arc::new(snapshot)),
                SnapshotOutcome::Missing(reason) => SnapshotReply::Missing(reason),
            };
            let _ = response.send(Ok(reply));
        }
        Ok(true)
    }

    pub async fn cancel(&self, correlation_id: u64, message: impl Into<String>) {
        let mut state = self.state.lock().await;
        state.client.cancel(correlation_id);
        if let Some(response) = state.pending.remove(&correlation_id) {
            let _ = response.send(Err(BridgeError::Transient(message.into())));
        }
    }

    pub async fn in_flight(&self) -> usize {
        self.state.lock().await.pending.len()
    }

    pub async fn abort(&self, message: impl Into<String>) {
        let message = message.into();
        let mut state = self.state.lock().await;
        state.aborted = true;
        state.client.abort_pending();
        for response in state.pending.drain().map(|(_, response)| response) {
            let _ = response.send(Err(BridgeError::Transient(message.clone())));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use squaremap_protocol::wire::{
        ChunkCoordinate, ChunkMissing, Envelope, WorldEnumerationComplete, WorldEnumerationItem,
        envelope,
    };
    use squaremap_render::Limits;

    fn world() -> WorldIdentity {
        WorldIdentity {
            namespace: "minecraft".into(),
            value: "overworld".into(),
            epoch: 3,
        }
    }

    #[tokio::test]
    async fn accept_forwards_permanent_missing_reason() {
        let client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let dispatcher = SnapshotDispatcher::new(client);
        let (response, result) = oneshot::channel();
        let request = SnapshotRequest {
            world: squaremap_state::WorldId::new("minecraft", "overworld", 3),
            coordinate: squaremap_state::ChunkCoordinate { x: 4, z: 5 },
            revision: 42,
        };
        let envelope = dispatcher
            .begin(OutboundSnapshotRequest {
                request,
                response,
                cancellation: oneshot::channel().1,
            })
            .await
            .unwrap();
        let missing = ChunkMissing {
            world: Some(world()),
            coordinate: Some(ChunkCoordinate { x: 4, z: 5 }),
            revision: 42,
            reason: squaremap_protocol::wire::ChunkMissingReason::Unavailable as i32,
        };
        assert!(
            dispatcher
                .accept(&Envelope {
                    session_id: envelope.session_id,
                    correlation_id: envelope.correlation_id,
                    payload: Some(envelope::Payload::ChunkMissing(missing)),
                    ..Default::default()
                })
                .await
                .unwrap()
        );
        assert!(matches!(
            result.await.unwrap().unwrap(),
            SnapshotReply::Missing(squaremap_protocol::wire::ChunkMissingReason::Unavailable)
        ));
    }
    #[tokio::test]
    async fn abort_resolves_pending_request_and_clears_in_flight() {
        let dispatcher =
            SnapshotDispatcher::new(SnapshotClient::new([8; 16], Limits::default()).unwrap());
        let (response, result) = oneshot::channel();
        let request = SnapshotRequest {
            world: squaremap_state::WorldId::new("minecraft", "overworld", 3),
            coordinate: squaremap_state::ChunkCoordinate { x: 6, z: 7 },
            revision: 9,
        };
        dispatcher
            .begin(OutboundSnapshotRequest {
                request,
                response,
                cancellation: oneshot::channel().1,
            })
            .await
            .unwrap();
        dispatcher.abort("bridge disconnected").await;
        assert!(matches!(
            result.await.unwrap(),
            Err(BridgeError::Transient(message)) if message == "bridge disconnected"
        ));
        assert_eq!(dispatcher.in_flight().await, 0);
    }
    #[tokio::test]
    async fn begin_after_abort_resolves_sender_without_pending_entry() {
        let dispatcher =
            SnapshotDispatcher::new(SnapshotClient::new([6; 16], Limits::default()).unwrap());
        dispatcher.abort("bridge disconnected").await;
        let (response, result) = oneshot::channel();
        let request = SnapshotRequest {
            world: squaremap_state::WorldId::new("minecraft", "overworld", 3),
            coordinate: squaremap_state::ChunkCoordinate { x: 1, z: 2 },
            revision: 1,
        };
        let error = dispatcher
            .begin(OutboundSnapshotRequest {
                request,
                response,
                cancellation: oneshot::channel().1,
            })
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            SnapshotClientError::InvalidResponse("bridge connection closed")
        ));
        assert!(matches!(
            result.await.unwrap(),
            Err(BridgeError::Transient(message)) if message == "bridge connection closed"
        ));
    }
    #[tokio::test]
    async fn missing_snapshot_registry_is_transient_and_keeps_dispatcher_alive() {
        use prost::Message;
        use squaremap_protocol::wire::ChunkSnapshot;

        let dispatcher =
            SnapshotDispatcher::new(SnapshotClient::new([9; 16], Limits::default()).unwrap());
        let (response, result) = oneshot::channel();
        let request = SnapshotRequest {
            world: squaremap_state::WorldId::new("minecraft", "overworld", 3),
            coordinate: squaremap_state::ChunkCoordinate { x: -7, z: 5 },
            revision: 42,
        };
        let envelope = dispatcher
            .begin(OutboundSnapshotRequest {
                request,
                response,
                cancellation: oneshot::channel().1,
            })
            .await
            .unwrap();
        let snapshot = ChunkSnapshot::decode(
            &include_bytes!("../../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..],
        )
        .unwrap();
        assert!(
            dispatcher
                .accept(&Envelope {
                    session_id: envelope.session_id,
                    correlation_id: envelope.correlation_id,
                    payload: Some(envelope::Payload::ChunkSnapshot(snapshot)),
                    ..Default::default()
                })
                .await
                .unwrap()
        );
        assert!(matches!(
            result.await.unwrap(),
            Err(BridgeError::Transient(message)) if message == "snapshot registry unavailable"
        ));
        assert_eq!(dispatcher.in_flight().await, 0);
    }
    #[tokio::test]
    async fn enumeration_cancellation_clears_pending_and_ignores_late_response() {
        let dispatcher = EnumerationDispatcher::new();
        let (response, result) = oneshot::channel();
        let (cancel_sender, cancellation) = oneshot::channel();
        let command = OutboundEnumerationRequest {
            request: WorldEnumerationRequest {
                world: Some(world()),
                max_items: MAX_ENUMERATION_ITEMS,
                ..Default::default()
            },
            response,
            cancellation,
        };
        let first = dispatcher
            .begin(command, &[1; 16], &[2; 16], 7, 11)
            .await
            .unwrap();
        cancel_sender.send(()).unwrap();
        for _ in 0..100 {
            if dispatcher.in_flight().await == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(dispatcher.in_flight().await, 0);
        assert!(
            matches!(result.await.unwrap(), Err(BridgeError::Transient(message)) if message == "enumeration request cancelled")
        );
        assert!(
            !dispatcher
                .accept(&Envelope {
                    correlation_id: first.correlation_id,
                    payload: Some(envelope::Payload::WorldEnumerationComplete(
                        WorldEnumerationComplete {
                            session_id: vec![1; 16],
                            bridge_id: vec![2; 16],
                            config_revision: 7,
                            enumeration_id: 11,
                            item_count: 0,
                            status: 1,
                            page_index: 0,
                            has_more: false,
                            failure_reason: String::new(),
                        }
                    )),
                    ..Default::default()
                })
                .await
                .unwrap()
        );
        assert_eq!(dispatcher.in_flight().await, 0);
    }
    #[tokio::test]
    async fn enumeration_aggregates_multiple_bounded_pages() {
        let dispatcher = EnumerationDispatcher::new();
        let (outbound, mut outbound_rx) = mpsc::channel(4);
        dispatcher.set_outbound(outbound).await;
        let (response, result) = oneshot::channel();
        let command = OutboundEnumerationRequest {
            request: WorldEnumerationRequest {
                world: Some(world()),
                max_items: MAX_ENUMERATION_ITEMS,
                ..Default::default()
            },
            response,
            cancellation: oneshot::channel().1,
        };
        let first = dispatcher
            .begin(command, &[1; 16], &[2; 16], 7, 11)
            .await
            .unwrap();
        let request = match first.payload.as_ref().unwrap() {
            envelope::Payload::WorldEnumerationRequest(request) => request.clone(),
            _ => unreachable!(),
        };
        for index in 0..MAX_ENUMERATION_ITEMS {
            dispatcher
                .accept(&Envelope {
                    correlation_id: 11,
                    payload: Some(envelope::Payload::WorldEnumerationItem(
                        WorldEnumerationItem {
                            session_id: request.session_id.clone(),
                            bridge_id: request.bridge_id.clone(),
                            config_revision: 7,
                            enumeration_id: 11,
                            item_index: index,
                            page_index: 0,
                            world: request.world.clone(),
                            coordinate: Some(ChunkCoordinate {
                                x: index as i32,
                                z: 0,
                            }),
                        },
                    )),
                    ..Default::default()
                })
                .await
                .unwrap();
        }
        dispatcher
            .accept(&Envelope {
                correlation_id: 11,
                payload: Some(envelope::Payload::WorldEnumerationComplete(
                    WorldEnumerationComplete {
                        session_id: request.session_id.clone(),
                        bridge_id: request.bridge_id.clone(),
                        config_revision: 7,
                        enumeration_id: 11,
                        item_count: MAX_ENUMERATION_ITEMS,
                        status: 1,
                        page_index: 0,
                        has_more: true,
                        failure_reason: String::new(),
                    },
                )),
                ..Default::default()
            })
            .await
            .unwrap();
        let second = outbound_rx.recv().await.unwrap();
        let second_request = match second.payload.as_ref().unwrap() {
            envelope::Payload::WorldEnumerationRequest(request) => request.clone(),
            _ => unreachable!(),
        };
        assert_eq!(second_request.page_index, 1);
        for index in 0..2 {
            dispatcher
                .accept(&Envelope {
                    correlation_id: 11,
                    payload: Some(envelope::Payload::WorldEnumerationItem(
                        WorldEnumerationItem {
                            session_id: request.session_id.clone(),
                            bridge_id: request.bridge_id.clone(),
                            config_revision: 7,
                            enumeration_id: 11,
                            item_index: index,
                            page_index: 1,
                            world: request.world.clone(),
                            coordinate: Some(ChunkCoordinate {
                                x: MAX_ENUMERATION_ITEMS as i32 + index as i32,
                                z: 0,
                            }),
                        },
                    )),
                    ..Default::default()
                })
                .await
                .unwrap();
        }
        dispatcher
            .accept(&Envelope {
                correlation_id: 11,
                payload: Some(envelope::Payload::WorldEnumerationComplete(
                    WorldEnumerationComplete {
                        session_id: request.session_id,
                        bridge_id: request.bridge_id,
                        config_revision: 7,
                        enumeration_id: 11,
                        item_count: 2,
                        status: 1,
                        page_index: 1,
                        has_more: false,
                        failure_reason: String::new(),
                    },
                )),
                ..Default::default()
            })
            .await
            .unwrap();
        let coordinates = result.await.unwrap().unwrap();
        assert_eq!(coordinates.len(), MAX_ENUMERATION_ITEMS as usize + 2);
        assert_eq!(
            coordinates.last().unwrap().x,
            MAX_ENUMERATION_ITEMS as i32 + 1
        );
    }
}
