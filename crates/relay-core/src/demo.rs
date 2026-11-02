//! Guided demo engine — runs the real `Profile` API on two synthetic profiles
//! inside a fresh private root directory, exercising every phase of the offline
//! delivery lifecycle on a single machine.
//!
//! # Secret-material assertion
//!
//! `DemoSnapshot` contains no secret material. All fields derive only from
//! `Profile` public API return types (`PeerCard`, `ParcelRecord`, `Inspection`,
//! `ReceiveResult`, `DeliveryState`) — none of which carry passwords, private
//! keys, age identity strings, or Minisign secret keys.
//!
//! The demo fixture passwords (`DEMO_PASSWORD_ALICE`, `DEMO_PASSWORD_BOB`) are
//! clearly labelled constants used only inside this module. They never appear in
//! any `DemoSnapshot` field or serialized JSON report.

use crate::{DeliveryState, Direction, Inspection, Profile};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Synthetic fixture constants — NOT secrets, purely synthetic demo fixtures.
// Never appear in report JSON or any public-facing output.
// ---------------------------------------------------------------------------
const DEMO_PASSWORD_ALICE: &str = "synthetic-demo-alice-v1";
const DEMO_PASSWORD_BOB: &str = "synthetic-demo-bob-v1";
const DEMO_NAME_ALICE: &str = "Demo Alice";
const DEMO_NAME_BOB: &str = "Demo Bob";
const DEMO_FILE_A_BYTES: usize = 20 * 1024 * 1024; // 20 MiB → full group 0
const DEMO_FILE_B_BYTES: usize = 1024 * 1024; //  1 MiB → tail group 1

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Phase of the guided demo. Transitions are strictly forward-only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DemoPhase {
    /// Profiles created, parcel sealed and exported to carrier,
    /// initial inspection stored.
    Prepared,
    /// Four shards permanently damaged (data 0,1 deleted; parity 20,21 XOR'd).
    /// Five-erasure `receive_parcel` refused with error. Fifth shard restored.
    /// Group 0 recoverable again.
    Damaged,
    /// Bob received and committed the parcel. Four shards repaired.
    /// Source and received SHA-256 hashes compared.
    Received,
    /// Bob's signed receipt imported by Alice.
    /// `sender_state == DeliveryState::Confirmed`.
    Confirmed,
}

/// Public-facts snapshot. Contains no secret material — fields derive only
/// from `Profile` public API return types (`PeerCard`, `ParcelRecord`,
/// `Inspection`, `ReceiveResult`, `DeliveryState`), none of which carry
/// passwords, private keys, age identity strings, or Minisign secret keys.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DemoSnapshot {
    /// Current phase.
    pub phase: DemoPhase,
    /// Absolute canonical path of the demo root directory.
    pub root: PathBuf,
    /// Absolute path of the carrier parcel directory. `None` before `Prepared`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub carrier_parcel_path: Option<PathBuf>,
    /// SHA-256 hex strings of source files, keyed by filename.
    #[serde(default)]
    pub source_sha256: BTreeMap<String, String>,
    /// SHA-256 hex strings of received files, keyed by filename. Populated at `Received`.
    #[serde(default)]
    pub received_sha256: BTreeMap<String, String>,
    /// Inspection immediately after export (group 0 fully healthy).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_inspection: Option<Inspection>,
    /// Inspection after 4 permanent erasures (group 0 recoverable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inspection_after_damage: Option<Inspection>,
    /// Inspection with 5th erasure present (group 0 NOT recoverable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub five_erasure_refusal_evidence: Option<Inspection>,
    /// Error string from the refused `receive_parcel` call (5 erasures).
    /// Must contain the repair-budget error substring.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub five_erasure_receive_error: Option<String>,
    /// Permanently damaged shard indices in group 0 (`[0, 1, 20, 21]`).
    #[serde(default)]
    pub damaged_shard_indices: Vec<usize>,
    /// Alice's outgoing delivery state. `None` before `Prepared`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_state: Option<DeliveryState>,
    /// Shards repaired by `receive_parcel`. `None` before `Received`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repaired_shards: Option<usize>,
    /// Absolute path of Bob's committed batch files directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub received_path: Option<PathBuf>,
    /// Absolute path of the exported receipt directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_path: Option<PathBuf>,
    /// Absolute path of the written JSON report file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_path: Option<PathBuf>,
}

