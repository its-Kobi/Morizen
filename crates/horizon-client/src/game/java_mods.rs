//! Narrow engine seams for the opt-in embedded Java bridge.
//!
//! This whole module and every call site disappear without `java-mods`.
//! Snapshots are copied before JNI and returned actions are applied only after
//! callbacks complete, so Java never receives `&mut Game` or a world pointer.

use super::Game;
use horizon_core::position::BlockPos;
use horizon_world::level::flags;
use morizen_compat::{ActionBatch, ActionKind, BridgeEvent, EventKind, Handle, HandleTable};

pub(super) struct JavaHandles {
    worlds: HandleTable<()>,
    players: HandleTable<()>,
    world: Option<Handle>,
    player: Option<Handle>,
}

impl JavaHandles {
    pub(super) fn new() -> Self {
        Self {
            worlds: HandleTable::with_limit(1),
            players: HandleTable::with_limit(1),
            world: None,
            player: None,
        }
    }

    pub(super) fn clear(&mut self) {
        self.worlds.clear();
        self.players.clear();
        self.world = None;
        self.player = None;
    }

    fn sync(&mut self, in_world: bool) {
        if in_world {
            if self.world.is_none() {
                self.world = self.worlds.insert(()).ok();
            }
            if self.player.is_none() {
                self.player = self.players.insert(()).ok();
            }
        } else {
            self.clear();
        }
    }
}

impl Game<'_> {
    pub(super) fn java_tick(&mut self) {
        let kind = if self.native_window() {
            EventKind::ClientTick
        } else {
            EventKind::ServerTick
        };
        let Some(mut event) = self.java_event(kind) else {
            return;
        };
        event.tick = self.ticks;
        if let Some(actions) = self.java_dispatch(event) {
            self.apply_java_actions(actions);
        }
    }

    pub(super) fn java_command(&mut self, command: &str) -> bool {
        let Some(mut event) = self.java_event(EventKind::Command) else {
            return false;
        };
        event.tick = self.ticks;
        event.text = command.to_owned();
        let Some(actions) = self.java_dispatch(event) else {
            return false;
        };
        self.apply_java_actions(actions)
    }

    pub(super) fn java_block_interact(&mut self, position: [i32; 3]) -> bool {
        let Some(mut event) = self.java_event(EventKind::BlockInteract) else {
            return false;
        };
        event.tick = self.ticks;
        event.position = Some(position);
        let Some(actions) = self.java_dispatch(event) else {
            return false;
        };
        self.apply_java_actions(actions)
    }

    pub(super) fn java_player_join(&mut self, name: &str) {
        self.java_player_event(EventKind::PlayerJoin, name);
    }

    pub(super) fn java_player_leave(&mut self, name: &str) {
        self.java_player_event(EventKind::PlayerLeave, name);
    }

    fn java_player_event(&mut self, kind: EventKind, name: &str) {
        let Some(mut event) = self.java_event(kind) else {
            return;
        };
        event.tick = self.ticks;
        event.text = name.to_owned();
        if let Some(actions) = self.java_dispatch(event) {
            self.apply_java_actions(actions);
        }
    }

    fn java_event(&mut self, kind: EventKind) -> Option<BridgeEvent> {
        self.java_handles.sync(self.player.is_some());
        let mut event = horizon_client::java_mods::dispatcher()?.event(kind);
        event.world = self.java_handles.world;
        event.player = self.java_handles.player;
        Some(event)
    }

    fn java_dispatch(&self, event: BridgeEvent) -> Option<ActionBatch> {
        match horizon_client::java_mods::dispatcher()?.dispatch_event(event) {
            Ok(actions) => Some(actions),
            Err(error) => {
                log::error!("Java mod callback disabled for this event: {error}");
                None
            }
        }
    }

    /// Returns whether an action claimed the source interaction/command.
    fn apply_java_actions(&mut self, actions: ActionBatch) -> bool {
        let mut claimed = false;
        for action in actions.actions {
            match action.kind {
                ActionKind::Chat => self.native_chat(&action.text),
                ActionKind::Command => {
                    self.run_command(&action.text);
                }
                ActionKind::Cancel => claimed = true,
                ActionKind::SetBlock => {
                    let Some(position) = action.position else {
                        continue;
                    };
                    let Some(state) = self.pack.assets.registry.default_state(&action.text) else {
                        log::warn!("Java mod requested unknown block {}", action.text);
                        continue;
                    };
                    let at = BlockPos::new(position[0], position[1], position[2]);
                    self.with_level(|blocks, level, ticks| {
                        blocks.set_block(level, ticks, at, state, flags::UPDATE_ALL)
                    });
                    self.world
                        .modified
                        .insert((position[0] >> 4, position[2] >> 4));
                    claimed = true;
                }
                ActionKind::GiveItem => {
                    if let Err(error) = self.native_give(&action.text, action.value.max(0) as u32) {
                        log::warn!("Java mod give-item action was refused: {error}");
                    } else {
                        claimed = true;
                    }
                }
                ActionKind::Error => log::error!("Java mod callback failed: {}", action.text),
                ActionKind::PlaySound | ActionKind::OpenScreen | ActionKind::CustomPayload => {
                    log::warn!(
                        "Java mod action {:?} is not connected to this engine path",
                        action.kind
                    )
                }
            }
        }
        claimed
    }
}
