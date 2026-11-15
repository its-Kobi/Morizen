//! Android's in-game control overlay. The serialisable/hit-tested layout and finger ownership live
//! in `horizon-client-input`; this module maps their events into the desktop game's ordinary input.

use super::*;
use horizon_client::touch::{Action as TouchAction, Layout, TouchEvent, Touches};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use winit::event::{MouseButton, Touch, TouchPhase};

/// A phone finger as the desktop pointer it stands for: where it is, and the left button down
/// when it lands or up when it lifts. The startup screen and the title menu answer the same
/// `CursorMoved`/`MouseInput` events as a mouse, so they need no touch code of their own.
pub(super) fn pointer_events(
    phase: TouchPhase,
    at: winit::dpi::PhysicalPosition<f64>,
) -> Vec<WindowEvent> {
    let mut events = vec![WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: at,
    }];
    let state = match phase {
        TouchPhase::Started => Some(ElementState::Pressed),
        TouchPhase::Ended | TouchPhase::Cancelled => Some(ElementState::Released),
        _ => None,
    };
    if let Some(state) = state {
        events.push(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Left,
        });
    }
    events
}

pub(super) struct TouchControls {
    pub layout: Layout,
    /// Android's virtual mouse: taps move a pointer and click instead of using the overlay.
    pub mouse: bool,
    /// The finger dragging the virtual mouse, so a second finger cannot move it.
    mouse_finger: Option<u64>,
    /// The finger that pressed the mouse-mode button; its release must not click.
    toggle_finger: Option<u64>,
    fingers: Touches,
    held: HashMap<TouchAction, usize>,
    path: PathBuf,
}

impl TouchControls {
    pub fn load(game_dir: &Path) -> Self {
        let path = game_dir.join("touch-controls.json");
        Self {
            layout: Layout::load(&path),
            mouse: false,
            mouse_finger: None,
            toggle_finger: None,
            fingers: Touches::default(),
            held: HashMap::new(),
            path,
        }
    }

    /// The mouse-mode button: top right, always the same place on every screen.
    pub fn toggle_bounds(screen: [f32; 2], scale: f32) -> [f32; 4] {
        let side = 0.075 * scale * screen[0].min(screen[1]);
        [screen[0] - side, 0.0, side, side]
    }

