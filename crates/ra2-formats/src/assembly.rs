//! RA2 mounting conventions -> a neutral robot assembly. No graphics/physics API.
use arena_core::{robot::*, *};
use std::sync::Arc;

pub struct ResolvedComponent {
    pub key: String,
    pub model: Arc<Model>,
    pub hidden: bool,
}

fn socket(model: &Model, id: u32) -> Result<Mat4, String> {
    model
        .attachments
        .iter()
        .find(|a| a.properties.get("id").and_then(|v| v.parse::<u32>().ok()) == Some(id))
        // Some shipped templates have one socket tagged 2 while the BOT refers to 1.
        // The sole-socket case is unambiguous; never guess between multiple sockets.
        .or_else(|| {
            if model.attachments.len() == 1 {
                model.attachments.first()
            } else {
                None
            }
        })
        .map(|a| a.transform)
        .ok_or_else(|| format!("Missing attachment point {id}"))
}

pub fn assemble(
    robot: &RobotDefinition,
    models: &[ResolvedComponent],
) -> Result<RobotAssembly, String> {
    if models.len() != robot.components.len() {
        return Err("Component/model count mismatch".into());
    }
    if models.len() > 4096 {
        return Err("Robot exceeds component limit".into());
    }
    fn solve(
        index: usize,
        robot: &RobotDefinition,
        models: &[ResolvedComponent],
        states: &mut [u8],
        transforms: &mut [Mat4],
        depth: usize,
    ) -> Result<Mat4, String> {
        if depth > 128 {
            return Err("Robot component hierarchy is too deep".into());
        }
        if states[index] == 2 {
            return Ok(transforms[index]);
        }
        if states[index] == 1 {
            return Err("Cyclic robot component hierarchy".into());
        }
        states[index] = 1;
        let c = &robot.components[index];
        let transform = if index == 0 {
            Mat4::IDENTITY
        } else {
            let parent = c
                .parent
                .ok_or_else(|| format!("Detached component {index}"))?;
            if parent >= models.len() {
                return Err("Parent outside component list".into());
            }
            let base = solve(parent, robot, models, states, transforms, depth + 1)?;
            let parent_mount = if parent == 0 {
                Mat4::IDENTITY
            } else {
                socket(&models[parent].model, c.parent_mount)?
                    * Mat4::from_rotation_x(std::f32::consts::PI)
            };
            base * parent_mount
                * c.mounting.matrix()
                * socket(&models[index].model, c.own_mount)?.inverse()
        };
        if !transform.is_finite() {
            return Err(format!("Invalid mounting transform for component {index}"));
        }
        states[index] = 2;
        transforms[index] = transform;
        Ok(transform)
    }
    let mut states = vec![0; models.len()];
    let mut transforms = vec![Mat4::IDENTITY; models.len()];
    let mut parts = vec![];
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut total = 0.0;
    for (i, model) in models.iter().enumerate() {
        let transform = solve(i, robot, models, &mut states, &mut transforms, 0)
            .map_err(|e| format!("{} component {i}: {e}", robot.name))?;
        let mass = if i == 0 {
            model
                .model
                .objects
                .iter()
                .map(|o| {
                    o.faces
                        .iter()
                        .map(|f| {
                            let [a, b, c] = f
                                .vertices
                                .map(|j| o.transform.transform_point3(o.vertices[j]));
                            (b - a).cross(c - a).length() * 0.5
                        })
                        .sum::<f32>()
                })
                .sum::<f32>()
                * robot.chassis.surface_density
        } else {
            model.model.all_bodies().iter().map(|b| b.mass).sum::<f32>()
        };
        let visible = !model.hidden;
        if visible {
            for p in model
                .model
                .objects
                .iter()
                .filter(|o| o.visible)
                .flat_map(|o| o.positions())
            {
                let p = transform.transform_point3(p);
                min = min.min(p);
                max = max.max(p);
            }
            total += mass;
        }
        parts.push(AssemblyPart {
            component: i,
            model: model.key.clone(),
            transform,
            mass,
            visible,
        });
    }
    if !min.is_finite() || !max.is_finite() || (max - min).max_element() > 30.0 {
        return Err(format!(
            "Implausible assembled bounds for {}: {min:?}..{max:?}",
            robot.name
        ));
    }
    Ok(RobotAssembly {
        name: robot.name.clone(),
        parts,
        mass: total.max(1.0),
        bounds: (min, max),
    })
}
