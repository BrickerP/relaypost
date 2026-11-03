use anyhow::{bail, ensure, Context, Result};
use relay_core::{DirectOfferArtifact, PeerCard, ReceiptArtifact};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream};
use std::time::{Duration, Instant};

pub(crate) const MAX_JSON: u64 = 140 * 1024;
pub(crate) const MAX_BUNDLE: u64 = relay_core::MAX_DIRECT_BUNDLE;
pub(crate) const IDLE: Duration = Duration::from_secs(30);
pub(crate) const DEADLINE: Duration = Duration::from_secs(600);
const MAGIC: &[u8; 8] = b"RLYPST1\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Kind {
    Card = 1,
    Pair = 2,
    PairStatus = 3,
    Offer = 4,
    OfferStatus = 5,
    Bundle = 6,
    Receipt = 7,
    Reply = 128,
}

impl TryFrom<u8> for Kind {
    type Error = anyhow::Error;
    fn try_from(value: u8) -> Result<Self> {
        Ok(match value {
            1 => Self::Card,
            2 => Self::Pair,
            3 => Self::PairStatus,
            4 => Self::Offer,
            5 => Self::OfferStatus,
            6 => Self::Bundle,
            7 => Self::Receipt,
            128 => Self::Reply,
            _ => bail!("Unknown direct-transfer request"),
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Empty {}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PairRequest {
    pub card: PeerCard,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PairStatusRequest {
    pub request_id: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParcelRequest {
    pub parcel_id: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) enum Decision {
    Waiting,
    Accepted,
    Declined,
    Stored,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Reply {
    Card {
        card: PeerCard,
    },
    Pairing {
        request_id: String,
        decision: Decision,
    },
    Offer {
        parcel_id: String,
        decision: Decision,
    },
    Stored {
        parcel_id: String,
    },
    Receipt {
        artifact: ReceiptArtifact,
    },
    Rejected {
        message: String,
    },
}

pub(crate) fn is_local(address: Ipv4Addr) -> bool {
    address.is_loopback() || address.is_private() || address.is_link_local()
}

pub(crate) fn connect(address: SocketAddrV4) -> Result<Stream> {
    ensure!(
        is_local(*address.ip()),
        "Only local-network IPv4 addresses are supported"
    );
    let stream = TcpStream::connect_timeout(&SocketAddr::V4(address), Duration::from_secs(2))?;
    stream.set_read_timeout(Some(IDLE))?;
    stream.set_write_timeout(Some(IDLE))?;
    stream.set_nodelay(true)?;
    Stream::new(stream)
}

pub(crate) struct Stream {
    pub socket: TcpStream,
    deadline: Instant,
}

impl Stream {
    pub fn new(socket: TcpStream) -> Result<Self> {
        socket.set_nodelay(true)?;
        Ok(Self {
            socket,
            deadline: Instant::now() + DEADLINE,
        })
    }
    fn remaining(&self) -> std::io::Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .map(|left| left.min(IDLE))
            .filter(|left| !left.is_zero())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Network operation deadline exceeded",
                )
            })
    }
}
impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.socket.set_read_timeout(Some(self.remaining()?))?;
        self.socket.read(buf)
    }
}
impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.socket.set_write_timeout(Some(self.remaining()?))?;
        self.socket.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.socket.flush()
    }
}

pub(crate) fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn read_header(stream: &mut impl Read) -> Result<(Kind, u64)> {
    let mut prefix = [0_u8; 17];
    stream.read_exact(&mut prefix)?;
    ensure!(
        &prefix[..8] == MAGIC,
        "Unsupported direct-transfer protocol"
    );
    let kind = Kind::try_from(prefix[8])?;
    let length = u64::from_be_bytes(prefix[9..17].try_into()?);
    ensure!(
        length
            <= if kind == Kind::Bundle {
                MAX_BUNDLE + 64
            } else {
                MAX_JSON
            },
        "Direct-transfer frame exceeds the limit"
    );
    Ok((kind, length))
}

