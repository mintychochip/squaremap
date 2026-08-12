use squaremap_protocol::wire::{BiomeDescriptor, BlockStateDescriptor, RegistryReplace};
use std::collections::HashMap;
use std::fmt;

#[derive(Clone, Debug, Default)]
pub struct Registry {
    revision: u64,
    world: Option<squaremap_protocol::wire::WorldIdentity>,
    blocks: HashMap<u32, BlockStateDescriptor>,
    biomes: HashMap<u32, BiomeDescriptor>,
}

pub const MAX_BLOCK_DESCRIPTORS: usize = 65_536;
pub const MAX_BIOME_DESCRIPTORS: usize = 16_384;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryError {
    DuplicateBlock(u32),
    DuplicateBiome(u32),
    ZeroId,
    DescriptorCount { kind: &'static str, count: usize, max: usize },
}
impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateBlock(id) => write!(f, "duplicate block descriptor {id}"),
            Self::DuplicateBiome(id) => write!(f, "duplicate biome descriptor {id}"),
            Self::ZeroId => write!(f, "descriptor ID must be nonzero"),
            Self::DescriptorCount { kind, count, max } => write!(f, "{kind} descriptor count {count} exceeds {max}"),
        }
    }
}
impl std::error::Error for RegistryError {}

impl Registry {
    pub fn revision(&self) -> u64 { self.revision }
    pub fn world(&self) -> Option<&squaremap_protocol::wire::WorldIdentity> { self.world.as_ref() }
    pub fn replace(&mut self, value: RegistryReplace) -> Result<(), RegistryError> {
        if value.block_states.len() > MAX_BLOCK_DESCRIPTORS {
            return Err(RegistryError::DescriptorCount { kind: "block", count: value.block_states.len(), max: MAX_BLOCK_DESCRIPTORS });
        }
        if value.biomes.len() > MAX_BIOME_DESCRIPTORS {
            return Err(RegistryError::DescriptorCount { kind: "biome", count: value.biomes.len(), max: MAX_BIOME_DESCRIPTORS });
        }
        let mut blocks = HashMap::with_capacity(value.block_states.len());
        for descriptor in value.block_states {
            let id = descriptor.id;
            if id == 0 { return Err(RegistryError::ZeroId); }
            if blocks.insert(id, descriptor).is_some() { return Err(RegistryError::DuplicateBlock(id)); }
        }
        let mut biomes = HashMap::with_capacity(value.biomes.len());
        for descriptor in value.biomes {
            let id = descriptor.id;
            if id == 0 { return Err(RegistryError::ZeroId); }
            if biomes.insert(id, descriptor).is_some() { return Err(RegistryError::DuplicateBiome(id)); }
        }
        self.revision = value.revision;
        self.world = value.world;
        self.blocks = blocks;
        self.biomes = biomes;
        Ok(())
    }
    pub fn with_replace(value: RegistryReplace) -> Result<Self, RegistryError> { let mut r = Self::default(); r.replace(value)?; Ok(r) }
    pub fn block(&self, id: u32) -> Option<&BlockStateDescriptor> { self.blocks.get(&id) }
    pub fn biome(&self, id: u32) -> Option<&BiomeDescriptor> { self.biomes.get(&id) }
    pub fn block_ids(&self) -> Vec<u32> { let mut ids: Vec<_> = self.blocks.keys().copied().collect(); ids.sort_unstable(); ids }
    pub fn biome_ids(&self) -> Vec<u32> { let mut ids: Vec<_> = self.biomes.keys().copied().collect(); ids.sort_unstable(); ids }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_are_returned_in_deterministic_order() {
        let mut replace = RegistryReplace { revision: 1, ..Default::default() };
        replace.block_states = vec![BlockStateDescriptor { id: 9, ..Default::default() }, BlockStateDescriptor { id: 2, ..Default::default() }];
        replace.biomes = vec![BiomeDescriptor { id: 8, ..Default::default() }, BiomeDescriptor { id: 1, ..Default::default() }];
        let registry = Registry::with_replace(replace).unwrap();
        assert_eq!(registry.block_ids(), vec![2, 9]);
        assert_eq!(registry.biome_ids(), vec![1, 8]);
    }
}
