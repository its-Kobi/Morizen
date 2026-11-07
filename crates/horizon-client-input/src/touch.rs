//! Customisable touch controls shared by the future Android shell and the game.
//!
//! Positions are normalised to the usable screen, and sizes are fractions of its shorter side,
//! so one layout survives phones, tablets and rotation. The runtime owns fingers, not game state:
//! it emits press/release and look events that the game maps through its ordinary key bindings.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

pub const SCHEMA: u32 = 1;
pub const MAX_BUTTONS: usize = 32;

/// What one touch button does. Bindings are Minecraft actions such as `key.jump`; keys are
/// physical names such as `key.keyboard.g`, for the custom-key buttons players asked for.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Action {
    Binding(String),
    Key(String),
    Mouse(String),
}

impl Action {
    pub fn valid(&self) -> bool {
        match self {
            Self::Binding(name) => name.starts_with("key.") && !name.contains(char::is_whitespace),
            Self::Key(name) => crate::keybinds::key_code(name).is_some(),
            Self::Mouse(name) => matches!(
                name.as_str(),
                "key.mouse.left" | "key.mouse.right" | "key.mouse.middle"
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Button {
    pub id: String,
    pub label: String,
    /// Centre, as fractions of the screen width and height.
    pub x: f32,
    pub y: f32,
    /// Side length as a fraction of the screen's shorter side.
    pub size: f32,
    pub action: Action,
}

impl Button {
    pub fn bounds(&self, screen: [f32; 2], scale: f32) -> [f32; 4] {
        let side = self.size * scale * screen[0].min(screen[1]);
        let (cx, cy) = (self.x * screen[0], self.y * screen[1]);
        [cx - side / 2.0, cy - side / 2.0, side, side]
    }

    pub fn contains(&self, point: [f32; 2], screen: [f32; 2], scale: f32) -> bool {
        let [x, y, w, h] = self.bounds(screen, scale);
        point[0] >= x && point[0] <= x + w && point[1] >= y && point[1] <= y + h
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    #[serde(default = "schema")]
    pub schema: u32,
    #[serde(default = "one")]
    pub scale: f32,
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    #[serde(default = "default_sensitivity")]
    pub look_sensitivity: f32,
    #[serde(default)]
    pub buttons: Vec<Button>,
}

const fn schema() -> u32 {
    SCHEMA
}
const fn one() -> f32 {
    1.0
}
const fn default_opacity() -> f32 {
    0.55
}
const fn default_sensitivity() -> f32 {
    0.16
}

impl Default for Layout {
    fn default() -> Self {
        let binding = |id: &str, label: &str, x, y, action: &str| Button {
            id: id.into(),
            label: label.into(),
            x,
            y,
            size: 0.105,
            action: Action::Binding(action.into()),
        };
        Self {
            schema: SCHEMA,
            scale: 1.0,
            opacity: default_opacity(),
            look_sensitivity: default_sensitivity(),
            buttons: vec![
                binding("forward", "W", 0.20, 0.68, "key.forward"),
                binding("left", "A", 0.12, 0.80, "key.left"),
                binding("back", "S", 0.20, 0.80, "key.back"),
                binding("right", "D", 0.28, 0.80, "key.right"),
                binding("sprint", "RUN", 0.12, 0.61, "key.sprint"),
                binding("jump", "JUMP", 0.88, 0.76, "key.jump"),
                binding("sneak", "SNEAK", 0.74, 0.81, "key.sneak"),
                Button {
                    id: "attack".into(),
                    label: "HIT".into(),
                    x: 0.89,
                    y: 0.48,
                    size: 0.115,
                    action: Action::Mouse("key.mouse.left".into()),
                },
                Button {
                    id: "use".into(),
                    label: "USE".into(),
                    x: 0.75,
                    y: 0.59,
                    size: 0.115,
                    action: Action::Mouse("key.mouse.right".into()),
                },
                binding("inventory", "...", 0.91, 0.13, "key.inventory"),
                binding("chat", "CHAT", 0.78, 0.13, "key.chat"),
                Button {
                    id: "pause".into(),
                    label: "II".into(),
                    x: 0.08,
                    y: 0.12,
                    size: 0.085,
                    action: Action::Key("key.keyboard.escape".into()),
                },
            ],
        }
    }
}

impl Layout {
    pub fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .map(Self::sanitised)
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let value = self.clone().sanitised();
        let bytes = serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let temporary = path.with_extension("json.new");
        fs::write(&temporary, bytes).map_err(|e| format!("{}: {e}", temporary.display()))?;
        if path.exists() {
            fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        fs::rename(&temporary, path).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn sanitised(mut self) -> Self {
        self.schema = SCHEMA;
        self.scale = finite(self.scale, 1.0).clamp(0.5, 2.0);
        self.opacity = finite(self.opacity, default_opacity()).clamp(0.15, 1.0);
        self.look_sensitivity =
            finite(self.look_sensitivity, default_sensitivity()).clamp(0.03, 1.0);
        let mut ids = HashSet::new();
        self.buttons.retain_mut(|button| {
            button.id = plain(&button.id, 32);
            button.label = plain(&button.label, 12);
            button.x = finite(button.x, 0.5).clamp(0.0, 1.0);
            button.y = finite(button.y, 0.5).clamp(0.0, 1.0);
            button.size = finite(button.size, 0.1).clamp(0.04, 0.25);
            !button.id.is_empty()
                && !button.label.is_empty()
                && button.action.valid()
                && ids.insert(button.id.clone())
        });
        self.buttons.truncate(MAX_BUTTONS);
        self
    }

    /// Top-most button at a physical-pixel position.
    pub fn hit(&self, point: [f32; 2], screen: [f32; 2]) -> Option<&Button> {
        self.buttons
            .iter()
            .rev()
            .find(|button| button.contains(point, screen, self.scale))
    }

    pub fn move_button(&mut self, id: &str, x: f32, y: f32) -> bool {
        let Some(button) = self.buttons.iter_mut().find(|b| b.id == id) else {
            return false;
        };
        button.x = finite(x, button.x).clamp(0.0, 1.0);
        button.y = finite(y, button.y).clamp(0.0, 1.0);
        true
    }

    pub fn resize_button(&mut self, id: &str, size: f32) -> bool {
        let Some(button) = self.buttons.iter_mut().find(|b| b.id == id) else {
            return false;
        };
        button.size = finite(size, button.size).clamp(0.04, 0.25);
        true
    }

    pub fn add_custom(&mut self, label: &str, action: Action) -> Result<&Button, String> {
        if self.buttons.len() >= MAX_BUTTONS {
            return Err(format!("A layout can have at most {MAX_BUTTONS} buttons."));
        }
        if !action.valid() {
            return Err("That key or action is not recognised.".into());
        }
        let label = plain(label, 12);
        if label.is_empty() {
            return Err("A button needs a label.".into());
        }
        let id = (1..)
            .map(|n| format!("custom-{n}"))
            .find(|id| self.buttons.iter().all(|b| &b.id != id))
            .unwrap();
        self.buttons.push(Button {
            id,
            label,
            x: 0.5,
            y: 0.5,
            size: 0.1,
            action,
        });
        Ok(self.buttons.last().unwrap())
    }

    pub fn remove_custom(&mut self, id: &str) -> bool {
        let before = self.buttons.len();
        self.buttons
            .retain(|button| button.id != id || !id.starts_with("custom-"));
        self.buttons.len() != before
    }
}

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

fn plain(value: &str, max: usize) -> String {
    value
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub enum TouchEvent {
    Press(Action),
    Release(Action),
    Look { dx: f32, dy: f32 },
    Tap { x: f32, y: f32 },
}

#[derive(Clone, Debug)]
enum Finger {
    Button(Action),
    Look { at: [f32; 2], start: [f32; 2] },
}

/// Multi-touch ownership. A finger that begins on a button keeps that button until release; all
/// other fingers turn the camera, and a look finger released without moving emits a menu tap.
#[derive(Default)]
pub struct Touches {
    fingers: HashMap<u64, Finger>,
}

impl Touches {
    pub fn pressed(
        &mut self,
        id: u64,
        at: [f32; 2],
        screen: [f32; 2],
        layout: &Layout,
    ) -> Vec<TouchEvent> {
        self.cancel(id)
            .into_iter()
            .chain(if let Some(button) = layout.hit(at, screen) {
                let action = button.action.clone();
                self.fingers.insert(id, Finger::Button(action.clone()));
                Some(TouchEvent::Press(action))
            } else {
                self.fingers.insert(id, Finger::Look { at, start: at });
                None
            })
            .collect()
    }

    pub fn moved(&mut self, id: u64, at: [f32; 2], layout: &Layout) -> Option<TouchEvent> {
        let Finger::Look { at: previous, .. } = self.fingers.get_mut(&id)? else {
            return None;
        };
        let (dx, dy) = (
            (at[0] - previous[0]) * layout.look_sensitivity,
            (at[1] - previous[1]) * layout.look_sensitivity,
        );
        *previous = at;
        Some(TouchEvent::Look { dx, dy })
    }

    pub fn released(&mut self, id: u64) -> Option<TouchEvent> {
        match self.fingers.remove(&id)? {
            Finger::Button(action) => Some(TouchEvent::Release(action)),
            Finger::Look { at, start } if distance(at, start) < 12.0 => {
                Some(TouchEvent::Tap { x: at[0], y: at[1] })
            }
            Finger::Look { .. } => None,
        }
    }

    pub fn cancel(&mut self, id: u64) -> Option<TouchEvent> {
        match self.fingers.remove(&id) {
            Some(Finger::Button(action)) => Some(TouchEvent::Release(action)),
            _ => None,
        }
    }

    pub fn cancel_all(&mut self) -> Vec<TouchEvent> {
        let ids: Vec<u64> = self.fingers.keys().copied().collect();
        ids.into_iter().filter_map(|id| self.cancel(id)).collect()
    }
}

fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_scale_with_the_short_side() {
        let layout = Layout::default();
        assert!(layout.buttons.iter().all(|b| b.action.valid()));
        let jump = layout.buttons.iter().find(|b| b.id == "jump").unwrap();
        assert!((jump.bounds([1000.0, 500.0], 1.0)[2] - 52.5).abs() < 0.001);
        assert_eq!(layout.hit([880.0, 380.0], [1000.0, 500.0]), Some(jump));
    }

    #[test]
    fn custom_keys_move_resize_save_and_load() {
        let mut layout = Layout::default();
        let id = layout
            .add_custom("G", Action::Key("key.keyboard.g".into()))
            .unwrap()
            .id
            .clone();
        assert!(layout.move_button(&id, 0.33, 0.44));
        assert!(layout.resize_button(&id, 0.17));
        let root = std::env::temp_dir().join(format!("horizon-touch-{}", std::process::id()));
        let path = root.join("touch-controls.json");
        layout.save(&path).unwrap();
        assert_eq!(Layout::load(&path), layout.sanitised());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_and_duplicate_buttons_are_removed() {
        let mut layout = Layout::default();
        let mut duplicate = layout.buttons[0].clone();
        duplicate.x = f32::NAN;
        layout.buttons.push(duplicate);
        layout.buttons.push(Button {
            id: "bad".into(),
            label: "bad".into(),
            x: 2.0,
            y: -2.0,
            size: 9.0,
            action: Action::Key("not-a-key".into()),
        });
        let clean = layout.sanitised();
        assert_eq!(clean.buttons.len(), Layout::default().buttons.len());
        assert!(clean.buttons.iter().all(|b| b.x <= 1.0 && b.y >= 0.0));
    }

    #[test]
    fn fingers_hold_buttons_and_look_independently() {
        let layout = Layout::default();
        let mut touches = Touches::default();
        assert!(matches!(
            touches.pressed(1, [880.0, 380.0], [1000.0, 500.0], &layout).as_slice(),
            [TouchEvent::Press(Action::Binding(key))] if key == "key.jump"
        ));
        assert!(touches.moved(1, [900.0, 390.0], &layout).is_none());
        assert!(matches!(
            touches
                .pressed(2, [500.0, 200.0], [1000.0, 500.0], &layout)
                .as_slice(),
            []
        ));
        assert!(matches!(
            touches.moved(2, [520.0, 190.0], &layout),
            Some(TouchEvent::Look { dx, dy }) if dx > 0.0 && dy < 0.0
        ));
        assert!(matches!(
            touches.released(1),
            Some(TouchEvent::Release(Action::Binding(key))) if key == "key.jump"
        ));
        assert_eq!(touches.released(2), None);
    }
}
