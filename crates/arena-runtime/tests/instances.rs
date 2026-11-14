use arena_core::{Material, Model};
use arena_runtime::{
    content::{TextureBytes, TextureSource},
    render::*,
};
use bevy::prelude::*;
use std::collections::BTreeMap;

struct NoTextures;
impl TextureSource for NoTextures {
    fn read(&self, _: &str) -> Result<TextureBytes, String> {
        Err("No texture expected".into())
    }
}

#[test]
fn model_instances_share_gpu_assets_and_have_visible_transform_roots() {
    let model = Model {
        materials: BTreeMap::from([(
            0,
            Material {
                name: "test".into(),
                color: [1.0; 3],
                alpha: 1.0,
                two_sided: false,
                texture: None,
                opacity_texture: None,
                submaterials: BTreeMap::new(),
            },
        )]),
        objects: vec![arena_core::Object {
            name: "part".into(),
            visible: true,
            parent: None,
            transform: Mat4::from_translation(Vec3::X),
            vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Z],
            faces: vec![arena_core::Face {
                vertices: [0, 1, 2],
                material: 0,
                uv: [[0.0; 2]; 3],
                normals: [Vec3::Y; 3],
            }],
            material: 0,
            uv_channels: BTreeMap::new(),
        }],
        ..Default::default()
    };
    let mut meshes = Assets::default();
    let mut materials = Assets::default();
    let mut images = Assets::default();
    let prepared = prepare_model(
        &model,
        &NoTextures,
        &RenderSettings::default(),
        &mut TextureCache::default(),
        &mut meshes,
        &mut materials,
        &mut images,
    )
    .unwrap();
    assert_eq!(meshes.len(), 1);
    assert_eq!(materials.len(), 1);
    let mut world = World::new();
    let first = prepared.spawn(&mut world.commands(), "first", Transform::default());
    let second = prepared.spawn(
        &mut world.commands(),
        "second",
        Transform::from_xyz(10.0, 0.0, 0.0),
    );
    world.flush();
    assert!(world.get::<Visibility>(first).is_some());
    assert!(world.get::<Visibility>(second).is_some());
    let handles: Vec<_> = world
        .query::<&Mesh3d>()
        .iter(&world)
        .map(|h| h.0.id())
        .collect();
    assert_eq!(handles.len(), 2);
    assert_eq!(handles[0], handles[1]);
    assert_eq!(meshes.len(), 1);
}

struct PerforatedTexture;
impl TextureSource for PerforatedTexture {
    fn read(&self, _: &str) -> Result<TextureBytes, String> {
        let mut bytes = vec![0; 18];
        bytes[2] = 2;
        bytes[12] = 2;
        bytes[14] = 1;
        bytes[16] = 32;
        bytes[17] = 8;
        bytes.extend([64, 64, 64, 255, 64, 64, 64, 0]);
        Ok(TextureBytes {
            key: "perforated-metal".into(),
            extension: "tga".into(),
            bytes,
        })
    }
}

#[test]
fn shared_diffuse_opacity_map_preserves_solid_metal_and_real_holes() {
    let map = arena_core::Texture {
        file: "sq_extender.tga".into(),
        scale: Vec2::ONE,
        offset: Vec2::ZERO,
        rotation: 0.0,
        channel: 1,
    };
    let model = Model {
        materials: BTreeMap::from([(
            0,
            Material {
                name: "perforated metal".into(),
                color: [1.0; 3],
                alpha: 1.0,
                two_sided: false,
                texture: Some(map.clone()),
                opacity_texture: Some(map),
                submaterials: Default::default(),
            },
        )]),
        ..Default::default()
    };
    let mut meshes = Assets::default();
    let mut materials = Assets::default();
    let mut images = Assets::default();
    prepare_model(
        &model,
        &PerforatedTexture,
        &RenderSettings::default(),
        &mut TextureCache::default(),
        &mut meshes,
        &mut materials,
        &mut images,
    )
    .unwrap();
    let (_, material) = materials.iter().next().unwrap();
    assert!(matches!(material.alpha_mode,AlphaMode::Mask(cutoff) if cutoff==0.5));
    assert_eq!(
        material.base_color.alpha(),
        1.0,
        "Metal must not inherit the 18% fallback opacity"
    );
    let image = images
        .get(material.base_color_texture.as_ref().unwrap())
        .unwrap();
    let rgba = image
        .convert(bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb)
        .unwrap();
    let data = rgba.data.as_ref().unwrap();
    assert_eq!(data[3], 255);
    assert_eq!(data[7], 0);
}
