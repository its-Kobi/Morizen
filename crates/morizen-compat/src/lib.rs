//! Engine-neutral contracts for Morizen's Java mod bridge.
//!
//! This crate deliberately does not depend on `horizon-client`. Java-facing
//! requests are converted to these data types before the engine applies them.

mod events;
mod handles;
mod registry;
mod report;
mod snapshots;

pub use events::{
    ActionBatch, ActionKind, BridgeAction, BridgeEvent, EVENT_WIRE_SCHEMA, EventBatch, EventKind,
    MAX_ACTIONS_PER_BATCH, MAX_EVENTS_PER_BATCH, MAX_TEXT_BYTES, MAX_WIRE_BYTES,
};
pub use handles::{Handle, HandleTable};
pub use registry::{
    BlockDeclaration, ContentPackage, ItemDeclaration, REGISTRY_SCHEMA, RegistrationSet,
};
pub use report::{
    CompatReport, LoaderInfo, ModCompatStatus, ModReport, UnsupportedCall, UnsupportedCollector,
    UnsupportedKey,
};
pub use snapshots::{
    BlockStateSnapshot, EntitySnapshot, ItemStackSnapshot, LocatedBlockSnapshot,
    MAX_COMPONENTS_PER_STACK, MAX_NBT_DEPTH, MAX_NBT_ENTRIES, MAX_SNAPSHOT_BLOCKS,
    MAX_SNAPSHOT_ENTITIES, NbtValue, SNAPSHOT_SCHEMA, WorldSnapshot,
};
