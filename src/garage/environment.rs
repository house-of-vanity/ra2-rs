//! Original practice arena and obstacles; independent from robot actuation.
use super::*;
use crate::editor::catalog::Catalog;
use crate::gameplay::spawn::read;
use arena_runtime::render::{self, RenderSettings, TextureCache};
/// The ground frame is supplied by the scene, never inferred by robot code.
pub(crate) fn start(catalog: &Catalog) -> Result<Transform, String> {
    let (arena, _) = read(catalog, "Arenas/practice arena/practice_arena.gmf")?;
    let start = arena
        .attachments
        .iter()
        .find(|a| {
            a.properties
                .get("type")
                .is_some_and(|v| v.eq_ignore_ascii_case("startpoint"))
        })
        .ok_or("Garage start point is missing")?
        .transform
        .w_axis
        .truncate();
    let floor = arena
        .object("h_box")
        .and_then(|o| o.positions().map(|p| p.y).reduce(f32::min))
        .ok_or("Garage floor is missing")?;
    Ok(Transform::from_xyz(start.x, floor, start.z))
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn environment(
    commands: &mut Commands,
    garage: &RobotInstance,
    catalog: &Catalog,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> Result<Vec<Entity>, String> {
    let (model, source) = read(catalog, "Arenas/practice arena/practice_arena.gmf")?;
    let model = ra2_formats::scene::display_groups(&model)?;
    let mut cache = TextureCache::default();
    let visual = render::prepare_model(
        &model,
        &source,
        &RenderSettings::default(),
        &mut cache,
        meshes,
        materials,
        images,
    )?
    .spawn_on_layer(commands, "Original test garage", Transform::default(), 9);
    let boundary = model
        .object("h_box")
        .ok_or("Garage boundary mesh is missing")?;
    let collider = Collider::trimesh(
        boundary.positions().collect(),
        boundary
            .faces
            .iter()
            .map(|f| f.vertices.map(|i| i as u32))
            .collect(),
    )
    .map_err(|e| e.to_string())?;
    let collision = commands
        .spawn((
            Name::new("Garage walls and floor"),
            RigidBody::Fixed,
            ReadWorldMassProperties::default(),
            collider,
            Transform::default(),
            Friction::coefficient(model.bodies.first().map_or(0.3, |b| b.friction)),
            CollisionGroups::new(Group::GROUP_1, Group::ALL),
            RapierContextEntityLink(garage.context),
        ))
        .id();
    Ok(vec![visual, collision])
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn obstacles(
    commands: &mut Commands,
    garage: &RobotInstance,
    catalog: &Catalog,
    path: &str,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> Result<Vec<Entity>, String> {
    if path.is_empty() {
        return Ok(vec![]);
    }
    let (model, source) = read(catalog, path)?;
    let mut cache = TextureCache::default();
    let mut entities = vec![];
    for body in &model.bodies {
        let mut names = vec![body.name.to_ascii_lowercase()];
        if !body.display_proxy.is_empty() {
            names.push(body.display_proxy.to_ascii_lowercase());
        }
        loop {
            let before = names.len();
            for o in &model.objects {
                if !names.contains(&o.name.to_ascii_lowercase())
                    && o.parent
                        .as_ref()
                        .is_some_and(|p| names.contains(&p.to_ascii_lowercase()))
                {
                    names.push(o.name.to_ascii_lowercase());
                }
            }
            if before == names.len() {
                break;
            }
        }
        let mut display = model.clone();
        display
            .objects
            .retain(|o| names.contains(&o.name.to_ascii_lowercase()));
        let position = body.transform.w_axis.truncate();
        let inverse = Mat4::from_translation(-position);
        for o in &mut display.objects {
            o.transform = inverse * o.transform;
        }
        let mut vertices = display
            .objects
            .iter()
            .filter(|o| o.visible)
            .flat_map(|o| o.positions())
            .collect::<Vec<_>>();
        if vertices.len() < 4
            && let Some(proxy) = model.object(&body.proxy)
        {
            vertices = proxy.positions().map(|p| p - position).collect();
        }
        let Some(collider) = Collider::convex_hull(&vertices) else {
            continue;
        };
        let entity = commands
            .spawn((
                Name::new(format!("Obstacle / {}", body.name)),
                ReadWorldMassProperties::default(),
                if body.mass > 0.0 {
                    RigidBody::Dynamic
                } else {
                    RigidBody::Fixed
                },
                collider,
                ColliderMassProperties::Mass(body.mass.max(1.0)),
                Transform::from_translation(position),
                Velocity::zero(),
                Ccd::enabled(),
                Friction::coefficient(body.friction.max(0.4)),
                CollisionGroups::new(Group::GROUP_1, Group::ALL),
                RapierContextEntityLink(garage.context),
            ))
            .id();
        let visual = render::prepare_model(
            &display,
            &source,
            &RenderSettings::default(),
            &mut cache,
            meshes,
            materials,
            images,
        )?
        .spawn_on_layer(commands, "Obstacle display", Transform::default(), 9);
        commands.entity(visual).insert(ChildOf(entity));
        entities.push(entity);
    }
    info!("Garage obstacles: {path}, {} rigid bodies", entities.len());
    Ok(entities)
}
