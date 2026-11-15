use super::{
    Dialog, Editor, Mode,
    catalog::Catalog,
    viewport::{Pick, View},
};
use crate::{
    frontend::{Draft, Navigation, Page},
    legacy_ui::{self, Events, Output, UiEvent, assets::LegacyAssets},
    settings::EngineSettings,
};
use arena_core::{
    editor::{chassis, paint},
    robot::*,
    *,
};
use bevy::prelude::*;
use std::path::{Path, PathBuf};

fn select_component(editor: &mut Editor, catalog: &Catalog, key: &str, preserve_socket: bool) {
    editor.component = key.into();
    editor.selected = None;
    editor.preview_burst_end = false;
    editor.pending = None;
    editor.ghost_pick = None;
    editor.ghost_pose = None;
    editor.ghost_model = None;
    editor.attach_offset = 0.0;
    editor.attach_angle = 0.0;
    if let Some(asset) = catalog.components.get(key) {
        let keep = preserve_socket
            && asset
                .model
                .attachments
                .iter()
                .any(|s| super::placement::socket_id(s) == Some(editor.selected_socket));
        if !keep {
            editor.selected_socket = super::placement::default_socket(&asset.model);
        }
    }
}

pub fn rebuild(editor: &mut Editor) -> Result<(), String> {
    let bottom = editor.outline.clone();
    let top = if editor.top.len() == bottom.len() {
        editor.top.clone()
    } else {
        bottom.clone()
    };
    let height = editor.height;
    editor.document.edit("Chassis structure", |robot| {
        robot.chassis.bottom = bottom;
        robot.chassis.top = top;
        robot.chassis.height = height;
        chassis::rebuild(&mut robot.chassis)
    })
}
pub fn actions(class: &str) -> &'static [&'static str] {
    match class {
        "SpinMotor" | "ServoMotor" => &[
            "None",
            "Spin Clockwise",
            "Spin Counter-clockwise",
            "On/Off Clockwise",
            "On/Off Counter-clockwise",
        ],
        "ServoPiston" => &["None", "Extend", "Retract"],
        "BurstMotor" | "BurstPiston" | "Cannon" => &["None", "Fire"],
        "Steering" => &["None", "Forward", "Backward", "Left", "Right"],
        _ => &["None", "Activate"],
    }
}
pub(super) fn begin_connection(editor: &mut Editor, index: usize) -> Result<(), String> {
    let robot = &editor.document.robot;
    let part = robot.components.get(index).ok_or("Missing component")?;
    let controller = editor
        .selected_controller
        .and_then(|i| robot.controllers.get(i))
        .ok_or("Select a controller from the grid first")?;
    if !matches!(
        part.class.as_str(),
        "SpinMotor" | "ServoMotor" | "BurstMotor" | "BurstPiston" | "ServoPiston" | "Steering"
    ) {
        return Err("This component has no controllable actions".into());
    }
    let choices = actions(&part.class);
    let mut maps = [0; 2];
    for binding in robot
        .bindings
        .iter()
        .filter(|b| b.component == index && b.controller == controller.position)
    {
        if let Some(map) = maps.get_mut(binding.key) {
            *map = choices
                .iter()
                .position(|&action| action == binding.action)
                .ok_or_else(|| {
                    format!("Cannot edit the existing wiring action: {}", binding.action)
                })?;
        }
    }
    editor.selected = Some(index);
    editor.binding_actions = maps;
    editor.dialog = Some(Dialog::Connection);
    Ok(())
}
pub fn save(editor: &mut Editor, path: &Path) -> Result<(), String> {
    if matches!(editor.mode, Mode::Base | Mode::Top) {
        rebuild(editor)?;
    }
    if editor.document.robot.chassis.model.is_none() {
        return Err("Build a chassis before saving the robot".into());
    }
    if editor.document.robot.name.trim().is_empty() {
        return Err("Enter a robot name before saving".into());
    }
    let bytes = ra2_formats::bot::encode(&editor.document.robot)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let temp = path.with_extension("bot.tmp");
    std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(temp, path).map_err(|e| e.to_string())?;
    editor.source = Some(path.into());
    editor.document.dirty = false;
    info!("Native BOT saved: {}", path.display());
    Ok(())
}
pub fn save_team_slot(editor: &mut Editor) -> Result<(), String> {
    let (team, slot) = editor.team_slot.ok_or("No team robot slot")?;
    let root = editor.team_root.clone().ok_or("No team directory")?;
    let path = crate::team_hq::robot_path(&root, team, slot);
    let previous = match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    let source = editor.source.clone();
    save(editor, &path)?;
    if let Err(error) = crate::team_hq::register_robot(&root, team, slot) {
        let rollback = if let Some(bytes) = previous {
            crate::team_hq::write_atomic(&path, &bytes)
        } else {
            std::fs::remove_file(&path).map_err(|e| e.to_string())
        };
        editor.source = source;
        editor.document.dirty = true;
        return Err(match rollback {
            Ok(()) => error,
            Err(rollback) => format!("{error}; could not restore robot slot: {rollback}"),
        });
    }
    Ok(())
}

