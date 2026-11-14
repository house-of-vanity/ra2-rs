//! Native paint/components plus an explicit line mesh, requiring no wireframe GPU feature.
use super::*;
use arena_runtime::dent_render::{SmoothingGroups, update as skin};
use arena_runtime::{
    content::FileTextures,
    render::{RenderSettings, TextureCache, prepare_model},
};
use bevy::{
    asset::RenderAssetUsages,
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    mesh::PrimitiveTopology,
};

#[derive(Component)]
pub(super) struct Solid;
#[derive(Component)]
pub(super) struct PartFrame(pub(super) usize);
#[derive(Component)]
pub(super) struct BoxVisual(pub(super) usize);
#[derive(Component)]
pub(super) struct Wire;
#[derive(Resource)]
pub(super) struct Graphics {
    root: Option<Entity>,
    parts: Vec<Option<Entity>>,
    chassis: Handle<Mesh>,
    wire: Handle<Mesh>,
    hammer: Entity,
    aim: Entity,
    impact: Entity,
    cube: Handle<Mesh>,
    lines: Handle<StandardMaterial>,
    colors: Vec<Handle<StandardMaterial>>,
    revision: u64,
    smoothing: SmoothingGroups,
    paint: Handle<Image>,
    surface: Handle<Image>,
    paint_key: (u64, u32),
    geometry_key: (usize, bool),
    pub surface_ms: f64,
}

fn empty(topology: PrimitiveTopology) -> Mesh {
    Mesh::new(topology, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new())
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, Vec::<[f32; 3]>::new())
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, Vec::<[f32; 2]>::new())
}
/// Cache topology once per loaded shell. Corners share lighting only when their
/// original panels are coplanar; current dent angles and UV seams do not split
/// a panel or round an authored chassis edge.
fn line_data(mesh: &mut Mesh, positions: Vec<[f32; 3]>) {
    let len = positions.len();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; len]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0; 2]; len]);
}
fn wire(mesh: &mut Mesh, loaded: &content::Loaded) {
    let mut positions = Vec::new();
    for t in &loaded.shell.triangles {
        let p = t.nodes.map(|i| loaded.shell.positions[i]);
        for i in 0..3 {
            positions.extend([p[i].to_array(), p[(i + 1) % 3].to_array()]);
        }
    }
    line_data(mesh, positions);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn setup(
    mut commands: Commands,
    lab: Res<Lab>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let content = &lab.active.loaded.content;
    let arena = prepare_model(
        &content.arena,
        &FileTextures::new(content.arena_path.parent().unwrap()),
        &RenderSettings::default(),
        &mut TextureCache::default(),
        &mut meshes,
        &mut materials,
        &mut images,
    )
    .expect("Validated native garage");
    arena.spawn(
        &mut commands,
        "Original test garage",
        Transform::from_translation(content.arena_offset),
    );
    commands.spawn((
        LabCamera,
        Camera3d::default(),
        Tonemapping::Reinhard,
        DebandDither::Disabled,
        Msaa::Sample4,
        Transform::from_xyz(3.0, 3.0, 4.0).looking_at(Vec3::new(0.0, 0.4, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 14000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.85, -0.6, 0.0)),
    ));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.8, 0.85, 1.0),
        brightness: 250.0,
        ..default()
    });
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let head = materials.add(StandardMaterial {
        base_color: Color::srgb(0.95, 0.4, 0.04),
        metallic: 0.65,
        ..default()
    });
    let handle = materials.add(StandardMaterial {
        base_color: Color::srgb(0.10, 0.11, 0.13),
        ..default()
    });
    let hammer = commands
        .spawn((Transform::default(), Visibility::Hidden))
        .id();
    commands.spawn((
        Mesh3d(cube.clone()),
        MeshMaterial3d(head),
        Transform::from_scale(Vec3::new(0.34, 0.20, 0.26)),
        ChildOf(hammer),
    ));
    commands.spawn((
        Mesh3d(cube.clone()),
        MeshMaterial3d(handle),
        Transform::from_xyz(0.0, 0.38, 0.0).with_scale(Vec3::new(0.055, 0.60, 0.055)),
        ChildOf(hammer),
    ));
    let lines = materials.add(StandardMaterial {
        base_color: Color::srgb(0.16, 0.85, 1.0),
        unlit: true,
        ..default()
    });
    let sphere = meshes.add(Sphere::new(0.018).mesh().ico(1).unwrap());
    let aim = commands
        .spawn((
            Mesh3d(sphere),
            MeshMaterial3d(lines.clone()),
            Transform::default(),
            Visibility::Hidden,
        ))
        .id();
    let impact_mesh = meshes.add(empty(PrimitiveTopology::LineList));
    let impact = commands
        .spawn((
            Mesh3d(impact_mesh),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(1.0, 0.65, 0.08),
                unlit: true,
                ..default()
            })),
            Transform::default(),
            Visibility::Hidden,
        ))
        .id();
    let colors = (0..=20)
        .map(|i| {
            let t = i as f32 / 20.0;
            materials.add(StandardMaterial {
                base_color: Color::srgb((t * 2.0).min(1.0), (2.0 - t * 2.0).min(1.0) * 0.85, 0.08),
                unlit: true,
                ..default()
            })
        })
        .collect();
    let mut graphics = Graphics {
        root: None,
        parts: Vec::new(),
        chassis: Handle::default(),
        wire: Handle::default(),
        hammer,
        aim,
        impact,
        cube,
        lines,
        colors,
        revision: u64::MAX,
        smoothing: SmoothingGroups::default(),
        paint: Handle::default(),
        surface: Handle::default(),
        paint_key: (u64::MAX, 0),
        geometry_key: (usize::MAX, false),
        surface_ms: 0.0,
    };
    replace_robot(
        &mut commands,
        &lab,
        &mut graphics,
        &mut meshes,
        &mut materials,
        &mut images,
    );
    commands.insert_resource(graphics);
}

