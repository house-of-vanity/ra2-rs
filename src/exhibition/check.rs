//! Focused integration exercise of native Exhibition UI, shared physics and cleanup.
use super::*;
use bevy::{
    camera::RenderTarget,
    input::{ButtonState, mouse::MouseButtonInput},
    time::TimeUpdateStrategy,
};
#[derive(Resource)]
struct Capture(Handle<Image>);
#[derive(Default)]
struct Check {
    stage: usize,
    frames: usize,
    release: Option<Entity>,
    position: Vec3,
    received: f32,
    watch: Option<(Entity, f32)>,
    rise: f32,
    picker_step: usize,
    chosen_bot: Option<usize>,
    mode_case: usize,
    mode_step: usize,
    mode_wait: usize,
    camera_step: usize,
    intro_step: usize,
    frozen: Vec<(Entity, Transform)>,
}
pub(crate) struct CheckPlugin;
impl Plugin for CheckPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1. / 60.),
        ))
        .add_systems(
            Startup,
            setup
                .after(crate::battle::setup)
                .after(crate::frontend::FrontendSetup),
        )
        .add_systems(
            Update,
            run.after(crate::legacy_ui::Paint)
                .after(crate::gameplay::numbers::project),
        );
    }
}
fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    cameras: Query<Entity, Or<(With<MatchCamera>, With<crate::frontend::MenuCamera>)>>,
) {
    let image = images.add(Image::new_target_texture(
        800,
        600,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    for camera in &cameras {
        commands
            .entity(camera)
            .insert(RenderTarget::Image(image.clone().into()));
    }
    commands.insert_resource(Capture(image));
}
fn shot(world: &mut World, name: &str) {
    let image = world.resource::<Capture>().0.clone();
    let mut capture = world.spawn(bevy::render::view::screenshot::Screenshot::image(image));
    capture.observe(bevy::render::view::screenshot::save_to_disk(format!(
        "logs/exhibition-{name}.png"
    )));
    if name == "battle" {
        capture.observe(|captured: On<bevy::render::view::screenshot::ScreenshotCaptured>| {
            // Inspect GPU output too: healthy physics and correct CPU widget
            // state do not detect a black scene or a stale overlay buffer.
            let image = &captured.image;
            let bytes = image.data.as_ref().expect("Captured GPU pixels");
            let width = image.width() as usize;
            let height = image.height() as usize;
            assert_eq!(bytes.len(), width * height * 4);
            let mut lit = 0;
            let mut sampled = 0;
            for y in height / 5..height * 4 / 5 {
                for x in width / 5..width * 4 / 5 {
                    let at = (y * width + x) * 4;
                    if bytes[at..at + 3].iter().copied().max().unwrap() > 30 { lit += 1; }
                    sampled += 1;
                }
            }
            assert!(lit > sampled / 3, "Battle camera was covered by a black/stale UI buffer: {lit}/{sampled} visible scene pixels");
            info!("PASS: GPU battle scene visible through transparent HUD ({lit}/{sampled} pixels)");
        });
    }
}
fn click(world: &mut World, name: &str) -> Entity {
    let output = world.resource::<legacy_ui::Output>();
    let hit = output
        .hits
        .iter()
        .rev()
        .find(|h| h.name == name)
        .unwrap_or_else(|| panic!("Missing Exhibition control {name}"));
    let rect = hit.rect.intersect(hit.clip);
    assert!(!rect.is_empty(), "Clipped Exhibition control {name}");
    let point = output.origin + rect.center() * output.scale;
    let (window, mut data) = world
        .query::<(Entity, &mut Window)>()
        .single_mut(world)
        .unwrap();
    data.set_cursor_position(Some(point));
    world.write_message(MouseButtonInput {
        button: MouseButton::Left,
        state: ButtonState::Pressed,
        window,
    });
    window
}
fn camera_key(world: &mut World, state: ButtonState) {
    let window = world
        .query_filtered::<Entity, With<Window>>()
        .single(world)
        .unwrap();
    world.write_message(bevy::input::keyboard::KeyboardInput {
        key_code: KeyCode::F3,
        logical_key: bevy::input::keyboard::Key::F3,
        state,
        text: None,
        repeat: false,
        window,
    });
}
fn run(world: &mut World, mut check: Local<Check>) {
    check.frames += 1;
    if let Some((entity, initial)) = check.watch
        && let Some(pose) = world.get::<Transform>(entity)
    {
        check.rise = check.rise.max(pose.translation.y - initial);
    }
    if let Some(window) = check.release.take() {
        world.write_message(MouseButtonInput {
            button: MouseButton::Left,
            state: ButtonState::Released,
            window,
        });
    }
    if check.stage == 0 && std::env::var_os("RA2_CHECK_MODES_ONLY").is_some() {
        check.stage = 35;
        check.mode_case = std::env::var("RA2_CHECK_START")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        world.resource_mut::<Navigation>().page = Page::Exhibition;
    }
    if check.frames < 12 {
        return;
    }
    if check.stage >= 35 {
        check.frames = 0;
        modes(world, &mut check);
        return;
    }
    check.frames = 0;
    match check.stage {
        0 => world.resource_mut::<Navigation>().page = Page::Exhibition,
        1 => {
            assert_eq!(
                world
                    .resource::<Session>()
                    .selected
                    .iter()
                    .flatten()
                    .count(),
                2
            );
            shot(world, "gametype");
            check.release = Some(click(world, "arena button"));
        }
        2 => {
            shot(world, "arena");
            check.release = Some(click(world, "robots button"));
        }
        3 => {
            shot(world, "robots");
            check.release = Some(click(world, "p1/change robot button"));
        }
        4 => {
            assert_eq!(world.resource::<Session>().picker, Some(1));
            match check.picker_step {
                0 => {
                    assert!(
                        world.resource::<legacy_ui::State>().choices["team dropdown"].len() > 1,
                        "Only one team loaded"
                    );
                    check.release = Some(click(world, "Down Button"));
                }
                1 => {
                    assert_eq!(
                        world.resource::<legacy_ui::State>().popup.as_deref(),
                        Some("team dropdown")
                    );
                    shot(world, "teams-dropdown");
                    let session = world.resource::<Session>();
                    let label = crate::content::robots::teams(&session.library)
                        .into_iter()
                        .find(|(e, _)| e.team_key == "AI/Team0")
                        .unwrap()
                        .1;
                    let index = world.resource::<legacy_ui::State>().choices["team dropdown"]
                        .iter()
                        .position(|s| s == &label)
                        .unwrap();
                    check.release = Some(click(world, &format!("choice:team dropdown:{index}")));
                }
                2 => {
                    assert_eq!(world.resource::<Session>().picker_team, "AI/Team0");
                    let cells = &world.resource::<legacy_ui::State>().cells["robots grid"];
                    let key = cells[cells.len().saturating_sub(1).min(1)].key.clone();
                    check.chosen_bot = key.strip_prefix("robot:").unwrap().parse().ok();
                    check.release = Some(click(world, &key));
                }
                3 => {
                    assert_eq!(world.resource::<Session>().picker_bot, check.chosen_bot);
                    shot(world, "select-robot");
                    check.release = Some(click(world, "ok button"));
                }
                _ => unreachable!(),
            }
            check.picker_step += 1;
            if check.picker_step < 4 {
                return;
            }
        }
        5 => check.release = Some(click(world, "start game button")),
        6 => {
            if matches!(world.resource::<Session>().phase, Phase::Introduction(_)) {
                let s = world.resource::<Session>();
                let position = world
                    .get::<Transform>(s.robots[0].bodies[0])
                    .unwrap()
                    .translation;
                if check.intro_step == 0 {
                    assert!(s.presentation.shots.len() >= 5);
                    assert_eq!(s.presentation.cues.len(), 4);
                    check.position = position;
                    check.intro_step = 1;
                    return;
                }
                assert!(
                    position.abs_diff_eq(check.position, 1e-5),
                    "Robot moved before countdown"
                );
                assert!(s.robots.iter().all(|r| r.damage.health.received == 0.));
                assert!(
                    !world
                        .get::<RapierConfiguration>(s.context)
                        .unwrap()
                        .physics_pipeline_active
                );
                shot(world, "introduction");
                check.release = Some(click(world, "skip button"));
                info!("PASS: native introduction shots/voices, frozen physics and Skip");
                return;
            }
            let session = world.resource::<Session>();
            assert_eq!(
                world.resource::<Navigation>().page,
                Page::Match,
                "Arena load failed: {:?}",
                session.error
            );
            assert_eq!(session.robots.len(), 2);
            assert_eq!(session.hazards_runtime.len(), 3);
            assert!(
                session
                    .robots
                    .iter()
                    .all(|r| r.document.is_some() && r.damage.shell.is_some())
            );
            assert!(
                world
                    .resource::<legacy_ui::Output>()
                    .regions
                    .contains_key("countdown group"),
                "Native countdown is hidden"
            );
            shot(world, "countdown");
        }
        7 => {
            if !matches!(world.resource::<Session>().phase, Phase::Playing) {
                return;
            }
            let body = world.resource::<Session>().robots[0].bodies[0];
            check.position = world.get::<Transform>(body).unwrap().translation;
            info!("Exhibition check player start {:?}", check.position);
            world
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::KeyW);
        }
        8 => {
            use crate::gameplay::camera::Mode as CameraMode;
            let mode = world.resource::<Session>().follow.mode;
            match check.camera_step {
                0 => {
                    assert_eq!(mode, CameraMode::Automatic);
                    camera_key(world, ButtonState::Pressed);
                    check.camera_step = 1;
                    return;
                }
                1 => {
                    assert_eq!(mode, CameraMode::Player);
                    assert_eq!(
                        world
                            .resource::<legacy_ui::State>()
                            .values
                            .get("camera textbox")
                            .map(String::as_str),
                        Some("Player Camera")
                    );
                    camera_key(world, ButtonState::Released);
                    camera_key(world, ButtonState::Pressed);
                    check.camera_step = 2;
                    return;
                }
                2 => {
                    assert_eq!(mode, CameraMode::Automatic);
                    assert_eq!(
                        world
                            .resource::<legacy_ui::State>()
                            .values
                            .get("camera textbox")
                            .map(String::as_str),
                        Some("Automatic Camera")
                    );
                    camera_key(world, ButtonState::Released);
                    check.camera_step = 3;
                    info!("PASS: F3 cycles Automatic / Player and updates the native HUD");
                }
                _ => {}
            }
            if check.frames == 0 {
                // Allow native drive input before moving assemblies into the collision exercise.
                assert!(
                    world
                        .resource::<legacy_ui::Output>()
                        .regions
                        .contains_key("time group"),
                    "Native timer is hidden"
                );
                shot(world, "battle");
                check.frames = 12;
                check.stage += 1;
                return;
            }
        }
        9 => {
            let body = world.resource::<Session>().robots[0].bodies[0];
            let position = world.get::<Transform>(body).unwrap().translation;
            info!("Exhibition check player after drive {position:?}");
            assert!(
                position.distance(check.position) > 0.1,
                "Native controls did not move the player: {position:?} / {:?}",
                check.position
            );
            world
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(KeyCode::KeyW);
            // Arrange a collision between native robots; do not inject HP or points.
            let assemblies = world
                .resource::<Session>()
                .robots
                .iter()
                .map(|r| r.bodies.clone())
                .collect::<Vec<_>>();
            for (i, bodies) in assemblies.iter().enumerate() {
                let origin = world.get::<Transform>(bodies[0]).unwrap().translation;
                let destination = Vec3::new(if i == 0 { -2. } else { 1.5 }, origin.y, 0.);
                for &body in bodies {
                    world.get_mut::<Transform>(body).unwrap().translation += destination - origin;
                    *world.get_mut::<Velocity>(body).unwrap() = Velocity {
                        linear: Vec3::X * if i == 0 { 16. } else { 0. },
                        angular: Vec3::ZERO,
                    };
                }
            }
            check.received = world.resource::<Session>().robots[1].damage.health.received;
        }
        10..=14 => {}
        15 => {
            let session = world.resource::<Session>();
            for r in &session.robots {
                info!(
                    "Collision check robot {}: pose {:?}, velocity {:?}, HP {}, score {:?}",
                    r.id,
                    world.get::<Transform>(r.bodies[0]),
                    world.get::<Velocity>(r.bodies[0]),
                    r.damage.health.received,
                    session.score
                );
            }
            assert!(
                session.robots[1].damage.health.received > check.received,
                "No damage between two native articulated robots"
            );
            assert!(
                session.score[0] > 0.,
                "Opponent damage was not attributed to player"
            );
            assert!(
                session
                    .robots
                    .iter()
                    .any(|r| r.damage.shell.as_ref().unwrap().revision > 0),
                "Collision did not dent the chassis"
            );
            assert!(
                session.score[1] < session.score[0] * 0.25,
                "AFK recoil mirrored attacker points: {:?}",
                session.score
            );
            let numbers = &world.resource::<legacy_ui::State>().floating_text;
            assert!(
                numbers.iter().any(|n| n.color[1] > n.color[0]),
                "No green player-inflicted damage"
            );
            assert!(
                numbers.iter().any(|n| n.color[0] > n.color[1]),
                "No red received damage"
            );
            assert!(
                world
                    .query_filtered::<&Text, With<crate::legacy_ui::floating::DamageLabel>>()
                    .iter(world)
                    .count()
                    > 0,
                "GPU damage text pool missing"
            );
            shot(world, "damage");
            // Exercise detachment and KO through the same scripted damage queue as hazards.
            let mut session = world.resource_mut::<Session>();
            let part = session.robots[1]
                .document
                .as_ref()
                .unwrap()
                .components
                .iter()
                .position(|c| c.class == "Wheel")
                .unwrap();
            session.robots[1]
                .damage
                .pending
                .push((part, 100000., Vec3::ZERO));
        }
        16 => {
            assert!(
                !world.resource::<Session>().robots[1]
                    .damage
                    .fragments
                    .is_empty(),
                "Broken wheel did not detach"
            );
            world.resource_mut::<Session>().robots[1]
                .damage
                .pending
                .push((0, 100000., Vec3::ZERO));
        }
        17 => {
            assert_eq!(world.resource::<Session>().phase, Phase::Finished(Some(0)));
            assert_eq!(world.resource::<Navigation>().page, Page::Results);
            shot(world, "results");
            check.release = Some(click(world, "continue button"));
        }
        18 => {
            assert_eq!(world.resource::<Navigation>().page, Page::Exhibition);
            assert!(world.resource::<Session>().robots.is_empty());
            assert_eq!(
                world
                    .query::<&crate::gameplay::RobotOwner>()
                    .iter(world)
                    .count(),
                0,
                "Robot or detached debris leaked on scene exit"
            );
            check.release = Some(click(world, "start game button"));
        }
        19 => {
            assert_eq!(world.resource::<Session>().robots.len(), 2);
            world.resource_mut::<Navigation>().page = Page::Pause;
        }
        20 => {
            let mut nav = world.resource_mut::<Navigation>();
            nav.back = Page::Pause;
            nav.page = Page::Settings;
        }
        21 => {
            assert_eq!(
                world.resource::<Session>().robots.len(),
                2,
                "Settings from pause discarded the match"
            );
            assert!(world.resource::<Time<Virtual>>().is_paused());
            world.resource_mut::<Navigation>().page = Page::Pause;
        }
        22 => {
            world.resource_mut::<Navigation>().page = Page::Match;
            world.resource_mut::<Session>().phase = Phase::Playing;
            watch_hazard(world, &mut check, arena::Kind::Spikes);
        }
        23..=27 => {}
        28 => {
            assert!(
                check.rise > 0.1,
                "Native spike zone did not fire its slider: {}",
                check.rise
            );
            watch_hazard(world, &mut check, arena::Kind::Saw);
        }
        29..=32 => {}
        33 => {
            assert!(
                check.rise > 0.1,
                "Native saw zone did not raise the saw: {}",
                check.rise
            );
            shot(world, "hazards");
            check.watch = None;
            world.resource_mut::<Navigation>().page = Page::Main;
        }
        34 => {
            assert!(world.resource::<Session>().robots.is_empty());
            assert_eq!(
                world
                    .query::<&crate::gameplay::RobotOwner>()
                    .iter(world)
                    .count(),
                0
            );
            info!(
                "PASS: native Exhibition UI/countdown/timer/controls, two-robot HP/points/dents, detachment, KO/results, replay, pause/options, native spikes/saw and full cleanup"
            );
            if std::env::var_os("RA2_CHECK_CONTACTS_ONLY").is_some() {
                world.write_message(AppExit::Success);
            }
            world.resource_mut::<Navigation>().page = Page::Exhibition;
        }
        _ => unreachable!(),
    }
    check.stage += 1;
}

fn watch_hazard(world: &mut World, check: &mut Check, kind: arena::Kind) {
    let session = world.resource::<Session>();
    let trap = session
        .hazards_runtime
        .iter()
        .find(|t| t.kind == kind)
        .unwrap();
    let destination = (trap.min + trap.max) * 0.5;
    let entity = trap.entity;
    let bodies = session.robots[0].bodies.clone();
    check.watch = Some((
        entity,
        world.get::<Transform>(entity).unwrap().translation.y,
    ));
    check.rise = 0.;
    let origin = world.get::<Transform>(bodies[0]).unwrap().translation;
    let shift = Vec3::new(destination.x - origin.x, 0., destination.z - origin.z);
    for body in bodies {
        world.get_mut::<Transform>(body).unwrap().translation += shift;
        *world.get_mut::<Velocity>(body).unwrap() = Velocity::zero();
    }
}

/// Cover real native controls and each mode's runtime objective, beyond pure rule tests.
fn modes(world: &mut World, check: &mut Check) {
    let cases = [
        (Mode::BattleRoyal, "box", "battle royal checkbox"),
        (Mode::Teams, "box", "teams checkbox"),
        (Mode::TableTop, "clawtop", "tabletop checkbox"),
        (Mode::KingOfHill, "king", "kingofhill checkbox"),
        (Mode::TableTop, "flextop", "tabletop checkbox"),
        (Mode::KingOfHill, "hilltop", "kingofhill checkbox"),
        (Mode::Deathmatch, "bridge", "deathmatch checkbox"),
        (Mode::Deathmatch, "compressor", "deathmatch checkbox"),
        (Mode::Deathmatch, "electric", "deathmatch checkbox"),
        (Mode::Deathmatch, "half", "deathmatch checkbox"),
        (Mode::Deathmatch, "skull", "deathmatch checkbox"),
        (Mode::Deathmatch, "parkinglot", "deathmatch checkbox"),
        (Mode::Deathmatch, "octagon", "deathmatch checkbox"),
    ];
    let end = std::env::var("RA2_CHECK_END")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(cases.len())
        .min(cases.len());
    if check.mode_case >= end {
        info!(
            "PASS: five native mode controls, arena filtering, four-player HUD, team victory, tabletop ringout, hill points and final cleanup"
        );
        world.write_message(AppExit::Success);
        return;
    }
    let (mode, key, button) = cases[check.mode_case];
    match check.mode_step {
        0 => {
            world.resource_mut::<Session>().tab = 0;
            world.resource_mut::<Session>().revision += 1;
        }
        1 => {
            check.release = Some(click(world, button));
        }
        2 => {
            let mut s = world.resource_mut::<Session>();
            assert_eq!(s.mode, mode);
            assert!(s.arenas[s.arena].modes.contains(&mode));
            s.tab = 1;
            s.revision += 1;
        }
        3 => {
            let i = world
                .resource::<Session>()
                .arenas
                .iter()
                .position(|a| a.key == key)
                .unwrap();
            check.release = Some(click(world, &format!("arena:{i}")));
        }
        4 => {
            assert_eq!(
                world.resource::<Session>().arenas[world.resource::<Session>().arena].key,
                key
            );
            shot(world, &format!("mode-{}-arena", check.mode_case));
            let mut s = world.resource_mut::<Session>();
            let first = s.selected[0];
            // Use four real robots in the shared runtime, with stable public slot IDs.
            s.selected = [first; 4];
            s.tab = 2;
            s.revision += 1;
        }
        5 => {
            let card = format!("p{}/change robot button", mode.slots() - 1);
            assert!(
                world
                    .resource::<legacy_ui::Output>()
                    .hits
                    .iter()
                    .any(|h| h.name == card),
                "Fourth native robot card missing"
            );
            shot(world, &format!("mode-{}-robots", check.mode_case));
            check.release = Some(click(world, "start game button"));
        }
        6 => {
            if key == "flextop" {
                let s = world.resource::<Session>();
                if check.mode_wait == 0 {
                    assert!(matches!(s.phase, Phase::Introduction(_)));
                    check.frozen = s
                        .robots
                        .iter()
                        .flat_map(|r| r.bodies.iter().copied())
                        .chain(s.arena_state.springs.iter().map(|p| p.entity))
                        .map(|e| (e, *world.get::<Transform>(e).unwrap()))
                        .collect();
                    check.mode_wait = 1;
                    return;
                }
                if !matches!(s.phase, Phase::Playing) {
                    for (e, pose) in &check.frozen {
                        assert_eq!(
                            world.get::<Transform>(*e).unwrap(),
                            pose,
                            "FlexTop moved before GO"
                        );
                    }
                    assert!(
                        !world
                            .get::<RapierConfiguration>(s.context)
                            .unwrap()
                            .physics_pipeline_active
                    );
                    assert_eq!(s.eliminated, [false; 4]);
                    assert!(s.robots.iter().all(|r| r.damage.health.received == 0.));
                    if matches!(s.phase, Phase::Introduction(_)) {
                        shot(world, "flextop-introduction");
                        world.resource_mut::<Session>().begin_countdown();
                    }
                    return;
                }
                assert_eq!(s.eliminated, [false; 4]);
                info!(
                    "PASS: native FlexTop and all four robots stay frozen throughout introduction/countdown; no pre-match elimination"
                );
                check.frozen.clear();
            }
            let mut s = world.resource_mut::<Session>();
            assert_eq!(s.robots.len(), mode.slots(), "{:?}", s.error);
            s.phase = Phase::Playing;
            check.mode_wait = 0;
        }
        7 => {
            check.mode_wait += 1;
            if check.mode_wait < 5 {
                return;
            }
            let s = world.resource::<Session>();
            assert_eq!(world.resource::<Navigation>().page, Page::Match);
            for r in &s.robots {
                let p = world.get::<Transform>(r.bodies[0]).unwrap();
                assert!(p.translation.is_finite() && p.translation.y > s.arena_state.ringout_y);
            }
            for i in 1..=mode.slots() {
                assert!(
                    !world
                        .resource::<legacy_ui::State>()
                        .visibility
                        .get(&format!("player{i} status"))
                        .is_some_and(|v| !*v)
                );
            }
            info!(
                "Arena view {}: player {:?}, camera {:?}",
                key,
                world.get::<Transform>(s.robots[0].bodies[0]),
                world.get::<Transform>(s.camera)
            );
            shot(world, &format!("mode-{}-battle", check.mode_case));
            check.mode_wait = 0;
        }
        8 => {
            let s = world.resource::<Session>();
            let destination = s.arena_state.king.as_ref().map(|z| (z.min + z.max) * 0.5);
            let y = s.arena_state.ringout_y - 2.;
            if mode == Mode::KingOfHill {
                move_robot(world, 0, destination.unwrap());
            } else if mode == Mode::TableTop {
                for slot in 1..4 {
                    move_robot(world, slot, Vec3::new(slot as f32, y, 0.));
                }
            } else {
                let mut s = world.resource_mut::<Session>();
                let start = if mode == Mode::Teams { 2 } else { 1 };
                for r in s.robots.iter_mut().skip(start) {
                    r.damage.pending.push((0, 100000., Vec3::ZERO));
                }
            }
        }
        9 => {
            check.mode_wait += 1;
            if mode == Mode::KingOfHill {
                if check.mode_wait < 10 {
                    return;
                }
                let mut s = world.resource_mut::<Session>();
                assert!(
                    s.score[0] >= 10.,
                    "Hill did not award exclusive occupancy: {:?}",
                    s.score
                );
                assert_eq!(&s.score[1..], &[0.; 3]);
                s.remaining = 0.;
            }
        }
        10 => {
            let s = world.resource::<Session>();
            assert_eq!(
                world.resource::<Navigation>().page,
                Page::Results,
                "{mode:?} objective did not finish match"
            );
            assert_eq!(
                s.winners,
                if mode == Mode::Teams {
                    [true, true, false, false]
                } else {
                    [true, false, false, false]
                }
            );
            shot(world, &format!("mode-{}-results", check.mode_case));
            world.resource_mut::<Navigation>().page = Page::Exhibition;
        }
        11 => {
            assert!(world.resource::<Session>().robots.is_empty());
            check.mode_case += 1;
            check.mode_step = 0;
            return;
        }
        _ => unreachable!(),
    }
    check.mode_step += 1;
}
fn move_robot(world: &mut World, slot: usize, destination: Vec3) {
    let bodies = world.resource::<Session>().robots[slot].bodies.clone();
    let origin = world.get::<Transform>(bodies[0]).unwrap().translation;
    for body in bodies {
        world.get_mut::<Transform>(body).unwrap().translation += destination - origin;
        *world.get_mut::<Velocity>(body).unwrap() = Velocity::zero();
    }
}
