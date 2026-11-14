//! Keep the presentation surface alive while the compositor leaves fullscreen.
//!
//! Destroying a fullscreen Wayland/Vulkan surface in the same step as disabling
//! VRR can trigger an NVIDIA GSP display-task fault (Xid 120). This is a driver
//! workaround, not a guarantee that all VRR transitions are safe.
use bevy::{
    app::AppExit,
    ecs::system::NonSendMarker,
    prelude::*,
    window::{PresentMode, WindowCloseRequested, WindowMode},
    winit::{WINIT_WINDOWS, WinitSettings},
};
use std::time::Duration;

const SETTLE_TIME: Duration = Duration::from_millis(500);
const EXIT_TIMEOUT: Duration = Duration::from_secs(3);
const SETTLE_FRAMES: u32 = 3;

/// Normal application exits must go through this message, rather than AppExit.
#[derive(Message)]
pub struct ExitRequest;

pub struct ShutdownPlugin;

impl Plugin for ShutdownPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ExitRequest>()
            .init_resource::<Shutdown>()
            // All UI/settings updates finish before we change the window. Winit
            // applies these changes in Last; rendering continues during the wait.
            .add_systems(PostUpdate, (request_exit, finish_exit).chain());
    }
}

#[derive(Resource, Default)]
struct Shutdown {
    pending: Option<PendingExit>,
    finished: bool,
}

struct PendingExit {
    windows: Vec<Entity>,
    requested_at: Duration,
    windowed_at: Option<Duration>,
    windowed_frames: u32,
}

impl PendingExit {
    fn ready(&mut self, now: Duration, windowed: bool) -> bool {
        if !windowed {
            self.windowed_at = None;
            self.windowed_frames = 0;
            return false;
        }
        let since = *self.windowed_at.get_or_insert(now);
        self.windowed_frames += 1;
        now.saturating_sub(since) >= SETTLE_TIME && self.windowed_frames >= SETTLE_FRAMES
    }
}

fn request_exit(
    mut requests: MessageReader<ExitRequest>,
    mut close_requests: MessageReader<WindowCloseRequested>,
    mut shutdown: ResMut<Shutdown>,
    time: Res<Time<Real>>,
    mut windows: Query<(Entity, &mut Window)>,
    winit_settings: Option<ResMut<WinitSettings>>,
    mut exit: MessageWriter<AppExit>,
) {
    let requested = requests.read().count() > 0;
    let closed = close_requests.read().any(|e| windows.contains(e.window));
    if (!requested && !closed) || shutdown.pending.is_some() || shutdown.finished {
        return;
    }

    let fullscreen: Vec<_> = windows
        .iter()
        .filter(|(_, window)| window.mode != WindowMode::Windowed)
        .map(|(entity, _)| entity)
        .collect();
    if fullscreen.is_empty() {
        shutdown.finished = true;
        exit.write(AppExit::Success);
        return;
    }

    for &entity in &fullscreen {
        let (_, mut window) = windows.get_mut(entity).unwrap();
        window.mode = WindowMode::Windowed;
        window.set_maximized(false);
        // Avoid retaining a desktop-sized surface that may still be scanned out
        // directly. Only the live Window changes; saved preferences stay intact.
        let width = window.resolution.physical_width().min(1280);
        let height = window.resolution.physical_height().min(720);
        window.resolution.set_physical_resolution(width, height);
        window.present_mode = PresentMode::Fifo;
    }
    if let Some(mut settings) = winit_settings {
        // A close request can arrive after losing focus. Keep producing frames
        // even then, instead of waiting on the unfocused reactive-update timer.
        *settings = WinitSettings::continuous();
    }
    info!("Exit requested: leaving fullscreen before releasing the presentation surface");
    shutdown.pending = Some(PendingExit {
        windows: fullscreen,
        requested_at: time.elapsed(),
        windowed_at: None,
        windowed_frames: 0,
    });
}