pub(super) fn replace_robot(
    commands: &mut Commands,
    lab: &Lab,
    graphics: &mut Graphics,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) {
    if let Some(root) = graphics.root.take() {
        commands.entity(root).despawn();
    }
    let loaded = &lab.active.loaded;
    let content = &loaded.content;
    let root = commands
        .spawn((
            Name::new(format!("Damage robot / {}", content.robot.name)),
            lab.active.physics.transform(),
            Visibility::default(),
        ))
        .id();
    graphics.root = Some(root);
    graphics.parts = vec![None; content.robot.components.len()];
    let mut painted = empty(PrimitiveTopology::TriangleList);
    graphics.smoothing = SmoothingGroups::new(&loaded.shell);
    skin(&mut painted, &loaded.shell, lab.smooth, &graphics.smoothing);
    graphics.chassis = meshes.add(painted);
    let strength = if lab.effect_settings.wear {
        lab.effect_settings.wear_strength
    } else {
        0.0
    };
    let (color, surface) = loaded.paint.textures(strength);
    graphics.paint = images.add(loaded.paint.image(color, true));
    graphics.surface = images.add(loaded.paint.image(surface, false));
    graphics.paint_key = (loaded.paint.revision, strength.to_bits());
    graphics.geometry_key = (lab.strikes, lab.smooth);
    let paint = materials.add(StandardMaterial {
        base_color_texture: Some(graphics.paint.clone()),
        metallic_roughness_texture: Some(graphics.surface.clone()),
        metallic: 1.0,
        base_color: Color::WHITE,
        double_sided: true,
        cull_mode: None,
        perceptual_roughness: 1.0,
        ..default()
    });
    commands.spawn((
        Solid,
        Mesh3d(graphics.chassis.clone()),
        MeshMaterial3d(paint),
        Transform::default(),
        ChildOf(root),
    ));
    let mut cache = TextureCache::default();
    for &i in &content.part_indices {
        match prepare_model(
            &content.models[i],
            &FileTextures::new(content.sources[i].parent().unwrap()),
            &RenderSettings::default(),
            &mut cache,
            meshes,
            materials,
            images,
        ) {
            Ok(prepared) => {
                // Keep textured and wire visuals as siblings so hiding the
                // textured subtree does not hide the detached wire representation.
                let frame = commands
                    .spawn((
                        Name::new(format!("Component frame {i}")),
                        PartFrame(i),
                        Transform::from_matrix(content.assembly.parts[i].transform),
                        Visibility::default(),
                        ChildOf(root),
                    ))
                    .id();
                graphics.parts[i] = Some(frame);
                let part =
                    prepared.spawn(commands, &format!("Component {i}"), Transform::default());
                commands.entity(part).insert((Solid, ChildOf(frame)));
                let mut positions = Vec::new();
                for object in content.models[i].objects.iter().filter(|o| o.visible) {
                    for face in &object.faces {
                        let p = face
                            .vertices
                            .map(|v| object.transform.transform_point3(object.vertices[v]));
                        for j in 0..3 {
                            positions.extend([p[j].to_array(), p[(j + 1) % 3].to_array()]);
                        }
                    }
                }
                let mut lines = empty(PrimitiveTopology::LineList);
                line_data(&mut lines, positions);
                commands.spawn((
                    Wire,
                    Mesh3d(meshes.add(lines)),
                    MeshMaterial3d(graphics.lines.clone()),
                    Transform::default(),
                    Visibility::Hidden,
                    ChildOf(frame),
                ));
            }
            Err(e) => warn!("Component {i}: {e}"),
        }
    }
    let mut lines = empty(PrimitiveTopology::LineList);
    wire(&mut lines, loaded);
    graphics.wire = meshes.add(lines);
    commands.spawn((
        Wire,
        Mesh3d(graphics.wire.clone()),
        MeshMaterial3d(graphics.lines.clone()),
        Transform::default(),
        Visibility::Hidden,
        ChildOf(root),
    ));
    for component in &loaded.boxes {
        let transform = component.transform
            * Mat4::from_translation((component.min + component.max) * 0.5)
            * Mat4::from_scale(component.max - component.min);
        commands.spawn((
            BoxVisual(component.component),
            Mesh3d(graphics.cube.clone()),
            MeshMaterial3d(graphics.colors[0].clone()),
            Transform::from_matrix(transform),
            Visibility::Hidden,
            ChildOf(root),
        ));
    }
    graphics.revision = u64::MAX;
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn synchronize(
    lab: Res<Lab>,
    mut graphics: ResMut<Graphics>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut roots: Query<&mut Transform, (Without<LabCamera>, Without<Mesh3d>)>,
    mut solids: Query<&mut Visibility, (With<Solid>, Without<Wire>, Without<BoxVisual>)>,
    mut wires: Query<&mut Visibility, (With<Wire>, Without<Solid>, Without<BoxVisual>)>,
    mut boxes: Query<
        (
            &BoxVisual,
            &mut Transform,
            &mut Visibility,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        (With<Mesh3d>, Without<Solid>, Without<Wire>),
    >,
    mut mesh_visuals: Query<
        (&mut Transform, &mut Visibility, &Mesh3d),
        (Without<Solid>, Without<Wire>, Without<BoxVisual>),
    >,
    mut hammer_visibility: Query<
        &mut Visibility,
        (
            Without<Mesh3d>,
            Without<Solid>,
            Without<Wire>,
            Without<BoxVisual>,
        ),
    >,
) {
    let transform = lab.active.physics.transform();
    if let Some(root) = graphics.root
        && let Ok(mut t) = roots.get_mut(root)
    {
        *t = transform;
    }
    let inverse = transform.to_matrix().inverse();
    for group in &lab.active.physics.detached {
        let delta = inverse * lab.active.physics.frame(group.parts[0]).to_matrix();
        for &i in &group.parts {
            if let Some(entity) = graphics.parts[i]
                && let Ok(mut t) = roots.get_mut(entity)
            {
                *t = Transform::from_matrix(
                    delta * lab.active.loaded.content.assembly.parts[i].transform,
                );
            }
        }
    }
    // Debug volumes follow their detached body, retaining each child's own HP.
    for (visual, mut t, _, _) in &mut boxes {
        if let Some(component) = lab
            .active
            .loaded
            .boxes
            .iter()
            .find(|b| b.component == visual.0)
        {
            let delta = inverse * lab.active.physics.frame(visual.0).to_matrix();
            *t = Transform::from_matrix(
                delta
                    * component.transform
                    * Mat4::from_translation((component.min + component.max) * 0.5)
                    * Mat4::from_scale(component.max - component.min),
            );
        }
    }
    if let Some(hammer) = &lab.active.physics.hammer {
        let body = &lab.active.physics.world.bodies[hammer.body];
        if let Ok(mut t) = roots.get_mut(graphics.hammer) {
            *t = Transform::from_translation(body.translation()).with_rotation(*body.rotation());
        }
        if let Ok(mut v) = hammer_visibility.get_mut(graphics.hammer) {
            *v = Visibility::Visible;
        }
    } else if let Ok(mut v) = hammer_visibility.get_mut(graphics.hammer) {
        *v = Visibility::Hidden;
    }
    if let Ok((mut t, mut v, _)) = mesh_visuals.get_mut(graphics.aim) {
        *v = if lab.aim.is_some() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if let Some(aim) = lab.aim {
            t.translation = lab.active.physics.aim_frame(aim).transform_point(aim.point)
                + lab.active.physics.aim_frame(aim).rotation * aim.normal * 0.012;
        }
    }
    if let Ok((mut t, mut v, mesh)) = mesh_visuals.get_mut(graphics.impact) {
        *t = lab
            .last_impact
            .map_or(transform, |aim| lab.active.physics.aim_frame(aim));
        *v = if lab.show_impact && lab.last_impact.is_some() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if lab.revision != graphics.revision
            && let Some(hit) = lab.last_impact
        {
            let mut lines = Vec::new();
            let rotation = Quat::from_rotation_arc(Vec3::Y, hit.normal);
            for i in 0..32 {
                for j in [i, i + 1] {
                    let angle = j as f32 * std::f32::consts::TAU / 32.0;
                    lines.push(
                        (hit.point
                            + hit.normal * 0.012
                            + rotation * Vec3::new(angle.cos(), 0.0, angle.sin()) * lab.radius)
                            .to_array(),
                    );
                }
            }
            lines.extend([
                hit.point.to_array(),
                (hit.point - hit.normal * lab.depth * lab.power).to_array(),
            ]);
            if let Some(mut mesh) = meshes.get_mut(&mesh.0) {
                line_data(&mut mesh, lines);
            }
        }
    }
    if lab.revision == graphics.revision {
        return;
    }
    let start = Instant::now();
    let geometry_key = (lab.strikes, lab.smooth);
    let strength = if lab.effect_settings.wear {
        lab.effect_settings.wear_strength
    } else {
        0.0
    };
    let paint_key = (lab.active.loaded.paint.revision, strength.to_bits());
    let updated = graphics.geometry_key != geometry_key || graphics.paint_key != paint_key;
    if graphics.geometry_key != geometry_key {
        if let Some(mut mesh) = meshes.get_mut(&graphics.chassis) {
            skin(
                &mut mesh,
                &lab.active.loaded.shell,
                lab.smooth,
                &graphics.smoothing,
            );
        }
        if let Some(mut mesh) = meshes.get_mut(&graphics.wire) {
            wire(&mut mesh, &lab.active.loaded);
        }
        graphics.geometry_key = geometry_key;
    }
    if graphics.paint_key != paint_key {
        let (color, surface) = lab.active.loaded.paint.textures(strength);
        if let Some(mut image) = images.get_mut(&graphics.paint) {
            image.data = Some(color);
        }
        if let Some(mut image) = images.get_mut(&graphics.surface) {
            image.data = Some(surface);
        }
        graphics.paint_key = paint_key;
    }
    if updated {
        graphics.surface_ms = start.elapsed().as_secs_f64() * 1000.0;
    }
    for mut v in &mut solids {
        *v = if lab.wire {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
    }
    for mut v in &mut wires {
        *v = if lab.wire {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    for (visual, _, mut visibility, mut material) in &mut boxes {
        *visibility = if lab.boxes {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if let Some(component) = lab
            .active
            .loaded
            .boxes
            .iter()
            .find(|c| c.component == visual.0)
        {
            material.0 =
                graphics.colors[((component.damage / 5.0).ceil() as usize).min(20)].clone();
        }
    }
    graphics.revision = lab.revision;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smooth_dents_join_uv_seams_preserve_panel_edges_and_do_not_change_geometry() {
        use arena_core::{Face, Model, Object, damage::DentMesh};
        let face = |vertices, uv, normal| Face {
            vertices,
            uv,
            normals: [normal; 3],
            material: 0,
        };
        let mut model = Model::default();
        model.objects.push(Object {
            name: "roof and side".into(),
            visible: true,
            parent: None,
            transform: Mat4::IDENTITY,
            vertices: vec![Vec3::ZERO, Vec3::X, Vec3::X + Vec3::Z, Vec3::Z, -Vec3::Y],
            faces: vec![
                face([0, 2, 1], [[0.0; 2]; 3], Vec3::Y),
                // Deliberately split the UVs at the coplanar diagonal.
                face([0, 3, 2], [[1.0; 2]; 3], Vec3::Y),
                face([0, 1, 4], [[0.5; 2]; 3], -Vec3::Z),
            ],
            material: 0,
            uv_channels: default(),
        });
        let mut shell = DentMesh::from_model(&model, 0.35).unwrap();
        let groups = SmoothingGroups::new(&shell);
        for p in &mut shell.positions {
            p.y -= 0.25 * p.x * p.z;
        }
        let before = shell.positions.clone();
        let mut flat = empty(PrimitiveTopology::TriangleList);
        let mut smooth = empty(PrimitiveTopology::TriangleList);
        skin(&mut flat, &shell, false, &groups);
        skin(&mut smooth, &shell, true, &groups);
        for attribute in [Mesh::ATTRIBUTE_POSITION, Mesh::ATTRIBUTE_UV_0] {
            assert_eq!(flat.attribute(attribute), smooth.attribute(attribute));
        }
        assert_eq!(before, shell.positions);
        let flat_normals = flat
            .attribute(Mesh::ATTRIBUTE_NORMAL)
            .unwrap()
            .as_float3()
            .unwrap();
        let normals = smooth
            .attribute(Mesh::ATTRIBUTE_NORMAL)
            .unwrap()
            .as_float3()
            .unwrap();
        assert!(
            flat_normals
                .iter()
                .zip(normals)
                .any(|(a, b)| Vec3::from_array(*a).distance(Vec3::from_array(*b)) > 0.01)
        );
        assert!(
            normals
                .iter()
                .all(|n| (Vec3::from_array(*n).length() - 1.0).abs() < 0.00001)
        );
        let mut roof_corner = Vec::new();
        let mut wall_corner = Vec::new();
        for (i, t) in shell.triangles.iter().enumerate() {
            for (corner, &node) in t.nodes.iter().enumerate() {
                if shell.rest[node] == Vec3::ZERO {
                    if t.normal.y > 0.9 {
                        roof_corner.push(Vec3::from_array(normals[i * 3 + corner]));
                    } else {
                        wall_corner.push(Vec3::from_array(normals[i * 3 + corner]));
                    }
                }
            }
        }
        assert!(roof_corner.len() >= 2);
        assert!(!wall_corner.is_empty());
        assert!(
            roof_corner
                .iter()
                .all(|n| n.abs_diff_eq(roof_corner[0], 0.00001))
        );
        assert!(roof_corner[0].y > 0.95);
        assert!(wall_corner.iter().all(|n| n.abs_diff_eq(-Vec3::Z, 0.00001)));
    }
}
