use crate::format::{Envelope, Manifest, Prepared, Receipt, MAX_ENVELOPE, MAX_MANIFEST, MAX_RAW};
use crate::{archive, codec, fsutil, identity, store};
use crate::{
    Contact, DeliveryState, Direction, Inspection, ParcelRecord, PeerCard, ReceiveResult,
    RecoverySummary,
};
use anyhow::{bail, ensure, Context, Result};
use rusqlite::Connection;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const ENVELOPE_COPIES: [(&str, &str); 2] = [
    ("envelope.json", "envelope.minisig"),
    ("envelope-copy.json", "envelope-copy.minisig"),
];

/// A cheap, cloneable handle. No unlocked keys, password, or shared database
/// connection are retained between operations. Cooperating processes serialize
/// operations with an OS advisory lock inside this private home.
#[derive(Clone, Debug)]
pub struct Profile {
    home: PathBuf,
}

struct AuthenticatedEnvelope {
    envelope: Envelope,
    raw: Vec<u8>,
    signature: String,
}

impl Profile {
    pub fn open(home: impl Into<PathBuf>) -> Result<Self> {
        let home = home.into();
        if let Ok(metadata) = fs::symlink_metadata(&home) {
            ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "profile home must be a real directory"
            );
        }
        fsutil::private_dir(&home)?;
        // Resolve user-selected ancestors once. Thereafter every managed path
        // is rooted in this canonical private directory and checked for links.
        let profile = Self {
            home: fs::canonicalize(home)?,
        };
        let _guard = fsutil::lock(&profile.home)?;
        for directory in [
            "outbox",
            "inbox",
            "inbox/.staging",
            "receipts",
            "acknowledgments",
            "work",
            "cancelled",
        ] {
            fsutil::private_dir(&profile.home.join(directory))?;
        }
        store::open(&profile.home)?;
        fsutil::sync_dir(&profile.home)?;
        Ok(profile)
    }

    pub fn initialize(&self, name: &str, password: &str) -> Result<PeerCard> {
        let _guard = fsutil::lock(&self.home)?;
        identity::initialize(&self.home.join("identity"), name, password)
    }

    pub fn public_card(&self) -> Result<PeerCard> {
        let _guard = fsutil::lock(&self.home)?;
        self.card()
    }

    pub fn export_card(&self, output: &Path) -> Result<PeerCard> {
        let _guard = fsutil::lock(&self.home)?;
        let card = self.card()?;
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = self.export_directory(parent)?;
        let output = parent.join(output.file_name().context("card output needs a filename")?);
        fsutil::write_new(&output, &serde_json::to_vec_pretty(&card)?)?;
        fsutil::sync_dir(&parent)?;
        Ok(card)
    }

    pub fn import_contact(&self, path: &Path) -> Result<Contact> {
        let _guard = fsutil::lock(&self.home)?;
        let path = fsutil::selected_file(path)?;
        let card: PeerCard = fsutil::json(&fsutil::read_bounded(&path, MAX_ENVELOPE)?)?;
        store::import_contact(&store::open(&self.home)?, &card)
    }

    pub fn trust_contact(&self, fingerprint: &str) -> Result<()> {
        let _guard = fsutil::lock(&self.home)?;
        store::trust(&store::open(&self.home)?, fingerprint)
    }

    pub fn contacts(&self) -> Result<Vec<Contact>> {
        let _guard = fsutil::lock(&self.home)?;
        store::contacts(&store::open(&self.home)?)
    }

    pub fn seal(
        &self,
        inputs: &[PathBuf],
        recipient: &str,
        password: &str,
    ) -> Result<ParcelRecord> {
        let _guard = fsutil::lock(&self.home)?;
        let connection = store::open(&self.home)?;
        let sender = self.card()?;
        let recipient = store::resolve(&connection, recipient)?;
        let unlocked = identity::unlock(&self.home.join("identity"), password)?;
        ensure!(
            !inputs.is_empty() && inputs.len() <= crate::format::MAX_FILES,
            "select 1–100 regular files"
        );
        let inputs: Vec<PathBuf> = inputs
            .iter()
            .map(|p| fsutil::selected_file(p))
            .collect::<Result<_>>()?;
        let estimated_raw = inputs.iter().try_fold(0u64, |total, path| -> Result<u64> {
            let meta = fs::symlink_metadata(path)?;
            ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "select static regular files only"
            );
            total
                .checked_add(meta.len())
                .context("raw input size overflow")
        })?;
        ensure!(estimated_raw <= MAX_RAW, "batch exceeds 64 MiB raw limit");
        fsutil::available_space(
            &self.home,
            estimated_raw.saturating_mul(5) + 8 * crate::format::MIB,
        )?;
        let work = tempfile::Builder::new()
            .prefix("seal-")
            .tempdir_in(self.home.join("work"))?;
        let (manifest, snapshots) = archive::snapshot(&inputs, work.path())?;
        let manifest_raw = serde_json::to_vec(&manifest)?;
        ensure!(
            manifest_raw.len() as u64 <= MAX_MANIFEST,
            "manifest exceeds limit"
        );
        let archive_path = work.path().join("archive.tar");
        archive::pack(&manifest_raw, &snapshots, &archive_path)?;
        let ciphertext = work.path().join("ciphertext.age");
        identity::encrypt_file(&archive_path, &ciphertext, &recipient.age_recipient)?;
        let staged = tempfile::Builder::new()
            .prefix(".seal-")
            .tempdir_in(self.home.join("outbox"))?;
        let (parcel_id, ciphertext_bytes, groups) = codec::encode(&ciphertext, staged.path())?;
        let envelope = Envelope {
            kind: "relaypost.parcel".into(),
            schema: 1,
            parcel_id: parcel_id.clone(),
            sender,
            recipient,
            ciphertext_bytes,
            manifest_sha256: fsutil::digest(&manifest_raw),
            files: manifest.files.len(),
            total_bytes: manifest.total_bytes,
            created_at: fsutil::now(),
            groups,
        };
        envelope.validate()?;
        let raw = serde_json::to_vec(&envelope)?;
        ensure!(raw.len() as u64 <= MAX_ENVELOPE, "envelope exceeds 64 KiB");
        let signature = identity::sign(&unlocked.signing, &raw)?;
        for (json, sig) in ENVELOPE_COPIES {
            fsutil::write_new(&staged.path().join(json), &raw)?;
            fsutil::write_new(&staged.path().join(sig), signature.as_bytes())?;
        }
        fsutil::sync_tree(staged.path())?;
        let destination = self.home.join("outbox").join(&parcel_id);
        fsutil::publish_noreplace(staged.path(), &destination)?;
        let record = self.outgoing_record(&envelope, destination, DeliveryState::Ready);
        store::save(&connection, &record)?;
        Ok(record)
    }

    pub fn export_parcel(&self, parcel_id: &str, carrier: &Path) -> Result<PathBuf> {
        let _guard = fsutil::lock(&self.home)?;
        fsutil::check_id(parcel_id)?;
        let connection = store::open(&self.home)?;
        let mut record = store::get(&connection, parcel_id, Direction::Outgoing)?
            .context("outgoing delivery not found")?;
        ensure!(
            record.state != DeliveryState::Cancelled && !self.cancelled(parcel_id)?,
            "delivery is cancelled; exported copies cannot be recalled"
        );
        let source = self.home.join("outbox").join(parcel_id);
        let authenticated = self.load_envelope(&source, &connection)?;
        ensure!(
            authenticated.envelope.parcel_id == parcel_id,
            "outgoing directory ID conflicts with envelope"
        );
        ensure!(
            authenticated.envelope.sender.fingerprint == self.card()?.fingerprint,
            "outgoing sender identity mismatch"
        );
        let carrier = self.export_directory(carrier)?;
        let destination = carrier.join(format!("{parcel_id}.parcel"));
        if fs::symlink_metadata(&destination).is_ok() {
            let existing = self.load_envelope(&destination, &connection)?;
            ensure!(
                authenticated.raw == existing.raw && authenticated.signature == existing.signature,
                "carrier contains conflicting envelope for parcel ID"
            );
            let health = codec::inspect(&destination, &existing.envelope.groups)?;
            ensure!(
                health.iter().all(|g| g.missing_shards.is_empty()),
                "existing carrier copy is damaged; choose an empty carrier directory"
            );
        } else {
            let needed: u64 = authenticated
                .envelope
                .groups
                .iter()
                .map(|g| g.shard_bytes as u64 * g.shards.len() as u64)
                .sum();
            fsutil::available_space(&carrier, needed + 4 * MAX_ENVELOPE)?;
            let staged = tempfile::Builder::new()
                .prefix(".relaypost-export-")
                .tempdir_in(&carrier)?;
            for (json, sig) in ENVELOPE_COPIES {
                fsutil::write_new(&staged.path().join(json), &authenticated.raw)?;
                fsutil::write_new(&staged.path().join(sig), authenticated.signature.as_bytes())?;
            }
            codec::copy_exact(&source, staged.path(), &authenticated.envelope.groups)?;
            fsutil::sync_tree(staged.path())?;
            // Verify actual carrier bytes before recording local export success.
            let health = codec::inspect(staged.path(), &authenticated.envelope.groups)?;
            ensure!(
                health.iter().all(|g| g.missing_shards.is_empty()),
                "carrier verification failed"
            );
            fsutil::publish_noreplace(staged.path(), &destination)?;
        }
        if record.state != DeliveryState::Confirmed {
            record.state = DeliveryState::WaitingReceipt;
            record.updated_at = fsutil::now();
            store::save(&connection, &record)?;
        }
        Ok(destination)
    }

    pub fn inspect_parcel(&self, parcel: &Path) -> Result<Inspection> {
        let _guard = fsutil::lock(&self.home)?;
        let parcel = fsutil::selected_directory(parcel)?;
        let authenticated = self.load_envelope(&parcel, &store::open(&self.home)?)?;
        let envelope = authenticated.envelope;
        let groups = codec::inspect(&parcel, &envelope.groups)?;
        Ok(Inspection {
            parcel_id: envelope.parcel_id,
            sender: envelope.sender.fingerprint,
            recipient: envelope.recipient.fingerprint,
            ciphertext_bytes: envelope.ciphertext_bytes,
            recoverable: groups.iter().all(|g| g.recoverable),
            groups,
        })
    }

    pub fn receive_parcel(&self, parcel: &Path, password: &str) -> Result<ReceiveResult> {
        let _guard = fsutil::lock(&self.home)?;
        let connection = store::open(&self.home)?;
        let parcel = fsutil::selected_directory(parcel)?;
        let authenticated = self.load_envelope(&parcel, &connection)?;
        ensure!(
            authenticated.envelope.recipient.fingerprint == self.card()?.fingerprint,
            "parcel belongs to a different recipient"
        );
        let unlocked = identity::unlock(&self.home.join("identity"), password)?;
        let id = &authenticated.envelope.parcel_id;
        let destination = self.home.join("inbox").join(id);
        if fs::symlink_metadata(&destination).is_ok() {
            let existing = match self.verify_committed(&destination, &connection) {
                Ok(existing) => existing,
                Err(error) => {
                    self.mark_failed(&connection, id)?;
                    return Err(error).context(
                        "existing received batch changed; refusing overwrite or success receipt",
                    );
                }
            };
            ensure!(
                existing.raw == authenticated.raw && existing.signature == authenticated.signature,
                "same parcel ID has conflicting authenticated envelopes"
            );
            let record = self.finish_incoming(&connection, &existing, &unlocked.signing)?;
            return Ok(ReceiveResult {
                record,
                repaired_shards: 0,
                duplicate: true,
            });
        }
        ensure!(store::get(&connection, id, Direction::Incoming)?.is_none(), "indexed received batch is missing; refusing to silently replace externally removed content");
        fsutil::available_space(
            &self.home.join("inbox"),
            authenticated.envelope.ciphertext_bytes
                + authenticated.envelope.total_bytes.saturating_mul(2)
                + 8 * crate::format::MIB,
        )?;
        let staging = tempfile::Builder::new()
            .prefix("batch-")
            .tempdir_in(self.home.join("inbox/.staging"))?;
        let ciphertext = staging.path().join("ciphertext.age");
        let repaired = codec::reconstruct(
            &parcel,
            &authenticated.envelope.groups,
            &ciphertext,
            id,
            authenticated.envelope.ciphertext_bytes,
        )?;
        let plain_archive = staging.path().join("archive.tar");
        identity::decrypt_file(
            &ciphertext,
            &plain_archive,
            &unlocked.age,
            archive::MAX_ARCHIVE,
        )?;
        let batch = staging.path().join("batch");
        fsutil::private_dir(&batch)?;
        let (_, manifest_raw) = archive::unpack(
            &plain_archive,
            &batch.join("files"),
            &authenticated.envelope.manifest_sha256,
            authenticated.envelope.files,
            authenticated.envelope.total_bytes,
        )?;
        let metadata = batch.join(".relaypost");
        fsutil::private_dir(&metadata)?;
        fsutil::write_new(&metadata.join("envelope.json"), &authenticated.raw)?;
        fsutil::write_new(
            &metadata.join("envelope.minisig"),
            authenticated.signature.as_bytes(),
        )?;
        fsutil::write_new(&metadata.join("manifest.json"), &manifest_raw)?;
        fsutil::write_new(
            &metadata.join("prepared.json"),
            &serde_json::to_vec(&Prepared::for_envelope(
                &authenticated.envelope,
                &authenticated.raw,
            ))?,
        )?;
        fsutil::sync_tree(&batch)?;
        // Recheck every staged byte and authenticated metadata immediately
        // before granting commit authority to the published directory.
        self.verify_committed(&batch, &connection)?;
        fsutil::publish_noreplace(&batch, &destination)?;
        let record = self.finish_incoming(&connection, &authenticated, &unlocked.signing)?;
        Ok(ReceiveResult {
            record,
            repaired_shards: repaired,
            duplicate: false,
        })
    }

    pub fn export_receipt(&self, parcel_id: &str, carrier: &Path) -> Result<PathBuf> {
        let _guard = fsutil::lock(&self.home)?;
        fsutil::check_id(parcel_id)?;
        let connection = store::open(&self.home)?;
        let authenticated =
            match self.verify_committed(&self.home.join("inbox").join(parcel_id), &connection) {
                Ok(value) => value,
                Err(error) => {
                    self.mark_failed(&connection, parcel_id)?;
                    return Err(error)
                        .context("current received content cannot authorize receipt export");
                }
            };
        let source = self.home.join("receipts").join(parcel_id);
        let (raw, signature) = self.read_receipt(&source, &authenticated)?;
        let carrier = self.export_directory(carrier)?;
        let destination = carrier.join(format!("{parcel_id}.receipt"));
        if fs::symlink_metadata(&destination).is_ok() {
            let (existing_raw, existing_sig) = self.read_receipt(&destination, &authenticated)?;
            ensure!(
                raw == existing_raw && signature == existing_sig,
                "existing carrier receipt conflicts with immutable receipt"
            );
            return Ok(destination);
        }
        let staging = tempfile::Builder::new()
            .prefix(".relaypost-receipt-")
            .tempdir_in(&carrier)?;
        fsutil::write_new(&staging.path().join("receipt.json"), &raw)?;
        fsutil::write_new(
            &staging.path().join("receipt.minisig"),
            signature.as_bytes(),
        )?;
        fsutil::sync_tree(staging.path())?;
        self.read_receipt(staging.path(), &authenticated)?;
        fsutil::publish_noreplace(staging.path(), &destination)?;
        Ok(destination)
    }

    pub fn import_receipt(&self, receipt: &Path) -> Result<ParcelRecord> {
        let _guard = fsutil::lock(&self.home)?;
        let connection = store::open(&self.home)?;
        let receipt = fsutil::selected_directory(receipt)?;
        let raw = fsutil::read_bounded(&receipt.join("receipt.json"), MAX_ENVELOPE)?;
        let claimed: Receipt = fsutil::json(&raw)?;
        fsutil::check_id(&claimed.parcel_id)?;
        let mut record = store::get(&connection, &claimed.parcel_id, Direction::Outgoing)?
            .context("receipt has no matching outgoing delivery")?;
        let authenticated = self.load_envelope(
            &self.home.join("outbox").join(&claimed.parcel_id),
            &connection,
        )?;
        ensure!(
            authenticated.envelope.sender.fingerprint == self.card()?.fingerprint,
            "receipt is not for this sender"
        );
        let (verified_raw, signature) = self.read_receipt(&receipt, &authenticated)?;
        ensure!(verified_raw == raw, "receipt changed while importing");
        let destination = self.home.join("acknowledgments").join(&claimed.parcel_id);
        if fs::symlink_metadata(&destination).is_ok() {
            let (old_raw, old_sig) = self.read_receipt(&destination, &authenticated)?;
            ensure!(
                old_raw == verified_raw && old_sig == signature,
                "conflicting receipt already recorded"
            );
        } else {
            let staging = tempfile::Builder::new()
                .prefix(".import-")
                .tempdir_in(self.home.join("acknowledgments"))?;
            fsutil::write_new(&staging.path().join("receipt.json"), &verified_raw)?;
            fsutil::write_new(
                &staging.path().join("receipt.minisig"),
                signature.as_bytes(),
            )?;
            fsutil::sync_tree(staging.path())?;
            fsutil::publish_noreplace(staging.path(), &destination)?;
        }
        record.state = DeliveryState::Confirmed;
        record.receipt_path = Some(destination);
        record.updated_at = fsutil::now();
        store::save(&connection, &record)?;
        Ok(record)
    }

    pub fn deliveries(&self) -> Result<Vec<ParcelRecord>> {
        let _guard = fsutil::lock(&self.home)?;
        store::deliveries(&store::open(&self.home)?)
    }

    pub fn recover(&self, password: &str) -> Result<RecoverySummary> {
        let _guard = fsutil::lock(&self.home)?;
        let connection = store::open(&self.home)?;
        let unlocked = identity::unlock(&self.home.join("identity"), password)?;
        let mut summary = RecoverySummary {
            recovered: 0,
            pending: 0,
            conflicts: Vec::new(),
        };
        let inbox = self.home.join("inbox");
        let mut seen = BTreeSet::new();
        for entry in fs::read_dir(&inbox)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".staging" {
                continue;
            }
            if !fsutil::valid_digest(&name) {
                summary
                    .conflicts
                    .push(format!("unexpected inbox entry: {name}"));
                continue;
            }
            seen.insert(name.clone());
            let prior = store::get(&connection, &name, Direction::Incoming)?;
            let receipt_missing = !self.home.join("receipts").join(&name).try_exists()?;
            let result = (|| -> Result<()> {
                let authenticated = self.verify_committed(&entry.path(), &connection)?;
                ensure!(
                    authenticated.envelope.parcel_id == name,
                    "published directory ID mismatch"
                );
                // Re-establish persistence before rebuilding the index/receipt.
                fsutil::sync_tree(&entry.path())?;
                fsutil::sync_dir(&inbox)?;
                self.finish_incoming(&connection, &authenticated, &unlocked.signing)?;
                Ok(())
            })();
            match result {
                Ok(()) => {
                    if prior.is_none()
                        || receipt_missing
                        || prior
                            .as_ref()
                            .is_some_and(|r| r.state != DeliveryState::SavedReceiptReady)
                    {
                        summary.recovered += 1;
                    }
                }
                Err(error) => {
                    self.mark_failed(&connection, &name)?;
                    summary.conflicts.push(format!("{name}: {error:#}"));
                }
            }
        }
        for record in store::deliveries(&connection)? {
            if record.direction == Direction::Incoming && !seen.contains(&record.parcel_id) {
                self.mark_failed(&connection, &record.parcel_id)?;
                summary
                    .conflicts
                    .push(format!("{}: published batch is missing", record.parcel_id));
            }
        }
        for entry in fs::read_dir(self.home.join("inbox/.staging"))? {
            entry?;
            summary.pending += 1;
        }
        let mut seen_outgoing = BTreeSet::new();
        for entry in fs::read_dir(self.home.join("outbox"))? {
            let entry = entry?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if !fsutil::valid_digest(&id) {
                summary.pending += 1;
                continue;
            }
            seen_outgoing.insert(id.clone());
            let result = (|| -> Result<bool> {
                let authenticated = self.load_envelope(&entry.path(), &connection)?;
                ensure!(
                    authenticated.envelope.parcel_id == id
                        && authenticated.envelope.sender.fingerprint == self.card()?.fingerprint,
                    "outbox identity mismatch"
                );
                let health = codec::inspect(&entry.path(), &authenticated.envelope.groups)?;
                ensure!(
                    health.iter().all(|g| g.missing_shards.is_empty()),
                    "local immutable outgoing parcel is damaged"
                );
                let previous = store::get(&connection, &id, Direction::Outgoing)?;
                let was_missing = previous.is_none();
                let was_failed = previous
                    .as_ref()
                    .is_some_and(|record| record.state == DeliveryState::Failed);
                let mut record = previous.unwrap_or_else(|| {
                    self.outgoing_record(
                        &authenticated.envelope,
                        entry.path(),
                        DeliveryState::Ready,
                    )
                });
                if was_failed {
                    record.state = DeliveryState::Ready;
                    record.updated_at = fsutil::now();
                }
                if self.cancelled(&id)? && record.state != DeliveryState::Confirmed {
                    record.state = DeliveryState::Cancelled;
                }
                let ack = self.home.join("acknowledgments").join(&id);
                if ack.try_exists()? {
                    self.read_receipt(&ack, &authenticated)?;
                    record.state = DeliveryState::Confirmed;
                    record.receipt_path = Some(ack);
                }
                store::save(&connection, &record)?;
                Ok(was_missing || was_failed)
            })();
            match result {
                Ok(true) => summary.recovered += 1,
                Ok(false) => {}
                Err(error) => {
                    self.mark_outgoing_failed(&connection, &id)?;
                    summary.conflicts.push(format!("{id}: {error:#}"));
                }
            }
        }
        for record in store::deliveries(&connection)? {
            if record.direction == Direction::Outgoing && !seen_outgoing.contains(&record.parcel_id)
            {
                self.mark_outgoing_failed(&connection, &record.parcel_id)?;
                summary.conflicts.push(format!(
                    "{}: local outgoing parcel is missing",
                    record.parcel_id
                ));
            }
        }
        Ok(summary)
    }

    pub fn cancel(&self, parcel_id: &str) -> Result<()> {
        let _guard = fsutil::lock(&self.home)?;
        fsutil::check_id(parcel_id)?;
        let connection = store::open(&self.home)?;
        let mut record = store::get(&connection, parcel_id, Direction::Outgoing)?
            .context("outgoing delivery not found")?;
        ensure!(
            record.state != DeliveryState::Confirmed,
            "confirmed delivery cannot be cancelled"
        );
        let marker = self.home.join("cancelled").join(parcel_id);
        if !self.cancelled(parcel_id)? {
            fsutil::write_new(&marker, b"RelayPost local re-export cancelled v1\n")?;
            fsutil::sync_dir(&self.home.join("cancelled"))?;
        }
        record.state = DeliveryState::Cancelled;
        record.updated_at = fsutil::now();
        store::save(&connection, &record)
    }

    fn export_directory(&self, path: &Path) -> Result<PathBuf> {
        let mut ancestor = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        ensure!(
            !ancestor
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir)),
            "export paths cannot contain parent-directory components"
        );
        let mut suffix = Vec::new();
        loop {
            match fs::symlink_metadata(&ancestor) {
                Ok(metadata) => {
                    ensure!(
                        !suffix.is_empty() || !metadata.file_type().is_symlink(),
                        "export directory cannot be a symbolic link"
                    );
                    ensure!(
                        fs::metadata(&ancestor)?.is_dir(),
                        "export ancestor is not a directory"
                    );
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    suffix.push(
                        ancestor
                            .file_name()
                            .context("export path has no existing ancestor")?
                            .to_os_string(),
                    );
                    ensure!(ancestor.pop(), "export path has no existing ancestor");
                }
                Err(error) => return Err(error.into()),
            }
        }
        let mut canonical = fs::canonicalize(ancestor)?;
        for component in suffix.into_iter().rev() {
            canonical.push(component);
        }
        ensure!(
            !canonical.starts_with(&self.home),
            "export destination must be outside the managed profile home"
        );
        fsutil::output_directory(&canonical)
    }

    fn card(&self) -> Result<PeerCard> {
        identity::public_card(&self.home.join("identity"))
    }

    fn cancelled(&self, id: &str) -> Result<bool> {
        let path = self.home.join("cancelled").join(id);
        match fs::symlink_metadata(path) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    fn authenticate(
        &self,
        raw: Vec<u8>,
        signature: String,
        connection: &Connection,
    ) -> Result<AuthenticatedEnvelope> {
        ensure!(raw.len() as u64 <= MAX_ENVELOPE, "envelope exceeds bound");
        let envelope: Envelope = fsutil::json(&raw)?;
        envelope.validate()?;
        identity::verify_card(&envelope.sender)?;
        identity::verify_card(&envelope.recipient)?;
        let own = self.card()?;
        if envelope.recipient.fingerprint == own.fingerprint {
            ensure!(
                envelope.recipient.age_recipient == own.age_recipient
                    && envelope.recipient.signing_public_key == own.signing_public_key,
                "recipient key binding mismatch"
            );
            store::pinned(connection, &envelope.sender)?;
        } else if envelope.sender.fingerprint == own.fingerprint {
            ensure!(
                envelope.sender.age_recipient == own.age_recipient
                    && envelope.sender.signing_public_key == own.signing_public_key,
                "sender key binding mismatch"
            );
            store::pinned(connection, &envelope.recipient)?;
        } else {
            bail!("parcel is not addressed to or sent by this identity");
        }
        identity::verify(&envelope.sender.signing_public_key, &raw, &signature)?;
        Ok(AuthenticatedEnvelope {
            envelope,
            raw,
            signature,
        })
    }

    fn load_envelope(
        &self,
        parcel: &Path,
        connection: &Connection,
    ) -> Result<AuthenticatedEnvelope> {
        fsutil::directory(parcel)?;
        fsutil::no_symlink_components(parcel)?;
        let mut accepted: Option<AuthenticatedEnvelope> = None;
        let mut errors = Vec::new();
        for (json, sig) in ENVELOPE_COPIES {
            let result = (|| -> Result<AuthenticatedEnvelope> {
                let raw = fsutil::read_bounded(&parcel.join(json), MAX_ENVELOPE)?;
                let signature = fsutil::read_bounded(&parcel.join(sig), 2048)?;
                self.authenticate(raw, fsutil::utf8(&signature)?.to_owned(), connection)
            })();
            match result {
                Ok(value) => {
                    if let Some(existing) = &accepted {
                        ensure!(
                            value.raw == existing.raw && value.signature == existing.signature,
                            "conflicting valid signed envelope copies"
                        );
                    } else {
                        accepted = Some(value);
                    }
                }
                Err(error) => errors.push(format!("{json}: {error:#}")),
            }
        }
        accepted
            .ok_or_else(|| anyhow::anyhow!("no valid trusted envelope copy: {}", errors.join("; ")))
    }

    fn verify_committed(
        &self,
        batch: &Path,
        connection: &Connection,
    ) -> Result<AuthenticatedEnvelope> {
        fsutil::directory(batch)?;
        self.exact_entries(batch, &["files", ".relaypost"])?;
        let metadata = batch.join(".relaypost");
        fsutil::directory(&metadata)?;
        self.exact_entries(
            &metadata,
            &[
                "envelope.json",
                "envelope.minisig",
                "manifest.json",
                "prepared.json",
            ],
        )?;
        let raw = fsutil::read_bounded(&metadata.join("envelope.json"), MAX_ENVELOPE)?;
        let signature = fsutil::read_bounded(&metadata.join("envelope.minisig"), 2048)?;
        let authenticated =
            self.authenticate(raw, fsutil::utf8(&signature)?.to_owned(), connection)?;
        ensure!(
            authenticated.envelope.recipient.fingerprint == self.card()?.fingerprint,
            "committed batch belongs to another recipient"
        );
        let prepared: Prepared = fsutil::json(&fsutil::read_bounded(
            &metadata.join("prepared.json"),
            MAX_ENVELOPE,
        )?)?;
        prepared.verify(&authenticated.envelope, &authenticated.raw)?;
        let manifest_raw = fsutil::read_bounded(&metadata.join("manifest.json"), MAX_MANIFEST)?;
        ensure!(
            fsutil::digest(&manifest_raw) == authenticated.envelope.manifest_sha256,
            "published manifest conflicts with authenticated envelope"
        );
        let manifest: Manifest = fsutil::json(&manifest_raw)?;
        ensure!(
            manifest.files.len() == authenticated.envelope.files
                && manifest.total_bytes == authenticated.envelope.total_bytes,
            "published file-list bounds mismatch"
        );
        archive::verify_files(&batch.join("files"), &manifest)?;
        Ok(authenticated)
    }

    fn exact_entries(&self, path: &Path, expected: &[&str]) -> Result<()> {
        let mut names = BTreeSet::new();
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF8 metadata entry"))?;
            ensure!(
                expected.contains(&name.as_str()),
                "unexpected published entry: {name}"
            );
            let meta = fs::symlink_metadata(entry.path())?;
            ensure!(
                !meta.file_type().is_symlink(),
                "published metadata link refused"
            );
            names.insert(name);
        }
        ensure!(
            names.len() == expected.len(),
            "published batch is missing required entries"
        );
        Ok(())
    }

    fn read_receipt(
        &self,
        directory: &Path,
        authenticated: &AuthenticatedEnvelope,
    ) -> Result<(Vec<u8>, String)> {
        fsutil::directory(directory)?;
        self.exact_entries(directory, &["receipt.json", "receipt.minisig"])?;
        let raw = fsutil::read_bounded(&directory.join("receipt.json"), MAX_ENVELOPE)?;
        let signature = fsutil::read_bounded(&directory.join("receipt.minisig"), 2048)?;
        let signature = fsutil::utf8(&signature)?.to_owned();
        let receipt: Receipt = fsutil::json(&raw)?;
        receipt.validate(&authenticated.envelope, &authenticated.raw)?;
        identity::verify(
            &authenticated.envelope.recipient.signing_public_key,
            &raw,
            &signature,
        )?;
        Ok((raw, signature))
    }

    fn finish_incoming(
        &self,
        connection: &Connection,
        authenticated: &AuthenticatedEnvelope,
        signing: &minisign::SecretKey,
    ) -> Result<ParcelRecord> {
        let envelope = &authenticated.envelope;
        let destination = self.home.join("inbox").join(&envelope.parcel_id);
        // The verified filesystem commit authorizes the durable receipt. Only
        // then publish SavedReceiptReady in the index; recover can reconstruct
        // either missing artifact from the authenticated committed batch.
        let now = fsutil::now();
        let mut record = ParcelRecord {
            parcel_id: envelope.parcel_id.clone(),
            direction: Direction::Incoming,
            state: DeliveryState::Ready,
            sender: envelope.sender.fingerprint.clone(),
            recipient: envelope.recipient.fingerprint.clone(),
            files: envelope.files,
            total_bytes: envelope.total_bytes,
            parcel_path: None,
            received_path: Some(destination.join("files")),
            receipt_path: None,
            created_at: envelope.created_at.clone(),
            updated_at: now.clone(),
        };
        let receipt_dir = self.home.join("receipts").join(&envelope.parcel_id);
        if fs::symlink_metadata(&receipt_dir).is_ok() {
            self.read_receipt(&receipt_dir, authenticated)?;
        } else {
            // Reverify immediately before signing. An existing directory alone
            // or a database state can never authorize a successful receipt.
            let verified = self.verify_committed(&destination, connection)?;
            ensure!(
                verified.raw == authenticated.raw && verified.signature == authenticated.signature,
                "commit envelope changed before receipt"
            );
            let receipt = Receipt {
                kind: "relaypost.receipt".into(),
                schema: 1,
                parcel_id: envelope.parcel_id.clone(),
                envelope_sha256: fsutil::digest(&authenticated.raw),
                manifest_sha256: envelope.manifest_sha256.clone(),
                recipient: envelope.recipient.fingerprint.clone(),
                signing_public_key: envelope.recipient.signing_public_key.clone(),
                result: "committed".into(),
                committed_at: now,
            };
            let raw = serde_json::to_vec(&receipt)?;
            let signature = identity::sign(signing, &raw)?;
            let staging = tempfile::Builder::new()
                .prefix(".receipt-")
                .tempdir_in(self.home.join("receipts"))?;
            fsutil::write_new(&staging.path().join("receipt.json"), &raw)?;
            fsutil::write_new(
                &staging.path().join("receipt.minisig"),
                signature.as_bytes(),
            )?;
            fsutil::sync_tree(staging.path())?;
            self.read_receipt(staging.path(), authenticated)?;
            fsutil::publish_noreplace(staging.path(), &receipt_dir)?;
        }
        record.state = DeliveryState::SavedReceiptReady;
        record.receipt_path = Some(receipt_dir);
        record.updated_at = fsutil::now();
        store::save(connection, &record)?;
        Ok(record)
    }

    fn outgoing_record(
        &self,
        envelope: &Envelope,
        path: PathBuf,
        state: DeliveryState,
    ) -> ParcelRecord {
        ParcelRecord {
            parcel_id: envelope.parcel_id.clone(),
            direction: Direction::Outgoing,
            state,
            sender: envelope.sender.fingerprint.clone(),
            recipient: envelope.recipient.fingerprint.clone(),
            files: envelope.files,
            total_bytes: envelope.total_bytes,
            parcel_path: Some(path),
            received_path: None,
            receipt_path: None,
            created_at: envelope.created_at.clone(),
            updated_at: fsutil::now(),
        }
    }

    fn mark_outgoing_failed(&self, connection: &Connection, parcel_id: &str) -> Result<()> {
        if let Some(mut record) = store::get(connection, parcel_id, Direction::Outgoing)? {
            // Confirmation and cancellation describe historical facts. Damage
            // to a retained local copy does not erase either fact.
            if !matches!(
                record.state,
                DeliveryState::Confirmed | DeliveryState::Cancelled
            ) {
                let state = if self.cancelled(parcel_id)? {
                    DeliveryState::Cancelled
                } else {
                    DeliveryState::Failed
                };
                if record.state != state {
                    record.state = state;
                    record.updated_at = fsutil::now();
                    store::save(connection, &record)?;
                }
            }
        }
        Ok(())
    }

    fn mark_failed(&self, connection: &Connection, parcel_id: &str) -> Result<()> {
        if let Some(mut record) = store::get(connection, parcel_id, Direction::Incoming)? {
            record.state = DeliveryState::Failed;
            record.updated_at = fsutil::now();
            store::save(connection, &record)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_malformed_duplicate_and_conflicting_envelopes_are_refused() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let root = fs::canonicalize(temporary.path())?;
        let alice = Profile::open(root.join("alice"))?;
        let bob = Profile::open(root.join("bob"))?;
        let alice_card = alice.initialize("Synthetic Alice", "synthetic-unit-password")?;
        let bob_card = bob.initialize("Synthetic Bob", "synthetic-unit-password")?;
        let a_card = root.join("alice.json");
        let b_card = root.join("bob.json");
        alice.export_card(&a_card)?;
        bob.export_card(&b_card)?;
        alice.import_contact(&b_card)?;
        bob.import_contact(&a_card)?;
        alice.trust_contact(&bob_card.fingerprint)?;
        bob.trust_contact(&alice_card.fingerprint)?;
        let input = root.join("synthetic.txt");
        fsutil::write_new(&input, b"synthetic hostile-format fixture")?;
        let record = alice.seal(&[input], &bob_card.fingerprint, "synthetic-unit-password")?;
        let parcel = record.parcel_path.context("fixture parcel missing")?;
        let raw = fs::read(parcel.join("envelope.json"))?;
        let valid: Envelope = fsutil::json(&raw)?;
        let signing = identity::unlock(&alice.home.join("identity"), "synthetic-unit-password")?;
        let replace_copy = |json: &str, signature: &str, bytes: &[u8]| -> Result<()> {
            fs::write(parcel.join(json), bytes)?;
            fs::write(
                parcel.join(signature),
                identity::sign(&signing.signing, bytes)?.as_bytes(),
            )?;
            Ok(())
        };
        for mutation in 0..6 {
            let mut invalid = valid.clone();
            match mutation {
                0 => invalid.schema = 2,
                1 => invalid.ciphertext_bytes = crate::format::MAX_CIPHERTEXT + 1,
                2 => invalid.groups[0].shard_bytes = crate::format::MIB as usize + 1,
                3 => invalid.groups[0].shards[0].path = "../escape".into(),
                4 => invalid.groups[0].parity_shards += 1,
                _ => invalid.groups.clear(),
            }
            let invalid = serde_json::to_vec(&invalid)?;
            for (json, signature) in ENVELOPE_COPIES {
                replace_copy(json, signature, &invalid)?;
            }
            assert!(bob.inspect_parcel(&parcel).is_err());
        }
        let duplicate = format!(
            "{{\"schema\":1,{}",
            fsutil::utf8(&raw)?
                .strip_prefix('{')
                .context("object fixture")?
        );
        for (json, signature) in ENVELOPE_COPIES {
            replace_copy(json, signature, duplicate.as_bytes())?;
        }
        assert!(bob.inspect_parcel(&parcel).is_err());
        replace_copy("envelope.json", "envelope.minisig", &raw)?;
        let mut conflict = valid.clone();
        conflict.created_at = "2026-10-31T00:00:00.000Z".into();
        let conflict = serde_json::to_vec(&conflict)?;
        assert_ne!(raw, conflict);
        replace_copy("envelope-copy.json", "envelope-copy.minisig", &conflict)?;
        assert!(bob.inspect_parcel(&parcel).is_err());
        // One authenticated copy remains sufficient if the other is damaged.
        fs::write(
            parcel.join("envelope-copy.minisig"),
            b"damaged synthetic signature",
        )?;
        assert!(bob.inspect_parcel(&parcel)?.recoverable);
        Ok(())
    }
}
