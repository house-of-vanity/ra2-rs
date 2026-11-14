//! Neutral model -> reusable GPU assets -> any number of scene instances.
use crate::content::TextureSource;
use arena_core::{Material, Model};
use bevy::{
    asset::RenderAssetUsages,
    image::{
        CompressedImageFormats, ImageAddressMode, ImageFilterMode, ImageSampler,
        ImageSamplerDescriptor, ImageType,
    },
    mesh::PrimitiveTopology,
    prelude::*,
};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug)]
pub struct RenderSettings {
    pub roughness: f32,
    /// Fallback for separate opacity maps. Shared diffuse/opacity RGBA textures
    /// use their real alpha mask, without this global transparency approximation.
    pub opacity_map_alpha: f32,
}
impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            roughness: 0.85,
            opacity_map_alpha: 0.18,
        }
    }
}

#[derive(Clone, Default, Resource)]
pub struct ImportStats {
    pub meshes: usize,
    pub triangles: usize,
    pub textures: usize,
    pub missing: usize,
}

#[derive(Default)]
pub struct TextureCache {
    images: HashMap<String, Handle<Image>>,
}
impl TextureCache {
    pub fn len(&self) -> usize {
        self.images.len()
    }
    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }
    fn load(
        &mut self,
        source: &dyn TextureSource,
        file: &str,
        images: &mut Assets<Image>,
    ) -> Result<Handle<Image>, String> {
        let content = source.read(file)?;
        if let Some(image) = self.images.get(&content.key) {
            return Ok(image.clone());
        }
        let image = Image::from_buffer(
            &content.bytes,
            ImageType::Extension(&content.extension),
            CompressedImageFormats::NONE,
            true,
            ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::Repeat,
                address_mode_v: ImageAddressMode::Repeat,
                mag_filter: ImageFilterMode::Linear,
                min_filter: ImageFilterMode::Linear,
                mipmap_filter: ImageFilterMode::Linear,
                ..default()
            }),
            RenderAssetUsages::default(),
        )
        .map_err(|e| format!("{}: {e}", content.key))?;
        let handle = images.add(image);
        self.images.insert(content.key, handle.clone());
        Ok(handle)
    }
}

#[derive(Clone)]
struct Part {
    object: usize,
    name: String,
    transform: Mat4,
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

#[derive(Clone)]
pub struct PreparedModel {
    parts: Vec<Part>,
    pub stats: ImportStats,
}

impl PreparedModel {
    pub fn mesh_handles(&self) -> impl Iterator<Item = &Handle<Mesh>> {
        self.parts.iter().map(|p| &p.mesh)
    }
    pub fn material_handles(&self) -> impl Iterator<Item = &Handle<StandardMaterial>> {
        self.parts.iter().map(|p| &p.material)
    }

