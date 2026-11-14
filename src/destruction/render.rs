//! Original component assets are shared. Deformable meshes keep native paint UVs across tears.
use super::*;
use arena_runtime::destruction::SurfaceTriangle;
use bevy::{
    asset::RenderAssetUsages,
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    image::{CompressedImageFormats, ImageSampler, ImageType},
    mesh::{PrimitiveTopology, VertexAttributeValues},
};
use std::{collections::HashMap, time::Instant};

#[derive(Component)]
pub(super) struct RigidVisual(RigidBodyHandle);
#[derive(Component)]
pub(super) struct SawVisual;
#[derive(Component)]
pub(super) struct SawTooth(f32);
#[derive(Component)]
pub(super) struct Aim;
#[derive(Component)]
pub(super) struct Spark {
    velocity: Vec3,
    life: f32,
    total: f32,
}

struct Skin {
    topology: u32,
    faces: Vec<SurfaceTriangle>,
    painted: Handle<Mesh>,
    metal: Handle<Mesh>,
}
#[derive(Resource)]
pub(super) struct RenderState {
    generation: u64,
    skins: HashMap<SoftBodyHandle, Skin>,
    hammer: Option<RigidBodyHandle>,
    paint: Handle<StandardMaterial>,
    metal: Handle<StandardMaterial>,
    amber: Handle<StandardMaterial>,
    black: Handle<StandardMaterial>,
    spark: Handle<StandardMaterial>,
    cube: Handle<Mesh>,
    wire: Handle<Mesh>,
    wire_entity: Entity,
    seed: u32,
}

fn mesh(topology: PrimitiveTopology) -> Mesh {
    Mesh::new(topology, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new())
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, Vec::<[f32; 3]>::new())
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, Vec::<[f32; 2]>::new())
}
fn piece(
    commands: &mut Commands,
    parent: Entity,
    shape: Handle<Mesh>,
    material: Handle<StandardMaterial>,
    position: Vec3,
    scale: Vec3,
) {
    commands.spawn((
        Mesh3d(shape),
        MeshMaterial3d(material),
        Transform::from_translation(position).with_scale(scale),
        ChildOf(parent),
    ));
}

