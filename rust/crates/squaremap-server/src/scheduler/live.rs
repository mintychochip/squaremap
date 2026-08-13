use super::{BridgeError, SnapshotBridge, SnapshotReply, SnapshotRequest};
use crate::snapshot_client::{SnapshotClient, SnapshotClientError, SnapshotOutcome};
use async_trait::async_trait;
use squaremap_protocol::wire::{Envelope, WorldIdentity};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

pub struct OutboundSnapshotRequest {
    pub request: SnapshotRequest,
    pub response: oneshot::Sender<Result<SnapshotReply, BridgeError>>,
}

#[derive(Clone)]
pub struct LiveSnapshotBridge {
    sender: mpsc::Sender<OutboundSnapshotRequest>,
}
impl LiveSnapshotBridge {
    pub fn channel(capacity: usize) -> (Arc<Self>, mpsc::Receiver<OutboundSnapshotRequest>) {
        let (sender, receiver) = mpsc::channel(capacity);
        (Arc::new(Self { sender }), receiver)
    }
}
#[async_trait]
impl SnapshotBridge for LiveSnapshotBridge {
    async fn request(&self, request: SnapshotRequest) -> Result<SnapshotReply, BridgeError> {
        let (response, result) = oneshot::channel();
        self.sender.send(OutboundSnapshotRequest { request, response }).await
            .map_err(|_| BridgeError::Transient("bridge connection closed".into()))?;
        result.await.map_err(|_| BridgeError::Transient("bridge connection closed".into()))?
    }
}

pub struct SnapshotDispatcher {
    client: Mutex<SnapshotClient>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<SnapshotReply, BridgeError>>>>,
}
impl SnapshotDispatcher {
    pub fn new(client: SnapshotClient) -> Self {
        Self { client: Mutex::new(client), pending: Mutex::new(HashMap::new()) }
    }

    pub async fn begin(&self, command: OutboundSnapshotRequest) -> Result<Envelope, SnapshotClientError> {
        let OutboundSnapshotRequest { request, response } = command;
        let world = WorldIdentity {
            namespace: request.world.namespace,
            value: request.world.value,
            epoch: request.world.epoch,
        };
        let envelope = match self.client.lock().await.request_once(
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
        self.pending.lock().await.insert(envelope.correlation_id, response);
        Ok(envelope)
    }

    pub async fn accept(&self, envelope: &Envelope) -> Result<bool, SnapshotClientError> {
        let outcome = self.client.lock().await.accept(envelope)?;
        let Some(outcome) = outcome else { return Ok(false); };
        if let Some(response) = self.pending.lock().await.remove(&envelope.correlation_id) {
            let reply = match outcome {
                SnapshotOutcome::Snapshot(snapshot) => SnapshotReply::Snapshot(Arc::new(snapshot)),
                SnapshotOutcome::Missing(_) => SnapshotReply::Missing,
            };
            let _ = response.send(Ok(reply));
        }
        Ok(true)
    }

    pub async fn abort(&self, message: impl Into<String>) {
        let message = message.into();
        self.client.lock().await.abort_pending();
        let pending = std::mem::take(&mut *self.pending.lock().await);
        for response in pending.into_values() {
            let _ = response.send(Err(BridgeError::Transient(message.clone())));
        }
    }

    pub async fn in_flight(&self) -> usize { self.pending.lock().await.len() }
}
