//! Application composition: choose assets/rules, then call format-independent adapters.
use crate::{Options, config::Config};
use arena_core::Model;
pub use arena_runtime::render::ImportStats;
use arena_runtime::{
    content::FileTextures,
    physics::{self, StaticSurface},
    render::{self, RenderSettings, TextureCache},
};
use bevy::prelude::*;
use ra2_formats::ModelImporters;
use std::path::{Path, PathBuf};

pub struct PreviewAsset {
    pub model: Model,
    pub source: PathBuf,
    pub label: String,
    pub position: Vec3,
    pub size: f32,
}

#[derive(Resource)]
pub struct Scene {
    pub lab_half_size: f32,
    pub model: Option<Model>,
    pub source: PathBuf,
    pub offset: Vec3,
    pub origin: Vec3,
    pub title: String,
    pub config: Config,
    pub previews: Vec<PreviewAsset>,
}

fn load_model(importers: &ModelImporters, path: &Path) -> Result<Model, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let imported = importers
        .decode(&bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    println!(
        "Loaded {}: {}, {} objects, {} attachments, {} compatibility notes, {} retained extension nodes",
        path.display(),
        imported.report.format,
        imported.model.objects.len(),
        imported.model.attachments.len(),
        imported.report.warnings.len(),
        imported.report.extensions.len()
    );
    Ok(imported.model)
}

impl Scene {
    pub fn load(options: &Options) -> Result<Self, String> {
        let config = Config::load(&options.config)?;
        let importers = ModelImporters::with_ra2();
        let mut scene = if options.arena {
            let source = options.game_dir.join(&config.arena.model);
            let model = load_model(&importers, &source)?;
            let floor = model
                .object(&config.arena.floor_object)
                .ok_or("Configured floor object not found")?;
            let floor_y = floor.positions().next().ok_or("Empty arena floor")?.y;
            let collision = model
                .object(&config.arena.collision_object)
                .ok_or("Configured collision object not found")?;
            if collision.faces.is_empty() {
                return Err("Empty arena collision mesh".into());
            }
            Self {
                lab_half_size: 32.0,
                model: Some(model),
                source,
                offset: Vec3::new(0.0, -floor_y, 0.0),
                origin: Vec3::from_array(config.arena.robot_spawn),
                title: config.arena.title.clone(),
                config,
                previews: vec![],
            }
        } else {
            Self {
                lab_half_size: 32.0,
                model: None,
                source: PathBuf::new(),
                offset: Vec3::ZERO,
                origin: Vec3::new(0.0, 1.1, 22.0),
                title: "Physics lab".into(),
                config,
                previews: vec![],
            }
        };
        if options.showcase {
            for preview in &scene.config.showcase {
                let source = options.game_dir.join(&preview.model);
                scene.previews.push(PreviewAsset {
                    model: load_model(&importers, &source)?,
                    source,
                    label: preview.label.clone(),
                    position: Vec3::from_array(preview.position),
                    size: preview.size,
                });
            }
        }
        Ok(scene)
    }
}

pub fn spawn_collision(commands: &mut Commands, scene: &Scene) {
    if let Some(model) = &scene.model {
        physics::spawn_static_surface(
            commands,
            model,
            &StaticSurface {
                object: &scene.config.arena.collision_object,
                transform: Mat4::from_translation(scene.offset),
                friction: Some(scene.config.arena.friction),
                restitution: Some(scene.config.arena.restitution),
            },
        )
        .expect("Validated configured arena collision");
    }
}

pub fn spawn_visuals(
    mut commands: Commands,
    scene: Res<Scene>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let settings = RenderSettings {
        roughness: scene.config.rendering.roughness,
        opacity_map_alpha: scene.config.rendering.opacity_map_alpha,
    };
    let mut cache = TextureCache::default();
    let mut total = ImportStats::default();
    if let Some(model) = &scene.model {
        let source = FileTextures::new(scene.source.parent().unwrap());
        let prepared = render::prepare_model(
            model,
            &source,
            &settings,
            &mut cache,
            &mut meshes,
            &mut materials,
            &mut images,
        )
        .expect("Validated arena model");
        prepared.spawn(
            &mut commands,
            &scene.title,
            Transform::from_translation(scene.offset),
        );
        total = prepared.stats;
    }
    for preview in &scene.previews {
        let source = FileTextures::new(preview.source.parent().unwrap());
        let prepared = render::prepare_model(
            &preview.model,
            &source,
            &settings,
            &mut cache,
            &mut meshes,
            &mut materials,
            &mut images,
        )
        .expect("Validated preview model");
        let mut positions = preview
            .model
            .objects
            .iter()
            .filter(|o| o.visible)
            .flat_map(|o| o.positions());
        if let Some(first) = positions.next() {
            let (min, max) =
                positions.fold((first, first), |(min, max), p| (min.min(p), max.max(p)));
            let scale = preview.size / (max - min).max_element().max(0.001);
            let pivot = Vec3::new((min.x + max.x) / 2.0, min.y, (min.z + max.z) / 2.0);
            let transform = Mat4::from_translation(preview.position)
                * Mat4::from_scale(Vec3::splat(scale))
                * Mat4::from_translation(-pivot);
            prepared.spawn(
                &mut commands,
                &preview.label,
                Transform::from_matrix(transform),
            );
            total.meshes += prepared.stats.meshes;
            total.triangles += prepared.stats.triangles;
            total.textures += prepared.stats.textures;
            total.missing += prepared.stats.missing;
        }
    }
    println!(
        "Model rendering: {} meshes, {} triangles, {} textures, {} missing; {} showcase models",
        total.meshes,
        total.triangles,
        total.textures,
        total.missing,
        scene.previews.len()
    );
    commands.insert_resource(total);
}
