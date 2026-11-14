//! Preserve each component's native mass when welding collision shapes. Scaling
//! one uniform-density compound to the total mass erases dense ballast and gives
//! small motor shafts the same density as large tyres/chassis volumes.
use super::{damage::Solid, *};
use bevy_rapier3d::rapier::{dynamics::MassProperties as NativeMass, math::Pose};

pub(super) fn properties(solids: &[Solid], masses: &[(usize, f32)]) -> MassProperties {
    let mut result = NativeMass::default();
    for &(component, mass) in masses {
        let mut part: NativeMass = solids
            .iter()
            .filter(|s| s.component == component)
            .map(|s| {
                s.collider
                    .raw
                    .mass_properties(1.)
                    .transform_by(&Pose::from_parts(s.position, s.rotation))
            })
            .sum();
        if part.mass() > 0. {
            part.set_mass(mass, true);
        } else {
            part = NativeMass::new(Vec3::ZERO, mass, Vec3::splat(mass * 0.00001));
        }
        result += part;
    }
    MassProperties::from_rapier(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dense_ballast_controls_centre_and_inertia_independent_of_mesh_volume() {
        let solids = vec![
            Solid {
                component: 0,
                position: Vec3::Y,
                rotation: Quat::IDENTITY,
                collider: Collider::cuboid(1., 0.5, 1.),
            },
            Solid {
                component: 1,
                position: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                collider: Collider::cuboid(0.1, 0.1, 0.1),
            },
        ];
        let props = properties(&solids, &[(0, 10.), (1, 90.)]);
        assert!((props.mass - 100.).abs() < 0.001);
        assert!(
            props
                .local_center_of_mass
                .abs_diff_eq(Vec3::Y * 0.1, 0.0001)
        );
        let expected = 10. * (0.5_f32.powi(2) + 1.) / 3. + 90. * (0.1_f32.powi(2) * 2.) / 3. + 9.;
        let tensor = props.into_rapier().reconstruct_inertia_matrix();
        assert!(
            (tensor.x_axis.x - expected).abs() < expected * 0.0002,
            "inertia {:?}, expected {expected}; properties {props:?}",
            tensor
        );
        // Removing the ballast recomputes, instead of leaving a hidden weight.
        let bare = properties(&solids[..1], &[(0, 10.)]);
        assert!(bare.local_center_of_mass.abs_diff_eq(Vec3::Y, 0.0001));
    }
}
