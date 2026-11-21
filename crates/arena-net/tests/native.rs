use arena_net::{
    codec::{self, Message},
    session::*,
    *,
};
fn frame() -> Frame {
    Frame {
        input_generation: 0,
        epoch: 1,
        tick: 120,
        phase: 2,
        phase_time: 0.,
        paused: false,
        remaining: 300.,
        scores: [0.; 4],
        eliminated: [false; 4],
        immobility_countdown: [0; 4],
        counted_out: [false; 4],
        winners: [false; 4],
        bodies: vec![Pose {
            id: 65536,
            position: [0., 1., 0.],
            rotation: [0., 0., 0., 1.],
            linear: [0.; 3],
            angular: [0.; 3],
        }],
        robots: vec![],
        traps: vec![Trap {
            index: 0,
            clock: 1.,
            active: true,
            occupied: false,
            zapping: vec![1],
        }],
    }
}
#[test]
fn native_state_codec_bounds_truncation_role_checks_and_non_finite_values() {
    let f = frame();
    f.validate().unwrap();
    let bytes = encode(&f).unwrap();
    let decoded: Frame = decode(&bytes).unwrap();
    assert_eq!(decoded.tick, 120);
    assert!(decoded.traps[0].active);
    for n in 0..bytes.len() {
        assert!(decode::<Frame>(&bytes[..n]).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(decode::<Frame>(&trailing).is_err());
    let mut invalid = f.clone();
    invalid.bodies[0].position[0] = f32::NAN;
    assert_eq!(invalid.validate(), Err(Error::Malformed));
    let mut invalid = f;
    invalid.bodies.push(invalid.bodies[0].clone());
    assert_eq!(invalid.validate(), Err(Error::Conflict));
    for m in [
        Message::Control(encode(&Control::Ready(true)).unwrap()),
        Message::GameState(bytes),
        Message::Wiring(
            encode(&Wiring {
                generation: 0,
                redundant: vec![],
                epoch: 1,
                sequence: 1,
                levels: vec![255],
                edges: vec![1],
            })
            .unwrap(),
        ),
    ] {
        assert_eq!(codec::decode(&codec::encode(&m).unwrap()).unwrap(), m);
    }
    assert_eq!(
        Message::GameState(vec![]).validate_direction(PeerId(1), HOST),
        Err(Error::Unauthorized)
    );
    assert_eq!(
        Message::Wiring(vec![]).validate_direction(HOST, PeerId(1)),
        Err(Error::Unauthorized)
    );
    assert!(
        Wiring {
            generation: 0,
            redundant: vec![],
            epoch: 1,
            sequence: 1,
            levels: vec![255; 129],
            edges: vec![1; 129]
        }
        .validate()
        .is_err()
    );
}
#[test]
fn native_commands_recover_redundant_steps_but_never_replay_a_consumed_edge() {
    use arena_net::native_input::CommandBuffer;
    let mut b = CommandBuffer::default();
    let packet = Wiring {
        epoch: 1,
        generation: 0,
        sequence: 3,
        levels: vec![0],
        edges: vec![1],
        redundant: vec![
            InputCommand {
                sequence: 1,
                levels: vec![255],
                edges: vec![1],
            },
            InputCommand {
                sequence: 2,
                levels: vec![0],
                edges: vec![1],
            },
        ],
    };
    b.receive(&packet).unwrap();
    assert_eq!(b.len(), 3);
    for sequence in 1..=3 {
        assert_eq!(b.advance(true).sequence, sequence);
    }
    b.receive(&packet).unwrap();
    assert!(b.is_empty());
    assert_eq!(b.advance(false).levels, vec![0]);
    assert_eq!(b.acknowledged, 4);
    let future = Wiring {
        sequence: 10000,
        ..packet
    };
    assert!(b.receive(&future).is_err());
    assert!(b.is_empty());
}

#[test]
fn native_command_runs_are_lossless_bounded_and_rejected_atomically() {
    use arena_net::native_input::CommandBuffer;
    let run = Wiring {
        epoch: 1,
        generation: 0,
        sequence: 33,
        levels: vec![0; 128],
        edges: vec![1; 128],
        redundant: vec![InputCommand {
            sequence: 1,
            levels: vec![255; 128],
            edges: vec![1; 128],
        }],
    };
    let commands = run.commands().unwrap();
    assert_eq!(commands.len(), 33);
    assert!(commands[..32].iter().all(|c| c.levels == vec![255; 128]));
    assert_eq!(commands[32].levels, vec![0; 128]);
    let mut b = CommandBuffer::default();
    b.receive(&run).unwrap();
    let mut conflict = run.clone();
    conflict.redundant[0].levels[0] = 0;
    assert_eq!(b.receive(&conflict), Err(Error::Conflict));
    assert_eq!(b.len(), 33);
    assert_eq!(b.advance(true).levels[0], 255);
    let mut too_long = run.clone();
    too_long.sequence = 34;
    assert!(too_long.validate().is_err());
    let mut unsorted = run.clone();
    unsorted.redundant.push(unsorted.redundant[0].clone());
    assert!(unsorted.validate().is_err());
    // Stale input releases continuous drive, retaining monotonic edge counters.
    let stale = b.advance(false);
    assert_eq!(stale.levels, vec![0; 128]);
    assert_eq!(stale.edges, vec![1; 128]);
    assert!(
        encode(&run).unwrap().len() < 1200,
        "held-input run no longer fits a single datagram payload"
    );
}

#[test]
fn native_history_gap_resync_skips_sequences_without_simulating_extra_steps() {
    use arena_net::native_input::CommandBuffer;
    let mut b = CommandBuffer::default();
    let mut w = Wiring {
        epoch: 1,
        generation: 0,
        sequence: 1,
        levels: vec![0],
        edges: vec![0],
        redundant: vec![],
    };
    b.receive(&w).unwrap();
    b.advance(true);
    w.sequence = 400;
    w.levels[0] = 255;
    w.edges[0] = 1;
    b.receive(&w).unwrap();
    assert_eq!(b.resyncs, 1);
    assert_eq!(b.acknowledged, 399);
    let c = b.advance(true);
    assert_eq!(c.sequence, 400);
    assert_eq!(c.edges, vec![1]);
    assert_eq!(b.advance(true).sequence, 401);
    assert!(b.len() <= HISTORY);
}
