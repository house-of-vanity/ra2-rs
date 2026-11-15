//! Match resolution independent of rendering, input, physics and AI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Winner(usize),
    Draw,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Deathmatch,
    BattleRoyal,
    Teams,
    TableTop,
    KingOfHill,
}
impl Mode {
    pub const ALL: [Self; 5] = [
        Self::Deathmatch,
        Self::BattleRoyal,
        Self::Teams,
        Self::TableTop,
        Self::KingOfHill,
    ];
    pub fn tag(self) -> &'static str {
        match self {
            Self::Deathmatch => "DEATHMATCH",
            Self::BattleRoyal => "BATTLE ROYAL",
            Self::Teams => "TEAM MATCH",
            Self::TableTop => "TABLETOP",
            Self::KingOfHill => "KING OF THE HILL",
        }
    }
    pub fn slots(self) -> usize {
        if self == Self::Deathmatch { 2 } else { 4 }
    }
    pub fn minimum(self) -> usize {
        match self {
            Self::BattleRoyal => 3,
            Self::Teams => 4,
            _ => 2,
        }
    }
    pub fn side(self, slot: usize) -> usize {
        if self == Self::Teams { slot / 2 } else { slot }
    }
}
/// A team victory includes both partners, including an already eliminated partner.
/// Empty slots never participate. Equal timeout scores and simultaneous KOs draw.
pub fn resolve(
    mode: Mode,
    active: [bool; 4],
    dead: [bool; 4],
    points: [f32; 4],
    expired: bool,
) -> Option<[bool; 4]> {
    let mut sides = std::collections::BTreeMap::<usize, (bool, f32)>::new();
    for i in 0..4 {
        if active[i] {
            let side = sides.entry(mode.side(i)).or_default();
            side.0 |= !dead[i];
            side.1 += points[i];
        }
    }
    let alive = sides
        .iter()
        .filter(|(_, (alive, _))| *alive)
        .map(|(&id, _)| id)
        .collect::<Vec<_>>();
    let winner = if alive.len() <= 1 {
        alive.first().copied()
    } else if expired {
        let highest = alive
            .iter()
            .map(|id| sides[id].1)
            .fold(f32::NEG_INFINITY, f32::max);
        let leaders = alive
            .iter()
            .copied()
            .filter(|id| (sides[id].1 - highest).abs() < 0.5)
            .collect::<Vec<_>>();
        (leaders.len() == 1).then(|| leaders[0])
    } else {
        return None;
    };
    Some(std::array::from_fn(|i| {
        active[i] && winner == Some(mode.side(i))
    }))
}
/// Elimination takes precedence over points. A passive robot remains a contestant.
pub fn deathmatch(defeated: [bool; 2], points: [f32; 2], time_expired: bool) -> Option<Outcome> {
    match defeated {
        [true, true] => Some(Outcome::Draw),
        [true, false] => Some(Outcome::Winner(1)),
        [false, true] => Some(Outcome::Winner(0)),
        [false, false] if time_expired => Some(if (points[0] - points[1]).abs() < 0.5 {
            Outcome::Draw
        } else {
            Outcome::Winner(usize::from(points[1] > points[0]))
        }),
        _ => None,
    }
}
/// Contact direction is A -> B. A passive target is not automatically an
/// attacker merely because the collision also damaged the incoming weapon.
/// The native engine supplies an attacker in its hit event; Rapier does not,
/// so closing point motion is our approximation of that event's provenance.
pub fn impact_initiators(a: crate::Vec3, b: crate::Vec3, normal: crate::Vec3) -> [bool; 2] {
    let closing = [a.dot(normal).max(0.), (-b).dot(normal).max(0.)];
    let dominant = closing[0].max(closing[1]);
    closing.map(|v| v > 0.25 && v >= dominant * 0.5)
}
/// The original score accumulator stores integers, truncating positive hit HP.
pub fn damage_points(applied_hp: f32) -> f32 {
    if applied_hp.is_finite() {
        applied_hp.max(0.).floor()
    } else {
        0.
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn four_way_and_team_resolution_respects_survivors_and_partner_points() {
        let active = [true; 4];
        assert_eq!(
            resolve(Mode::BattleRoyal, active, [false; 4], [0.; 4], false),
            None
        );
        assert_eq!(
            resolve(
                Mode::BattleRoyal,
                active,
                [true, true, false, true],
                [0.; 4],
                false
            ),
            Some([false, false, true, false])
        );
        assert_eq!(
            resolve(
                Mode::Teams,
                active,
                [true, false, true, true],
                [0.; 4],
                false
            ),
            Some([true, true, false, false])
        );
        assert_eq!(
            resolve(Mode::Teams, active, [false; 4], [10., 15., 24., 0.], true),
            Some([true, true, false, false])
        );
        assert_eq!(
            resolve(
                Mode::KingOfHill,
                active,
                [false; 4],
                [20., 50., 10., 30.],
                true
            ),
            Some([false, true, false, false])
        );
        assert_eq!(
            resolve(Mode::TableTop, active, [true; 4], [100., 0., 0., 0.], false),
            Some([false; 4])
        );
    }
    #[test]
    fn strike_provenance_does_not_mirror_recoil_into_opponent_points() {
        use crate::Vec3;
        assert_eq!(
            impact_initiators(Vec3::X * 8., Vec3::ZERO, Vec3::X),
            [true, false]
        );
        assert_eq!(
            impact_initiators(Vec3::ZERO, Vec3::NEG_X * 8., Vec3::X),
            [false, true]
        );
        assert_eq!(
            impact_initiators(Vec3::X * 8., Vec3::X * 3., Vec3::X),
            [true, false]
        );
        assert_eq!(
            impact_initiators(Vec3::X * 8., Vec3::NEG_X * 8., Vec3::X),
            [true, true]
        );
        assert_eq!(
            impact_initiators(Vec3::ZERO, Vec3::ZERO, Vec3::X),
            [false; 2]
        );
        assert_eq!(damage_points(12.9), 12.);
        assert_eq!(damage_points(0.9), 0.);
        assert_eq!(damage_points(f32::NAN), 0.);
    }
    #[test]
    fn passive_opponents_only_lose_when_destroyed_or_time_runs_out() {
        assert_eq!(deathmatch([false; 2], [100., 0.], false), None);
        assert_eq!(
            deathmatch([false; 2], [100., 0.], true),
            Some(Outcome::Winner(0))
        );
        assert_eq!(
            deathmatch([true, false], [100., 0.], true),
            Some(Outcome::Winner(1))
        );
        assert_eq!(
            deathmatch([true; 2], [100., 0.], false),
            Some(Outcome::Draw)
        );
        assert_eq!(deathmatch([false; 2], [0.; 2], true), Some(Outcome::Draw));
    }
}
