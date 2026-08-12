use squaremap_protocol::wire::{BiomeDescriptor, BlockStateDescriptor, RegistryReplace};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

#[derive(Debug)]
pub struct RegistryGeneration {
    revision: u64,
    world: Option<squaremap_protocol::wire::WorldIdentity>,
    blocks: Arc<HashMap<u32, BlockStateDescriptor>>,
    biomes: Arc<HashMap<u32, BiomeDescriptor>>,
}
pub type GenerationToken = Arc<RegistryGeneration>;
impl RegistryGeneration {
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn world(&self) -> Option<&squaremap_protocol::wire::WorldIdentity> {
        self.world.as_ref()
    }
    pub fn block(&self, id: u32) -> Option<&BlockStateDescriptor> {
        self.blocks.get(&id)
    }
    pub fn biome(&self, id: u32) -> Option<&BiomeDescriptor> {
        self.biomes.get(&id)
    }
    pub fn block_ids(&self) -> Vec<u32> {
        let mut ids: Vec<_> = self.blocks.keys().copied().collect();
        ids.sort_unstable();
        ids
    }
    pub fn biome_ids(&self) -> Vec<u32> {
        let mut ids: Vec<_> = self.biomes.keys().copied().collect();
        ids.sort_unstable();
        ids
    }
}

#[derive(Clone, Debug)]
pub struct Registry {
    revision: u64,
    world: Option<squaremap_protocol::wire::WorldIdentity>,
    blocks: HashMap<u32, BlockStateDescriptor>,
    biomes: HashMap<u32, BiomeDescriptor>,
    generation: GenerationToken,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            revision: 0,
            world: None,
            blocks: HashMap::new(),
            biomes: HashMap::new(),
            generation: Arc::new(RegistryGeneration {
                revision: 0,
                world: None,
                blocks: Arc::new(HashMap::new()),
                biomes: Arc::new(HashMap::new()),
            }),
        }
    }
}
pub const MAX_BLOCK_DESCRIPTORS: usize = 65_536;
pub const MAX_BIOME_DESCRIPTORS: usize = 16_384;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryError {
    OlderRevision {
        current: u64,
        incoming: u64,
    },
    DuplicateBlock(u32),
    DuplicateBiome(u32),
    ZeroId,
    DescriptorCount {
        kind: &'static str,
        count: usize,
        max: usize,
    },
    InvalidTransparency {
        id: u32,
        value: i32,
    },
    InvalidFluid {
        id: u32,
        value: i32,
    },
    InvalidAlpha {
        id: u32,
        value: u32,
    },
    InvalidTint {
        id: u32,
        value: u32,
    },
}
impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for RegistryError {}