fn finish_exit(
    mut shutdown: ResMut<Shutdown>,
    time: Res<Time<Real>>,
    mut exit: MessageWriter<AppExit>,
    _main_thread: NonSendMarker,
) {
    let Some(pending) = shutdown.pending.as_mut() else {
        return;
    };
    // Window.mode is only our request. On Wayland fullscreen() reflects the
    // compositor's last xdg_toplevel configure, so wait for that acknowledgement.
    let windowed = WINIT_WINDOWS.with_borrow(|windows| {
        pending.windows.iter().all(|entity| {
            windows
                .get_window(*entity)
                .is_none_or(|window| window.fullscreen().is_none())
        })
    });
    let now = time.elapsed();
    let ready = pending.ready(now, windowed);
    let timed_out = now.saturating_sub(pending.requested_at) >= EXIT_TIMEOUT;
    if ready || timed_out {
        if !ready {
            warn!("Fullscreen exit did not settle within 3 seconds; finishing shutdown");
        } else {
            info!("Windowed presentation settled; finishing shutdown");
        }
        shutdown.pending = None;
        shutdown.finished = true;
        exit.write(AppExit::Success);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(fullscreen: bool) -> App {
        let mut app = App::new();
        app.insert_resource(Time::<Real>::default())
            .add_plugins(WindowPlugin {
                primary_window: Some(Window {
                    mode: if fullscreen {
                        WindowMode::BorderlessFullscreen(bevy::window::MonitorSelection::Current)
                    } else {
                        WindowMode::Windowed
                    },
                    resolution: (3840, 2160).into(),
                    ..default()
                }),
                close_when_requested: false,
                ..default()
            })
            .add_plugins(ShutdownPlugin);
        app
    }

    #[test]
    fn quit_keeps_fullscreen_window_alive_and_preferences_unchanged() {
        let mut app = app(true);
        let preferences = crate::settings::EngineSettings {
            fullscreen: true,
            ..default()
        };
        app.insert_resource(preferences.clone());
        app.world_mut().write_message(ExitRequest);
        app.update();
        assert!(app.should_exit().is_none());
        let window = app
            .world_mut()
            .query::<&Window>()
            .single(app.world())
            .unwrap();
        assert_eq!(window.mode, WindowMode::Windowed);
        assert_eq!(window.resolution.physical_width(), 1280);
        assert_eq!(window.present_mode, PresentMode::Fifo);
        assert_eq!(
            *app.world().resource::<crate::settings::EngineSettings>(),
            preferences
        );

        // Repeated Quit must not restart the timer or bypass the wait.
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .advance_by(Duration::from_millis(100));
        app.world_mut().write_message(ExitRequest);
        app.update();
        assert!(app.should_exit().is_none());
        assert_eq!(
            app.world()
                .resource::<Shutdown>()
                .pending
                .as_ref()
                .unwrap()
                .requested_at,
            Duration::ZERO
        );
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .advance_by(SETTLE_TIME);
        app.update();
        assert_eq!(app.should_exit(), Some(AppExit::Success));
    }

    #[test]
    fn window_manager_close_uses_the_same_transition() {
        let mut app = app(true);
        let entity = app
            .world_mut()
            .query_filtered::<Entity, With<Window>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .write_message(WindowCloseRequested { window: entity });
        app.update();
        assert!(app.should_exit().is_none());
        assert!(app.world().get::<Window>(entity).is_some());
        assert!(app.world().resource::<Shutdown>().pending.is_some());
    }

    #[test]
    fn windowed_quit_does_not_need_a_transition() {
        let mut app = app(false);
        app.world_mut().write_message(ExitRequest);
        app.update();
        assert_eq!(app.should_exit(), Some(AppExit::Success));
    }

    #[test]
    fn unresponsive_backend_cannot_leave_a_lingering_process() {
        let mut app = app(true);
        app.world_mut().write_message(ExitRequest);
        app.update();
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .advance_by(EXIT_TIMEOUT);
        // Only two updates: the normal minimum-frame condition cannot pass.
        app.update();
        assert_eq!(app.should_exit(), Some(AppExit::Success));
    }

    #[test]
    fn wait_starts_at_backend_acknowledgement_and_requires_multiple_frames() {
        let mut pending = PendingExit {
            windows: Vec::new(),
            requested_at: Duration::ZERO,
            windowed_at: None,
            windowed_frames: 0,
        };
        assert!(!pending.ready(Duration::from_secs(1), false));
        assert!(!pending.ready(Duration::from_secs(1), true));
        assert!(!pending.ready(Duration::from_secs(2), true));
        assert!(pending.ready(Duration::from_secs(2), true));
        // A fullscreen configure arriving again invalidates the previous wait.
        assert!(!pending.ready(Duration::from_secs(2), false));
        assert!(!pending.ready(Duration::from_secs(2), true));
    }
}
