//! Actual BOT geometry and production mechanics, without a window, renderer or OS sockets.
use super::*;
use crate::garage::Garage;
use arena_net::{codec::Message, native_input::CommandBuffer, transport::*};
use arena_net_io::LinkEndpoint;
pub(super) fn source(name: &str) -> (Blueprint, Frame) {
    let root = std::env::var_os("RA2_GAME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../RobotArena2"));
    let library = crate::content::robots::load(&root).unwrap();
    let entry = library
        .iter()
        .find(|e| e.robot.name.to_uppercase().contains(&name.to_uppercase()))
        .unwrap();
    let (mut app, _) = crate::garage::tests::native_garage(entry.path.to_str().unwrap());
    for _ in 0..180 {
        app.update();
    }
    app.world_mut().resource_mut::<Garage>().id = 1;
    let source =
        Blueprint::capture_robot(app.world(), &app.world().resource::<Garage>().instance).unwrap();
    let r = &app.world().resource::<Garage>().instance;
    let frame = Frame {
        epoch: 1,
        input_generation: 0,
        tick: 0,
        phase: 2,
        phase_time: 0.,
        paused: false,
        remaining: 300.,
        scores: [0.; 4],
        eliminated: [false; 4],
        winners: [false; 4],
        bodies: r
            .bodies
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let t = app.world().get::<Transform>(*e).unwrap();
                let v = app
                    .world()
                    .get::<bevy_rapier3d::prelude::Velocity>(*e)
                    .unwrap();
                Pose {
                    id: super::super::world::id(0, i, false),
                    position: t.translation.to_array(),
                    rotation: t.rotation.to_array(),
                    linear: v.linear.to_array(),
                    angular: v.angular.to_array(),
                }
            })
            .collect(),
        robots: vec![arena_net::session::Robot {
            slot: 0,
            hp: r.damage.health.parts.iter().map(|p| p.current).collect(),
            energy: r.rig.as_ref().unwrap().electrical.remaining,
            air: r.rig.as_ref().unwrap().pneumatic.remaining,
            motors: r
                .states
                .iter()
                .map(|s| {
                    [
                        s.position,
                        s.burst_time,
                        f32::from(s.initialized),
                        f32::from(s.returning),
                    ]
                })
                .collect(),
            commands: r
                .commands
                .iter()
                .map(|c| [c.drive, c.steer, f32::from(c.fire)])
                .collect(),
            detached: vec![],
            numbers: vec![],
            shell_revision: 0,
            controls: r.controls.snapshot(),
            acknowledged: 0,
            edges: vec![],
        }],
        traps: vec![],
    };
    (source, frame)
}
fn frame(scene: &Scene, ack: u64, tick: u64) -> Frame {
    Frame {
        epoch: 1,
        input_generation: 0,
        tick,
        phase: 2,
        phase_time: 0.,
        paused: false,
        remaining: 300. - tick as f32 / 120.,
        scores: [0.; 4],
        eliminated: [false; 4],
        winners: [false; 4],
        bodies: scene.poses(),
        robots: vec![scene.checkpoint(ack)],
        traps: vec![],
    }
}
fn input(scene: &Scene, tick: u64) -> Vec<u8> {
    scene
        .robot()
        .document
        .as_ref()
        .unwrap()
        .controllers
        .iter()
        .flat_map(|c| c.keys.iter())
        .map(|key| {
            let dik = key.get(key.len().saturating_sub(2)).copied().unwrap_or(-1);
            let forward = tick < 120 || (240..360).contains(&tick);
            let reverse = (360..420).contains(&tick);
            let turn = (120..240).contains(&tick);
            if dik == 17 && forward
                || dik == 31 && reverse
                || dik == 30 && turn
                || dik == 57 && tick % 120 < 3
            {
                255
            } else {
                0
            }
        })
        .collect()
}
#[test]
#[ignore = "requires original BOT/CFZ/garage assets; no window or GPU"]
fn native_prediction_matrix_real_burst_and_piston_bots() {
    let mut csv = String::from(
        "bot,rtt_ms,loss_per_10000,bandwidth,blackout,p95_m,max_m,replayed,cpu_ms,step_ms,hard_resyncs,missing,wire_bytes,travel_m,motor_travel,energy_error,air_error\n",
    );
    for name in ["C.R.U.S.H", "Walrus"] {
        let (source, initial) = source(name);
        for (rtt, loss, bandwidth, blackout) in [
            (0, 0, 0, false),
            (50, 0, 0, false),
            (100, 0, 0, false),
            (150, 0, 0, false),
            (200, 0, 0, false),
            (200, 300, 0, false),
            (200, 300, 65536, false),
            (200, 300, 65536, true),
        ] {
            let mut profile = NetworkProfile::ideal();
            profile.upstream_ms = rtt / 2;
            profile.downstream_ms = rtt / 2;
            profile.loss_per_10000 = loss;
            if loss > 0 {
                profile.jitter_ms = 20;
                profile.duplicate_per_10000 = 100;
            }
            profile.bytes_per_second = bandwidth;
            if blackout {
                profile.blackout = Some((1_500_000, 1_900_000));
            }
            let mut wire = VirtualWire::new(profile, 19);
            let mut hlink = LinkEndpoint::new(
                arena_net::HOST,
                arena_net::PeerId(1),
                arena_net::SessionId(9),
            );
            let mut clink = LinkEndpoint::new(
                arena_net::PeerId(1),
                arena_net::HOST,
                arena_net::SessionId(9),
            );
            let mut host = Scene::new(source.clone()).unwrap();
            host.restore(&initial).unwrap();
            let mut predicted = Prediction::default();
            let mut stats = Stats::default();
            predicted
                .confirm(Some(source.clone()), &initial, 0, &mut stats)
                .unwrap();
            let mut commands = CommandBuffer::default();
            let mut snapshots = 0;
            let mut received = 0;
            let mut travel = 0_f32;
            let mut motor_travel = 0_f32;
            for tick in 1..=480 {
                let now = tick * 1_000_000 / 120;
                for message in clink
                    .update(now, wire.receive(now, arena_net::PeerId(1)), &mut wire)
                    .unwrap()
                {
                    if let Message::GameState(bytes) = message {
                        let f: Frame = arena_net::session::decode(&bytes).unwrap();
                        predicted.confirm(None, &f, 0, &mut stats).unwrap();
                        snapshots += 1;
                    }
                }
                let levels = input(predicted.scene.as_ref().unwrap(), tick);
                predicted.advance(levels, &mut stats);
                if tick % 2 == 0 {
                    clink
                        .send(&Message::Wiring(
                            arena_net::session::encode(&predicted.wire().unwrap()).unwrap(),
                        ))
                        .unwrap();
                }
                for message in hlink
                    .update(now, wire.receive(now, arena_net::HOST), &mut wire)
                    .unwrap()
                {
                    if let Message::Wiring(bytes) = message {
                        let w: Wiring = arena_net::session::decode(&bytes).unwrap();
                        commands.receive(&w).unwrap();
                        received = now;
                    }
                }
                let c = commands.advance(now.saturating_sub(received) < 250_000);
                host.step(&c);
                travel = travel.max(
                    Vec3::from_array(host.poses()[0].position)
                        .distance(Vec3::from_array(initial.bodies[0].position)),
                );
                motor_travel = motor_travel.max(
                    host.robot()
                        .states
                        .iter()
                        .zip(&initial.robots[0].motors)
                        .map(|(s, m)| (s.position - m[0]).abs())
                        .fold(0_f32, f32::max),
                );
                if tick % 4 == 0 {
                    hlink
                        .send(&Message::GameState(
                            arena_net::session::encode(&frame(&host, commands.acknowledged, tick))
                                .unwrap(),
                        ))
                        .unwrap();
                }
            }
            // Stable checkpoint and no remaining commands must restore exactly, including reservoirs.
            let mut final_state = frame(&host, commands.acknowledged, 481);
            final_state.paused = true;
            predicted
                .confirm(None, &final_state, 0, &mut stats)
                .unwrap();
            let replica = predicted
                .scene
                .as_ref()
                .unwrap()
                .checkpoint(commands.acknowledged);
            let energy = (replica.energy - final_state.robots[0].energy).abs();
            let air = (replica.air - final_state.robots[0].air).abs();
            println!(
                "native {name} RTT={rtt} loss={loss} bandwidth={bandwidth} blackout={blackout} p95={:.4} max={:.4} replay={} hard={} travel={travel:.2} motor={motor_travel:.2}",
                stats.p95(),
                stats.maximum,
                stats.replayed,
                stats.hard_resyncs
            );
            csv.push_str(&format!("{name},{rtt},{loss},{bandwidth},{blackout},{},{},{},{},{},{},{},{},{travel},{motor_travel},{energy},{air}\n",stats.p95(),stats.maximum,stats.replayed,stats.cpu_ms,stats.step_ms,stats.hard_resyncs,commands.missing,wire.metrics.bytes));
            assert!(travel > 0.1, "Native test robot did not drive");
            assert!(motor_travel > 0.01, "Native test mechanism never actuated");
            assert!(snapshots > 70);
            assert!(
                stats.p95() < if blackout { 0.15 } else { 0.05 },
                "Native P95 prediction error exceeded budget"
            );
            assert!(
                stats.maximum < if blackout { 1.5 } else { 0.5 },
                "Native correction exceeded budget"
            );
            assert_eq!(stats.hard_resyncs, 0);
            assert_eq!(replica.hp, initial.robots[0].hp, "prediction committed HP");
            assert!(energy < 0.001 && air < 0.001);
            assert_eq!(
                predicted
                    .scene
                    .as_ref()
                    .unwrap()
                    .poses()
                    .iter()
                    .map(|p| p.position)
                    .collect::<Vec<_>>(),
                final_state
                    .bodies
                    .iter()
                    .map(|p| p.position)
                    .collect::<Vec<_>>()
            );
            assert!(stats.maximum.is_finite());
        }
    }
    std::fs::create_dir_all("logs").unwrap();
    std::fs::write("logs/native-prediction.csv", csv).unwrap();
}
#[test]
#[ignore = "requires original BOT/CFZ/garage assets; no window or GPU"]
fn native_prediction_replay_is_resource_safe_and_phase_reset_drops_old_fire() {
    let (source, initial) = source("Walrus");
    let mut p = Prediction::default();
    let mut stats = Stats::default();
    p.confirm(Some(source), &initial, 0, &mut stats).unwrap();
    for tick in 1..=60 {
        let v = input(p.scene.as_ref().unwrap(), tick);
        p.advance(v, &mut stats);
    }
    let expected = p.scene.as_ref().unwrap().checkpoint(60);
    for tick in 1..20 {
        let mut f = initial.clone();
        f.tick = tick;
        p.confirm(None, &f, 0, &mut stats).unwrap();
        let r = p.scene.as_ref().unwrap().checkpoint(60);
        assert_eq!(r.air, expected.air);
        assert_eq!(r.energy, expected.energy);
        assert_eq!(r.motors, expected.motors);
        assert_eq!(r.controls, expected.controls);
    }
    let mut paused = initial.clone();
    paused.tick = 21;
    paused.input_generation = 1;
    paused.paused = true;
    p.confirm(None, &paused, 0, &mut stats).unwrap();
    assert!(p.pending.is_empty());
    assert!(!p.active);
    assert_eq!(p.sequence, 0);
}

