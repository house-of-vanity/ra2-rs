mod benchmark;
mod config;
mod damage;
mod destruction;
mod editor;
mod fleet;
mod fleet_validation;
mod frontend;
mod garage;
mod launch;
mod legacy_ui;
mod scene;
mod settings;
mod shutdown;
mod simulation;
mod team_hq;
mod ui_sound;
mod validation;
mod view;
mod workshop_check;

use bevy::{diagnostic::FrameTimeDiagnosticsPlugin, prelude::*};
use bevy_rapier3d::prelude::*;

const STEP: f64 = 1.0 / 120.0;
const MAX_OBJECTS: usize = 10_000;

#[derive(Resource, Clone)]
struct Options {
    objects: usize,
    headless: bool,
    smoke_test: bool,
    arena: bool,
    showcase: bool,
    config: std::path::PathBuf,
    game_dir: std::path::PathBuf,
    all_bots: bool,
    bot_copies: usize,
    bot_physics: bool,
    menu: bool,
    menu_smoke_test: bool,
    settings_file: std::path::PathBuf,
    editor: bool,
    editor_file: Option<std::path::PathBuf>,
    editor_smoke_test: bool,
    workshop_check: bool,
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
    let mut scene = scene::Scene::load(&options).unwrap_or_else(|error| {
        eprintln!("Cannot load arena: {error}");
        std::process::exit(1);
    });
    let fleet = fleet::Fleet::load(&options).unwrap_or_else(|error| {
        eprintln!("Cannot load robots: {error}");
        std::process::exit(1)
    });
    if options.all_bots {
        scene.lab_half_size = fleet.arena_half_size();
        scene.origin = Vec3::new(0.0, 1.1, scene.lab_half_size - 3.5);
        scene.title = "RA2 / All Robots Benchmark".into();
    }
    if options.headless {
        if options.all_bots {
            fleet_validation::check(options, scene, fleet);
            return;
        }
        validation::check_physics(options, scene);
        return;
    }
    println!(
        "WASD/arrows: test robot | +/-: 100 cubes (Shift: 1000) | Space: shake | C: clear | R: reset | F: camera | Tab: inspect bot | H: HUD | V: VSync | T: shadows | Esc: menu"
    );
    let store = settings::SettingsStore(options.settings_file.clone());
    let preferences = if options.smoke_test || options.menu_smoke_test || options.editor_smoke_test
    {
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
    let workshop_check = options.workshop_check;
    let mut app = App::new();
    app.insert_resource(options)
        .insert_resource(scene)
        .insert_resource(fleet)
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
        simulation::SimulationPlugin,
        RapierDiagnosticsPlugin::default().in_schedule(FixedUpdate),
        view::ViewPlugin,
        frontend::FrontendPlugin,
        ui_sound::UiSoundPlugin,
        legacy_ui::LegacyUiPlugin,
        team_hq::TeamHqPlugin,
        editor::EditorPlugin,
        garage::GaragePlugin,
    ))
    .add_systems(Startup, fleet::spawn.after(simulation::setup))
    .add_systems(Update, benchmark::sample);
    if workshop_check {
        app.add_plugins(workshop_check::WorkshopCheckPlugin);
    }
    app.run();
}
