//! Native hazard state machines over common rigid bodies and original particle textures.
use super::arena::{Joint, Zone};
use super::*;
use crate::content::arenas::Hazard;
pub(crate) use crate::content::arenas::HazardKind as Kind;
use bevy::asset::RenderAssetUsages;
#[derive(Component)]
pub(crate) struct HazardVisual;
pub(crate) struct Trap {
    pub kind: Kind,
    pub entity: Entity,
    pub min: Vec3,
    pub max: Vec3,
    pub spec: Hazard,
    pub joints: Vec<Joint>,
    pub zones: Vec<Zone>,
    pub buttons: Vec<Zone>,
    pub clock: f32,
    pub occupied: bool,
    pub active: bool,
    pub sound_position: Vec3,
    pub hit_sound: String,
    damage_clock: f32,
    phase: u8,
    phase_clock: f32,
    cooldown: f32,
    zap_time: f32,
    pub zapping: Vec<u32>,
    pub fx: Vec<Entity>,
    pub index: usize,
    button_occupied: bool,
}
impl Trap {
    pub fn new(
        spec: Hazard,
        joints: Vec<Joint>,
        zones: Vec<Zone>,
        buttons: Vec<Zone>,
        position: Vec3,
        hit_sound: String,
        index: usize,
    ) -> Self {
        let entity = joints.first().map_or(Entity::PLACEHOLDER, |j| j.entity);
        let (min, max) = zones
            .first()
            .map_or((position, position), |z| (z.min, z.max));
        Self {
            kind: spec.kind,
            entity,
            min,
            max,
            spec,
            joints,
            zones,
            buttons,
            clock: 0.,
            occupied: false,
            active: false,
            sound_position: position,
            hit_sound,
            damage_clock: 0.,
            phase: 0,
            phase_clock: 0.,
            cooldown: 0.,
            zap_time: 0.,
            zapping: vec![],
            fx: vec![],
            index,
            button_occupied: false,
        }
    }
    pub fn loop_sound(&self) -> Option<&'static str> {
        match self.kind {
            Kind::Flame if self.active => Some("Sounds/flame_thrower.wav"),
            Kind::Electricity if self.active => Some("Sounds/zap_loop.wav"),
            Kind::Rotator => Some("Sounds/hzd_rotclaw_loop.wav"),
            Kind::PitSlider if self.active => Some("Sounds/hzd_trapfloor_loop.wav"),
            _ => None,
        }
    }
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn effects(
    commands: &mut Commands,
    root: Entity,
    trap: &mut Trap,
    catalog: &crate::editor::catalog::Catalog,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> Result<(), String> {
    let flame = trap.kind == Kind::Flame;
    let texture = if flame {
        let path = catalog
            .index
            .get("arenas/particles/flame.tga")
            .ok_or("Missing original flame texture")?;
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        Some(
            images.add(
                Image::from_buffer(
                    &bytes,
                    bevy::image::ImageType::Extension("tga"),
                    bevy::image::CompressedImageFormats::NONE,
                    true,
                    bevy::image::ImageSampler::linear(),
                    RenderAssetUsages::default(),
                )
                .map_err(|e| e.to_string())?,
            ),
        )
    } else {
        None
    };
    let mesh = if flame {
        meshes.add(Rectangle::new(1., 1.))
    } else {
        meshes.add(Cylinder::new(0.025, 1.).mesh().resolution(5))
    };
    let material = materials.add(StandardMaterial {
        base_color: if flame {
            Color::WHITE
        } else {
            Color::srgb(0.15, 0.5, 1.)
        },
        base_color_texture: texture,
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        cull_mode: None,
        emissive: if flame {
            LinearRgba::rgb(0.25, 0.1, 0.)
        } else {
            LinearRgba::rgb(0.4, 0.8, 3.)
        },
        ..default()
    });
    for _ in 0..16 {
        trap.fx.push(
            commands
                .spawn((
                    HazardVisual,
                    ChildOf(root),
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::default(),
                    Visibility::Hidden,
                    RenderLayers::layer(LAYER),
                ))
                .id(),
        );
    }
    Ok(())
}
fn motor(joint: &Joint, query: &mut Query<&mut ImpulseJoint>, speed: f32, force: f32, hold: bool) {
    if let Ok(mut target) = query.get_mut(joint.entity) {
        let axis = if joint.linear {
            JointAxis::LinX
        } else {
            JointAxis::AngX
        };
        let data = target.data.as_mut();
        data.set_motor_model(axis, MotorModel::AccelerationBased);
        if hold {
            data.set_motor_position(axis, 0., 400., 80.);
        } else {
            data.set_motor_velocity(axis, speed, 40.);
        }
        data.set_motor_max_force(axis, force * joint.force_scale);
    }
}
fn flame_hit(
    robots: &[RobotInstance],
    poses: &Query<&Transform>,
    origin: Vec3,
    direction: Vec3,
    distance: f32,
) -> Option<(usize, usize, Vec3)> {
    let mut nearest = None;
    let mut best = distance;
    for (robot, r) in robots.iter().enumerate().filter(|(_, r)| r.running) {
        for (body, parts) in r.damage.bodies.iter().enumerate() {
            let Some(pose) = r.bodies.get(body).and_then(|e| poses.get(*e).ok()) else {
                continue;
            };
            for solid in &parts.solids {
                let position = pose.transform_point(solid.position);
                let rotation = pose.rotation * solid.rotation;
                if let Some(distance) = solid
                    .collider
                    .cast_ray(position, rotation, origin, direction, best, false)
                {
                    best = distance;
                    nearest = Some((robot, solid.component, origin + direction * distance));
                }
            }
        }
    }
    nearest
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn tick(
    mut commands: Commands,
    mut session: ResMut<Session>,
    poses: Query<&Transform>,
    contexts: Query<(
        &RapierContextSimulation,
        &RapierContextColliders,
        &RapierRigidBodySet,
    )>,
    mut joints: Query<&mut ImpulseJoint>,
    mut impulses: Query<&mut ExternalImpulse>,
    velocities: Query<&Velocity>,
) {
    if !matches!(session.phase, Phase::Playing)
        || session.network && (!session.authority || session.net_paused || session.net_waiting)
    {
        return;
    }
    let dt = crate::STEP as f32;
    let positions = session
        .robots
        .iter()
        .filter_map(|r| {
            r.bodies
                .first()
                .and_then(|e| poses.get(*e).ok())
                .map(|p| (r.id, p.translation, r.running))
        })
        .collect::<Vec<_>>();
    let context = session.context;
    let mut sounds = vec![];
    let Session {
        robots,
        hazards_runtime,
        arena_state,
        ..
    } = &mut *session;
    for spring in &arena_state.springs {
        if let (Ok(pose), Ok(mut impulse)) =
            (poses.get(spring.entity), impulses.get_mut(spring.entity))
        {
            let rotation = (spring.rest * pose.rotation.inverse()).to_scaled_axis();
            // Convert Havok angular-dashpot strength to this solver's force units.
            let stiffness = spring.strength * spring.mass * 0.1;
            let velocity = velocities
                .get(spring.entity)
                .map_or(Vec3::ZERO, |v| v.angular);
            impulse.torque_impulse += (rotation * stiffness
                - velocity * 2. * stiffness.sqrt() * spring.mass.sqrt() * (spring.damping / 0.08))
                * dt;
        }
    }
    for trap in hazards_runtime {
        let members = positions
            .iter()
            .filter(|(_, p, running)| *running && trap.zones.iter().any(|z| z.contains(*p)))
            .map(|(id, _, _)| *id)
            .collect::<std::collections::BTreeSet<_>>();
        let occupied = !members.is_empty();
        let entering = occupied && !trap.occupied;
        let button = positions
            .iter()
            .any(|(_, p, r)| *r && trap.buttons.iter().any(|z| z.contains(*p)));
        let button_enter = button && !trap.button_occupied;
        trap.clock += dt;
        trap.phase_clock += dt;
        trap.cooldown = (trap.cooldown - dt).max(0.);
        trap.damage_clock += dt;
        match trap.kind {
            Kind::Spikes => {
                if entering {
                    trap.clock = 0.;
                }
                let cycle = trap.clock % 2.5;
                if occupied && cycle >= 1. && cycle - dt < 1. {
                    for joint in &trap.joints {
                        if let Ok(mut impulse) = impulses.get_mut(joint.entity) {
                            impulse.impulse += joint.axis * trap.spec.power * dt;
                        }
                    }
                    sounds.push(("Sounds/hzd_spike_fire.wav".into(), trap.sound_position));
                }
            }
            Kind::Saw => {
                for joint in &trap.joints {
                    motor(
                        joint,
                        &mut joints,
                        if occupied { 2. } else { -2. },
                        2000.,
                        false,
                    );
                }
            }
            Kind::PitSlider => {
                trap.active = trap.clock >= trap.spec.delay && trap.clock < trap.spec.delay + 16.;
                for joint in &trap.joints {
                    motor(
                        joint,
                        &mut joints,
                        if trap.clock >= trap.spec.delay {
                            (joint.limits[1] - joint.limits[0]).signum() * trap.spec.speed
                        } else {
                            0.
                        },
                        trap.spec.power,
                        false,
                    );
                }
            }
            Kind::Rotator => {
                trap.active = true;
                for j in &trap.joints {
                    motor(j, &mut joints, -2., 1000., false);
                }
            }
            Kind::Hammer | Kind::HellRaiser | Kind::Smasher => {
                if entering {
                    trap.phase = 1;
                    trap.phase_clock = 0.;
                    sounds.push((
                        match trap.kind {
                            Kind::HellRaiser => "Sounds/hellraiser_trigger.wav",
                            Kind::Smasher => "Sounds/hzd_comp_fire.wav",
                            _ => "Sounds/hzd_hammer_fire.wav",
                        }
                        .into(),
                        trap.sound_position,
                    ));
                }
                let out_time = if trap.kind == Kind::Smasher { 2.5 } else { 1.5 };
                let back_time = if trap.kind == Kind::Smasher { 3.5 } else { 3. };
                if trap.phase == 1
                    && (trap.phase_clock >= out_time
                        || (!occupied && trap.kind != Kind::HellRaiser))
                {
                    trap.phase = 2;
                    trap.phase_clock = 0.;
                }
                if trap.phase == 2 && trap.phase_clock >= back_time {
                    trap.phase = if occupied && trap.kind != Kind::HellRaiser {
                        1
                    } else {
                        0
                    };
                    trap.phase_clock = 0.;
                    if trap.phase == 1 {
                        sounds.push((
                            if trap.kind == Kind::Smasher {
                                "Sounds/hzd_comp_fire.wav"
                            } else {
                                "Sounds/hzd_hammer_fire.wav"
                            }
                            .into(),
                            trap.sound_position,
                        ));
                    }
                }
                let (speed, force) = match (trap.kind, trap.phase) {
                    (Kind::HellRaiser, 1) => (80., 1500.),
                    (Kind::HellRaiser, 2) => (-4., 200.),
                    (Kind::Smasher, 1) => (95., 15000.),
                    (Kind::Smasher, 2) => (-12., 700.),
                    (_, 1) => (95., 1200.),
                    (_, 2) => (-3., 300.),
                    _ => (0., 20000.),
                };
                for (i, j) in trap.joints.iter().enumerate() {
                    motor(
                        j,
                        &mut joints,
                        speed
                            * if trap.kind == Kind::Smasher && i == 1 {
                                -1.
                            } else {
                                1.
                            },
                        force,
                        trap.phase == 0,
                    );
                }
            }
            Kind::TrapDoor => {
                if entering && trap.phase == 0 {
                    trap.phase = 1;
                    sounds.push(("Sounds/hzd_trapdoor.wav".into(), trap.sound_position));
                    for &(e, group) in &arena_state.debris {
                        if group == trap.index {
                            commands.entity(e).insert(RigidBody::Dynamic);
                        }
                    }
                    for j in &trap.joints {
                        motor(j, &mut joints, 0., 0., false);
                        if let Ok(mut impulse) = impulses.get_mut(j.entity) {
                            impulse.torque_impulse += j.axis * 300.;
                        }
                    }
                } else if trap.phase == 0 {
                    for j in &trap.joints {
                        motor(j, &mut joints, 0., 20000., true);
                    }
                }
            }
            Kind::Flame => {
                if entering {
                    trap.clock = 0.;
                }
                trap.active = occupied;
                if occupied && trap.damage_clock >= 0.25 {
                    let origin = trap.spec.location + Vec3::Y * trap.spec.ray_offset;
                    if let Some((robot, part, point)) = flame_hit(
                        robots,
                        &poses,
                        origin,
                        trap.spec.velocity.normalize_or_zero(),
                        trap.spec.velocity.length() * trap.clock.min(1.),
                    ) {
                        robots[robot].damage.pending.push((part, 25., point));
                    }
                }
            }
            Kind::Electricity => {
                if trap.cooldown <= 0.
                    && (members.len() == 1 || button_enter)
                    && !members.is_empty()
                {
                    trap.zapping = members.iter().copied().collect();
                    trap.zap_time = 3.;
                    trap.cooldown = 5.;
                    sounds.push(("Sounds/zap_charge2.wav".into(), trap.spec.location));
                }
                trap.zap_time = (trap.zap_time - dt).max(0.);
                trap.active = trap.zap_time > 0.;
                if trap.active && trap.damage_clock >= 0.25 {
                    for r in robots
                        .iter_mut()
                        .filter(|r| r.running && trap.zapping.contains(&r.id))
                    {
                        if let Some(&body) = r.bodies.first()
                            && let Ok(p) = poses.get(body)
                        {
                            r.damage.pending.push((0, 25., p.translation));
                            if members.contains(&r.id)
                                && let Ok(mut impulse) = impulses.get_mut(body)
                            {
                                let inverse = r.damage.connected_properties().effective_inv_mass.x;
                                if inverse > 0. {
                                    impulse.impulse += Vec3::Y * 115. / inverse * dt;
                                }
                            }
                        }
                    }
                }
            }
        }
        if trap.damage_clock >= 0.25 {
            if matches!(
                trap.kind,
                Kind::Saw
                    | Kind::Spikes
                    | Kind::Hammer
                    | Kind::HellRaiser
                    | Kind::Smasher
                    | Kind::Rotator
            ) && let Ok((simulation, colliders, bodies)) = contexts.get(context)
            {
                let trap_bodies = trap.joints.iter().map(|j| j.entity).collect::<Vec<_>>();
                let mut hit = std::collections::BTreeSet::new();
                for pair in simulation
                    .contact_pairs(colliders, bodies)
                    .filter(|p| p.has_any_active_contact())
                {
                    let (Some(a), Some(b)) = (pair.collider1(), pair.collider2()) else {
                        continue;
                    };
                    let other = if trap_bodies.contains(&a) {
                        b
                    } else if trap_bodies.contains(&b) {
                        a
                    } else {
                        continue;
                    };
                    if let Some(r) = robots
                        .iter_mut()
                        .find(|r| r.running && r.bodies.contains(&other))
                        && hit.insert(r.id)
                        && let Some(point) = r
                            .bodies
                            .first()
                            .and_then(|e| poses.get(*e).ok())
                            .map(|p| p.translation - Vec3::Y * 0.3)
                    {
                        if matches!(trap.kind, Kind::Saw | Kind::Rotator) {
                            r.damage.pending.push((0, 3., point));
                        }
                        sounds.push((trap.hit_sound.clone(), point));
                    }
                }
            }
            trap.damage_clock %= 0.25;
        }
        trap.occupied = occupied;
        trap.button_occupied = button;
    }
    session.sound_events.extend(sounds);
}
/// Reuse small fixed pools; no particle physics or per-frame entity churn.
pub(crate) fn animate(
    session: Res<Session>,
    nav: Res<Navigation>,
    camera: Query<&Transform, (With<MatchCamera>, Without<HazardVisual>)>,
    poses: Query<&Transform, (With<RobotBody>, Without<HazardVisual>)>,
    mut fx: Query<(&mut Transform, &mut Visibility), With<HazardVisual>>,
) {
    let Ok(camera) = camera.get(session.camera) else {
        return;
    };
    for trap in &session.hazards_runtime {
        for (i, e) in trap.fx.iter().enumerate() {
            if let Ok((mut pose, mut visible)) = fx.get_mut(*e) {
                *visible = if trap.active
                    && matches!(
                        nav.page,
                        Page::Match | Page::Pause | Page::Results | Page::Settings
                    ) {
                    Visibility::Visible
                } else {
                    Visibility::Hidden
                };
                if trap.kind == Kind::Flame {
                    let t = (trap.clock * 1.6 + i as f32 / 16.) % 1.;
                    let size = 0.2 + 0.55 * t;
                    pose.translation = trap.spec.location
                        + trap.spec.velocity * t * trap.clock.min(1.)
                        + Vec3::new((i as f32 * 2.3).sin(), 0., (i as f32 * 1.7).cos()) * t * 0.18;
                    pose.rotation = camera.rotation;
                    pose.scale = Vec3::splat(size * (1. - t * 0.5));
                } else if !trap.zapping.is_empty() {
                    let target = trap.zapping[(i / 4) % trap.zapping.len()];
                    if let Some(p) = session
                        .robots
                        .iter()
                        .find(|r| r.id == target)
                        .and_then(|r| r.bodies.first())
                        .and_then(|e| poses.get(*e).ok())
                    {
                        let from = trap.spec.location + Vec3::Y * 0.2;
                        let to = p.translation + Vec3::Y * 0.3;
                        let phase = (i % 4) as f32 / 4.;
                        let step = (to - from) / 4.;
                        let jitter = Vec3::Y * (trap.clock * 60. + i as f32).sin() * 0.12;
                        pose.translation = from + (to - from) * phase + step * 0.5 + jitter;
                        pose.rotation = Quat::from_rotation_arc(Vec3::Y, step.normalize_or_zero());
                        pose.scale = Vec3::new(1., step.length(), 1.);
                    }
                }
            }
        }
    }
}
