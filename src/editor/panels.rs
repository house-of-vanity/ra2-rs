//! Bind an editable document to the original widget trees.
use super::{Dialog, Editor, Mode, catalog::Catalog, controls};
use crate::legacy_ui::{
    Cell, State,
    assets::{LegacyAssets, decode_image},
    canvas::Pixels,
};
use ra2_formats::gib::Widget;
use std::sync::Arc;

pub const EFFECTS: [&str; 6] = [
    "Color Fill",
    "Texture Fill",
    "Surface Layer",
    "Edge Lines",
    "Decals",
    "Supershine",
];
pub const EFFECT_FILES: [&str; 6] = [
    "colorfill",
    "texturefill",
    "surfacelayer",
    "edgelines",
    "decals",
    "supershine",
];
pub fn patch(w: &mut Widget, name: &str, key: &str, value: &str) {
    if w.name() == name {
        w.fields.insert(key.into(), vec![value.into()]);
    }
    if let Some(inner) = &mut w.wrapped {
        patch(inner, name, key, value);
    }
    for child in &mut w.children {
        patch(child, name, key, value);
    }
}

fn rename_exact(w: &mut Widget, from: &str, to: &str) {
    if w.name() == from {
        w.fields.insert("Name".into(), vec![to.into()]);
    }
    if let Some(inner) = &mut w.wrapped {
        rename_exact(inner, from, to);
    }
    for child in &mut w.children {
        rename_exact(child, from, to);
    }
}

