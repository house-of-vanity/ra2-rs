//! Application navigation; game simulation, preferences, artwork and audio stay separate.
mod art;
mod music;
pub(crate) use music::WorkshopMusic;
mod ui;

use crate::{
    Options,
    settings::{self, EngineSettings},
};
use bevy::prelude::*;
use bevy_rapier3d::prelude::RapierConfiguration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Main,
    Settings,
    Pause,
    Workshop,
    TeamSelect,
    TeamHQ,
    Exhibition,
    Multiplayer,
    Match,
    Results,
}

#[derive(Resource)]
pub struct Navigation {
    pub page: Page,
    pub back: Page,
}
impl Navigation {
    pub fn legacy_visible(&self) -> bool {
        let page = if self.page == Page::Settings {
            self.back
        } else {
            self.page
        };
        matches!(
            page,
            Page::Workshop
                | Page::TeamSelect
                | Page::TeamHQ
                | Page::Multiplayer
                | Page::Exhibition
                | Page::Match
                | Page::Results
        )
    }
    pub fn covers_scene(&self) -> bool {
        matches!(
            self.page,
            Page::Main
                | Page::Workshop
                | Page::TeamSelect
                | Page::TeamHQ
                | Page::Multiplayer
                | Page::Exhibition
        ) || (self.page == Page::Settings
            && matches!(
                self.back,
                Page::Main
                    | Page::Workshop
                    | Page::TeamSelect
                    | Page::TeamHQ
                    | Page::Multiplayer
                    | Page::Exhibition
            ))
    }
}

#[derive(Resource)]
pub struct Draft(pub EngineSettings);

#[derive(Component)]
pub struct MenuCamera;

pub struct FrontendPlugin;
#[derive(SystemSet, Debug, Clone, Hash, PartialEq, Eq)]
pub struct FrontendSetup;
impl Plugin for FrontendPlugin {
    fn build(&self, app: &mut App) {
        let settings = app.world().resource::<EngineSettings>().clone();
        app.insert_resource(Navigation {
            page: if app.world().resource::<Options>().editor {
                Page::Workshop
            } else {
                Page::Main
            },
            back: Page::Main,
        })
        .insert_resource(Draft(settings))
        .add_observer(ui::activate)
        .add_systems(
            Startup,
            (art::load, ui::setup, music::setup)
                .chain()
                .in_set(FrontendSetup),
        )
        .add_systems(PreUpdate, (keyboard, synchronize).chain())
        .add_systems(
            Update,
            (
                ui::interact,
                settings::apply,
                music::synchronize,
                ui::refresh,
            )
                .chain(),
        )
        .add_systems(Update, ui::resize);
    }
}

fn keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    options: Res<Options>,
    mut nav: ResMut<Navigation>,
    mut settings: ResMut<EngineSettings>,
    mut draft: ResMut<Draft>,
) {
    if options.smoke_test || options.editor_smoke_test {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        nav.page = match nav.page {
            Page::Settings => nav.back,
            Page::Pause => Page::Match,
            Page::Main => Page::Main,
            Page::Workshop => Page::Workshop,
            Page::TeamSelect => Page::Main,
            Page::TeamHQ => Page::TeamSelect,
            Page::Exhibition | Page::Multiplayer => Page::Main,
            Page::Match => {
                nav.back = Page::Match;
                Page::Pause
            }
            Page::Results => Page::Results,
        };
    }
    if keys.just_pressed(KeyCode::Enter)
        && (keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight))
    {
        settings.fullscreen = !settings.fullscreen;
        if nav.page == Page::Settings {
            draft.0.fullscreen = settings.fullscreen;
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "Bevy resources and independent camera queries"
)]
fn synchronize(
    nav: Res<Navigation>,
    session: Res<crate::battle::Session>,
    mut time: ResMut<Time<Virtual>>,
    mut physics: Query<(&mut RapierConfiguration, Has<crate::garage::GarageContext>)>,
    editor: Res<crate::editor::Editor>,
    mut menu_cameras: Query<&mut Camera, (With<MenuCamera>, Without<Camera3d>)>,
) {
    let testing = nav.page == Page::Workshop
        && editor.mode == crate::editor::Mode::Test
        && editor.dialog.is_none();
    let in_match = nav.page == Page::Match;
    let online = session.network && !session.robots.is_empty() && !session.net_paused;
    if testing || in_match || online {
        time.unpause();
    } else {
        time.pause();
    }
    for (mut config, garage) in &mut physics {
        config.physics_pipeline_active = if garage {
            testing
        } else {
            (in_match || online)
                && matches!(session.phase, crate::battle::Phase::Playing)
                && !session.net_waiting
                && !session.net_paused
                && (!session.network || session.authority)
        };
    }
    for mut camera in &mut menu_cameras {
        camera.is_active = true;
        camera.clear_color = if nav.covers_scene() {
            ClearColorConfig::Custom(Color::BLACK)
        } else {
            ClearColorConfig::None
        };
    }
}

#[cfg(test)]
mod physics_start_tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use bevy_rapier3d::prelude::*;
    #[test]
    fn introduction_countdown_and_pause_do_not_simulate_the_tabletop() {
        use crate::battle::{Phase, Session};
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .insert_resource(Time::<Fixed>::from_seconds(crate::STEP))
            .insert_resource(TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs_f64(crate::STEP),
            ))
            .insert_resource(TimestepMode::Fixed {
                dt: crate::STEP as f32,
                substeps: 1,
            })
            .add_plugins(RapierPhysicsPlugin::<NoUserData>::default().in_fixed_schedule())
            .init_resource::<Session>()
            .init_resource::<crate::editor::Editor>()
            .insert_resource(Navigation {
                page: Page::Match,
                back: Page::Main,
            })
            .add_systems(PreUpdate, synchronize);
        let context = app
            .world_mut()
            .spawn((
                RapierContextSimulation::default(),
                RapierConfiguration::new(1.),
            ))
            .id();
        let start = Transform::from_xyz(0., 5., 0.);
        let floor = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                Collider::cuboid(4., 0.2, 4.),
                start,
                Velocity {
                    linear: Vec3::ZERO,
                    angular: Vec3::X,
                },
                RapierContextEntityLink(context),
            ))
            .id();
        for phase in [
            Phase::Introduction(0.),
            Phase::Countdown(3.),
            Phase::Countdown(1.),
        ] {
            app.world_mut().resource_mut::<Session>().phase = phase;
            for _ in 0..120 {
                app.update();
            }
            assert_eq!(*app.world().get::<Transform>(floor).unwrap(), start);
            assert!(
                !app.world()
                    .get::<RapierConfiguration>(context)
                    .unwrap()
                    .physics_pipeline_active
            );
        }
        app.world_mut().resource_mut::<Session>().phase = Phase::Playing;
        for _ in 0..20 {
            app.update();
        }
        assert!(app.world().get::<Transform>(floor).unwrap().translation.y < 4.95);
        app.world_mut().resource_mut::<Navigation>().page = Page::Pause;
        app.update();
        let paused = *app.world().get::<Transform>(floor).unwrap();
        for _ in 0..20 {
            app.update();
        }
        assert_eq!(*app.world().get::<Transform>(floor).unwrap(), paused);
    }
}