#[test]
#[ignore = "requires original BOT/CFZ/garage assets; no window or GPU"]
fn native_prediction_history_is_bounded_and_old_checkpoint_recovers() {
    let (source, initial) = source("C.R.U.S.H");
    let mut p = Prediction::default();
    let mut stats = Stats::default();
    p.confirm(Some(source), &initial, 0, &mut stats).unwrap();
    for tick in 1..=400 {
        let v = input(p.scene.as_ref().unwrap(), tick);
        p.advance(v, &mut stats);
    }
    assert_eq!(p.pending.len(), HISTORY);
    assert_eq!(p.history.len(), HISTORY);
    let wire = p.wire().unwrap();
    assert!(wire.commands().unwrap().len() <= 33);
    let mut old = initial.clone();
    old.tick = 1;
    p.confirm(None, &old, 0, &mut stats).unwrap();
    assert_eq!(stats.hard_resyncs, 1);
    assert!(p.pending.is_empty());
    assert_eq!(p.sequence, 400);
    // Old snapshots cannot rewind a newer checkpoint or fire a previously consumed burst.
    assert_eq!(p.confirmed_tick, 1);
    p.confirm(None, &initial, 0, &mut stats).unwrap();
    assert_eq!(p.confirmed_tick, 1);
    let v = input(p.scene.as_ref().unwrap(), 1);
    p.advance(v, &mut stats);
    assert_eq!(p.sequence, 401);
    let mut resumed = initial.clone();
    resumed.tick = 2;
    resumed.input_generation = 2;
    p.confirm(None, &resumed, 0, &mut stats).unwrap();
    assert!(p.active);
    assert!(p.pending.is_empty());
    assert!(p.wire().is_none());
}