/// Stateful guided demo session. Owns both `Profile` handles and all session
/// state. Must not be serialized (contains profile paths that encode no secrets
/// but are opaque to the snapshot DTO).
pub struct DemoSession {
    root: PathBuf,
    alice: Profile,
    bob: Profile,
    parcel_id: String,
    snapshot: DemoSnapshot,
}

// ---------------------------------------------------------------------------
// Synthetic data helpers
// ---------------------------------------------------------------------------

/// Deterministic 20 MiB fill — upper byte of LCG with Knuth's multiplier.
/// Uses `u64` indices and wrapping arithmetic; never panics in debug mode.
fn synthetic_file_a() -> Vec<u8> {
    (0u64..DEMO_FILE_A_BYTES as u64)
        .map(|i| (i.wrapping_mul(6_364_136_223_846_793_005_u64) >> 56) as u8)
        .collect()
}

/// Deterministic 1 MiB fill — second LCG variant.
fn synthetic_file_b() -> Vec<u8> {
    (0u64..DEMO_FILE_B_BYTES as u64)
        .map(|i| {
            (i.wrapping_add(1)
                .wrapping_mul(2_862_933_555_777_941_757_u64)
                >> 56) as u8
        })
        .collect()
}

/// SHA-256 of `bytes` as a lowercase hex string.
fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// SHA-256 of the file at `path` read via `fsutil::read_bounded`.
/// Bound: 32 MiB (larger than any single source file).
fn sha256_file(path: &Path) -> Result<String> {
    let bytes = crate::fsutil::read_bounded(path, 32 * crate::format::MIB)?;
    Ok(sha256_hex(&bytes))
}

// ---------------------------------------------------------------------------
// Shard path helpers
// ---------------------------------------------------------------------------

fn carrier_shard_path(carrier_parcel: &Path, group: usize, shard: usize) -> PathBuf {
    carrier_parcel.join(crate::format::shard_path(group, shard))
}

fn outbox_shard_path(root: &Path, parcel_id: &str, group: usize, shard: usize) -> PathBuf {
    root.join("alice")
        .join("outbox")
        .join(parcel_id)
        .join(crate::format::shard_path(group, shard))
}

/// A paused demonstration must not mutate a linked external file. The profile
/// directory is trusted; concurrent malicious filesystem races are excluded.
fn check_shard(path: &Path) -> Result<()> {
    crate::fsutil::no_symlink_components(path)?;
    let metadata = fs::symlink_metadata(path)?;
    ensure!(metadata.is_file(), "demo shard must be a regular file");
    ensure!(metadata.len() <= crate::format::MIB, "demo shard exceeds its size limit");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(metadata.nlink() == 1, "demo refuses hard-linked shards");
    }
    Ok(())
}

/// Replace the carrier entry, never write through its existing inode.
fn replace_shard(path: &Path, bytes: &[u8]) -> Result<()> {
    check_shard(path)?;
    let parent = path.parent().context("demo shard has no parent")?;
    let mut staged = tempfile::Builder::new().prefix(".demo-shard-").tempfile_in(parent)?;
    staged.write_all(bytes)?;
    staged.as_file().sync_all()?;
    staged.persist(path).map_err(|error| error.error)
        .with_context(|| format!("replace demo carrier shard {}", path.display()))?;
    crate::fsutil::sync_dir(parent)
}