#[allow(clippy::too_many_arguments)]
pub(super) fn setup(
    mut commands: Commands,
    content: Res<Content>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let prepared = content
        .prepare(&mut meshes, &mut materials, &mut images)
        .expect("Native demo assets");
    prepared.arena.spawn(
        &mut commands,
        "Original Robot Arena 2 test garage",
        Transform::from_translation(content.arena_offset),
    );
    commands.insert_resource(prepared);
    let mut paint_bytes = content
        .robot
        .chassis
        .texture
        .clone()
        .expect("Scout native paint");
    for pixel in paint_bytes[18..].as_chunks_mut::<4>().0 {
        pixel[3] = 255;
    }
    let image = Image::from_buffer(
        &paint_bytes,
        ImageType::Extension("tga"),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::linear(),
        RenderAssetUsages::default(),
    )
    .expect("Native paint TGA");
    let paint = materials.add(StandardMaterial {
        base_color_texture: Some(images.add(image)),
        perceptual_roughness: 0.4,
        metallic: 0.25,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    let metal = materials.add(StandardMaterial {
        base_color: Color::srgb(0.22, 0.25, 0.29),
        metallic: 0.8,
        perceptual_roughness: 0.4,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    let amber = materials.add(StandardMaterial {
        base_color: Color::srgb(0.95, 0.38, 0.035),
        metallic: 0.55,
        perceptual_roughness: 0.32,
        ..default()
    });
    let black = materials.add(StandardMaterial {
        base_color: Color::srgb(0.035, 0.045, 0.06),
        perceptual_roughness: 0.6,
        ..default()
    });
    let spark = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.65, 0.12),
        emissive: LinearRgba::new(12.0, 3.0, 0.2, 1.0),
        unlit: true,
        ..default()
    });
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let sphere = meshes.add(Sphere::new(1.0).mesh().ico(1).unwrap());
    let saw = commands
        .spawn((SawVisual, Transform::default(), Visibility::Hidden))
        .id();
    piece(
        &mut commands,
        saw,
        cube.clone(),
        metal.clone(),
        Vec3::new(0.02, 0.0, 0.0),
        Vec3::new(0.58, 0.025, 0.15),
    );
    piece(
        &mut commands,
        saw,
        cube.clone(),
        amber.clone(),
        Vec3::new(-0.39, 0.08, 0.0),
        Vec3::new(0.3, 0.2, 0.24),
    );
    piece(
        &mut commands,
        saw,
        cube.clone(),
        black.clone(),
        Vec3::new(-0.43, 0.22, 0.0),
        Vec3::new(0.15, 0.06, 0.3),
    );
    for i in 0..22 {
        let angle = i as f32 / 22.0 * std::f32::consts::TAU;
        commands.spawn((
            SawTooth(angle),
            Mesh3d(cube.clone()),
            MeshMaterial3d(black.clone()),
            Transform::default(),
            ChildOf(saw),
        ));
    }
    commands.spawn((
        Aim,
        Mesh3d(sphere.clone()),
        MeshMaterial3d(spark.clone()),
        Transform::from_scale(Vec3::splat(0.012)),
        Visibility::Hidden,
    ));
    let wire = meshes.add(
        mesh(PrimitiveTopology::LineList)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0; 3]; 2])
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 2])
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0; 2]; 2]),
    );
    let wire_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.1, 0.85, 1.0),
        unlit: true,
        ..default()
    });
    let wire_entity = commands
        .spawn((
            Mesh3d(wire.clone()),
            MeshMaterial3d(wire_mat),
            Transform::default(),
            Visibility::Hidden,
        ))
        .id();
    commands.insert_resource(RenderState {
        generation: u64::MAX,
        skins: HashMap::new(),
        hammer: None,
        paint,
        metal,
        amber,
        black,
        spark,
        cube,
        wire,
        wire_entity,
        seed: 0x731b9c21,
    });
    commands.spawn((
        LabCamera,
        Camera3d::default(),
        Tonemapping::Reinhard,
        DebandDither::Disabled,
        Msaa::Sample4,
        Transform::from_xyz(2.8, 2.8, 3.5).looking_at(Vec3::new(0.0, 0.35, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 13000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.85, -0.6, 0.0)),
    ));
    commands.spawn((
        PointLight {
            intensity: 350000.0,
            color: Color::srgb(0.3, 0.65, 1.0),
            range: 8.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(-2.0, 2.5, -1.0),
    ));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.72, 0.82, 1.0),
        brightness: 220.0,
        ..default()
    });
}

