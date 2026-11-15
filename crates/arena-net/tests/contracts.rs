use arena_core::{Quat, Vec3};
use arena_net::{
    codec::{self, Message},
    input::*,
    lobby::*,
    snapshot::*,
    transport::*,
    *,
};
fn state() -> Snapshot {
    Snapshot {
        tick: 10,
        sim_tick: 10,
        paused: false,
        bodies: vec![BodyState {
            id: BodyId(1),
            owner: HOST,
            parent: None,
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            linear: Vec3::X,
            angular: Vec3::ZERO,
        }],
        robots: vec![RobotState {
            owner: HOST,
            hp: vec![100.],
            energy: 100.,
            air: 10.,
            score: 0.,
            revision: 0,
            action: 0,
            motors: vec![],
        }],
    }
}
#[test]
fn wire_roundtrip_truncation_limits_and_non_finite_values() {
    let cases = [
        Message::State(state()),
        Message::Inputs(vec![Input {
            tick: 5,
            throttle: 123,
            steer: -45,
            action: 7,
        }]),
        Message::Event(Event {
            id: 1,
            tick: 2,
            kind: EventKind::Damage {
                victim: HOST,
                part: 0,
                hp: 12.,
            },
        }),
        Message::Pause(true),
        Message::Goodbye,
    ];
    for value in cases {
        let bytes = codec::encode(&value).unwrap();
        assert_eq!(codec::decode(&bytes).unwrap(), value);
        for n in 0..bytes.len() {
            assert!(
                codec::decode(&bytes[..n]).is_err(),
                "accepted truncated packet {n}"
            );
        }
        let mut bad = bytes.clone();
        bad.push(0);
        assert!(codec::decode(&bad).is_err());
        let mut version = bytes;
        version[4] = 99;
        assert_eq!(codec::decode(&version), Err(Error::Version));
    }
    assert_eq!(
        codec::decode(&vec![0; codec::MAX_MESSAGE + 1]),
        Err(Error::Limit)
    );
    let mut s = state();
    s.bodies[0].position.x = f32::NAN;
    assert!(codec::encode(&Message::State(s)).is_err());
    let mut s = state();
    s.bodies[0].rotation = Quat::from_xyzw(0., 0., 0., 0.);
    assert!(s.validate().is_err());
    // Unknown/malformed bytes must produce errors, never panics or unbounded allocations.
    let mut rng = 7u64;
    for len in 0..2048 {
        let mut b = vec![0; len];
        for x in &mut b {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            *x = rng as u8;
        }
        let _ = codec::decode(&b);
    }
}
#[test]
fn connection_roles_cannot_publish_foreign_inputs_or_authoritative_state() {
    let client = PeerId(1);
    let stranger = PeerId(2);
    let inputs = Message::Inputs(vec![]);
    let state = Message::State(state());
    assert_eq!(inputs.validate_direction(client, HOST), Ok(()));
    assert_eq!(
        inputs.validate_direction(HOST, client),
        Err(Error::Unauthorized)
    );
    assert_eq!(
        inputs.validate_direction(client, stranger),
        Err(Error::Unauthorized)
    );
    assert_eq!(state.validate_direction(HOST, client), Ok(()));
    assert_eq!(
        state.validate_direction(client, HOST),
        Err(Error::Unauthorized)
    );
    assert_eq!(
        Message::Pause(true).validate_direction(client, HOST),
        Err(Error::Unauthorized)
    );
    assert_eq!(
        Message::Goodbye.validate_direction(client, client),
        Err(Error::Unauthorized)
    );
}
#[test]
fn input_duplicates_do_not_repeat_fire_and_a_lost_release_cannot_stick() {
    let mut b = InputBuffer::default();
    let f = Input {
        tick: 5,
        throttle: 32767,
        steer: 0,
        action: 1,
    };
    for _ in 0..100 {
        b.receive(0, f).unwrap();
    }
    assert_eq!(b.len(), 1);
    assert!(b.consume(5).fire);
    for t in 6..40 {
        assert!(!b.consume(t).fire)
    }
    assert_eq!(b.consume(41).input.throttle, 0);
    assert_eq!(b.receive(0, Input { tick: HZ + 1, ..f }), Err(Error::Limit));
    assert_eq!(
        b.receive(
            0,
            Input {
                tick: 6,
                throttle: i16::MIN,
                ..f
            }
        ),
        Err(Error::Malformed)
    );
    b.receive(0, Input { tick: 6, ..f }).unwrap();
    assert_eq!(
        b.receive(
            0,
            Input {
                tick: 6,
                steer: 1,
                ..f
            }
        ),
        Err(Error::Conflict)
    );
}
#[test]
fn lobby_checks_content_ownership_start_pause_and_host_exit() {
    let manifest = Manifest {
        protocol: PROTOCOL,
        build: [1; 32],
        assets: [2; 32],
        physics: [3; 32],
    };
    let bytes = b"test fixture BOT bytes";
    let digest = *blake3::hash(bytes).as_bytes();
    let mut l = Lobby::new(manifest.clone());
    for peer in [HOST, PeerId(1)] {
        l.join(peer, &manifest, digest).unwrap();
        if peer == HOST {
            assert_eq!(l.ready(peer), Err(Error::Incompatible));
            let mut upload = BotUpload::new(digest, bytes.len()).unwrap();
            upload.chunk(0, bytes).unwrap();
            l.install_bot(peer, upload).unwrap();
        }
        l.ready(peer).unwrap();
    }
    let mut wrong = manifest.clone();
    wrong.physics = [4; 32];
    assert_eq!(l.join(PeerId(2), &wrong, digest), Err(Error::Incompatible));
    assert_eq!(l.start(PeerId(1)), Err(Error::Unauthorized));
    l.start(HOST).unwrap();
    assert_eq!(l.join(PeerId(2), &manifest, digest), Err(Error::Phase));
    assert_eq!(l.pause(PeerId(1), true), Err(Error::Unauthorized));
    l.pause(HOST, true).unwrap();
    assert_eq!(l.phase, Phase::Paused);
    l.pause(HOST, false).unwrap();
    l.leave(HOST);
    assert_eq!(l.phase, Phase::Finished);
}
#[test]
fn bot_transfer_is_bounded_ordered_and_hash_verified() {
    assert!(BotUpload::new([0; 32], MAX_BOT_BYTES + 1).is_err());
    let mut u = BotUpload::new([0; 32], 3).unwrap();
    assert_eq!(u.chunk(1, b"abc"), Err(Error::Conflict));
    u.chunk(0, b"abc").unwrap();
    assert_eq!(u.chunk(3, b"x"), Err(Error::Limit));
    assert_eq!(u.finish(), Err(Error::Hash));
}
#[test]
fn interpolation_uses_host_time_rejects_stale_and_does_not_blend_detach() {
    let mut a = state();
    a.tick = 0;
    a.sim_tick = 0;
    let mut b = a.clone();
    b.tick = 10;
    b.bodies[0].position.x = 10.;
    let mut i = Interpolation::default();
    i.push(a.clone()).unwrap();
    i.push(b.clone()).unwrap();
    assert!(!i.push(a).unwrap());
    assert!((i.sample(5.).unwrap().bodies[0].position.x - 5.).abs() < 1e-5);
    // Extrapolation is capped to 50ms even after a long blackout.
    assert!(i.sample(1000.).unwrap().bodies[0].position.x <= 10.051);
    let mut c = b.clone();
    c.tick = 20;
    c.bodies[0].position.x = 40.;
    c.robots[0].revision += 1;
    i.push(c).unwrap();
    assert_eq!(i.sample(15.).unwrap().bodies[0].position.x, 10.);
}
#[test]
fn snapshot_rejects_cycles_duplicate_ids_and_cross_robot_parents() {
    let mut s = state();
    s.bodies[0].parent = Some(BodyId(1));
    assert!(s.validate().is_err());
    let mut s = state();
    s.bodies.push(s.bodies[0].clone());
    assert!(s.validate().is_err());
    let mut s = state();
    let mut foreign = s.robots[0].clone();
    foreign.owner = PeerId(1);
    s.robots.push(foreign);
    let mut child = s.bodies[0].clone();
    child.id = BodyId(2);
    child.owner = PeerId(1);
    child.parent = Some(BodyId(1));
    s.bodies.push(child);
    assert!(s.validate().is_err());
    let mut s = state();
    s.sim_tick = s.tick + 1;
    assert_eq!(s.validate(), Err(Error::Limit));
}
#[test]
fn events_are_ordered_deduplicated_and_do_not_subtract_hp_twice() {
    let mut cursor = EventCursor::default();
    let e = Event {
        id: 1,
        tick: 5,
        kind: EventKind::Paused(true),
    };
    assert!(cursor.accept(&e).unwrap());
    assert!(!cursor.accept(&e).unwrap());
    assert_eq!(
        cursor.accept(&Event { id: 3, ..e.clone() }),
        Err(Error::Conflict)
    );
    assert!(cursor.accept(&Event { id: 2, ..e }).unwrap());
}
#[test]
fn virtual_wire_is_seeded_bounded_and_models_each_direction() {
    let mut p = NetworkProfile::ideal();
    p.upstream_ms = 20;
    p.downstream_ms = 180;
    let mut w = VirtualWire::new(p, 7);
    w.send(
        0,
        Datagram {
            from: PeerId(1),
            to: HOST,
            bytes: vec![1],
        },
    )
    .unwrap();
    w.send(
        0,
        Datagram {
            from: HOST,
            to: PeerId(1),
            bytes: vec![2],
        },
    )
    .unwrap();
    assert!(w.receive(19_999, HOST).is_empty());
    assert_eq!(w.receive(20_000, HOST).len(), 1);
    assert!(w.receive(179_999, PeerId(1)).is_empty());
    assert_eq!(w.receive(180_000, PeerId(1)).len(), 1);
    assert_eq!(
        w.send(
            0,
            Datagram {
                from: HOST,
                to: PeerId(1),
                bytes: vec![0; MTU + 1]
            }
        ),
        Err(Error::Limit)
    );
}
#[test]
fn four_timestamp_clock_filters_queueing_and_exposes_asymmetric_bias() {
    use arena_net::clock::ClockSync;
    let mut c = ClockSync::default();
    c.observe(0, 150_000, 160_000, 210_000).unwrap();
    assert_eq!(c.rtt_us(), Some(200_000));
    assert_eq!(c.host_time_us(1_000_000), 1_050_000.);
    // A higher-RTT queued sample must not pull the established clock around.
    c.observe(1_000_000, 1_350_000, 1_360_000, 1_410_000)
        .unwrap();
    assert_eq!(c.host_time_us(1_000_000), 1_050_000.);
    assert_eq!(c.observe(100, 0, 200, 101), Err(Error::Malformed));
}
#[test]
fn interpolation_keeps_articulated_attachment_anchors_coincident() {
    let mut a = state();
    a.tick = 0;
    a.sim_tick = 0;
    let mut child = a.bodies[0].clone();
    child.id = BodyId(2);
    child.parent = Some(BodyId(1));
    child.position = Vec3::X;
    a.bodies.push(child);
    let mut b = a.clone();
    b.tick = 10;
    b.bodies[0].rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    b.bodies[1].rotation = b.bodies[0].rotation;
    b.bodies[1].position = Vec3::NEG_Z;
    let mut i = Interpolation::with_joints([JointFrames {
        child: BodyId(2),
        parent: BodyId(1),
        parent_anchor: Vec3::ZERO,
        child_anchor: Vec3::NEG_X,
    }])
    .unwrap();
    i.push(a).unwrap();
    i.push(b).unwrap();
    let s = i.sample(5.).unwrap();
    assert!((s.bodies[1].position.length() - 1.).abs() < 1e-5);
    assert!(s.bodies[1].position.abs_diff_eq(
        s.bodies[0].position - s.bodies[1].rotation * Vec3::NEG_X,
        1e-5
    ));
}
