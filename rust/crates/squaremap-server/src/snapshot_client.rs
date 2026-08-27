use squaremap_protocol::wire::{
    ChunkMissingReason, ChunkSnapshot, ChunkSnapshotRequest, Envelope, RegistryReplace,
    WorldIdentity, envelope,
};
use squaremap_render::{GenerationToken, Limits, Registry, RegistryError, Snapshot};
use std::collections::HashMap;
use std::fmt;

pub const MAX_IN_FLIGHT: usize = 96;
pub const MAX_CANCELLED: usize = MAX_IN_FLIGHT * 2;

#[derive(Debug)]
pub enum SnapshotClientError {
    InvalidSession,
    Saturated,
    InvalidResponse(&'static str),
    Registry(RegistryError),
    Snapshot(squaremap_render::SnapshotError),
}
impl fmt::Display for SnapshotClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SnapshotClientError {}
impl From<RegistryError> for SnapshotClientError {
    fn from(error: RegistryError) -> Self {
        Self::Registry(error)
    }
}
impl From<squaremap_render::SnapshotError> for SnapshotClientError {
    fn from(error: squaremap_render::SnapshotError) -> Self {
        Self::Snapshot(error)
    }
}

#[derive(Debug)]
pub enum SnapshotOutcome {
    Snapshot(Snapshot),
    Missing(ChunkMissingReason),
}

#[derive(Clone)]
struct Pending {
    world: WorldIdentity,
    coordinate: squaremap_protocol::wire::ChunkCoordinate,
    revision: u64,
    generation: Option<GenerationToken>,
    generation_from_cache: bool,
    held_snapshot: Option<ChunkSnapshot>,
}
fn world_key(world: &WorldIdentity) -> String {
    format!("{}\0{}\0{}", world.namespace, world.value, world.epoch)
}

pub struct SnapshotClient {
    session_id: [u8; 16],
    next_sequence: u64,
    next_request_id: u64,
    next_correlation: u64,
    pending: HashMap<u64, Pending>,
    cancelled: HashMap<u64, ()>,
    cancelled_before: u64,
    registries: HashMap<String, Registry>,
    registry_replacements: HashMap<String, RegistryReplace>,
    limits: Limits,
}
impl SnapshotClient {
    pub fn new(session_id: [u8; 16], limits: Limits) -> Result<Self, SnapshotClientError> {
        if session_id == [0; 16] {
            return Err(SnapshotClientError::InvalidSession);
        }
        Ok(Self {
            session_id,
            next_sequence: 1,
            next_request_id: 1,
            next_correlation: 1,
            cancelled_before: 0,
            pending: HashMap::new(),
            cancelled: HashMap::new(),
            registries: HashMap::new(),
            registry_replacements: HashMap::new(),
            limits,
        })
    }
    pub fn request_once(
        &mut self,
        world: WorldIdentity,
        x: i32,
        z: i32,
        revision: u64,
    ) -> Result<Envelope, SnapshotClientError> {
        self.request_once_loaded_only(world, x, z, revision, false)
    }

    pub fn request_once_loaded_only(
        &mut self,
        world: WorldIdentity,
        x: i32,
        z: i32,
        revision: u64,
        loaded_only: bool,
    ) -> Result<Envelope, SnapshotClientError> {
        if self.pending.len() >= MAX_IN_FLIGHT {
            return Err(SnapshotClientError::Saturated);
        }
        let correlation = self.allocate(self.next_correlation, true)?;
        let request_id = self.allocate(self.next_request_id, false)?;
        let request = ChunkSnapshotRequest {
            world: Some(world.clone()),
            coordinate: Some(squaremap_protocol::wire::ChunkCoordinate { x, z }),
            revision,
            request_id,
            loaded_only,
        };
        let generation = self
            .registries
            .get(&world_key(&world))
            .map(Registry::generation);
        let generation_from_cache = generation.is_some();
        self.pending.insert(
            correlation,
            Pending {
                world,
                coordinate: squaremap_protocol::wire::ChunkCoordinate { x, z },
                revision,
                generation,
                generation_from_cache,
                held_snapshot: None,
            },
        );
        Ok(Envelope {
            protocol_major: 1,
            protocol_minor: 0,
            session_id: self.session_id.to_vec(),
            sequence: self.take_sequence(),
            correlation_id: correlation,
            payload: Some(envelope::Payload::ChunkSnapshotRequest(request)),
        })
    }

