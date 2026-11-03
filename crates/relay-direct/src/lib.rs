//! Local-network discovery and transport. Trust, saved files and confirmation
//! remain exclusively under relay-core's authenticated parcel/receipt rules.
mod discovery;
mod engine;
mod wire;

use relay_core::{Inspection, PeerCard, Profile};
use std::path::PathBuf;
use std::sync::Arc;

pub use engine::DirectHandle;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct PeerId(pub String);

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct PairingId(pub String);

#[derive(Clone, Debug)]
pub enum ServiceState {
    DormantNoIdentity,
    Starting,
    Ready,
    Unavailable {
        kind: UnavailableKind,
        message: String,
    },
}

#[derive(Clone, Debug)]
pub enum UnavailableKind {
    Listener,
    Discovery,
    LocalNetwork,
}

#[derive(Clone, Debug)]
pub struct NearbyPeer {
    pub id: PeerId,
    pub card: PeerCard,
    pub locally_verified: bool,
}

#[derive(Clone, Debug)]
pub struct IncomingPairing {
    pub id: PairingId,
    pub card: PeerCard,
    pub state: IncomingPairingState,
}

#[derive(Clone, Debug)]
pub enum IncomingPairingState {
    AwaitingDecision,
    Accepted,
    Declined,
    Failed { message: String },
}

#[derive(Clone, Debug)]
pub struct PairingAttempt {
    pub peer: PeerId,
    pub card: PeerCard,
    pub state: PairingAttemptState,
}

#[derive(Clone, Debug)]
pub enum PairingAttemptState {
    Sending,
    WaitingForPeer,
    PeerAccepted,
    Declined,
    Failed { message: String, retryable: bool },
}

#[derive(Clone, Debug)]
pub enum IncomingOfferState {
    AwaitingDecision,
    Receiving {
        received: u64,
        total: u64,
    },
    ReadyToSave {
        parcel_path: PathBuf,
        inspection: Inspection,
    },
    Declined,
    Failed {
        message: String,
        retryable: bool,
    },
}

#[derive(Clone, Debug)]
pub struct IncomingOffer {
    pub parcel_id: String,
    pub sender_name: String,
    pub sender_fingerprint: String,
    pub files: usize,
    pub total_bytes: u64,
    pub state: IncomingOfferState,
}

#[derive(Clone, Debug)]
pub enum OutgoingTransferState {
    Offering,
    WaitingForRecipient,
    Sending { sent: u64, total: u64 },
    WaitingForSave,
    Confirmed,
    Declined,
    Failed { message: String, retryable: bool },
}

#[derive(Clone, Debug)]
pub struct OutgoingTransfer {
    pub parcel_id: String,
    pub recipient: String,
    pub peer: PeerId,
    pub state: OutgoingTransferState,
}

#[derive(Clone, Debug)]
pub struct DirectSnapshot {
    pub generation: u64,
    pub profile_revision: u64,
    pub service: ServiceState,
    pub visible: bool,
    pub peers: Vec<NearbyPeer>,
    pub incoming_pairings: Vec<IncomingPairing>,
    pub pairing_attempts: Vec<PairingAttempt>,
    pub incoming: Vec<IncomingOffer>,
    pub outgoing: Vec<OutgoingTransfer>,
}

pub enum DirectCommand {
    SetVisible(bool),
    RefreshDiscovery,
    RequestPairing { peer: PeerId },
    AcceptPairing { request: PairingId },
    DeclinePairing { request: PairingId },
    Send { peer: PeerId, parcel_id: String },
    AcceptOffer { parcel_id: String },
    DeclineOffer { parcel_id: String },
    Retry { parcel_id: String },
    ReloadProfile,
}

#[derive(Clone, Debug)]
pub enum DirectCommandError {
    PeerUnavailable,
    QueueFull,
    Stopped,
    ShutdownTimeout,
}

impl std::fmt::Display for DirectCommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::PeerUnavailable => {
                "That computer is no longer nearby. Scan again before adding it."
            }
            Self::QueueFull => "Too many network actions are pending. Try again shortly.",
            Self::Stopped => "Local-network transfers have stopped.",
            Self::ShutdownTimeout => "A network operation did not stop within two seconds.",
        })
    }
}

impl std::error::Error for DirectCommandError {}

/// Explicit listener address and discovery policy for embedding or controlled
/// network environments. Default desktop operation uses private LAN IPv4.
#[derive(Clone, Debug)]
pub struct DirectConfig {
    pub listen_addr: std::net::SocketAddrV4,
    pub discovery: bool,
}

impl Default for DirectConfig {
    fn default() -> Self {
        Self {
            listen_addr: std::net::SocketAddrV4::new(std::net::Ipv4Addr::UNSPECIFIED, 0),
            discovery: true,
        }
    }
}

impl DirectHandle {
    pub fn spawn(profile: Profile, on_change: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self::spawn_with_config(profile, on_change, DirectConfig::default())
    }
}
