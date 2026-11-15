//! Deterministic virtual-time experiments through the real Lightyear packet layer.
use crate::{link::LinkEndpoint, physics::Physics};
use arena_net::{
    codec::Message,
    input::Input,
    prediction::{Host, Predictor, Simulation},
    snapshot::*,
    transport::*,
    *,
};
use std::{collections::VecDeque, time::Instant};
pub struct Client {
    pub prediction: Predictor<Physics>,
    pub link: LinkEndpoint,
    pub interpolation: Interpolation,
    pub events: EventCursor,
    pub last: Snapshot,
}
#[derive(Debug, Clone)]
pub struct Trace {
    pub arrival_tick: Tick,
    pub authoritative_tick: Tick,
    pub peer: PeerId,
    pub position_error_m: f32,
    pub angle_error_deg: f32,
    pub replayed_ticks: u64,
    pub cpu_us: f64,
    pub topology_change: bool,
    pub phase_change: bool,
}
impl Trace {
    pub fn csv_header() -> &'static str {
        "arrival_tick,authoritative_tick,peer,position_error_m,angle_error_deg,replayed_ticks,cpu_us,topology_change,phase_change"
    }
    pub fn csv(&self) -> String {
        format!(
            "{},{},{},{:.7},{:.5},{},{:.2},{},{}",
            self.arrival_tick,
            self.authoritative_tick,
            self.peer.0,
            self.position_error_m,
            self.angle_error_deg,
            self.replayed_ticks,
            self.cpu_us,
            self.topology_change,
            self.phase_change
        )
    }
}
#[derive(Debug, Clone)]
pub struct Report {
    pub profile: String,
    pub players: usize,
    pub seed: u64,
    pub rtt_ms: u32,
    pub jitter_ms: u32,
    pub loss_per_10000: u32,
    pub samples: usize,
    pub correction_p95_m: f32,
    pub correction_max_m: f32,
    pub angle_p95_deg: f32,
    pub interpolation_p95_m: f32,
    pub interpolation_max_m: f32,
    pub underrun_ratio: f64,
    pub replayed: u64,
    pub hard_resyncs: u64,
    pub wire: WireMetrics,
    pub host_kib_per_s: f64,
    pub cpu_ms: f64,
    pub reconciliation_cpu_ms: f64,
    pub contacts: u64,
    pub shots: u64,
    pub breaks: u64,
    pub events_sent: u64,
    pub events_received: u64,
    pub late_inputs: u64,
    pub max_final_position_error_m: f32,
    pub final_state_equal: bool,
    pub failures: Vec<String>,
    pub upstream_ms: u32,
    pub downstream_ms: u32,
    pub input_lead_ms: f64,
    pub render_delay_ms: f64,
    pub trace: Vec<Trace>,
    pub motion_max_m: f32,
    pub topology_max_m: f32,
    pub topology_corrections: u64,
    pub phase_max_m: f32,
    pub phase_corrections: u64,
}
impl Report {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
    pub fn csv_header() -> &'static str {
        "profile,players,seed,rtt_ms,jitter_ms,loss_per_10000,samples,correction_p95_m,correction_max_m,angle_p95_deg,interpolation_p95_m,interpolation_max_m,underrun_ratio,replayed_ticks,hard_resyncs,packets,bytes,dropped,peak_queue,host_kib_per_s,cpu_ms,reconciliation_cpu_ms,contacts,shots,breaks,events_sent,events_received,late_inputs,final_position_error_m,final_state_equal,passed,upstream_ms,downstream_ms,input_lead_ms,render_delay_ms,motion_max_m,topology_max_m,topology_corrections,phase_max_m,phase_corrections"
    }
    pub fn csv(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{:.6},{:.6},{:.4},{:.6},{:.6},{:.6},{},{},{},{},{},{},{:.3},{:.3},{:.3},{},{},{},{},{},{},{:.6},{},{},{},{},{:.3},{:.3},{:.6},{:.6},{},{:.6},{}",
            self.profile,
            self.players,
            self.seed,
            self.rtt_ms,
            self.jitter_ms,
            self.loss_per_10000,
            self.samples,
            self.correction_p95_m,
            self.correction_max_m,
            self.angle_p95_deg,
            self.interpolation_p95_m,
            self.interpolation_max_m,
            self.underrun_ratio,
            self.replayed,
            self.hard_resyncs,
            self.wire.packets,
            self.wire.bytes,
            self.wire.dropped,
            self.wire.peak_queue,
            self.host_kib_per_s,
            self.cpu_ms,
            self.reconciliation_cpu_ms,
            self.contacts,
            self.shots,
            self.breaks,
            self.events_sent,
            self.events_received,
            self.late_inputs,
            self.max_final_position_error_m,
            self.final_state_equal,
            self.passed(),
            self.upstream_ms,
            self.downstream_ms,
            self.input_lead_ms,
            self.render_delay_ms,
            self.motion_max_m,
            self.topology_max_m,
            self.topology_corrections,
            self.phase_max_m,
            self.phase_corrections
        )
    }
}
fn percentile(values: &[f32], q: f32) -> f32 {
    if values.is_empty() {
        return 0.;
    }
    let mut v = values.to_vec();
    v.sort_by(f32::total_cmp);
    v[((v.len() - 1) as f32 * q).ceil() as usize]
}
fn script(owner: PeerId, tick: Tick) -> Input {
    let sec = tick / HZ;
    let throttle = match sec {
        0 => 0,
        1..=3 => {
            if owner.0.is_multiple_of(2) {
                32767
            } else {
                -32767
            }
        }
        4..=5 => 32767,
        6..=7 => -32767,
        8..=9 => 32767,
        _ => 0,
    };
    let steer = if (3..=5).contains(&sec) {
        if owner.0.is_multiple_of(2) {
            16000
        } else {
            -16000
        }
    } else {
        0
    };
    Input {
        tick,
        throttle,
        steer,
        action: if tick < HZ {
            0
        } else {
            ((tick - HZ) / (HZ / 2) + 1).min(16) as u32
        },
    }
}
pub fn profiles() -> Vec<NetworkProfile> {
    let mut profiles = vec![NetworkProfile::ideal()];
    for rtt in [25, 50, 100, 150, 200] {
        let mut p = NetworkProfile::ideal();
        p.name = format!("rtt-{rtt}");
        p.upstream_ms = rtt / 2;
        p.downstream_ms = rtt - rtt / 2;
        profiles.push(p)
    }
    let mut p = NetworkProfile::ideal();
    p.name = "rtt-200-loss".into();
    p.upstream_ms = 100;
    p.downstream_ms = 100;
    p.jitter_ms = 20;
    p.loss_per_10000 = 300;
    p.duplicate_per_10000 = 100;
    profiles.push(p);
    let mut p = NetworkProfile::ideal();
    p.name = "oneway-200".into();
    p.upstream_ms = 200;
    p.downstream_ms = 200;
    p.jitter_ms = 20;
    profiles.push(p);
    let mut p = NetworkProfile::ideal();
    p.name = "asymmetric".into();
    p.upstream_ms = 20;
    p.downstream_ms = 180;
    p.jitter_ms = 15;
    profiles.push(p);
    let mut p = NetworkProfile::ideal();
    p.name = "limited-512kbit".into();
    p.upstream_ms = 50;
    p.downstream_ms = 50;
    p.bytes_per_second = 64 * 1024;
    p.loss_per_10000 = 100;
    profiles.push(p);
    let mut p = NetworkProfile::ideal();
    p.name = "outage-400ms".into();
    p.upstream_ms = 50;
    p.downstream_ms = 50;
    p.blackout = Some((4_000_000, 4_400_000));
    profiles.push(p);
    profiles
}

