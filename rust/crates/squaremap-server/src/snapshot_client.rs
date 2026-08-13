use squaremap_protocol::wire::{
    ChunkMissingReason, ChunkSnapshotRequest, Envelope, RegistryReplace, WorldIdentity, envelope,
};
use squaremap_render::{GenerationToken, Limits, Registry, RegistryError, Snapshot};
use std::collections::HashMap;
use std::fmt;

pub const MAX_IN_FLIGHT: usize = 96;

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
            pending: HashMap::new(),
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
        };
        self.pending.insert(
            correlation,
            Pending {
                world,
                coordinate: squaremap_protocol::wire::ChunkCoordinate { x, z },
                revision,
                generation: None,
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
                let generation =
                    pending
                        .generation
                        .as_ref()
                        .ok_or(SnapshotClientError::InvalidResponse(
                            "snapshot registry unavailable",
                        ))?;
                let decoded = Snapshot::decode_for(snapshot, generation, self.limits)?;
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
        let world = replacement
            .world
            .as_ref()
            .ok_or(SnapshotClientError::InvalidResponse(
                "registry missing world",
            ))?;
        let Some(pending) = self.pending.get(&correlation_id) else {
            return Ok(None);
        };
        if pending.revision != replacement.revision || pending.world != *world {
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
            return Ok(None);
        }
        let registry = self.registries.entry(key.clone()).or_default();
        registry.replace(replacement.clone())?;
        if let Some(pending) = self.pending.get_mut(&correlation_id) {
            if pending.generation.is_none() {
                pending.generation = Some(registry.generation());
            }
        }
        self.registry_replacements.insert(key, replacement.clone());
        Ok(None)
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
}
