use super::*;
use crate::session::{Notice, Request};
use arena_net::session::Player;
use iroh::address_lookup::memory::MemoryLookup;
use std::{net::Ipv4Addr, thread, time::Instant};

fn settings() -> Settings {
    Settings {
        name: "Iroh room".into(),
        arena: "box".into(),
        mode: 3,
        minutes: 5,
        hazards: false,
        weight: None,
    }
}
fn manifest() -> Manifest {
    Manifest {
        protocol: arena_net::PROTOCOL,
        build: [1; 32],
        assets: [2; 32],
        physics: [3; 32],
    }
}
fn loopback() -> Builder {
    Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .relay_mode(iroh::RelayMode::Disabled)
}
fn directory(net: &mainline::Testnet) -> Directory {
    let node = mainline::Dht::builder()
        .bootstrap(&net.bootstrap)
        .bind_address(Ipv4Addr::LOCALHOST)
        .port(0)
        .build()
        .unwrap();
    Directory::Test(node.as_async())
}

fn select_robot(game: &mut NativeGame, size: usize) {
    let slot = game.session.node.local.0 as u8;
    game.node()
        .request(Request::Select(
            Player {
                slot,
                alias: format!("Player{slot}"),
                team: "Team".into(),
                bot_name: format!("BOT{slot}"),
                digest: [0; 32],
                size: 0,
                ready: false,
            },
            vec![slot + 1; size],
        ))
        .unwrap();
}

fn until(
    origin: Instant,
    games: &mut [&mut NativeGame],
    wait: Duration,
    done: impl Fn(&[&mut NativeGame]) -> bool,
) {
    let deadline = Instant::now() + wait;
    loop {
        let now = origin.elapsed().as_micros() as u64;
        for game in games.iter_mut() {
            game.poll(now).unwrap();
        }
        if done(games) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "session timed out: {:?}",
            games
                .iter()
                .map(|g| (
                    g.session.node.local,
                    g.session.node.stage,
                    g.session.node.players.len(),
                    g.publication
                ))
                .collect::<Vec<_>>()
        );
        thread::sleep(Duration::from_millis(3));
    }
}

