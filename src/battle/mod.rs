//! Original Exhibition workflow. Rules and articulated robots live in reusable modules.
mod ai;
mod ai_controls;
mod ai_obstacles;
mod ai_tactics;
#[cfg(test)]
mod ai_tests;
pub(crate) mod arena;
mod audio;
mod camera;
mod hazards;
mod immobility;
mod immobility_marker;
pub(crate) mod ui;
use crate::content::{arenas::Arena, robots::Entry};
use crate::{
    frontend::{Navigation, Page},
    gameplay::{RobotBody, RobotInstance},
    legacy_ui,
};
use arena_core::match_rules::Mode;
use bevy::{camera::visibility::RenderLayers, core_pipeline::tonemapping::Tonemapping, prelude::*};
use bevy_rapier3d::prelude::*;
pub(crate) const LAYER: usize = 10;
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Phase {
    Setup,
    Introduction(f32),
    Countdown(f32),
    Playing,
    Finished(Option<usize>),
}
#[derive(Resource)]
pub(crate) struct Session {
    pub network: bool,
    pub authority: bool,
    pub local_slot: usize,
    pub net_waiting: bool,
    pub net_paused: bool,
    pub robots: Vec<RobotInstance>,
    pub library: Vec<Entry>,
    pub arenas: Vec<Arena>,
    pub arena: usize,
    pub mode: Mode,
    pub arena_state: arena::State,
    pub eliminated: [bool; 4],
    pub(crate) immobility: [immobility::Immobility; 4],
    pub immobility_countdown: [u8; 4],
    pub counted_out: [bool; 4],
    pub winners: [bool; 4],
    pub selected: [Option<usize>; 4],
    pub minutes: u32,
    pub weight: Option<u32>,
    pub hazards: bool,
    pub tab: usize,
    pub picker: Option<usize>,
    pub picker_team: String,
    pub picker_bot: Option<usize>,
    pub phase: Phase,
    pub remaining: f32,
    pub context: Entity,
    pub camera: Entity,
    pub environment: Vec<Entity>,
    pub hazards_runtime: Vec<arena::Trap>,
    pub error: Option<String>,
    pub start: bool,
    pub revision: u64,
    pub score: [f32; 4],
    pub follow: crate::gameplay::camera::Rig,
    pub presentation: crate::content::introduction::Timeline,
    pub intro_cue: usize,
    pub announcement: u64,
    pub sound_events: Vec<(String, Vec3)>,
    pub impacts: Vec<(u32, usize, f32, Vec3)>,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            network: false,
            authority: true,
            local_slot: 0,
            net_waiting: false,
            net_paused: false,
            robots: vec![],
            library: vec![],
            arenas: vec![],
            arena: 0,
            mode: Mode::Deathmatch,
            arena_state: default(),
            eliminated: [false; 4],
            immobility: default(),
            immobility_countdown: [0; 4],
            counted_out: [false; 4],
            winners: [false; 4],
            selected: [None; 4],
            minutes: 5,
            weight: None,
            hazards: true,
            tab: 0,
            picker: None,
            picker_team: String::new(),
            picker_bot: None,
            phase: Phase::Setup,
            remaining: 300.,
            context: Entity::PLACEHOLDER,
            camera: Entity::PLACEHOLDER,
            environment: vec![],
            hazards_runtime: vec![],
            error: None,
            start: false,
            revision: 0,
            score: [0.; 4],
            follow: default(),
            presentation: default(),
            intro_cue: 0,
            announcement: 0,
            sound_events: vec![],
            impacts: vec![],
        }
    }
}
impl Session {
    fn announce(&mut self, prefix: &str) {
        let options: &[&str] = match prefix {
            "Ready" => &[
                "Ready_BotsAreReady",
                "Ready_AllOpponentsReady",
                "Ready_ReadyToGetUnderway",
                "Ready_LetDestructionBegin",
            ],
            "Begin" => &[
                "Begin_AndWereOff",
                "Begin_HereWeGo",
                "Begin_MayStrongestWin",
                "Begin_BuzzerSignalsStart",
                "Begin_KickMatchInGear",
                "Begin_LetsBegin",
            ],
            _ => &["End_MatchIsOver", "End_MatchIsDone", "End_GreatMatch"],
        };
        self.announcement = self
            .announcement
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1);
        let name = options[(self.announcement >> 32) as usize % options.len()];
        self.sound_events
            .push((format!("Sounds/announcers/{name}.wav"), Vec3::ZERO));
    }
    pub(crate) fn begin_countdown(&mut self) {
        self.phase = Phase::Countdown(3.);
        self.sound_events.clear();
        self.announce("Ready");
        self.sound_events
            .push(("Sounds/match_countdown.wav".into(), Vec3::ZERO));
        self.revision += 1;
    }
    pub fn choose_mode(&mut self, mode: Mode) {
        self.mode = mode;
        if self
            .arenas
            .get(self.arena)
            .is_none_or(|a| !a.modes.contains(&mode))
        {
            self.arena = self
                .arenas
                .iter()
                .position(|a| a.modes.contains(&mode))
                .unwrap_or(0);
        }
        for slot in mode.slots()..4 {
            self.selected[slot] = None;
        }
        for slot in 0..mode.minimum() {
            if self.selected[slot].is_none() && !self.library.is_empty() {
                self.selected[slot] = Some(slot.min(self.library.len() - 1));
            }
        }
    }
    pub fn ready(&self) -> bool {
        self.selected[0].is_some()
            && self.selected[..self.mode.slots()].iter().flatten().count() >= self.mode.minimum()
            && self
                .arenas
                .get(self.arena)
                .is_some_and(|a| a.modes.contains(&self.mode))
    }
}
#[derive(Component)]
pub(crate) struct MatchContext;
#[derive(Component)]
pub(crate) struct MatchCamera;
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Presentation;
pub(crate) struct BattlePlugin;
impl Plugin for BattlePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Session>()
            .add_systems(Startup, setup.after(crate::editor::setup))
            .add_systems(
                Update,
                (ui::input, lifecycle, ui::bind)
                    .chain()
                    .after(legacy_ui::Input)
                    .before(legacy_ui::Paint),
            )
            .add_systems(
                FixedUpdate,
                ai::update.before(crate::gameplay::drive::motors),
            )
            .add_systems(
                FixedUpdate,
                (arena::tick, step)
                    .chain()
                    .after(PhysicsSet::Writeback)
                    .after(crate::gameplay::damage::contacts),
            )
            .add_systems(
                Startup,
                immobility_marker::setup.after(legacy_ui::assets::load),
            )
            .add_systems(Update, immobility_marker::update.after(Presentation))
            .add_systems(Update, hazards::animate.after(lifecycle))
            .add_systems(
                Update,
                camera::update
                    .in_set(Presentation)
                    .after(lifecycle)
                    .before(legacy_ui::Paint),
            )
            .add_plugins(audio::MatchAudioPlugin);
    }
}
pub(crate) fn setup(
    mut commands: Commands,
    mut session: ResMut<Session>,
    options: Res<crate::Options>,
    catalog: Res<crate::editor::catalog::Catalog>,
    mut assets: ResMut<legacy_ui::assets::LegacyAssets>,
) {
    session.context = commands
        .spawn((
            MatchContext,
            RapierContextSimulation::default(),
            RapierConfiguration {
                physics_pipeline_active: false,
                ..RapierConfiguration::new(1.)
            },
        ))
        .id();
    session.camera = commands
        .spawn((
            MatchCamera,
            Camera3d::default(),
            Camera {
                is_active: false,
                clear_color: ClearColorConfig::Custom(Color::BLACK),
                ..default()
            },
            RenderLayers::layer(LAYER),
            Tonemapping::None,
            bevy::core_pipeline::tonemapping::DebandDither::Disabled,
            Msaa::Sample4,
            Transform::default(),
        ))
        .id();
    commands.spawn((
        DirectionalLight {
            illuminance: 7000.,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -1.0, -0.4, 0.)),
        RenderLayers::layer(LAYER),
    ));
    refresh_library(&mut session, &options, &catalog, &mut assets);
    match crate::content::arenas::load(&options.game_dir) {
        Ok(arenas) => {
            for a in &arenas {
                let key = ra2_formats::components::normalize(&a.preview);
                let path = catalog
                    .index
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| options.game_dir.join(&a.preview));
                if let Ok(bytes) = std::fs::read(path)
                    && let Ok(image) = legacy_ui::assets::decode_image(&bytes, "bmp")
                {
                    assets.images.insert(key, image);
                }
            }
            info!("Exhibition: {} original arenas, five modes", arenas.len());
            session.arenas = arenas;
        }
        Err(error) => session.error = Some(error),
    }
}
pub(crate) fn refresh_library(
    session: &mut Session,
    options: &crate::Options,
    catalog: &crate::editor::catalog::Catalog,
    assets: &mut legacy_ui::assets::LegacyAssets,
) {
    let selected = session.selected.map(|i| {
        i.and_then(|n| session.library.get(n))
            .map(|e| e.path.clone())
    });
    match crate::content::robots::load(&options.game_dir) {
        Ok(library) => {
            for (i, entry) in library.iter().enumerate() {
                if let Some(bytes) = &entry.robot.snapshot
                    && let Ok(image) = legacy_ui::assets::decode_image(bytes, "tga")
                {
                    assets
                        .images
                        .insert(format!("exhibition/bot-{i}.tga"), image);
                }
                if let Ok(bytes) = std::fs::read(
                    catalog
                        .index
                        .get(&ra2_formats::components::normalize(&entry.logo))
                        .cloned()
                        .unwrap_or_else(|| options.game_dir.join(&entry.logo)),
                ) && let Ok(image) = legacy_ui::assets::decode_image(&bytes, "bmp")
                {
                    assets
                        .images
                        .insert(ra2_formats::components::normalize(&entry.logo), image);
                }
            }
            session.selected = std::array::from_fn(|slot| {
                selected[slot]
                    .as_ref()
                    .and_then(|p| library.iter().position(|e| &e.path == p))
                    .or_else(|| {
                        (slot < session.mode.minimum() && !library.is_empty())
                            .then_some(slot.min(library.len().saturating_sub(1)))
                    })
            });
            session.library = library;
        }
        Err(e) => session.error = Some(e),
    }
}
pub(crate) fn refresh(
    mut s: ResMut<Session>,
    catalog: Res<crate::editor::catalog::Catalog>,
    options: Res<crate::Options>,
    mut assets: ResMut<legacy_ui::assets::LegacyAssets>,
    nav: Res<Navigation>,
    mut previous: Local<Option<Page>>,
) {
    if nav.page == Page::Exhibition && *previous != Some(Page::Exhibition) {
        s.network = false;
        s.authority = true;
        s.local_slot = 0;
        s.net_waiting = false;
        s.net_paused = false;
        refresh_library(&mut s, &options, &catalog, &mut assets);
        s.revision += 1;
    }
    *previous = Some(nav.page);
}
pub(crate) fn clear(session: &mut Session, commands: &mut Commands) {
    for r in &mut session.robots {
        r.clear(commands);
    }
    session.robots.clear();
    for e in session.environment.drain(..) {
        commands.entity(e).despawn();
    }
    session.hazards_runtime.clear();
    session.arena_state = default();
    session.eliminated = [false; 4];
    session.immobility = default();
    session.immobility_countdown = [0; 4];
    session.counted_out = [false; 4];
    session.winners = [false; 4];
    session.impacts.clear();
    session.sound_events.clear();
    session.phase = Phase::Setup;
    session.presentation = default();
    session.intro_cue = 0;
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn lifecycle(
    mut commands: Commands,
    mut s: ResMut<Session>,
    mut nav: ResMut<Navigation>,
    catalog: Res<crate::editor::catalog::Catalog>,
    config: Res<crate::config::Config>,
    keys: Res<ButtonInput<KeyCode>>,
    mut cameras: Query<(&mut Camera, &mut Transform), With<MatchCamera>>,
    poses: Query<&Transform, (With<RobotBody>, Without<MatchCamera>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let visible = matches!(nav.page, Page::Match | Page::Results | Page::Pause)
        || (nav.page == Page::Settings && matches!(nav.back, Page::Match | Page::Pause));
    if let Ok((mut camera, _)) = cameras.get_mut(s.camera) {
        camera.is_active = visible;
    }
    if !visible && !s.robots.is_empty() {
        clear(&mut s, &mut commands);
    }
    if s.start {
        s.start = false;
        clear(&mut s, &mut commands);
        let result = (|| -> Result<(), String> {
            if !s.ready() {
                return Err(format!(
                    "Select {}–{} robots for {}",
                    s.mode.minimum(),
                    s.mode.slots(),
                    s.mode.tag()
                ));
            }
            let entries = s
                .selected
                .iter()
                .take(s.mode.slots())
                .enumerate()
                .filter_map(|(slot, i)| {
                    i.and_then(|i| s.library.get(i)).cloned().map(|e| (slot, e))
                })
                .collect::<Vec<_>>();
            if let Some(weight) = s.weight
                && entries.iter().any(|(_, e)| e.robot.weight_class != weight)
            {
                return Err("Selected robot does not match the chosen weight class".into());
            }
            let loaded = arena::spawn(
                &mut commands,
                s.context,
                s.hazards,
                &s.arenas[s.arena],
                &catalog,
                &config.garage,
                &mut meshes,
                &mut materials,
                &mut images,
            )?;
            s.environment = loaded.entities;
            s.hazards_runtime = loaded.traps;
            s.arena_state = loaded.state;
            for (i, entry) in entries {
                let mut robot = RobotInstance::new(i as u32 + 1, s.context, config.garage.clone());
                robot.layer = LAYER;
                robot.group = Group::from_bits_retain(1 << (i + 1));
                let start = if s.mode == Mode::Teams {
                    [0, 2, 1, 3][i]
                } else {
                    i
                };
                robot.placement = Some(loaded.starts[start]);
                let spawned = crate::gameplay::spawn::robot(
                    &mut commands,
                    &mut robot,
                    &entry.robot,
                    &catalog,
                    &mut meshes,
                    &mut materials,
                    &mut images,
                );
                if let Err(error) = spawned {
                    robot.clear(&mut commands);
                    return Err(error);
                }
                robot.active = true;
                s.robots.push(robot);
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                s.follow = default();
                s.remaining = s.minutes as f32 * 60.;
                s.announcement = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64;
                let players =
                    std::array::from_fn(|slot| s.robots.iter().any(|r| r.id as usize == slot + 1));
                s.presentation =
                    s.arenas[s.arena]
                        .presentation
                        .timeline(s.hazards, players, s.announcement);
                s.intro_cue = 0;
                if s.network {
                    s.phase = Phase::Setup;
                } else if s.presentation.shots.is_empty() {
                    s.begin_countdown();
                } else {
                    s.phase = Phase::Introduction(0.);
                }
                s.score = [0.; 4];
                s.error = None;
                nav.page = Page::Match;
            }
            Err(e) => {
                clear(&mut s, &mut commands);
                s.error = Some(e);
                nav.page = if s.network {
                    Page::Multiplayer
                } else {
                    Page::Exhibition
                };
            }
        }
        s.revision += 1;
    }
    let playing = (nav.page == Page::Match || s.network)
        && matches!(s.phase, Phase::Playing)
        && !s.net_waiting
        && !s.net_paused;
    if nav.page == Page::Match
        && !s.network
        && matches!(s.phase, Phase::Introduction(_))
        && (keys.just_pressed(KeyCode::Space) || keys.just_pressed(KeyCode::Enter))
    {
        s.begin_countdown();
    }
    let running = playing;
    let eliminated = s.eliminated;
    for r in &mut s.robots {
        r.running = running
            && !eliminated[r.id as usize - 1]
            && r.bodies
                .first()
                .and_then(|e| poses.get(*e).ok())
                .is_some_and(|p| !defeated(r, p.translation));
    }
    if !s.network
        && let Some(player) = s.robots.first_mut()
        && player.running
        && playing
        && let Some(robot) = &player.document
    {
        let input = crate::editor::controls::samples(robot, &keys, true);
        let mut next = player.controls.evaluate(robot, &input);
        for (n, old) in next.iter_mut().zip(&player.commands) {
            n.fire |= old.fire;
        }
        player.commands = next;
    }
}

pub(crate) fn defeated(robot: &RobotInstance, _position: Vec3) -> bool {
    robot.damage.health.parts.first().is_none_or(|p| !p.alive())
        || robot.document.as_ref().is_none_or(|d| {
            !d.components
                .iter()
                .enumerate()
                .any(|(i, c)| c.class == "ControlBoard" && robot.damage.health.operational(i))
        })
}
pub(crate) fn step(
    mut s: ResMut<Session>,
    mut nav: ResMut<Navigation>,
    poses: Query<&Transform, With<RobotBody>>,
    owners: Query<&crate::gameplay::RobotOwner>,
) {
    if (nav.page != Page::Match && !s.network)
        || (s.network && (!s.authority || s.net_waiting || s.net_paused))
    {
        return;
    }
    match s.phase.clone() {
        Phase::Introduction(age) => {
            let age = age + crate::STEP as f32;
            while let Some((at, path)) = s.presentation.cues.get(s.intro_cue).cloned() {
                if at > age {
                    break;
                }
                s.sound_events.push((path, Vec3::ZERO));
                s.intro_cue += 1;
            }
            if age >= s.presentation.duration {
                s.begin_countdown();
            } else {
                s.phase = Phase::Introduction(age);
            }
        }
        Phase::Countdown(t) => {
            let old = t;
            let t = (t - crate::STEP as f32).max(0.);
            if t > 0. && t.ceil() < old.ceil() {
                s.sound_events
                    .push(("Sounds/match_countdown.wav".into(), Vec3::ZERO));
            }
            s.phase = if t <= 0. {
                s.announce("Begin");
                Phase::Playing
            } else {
                Phase::Countdown(t)
            };
            s.revision += 1;
        }
        Phase::Playing => {
            s.remaining = (s.remaining - crate::STEP as f32).max(0.);
            let mut scored = Vec::new();
            let mut effects = Vec::new();
            for robot in &mut s.robots {
                for (other, part, amount, point) in std::mem::take(&mut robot.damage.impacts) {
                    if let Ok(owner) = owners.get(other)
                        && owner.0 != robot.id
                        && owner.0 > 0
                    {
                        scored.push((
                            owner.0 as usize - 1,
                            arena_core::match_rules::damage_points(amount),
                            robot.id as usize - 1,
                        ));
                    }
                    effects.push((robot.id, part, amount, point));
                }
            }
            if s.mode != Mode::KingOfHill {
                for (index, amount, victim) in scored {
                    if index < 4
                        && (s.mode != Mode::Teams || s.mode.side(index) != s.mode.side(victim))
                    {
                        s.score[index] += amount;
                    }
                }
            }
            s.impacts.extend(effects);
            let mut active = [false; 4];
            let mut eliminated = s.eliminated;
            let mode = s.mode;
            let mut countdown = [0; 4];
            let s = &mut *s;
            let ringout_y = s.arena_state.ringout_y;
            for r in &mut s.robots {
                let slot = r.id as usize - 1;
                active[slot] = true;
                let pose = r.bodies.first().and_then(|e| poses.get(*e).ok());
                let ringout = pose.is_none_or(|p| p.translation.y < ringout_y);
                let dead = pose.is_none_or(|p| defeated(r, p.translation));
                // Tabletop's objective is falling off; a disabled robot can still be pushed off.
                eliminated[slot] |= ringout || (mode != Mode::TableTop && dead);
                if mode != Mode::TableTop
                    && !eliminated[slot]
                    && let Some(p) = pose
                {
                    (countdown[slot], eliminated[slot]) =
                        s.immobility[slot].advance(p.translation.xz());
                    s.counted_out[slot] = eliminated[slot];
                }
                if eliminated[slot] || dead {
                    r.running = false;
                }
            }
            if countdown
                .iter()
                .zip(s.immobility_countdown)
                .any(|(&now, old)| now > 0 && now != old)
            {
                s.sound_events
                    .push(("Sounds/immobile_countdown.wav".into(), Vec3::ZERO));
            }
            s.immobility_countdown = countdown;
            s.eliminated = eliminated;
            if s.mode == Mode::KingOfHill {
                let occupants = s
                    .robots
                    .iter()
                    .filter(|r| !eliminated[r.id as usize - 1])
                    .filter_map(|r| {
                        r.bodies
                            .first()
                            .and_then(|e| poses.get(*e).ok())
                            .map(|p| (r.id as usize - 1, p.translation))
                    })
                    .filter(|(_, p)| s.arena_state.king.as_ref().is_some_and(|z| z.contains(*p)))
                    .map(|(i, _)| i)
                    .collect::<Vec<_>>();
                if occupants.len() == 1 {
                    s.arena_state.king_owner = Some(occupants[0]);
                    s.arena_state.hill_clock += crate::STEP as f32;
                    while s.arena_state.hill_clock > 0.5 {
                        s.arena_state.hill_clock -= 0.5;
                        s.score[occupants[0]] += 10.;
                        s.sound_events
                            .push(("Sounds/start_scoring.wav".into(), Vec3::ZERO));
                    }
                } else {
                    s.arena_state.king_owner = None;
                    s.arena_state.hill_clock = 0.;
                }
            }
            if let Some(winners) = arena_core::match_rules::resolve(
                s.mode,
                active,
                eliminated,
                s.score,
                s.remaining <= 0.,
            ) {
                s.winners = winners;
                s.phase = Phase::Finished(winners.iter().position(|w| *w));
                for r in &mut s.robots {
                    r.running = false;
                }
                for trap in &mut s.hazards_runtime {
                    trap.active = false;
                }
                nav.page = Page::Results;
                s.announce("End");
            }
            s.revision += 1;
        }
        _ => {}
    }
}
