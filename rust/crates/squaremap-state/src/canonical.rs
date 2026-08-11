use std::collections::{BTreeMap, HashMap};

/// Retained bridge replacement state, kept separately from output-byte caches.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanonicalState {
    pub worlds_revision: u64,
    pub players_revision: u64,
    pub icons_revision: u64,
    pub marker_revisions: HashMap<String, u64>,
    pub world_epochs: HashMap<String, u64>,
    pub worlds: Vec<u8>,
    pub players: Vec<u8>,
    pub icons: Vec<u8>,
    pub world_outputs: BTreeMap<String, Vec<u8>>,
    pub marker_outputs: BTreeMap<String, Vec<u8>>,
    pub icon_assets: BTreeMap<String, Vec<u8>>,
}
