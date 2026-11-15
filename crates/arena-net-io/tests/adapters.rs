use arena_net::{
    codec::Message,
    lobby::BotUpload,
    snapshot::{BodyState, Event, EventCursor, EventKind, RobotState, Snapshot},
    transport::*,
    *,
};
use arena_net_io::{LinkEndpoint, udp::*};
#[test]
fn maximum_body_snapshot_is_fragmented_under_mtu_and_reassembled_after_loss() {
    let mut profile = NetworkProfile::ideal();
    profile.upstream_ms = 100;
    profile.downstream_ms = 100;
    profile.jitter_ms = 20;
    profile.loss_per_10000 = 300;
    let mut wire = VirtualWire::new(profile, 41);
    let mut host = LinkEndpoint::new(HOST, PeerId(1), SessionId(99));
    let mut client = LinkEndpoint::new(PeerId(1), HOST, SessionId(99));
    let mut state = Snapshot {
        bodies: (0..MAX_BODIES)
            .map(|i| BodyState {
                id: BodyId(i as u32),
                owner: PeerId((i / 32) as u64),
                parent: if i.is_multiple_of(32) {
                    None
                } else {
                    Some(BodyId((i - 1) as u32))
                },
                position: Default::default(),
                rotation: Default::default(),
                linear: Default::default(),
                angular: Default::default(),
            })
            .collect(),
        robots: (0..4)
            .map(|i| RobotState {
                owner: PeerId(i),
                hp: vec![100.; 32],
                energy: 100.,
                air: 100.,
                score: 0.,
                revision: 0,
                action: 0,
                motors: vec![],
            })
            .collect(),
        ..Default::default()
    };
    assert!(
        arena_net::codec::encode(&Message::State(state.clone()))
            .unwrap()
            .len()
            > MTU * 4
    );
    assert_eq!(
        client.send(&Message::State(state.clone())),
        Err(Error::Unauthorized)
    );
    let mut received = 0;
    for tick in 1..=360 {
        let now = tick * 1_000_000 / HZ;
        if tick.is_multiple_of(4) {
            state.tick = tick;
            state.sim_tick = tick;
            host.send(&Message::State(state.clone())).unwrap();
        }
        let packets = wire.receive(now, HOST);
        host.update(now, packets, &mut wire).unwrap();
        client.send(&Message::Inputs(vec![])).unwrap();
        let packets = wire.receive(now, PeerId(1));
        for message in client.update(now, packets, &mut wire).unwrap() {
            if let Message::State(snapshot) = message {
                assert!(
                    snapshot.tick > received,
                    "sequenced lane accepted an old snapshot"
                );
                received = snapshot.tick;
                let expected = Snapshot {
                    tick: received,
                    sim_tick: received,
                    ..state.clone()
                };
                assert_eq!(snapshot, expected);
            }
        }
    }
    assert!(
        received > 300,
        "large snapshots failed to recover after dropped fragments"
    );
    assert!(wire.metrics.dropped > 0);
}
#[test]
fn lightyear_reliable_events_and_bot_bulk_survive_loss_without_sharing_order() {
    let mut p = NetworkProfile::ideal();
    p.upstream_ms = 100;
    p.downstream_ms = 100;
    p.jitter_ms = 20;
    p.loss_per_10000 = 500;
    p.duplicate_per_10000 = 200;
    p.bytes_per_second = 32 * 1024;
    let mut wire = VirtualWire::new(p, 19);
    let mut a = LinkEndpoint::new(HOST, PeerId(1), SessionId(77));
    let mut b = LinkEndpoint::new(PeerId(1), HOST, SessionId(77));
    let data = (0..32768).map(|n| (n % 251) as u8).collect::<Vec<_>>();
    let digest = blake3_hash(&data);
    let mut upload = BotUpload::new(digest, data.len()).unwrap();
    for (i, chunk) in data.chunks(1020).enumerate() {
        let mut bytes = ((i * 1020) as u32).to_le_bytes().to_vec();
        bytes.extend(chunk);
        a.send_bulk(bytes).unwrap();
    }
    let e = Event {
        id: 1,
        tick: 1,
        kind: EventKind::Paused(true),
    };
    a.send(&Message::Event(e.clone())).unwrap();
    let mut cursor = EventCursor::default();
    let mut event_at = None;
    let mut bulk_at = None;
    for tick in 1..=1200u64 {
        let now = tick * 1_000_000 / HZ;
        let input = wire.receive(now, HOST);
        a.update(now, input, &mut wire).unwrap();
        b.send(&Message::Inputs(vec![])).unwrap();
        let input = wire.receive(now, PeerId(1));
        for message in b.update(now, input, &mut wire).unwrap() {
            if let Message::Event(event) = message {
                assert_eq!(event, e);
                assert!(cursor.accept(&event).unwrap());
                event_at = Some(tick);
            }
        }
        for bytes in std::mem::take(&mut b.bulk_received) {
            let offset = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
            upload.chunk(offset, &bytes[4..]).unwrap();
            if offset + bytes.len() - 4 == data.len() {
                bulk_at = Some(tick);
            }
        }
    }
    assert_eq!(upload.finish().unwrap().1, data);
    assert_eq!(cursor.accepted, 1);
    assert!(
        event_at.unwrap() < bulk_at.unwrap(),
        "bulk transfer blocked the control lane"
    );
    assert!(wire.metrics.dropped > 0);
    assert!(wire.metrics.duplicated > 0);
}
// Test uses the public content hash contract without adding another hashing implementation.
fn blake3_hash(bytes: &[u8]) -> [u8; 32] {
    blake3::hash(bytes).into()
}
#[test]
fn old_session_packets_cannot_enter_lightyear_or_mutate_a_new_session() {
    let mut link = LinkEndpoint::new(PeerId(1), HOST, SessionId(2));
    let mut wire = VirtualWire::new(NetworkProfile::ideal(), 7);
    let mut bytes = 1u128.to_le_bytes().to_vec();
    bytes.extend([255; 32]);
    assert!(
        link.update(
            0,
            vec![Datagram {
                from: HOST,
                to: PeerId(1),
                bytes
            }],
            &mut wire
        )
        .unwrap()
        .is_empty()
    );
    assert_eq!(link.rejected, 1);
}
#[test]
#[cfg(feature = "real-sockets")]
fn actual_udp_loopback_and_lan_query_use_the_same_bounded_packet_contract() {
    use std::{
        net::{Ipv4Addr, SocketAddr},
        time::{Duration, Instant},
    };
    let bind = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    let mut a = UdpIo::bind(HOST, bind).unwrap();
    let mut b = UdpIo::bind(PeerId(1), bind).unwrap();
    a.add_route(PeerId(1), b.address().unwrap()).unwrap();
    b.add_route(HOST, a.address().unwrap()).unwrap();
    let packet = Datagram {
        from: HOST,
        to: PeerId(1),
        bytes: b"actual UDP packet".to_vec(),
    };
    a.send(0, packet.clone()).unwrap();
    let until = Instant::now() + Duration::from_secs(1);
    loop {
        let received = b.receive(0, PeerId(1));
        if !received.is_empty() {
            assert_eq!(received, vec![packet]);
            break;
        }
        assert!(Instant::now() < until, "loopback packet timeout");
        std::thread::yield_now();
    }
    let server = Discovery::bind(0).unwrap();
    let client = Discovery::bind(0).unwrap();
    let target = SocketAddr::from((Ipv4Addr::LOCALHOST, server.address().unwrap().port()));
    client.query(target, 123).unwrap();
    let ad = Advertisement {
        name: "LAN test".into(),
        port: a.address().unwrap().port(),
        occupied: 1,
        capacity: 4,
        nonce: 0,
    };
    let until = Instant::now() + Duration::from_secs(1);
    loop {
        server.answer(&ad, true).unwrap();
        let replies = client.results(123);
        if !replies.is_empty() {
            assert_eq!(replies[0].1.name, ad.name);
            assert_eq!(replies[0].0.port(), ad.port);
            break;
        }
        assert!(Instant::now() < until, "discovery timeout");
        std::thread::yield_now();
    }
    let bytes = Advertisement { nonce: 123, ..ad }.encode().unwrap();
    assert_eq!(Advertisement::decode(&bytes, 124), Err(Error::Conflict));
    for n in 0..bytes.len() {
        assert!(Advertisement::decode(&bytes[..n], 123).is_err());
    }
}
#[test]
fn discovery_payload_validates_nonce_version_utf8_size_and_capacity() {
    let ad = Advertisement {
        name: "LAN test".into(),
        port: 1234,
        occupied: 1,
        capacity: 4,
        nonce: 123,
    };
    let bytes = ad.encode().unwrap();
    assert_eq!(Advertisement::decode(&bytes, 123).unwrap(), ad);
    assert_eq!(Advertisement::decode(&bytes, 124), Err(Error::Conflict));
    for n in 0..bytes.len() {
        assert!(Advertisement::decode(&bytes[..n], 123).is_err());
    }
    let mut invalid = ad;
    invalid.capacity = 5;
    assert_eq!(invalid.encode(), Err(Error::Limit));
    let mut bytes = bytes;
    bytes[4] = 99;
    assert_eq!(Advertisement::decode(&bytes, 123), Err(Error::Version));
}
