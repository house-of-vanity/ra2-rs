//! Bounded, cosmetic particles: no entities per particle and no Rapier bodies.
use super::*;
use bevy::{
    asset::RenderAssetUsages,
    camera::primitives::Aabb,
    image::ImageSampler,
    light::NotShadowCaster,
    mesh::PrimitiveTopology,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
const CAPACITY: usize = 1024;
pub(super) const MAX_DEBRIS: usize = 8192;
const DEFAULT_DEBRIS_LIMIT: usize = 2048;
const CHUNK_SIZE: usize = 256;
const GROUND_CHUNKS: usize = MAX_DEBRIS / CHUNK_SIZE;
// Content::load_file translates the practice arena's floor to world Y=0.
const REST_Y: f32 = 0.003;

#[derive(Clone)]
pub(super) struct Settings {
    pub enabled: bool,
    pub wear: bool,
    pub sparks: f32,
    pub chips: f32,
    pub chip_scale: f32,
    pub debris_limit: usize,
    pub speed: f32,
    pub lifetime: f32,
    pub wear_strength: f32,
    pub wear_radius: f32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            wear: true,
            sparks: 28.0,
            chips: 18.0,
            chip_scale: 1.0,
            debris_limit: DEFAULT_DEBRIS_LIMIT,
            speed: 1.0,
            lifetime: 0.8,
            wear_strength: 0.85,
            wear_radius: 0.075,
        }
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Spark,
    Chip,
}
#[derive(Clone, Copy)]
enum Shape {
    Flake,
    Chunk,
}
#[derive(Clone, Copy)]
struct Debris {
    position: Vec3,
    rotation: Quat,
    size: f32,
    color: [f32; 4],
    shape: Shape,
}
/// Fixed-size chronological ring. Only overwritten chunks become dirty;
/// removing the oldest object does not shift/rebuild the rest of the map.
struct Ground {
    items: Vec<Debris>,
    cursor: usize,
    limit: usize,
    revisions: [u64; GROUND_CHUNKS],
}
impl Default for Ground {
    fn default() -> Self {
        Self {
            items: Vec::with_capacity(DEFAULT_DEBRIS_LIMIT),
            cursor: 0,
            limit: DEFAULT_DEBRIS_LIMIT,
            revisions: [0; GROUND_CHUNKS],
        }
    }
}
impl Ground {
    fn dirty_all(&mut self) {
        for revision in &mut self.revisions {
            *revision = revision.wrapping_add(1);
        }
    }
    fn clear(&mut self) {
        if !self.items.is_empty() {
            self.items.clear();
            self.cursor = 0;
            self.dirty_all();
        }
    }
    fn set_limit(&mut self, limit: usize) {
        let limit = limit.min(MAX_DEBRIS);
        if limit == self.limit {
            return;
        }
        let moved = self.cursor != 0 || self.items.len() > limit;
        if self.cursor != 0 {
            self.items.rotate_left(self.cursor);
        }
        if self.items.len() > limit {
            self.items.drain(..self.items.len() - limit);
        }
        self.cursor = 0;
        self.limit = limit;
        if moved {
            self.dirty_all();
        }
    }
    fn land(&mut self, debris: Debris) {
        if self.limit == 0 {
            return;
        }
        let slot = if self.items.len() < self.limit {
            let slot = self.items.len();
            self.items.push(debris);
            slot
        } else {
            let slot = self.cursor;
            self.items[slot] = debris;
            self.cursor = (slot + 1) % self.limit;
            slot
        };
        self.revisions[slot / CHUNK_SIZE] = self.revisions[slot / CHUNK_SIZE].wrapping_add(1);
    }
}
struct Particle {
    kind: Kind,
    shape: Shape,
    position: Vec3,
    velocity: Vec3,
    axis: Vec3,
    angle: f32,
    spin: f32,
    size: f32,
    life: f32,
    duration: f32,
    color: [f32; 4],
}
pub(super) struct Effects {
    particles: Vec<Particle>,
    ground: Ground,
    cursor: usize,
    rng: u32,
    pub frame_ms: f64,
}
impl Default for Effects {
    fn default() -> Self {
        Self {
            particles: Vec::with_capacity(CAPACITY),
            ground: Ground::default(),
            cursor: 0,
            rng: 0xa51f73c9,
            frame_ms: 0.0,
        }
    }
}
impl Effects {
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }
    fn push(&mut self, p: Particle) {
        if self.particles.len() < CAPACITY {
            self.particles.push(p)
        } else {
            let i = self.cursor % CAPACITY;
            self.particles[i] = p;
            self.cursor = (i + 1) % CAPACITY;
        }
    }
    pub fn clear(&mut self) {
        self.clear_airborne();
        self.clear_debris();
    }
    pub fn clear_debris(&mut self) {
        self.ground.clear();
    }
    pub fn set_debris_limit(&mut self, limit: usize) {
        self.ground.set_limit(limit);
    }
    pub fn debris_count(&self) -> usize {
        self.ground.items.len()
    }
    pub fn clear_airborne(&mut self) {
        self.particles.clear();
        self.cursor = 0;
        self.frame_ms = 0.0;
    }
    pub fn count(&self) -> usize {
        self.particles.len()
    }
    pub fn emit(
        &mut self,
        point: Vec3,
        normal: Vec3,
        color: [f32; 4],
        power: f32,
        settings: &Settings,
    ) {
        self.set_debris_limit(settings.debris_limit);
        if !settings.enabled {
            return;
        }
        let rotation = Quat::from_rotation_arc(Vec3::Y, normal);
        for (kind, count) in [(Kind::Spark, settings.sparks), (Kind::Chip, settings.chips)] {
            for _ in 0..(count * power.sqrt()).round().clamp(0.0, 160.0) as usize {
                let angle = self.random() * std::f32::consts::TAU;
                let spread = 0.3 + self.random() * 0.8;
                let direction = rotation
                    * Vec3::new(
                        angle.cos() * spread,
                        0.25 + self.random() * 0.8,
                        angle.sin() * spread,
                    )
                    .normalize();
                let spark = kind == Kind::Spark;
                let speed = (if spark {
                    1.5 + self.random() * 3.5
                } else {
                    0.5 + self.random() * 1.8
                }) * settings.speed
                    * power.sqrt();
                let duration = if spark {
                    settings.lifetime * (0.5 + self.random() * 0.8) * 0.65
                } else {
                    8.0
                }; // Flight safety bound, independent of spark fading.
                let metal = !spark && self.random() < 0.3;
                let tint = if spark {
                    [5.0, 1.6 + self.random() * 1.5, 0.12, 1.0]
                } else if metal {
                    [0.42, 0.45, 0.48, 1.0]
                } else {
                    color
                };
                let axis = Vec3::new(
                    self.random() - 0.5,
                    self.random() - 0.5,
                    self.random() - 0.5,
                )
                .normalize_or_zero();
                let p = Particle {
                    kind,
                    shape: if metal { Shape::Chunk } else { Shape::Flake },
                    position: point + normal * 0.012,
                    velocity: direction * speed,
                    axis,
                    angle: self.random() * std::f32::consts::TAU,
                    spin: (self.random() - 0.5) * 24.0,
                    size: if spark {
                        0.5 + self.random()
                    } else {
                        (if metal {
                            0.012 + self.random() * 0.018
                        } else {
                            0.007 + self.random() * 0.014
                        }) * settings.chip_scale
                    },
                    life: duration,
                    duration,
                    color: tint,
                };
                self.push(p);
            }
        }
    }
    pub fn step(&mut self, dt: f32) {
        if self.particles.is_empty() || dt <= 0.0 {
            self.frame_ms = 0.0;
            return;
        }
        let start = Instant::now();
        let ground = &mut self.ground;
        self.particles.retain_mut(|p| {
            let previous = p.position;
            p.life -= dt;
            p.velocity.y -= if p.kind == Kind::Spark {
                5.5 * dt
            } else {
                9.81 * dt
            };
            p.position += p.velocity * dt;
            p.angle += p.spin * dt;
            if !p.position.is_finite() {
                return false;
            }
            if p.kind == Kind::Chip && p.position.y <= REST_Y && p.velocity.y < 0.0 {
                let fraction =
                    ((previous.y - REST_Y) / (previous.y - p.position.y).max(1e-8)).clamp(0.0, 1.0);
                let mut position = previous.lerp(p.position, fraction);
                position.y = REST_Y;
                ground.land(Debris {
                    position,
                    rotation: Quat::from_rotation_y(p.angle),
                    size: p.size,
                    color: p.color,
                    shape: p.shape,
                });
                return false;
            }
            p.life > 0.0
        });
        self.frame_ms = start.elapsed().as_secs_f64() * 1000.0;
    }
}

