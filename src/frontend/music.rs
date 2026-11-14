//! One looping original menu track. No external player or original executable.
use super::{Navigation, Page, art::MenuAssets};
use crate::{Options, settings::EngineSettings};
use bevy::{
    audio::{AudioSinkPlayback, Volume},
    prelude::*,
};

#[derive(Component)]
pub struct MenuMusic;
#[derive(Component)]
pub struct WorkshopMusic;

pub fn setup(
    mut commands: Commands,
    mut audio: ResMut<Assets<AudioSource>>,
    assets: Res<MenuAssets>,
    options: Res<Options>,
    settings: Res<EngineSettings>,
    nav: Res<Navigation>,
) {
    for (relative, workshop) in [
        (assets.theme.music.as_str(), false),
        ("Music/Workshop/Deep inside the Bot Lab.mp3", true),
    ] {
        let path = options.game_dir.join(relative);
        match std::fs::read(&path) {
            Ok(bytes) => {
                let handle = audio.add(AudioSource {
                    bytes: bytes.into(),
                });
                let in_workshop = nav.page == Page::Workshop
                    || (nav.page == Page::Settings && nav.back == Page::Workshop);
                let mut entity = commands.spawn((
                    AudioPlayer::new(handle),
                    PlaybackSettings {
                        paused: !nav.visible() || workshop != in_workshop,
                        volume: Volume::Linear(settings.music_volume as f32 / 100.0),
                        ..PlaybackSettings::LOOP
                    },
                ));
                if workshop {
                    entity.insert(WorkshopMusic);
                } else {
                    entity.insert(MenuMusic);
                }
                info!("UI music: {}", path.display());
            }
            Err(e) => warn!("Cannot load UI music {}: {e}", path.display()),
        }
    }
}

#[allow(clippy::type_complexity, reason = "Two distinct UI music channels")]
pub fn synchronize(
    nav: Res<Navigation>,
    settings: Res<EngineSettings>,
    mut sinks: Query<
        (&mut AudioSink, Option<&WorkshopMusic>),
        Or<(With<MenuMusic>, With<WorkshopMusic>)>,
    >,
) {
    let in_workshop =
        nav.page == Page::Workshop || (nav.page == Page::Settings && nav.back == Page::Workshop);
    for (mut sink, workshop) in &mut sinks {
        let playing = nav.visible() && workshop.is_some() == in_workshop;
        sink.set_volume(Volume::Linear(settings.music_volume as f32 / 100.0));
        if playing && sink.is_paused() {
            sink.play();
        }
        if !playing && !sink.is_paused() {
            sink.pause();
        }
    }
}