    /// Each object gets its own transform. Mesh/material handles are shared across instances.
    pub fn spawn(&self, commands: &mut Commands, name: &str, transform: Transform) -> Entity {
        self.spawn_on_layer(commands, name, transform, 0)
    }
    pub fn spawn_on_layer(
        &self,
        commands: &mut Commands,
        name: &str,
        transform: Transform,
        layer: usize,
    ) -> Entity {
        self.spawn_tracked(commands, name, transform, layer).0
    }
    /// Return object/entity identities for articulated previews. Submaterials of
    /// the same object share its motion; GPU meshes and materials stay cached.
    pub fn spawn_tracked(
        &self,
        commands: &mut Commands,
        name: &str,
        transform: Transform,
        layer: usize,
    ) -> (Entity, Vec<(usize, Entity)>) {
        let root = commands
            .spawn((
                Name::new(name.to_string()),
                transform,
                Visibility::default(),
            ))
            .id();
        let mut objects = Vec::with_capacity(self.parts.len());
        for part in &self.parts {
            let entity = commands
                .spawn((
                    Name::new(part.name.clone()),
                    Mesh3d(part.mesh.clone()),
                    MeshMaterial3d(part.material.clone()),
                    Transform::from_matrix(part.transform),
                    bevy::camera::visibility::RenderLayers::layer(layer),
                    ChildOf(root),
                ))
                .id();
            objects.push((part.object, entity));
        }
        (root, objects)
    }
}
pub fn prepare_model(
    model: &Model,
    source: &dyn TextureSource,
    settings: &RenderSettings,
    cache: &mut TextureCache,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> Result<PreparedModel, String> {
    model.validate()?;
    let mut stats = ImportStats::default();
    let mut handles = BTreeMap::new();
    let mut loaded = HashMap::new();
    for (&id, material) in &model.materials {
        let submaterials: Vec<(usize, &Material)> = if material.submaterials.is_empty() {
            vec![(0, material)]
        } else {
            material
                .submaterials
                .iter()
                .map(|(&id, material)| (id, material))
                .collect()
        };
        for (sub, material) in submaterials {
            let texture = material.texture.as_ref().and_then(|t| {
                loaded
                    .entry(t.file.clone())
                    .or_insert_with(|| match cache.load(source, &t.file, images) {
                        Ok(h) => {
                            stats.textures += 1;
                            Some(h)
                        }
                        Err(e) => {
                            stats.missing += 1;
                            warn!("{e}");
                            None
                        }
                    })
                    .clone()
            });
            let alpha_mask = texture.is_some()
                && material
                    .texture
                    .as_ref()
                    .zip(material.opacity_texture.as_ref())
                    .is_some_and(|(color, opacity)| {
                        color
                            .file
                            .replace('\\', "/")
                            .eq_ignore_ascii_case(&opacity.file.replace('\\', "/"))
                            && color.channel == opacity.channel
                            && color.scale.abs_diff_eq(opacity.scale, 0.00001)
                            && color.offset.abs_diff_eq(opacity.offset, 0.00001)
                            && (color.rotation - opacity.rotation).abs() < 0.00001
                    });
            let alpha = if material.opacity_texture.is_some() && !alpha_mask {
                material.alpha.min(settings.opacity_map_alpha)
            } else {
                material.alpha
            };
            let color = if texture.is_some() {
                [1.0; 3]
            } else {
                material.color
            };
            handles.insert(
                (id, sub),
                materials.add(StandardMaterial {
                    base_color: Color::srgba(color[0], color[1], color[2], alpha),
                    base_color_texture: texture,
                    perceptual_roughness: settings.roughness,
                    alpha_mode: if alpha < 0.99 {
                        AlphaMode::Blend
                    } else if alpha_mask {
                        // Perforated metal: solid pixels write depth and holes are
                        // discarded. Blending the whole part makes it look like glass.
                        AlphaMode::Mask(0.5)
                    } else {
                        AlphaMode::Opaque
                    },
                    double_sided: material.two_sided,
                    cull_mode: if material.two_sided {
                        None
                    } else {
                        Some(bevy::render::render_resource::Face::Back)
                    },
                    ..default()
                }),
            );
        }
    }
    let mut parts = vec![];
    for (object_index, object) in model.objects.iter().enumerate() {
        if !object.visible || object.faces.is_empty() {
            continue;
        }
        let material = &model.materials[&object.material];
        let mut groups = BTreeMap::<usize, Vec<(usize, &arena_core::Face)>>::new();
        for (index, face) in object.faces.iter().enumerate() {
            let sub = if material.submaterials.is_empty() {
                0
            } else if material.submaterials.contains_key(&face.material) {
                face.material
            } else {
                *material.submaterials.keys().next().unwrap()
            };
            groups.entry(sub).or_default().push((index, face));
        }
        for (sub, faces) in groups {
            let mat = material.submaterials.get(&sub).unwrap_or(material);
            let mut positions = vec![];
            let mut normals = vec![];
            let mut uv = vec![];
            for &(index, face) in &faces {
                for i in 0..3 {
                    positions.push(object.vertices[face.vertices[i]].to_array());
                    normals.push(face.normals[i].to_array());
                    let mut point = Vec2::from_array(face.uv[i]);
                    if let Some(t) = &mat.texture {
                        if t.channel > 1
                            && let Some(channel) = object.uv_channels.get(&t.channel)
                            && let Some(face) = channel.faces.get(index)
                        {
                            point = channel.vertices[face[i]];
                        }
                        let source_uv = Vec2::new(point.x, 1.0 - point.y) * t.scale;
                        let (sin, cos) = t.rotation.sin_cos();
                        let mapped = Vec2::new(
                            cos * source_uv.x - sin * source_uv.y,
                            sin * source_uv.x + cos * source_uv.y,
                        ) + t.offset;
                        point = Vec2::new(mapped.x, 1.0 - mapped.y);
                    }
                    uv.push(point.to_array());
                }
            }
            let mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::default(),
            )
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv);
            parts.push(Part {
                object: object_index,
                name: format!("{} / {}", object.name, mat.name),
                transform: object.transform,
                mesh: meshes.add(mesh),
                material: handles[&(object.material, sub)].clone(),
            });
            stats.meshes += 1;
            stats.triangles += faces.len();
        }
    }
    Ok(PreparedModel { parts, stats })
}
