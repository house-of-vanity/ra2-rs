//! Absolute HP, attachment state and native damage-number grouping. This state
//! belongs to a running simulation, never to a saved robot blueprint.
use crate::{Vec3, robot::RobotDefinition};

#[derive(Clone, Debug)]
pub struct PartHealth {
    pub maximum: f32,
    pub current: f32,
    pub damageable: bool,
    pub attached: bool,
}
impl PartHealth {
    pub fn new(maximum: f32, damageable: bool) -> Self {
        let maximum = if maximum.is_finite() {
            maximum.max(1.)
        } else {
            100.
        };
        Self {
            maximum,
            current: maximum,
            damageable,
            attached: true,
        }
    }
    pub fn fraction(&self) -> f32 {
        (self.current / self.maximum).clamp(0., 1.)
    }
    pub fn alive(&self) -> bool {
        self.current > 0. && self.attached
    }
}
#[derive(Clone, Copy, Debug)]
pub struct DamageEvent {
    pub component: usize,
    pub amount: f32,
    pub position: Vec3,
    pub destroyed: bool,
}
#[derive(Clone, Debug)]
pub struct DamageNumber {
    pub amount: f32,
    pub position: Vec3,
    pub age: f32,
    pub source: Option<u32>,
}
#[derive(Default, Debug)]
pub struct Health {
    pub parts: Vec<PartHealth>,
    pub numbers: Vec<DamageNumber>,
    pub received: f32,
}
impl Health {
    pub fn apply(&mut self, component: usize, amount: f32, position: Vec3) -> Option<DamageEvent> {
        self.apply_from(component, amount, position, None)
    }
    pub fn apply_from(
        &mut self,
        component: usize,
        amount: f32,
        position: Vec3,
        source: Option<u32>,
    ) -> Option<DamageEvent> {
        let part = self.parts.get_mut(component)?;
        if !part.damageable
            || !part.alive()
            || !amount.is_finite()
            || amount <= 0.
            || !position.is_finite()
        {
            return None;
        }
        let amount = amount.min(part.current);
        part.current = (part.current - amount).max(0.);
        let event = DamageEvent {
            component,
            amount,
            position,
            destroyed: part.current == 0.,
        };
        self.received += amount;
        // Native 0x417940: merge within 0.15 s and squared distance < 2.
        if let Some(n) = self.numbers.iter_mut().rev().find(|n| {
            n.source == source && n.age < 0.15 && n.position.distance_squared(position) < 2.
        }) {
            n.amount += amount;
        } else {
            // Fixed upper bound, even when many fragments hit at once.
            if self.numbers.len() == 32 {
                self.numbers.remove(0);
            }
            self.numbers.push(DamageNumber {
                amount,
                position,
                age: 0.,
                source,
            });
        }
        Some(event)
    }
    pub fn advance(&mut self, dt: f32) {
        for n in &mut self.numbers {
            n.age += dt;
        }
        self.numbers.retain(|n| n.age < 2.15);
    }
    /// A broken mount disconnects its subtree without damaging descendants or
    /// turning each intact connection into a separate break.
    pub fn disconnect(&mut self, robot: &RobotDefinition, root: usize) -> Vec<usize> {
        if root == 0 {
            return vec![];
        }
        let mut removed = vec![false; self.parts.len()];
        if root >= removed.len() {
            return vec![];
        }
        removed[root] = true;
        for _ in 0..removed.len() {
            let mut changed = false;
            for (i, c) in robot.components.iter().enumerate() {
                if !removed[i]
                    && c.parent
                        .is_some_and(|p| removed.get(p).copied().unwrap_or(false))
                {
                    removed[i] = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        removed
            .into_iter()
            .enumerate()
            .filter_map(|(i, removed)| {
                if removed && self.parts[i].attached {
                    self.parts[i].attached = false;
                    Some(i)
                } else {
                    None
                }
            })
            .collect()
    }
    pub fn operational(&self, component: usize) -> bool {
        self.parts.get(component).is_some_and(PartHealth::alive)
    }
}

/// Native 0x45a5a0 gameplay inverse mass contribution. Dynamic masses above
/// 100 are treated as 100; zero-mass scenery contributes 0.08, not zero. The
/// angular term stays separate so clamping does not erase impact lever arms.
pub fn impact_inverse_mass(mass: f32, angular_inverse_mass: f32) -> f32 {
    let linear = if mass.is_finite() && mass > 0. {
        mass.recip().max(0.01)
    } else {
        0.08
    };
    linear + angular_inverse_mass.max(0.)
}

/// Original impact strength is half the normal closing speed divided by the
/// two gameplay inverse masses plus their angular terms (0x45a5a0). It grows
/// linearly with speed, rather than kinetic energy's speed squared. The caller's
/// material/solver conversion is explicit; the default native material is 0.1
/// (0x402c70), and material-specific code may override it (0x405090/0x489470).
pub fn impact_damage(closing_speed: f32, inverse_effective_mass: f32, scale: f32) -> f32 {
    if !closing_speed.is_finite()
        || !inverse_effective_mass.is_finite()
        || !scale.is_finite()
        || scale <= 0.
        // Rapier noise gate; original damage callers reject <= 0.5 HP.
        || closing_speed < 1.5
        || inverse_effective_mass <= 0.
    {
        return 0.;
    }
    let amount = 0.5 * closing_speed / inverse_effective_mass * scale;
    if amount > 0.5 { amount } else { 0. }
}

/// Rapier reports one reciprocal mechanical contact, not two independent
/// attacks. A striking tool receives concussion from the target's own closing
/// motion, not a duplicate of the motion its motor supplied. Scenery and other
/// weapons retain their full response (including stationary spikes/blades).
/// This is a gameplay adaptation, not recovered Havok arithmetic.
pub fn weapon_recoil_fraction(
    recipient_tool: bool,
    source_robot: bool,
    source_tool: bool,
    source_closing_speed: f32,
    relative_closing_speed: f32,
) -> f32 {
    if !recipient_tool || !source_robot || source_tool {
        return 1.;
    }
    if !source_closing_speed.is_finite()
        || !relative_closing_speed.is_finite()
        || source_closing_speed <= 0.25
        || relative_closing_speed <= 0.
    {
        return 0.;
    }
    (source_closing_speed / relative_closing_speed).clamp(0., 1.)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tool_recoil_is_not_a_mirrored_attack_or_invulnerability() {
        // Hammer/piston into passive armor; exchanging the roles is symmetric.
        assert_eq!(weapon_recoil_fraction(true, true, false, 0., 8.), 0.);
        assert_eq!(weapon_recoil_fraction(true, true, false, -8., 8.), 0.);
        assert_eq!(weapon_recoil_fraction(true, true, false, 8., 8.), 1.);
        assert_eq!(weapon_recoil_fraction(true, true, false, 4., 12.), 1. / 3.);
        // Weapon counters, passive blades, walls and body rams still hurt.
        assert_eq!(weapon_recoil_fraction(true, true, true, 0., 8.), 1.);
        assert_eq!(weapon_recoil_fraction(true, false, false, 0., 8.), 1.);
        assert_eq!(weapon_recoil_fraction(false, true, false, 0., 8.), 1.);
    }
    #[test]
    fn hp_is_absolute_capped_and_non_damageable_parts_ignore_impacts() {
        let mut h = Health {
            parts: vec![PartHealth::new(400., true), PartHealth::new(100., false)],
            ..Default::default()
        };
        assert_eq!(h.apply(0, 25., Vec3::ZERO).unwrap().amount, 25.);
        assert_eq!(h.parts[0].current, 375.);
        assert!(h.apply(1, 200., Vec3::ZERO).is_none());
        assert!(h.apply(0, f32::NAN, Vec3::ZERO).is_none());
        assert_eq!(h.apply(0, 1000., Vec3::ZERO).unwrap().amount, 375.);
        assert!(h.apply(0, 1000., Vec3::ZERO).is_none());
        assert_eq!(h.received, 400.);
    }
    #[test]
    fn damage_numbers_use_native_time_and_squared_distance_rules() {
        let mut h = Health {
            parts: vec![PartHealth::new(1000., true)],
            ..Default::default()
        };
        h.apply(0, 10., Vec3::ZERO);
        h.advance(0.1);
        h.apply(0, 15., Vec3::X);
        assert_eq!(h.numbers.len(), 1);
        assert_eq!(h.numbers[0].amount, 25.);
        h.apply(0, 5., Vec3::X * 1.5);
        assert_eq!(h.numbers.len(), 2);
        h.advance(0.06);
        h.apply(0, 5., Vec3::ZERO);
        assert_eq!(h.numbers.len(), 3);
        h.advance(2.2);
        assert!(h.numbers.is_empty());
    }
    #[test]
    fn damage_numbers_keep_attackers_separate_when_grouping() {
        let mut h = Health {
            parts: vec![PartHealth::new(100., true)],
            ..Default::default()
        };
        h.apply_from(0, 10., Vec3::ZERO, Some(1));
        h.apply_from(0, 5., Vec3::ZERO, None);
        h.apply_from(0, 4., Vec3::ZERO, Some(1));
        assert_eq!(h.numbers.len(), 2);
        assert_eq!(h.numbers[0].amount, 14.);
        assert_eq!(h.numbers[0].source, Some(1));
    }
    #[test]
    fn a_mount_break_keeps_child_hp_and_intact_assembly_connections() {
        let mut robot = crate::editor::Workshop::new().robot;
        for parent in [0, 1, 2, 0] {
            let mut c = robot.components[0].clone();
            c.parent = Some(parent);
            robot.components.push(c);
        }
        let mut h = Health {
            parts: vec![PartHealth::new(100., true); 5],
            ..Default::default()
        };
        h.apply(1, 100., Vec3::ZERO);
        assert_eq!(h.disconnect(&robot, 1), vec![1, 2, 3]);
        assert_eq!(h.parts[2].current, 100.);
        assert!(!h.operational(2));
        assert!(h.operational(4));
        assert_eq!(robot.components[3].parent, Some(2));
    }
    #[test]
    fn impact_strength_matches_unmodified_original_executable() {
        // QEMU runs original 0x45a5a0 on these inputs. Expected values are its
        // recorded float outputs, not values generated by this implementation.
        // See tools/native-mount-probe/probe.c and logs/native-impact-probe.log.
        for (speed, a, b, lever, inverse_inertia, expected) in [
            (2., 400., 0., 0., 0., 0x4131c71d),
            (8., 400., 0., 0., 0., 0x4231c71d),
            (16., 400., 0., 0., 0., 0x42b1c71d),
            (8., 40., 0., 0., 0., 0x42186186),
            (8., 4000., 0., 0., 0., 0x4231c71d),
            (8., 400., 400., 0., 0., 0x43480000),
            (8., 40., 40., 0., 0., 0x42a00000),
            (8., 400., 0., 1., 0.02, 0x41f62763),
            (8., 400., 0., 1., 0.2, 0x41029cbc),
            (8., 0., 0., 0., 0., 0x41c80000),
        ] {
            let angular = lever * lever * inverse_inertia;
            let inverse = impact_inverse_mass(a, angular) + impact_inverse_mass(b, angular);
            let measured = f32::from_bits(expected);
            let actual = impact_damage(speed, inverse, 1.);
            assert!(
                (actual - measured).abs() <= measured * 0.000002,
                "speed={speed}, mass={a}/{b}: {actual} != original {measured}"
            );
        }
    }
    #[test]
    fn fast_wall_hits_stay_in_tens_without_capping_weapon_damage() {
        let wall = impact_inverse_mass(400., 0.) + impact_inverse_mass(0., 0.);
        let slow = impact_damage(10., wall, 0.25);
        let fast = impact_damage(20., wall, 0.25);
        assert!((fast - 2. * slow).abs() < 0.001);
        assert!((20. ..30.).contains(&fast));
        assert_eq!(
            fast,
            impact_damage(
                20.,
                impact_inverse_mass(4000., 0.) + impact_inverse_mass(0., 0.),
                0.25
            )
        );
        let weapon = impact_damage(80., impact_inverse_mass(400., 0.) * 2., 1.);
        assert!(
            weapon > 450.,
            "a blanket HP ceiling would weaken weapon strikes too"
        );
    }
    #[test]
    fn resting_contacts_do_not_damage_and_faster_impacts_do_more_damage() {
        assert_eq!(impact_damage(0.1, 1. / 500., 0.08), 0.);
        assert_eq!(impact_damage(2., 0., 0.08), 0.);
        assert!(impact_damage(4., 0.09, 0.25) > impact_damage(2., 0.09, 0.25));
    }
}

/// Native contact material IDs (registration at 488dd0..489024).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContactMaterial {
    #[default]
    Metal,
    Rubber,
    Arenium,
    Weapon,
}
#[derive(Clone, Copy, Debug)]
pub struct ContactProfile {
    pub material: ContactMaterial,
    pub concussion: f32,
    pub piercing: f32,
}
impl Default for ContactProfile {
    fn default() -> Self {
        Self {
            material: ContactMaterial::Metal,
            concussion: 1.,
            piercing: 0.,
        }
    }
}
impl ContactProfile {
    /// 489470: this source computes the HP applied to the other component.
    /// 414e99 computes it; 4176df resolves the target after two stack pushes.
    /// Equal physical reaction impulses do not imply equal damage amounts.
    pub fn response(self, other: Self, strength: f32, speed: f32) -> f32 {
        if other.material == ContactMaterial::Arenium
            || (self.material == ContactMaterial::Arenium
                && other.material == ContactMaterial::Rubber)
        {
            return 0.;
        }
        if !strength.is_finite() || !speed.is_finite() {
            return 0.;
        }
        let coefficient = if self.material == ContactMaterial::Arenium {
            0.5
        } else {
            0.1
        };
        (strength.max(0.) * self.concussion + speed.max(0.) * 10. * self.piercing) * coefficient
    }
}
#[cfg(test)]
mod material_tests {
    use super::*;
    #[test]
    fn material_response_matches_original_executable_fixtures() {
        // Original 489470 in the QEMU audit, same contact: speed 8, strength 200.
        for (piercing, concussion, material, other, expected) in [
            (
                0.,
                1.,
                ContactMaterial::Weapon,
                ContactMaterial::Metal,
                0x41a00000,
            ),
            (
                1.,
                0.,
                ContactMaterial::Weapon,
                ContactMaterial::Metal,
                0x41000000,
            ),
            (
                1.,
                1.,
                ContactMaterial::Weapon,
                ContactMaterial::Metal,
                0x41e00000,
            ),
            (0., 1., ContactMaterial::Arenium, ContactMaterial::Rubber, 0),
            (1., 0., ContactMaterial::Weapon, ContactMaterial::Arenium, 0),
            (
                0.,
                1.,
                ContactMaterial::Metal,
                ContactMaterial::Metal,
                0x41a00000,
            ),
        ] {
            let result = ContactProfile {
                material,
                piercing,
                concussion,
            }
            .response(
                ContactProfile {
                    material: other,
                    ..Default::default()
                },
                200.,
                8.,
            );
            assert_eq!(result, f32::from_bits(expected));
        }
    }
}
