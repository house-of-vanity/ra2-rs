//! Native PCM cues and bounded spatial voices. Files are decoded only when used.
use super::*;
use arena_core::mechanism::ActuatorKind;
use bevy::audio::{SpatialScale, Volume};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Resource, Default)]
struct Bank {
    sources: BTreeMap<String, Option<Handle<crate::ui_sound::Pcm>>>,
    firing: BTreeSet<(u32, usize)>,
}
impl Bank {
    fn get(
        &mut self,
        name: &str,
        catalog: &crate::editor::catalog::Catalog,
        assets: &mut Assets<crate::ui_sound::Pcm>,
    ) -> Option<Handle<crate::ui_sound::Pcm>> {
        let key = ra2_formats::components::normalize(name);
        self.sources
            .entry(key.clone())
            .or_insert_with(|| {
                let path = catalog.index.get(&key)?;
                match std::fs::read(path)
                    .map_err(|e| e.to_string())
                    .and_then(|b| crate::ui_sound::Pcm::decode(&b))
                {
                    Ok(source) => Some(assets.add(source)),
                    Err(error) => {
                        warn!("Battle sound {}: {error}", path.display());
                        None
                    }
                }
            })
            .clone()
    }
}
#[derive(Component)]
struct BattleVoice;
#[derive(Component)]
struct IntroVoice;
#[derive(Component)]
struct LoopVoice(u32, usize);
pub(crate) struct MatchAudioPlugin;
impl Plugin for MatchAudioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Bank>()
            .add_systems(Startup, listener.after(super::setup))
            .add_systems(Update, play.after(super::lifecycle));
    }
}
fn listener(mut commands: Commands, cameras: Query<Entity, With<MatchCamera>>) {
    for camera in &cameras {
        commands.entity(camera).insert(SpatialListener::new(0.3));
    }
}
fn settings(spatial: bool, looping: bool, volume: f32) -> PlaybackSettings {
    PlaybackSettings {
        spatial,
        volume: Volume::Linear(volume),
        spatial_scale: Some(SpatialScale::new(0.15)),
        ..if looping {
            PlaybackSettings::LOOP
        } else {
            PlaybackSettings::DESPAWN
        }
    }
}
fn choose<'a>(paths: &'a str, sequence: usize) -> Option<&'a str> {
    let paths = paths
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    (!paths.is_empty()).then(|| paths[sequence % paths.len()])
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn play(
    mut commands: Commands,
    mut session: ResMut<Session>,
    mut bank: ResMut<Bank>,
    mut assets: ResMut<Assets<crate::ui_sound::Pcm>>,
    catalog: Res<crate::editor::catalog::Catalog>,
    voices: Query<Entity, With<BattleVoice>>,
    mut motors: Query<(
        Entity,
        &LoopVoice,
        &mut Transform,
        Option<&mut SpatialAudioSink>,
        Option<&mut AudioSink>,
    )>,
    poses: Query<&Transform, (With<RobotBody>, Without<LoopVoice>)>,
    nav: Res<Navigation>,
    intro_voices: Query<Entity, With<IntroVoice>>,
    preferences: Res<crate::settings::EngineSettings>,
    mut network: ResMut<crate::multiplayer::Multiplayer>,
) {
    let audible = matches!(nav.page, Page::Match | Page::Results);
    let running = nav.page == Page::Match && matches!(session.phase, Phase::Playing);
    let intro = matches!(session.phase, Phase::Introduction(_));
    if !intro && audible {
        for entity in &intro_voices {
            commands.entity(entity).despawn();
        }
    }
    let intro_volume = if let Phase::Introduction(age) = session.phase {
        (age / 0.25).clamp(0., 1.)
            * ((session.presentation.duration - age) / session.presentation.fade_out.max(0.01))
                .clamp(0., 1.)
            * 0.2
            * preferences.music_volume as f32
            / 100.
    } else {
        0.
    };
    if !audible {
        for e in &voices {
            commands.entity(e).despawn();
        }
        session.sound_events.clear();
        session.impacts.clear();
        bank.firing.clear();
    }
    let mut desired = BTreeMap::new();
    let mut firing = BTreeSet::new();
    if audible {
        desired.insert(
            (0, usize::MAX),
            ("Sounds/crowd/LoudCheer_Loop.wav".into(), Vec3::ZERO),
        );
        if intro && let Some(music) = &session.presentation.music {
            desired.insert((0, usize::MAX - 1), (music.clone(), Vec3::ZERO));
        }
    }
    let mut events = if audible {
        std::mem::take(&mut session.sound_events)
    } else {
        vec![]
    };
    // Sounds follow native CFZ definitions and operational actuator state.
    for robot in session.robots.iter().filter(|r| running && r.running) {
        let (Some(rig), Some(document)) = (&robot.rig, &robot.document) else {
            continue;
        };
        for (j, joint) in rig.joints.iter().enumerate() {
            let Some(component) = document.components.get(joint.component) else {
                continue;
            };
            let Some(asset) = catalog
                .components
                .get(&ra2_formats::components::normalize(&component.definition))
            else {
                continue;
            };
            if !robot.damage.health.operational(joint.component) {
                continue;
            }
            let position = robot
                .bodies
                .get(joint.child)
                .and_then(|e| poses.get(*e).ok())
                .map_or(Vec3::ZERO, |p| p.translation);
            let key = (robot.id, joint.component);
            let state = &robot.states[j];
            let command = robot
                .commands
                .get(joint.component)
                .copied()
                .unwrap_or_default();
            let burst = matches!(joint.kind, ActuatorKind::Burst | ActuatorKind::BurstPiston);
            let active = if burst {
                state.burst_time > 0.
            } else {
                command.drive.abs().max(command.steer.abs()) > 0.02
            };
            let powered = if matches!(joint.kind, ActuatorKind::Piston | ActuatorKind::BurstPiston)
            {
                rig.pneumatic.remaining > 0.
            } else {
                rig.electrical.remaining > 0.
            };
            if active && powered {
                let motor = asset.definition.value("motorsound");
                if !motor.is_empty() {
                    desired.insert(key, (motor.to_string(), position));
                }
                if !asset.definition.value("firesound").is_empty() {
                    firing.insert(key);
                    if !bank.firing.contains(&key) && (!session.network || session.authority) {
                        events.push((asset.definition.value("firesound").into(), position));
                    }
                }
            }
        }
    }
    if running {
        if session.hazards
            && let Some(arena) = session.arenas.get(session.arena)
        {
            for (index, path) in arena.ambience.iter().enumerate() {
                desired.insert((u32::MAX - 1, index), (path.clone(), Vec3::ZERO));
            }
        }
        for trap in &session.hazards_runtime {
            if let Some(sound) = trap.loop_sound() {
                desired.insert(
                    (u32::MAX, trap.index),
                    (sound.to_string(), trap.sound_position),
                );
            }
        }
    }
    bank.firing = firing;
    for (id, part, amount, position) in std::mem::take(&mut session.impacts) {
        let paths = session
            .robots
            .iter()
            .find(|r| r.id == id)
            .and_then(|r| r.document.as_ref())
            .and_then(|r| r.components.get(part))
            .and_then(|c| {
                catalog
                    .components
                    .get(&ra2_formats::components::normalize(&c.definition))
            })
            .map(|c| c.definition.value("damagesounds"))
            .unwrap_or("");
        events.push((
            choose(paths, amount as usize)
                .unwrap_or("Sounds/steel_collision2.wav")
                .into(),
            position,
        ));
        if amount >= 35. {
            events.push(("Sounds/critical_hit.wav".into(), position));
        }
    }
    for (entity, motor, mut pose, spatial, plain) in &mut motors {
        if let Some((_, position)) = desired.remove(&(motor.0, motor.1)) {
            pose.translation = position;
            let volume = Volume::Linear(if motor.0 == 0 {
                if motor.1 == usize::MAX - 1 {
                    intro_volume
                } else {
                    0.08
                }
            } else {
                0.3
            });
            if let Some(mut sink) = spatial {
                sink.set_volume(volume);
            }
            if let Some(mut sink) = plain {
                sink.set_volume(volume);
            }
        } else {
            commands.entity(entity).despawn();
        }
    }
    // Each actuator owns at most one loop. Pausing, destruction or scene cleanup stops it.
    for ((id, part), (path, position)) in desired
        .into_iter()
        .take(32usize.saturating_sub(motors.iter().count()))
    {
        if let Some(handle) = bank.get(&path, &catalog, &mut assets) {
            commands.spawn((
                LoopVoice(id, part),
                Transform::from_translation(position),
                AudioPlayer(handle),
                settings(
                    id != 0,
                    true,
                    if id == 0 {
                        if part == usize::MAX - 1 {
                            intro_volume
                        } else {
                            0.08
                        }
                    } else {
                        0.3
                    },
                ),
            ));
        }
    }
    if session.network && session.authority && !events.is_empty() {
        network.sounds(
            events
                .iter()
                .map(|(p, v)| (p.clone(), v.to_array()))
                .collect(),
        );
    }
    for (path, position) in events
        .into_iter()
        .take(24usize.saturating_sub(voices.iter().count()))
    {
        if let Some(handle) = bank.get(&path, &catalog, &mut assets) {
            let mut entity = commands.spawn((
                BattleVoice,
                Transform::from_translation(position),
                AudioPlayer(handle),
                settings(
                    !path.to_ascii_lowercase().contains("announcers/")
                        && !path.ends_with("match_countdown.wav")
                        && !path.ends_with("immobile_countdown.wav"),
                    false,
                    0.7,
                ),
            ));
            if intro {
                entity.insert(IntroVoice);
            }
        }
    }
}
