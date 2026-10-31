use crate::{fsutil, PeerCard};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

pub(crate) const MIB: u64 = 1024 * 1024;
pub(crate) const MAX_RAW: u64 = 64 * MIB;
pub(crate) const MAX_CIPHERTEXT: u64 = 80 * MIB;
pub(crate) const MAX_ENVELOPE: u64 = 64 * 1024;
pub(crate) const MAX_MANIFEST: u64 = 256 * 1024;
pub(crate) const MAX_FILES: usize = 100;
pub(crate) const GROUP_BYTES: u64 = 20 * MIB;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Shard {
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Group {
    pub index: usize,
    pub bytes: u64,
    pub data_shards: usize,
    pub parity_shards: usize,
    pub shard_bytes: usize,
    pub shards: Vec<Shard>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Envelope {
    pub kind: String,
    pub schema: u32,
    pub parcel_id: String,
    pub sender: PeerCard,
    pub recipient: PeerCard,
    pub ciphertext_bytes: u64,
    pub manifest_sha256: String,
    pub files: usize,
    pub total_bytes: u64,
    pub created_at: String,
    pub groups: Vec<Group>,
}

impl Envelope {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.kind == "relaypost.parcel" && self.schema == 1, "unsupported parcel format");
        fsutil::check_id(&self.parcel_id)?;
        fsutil::check_id(&self.manifest_sha256)?;
        ensure!(self.ciphertext_bytes > 0 && self.ciphertext_bytes <= MAX_CIPHERTEXT, "ciphertext length out of bounds");
        ensure!(self.files > 0 && self.files <= MAX_FILES && self.total_bytes <= MAX_RAW, "raw content limits exceeded");
        ensure!(self.created_at.len() <= 64 && chrono::DateTime::parse_from_rfc3339(&self.created_at).is_ok(), "invalid creation time");
        ensure!(!self.groups.is_empty() && self.groups.len() <= 4, "invalid group count");
        let expected_groups = self.ciphertext_bytes.div_ceil(GROUP_BYTES) as usize;
        ensure!(self.groups.len() == expected_groups, "noncanonical group partition");
        let mut remaining = self.ciphertext_bytes;
        for (i, group) in self.groups.iter().enumerate() {
            let bytes = remaining.min(GROUP_BYTES);
            let n = bytes.div_ceil(MIB) as usize;
            let r = n.div_ceil(5);
            let size = bytes.div_ceil(n as u64) as usize;
            ensure!(group.index == i && group.bytes == bytes && group.data_shards == n && group.parity_shards == r && group.shard_bytes == size, "invalid authenticated group parameters");
            ensure!(n >= 1 && n <= 20 && n + r <= 24 && size > 0 && size <= MIB as usize, "group exceeds resource limits");
            ensure!(group.shards.len() == n + r, "invalid shard count");
            for (j, shard) in group.shards.iter().enumerate() {
                ensure!(shard.path == shard_path(i, j), "unsafe or noncanonical shard path");
                fsutil::check_id(&shard.sha256)?;
            }
            remaining -= bytes;
        }
        ensure!(remaining == 0, "group lengths do not cover ciphertext");
        Ok(())
    }
}

pub(crate) fn shard_path(group: usize, shard: usize) -> String {
    format!("groups/g{group:03}/s{shard:03}.bin")
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManifestFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub kind: String,
    pub schema: u32,
    pub files: Vec<ManifestFile>,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Prepared {
    pub kind: String,
    pub schema: u32,
    pub parcel_id: String,
    pub sender: String,
    pub recipient: String,
    pub envelope_sha256: String,
    pub manifest_sha256: String,
}

impl Prepared {
    pub fn for_envelope(envelope: &Envelope, raw: &[u8]) -> Self {
        Self {
            kind: "relaypost.prepared".into(),
            schema: 1,
            parcel_id: envelope.parcel_id.clone(),
            sender: envelope.sender.fingerprint.clone(),
            recipient: envelope.recipient.fingerprint.clone(),
            envelope_sha256: fsutil::digest(raw),
            manifest_sha256: envelope.manifest_sha256.clone(),
        }
    }
    pub fn verify(&self, envelope: &Envelope, raw: &[u8]) -> Result<()> {
        let expected = Self::for_envelope(envelope, raw);
        ensure!(serde_json::to_vec(self)? == serde_json::to_vec(&expected)?, "published commit metadata conflicts with envelope");
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Receipt {
    pub kind: String,
    pub schema: u32,
    pub parcel_id: String,
    pub envelope_sha256: String,
    pub manifest_sha256: String,
    pub recipient: String,
    pub signing_public_key: String,
    pub result: String,
    pub committed_at: String,
}

impl Receipt {
    pub fn validate(&self, envelope: &Envelope, envelope_raw: &[u8]) -> Result<()> {
        ensure!(self.kind == "relaypost.receipt" && self.schema == 1 && self.result == "committed", "unsupported receipt");
        ensure!(self.parcel_id == envelope.parcel_id && self.envelope_sha256 == fsutil::digest(envelope_raw) && self.manifest_sha256 == envelope.manifest_sha256, "receipt does not bind this exact parcel");
        ensure!(self.recipient == envelope.recipient.fingerprint && self.signing_public_key == envelope.recipient.signing_public_key, "wrong receipt signing identity");
        ensure!(self.committed_at.len() <= 64 && chrono::DateTime::parse_from_rfc3339(&self.committed_at).is_ok(), "invalid receipt time");
        Ok(())
    }
}
