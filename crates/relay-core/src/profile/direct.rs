//! The encrypted parcel is the transport boundary. Networking never receives
//! private keys or authority to decrypt, publish plaintext, or sign receipts.
use super::{AuthenticatedEnvelope, Profile, ENVELOPE_COPIES};
use crate::format::MAX_ENVELOPE;
use crate::{archive, codec, fsutil, identity, store};
use crate::{Contact, DeliveryState, Direction, Inspection, ParcelRecord, PeerCard};
use anyhow::{ensure, Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const MAX_DIRECT_BUNDLE: u64 = 100 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectOfferArtifact {
    pub schema: u32,
    pub envelope_json: String,
    pub envelope_minisig: String,
    pub bundle_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct DirectOffer {
    pub parcel_id: String,
    pub sender: String,
    pub recipient: String,
    pub files: usize,
    pub total_bytes: u64,
    pub ciphertext_bytes: u64,
    pub bundle_bytes: u64,
}

pub struct PreparedDirectHandoff {
    pub offer: DirectOfferArtifact,
    bundle: tempfile::NamedTempFile,
    bundle_bytes: u64,
}

impl PreparedDirectHandoff {
    pub fn bundle_len(&self) -> u64 {
        self.bundle_bytes
    }

    pub fn open_bundle(&self) -> Result<File> {
        Ok(self.bundle.reopen()?)
    }
}

#[derive(Clone, Debug)]
pub struct StagedDirectParcel {
    pub parcel_path: PathBuf,
    pub inspection: Inspection,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptArtifact {
    pub receipt_json: String,
    pub receipt_minisig: String,
}

impl Profile {
    /// Inspect an untrusted discovery card without persisting it or granting trust.
    pub fn inspect_contact_card(&self, card: &PeerCard) -> Result<Contact> {
        let _guard = fsutil::lock(&self.home)?;
        self.inspect_contact_card_unlocked(card, &store::open(&self.home)?)
    }

    /// Persist public identity only. Full-code verification remains a separate action.
    pub fn import_contact_card(&self, card: &PeerCard) -> Result<Contact> {
        let _guard = fsutil::lock(&self.home)?;
        let connection = store::open(&self.home)?;
        self.inspect_contact_card_unlocked(card, &connection)?;
        store::import_contact(&connection, card)
    }

    fn inspect_contact_card_unlocked(
        &self,
        card: &PeerCard,
        connection: &Connection,
    ) -> Result<Contact> {
        identity::verify_card(card)?;
        ensure!(
            card.fingerprint != self.card()?.fingerprint,
            "this is your own identity card; select another computer"
        );
        let existing = store::contacts(connection)?
            .into_iter()
            .find(|c| c.card.fingerprint == card.fingerprint);
        if let Some(existing) = &existing {
            ensure!(
                existing.card.age_recipient == card.age_recipient
                    && existing.card.signing_public_key == card.signing_public_key,
                "pinned identity cannot be replaced"
            );
        }
        Ok(Contact {
            card: card.clone(),
            verified: existing.is_some_and(|c| c.verified),
        })
    }

    pub fn prepare_direct_handoff(&self, parcel_id: &str) -> Result<PreparedDirectHandoff> {
        let _guard = fsutil::lock(&self.home)?;
        fsutil::check_id(parcel_id)?;
        let connection = store::open(&self.home)?;
        ensure!(!self.cancelled(parcel_id)?, "delivery is cancelled");
        if let Some(record) = store::get(&connection, parcel_id, Direction::Outgoing)? {
            ensure!(
                record.state != DeliveryState::Cancelled,
                "delivery is cancelled"
            );
        }
        let source = self.home.join("outbox").join(parcel_id);
        let authenticated = self.load_envelope(&source, &connection)?;
        ensure!(
            authenticated.envelope.parcel_id == parcel_id
                && authenticated.envelope.sender.fingerprint == self.card()?.fingerprint,
            "outgoing identity or directory mismatch"
        );
        self.verify_outgoing_shards(&source, &authenticated)?;
        let entries = bundle_entries(&authenticated);
        let bundle_bytes = bundle_length(&entries)?;
        fsutil::available_space(&self.home, bundle_bytes)?;
        let mut bundle = tempfile::Builder::new()
            .prefix("direct-send-")
            .tempfile_in(self.home.join("work"))?;
        {
            let mut builder = tar::Builder::new(bundle.as_file_mut());
            for (path, bytes) in &entries {
                let header = archive::header(path, *bytes)?;
                if path.ends_with(".json") {
                    builder.append(&header, authenticated.raw.as_slice())?;
                } else if path.ends_with(".minisig") {
                    builder.append(&header, authenticated.signature.as_bytes())?;
                } else {
                    let input = fsutil::read_bounded(&source.join(path), *bytes)?;
                    let expected = authenticated
                        .envelope
                        .groups
                        .iter()
                        .flat_map(|g| &g.shards)
                        .find(|s| &s.path == path)
                        .context("undeclared outgoing shard")?;
                    ensure!(
                        input.len() as u64 == *bytes && fsutil::digest(&input) == expected.sha256,
                        "outgoing shard changed while preparing direct bundle"
                    );
                    builder.append(&header, input.as_slice())?;
                }
            }
            builder.finish()?;
        }
        bundle.as_file().sync_all()?;
        ensure!(
            bundle.as_file().metadata()?.len() == bundle_bytes,
            "noncanonical direct bundle length"
        );
        let offer = DirectOfferArtifact {
            schema: 1,
            envelope_json: fsutil::utf8(&authenticated.raw)?.to_owned(),
            envelope_minisig: authenticated.signature,
            bundle_bytes,
        };
        Ok(PreparedDirectHandoff {
            offer,
            bundle,
            bundle_bytes,
        })
    }

    pub fn inspect_direct_offer(&self, artifact: &DirectOfferArtifact) -> Result<DirectOffer> {
        let _guard = fsutil::lock(&self.home)?;
        let authenticated = self.authenticate_direct(artifact, &store::open(&self.home)?)?;
        Ok(offer_details(&authenticated, artifact.bundle_bytes))
    }

    /// Resolve a restarted transfer only when the exact authenticated offer is
    /// already staged. A ciphertext ID alone cannot identify signed metadata.
    pub fn existing_direct_offer(
        &self,
        artifact: &DirectOfferArtifact,
    ) -> Result<Option<StagedDirectParcel>> {
        let _guard = fsutil::lock(&self.home)?;
        let connection = store::open(&self.home)?;
        let authenticated = self.authenticate_direct(artifact, &connection)?;
        self.existing_direct_offer_unlocked(&authenticated, &connection)
    }

    fn existing_direct_offer_unlocked(
        &self,
        authenticated: &AuthenticatedEnvelope,
        connection: &Connection,
    ) -> Result<Option<StagedDirectParcel>> {
        let root = self.home.join("direct/incoming");
        fsutil::no_symlink_components(&root)?;
        let destination = root.join(format!("{}.parcel", authenticated.envelope.parcel_id));
        match fs::symlink_metadata(&destination) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
        let existing = self.load_envelope(&destination, connection)?;
        ensure!(
            existing.raw == authenticated.raw && existing.signature == authenticated.signature,
            "conflicting direct parcel already staged"
        );
        let inspection = inspect_direct_directory(self, &destination, &existing)?;
        ensure!(
            inspection.recoverable,
            "staged direct parcel exceeds repair budget"
        );
        Ok(Some(StagedDirectParcel {
            parcel_path: destination,
            inspection,
        }))
    }

    pub fn stage_direct_bundle(
        &self,
        artifact: &DirectOfferArtifact,
        bundle_file: &Path,
    ) -> Result<StagedDirectParcel> {
        let _guard = fsutil::lock(&self.home)?;
        let connection = store::open(&self.home)?;
        let authenticated = self.authenticate_direct(artifact, &connection)?;
        let path = fsutil::selected_file(bundle_file)?;
        let mut input = fsutil::open_read(&path)?;
        ensure!(
            input.metadata()?.len() == artifact.bundle_bytes,
            "truncated or oversized direct bundle"
        );
        let root = self.home.join("direct/incoming");
        fsutil::no_symlink_components(&root)?;
        fsutil::available_space(&root, artifact.bundle_bytes)?;
        let staging = tempfile::Builder::new()
            .prefix(".stage-")
            .tempdir_in(&root)?;
        unpack_bundle(&mut input, staging.path(), &authenticated)?;
        let inspection = inspect_direct_directory(self, staging.path(), &authenticated)?;
        ensure!(
            inspection.recoverable,
            "direct bundle exceeds repair budget"
        );
        fsutil::sync_tree(staging.path())?;
        let destination = root.join(format!("{}.parcel", authenticated.envelope.parcel_id));
        match self.existing_direct_offer_unlocked(&authenticated, &connection)? {
            Some(existing) => {
                for (name, bytes) in bundle_entries(&authenticated) {
                    ensure!(
                        archive::hash_file(&existing.parcel_path.join(&name), bytes)?
                            == archive::hash_file(&staging.path().join(&name), bytes)?,
                        "staged direct parcel conflicts with retry bytes"
                    );
                }
            }
            None => {
                fsutil::publish_noreplace(staging.path(), &destination)?;
            }
        }
        Ok(StagedDirectParcel {
            parcel_path: destination,
            inspection,
        })
    }

    pub fn direct_offers(&self) -> Result<Vec<StagedDirectParcel>> {
        let _guard = fsutil::lock(&self.home)?;
        let connection = store::open(&self.home)?;
        let root = self.home.join("direct/incoming");
        fsutil::no_symlink_components(&root)?;
        let mut offers = Vec::new();
        for (count, entry) in fs::read_dir(&root)?.enumerate() {
            ensure!(count < 512, "too many staged direct entries");
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF8 direct staging entry"))?;
            if name.starts_with(".stage-") {
                continue;
            }
            let id = name
                .strip_suffix(".parcel")
                .context("unexpected direct staging entry")?;
            fsutil::check_id(id)?;
            let authenticated = self.load_envelope(&entry.path(), &connection)?;
            ensure!(
                authenticated.envelope.parcel_id == id,
                "direct stage directory ID mismatch"
            );
            let inspection = inspect_direct_directory(self, &entry.path(), &authenticated)?;
            ensure!(
                inspection.recoverable,
                "staged direct parcel exceeds repair budget"
            );
            offers.push(StagedDirectParcel {
                parcel_path: entry.path(),
                inspection,
            });
        }
        offers.sort_by(|a, b| a.inspection.parcel_id.cmp(&b.inspection.parcel_id));
        Ok(offers)
    }

    /// The caller invokes this only after the receiver acknowledges a complete
    /// encrypted stage. It grants no plaintext-save or confirmation authority.
    pub fn record_direct_handoff(&self, parcel_id: &str) -> Result<ParcelRecord> {
        let _guard = fsutil::lock(&self.home)?;
        fsutil::check_id(parcel_id)?;
        let connection = store::open(&self.home)?;
        ensure!(!self.cancelled(parcel_id)?, "delivery is cancelled");
        let source = self.home.join("outbox").join(parcel_id);
        let authenticated = self.load_envelope(&source, &connection)?;
        ensure!(
            authenticated.envelope.parcel_id == parcel_id
                && authenticated.envelope.sender.fingerprint == self.card()?.fingerprint,
            "outgoing identity or directory mismatch"
        );
        self.verify_outgoing_shards(&source, &authenticated)?;
        let mut record =
            store::get(&connection, parcel_id, Direction::Outgoing)?.unwrap_or_else(|| {
                self.outgoing_record(&authenticated.envelope, source, DeliveryState::Ready)
            });
        ensure!(
            record.state != DeliveryState::Cancelled,
            "delivery is cancelled"
        );
        if record.state != DeliveryState::Confirmed {
            record.state = DeliveryState::WaitingReceipt;
        }
        record.local_error = None;
        record.updated_at = fsutil::now();
        store::save(&connection, &record)?;
        Ok(record)
    }

    pub fn receipt_artifact(&self, parcel_id: &str) -> Result<ReceiptArtifact> {
        let _guard = fsutil::lock(&self.home)?;
        fsutil::check_id(parcel_id)?;
        let connection = store::open(&self.home)?;
        let authenticated = match self
            .verify_committed(&self.home.join("inbox").join(parcel_id), &connection)
        {
            Ok(value) => value,
            Err(error) => {
                self.mark_failed(&connection, parcel_id)?;
                return Err(error).context("current received content cannot authorize a receipt");
            }
        };
        let (raw, signature) =
            self.read_receipt(&self.home.join("receipts").join(parcel_id), &authenticated)?;
        Ok(ReceiptArtifact {
            receipt_json: fsutil::utf8(&raw)?.to_owned(),
            receipt_minisig: signature,
        })
    }

    pub fn import_receipt_artifact(&self, artifact: &ReceiptArtifact) -> Result<ParcelRecord> {
        let _guard = fsutil::lock(&self.home)?;
        self.import_receipt_bytes(
            artifact.receipt_json.as_bytes().to_vec(),
            artifact.receipt_minisig.clone(),
            &store::open(&self.home)?,
        )
    }

    /// Remove only an owned encrypted stage whose exact envelope is backed by
    /// currently verified committed content and a valid signed receipt.
    pub fn finalize_direct_offer(&self, parcel_id: &str) -> Result<bool> {
        let _guard = fsutil::lock(&self.home)?;
        fsutil::check_id(parcel_id)?;
        let destination = self
            .home
            .join("direct/incoming")
            .join(format!("{parcel_id}.parcel"));
        match fs::symlink_metadata(&destination) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
        if !self.home.join("receipts").join(parcel_id).try_exists()? {
            return Ok(false);
        }
        let connection = store::open(&self.home)?;
        let committed =
            self.verify_committed(&self.home.join("inbox").join(parcel_id), &connection)?;
        self.read_receipt(&self.home.join("receipts").join(parcel_id), &committed)?;
        let staged = self.load_envelope(&destination, &connection)?;
        ensure!(
            committed.raw == staged.raw && committed.signature == staged.signature,
            "staged parcel conflicts with saved receipt"
        );
        inspect_direct_directory(self, &destination, &staged)?;
        // Remove the queue entry atomically before cleaning its verified files.
        // An interrupted cleanup cannot leave a half-deleted incoming offer.
        let cleanup = tempfile::Builder::new()
            .prefix("direct-finalized-")
            .tempdir_in(self.home.join("work"))?;
        let retired = cleanup.path().join("parcel");
        fsutil::publish_noreplace(&destination, &retired)?;
        fsutil::sync_dir(destination.parent().context("direct stage needs parent")?)?;
        for (path, _) in bundle_entries(&staged) {
            fs::remove_file(retired.join(path))?;
        }
        for group in &staged.envelope.groups {
            fs::remove_dir(retired.join(format!("groups/g{:03}", group.index)))?;
        }
        fs::remove_dir(retired.join("groups"))?;
        fs::remove_dir(&retired)?;
        Ok(true)
    }

    fn authenticate_direct(
        &self,
        artifact: &DirectOfferArtifact,
        connection: &Connection,
    ) -> Result<AuthenticatedEnvelope> {
        ensure!(
            artifact.schema == 1
                && artifact.envelope_json.len() as u64 <= MAX_ENVELOPE
                && artifact.envelope_minisig.len() <= 2048,
            "unsupported or oversized direct offer"
        );
        let authenticated = self.authenticate(
            artifact.envelope_json.as_bytes().to_vec(),
            artifact.envelope_minisig.clone(),
            connection,
        )?;
        ensure!(
            authenticated.envelope.recipient.fingerprint == self.card()?.fingerprint,
            "direct offer belongs to a different recipient"
        );
        ensure!(
            artifact.bundle_bytes == bundle_length(&bundle_entries(&authenticated))?,
            "direct bundle length does not match authenticated resources"
        );
        Ok(authenticated)
    }
}

fn offer_details(authenticated: &AuthenticatedEnvelope, bundle_bytes: u64) -> DirectOffer {
    let envelope = &authenticated.envelope;
    DirectOffer {
        parcel_id: envelope.parcel_id.clone(),
        sender: envelope.sender.fingerprint.clone(),
        recipient: envelope.recipient.fingerprint.clone(),
        files: envelope.files,
        total_bytes: envelope.total_bytes,
        ciphertext_bytes: envelope.ciphertext_bytes,
        bundle_bytes,
    }
}

fn bundle_entries(authenticated: &AuthenticatedEnvelope) -> Vec<(String, u64)> {
    let mut entries = Vec::new();
    for (json, signature) in ENVELOPE_COPIES {
        entries.push((json.to_owned(), authenticated.raw.len() as u64));
        entries.push((signature.to_owned(), authenticated.signature.len() as u64));
    }
    for group in &authenticated.envelope.groups {
        for shard in &group.shards {
            entries.push((shard.path.clone(), group.shard_bytes as u64));
        }
    }
    entries
}

fn bundle_length(entries: &[(String, u64)]) -> Result<u64> {
    let length = entries.iter().try_fold(1024u64, |total, (_, bytes)| {
        total
            .checked_add(512 + bytes.div_ceil(512) * 512)
            .context("direct bundle size overflow")
    })?;
    ensure!(length <= MAX_DIRECT_BUNDLE, "direct bundle exceeds 100 MiB");
    Ok(length)
}

fn unpack_bundle(
    input: &mut File,
    output: &Path,
    authenticated: &AuthenticatedEnvelope,
) -> Result<()> {
    for (path, expected_bytes) in bundle_entries(authenticated) {
        let bytes = archive::read_header(input, &path, expected_bytes)?;
        ensure!(
            bytes == expected_bytes,
            "direct entry length disagrees with signed envelope"
        );
        let target = output.join(&path);
        fsutil::private_dir(target.parent().context("direct entry needs parent")?)?;
        let mut file = fsutil::create_new(&target)?;
        ensure!(
            std::io::copy(&mut (&mut *input).take(bytes), &mut file)? == bytes,
            "truncated direct entry"
        );
        file.flush()?;
        file.sync_all()?;
        archive::padding(input, bytes)?;
    }
    let mut trailer = [0u8; 1024];
    input.read_exact(&mut trailer)?;
    ensure!(
        trailer.iter().all(|b| *b == 0),
        "undeclared direct archive entries"
    );
    let mut extra = [0];
    ensure!(input.read(&mut extra)? == 0, "trailing direct archive data");
    Ok(())
}

fn inspect_direct_directory(
    profile: &Profile,
    path: &Path,
    authenticated: &AuthenticatedEnvelope,
) -> Result<Inspection> {
    fsutil::no_symlink_components(path)?;
    profile.exact_entries(
        path,
        &[
            "envelope.json",
            "envelope.minisig",
            "envelope-copy.json",
            "envelope-copy.minisig",
            "groups",
        ],
    )?;
    let expected_groups: Vec<_> = authenticated
        .envelope
        .groups
        .iter()
        .map(|g| format!("g{:03}", g.index))
        .collect();
    profile.exact_entries(
        &path.join("groups"),
        &expected_groups
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    )?;
    for group in &authenticated.envelope.groups {
        let names: Vec<_> = (0..group.shards.len())
            .map(|i| format!("s{i:03}.bin"))
            .collect();
        profile.exact_entries(
            &path.join(format!("groups/g{:03}", group.index)),
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        )?;
    }
    for (name, bytes) in bundle_entries(authenticated) {
        let file = fsutil::open_read(&path.join(&name))?;
        ensure!(
            file.metadata()?.len() == bytes,
            "direct entry length changed"
        );
        if name.ends_with(".json") {
            ensure!(
                fsutil::read_bounded(&path.join(name), MAX_ENVELOPE)? == authenticated.raw,
                "direct envelope differs from accepted offer"
            );
        } else if name.ends_with(".minisig") {
            ensure!(
                fsutil::read_bounded(&path.join(name), 2048)? == authenticated.signature.as_bytes(),
                "direct signature differs from accepted offer"
            );
        }
    }
    let groups = codec::inspect(path, &authenticated.envelope.groups)?;
    Ok(Inspection {
        parcel_id: authenticated.envelope.parcel_id.clone(),
        sender: authenticated.envelope.sender.fingerprint.clone(),
        recipient: authenticated.envelope.recipient.fingerprint.clone(),
        ciphertext_bytes: authenticated.envelope.ciphertext_bytes,
        recoverable: groups.iter().all(|g| g.recoverable),
        groups,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::Envelope;

    #[test]
    fn restart_lookup_refuses_different_valid_metadata_for_same_ciphertext() -> Result<()> {
        const PASSWORD: &str = "SYNTHETIC-direct-metadata-test-only";
        let temporary = tempfile::tempdir()?;
        let root = fs::canonicalize(temporary.path())?;
        let alice = Profile::open(root.join("alice"))?;
        let bob = Profile::open(root.join("bob"))?;
        let alice_card = alice.initialize("Synthetic Metadata Alice", PASSWORD)?;
        let bob_card = bob.initialize("Synthetic Metadata Bob", PASSWORD)?;
        alice.import_contact_card(&bob_card)?;
        bob.import_contact_card(&alice_card)?;
        alice.trust_contact(&bob_card.fingerprint)?;
        bob.trust_contact(&alice_card.fingerprint)?;
        let input = root.join("synthetic.txt");
        fs::write(&input, b"Synthetic same-ciphertext metadata regression")?;
        let record = alice.seal(&[input], &bob_card.fingerprint, PASSWORD)?;
        let prepared = alice.prepare_direct_handoff(&record.parcel_id)?;
        assert!(bob.existing_direct_offer(&prepared.offer)?.is_none());
        let bundle = root.join("bundle.tar");
        let mut output = fsutil::create_new(&bundle)?;
        std::io::copy(&mut prepared.open_bundle()?, &mut output)?;
        output.sync_all()?;
        let staged = bob.stage_direct_bundle(&prepared.offer, &bundle)?;
        let restarted_bob = Profile::open(root.join("bob"))?;
        assert_eq!(
            restarted_bob
                .existing_direct_offer(&prepared.offer)?
                .unwrap()
                .parcel_path,
            staged.parcel_path
        );

        // The ciphertext digest deliberately stays identical. This is a real,
        // independently valid signature, not a malformed-signature test.
        let mut envelope: Envelope = fsutil::json(prepared.offer.envelope_json.as_bytes())?;
        envelope.created_at = "2001-01-01T00:00:00.000Z".to_owned();
        let raw = serde_json::to_vec(&envelope)?;
        let unlocked = identity::unlock(&alice.home.join("identity"), PASSWORD)?;
        let mut different = prepared.offer.clone();
        different.envelope_json = fsutil::utf8(&raw)?.to_owned();
        different.envelope_minisig = identity::sign(&unlocked.signing, &raw)?;
        assert_eq!(
            restarted_bob.inspect_direct_offer(&different)?.parcel_id,
            record.parcel_id
        );
        assert!(restarted_bob.existing_direct_offer(&different).is_err());
        assert_eq!(
            restarted_bob
                .existing_direct_offer(&prepared.offer)?
                .unwrap()
                .parcel_path,
            staged.parcel_path
        );
        assert!(restarted_bob.deliveries()?.is_empty());
        assert!(restarted_bob.receipt_artifact(&record.parcel_id).is_err());
        Ok(())
    }
}
