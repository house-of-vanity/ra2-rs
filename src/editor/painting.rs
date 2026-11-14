use super::{Editor, catalog::Catalog};
use crate::legacy_ui::{
    State,
    assets::{LegacyAssets, decode_image},
};
use arena_core::{Vec2, editor::paint};
use std::path::Path;
pub fn apply(
    editor: &mut Editor,
    catalog: &Catalog,
    assets: &LegacyAssets,
    panel: Option<usize>,
    uv: Option<Vec2>,
) -> Result<(), String> {
    let color = editor.color;
    if editor.paint_effect == 0 {
        return editor
            .document
            .edit("Color fill", |r| paint::fill(&mut r.chassis, panel, color));
    }
    if editor.paint_effect == 5 {
        return Ok(());
    }
    let mut key = editor.paint_texture.clone();
    let mut width = 5.0;
    let mut ratio = 0.1;
    if editor.paint_effect == 3 {
        let path = catalog.index.get(&key).ok_or("Select an edge style")?;
        let metadata = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        for line in metadata.lines() {
            if let Some((name, value)) = line.split_once(':') {
                let value = value.trim();
                match name.trim() {
                    "Filename" => {
                        key = ra2_formats::components::normalize(&format!(
                            "texture library/edges/{value}"
                        ));
                    }
                    "Width (world pixels)" => width = value.parse().unwrap_or(5.0),
                    "Length ratio (world pixels per file pixel)" => {
                        ratio = value.parse().unwrap_or(0.1)
                    }
                    _ => (),
                }
            }
        }
    }
    let source = if let Some(image) = assets.image(&key) {
        image.clone()
    } else {
        let path = catalog
            .index
            .get(&key)
            .ok_or("Select a texture from the list")?;
        decode_image(
            &std::fs::read(path).map_err(|e| e.to_string())?,
            path.extension().and_then(|s| s.to_str()).unwrap_or("bmp"),
        )?
    };
    let effect = match editor.paint_effect {
        1 => paint::TextureEffect::Fill,
        2 => paint::TextureEffect::Layer,
        3 => paint::TextureEffect::Edge {
            width,
            length_ratio: ratio,
        },
        4 => paint::TextureEffect::Decal {
            center: uv.ok_or("Click on a panel to place a decal")? * 256.0,
            size: editor.decal_scale * 128.0 + 4.0,
        },
        _ => unreachable!(),
    };
    editor.document.edit("Texture paint", |r| {
        paint::texture(
            &mut r.chassis,
            panel,
            (source.width, source.height, &source.rgba),
            effect,
        )
    })
}
pub fn exchange(
    editor: &mut Editor,
    catalog: &Catalog,
    ui: &State,
    import: bool,
) -> Result<(), String> {
    if import {
        let name = ui.values.get("import path").ok_or("Select a BMP file")?;
        if Path::new(name).file_name().and_then(|n| n.to_str()) != Some(name) {
            return Err("Invalid texture filename".into());
        }
        let path = catalog.root.join("Custom Textures").join(name);
        let image = decode_image(&std::fs::read(path).map_err(|e| e.to_string())?, "bmp")?;
        if image.width != 256 || image.height != 256 {
            return Err("Custom textures must be 256 x 256 pixels".into());
        }
        let tga = paint::from_rgba(256, 256, &image.rgba)?;
        editor.document.edit("Import chassis texture", |r| {
            r.chassis.texture = Some(tga);
            Ok(())
        })
    } else {
        let name = ui
            .values
            .get("export name textbox")
            .map(String::as_str)
            .unwrap_or("Chassis")
            .trim()
            .trim_end_matches(".bmp");
        if name.is_empty() || name.contains(['/', '\\', ':', '\0']) || name == ".." {
            return Err("Enter a valid filename".into());
        }
        let mut chassis = editor.document.robot.chassis.clone();
        if !ui
            .checked
            .get("texture or template cb")
            .copied()
            .unwrap_or(false)
        {
            paint::fill(&mut chassis, None, [255; 4])?;
            paint::texture(
                &mut chassis,
                None,
                (1, 1, &[0, 0, 0, 255]),
                paint::TextureEffect::Edge {
                    width: 1.0,
                    length_ratio: 1.0,
                },
            )?;
        }
        let (w, h, rgba) = paint::rgba(chassis.texture.as_deref().ok_or("Build a chassis first")?)?;
        let path = catalog
            .root
            .join("Custom Textures")
            .join(format!("{name}.bmp"));
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(path, paint::bmp(w, h, &rgba)?).map_err(|e| e.to_string())
    }
}