fn export_path(catalog: &Catalog, name: &str) -> Result<PathBuf, String> {
    let name = name.trim().trim_end_matches(".bot");
    if name.is_empty() || name.len() > 120 || name.contains(['/', '\\', ':', '\0']) || name == ".."
    {
        return Err("Enter a filename without path separators".into());
    }
    Ok(catalog
        .root
        .join("Robot Designs")
        .join(format!("{name}.bot")))
}
pub fn load(editor: &mut Editor, path: &Path) -> Result<(), String> {
    let mut robot = ra2_formats::bot::decode(&std::fs::read(path).map_err(|e| e.to_string())?)?;
    robot.source_version = "1.12".into();
    for controller in &mut robot.controllers {
        for key in &mut controller.keys {
            if key.len() == 3 {
                key.insert(0, 0);
            }
        }
    }
    editor.outline = robot.chassis.bottom.clone();
    editor.top = robot.chassis.top.clone();
    editor.height = robot.chassis.height;
    editor.closed = editor.outline.len() >= 3;
    editor.document.load(robot);
    editor.source = Some(path.into());
    editor.selected = None;
    editor.preview_burst_end = false;
    editor.pending = None;
    editor.ui_revision += 1;
    Ok(())
}
pub fn attach(
    editor: &mut Editor,
    catalog: &Catalog,
    pick: &Pick,
    view: &View,
) -> Result<(), String> {
    let component = super::placement::candidate_on_view(editor, catalog, pick, view)?;
    super::placement::pose(editor, catalog, pick, view)?;
    editor.document.edit("Attach component", |robot| {
        robot.components.push(component);
        Ok(())
    })?;
    editor.selected = Some(editor.document.robot.components.len() - 1);
    editor.pending = None;
    editor.ui_revision += 1;
    Ok(())
}
fn armor(editor: &mut Editor, assets: &LegacyAssets, id: u32) -> Result<(), String> {
    let definition = assets
        .armors
        .iter()
        .find(|a| a.id == id)
        .ok_or("Unknown armor type")?;
    let density = definition.density;
    let hardness = definition.hardness;
    let stem = std::path::Path::new(&definition.image.replace('\\', "/"))
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .trim_start_matches("armor_")
        .to_string();
    let texture = assets.image(&format!("armor/{stem}_tex.bmp"));
    editor.document.edit("Chassis armor", |robot| {
        robot.chassis.armour = id;
        robot.chassis.surface_density = density;
        robot.chassis.hardness = hardness;
        robot.chassis.armour_flags = definition.flags;
        if let Some(texture) = texture.filter(|_| robot.default_paint != 0) {
            let mut pixels = vec![0; 256 * 256 * 4];
            for y in 0..256 {
                for x in 0..256 {
                    let src = ((y % texture.height) * texture.width + x % texture.width) * 4;
                    pixels[(y * 256 + x) * 4..(y * 256 + x) * 4 + 4]
                        .copy_from_slice(&texture.rgba[src..src + 4]);
                }
            }
            robot.chassis.texture = Some(paint::from_rgba(256, 256, &pixels)?);
        }
        Ok(())
    })
}
fn dialog_ok(
    editor: &mut Editor,
    ui: &legacy_ui::State,
    catalog: &Catalog,
    nav: &mut Navigation,
) -> Result<(), String> {
    match editor.dialog.clone() {
        Some(Dialog::Export) => {
            let name = ui
                .values
                .get("file name")
                .map(String::as_str)
                .unwrap_or(&editor.document.robot.name);
            let path = export_path(catalog, name)?;
            let source = editor.source.clone();
            let dirty = editor.document.dirty;
            save(editor, &path)?;
            // An exported copy does not save changes back into the team's slot.
            if editor.team_slot.is_some() {
                editor.source = source;
                editor.document.dirty = dirty;
            }
            editor.dialog = None;
            if editor.exit_after_save {
                nav.page = editor.exit_page;
                editor.exit_after_save = false;
            }
        }
        Some(Dialog::Import) => {
            let name = ui
                .values
                .get("import path")
                .ok_or("Select a robot design")?;
            let path = catalog.root.join("Robot Designs").join(name);
            let source = editor.source.clone();
            load(editor, &path)?;
            if editor.team_slot.is_some() {
                editor.source = source;
                editor.document.dirty = true;
            }
            editor.dialog = None;
        }
        Some(Dialog::Controller(kind)) => {
            let name = ui
                .values
                .get("control name")
                .cloned()
                .unwrap_or_else(|| "New Control".into());
            let position = editor.selected_control_slot;
            let positive = editor.key_positive;
            let negative = editor.key_negative;
            let native = editor.key_native.clone();
            editor.document.edit("Controller", |robot| {
                let controller = Controller {
                    name: format!("Name: {name}"),
                    position,
                    kind,
                    keys: if kind == 1 {
                        vec![
                            native[0].clone().unwrap_or_else(|| vec![0, 0, positive, 0]),
                            native[1].clone().unwrap_or_else(|| vec![0, 0, negative, 0]),
                        ]
                    } else {
                        vec![native[0].clone().unwrap_or_else(|| vec![0, 0, positive, 0])]
                    },
                };
                if let Some(old) = robot
                    .controllers
                    .iter_mut()
                    .find(|c| c.position == position)
                {
                    *old = controller;
                } else {
                    robot.controllers.push(controller);
                }
                Ok(())
            })?;
            editor.selected_controller = editor
                .document
                .robot
                .controllers
                .iter()
                .position(|c| c.position == position);
            editor.dialog = None;
        }
        Some(Dialog::Connection) => {
            let index = editor.selected.ok_or("Select a component")?;
            let controller = editor
                .document
                .robot
                .controllers
                .get(editor.selected_controller.ok_or("Select a control")?)
                .ok_or("Missing controller")?
                .clone();
            let choices = actions(&editor.document.robot.components[index].class);
            let maps = editor.binding_actions;
            editor.document.edit("Wire component", |robot| {
                robot
                    .bindings
                    .retain(|b| !(b.component == index && b.controller == controller.position));
                for key in 0..controller.keys.len() {
                    let action = choices[maps[key.min(1)] % choices.len()];
                    if action != "None" {
                        robot.bindings.push(ControlBinding {
                            component: index,
                            action: action.into(),
                            controller: controller.position,
                            key,
                        });
                    }
                }
                Ok(())
            })?;
            editor.dialog = None;
        }
        Some(Dialog::PaintImport) => {
            super::painting::exchange(editor, catalog, ui, true)?;
            editor.dialog = None;
        }
        Some(Dialog::PaintExport) => {
            super::painting::exchange(editor, catalog, ui, false)?;
            editor.dialog = None;
        }
        Some(Dialog::Message(_)) => editor.dialog = None,
        _ => editor.dialog = None,
    }
    Ok(())
}
#[allow(
    clippy::too_many_arguments,
    reason = "Dispatch source widget actions through app adapters"
)]
fn activate(
    editor: &mut Editor,
    name: &str,
    owner: &str,
    point: Vec2,
    catalog: &Catalog,
    assets: &LegacyAssets,
    ui: &mut legacy_ui::State,
    output: &Output,
    view: &mut View,
    nav: &mut Navigation,
    draft: &mut Draft,
    settings: &EngineSettings,
) -> Result<(), String> {
    if let Some(choice) = name.strip_prefix("choice:")
        && let Some((owner, index)) = choice.rsplit_once(':')
        && let Ok(index) = index.parse::<usize>()
    {
        match owner {
            "effect dropdown" => {
                editor.paint_effect = index.min(5);
                editor.paint_texture.clear();
            }
            "component option" => {
                if let Some((_, key)) = catalog.styles_for(&editor.component).get(index) {
                    select_component(editor, catalog, key, true);
                }
            }
            "input1 map" => editor.binding_actions[0] = index,
            "input2 map" => editor.binding_actions[1] = index,
            _ => (),
        }
        ui.popup = None;
        return Ok(());
    }
    let dropdown = if name == "Down Button" { owner } else { name };
    if ui.choices.contains_key(dropdown) {
        ui.popup = if ui.popup.as_deref() == Some(dropdown) {
            None
        } else {
            Some(dropdown.into())
        };
        return Ok(());
    }
    ui.popup = None;
    if let Some(key) = name.strip_prefix("texture:") {
        editor.paint_texture = key.into();
        return Ok(());
    }
    if name == "ok button" {
        return dialog_ok(editor, ui, catalog, nav);
    }
    if name == "cancel button" || name == "close button" {
        editor.dialog = None;
        editor.capture_key = None;
        editor.exit_after_save = false;
        return Ok(());
    }
    if name == "save button" {
        if editor.team_slot.is_some() {
            save_team_slot(editor)?;
            editor.dialog = None;
            if editor.exit_after_save {
                nav.page = editor.exit_page;
                editor.exit_after_save = false;
            }
            return Ok(());
        }
        editor.dialog = Some(Dialog::Export);
        ui.values
            .insert("file name".into(), editor.document.robot.name.clone());
        return Ok(());
    }
    if name == "discard button" {
        editor.document.dirty = false;
        editor.dialog = None;
        nav.page = editor.exit_page;
        return Ok(());
    }
    if name == "Options button" {
        nav.back = Page::Workshop;
        nav.page = Page::Settings;
        draft.0 = settings.clone();
        return Ok(());
    }
    if name == "back button"
        || name.eq_ignore_ascii_case("main menu button")
        || name.eq_ignore_ascii_case("MainMenu button")
    {
        let destination = if name == "back button" {
            editor.return_page
        } else {
            Page::Main
        };
        editor.exit_page = destination;
        if editor.document.dirty {
            editor.dialog = Some(Dialog::SavePrompt);
            editor.exit_after_save = true;
        } else {
            nav.page = destination;
        }
        return Ok(());
    }
    if matches!(
        name,
        "overview button"
            | "chassis button"
            | "components button"
            | "wiring button"
            | "paintshop button"
            | "structure design button"
            | "armor button"
            | "test robot button"
    ) {
        editor.preview_burst_end = false;
        editor.pending = None;
        editor.ghost_pick = None;
        editor.ghost_pose = None;
    }
    if let Some(key) = name.strip_prefix("component:") {
        select_component(editor, catalog, key, false);
        return Ok(());
    }
    if let Some(index) = name.strip_prefix("armor:").and_then(|s| s.parse().ok()) {
        return armor(editor, assets, index);
    }
    if let Some(slot) = name
        .strip_prefix("control:")
        .and_then(|s| s.parse::<i32>().ok())
    {
        let position = [slot % 4, slot / 4];
        if editor.selected_control_slot == position
            && let Some(c) = editor
                .document
                .robot
                .controllers
                .iter()
                .find(|c| c.position == position)
        {
            let c = c.clone();
            editor.key_native = [c.keys.first().cloned(), c.keys.get(1).cloned()];
            editor.key_positive = c
                .keys
                .first()
                .and_then(|k| k.get(k.len().saturating_sub(2)))
                .copied()
                .unwrap_or(17);
            editor.key_negative = c
                .keys
                .get(1)
                .and_then(|k| k.get(k.len().saturating_sub(2)))
                .copied()
                .unwrap_or(31);
            ui.values.insert(
                "control name".into(),
                c.name.trim_start_matches("Name: ").into(),
            );
            editor.dialog = Some(Dialog::Controller(c.kind));
        }
        editor.selected_control_slot = position;
        editor.selected_controller = editor
            .document
            .robot
            .controllers
            .iter()
            .position(|c| c.position == editor.selected_control_slot);
        return Ok(());
    }
    if let Some(file) = name.strip_prefix("file:") {
        ui.values.insert("import path".into(), file.into());
        return Ok(());
    }
    match name {
        "overview button" => editor.mode = Mode::Overview,
        "chassis button" => editor.mode = Mode::Chassis,
        "components button" => editor.mode = Mode::Components,
        "wiring button" => editor.mode = Mode::Wiring,
        "paintshop button" => editor.mode = Mode::Paint,
        "test robot button" => {
            if editor.document.robot.chassis.model.is_none() {
                return Err("Build a chassis before testing the robot".into());
            }
            editor.pending = None;
            editor.ghost_pick = None;
            editor.ghost_pose = None;
            editor.mode = Mode::Test;
        }
        "Help button" => editor.dialog = Some(Dialog::Help),
        "structure design button" => {
            if editor.document.robot.components.len() > 1 {
                return Err(
                    "Remove attached components before changing the chassis structure".into(),
                );
            }
            editor.mode = Mode::Base;
        }
        "armor button" => {
            if editor.document.robot.chassis.model.is_none() {
                return Err("Build a chassis first".into());
            }
            editor.mode = Mode::Armor;
        }
        "default appearance checkbox" => {
            editor.document.edit("Default appearance", |r| {
                r.default_paint = 1 - r.default_paint;
                Ok(())
            })?;
        }
        "adjustments tab" => editor.properties_tab = 0,
        "appearance tab" => editor.properties_tab = 1,
        "info tab" => editor.properties_tab = 2,
        "texture or template cb" => {
            editor.paint_export_current = !editor.paint_export_current;
        }
        "color picker img" => {
            if let (Some(r), Some(image)) = (
                output.regions.get(name),
                assets.image("botlab/colorpicker.bmp"),
            ) {
                let uv = ((point - r.min) / r.size()).clamp(Vec2::ZERO, Vec2::splat(0.999));
                let at = ((uv.y * image.height as f32) as usize * image.width
                    + (uv.x * image.width as f32) as usize)
                    * 4;
                editor.hue_color.copy_from_slice(&image.rgba[at..at + 4]);
                editor.color = editor.hue_color;
                editor.luminance = 0.5;
            }
        }
        "texture entire" | "edge entire" => {
            super::painting::apply(editor, catalog, assets, None, None)?
        }
        "supershine on" | "supershine off" => {
            let shine = name == "supershine on";
            editor.document.edit("Supershine", |r| {
                r.chassis.material_parameters[3] = if shine { 1.0 } else { 0.85 };
                r.chassis.material_parameters[4] = if shine { 100.0 } else { 20.0 };
                Ok(())
            })?;
        }

        "clear outline button" => {
            editor.outline.clear();
            editor.top.clear();
            editor.closed = false;
        }
        "freeform checkbox" => {
            if editor.draw_style != 0 {
                editor.outline.clear();
                editor.top.clear();
                editor.closed = false;
            }
            editor.draw_style = 0;
        }
        "circle checkbox" => {
            editor.draw_style = 1;
            editor.outline = chassis::circle(editor.radius);
            editor.closed = true;
        }
        "rectangle checkbox" => {
            editor.draw_style = 2;
            editor.outline = chassis::rectangle(editor.length, editor.width);
            editor.closed = true;
        }
        "step 1 button" => editor.mode = Mode::Base,
        "step 2 button" => {
            chassis::validate_outline(&editor.outline)?;
            editor.closed = true;
            if editor.top.len() != editor.outline.len() {
                editor.top = editor.outline.clone();
            }
            rebuild(editor)?;
            editor.mode = Mode::Top;
        }
        "finished button" => {
            if matches!(editor.mode, Mode::Base | Mode::Top) {
                editor.closed = true;
                rebuild(editor)?;
            }
            editor.mode = Mode::Chassis;
        }
        "snap checkbox" => editor.snap = !editor.snap,
        "power button" => editor.category = "power".into(),
        "extenders button" => editor.category = "extenders".into(),
        "mechanics button" => editor.category = "mechanics".into(),
        "mobility button" => editor.category = "mobility".into(),
        "weapons button" => editor.category = "weapons".into(),
        "extras button" => editor.category = "extras".into(),
        "attach button" => {
            if editor.document.robot.chassis.model.is_none() {
                return Err("Build a chassis before attaching components".into());
            }
            editor.pending = Some(editor.component.clone());
            editor.ghost_model = None;
        }
        "remove button" => {
            editor
                .document
                .remove_component(editor.selected.ok_or("Select a component")?)?;
            editor.selected = None;
        }
        "snapshot button" => editor.snapshot_requested = true,
        "new analog" | "new switch" | "new button" => {
            let kind = match name {
                "new analog" => 1,
                "new switch" => 2,
                _ => 4,
            };
            editor.controller_kind = kind;
            editor.key_native = [None, None];
            ui.values.insert(
                "control name".into(),
                format!(
                    "New {}",
                    match kind {
                        1 => "Analog",
                        2 => "Switch",
                        _ => "Button",
                    }
                ),
            );
            editor.dialog = Some(Dialog::Controller(kind));
        }
        "delete button" if editor.mode == Mode::Wiring => {
            let position = editor.selected_control_slot;
            editor.document.edit("Delete controller", |r| {
                r.controllers.retain(|c| c.position != position);
                r.bindings.retain(|b| b.controller != position);
                Ok(())
            })?;
            editor.selected_controller = None;
        }
        "input1" => editor.capture_key = Some(0),
        "input2" => editor.capture_key = Some(1),
        "input1 map" | "Down Button" if name == "input1 map" || owner == "input1 map" => {
            editor.binding_actions[0] += 1
        }
        "input2 map" | "Down Button" if name == "input2 map" || owner == "input2 map" => {
            editor.binding_actions[1] += 1
        }
        "paint entire" => {
            let color = editor.color;
            editor.document.edit("Paint chassis", |r| {
                paint::fill(&mut r.chassis, None, color)
            })?;
        }
        "undo button" => {
            editor.document.undo();
            editor.outline = editor.document.robot.chassis.bottom.clone();
            editor.top = editor.document.robot.chassis.top.clone();
            editor.height = editor.document.robot.chassis.height;
            editor.closed = editor.outline.len() >= 3 && editor.mode != Mode::Base;
        }
        "export button" => {
            editor.dialog = Some(Dialog::PaintExport);
            ui.values.insert(
                "export name textbox".into(),
                editor.document.robot.name.clone(),
            );
        }
        "import button" => editor.dialog = Some(Dialog::PaintImport),
        "Down Button" if owner == "effect dropdown" => {
            editor.paint_effect = (editor.paint_effect + 1) % super::panels::EFFECTS.len()
        }
        "component option" | "Down Button"
            if owner == "component option" || name == "component option" =>
        {
            if let Some(asset) = catalog.components.get(&editor.component) {
                let styles = asset.styles();
                if styles.len() > 1 {
                    select_component(editor, catalog, &styles[1].1, true);
                }
            }
        }
        "Editing Area" if editor.mode == Mode::Base => {
            let rect = output
                .regions
                .get("Editing Area")
                .ok_or("Blueprint not ready")?;
            let mut p =
                ((point - rect.min) / rect.size() * 1000.0).clamp(Vec2::ZERO, Vec2::splat(1000.0));
            if editor.snap {
                p = (p / 50.0).round() * 50.0;
            }
            if editor.outline.len() >= 3 && p.distance(editor.outline[0]) < 60.0 {
                chassis::validate_outline(&editor.outline)?;
                editor.closed = true;
            } else if !editor.closed && editor.outline.len() < chassis::MAX_POINTS {
                if editor.outline.iter().any(|v| v.distance(p) < 25.0) {
                    return Ok(());
                }
                editor.outline.push(p.round());
            }
        }
        "Less" | "Down" => {
            let old = ui.scroll.get(owner).copied().unwrap_or(0);
            ui.scroll.insert(
                owner.into(),
                if name == "Less" {
                    old.saturating_sub(1)
                } else {
                    old + 1
                },
            );
        }
        "component 3d preview" => {
            if let Some(rect) = output.regions.get(name)
                && let Some(id) = view.preview_socket(point, *rect, catalog, &editor.component)
            {
                editor.selected_socket = id;
                if editor.ghost_pick.is_some() {
                    super::placement::update_preview(editor, catalog, view);
                } else {
                    editor.ghost_valid = false;
                }
            }
        }
        name if name.contains("3d") => {
            if let Some(rect) = output.regions.get(name)
                && let Some(pick) = if editor.pending.is_some() {
                    view.placement_pick(
                        point,
                        *rect,
                        super::placement::pending_base_mount(editor, catalog),
                    )
                } else {
                    view.pick(point, *rect, true)
                        .or_else(|| view.pick(point, *rect, false))
                }
            {
                if editor.pending.is_some() {
                    let placement = editor.ghost_pick.clone().unwrap_or(pick);
                    attach(editor, catalog, &placement, view)?;
                } else if editor.mode == Mode::Wiring && pick.component > 0 {
                    begin_connection(editor, pick.component)?;
                } else if editor.mode == Mode::Paint && pick.component == 0 {
                    let panel = editor
                        .document
                        .robot
                        .chassis
                        .paint_layout
                        .face_groups
                        .get(pick.face)
                        .map(|g| g[0] as usize);
                    super::painting::apply(editor, catalog, assets, panel, Some(pick.uv))?;
                } else {
                    if editor.selected != Some(pick.component) {
                        editor.preview_burst_end = false;
                    }
                    editor.selected = Some(pick.component);
                }
            }
        }
        _ => (),
    }
    Ok(())
}
fn value(editor: &mut Editor, name: &str, text: &str) -> Result<(), String> {
    let scalar = text.parse::<f32>().unwrap_or(0.0).clamp(0.0, 1.0);
    match name {
        "bot name textbox" => {
            let name = text.to_string();
            editor.document.edit("Robot name", |r| {
                r.name = name;
                Ok(())
            })?;
        }
        "scale slider" => editor.decal_scale = scalar,
        "lum slider" => {
            editor.luminance = scalar;
            for i in 0..3 {
                let base = editor.hue_color[i] as f32;
                editor.color[i] = if scalar <= 0.5 {
                    base * scalar * 2.0
                } else {
                    base + (255.0 - base) * (scalar - 0.5) * 2.0
                }
                .round() as u8;
            }
        }
        "zone name" => {
            let index = editor.selected.ok_or("Select a zone")?;
            editor.document.edit("Zone name", |r| {
                r.components[index]
                    .settings
                    .insert("zone".into(), text.into());
                Ok(())
            })?;
        }
        "height slider" => {
            editor.height = 0.05 + scalar * 0.95;
            if editor.closed {
                rebuild(editor)?;
            }
        }
        "heading slider" => {
            editor.document.edit("Heading", |r| {
                r.heading = super::heading::from_slider(scalar);
                Ok(())
            })?;
        }
        "radius slider" => {
            editor.radius = 25.0 + scalar * 475.0;
            if editor.draw_style == 1 {
                editor.outline = chassis::circle(editor.radius);
                editor.closed = true;
            }
        }
        "length slider" => {
            editor.length = 50.0 + scalar * 950.0;
            if editor.draw_style == 2 {
                editor.outline = chassis::rectangle(editor.length, editor.width);
                editor.closed = true;
            }
        }
        "width slider" => {
            editor.width = 50.0 + scalar * 950.0;
            if editor.draw_style == 2 {
                editor.outline = chassis::rectangle(editor.length, editor.width);
                editor.closed = true;
            }
        }
        "start angle slider" | "end angle slider" => {
            let index = editor.selected.ok_or("Select a burst motor")?;
            let key = if name == "start angle slider" {
                "start_angle"
            } else {
                "stop_angle"
            };
            editor.preview_burst_end = key == "stop_angle";
            let angle = if editor.preview_burst_end {
                (scalar - 0.5) * 4.8
            } else {
                scalar * std::f32::consts::TAU
            };
            editor.document.edit("Burst motor range", |r| {
                r.components[index]
                    .settings
                    .insert(key.into(), angle.to_string());
                Ok(())
            })?;
        }
        _ => (),
    }
    Ok(())
}
pub(crate) fn dik(key: KeyCode) -> Option<i32> {
    Some(match key {
        KeyCode::KeyA => 30,
        KeyCode::KeyB => 48,
        KeyCode::KeyC => 46,
        KeyCode::KeyD => 32,
        KeyCode::KeyE => 18,
        KeyCode::KeyF => 33,
        KeyCode::KeyG => 34,
        KeyCode::KeyH => 35,
        KeyCode::KeyI => 23,
        KeyCode::KeyJ => 36,
        KeyCode::KeyK => 37,
        KeyCode::KeyL => 38,
        KeyCode::KeyM => 50,
        KeyCode::KeyN => 49,
        KeyCode::KeyO => 24,
        KeyCode::KeyP => 25,
        KeyCode::KeyQ => 16,
        KeyCode::KeyR => 19,
        KeyCode::KeyS => 31,
        KeyCode::KeyT => 20,
        KeyCode::KeyU => 22,
        KeyCode::KeyV => 47,
        KeyCode::KeyW => 17,
        KeyCode::KeyX => 45,
        KeyCode::KeyY => 21,
        KeyCode::KeyZ => 44,
        KeyCode::Enter => 28,
        KeyCode::Tab => 15,
        KeyCode::Backspace => 14,
        KeyCode::Delete => 211,
        KeyCode::Insert => 210,
        KeyCode::Home => 199,
        KeyCode::End => 207,
        KeyCode::PageUp => 201,
        KeyCode::PageDown => 209,
        KeyCode::ShiftLeft => 42,
        KeyCode::ShiftRight => 54,
        KeyCode::ControlLeft => 29,
        KeyCode::ControlRight => 157,
        KeyCode::AltLeft => 56,
        KeyCode::AltRight => 184,
        KeyCode::Minus => 12,
        KeyCode::Equal => 13,
        KeyCode::BracketLeft => 26,
        KeyCode::BracketRight => 27,
        KeyCode::Semicolon => 39,
        KeyCode::Quote => 40,
        KeyCode::Backquote => 41,
        KeyCode::Backslash => 43,
        KeyCode::Comma => 51,
        KeyCode::Period => 52,
        KeyCode::Slash => 53,
        KeyCode::F1 => 59,
        KeyCode::F2 => 60,
        KeyCode::F3 => 61,
        KeyCode::F4 => 62,
        KeyCode::F5 => 63,
        KeyCode::F6 => 64,
        KeyCode::F7 => 65,
        KeyCode::F8 => 66,
        KeyCode::F9 => 67,
        KeyCode::F10 => 68,
        KeyCode::F11 => 87,
        KeyCode::F12 => 88,
        KeyCode::Space => 57,
        KeyCode::ArrowUp => 200,
        KeyCode::ArrowDown => 208,
        KeyCode::ArrowLeft => 203,
        KeyCode::ArrowRight => 205,
        KeyCode::Digit1 => 2,
        KeyCode::Digit2 => 3,
        KeyCode::Digit3 => 4,
        KeyCode::Digit4 => 5,
        KeyCode::Digit5 => 6,
        KeyCode::Digit6 => 7,
        KeyCode::Digit7 => 8,
        KeyCode::Digit8 => 9,
        KeyCode::Digit9 => 10,
        KeyCode::Digit0 => 11,
        _ => return None,
    })
}
#[allow(clippy::too_many_arguments, reason = "Workshop interaction adapters")]
pub fn input(
    mut editor: ResMut<Editor>,
    mut events: ResMut<Events>,
    mut ui: ResMut<legacy_ui::State>,
    output: Res<Output>,
    mut view: ResMut<View>,
    catalog: Res<Catalog>,
    assets: Res<LegacyAssets>,
    mut nav: ResMut<Navigation>,
    mut draft: ResMut<Draft>,
    settings: Res<EngineSettings>,
    keys: Res<ButtonInput<KeyCode>>,
    mut sounds: ResMut<crate::ui_sound::Sounds>,
) {
    if nav.page != Page::Workshop {
        return;
    }
    if let Some(index) = editor.capture_key {
        for key in keys.get_just_pressed() {
            if let Some(code) = dik(*key) {
                if index == 0 {
                    editor.key_positive = code;
                } else {
                    editor.key_negative = code;
                }
                editor.key_native[index] = None;
                editor.capture_key = None;
                editor.ui_revision += 1;
                break;
            }
        }
    } else {
        let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
        if ctrl && keys.just_pressed(KeyCode::KeyZ) {
            editor.document.undo();
            editor.outline = editor.document.robot.chassis.bottom.clone();
            editor.top = editor.document.robot.chassis.top.clone();
            editor.height = editor.document.robot.chassis.height;
            editor.closed = editor.outline.len() >= 3 && editor.mode != Mode::Base;
        }
        if ctrl && keys.just_pressed(KeyCode::KeyY) {
            editor.document.redo();
            editor.outline = editor.document.robot.chassis.bottom.clone();
            editor.top = editor.document.robot.chassis.top.clone();
            editor.height = editor.document.robot.chassis.height;
            editor.closed = editor.outline.len() >= 3 && editor.mode != Mode::Base;
        }
        if ctrl && keys.just_pressed(KeyCode::KeyS) && editor.team_slot.is_some() {
            if let Err(message) = save_team_slot(&mut editor) {
                editor.dialog = Some(Dialog::Message(message));
            }
            editor.ui_revision += 1;
        } else if ctrl && keys.just_pressed(KeyCode::KeyS) {
            editor.dialog = Some(Dialog::Export);
            let name = editor.document.robot.name.clone();
            ui.values.insert("file name".into(), name);
            editor.ui_revision += 1;
        }
        if ctrl && keys.just_pressed(KeyCode::KeyO) {
            editor.dialog = Some(Dialog::Import);
            editor.ui_revision += 1;
        }
        if keys.just_pressed(KeyCode::Escape) {
            if editor.mode == Mode::Test && editor.dialog.is_none() {
                editor.mode = Mode::Components;
            }
            editor.pending = None;
            editor.dialog = None;
            ui.focused.clear();
            ui.popup = None;
            editor.ui_revision += 1;
        }
    }
    for event in std::mem::take(&mut events.0) {
        let click = matches!(&event, UiEvent::Activate { .. });
        let old_parts = editor.document.robot.components.len();
        let old_points = editor.outline.len();
        let old_snapshot = editor.snapshot_requested;
        let dragging = matches!(&event, UiEvent::Drag { .. });
        let result = match event {
            UiEvent::Activate { name, owner, point } => activate(
                &mut editor,
                &name,
                &owner,
                point,
                &catalog,
                &assets,
                &mut ui,
                &output,
                &mut view,
                &mut nav,
                &mut draft,
                &settings,
            ),
            UiEvent::Value { name, value: text } => value(&mut editor, &name, &text),
            UiEvent::Move { name, point, delta } => {
                if editor.pending.is_some() && name.contains("3d") && name != "component 3d preview"
                {
                    let shift =
                        keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
                    let ctrl =
                        keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
                    if shift {
                        editor.attach_angle += delta.x * 0.02;
                    } else if ctrl {
                        editor.attach_offset =
                            (editor.attach_offset - delta.y * 0.005).clamp(-1.0, 1.5);
                    } else if let Some(rect) = output.regions.get(&name) {
                        editor.ghost_pick = view
                            .placement_pick(
                                point,
                                *rect,
                                super::placement::pending_base_mount(&editor, &catalog),
                            )
                            .or_else(|| Some(view.floor_pick(point, *rect)));
                    }
                    super::placement::update_preview(&mut editor, &catalog, &view);
                }
                continue;
            }
            UiEvent::Wheel { name, delta } => {
                if name.contains("3d") {
                    view.distance = (view.distance - delta * 0.2).clamp(0.7, 12.0);
                }
                Ok(())
            }
            UiEvent::Press {
                name,
                button: MouseButton::Right,
                ..
            } if name == "Editing Area" && editor.mode == Mode::Base => {
                if editor.closed {
                    editor.closed = false;
                } else {
                    editor.outline.pop();
                }
                editor.top = editor.outline.clone();
                Ok(())
            }
            UiEvent::Press {
                name,
                point,
                button: MouseButton::Left,
            } if name == "Editing Area" && matches!(editor.mode, Mode::Base | Mode::Top) => {
                if let Some(r) = output.regions.get(&name) {
                    let p = (point - r.min) / r.size() * 1000.0;
                    let points = if editor.mode == Mode::Top {
                        &editor.top
                    } else {
                        &editor.outline
                    };
                    editor.active_point = points
                        .iter()
                        .enumerate()
                        .filter(|(_, v)| v.distance(p) < 65.0)
                        .min_by(|a, b| a.1.distance_squared(p).total_cmp(&b.1.distance_squared(p)))
                        .map(|(i, _)| i);
                }
                Ok(())
            }
            UiEvent::Drag {
                name,
                point,
                delta,
                button,
            } => {
                if name == "component 3d preview" {
                    if button == MouseButton::Right {
                        view.preview_yaw -= delta.x * 0.015;
                        view.preview_pitch =
                            (view.preview_pitch + delta.y * 0.015).clamp(-1.3, 1.3);
                    }
                    Ok(())
                } else if name.contains("3d") {
                    match button {
                        MouseButton::Right => {
                            view.yaw -= delta.x * 0.015;
                            view.pitch = (view.pitch + delta.y * 0.015).clamp(-1.3, 1.3);
                        }
                        MouseButton::Middle => {
                            view.distance = (view.distance + delta.y * 0.02).clamp(0.7, 12.0)
                        }
                        _ => (),
                    }
                    Ok(())
                } else if name == "Editing Area" && matches!(editor.mode, Mode::Base | Mode::Top) {
                    if let (Some(index), Some(rect)) =
                        (editor.active_point, output.regions.get(&name))
                    {
                        let mut p = ((point - rect.min) / rect.size() * 1000.0)
                            .clamp(Vec2::ZERO, Vec2::splat(1000.0));
                        if editor.snap {
                            p = (p / 50.0).round() * 50.0;
                        }
                        if editor.mode == Mode::Top {
                            let previous = editor.top[index];
                            editor.top[index] = p;
                            if rebuild(&mut editor).is_err() {
                                editor.top[index] = previous;
                            }
                        } else {
                            let mut outline = editor.outline.clone();
                            outline[index] = p;
                            if !editor.closed || chassis::validate_outline(&outline).is_ok() {
                                editor.outline = outline;
                                editor.top = editor.outline.clone();
                            }
                        }
                        Ok(())
                    } else {
                        Ok(())
                    }
                } else {
                    Ok(())
                }
            }
            UiEvent::Release {
                name,
                point,
                button,
            } => {
                if button == MouseButton::Left
                    && name.starts_with("control:")
                    && let Some(target) = output.hits.iter().rev().find(|h| {
                        h.name.starts_with("control:")
                            && h.rect.contains(point)
                            && h.clip.contains(point)
                    })
                    && target.name != name
                    && let (Some(from), Some(to)) = (
                        name.strip_prefix("control:")
                            .and_then(|s| s.parse::<i32>().ok()),
                        target
                            .name
                            .strip_prefix("control:")
                            .and_then(|s| s.parse::<i32>().ok()),
                    )
                {
                    let a = [from % 4, from / 4];
                    let b = [to % 4, to / 4];
                    let _ = editor.document.edit("Move controller", |r| {
                        for c in &mut r.controllers {
                            if c.position == a {
                                c.position = b;
                            } else if c.position == b {
                                c.position = a;
                            }
                        }
                        for binding in &mut r.bindings {
                            if binding.controller == a {
                                binding.controller = b;
                            } else if binding.controller == b {
                                binding.controller = a;
                            }
                        }
                        Ok(())
                    });
                }

                editor.active_point = None;
                Ok(())
            }
            _ => Ok(()),
        };
        if result.is_ok() {
            if editor.mode == Mode::Base && editor.document.robot.chassis.bottom != editor.outline {
                editor.document.dirty = true;
                if !dragging {
                    let bottom = editor.outline.clone();
                    editor.top = bottom.clone();
                    let _ = editor.document.edit("Baseplate outline", |r| {
                        r.chassis.bottom = bottom.clone();
                        r.chassis.top = bottom;
                        Ok(())
                    });
                }
            }
            if editor.document.robot.components.len() > old_parts {
                if let Some(part) = editor.document.robot.components.last()
                    && let Some(asset) = catalog
                        .components
                        .get(&ra2_formats::components::normalize(&part.definition))
                {
                    let cue = asset.definition.value("attachsound");
                    sounds.play(if cue.is_empty() {
                        "Sounds/cmp_generic.wav"
                    } else {
                        cue
                    });
                }
            } else if editor.document.robot.components.len() < old_parts {
                sounds.play("Sounds/cmp_removal.wav");
            } else if editor.outline.len() > old_points {
                sounds.play("Sounds/point_plotted.wav");
            } else if !old_snapshot && editor.snapshot_requested {
                sounds.play("Sounds/snapshot.wav");
            } else if click {
                sounds.play("Sounds/button_click.wav");
            }
        }
        if let Err(message) = result {
            warn!("Workshop: {message}");
            editor.dialog = Some(Dialog::Message(message));
        }
        editor.ui_revision += 1;
    }
}

