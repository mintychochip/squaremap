use squaremap_protocol::wire::{
    ChunkCoordinate, DirtyReplayItem, DirtyReplayRequest, DirtyResyncComplete, ResumeWatermark,
    WorldEnumerationComplete, WorldEnumerationItem, WorldEnumerationRequest, WorldIdentity,
};
use squaremap_state::{DirtyChunk, Repository, RepositoryError};
pub const MAX_REPLAY_ITEMS: u32 = 1024;
pub const MAX_ENUMERATION_ITEMS: u32 = 1024;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayState {
    request: DirtyReplayRequest,
    next_index: u32,
    terminal: bool,
}

impl ReplayState {
    pub fn new(
        request: DirtyReplayRequest,
        bridge_id: &[u8],
        session_id: &[u8],
        config_revision: u64,
    ) -> Result<Self, DirtyResyncError> {
        validate_request(
            &request,
            bridge_id,
            session_id,
            config_revision,
            MAX_REPLAY_ITEMS,
        )?;
        Ok(Self {
            request,
            next_index: 0,
            terminal: false,
        })
    }

    pub fn accept_item(&mut self, item: &DirtyReplayItem) -> Result<(), DirtyResyncError> {
        if self.terminal {
            return Err(DirtyResyncError::InvalidCompletion);
        }
        validate_item(item, &self.request, self.next_index)?;
        self.next_index = self.next_index.saturating_add(1);
        Ok(())
    }

    pub fn complete(&mut self, completion: &DirtyResyncComplete) -> Result<(), DirtyResyncError> {
        if self.terminal {
            return Err(DirtyResyncError::InvalidCompletion);
        }
        validate_completion(completion, &self.request, self.next_index)?;
        self.terminal = true;
        Ok(())
    }

    pub fn request(&self) -> &DirtyReplayRequest {
        &self.request
    }
    pub fn next_index(&self) -> u32 {
        self.next_index
    }
    pub fn terminal(&self) -> bool {
        self.terminal
    }
}

pub async fn resume_watermark(
    repository: &Repository,
    bridge_id: &[u8],
    session_id: &[u8],
    config_revision: u64,
) -> Result<ResumeWatermark, RepositoryError> {
    let checkpoint = repository.bridge_checkpoint(bridge_id).await?;
    Ok(ResumeWatermark {
        bridge_id: bridge_id.to_vec(),
        session_id: session_id.to_vec(),
        config_revision,
        last_durable_sequence: checkpoint.map_or(0, |value| value.durable_sequence),
    })
}

