use arena_net::{session::*, transport::*, *};
use arena_net_io::session::{Node, Notice, Request, Stage};
fn settings() -> Settings {
    Settings {
        name: "LAN match".into(),
        arena: "combatarena".into(),
        mode: 1,
        minutes: 5,
        hazards: true,
        weight: None,
    }
}
fn player(slot: u8) -> Player {
    Player {
        slot,
        alias: format!("P{slot}"),
        team: "Custom team".into(),
        bot_name: format!("BOT{slot}"),
        digest: [0; 32],
        size: 0,
        ready: false,
    }
}
#[test]
fn native_lobby_transfers_custom_bots_and_starts_only_after_all_worlds_are_loaded() {
    let mut profile = NetworkProfile::ideal();
    profile.upstream_ms = 100;
    profile.downstream_ms = 100;
    profile.jitter_ms = 20;
    profile.loss_per_10000 = 500;
    profile.duplicate_per_10000 = 200;
    let mut wire = VirtualWire::new(profile, 41);
    let mut host = Node::new(HOST, settings()).unwrap();
    let mut a = Node::new(PeerId(1), settings()).unwrap();
    let mut b = Node::new(PeerId(2), settings()).unwrap();
    host.add_peer(PeerId(1), SessionId(1), 0).unwrap();
    a.add_peer(HOST, SessionId(1), 0).unwrap();
    host.add_peer(PeerId(2), SessionId(2), 0).unwrap();
    b.add_peer(HOST, SessionId(2), 0).unwrap();
    let bots = (0..3)
        .map(|slot| {
            (0..(16000 + slot * 100))
                .map(|i| ((i + slot) % 251) as u8)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    host.request(Request::Select(player(0), bots[0].clone()))
        .unwrap();
    a.request(Request::Select(player(1), bots[1].clone()))
        .unwrap();
    b.request(Request::Select(player(2), bots[2].clone()))
        .unwrap();
    assert!(host.request(Request::Start).is_err());
    for t in 1..=1800 {
        let now = t * 1_000_000 / HZ;
        host.pump(now, &mut wire).unwrap();
        a.pump(now, &mut wire).unwrap();
        b.pump(now, &mut wire).unwrap();
        host.notices.clear();
        a.notices.clear();
        b.notices.clear();
    }
    for node in [&host, &a, &b] {
        assert_eq!(node.players.len(), 3);
        for bot in &bots {
            let d = *blake3::hash(bot).as_bytes();
            assert_eq!(node.bots.get(&d), Some(bot));
        }
    }
    host.request(Request::Ready(true)).unwrap();
    a.request(Request::Ready(true)).unwrap();
    b.request(Request::Ready(true)).unwrap();
    for t in 1801..=1920 {
        let now = t * 1_000_000 / HZ;
        for n in [&mut host, &mut a, &mut b] {
            n.pump(now, &mut wire).unwrap();
            n.notices.clear();
        }
    }
    assert!(host.can_start());
    host.request(Request::Start).unwrap();
    assert_eq!(host.stage, Stage::Loading);
    assert!(host.add_peer(PeerId(3), SessionId(3), 16_000_000).is_err());
    for t in 1921..=2040 {
        let now = t * 1_000_000 / HZ;
        for n in [&mut host, &mut a, &mut b] {
            n.pump(now, &mut wire).unwrap();
        }
    }
    let epoch = host.epoch;
    host.request(Request::Loaded(epoch)).unwrap();
    a.request(Request::Loaded(epoch)).unwrap();
    for t in 2041..=2100 {
        let now = t * 1_000_000 / HZ;
        for n in [&mut host, &mut a, &mut b] {
            n.pump(now, &mut wire).unwrap();
        }
    }
    assert_eq!(
        host.stage,
        Stage::Loading,
        "a slow third loader was skipped"
    );
    b.request(Request::Loaded(epoch)).unwrap();
    for t in 2101..=2220 {
        let now = t * 1_000_000 / HZ;
        for n in [&mut host, &mut a, &mut b] {
            n.pump(now, &mut wire).unwrap();
        }
    }
    for node in [&host, &a, &b] {
        assert_eq!(node.stage, Stage::Playing);
        assert!(
            node.notices
                .iter()
                .any(|n| matches!(n,Notice::Begin(e) if *e==epoch))
        );
    }
    // Native control levels and counters survive duplicates; only this connection's slot is writable.
    a.request(Request::Wiring(Wiring {
        generation: 0,
        redundant: vec![],
        epoch,
        sequence: 4,
        levels: vec![255, 0],
        edges: vec![1, 0],
    }))
    .unwrap();
    a.request(Request::Wiring(Wiring {
        generation: 0,
        redundant: vec![],
        epoch,
        sequence: 3,
        levels: vec![0, 0],
        edges: vec![0, 0],
    }))
    .unwrap();
    host.request(Request::Pause(17, true)).unwrap();
    for t in 2221..=2400 {
        let now = t * 1_000_000 / HZ;
        for n in [&mut host, &mut a, &mut b] {
            n.pump(now, &mut wire).unwrap();
        }
    }
    assert_eq!(host.inputs[&1].sequence, 4);
    assert!(a.request(Request::Pause(18, false)).is_err());
    assert!(
        a.notices
            .iter()
            .any(|n| matches!(n, Notice::Pause(17, true)))
    );
    host.request(Request::ReturnLobby).unwrap();
    for t in 2401..=2520 {
        let now = t * 1_000_000 / HZ;
        for n in [&mut host, &mut a, &mut b] {
            n.pump(now, &mut wire).unwrap();
        }
    }
    assert_eq!(host.stage, Stage::Lobby);
    assert_eq!(a.stage, Stage::Lobby);
    assert_eq!(b.stage, Stage::Lobby);
    assert!(host.players.values().all(|p| !p.ready));
    assert!(!host.can_start());
    host.request(Request::Leave).unwrap();
    for t in 2521..=2640 {
        let now = t * 1_000_000 / HZ;
        for n in [&mut host, &mut a, &mut b] {
            n.pump(now, &mut wire).unwrap();
        }
    }
    assert_eq!(a.stage, Stage::Closed);
    assert_eq!(b.stage, Stage::Closed);
}
#[test]
fn peer_timeout_ends_running_session_and_bot_selection_cannot_change_in_a_match() {
    let mut node = Node::new(HOST, settings()).unwrap();
    node.add_peer(PeerId(1), SessionId(1), 0).unwrap();
    node.stage = Stage::Playing;
    assert!(
        node.request(Request::Select(player(0), vec![1; 32]))
            .is_err()
    );
    let mut wire = VirtualWire::new(NetworkProfile::ideal(), 1);
    node.pump(TIMEOUT_US + 1, &mut wire).unwrap();
    assert_eq!(node.stage, Stage::Closed);
}

#[test]
fn input_generation_accepts_restarted_sequences_and_rejects_late_or_future_pulses() {
    let mut host = Node::new(HOST, settings()).unwrap();
    let mut client = Node::new(PeerId(1), settings()).unwrap();
    host.add_peer(PeerId(1), SessionId(5), 0).unwrap();
    client.add_peer(HOST, SessionId(5), 0).unwrap();
    host.stage = Stage::Playing;
    client.stage = Stage::Playing;
    let mut wire = VirtualWire::new(NetworkProfile::ideal(), 1);
    let packet = |generation, sequence| Wiring {
        epoch: 0,
        generation,
        sequence,
        levels: vec![255],
        edges: vec![1],
        redundant: vec![],
    };
    client.request(Request::Wiring(packet(0, 200))).unwrap();
    for t in 1..=10 {
        client.pump(t * 10_000, &mut wire).unwrap();
        host.pump(t * 10_000, &mut wire).unwrap();
    }
    assert_eq!(host.inputs[&1].sequence, 200);
    let frame = Frame {
        epoch: 0,
        input_generation: 2,
        tick: 240,
        phase: 2,
        phase_time: 0.,
        paused: false,
        remaining: 300.,
        scores: [0.; 4],
        eliminated: [false; 4],
        immobility_countdown: [0; 4],
        counted_out: [false; 4],
        winners: [false; 4],
        bodies: vec![],
        robots: vec![],
        traps: vec![],
    };
    host.request(Request::Frame(frame)).unwrap();
    for command in [packet(2, 1), packet(0, 201), packet(100, 999)] {
        client.request(Request::Wiring(command)).unwrap();
    }
    for t in 11..=20 {
        client.pump(t * 10_000, &mut wire).unwrap();
        host.pump(t * 10_000, &mut wire).unwrap();
    }
    assert_eq!(
        (host.inputs[&1].generation, host.inputs[&1].sequence),
        (2, 1)
    );
    client.request(Request::Wiring(packet(2, 2))).unwrap();
    for t in 21..=30 {
        client.pump(t * 10_000, &mut wire).unwrap();
        host.pump(t * 10_000, &mut wire).unwrap();
    }
    assert_eq!(host.inputs[&1].sequence, 2);
}