/// Make a bounded, hash-bad carrier copy without mutating linked file contents.
fn xor_shard(path: &Path) -> Result<()> {
    check_shard(path)?;
    let mut bytes = crate::fsutil::read_bounded(path, crate::format::MIB)?;
    for byte in &mut bytes {
        *byte ^= 0xFF;
    }
    replace_shard(path, &bytes)
}

/// Delete a shard file from the carrier (missing erasure).
fn delete_shard(path: &Path) -> Result<()> {
    check_shard(path)?;
    fs::remove_file(path).with_context(|| format!("delete shard {}", path.display()))?;
    crate::fsutil::sync_dir(path.parent().context("demo shard has no parent")?)
}

/// Restore `carrier_path` from Alice's immutable outbox copy.
fn restore_shard(outbox_path: &Path, carrier_path: &Path) -> Result<()> {
    check_shard(outbox_path)?;
    let bytes = crate::fsutil::read_bounded(outbox_path, crate::format::MIB)?;
    replace_shard(carrier_path, &bytes)
}

// ---------------------------------------------------------------------------
// DemoSession implementation
// ---------------------------------------------------------------------------

impl DemoSession {
    /// Create a fresh demo root, initialize both profiles, exchange cards,
    /// seal the 21 MiB synthetic parcel, export it to the carrier, and store
    /// the initial inspection.
    ///
    /// `root` must not already exist as any filesystem object. Returns `Err`
    /// if the path already exists or any core operation fails.
    pub fn start(root: impl Into<PathBuf>) -> Result<Self> {
        let root: PathBuf = root.into();

        // --- 1. Atomically create the root directory ----------------------
        // DirBuilder::create uses create-new semantics on a fresh path; the
        // parent (OS temp dir) must already exist. If root already exists as
        // a file, directory, or symlink, create() returns Err.
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&root)
            .with_context(|| format!("demo root already exists or cannot be created: {}", root.display()))?;
        let root = fs::canonicalize(&root)
            .with_context(|| format!("cannot canonicalize demo root {}", root.display()))?;

        // --- 2. Initialize profiles ---------------------------------------
        let alice = Profile::open(root.join("alice"))?;
        let bob = Profile::open(root.join("bob"))?;

        let alice_card = alice.initialize(DEMO_NAME_ALICE, DEMO_PASSWORD_ALICE)?;
        let bob_card = bob.initialize(DEMO_NAME_BOB, DEMO_PASSWORD_BOB)?;

        // --- 3. Export contact cards and exchange them --------------------
        let alice_card_path = root.join("alice-card.json");
        let bob_card_path = root.join("bob-card.json");
        alice.export_card(&alice_card_path)?;
        bob.export_card(&bob_card_path)?;

        alice.import_contact(&bob_card_path)?;
        alice.trust_contact(&bob_card.fingerprint)?;
        bob.import_contact(&alice_card_path)?;
        bob.trust_contact(&alice_card.fingerprint)?;

        // --- 4. Generate synthetic source files ---------------------------
        let source_dir = root.join("source_files");
        crate::fsutil::private_dir(&source_dir)?;

        let file_a_data = synthetic_file_a();
        let file_b_data = synthetic_file_b();

        let file_a_sha256 = sha256_hex(&file_a_data);
        let file_b_sha256 = sha256_hex(&file_b_data);

        let file_a_path = source_dir.join("file_a.bin");
        let file_b_path = source_dir.join("file_b.bin");
        crate::fsutil::write_new(&file_a_path, &file_a_data)?;
        crate::fsutil::write_new(&file_b_path, &file_b_data)?;

        // Verify round-trip through disk.
        let disk_a = sha256_file(&file_a_path)?;
        let disk_b = sha256_file(&file_b_path)?;
        if disk_a != file_a_sha256 {
            bail!("file_a.bin SHA-256 mismatch after writing to disk");
        }
        if disk_b != file_b_sha256 {
            bail!("file_b.bin SHA-256 mismatch after writing to disk");
        }

        let mut source_sha256 = BTreeMap::new();
        source_sha256.insert("file_a.bin".to_owned(), file_a_sha256);
        source_sha256.insert("file_b.bin".to_owned(), file_b_sha256);

