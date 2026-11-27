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
    last_click: Option<(String, std::time::Duration, Vec2)>,
    paste: Option<(String, bevy::clipboard::ClipboardRead)>,
    text_cursor: bool,
    hover_last: Option<Vec2>,
}
#[allow(
    clippy::too_many_arguments,
    reason = "Pointer, keyboard and legacy widget state"
)]
pub fn input(
    nav: Res<Navigation>,
    options: Res<crate::Options>,
    windows: Query<(Entity, &Window)>,
    mut commands: Commands,
    time: Res<Time<Real>>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut keyboard: MessageReader<KeyboardInput>,
    mut wheel: MessageReader<MouseWheel>,
    output: Res<Output>,
    mut state: ResMut<State>,
    mut events: ResMut<Events>,
    mut drag: Local<Drag>,
    mut clipboard: ResMut<bevy::clipboard::Clipboard>,
) {
    let Ok((window_id, window)) = windows.single() else {
        return;
    };
    if !nav.legacy_visible() || nav.page == Page::Settings || options.editor_smoke_test {
        if drag.text_cursor {
            commands
                .entity(window_id)
                .insert(bevy::window::CursorIcon::default());
            drag.text_cursor = false;
        }
        if !state.focused.is_empty() {
            state.focused.clear();
            state.edit = default();
            state.revision += 1;
        }
        keyboard.clear();
        wheel.clear();
        return;
    }
    let now = time.elapsed();
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
    let text_cursor = hit
        .as_ref()
        .is_some_and(|h| h.editable && output.fields.contains_key(&h.name));
    if drag.text_cursor != text_cursor {
        let icon = if text_cursor {
            bevy::window::SystemCursorIcon::Text
        } else {
            bevy::window::SystemCursorIcon::Default
        };
        commands
            .entity(window_id)
            .insert(bevy::window::CursorIcon::from(icon));
        drag.text_cursor = text_cursor;
    }
    if buttons.just_pressed(MouseButton::Left) && hit.is_none() && !state.focused.is_empty() {
        state.focused.clear();
        state.revision += 1;
    }
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
                if let Some(field) = output.fields.get(&hit.name) {
                    let extend = state.focused == hit.name
                        && keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
                    state.focused = hit.name.clone();
                    state.edit.focus(&hit.name, &field.value);
                    let double = drag.last_click.as_ref().is_some_and(|(name, at, point)| {
                        *name == hit.name
                            && now.saturating_sub(*at).as_millis() < 400
                            && point.distance(p) <= 4.
                    });
                    let at = field.pick(p);
                    state.edit.select_to(at, extend);
                    if double && !extend {
                        let (start, end) = super::text_field::word_at(&field.value, at);
                        state.edit.anchor = start;
                        state.edit.head = end;
                    }
                    drag.last_click = Some((hit.name.clone(), now, p));
                    state.edit.wake(now);
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
    if let Some(point) = cursor
        && drag.button == Some(MouseButton::Left)
        && state.focused == drag.name
        && let Some(field) = output.fields.get(&drag.name)
        && (point != drag.last || !field.clip.contains(point) && point.distance(drag.start) >= 4.)
    {
        let position = field.pick(point);
        if position != state.edit.head {
            state.edit.select_to(position, true);
            state.edit.wake(now);
            state.revision += 1;
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
    edit_fields(
        &mut state,
        &output,
        &keys,
        &mut keyboard,
        &mut events,
        &mut clipboard,
        &mut drag,
        now,
    );
}

#[allow(
    clippy::too_many_arguments,
    reason = "Text editing uses the active field, input devices and their event stream"
)]
fn edit_fields(
    state: &mut State,
    output: &Output,
    keys: &ButtonInput<KeyCode>,
    keyboard: &mut MessageReader<KeyboardInput>,
    events: &mut Events,
    clipboard: &mut bevy::clipboard::Clipboard,
    drag: &mut Drag,
    now: std::time::Duration,
) {
    let shift_keys = [KeyCode::ShiftLeft, KeyCode::ShiftRight];
    let mut shift = keys.any_pressed(shift_keys);
    if keys.just_pressed(KeyCode::Tab) {
        let fields: Vec<_> = output
            .hits
            .iter()
            .filter(|h| h.editable && output.fields.contains_key(&h.name))
            .collect();
        if !fields.is_empty() {
            let i = fields.iter().position(|h| h.name == state.focused);
            let next = if shift {
                i.unwrap_or(0).wrapping_add(fields.len() - 1) % fields.len()
            } else {
                i.map_or(0, |i| (i + 1) % fields.len())
            };
            state.focused = fields[next].name.clone();
        }
    }
    let name = state.focused.clone();
    let Some(field) = output.fields.get(&name) else {
        if !name.is_empty() || !state.edit.field.is_empty() {
            state.focused.clear();
            state.edit = default();
            state.revision += 1;
        }
        drag.paste = None;
        keyboard.clear();
        return;
    };
    let mut text = state.values.get(&name).unwrap_or(&field.value).clone();
    let before = text.clone();
    let new_focus = state.edit.field != name;
    state.edit.focus(&name, &text);
    state.edit.scroll = field.scroll;
    let old_caret = state.edit.caret_visible;
    let max = output
        .hits
        .iter()
        .find(|h| h.name == name && h.max_chars > 0)
        .map_or(128, |h| h.max_chars);
    let control_keys = [KeyCode::ControlLeft, KeyCode::ControlRight];
    let super_keys = [KeyCode::SuperLeft, KeyCode::SuperRight];
    let alt_keys = [KeyCode::AltLeft, KeyCode::AltRight];
    let mut ctrl = keys.any_pressed(control_keys);
    let mut meta = keys.any_pressed(super_keys);
    let mut alt = keys.any_pressed(alt_keys);
    let mut handled = new_focus;
    let mut submit = false;
    for event in keyboard.read() {
        let down = event.state == ButtonState::Pressed;
        match event.key_code {
            key if control_keys.contains(&key) => {
                ctrl = down;
                continue;
            }
            key if super_keys.contains(&key) => {
                meta = down;
                continue;
            }
            key if shift_keys.contains(&key) => {
                shift = down;
                continue;
            }
            key if alt_keys.contains(&key) => {
                alt = down;
                continue;
            }
            _ => {}
        }
        if !down {
            continue;
        }
        let shortcut = meta || ctrl && !alt;
        let word = if cfg!(target_os = "macos") { alt } else { ctrl };
        let edit = &mut state.edit;
        let head = edit.head;
        match event.key_code {
            KeyCode::KeyA if shortcut => {
                edit.anchor = 0;
                edit.head = text.len();
            }
            KeyCode::KeyZ if shortcut => edit.restore(&mut text, shift),
            KeyCode::KeyY if shortcut => edit.restore(&mut text, true),
            KeyCode::KeyC | KeyCode::KeyX if shortcut => {
                let range = edit.selection();
                if !range.is_empty()
                    && clipboard.set_text(&text[range]).is_ok()
                    && event.key_code == KeyCode::KeyX
                {
                    edit.replace(&mut text, "", max, field.allow_newlines);
                }
            }
            KeyCode::KeyV if shortcut => {
                let mut paste = clipboard.fetch_text();
                if let Some(result) = paste.poll_result() {
                    if let Ok(value) = result {
                        edit.replace(&mut text, &value, max, field.allow_newlines);
                    }
                } else {
                    drag.paste = Some((name.clone(), paste));
                }
            }
            KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                let right = event.key_code == KeyCode::ArrowRight;
                if meta {
                    let pos = if !field.multiline && right {
                        text.len()
                    } else {
                        field.line_edge(head, right)
                    };
                    edit.select_to(text.floor_char_boundary(pos.min(text.len())), shift);
                } else {
                    edit.horizontal(&text, right, shift, word);
                }
            }
            KeyCode::Home | KeyCode::End => {
                let end = event.key_code == KeyCode::End;
                let pos = if !field.multiline || shortcut {
                    if end { text.len() } else { 0 }
                } else {
                    field.line_edge(head, end)
                };
                edit.select_to(text.floor_char_boundary(pos.min(text.len())), shift);
            }
            KeyCode::ArrowUp | KeyCode::ArrowDown if field.multiline => {
                let pos = field.vertical(head, event.key_code == KeyCode::ArrowDown);
                edit.select_to(text.floor_char_boundary(pos.min(text.len())), shift);
            }
            KeyCode::Backspace | KeyCode::Delete => {
                if edit.selection().is_empty() {
                    edit.horizontal(&text, event.key_code == KeyCode::Delete, true, word);
                }
                edit.replace(&mut text, "", max, field.allow_newlines);
            }
            KeyCode::Enter | KeyCode::NumpadEnter if !alt => {
                if field.allow_newlines {
                    edit.replace(&mut text, "\n", max, true);
                } else {
                    submit = true;
                    state.focused.clear();
                    handled = true;
                    break;
                }
            }
            KeyCode::Escape => {
                state.focused.clear();
                handled = true;
                break;
            }
            _ if !shortcut => {
                let Some(chars) = &event.text else {
                    continue;
                };
                edit.replace(&mut text, chars, max, field.allow_newlines);
            }
            _ => continue,
        }
        handled = true;
    }
    if let Some((target, read)) = &mut drag.paste {
        if *target != name || state.focused != name {
            drag.paste = None;
        } else if let Some(result) = read.poll_result() {
            if let Ok(value) = result {
                state
                    .edit
                    .replace(&mut text, &value, max, field.allow_newlines);
                handled = true;
            }
            drag.paste = None;
        }
    }
    if handled {
        state.edit.wake(now);
    } else {
        state.edit.caret_visible =
            (now.saturating_sub(state.edit.blink_start).as_millis() / 500).is_multiple_of(2);
    }
    if handled || old_caret != state.edit.caret_visible {
        state.revision += 1;
    }
    if text != before {
        state.values.insert(name.clone(), text.clone());
        events.0.push(UiEvent::Value {
            name: name.clone(),
            value: text,
        });
    }
    if submit {
        events.0.push(UiEvent::Submit(name));
    }
}