#[test]
fn code_and_endpoint_join_same_match_and_a_collision_preserves_the_first_host() {
    let net = mainline::Testnet::builder(6).build().unwrap();
    let invite = HostInvite::generate();
    let id = invite.endpoint_id();
    let code = invite.code;
    let origin = Instant::now();
    let wait = Duration::from_secs(35);
    let mut host = NativeGame::start(
        settings(),
        manifest(),
        Role::Host(invite),
        loopback(),
        directory(&net),
    )
    .unwrap();
    // Start dialing before the host has finished its first DHT publication.
    let mut guest = NativeGame::start(
        settings(),
        manifest(),
        Role::Guest(JoinTarget::Code(code)),
        loopback(),
        directory(&net),
    )
    .unwrap();
    until(origin, &mut [&mut host, &mut guest], wait, |g| {
        g[0].publication == Some(Publication::Ready(code)) && g[1].session.node.players.len() == 2
    });
    assert!(guest.session.io.links.contains_key(&id));

    let mut collision = HostInvite::generate();
    collision.code = code;
    let mut other = NativeGame::start(
        settings(),
        manifest(),
        Role::Host(collision),
        loopback(),
        directory(&net),
    )
    .unwrap();
    until(
        origin,
        &mut [&mut host, &mut guest, &mut other],
        wait,
        |g| matches!(g[2].publication, Some(Publication::Ready(new)) if new != code),
    );
    let original = host
        .runtime
        .as_ref()
        .unwrap()
        .block_on(async { room::resolve(&directory(&net).open().await?, code).await })
        .unwrap();
    assert_eq!(original.id, id);
    let lookup = MemoryLookup::new();
    lookup.add_endpoint_info(original);

    // Full-ID dialing goes through iroh address lookup. No room code is available to this guest.
    let mut guest_id = NativeGame::start(
        settings(),
        manifest(),
        Role::Guest(JoinTarget::Endpoint(id)),
        loopback().address_lookup(lookup),
        directory(&net),
    )
    .unwrap();
    let mut games = [&mut host, &mut guest, &mut guest_id];
    until(origin, &mut games, wait, |g| {
        g.iter().all(|g| g.session.node.players.len() == 3)
    });
    for game in &mut games {
        select_robot(game, 16_000);
        // Guests can click as soon as they select a robot, before the host has its bytes.
        game.node().request(Request::Ready(true)).unwrap();
    }
    until(origin, &mut games, wait, |g| {
        g.iter()
            .all(|g| g.session.node.players.values().all(|p| p.ready))
            && g[0].session.node.can_start()
    });
    let slot = games[1].session.node.local.0 as u8;
    games[1].node().request(Request::Ready(false)).unwrap();
    until(origin, &mut games, wait, |g| {
        g.iter().all(|g| {
            g.session
                .node
                .players
                .values()
                .all(|p| p.ready == (p.slot != slot))
        })
    });
    assert!(!games[0].session.node.can_start());
    games[1].node().request(Request::Ready(true)).unwrap();
    until(origin, &mut games, wait, |g| {
        g.iter().all(|g| g.session.node.players[&slot].ready) && g[0].session.node.can_start()
    });
    games[0].node().request(Request::Start).unwrap();
    until(origin, &mut games, wait, |g| {
        g.iter().all(|g| g.session.node.stage == Stage::Loading)
    });
    let epoch = games[0].session.node.epoch;
    for game in &mut games {
        game.node().request(Request::Loaded(epoch)).unwrap();
    }
    until(origin, &mut games, wait, |g| {
        g.iter().all(|g| g.session.node.stage == Stage::Playing)
    });

    games[1]
        .node()
        .request(Request::Wiring(arena_net::session::Wiring {
            epoch,
            generation: 0,
            sequence: 1,
            levels: vec![255],
            edges: vec![1],
            redundant: vec![],
        }))
        .unwrap();
    let slot = games[1].session.node.local.0 as u8;
    until(origin, &mut games, wait, |g| {
        g[0].session.node.inputs.contains_key(&slot)
    });
}

#[test]
#[ignore = "uses public Mainline DHT and the public iroh infrastructure"]
fn public_room_code_and_endpoint_id() {
    let invite = HostInvite::generate();
    let (code, id) = (invite.code, invite.endpoint_id());
    let origin = Instant::now();
    // Keep Mainline UDP enabled but remove all direct paths for game traffic.
    let start = |role| {
        NativeGame::start(
            settings(),
            manifest(),
            role,
            Endpoint::builder(presets::N0).clear_ip_transports(),
            Directory::Public,
        )
        .unwrap()
    };
    let mut host = start(Role::Host(invite));
    let mut by_code = start(Role::Guest(JoinTarget::Code(code)));
    let mut by_id = start(Role::Guest(JoinTarget::Endpoint(id)));
    let mut games = [&mut host, &mut by_code, &mut by_id];
    until(origin, &mut games, Duration::from_secs(115), |g| {
        g.iter().all(|g| g.session.node.players.len() == 3)
    });
    games[1]
        .node()
        .request(Request::Chat("Connected by code".into()))
        .unwrap();
    until(origin, &mut games, Duration::from_secs(10), |g| {
        g[2].session
            .node
            .notices
            .iter()
            .any(|n| matches!(n, Notice::Chat(_, text) if text == "Connected by code"))
    });
    // Only the guest joining by code changes its readiness. Each other lobby observes both edges.
    let slot = games[1].session.node.local.0 as u8;
    select_robot(games[1], 512);
    for ready in [true, false] {
        games[1].node().request(Request::Ready(ready)).unwrap();
        until(origin, &mut games, Duration::from_secs(15), |g| {
            g.iter().all(|g| {
                let p = &g.session.node.players[&slot];
                p.size == 512 && p.ready == ready
            })
        });
    }
}
