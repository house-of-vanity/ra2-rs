//! User preferences and runtime adapters, independent of the imported game settings.
use bevy::{
    prelude::*,
    window::{MonitorSelection, PresentMode, WindowMode},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Resource, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EngineSettings {
    pub version: u32,
    pub fullscreen: bool,
    pub resolution: [u32; 2],
    pub vsync: bool,
    pub msaa: u32,
    pub shadows: bool,
    pub diagnostics: bool,
    pub music_volume: u32,
}
impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            version: 1,
            fullscreen: false,
            resolution: [1600, 900],
            vsync: false,
            msaa: 4,
            shadows: true,
            diagnostics: true,
            music_volume: 50,
        }
    }
}
impl EngineSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("Unsupported settings version".into());
        }
        if !(640..=7680).contains(&self.resolution[0])
            || !(480..=4320).contains(&self.resolution[1])
        {
            return Err("Invalid window resolution".into());
        }
        if ![1, 2, 4, 8].contains(&self.msaa) || self.music_volume > 100 {
            return Err("Invalid MSAA or music volume".into());
        }
        Ok(())
    }
    pub fn window_mode(&self) -> WindowMode {
        if self.fullscreen {
            WindowMode::BorderlessFullscreen(MonitorSelection::Current)
        } else {
            WindowMode::Windowed
        }
    }
    pub fn present_mode(&self) -> PresentMode {
        if self.vsync {
            PresentMode::AutoVsync
        } else {
            PresentMode::AutoNoVsync
        }
    }
    pub fn anti_aliasing(&self) -> Msaa {
        match self.msaa {
            2 => Msaa::Sample2,
            4 => Msaa::Sample4,
            8 => Msaa::Sample8,
            _ => Msaa::Off,
        }
    }
}

#[derive(Resource)]
pub struct SettingsStore(pub PathBuf);
impl SettingsStore {
    pub fn load(&self) -> EngineSettings {
        let result = std::fs::read_to_string(&self.0)
            .map_err(|e| e.to_string())
            .and_then(|s| ron::from_str::<EngineSettings>(&s).map_err(|e| e.to_string()))
            .and_then(|s| s.validate().map(|()| s));
        match result {
            Ok(s) => s,
            Err(e) => {
                if self.0.exists() {
                    warn!(
                        "Cannot load settings {}: {e}; using defaults",
                        self.0.display()
                    );
                }
                EngineSettings::default()
            }
        }
    }
    pub fn save(&self, settings: &EngineSettings) -> Result<(), String> {
        settings.validate()?;
        let data = ron::ser::to_string_pretty(settings, ron::ser::PrettyConfig::default())
            .map_err(|e| e.to_string())?;
        if let Some(parent) = self.0.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let temporary = self.0.with_extension("ron.tmp");
        std::fs::write(&temporary, data).map_err(|e| e.to_string())?;
        std::fs::rename(temporary, &self.0).map_err(|e| e.to_string())
    }
}

pub fn apply(
    settings: Res<EngineSettings>,
    store: Res<SettingsStore>,
    mut previous: Local<Option<EngineSettings>>,
    mut windows: Query<&mut Window>,
    // Bevy keys shared camera buffers by MSAA as well as the output target.
    // The window's overlay must share the scene's samples: otherwise a
    // non-clearing UI camera loads its own previous frame instead of the scene.
    mut cameras: Query<&mut Msaa, Or<(With<Camera3d>, With<crate::frontend::MenuCamera>)>>,
    mut lights: Query<&mut DirectionalLight>,
) {
    if previous.as_ref() == Some(&*settings) {
        return;
    }
    let old = previous.as_ref();
    for mut window in &mut windows {
        // Borderless fullscreen always follows the desktop resolution on Wayland.
        // Remember the independent window size for when the user leaves fullscreen.
        if old.is_none_or(|s| s.fullscreen != settings.fullscreen) {
            window.mode = settings.window_mode();
        }
        if !settings.fullscreen
            && old.is_none_or(|s| s.fullscreen || s.resolution != settings.resolution)
        {
            window
                .resolution
                .set_physical_resolution(settings.resolution[0], settings.resolution[1]);
        }
        window.present_mode = settings.present_mode();
    }
    for mut msaa in &mut cameras {
        *msaa = settings.anti_aliasing();
    }
    for mut light in &mut lights {
        light.shadow_maps_enabled = settings.shadows;
    }
    if old.is_some() {
        if let Err(e) = store.save(&settings) {
            error!("Cannot save settings: {e}");
        } else {
            info!("Settings saved to {}", store.0.display());
        }
    }
    *previous = Some(settings.clone());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_roundtrip_and_reject_incompatible_values() {
        let s = EngineSettings {
            fullscreen: true,
            msaa: 8,
            music_volume: 0,
            ..default()
        };
        let restored: EngineSettings = ron::from_str(&ron::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, restored);
        assert!(restored.validate().is_ok());
        assert!(
            EngineSettings {
                version: 2,
                ..default()
            }
            .validate()
            .is_err()
        );
        assert!(
            EngineSettings {
                msaa: 3,
                ..default()
            }
            .validate()
            .is_err()
        );
        assert!(
            EngineSettings {
                resolution: [0, 720],
                ..default()
            }
            .validate()
            .is_err()
        );
        assert!(
            EngineSettings {
                music_volume: 101,
                ..default()
            }
            .validate()
            .is_err()
        );
    }
}