        // --- 5. Seal the parcel -------------------------------------------
        let record =
            alice.seal(&[file_a_path, file_b_path], &bob_card.fingerprint, DEMO_PASSWORD_ALICE)?;
        let parcel_id = record.parcel_id.clone();

        // --- 6. Export to carrier ----------------------------------------
        let carrier_parcel_path = alice.export_parcel(&parcel_id, &root.join("carrier"))?;

        // --- 7. Initial inspection (must be fully healthy) ----------------
        let initial_inspection = alice.inspect_parcel(&carrier_parcel_path)?;
        if !initial_inspection.recoverable {
            bail!("initial inspection reports parcel not recoverable — unexpected");
        }
        {
            let g0 = initial_inspection
                .groups
                .first()
                .context("initial inspection has no groups")?;
            if g0.data_shards != 20 {
                bail!(
                    "initial inspection group 0 has data_shards={}, expected 20",
                    g0.data_shards
                );
            }
            if g0.parity_shards != 4 {
                bail!(
                    "initial inspection group 0 has parity_shards={}, expected 4",
                    g0.parity_shards
                );
            }
            if !g0.missing_shards.is_empty() {
                bail!(
                    "initial inspection group 0 has {} missing shard(s), expected 0: {:?}",
                    g0.missing_shards.len(),
                    g0.missing_shards
                );
            }
        }

        // --- 8. Determine sender_state from Alice's actual deliveries -----
        let sender_state = alice
            .deliveries()?
            .into_iter()
            .find(|r| r.parcel_id == parcel_id && r.direction == Direction::Outgoing)
            .map(|r| r.state);
        if sender_state.as_ref() != Some(&DeliveryState::WaitingReceipt) {
            bail!(
                "expected Alice's delivery state WaitingReceipt after export, got {:?}",
                sender_state
            );
        }

        let snapshot = DemoSnapshot {
            phase: DemoPhase::Prepared,
            root: root.clone(),
            carrier_parcel_path: Some(carrier_parcel_path),
            source_sha256,
            received_sha256: BTreeMap::new(),
            initial_inspection: Some(initial_inspection),
            inspection_after_damage: None,
            five_erasure_refusal_evidence: None,
            five_erasure_receive_error: None,
            damaged_shard_indices: Vec::new(),
            sender_state,
            repaired_shards: None,
            received_path: None,
            receipt_path: None,
            report_path: None,
        };