#[test]
#[ignore = "requires original BOT/CFZ/garage assets; no window or GPU"]
fn native_prediction_rebuild_keeps_authoritative_detachment_and_hp() {
    use bevy::ecs::system::RunSystemOnce;
    use bevy_rapier3d::prelude::*;
    let (source, initial) = source("C.R.U.S.H");
    let mut host = Scene::new(source.clone()).unwrap();
    host.restore(&initial).unwrap();
    let root = host
        .robot()
        .damage
        .bodies
        .iter()
        .flat_map(|b| b.wheels.iter())
        .next()
        .unwrap()
        .0;
    host.app
        .world_mut()
        .run_system_once(
            move |mut commands: Commands,
                  mut g: ResMut<Garage>,
                  poses: Query<
                (&Transform, &Velocity, &ReadWorldMassProperties),
                With<crate::gameplay::RobotBody>,
            >| {
                crate::gameplay::damage::detach(&mut commands, &mut g.instance, &poses, root);
            },
        )
        .unwrap();
    let source = Blueprint::capture_robot(host.app.world(), host.robot()).unwrap();
    let changed = frame(&host, 0, 1);
    let mut p = Prediction::default();
    let mut stats = Stats::default();
    let (original, _) = self::source("C.R.U.S.H");
    p.confirm(Some(original), &initial, 0, &mut stats).unwrap();
    for tick in 1..=5 {
        let v = input(p.scene.as_ref().unwrap(), tick);
        p.advance(v, &mut stats);
    }
    p.confirm(Some(source), &changed, 0, &mut stats).unwrap();
    assert_eq!(p.pending.len(), 5);
    assert_eq!(p.sequence, 5);
    assert_eq!(stats.topology_resets, 1);
    assert!(
        !p.scene
            .as_ref()
            .unwrap()
            .robot()
            .damage
            .health
            .operational(root)
    );
    for tick in 1..=30 {
        let v = input(p.scene.as_ref().unwrap(), tick);
        p.advance(v, &mut stats);
    }
    assert_eq!(
        p.scene
            .as_ref()
            .unwrap()
            .robot()
            .damage
            .health
            .parts
            .iter()
            .map(|p| p.current)
            .collect::<Vec<_>>(),
        changed.robots[0].hp
    );
    assert_eq!(
        p.scene.as_ref().unwrap().robot().damage.detached_roots,
        vec![root]
    );
    let poses = p.scene.as_ref().unwrap().poses();
    assert!(
        poses
            .iter()
            .all(|p| p.position.iter().all(|v| v.is_finite()))
    );
    let mut bad = changed.clone();
    bad.tick = 2;
    bad.bodies.pop();
    assert!(p.scene.as_mut().unwrap().restore(&bad).is_err());
    assert_eq!(
        poses.iter().map(|p| p.position).collect::<Vec<_>>(),
        p.scene
            .as_ref()
            .unwrap()
            .poses()
            .iter()
            .map(|p| p.position)
            .collect::<Vec<_>>()
    );
}
