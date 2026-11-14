//! Initial robot physics adapter: a welded compound body made from component hulls.
//! Articulated joints/motors can be added independently of BOT decoding and rendering.
use arena_core::{Model, Quat, Vec3, robot::RobotAssembly};
use bevy_rapier3d::prelude::Collider;
use std::collections::BTreeMap;

pub fn component_hull(model: &Model) -> Option<Collider> {
    let vertices: Vec<_> = model
        .objects
        .iter()
        .filter(|o| o.visible)
        .flat_map(|o| o.positions())
        .collect();
    if vertices.len() < 4 {
        return None;
    }
    Collider::convex_hull(&vertices)
}

pub fn compound(
    assembly: &RobotAssembly,
    hulls: &BTreeMap<String, Collider>,
) -> Result<Collider, String> {
    let mut shapes = vec![];
    for part in assembly.parts.iter().filter(|p| p.visible) {
        if let Some(hull) = hulls.get(&part.model) {
            let (scale, rotation, position) = part.transform.to_scale_rotation_translation();
            if (scale - arena_core::Vec3::ONE).abs().max_element() > 0.01 {
                return Err(format!("Non-rigid part transform: {}", part.model));
            }
            shapes.push((position, rotation.normalize(), hull.clone()));
        }
    }
    if shapes.is_empty() {
        return Err("Robot has no collision shapes".into());
    }
    Ok(Collider::compound(shapes))
}

/// Preserve separate collision proxies so sockets and pass-through shafts are not
/// replaced by one convex envelope around the entire component.
pub fn collision_parts(model: &Model) -> Vec<(String, Collider)> {
    let mut shapes = Vec::new();
    for body in model.all_bodies().into_iter().filter(|b| !b.disabled) {
        let object = model
            .object(&body.proxy)
            .or_else(|| model.object(&body.name));
        if let Some(object) = object {
            let vertices = object.positions().collect::<Vec<_>>();
            if let Some(shape) = Collider::convex_hull(&vertices) {
                shapes.push((object.name.to_ascii_lowercase(), shape));
            }
        }
    }
    if shapes.is_empty() {
        for object in model.objects.iter().filter(|o| o.visible) {
            if let Some(shape) = Collider::convex_hull(&object.positions().collect::<Vec<_>>()) {
                shapes.push((object.name.to_ascii_lowercase(), shape));
            }
        }
    }
    shapes
}

/// Smooth tyre contact independent of the old low-polygon display mesh. The
/// display geometry supplies the dimensions; the axle socket supplies direction.
/// Hidden `contact` meshes are sensors in the original and may be oversized.
pub fn wheel_collider(model: &Model) -> Option<(String, Vec3, Quat, Collider)> {
    let socket = model.attachments.iter().find(|a| {
        a.properties
            .get("attach")
            .is_some_and(|s| s.eq_ignore_ascii_case("Axle_F"))
    })?;
    let axis = socket.transform.z_axis.truncate().try_normalize()?;
    let rotation = Quat::from_rotation_arc(Vec3::Y, axis);
    let objects: Vec<_> = model
        .objects
        .iter()
        .filter(|o| o.visible && !o.faces.is_empty())
        .collect();
    let vertices: Vec<_> = objects
        .iter()
        .flat_map(|o| o.positions())
        .map(|p| rotation.inverse() * p)
        .collect();
    let first = *vertices.first()?;
    let (min, max) = vertices
        .iter()
        .fold((first, first), |(a, b), &p| (a.min(p), b.max(p)));
    let centre = (min + max) * 0.5;
    let radius = vertices
        .iter()
        .map(|p| {
            let d = *p - centre;
            Vec3::new(d.x, 0., d.z).length()
        })
        .reduce(f32::max)?;
    let half_width = (max.y - min.y) * 0.5;
    if !radius.is_finite() || !half_width.is_finite() || radius < 0.005 || half_width < 0.001 {
        return None;
    }
    let border = (radius * 0.04).min(half_width * 0.25);
    let shape = Collider::round_cylinder(half_width - border, radius - border, border);
    Some((
        objects[0].name.to_ascii_lowercase(),
        rotation * centre,
        rotation,
        shape,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arena_core::{Attachment, Face, Mat4, Object};

    #[test]
    fn wheel_fits_rotated_display_geometry_and_ignores_oversized_contact_sensor() {
        let tilt = Quat::from_rotation_z(0.7);
        let centre = Vec3::new(0.2, -0.1, 0.3);
        let vertices = [-0.2, 0.2]
            .into_iter()
            .flat_map(|y| {
                (0..16).map(move |i| {
                    let angle = i as f32 * std::f32::consts::TAU / 16.;
                    Vec3::new(angle.cos(), y, angle.sin())
                })
            })
            .collect();
        let tyre = Object {
            name: "tyre".into(),
            visible: true,
            parent: None,
            transform: Mat4::from_rotation_translation(tilt, centre),
            vertices,
            faces: vec![Face {
                vertices: [0, 1, 16],
                material: 0,
                uv: [[0.; 2]; 3],
                normals: [Vec3::Y; 3],
            }],
            material: 0,
            uv_channels: BTreeMap::new(),
        };
        let mut sensor = tyre.clone();
        sensor.visible = false;
        sensor.name = "oversized_traction_sensor".into();
        for p in &mut sensor.vertices {
            *p *= 1.7;
        }
        let model = Model {
            objects: vec![sensor, tyre],
            attachments: vec![Attachment {
                name: "axle".into(),
                transform: Mat4::from_quat(Quat::from_rotation_arc(Vec3::Z, tilt * Vec3::Y)),
                properties: BTreeMap::from([("attach".into(), "Axle_F".into())]),
            }],
            ..Default::default()
        };
        let (owner, position, orientation, collider) = wheel_collider(&model).unwrap();
        let shape = collider.raw.as_round_cylinder().unwrap();
        assert_eq!(owner, "tyre");
        assert!(position.abs_diff_eq(centre, 1e-5));
        assert!((orientation * Vec3::Y).abs_diff_eq(tilt * Vec3::Y, 1e-5));
        assert!((shape.inner_shape.radius + shape.border_radius - 1.).abs() < 1e-5);
        assert!((shape.inner_shape.half_height + shape.border_radius - 0.2).abs() < 1e-5);
    }
}