    fn allocate(&mut self, current: u64, correlation: bool) -> Result<u64, SnapshotClientError> {
        if current == 0 || current == u64::MAX {
            return Err(SnapshotClientError::InvalidResponse(if correlation {
                "correlation exhausted"
            } else {
                "request ID exhausted"
            }));
        }
        if correlation {
            self.next_correlation = current + 1;
        } else {
            self.next_request_id = current + 1;
        }
        Ok(current)
    }
    fn take_sequence(&mut self) -> u64 {
        let value = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        value
    }
    pub fn cancel(&mut self, correlation_id: u64) -> bool {
        let removed = self.pending.remove(&correlation_id).is_some();
        if removed {
            self.cancelled.insert(correlation_id, ());
            while self.cancelled.len() > MAX_CANCELLED {
                let Some(oldest_pending) = self.pending.keys().copied().min() else {
                    self.cancelled.clear();
                    self.cancelled_before = self.next_correlation;
                    break;
                };
                let safe_floor = oldest_pending;
                self.cancelled
                    .retain(|correlation, _| *correlation >= safe_floor);
                self.cancelled_before = self.cancelled_before.max(safe_floor);
                if self.cancelled.len() <= MAX_CANCELLED {
                    break;
                }
                let Some(removable) = self
                    .cancelled
                    .keys()
                    .copied()
                    .filter(|correlation| *correlation > safe_floor)
                    .max()
                else {
                    break;
                };
                self.cancelled.remove(&removable);
            }
        }
        removed
    }
    pub fn accept(
        &mut self,
        envelope: &Envelope,
    ) -> Result<Option<SnapshotOutcome>, SnapshotClientError> {
        if envelope.session_id.as_slice() != self.session_id {
            return Err(SnapshotClientError::InvalidSession);
        }
        match envelope.payload.as_ref() {
            Some(envelope::Payload::RegistryReplace(registry)) => {
                self.accept_registry(envelope.correlation_id, registry)
            }
            Some(envelope::Payload::ChunkSnapshot(snapshot)) => {
                let Some(pending) = self.pending.get(&envelope.correlation_id).cloned() else {
                    return Ok(None);
                };
                if snapshot.world.as_ref() != Some(&pending.world)
                    || snapshot.coordinate.as_ref() != Some(&pending.coordinate)
                    || snapshot.revision != pending.revision
                {
                    return Err(SnapshotClientError::InvalidResponse(
                        "snapshot binding mismatch",
                    ));
                }
                // Stamped cache is from an earlier dirty revision. Prefer the current
                // world registry so a sibling RegistryReplace at this revision wins.
                let generation = if pending.generation_from_cache {
                    self.registries
                        .get(&world_key(&pending.world))
                        .map(Registry::generation)
                        .or_else(|| pending.generation.clone())
                } else {
                    pending.generation.clone().or_else(|| {
                        self.registries
                            .get(&world_key(&pending.world))
                            .map(Registry::generation)
                    })
                };
                let Some(generation) = generation else {
                    if let Some(pending) = self.pending.get_mut(&envelope.correlation_id) {
                        pending.held_snapshot = Some(snapshot.clone());
                    }
                    return Ok(None);
                };
                let decoded = Snapshot::decode_for_live(snapshot, &generation, self.limits)?;
                self.pending.remove(&envelope.correlation_id);
                Ok(Some(SnapshotOutcome::Snapshot(decoded)))
            }
            Some(envelope::Payload::ChunkMissing(missing)) => {
                let Some(pending) = self.pending.get(&envelope.correlation_id).cloned() else {
                    return Ok(None);
                };
                if missing.world.as_ref() != Some(&pending.world)
                    || missing.coordinate.as_ref() != Some(&pending.coordinate)
                    || missing.revision != pending.revision
                {
                    return Err(SnapshotClientError::InvalidResponse(
                        "missing binding mismatch",
                    ));
                }
                self.pending.remove(&envelope.correlation_id);
                Ok(Some(SnapshotOutcome::Missing(
                    ChunkMissingReason::try_from(missing.reason)
                        .unwrap_or(ChunkMissingReason::Unspecified),
                )))
            }
            _ => Ok(None),
        }
    }

    fn accept_registry(
        &mut self,
        correlation_id: u64,
        replacement: &RegistryReplace,
    ) -> Result<Option<SnapshotOutcome>, SnapshotClientError> {
        let cancelled = correlation_id < self.cancelled_before
            || self.cancelled.contains_key(&correlation_id);
        let pending = self.pending.get(&correlation_id);
        let Some(world) = replacement.world.as_ref() else {
            if cancelled {
                return Ok(None);
            }
            return Err(SnapshotClientError::InvalidResponse(
                "registry missing world",
            ));
        };
        if let Some(pending) = pending {
            if pending.revision != replacement.revision || pending.world != *world {
                return Ok(None);
            }
        } else if !cancelled {
            return Ok(None);
        }
        let key = world_key(world);
        let current_revision = self.registries.get(&key).map_or(0, Registry::revision);
        if replacement.revision < current_revision {
            return Ok(None);
        }
        let unchanged = self
            .registry_replacements
            .get(&key)
            .is_some_and(|current| current == replacement);
        if unchanged {
            if let Some(generation) = self.registries.get(&key).map(Registry::generation) {
                if let Some(pending) = self.pending.get_mut(&correlation_id) {
                    if pending.generation.is_none() {
                        pending.generation = Some(generation);
                    }
                }
            }
            return self.complete_held_snapshot(correlation_id);
        }
        let registry = self.registries.entry(key.clone()).or_default();
        registry.replace(replacement.clone())?;
        if let Some(pending) = self.pending.get_mut(&correlation_id) {
            if pending.generation.is_none() || pending.generation_from_cache {
                pending.generation = Some(registry.generation());
                pending.generation_from_cache = false;
            }
        }
        self.registry_replacements.insert(key, replacement.clone());
        self.complete_held_snapshot(correlation_id)
    }

