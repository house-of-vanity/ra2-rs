mod battle;
mod config;
mod content;
mod damage;
mod destruction;
mod editor;
mod exhibition;
mod frontend;
mod gameplay;
mod garage;
mod launch;
mod legacy_ui;
mod multiplayer;
mod settings;
mod shutdown;
mod team_hq;
mod ui_sound;
mod workshop_check;

use bevy::{diagnostic::FrameTimeDiagnosticsPlugin, prelude::*};
use bevy_rapier3d::prelude::*;

const STEP: f64 = 1.0 / 120.0;

#[derive(Resource, Clone)]
struct Options {
    smoke_test: bool,
    config: std::path::PathBuf,
    game_dir: std::path::PathBuf,
    settings_file: std::path::PathBuf,
    editor: bool,
    editor_file: Option<std::path::PathBuf>,
    editor_smoke_test: bool,
    workshop_check: bool,
    exhibition_check: bool,
    multiplayer_check: bool,
    destruction: bool,
    destruction_check: bool,
    damage_demo: bool,
    damage_check: bool,
}

fn main() {
    if std::env::args_os()
        .skip(1)
        .any(|a| a == "--help" || a == "-h")
    {
        println!("{}", launch::USAGE);
        return;
    }
    if std::env::args_os().skip(1).any(|a| a == "--version") {
        println!("physics-demo {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let options = Options::parse().unwrap_or_else(|error| {
        eprintln!("{error}\n\n{}", launch::USAGE);
        std::process::exit(2);
    });
    if options.damage_demo {
        if let Err(error) = damage::run(options) {
            eprintln!("Cannot start damage demo: {error}");
            std::process::exit(1);
        }
        return;
    }
    if options.destruction {
        if let Err(error) = destruction::run(options) {
            eprintln!("Cannot start destruction demo: {error}");
            std::process::exit(1);
        }
        return;
    }
    let config = config::Config::load(&options.config).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1)
    });
    let store = settings::SettingsStore(options.settings_file.clone());
    let preferences = if options.smoke_test || options.editor_smoke_test {
        settings::EngineSettings::default()
    } else {
        store.load()
    };
    let mut window = Window {
        title: "Robot Arena 2 / Open Arena".into(),
        mode: preferences.window_mode(),
        present_mode: preferences.present_mode(),
        ..default()
    };
    window
        .resolution
        .set_physical_resolution(preferences.resolution[0], preferences.resolution[1]);
    let workshop_check =
        options.workshop_check || options.exhibition_check || options.multiplayer_check;
    let mut app = App::new();
    app.insert_resource(options)
        .insert_resource(config)
        .insert_resource(preferences)
        .insert_resource(store)
        .insert_resource(ClearColor(Color::srgb(0.045, 0.065, 0.10)));
    let plugins = DefaultPlugins.set(WindowPlugin {
        primary_window: Some(window),
        close_when_requested: false,
        ..default()
    });
    if workshop_check {
        app.add_plugins(plugins.disable::<bevy::winit::WinitPlugin>())
            .add_plugins(bevy::app::ScheduleRunnerPlugin::run_loop(
                std::time::Duration::from_secs_f64(1. / 60.),
            ));
    } else {
        app.add_plugins(plugins);
    }
    app.add_plugins((
        shutdown::ShutdownPlugin,
        FrameTimeDiagnosticsPlugin::new(120),
        RapierDiagnosticsPlugin::default().in_schedule(FixedUpdate),
        frontend::FrontendPlugin,
        ui_sound::UiSoundPlugin,
        legacy_ui::LegacyUiPlugin,
        team_hq::TeamHqPlugin,
        editor::EditorPlugin,
        garage::GaragePlugin,
        gameplay::GameplayPlugin,
        battle::BattlePlugin,
        multiplayer::MultiplayerPlugin,
        exhibition::ExhibitionPlugin,
    ));
    if app.world().resource::<Options>().workshop_check {
        app.add_plugins(workshop_check::WorkshopCheckPlugin);
    }
    if app.world().resource::<Options>().exhibition_check {
        app.add_plugins(exhibition::check::CheckPlugin);
    }
    if app.world().resource::<Options>().multiplayer_check {
        app.add_plugins(multiplayer::check::CheckPlugin);
    }
    app.run();
}
