//! Application navigation; game simulation, preferences, artwork and audio stay separate.
mod art;
mod music;
pub(crate) use music::WorkshopMusic;
mod smoke;
mod ui;

use crate::{
    Options,
    settings::{self, EngineSettings},
    simulation::DriveInput,
};
use bevy::prelude::*;
use bevy_rapier3d::prelude::RapierConfiguration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Main,
    Settings,
    Pause,
    Hidden,
    Workshop,
    TeamSelect,
    TeamHQ,
}

#[derive(Resource)]
pub struct Navigation {
    pub page: Page,
    pub back: Page,
    pub started: bool,
}
impl Navigation {
    pub fn visible(&self) -> bool {
        self.page != Page::Hidden
    }
    pub fn legacy_visible(&self) -> bool {
        let page = if self.page == Page::Settings {
            self.back
        } else {
            self.page
        };
        matches!(page, Page::Workshop | Page::TeamSelect | Page::TeamHQ)
    }
    pub fn covers_scene(&self) -> bool {
        matches!(
            self.page,
            Page::Main | Page::Workshop | Page::TeamSelect | Page::TeamHQ
        ) || (self.page == Page::Settings
            && matches!(
                self.back,
                Page::Main | Page::Workshop | Page::TeamSelect | Page::TeamHQ
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
        let start_menu = app.world().resource::<Options>().menu;
        let settings = app.world().resource::<EngineSettings>().clone();
        app.insert_resource(Navigation {
            page: if app.world().resource::<Options>().editor {
                Page::Workshop
            } else if start_menu {
                Page::Main
            } else {
                Page::Hidden
            },
            back: Page::Main,
            started: !start_menu,
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
        .add_systems(Update, (ui::resize, smoke::run));
    }
}

fn keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    options: Res<Options>,
    mut nav: ResMut<Navigation>,
    mut settings: ResMut<EngineSettings>,
    mut draft: ResMut<Draft>,
) {
    if options.smoke_test || options.menu_smoke_test || options.editor_smoke_test {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        nav.page = match nav.page {
            Page::Hidden => Page::Pause,
            Page::Settings => nav.back,
            Page::Pause => Page::Hidden,
            Page::Main => Page::Main,
            Page::Workshop => Page::Workshop,
            Page::TeamSelect => Page::Main,
            Page::TeamHQ => Page::TeamSelect,
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
    mut time: ResMut<Time<Virtual>>,
    mut drive: ResMut<DriveInput>,
    mut physics: Query<(&mut RapierConfiguration, Has<crate::garage::GarageContext>)>,
    editor: Res<crate::editor::Editor>,
    mut world_cameras: Query<&mut Camera, (With<crate::view::DemoCamera>, Without<MenuCamera>)>,
    mut menu_cameras: Query<&mut Camera, (With<MenuCamera>, Without<Camera3d>)>,
    settings: Res<EngineSettings>,
    mut overlays: Query<&mut Node, With<crate::view::HudOverlay>>,
) {
    let testing = nav.page == Page::Workshop
        && editor.mode == crate::editor::Mode::Test
        && editor.dialog.is_none();
    if nav.visible() && !testing {
        time.pause();
        drive.throttle = 0.0;
        drive.turn = 0.0;
    } else {
        time.unpause();
    }
    for (mut config, garage) in &mut physics {
        config.physics_pipeline_active = if garage { testing } else { !nav.visible() };
    }
    for mut camera in &mut world_cameras {
        camera.is_active = !nav.covers_scene();
    }
    for mut camera in &mut menu_cameras {
        camera.is_active = nav.visible();
        camera.clear_color = if nav.covers_scene() {
            ClearColorConfig::Custom(Color::BLACK)
        } else {
            ClearColorConfig::None
        };
    }
    for mut node in &mut overlays {
        let display = if !nav.visible() && settings.diagnostics {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
}