        Ok(DemoSession {
            root,
            alice,
            bob,
            parcel_id,
            snapshot,
        })
    }

    /// Return a copy of the current snapshot without advancing.
    pub fn snapshot(&self) -> DemoSnapshot {
        self.snapshot.clone()
    }

    /// Advance to the next phase.
    ///
    /// - `Prepared` → `Damaged`: damage 4 shards, prove 5-erasure receive
    ///   refusal, restore 5th shard.
    /// - `Damaged` → `Received`: receive with repair, verify hashes.
    /// - `Received` → `Confirmed`: export + import receipt.
    /// - `Confirmed` → error.
    ///
    /// On error the snapshot keeps the last completed phase. Carrier changes
    /// may already exist; start a fresh run instead of assuming a clean retry.
    pub fn advance(&mut self) -> Result<DemoSnapshot> {
        match self.snapshot.phase {
            DemoPhase::Prepared => self.advance_to_damaged(),
            DemoPhase::Damaged => self.advance_to_received(),
            DemoPhase::Received => self.advance_to_confirmed(),
            DemoPhase::Confirmed => bail!("demo is already complete; advance cannot be called again"),
        }
    }

    /// Serialize the current snapshot as pretty JSON to `<root>/demo-report.json`.
    /// May only be called when phase is `Confirmed`. Returns `Err` if called
    /// earlier, or if the file already exists (idempotent-once semantics).
    pub fn export_report(&mut self) -> Result<PathBuf> {
        if self.snapshot.phase != DemoPhase::Confirmed {
            bail!("export_report is only allowed after the demo reaches the Confirmed phase");
        }
        let report_path = self.root.join("demo-report.json");
        if self.snapshot.report_path.is_some() {
            bail!("report already exported to {}", report_path.display());
        }
        // Set report_path in snapshot before serializing so the JSON includes it.
        self.snapshot.report_path = Some(report_path.clone());
        let json = serde_json::to_vec_pretty(&self.snapshot)
            .context("failed to serialize demo snapshot")?;
        // write_new fails if the file already exists — preserves original on
        // any concurrent race.
        if let Err(err) = crate::fsutil::write_new(&report_path, &json) {
            // Revert the field so the caller can retry with a fresh path if desired.
            self.snapshot.report_path = None;
            return Err(err).context("failed to write demo report");
        }
        Ok(report_path)
    }

    // -----------------------------------------------------------------------
    // Phase transition: Prepared → Damaged
    // -----------------------------------------------------------------------
    fn advance_to_damaged(&mut self) -> Result<DemoSnapshot> {
        let carrier_parcel_path = self
            .snapshot
            .carrier_parcel_path
            .as_ref()
            .context("carrier parcel path missing")?
            .clone();
        let parcel_id = &self.parcel_id;
        let root = &self.root;

        // -- Permanently damage 4 shards in carrier only -------------------
        //
        // Data shards 0 and 1: DELETE the file entirely (missing erasure).
        // Parity shards 20 and 21: XOR 0xFF (hash-bad erasure — file present
        // but content fails SHA-256 check).
        //
        // The test suite verifies:
        //   index < 20  → path must NOT exist (deleted data shards)
        //   index >= 20 → path IS a file      (corrupted parity shards)
        for &idx in &[0usize, 1] {
            let carrier = carrier_shard_path(&carrier_parcel_path, 0, idx);
            delete_shard(&carrier)
                .with_context(|| format!("delete data shard {idx}"))?;
        }
        for &idx in &[20usize, 21] {
            let carrier = carrier_shard_path(&carrier_parcel_path, 0, idx);
            xor_shard(&carrier)
                .with_context(|| format!("corrupt parity shard {idx}"))?;
        }

        // -- Inspect: expect 4 erasures, recoverable -----------------------
        let inspection_after_damage = self.bob.inspect_parcel(&carrier_parcel_path)?;
        {
            let g0 = inspection_after_damage
                .groups
                .first()
                .context("no groups in inspection after damage")?;
            if g0.missing_shards.len() != 4 {
                bail!(
                    "expected 4 missing shards after damage, got {}",
                    g0.missing_shards.len()
                );
            }
            if !g0.recoverable {
                bail!("group 0 is not recoverable after 4 erasures — unexpected");
            }
        }

        // -- Temporarily damage shard 2 (5th erasure, data) ---------------
        // XOR shard 2 so it becomes a hash-bad erasure (file remains present).
        let carrier_shard2 = carrier_shard_path(&carrier_parcel_path, 0, 2);
        xor_shard(&carrier_shard2).context("corrupt shard 2 (5th erasure)")?;

        // -- Inspect: expect 5 erasures, NOT recoverable -------------------
        let five_erasure_refusal_evidence = self.bob.inspect_parcel(&carrier_parcel_path)?;
        {
            let g0 = five_erasure_refusal_evidence
                .groups
                .first()
                .context("no groups in 5-erasure inspection")?;
            if g0.missing_shards.len() != 5 {
                bail!(
                    "expected 5 missing shards for 5-erasure evidence, got {}",
                    g0.missing_shards.len()
                );
            }
            if g0.recoverable {
                bail!("group 0 is still marked recoverable with 5 erasures — unexpected");
            }
        }

        // -- Attempt receive with 5 erasures — must fail -------------------
        let receive_err = match self.bob.receive_parcel(&carrier_parcel_path, DEMO_PASSWORD_BOB) {
            Err(e) => e,
            Ok(_) => {
                bail!("five-erasure receive_parcel unexpectedly succeeded; demo cannot claim refusal");
            }
        };
        let five_erasure_receive_error = format!("{receive_err:#}");

        // Verify the error is genuinely a repair-budget error, not a
        // password or filesystem error.
        if !five_erasure_receive_error.contains("exceeds repair budget") {
            bail!(
                "five-erasure receive error does not contain 'exceeds repair budget': {five_erasure_receive_error}"
            );
        }

        // Verify no inbox directory was created for this parcel.
        let bob_inbox_parcel = root.join("bob").join("inbox").join(parcel_id.as_str());
        if bob_inbox_parcel.try_exists()? {
            bail!(
                "Bob inbox directory exists after refused receive — unexpected commit: {}",
                bob_inbox_parcel.display()
            );
        }

        // Verify no incoming delivery record was created.
        let bob_incoming = self
            .bob
            .deliveries()?
            .into_iter()
            .any(|r| r.parcel_id == *parcel_id && r.direction == Direction::Incoming);
        if bob_incoming {
            bail!("Bob has an incoming delivery record after refused receive — unexpected");
        }

        // -- Restore shard 2 from Alice's immutable outbox -----------------
        // Shard 2 was XOR'd; restore from the untouched outbox copy.
        //
        // Before restoring, verify that no receipt exists for this parcel
        // in Bob's receipts directory, and that export_receipt returns Err
        // (Bob has no committed batch to sign). If export_receipt unexpectedly
        // succeeds, we cannot trust the refusal demonstration.
        let bob_receipt_path = root
            .join("bob")
            .join("receipts")
            .join(parcel_id.as_str());
        if bob_receipt_path.try_exists()? {
            bail!(
                "Bob receipts directory for parcel exists before restore — state falsely claims Saved: {}",
                bob_receipt_path.display()
            );
        }
        let refused_probe_dir = root.join("refused-receipt-probe");
        match self.bob.export_receipt(parcel_id, &refused_probe_dir) {
            Ok(_) => {
                bail!(
                    "Bob export_receipt unexpectedly succeeded before receiving the parcel — \
                     five-erasure refusal demonstration is invalid"
                );
            }
            Err(_) => {
                // Expected: Bob has no committed receipt to export.
            }
        }
        // Confirm the probe did not create a carrier.
        if refused_probe_dir.try_exists()? {
            bail!(
                "export_receipt created a directory despite returning Err: {}",
                refused_probe_dir.display()
            );
        }

        let outbox_shard2 = outbox_shard_path(root, parcel_id, 0, 2);
        restore_shard(&outbox_shard2, &carrier_shard2)
            .context("restore shard 2 from Alice outbox")?;

        // -- Final confirm: 4 erasures (0,1,20,21), recoverable ------------
        let post_restore = self.bob.inspect_parcel(&carrier_parcel_path)?;
        {
            let g0 = post_restore
                .groups
                .first()
                .context("no groups in post-restore inspection")?;
            if !g0.recoverable {
                bail!("group 0 is not recoverable after restoring shard 2 — unexpected");
            }
            let expected_missing: Vec<usize> = vec![0, 1, 20, 21];
            if g0.missing_shards != expected_missing {
                bail!(
                    "post-restore group 0 missing_shards expected {:?}, got {:?}",
                    expected_missing,
                    g0.missing_shards
                );
            }
        }

        // -- Update snapshot (only after all checks pass) ------------------
        let mut snap = self.snapshot.clone();
        snap.phase = DemoPhase::Damaged;
        snap.inspection_after_damage = Some(inspection_after_damage);
        snap.five_erasure_refusal_evidence = Some(five_erasure_refusal_evidence);
        snap.five_erasure_receive_error = Some(five_erasure_receive_error);
        snap.damaged_shard_indices = vec![0, 1, 20, 21];
        self.snapshot = snap;
        Ok(self.snapshot.clone())
    }

    // -----------------------------------------------------------------------
    // Phase transition: Damaged → Received
    // -----------------------------------------------------------------------
    fn advance_to_received(&mut self) -> Result<DemoSnapshot> {
        let carrier_parcel_path = self
            .snapshot
            .carrier_parcel_path
            .as_ref()
            .context("carrier parcel path missing")?
            .clone();

        // -- Actual receive with repair ------------------------------------
        let result = self
            .bob
            .receive_parcel(&carrier_parcel_path, DEMO_PASSWORD_BOB)?;

        if result.repaired_shards != 4 {
            bail!(
                "expected 4 repaired shards, got {}",
                result.repaired_shards
            );
        }

        // -- Locate received files via record.received_path ----------------
        let received_files_dir = result
            .record
            .received_path
            .as_ref()
            .context("receive result has no received_path")?
            .clone();

        // -- Hash received files and compare with source -------------------
        let mut received_sha256 = BTreeMap::new();
        for name in ["file_a.bin", "file_b.bin"] {
            let received_file = received_files_dir.join(name);
            let hash = sha256_file(&received_file)
                .with_context(|| format!("hash received file {name}"))?;
            let expected = self
                .snapshot
                .source_sha256
                .get(name)
                .with_context(|| format!("source SHA-256 for {name} missing"))?;
            if &hash != expected {
                bail!(
                    "SHA-256 mismatch for {name}: received {hash} != source {expected}"
                );
            }
            received_sha256.insert(name.to_owned(), hash);
        }

        // -- Read Alice's current sender_state ----------------------------
        let sender_state = self
            .alice
            .deliveries()?
            .into_iter()
            .find(|r| r.parcel_id == self.parcel_id && r.direction == Direction::Outgoing)
            .map(|r| r.state);
        if sender_state.as_ref() != Some(&DeliveryState::WaitingReceipt) {
            bail!(
                "expected Alice's delivery state WaitingReceipt after Bob receives, got {:?}",
                sender_state
            );
        }

        // -- Update snapshot ----------------------------------------------
        let mut snap = self.snapshot.clone();
        snap.phase = DemoPhase::Received;
        snap.received_sha256 = received_sha256;
        snap.repaired_shards = Some(result.repaired_shards);
        snap.received_path = Some(received_files_dir);
        snap.sender_state = sender_state;
        self.snapshot = snap;
        Ok(self.snapshot.clone())
    }

    // -----------------------------------------------------------------------
    // Phase transition: Received → Confirmed
    // -----------------------------------------------------------------------
    fn advance_to_confirmed(&mut self) -> Result<DemoSnapshot> {
        let root = self.root.clone();
        let parcel_id = self.parcel_id.clone();

        // -- Bob exports signed receipt to carrier -------------------------
        let receipt_path = self
            .bob
            .export_receipt(&parcel_id, &root.join("carrier"))?;

        // -- Alice imports receipt -----------------------------------------
        let record = self.alice.import_receipt(&receipt_path)?;
        if record.state != DeliveryState::Confirmed {
            bail!(
                "expected DeliveryState::Confirmed after import_receipt, got {:?}",
                record.state
            );
        }

        // -- Read Alice's actual sender_state (must be Confirmed) ---------
        let sender_state = self
            .alice
            .deliveries()?
            .into_iter()
            .find(|r| r.parcel_id == parcel_id && r.direction == Direction::Outgoing)
            .map(|r| r.state);

        if sender_state.as_ref() != Some(&DeliveryState::Confirmed) {
            bail!(
                "Alice's delivery record is not Confirmed after import_receipt: {:?}",
                sender_state
            );
        }

        // -- Update snapshot ----------------------------------------------
        let mut snap = self.snapshot.clone();
        snap.phase = DemoPhase::Confirmed;
        snap.receipt_path = Some(receipt_path);
        snap.sender_state = sender_state;
        self.snapshot = snap;
        Ok(self.snapshot.clone())
    }
}
