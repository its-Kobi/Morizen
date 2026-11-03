//! Immutable world/entity/item/NBT snapshots copied across the Java boundary.

use crate::Handle;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SNAPSHOT_SCHEMA: u32 = 1;
pub const MAX_SNAPSHOT_BLOCKS: usize = 4096;
pub const MAX_SNAPSHOT_ENTITIES: usize = 1024;
pub const MAX_COMPONENTS_PER_STACK: usize = 256;
pub const MAX_NBT_DEPTH: usize = 32;
pub const MAX_NBT_ENTRIES: usize = 16_384;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum NbtValue {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    String(String),
    ByteArray(Vec<i8>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
    List(Vec<NbtValue>),
    Compound(BTreeMap<String, NbtValue>),
}

impl NbtValue {
    fn validate(&self, depth: usize, entries: &mut usize) -> Result<(), String> {
        if depth > MAX_NBT_DEPTH {
            return Err(format!("NBT exceeds {MAX_NBT_DEPTH} levels"));
        }
        *entries += 1;
        if *entries > MAX_NBT_ENTRIES {
            return Err(format!("NBT exceeds {MAX_NBT_ENTRIES} entries"));
        }
        match self {
            Self::String(value) if value.len() > 65_535 => {
                return Err("NBT string exceeds 65535 UTF-8 bytes".into());
            }
            Self::Float(value) if !value.is_finite() => {
                return Err("NBT float is not finite".into());
            }
            Self::Double(value) if !value.is_finite() => {
                return Err("NBT double is not finite".into());
            }
            Self::List(values) => {
                for value in values {
                    value.validate(depth + 1, entries)?;
                }
            }
            Self::Compound(values) => {
                for (name, value) in values {
                    if name.len() > 1024 {
                        return Err("NBT key exceeds 1024 UTF-8 bytes".into());
                    }
                    value.validate(depth + 1, entries)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlockStateSnapshot {
    pub id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocatedBlockSnapshot {
    pub position: [i32; 3],
    pub state: BlockStateSnapshot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemStackSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<Handle>,
    pub item: String,
    pub count: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub components: BTreeMap<String, NbtValue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntitySnapshot {
    pub handle: Handle,
    pub entity_id: i32,
    pub type_id: String,
    pub position: [f64; 3],
    pub velocity: [f64; 3],
    pub health: f32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub equipment: Vec<ItemStackSnapshot>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub data: BTreeMap<String, NbtValue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldSnapshot {
    pub schema: u32,
    pub world: Handle,
    pub dimension: String,
    pub game_time: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<LocatedBlockSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<EntitySnapshot>,
}

impl WorldSnapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != SNAPSHOT_SCHEMA {
            return Err(format!("unsupported world snapshot schema {}", self.schema));
        }
        check_id(&self.dimension, "dimension")?;
        if self.blocks.len() > MAX_SNAPSHOT_BLOCKS {
            return Err(format!(
                "snapshot has more than {MAX_SNAPSHOT_BLOCKS} blocks"
            ));
        }
        if self.entities.len() > MAX_SNAPSHOT_ENTITIES {
            return Err(format!(
                "snapshot has more than {MAX_SNAPSHOT_ENTITIES} entities"
            ));
        }
        for block in &self.blocks {
            check_id(&block.state.id, "block")?;
            if block.state.properties.len() > 64 {
                return Err("block state has more than 64 properties".into());
            }
        }
        for entity in &self.entities {
            check_id(&entity.type_id, "entity type")?;
            if !entity
                .position
                .iter()
                .chain(&entity.velocity)
                .all(|v| v.is_finite())
                || !entity.health.is_finite()
            {
                return Err(format!(
                    "entity {} has a non-finite number",
                    entity.entity_id
                ));
            }
            for stack in &entity.equipment {
                validate_stack(stack)?;
            }
            validate_nbt_map(&entity.data)?;
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String, String> {
        self.validate()?;
        serde_json::to_string(self).map_err(|error| error.to_string())
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let snapshot: Self = serde_json::from_str(text)
            .map_err(|error| format!("world snapshot is not valid JSON: {error}"))?;
        snapshot.validate()?;
        Ok(snapshot)
    }
}

fn validate_stack(stack: &ItemStackSnapshot) -> Result<(), String> {
    check_id(&stack.item, "item")?;
    if stack.count > 99 {
        return Err(format!(
            "{} stack count {} exceeds 99",
            stack.item, stack.count
        ));
    }
    if stack.components.len() > MAX_COMPONENTS_PER_STACK {
        return Err(format!(
            "{} has more than {MAX_COMPONENTS_PER_STACK} components",
            stack.item
        ));
    }
    validate_nbt_map(&stack.components)
}

fn validate_nbt_map(values: &BTreeMap<String, NbtValue>) -> Result<(), String> {
    let mut entries = 0;
    for (name, value) in values {
        check_id(name, "component")?;
        value.validate(0, &mut entries)?;
    }
    Ok(())
}

fn check_id(id: &str, kind: &str) -> Result<(), String> {
    let Some((namespace, path)) = id.split_once(':') else {
        return Err(format!("{kind} id {id:?} is not namespace:path"));
    };
    let valid = |part: &str, path: bool| {
        !part.is_empty()
            && part.len() <= 256
            && part.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'_' | b'-' | b'.')
                    || (path && byte == b'/')
            })
    };
    if !valid(namespace, false) || !valid(path, true) {
        return Err(format!("{kind} id {id:?} is invalid"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(slot: u32) -> Handle {
        Handle::from_raw((2_u64 << 32) | u64::from(slot + 1)).unwrap()
    }

    fn snapshot() -> WorldSnapshot {
        WorldSnapshot {
            schema: SNAPSHOT_SCHEMA,
            world: handle(0),
            dimension: "minecraft:overworld".into(),
            game_time: 42,
            blocks: vec![LocatedBlockSnapshot {
                position: [1, 64, -3],
                state: BlockStateSnapshot {
                    id: "morizen_registry:ruby_lamp".into(),
                    properties: BTreeMap::from([("lit".into(), "true".into())]),
                },
            }],
            entities: vec![EntitySnapshot {
                handle: handle(1),
                entity_id: 7,
                type_id: "minecraft:player".into(),
                position: [1.5, 65.0, -2.5],
                velocity: [0.0; 3],
                health: 20.0,
                equipment: vec![ItemStackSnapshot {
                    handle: Some(handle(2)),
                    item: "morizen_registry:ruby".into(),
                    count: 3,
                    components: BTreeMap::from([(
                        "minecraft:custom_data".into(),
                        NbtValue::Compound(BTreeMap::from([(
                            "label".into(),
                            NbtValue::String("ruby".into()),
                        )])),
                    )]),
                }],
                data: BTreeMap::new(),
            }],
        }
    }

    #[test]
    fn canonical_snapshot_round_trips_without_losing_components() {
        let expected = snapshot();
        let json = expected.to_json().unwrap();
        assert_eq!(WorldSnapshot::from_json(&json).unwrap(), expected);
        assert_eq!(json, expected.to_json().unwrap());
        assert!(json.contains("minecraft:custom_data"));
        assert!(json.contains("morizen_registry:ruby_lamp"));
    }

    #[test]
    fn snapshots_enforce_counts_ids_depth_and_finite_numbers() {
        let mut value = snapshot();
        value.entities[0].health = f32::NAN;
        assert!(value.validate().unwrap_err().contains("non-finite"));
        let mut value = snapshot();
        value.blocks[0].state.id = "bad id".into();
        assert!(value.validate().is_err());
        let mut nested = NbtValue::Int(1);
        for _ in 0..=MAX_NBT_DEPTH {
            nested = NbtValue::List(vec![nested]);
        }
        let mut value = snapshot();
        value.entities[0].data.insert("test:nested".into(), nested);
        assert!(value.validate().unwrap_err().contains("levels"));
    }
}
