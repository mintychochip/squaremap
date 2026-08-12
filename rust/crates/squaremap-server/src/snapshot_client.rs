use squaremap_protocol::wire::{envelope, ChunkMissingReason, ChunkSnapshotRequest, Envelope, RegistryReplace, WorldIdentity};
use squaremap_render::{Limits, Registry, Snapshot, SnapshotError};
use std::collections::HashMap;
use std::fmt;

const MAX_IN_FLIGHT: usize = 96;

#[derive(Debug)]
pub enum SnapshotClientError {
    InvalidSession,
    Saturated,
    InvalidResponse(&'static str),
    Registry(squaremap_render::RegistryError),
    Snapshot(SnapshotError),
}
impl fmt::Display for SnapshotClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{self:?}") }
}
impl std::error::Error for SnapshotClientError {}
impl From<squaremap_render::RegistryError> for SnapshotClientError { fn from(error: squaremap_render::RegistryError) -> Self { Self::Registry(error) } }
impl From<SnapshotError> for SnapshotClientError { fn from(error: SnapshotError) -> Self { Self::Snapshot(error) } }

#[derive(Debug)]
pub enum SnapshotOutcome { Snapshot(Snapshot), Missing(ChunkMissingReason) }

struct Pending { world: WorldIdentity, revision: u64 }

pub struct SnapshotClient {
    session_id: [u8; 16],
    next_sequence: u64,
    next_request_id: u64,
    next_correlation: u64,
    pending: HashMap<u64, Pending>,
    registry: Registry,
    registry_generations: HashMap<String, u64>,
    limits: Limits,
}
impl SnapshotClient {
    pub fn new(session_id: [u8; 16], limits: Limits) -> Result<Self, SnapshotClientError> {
        if session_id == [0; 16] { return Err(SnapshotClientError::InvalidSession); }
        Ok(Self {
            session_id, next_sequence: 1, next_request_id: 1, next_correlation: 1,
            pending: HashMap::new(), registry: Registry::default(), registry_generations: HashMap::new(), limits,
        })
    }

    pub fn request_once(&mut self, world: WorldIdentity, x: i32, z: i32, revision: u64) -> Result<Envelope, SnapshotClientError> {
        if self.pending.len() >= MAX_IN_FLIGHT { return Err(SnapshotClientError::Saturated); }
        let correlation = self.allocate(self.next_correlation, true)?;
        let request_id = self.allocate(self.next_request_id, false)?;
        let request = ChunkSnapshotRequest {
            world: Some(world.clone()),
            coordinate: Some(squaremap_protocol::wire::ChunkCoordinate { x, z }),
            revision,
            request_id,
        };
        self.pending.insert(correlation, Pending { world, revision });
        Ok(Envelope {
            protocol_major: 1, protocol_minor: 0, session_id: self.session_id.to_vec(),
            sequence: self.take_sequence(), correlation_id: correlation,
            payload: Some(envelope::Payload::ChunkSnapshotRequest(request)),
        })
    }

    fn allocate(&mut self, current: u64, correlation: bool) -> Result<u64, SnapshotClientError> {
        if current == 0 || current == u64::MAX {
            return Err(SnapshotClientError::InvalidResponse(if correlation { "correlation exhausted" } else { "request ID exhausted" }));
        }
        if correlation { self.next_correlation = current + 1; } else { self.next_request_id = current + 1; }
        Ok(current)
    }
    fn take_sequence(&mut self) -> u64 { let value = self.next_sequence; self.next_sequence = self.next_sequence.saturating_add(1); value }

    pub fn accept(&mut self, envelope: &Envelope) -> Result<Option<SnapshotOutcome>, SnapshotClientError> {
        if envelope.session_id.as_slice() != self.session_id { return Err(SnapshotClientError::InvalidSession); }
        match envelope.payload.as_ref() {
            Some(envelope::Payload::RegistryReplace(registry)) => self.accept_registry(envelope.correlation_id, registry),
            Some(envelope::Payload::ChunkSnapshot(snapshot)) => {
                let Some(pending) = self.pending.get(&envelope.correlation_id) else { return Ok(None); };
                if snapshot.world.as_ref() != Some(&pending.world) || snapshot.revision != pending.revision {
                    return Err(SnapshotClientError::InvalidResponse("snapshot binding mismatch"));
                }
                self.pending.remove(&envelope.correlation_id);
                Ok(Some(SnapshotOutcome::Snapshot(Snapshot::decode(snapshot, &self.registry, self.limits)?)))
            }
            Some(envelope::Payload::ChunkMissing(missing)) => {
                let Some(pending) = self.pending.get(&envelope.correlation_id) else { return Ok(None); };
                if missing.world.as_ref() != Some(&pending.world) || missing.revision != pending.revision {
                    return Err(SnapshotClientError::InvalidResponse("missing binding mismatch"));
                }
                self.pending.remove(&envelope.correlation_id);
                Ok(Some(SnapshotOutcome::Missing(ChunkMissingReason::try_from(missing.reason).unwrap_or(ChunkMissingReason::Unspecified))))
            }
            _ => Ok(None),
        }
    }