pub(crate) fn text(w: &mut Widget, name: &str, value: &str) {
    if w.name() == name {
        let value = value.to_string();
        fn descend(w: &mut Widget, value: &str) {
            if w.kind == "Text" {
                w.fields.insert("_Text".into(), vec![value.into()]);
            }
            if let Some(i) = &mut w.wrapped {
                descend(i, value);
            }
            for c in &mut w.children {
                descend(c, value);
            }
        }
        descend(w, &value);
        return;
    }
    if let Some(i) = &mut w.wrapped {
        text(i, name, value);
    }
    for c in &mut w.children {
        text(c, name, value);
    }
}
pub fn key_name(code: i32) -> String {
    match code {
        17 => "W".into(),
        31 => "S".into(),
        30 => "A".into(),
        32 => "D".into(),
        57 => "Space".into(),
        200 => "Up".into(),
        208 => "Down".into(),
        203 => "Left".into(),
        205 => "Right".into(),
        _ => format!("Key {code}"),
    }
}
fn slot(ui: &mut State, assets: &LegacyAssets, name: &str, file: &str) {
    if let Ok(w) = assets.widget(&format!("botlab/{file}.gib")) {
        ui.slots.insert(name.into(), w);
    }
}
pub fn sync(editor: &Editor, catalog: &Catalog, assets: &mut LegacyAssets, ui: &mut State) {
    ui.checked
        .insert("texture or template cb".into(), editor.paint_export_current);
    ui.visibility.clear();
    ui.interactive.clear();
    ui.images.clear();
    ui.colors.clear();
    if editor.document.robot.snapshot.is_none() {
        for name in ["snapshot img", "export snapshot"] {
            ui.colors.insert(name.into(), [0.12, 0.13, 0.13, 1.0]);
        }
    }
    ui.choices.clear();
    ui.values.insert(
        "radius slider".into(),
        ((editor.radius - 25.0) / 475.0).to_string(),
    );
    ui.values.insert(
        "length slider".into(),
        ((editor.length - 50.0) / 950.0).to_string(),
    );
    ui.values.insert(
        "width slider".into(),
        ((editor.width - 50.0) / 950.0).to_string(),
    );
    ui.values.insert(
        "height slider".into(),
        ((editor.height - 0.05) / 0.95).to_string(),
    );
    ui.checked.insert(
        "default appearance checkbox".into(),
        editor.document.robot.default_paint != 0,
    );
    if let Some(bytes) = &editor.document.robot.snapshot
        && let Ok((w, h, rgba)) = arena_core::editor::paint::rgba(bytes)
    {
        assets.images.insert(
            "@snapshot".into(),
            Pixels {
                width: w,
                height: h,
                rgba,
            },
        );
        for name in ["snapshot img", "export snapshot"] {
            ui.images.insert(name.into(), "@snapshot".into());
        }
    }
    if editor.mode == Mode::Armor {
        let mut cells = vec![];
        for definition in &assets.armors {
            let id = definition.id;
            if let Ok(base) = assets.widget("botlab/armor_cell.gib") {
                let mut template = (*base).clone();
                patch(&mut template, "armor img", "_Filename", &definition.image);
                patch(
                    &mut template,
                    "description",
                    "Display",
                    &format!(
                        "<text font fg_mcn14 color .8 .8 .8 1>{}</text>",
                        definition.description
                    ),
                );
                text(&mut template, "armor name", &definition.name);
                text(
                    &mut template,
                    "weight value",
                    &format!("{}", definition.density),
                );
                text(
                    &mut template,
                    "concussion value",
                    &format!("{}", definition.hardness),
                );
                cells.push(Cell {
                    key: format!("armor:{id}"),
                    selected: editor.document.robot.chassis.armour == id,
                    template: Some(Arc::new(template)),
                    ..Default::default()
                });
            }
        }
        ui.cells.insert("armor grid".into(), cells);
    }
    if editor.mode == Mode::Components {
        if let Some(index) = editor.selected.filter(|i| *i > 0)
            && let Some(part) = editor.document.robot.components.get(index)
            && let Some(asset) = catalog
                .components
                .get(&ra2_formats::components::normalize(&part.definition))
        {
            let file = match part.class.as_str() {
                "BurstMotor" => "adjust_burst",
                "SmartZone" => "adjust_zone",
                _ => "adjust_none",
            };
            slot(ui, assets, "component content group", file);
            ui.values
                .insert("component name textbox".into(), asset.name().into());
            for (name, key) in [
                ("start angle slider", "start_angle"),
                ("end angle slider", "stop_angle"),
            ] {
                let angle = part
                    .settings
                    .get(key)
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(0.0);
                let value = if key == "stop_angle" {
                    angle / 4.8 + 0.5
                } else {
                    angle.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU
                };
                ui.values.insert(name.into(), value.to_string());
            }
            ui.values.insert(
                "zone name".into(),
                part.settings.get("zone").cloned().unwrap_or_default(),
            );
        } else if let Some(asset) = catalog.components.get(&editor.component) {
            let styles = catalog.styles_for(&editor.component);
            let selected = styles
                .iter()
                .find(|(_, key)| key == &asset.key)
                .map(|(name, _)| name.clone())
                .unwrap_or_else(|| "Default".into());
            ui.values.insert("component option".into(), selected);
            ui.choices.insert(
                "component option".into(),
                styles.into_iter().map(|(n, _)| n).collect(),
            );
        }
    }
    if editor.mode == Mode::Wiring
        && let Some(cells) = ui.cells.get_mut("control grid")
    {
        for (i, cell) in cells.iter_mut().enumerate() {
            if let Some(c) = editor
                .document
                .robot
                .controllers
                .iter()
                .find(|c| c.position == [(i % 4) as i32, (i / 4) as i32])
                && let Ok(base) = assets.widget("botlab/control_cell.gib")
            {
                let mut template = (*base).clone();
                patch(&mut template, "control img", "_Filename", &cell.image);
                let name_widget = format!("control name:{i}");
                rename_exact(&mut template, "control name", &name_widget);
                text(
                    &mut template,
                    &name_widget,
                    c.name.trim_start_matches("Name: "),
                );
                let keys = c
                    .keys
                    .iter()
                    .map(|k| key_name(*k.get(k.len().saturating_sub(2)).unwrap_or(&0)))
                    .collect::<Vec<_>>()
                    .join(" / ");
                text(&mut template, "control input", &keys);
                cell.template = Some(Arc::new(template));
                cell.image.clear();
                cell.label.clear();
            }
        }
    }
    if editor.mode == Mode::Paint {
        slot(
            ui,
            assets,
            "effect settings group",
            EFFECT_FILES[editor.paint_effect],
        );
        ui.choices.insert(
            "effect dropdown".into(),
            EFFECTS.iter().map(|v| v.to_string()).collect(),
        );
        ui.values.insert(
            "effect dropdown".into(),
            EFFECTS[editor.paint_effect].into(),
        );
        ui.colors.insert(
            "selected color img".into(),
            editor.color.map(|v| v as f32 / 255.0),
        );
        ui.interactive.insert("color picker img".into());
        let prefix = if editor.paint_effect == 3 {
            "texture library/edges/"
        } else if editor.paint_effect == 4 {
            "texture library/decals/"
        } else {
            "texture library/"
        };
        let selected = &editor.paint_texture;
        let list = if editor.paint_effect == 3 {
            "edge list"
        } else if editor.paint_effect == 4 {
            "decal list"
        } else {
            "texture list"
        };
        let cells = catalog
            .index
            .keys()
            .filter(|k| {
                k.starts_with(prefix)
                    && !k[prefix.len()..].contains('/')
                    && if editor.paint_effect == 3 {
                        k.ends_with(".txt")
                    } else {
                        k.ends_with(".bmp") || k.ends_with(".tga")
                    }
            })
            .map(|k| Cell {
                key: format!("texture:{k}"),
                label: std::path::Path::new(k)
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .into(),
                selected: selected == k,
                ..Default::default()
            })
            .collect();
        ui.cells.insert(list.into(), cells);
        if !selected.is_empty() {
            ui.images
                .insert("selected texture img".into(), selected.clone());
        }
        ui.values
            .insert("scale slider".into(), editor.decal_scale.to_string());
    }
    if let Some(dialog) = &editor.dialog {
        match dialog {
            Dialog::Help => {
                let suffix = match editor.mode {
                    Mode::Overview => "overview",
                    Mode::Chassis | Mode::Base | Mode::Top => "chassis",
                    Mode::Armor => "armor",
                    Mode::Components => "components",
                    Mode::Wiring => "wiring",
                    Mode::Paint => "paintshop",
                    Mode::Test => "testrobot",
                };
                if let Ok(panel) = assets.widget(&format!("help/help_ws_{suffix}.gib")) {
                    ui.slots.insert("help content group".into(), panel);
                }
                ui.values
                    .insert("caption bar".into(), "Robot Workshop Help".into());
            }
            Dialog::Message(message) => {
                for name in ["yes button", "no button", "cancel button"] {
                    ui.visibility.insert(name.into(), false);
                }
                ui.visibility.insert("ok button".into(), true);

                ui.values
                    .insert("caption bar".into(), "Robot Workshop".into());
                ui.values.insert("text box".into(), message.clone());
            }
            Dialog::Controller(_) => {
                ui.values.insert(
                    "input1".into(),
                    if editor.capture_key == Some(0) {
                        "Press a key…".into()
                    } else {
                        key_name(editor.key_positive)
                    },
                );
                ui.values.insert(
                    "input2".into(),
                    if editor.capture_key == Some(1) {
                        "Press a key…".into()
                    } else {
                        key_name(editor.key_negative)
                    },
                );
            }
            Dialog::Connection => {
                if let Some(index) = editor.selected
                    && let Some(part) = editor.document.robot.components.get(index)
                    && let Some(asset) = catalog
                        .components
                        .get(&ra2_formats::components::normalize(&part.definition))
                {
                    ui.values
                        .insert("component name".into(), asset.name().into());
                    ui.images
                        .insert("component image".into(), asset.preview.clone());
                    let choices = controls::actions(&part.class);
                    for (name, i) in [("input1 map", 0), ("input2 map", 1)] {
                        ui.choices
                            .insert(name.into(), choices.iter().map(|s| s.to_string()).collect());
                        ui.values.insert(
                            name.into(),
                            choices[editor.binding_actions[i] % choices.len()].into(),
                        );
                    }
                }
                if let Some(c) = editor
                    .selected_controller
                    .and_then(|i| editor.document.robot.controllers.get(i))
                {
                    ui.values.insert(
                        "control label".into(),
                        c.name.trim_start_matches("Name: ").into(),
                    );
                    for (i, key) in c.keys.iter().enumerate() {
                        ui.values.insert(
                            format!("input{} key", i + 1),
                            key_name(*key.get(key.len().saturating_sub(2)).unwrap_or(&0)),
                        );
                    }
                    ui.images.insert(
                        "control image".into(),
                        format!(
                            "botlab/ctrl_{}.bmp",
                            match c.kind {
                                1 => "analog",
                                2 => "switch",
                                _ => "button",
                            }
                        ),
                    );
                }
            }
            Dialog::Import | Dialog::PaintImport => {
                let ext = if matches!(dialog, Dialog::Import) {
                    "bot"
                } else {
                    "bmp"
                };
                let dir = catalog.root.join(if matches!(dialog, Dialog::Import) {
                    "Robot Designs"
                } else {
                    "Custom Textures"
                });
                let mut files = std::fs::read_dir(&dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.extension()
                            .and_then(|v| v.to_str())
                            .is_some_and(|s| s.eq_ignore_ascii_case(ext))
                    })
                    .collect::<Vec<_>>();
                files.sort();
                let cells = files
                    .into_iter()
                    .map(|p| {
                        let file = p.file_name().unwrap().to_string_lossy().to_string();
                        Cell {
                            key: format!("file:{file}"),
                            label: file.clone(),
                            selected: ui.values.get("import path") == Some(&file),
                            ..Default::default()
                        }
                    })
                    .collect();
                ui.cells.insert("file list".into(), cells);
            }
            _ => (),
        }
    }
    // Paint sources are loaded lazily, preserving original relative paths and case.
    if !editor.paint_texture.is_empty()
        && !assets.images.contains_key(&editor.paint_texture)
        && let Some(path) = catalog.index.get(&editor.paint_texture)
        && let Ok(bytes) = std::fs::read(path)
        && let Ok(image) = decode_image(
            &bytes,
            path.extension().and_then(|s| s.to_str()).unwrap_or("bmp"),
        )
    {
        assets.images.insert(editor.paint_texture.clone(), image);
    }
}
