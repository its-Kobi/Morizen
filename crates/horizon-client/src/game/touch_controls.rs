//! Android's in-game control overlay. The serialisable/hit-tested layout and finger ownership live
//! in `horizon-client-input`; this module maps their events into the desktop game's ordinary input.

use super::*;
use horizon_client::touch::{Action as TouchAction, Layout, TouchEvent, Touches};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use winit::event::{MouseButton, Touch, TouchPhase};

pub(super) struct TouchControls {
    pub layout: Layout,
    fingers: Touches,
    held: HashMap<TouchAction, usize>,
    path: PathBuf,
}

impl TouchControls {
    pub fn load(game_dir: &Path) -> Self {
        let path = game_dir.join("touch-controls.json");
        Self {
            layout: Layout::load(&path),
            fingers: Touches::default(),
            held: HashMap::new(),
            path,
        }
    }

    #[allow(dead_code)] // Used by the touch-controls options editor as it lands.
    pub fn save(&self) -> std::result::Result<(), String> {
        self.layout.save(&self.path)
    }

    fn events(&mut self, touch: Touch, screen: [f32; 2], active: bool) -> Vec<TouchEvent> {
        let at = [touch.location.x as f32, touch.location.y as f32];
        let raw = match touch.phase {
            TouchPhase::Started if active => {
                self.fingers.pressed(touch.id, at, screen, &self.layout)
            }
            TouchPhase::Moved if active => self
                .fingers
                .moved(touch.id, at, &self.layout)
                .into_iter()
                .collect(),
            TouchPhase::Ended if active => self.fingers.released(touch.id).into_iter().collect(),
            TouchPhase::Cancelled | TouchPhase::Ended => {
                self.fingers.cancel(touch.id).into_iter().collect()
            }
            _ => Vec::new(),
        };
        self.edges(raw)
    }

    fn edges(&mut self, events: Vec<TouchEvent>) -> Vec<TouchEvent> {
        events
            .into_iter()
            .filter_map(|event| match event {
                TouchEvent::Press(action) => {
                    let count = self.held.entry(action.clone()).or_default();
                    *count += 1;
                    (*count == 1).then_some(TouchEvent::Press(action))
                }
                TouchEvent::Release(action) => {
                    let count = self.held.entry(action.clone()).or_default();
                    *count = count.saturating_sub(1);
                    (*count == 0).then_some(TouchEvent::Release(action))
                }
                other => Some(other),
            })
            .collect()
    }

    fn release_all(&mut self) -> Vec<TouchEvent> {
        self.fingers.cancel_all();
        self.held
            .drain()
            .filter_map(|(action, count)| (count > 0).then_some(TouchEvent::Release(action)))
            .collect()
    }
}

impl Game<'_> {
    pub(super) fn touch_event(&mut self, touch: Touch) -> Result<()> {
        let screen = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width as f32, r.config.height as f32])
            .unwrap_or([1.0, 1.0]);
        let active =
            self.screen == Screen::Playing && self.title.is_none() && self.player.is_some();
        let events = self.touch.events(touch, screen, active);
        for event in events {
            self.apply_touch_event(event)?;
        }
        self.last_input = Instant::now();
        Ok(())
    }

    pub(super) fn release_touch_controls(&mut self) {
        let events = self.touch.release_all();
        for event in events {
            let _ = self.apply_touch_event(event);
        }
    }

    fn apply_touch_event(&mut self, event: TouchEvent) -> Result<()> {
        match event {
            TouchEvent::Press(action) => self.touch_action(action, true),
            TouchEvent::Release(action) => self.touch_action(action, false),
            TouchEvent::Look { dx, dy } => {
                let x = if self.settings.invert_x { -dx } else { dx };
                let y = if self.settings.invert_y { -dy } else { dy };
                self.yaw += x;
                self.pitch = (self.pitch + y).clamp(-90.0, 90.0);
                Ok(())
            }
            TouchEvent::Tap { .. } => Ok(()),
        }
    }

    fn touch_action(&mut self, action: TouchAction, pressed: bool) -> Result<()> {
        match action {
            TouchAction::Binding(binding) => {
                if let Some(key) = binding_key(&binding) {
                    self.mapped_key(key, pressed, false)?;
                }
            }
            TouchAction::Key(name) => {
                let Some(key) = horizon_client::keybinds::key_code(&name) else {
                    return Ok(());
                };
                let mapped = self.settings.keys.translate_all(key);
                for input in mapped {
                    match input {
                        Ok(key) => self.mapped_key(key, pressed, false)?,
                        Err(button) => self.play_mouse(button, pressed)?,
                    }
                }
            }
            TouchAction::Mouse(name) => {
                let button = match name.as_str() {
                    "key.mouse.left" => MouseButton::Left,
                    "key.mouse.right" => MouseButton::Right,
                    "key.mouse.middle" => MouseButton::Middle,
                    _ => return Ok(()),
                };
                self.play_mouse(button, pressed)?;
            }
        }
        Ok(())
    }

    pub(super) fn draw_touch_controls(&mut self, batch: &mut HudBatch) {
        if self.screen != Screen::Playing || self.title.is_some() || self.player.is_none() {
            return;
        }
        let atlas = &self.gui.atlas;
        let screen = [batch.width * batch.scale, batch.height * batch.scale];
        let alpha = (self.touch.layout.opacity * 190.0).round() as u8;
        for button in &self.touch.layout.buttons {
            let [x, y, w, h] = button.bounds(screen, self.touch.layout.scale);
            let [x, y, w, h] = [x, y, w, h].map(|value| value / batch.scale);
            let down = self.touch.held.contains_key(&button.action);
            let fill = if down {
                rgba(212, 162, 76, alpha)
            } else {
                rgba(15, 18, 24, alpha)
            };
            batch.fill(atlas, x, y, w, h, fill);
            let border = rgba(255, 255, 255, alpha.saturating_add(35));
            let edge = 1.0;
            batch.fill(atlas, x, y, w, edge, border);
            batch.fill(atlas, x, y + h - edge, w, edge, border);
            batch.fill(atlas, x, y, edge, h, border);
            batch.fill(atlas, x + w - edge, y, edge, h, border);
            let tw = HudBatch::text_width(atlas, &button.label);
            batch.text(
                atlas,
                &button.label,
                x + (w - tw) / 2.0,
                y + (h - 8.0) / 2.0,
                u32::MAX,
                true,
            );
        }
    }
}

fn binding_key(binding: &str) -> Option<KeyCode> {
    Some(match binding {
        "key.forward" => KeyCode::KeyW,
        "key.left" => KeyCode::KeyA,
        "key.back" => KeyCode::KeyS,
        "key.right" => KeyCode::KeyD,
        "key.jump" => KeyCode::Space,
        "key.sneak" => KeyCode::ShiftLeft,
        "key.sprint" => KeyCode::ControlLeft,
        "key.inventory" => KeyCode::KeyE,
        "key.chat" => KeyCode::KeyT,
        "key.drop" => KeyCode::KeyQ,
        "key.swapOffhand" => KeyCode::KeyF,
        "key.command" => KeyCode::Slash,
        "key.togglePerspective" => KeyCode::F5,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mobile_bindings_end_at_the_desktop_actions() {
        assert_eq!(binding_key("key.forward"), Some(KeyCode::KeyW));
        assert_eq!(binding_key("key.inventory"), Some(KeyCode::KeyE));
        assert_eq!(binding_key("unknown"), None);
    }
}
