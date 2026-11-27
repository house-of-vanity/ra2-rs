use bevy::{app::PluginGroupBuilder, prelude::*};

/// Shared window identity for the game, editor and standalone labs.
pub fn plugins(mut window: Window) -> PluginGroupBuilder {
    window.name = Some(env!("CARGO_PKG_NAME").into());
    DefaultPlugins
        .set(WindowPlugin {
            primary_window: Some(window),
            close_when_requested: false,
            ..default()
        })
        .add(DesktopIconPlugin)
}

struct DesktopIconPlugin;

impl Plugin for DesktopIconPlugin {
    fn build(&self, app: &mut App) {
        if app.is_plugin_added::<bevy::winit::WinitPlugin>() {
            #[cfg(target_os = "macos")]
            app.add_systems(Startup, set_dock_icon);
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            app.add_systems(First, set_created_window_icons);
        }
    }
}

#[cfg(target_os = "macos")]
fn set_dock_icon(_: bevy::ecs::system::NonSendMarker) {
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::{MainThreadMarker, NSData};

    let main_thread = MainThreadMarker::new().expect("AppKit runs on the main thread");
    let data = NSData::with_bytes(include_bytes!("../assets/icons/ra2-rs.icns"));
    if let Some(icon) = NSImage::initWithData(main_thread.alloc(), &data) {
        let application = NSApplication::sharedApplication(main_thread);
        // SAFETY: AppKit receives a valid non-nil NSImage on its main thread.
        unsafe { application.setApplicationIconImage(Some(&icon)) };
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn set_created_window_icons(
    mut created: MessageReader<bevy::window::WindowCreated>,
    _: bevy::ecs::system::NonSendMarker,
) {
    for event in created.read() {
        bevy::winit::WINIT_WINDOWS.with_borrow(|windows| {
            if let Some(window) = windows.get_window(event.window) {
                set_window_icon(window);
            }
        });
    }
}

#[cfg(target_os = "windows")]
fn set_window_icon(window: &winit::window::Window) {
    use winit::{
        dpi::LogicalSize,
        platform::windows::{IconExtWindows, WindowExtWindows},
        window::Icon,
    };

    let load = |size| {
        let pixels = LogicalSize::new(size, size).to_physical(window.scale_factor());
        Icon::from_resource(1, Some(pixels))
            .inspect_err(|error| warn!("Could not load the Windows icon: {error}"))
            .ok()
    };
    window.set_window_icon(load(16));
    window.set_taskbar_icon(load(32));
}

#[cfg(target_os = "linux")]
fn set_window_icon(window: &winit::window::Window) {
    // X11 accepts window pixels. Wayland looks up the desktop entry by Window::name.
    let pixels = image::load_from_memory_with_format(
        include_bytes!("../assets/icons/ra2-rs-256.png"),
        image::ImageFormat::Png,
    )
    .expect("embedded window icon")
    .into_rgba8();
    let (width, height) = pixels.dimensions();
    let icon = winit::window::Icon::from_rgba(pixels.into_raw(), width, height)
        .expect("valid RGBA window icon");
    window.set_window_icon(Some(icon));
}
