use squaremap_protocol::wire::{backend_substitution, BackendResultCode, BackendSubstitution, ControlKind, ControlRequest, ControlResult, WorldIdentity};
use std::collections::HashSet;

#[derive(Default)]
pub struct ControlState { in_config: bool, worlds: HashSet<(String, String, u64)> }

impl ControlState {
    pub fn is_in_config(&self) -> bool { self.in_config }
    pub fn replace_worlds(&mut self, identities: Vec<WorldIdentity>) {
        let mut next = HashSet::new();
        for identity in identities { next.insert(key(&identity)); }
        self.worlds = next;
        self.in_config = true;
    }
    pub fn world_count(&self) -> usize { self.worlds.len() }
    pub fn handle(&mut self, request: &ControlRequest) -> ControlResult {
        let Some(kind) = ControlKind::try_from(request.kind).ok() else { return result(BackendResultCode::InvalidRequest, None); };
        if kind == ControlKind::Health {
            return result(if self.in_config { BackendResultCode::Healthy } else { BackendResultCode::BackendUnavailable }, None);
        }
        if kind == ControlKind::Reload {
            return result(BackendResultCode::BackendUnavailable, None);
        }
        let Some(identity) = request.world.as_ref() else { return result(BackendResultCode::InvalidRequest, None); };
        if !self.worlds.contains(&key(identity)) { return result(BackendResultCode::UnknownWorld, Some(identity)); }
        match kind {
            ControlKind::FullRender | ControlKind::RadiusRender | ControlKind::CancelRender
            | ControlKind::PauseRenders | ControlKind::ResetMap => result(BackendResultCode::BackendUnavailable, Some(identity)),
            ControlKind::Health | ControlKind::Reload | ControlKind::Unspecified => result(BackendResultCode::InvalidRequest, Some(identity)),
        }
    }
}
fn key(identity: &WorldIdentity) -> (String, String, u64) { (identity.namespace.clone(), identity.value.clone(), identity.epoch) }
fn result(code: BackendResultCode, identity: Option<&WorldIdentity>) -> ControlResult {
    let substitutions = identity.into_iter().map(|world| BackendSubstitution { key: "world".into(), value: Some(backend_substitution::Value::WorldIdentity(world.clone())) }).collect();
    ControlResult { code: code as i32, substitutions, rendered_chunks: 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn world(namespace: &str, value: &str, epoch: u64) -> WorldIdentity { WorldIdentity { namespace: namespace.into(), value: value.into(), epoch } }
    fn request(kind: ControlKind, identity: Option<WorldIdentity>) -> ControlRequest { ControlRequest { kind: kind as i32, world: identity, center_x: 0, center_z: 0, radius: 4 } }

    #[test]
    fn controls_are_unavailable_without_fake_state_mutation() {
        let configured = world("minecraft", "overworld", 7);
        let mut state = ControlState::default();
        state.replace_worlds(vec![configured.clone()]);
        for kind in [ControlKind::FullRender, ControlKind::RadiusRender, ControlKind::CancelRender, ControlKind::PauseRenders, ControlKind::ResetMap] {
            assert_eq!(state.handle(&request(kind, Some(configured.clone()))).code, BackendResultCode::BackendUnavailable as i32);
        }
        assert_eq!(state.world_count(), 1);
    }

    #[test]
    fn health_requires_an_accepted_configuration() {
        let health = request(ControlKind::Health, None);
        let mut state = ControlState::default();
        assert_eq!(state.handle(&health).code, BackendResultCode::BackendUnavailable as i32);
        state.replace_worlds(vec![world("minecraft", "overworld", 1)]);
        assert_eq!(state.handle(&health).code, BackendResultCode::Healthy as i32);
    }

    #[test]
    fn accepted_world_epoch_is_resolved_and_unknown_world_is_rejected() {
        let configured = world("minecraft", "overworld", 9);
        let mut state = ControlState::default();
        state.replace_worlds(vec![configured.clone()]);
        assert_eq!(state.handle(&request(ControlKind::FullRender, Some(configured))).code, BackendResultCode::BackendUnavailable as i32);
        assert_eq!(state.handle(&request(ControlKind::FullRender, Some(world("minecraft", "overworld", 0)))).code, BackendResultCode::UnknownWorld as i32);
    }

    #[test]
    fn accepted_config_replaces_old_world_table() {
        let mut state = ControlState::default();
        let first = world("minecraft", "overworld", 1);
        let second = world("minecraft", "nether", 2);
        state.replace_worlds(vec![first.clone()]);
        state.replace_worlds(vec![second.clone()]);
        assert_eq!(state.world_count(), 1);
        assert_eq!(state.handle(&request(ControlKind::FullRender, Some(first))).code, BackendResultCode::UnknownWorld as i32);
        assert_eq!(state.handle(&request(ControlKind::FullRender, Some(second))).code, BackendResultCode::BackendUnavailable as i32);
    }

    #[test]
    fn reload_does_not_fake_success_without_java_reload_executor() {
        let mut state = ControlState::default();
        state.replace_worlds(vec![world("minecraft", "overworld", 1)]);
        assert_eq!(state.handle(&request(ControlKind::Reload, None)).code, BackendResultCode::BackendUnavailable as i32);
    }
}
