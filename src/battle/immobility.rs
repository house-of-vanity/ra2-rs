//! Measure displacement of the chassis across fixed physics steps.
use bevy::prelude::Vec2;

const MOVEMENT: f32 = 0.25;
const GRACE_SECONDS: u32 = 5;
const WARNING_SECONDS: u32 = 10;
const TICKS_PER_SECOND: u32 = (1.0 / crate::STEP) as u32;

#[derive(Clone, Copy, Default)]
pub(crate) struct Immobility {
    anchor: Option<Vec2>,
    ticks: u32,
}

impl Immobility {
    /// Returns (visible countdown, counted out).
    pub(super) fn advance(&mut self, position: Vec2) -> (u8, bool) {
        if self
            .anchor
            .is_none_or(|p| p.distance_squared(position) >= MOVEMENT * MOVEMENT)
        {
            self.anchor = Some(position);
            self.ticks = 0;
        }
        self.ticks += 1;
        let seconds = self.ticks / TICKS_PER_SECOND;
        if seconds < GRACE_SECONDS {
            return (0, false);
        }
        let left = (GRACE_SECONDS + WARNING_SECONDS).saturating_sub(seconds);
        (left as u8, left == 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_counts_down_and_slow_travel_cancels_warning() {
        let mut timer = Immobility::default();
        for tick in 1..=15 * TICKS_PER_SECOND {
            let position = Vec2::X * (tick % 2) as f32 * 0.01;
            let (warning, out) = timer.advance(position);
            let seconds = tick / TICKS_PER_SECOND;
            assert_eq!(warning, if seconds < 5 { 0 } else { (15 - seconds) as u8 });
            assert_eq!(out, seconds == 15);
        }
        let mut timer = Immobility::default();
        for _ in 0..12 * TICKS_PER_SECOND {
            timer.advance(Vec2::ZERO);
        }
        for cm in 1..25 {
            assert_eq!(timer.advance(Vec2::X * cm as f32 / 100.), (3, false));
        }
        assert_eq!(timer.advance(Vec2::X * 0.25), (0, false));
    }

    #[test]
    #[ignore = "requires original Scout (RA2_GAME_DIR)"]
    fn countout_resolves_a_match_after_pause_and_movement() {
        use crate::battle::{Phase, Session, step};
        use crate::frontend::{Navigation, Page};
        use crate::gameplay::{RobotBody, RobotInstance};
        use arena_core::health::PartHealth;
        use bevy::prelude::*;

        let root = std::path::PathBuf::from(std::env::var_os("RA2_GAME_DIR").unwrap());
        let doc = ra2_formats::bot::decode(&std::fs::read(root.join("AI/Team0/Bot0.bot")).unwrap())
            .unwrap();
        let mut app = App::new();
        let mut session = Session {
            phase: Phase::Playing,
            ..default()
        };
        for id in 1..=2 {
            let mut robot = RobotInstance::new(id, Entity::PLACEHOLDER, default());
            robot.bodies.push(
                app.world_mut()
                    .spawn((RobotBody, Transform::default()))
                    .id(),
            );
            robot.damage.health.parts = vec![PartHealth::new(100., true); doc.components.len()];
            robot.document = Some(doc.clone());
            session.robots.push(robot);
        }
        let moving = session.robots[1].bodies[0];
        app.insert_resource(session)
            .insert_resource(Navigation {
                page: Page::Match,
                back: Page::Main,
            })
            .add_systems(Update, step);
        for _ in 0..5 * TICKS_PER_SECOND {
            app.update();
        }
        assert_eq!(
            app.world().resource::<Session>().immobility_countdown,
            [10, 10, 0, 0]
        );
        app.world_mut().resource_mut::<Navigation>().page = Page::Pause;
        for _ in 0..2 * TICKS_PER_SECOND {
            app.update();
        }
        assert_eq!(
            app.world().resource::<Session>().immobility_countdown,
            [10, 10, 0, 0]
        );
        app.world_mut().resource_mut::<Navigation>().page = Page::Match;
        app.world_mut()
            .get_mut::<Transform>(moving)
            .unwrap()
            .translation
            .x += 1.;
        app.update();
        assert_eq!(
            app.world().resource::<Session>().immobility_countdown,
            [10, 0, 0, 0]
        );
        for _ in 1..10 * TICKS_PER_SECOND {
            app.update();
        }
        assert_eq!(
            app.world().resource::<Session>().phase,
            Phase::Finished(Some(1))
        );
        assert_eq!(app.world().resource::<Navigation>().page, Page::Results);
    }
}
