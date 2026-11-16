use super::{Events, Output, State, UiEvent};
use crate::frontend::{Navigation, Page};
use bevy::{
    input::{ButtonState, keyboard::KeyboardInput, mouse::MouseWheel},
    prelude::*,
};

#[derive(Default)]
pub struct Drag {
    name: String,
    owner: String,
    kind: String,
    button: Option<MouseButton>,
    last: Vec2,
    start: Vec2,
    select_all: bool,
    hover_last: Option<Vec2>,
}
#[allow(
    clippy::too_many_arguments,
    reason = "Pointer, keyboard and legacy widget state"
)]
pub fn input(
    nav: Res<Navigation>,
    options: Res<crate::Options>,
    windows: Query<&Window>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut keyboard: MessageReader<KeyboardInput>,
    mut wheel: MessageReader<MouseWheel>,
    output: Res<Output>,
    mut state: ResMut<State>,
    mut events: ResMut<Events>,
    mut drag: Local<Drag>,
) {
    if !nav.legacy_visible() || nav.page == Page::Settings || options.editor_smoke_test {
        keyboard.clear();
        wheel.clear();
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let cursor = window
        .cursor_position()
        .map(|p| (p - output.origin) / output.scale);
    let hit = cursor
        .and_then(|p| {
            output
                .hits
                .iter()
                .rev()
                .find(|h| h.rect.contains(p) && h.clip.contains(p))
        })
        .cloned();
    if let Some(p) = cursor {
        let delta = drag.hover_last.map(|last| p - last).unwrap_or_default();
        drag.hover_last = Some(p);
        events.0.push(UiEvent::Move {
            name: hit.as_ref().map(|h| h.name.clone()).unwrap_or_default(),
            point: p,
            delta,
        });
    }
    let hover = hit.as_ref().map(|h| h.name.clone()).unwrap_or_default();
    if state.hover != hover {
        state.hover = hover;
        state.revision += 1;
    }
    for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
        if buttons.just_pressed(button)
            && let (Some(p), Some(hit)) = (cursor, &hit)
        {
            drag.name = hit.name.clone();
            drag.owner = hit.owner.clone();
            drag.kind = hit.kind.clone();
            drag.button = Some(button);
            drag.last = p;
            drag.start = p;
            if hit.kind == "Slider" {
                let ratio = if hit.horizontal {
                    (p.x - hit.rect.min.x) / hit.rect.width()
                } else {
                    (p.y - hit.rect.min.y) / hit.rect.height()
                };
                let value = if hit.reversed { 1.0 - ratio } else { ratio }
                    .clamp(0.0, 1.0)
                    .to_string();
                state.values.insert(hit.name.clone(), value.clone());
                if let Some(owner) = hit.name.strip_prefix("scroll:") {
                    let limit = output.scroll_limits.get(owner).copied().unwrap_or(0);
                    state.scroll.insert(
                        owner.into(),
                        (value.parse::<f32>().unwrap_or(0.) * limit as f32).round() as usize,
                    );
                }
                events.0.push(UiEvent::Value {
                    name: hit.name.clone(),
                    value,
                });
            }
            if button == MouseButton::Left {
                state.pressed = hit.name.clone();
                if hit.editable {
                    state.focused = hit.name.clone();
                    drag.select_all = true;
                } else {
                    state.focused.clear();
                }
                state.revision += 1;
            }
            events.0.push(UiEvent::Press {
                name: hit.name.clone(),
                point: p,
                button,
            });
        }
        if buttons.just_released(button) && drag.button == Some(button) {
            if let Some(p) = cursor {
                events.0.push(UiEvent::Release {
                    name: drag.name.clone(),
                    point: p,
                    button,
                });
                if button == MouseButton::Left
                    && p.distance(drag.start) < 4.0
                    && hit.as_ref().is_some_and(|h| h.name == drag.name)
                {
                    events.0.push(UiEvent::Activate {
                        name: drag.name.clone(),
                        owner: drag.owner.clone(),
                        point: p,
                    });
                }
            }
            state.pressed.clear();
            state.control_drag = None;
            state.revision += 1;
            drag.button = None;
        }
    }
    if let (Some(p), Some(button)) = (cursor, drag.button)
        && p != drag.last
    {
        if button == MouseButton::Left && p.distance(drag.start) >= 4.0 {
            let image = match drag.name.as_str() {
                "new analog" => Some("botlab/ctrl_analog.bmp".to_string()),
                "new switch" => Some("botlab/ctrl_switch.bmp".to_string()),
                "new button" => Some("botlab/ctrl_button.bmp".to_string()),
                name if name.starts_with("control:") => state
                    .cells
                    .get("control grid")
                    .and_then(|cells| cells.iter().find(|cell| cell.key == name))
                    .and_then(|cell| cell.template.as_ref())
                    .and_then(|template| template.find("control img"))
                    .map(|widget| widget.value("_Filename").to_string()),
                _ => None,
            };
            if let Some(image) = image.filter(|image| !image.is_empty()) {
                state.control_drag = Some((image, p));
                state.revision += 1;
            }
        }
        if drag.kind == "Slider"
            && let Some(hit) = output.hits.iter().find(|h| h.name == drag.name)
        {
            let ratio = (if hit.horizontal {
                (p.x - hit.rect.min.x) / hit.rect.width()
            } else {
                (p.y - hit.rect.min.y) / hit.rect.height()
            })
            .clamp(0.0, 1.0);
            let value = if hit.reversed { 1.0 - ratio } else { ratio }.to_string();
            state.values.insert(drag.name.clone(), value.clone());
            if let Some(owner) = drag.name.strip_prefix("scroll:") {
                let limit = output.scroll_limits.get(owner).copied().unwrap_or(0);
                state.scroll.insert(
                    owner.into(),
                    (value.parse::<f32>().unwrap_or(0.) * limit as f32).round() as usize,
                );
            }
            events.0.push(UiEvent::Value {
                name: drag.name.clone(),
                value,
            });
            state.revision += 1;
        }
        events.0.push(UiEvent::Drag {
            name: drag.name.clone(),
            point: p,
            delta: p - drag.last,
            button,
        });
        drag.last = p;
    }
    for event in wheel.read() {
        if let Some(hit) = &hit
            && hit.name.contains("3d")
        {
            events.0.push(UiEvent::Wheel {
                name: hit.name.clone(),
                delta: event.y,
            });
        }
        if let Some(hit) = &hit {
            let owner = if state.cells.contains_key(&hit.name) {
                &hit.name
            } else {
                &hit.owner
            };
            if let Some(cells) = state.cells.get(owner) {
                let limit = output
                    .scroll_limits
                    .get(owner)
                    .copied()
                    .unwrap_or_else(|| cells.len().saturating_sub(1));
                let old = state.scroll.get(owner).copied().unwrap_or(0);
                let next = (old as i32 - event.y.signum() as i32).clamp(0, limit as i32) as usize;
                state.scroll.insert(owner.clone(), next);
                state.revision += 1;
            }
        }
    }
    for (name, offset) in &mut state.scroll {
        if let Some(limit) = output.scroll_limits.get(name) {
            *offset = (*offset).min(*limit);
        }
    }
    if state.focused.is_empty() {
        keyboard.clear();
        return;
    }
    let name = state.focused.clone();
    let max = output
        .hits
        .iter()
        .find(|h| h.name == name)
        .map(|h| h.max_chars)
        .filter(|n| *n > 0)
        .unwrap_or(128);
    let mut text = state.values.get(&name).cloned().unwrap_or_default();
    let before = text.clone();
    let mut ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    if ctrl && keys.just_pressed(KeyCode::KeyA) {
        drag.select_all = true;
    }
    for event in keyboard.read() {
        if matches!(event.key_code, KeyCode::ControlLeft | KeyCode::ControlRight) {
            ctrl = event.state == ButtonState::Pressed;
            continue;
        }
        if event.state != ButtonState::Pressed {
            continue;
        }
        match event.key_code {
            KeyCode::KeyA if ctrl => {
                drag.select_all = true;
            }
            KeyCode::Backspace | KeyCode::Delete => {
                if drag.select_all {
                    text.clear();
                } else {
                    text.pop();
                }
                drag.select_all = false;
            }
            KeyCode::Enter => {
                state.focused.clear();
            }
            _ if !ctrl => {
                if let Some(chars) = &event.text
                    && chars.chars().any(|c| c.is_ascii_graphic() || c == ' ')
                {
                    if drag.select_all {
                        text.clear();
                        drag.select_all = false;
                    }
                    for ch in chars.chars().filter(|c| c.is_ascii_graphic() || *c == ' ') {
                        if text.len() < max {
                            text.push(ch);
                        }
                    }
                }
            }
            _ => (),
        }
    }
    if text != before {
        state.values.insert(name.clone(), text.clone());
        state.revision += 1;
        events.0.push(UiEvent::Value { name, value: text });
    }
}
