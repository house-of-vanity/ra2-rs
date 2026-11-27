//! Original artwork, with a small editable layout transcribed from mainmenu.gib.
use crate::Options;
use bevy::{
    asset::RenderAssetUsages,
    image::{CompressedImageFormats, ImageSampler, ImageType},
    prelude::*,
};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artwork {
    pub path: String,
    pub rect: (f32, f32, f32, f32),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenuButton {
    pub caption: String,
    pub rect: (f32, f32, f32, f32),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Theme {
    pub version: u32,
    pub reference_size: (f32, f32),
    pub background: String,
    pub logo: String,
    pub music: String,
    pub artwork: Vec<Artwork>,
    pub buttons: Vec<MenuButton>,
}
#[derive(Resource)]
pub struct MenuAssets {
    pub theme: Theme,
    pub images: BTreeMap<String, Handle<Image>>,
}

pub fn load(mut commands: Commands, options: Res<Options>, mut images: ResMut<Assets<Image>>) {
    let path = options.game_dir.join(".ra2-rs/menu.ron");
    let theme: Theme = ron::from_str(
        &std::fs::read_to_string(path)
            .unwrap_or_else(|_| include_str!("../../config/menu.ron").into()),
    )
    .expect("Valid menu layout");
    assert!(theme.version == 1 && theme.reference_size.0 > 0.0 && theme.reference_size.1 > 0.0);
    let mut names: Vec<_> = theme.artwork.iter().map(|a| a.path.clone()).collect();
    names.extend([
        theme.background.clone(),
        theme.logo.clone(),
        "UI/common/little_gear.tga".into(),
        "UI/common/rounded_rectL.tga".into(),
        "UI/common/rounded_rectR.tga".into(),
    ]);
    let mut loaded = BTreeMap::new();
    let mut missing = 0;
    for name in names {
        let result = std::fs::read(options.game_dir.join(&name))
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                Image::from_buffer(
                    &bytes,
                    ImageType::Extension(name.rsplit('.').next().unwrap()),
                    CompressedImageFormats::NONE,
                    true,
                    ImageSampler::linear(),
                    RenderAssetUsages::default(),
                )
                .map_err(|e| e.to_string())
            });
        match result {
            Ok(image) => {
                loaded.insert(name, images.add(image));
            }
            Err(e) => {
                missing += 1;
                warn!("Menu image {name}: {e}");
            }
        }
    }
    info!("Menu: {} original images, {missing} missing", loaded.len());
    commands.insert_resource(MenuAssets {
        theme,
        images: loaded,
    });
}