impl Registry {
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn world(&self) -> Option<&squaremap_protocol::wire::WorldIdentity> {
        self.world.as_ref()
    }
    pub fn generation(&self) -> GenerationToken {
        Arc::clone(&self.generation)
    }
    pub fn replace(&mut self, value: RegistryReplace) -> Result<(), RegistryError> {
        if value.revision < self.revision {
            return Err(RegistryError::OlderRevision {
                current: self.revision,
                incoming: value.revision,
            });
        }
        if value.block_states.len() > MAX_BLOCK_DESCRIPTORS {
            return Err(RegistryError::DescriptorCount {
                kind: "block",
                count: value.block_states.len(),
                max: MAX_BLOCK_DESCRIPTORS,
            });
        }
        if value.biomes.len() > MAX_BIOME_DESCRIPTORS {
            return Err(RegistryError::DescriptorCount {
                kind: "biome",
                count: value.biomes.len(),
                max: MAX_BIOME_DESCRIPTORS,
            });
        }
        let mut blocks = HashMap::with_capacity(value.block_states.len());
        for descriptor in value.block_states {
            let id = descriptor.id;
            if id == 0 {
                return Err(RegistryError::ZeroId);
            }
            if !(1..=3).contains(&descriptor.transparency) {
                return Err(RegistryError::InvalidTransparency {
                    id,
                    value: descriptor.transparency,
                });
            }
            if !(1..=4).contains(&descriptor.fluid) {
                return Err(RegistryError::InvalidFluid {
                    id,
                    value: descriptor.fluid,
                });
            }
            if descriptor.tint_index > 3 {
                return Err(RegistryError::InvalidTint {
                    id,
                    value: descriptor.tint_index,
                });
            }
            if (descriptor.glass && !matches!(descriptor.glass_alpha_percent, 25 | 50))
                || (!descriptor.glass && descriptor.glass_alpha_percent != 0)
            {
                return Err(RegistryError::InvalidAlpha {
                    id,
                    value: descriptor.glass_alpha_percent,
                });
            }
            if blocks.insert(id, descriptor).is_some() {
                return Err(RegistryError::DuplicateBlock(id));
            }
        }
        let mut biomes = HashMap::with_capacity(value.biomes.len());
        for descriptor in value.biomes {
            let id = descriptor.id;
            if id == 0 {
                return Err(RegistryError::ZeroId);
            }
            if descriptor.tint_index > 3 {
                return Err(RegistryError::InvalidTint {
                    id,
                    value: descriptor.tint_index,
                });
            }
            if biomes.insert(id, descriptor).is_some() {
                return Err(RegistryError::DuplicateBiome(id));
            }
        }
        self.revision = value.revision;
        self.world = value.world;
        self.blocks = blocks;
        self.biomes = biomes;
        self.generation = Arc::new(RegistryGeneration {
            revision: self.revision,
            world: self.world.clone(),
            blocks: Arc::new(self.blocks.clone()),
            biomes: Arc::new(self.biomes.clone()),
        });
        Ok(())
    }
    pub fn with_replace(value: RegistryReplace) -> Result<Self, RegistryError> {
        let mut r = Self::default();
        r.replace(value)?;
        Ok(r)
    }
    pub fn block(&self, id: u32) -> Option<&BlockStateDescriptor> {
        self.blocks.get(&id)
    }
    pub fn biome(&self, id: u32) -> Option<&BiomeDescriptor> {
        self.biomes.get(&id)
    }
    pub fn block_ids(&self) -> Vec<u32> {
        let mut ids: Vec<_> = self.blocks.keys().copied().collect();
        ids.sort_unstable();
        ids
    }
    pub fn biome_ids(&self) -> Vec<u32> {
        let mut ids: Vec<_> = self.biomes.keys().copied().collect();
        ids.sort_unstable();
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_are_returned_in_deterministic_order() {
        let mut replace = RegistryReplace {
            revision: 1,
            ..Default::default()
        };
        replace.block_states = vec![
            BlockStateDescriptor {
                id: 9,
                transparency: 1,
                fluid: 1,
                ..Default::default()
            },
            BlockStateDescriptor {
                id: 2,
                transparency: 1,
                fluid: 1,
                ..Default::default()
            },
        ];
        replace.biomes = vec![
            BiomeDescriptor {
                id: 8,
                ..Default::default()
            },
            BiomeDescriptor {
                id: 1,
                ..Default::default()
            },
        ];
        let registry = Registry::with_replace(replace).unwrap();
        assert_eq!(registry.block_ids(), vec![2, 9]);
        assert_eq!(registry.biome_ids(), vec![1, 8]);
    }
    #[test]
    fn lower_revision_does_not_replace_generation() {
        let mut current = RegistryReplace {
            revision: 43,
            ..Default::default()
        };
        current.block_states.push(BlockStateDescriptor {
            id: 1,
            transparency: 1,
            fluid: 1,
            ..Default::default()
        });
        let mut registry = Registry::with_replace(current).unwrap();
        let generation = registry.generation();
        let older = RegistryReplace {
            revision: 42,
            ..Default::default()
        };
        assert!(matches!(
            registry.replace(older),
            Err(RegistryError::OlderRevision {
                current: 43,
                incoming: 42
            })
        ));
        assert!(std::sync::Arc::ptr_eq(&generation, &registry.generation()));
    }
    #[test]
    fn invalid_descriptor_fields_are_rejected_at_registry_boundary() {
        for descriptor in [
            BlockStateDescriptor {
                id: 1,
                transparency: 0,
                fluid: 1,
                ..Default::default()
            },
            BlockStateDescriptor {
                id: 1,
                transparency: 1,
                fluid: 0,
                ..Default::default()
            },
            BlockStateDescriptor {
                id: 1,
                transparency: 1,
                fluid: 1,
                tint_index: 4,
                ..Default::default()
            },
            BlockStateDescriptor {
                id: 1,
                transparency: 1,
                fluid: 1,
                glass: true,
                glass_alpha_percent: 30,
                ..Default::default()
            },
        ] {
            assert!(
                Registry::with_replace(RegistryReplace {
                    revision: 1,
                    block_states: vec![descriptor],
                    ..Default::default()
                })
                .is_err()
            );
        }
        assert!(
            Registry::with_replace(RegistryReplace {
                revision: 1,
                biomes: vec![BiomeDescriptor {
                    id: 1,
                    tint_index: 4,
                    ..Default::default()
                }],
                ..Default::default()
            })
            .is_err()
        );
    }
}
