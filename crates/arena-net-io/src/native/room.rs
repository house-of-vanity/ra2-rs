//! A single BEP44 mutable item per spoken room code. The endpoint has its own identity.
use super::{Signal, Signals};
use futures_lite::StreamExt;
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayUrl, Watcher};
use mainline::{Dht, MutableItem, SigningKey, async_dht::AsyncDht};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    net::SocketAddr,
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::time::{sleep, timeout};

const SALT: &[u8] = b"ra2-rs-room-v1";
const QUERY: Duration = Duration::from_secs(12);
const REFRESH: Duration = Duration::from_secs(240);
const TTL: u64 = 15 * 60;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RoomCode(u32);

impl RoomCode {
    pub(super) fn generate() -> Self {
        Self(rand::random_range(0..100_000_000))
    }

    fn signer(self) -> SigningKey {
        let digits = format!("{:08}", self.0);
        SigningKey::from_bytes(&blake3::derive_key(
            "ra2-rs Mainline room signing key v1",
            digits.as_bytes(),
        ))
    }
}

impl fmt::Display for RoomCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:04}", self.0 / 10_000, self.0 % 10_000)
    }
}

impl FromStr for RoomCode {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let digits: String = s.chars().filter(|c| !matches!(c, '-' | ' ')).collect();
        if digits.len() != 8 || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err("Enter an eight-digit room code");
        }
        digits.parse().map(Self).map_err(|_| "Invalid room code")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Publication {
    Publishing(RoomCode),
    Ready(RoomCode),
    Retrying(RoomCode),
}

#[derive(Serialize, Deserialize)]
struct Record {
    version: u8,
    published: u64,
    id: [u8; 32],
    relay: Option<String>,
    addresses: Vec<SocketAddr>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn usable(a: &SocketAddr) -> bool {
    a.port() != 0
        && !a.ip().is_unspecified()
        && !a.ip().is_multicast()
        && !matches!(a, SocketAddr::V4(v) if v.ip().is_broadcast())
}

impl Record {
    fn encode(addr: &EndpointAddr, published: u64) -> Option<Vec<u8>> {
        let record = Self {
            version: 1,
            published,
            id: *addr.id.as_bytes(),
            relay: addr.relay_urls().next().map(ToString::to_string),
            addresses: addr
                .ip_addrs()
                .filter(|a| usable(a))
                .take(8)
                .copied()
                .collect(),
        };
        if record.relay.is_none() && record.addresses.is_empty() {
            return None;
        }
        postcard::to_allocvec(&record)
            .ok()
            .filter(|b| b.len() <= 990)
    }

    fn decode(data: &[u8], current: u64) -> Option<EndpointAddr> {
        if data.len() > 990 {
            return None;
        }
        let (r, tail) = postcard::take_from_bytes::<Self>(data).ok()?;
        if !tail.is_empty()
            || r.version != 1
            || r.addresses.len() > 8
            || r.published > current.saturating_add(120)
            || current.saturating_sub(r.published) >= TTL
        {
            return None;
        }
        let mut addr = EndpointAddr::new(EndpointId::from_bytes(&r.id).ok()?);
        if let Some(relay) = r.relay {
            let url: RelayUrl = relay.parse().ok()?;
            if url.scheme() != "https" || url.host_str().is_none() {
                return None;
            }
            addr = addr.with_relay_url(url);
        }
        for socket in r.addresses.into_iter().filter(usable) {
            addr = addr.with_ip_addr(socket);
        }
        (!addr.addrs.is_empty()).then_some(addr)
    }
}

#[derive(Clone, Default)]
pub(super) enum Directory {
    #[default]
    Public,
    #[cfg(all(test, feature = "real-sockets"))]
    Test(AsyncDht),
}

impl Directory {
    pub(super) async fn open(&self) -> Result<AsyncDht, String> {
        #[cfg(all(test, feature = "real-sockets"))]
        if let Self::Test(node) = self {
            return Ok(node.clone());
        }
        tokio::task::spawn_blocking(|| Dht::builder().port(0).build().map(Dht::as_async))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())
    }
}

