//! Opt-in integration check: real window changes, paused simulation and audio sink.
use super::{
    Draft, Navigation, Page,
    art::MenuAssets,
    music::MenuMusic,
    ui::{self, Action, Status},
};
use crate::{
    Options,
    settings::{EngineSettings, SettingsStore},
    simulation::PhysicsTicks,
};
use bevy::{
    audio::AudioSinkPlayback,
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    window::WindowMode,
};

#[derive(Default)]
pub struct Capture {
    stage: u8,
    entered: f64,
    ticks: u64,
}

#[allow(
    clippy::too_many_arguments,
    reason = "Integration check queries real app resources"
)]
pub fn run(
    options: Res<Options>,
    time: Res<Time<Real>>,
    mut capture: Local<Capture>,
    mut nav: ResMut<Navigation>,
    mut draft: ResMut<Draft>,
    mut settings: ResMut<EngineSettings>,
    mut status: ResMut<Status>,
    assets: Res<MenuAssets>,
    sinks: Query<&AudioSink, With<MenuMusic>>,
    windows: Query<&Window>,
    cameras: Query<&Msaa, With<crate::view::DemoCamera>>,
    lights: Query<&DirectionalLight, Without<bevy::camera::visibility::RenderLayers>>,
    ticks: Res<PhysicsTicks>,
    store: Res<SettingsStore>,
    mut commands: Commands,
    mut exit: MessageWriter<crate::shutdown::ExitRequest>,
) {
    if !options.menu_smoke_test {
        return;
    }
    let seconds = time.elapsed_secs_f64();
    let delay = if capture.stage == 0 { 2.0 } else { 1.2 };
    if seconds - capture.entered < delay {
        return;
    }
    let snapshot = |commands: &mut Commands, name: &str| {
        std::fs::create_dir_all("logs").unwrap();
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(format!("logs/menu-{name}.png")));
    };
    match capture.stage {
        0 => {
            assert_eq!(assets.missing, 0);
            let sink = sinks
                .single()
                .expect("Menu music must have an audio output sink");
            assert!(!sink.is_paused() && !sink.empty() && sink.position().as_secs_f32() > 0.2);
            info!(
                "MENU CHECK: original artwork and MP3 playback active at {:?}",
                sink.position()
            );
            snapshot(&mut commands, "main");
            capture.ticks = ticks.0;
        }
        1 => {
            assert_eq!(capture.ticks, ticks.0, "Main menu must pause physics");
            ui::dispatch(
                Action::Settings,
                &mut nav,
                &mut draft,
                &mut settings,
                &mut status,
            );
        }
        2 => {
            draft.0.resolution = [1280, 720];
            draft.0.vsync = true;
            draft.0.msaa = 2;
            draft.0.shadows = false;
            draft.0.music_volume = 20;
            draft.0.diagnostics = false;
            ui::dispatch(
                Action::Apply,
                &mut nav,
                &mut draft,
                &mut settings,
                &mut status,
            );
        }
        3 => {
            let window = windows.single().unwrap();
            // Fractional display scaling rounds compositor sizes to logical pixels.
            assert!(window.physical_width().abs_diff(1280) <= 2);
            assert!(window.physical_height().abs_diff(720) <= 2);
            assert_eq!(*cameras.single().unwrap(), Msaa::Sample2);
            assert!(!lights.single().unwrap().shadow_maps_enabled);
            assert_eq!(store.load(), *settings, "Saved settings must roundtrip");
            assert!((sinks.single().unwrap().volume().to_linear() - 0.2).abs() < 0.01);
            snapshot(&mut commands, "settings");
        }
        4 => {
            draft.0.fullscreen = true;
            ui::dispatch(
                Action::Apply,
                &mut nav,
                &mut draft,
                &mut settings,
                &mut status,
            );
        }
        5 => {
            assert!(matches!(
                windows.single().unwrap().mode,
                WindowMode::BorderlessFullscreen(_)
            ));
            info!(
                "MENU CHECK: fullscreen, resolution, VSync, MSAA, shadows, volume and save passed"
            );
            snapshot(&mut commands, "fullscreen");
        }
        6 => {
            draft.0 = EngineSettings::default();
            ui::dispatch(
                Action::Apply,
                &mut nav,
                &mut draft,
                &mut settings,
                &mut status,
            );
        }
        7 => {
            assert_eq!(windows.single().unwrap().mode, WindowMode::Windowed);
            ui::dispatch(
                Action::Demo,
                &mut nav,
                &mut draft,
                &mut settings,
                &mut status,
            );
            capture.ticks = ticks.0;
        }
        8 => {
            assert!(ticks.0 > capture.ticks + 30, "Demo must resume physics");
            assert!(
                sinks.single().unwrap().is_paused(),
                "Menu music must pause outside menu"
            );
            nav.page = Page::Pause;
        }
        9 => {
            capture.ticks = ticks.0;
            snapshot(&mut commands, "pause");
        }
        10 => {
            assert_eq!(ticks.0, capture.ticks);
            assert!(!sinks.single().unwrap().is_paused());
            println!(
                "PASS: original menu artwork; MP3 output; settings applied/saved; fullscreen/windowed/resolution; menu and pause stop physics, demo resumes it"
            );
            exit.write(crate::shutdown::ExitRequest);
        }
        _ => return,
    }
    capture.stage += 1;
    capture.entered = seconds;
}
