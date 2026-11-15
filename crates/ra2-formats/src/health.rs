//! Native component category HP and chassis surface-area strength.
use crate::components::ComponentDefinition;
use arena_core::{
    health::{Health, PartHealth},
    robot::RobotDefinition,
};

pub fn build(
    robot: &RobotDefinition,
    definitions: &[ComponentDefinition],
) -> Result<Health, String> {
    if robot.components.len() != definitions.len() {
        return Err("HP definition count mismatch".into());
    }
    let parts = definitions
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let maximum = if i == 0 {
                let area: f32 = robot
                    .chassis
                    .model
                    .as_ref()
                    .map(|model| {
                        model
                            .objects
                            .iter()
                            .filter(|o| o.visible)
                            .flat_map(|o| {
                                o.faces.iter().map(|f| {
                                    let [a, b, c] = f
                                        .vertices
                                        .map(|n| o.transform.transform_point3(o.vertices[n]));
                                    (b - a).cross(c - a).length() * 0.5
                                })
                            })
                            .sum()
                    })
                    .unwrap_or(0.);
                (area * robot.chassis.hardness).round().max(1.)
            } else {
                let category = match d.value("type").to_ascii_lowercase().as_str() {
                    "power" | "mechanics" | "wheels" | "mobility" => 400.,
                    "weapons" | "extras" => 1000.,
                    _ => 100.,
                };
                d.value("hitpoints")
                    .parse::<f32>()
                    .ok()
                    .filter(|v| v.is_finite() && *v > 0.)
                    .unwrap_or(category)
            };
            PartHealth::new(
                maximum,
                !d.value("damageable").eq_ignore_ascii_case("false")
                    && d.value("damageable") != "0",
            )
        })
        .collect();
    Ok(Health {
        parts,
        ..Default::default()
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_category_override_flags_and_chassis_area_are_independent() {
        let mut r = arena_core::editor::Workshop::new().robot;
        r.chassis.bottom = vec![
            arena_core::Vec2::new(200., 200.),
            arena_core::Vec2::new(800., 200.),
            arena_core::Vec2::new(800., 800.),
            arena_core::Vec2::new(200., 800.),
        ];
        r.chassis.top = r.chassis.bottom.clone();
        arena_core::editor::chassis::rebuild(&mut r.chassis).unwrap();
        let mut defs = vec![ComponentDefinition::parse("")];
        for text in [
            "type=power",
            "type=extenders",
            "type=mechanics",
            "type=wheels",
            "type=weapons",
            "type=extras",
            "hitpoints=235\ndamageable=false",
        ] {
            r.components.push(r.components[0].clone());
            defs.push(ComponentDefinition::parse(text));
        }
        let h = build(&r, &defs).unwrap();
        assert!(h.parts[0].maximum > 100.);
        assert_eq!(
            h.parts[1..].iter().map(|p| p.maximum).collect::<Vec<_>>(),
            vec![400., 100., 400., 400., 1000., 1000., 235.]
        );
        assert!(!h.parts[7].damageable);
        let before = h.parts[0].maximum;
        r.chassis.hardness *= 2.;
        assert!((build(&r, &defs).unwrap().parts[0].maximum - before * 2.).abs() <= 1.);
    }
}

/// Contact response parameters are read from native CFZ/style definitions, not BOT names.
pub fn contact_profile(
    definition: &ComponentDefinition,
    material_name: &str,
) -> arena_core::health::ContactProfile {
    use arena_core::health::{ContactMaterial as M, ContactProfile};
    let base = definition.value("base").to_ascii_lowercase();
    let explicit = definition.value("material");
    let name = if explicit.is_empty() {
        material_name
    } else {
        explicit
    }
    .trim()
    .to_ascii_lowercase();
    let material = match name.as_str() {
        "arenium" => M::Arenium,
        "rubber" => M::Rubber,
        "metal" => M::Metal,
        _ if base == "weapon" => M::Weapon,
        _ if base == "wheel" => M::Rubber,
        _ => M::Metal,
    };
    let number = |key, default| {
        definition
            .value(key)
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite() && *v >= 0.)
            .unwrap_or(default)
    };
    ContactProfile {
        material,
        concussion: number("concussion", 1.),
        piercing: number("piercing", 0.),
    }
}

#[cfg(test)]
mod contact_tests {
    use super::*;
    use arena_core::health::ContactMaterial;
    #[test]
    fn native_spike_parameters_are_not_replaced_by_default_concussion() {
        let spike = contact_profile(
            &ComponentDefinition::parse("base=Weapon\nconcussion=0\npiercing=1"),
            "Material #12",
        );
        assert_eq!((spike.concussion, spike.piercing), (0., 1.));
        assert_eq!(spike.material, ContactMaterial::Weapon);
        let ordinary = contact_profile(&ComponentDefinition::parse(""), "");
        assert_eq!((ordinary.concussion, ordinary.piercing), (1., 0.));
        assert_eq!(
            contact_profile(&ComponentDefinition::parse("base=Wheel"), "").material,
            ContactMaterial::Rubber
        );
    }
}