    pub fn toggle_hit(&self, at: [f32; 2], screen: [f32; 2]) -> bool {
        let [x, y, w, h] = Self::toggle_bounds(screen, self.layout.scale);
        at[0] >= x && at[0] <= x + w && at[1] >= y && at[1] <= y + h
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
    pub(super) fn touch_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        touch: Touch,
    ) -> Result<()> {
        let screen = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width as f32, r.config.height as f32])
            .unwrap_or([1.0, 1.0]);
        let at = [touch.location.x as f32, touch.location.y as f32];
        // The mouse-mode button is always live, on the title screen and on every menu, so a
        // phone can leave a menu that has no other way out.
        if touch.phase == TouchPhase::Started && self.touch.toggle_hit(at, screen) {
            self.release_touch_controls();
            self.touch.mouse = !self.touch.mouse;
            self.touch.mouse_finger = None;
            self.touch.toggle_finger = Some(touch.id);
            self.last_input = Instant::now();
            return Ok(());
        }
        if self.touch.toggle_finger == Some(touch.id) {
            if matches!(touch.phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                self.touch.toggle_finger = None;
            }
            return Ok(());
        }
        if self.touch.mouse {
            self.virtual_mouse(event_loop, id, touch, at)?;
            self.last_input = Instant::now();
            return Ok(());
        }
        let active =
            self.screen == Screen::Playing && self.title.is_none() && self.player.is_some();
        // Off the in-game HUD there is no overlay to press, so the finger is the pointer: a tap
        // moves the cursor there and clicks, exactly like the desktop mouse. Without this a
        // phone can only look at the title screen and its menus.
        if !active {
            self.virtual_mouse(event_loop, id, touch, at)?;
            self.last_input = Instant::now();
            return Ok(());
        }
        let events = self.touch.events(touch, screen, active);
        for event in events {
            self.apply_touch_event(event)?;
        }
        self.last_input = Instant::now();
        Ok(())
    }

    /// One finger of the virtual mouse: press where the finger lands, drag the pointer while it
    /// moves, release where it lifts. The press goes through the ordinary desktop handler, so
    /// every menu, button, slider and inventory slot answers a phone tap like a real click.
    fn virtual_mouse(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        touch: Touch,
        at: [f32; 2],
    ) -> Result<()> {
        match touch.phase {
            TouchPhase::Started if self.touch.mouse_finger.is_none() => {
                self.touch.mouse_finger = Some(touch.id);
                self.move_cursor(at[0], at[1])?;
                self.send_mouse(event_loop, id, ElementState::Pressed, MouseButton::Left)?;
            }
            TouchPhase::Moved if self.touch.mouse_finger == Some(touch.id) => {
                self.move_cursor(at[0], at[1])?;
            }
            TouchPhase::Ended | TouchPhase::Cancelled
                if self.touch.mouse_finger == Some(touch.id) =>
            {
                self.touch.mouse_finger = None;
                self.send_mouse(event_loop, id, ElementState::Released, MouseButton::Left)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Feeds a virtual mouse press through the ordinary desktop handler, so every menu,
    /// button, slider and inventory slot answers a phone tap exactly like a pointer click.
    fn send_mouse(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        state: ElementState,
        button: MouseButton,
    ) -> Result<()> {
        self.window_event(
            event_loop,
            id,
            WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state,
                button,
            },
        );
        Ok(())
    }

    pub(super) fn release_touch_controls(&mut self) {
        self.touch.mouse_finger = None;
        self.touch.toggle_finger = None;
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
        let atlas = &self.gui.atlas;
        let screen = [batch.width * batch.scale, batch.height * batch.scale];
        let playing =
            self.screen == Screen::Playing && self.title.is_none() && self.player.is_some();
        if playing {
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
        // The mouse button is drawn over every screen, so the title, options and every menu
        // have a way to be pressed on a phone.
        let [x, y, w, h] =
            TouchControls::toggle_bounds(screen, self.touch.layout.scale).map(|v| v / batch.scale);
        let active = self.touch.mouse;
        let fill = if active {
            rgba(212, 162, 76, 210)
        } else {
            rgba(15, 18, 24, 165)
        };
        batch.fill(atlas, x, y, w, h, fill);
        let border = rgba(255, 255, 255, 210);
        let edge = 1.0;
        batch.fill(atlas, x, y, w, edge, border);
        batch.fill(atlas, x, y + h - edge, w, edge, border);
        batch.fill(atlas, x, y, edge, h, border);
        batch.fill(atlas, x + w - edge, y, edge, h, border);
        let tw = HudBatch::text_width(atlas, "MOUSE");
        batch.text(
            atlas,
            "MOUSE",
            x + (w - tw) / 2.0,
            y + (h - 8.0) / 2.0,
            u32::MAX,
            true,
        );
        if active {
            draw_pointer(batch, atlas, self.cursor);
        }
    }
}

/// The virtual mouse: a small outlined arrow at the pointer's HUD position.
fn draw_pointer(batch: &mut HudBatch, atlas: &HudAtlas, at: (f32, f32)) {
    let (x, y) = (at.0.floor(), at.1.floor());
    const ROWS: [u8; 10] = [1, 2, 3, 4, 5, 6, 7, 4, 2, 1];
    for (i, &w) in ROWS.iter().enumerate() {
        let row = y + i as f32;
        batch.fill(
            atlas,
            x - 1.0,
            row,
            f32::from(w) + 2.0,
            1.0,
            rgba(0, 0, 0, 230),
        );
    }
    batch.fill(atlas, x - 1.0, y + 10.0, 2.0, 1.0, rgba(0, 0, 0, 230));
    for (i, &w) in ROWS.iter().enumerate().skip(1).take(8) {
        let row = y + i as f32;
        batch.fill(
            atlas,
            x,
            row,
            f32::from(w) - 1.0,
            1.0,
            rgba(255, 255, 255, 240),
        );
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
