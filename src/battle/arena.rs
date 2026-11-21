//! Generic original GMF scene instantiation: collision, articulated scenery and native starts.
pub(crate) use super::hazards::{Kind, Trap, tick};
use super::*;
use crate::{
    content::arenas::{Arena, HazardKind},
    editor::catalog::Catalog,
};
use arena_core::{Body, JointKind, Model};
use arena_runtime::{
    content::FileTextures,
    render::{self, RenderSettings, TextureCache},
};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug)]
pub(crate) struct Zone {
    pub min: Vec3,
    pub max: Vec3,
    inverse: Mat4,
    local_min: Vec3,
    local_max: Vec3,
}
impl Zone {
    pub fn from_object(model: &Model, name: &str) -> Result<Self, String> {
        let o = model
            .object(name)
            .ok_or_else(|| format!("Missing native arena zone {name}"))?;
        let (local_min, local_max) = aabb(o.vertices.iter().copied()).ok_or("Empty arena zone")?;
        let (min, max) = aabb(o.positions()).unwrap();
        Ok(Self {
            min,
            max,
            inverse: o.transform.inverse(),
            local_min,
            local_max,
        })
    }
    pub fn contains(&self, p: Vec3) -> bool {
        let p = self.inverse.transform_point3(p);
        let margin = Vec3::new(0.25, 0.5, 0.25);
        p.cmpge(self.local_min - margin).all() && p.cmple(self.local_max + margin).all()
    }
}
pub(crate) fn aabb(points: impl Iterator<Item = Vec3>) -> Option<(Vec3, Vec3)> {
    points.fold(None, |bounds: Option<(Vec3, Vec3)>, p| {
        Some(bounds.map_or((p, p), |(a, b)| (a.min(p), b.max(p))))
    })
}
#[derive(Clone)]
pub(crate) struct Spring {
    pub entity: Entity,
    pub rest: Quat,
    pub strength: f32,
    pub damping: f32,
    pub mass: f32,
}
#[derive(Clone)]
pub(crate) struct Joint {
    pub entity: Entity,
    pub linear: bool,
    pub axis: Vec3,
    pub limits: [f32; 2],
    pub force_scale: f32,
}
#[derive(Default)]
pub(crate) struct State {
    pub bounds: Option<(Vec3, Vec3)>,
    pub camera_shell: Option<Collider>,
    pub ringout_y: f32,
    pub king: Option<Zone>,
    pub hill_clock: f32,
    pub springs: Vec<Spring>,
    pub debris: Vec<(Entity, usize)>,
    /// Local collision triangles, keyed by their arena body for live transforms.
    pub ai_obstacles: ai_obstacles::Geometry,
    pub king_owner: Option<usize>,
}
pub(crate) struct Loaded {
    pub entities: Vec<Entity>,
    pub traps: Vec<Trap>,
    pub starts: Vec<Transform>,
    pub state: State,
}
fn display_names(raw: &Model, body: &Body) -> BTreeSet<String> {
    let mut names = BTreeSet::from([body.name.clone()]);
    if body.use_display_proxy && !body.display_proxy.is_empty() {
        names.insert(body.display_proxy.clone());
    }
    loop {
        let before = names.len();
        for o in &raw.objects {
            if o.parent.as_ref().is_some_and(|p| names.contains(p)) {
                names.insert(o.name.clone());
            }
        }
        if before == names.len() {
            break;
        }
    }
    names
}
fn shape(model: &Model, body: &Body) -> Result<Option<Collider>, String> {
    if body.disabled {
        return Ok(None);
    }
    let object = if !body.proxy.is_empty() {
        model.object(&body.proxy)
    } else {
        None
    }
    .or_else(|| model.object(&body.name));
    let Some(object) = object.filter(|o| !o.faces.is_empty()) else {
        return Ok(None);
    };
    // Proxies are reusable local geometry, e.g. both saws/teeth share one authored mesh.
    let vertices = object.vertices.clone();
    Ok(Some(
        Collider::trimesh(
            vertices,
            object
                .faces
                .iter()
                .map(|f| f.vertices.map(|i| i as u32))
                .collect(),
        )
        .map_err(|e| format!("{}: {e}", body.name))?,
    ))
}
fn surface_height(shapes: &[(Transform, Collider)], start: Vec3) -> Result<f32, String> {
    let origin = start + Vec3::Y * 0.1;
    shapes
        .iter()
        .filter_map(|(t, c)| c.cast_ray(t.translation, t.rotation, origin, Vec3::NEG_Y, 50., false))
        .map(|d| origin.y - d)
        .max_by(f32::total_cmp)
        .ok_or_else(|| format!("No native collision surface under start {start:?}"))
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn(
    commands: &mut Commands,
    context: Entity,
    hazards: bool,
    descriptor: &Arena,
    catalog: &Catalog,
    tuning: &crate::config::GarageConfig,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> Result<Loaded, String> {
    let root = commands
        .spawn((
            Name::new(format!("Arena / {}", descriptor.name)),
            Transform::default(),
            Visibility::default(),
        ))
        .id();
    let result = spawn_inner(
        commands, context, root, hazards, descriptor, catalog, tuning, meshes, materials, images,
    );
    if result.is_err() {
        commands.entity(root).despawn();
    }
    result
}
#[allow(clippy::too_many_arguments)]
fn spawn_inner(
    commands: &mut Commands,
    context: Entity,
    root: Entity,
    enabled: bool,
    descriptor: &Arena,
    catalog: &Catalog,
    tuning: &crate::config::GarageConfig,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> Result<Loaded, String> {
    let path = catalog
        .index
        .get(&ra2_formats::components::normalize(&descriptor.model))
        .ok_or_else(|| format!("Missing {}", descriptor.model))?;
    let imported = ra2_formats::ModelImporters::with_ra2()
        .decode(&std::fs::read(path).map_err(|e| e.to_string())?)?;
    let dashpots = ra2_formats::scene::angular_dashpots(&imported.report)?;
    let raw = &imported.model;
    let model = ra2_formats::scene::display_groups(raw)?;
    let source = FileTextures::new(path.parent().unwrap());
    let mut cache = TextureCache::default();
    let all = model.all_bodies();
    let movable = all
        .iter()
        .filter(|b| b.mass > 0.)
        .map(|b| b.name.clone())
        .collect::<BTreeSet<_>>();
    let moving_names = all
        .iter()
        .filter(|b| b.mass > 0.)
        .flat_map(|b| display_names(raw, b))
        .collect::<BTreeSet<_>>();
    let mut display = model.clone();
    display.objects.retain(|o| !moving_names.contains(&o.name));
    let visual = render::prepare_model(
        &display,
        &source,
        &RenderSettings::default(),
        &mut cache,
        meshes,
        materials,
        images,
    )?
    .spawn_on_layer(commands, &descriptor.name, Transform::default(), LAYER);
    commands.entity(visual).insert(ChildOf(root));
    let mut entities = BTreeMap::<String, Entity>::new();
    let mut bodies = BTreeMap::new();
    let mut collision_shapes = vec![];
    let mut state = State::default();
    // Visible trim can extend inside the coarse native physics shell. A private
    // ray-only BVH keeps the camera out of it without adding gameplay colliders.
    let mut vertices = vec![];
    let mut triangles = vec![];
    for object in display
        .objects
        .iter()
        .filter(|o| o.visible && !o.faces.is_empty())
    {
        let base = vertices.len() as u32;
        vertices.extend(object.positions());
        let material = &display.materials[&object.material];
        for face in &object.faces {
            let material = material
                .submaterials
                .get(&face.material)
                .unwrap_or(material);
            if material.alpha >= 0.95 && material.opacity_texture.is_none() {
                triangles.push(face.vertices.map(|v| base + v as u32));
            }
        }
    }
    if !triangles.is_empty() {
        state.camera_shell =
            Some(Collider::trimesh(vertices, triangles).map_err(|e| e.to_string())?);
    }
    let mut bounds = None;
    for body in &all {
        let shape = shape(&model, body)?;
        let pose = Transform::from_matrix(body.transform);
        if let Some(collider) = &shape {
            collision_shapes.push((pose, collider.clone()));
            if !body.disabled
                && body.mass <= 0.
                && state.bounds.is_none()
                && let Some(o) = model.object(&body.name)
            {
                // Prefer the arena's named collision shell, not a decorative mesh's extents.
                if o.vertices.len() > 4
                    && (body.name.to_lowercase().contains("collision") || body.name == "h_floor")
                {
                    state.bounds = aabb(o.positions());
                }
            }
        }
        if let Some(o) = model.object(&body.name) {
            if let Some((a, b)) = aabb(o.positions()) {
                bounds = Some(bounds.map_or((a, b), |(c, d): (Vec3, Vec3)| (a.min(c), b.max(d))));
            }
        }
        let mut entity = commands.spawn((
            Name::new(format!("Native arena body / {}", body.name)),
            ChildOf(root),
            pose,
            Velocity::zero(),
            ReadWorldMassProperties::default(),
            if body.mass > 0. && enabled {
                RigidBody::Dynamic
            } else {
                RigidBody::Fixed
            },
            Friction::coefficient(body.friction),
            Restitution::coefficient(body.restitution.min(0.3)),
            CollisionGroups::new(Group::GROUP_1, Group::ALL),
            RapierContextEntityLink(context),
        ));
        if let Some(collider) = shape {
            entity.insert(collider);
        }
        if body.mass > 0. {
            let extent = model
                .object(&body.name)
                .and_then(|o| aabb(o.vertices.iter().copied()))
                .map_or(Vec3::ONE, |(a, b)| b - a)
                .max(Vec3::splat(0.05));
            let mass = body.mass.max(1.);
            let sq = extent * extent;
            entity.insert((
                ColliderMassProperties::MassProperties(MassProperties {
                    mass,
                    principal_inertia: Vec3::new(sq.y + sq.z, sq.x + sq.z, sq.x + sq.y) * mass
                        / 12.,
                    ..default()
                }),
                ExternalImpulse::default(),
                Ccd::enabled(),
            ));
            if descriptor.key == "parkinglot" && body.name.starts_with("cinderblock") {
                entity.insert(RigidBody::Fixed);
                let number = body
                    .name
                    .trim_start_matches("cinderblock")
                    .parse::<usize>()
                    .map_err(|_| "Invalid native block number")?;
                state.debris.push((entity.id(), usize::from(number >= 15)));
            }
        }
        let id = entity.id();
        if !body.disabled {
            let object = (!body.proxy.is_empty())
                .then(|| model.object(&body.proxy))
                .flatten()
                .or_else(|| model.object(&body.name));
            if let Some(object) = object {
                state.ai_obstacles.add_mesh(
                    id,
                    &object.vertices,
                    object.faces.iter().map(|face| face.vertices),
                );
            }
        }
        entities.insert(body.name.clone(), id);
        bodies.insert(body.name.clone(), *body);
        if body.mass > 0. {
            let names = display_names(raw, body);
            let mut part = model.clone();
            part.objects.retain(|o| names.contains(&o.name));
            for o in &mut part.objects {
                o.transform = body.transform.inverse() * o.transform;
            }
            let display = render::prepare_model(
                &part,
                &source,
                &RenderSettings::default(),
                &mut cache,
                meshes,
                materials,
                images,
            )?
            .spawn_on_layer(commands, &body.name, Transform::default(), LAYER);
            commands.entity(display).insert(ChildOf(id));
        }
    }
    state.bounds = state.bounds.or(bounds);
    let mut joints = BTreeMap::<String, Joint>::new();
    for spec in &model.joints {
        let [a, b] = &spec.bodies;
        let (parent, child, axis) = if movable.contains(b) {
            (a, b, spec.axis)
        } else if movable.contains(a) {
            (b, a, -spec.axis)
        } else {
            continue;
        };
        let (Some(&p), Some(&c), Some(pb), Some(cb)) = (
            entities.get(parent),
            entities.get(child),
            bodies.get(parent),
            bodies.get(child),
        ) else {
            return Err(format!("Unresolved native joint {}", spec.name));
        };
        let basis = Quat::from_rotation_arc(Vec3::X, axis.normalize());
        let ball = matches!(spec.kind, JointKind::Ball | JointKind::Ragdoll { .. });
        let (limits, has_limits) = match spec.kind {
            JointKind::Hinge {
                limited, limits, ..
            } => (limits, limited),
            _ => ([-std::f32::consts::PI, std::f32::consts::PI], false),
        };
        let mut joint = GenericJointBuilder::new(if ball {
            JointAxesMask::LOCKED_SPHERICAL_AXES
        } else {
            JointAxesMask::LOCKED_REVOLUTE_AXES
        })
        .local_anchor1(pb.transform.inverse().transform_point3(spec.pivot))
        .local_anchor2(cb.transform.inverse().transform_point3(spec.pivot))
        .local_basis1(Quat::from_mat4(&pb.transform).inverse() * basis)
        .local_basis2(Quat::from_mat4(&cb.transform).inverse() * basis)
        .contacts_enabled(false)
        .build();
        if has_limits {
            joint.set_limits(
                JointAxis::AngX,
                [limits[0].min(limits[1]), limits[0].max(limits[1])],
            );
        }
        if !ball {
            // Native hazards start locked; keep their authored pose through the countdown.
            joint.set_motor_position(JointAxis::AngX, 0., 400., 80.);
            joint.set_motor_max_force(JointAxis::AngX, 20000.);
            if !enabled {
                joint.set_limits(JointAxis::AngX, [0., 0.]);
            }
        }
        commands.entity(c).insert(ImpulseJoint::new(p, joint));
        joints.insert(
            spec.name.clone(),
            Joint {
                entity: c,
                linear: false,
                axis,
                limits,
                force_scale: tuning.motor_force_scale,
            },
        );
    }
    for spec in &descriptor.sliders {
        let (Some(&parent), Some(&child), Some(pb), Some(cb)) = (
            entities.get(&spec.parent),
            entities.get(&spec.child),
            bodies.get(&spec.parent),
            bodies.get(&spec.child),
        ) else {
            return Err(format!("Missing native slider bodies {}", spec.name));
        };
        let axis = spec.axis;
        let basis = Quat::from_rotation_arc(Vec3::X, axis);
        let mut joint = GenericJointBuilder::new(JointAxesMask::LOCKED_PRISMATIC_AXES)
            .local_anchor1(
                pb.transform
                    .inverse()
                    .transform_point3(cb.transform.w_axis.truncate()),
            )
            .local_anchor2(Vec3::ZERO)
            .local_basis1(Quat::from_mat4(&pb.transform).inverse() * basis)
            .local_basis2(Quat::from_mat4(&cb.transform).inverse() * basis)
            .contacts_enabled(false)
            .limits(
                JointAxis::LinX,
                [
                    spec.limits[0].min(spec.limits[1]),
                    spec.limits[0].max(spec.limits[1]),
                ],
            )
            .build();
        if !enabled {
            joint.set_limits(JointAxis::LinX, [0., 0.]);
        }
        commands
            .entity(child)
            .insert(ImpulseJoint::new(parent, joint));
        joints.insert(
            spec.name.clone(),
            Joint {
                entity: child,
                linear: true,
                axis,
                limits: spec.limits,
                force_scale: 1.,
            },
        );
    }
    if enabled {
        for spring in dashpots {
            if let Some(&entity) = entities.get(&spring.bodies[1])
                && let Some(body) = bodies.get(&spring.bodies[1])
            {
                state.springs.push(Spring {
                    entity,
                    rest: Quat::from_mat4(&body.transform),
                    strength: spring.strength,
                    damping: spring.damping,
                    mass: body.mass,
                });
            }
        }
    }
    if let Some(name) = &descriptor.king_zone {
        state.king = Some(Zone::from_object(&model, name)?);
    }
    let mut starts = raw
        .attachments
        .iter()
        .filter(|a| {
            a.properties
                .get("type")
                .is_some_and(|v| v.eq_ignore_ascii_case("startpoint"))
        })
        .collect::<Vec<_>>();
    starts.sort_by_key(|a| a.name.clone());
    if starts.len() < 4 {
        return Err(format!(
            "{} has fewer than four native startpoints",
            descriptor.name
        ));
    }
    let centre = state.bounds.map_or(Vec3::ZERO, |(a, b)| (a + b) * 0.5);
    let starts = starts
        .iter()
        .take(4)
        .map(|a| {
            let mut p = a.transform.w_axis.truncate();
            p.y = surface_height(&collision_shapes, p)?;
            Ok(Transform::from_translation(p)
                .looking_at(Vec3::new(centre.x, p.y, centre.z), Vec3::Y))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let ground = starts
        .iter()
        .map(|s| s.translation.y)
        .fold(f32::INFINITY, f32::min);
    state.ringout_y = descriptor.ringout_y.unwrap_or(ground - 6.);
    let mut traps = vec![];
    if enabled {
        for (index, hazard) in descriptor.hazards.iter().enumerate() {
            let linked = hazard
                .joints
                .iter()
                .map(|n| {
                    joints
                        .get(n)
                        .cloned()
                        .ok_or_else(|| format!("Unknown arena joint {n}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let zones = hazard
                .zones
                .iter()
                .map(|n| Zone::from_object(&model, n))
                .collect::<Result<Vec<_>, _>>()?;
            let buttons = hazard
                .buttons
                .iter()
                .map(|n| Zone::from_object(&model, n))
                .collect::<Result<Vec<_>, _>>()?;
            let position = linked
                .first()
                .and_then(|j| {
                    all.iter()
                        .find(|b| entities.get(&b.name) == Some(&j.entity))
                })
                .map_or(hazard.location, |b| b.transform.w_axis.truncate());
            let sound = linked
                .first()
                .and_then(|j| entities.iter().find(|(_, e)| **e == j.entity))
                .and_then(|(n, _)| descriptor.sounds.get(n))
                .cloned()
                .unwrap_or_else(|| {
                    match hazard.kind {
                        HazardKind::Saw => "Sounds/sawblade_hits_loop.wav",
                        HazardKind::Spikes => "Sounds/hzd_spike_hit.wav",
                        HazardKind::Hammer | HazardKind::HellRaiser => "Sounds/hzd_hammer_thud.wav",
                        HazardKind::Smasher => "Sounds/hzd_comp_hit.wav",
                        _ => "Sounds/steel_collision2.wav",
                    }
                    .into()
                });
            let mut trap = Trap::new(
                hazard.clone(),
                linked,
                zones,
                buttons,
                position,
                sound,
                index,
            );
            if matches!(hazard.kind, HazardKind::Flame | HazardKind::Electricity) {
                super::hazards::effects(
                    commands, root, &mut trap, catalog, meshes, materials, images,
                )?;
            }
            traps.push(trap);
        }
    }
    info!(
        "Arena {}: {} bodies, {} native joints, {} hazards, four startpoints",
        descriptor.name,
        all.len(),
        joints.len(),
        traps.len()
    );
    Ok(Loaded {
        entities: vec![root],
        traps,
        starts,
        state,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{ecs::system::RunSystemOnce, time::TimeUpdateStrategy};

    /// Run with:
    /// RA2_GAME_DIR='/path/to/Robot Arena 2' cargo test --bin physics-demo \
    ///   native_ai_obstacle_geometry -- --ignored --nocapture
    #[test]
    #[ignore = "requires original arena assets in RA2_GAME_DIR"]
    fn native_ai_obstacle_geometry() {
        use std::time::Instant;

        let root = std::env::var_os("RA2_GAME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../RobotArena2")
            });
        let arenas = crate::content::arenas::load(&root)
            .unwrap_or_else(|e| panic!("Cannot load {}: {e}; set RA2_GAME_DIR", root.display()));
        let mut files = vec![];
        crate::content::scan(&root.join("Arenas"), &mut files).unwrap();
        let index = files
            .into_iter()
            .map(|p| {
                (
                    ra2_formats::components::normalize(
                        &p.strip_prefix(&root).unwrap().to_string_lossy(),
                    ),
                    p,
                )
            })
            .collect::<BTreeMap<_, _>>();

        for key in ["compressor", "bridge"] {
            let descriptor = arenas
                .iter()
                .find(|a| a.key == key)
                .unwrap_or_else(|| panic!("missing arena {key}"));
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, TransformPlugin))
                .init_resource::<Assets<Mesh>>()
                .init_resource::<Assets<StandardMaterial>>()
                .init_resource::<Assets<Image>>()
                .insert_resource(Catalog {
                    root: root.clone(),
                    index: index.clone(),
                    ..default()
                });
            let context = app.world_mut().spawn_empty().id();
            let arena = descriptor.clone();
            let loaded = app
                .world_mut()
                .run_system_once(
                    move |mut commands: Commands,
                          catalog: Res<Catalog>,
                          mut meshes: ResMut<Assets<Mesh>>,
                          mut materials: ResMut<Assets<StandardMaterial>>,
                          mut images: ResMut<Assets<Image>>| {
                        spawn(
                            &mut commands,
                            context,
                            true,
                            &arena,
                            &catalog,
                            &crate::config::GarageConfig::default(),
                            &mut meshes,
                            &mut materials,
                            &mut images,
                        )
                    },
                )
                .unwrap()
                .unwrap_or_else(|e| panic!("{key}: {e}"));
            let (min, max) = loaded.state.bounds.expect("arena bounds");
            let centre = (min + max) * 0.5;
            let floor = loaded.starts[0].translation.y;
            let range = (max - min).length();
            let now = Instant::now();
            let obstacles = loaded.state.ai_obstacles.nearby(
                Vec3::new(centre.x, floor, centre.z),
                1.0,
                range,
                |entity| app.world().get::<Transform>(entity).copied(),
            );
            let scan_time = now.elapsed();
            if key == "bridge" {
                let from = Vec3::new(0.0, -2.0, 6.5);
                let to = Vec3::new(0.0, 0.0, 0.0);
                let step = loaded
                    .state
                    .ai_obstacles
                    .height_waypoint(from, to, |entity| {
                        app.world().get::<Transform>(entity).copied()
                    });
                assert!(step.is_some(), "bridge: no path from lower floor to bridge");
            }
            // Known interior collision surfaces in the original GMFs, not the
            // surrounding shell: the bridge rail and compressor's inner wall.
            let probe = if key == "bridge" {
                Vec2::new(2.5, -2.471927)
            } else {
                Vec2::new(13.802465, 5.0)
            };
            let wall = *obstacles
                .iter()
                .find(|r| {
                    let min = Vec2::from_array(r.min) - Vec2::splat(0.01);
                    let max = Vec2::from_array(r.max) + Vec2::splat(0.01);
                    probe.cmpge(min).all() && probe.cmple(max).all()
                })
                .unwrap_or_else(|| panic!("{key}: missing interior wall at {probe:?}"));
            println!(
                "{key}: floor={floor:.2}, obstacles={}, scan={scan_time:?}, interior={wall:?}",
                obstacles.len()
            );
            let a = Vec2::from_array(wall.min);
            let b = Vec2::from_array(wall.max);
            let mid = (a + b) * 0.5;
            let axis = if b.x - a.x < b.y - a.y {
                Vec2::X
            } else {
                Vec2::Y
            };
            let reach = (b - a).dot(axis).abs() * 0.5 + 2.0;
            let (start, goal) = (mid - axis * reach, mid + axis * reach);
            let now = Instant::now();
            let next = ai_obstacles::waypoint(start, goal, &[wall], 0.5);
            let route_time = now.elapsed();
            assert!(
                next.is_some_and(|p| p != goal),
                "{key}: cannot route around real obstacle {wall:?}"
            );
            let repeated = vec![wall; 129];
            let now = Instant::now();
            assert!(ai_obstacles::waypoint(start, goal, &repeated[..128], 0.5).is_some());
            let at_limit_time = now.elapsed();
            let now = Instant::now();
            assert!(ai_obstacles::waypoint(start, goal, &repeated, 0.5).is_none());
            let limit_time = now.elapsed();
            let clear_start = mid + axis * (reach + 3.0);
            let clear_goal = mid + axis * (reach + 5.0);
            assert_eq!(
                ai_obstacles::waypoint(clear_start, clear_goal, &repeated, 0.5),
                Some(clear_goal)
            );
            let now = Instant::now();
            let full_result = ai_obstacles::waypoint(start, goal, &obstacles, 0.5);
            if key == "bridge" {
                assert!(
                    full_result.is_some_and(|p| p != goal),
                    "bridge: full scene has no detour"
                );
            }
            println!(
                "{key}: single-wall={route_time:?}, 128 repeated={at_limit_time:?}, 129 blocked={limit_time:?}, full scene={:?} result={full_result:?}",
                now.elapsed()
            );
        }
    }
    /// Exercise native frames, proxies, hidden colliders, four starts and cleanup,
    /// including both the articulated and hazards-disabled variants, without a GPU.
    #[test]
    #[ignore = "requires installed original game"]
    fn all_native_arenas_support_four_robots_and_clean_up() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../RobotArena2");
        let arenas = crate::content::arenas::load(&root).unwrap();
        let mut files = vec![];
        crate::content::scan(&root.join("Arenas"), &mut files).unwrap();
        let index = files
            .into_iter()
            .map(|p| {
                (
                    ra2_formats::components::normalize(
                        &p.strip_prefix(&root).unwrap().to_string_lossy(),
                    ),
                    p,
                )
            })
            .collect::<BTreeMap<_, _>>();
        for descriptor in arenas {
            for enabled in [true, false] {
                let mut app = App::new();
                app.add_plugins((MinimalPlugins, TransformPlugin));
                app.insert_resource(Time::<Fixed>::from_seconds(crate::STEP))
                    .insert_resource(TimeUpdateStrategy::ManualDuration(
                        std::time::Duration::from_secs_f64(crate::STEP),
                    ))
                    .insert_resource(TimestepMode::Fixed {
                        dt: crate::STEP as f32,
                        substeps: 1,
                    })
                    .add_plugins(RapierPhysicsPlugin::<NoUserData>::default().in_fixed_schedule())
                    .init_resource::<Assets<Mesh>>()
                    .init_resource::<Assets<StandardMaterial>>()
                    .init_resource::<Assets<Image>>()
                    .insert_resource(Catalog {
                        root: root.clone(),
                        index: index.clone(),
                        ..default()
                    })
                    .init_resource::<Session>();
                let context = app
                    .world_mut()
                    .spawn((
                        RapierContextSimulation::default(),
                        RapierConfiguration::new(1.),
                    ))
                    .id();
                let arena = descriptor.clone();
                let loaded = app
                    .world_mut()
                    .run_system_once(
                        move |mut commands: Commands,
                              catalog: Res<Catalog>,
                              mut meshes: ResMut<Assets<Mesh>>,
                              mut materials: ResMut<Assets<StandardMaterial>>,
                              mut images: ResMut<Assets<Image>>| {
                            spawn(
                                &mut commands,
                                context,
                                enabled,
                                &arena,
                                &catalog,
                                &crate::config::GarageConfig::default(),
                                &mut meshes,
                                &mut materials,
                                &mut images,
                            )
                        },
                    )
                    .unwrap()
                    .unwrap_or_else(|e| panic!("{} hazards={enabled}: {e}", descriptor.name));
                assert_eq!(loaded.starts.len(), 4);
                assert_eq!(
                    loaded.traps.len(),
                    if enabled { descriptor.hazards.len() } else { 0 }
                );
                assert_eq!(loaded.state.king.is_some(), descriptor.king_zone.is_some());
                let mut probes = vec![];
                for start in &loaded.starts {
                    assert!(start.translation.is_finite());
                    assert!(start.translation.y > loaded.state.ringout_y);
                    let p = start.translation + Vec3::Y * 0.22;
                    let e = app
                        .world_mut()
                        .spawn((
                            RigidBody::Dynamic,
                            Collider::ball(0.2),
                            Transform::from_translation(p),
                            Velocity::zero(),
                            RapierContextEntityLink(context),
                        ))
                        .id();
                    probes.push((e, start.translation.y));
                }
                {
                    let mut session = app.world_mut().resource_mut::<Session>();
                    session.context = context;
                    session.arena_state = loaded.state;
                    session.hazards_runtime = loaded.traps;
                    session.phase = Phase::Playing;
                }
                app.add_systems(
                    FixedUpdate,
                    super::super::hazards::tick.after(PhysicsSet::Writeback),
                );
                for _ in 0..100 {
                    app.update();
                }
                for (entity, floor) in &probes {
                    let p = app.world().get::<Transform>(*entity).unwrap().translation;
                    assert!(
                        p.is_finite() && (p.y - floor - 0.2).abs() < 0.1,
                        "{} hazards={enabled}: start floor={floor}, probe={p:?}",
                        descriptor.name
                    );
                }
                let mut actors = vec![];
                if enabled {
                    let zones = app
                        .world()
                        .resource::<Session>()
                        .hazards_runtime
                        .iter()
                        .filter_map(|t| t.zones.first())
                        .map(|z| (z.min + z.max) * 0.5)
                        .collect::<Vec<_>>();
                    for (i, position) in zones.into_iter().enumerate() {
                        let actor = app
                            .world_mut()
                            .spawn(Transform::from_translation(position))
                            .id();
                        actors.push(actor);
                        let mut robot = RobotInstance::new(i as u32 + 1, context, default());
                        robot.bodies.push(actor);
                        robot.running = true;
                        app.world_mut().resource_mut::<Session>().robots.push(robot);
                    }
                    let watched = app
                        .world()
                        .resource::<Session>()
                        .hazards_runtime
                        .iter()
                        .flat_map(|t| t.joints.iter().map(move |j| (t.kind, j.entity)))
                        .collect::<Vec<_>>();
                    let rest = watched
                        .iter()
                        .map(|(_, e)| *app.world().get::<Transform>(*e).unwrap())
                        .collect::<Vec<_>>();
                    let mut movement = vec![0.0f32; watched.len()];
                    for t in &mut app.world_mut().resource_mut::<Session>().hazards_runtime {
                        if t.kind == Kind::PitSlider {
                            t.clock = t.spec.delay;
                        }
                    }
                    for _ in 0..480 {
                        app.update();
                        for (i, (_, entity)) in watched.iter().enumerate() {
                            let pose = app.world().get::<Transform>(*entity).unwrap();
                            movement[i] = movement[i].max(
                                pose.translation.distance(rest[i].translation)
                                    + pose.rotation.angle_between(rest[i].rotation).abs(),
                            );
                        }
                    }
                    for (i, (kind, _)) in watched.iter().enumerate() {
                        assert!(
                            movement[i] > 0.05,
                            "{} {kind:?} joint {i} did not actuate: {}",
                            descriptor.name,
                            movement[i]
                        );
                    }
                    for t in &app.world().resource::<Session>().hazards_runtime {
                        if matches!(t.kind, Kind::Flame | Kind::Electricity) {
                            assert!(t.clock > 0. && !t.fx.is_empty());
                        }
                    }
                }
                for actor in actors {
                    app.world_mut().despawn(actor);
                }
                for entity in loaded.entities {
                    app.world_mut().despawn(entity);
                }
                for (entity, _) in probes {
                    app.world_mut().despawn(entity);
                }
                {
                    let mut s = app.world_mut().resource_mut::<Session>();
                    s.phase = Phase::Setup;
                    s.hazards_runtime.clear();
                    s.arena_state = default();
                }
                app.update();
                assert_eq!(
                    app.world_mut()
                        .query::<&Collider>()
                        .iter(app.world())
                        .count(),
                    0,
                    "{} collider leaked",
                    descriptor.name
                );
                println!(
                    "PASS: {} / hazards={enabled} / four supported starts and cleanup",
                    descriptor.name
                );
            }
        }
    }
}
