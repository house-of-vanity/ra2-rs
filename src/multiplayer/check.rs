//! Offscreen native GUI + native robot physics exercise through two real packet endpoints.
//! VirtualWire replaces the OS socket only; all session messages/BOTs/frames are serialized.
use super::*;
use arena_net::{
    HOST, PeerId, SessionId,
    transport::{NetworkProfile, VirtualWire},
};
use arena_net_io::session::{Node, Stage};
use bevy::{
    camera::RenderTarget,
    input::{ButtonState, mouse::MouseButtonInput},
    time::TimeUpdateStrategy,
};
use std::sync::mpsc;
struct Harness {
    started: bool,
    host: Node,
    client: Node,
    wire: VirtualWire,
    commands: mpsc::Receiver<service::Command>,
    events: mpsc::SyncSender<service::Event>,
    frames: u64,
    stage: u32,
    wait: u32,
    release: Option<Entity>,
    latest: Option<Frame>,
    moved: Option<Vec3>,
    image: Handle<Image>,
    frozen: Option<Vec3>,
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
        .add_systems(Update, tick.before(super::poll).after(legacy_ui::Input))
        .add_systems(
            Update,
            inspect
                .after(legacy_ui::Paint)
                .after(crate::gameplay::numbers::project),
        );
    }
}
fn setup(world: &mut World) {
    {
        let mut prefs = world.resource_mut::<crate::settings::EngineSettings>();
        prefs.fullscreen = false;
        prefs.resolution = [800, 600];
    }
    for mut window in world.query::<&mut Window>().iter_mut(world) {
        window.resolution.set_physical_resolution(800, 600);
    }
    let index = world
        .resource::<battle::Session>()
        .library
        .iter()
        .position(|e| e.robot.name.to_ascii_uppercase().contains("C.R.U.S.H"))
        .unwrap_or(0);
    let robot = world.resource::<battle::Session>().library[index]
        .robot
        .clone();
    let bytes = ra2_formats::bot::encode(&robot).unwrap();
    let arena = world
        .resource::<battle::Session>()
        .arenas
        .iter()
        .find(|a| a.key == "box")
        .unwrap()
        .key
        .clone();
    let settings = Settings {
        name: "Native network check".into(),
        arena,
        mode: 0,
        minutes: 5,
        hazards: false,
        weight: None,
    };
    let mut host = Node::new(HOST, settings.clone()).unwrap();
    let mut client = Node::new(PeerId(1), settings.clone()).unwrap();
    host.add_peer(PeerId(1), SessionId(777), 0).unwrap();
    client.add_peer(HOST, SessionId(777), 0).unwrap();
    client
        .request(Request::Select(
            Player {
                slot: 1,
                alias: "Remote".into(),
                team: "Native".into(),
                bot_name: robot.name.clone(),
                digest: [0; 32],
                size: 0,
                ready: false,
            },
            bytes,
        ))
        .unwrap();
    let (tx, commands) = mpsc::sync_channel(256);
    let (events, rx) = mpsc::sync_channel(512);
    {
        let mut n = world.resource_mut::<Multiplayer>();
        n.service = Some(service::Service {
            tx,
            rx: std::sync::Mutex::new(rx),
        });
        n.settings = settings;
        n.selected = Some(index);
        n.manifest = Some(Manifest {
            protocol: arena_net::PROTOCOL,
            build: [0; 32],
            assets: [0; 32],
            physics: [0; 32],
        });
    }
    let image = world
        .resource_mut::<Assets<Image>>()
        .add(Image::new_target_texture(
            800,
            600,
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            None,
        ));
    let cameras=world.query_filtered::<Entity,Or<(With<battle::MatchCamera>,With<crate::frontend::MenuCamera>)>>().iter(world).collect::<Vec<_>>();
    for e in cameras {
        world
            .entity_mut(e)
            .insert(RenderTarget::Image(image.clone().into()));
    }
    let mut profile = NetworkProfile::ideal();
    profile.upstream_ms = 30;
    profile.downstream_ms = 30;
    profile.jitter_ms = 5;
    profile.loss_per_10000 = 100;
    world.insert_non_send(Harness {
        started: false,
        host,
        client,
        wire: VirtualWire::new(profile, 19),
        commands,
        events,
        frames: 0,
        stage: 0,
        wait: 0,
        release: None,
        latest: None,
        moved: None,
        image,
        frozen: None,
    });
    world.resource_mut::<Navigation>().page = Page::Multiplayer;
}
fn tick(world: &mut World) {
    let mut h = world.remove_non_send::<Harness>().unwrap();
    h.frames += 1;
    let now = h.frames * 1_000_000 / 60;
    for c in h.commands.try_iter() {
        match c {
            service::Command::Game(r) => {
                if h.stage >= 12 {
                    h.client.request(r).unwrap();
                } else {
                    h.host.request(r).unwrap();
                }
            }
            service::Command::Host(settings, _, _) => {
                h.started = true;
                h.host.request(Request::Settings(settings)).unwrap();
            }
            service::Command::Search => {}
            service::Command::Stop => {}
            _ => panic!("Unexpected harness command"),
        }
    }
    if !h.started {
        world.insert_non_send(h);
        return;
    }
    h.host.pump(now, &mut h.wire).unwrap();
    h.client.pump(now, &mut h.wire).unwrap();
    for n in h.host.notices.drain(..) {
        let _ = h.events.try_send(service::Event::Game(n));
    }
    for (slot, w) in std::mem::take(&mut h.host.inputs) {
        let _ = h.events.try_send(service::Event::Input(slot, w));
    }
    for n in h.client.notices.drain(..).collect::<Vec<_>>() {
        match n {
            Notice::Prepare(e) => h.client.request(Request::Loaded(e)).unwrap(),
            Notice::Frame(f) => {
                h.latest = Some(f.clone());
                let _ = h.events.try_send(service::Event::Game(Notice::Frame(f)));
            }
            _ => {}
        }
    }
    if h.client.stage == Stage::Lobby
        && h.client.players.len() == 2
        && h.client
            .players
            .values()
            .all(|p| p.size > 0 && h.client.bots.contains_key(&p.digest))
        && !h.client.players[&1].ready
    {
        h.client.request(Request::Ready(true)).unwrap();
    }
    if h.stage >= 5 && h.stage <= 7 && h.client.stage == Stage::Playing {
        let document = world
            .resource::<battle::Session>()
            .robots
            .get(1)
            .and_then(|r| r.document.as_ref());
        if let Some(doc) = document {
            let levels = doc
                .controllers
                .iter()
                .flat_map(|c| c.keys.iter())
                .map(|k| {
                    if k.get(k.len().saturating_sub(2)) == Some(&17) {
                        255
                    } else {
                        0
                    }
                })
                .collect::<Vec<_>>();
            let edges = levels.iter().map(|v| u32::from(*v > 0)).collect();
            let epoch = h.client.epoch;
            let sequence = h.frames;
            h.client
                .request(Request::Wiring(Wiring {
                    generation: 0,
                    redundant: vec![],
                    epoch,
                    sequence,
                    levels,
                    edges,
                }))
                .unwrap();
        }
    }
    world.insert_non_send(h);
}
fn click(world: &mut World, h: &mut Harness, name: &str) {
    let output = world.resource::<legacy_ui::Output>();
    let hit = output
        .hits
        .iter()
        .rev()
        .find(|v| v.name == name)
        .unwrap_or_else(|| panic!("Missing native multiplayer control {name}"));
    let rect = hit.rect.intersect(hit.clip);
    assert!(!rect.is_empty(), "Clipped control {name}");
    let point = output.origin + rect.center() * output.scale;
    let (e, mut w) = world
        .query::<(Entity, &mut Window)>()
        .single_mut(world)
        .unwrap();
    w.set_cursor_position(Some(point));
    world.write_message(MouseButtonInput {
        button: MouseButton::Left,
        state: ButtonState::Pressed,
        window: e,
    });
    h.release = Some(e);
}
fn shot(world: &mut World, h: &Harness, name: &str) {
    world
        .spawn(bevy::render::view::screenshot::Screenshot::image(
            h.image.clone(),
        ))
        .observe(bevy::render::view::screenshot::save_to_disk(format!(
            "logs/multiplayer-{name}.png"
        )));
}
fn inspect(world: &mut World) {
    let mut h = world.remove_non_send::<Harness>().unwrap();
    if let Some(window) = h.release.take() {
        world.write_message(MouseButtonInput {
            button: MouseButton::Left,
            state: ButtonState::Released,
            window,
        });
    }
    h.wait += 1;
    if h.wait < 16 {
        world.insert_non_send(h);
        return;
    }
    h.wait = 0;
    if let Some(e) = &world.resource::<Multiplayer>().error {
        panic!("Multiplayer UI error: {e}");
    }
    match h.stage {
        0 => {
            assert_eq!(world.resource::<legacy_ui::Output>().origin, Vec2::ZERO);
            assert_eq!(world.resource::<legacy_ui::Output>().scale, 1.);
            assert!(
                world
                    .resource::<legacy_ui::Output>()
                    .hits
                    .iter()
                    .any(|h| h.name == "IROH button" && !h.rect.intersect(h.clip).is_empty())
            );
            let ui = world.resource::<legacy_ui::State>();
            assert_eq!(ui.root.as_ref().unwrap().name(), "multiplayer win");
            assert!(
                ui.root
                    .as_ref()
                    .unwrap()
                    .find("Gamespy Logo [Image]")
                    .is_none()
            );
            shot(world, &h, "browse");
            click(world, &mut h, "IP connect button");
            h.stage = 1;
        }
        1 => {
            assert!(world.resource::<Multiplayer>().ip_dialog);
            shot(world, &h, "ip");
            click(world, &mut h, "cancel button");
            h.stage = 2;
        }
        2 => {
            click(world, &mut h, "Create Game button");
            h.stage = 3;
        }
        3 => {
            shot(world, &h, "create");
            click(world, &mut h, "start game button");
            h.stage = 4;
        }
        4 => {
            if world.resource::<Multiplayer>().players.len() < 2
                || !world
                    .resource::<Multiplayer>()
                    .bots
                    .contains_key(&h.client.players[&1].digest)
            {
                world.insert_non_send(h);
                return;
            }
            shot(world, &h, "lobby");
            click(world, &mut h, "p0/ready checkbox");
            h.stage = 5;
        }
        5 => {
            if !h.host.can_start() {
                world.insert_non_send(h);
                return;
            }
            click(world, &mut h, "begin match button");
            h.stage = 6;
        }
        6 => {
            if !matches!(
                world.resource::<battle::Session>().phase,
                battle::Phase::Playing
            ) {
                world.insert_non_send(h);
                return;
            }
            assert!(!world.resource::<battle::Session>().net_waiting);
            let e = world.resource::<battle::Session>().robots[1].bodies[0];
            h.moved = Some(world.get::<Transform>(e).unwrap().translation);
            h.stage = 7;
            h.wait = 0;
        }
        7 => {
            if h.frames % 120 > 16 {
                world.insert_non_send(h);
                return;
            }
            let s = world.resource::<battle::Session>();
            let e = s.robots[1].bodies[0];
            assert!(
                world
                    .get::<Transform>(e)
                    .unwrap()
                    .translation
                    .distance(h.moved.unwrap())
                    > 0.02,
                "Remote Wiring did not move the native robot"
            );
            assert!(
                h.latest
                    .as_ref()
                    .is_some_and(|f| f.phase == 2 && f.bodies.len() > 5)
            );
            shot(world, &h, "host-battle");
            world.resource_mut::<battle::Session>().robots[1]
                .damage
                .pending
                .push((0, 25., Vec3::ZERO));
            h.stage = 8;
        }
        8 => {
            let tick = world.resource::<Multiplayer>().tick;
            world.resource_mut::<Multiplayer>().pause(tick, true);
            h.stage = 9;
        }
        9 => {
            if !world.resource::<battle::Session>().net_paused {
                world.insert_non_send(h);
                return;
            }
            let e = world.resource::<battle::Session>().robots[1].bodies[0];
            h.frozen = Some(world.get::<Transform>(e).unwrap().translation);
            h.stage = 10;
        }
        10 => {
            let e = world.resource::<battle::Session>().robots[1].bodies[0];
            assert_eq!(
                world.get::<Transform>(e).unwrap().translation,
                h.frozen.unwrap()
            );
            let tick = world.resource::<Multiplayer>().tick;
            world.resource_mut::<Multiplayer>().pause(tick, false);
            h.stage = 11;
        }
        11 => {
            let frame = h.latest.clone().unwrap();
            if frame.paused || world.resource::<Multiplayer>().paused {
                world.insert_non_send(h);
                return;
            }
            assert!(frame.robots.iter().any(|r| {
                r.hp[0]
                    < world.resource::<battle::Session>().robots[r.slot as usize]
                        .damage
                        .health
                        .parts[0]
                        .maximum
            }));
            let at = world.resource::<Time<Real>>().elapsed_secs_f64();
            {
                let mut n = world.resource_mut::<Multiplayer>();
                n.host = false;
                n.local = 1;
                n.frames = VecDeque::from([(frame, at)]);
            }
            {
                let mut s = world.resource_mut::<battle::Session>();
                s.authority = false;
                s.local_slot = 1;
            }
            h.stage = 12;
        }
        12 => {
            assert!(world.resource::<Multiplayer>().last_frame_tick > 0);
            let stats = world.resource::<prediction::Stats>();
            assert!(stats.steps > 0, "Native client predictor never stepped");
            let e = world.resource::<battle::Session>().robots[1].bodies[0];
            h.moved = Some(world.get::<Transform>(e).unwrap().translation);
            world
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::KeyW);
            world.resource_mut::<Multiplayer>().diagnostics = true;
            shot(world, &h, "client-battle");
            h.stage = 13;
        }
        13 => {
            let stats = world.resource::<prediction::Stats>();
            assert!(
                stats.steps > 20 && stats.pending > 0,
                "Client did not predict unacknowledged inputs"
            );
            let e = world.resource::<battle::Session>().robots[1].bodies[0];
            assert!(
                world
                    .get::<Transform>(e)
                    .unwrap()
                    .translation
                    .distance(h.moved.unwrap())
                    > 0.005,
                "Local native controls waited for an authoritative snapshot"
            );
            let mut frame = h.latest.clone().unwrap();
            frame.tick += 1000;
            frame.input_generation += 1;
            frame.paused = true;
            let at = world.resource::<Time<Real>>().elapsed_secs_f64();
            world
                .resource_mut::<Multiplayer>()
                .frames
                .push_back((frame, at));
            world
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(KeyCode::KeyW);
            shot(world, &h, "client-prediction");
            h.stage = 14;
        }
        14 => {
            assert_eq!(
                world.resource::<prediction::Stats>().pending,
                0,
                "Pause kept old predicted fire/drive commands"
            );
            info!(
                "PASS: native multiplayer menus/IP dialog/lobby/Ready, BOT transfer, load barrier, native remote Wiring/physics, HP snapshots, host pause, client native prediction without new snapshots and pending-input reset"
            );
            h.stage = 15;
        }
        15 => {
            world.write_message(crate::shutdown::ExitRequest);
        }
        _ => {}
    }
    assert!(h.frames < 2400, "Native multiplayer check timed out");
    world.insert_non_send(h);
}
