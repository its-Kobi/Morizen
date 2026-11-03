//! Bounded event snapshots and action batches exchanged with Java.
//!
//! The wire form is deliberately tiny and dependency-free on the Java side:
//! one header and one tab-separated row per event/action, with UTF-8 text hex
//! encoded. A whole tick crosses JNI in one string rather than one call per
//! entity, block or action.

use crate::Handle;
use serde::{Deserialize, Serialize};

pub const EVENT_WIRE_SCHEMA: u32 = 1;
pub const MAX_EVENTS_PER_BATCH: usize = 256;
pub const MAX_ACTIONS_PER_BATCH: usize = 256;
pub const MAX_TEXT_BYTES: usize = 4096;
pub const MAX_WIRE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Start,
    ClientTick,
    ServerTick,
    PlayerJoin,
    PlayerLeave,
    BlockInteract,
    Command,
    WorldSnapshot,
    NetworkPayload,
    Screen,
    Render,
    Sound,
    Recipe,
    Loot,
    Stop,
}

impl EventKind {
    pub fn wire(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::ClientTick => "client_tick",
            Self::ServerTick => "server_tick",
            Self::PlayerJoin => "player_join",
            Self::PlayerLeave => "player_leave",
            Self::BlockInteract => "block_interact",
            Self::Command => "command",
            Self::WorldSnapshot => "world_snapshot",
            Self::NetworkPayload => "network_payload",
            Self::Screen => "screen",
            Self::Render => "render",
            Self::Sound => "sound",
            Self::Recipe => "recipe",
            Self::Loot => "loot",
            Self::Stop => "stop",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "start" => Self::Start,
            "client_tick" => Self::ClientTick,
            "server_tick" => Self::ServerTick,
            "player_join" => Self::PlayerJoin,
            "player_leave" => Self::PlayerLeave,
            "block_interact" => Self::BlockInteract,
            "command" => Self::Command,
            "world_snapshot" => Self::WorldSnapshot,
            "network_payload" => Self::NetworkPayload,
            "screen" => Self::Screen,
            "render" => Self::Render,
            "sound" => Self::Sound,
            "recipe" => Self::Recipe,
            "loot" => Self::Loot,
            "stop" => Self::Stop,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeEvent {
    pub sequence: u64,
    pub kind: EventKind,
    #[serde(default)]
    pub tick: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world: Option<Handle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<Handle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[i32; 3]>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
}

impl BridgeEvent {
    pub fn new(sequence: u64, kind: EventKind) -> Self {
        Self {
            sequence,
            kind,
            tick: 0,
            world: None,
            player: None,
            position: None,
            text: String::new(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.text.len() > MAX_TEXT_BYTES {
            return Err(format!(
                "event {} text is {} bytes; the limit is {MAX_TEXT_BYTES}",
                self.sequence,
                self.text.len()
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventBatch {
    pub events: Vec<BridgeEvent>,
}

impl EventBatch {
    pub fn single(event: BridgeEvent) -> Self {
        Self {
            events: vec![event],
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.events.len() > MAX_EVENTS_PER_BATCH {
            return Err(format!(
                "event batch has {} entries; the limit is {MAX_EVENTS_PER_BATCH}",
                self.events.len()
            ));
        }
        let mut previous = None;
        for event in &self.events {
            event.validate()?;
            if previous.is_some_and(|sequence| event.sequence <= sequence) {
                return Err("event sequences must increase within a batch".into());
            }
            previous = Some(event.sequence);
        }
        Ok(())
    }

    pub fn to_wire(&self) -> Result<String, String> {
        self.validate()?;
        let mut wire = format!("MORIZEN_EVENTS\t{EVENT_WIRE_SCHEMA}\n");
        for event in &self.events {
            let (x, y, z) = event.position.map_or_else(
                || ("_".into(), "_".into(), "_".into()),
                |[x, y, z]| (x.to_string(), y.to_string(), z.to_string()),
            );
            wire.push_str(&format!(
                "E\t{}\t{}\t{}\t{}\t{}\t{x}\t{y}\t{z}\t{}\n",
                event.sequence,
                event.kind.wire(),
                event.tick,
                event.world.map_or(0, Handle::get),
                event.player.map_or(0, Handle::get),
                hex_encode(event.text.as_bytes()),
            ));
        }
        if wire.len() > MAX_WIRE_BYTES {
            return Err(format!(
                "encoded event batch is {} bytes; the limit is {MAX_WIRE_BYTES}",
                wire.len()
            ));
        }
        Ok(wire)
    }

    pub fn from_wire(wire: &str) -> Result<Self, String> {
        if wire.len() > MAX_WIRE_BYTES {
            return Err("event wire exceeds its byte limit".into());
        }
        let mut lines = wire.lines();
        if lines.next() != Some("MORIZEN_EVENTS\t1") {
            return Err("unsupported event wire header".into());
        }
        let mut events = Vec::new();
        for line in lines.filter(|line| !line.is_empty()) {
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() != 10 || fields[0] != "E" {
                return Err("malformed event row".into());
            }
            let position = match (fields[6], fields[7], fields[8]) {
                ("_", "_", "_") => None,
                (x, y, z) => Some([
                    parse(x, "event x")?,
                    parse(y, "event y")?,
                    parse(z, "event z")?,
                ]),
            };
            let world = raw_handle(fields[4], "world handle")?;
            let player = raw_handle(fields[5], "player handle")?;
            events.push(BridgeEvent {
                sequence: parse(fields[1], "event sequence")?,
                kind: EventKind::parse(fields[2])
                    .ok_or_else(|| format!("unknown event kind {:?}", fields[2]))?,
                tick: parse(fields[3], "event tick")?,
                world,
                player,
                position,
                text: hex_text(fields[9])?,
            });
        }
        let batch = Self { events };
        batch.validate()?;
        Ok(batch)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Chat,
    Command,
    Cancel,
    SetBlock,
    PlaySound,
    OpenScreen,
    CustomPayload,
    GiveItem,
    Error,
}

impl ActionKind {
    pub fn wire(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Command => "command",
            Self::Cancel => "cancel",
            Self::SetBlock => "set_block",
            Self::PlaySound => "play_sound",
            Self::OpenScreen => "open_screen",
            Self::CustomPayload => "custom_payload",
            Self::GiveItem => "give_item",
            Self::Error => "error",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "chat" => Self::Chat,
            "command" => Self::Command,
            "cancel" => Self::Cancel,
            "set_block" => Self::SetBlock,
            "play_sound" => Self::PlaySound,
            "open_screen" => Self::OpenScreen,
            "custom_payload" => Self::CustomPayload,
            "give_item" => Self::GiveItem,
            "error" => Self::Error,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeAction {
    pub event_sequence: u64,
    pub kind: ActionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[i32; 3]>,
    #[serde(default)]
    pub value: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
}

impl BridgeAction {
    pub fn text(event_sequence: u64, kind: ActionKind, text: impl Into<String>) -> Self {
        Self {
            event_sequence,
            kind,
            position: None,
            value: 0,
            text: text.into(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.text.len() > MAX_TEXT_BYTES {
            return Err(format!(
                "action for event {} has {} text bytes; the limit is {MAX_TEXT_BYTES}",
                self.event_sequence,
                self.text.len()
            ));
        }
        if self.kind == ActionKind::SetBlock && self.position.is_none() {
            return Err("set_block action has no position".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionBatch {
    pub actions: Vec<BridgeAction>,
}

impl ActionBatch {
    pub fn validate(&self) -> Result<(), String> {
        if self.actions.len() > MAX_ACTIONS_PER_BATCH {
            return Err(format!(
                "action batch has {} entries; the limit is {MAX_ACTIONS_PER_BATCH}",
                self.actions.len()
            ));
        }
        for action in &self.actions {
            action.validate()?;
        }
        Ok(())
    }

    pub fn to_wire(&self) -> Result<String, String> {
        self.validate()?;
        let mut wire = format!("MORIZEN_ACTIONS\t{EVENT_WIRE_SCHEMA}\n");
        for action in &self.actions {
            let (x, y, z) = action.position.map_or_else(
                || ("_".into(), "_".into(), "_".into()),
                |[x, y, z]| (x.to_string(), y.to_string(), z.to_string()),
            );
            wire.push_str(&format!(
                "A\t{}\t{}\t{x}\t{y}\t{z}\t{}\t{}\n",
                action.event_sequence,
                action.kind.wire(),
                action.value,
                hex_encode(action.text.as_bytes()),
            ));
        }
        if wire.len() > MAX_WIRE_BYTES {
            return Err("encoded action batch exceeds its byte limit".into());
        }
        Ok(wire)
    }

    pub fn from_wire(wire: &str) -> Result<Self, String> {
        if wire.len() > MAX_WIRE_BYTES {
            return Err("action wire exceeds its byte limit".into());
        }
        let mut lines = wire.lines();
        if lines.next() != Some("MORIZEN_ACTIONS\t1") {
            return Err("unsupported action wire header".into());
        }
        let mut actions = Vec::new();
        for line in lines.filter(|line| !line.is_empty()) {
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() != 8 || fields[0] != "A" {
                return Err("malformed action row".into());
            }
            let position = match (fields[3], fields[4], fields[5]) {
                ("_", "_", "_") => None,
                (x, y, z) => Some([
                    parse(x, "action x")?,
                    parse(y, "action y")?,
                    parse(z, "action z")?,
                ]),
            };
            actions.push(BridgeAction {
                event_sequence: parse(fields[1], "action event sequence")?,
                kind: ActionKind::parse(fields[2])
                    .ok_or_else(|| format!("unknown action kind {:?}", fields[2]))?,
                position,
                value: parse(fields[6], "action value")?,
                text: hex_text(fields[7])?,
            });
        }
        let batch = Self { actions };
        batch.validate()?;
        Ok(batch)
    }
}

fn parse<T: std::str::FromStr>(value: &str, field: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("{field} is not a number: {value:?}"))
}

fn raw_handle(value: &str, field: &str) -> Result<Option<Handle>, String> {
    let raw: u64 = parse(value, field)?;
    if raw == 0 {
        return Ok(None);
    }
    Handle::from_raw(raw)
        .map(Some)
        .ok_or_else(|| format!("{field} is malformed"))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn hex_text(text: &str) -> Result<String, String> {
    if !text.len().is_multiple_of(2) {
        return Err("hex text has an odd length".into());
    }
    let mut bytes = Vec::with_capacity(text.len() / 2);
    for pair in text.as_bytes().chunks_exact(2) {
        let high = hex_digit(pair[0]).ok_or("hex text has a non-hex digit")?;
        let low = hex_digit(pair[1]).ok_or("hex text has a non-hex digit")?;
        bytes.push((high << 4) | low);
    }
    String::from_utf8(bytes).map_err(|_| "hex text is not UTF-8".into())
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(slot: u32) -> Handle {
        Handle::from_raw((7_u64 << 32) | u64::from(slot + 1)).unwrap()
    }

    #[test]
    fn event_wire_round_trips_unicode_handles_and_positions() {
        let mut event = BridgeEvent::new(9, EventKind::BlockInteract);
        event.tick = 44;
        event.world = Some(handle(1));
        event.player = Some(handle(2));
        event.position = Some([-3, 64, 8]);
        event.text = "morizen:test/é".into();
        let batch = EventBatch::single(event);
        assert_eq!(
            EventBatch::from_wire(&batch.to_wire().unwrap()).unwrap(),
            batch
        );
    }

    #[test]
    fn action_wire_round_trips_every_field() {
        let batch = ActionBatch {
            actions: vec![BridgeAction {
                event_sequence: 9,
                kind: ActionKind::SetBlock,
                position: Some([1, -2, 3]),
                value: 42,
                text: "morizen_registry:ruby_block".into(),
            }],
        };
        assert_eq!(
            ActionBatch::from_wire(&batch.to_wire().unwrap()).unwrap(),
            batch
        );
    }

    #[test]
    fn batches_enforce_order_counts_and_text_bounds() {
        let mut unordered = EventBatch {
            events: vec![
                BridgeEvent::new(2, EventKind::ClientTick),
                BridgeEvent::new(1, EventKind::ClientTick),
            ],
        };
        assert!(unordered.validate().unwrap_err().contains("increase"));
        unordered.events[1].sequence = 3;
        assert!(unordered.validate().is_ok());
        unordered.events[0].text = "x".repeat(MAX_TEXT_BYTES + 1);
        assert!(unordered.validate().unwrap_err().contains("limit"));

        let event = BridgeEvent::new(1, EventKind::ClientTick);
        let too_many = EventBatch {
            events: vec![event; MAX_EVENTS_PER_BATCH + 1],
        };
        assert!(too_many.validate().is_err());
    }

    #[test]
    fn malformed_wire_is_refused() {
        assert!(EventBatch::from_wire("wrong\n").is_err());
        assert!(ActionBatch::from_wire("MORIZEN_ACTIONS\t1\nA\t0\tchat\t_\t_\t_\t0\tz\n").is_err());
    }
}
