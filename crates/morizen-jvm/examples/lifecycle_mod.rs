//! Phases 3-5 callback-batch proof. A real Fabric entrypoint registers Java
//! callbacks; Rust sends ordered lifecycle/world/network/UI events and checks
//! the bounded actions returned on the dedicated JVM owner thread.

use morizen_compat::{
    ActionKind, BlockStateSnapshot, EntitySnapshot, EventBatch, EventKind, Handle, HandleTable,
    ItemStackSnapshot, LocatedBlockSnapshot, NbtValue, SNAPSHOT_SCHEMA, WorldSnapshot,
};
use morizen_jvm::{JavaModHost, LaunchConfig};
use std::collections::BTreeMap;
use std::process::ExitCode;

fn handle(slot: u32) -> Handle {
    Handle::from_raw((1_u64 << 32) | u64::from(slot + 1)).expect("valid test handle")
}

fn run() -> morizen_jvm::Result<()> {
    let path = std::env::args_os().nth(1).ok_or_else(|| {
        morizen_jvm::BridgeError::Config("usage: lifecycle_mod <run-config.json>".into())
    })?;
    let config = LaunchConfig::from_path(path)?;
    let host = JavaModHost::start(config)?;
    let dispatcher = host.dispatcher();

    let mut client_tick = dispatcher.event(EventKind::ClientTick);
    client_tick.tick = 1;
    dispatcher.dispatch_event(client_tick)?;

    let mut server_tick = dispatcher.event(EventKind::ServerTick);
    server_tick.tick = 1;
    let mut join = dispatcher.event(EventKind::PlayerJoin);
    join.world = Some(handle(0));
    join.player = Some(handle(1));
    join.text = "TestPlayer".into();
    let isolated = dispatcher.dispatch(EventBatch {
        events: vec![server_tick, join],
    })?;
    if isolated.actions.len() != 1 || isolated.actions[0].kind != ActionKind::Error {
        return Err(morizen_jvm::BridgeError::Event(format!(
            "expected one isolated callback error, got {:?}",
            isolated.actions
        )));
    }

    let mut interact = dispatcher.event(EventKind::BlockInteract);
    interact.tick = 2;
    interact.world = Some(handle(0));
    interact.player = Some(handle(1));
    interact.position = Some([3, 64, -2]);
    let actions = dispatcher.dispatch_event(interact)?;
    if actions.actions.len() != 1
        || actions.actions[0].kind != ActionKind::SetBlock
        || actions.actions[0].position != Some([3, 65, -2])
    {
        return Err(morizen_jvm::BridgeError::Event(format!(
            "unexpected interaction actions: {:?}",
            actions.actions
        )));
    }

    let mut command = dispatcher.event(EventKind::Command);
    command.text = "bridge ping".into();
    let actions = dispatcher.dispatch_event(command)?;
    let kinds: Vec<ActionKind> = actions.actions.iter().map(|action| action.kind).collect();
    if kinds != [ActionKind::Chat, ActionKind::Cancel] || actions.actions[0].text != "bridge pong" {
        return Err(morizen_jvm::BridgeError::Event(format!(
            "unexpected command actions: {:?}",
            actions.actions
        )));
    }

    let mut leave = dispatcher.event(EventKind::PlayerLeave);
    leave.player = Some(handle(1));
    leave.text = "TestPlayer".into();
    dispatcher.dispatch_event(leave)?;

    let mut worlds = HandleTable::with_limit(1);
    let mut players = HandleTable::with_limit(1);
    let mut stacks = HandleTable::with_limit(1);
    let world_handle = worlds.insert("minecraft:overworld").unwrap();
    let player_handle = players.insert("TestPlayer").unwrap();
    let stack_handle = stacks.insert("morizen_registry:ruby").unwrap();
    let snapshot = WorldSnapshot {
        schema: SNAPSHOT_SCHEMA,
        world: world_handle,
        dimension: "minecraft:overworld".into(),
        game_time: 42,
        blocks: vec![LocatedBlockSnapshot {
            position: [3, 65, -2],
            state: BlockStateSnapshot {
                id: "morizen_registry:ruby_lamp".into(),
                properties: BTreeMap::new(),
            },
        }],
        entities: vec![EntitySnapshot {
            handle: player_handle,
            entity_id: 1,
            type_id: "minecraft:player".into(),
            position: [3.5, 65.0, -1.5],
            velocity: [0.0; 3],
            health: 20.0,
            equipment: vec![ItemStackSnapshot {
                handle: Some(stack_handle),
                item: "morizen_registry:ruby".into(),
                count: 2,
                components: BTreeMap::from([(
                    "minecraft:custom_data".into(),
                    NbtValue::Compound(BTreeMap::from([(
                        "origin".into(),
                        NbtValue::String("bridge".into()),
                    )])),
                )]),
            }],
            data: BTreeMap::new(),
        }],
    };
    let mut world_event = dispatcher.event(EventKind::WorldSnapshot);
    world_event.world = Some(world_handle);
    world_event.player = Some(player_handle);
    world_event.text = snapshot
        .to_json()
        .map_err(morizen_jvm::BridgeError::Event)?;
    let actions = dispatcher.dispatch_event(world_event)?;
    if actions.actions.len() != 1
        || actions.actions[0].kind != ActionKind::GiveItem
        || actions.actions[0].value != 2
    {
        return Err(morizen_jvm::BridgeError::Event(format!(
            "unexpected world snapshot actions: {:?}",
            actions.actions
        )));
    }
    players.remove(player_handle);
    stacks.clear();
    if players.get(player_handle).is_some() || stacks.get(stack_handle).is_some() {
        return Err(morizen_jvm::BridgeError::Event(
            "a stale world snapshot handle still resolved".into(),
        ));
    }

    let mut phase_five = Vec::new();
    for (kind, text) in [
        (EventKind::NetworkPayload, "morizen:test/010203"),
        (EventKind::Screen, "morizen:test"),
        (EventKind::Sound, "minecraft:block.note_block.pling"),
        (EventKind::Recipe, "morizen_registry:ruby"),
        (EventKind::Loot, "morizen_registry:ruby"),
    ] {
        let mut event = dispatcher.event(kind);
        event.text = text.into();
        phase_five.push(event);
    }
    let actions = dispatcher.dispatch(EventBatch { events: phase_five })?;
    let kinds: Vec<ActionKind> = actions.actions.iter().map(|action| action.kind).collect();
    if kinds
        != [
            ActionKind::CustomPayload,
            ActionKind::OpenScreen,
            ActionKind::PlaySound,
            ActionKind::GiveItem,
            ActionKind::GiveItem,
        ]
    {
        return Err(morizen_jvm::BridgeError::Event(format!(
            "unexpected Phase 5 actions: {:?}",
            actions.actions
        )));
    }

    let render_events = (0..morizen_compat::MAX_EVENTS_PER_BATCH)
        .map(|_| dispatcher.event(EventKind::Render))
        .collect();
    let render_actions = dispatcher.dispatch(EventBatch {
        events: render_events,
    })?;
    if !render_actions.actions.is_empty() {
        return Err(morizen_jvm::BridgeError::Event(
            "render call-budget batch unexpectedly returned actions".into(),
        ));
    }
    println!("morizen-java: Phase 4 world snapshot and stale handles passed");
    println!("morizen-java: Phase 5 network/UI/sound/data call budget passed");

    host.shutdown()?;
    let after_stop = dispatcher.event(EventKind::ClientTick);
    if dispatcher.dispatch_event(after_stop).is_ok() {
        return Err(morizen_jvm::BridgeError::Event(
            "a callback ran after stop".into(),
        ));
    }
    println!("morizen-java: Phase 3 lifecycle/event batch passed");
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("morizen-java: {error}");
            ExitCode::FAILURE
        }
    }
}