#[derive(Resource)]
pub(super) struct Graphics {
    sparks: Handle<Mesh>,
    chips: Handle<Mesh>,
    count: usize,
    pub draw_ms: f64,
    ground: Vec<GroundGraphics>,
}
struct GroundGraphics {
    entity: Entity,
    mesh: Handle<Mesh>,
    revision: u64,
}
#[derive(Component)]
pub(super) struct GroundVisual;

#[derive(Default)]
struct Geometry {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    uvs: Vec<[f32; 2]>,
}
impl Geometry {
    fn triangle(&mut self, p: [Vec3; 3], color: [f32; 4]) {
        let normal = (p[1] - p[0])
            .cross(p[2] - p[0])
            .normalize_or_zero()
            .to_array();
        self.positions.extend(p.map(|p| p.to_array()));
        self.normals.extend([normal; 3]);
        self.colors.extend([color; 3]);
        self.uvs.extend([[0.0, 0.0]; 3]);
    }
    fn chip(&mut self, debris: Debris) {
        let vertices = [
            Vec3::new(-1., 0., -0.6),
            Vec3::new(0.8, 0., -0.4),
            Vec3::new(0.1, 0., 1.),
            Vec3::new(0.0, 0.8, 0.0),
        ]
        .map(|v| debris.position + debris.rotation * v * debris.size);
        self.triangle([vertices[0], vertices[1], vertices[2]], debris.color);
        if matches!(debris.shape, Shape::Chunk) {
            for ids in [[0, 3, 1], [1, 3, 2], [2, 3, 0]] {
                self.triangle(ids.map(|i| vertices[i]), debris.color);
            }
        }
    }
    fn bounds(&self) -> Aabb {
        let (min, max) = self.positions.iter().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(min, max), p| {
                let p = Vec3::from_array(*p);
                (min.min(p), max.max(p))
            },
        );
        if self.positions.is_empty() {
            Aabb::from_min_max(Vec3::ZERO, Vec3::ZERO)
        } else {
            Aabb::from_min_max(min, max)
        }
    }
    fn write(self, mesh: &mut Mesh) {
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colors);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs);
    }
}
fn mesh() -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new())
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, Vec::<[f32; 3]>::new())
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, Vec::<[f32; 2]>::new())
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, Vec::<[f32; 4]>::new())
}
pub(super) fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    // Tiny analytical glow mask is embedded by construction; no external files.
    let mut pixels = Vec::new();
    for y in 0..64 {
        for x in 0..16 {
            let u = (x as f32 + 0.5) / 8.0 - 1.0;
            let v = (y as f32 + 0.5) / 32.0 - 1.0;
            let alpha =
                ((-u * u * 5.0 - v * v * 2.0).exp() * (1.0 - v.abs()).max(0.0) * 255.0) as u8;
            pixels.extend_from_slice(&[255, 255, 255, alpha]);
        }
    }
    let mut mask_image = Image::new(
        Extent3d {
            width: 16,
            height: 64,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    mask_image.sampler = ImageSampler::linear();
    let mask = images.add(mask_image);
    let sparks = meshes.add(mesh());
    let chips = meshes.add(mesh());
    let spark_material = materials.add(StandardMaterial {
        base_color_texture: Some(mask),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        cull_mode: None,
        ..default()
    });
    let chip_material = materials.add(StandardMaterial {
        metallic: 0.35,
        perceptual_roughness: 0.6,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    for (name, handle, material) in [
        ("Impact sparks", sparks.clone(), spark_material),
        (
            "Paint and metal chips",
            chips.clone(),
            chip_material.clone(),
        ),
    ] {
        commands.spawn((
            Name::new(name),
            Mesh3d(handle),
            MeshMaterial3d(material),
            Transform::default(),
            NotShadowCaster,
            bevy::camera::visibility::NoFrustumCulling,
        ));
    }
    let ground = (0..GROUND_CHUNKS)
        .map(|i| {
            let mesh = meshes.add(mesh());
            let entity = commands
                .spawn((
                    Name::new(format!("Ground debris / {i}")),
                    GroundVisual,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(chip_material.clone()),
                    Transform::default(),
                    Visibility::Hidden,
                    NotShadowCaster,
                    Aabb::from_min_max(Vec3::ZERO, Vec3::ZERO),
                ))
                .id();
            GroundGraphics {
                entity,
                mesh,
                revision: 0,
            }
        })
        .collect();
    commands.insert_resource(Graphics {
        sparks,
        chips,
        count: 0,
        draw_ms: 0.0,
        ground,
    });
}
pub(super) fn animate(time: Res<Time<Real>>, mut lab: ResMut<Lab>) {
    let limit = lab.effect_settings.debris_limit;
    lab.effects.set_debris_limit(limit);
    if !lab.paused && lab.loading.is_none() {
        lab.effects.step(time.delta_secs().min(0.05));
    }
}
pub(super) fn draw(
    lab: Res<Lab>,
    mut graphics: ResMut<Graphics>,
    mut meshes: ResMut<Assets<Mesh>>,
    camera: Query<&Transform, With<LabCamera>>,
    mut chunks: Query<(&mut Visibility, &mut Aabb), With<GroundVisual>>,
) {
    let count = lab.effects.count();
    let airborne_changed = count != 0 || graphics.count != 0;
    let ground_changed = graphics
        .ground
        .iter()
        .enumerate()
        .any(|(i, chunk)| chunk.revision != lab.effects.ground.revisions[i]);
    if !airborne_changed && !ground_changed {
        graphics.draw_ms = 0.0;
        return;
    }
    let start = Instant::now();
    if airborne_changed {
        let camera = camera.iter().next().copied().unwrap_or_default();
        let right = camera.rotation * Vec3::X;
        let up = camera.rotation * Vec3::Y;
        for (kind, handle) in [
            (Kind::Spark, &graphics.sparks),
            (Kind::Chip, &graphics.chips),
        ] {
            let mut geometry = Geometry::default();
            for p in lab.effects.particles.iter().filter(|p| p.kind == kind) {
                if kind == Kind::Spark {
                    let along =
                        (right * p.velocity.dot(right) + up * p.velocity.dot(up)).normalize_or(up);
                    let across = along.cross(camera.rotation * Vec3::Z).normalize_or(right);
                    let long = along * (0.025 + p.velocity.length() * 0.013) * p.size;
                    let short = across * 0.008 * p.size;
                    let corners = [
                        p.position - short - long,
                        p.position + short - long,
                        p.position + short + long,
                        p.position - short + long,
                    ];
                    for i in [0, 1, 2, 0, 2, 3] {
                        geometry.positions.push(corners[i].to_array());
                        geometry.normals.push([0., 1., 0.]);
                        geometry
                            .uvs
                            .push([[0., 0.], [1., 0.], [1., 1.], [0., 1.]][i]);
                        let mut color = p.color;
                        color[3] = (p.life / p.duration).clamp(0., 1.).powi(2);
                        geometry.colors.push(color);
                    }
                } else {
                    geometry.chip(Debris {
                        position: p.position,
                        rotation: Quat::from_axis_angle(p.axis.normalize_or(Vec3::Y), p.angle),
                        size: p.size,
                        color: p.color,
                        shape: p.shape,
                    });
                }
            }
            if let Some(mut mesh) = meshes.get_mut(handle) {
                geometry.write(&mut mesh);
            }
        }
    }
    for (i, chunk) in graphics.ground.iter_mut().enumerate() {
        let revision = lab.effects.ground.revisions[i];
        if revision == chunk.revision {
            continue;
        }
        let mut geometry = Geometry::default();
        let begin = i * CHUNK_SIZE;
        let end = ((i + 1) * CHUNK_SIZE).min(lab.effects.ground.items.len());
        if let Some(items) = lab.effects.ground.items.get(begin..end) {
            for &debris in items {
                geometry.chip(debris);
            }
        }
        if let Ok((mut visibility, mut bounds)) = chunks.get_mut(chunk.entity) {
            *visibility = if geometry.positions.is_empty() {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
            *bounds = geometry.bounds();
        }
        if let Some(mut mesh) = meshes.get_mut(&chunk.mesh) {
            geometry.write(&mut mesh);
        }
        chunk.revision = revision;
    }
    graphics.count = count;
    graphics.draw_ms = start.elapsed().as_secs_f64() * 1000.0;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn debris(id: usize) -> Debris {
        Debris {
            position: Vec3::new(id as f32, REST_Y, 0.),
            rotation: Quat::IDENTITY,
            size: 0.01,
            color: [1., 0., 0., 1.],
            shape: Shape::Flake,
        }
    }
    #[test]
    fn ground_ring_keeps_newest_when_resized_and_dirties_only_changed_chunks() {
        let mut ground = Ground::default();
        ground.set_limit(CHUNK_SIZE * 2);
        for i in 0..CHUNK_SIZE * 2 {
            ground.land(debris(i));
        }
        let revisions = ground.revisions;
        ground.land(debris(900));
        assert_eq!(ground.revisions[0], revisions[0] + 1);
        assert_eq!(ground.revisions[1..], revisions[1..]);
        ground.land(debris(901));
        ground.set_limit(3);
        assert_eq!(
            ground
                .items
                .iter()
                .map(|d| d.position.x as usize)
                .collect::<Vec<_>>(),
            [511, 900, 901]
        );
        ground.land(debris(902));
        ground.set_limit(5);
        ground.land(debris(903));
        assert_eq!(
            ground
                .items
                .iter()
                .map(|d| d.position.x as usize)
                .collect::<Vec<_>>(),
            [900, 901, 902, 903]
        );
        ground.set_limit(usize::MAX);
        assert_eq!(ground.limit, MAX_DEBRIS);
        ground.set_limit(0);
        ground.land(debris(904));
        assert!(ground.items.is_empty());
    }
    #[test]
    fn fragments_land_keep_color_and_stop_updating_independently_of_spark_lifetime() {
        let mut effects = Effects::default();
        let settings = Settings {
            sparks: 0.,
            chips: 80.,
            lifetime: 0.2,
            speed: 0.,
            ..default()
        };
        let paint = [0.8, 0.1, 0.2, 1.];
        effects.emit(Vec3::Y, Vec3::Y, paint, 1., &settings);
        effects.step(0.25);
        assert_eq!(effects.count(), 80, "Chips must not fade with sparks");
        for _ in 0..240 {
            effects.step(1. / 120.);
        }
        assert_eq!(effects.count(), 0);
        assert_eq!(effects.debris_count(), 80);
        assert!(effects.ground.items.iter().all(|d| d.position.y == REST_Y));
        assert!(effects.ground.items.iter().any(|d| d.color == paint));
        assert!(
            effects
                .ground
                .items
                .iter()
                .any(|d| matches!(d.shape, Shape::Chunk))
        );
        let positions: Vec<_> = effects.ground.items.iter().map(|d| d.position).collect();
        let revisions = effects.ground.revisions;
        for _ in 0..600 {
            effects.step(1. / 60.);
        }
        assert_eq!(effects.frame_ms, 0.);
        assert_eq!(effects.ground.revisions, revisions);
        assert_eq!(
            effects
                .ground
                .items
                .iter()
                .map(|d| d.position)
                .collect::<Vec<_>>(),
            positions
        );
        effects.emit(Vec3::Y, Vec3::Y, paint, 1., &settings);
        effects.clear_airborne();
        assert_eq!(effects.debris_count(), 80);
        effects.emit(Vec3::Y, Vec3::Y, paint, 1., &settings);
        effects.clear_debris();
        assert_eq!(effects.debris_count(), 0);
        assert_eq!(effects.count(), 80);
        effects.clear();
        assert_eq!(effects.count(), 0);
    }
    #[test]
    fn particle_pool_is_bounded_expires_and_disabled_emission_is_empty() {
        let mut particles = Effects::default();
        let mut settings = Settings {
            enabled: false,
            ..default()
        };
        particles.emit(Vec3::Y, Vec3::Y, [1., 0., 0., 1.], 1., &settings);
        assert_eq!(particles.count(), 0);
        settings.enabled = true;
        settings.sparks = 100.;
        settings.chips = 80.;
        for _ in 0..100 {
            particles.emit(Vec3::Y, Vec3::Y, [1., 0., 0., 1.], 2., &settings);
        }
        assert_eq!(particles.count(), CAPACITY);
        particles.step(0.1);
        assert!(
            particles
                .particles
                .iter()
                .all(|p| p.position.is_finite() && p.velocity.is_finite())
        );
        particles.step(10.0);
        assert_eq!(particles.count(), 0);
        particles.emit(Vec3::Y, Vec3::Y, [1., 0., 0., 1.], 1., &settings);
        particles.clear();
        assert_eq!(particles.count(), 0);
    }
}