    fn accept_registry(&mut self, correlation_id: u64, replacement: &RegistryReplace) -> Result<Option<SnapshotOutcome>, SnapshotClientError> {
        let world = replacement.world.as_ref().ok_or(SnapshotClientError::InvalidResponse("registry missing world"))?;
        let key = format!("{}\0{}\0{}", world.namespace, world.value, world.epoch);
        if self.registry_generations.get(&key).is_some_and(|current| replacement.revision <= *current) { return Ok(None); }
        let Some(pending) = self.pending.get(&correlation_id) else { return Ok(None); };
        if pending.revision != replacement.revision || pending.world != *world { return Ok(None); }
        self.registry.replace(replacement.clone())?;
        self.registry_generations.insert(key, replacement.revision);
        Ok(None)
    }
    pub fn in_flight(&self) -> usize { self.pending.len() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;
    use squaremap_protocol::wire::{ChunkMissing, ChunkSnapshot, envelope, Envelope, RegistryReplace};

    fn world() -> WorldIdentity { WorldIdentity { namespace: "minecraft".into(), value: "overworld".into(), epoch: 3 } }

    #[test]
    fn request_is_authenticated_bounded_and_unique() {
        let mut client = SnapshotClient::new([7; 16], Limits::default()).unwrap();
        let first = client.request_once(world(), 1, 2, 42).unwrap();
        assert_eq!(first.session_id, vec![7; 16]);
        assert_eq!(first.sequence, 1);
        assert_eq!(first.correlation_id, 1);
        let request = match first.payload.unwrap() { envelope::Payload::ChunkSnapshotRequest(value) => value, _ => unreachable!() };
        assert_eq!(request.request_id, 1);
        for i in 0..95 { client.request_once(world(), i, i, 42).unwrap(); }
        assert_eq!(client.in_flight(), 96);
        assert!(matches!(client.request_once(world(), 0, 0, 42), Err(SnapshotClientError::Saturated)));
    }

    #[test]
    fn fixture_request_roundtrips_registry_snapshot_and_missing() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let request = client.request_once(world(), -7, 5, 42).unwrap();
        let correlation = request.correlation_id;
        let registry = RegistryReplace::decode(&include_bytes!("../../../../testdata/bridge/v1/registry_replace_valid.bin")[..]).unwrap();
        client.accept(&Envelope { session_id: request.session_id.clone(), correlation_id: correlation, payload: Some(envelope::Payload::RegistryReplace(registry)), ..Default::default() }).unwrap();
        let snapshot = ChunkSnapshot::decode(&include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..]).unwrap();
        let response = client.accept(&Envelope { session_id: request.session_id.clone(), correlation_id: correlation, payload: Some(envelope::Payload::ChunkSnapshot(snapshot)), ..Default::default() }).unwrap();
        match response { Some(SnapshotOutcome::Snapshot(snapshot)) => { assert_eq!(snapshot.coordinate.x, -7); assert_eq!(snapshot.surface.heightmap.len(), 256); }, _ => panic!("expected decoded snapshot") }
        let missing_request = client.request_once(world(), 0, 0, 42).unwrap();
        let missing = ChunkMissing { world: Some(world()), coordinate: Some(squaremap_protocol::wire::ChunkCoordinate { x: 0, z: 0 }), revision: 42, reason: ChunkMissingReason::Unloaded as i32 };
        let outcome = client.accept(&Envelope { session_id: missing_request.session_id, correlation_id: missing_request.correlation_id, payload: Some(envelope::Payload::ChunkMissing(missing)), ..Default::default() }).unwrap();
        assert!(matches!(outcome, Some(SnapshotOutcome::Missing(ChunkMissingReason::Unloaded))));
    }

    #[test]
    fn registry_replace_requires_matching_pending_correlation() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let request = client.request_once(world(), -7, 5, 42).unwrap();
        let mut foreign = RegistryReplace::decode(&include_bytes!("../../../../testdata/bridge/v1/registry_replace_valid.bin")[..]).unwrap();
        foreign.block_states.clear();
        assert!(client.accept(&Envelope {
            session_id: request.session_id.clone(), correlation_id: request.correlation_id + 1,
            payload: Some(envelope::Payload::RegistryReplace(foreign)), ..Default::default()
        }).unwrap().is_none());
        let registry = RegistryReplace::decode(&include_bytes!("../../../../testdata/bridge/v1/registry_replace_valid.bin")[..]).unwrap();
        client.accept(&Envelope {
            session_id: request.session_id.clone(), correlation_id: request.correlation_id,
            payload: Some(envelope::Payload::RegistryReplace(registry)), ..Default::default()
        }).unwrap();
        let snapshot = ChunkSnapshot::decode(&include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..]).unwrap();
        let response = client.accept(&Envelope {
            session_id: request.session_id, correlation_id: request.correlation_id,
            payload: Some(envelope::Payload::ChunkSnapshot(snapshot)), ..Default::default()
        }).unwrap();
        assert!(matches!(response, Some(SnapshotOutcome::Snapshot(_))));
    }

    #[test]
    fn registry_generation_is_reset_for_new_world_epoch() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let first = client.request_once(world(), 0, 0, 42).unwrap();
        let mut first_registry = RegistryReplace::decode(&include_bytes!("../../../../testdata/bridge/v1/registry_replace_valid.bin")[..]).unwrap();
        first_registry.world = Some(world());
        client.accept(&Envelope {
            session_id: first.session_id.clone(), correlation_id: first.correlation_id,
            payload: Some(envelope::Payload::RegistryReplace(first_registry)), ..Default::default()
        }).unwrap();
        let new_world = WorldIdentity { epoch: 4, ..world() };
        let second = client.request_once(new_world.clone(), 1, 1, 42).unwrap();
        let mut second_registry = RegistryReplace::decode(&include_bytes!("../../../../testdata/bridge/v1/registry_replace_valid.bin")[..]).unwrap();
        second_registry.world = Some(new_world.clone());
        client.accept(&Envelope {
            session_id: second.session_id.clone(), correlation_id: second.correlation_id,
            payload: Some(envelope::Payload::RegistryReplace(second_registry)), ..Default::default()
        }).unwrap();
        let mut snapshot = ChunkSnapshot::decode(&include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..]).unwrap();
        snapshot.world = Some(new_world);
        let response = client.accept(&Envelope {
            session_id: second.session_id, correlation_id: second.correlation_id,
            payload: Some(envelope::Payload::ChunkSnapshot(snapshot)), ..Default::default()
        }).unwrap();
        assert!(matches!(response, Some(SnapshotOutcome::Snapshot(_))));
    }

    #[test]
    fn unsolicited_snapshot_and_registry_are_ignored_without_mutation() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let registry = RegistryReplace::decode(&include_bytes!("../../../../testdata/bridge/v1/registry_replace_valid.bin")[..]).unwrap();
        assert!(client.accept(&Envelope { session_id: vec![9; 16], payload: Some(envelope::Payload::RegistryReplace(registry)), ..Default::default() }).unwrap().is_none());
        let snapshot = ChunkSnapshot::decode(&include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..]).unwrap();
        assert!(client.accept(&Envelope { session_id: vec![9; 16], correlation_id: 999, payload: Some(envelope::Payload::ChunkSnapshot(snapshot)), ..Default::default() }).unwrap().is_none());
        assert_eq!(client.in_flight(), 0);
    }

    #[test]
    fn binding_error_keeps_pending_request() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let request = client.request_once(world(), 0, 0, 42).unwrap();
        let bad = ChunkMissing { world: Some(WorldIdentity { value: "wrong".into(), ..world() }), coordinate: Some(Default::default()), revision: 42, reason: ChunkMissingReason::Unavailable as i32 };
        assert!(matches!(client.accept(&Envelope { session_id: request.session_id, correlation_id: request.correlation_id, payload: Some(envelope::Payload::ChunkMissing(bad)), ..Default::default() }), Err(SnapshotClientError::InvalidResponse(_))));
        assert_eq!(client.in_flight(), 1);
    }
}
