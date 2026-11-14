//! Translation of neutral geometry into Rapier. Scene policy selects the surface by name.
use arena_core::Model;
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

pub struct StaticSurface<'a> {
    pub object: &'a str,
    pub transform: Mat4,
    pub friction: Option<f32>,
    pub restitution: Option<f32>,
}

pub fn spawn_static_surface(
    commands: &mut Commands,
    model: &Model,
    settings: &StaticSurface,
) -> Result<Entity, String> {
    let object = model
        .object(settings.object)
        .ok_or_else(|| format!("No collision object {}", settings.object))?;
    let body = model
        .all_bodies()
        .into_iter()
        .find(|b| b.name == object.name);
    let collider = Collider::trimesh(
        object
            .positions()
            .map(|p| settings.transform.transform_point3(p))
            .collect(),
        object
            .faces
            .iter()
            .map(|f| f.vertices.map(|v| v as u32))
            .collect(),
    )
    .map_err(|e| format!("Invalid collision mesh {}: {e}", object.name))?;
    Ok(commands
        .spawn((
            Name::new(format!("Static collision / {}", object.name)),
            RigidBody::Fixed,
            collider,
            Transform::default(),
            Friction::coefficient(
                settings
                    .friction
                    .unwrap_or_else(|| body.map_or(0.7, |b| b.friction)),
            ),
            Restitution::coefficient(
                settings
                    .restitution
                    .unwrap_or_else(|| body.map_or(0.0, |b| b.restitution)),
            ),
        ))
        .id())
}
