use arena_net::{input::*, prediction::*, *};
use arena_net_lab::{physics::Physics, runner};
use std::collections::BTreeMap;
#[test]
fn reliable_pause_rewinds_to_its_tick_and_old_events_cannot_repause_a_new_checkpoint() {
    let mut p = Predictor::new(Physics::new(2), PeerId(1));
    assert!(p.control_pause(0, true).unwrap());
    for tick in 1..=60 {
        p.advance(Input {
            tick,
            throttle: 32767,
            ..Default::default()
        })
        .unwrap();
    }
    assert_eq!(p.sim.snapshot().sim_tick, 0);
    assert!(p.control_pause(30, false).unwrap());
    assert_eq!(p.sim.snapshot().tick, 60);
    assert_eq!(p.sim.snapshot().sim_tick, 30);
    let checkpoint = p.sim.snapshot();
    p.reconcile(&checkpoint).unwrap();
    assert!(!p.control_pause(0, true).unwrap());
    assert_eq!(p.sim.snapshot(), checkpoint);
}
#[test]
fn three_players_use_the_same_runtime_and_invalid_input_batches_do_not_partially_apply() {
    let r = runner::run(runner::profiles().remove(0), 3, 7).unwrap();
    assert!(r.passed(), "{:?}", r.failures);
    let mut host = Host::new(Physics::new(3), [HOST, PeerId(1), PeerId(2)]);
    let valid = Input {
        tick: 1,
        throttle: 32767,
        ..Default::default()
    };
    assert_eq!(host.receive(PeerId(3), &[valid]), Err(Error::Unauthorized));
    assert_eq!(
        host.receive(
            PeerId(1),
            &[
                valid,
                Input {
                    tick: HZ + 1,
                    ..valid
                }
            ]
        ),
        Err(Error::Limit)
    );
    assert!(host.inputs[&PeerId(1)].is_empty());
    host.receive(PeerId(1), &[valid]).unwrap();
    assert_eq!(host.inputs[&PeerId(1)].len(), 1);
}
#[test]
fn quality_matrix_two_and_four_players_all_profiles_three_seeds() {
    let mut failures = vec![];
    for profile in runner::profiles() {
        for players in [2, 4] {
            for seed in [7, 19, 41] {
                let r = runner::run(profile.clone(), players, seed).unwrap();
                println!(
                    "{} peers={players} seed={seed} RTT={} p95={:.4} max={:.4} interp={:.4} events={}/{} final={} {:?}",
                    r.profile,
                    r.rtt_ms,
                    r.correction_p95_m,
                    r.correction_max_m,
                    r.interpolation_p95_m,
                    r.events_received,
                    r.events_sent * (players - 1) as u64,
                    r.final_state_equal,
                    r.failures
                );
                if !r.passed() {
                    failures.push(format!(
                        "{} / {players} / {seed}: {:?}",
                        r.profile, r.failures
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[test]
fn a_seed_reproduces_packets_and_gameplay_metrics_not_wall_clock_timings() {
    let p = runner::profiles()
        .into_iter()
        .find(|p| p.name == "rtt-200-loss")
        .unwrap();
    let a = runner::run(p.clone(), 4, 19).unwrap();
    let b = runner::run(p, 4, 19).unwrap();
    assert_eq!(a.wire, b.wire);
    assert_eq!(a.correction_p95_m, b.correction_p95_m);
    assert_eq!(a.correction_max_m, b.correction_max_m);
    assert_eq!(a.events_sent, b.events_sent);
    assert_eq!(a.shots, b.shots);
}
#[test]
fn restoring_four_robot_checkpoint_discards_all_old_rapier_island_links() {
    let mut sim = Physics::new(4);
    let inputs = (0..4)
        .map(|i| {
            (
                PeerId(i),
                AppliedInput {
                    input: Input {
                        tick: 1,
                        throttle: 32767,
                        ..Default::default()
                    },
                    fire: false,
                },
            )
        })
        .collect();
    for t in 1..180 {
        sim.step(t, &inputs, true);
    }
    let checkpoint = sim.snapshot();
    for _ in 0..20 {
        sim.restore(&checkpoint).unwrap();
        for t in checkpoint.tick + 1..checkpoint.tick + 30 {
            sim.step(t, &inputs, false);
        }
        assert!(sim.snapshot().bodies.iter().all(|b| b.position.is_finite()));
    }
}
#[test]
fn detached_arm_keeps_its_child_joint_and_invalid_restore_is_atomic() {
    let mut sim = Physics::new(2);
    let before = sim.snapshot();
    let mut invalid = before.clone();
    invalid.robots[0].motors.clear();
    assert_eq!(sim.restore(&invalid), Err(Error::Incompatible));
    assert_eq!(sim.snapshot(), before);
    let mut detached = before;
    detached.robots[0].hp[1] = 0.;
    detached.robots[0].revision = 1;
    detached.bodies[1].parent = None;
    sim.restore(&detached).unwrap();
    assert_eq!(sim.world.impulse_joints.len(), 3);
    for t in 1..180 {
        sim.step(t, &BTreeMap::new(), false);
    }
    let s = sim.snapshot();
    assert_eq!(s.body(BodyId(1)).unwrap().parent, None);
    assert_eq!(s.body(BodyId(2)).unwrap().parent, Some(BodyId(1)));
    assert!(
        (s.body(BodyId(1))
            .unwrap()
            .position
            .distance(s.body(BodyId(2)).unwrap().position)
            - 0.65)
            .abs()
            < 0.03
    );
}
#[test]
fn prediction_cannot_commit_hp_score_or_damage_events_and_host_pause_freezes_resources() {
    let mut host = Host::new(Physics::new(2), [HOST, PeerId(1)]);
    let mut local = Physics::new(2);
    let original = local.snapshot();
    let mut damage = 0;
    for t in 1..480 {
        let commands = BTreeMap::from([
            (
                HOST,
                AppliedInput {
                    input: Input {
                        tick: t,
                        throttle: 32767,
                        ..Default::default()
                    },
                    fire: false,
                },
            ),
            (
                PeerId(1),
                AppliedInput {
                    input: Input {
                        tick: t,
                        throttle: -32767,
                        ..Default::default()
                    },
                    fire: false,
                },
            ),
        ]);
        damage += host.sim.step(t, &commands, true).len();
        assert!(local.step(t, &commands, false).is_empty());
    }
    assert!(damage > 0);
    for (i, r) in local.snapshot().robots.iter().enumerate() {
        assert_eq!(r.hp, original.robots[i].hp);
        assert_eq!(r.score, 0.);
    }
    assert_eq!(host.pause(PeerId(1), true), Err(Error::Unauthorized));
    host.pause(HOST, true).unwrap();
    let before = host.sim.snapshot();
    for _ in 0..120 {
        host.advance();
    }
    let after = host.sim.snapshot();
    assert_eq!(after.sim_tick, before.sim_tick);
    assert_eq!(after.bodies, before.bodies);
    assert_eq!(after.robots, before.robots);
    assert_eq!(after.tick, before.tick + 120);
}
#[test]
fn prediction_history_is_bounded_and_long_outages_hard_resync_instead_of_spiking_cpu() {
    let sim = Physics::new(2);
    let old = sim.snapshot();
    let mut p = Predictor::new(sim, PeerId(1));
    for tick in 1..=(HISTORY + 40) as u64 {
        p.advance(Input {
            tick,
            ..Default::default()
        })
        .unwrap();
    }
    assert!(p.recent_inputs(usize::MAX).len() <= 16);
    p.reconcile(&old).unwrap();
    assert_eq!(p.metrics.hard_resyncs, 1);
    assert_eq!(p.sim.snapshot().tick, 0);
    assert_eq!(p.metrics.replayed_ticks, 0);
}
