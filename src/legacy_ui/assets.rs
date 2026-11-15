use super::canvas::Pixels;
use crate::Options;
use bevy::{
    asset::RenderAssetUsages,
    image::{CompressedImageFormats, ImageSampler, ImageType},
    prelude::*,
};
use ra2_formats::{
    archive::CfzArchive,
    components::normalize,
    gib::{self, Widget},
    giff::{self, BitmapFont},
};
use std::{collections::BTreeMap, path::Path, sync::Arc};

#[derive(Resource, Default)]
pub struct LegacyAssets {
    pub widgets: BTreeMap<String, Arc<Widget>>,
    pub images: BTreeMap<String, Pixels>,
    pub fonts: BTreeMap<String, BitmapFont>,
    pub armors: Vec<ra2_formats::armor::ArmorDefinition>,
}
pub fn decode_image(bytes: &[u8], extension: &str) -> Result<Pixels, String> {
    let image = Image::from_buffer(
        bytes,
        ImageType::Extension(extension),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::linear(),
        RenderAssetUsages::default(),
    )
    .map_err(|e| e.to_string())?;
    let width = image.width() as usize;
    let height = image.height() as usize;
    let data = image.data.ok_or("Image has no CPU pixels")?;
    if data.len() != width * height * 4 {
        return Err("Expected RGBA UI pixels".into());
    }
    Ok(Pixels {
        width,
        height,
        rgba: data,
    })
}
impl LegacyAssets {
    pub fn load(root: &Path) -> Result<Self, String> {
        let mut this = Self::default();
        let mut files = vec![];
        crate::content::scan(&root.join("UI"), &mut files)?;
        for path in files {
            let key = normalize(
                &path
                    .strip_prefix(root.join("UI"))
                    .unwrap()
                    .to_string_lossy(),
            );
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext == "gib" {
                this.widgets.insert(
                    key,
                    Arc::new(gib::decode(
                        &std::fs::read(&path).map_err(|e| e.to_string())?,
                    )?),
                );
            } else if ["bmp", "tga", "png"].contains(&ext.as_str()) {
                let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                this.images.insert(
                    key,
                    decode_image(&bytes, &ext).map_err(|e| format!("{}: {e}", path.display()))?,
                );
            }
        }
        let archive =
            CfzArchive::decode(&std::fs::read(root.join("RA2.cfz")).map_err(|e| e.to_string())?)?;
        this.armors = ra2_formats::armor::decode(archive.get("Armor_Definitions.txt")?)?;
        for (name, bytes) in archive.files {
            if let Some(name) = name.strip_suffix(".giff") {
                this.fonts.insert(name.into(), giff::decode(&bytes)?);
            } else if let Some(extension) = name
                .rsplit('.')
                .next()
                .filter(|e| ["tga", "bmp", "png"].contains(e))
            {
                this.images
                    .entry(normalize(&name))
                    .or_insert(decode_image(&bytes, extension)?);
            }
        }
        info!(
            "Legacy GUI: {} layouts, {} images, {} original fonts",
            this.widgets.len(),
            this.images.len(),
            this.fonts.len()
        );
        Ok(this)
    }
    pub fn widget(&self, path: &str) -> Result<Arc<Widget>, String> {
        self.widgets
            .get(&normalize(path))
            .cloned()
            .ok_or_else(|| format!("Missing UI layout {path}"))
    }
    pub fn image(&self, path: &str) -> Option<&Pixels> {
        let path = normalize(path.trim_start_matches('.'));
        self.images.get(&path)
    }
}
pub fn load(mut commands: Commands, options: Res<Options>) {
    match LegacyAssets::load(&options.game_dir) {
        Ok(assets) => {
            commands.insert_resource(assets);
        }
        Err(e) => {
            warn!("Legacy editor resources unavailable: {e}");
            commands.init_resource::<LegacyAssets>();
        }
    }
}