pub fn run(profile: NetworkProfile, players: usize, seed: u64) -> Result<Report> {
    if !(2..=MAX_PLAYERS).contains(&players) {
        return Err(Error::Limit);
    }
    let started = Instant::now();
    let mut replay_cpu = 0.;
    let mut trace = vec![];
    let mut wire = VirtualWire::new(profile.clone(), seed);
    let mut host = Host::new(
        Physics::new(players),
        (0..players).map(|i| PeerId(i as u64)),
    );
    let mut host_links = (1..players)
        .map(|i| LinkEndpoint::new(HOST, PeerId(i as u64), SessionId(seed as u128)))
        .collect::<Vec<_>>();
    let initial = host.sim.snapshot();
    let mut clients = (1..players)
        .map(|i| Client {
            prediction: Predictor::new(Physics::new(players), PeerId(i as u64)),
            link: LinkEndpoint::new(PeerId(i as u64), HOST, SessionId(seed as u128)),
            interpolation: Interpolation::with_joints(host.sim.joint_frames()).unwrap(),
            events: Default::default(),
            last: initial.clone(),
        })
        .collect::<Vec<_>>();
    let mut history = VecDeque::from([initial]);
    let mut interpolation_errors = vec![];
    let mut rendered = 0u64;
    let mut measured_underruns = 0u64;
    let total = 12 * HZ;
    let mut event_count = 0;
    for wall_tick in 1..=total {
        let now = wall_tick * 1_000_000 / HZ;
        for c in &mut clients {
            let mut messages =
                c.link
                    .update(now, wire.receive(now, c.prediction.owner), &mut wire)?;
            messages.sort_by_key(|m| if matches!(m, Message::Event(_)) { 0 } else { 1 });
            for message in messages {
                match message {
                    Message::State(s) => {
                        let before = Instant::now();
                        let replay_before = c.prediction.metrics.replayed_ticks;
                        let accepted = c.prediction.reconcile(&s)?;
                        let cpu_us = before.elapsed().as_secs_f64() * 1_000_000.;
                        replay_cpu += cpu_us / 1000.;
                        if accepted {
                            trace.push(Trace {
                                arrival_tick: wall_tick,
                                authoritative_tick: s.tick,
                                peer: c.prediction.owner,
                                position_error_m: c
                                    .prediction
                                    .metrics
                                    .corrections
                                    .back()
                                    .copied()
                                    .unwrap_or(0.),
                                angle_error_deg: c
                                    .prediction
                                    .metrics
                                    .angles
                                    .back()
                                    .copied()
                                    .unwrap_or(0.)
                                    .to_degrees(),
                                replayed_ticks: c.prediction.metrics.replayed_ticks - replay_before,
                                cpu_us,
                                topology_change: c.prediction.metrics.last_topology_change,
                                phase_change: c.prediction.metrics.last_phase_change,
                            });
                        }
                        c.interpolation.push(s.clone())?;
                        c.last = s;
                    }
                    Message::Event(e) => {
                        if c.events.accept(&e)?
                            && let EventKind::Paused(paused) = e.kind
                        {
                            c.prediction.control_pause(e.tick, paused)?;
                        }
                    }
                    _ => return Err(Error::Unauthorized),
                }
            }
            let target = wall_tick + profile.lead_ticks();
            while c.prediction.sim.snapshot().tick < target {
                let tick = c.prediction.sim.snapshot().tick + 1;
                c.prediction.advance(script(c.prediction.owner, tick))?;
            }
            if wall_tick % 2 == 0 {
                c.link
                    .send(&Message::Inputs(c.prediction.recent_inputs(12)))?;
            }
            // Compare remote interpolation with the host history at its render timestamp.
            let render = wall_tick.saturating_sub(profile.render_delay_ticks());
            let old = c.interpolation.underruns;
            if let Some(shown) = c.interpolation.sample(render as f64)
                && let Some(truth) = history.iter().find(|s| s.tick == render)
                && wall_tick > HZ
            {
                rendered += 1;
                measured_underruns += c.interpolation.underruns - old;
                for b in shown
                    .bodies
                    .iter()
                    .filter(|b| b.owner != c.prediction.owner)
                {
                    if let Some(actual) = truth.body(b.id) {
                        interpolation_errors.push(b.position.distance(actual.position));
                    }
                }
            }
        }
        let all = wire.receive(now, HOST);
        for link in &mut host_links {
            let packets = all
                .iter()
                .filter(|p| p.from == link.remote)
                .cloned()
                .collect();
            for message in link.update(now, packets, &mut wire)? {
                match message {
                    Message::Inputs(frames) => host.receive(link.remote, &frames)?,
                    _ => return Err(Error::Unauthorized),
                }
            }
        }
        // Host controls use the same tick-addressed input path, without socket latency.
        host.receive(HOST, &[script(HOST, wall_tick)])?;
        if wall_tick == 6 * HZ {
            host.pause(HOST, true)?;
        }
        if wall_tick == 7 * HZ {
            host.pause(HOST, false)?;
        }
        host.advance();
        let snapshot = host.sim.snapshot();
        history.push_back(snapshot.clone());
        while history.len() > HISTORY {
            history.pop_front();
        }
        if wall_tick % 4 == 0 {
            for link in &mut host_links {
                link.send(&Message::State(snapshot.clone()))?;
            }
        }
        for event in host.events.drain(..) {
            event_count += 1;
            for link in &mut host_links {
                link.send(&Message::Event(event.clone()))?;
            }
        }
    }
    // A stable final authoritative checkpoint must converge without any live input or physics.
    host.sim.set_paused(true);
    host.advance();
    let final_state = host.sim.snapshot();
    let mut failures = vec![];
    for tail in 1..=3 * HZ {
        let now = (total + tail) * 1_000_000 / HZ;
        if tail % 12 == 1 {
            for link in &mut host_links {
                link.send(&Message::State(final_state.clone()))?;
            }
        }
        for c in &mut clients {
            for m in c
                .link
                .update(now, wire.receive(now, c.prediction.owner), &mut wire)?
            {
                match m {
                    Message::State(s) => {
                        c.prediction.reconcile(&s)?;
                        c.last = s;
                    }
                    Message::Event(e) => {
                        c.events.accept(&e)?;
                    }
                    _ => return Err(Error::Unauthorized),
                }
            }
        }
        let all = wire.receive(now, HOST);
        for link in &mut host_links {
            let p = all
                .iter()
                .filter(|p| p.from == link.remote)
                .cloned()
                .collect();
            let _ = link.update(now, p, &mut wire)?;
        }
    }
    let mut corrections = vec![];
    let mut angles = vec![];
    let mut replayed = 0;
    let mut hard = 0;
    let mut received = 0;
    let mut final_equal = true;
    let mut final_error = 0f32;
    for c in &clients {
        corrections.extend(&c.prediction.metrics.corrections);
        angles.extend(&c.prediction.metrics.angles);
        replayed += c.prediction.metrics.replayed_ticks;
        hard += c.prediction.metrics.hard_resyncs;
        received += c.events.accepted;
        let predicted = c.prediction.sim.snapshot();
        final_equal &= c.last == final_state
            && predicted.bodies == final_state.bodies
            && predicted.robots == final_state.robots
            && predicted.paused;
        for b in &predicted.bodies {
            final_error = final_error.max(
                b.position
                    .distance(final_state.body(b.id).unwrap().position),
            );
        }
        if c.events.accepted != event_count {
            failures.push(format!(
                "peer {} missed authoritative events: {} / {event_count}",
                c.prediction.owner.0, c.events.accepted
            ));
        }
    }
    if !final_equal {
        failures.push("final snapshot did not converge".into());
    }
    if corrections.is_empty() {
        failures.push("no aligned prediction samples".into());
    }
    if host.sim.contacts == 0 {
        failures.push("scenario produced no physical collision".into());
    }
    if host.sim.shots == 0 {
        failures.push("scenario produced no actuator activation".into());
    }
    if corrections
        .iter()
        .chain(&interpolation_errors)
        .any(|x| !x.is_finite())
    {
        failures.push("non-finite error".into());
    }
    if hard > 0 {
        failures.push(format!("{hard} unplanned hard resyncs"));
    }
    let p95 = percentile(&corrections, 0.95);
    let max = percentile(&corrections, 1.);
    // Initial engineering budgets, deliberately separate from correctness gates.
    if p95 > 0.05 {
        failures.push(format!("prediction p95 {p95:.3}m exceeds 0.05m budget"));
    }
    let motion_max_m = clients
        .iter()
        .map(|c| c.prediction.metrics.motion_max)
        .fold(0_f32, f32::max);
    let topology_max_m = clients
        .iter()
        .map(|c| c.prediction.metrics.topology_max)
        .fold(0_f32, f32::max);
    let topology_corrections = clients
        .iter()
        .map(|c| c.prediction.metrics.topology_changes)
        .sum();
    let phase_max_m = clients
        .iter()
        .map(|c| c.prediction.metrics.phase_max)
        .fold(0_f32, f32::max);
    let phase_corrections = clients
        .iter()
        .map(|c| c.prediction.metrics.phase_changes)
        .sum();
    if phase_max_m > 1. {
        failures.push(format!(
            "phase change correction {phase_max_m:.3}m exceeds 1m budget"
        ));
    }
    if motion_max_m > 0.5 {
        failures.push(format!(
            "motion prediction max {motion_max_m:.3}m exceeds 0.5m budget"
        ));
    }
    if topology_max_m > 1. {
        failures.push(format!(
            "topology prediction max {topology_max_m:.3}m exceeds 1m budget"
        ));
    }
    if percentile(&angles, 0.95).to_degrees() > 5. {
        failures.push("prediction angle p95 exceeds 5 degrees".into());
    }
    if percentile(&interpolation_errors, 0.95) > 0.05 {
        failures.push("interpolation p95 exceeds 0.05m".into());
    }
    if measured_underruns as f64 / rendered.max(1) as f64 > 0.06 {
        failures.push("render buffer underruns exceed 6%".into());
    }
    let host_bytes = host_links.iter().map(|l| l.sent_bytes).sum::<u64>();
    Ok(Report {
        profile: profile.name.clone(),
        players,
        seed,
        rtt_ms: profile.rtt_ms(),
        jitter_ms: profile.jitter_ms,
        loss_per_10000: profile.loss_per_10000,
        samples: corrections.len(),
        correction_p95_m: p95,
        correction_max_m: max,
        angle_p95_deg: percentile(&angles, 0.95).to_degrees(),
        interpolation_p95_m: percentile(&interpolation_errors, 0.95),
        interpolation_max_m: percentile(&interpolation_errors, 1.),
        underrun_ratio: measured_underruns as f64 / rendered.max(1) as f64,
        replayed,
        hard_resyncs: hard,
        wire: wire.metrics,
        host_kib_per_s: host_bytes as f64 / 1024. / 15.,
        cpu_ms: started.elapsed().as_secs_f64() * 1000.,
        reconciliation_cpu_ms: replay_cpu,
        contacts: host.sim.contacts,
        shots: host.sim.shots,
        breaks: host.sim.breaks,
        events_sent: event_count,
        events_received: received,
        late_inputs: host.inputs.values().map(|b| b.late).sum(),
        max_final_position_error_m: final_error,
        final_state_equal: final_equal,
        failures,
        upstream_ms: profile.upstream_ms,
        downstream_ms: profile.downstream_ms,
        input_lead_ms: profile.lead_ticks() as f64 * 1000. / HZ as f64,
        render_delay_ms: profile.render_delay_ticks() as f64 * 1000. / HZ as f64,
        trace,
        motion_max_m,
        topology_max_m,
        topology_corrections,
        phase_max_m,
        phase_corrections,
    })
}