enum Put {
    Stored,
    Occupied,
    Retry,
}

async fn put(dht: &AsyncDht, code: RoomCode, addr: &EndpointAddr) -> Put {
    let Some(value) = Record::encode(addr, now()) else {
        return Put::Retry;
    };
    let signer = code.signer();
    let key = signer.verifying_key().to_bytes();
    let Ok(old) = timeout(QUERY, dht.get_mutable_most_recent(&key, Some(SALT))).await else {
        return Put::Retry;
    };
    if old
        .as_ref()
        .and_then(|i| Record::decode(i.value(), now()))
        .is_some_and(|other| other.id != addr.id)
    {
        return Put::Occupied;
    }
    let cas = old.as_ref().map(MutableItem::seq);
    let Some(seq) = cas.unwrap_or(0).checked_add(1) else {
        return Put::Occupied;
    };
    let item = MutableItem::new(signer, &value, seq, Some(SALT));
    match timeout(QUERY, dht.put_mutable(item, cas)).await {
        Ok(Ok(out)) if out.stored_at > 0 => Put::Stored,
        _ => Put::Retry,
    }
}

pub(super) async fn publish(
    directory: Directory,
    endpoint: Endpoint,
    mut code: RoomCode,
    events: Signals,
) {
    let mut addresses = endpoint.watch_addr();
    let mut dht = None;
    let mut last = None;
    loop {
        if dht.is_none() {
            dht = directory.open().await.ok();
        }
        let addr = addresses.get();
        let result = match &dht {
            Some(dht) => put(dht, code, &addr).await,
            None => Put::Retry,
        };
        let (status, delay) = match result {
            Put::Stored => (Publication::Ready(code), REFRESH),
            Put::Occupied => {
                code = RoomCode::generate();
                (Publication::Publishing(code), Duration::ZERO)
            }
            Put::Retry => (Publication::Retrying(code), Duration::from_secs(5)),
        };
        if last != Some(status) {
            if events.send(Signal::Publication(status)).await.is_err() {
                return;
            }
            last = Some(status);
        }
        tokio::select! {
            _ = sleep(delay) => {}
            changed = addresses.updated() => if changed.is_err() { return; }
        }
    }
}

pub(super) async fn resolve(dht: &AsyncDht, code: RoomCode) -> Result<EndpointAddr, String> {
    let key = code.signer().verifying_key().to_bytes();
    let lookup = async {
        loop {
            let mut records = dht.get_mutable(&key, Some(SALT), None);
            let round = async {
                while let Some(item) = records.next().await {
                    if let Some(addr) = Record::decode(item.value(), now()) {
                        return Some(addr);
                    }
                }
                None
            };
            if let Ok(Some(addr)) = timeout(QUERY, round).await {
                return Ok(addr);
            }
            sleep(Duration::from_secs(2)).await;
        }
    };
    timeout(Duration::from_secs(65), lookup).await
        .map_err(|_| format!("Room {code} could not be found on Mainline. Check the code or use the host's EndpointID."))?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codes_and_freshness() {
        let code: RoomCode = "00273106".parse().unwrap();
        assert_eq!(code.to_string(), "0027-3106");
        assert_eq!(code, "0027 3106".parse().unwrap());
        assert!("0027-310".parse::<RoomCode>().is_err());
        let addr = EndpointAddr::new(iroh::SecretKey::generate().public())
            .with_ip_addr("127.0.0.1:1234".parse().unwrap());
        let record = Record::encode(&addr, 1000).unwrap();
        assert_eq!(Record::decode(&record, 1001), Some(addr));
        assert!(Record::decode(&record, 1000 + TTL).is_none());
        assert!(Record::decode(&record, 850).is_none());
    }
}