fn fill(mesh: &mut Mesh, body: &SoftBody, faces: &[SurfaceTriangle], painted: bool, rebuild: bool) {
    let relevant = || faces.iter().filter(|f| f.uv.is_some() == painted);
    if rebuild {
        let count = relevant().count() * 3;
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0; 3]; count]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0; 3]; count]);
        let uv: Vec<_> = relevant()
            .flat_map(|f| f.uv.unwrap_or([Vec2::ZERO; 3]).map(|v| v.to_array()))
            .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    }
    if let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    {
        for (out, face) in positions.as_chunks_mut::<3>().0.iter_mut().zip(relevant()) {
            for (out, i) in out.iter_mut().zip(face.ids) {
                *out = body.particle_position(i as usize).to_array();
            }
        }
    }
    if let Some(VertexAttributeValues::Float32x3(normals)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_NORMAL)
    {
        for (out, face) in normals.as_chunks_mut::<3>().0.iter_mut().zip(relevant()) {
            let p = face.ids.map(|i| body.particle_position(i as usize));
            let normal = (p[1] - p[0])
                .cross(p[2] - p[0])
                .normalize_or_zero()
                .to_array();
            out.fill(normal);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn synchronize(
    mut commands: Commands,
    mut lab: ResMut<Lab>,
    mut state: ResMut<RenderState>,
    prepared: Res<content::PreparedContent>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut rigid: Query<(Entity, &RigidVisual, &mut Transform)>,
    mut visibility: Query<&mut Visibility>,
) {
    if lab.physics.unstable {
        return;
    }
    let start = Instant::now();
    if state.generation != lab.generation {
        state.generation = lab.generation;
        state.skins.clear();
        state.hammer = None;
        for (i, body) in lab.physics.parts.iter().enumerate() {
            let entity = prepared.parts[i].spawn(
                &mut commands,
                "Original Scout component",
                Transform::default(),
            );
            commands
                .entity(entity)
                .insert((LabEntity, RigidVisual(*body)));
        }
    }
    for (entity, visual, mut transform) in &mut rigid {
        if let Some(body) = lab.physics.world.bodies.get(visual.0) {
            transform.translation = body.translation();
            transform.rotation = *body.rotation();
        } else {
            commands.entity(entity).despawn();
        }
    }
    for (handle, body) in lab.physics.world.soft_bodies.iter() {
        let new = !state.skins.contains_key(&handle);
        if new {
            let painted = meshes.add(mesh(PrimitiveTopology::TriangleList));
            let metal = meshes.add(mesh(PrimitiveTopology::TriangleList));
            for (m, material) in [
                (painted.clone(), state.paint.clone()),
                (metal.clone(), state.metal.clone()),
            ] {
                commands.spawn((
                    LabEntity,
                    Mesh3d(m),
                    MeshMaterial3d(material),
                    Transform::default(),
                    bevy::camera::visibility::NoFrustumCulling,
                ));
            }
            state.skins.insert(
                handle,
                Skin {
                    topology: u32::MAX,
                    faces: Vec::new(),
                    painted,
                    metal,
                },
            );
        }
        let skin = state.skins.get_mut(&handle).unwrap();
        let rebuild = skin.topology != body.topology_version();
        if rebuild {
            skin.faces = lab.physics.shell.surface(body);
            skin.topology = body.topology_version();
        }
        fill(
            &mut meshes.get_mut(&skin.painted).unwrap(),
            body,
            &skin.faces,
            true,
            rebuild,
        );
        fill(
            &mut meshes.get_mut(&skin.metal).unwrap(),
            body,
            &skin.faces,
            false,
            rebuild,
        );
    }
    if let Some(hammer) = &lab.physics.hammer {
        if state.hammer != Some(hammer.body) {
            state.hammer = Some(hammer.body);
            let pose = lab.physics.world.bodies[hammer.body].position();
            let parent = commands
                .spawn((
                    LabEntity,
                    RigidVisual(hammer.body),
                    Transform::from_translation(pose.translation).with_rotation(pose.rotation),
                    Visibility::default(),
                ))
                .id();
            piece(
                &mut commands,
                parent,
                state.cube.clone(),
                state.metal.clone(),
                Vec3::ZERO,
                Vec3::new(0.36, 0.24, 0.28),
            );
            piece(
                &mut commands,
                parent,
                state.cube.clone(),
                state.amber.clone(),
                Vec3::new(0.0, 0.125, 0.0),
                Vec3::new(0.3, 0.025, 0.22),
            );
            piece(
                &mut commands,
                parent,
                state.cube.clone(),
                state.black.clone(),
                Vec3::new(0.0, 0.57, 0.0),
                Vec3::new(0.065, 1.0, 0.065),
            );
            piece(
                &mut commands,
                parent,
                state.cube.clone(),
                state.amber.clone(),
                Vec3::new(0.0, 0.85, 0.0),
                Vec3::new(0.08, 0.3, 0.08),
            );
        }
    } else {
        state.hammer = None;
    }
    if let Ok(mut v) = visibility.get_mut(state.wire_entity) {
        *v = if lab.wire {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    if lab.wire {
        let points: Vec<[f32; 3]> = lab
            .physics
            .world
            .soft_bodies
            .iter()
            .flat_map(|(_, body)| {
                body.boundary()
                    .iter()
                    .take(4000)
                    .flat_map(|face| {
                        [[face[0], face[1]], [face[1], face[2]], [face[2], face[0]]]
                            .into_iter()
                            .flatten()
                            .map(|i| {
                                let p = body.particle_position(i as usize);
                                (p + (p - body.center_of_mass()).normalize_or_zero() * 0.003)
                                    .to_array()
                            })
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let count = points.len();
        let mut mesh = meshes.get_mut(&state.wire).unwrap();
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, points);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; count]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; count]);
    }
    lab.render_ms = start.elapsed().as_secs_f64() * 1000.0;
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn effects(
    mut commands: Commands,
    time: Res<Time>,
    mut lab: ResMut<Lab>,
    mut state: ResMut<RenderState>,
    mut sparks: Query<(Entity, &mut Spark, &mut Transform)>,
    mut saw: Query<
        (&mut Transform, &mut Visibility),
        (
            With<SawVisual>,
            Without<Spark>,
            Without<Aim>,
            Without<SawTooth>,
        ),
    >,
    mut teeth: Query<
        (&SawTooth, &mut Transform),
        (Without<Spark>, Without<SawVisual>, Without<Aim>),
    >,
    mut aim: Query<
        (&mut Transform, &mut Visibility),
        (
            With<Aim>,
            Without<Spark>,
            Without<SawVisual>,
            Without<SawTooth>,
        ),
    >,
) {
    let dt = if lab.paused {
        0.0
    } else {
        time.delta_secs() * if lab.slow { 0.2 } else { 1.0 }
    };
    for (entity, mut spark, mut transform) in &mut sparks {
        spark.life -= dt;
        if spark.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        spark.velocity.y -= 7.0 * dt;
        transform.translation += spark.velocity * dt;
        transform.scale = Vec3::new(0.006, 0.006, 0.035) * (spark.life / spark.total).sqrt();
        transform.rotation = Quat::from_rotation_arc(Vec3::Z, spark.velocity.normalize_or_zero());
    }
    let impacts = std::mem::take(&mut lab.physics.impacts);
    let mut remaining = 280_usize.saturating_sub(sparks.iter().count());
    for impact in impacts {
        let count = (impact.power * 45.0).ceil() as usize;
        for _ in 0..count.min(remaining) {
            let mut random = || {
                state.seed = state.seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (state.seed >> 8) as f32 / 16777216.0
            };
            let direction = (Vec3::new(
                random() * 2.0 - 1.0,
                random() * 2.0 - 1.0,
                random() * 2.0 - 1.0,
            ) + impact.normal * 1.5)
                .normalize_or_zero();
            let speed = 1.0 + random() * 3.0;
            let life = 0.25 + random() * 0.45;
            commands.spawn((
                LabEntity,
                Spark {
                    velocity: direction * speed,
                    life,
                    total: life,
                },
                Mesh3d(state.cube.clone()),
                MeshMaterial3d(state.spark.clone()),
                Transform::from_translation(impact.point).with_scale(Vec3::splat(0.008)),
            ));
        }
        remaining = remaining.saturating_sub(count);
    }
    if let Some((_, point, normal)) = lab.target {
        for (mut transform, mut visibility) in &mut aim {
            *visibility = Visibility::Visible;
            transform.translation = point + normal * 0.012;
        }
        for (mut transform, mut visibility) in &mut saw {
            *visibility = if lab.tool == tools::Tool::Saw {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
            transform.translation = point + normal * if lab.saw_active { -0.025 } else { 0.055 };
            transform.rotation = Quat::from_axis_angle(normal, lab.cut_angle)
                * Quat::from_rotation_arc(Vec3::Y, normal);
        }
    } else {
        for (_, mut visibility) in &mut aim {
            *visibility = Visibility::Hidden;
        }
        for (_, mut visibility) in &mut saw {
            *visibility = Visibility::Hidden;
        }
    }
    let speed = if lab.saw_active { 35.0 } else { 0.0 };
    for (tooth, mut transform) in &mut teeth {
        let angle = tooth.0 + time.elapsed_secs() * speed;
        transform.translation = Vec3::new(angle.cos() * 0.30, 0.0, angle.sin() * 0.09);
        transform.scale = Vec3::new(0.035, 0.042, 0.028);
        transform.rotation = Quat::from_rotation_y(-angle);
    }
    // Keep the tool resource fields explicit: future effects can share the same cached primitives.
}
