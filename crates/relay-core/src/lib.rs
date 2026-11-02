//! RelayPost's shared offline delivery engine.
//!
//! The public DTOs contain no secret material. A profile only holds its isolated
//! home path; keys are unlocked for the duration of an individual operation.
mod archive;
mod codec;
pub mod demo;
mod format;
mod fsutil;
mod identity;
mod profile;
mod store;

pub use demo::{DemoPhase, DemoSession, DemoSnapshot};

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub use profile::Profile;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PeerCard {
    pub version: u32,
    pub name: String,
    pub fingerprint: String,
    pub age_recipient: String,
    pub signing_public_key: String,
    pub signature: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Contact {
    pub card: PeerCard,
    pub verified: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Direction {
    Outgoing,
    Incoming,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DeliveryState {
    Ready,
    WaitingReceipt,
    SavedReceiptReady,
    Confirmed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParcelRecord {
    pub parcel_id: String,
    pub direction: Direction,
    pub state: DeliveryState,
    pub sender: String,
    pub recipient: String,
    pub files: usize,
    pub total_bytes: u64,
    pub parcel_path: Option<PathBuf>,
    pub received_path: Option<PathBuf>,
    pub receipt_path: Option<PathBuf>,
    /// Retained local artifact damage, independent of the outgoing delivery phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupInspection {
    pub index: usize,
    pub data_shards: usize,
    pub parity_shards: usize,
    pub valid_shards: usize,
    pub missing_shards: Vec<usize>,
    pub recoverable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Inspection {
    pub parcel_id: String,
    pub sender: String,
    pub recipient: String,
    pub ciphertext_bytes: u64,
    pub groups: Vec<GroupInspection>,
    pub recoverable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReceiveResult {
    pub record: ParcelRecord,
    pub repaired_shards: usize,
    pub duplicate: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecoverySummary {
    pub recovered: usize,
    pub pending: usize,
    pub conflicts: Vec<String>,
}
