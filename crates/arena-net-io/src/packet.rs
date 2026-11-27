//! Lightyear packet/channel adapter. The byte IO below it has no Lightyear dependency.
use arena_net::Result;
use arena_net::{
    codec::{self, Message},
    transport::*,
    *,
};
use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_time::{Real, Time};
use lightyear_core::prelude::LocalTimeline;
use lightyear_link::{Link, LinkMtu, Linked};
use lightyear_transport::{
    plugin::{PacketAcked, TransportPlugin},
    prelude::*,
};
use std::time::Duration;
pub struct InputLane;
pub struct SnapshotLane;
pub struct EventLane;
pub struct BulkLane;
pub struct LinkEndpoint {
    app: App,
    entity: Entity,
    pub local: PeerId,
    pub remote: PeerId,
    pub session: SessionId,
    elapsed: u64,
    pub sent_bytes: u64,
    pub received_bytes: u64,
    pub bulk_received: Vec<Vec<u8>>,
    pub rejected: u64,
}
impl LinkEndpoint {
    pub fn new(local: PeerId, remote: PeerId, session: SessionId) -> Self {
        Self::with_mtu(local, remote, session, MTU)
    }
    pub(crate) fn with_mtu(local: PeerId, remote: PeerId, session: SessionId, mtu: usize) -> Self {
        let mut app = App::new();
        app.init_resource::<Time<Real>>()
            .init_resource::<LocalTimeline>()
            .add_plugins(TransportPlugin);
        app.add_observer(|ack: On<PacketAcked>, mut links: Query<&mut Link>| {
            if let Ok(mut link) = links.get_mut(ack.entity) {
                let sample = ack
                    .rtt_sample
                    .clamp(Duration::from_millis(1), Duration::from_secs(2));
                let old = link.stats.rtt;
                link.stats.rtt = old.mul_f32(0.875) + sample.mul_f32(0.125);
                link.stats.jitter =
                    link.stats.jitter.mul_f32(0.75) + old.abs_diff(sample).mul_f32(0.25);
            }
        });
        app.add_channel::<InputLane>(ChannelSettings {
            mode: ChannelMode::UnorderedUnreliable,
            priority: 10.,
            retry_unsent_messages: false,
            ..Default::default()
        });
        app.add_channel::<SnapshotLane>(ChannelSettings {
            mode: ChannelMode::SequencedUnreliable,
            priority: 5.,
            retry_unsent_messages: false,
            ..Default::default()
        });
        let reliable = ReliableSettings {
            rtt_resend_min_delay: Duration::from_millis(50),
            ..Default::default()
        };
        app.add_channel::<EventLane>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(reliable),
            priority: 8.,
            ..Default::default()
        });
        app.add_channel::<BulkLane>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(reliable),
            priority: 0.5,
            ..Default::default()
        });
        app.finish();
        app.cleanup();
        let mut transport = Transport::default();
        let registry = app.world().resource::<ChannelRegistry>();
        for channel in 0..4 {
            match channel {
                0 => {
                    transport.add_channel_send_from_registry::<InputLane>(registry);
                    transport.add_channel_receive_from_registry::<InputLane>(registry);
                }
                1 => {
                    transport.add_channel_send_from_registry::<SnapshotLane>(registry);
                    transport.add_channel_receive_from_registry::<SnapshotLane>(registry);
                }
                2 => {
                    transport.add_channel_send_from_registry::<EventLane>(registry);
                    transport.add_channel_receive_from_registry::<EventLane>(registry);
                }
                _ => {
                    transport.add_channel_send_from_registry::<BulkLane>(registry);
                    transport.add_channel_receive_from_registry::<BulkLane>(registry);
                }
            }
        }
        let mut link = Link::default().with_mtu(LinkMtu::new(mtu - 16));
        link.stats.rtt = Duration::from_millis(100);
        let entity = app.world_mut().spawn((link, Linked, transport)).id();
        Self {
            app,
            entity,
            local,
            remote,
            session,
            elapsed: 0,
            sent_bytes: 0,
            received_bytes: 0,
            bulk_received: vec![],
            rejected: 0,
        }
    }
    pub fn send(&mut self, message: &Message) -> Result<()> {
        message.validate_direction(self.local, self.remote)?;
        let bytes = codec::encode(message)?.into();
        let t = self.app.world().get::<Transport>(self.entity).unwrap();
        match message {
            Message::Inputs(_) | Message::Wiring(_) => t.send::<InputLane>(bytes),
            Message::State(_) | Message::GameState(_) => t.send::<SnapshotLane>(bytes),
            _ => t.send::<EventLane>(bytes),
        }
        .map_err(|_| Error::Limit)
    }
    pub fn send_bulk(&mut self, bytes: Vec<u8>) -> Result<()> {
        if bytes.len() > 1024 {
            return Err(Error::Limit);
        }
        self.app
            .world()
            .get::<Transport>(self.entity)
            .unwrap()
            .send::<BulkLane>(bytes.into())
            .map_err(|_| Error::Limit)
    }
    pub fn update(
        &mut self,
        now: u64,
        packets: Vec<Datagram>,
        wire: &mut impl DatagramIo,
    ) -> Result<Vec<Message>> {
        if now < self.elapsed {
            return Err(Error::Conflict);
        }
        self.app
            .world_mut()
            .resource_mut::<Time<Real>>()
            .advance_by(Duration::from_micros(now - self.elapsed));
        self.elapsed = now;
        self.app
            .world_mut()
            .resource_mut::<LocalTimeline>()
            .apply_delta(1);
        for p in packets {
            if p.from != self.remote || p.to != self.local || p.bytes.len() > MTU {
                return Err(Error::Unauthorized);
            }
            if p.bytes.len() < 16 || p.bytes[..16] != self.session.0.to_le_bytes() {
                self.rejected += 1;
                continue;
            }
            self.received_bytes += p.bytes.len() as u64;
            self.app
                .world_mut()
                .get_mut::<Link>(self.entity)
                .unwrap()
                .recv
                .push_raw(bytes::BytesMut::from(&p.bytes[16..]));
        }
        self.app.update();
        let outgoing = self
            .app
            .world_mut()
            .get_mut::<Link>(self.entity)
            .unwrap()
            .send
            .drain()
            .map(|b| b.to_vec())
            .collect::<Vec<_>>();
        for bytes in outgoing {
            let mut framed = self.session.0.to_le_bytes().to_vec();
            framed.extend(bytes);
            self.sent_bytes += framed.len() as u64;
            wire.send(
                now,
                Datagram {
                    from: self.local,
                    to: self.remote,
                    bytes: framed,
                },
            )?;
        }
        let bulk = *self
            .app
            .world()
            .resource::<ChannelRegistry>()
            .get_net_from_kind(&lightyear_transport::channel::ChannelKind::of::<BulkLane>())
            .unwrap();
        let mut t = self
            .app
            .world_mut()
            .get_mut::<Transport>(self.entity)
            .unwrap();
        let mut messages = vec![];
        while let Some((_, bytes, _)) = t.channel_receive_mut(bulk).unwrap().read_message() {
            self.bulk_received.push(bytes.to_vec());
        }
        for receiver in t.channel_receives_mut() {
            while let Some((_, bytes, _)) = receiver.read_message() {
                let message = codec::decode(&bytes)?;
                message.validate_direction(self.remote, self.local)?;
                messages.push(message);
            }
        }
        Ok(messages)
    }
}
