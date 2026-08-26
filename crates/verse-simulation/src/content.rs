// SPDX-License-Identifier: AGPL-3.0-or-later

use std::sync::OnceLock;

use serde::Deserialize;
use verse_protocol::{BlockKind, VoxelMaterial};

const P0_CONTENT: &str = include_str!("../../../content/definitions/p0-content.json");

#[derive(Debug, Clone, Deserialize)]
pub struct ContentManifest {
    pub schema_version: u32,
    pub manifest_version: String,
    pub license: String,
    pub voxel_materials: Vec<VoxelDefinition>,
    pub blocks: Vec<BlockDefinition>,
    pub recipes: Recipes,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VoxelDefinition {
    pub material: VoxelMaterial,
    pub ore_yield: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlockDefinition {
    pub kind: BlockKind,
    pub max_health: u16,
    pub power_production: f64,
    pub power_requirement: f64,
    pub stored_power: f64,
    pub component_cost: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Recipes {
    pub refining: RefiningRecipe,
    pub component_crafting: ComponentRecipe,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RefiningRecipe {
    pub ore_input: u64,
    pub refined_output: u64,
    pub defined_loss: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ComponentRecipe {
    pub refined_input: u64,
    pub component_output: u64,
}

pub fn manifest() -> &'static ContentManifest {
    static MANIFEST: OnceLock<ContentManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        let parsed: ContentManifest =
            serde_json::from_str(P0_CONTENT).expect("embedded P0 content must be valid JSON");
        assert_eq!(parsed.schema_version, 1, "unsupported P0 content schema");
        assert_eq!(
            parsed.license, "AGPL-3.0-or-later",
            "content definition license must be explicit"
        );
        assert_eq!(parsed.blocks.len(), 8, "every P0 block must be defined");
        assert_eq!(
            parsed.voxel_materials.len(),
            2,
            "every P0 voxel material must be defined"
        );
        assert_eq!(
            parsed.recipes.refining.ore_input,
            parsed.recipes.refining.refined_output + parsed.recipes.refining.defined_loss,
            "refining recipe must conserve its registered material units"
        );
        assert_eq!(
            parsed.recipes.component_crafting.component_output, 1,
            "P0 crafting requests are expressed as individual component quantities"
        );
        parsed
    })
}

pub fn block(kind: BlockKind) -> &'static BlockDefinition {
    manifest()
        .blocks
        .iter()
        .find(|definition| definition.kind == kind)
        .expect("all BlockKind values must exist in the embedded content manifest")
}

pub fn voxel(material: VoxelMaterial) -> &'static VoxelDefinition {
    manifest()
        .voxel_materials
        .iter()
        .find(|definition| definition.material == material)
        .expect("all VoxelMaterial values must exist in the embedded content manifest")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn manifest_definitions_are_unique_and_conserved() {
        let content = manifest();
        let block_kinds = content
            .blocks
            .iter()
            .map(|definition| format!("{:?}", definition.kind))
            .collect::<BTreeSet<_>>();
        assert_eq!(block_kinds.len(), content.blocks.len());
        assert!(
            content
                .blocks
                .iter()
                .all(|definition| { definition.max_health > 0 && definition.component_cost > 0 })
        );
        assert_eq!(
            content.recipes.component_crafting.refined_input,
            content.recipes.component_crafting.component_output
        );
    }
}