    fn complete_held_snapshot(
        &mut self,
        correlation_id: u64,
    ) -> Result<Option<SnapshotOutcome>, SnapshotClientError> {
        let Some(pending) = self.pending.get(&correlation_id) else {
            return Ok(None);
        };
        let Some(generation) = pending.generation.clone().or_else(|| {
            self.registries
                .get(&world_key(&pending.world))
                .map(Registry::generation)
        }) else {
            return Ok(None);
        };
        let Some(snapshot) = self
            .pending
            .get_mut(&correlation_id)
            .and_then(|pending| pending.held_snapshot.take())
        else {
            return Ok(None);
        };
        let decoded = Snapshot::decode_for_live(&snapshot, &generation, self.limits)?;
        self.pending.remove(&correlation_id);
        Ok(Some(SnapshotOutcome::Snapshot(decoded)))
    }
    pub fn in_flight(&self) -> usize {
        self.pending.len()
    }

    /// Drops every request bound to a disconnected transport.
    ///
    /// Registry generations remain cached for the authenticated session; callers must construct a
    /// new client when the session ID changes.
    pub fn abort_pending(&mut self) -> usize {
        let count = self.pending.len();
        self.pending.clear();
        self.cancelled.clear();
        self.cancelled_before = self.next_correlation;
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;
    use squaremap_protocol::wire::{
        ChunkMissing, ChunkSnapshot, Envelope, RegistryReplace, envelope,
    };

    fn world() -> WorldIdentity {
        WorldIdentity {
            namespace: "minecraft".into(),
            value: "overworld".into(),
            epoch: 3,
        }
    }

    #[test]
    fn live_request_sets_loaded_only_flag() {
        let mut client = SnapshotClient::new([7; 16], Limits::default()).unwrap();
        let envelope = client
            .request_once_loaded_only(world(), 1, 2, 9, true)
            .unwrap();
        let request = match envelope.payload.unwrap() {
            envelope::Payload::ChunkSnapshotRequest(value) => value,
            _ => unreachable!(),
        };
        assert!(request.loaded_only);
        let job = client.request_once(world(), 3, 4, 9).unwrap();
        let job_request = match job.payload.unwrap() {
            envelope::Payload::ChunkSnapshotRequest(value) => value,
            _ => unreachable!(),
        };
        assert!(!job_request.loaded_only);
    }

    #[test]
    fn request_is_authenticated_bounded_and_unique() {
        let mut client = SnapshotClient::new([7; 16], Limits::default()).unwrap();
        let first = client.request_once(world(), 1, 2, 42).unwrap();
        assert_eq!(first.session_id, vec![7; 16]);
        assert_eq!(first.sequence, 1);
        assert_eq!(first.correlation_id, 1);
        let request = match first.payload.unwrap() {
            envelope::Payload::ChunkSnapshotRequest(value) => value,
            _ => unreachable!(),
        };
        assert_eq!(request.request_id, 1);
        for i in 0..95 {
            client.request_once(world(), i, i, 42).unwrap();
        }
        assert_eq!(client.in_flight(), 96);
        assert!(matches!(
            client.request_once(world(), 0, 0, 42),
            Err(SnapshotClientError::Saturated)
        ));
    }

    fn push_registry(
        client: &mut SnapshotClient,
        correlation: u64,
        world: &WorldIdentity,
        revision: u64,
    ) {
        let mut registry = RegistryReplace::decode(
            &include_bytes!("../../../../testdata/bridge/v1/registry_replace_valid.bin")[..],
        )
        .unwrap();
        registry.world = Some(world.clone());
        registry.revision = revision;
        assert!(
            client
                .accept(&Envelope {
                    session_id: vec![9; 16],
                    correlation_id: correlation,
                    payload: Some(envelope::Payload::RegistryReplace(registry)),
                    ..Default::default()
                })
                .unwrap()
                .is_none()
        );
    }

    fn push_registry_with_descriptor_two_color(
        client: &mut SnapshotClient,
        correlation: u64,
        world: &WorldIdentity,
        revision: u64,
        color: u32,
    ) {
        let mut registry = RegistryReplace::decode(
            &include_bytes!("../../../../testdata/bridge/v1/registry_replace_valid.bin")[..],
        )
        .unwrap();
        registry.world = Some(world.clone());
        registry.revision = revision;
        registry
            .block_states
            .iter_mut()
            .find(|descriptor| descriptor.id == 2)
            .expect("fixture water descriptor")
            .map_color = color;
        assert!(
            client
                .accept(&Envelope {
                    session_id: vec![9; 16],
                    correlation_id: correlation,
                    payload: Some(envelope::Payload::RegistryReplace(registry)),
                    ..Default::default()
                })
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn fixture_request_roundtrips_registry_snapshot_and_missing() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let request = client.request_once(world(), -7, 5, 42).unwrap();
        push_registry(&mut client, request.correlation_id, &world(), 42);
        let snapshot = ChunkSnapshot::decode(
            &include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..],
        )
        .unwrap();
        let response = client
            .accept(&Envelope {
                session_id: request.session_id.clone(),
                correlation_id: request.correlation_id,
                payload: Some(envelope::Payload::ChunkSnapshot(snapshot)),
                ..Default::default()
            })
            .unwrap();
        match response {
            Some(SnapshotOutcome::Snapshot(snapshot)) => {
                assert_eq!(snapshot.coordinate.x, -7);
                assert_eq!(snapshot.surface.heightmap.len(), 256);
            }
            _ => panic!("expected decoded snapshot"),
        }
        let missing_request = client.request_once(world(), 0, 0, 42).unwrap();
        let missing = ChunkMissing {
            world: Some(world()),
            coordinate: Some(squaremap_protocol::wire::ChunkCoordinate { x: 0, z: 0 }),
            revision: 42,
            reason: ChunkMissingReason::Unloaded as i32,
        };
        let outcome = client
            .accept(&Envelope {
                session_id: missing_request.session_id,
                correlation_id: missing_request.correlation_id,
                payload: Some(envelope::Payload::ChunkMissing(missing)),
                ..Default::default()
            })
            .unwrap();
        assert!(matches!(
            outcome,
            Some(SnapshotOutcome::Missing(ChunkMissingReason::Unloaded))
        ));
    }
    #[test]
    fn cancelled_request_does_not_poison_later_registry_generation() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let first = client.request_once(world(), 1, 1, 42).unwrap();
        client.cancel(first.correlation_id);
        let second = client.request_once(world(), 2, 2, 42).unwrap();
        let mut registry = RegistryReplace::decode(
            &include_bytes!("../../../../testdata/bridge/v1/registry_replace_valid.bin")[..],
        )
        .unwrap();
        registry.world = Some(world());
        registry.revision = 42;
        assert!(
            client
                .accept(&Envelope {
                    session_id: second.session_id.clone(),
                    correlation_id: second.correlation_id,
                    payload: Some(envelope::Payload::RegistryReplace(registry)),
                    ..Default::default()
                })
                .unwrap()
                .is_none()
        );
        assert_eq!(client.in_flight(), 1);
    }