/// Native keyboard slots are shared by the garage and Wiring action preview.
pub fn samples(
    robot: &arena_core::robot::RobotDefinition,
    keys: &ButtonInput<KeyCode>,
    enabled: bool,
) -> Vec<Vec<f32>> {
    robot
        .controllers
        .iter()
        .map(|c| {
            c.keys
                .iter()
                .map(|native| {
                    let code = native
                        .get(native.len().saturating_sub(2))
                        .copied()
                        .unwrap_or(-1);
                    f32::from(
                        enabled
                            && native.first() == Some(&0)
                            && keys.get_pressed().any(|k| dik(*k) == Some(code)),
                    )
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod slot_tests {
    use super::*;
    #[test]
    fn forward_heading_slider_roundtrips_and_supports_undo_at_full_turn() {
        let mut editor = Editor::default();
        editor.outline = chassis::rectangle(600., 600.);
        editor.top = editor.outline.clone();
        rebuild(&mut editor).unwrap();
        for (ratio, step) in [
            ("0", 0.),
            ("0.26", 5.),
            ("0.5", 10.),
            ("0.75", 15.),
            ("1", 20.),
        ] {
            value(&mut editor, "heading slider", ratio).unwrap();
            let expected = step * 0.05 * std::f32::consts::TAU;
            assert_eq!(editor.document.robot.heading, expected);
            let saved = ra2_formats::bot::encode(&editor.document.robot).unwrap();
            let loaded = ra2_formats::bot::decode(&saved).unwrap();
            assert!((loaded.heading - expected).abs() < 0.00001);
        }
        assert_eq!(
            super::super::heading::slider_value(editor.document.robot.heading),
            1.
        );
        assert!(editor.document.undo());
        assert!((editor.document.robot.heading - std::f32::consts::TAU * 0.75).abs() < 0.00001);
        assert!(editor.document.redo());
        assert_eq!(
            super::super::heading::slider_value(editor.document.robot.heading),
            1.
        );
    }
    fn bindings(editor: &Editor) -> Vec<(usize, [i32; 2], usize, String)> {
        let mut result: Vec<_> = editor
            .document
            .robot
            .bindings
            .iter()
            .map(|b| (b.component, b.controller, b.key, b.action.clone()))
            .collect();
        result.sort();
        result
    }
    #[test]
    fn wiring_survives_reopening_confirmation_and_bot_reload_for_all_control_types() {
        for kind in [1, 2, 4] {
            let mut editor = Editor::default();
            editor.outline = chassis::rectangle(600., 600.);
            editor.top = editor.outline.clone();
            rebuild(&mut editor).unwrap();
            for _ in 0..2 {
                editor.document.robot.components.push(RobotComponent {
                    parent: Some(0),
                    class: "SpinMotor".into(),
                    definition: "Components/SpinMotor.txt".into(),
                    parent_mount: 0,
                    own_mount: 0,
                    mounting: Default::default(),
                    settings: Default::default(),
                });
            }
            for position in [[0, 0], [1, 0]] {
                editor.document.robot.controllers.push(Controller {
                    name: "Name: Test".into(),
                    position,
                    kind,
                    keys: if kind == 1 {
                        vec![vec![0, 0, 17, 0], vec![0, 0, 31, 0]]
                    } else {
                        vec![vec![0, 0, 17, 0]]
                    },
                });
            }
            // Other motors and controllers must keep their existing assignments.
            for (component, controller) in [(2, [0, 0]), (1, [1, 0])] {
                editor.document.robot.bindings.push(ControlBinding {
                    component,
                    controller,
                    key: 0,
                    action: "Spin Clockwise".into(),
                });
            }
            editor.selected_controller = Some(0);
            begin_connection(&mut editor, 1).unwrap();
            assert_eq!(editor.binding_actions, [0, 0]);
            let expected = if kind == 1 { [1, 2] } else { [2, 0] };
            editor.binding_actions = expected;
            let mut nav = Navigation {
                page: Page::Workshop,
                back: Page::TeamHQ,
            };
            let ui = legacy_ui::State::default();
            let catalog = Catalog::default();
            dialog_ok(&mut editor, &ui, &catalog, &mut nav).unwrap();
            let saved = bindings(&editor);
            editor.document.dirty = false;
            let revision = editor.document.revision;
            begin_connection(&mut editor, 1).unwrap();
            assert_eq!(
                editor.binding_actions, expected,
                "Reopening reset a saved mapping for controller kind {kind}"
            );
            assert!(!editor.document.dirty);
            assert_eq!(editor.document.revision, revision);
            dialog_ok(&mut editor, &ui, &catalog, &mut nav).unwrap();
            assert_eq!(
                bindings(&editor),
                saved,
                "Confirming the unchanged dialog deleted existing wiring"
            );
            let path =
                std::env::temp_dir().join(format!("ra2-wiring-{}-{kind}.bot", std::process::id()));
            save(&mut editor, &path).unwrap();
            load(&mut editor, &path).unwrap();
            begin_connection(&mut editor, 1).unwrap();
            assert_eq!(editor.binding_actions, expected);
            assert_eq!(bindings(&editor), saved);
            editor.binding_actions[0] = 0;
            dialog_ok(&mut editor, &ui, &catalog, &mut nav).unwrap();
            assert!(
                editor
                    .document
                    .robot
                    .bindings
                    .iter()
                    .all(|b| !(b.component == 1 && b.controller == [0, 0] && b.key == 0))
            );
            assert_eq!(
                editor
                    .document
                    .robot
                    .bindings
                    .iter()
                    .filter(|b| b.component == 2 || b.controller == [1, 0])
                    .count(),
                2
            );
            std::fs::remove_file(path).unwrap();
        }
    }
    #[test]
    fn exporting_and_importing_designs_keep_team_slot_save_state() {
        let root =
            std::env::temp_dir().join(format!("ra2-editor-slot-copy-{}", std::process::id()));
        let mut editor = Editor::default();
        editor.outline = chassis::rectangle(600., 600.);
        editor.top = editor.outline.clone();
        editor.document.robot.name = "Native slot".into();
        rebuild(&mut editor).unwrap();
        let native = crate::team_hq::robot_path(&root, 0, 0);
        save(&mut editor, &native).unwrap();
        let original = std::fs::read(&native).unwrap();
        editor.team_slot = Some((0, 0));
        editor
            .document
            .edit("Rename", |r| {
                r.name = "Exported copy".into();
                Ok(())
            })
            .unwrap();
        editor.dialog = Some(Dialog::Export);
        let catalog = Catalog {
            root: root.clone(),
            ..Default::default()
        };
        let mut ui = legacy_ui::State::default();
        ui.values.insert("file name".into(), "copy".into());
        let mut nav = Navigation {
            page: Page::Workshop,
            back: Page::TeamHQ,
        };
        dialog_ok(&mut editor, &ui, &catalog, &mut nav).unwrap();
        assert!(editor.document.dirty, "Export marked the native slot saved");
        assert_eq!(editor.source, Some(native.clone()));
        assert_eq!(std::fs::read(&native).unwrap(), original);
        editor.document.dirty = false;
        editor.dialog = Some(Dialog::Import);
        ui.values.insert("import path".into(), "copy.bot".into());
        dialog_ok(&mut editor, &ui, &catalog, &mut nav).unwrap();
        assert!(
            editor.document.dirty,
            "Imported design silently replaced a team document without a save prompt"
        );
        assert_eq!(editor.team_slot, Some((0, 0)));
        assert_eq!(editor.source, Some(native.clone()));
        assert_eq!(editor.document.robot.name, "Exported copy");
        assert_eq!(std::fs::read(&native).unwrap(), original);
        std::fs::remove_dir_all(root).unwrap();
    }
}
