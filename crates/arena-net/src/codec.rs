//! Versioned, bounded little-endian protocol. All adapters carry exactly these bytes.
use crate::{input::Input, snapshot::*, *};
use arena_core::{Quat, Vec3};
pub const MAX_MESSAGE: usize = 64 * 1024;
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Inputs(Vec<Input>),
    State(Snapshot),
    Event(Event),
    Pause(bool),
    Goodbye,
    /// Application session protocol, reliable and independent from bulk transfer.
    Control(Vec<u8>),
    /// Authoritative native match frame, sequenced and unreliable.
    GameState(Vec<u8>),
    /// Native controller levels and edge counters, redundant/unreliable.
    Wiring(Vec<u8>),
}
impl Message {
    /// Identity is assigned by the admitted connection, never by the payload.
    pub fn validate_direction(&self, from: PeerId, to: PeerId) -> Result<()> {
        let allowed = from != to
            && match self {
                Self::Inputs(_) => from != HOST && to == HOST,
                Self::State(_) | Self::Event(_) | Self::Pause(_) => from == HOST && to != HOST,
                Self::Control(_) => from == HOST || to == HOST,
                Self::GameState(_) => from == HOST && to != HOST,
                Self::Wiring(_) => from != HOST && to == HOST,
                Self::Goodbye => from == HOST || to == HOST,
            };
        if allowed {
            Ok(())
        } else {
            Err(Error::Unauthorized)
        }
    }
}
struct Writer(Vec<u8>);
impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v)
    }
    fn u16(&mut self, v: u16) {
        self.0.extend(v.to_le_bytes())
    }
    fn u32(&mut self, v: u32) {
        self.0.extend(v.to_le_bytes())
    }
    fn u64(&mut self, v: u64) {
        self.0.extend(v.to_le_bytes())
    }
    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits())
    }
    fn vec3(&mut self, v: Vec3) {
        for x in v.to_array() {
            self.f32(x)
        }
    }
    fn boolean(&mut self, v: bool) {
        self.u8(u8::from(v))
    }
}
struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N]> {
        let a = self.0.get(..N).ok_or(Error::Malformed)?;
        self.0 = &self.0[N..];
        Ok(a.try_into().unwrap())
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take()?))
    }
    fn f32(&mut self) -> Result<f32> {
        let v = f32::from_bits(self.u32()?);
        if v.is_finite() {
            Ok(v)
        } else {
            Err(Error::Malformed)
        }
    }
    fn vec3(&mut self) -> Result<Vec3> {
        Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }
    fn boolean(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::Malformed),
        }
    }
    fn count(&mut self, max: usize) -> Result<usize> {
        let n = self.u16()? as usize;
        if n <= max { Ok(n) } else { Err(Error::Limit) }
    }
}
pub fn encode(message: &Message) -> Result<Vec<u8>> {
    let mut w = Writer(b"OAN1".to_vec());
    w.u16(PROTOCOL);
    match message {
        Message::Inputs(frames) => {
            if frames.len() > 16 || frames.iter().any(|f| !f.valid()) {
                return Err(Error::Limit);
            }
            w.u8(0);
            w.u16(frames.len() as u16);
            for f in frames {
                w.u64(f.tick);
                w.u16(f.throttle as u16);
                w.u16(f.steer as u16);
                w.u32(f.action)
            }
        }
        Message::State(s) => {
            s.validate()?;
            w.u8(1);
            w.u64(s.tick);
            w.u64(s.sim_tick);
            w.boolean(s.paused);
            w.u16(s.robots.len() as u16);
            for r in &s.robots {
                w.u64(r.owner.0);
                w.u32(r.revision);
                w.u32(r.action);
                w.f32(r.energy);
                w.f32(r.air);
                w.f32(r.score);
                w.u16(r.hp.len() as u16);
                for hp in &r.hp {
                    w.f32(*hp)
                }
                w.u16(r.motors.len() as u16);
                for m in &r.motors {
                    w.f32(m.position);
                    w.f32(m.burst_time);
                    w.boolean(m.initialized);
                    w.boolean(m.returning)
                }
            }
            w.u16(s.bodies.len() as u16);
            for b in &s.bodies {
                w.u32(b.id.0);
                w.u64(b.owner.0);
                w.boolean(b.parent.is_some());
                if let Some(p) = b.parent {
                    w.u32(p.0)
                }
                w.vec3(b.position);
                for v in b.rotation.to_array() {
                    w.f32(v)
                }
                w.vec3(b.linear);
                w.vec3(b.angular)
            }
        }
        Message::Event(e) => {
            w.u8(2);
            w.u64(e.id);
            w.u64(e.tick);
            match e.kind {
                EventKind::Damage { victim, part, hp } => {
                    if !hp.is_finite() || hp <= 0. {
                        return Err(Error::Malformed);
                    }
                    w.u8(0);
                    w.u64(victim.0);
                    w.u16(part);
                    w.f32(hp)
                }
                EventKind::Detached { body } => {
                    w.u8(1);
                    w.u32(body.0)
                }
                EventKind::Paused(p) => {
                    w.u8(2);
                    w.boolean(p)
                }
                EventKind::Finished(p) => {
                    w.u8(3);
                    w.boolean(p.is_some());
                    if let Some(p) = p {
                        w.u64(p.0)
                    }
                }
            }
        }
        Message::Pause(p) => {
            w.u8(3);
            w.boolean(*p)
        }
        Message::Goodbye => w.u8(4),
        Message::Control(bytes) | Message::GameState(bytes) | Message::Wiring(bytes) => {
            w.u8(match message {
                Message::Control(_) => 5,
                Message::GameState(_) => 6,
                _ => 7,
            });
            if bytes.len() > MAX_MESSAGE - 11 {
                return Err(Error::Limit);
            }
            w.u32(bytes.len() as u32);
            w.0.extend(bytes);
        }
    }
    if w.0.len() > MAX_MESSAGE {
        return Err(Error::Limit);
    }
    Ok(w.0)
}
pub fn decode(bytes: &[u8]) -> Result<Message> {
    if bytes.len() > MAX_MESSAGE {
        return Err(Error::Limit);
    }
    let mut r = Reader(bytes);
    if r.take::<4>()? != *b"OAN1" {
        return Err(Error::Malformed);
    }
    if r.u16()? != PROTOCOL {
        return Err(Error::Version);
    }
    let message = match r.u8()? {
        0 => {
            let n = r.count(16)?;
            let mut frames = Vec::with_capacity(n);
            for _ in 0..n {
                let f = Input {
                    tick: r.u64()?,
                    throttle: r.u16()? as i16,
                    steer: r.u16()? as i16,
                    action: r.u32()?,
                };
                if !f.valid() {
                    return Err(Error::Malformed);
                }
                frames.push(f)
            }
            Message::Inputs(frames)
        }
        1 => {
            let mut s = Snapshot {
                tick: r.u64()?,
                sim_tick: r.u64()?,
                paused: r.boolean()?,
                ..Default::default()
            };
            for _ in 0..r.count(MAX_PLAYERS)? {
                let owner = PeerId(r.u64()?);
                let revision = r.u32()?;
                let action = r.u32()?;
                let energy = r.f32()?;
                let air = r.f32()?;
                let score = r.f32()?;
                let mut hp = vec![];
                for _ in 0..r.count(64)? {
                    hp.push(r.f32()?)
                }
                let mut motors = vec![];
                for _ in 0..r.count(64)? {
                    motors.push(MotorState {
                        position: r.f32()?,
                        burst_time: r.f32()?,
                        initialized: r.boolean()?,
                        returning: r.boolean()?,
                    })
                }
                s.robots.push(RobotState {
                    owner,
                    hp,
                    energy,
                    air,
                    score,
                    revision,
                    action,
                    motors,
                });
            }
            for _ in 0..r.count(MAX_BODIES)? {
                let id = BodyId(r.u32()?);
                let owner = PeerId(r.u64()?);
                let parent = if r.boolean()? {
                    Some(BodyId(r.u32()?))
                } else {
                    None
                };
                let position = r.vec3()?;
                let rotation = Quat::from_array([r.f32()?, r.f32()?, r.f32()?, r.f32()?]);
                let linear = r.vec3()?;
                let angular = r.vec3()?;
                s.bodies.push(BodyState {
                    id,
                    owner,
                    parent,
                    position,
                    rotation,
                    linear,
                    angular,
                })
            }
            s.validate()?;
            Message::State(s)
        }
        2 => {
            let id = r.u64()?;
            let tick = r.u64()?;
            let kind = match r.u8()? {
                0 => {
                    let victim = PeerId(r.u64()?);
                    let part = r.u16()?;
                    let hp = r.f32()?;
                    if hp <= 0. {
                        return Err(Error::Malformed);
                    }
                    EventKind::Damage { victim, part, hp }
                }
                1 => EventKind::Detached {
                    body: BodyId(r.u32()?),
                },
                2 => EventKind::Paused(r.boolean()?),
                3 => EventKind::Finished(if r.boolean()? {
                    Some(PeerId(r.u64()?))
                } else {
                    None
                }),
                _ => return Err(Error::Malformed),
            };
            Message::Event(Event { id, tick, kind })
        }
        3 => Message::Pause(r.boolean()?),
        4 => Message::Goodbye,
        tag @ 5..=7 => {
            let n = r.u32()? as usize;
            if n > MAX_MESSAGE - 11 || r.0.len() != n {
                return Err(Error::Limit);
            }
            let bytes = r.0.to_vec();
            r.0 = &[];
            match tag {
                5 => Message::Control(bytes),
                6 => Message::GameState(bytes),
                _ => Message::Wiring(bytes),
            }
        }
        _ => return Err(Error::Malformed),
    };
    if !r.0.is_empty() {
        return Err(Error::Malformed);
    }
    Ok(message)
}