    #[test]
    fn cancel_then_delayed_registry_preserves_next_request() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let first = client.request_once(world(), 1, 1, 42).unwrap();
        let second = client.request_once(world(), 2, 2, 42).unwrap();
        assert!(client.cancel(first.correlation_id));
        push_registry(&mut client, first.correlation_id, &world(), 42);
        assert_eq!(client.in_flight(), 1);
        push_registry(&mut client, second.correlation_id, &world(), 42);
        let mut snapshot = ChunkSnapshot::decode(
            &include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..],
        )
        .unwrap();
        snapshot.world = Some(world());
        snapshot.coordinate = Some(squaremap_protocol::wire::ChunkCoordinate { x: 2, z: 2 });
        snapshot.revision = 42;
        let outcome = client
            .accept(&Envelope {
                session_id: second.session_id,
                correlation_id: second.correlation_id,
                payload: Some(envelope::Payload::ChunkSnapshot(snapshot)),
                ..Default::default()
            })
            .unwrap();
        assert!(matches!(outcome, Some(SnapshotOutcome::Snapshot(_))));
        assert_eq!(client.in_flight(), 0);
    }

    #[test]
    fn cancelled_tombstones_are_bounded() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let request = client.request_once(world(), 0, 0, 42).unwrap();
        for i in 0..(MAX_CANCELLED + 7) {
            let extra = client.request_once(world(), i as i32 + 1, 0, 42).unwrap();
            assert!(client.cancel(extra.correlation_id));
        }
        assert!(client.cancelled.len() <= MAX_CANCELLED);
        assert!(client.pending.contains_key(&request.correlation_id));
        assert!(client.cancel(request.correlation_id));
    }

    #[test]
    fn delayed_registry_then_cancel_leaves_no_pending_request() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let request = client.request_once(world(), 1, 1, 42).unwrap();
        push_registry(&mut client, request.correlation_id, &world(), 42);
        assert!(client.cancel(request.correlation_id));
        assert_eq!(client.in_flight(), 0);
    }
    #[test]
    fn registry_replace_requires_matching_pending_correlation() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let request = client.request_once(world(), -7, 5, 42).unwrap();
        let mut foreign = RegistryReplace::decode(
            &include_bytes!("../../../../testdata/bridge/v1/registry_replace_valid.bin")[..],
        )
        .unwrap();
        foreign.block_states.clear();
        assert!(
            client
                .accept(&Envelope {
                    session_id: request.session_id.clone(),
                    correlation_id: request.correlation_id + 1,
                    payload: Some(envelope::Payload::RegistryReplace(foreign)),
                    ..Default::default()
                })
                .unwrap()
                .is_none()
        );
        push_registry(&mut client, request.correlation_id, &world(), 42);
        let snapshot = ChunkSnapshot::decode(
            &include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..],
        )
        .unwrap();
        let response = client
            .accept(&Envelope {
                session_id: request.session_id,
                correlation_id: request.correlation_id,
                payload: Some(envelope::Payload::ChunkSnapshot(snapshot)),
                ..Default::default()
            })
            .unwrap();
        assert!(matches!(response, Some(SnapshotOutcome::Snapshot(_))));
    }

    #[test]
    fn malformed_delayed_registry_for_cancelled_request_is_ignored() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let request = client.request_once(world(), 1, 1, 42).unwrap();
        assert!(client.cancel(request.correlation_id));
        let result = client.accept(&Envelope {
            session_id: request.session_id,
            correlation_id: request.correlation_id,
            payload: Some(envelope::Payload::RegistryReplace(
                RegistryReplace::default(),
            )),
            ..Default::default()
        });
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn cached_registry_is_bound_to_later_request() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let first = client.request_once(world(), -7, 5, 42).unwrap();
        push_registry(&mut client, first.correlation_id, &world(), 42);
        let second = client.request_once(world(), -7, 5, 42).unwrap();
        let snapshot = ChunkSnapshot::decode(
            &include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..],
        )
        .unwrap();
        let outcome = client
            .accept(&Envelope {
                session_id: second.session_id,
                correlation_id: second.correlation_id,
                payload: Some(envelope::Payload::ChunkSnapshot(snapshot)),
                ..Default::default()
            })
            .unwrap();
        assert!(matches!(outcome, Some(SnapshotOutcome::Snapshot(_))));
    }

    #[test]
    fn sibling_in_flight_snapshot_uses_registry_cached_by_the_other_request() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let first = client.request_once(world(), 0, 0, 42).unwrap();
        let second = client.request_once(world(), 1, 0, 42).unwrap();
        push_registry(&mut client, first.correlation_id, &world(), 42);
        let mut snapshot = ChunkSnapshot::decode(
            &include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..],
        )
        .unwrap();
        snapshot.world = Some(world());
        snapshot.coordinate = Some(squaremap_protocol::wire::ChunkCoordinate { x: 1, z: 0 });
        snapshot.revision = 42;
        let outcome = client
            .accept(&Envelope {
                session_id: second.session_id,
                correlation_id: second.correlation_id,
                payload: Some(envelope::Payload::ChunkSnapshot(snapshot)),
                ..Default::default()
            })
            .unwrap();
        assert!(matches!(outcome, Some(SnapshotOutcome::Snapshot(_))));
    }

    #[test]
    fn later_dirty_revision_sibling_uses_fresh_registry_not_stale_request_cache() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let first = client.request_once(world(), 0, 0, 42).unwrap();
        push_registry(&mut client, first.correlation_id, &world(), 42);
        let mut first_snapshot = ChunkSnapshot::decode(
            &include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..],
        )
        .unwrap();
        first_snapshot.world = Some(world());
        first_snapshot.coordinate = Some(squaremap_protocol::wire::ChunkCoordinate { x: 0, z: 0 });
        first_snapshot.revision = 42;
        assert!(matches!(
            client
                .accept(&Envelope {
                    session_id: first.session_id,
                    correlation_id: first.correlation_id,
                    payload: Some(envelope::Payload::ChunkSnapshot(first_snapshot)),
                    ..Default::default()
                })
                .unwrap(),
            Some(SnapshotOutcome::Snapshot(_))
        ));

        let center = client.request_once(world(), 2, 0, 43).unwrap();
        let neighbor = client.request_once(world(), 3, 0, 43).unwrap();
        push_registry(&mut client, center.correlation_id, &world(), 43);
        let mut neighbor_snapshot = ChunkSnapshot::decode(
            &include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..],
        )
        .unwrap();
        neighbor_snapshot.world = Some(world());
        neighbor_snapshot.coordinate =
            Some(squaremap_protocol::wire::ChunkCoordinate { x: 3, z: 0 });
        neighbor_snapshot.revision = 43;
        let outcome = client
            .accept(&Envelope {
                session_id: neighbor.session_id,
                correlation_id: neighbor.correlation_id,
                payload: Some(envelope::Payload::ChunkSnapshot(neighbor_snapshot)),
                ..Default::default()
            })
            .unwrap();
        assert!(matches!(outcome, Some(SnapshotOutcome::Snapshot(_))));
    }

    #[test]
    fn cached_palette_decodes_a_later_dirty_revision_without_a_new_registry_replace() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let first = client.request_once(world(), 0, 0, 42).unwrap();
        push_registry(&mut client, first.correlation_id, &world(), 42);
        let mut first_snapshot = ChunkSnapshot::decode(
            &include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..],
        )
        .unwrap();
        first_snapshot.world = Some(world());
        first_snapshot.coordinate = Some(squaremap_protocol::wire::ChunkCoordinate { x: 0, z: 0 });
        first_snapshot.revision = 42;
        assert!(matches!(
            client
                .accept(&Envelope {
                    session_id: first.session_id,
                    correlation_id: first.correlation_id,
                    payload: Some(envelope::Payload::ChunkSnapshot(first_snapshot)),
                    ..Default::default()
                })
                .unwrap(),
            Some(SnapshotOutcome::Snapshot(_))
        ));

        let second = client.request_once(world(), 1, 0, 43).unwrap();
        let mut snapshot = ChunkSnapshot::decode(
            &include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..],
        )
        .unwrap();
        snapshot.world = Some(world());
        snapshot.coordinate = Some(squaremap_protocol::wire::ChunkCoordinate { x: 1, z: 0 });
        snapshot.revision = 43;
        let outcome = client
            .accept(&Envelope {
                session_id: second.session_id,
                correlation_id: second.correlation_id,
                payload: Some(envelope::Payload::ChunkSnapshot(snapshot)),
                ..Default::default()
            })
            .unwrap();
        assert!(matches!(outcome, Some(SnapshotOutcome::Snapshot(_))));
    }

    #[test]
    fn cancelled_request_still_caches_registry_for_sibling_snapshot() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let first = client.request_once(world(), 0, 0, 42).unwrap();
        let second = client.request_once(world(), 1, 0, 42).unwrap();
        client.cancel(first.correlation_id);
        push_registry(&mut client, first.correlation_id, &world(), 42);
        let mut snapshot = ChunkSnapshot::decode(
            &include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..],
        )
        .unwrap();
        snapshot.world = Some(world());
        snapshot.coordinate = Some(squaremap_protocol::wire::ChunkCoordinate { x: 1, z: 0 });
        snapshot.revision = 42;
        let outcome = client
            .accept(&Envelope {
                session_id: second.session_id,
                correlation_id: second.correlation_id,
                payload: Some(envelope::Payload::ChunkSnapshot(snapshot)),
                ..Default::default()
            })
            .unwrap();
        assert!(matches!(outcome, Some(SnapshotOutcome::Snapshot(_))));
    }

    #[test]
    fn registry_generation_is_reset_for_new_world_epoch() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let first = client.request_once(world(), 0, 0, 42).unwrap();
        push_registry(&mut client, first.correlation_id, &world(), 42);
        let new_world = WorldIdentity {
            epoch: 4,
            ..world()
        };
        let second = client.request_once(new_world.clone(), 1, 1, 42).unwrap();
        push_registry(&mut client, second.correlation_id, &new_world, 42);
        let mut snapshot = ChunkSnapshot::decode(
            &include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..],
        )
        .unwrap();
        snapshot.world = Some(new_world);
        snapshot.coordinate = Some(squaremap_protocol::wire::ChunkCoordinate { x: 1, z: 1 });
        let response = client
            .accept(&Envelope {
                session_id: second.session_id,
                correlation_id: second.correlation_id,
                payload: Some(envelope::Payload::ChunkSnapshot(snapshot)),
                ..Default::default()
            })
            .unwrap();
        assert!(matches!(response, Some(SnapshotOutcome::Snapshot(_))));
    }

    #[test]
    fn unsolicited_snapshot_and_registry_are_ignored_without_mutation() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let registry = RegistryReplace::decode(
            &include_bytes!("../../../../testdata/bridge/v1/registry_replace_valid.bin")[..],
        )
        .unwrap();
        assert!(
            client
                .accept(&Envelope {
                    session_id: vec![9; 16],
                    payload: Some(envelope::Payload::RegistryReplace(registry)),
                    ..Default::default()
                })
                .unwrap()
                .is_none()
        );
        let snapshot = ChunkSnapshot::decode(
            &include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..],
        )
        .unwrap();
        assert!(
            client
                .accept(&Envelope {
                    session_id: vec![9; 16],
                    correlation_id: 999,
                    payload: Some(envelope::Payload::ChunkSnapshot(snapshot)),
                    ..Default::default()
                })
                .unwrap()
                .is_none()
        );
        assert_eq!(client.in_flight(), 0);
    }

    #[test]
    fn binding_error_keeps_pending_request() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let request = client.request_once(world(), 0, 0, 42).unwrap();
        let bad = ChunkMissing {
            world: Some(WorldIdentity {
                value: "wrong".into(),
                ..world()
            }),
            coordinate: Some(Default::default()),
            revision: 42,
            reason: ChunkMissingReason::Unavailable as i32,
        };
        assert!(matches!(
            client.accept(&Envelope {
                session_id: request.session_id,
                correlation_id: request.correlation_id,
                payload: Some(envelope::Payload::ChunkMissing(bad)),
                ..Default::default()
            }),
            Err(SnapshotClientError::InvalidResponse(_))
        ));
        assert_eq!(client.in_flight(), 1);
    }

    #[test]
    fn coordinate_binding_rejects_wrong_x_and_z_for_snapshot_and_missing() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let request = client.request_once(world(), -7, 5, 42).unwrap();
        push_registry(&mut client, request.correlation_id, &world(), 42);
        let mut wrong_x = ChunkSnapshot::decode(
            &include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..],
        )
        .unwrap();
        wrong_x.coordinate.as_mut().unwrap().x += 1;
        assert!(matches!(
            client.accept(&Envelope {
                session_id: request.session_id.clone(),
                correlation_id: request.correlation_id,
                payload: Some(envelope::Payload::ChunkSnapshot(wrong_x)),
                ..Default::default()
            }),
            Err(SnapshotClientError::InvalidResponse(_))
        ));
        assert_eq!(client.in_flight(), 1);
        let mut wrong_z = ChunkSnapshot::decode(
            &include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..],
        )
        .unwrap();
        wrong_z.coordinate.as_mut().unwrap().z += 1;
        assert!(matches!(
            client.accept(&Envelope {
                session_id: request.session_id,
                correlation_id: request.correlation_id,
                payload: Some(envelope::Payload::ChunkSnapshot(wrong_z)),
                ..Default::default()
            }),
            Err(SnapshotClientError::InvalidResponse(_))
        ));
        assert_eq!(client.in_flight(), 1);

        let mut missing_client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let missing_request = missing_client.request_once(world(), -7, 5, 42).unwrap();
        for (x, z) in [(-6, 5), (-7, 6)] {
            let missing = ChunkMissing {
                world: Some(world()),
                coordinate: Some(squaremap_protocol::wire::ChunkCoordinate { x, z }),
                revision: 42,
                reason: ChunkMissingReason::Unavailable as i32,
            };
            assert!(matches!(
                missing_client.accept(&Envelope {
                    session_id: missing_request.session_id.clone(),
                    correlation_id: missing_request.correlation_id,
                    payload: Some(envelope::Payload::ChunkMissing(missing)),
                    ..Default::default()
                }),
                Err(SnapshotClientError::InvalidResponse(_))
            ));
            assert_eq!(missing_client.in_flight(), 1);
        }
    }

    #[test]
    fn identical_registry_replacement_pins_new_request_generation() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let first = client.request_once(world(), 4, 4, 42).unwrap();
        push_registry(&mut client, first.correlation_id, &world(), 42);
        let current = client
            .registries
            .get(&world_key(&world()))
            .unwrap()
            .generation();
        let mut first_snapshot = ChunkSnapshot::decode(
            &include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..],
        )
        .unwrap();
        first_snapshot.world = Some(world());
        first_snapshot.coordinate = Some(squaremap_protocol::wire::ChunkCoordinate { x: 4, z: 4 });
        assert!(matches!(
            client
                .accept(&Envelope {
                    session_id: first.session_id.clone(),
                    correlation_id: first.correlation_id,
                    payload: Some(envelope::Payload::ChunkSnapshot(first_snapshot)),
                    ..Default::default()
                })
                .unwrap(),
            Some(SnapshotOutcome::Snapshot(_))
        ));
        let second = client.request_once(world(), 3, 4, 42).unwrap();
        let mut identical = RegistryReplace::decode(
            &include_bytes!("../../../../testdata/bridge/v1/registry_replace_valid.bin")[..],
        )
        .unwrap();
        identical.world = Some(world());
        identical.revision = 42;
        client
            .accept(&Envelope {
                session_id: second.session_id.clone(),
                correlation_id: second.correlation_id,
                payload: Some(envelope::Payload::RegistryReplace(identical)),
                ..Default::default()
            })
            .unwrap();
        let pinned = client
            .pending
            .get(&second.correlation_id)
            .and_then(|pending| pending.generation.clone())
            .unwrap();
        assert!(std::sync::Arc::ptr_eq(&current, &pinned));

        let mut snapshot = ChunkSnapshot::decode(
            &include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..],
        )
        .unwrap();
        snapshot.world = Some(world());
        snapshot.coordinate = Some(squaremap_protocol::wire::ChunkCoordinate { x: 3, z: 4 });
        assert!(matches!(
            client
                .accept(&Envelope {
                    session_id: second.session_id,
                    correlation_id: second.correlation_id,
                    payload: Some(envelope::Payload::ChunkSnapshot(snapshot)),
                    ..Default::default()
                })
                .unwrap(),
            Some(SnapshotOutcome::Snapshot(_))
        ));
    }
    #[test]
    fn delayed_same_revision_snapshots_keep_their_registry_generation() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let first = client.request_once(world(), 4, 4, 42).unwrap();
        push_registry_with_descriptor_two_color(
            &mut client,
            first.correlation_id,
            &world(),
            42,
            0x0001_0203,
        );
        let first_generation = client
            .pending
            .get(&first.correlation_id)
            .and_then(|pending| pending.generation.clone())
            .unwrap();
        assert_eq!(
            first_generation.block(2).expect("descriptor 2").map_color,
            0x0001_0203,
        );

        let second = client.request_once(world(), 5, 4, 42).unwrap();
        push_registry_with_descriptor_two_color(
            &mut client,
            second.correlation_id,
            &world(),
            42,
            0x00f0_e0d0,
        );
        let second_generation = client
            .pending
            .get(&second.correlation_id)
            .and_then(|pending| pending.generation.clone())
            .unwrap();
        assert!(!std::sync::Arc::ptr_eq(
            &first_generation,
            &second_generation,
        ));
        assert_eq!(
            second_generation.block(2).expect("descriptor 2").map_color,
            0x00f0_e0d0,
        );
        let current = client
            .registries
            .get(&world_key(&world()))
            .expect("current registry")
            .generation();
        assert!(std::sync::Arc::ptr_eq(&current, &second_generation));
        assert_eq!(
            current.block(2).expect("descriptor 2").map_color,
            0x00f0_e0d0,
        );

        let mut second_snapshot = ChunkSnapshot::decode(
            &include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..],
        )
        .unwrap();
        second_snapshot.world = Some(world());
        second_snapshot.coordinate = Some(squaremap_protocol::wire::ChunkCoordinate { x: 5, z: 4 });
        second_snapshot.revision = 42;
        let second_outcome = client
            .accept(&Envelope {
                session_id: second.session_id,
                correlation_id: second.correlation_id,
                payload: Some(envelope::Payload::ChunkSnapshot(second_snapshot)),
                ..Default::default()
            })
            .unwrap();
        let Some(SnapshotOutcome::Snapshot(second_decoded)) = second_outcome else {
            panic!("expected second decoded snapshot");
        };
        assert!(std::sync::Arc::ptr_eq(
            &second_decoded.registry_generation,
            &second_generation,
        ));
        assert_eq!(
            second_decoded
                .registry_generation
                .block(2)
                .expect("descriptor 2")
                .map_color,
            0x00f0_e0d0,
        );

        let mut first_snapshot = ChunkSnapshot::decode(
            &include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..],
        )
        .unwrap();
        first_snapshot.world = Some(world());
        first_snapshot.coordinate = Some(squaremap_protocol::wire::ChunkCoordinate { x: 4, z: 4 });
        first_snapshot.revision = 42;
        let first_outcome = client
            .accept(&Envelope {
                session_id: first.session_id,
                correlation_id: first.correlation_id,
                payload: Some(envelope::Payload::ChunkSnapshot(first_snapshot)),
                ..Default::default()
            })
            .unwrap();
        let Some(SnapshotOutcome::Snapshot(first_decoded)) = first_outcome else {
            panic!("expected delayed first decoded snapshot");
        };
        assert!(std::sync::Arc::ptr_eq(
            &first_decoded.registry_generation,
            &first_generation,
        ));
        assert_eq!(
            first_decoded
                .registry_generation
                .block(2)
                .expect("descriptor 2")
                .map_color,
            0x0001_0203,
        );
        assert_eq!(client.in_flight(), 0);
    }

    #[test]
    fn registry_revisions_are_monotonic_but_equal_revision_replaces_generation() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let first = client.request_once(world(), 0, 0, 43).unwrap();
        push_registry(&mut client, first.correlation_id, &world(), 43);
        let key = world_key(&world());
        assert_eq!(client.registries.get(&key).unwrap().revision(), 43);
        let pinned = client
            .pending
            .get(&first.correlation_id)
            .and_then(|pending| pending.generation.clone())
            .unwrap();
        let mut delayed_replacement = RegistryReplace::decode(
            &include_bytes!("../../../../testdata/bridge/v1/registry_replace_valid.bin")[..],
        )
        .unwrap();
        delayed_replacement.world = Some(world());
        delayed_replacement.revision = 43;
        delayed_replacement.block_states[0].map_color ^= 1;
        client
            .accept(&Envelope {
                session_id: first.session_id.clone(),
                correlation_id: first.correlation_id,
                payload: Some(envelope::Payload::RegistryReplace(delayed_replacement)),
                ..Default::default()
            })
            .unwrap();
        let still_pinned = client
            .pending
            .get(&first.correlation_id)
            .and_then(|pending| pending.generation.clone())
            .unwrap();
        assert!(std::sync::Arc::ptr_eq(&pinned, &still_pinned));
        let second = client.request_once(world(), 1, 1, 42).unwrap();
        push_registry(&mut client, second.correlation_id, &world(), 42);
        assert_eq!(client.registries.get(&key).unwrap().revision(), 43);

        let third = client.request_once(world(), 2, 2, 43).unwrap();
        let mut replacement = RegistryReplace::decode(
            &include_bytes!("../../../../testdata/bridge/v1/registry_replace_valid.bin")[..],
        )
        .unwrap();
        replacement.world = Some(world());
        replacement.revision = 43;
        replacement.block_states[0].map_color ^= 2;
        let before = client.registries.get(&key).unwrap().generation();
        client
            .accept(&Envelope {
                session_id: third.session_id,
                correlation_id: third.correlation_id,
                payload: Some(envelope::Payload::RegistryReplace(replacement)),
                ..Default::default()
            })
            .unwrap();
        let after = client.registries.get(&key).unwrap().generation();
        assert!(!std::sync::Arc::ptr_eq(&before, &after));
    }

    #[test]
    fn snapshot_before_registry_completes_when_registry_arrives() {
        let mut client = SnapshotClient::new([9; 16], Limits::default()).unwrap();
        let request = client.request_once(world(), -7, 5, 42).unwrap();
        let snapshot = ChunkSnapshot::decode(
            &include_bytes!("../../../../testdata/bridge/v1/chunk_snapshot_valid.bin")[..],
        )
        .unwrap();
        let held = client
            .accept(&Envelope {
                session_id: request.session_id.clone(),
                correlation_id: request.correlation_id,
                payload: Some(envelope::Payload::ChunkSnapshot(snapshot)),
                ..Default::default()
            })
            .unwrap();
        assert!(
            held.is_none(),
            "snapshot without registry must wait, not fail"
        );
        assert_eq!(client.in_flight(), 1);
        let mut registry = RegistryReplace::decode(
            &include_bytes!("../../../../testdata/bridge/v1/registry_replace_valid.bin")[..],
        )
        .unwrap();
        registry.world = Some(world());
        registry.revision = 42;
        let completed = client
            .accept(&Envelope {
                session_id: request.session_id,
                correlation_id: request.correlation_id,
                payload: Some(envelope::Payload::RegistryReplace(registry)),
                ..Default::default()
            })
            .unwrap();
        assert!(
            matches!(completed, Some(SnapshotOutcome::Snapshot(_))),
            "held snapshot must decode after registry, got {completed:?}"
        );
        assert_eq!(client.in_flight(), 0);
    }
}