pub(crate) fn write_header(stream: &mut impl Write, kind: Kind, length: u64) -> Result<()> {
    ensure!(
        length
            <= if kind == Kind::Bundle {
                MAX_BUNDLE + 64
            } else {
                MAX_JSON
            },
        "Direct-transfer frame exceeds the limit"
    );
    stream.write_all(MAGIC)?;
    stream.write_all(&[kind as u8])?;
    stream.write_all(&length.to_be_bytes())?;
    Ok(())
}

pub(crate) fn read_json<T: DeserializeOwned>(stream: &mut impl Read, length: u64) -> Result<T> {
    ensure!(length <= MAX_JSON, "JSON request exceeds the limit");
    let mut bytes = vec![0; usize::try_from(length)?];
    stream.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).context("Invalid direct-transfer request")
}

pub(crate) fn write_json<T: Serialize>(
    stream: &mut impl Write,
    kind: Kind,
    value: &T,
) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    write_header(stream, kind, bytes.len() as u64)?;
    stream.write_all(&bytes)?;
    stream.flush()?;
    Ok(())
}

pub(crate) fn read_reply(stream: &mut impl Read) -> Result<Reply> {
    let (kind, length) = read_header(stream)?;
    ensure!(kind == Kind::Reply, "Expected a direct-transfer reply");
    let reply: Reply = read_json(stream, length)?;
    if let Reply::Rejected { message } = &reply {
        bail!("{message}");
    }
    Ok(reply)
}

pub(crate) fn read_offer(stream: &mut impl Read, length: u64) -> Result<DirectOfferArtifact> {
    read_json(stream, length)
}

pub(crate) fn copy_exact(
    reader: &mut impl Read,
    writer: &mut impl Write,
    bytes: u64,
    stopped: &std::sync::atomic::AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<()> {
    let start = Instant::now();
    let mut copied = 0;
    let mut buffer = [0_u8; 64 * 1024];
    while copied < bytes {
        ensure!(
            !stopped.load(std::sync::atomic::Ordering::Acquire),
            "Network transfer stopped"
        );
        ensure!(
            start.elapsed() < DEADLINE,
            "Network transfer exceeded its deadline"
        );
        let remaining = usize::try_from((bytes - copied).min(buffer.len() as u64))?;
        let count = reader.read(&mut buffer[..remaining])?;
        ensure!(
            count > 0,
            "Network transfer ended before the complete package arrived"
        );
        writer.write_all(&buffer[..count])?;
        copied += count as u64;
        progress(copied);
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn rejects_invalid_frames_before_body_allocation() {
        let mut bytes = Vec::from(&b"RLYPST1\n"[..]);
        bytes.push(Kind::Card as u8);
        bytes.extend_from_slice(&(MAX_JSON + 1).to_be_bytes());
        assert!(read_header(&mut Cursor::new(bytes)).is_err());
        let mut invalid = Vec::new();
        write_header(&mut invalid, Kind::Card, 0).unwrap();
        invalid[0] = b'X';
        assert!(read_header(&mut Cursor::new(invalid)).is_err());
        assert!(
            serde_json::from_str::<ParcelRequest>(r#"{"parcel_id":"a","unexpected":true}"#)
                .is_err()
        );
    }
    #[test]
    fn local_network_and_parcel_boundaries() {
        assert!(is_local(Ipv4Addr::LOCALHOST));
        assert!(is_local(Ipv4Addr::new(192, 168, 1, 2)));
        assert!(!is_local(Ipv4Addr::new(8, 8, 8, 8)));
        assert!(!is_local(Ipv4Addr::UNSPECIFIED));
        assert!(!valid_id("../receipt"));
        assert!(!valid_id(&"A".repeat(64)));
        assert!(valid_id(&"a".repeat(64)));
    }
}