pub fn replay_item(
    dirty: &DirtyChunk,
    bridge_id: &[u8],
    session_id: &[u8],
    config_revision: u64,
    replay_id: u64,
    item_index: u32,
) -> DirtyReplayItem {
    DirtyReplayItem {
        bridge_id: bridge_id.to_vec(),
        session_id: session_id.to_vec(),
        config_revision,
        replay_id,
        item_index,
        world: Some(WorldIdentity {
            namespace: dirty.world.namespace.clone(),
            value: dirty.world.value.clone(),
            epoch: dirty.world.epoch,
        }),
        coordinate: Some(ChunkCoordinate {
            x: dirty.coordinate.x,
            z: dirty.coordinate.z,
        }),
        revision: dirty.revision,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirtyResyncError {
    InvalidSession,
    RevisionMismatch,
    WorldMismatch,
    MissingCursor,
    InvalidCursor,
    InvalidPageSize { max: u32 },
    InvalidIndex { expected: u32, actual: u32 },
    DuplicateIndex(u32),
    ReplayMismatch,
    InvalidCompletion,
}
pub fn validate_bridge_identity(bridge_id: &[u8]) -> Result<(), DirtyResyncError> {
    if bridge_id.len() != 16 || bridge_id.iter().all(|byte| *byte == 0) {
        return Err(DirtyResyncError::InvalidSession);
    }
    Ok(())
}

pub fn validate_resume(
    watermark: &ResumeWatermark,
    bridge_id: &[u8],
    session_id: &[u8],
    config_revision: u64,
) -> Result<(), DirtyResyncError> {
    if validate_bridge_identity(bridge_id).is_err() || watermark.bridge_id.as_slice() != bridge_id {
        return Err(DirtyResyncError::InvalidSession);
    }
    if watermark.session_id.as_slice() != session_id || session_id.len() != 16 {
        return Err(DirtyResyncError::InvalidSession);
    }
    if watermark.config_revision != config_revision {
        return Err(DirtyResyncError::RevisionMismatch);
    }
    Ok(())
}
pub fn validate_request(
    request: &DirtyReplayRequest,
    bridge_id: &[u8],
    session_id: &[u8],
    config_revision: u64,
    max_items: u32,
) -> Result<(), DirtyResyncError> {
    if validate_bridge_identity(bridge_id).is_err() || request.bridge_id.as_slice() != bridge_id {
        return Err(DirtyResyncError::InvalidSession);
    }
    if request.session_id.as_slice() != session_id || session_id.len() != 16 {
        return Err(DirtyResyncError::InvalidSession);
    }
    if request.config_revision != config_revision {
        return Err(DirtyResyncError::RevisionMismatch);
    }
    if request.world.as_ref().is_none_or(|world| {
        world.epoch == 0 || world.namespace.is_empty() || world.value.is_empty()
    }) {
        return Err(DirtyResyncError::WorldMismatch);
    }
    if request.max_items == 0
        || request.max_items > max_items
        || request.max_items > MAX_REPLAY_ITEMS
    {
        return Err(DirtyResyncError::InvalidPageSize {
            max: max_items.min(MAX_REPLAY_ITEMS),
        });
    }
    if request.cursor.is_none() {
        return Err(DirtyResyncError::MissingCursor);
    }
    Ok(())
}
pub fn validate_item(
    item: &DirtyReplayItem,
    request: &DirtyReplayRequest,
    expected_index: u32,
) -> Result<(), DirtyResyncError> {
    if validate_bridge_identity(&request.bridge_id).is_err() || item.bridge_id != request.bridge_id
    {
        return Err(DirtyResyncError::InvalidSession);
    }
    if item.session_id != request.session_id || item.session_id.len() != 16 {
        return Err(DirtyResyncError::InvalidSession);
    }
    if expected_index >= request.max_items {
        return Err(DirtyResyncError::InvalidIndex {
            expected: request.max_items,
            actual: expected_index,
        });
    }
    if item.config_revision != request.config_revision {
        return Err(DirtyResyncError::RevisionMismatch);
    }
    if item.replay_id != request.replay_id {
        return Err(DirtyResyncError::ReplayMismatch);
    }
    if item.world != request.world {
        return Err(DirtyResyncError::WorldMismatch);
    }
    if item.item_index < expected_index {
        return Err(DirtyResyncError::DuplicateIndex(item.item_index));
    }
    if item.item_index != expected_index {
        return Err(DirtyResyncError::InvalidIndex {
            expected: expected_index,
            actual: item.item_index,
        });
    }
    if item.coordinate.is_none() || item.revision == 0 {
        return Err(DirtyResyncError::WorldMismatch);
    }
    Ok(())
}
pub fn validate_completion(
    completion: &DirtyResyncComplete,
    request: &DirtyReplayRequest,
    expected_count: u32,
) -> Result<(), DirtyResyncError> {
    if validate_bridge_identity(&request.bridge_id).is_err()
        || completion.bridge_id != request.bridge_id
    {
        return Err(DirtyResyncError::InvalidSession);
    }
    if completion.session_id != request.session_id || completion.session_id.len() != 16 {
        return Err(DirtyResyncError::InvalidSession);
    }
    if expected_count > request.max_items {
        return Err(DirtyResyncError::InvalidIndex {
            expected: request.max_items,
            actual: expected_count,
        });
    }
    if completion.config_revision != request.config_revision
        || completion.replay_id != request.replay_id
    {
        return Err(DirtyResyncError::RevisionMismatch);
    }
    if completion.item_count != expected_count {
        return Err(DirtyResyncError::InvalidIndex {
            expected: expected_count,
            actual: completion.item_count,
        });
    }
    if completion.has_more {
        if completion.last_coordinate.is_none() {
            return Err(DirtyResyncError::InvalidCursor);
        }
        if completion.item_count != request.max_items {
            return Err(DirtyResyncError::InvalidCompletion);
        }
    } else if completion.last_coordinate.is_some() {
        return Err(DirtyResyncError::InvalidCompletion);
    }
    let status = squaremap_protocol::wire::DirtyResyncStatus::try_from(completion.status).ok();
    if status == Some(squaremap_protocol::wire::DirtyResyncStatus::Failed)
        && completion.failure_reason.is_empty()
    {
        return Err(DirtyResyncError::InvalidCompletion);
    }
    if status.is_none() || status == Some(squaremap_protocol::wire::DirtyResyncStatus::Unspecified)
    {
        return Err(DirtyResyncError::InvalidCompletion);
    }
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldEnumerationError {
    InvalidSession,
    RevisionMismatch,
    WorldMismatch,
    InvalidPageSize { max: u32 },
    InvalidIndex { expected: u32, actual: u32 },
    DuplicateIndex(u32),
    EnumerationMismatch,
    InvalidCompletion,
}

pub fn validate_enumeration_request(
    request: &WorldEnumerationRequest,
    bridge_id: &[u8],
    session_id: &[u8],
    config_revision: u64,
    max_items: u32,
) -> Result<(), WorldEnumerationError> {
    if validate_bridge_identity(bridge_id).is_err()
        || request.bridge_id.as_slice() != bridge_id
        || request.session_id.as_slice() != session_id
        || session_id.len() != 16
    {
        return Err(WorldEnumerationError::InvalidSession);
    }
    if request.config_revision != config_revision {
        return Err(WorldEnumerationError::RevisionMismatch);
    }
    if request.world.as_ref().is_none_or(|world| {
        world.epoch == 0 || world.namespace.is_empty() || world.value.is_empty()
    }) {
        return Err(WorldEnumerationError::WorldMismatch);
    }
    if request.enumeration_id == 0
        || request.max_items == 0
        || request.max_items > max_items
        || request.max_items > MAX_ENUMERATION_ITEMS
    {
        return Err(WorldEnumerationError::InvalidPageSize {
            max: max_items.min(MAX_ENUMERATION_ITEMS),
        });
    }
    Ok(())
}

pub fn validate_enumeration_item(
    item: &WorldEnumerationItem,
    request: &WorldEnumerationRequest,
    expected_index: u32,
) -> Result<(), WorldEnumerationError> {
    if item.bridge_id != request.bridge_id
        || item.session_id != request.session_id
        || item.session_id.len() != 16
    {
        return Err(WorldEnumerationError::InvalidSession);
    }
    if item.config_revision != request.config_revision {
        return Err(WorldEnumerationError::RevisionMismatch);
    }
    if item.enumeration_id != request.enumeration_id {
        return Err(WorldEnumerationError::EnumerationMismatch);
    }
    if expected_index >= request.max_items {
        return Err(WorldEnumerationError::InvalidIndex {
            expected: request.max_items,
            actual: expected_index,
        });
    }
    if item.item_index < expected_index {
        return Err(WorldEnumerationError::DuplicateIndex(item.item_index));
    }
    if item.item_index != expected_index {
        return Err(WorldEnumerationError::InvalidIndex {
            expected: expected_index,
            actual: item.item_index,
        });
    }
    if item.world != request.world || item.coordinate.is_none() {
        return Err(WorldEnumerationError::WorldMismatch);
    }
    Ok(())
}

pub fn validate_enumeration_completion(
    completion: &WorldEnumerationComplete,
    request: &WorldEnumerationRequest,
    expected_count: u32,
) -> Result<(), WorldEnumerationError> {
    if completion.bridge_id != request.bridge_id
        || completion.session_id != request.session_id
        || completion.session_id.len() != 16
    {
        return Err(WorldEnumerationError::InvalidSession);
    }
    if completion.config_revision != request.config_revision
        || completion.enumeration_id != request.enumeration_id
    {
        return Err(WorldEnumerationError::RevisionMismatch);
    }
    if expected_count > request.max_items || completion.item_count != expected_count {
        return Err(WorldEnumerationError::InvalidIndex {
            expected: expected_count,
            actual: completion.item_count,
        });
    }
    if completion.page_index != request.page_index {
        return Err(WorldEnumerationError::InvalidIndex {
            expected: request.page_index,
            actual: completion.page_index,
        });
    }
    Ok(())
}
pub fn validate_enumeration_page(
    request: &WorldEnumerationRequest,
    expected_page: u32,
) -> Result<(), WorldEnumerationError> {
    if request.page_index != expected_page {
        return Err(WorldEnumerationError::InvalidIndex {
            expected: expected_page,
            actual: request.page_index,
        });
    }
    Ok(())
}
