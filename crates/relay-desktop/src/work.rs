use anyhow::{bail, Context, Result};
use relay_core::{
    Contact, DemoSession, DemoSnapshot, Inspection, ParcelRecord, PeerCard, Profile, ReceiveResult,
    RecoverySummary,
};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

pub struct Snapshot {
    pub identity: std::result::Result<PeerCard, String>,
    pub contacts: Vec<Contact>,
    pub deliveries: Vec<ParcelRecord>,
}

impl Snapshot {
    fn load(profile: &Profile) -> Result<Self> {
        Ok(Self {
            identity: profile.public_card().map_err(|error| format!("{error:#}")),
            contacts: profile.contacts()?,
            deliveries: profile.deliveries()?,
        })
    }
}

pub enum Action {
    Refresh,
    Measure(Vec<PathBuf>),
    Initialize(String, Zeroizing<String>),
    ExportCard(PathBuf),
    ImportContact(PathBuf),
    Trust(String),
    Seal(Vec<PathBuf>, String, Zeroizing<String>),
    ExportParcel(String, PathBuf),
    Inspect(PathBuf),
    Receive(PathBuf, Zeroizing<String>),
    ExportReceipt(String, PathBuf),
    ImportReceipt(PathBuf),
    Recover(Zeroizing<String>),
    DiscardIncomplete,
    Cancel(String),
    /// Start a new demo session at the given (non-existent) root path.
    DemoStart(PathBuf),
    /// Advance the demo session one phase. Takes ownership; returns it.
    DemoAdvance(Box<DemoSession>),
    /// Copy the demo report from source to a user-chosen target path.
    DemoCopyReport(PathBuf, PathBuf),
}

impl Action {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Refresh => "Reading local records…",
            Self::Measure(_) => "Reading selected file sizes…",
            Self::Initialize(..) => "Protecting this device's identity…",
            Self::ExportCard(_) => "Exporting public identity card…",
            Self::ImportContact(_) => "Verifying contact card…",
            Self::Trust(_) => "Pinning verified fingerprint…",
            Self::Seal(..) => "Snapshotting, encrypting, and sealing files…",
            Self::ExportParcel(..) => "Copying and verifying parcel on carrier…",
            Self::Inspect(_) => "Verifying signed envelope and shard health…",
            Self::Receive(..) => "Repairing, decrypting, and validating batch…",
            Self::ExportReceipt(..) => "Exporting signed receipt…",
            Self::ImportReceipt(_) => "Verifying returned receipt…",
            Self::Recover(_) => "Checking committed files and recovering local records…",
            Self::DiscardIncomplete => "Discarding interrupted receive work…",
            Self::Cancel(_) => "Cancelling future local exports…",
            Self::DemoStart(_) => "Starting guided demo — creating synthetic profiles…",
            Self::DemoAdvance(_) => "Advancing guided demo to next phase…",
            Self::DemoCopyReport(..) => "Saving demo report to chosen location…",
        }
    }
}

pub enum Payload {
    None,
    Measured(u64),
    Sealed(Box<SealedDetails>),
    Inspected(Inspection),
    Received(Box<ReceiveResult>),
    Recovered(RecoverySummary),
    Discarded,
    /// Demo phase completed. Carries back the session (for the next advance)
    /// and the updated public snapshot. `DemoSnapshot` is boxed to keep this
    /// enum variant's inline size reasonable.
    DemoUpdate {
        session: Box<DemoSession>,
        snapshot: Box<DemoSnapshot>,
    },
}

pub struct SealedDetails {
    pub record: ParcelRecord,
    pub inspection: Inspection,
    pub file_bytes: u64,
}

pub struct Completed {
    pub notice: String,
    pub snapshot: Option<Snapshot>,
    pub payload: Payload,
}

pub fn execute(profile: Profile, action: Action) -> Result<Completed> {
    let refresh = !matches!(&action, Action::Measure(_) | Action::Inspect(_));
    let (notice, payload) = match action {
        Action::Refresh => (String::new(), Payload::None),
        Action::Measure(inputs) => {
            let mut total = 0_u64;
            for input in inputs {
                let metadata = std::fs::symlink_metadata(&input)
                    .with_context(|| format!("Cannot read {}", input.display()))?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    bail!("Select static regular files only: {}", input.display());
                }
                total = total
                    .checked_add(metadata.len())
                    .context("Selected file sizes overflow")?;
            }
            (String::new(), Payload::Measured(total))
        }
        Action::Initialize(name, password) => {
            profile.initialize(&name, &password)?;
            (
                "Identity created. Exchange public cards and compare complete fingerprints."
                    .to_owned(),
                Payload::None,
            )
        }
        Action::ExportCard(output) => {
            profile.export_card(&output)?;
            (
                format!("Public card exported to {}", output.display()),
                Payload::None,
            )
        }
        Action::ImportContact(input) => {
            let contact = profile.import_contact(&input)?;
            (
                format!(
                    "Imported {}. Compare the complete fingerprint before pinning trust.",
                    contact.card.name
                ),
                Payload::None,
            )
        }
        Action::Trust(fingerprint) => {
            profile.trust_contact(&fingerprint)?;
            (
                "Contact fingerprint pinned. The other device must also trust your card."
                    .to_owned(),
                Payload::None,
            )
        }
        Action::Seal(inputs, recipient, password) => {
            let record = profile.seal(&inputs, &recipient, &password)?;
            let path = record
                .parcel_path
                .as_deref()
                .context("Sealed parcel has no local path")?;
            let inspection = profile.inspect_parcel(path)?;
            let file_bytes = directory_file_bytes(path)?;
            (
                "Parcel sealed. Choose a carrier directory to export it.".to_owned(),
                Payload::Sealed(Box::new(SealedDetails {
                    record,
                    inspection,
                    file_bytes,
                })),
            )
        }
        Action::ExportParcel(id, carrier) => {
            let output = profile.export_parcel(&id, &carrier)?;
            (
                format!(
                    "Parcel exported to {}. Awaiting a verified signed receipt.",
                    output.display()
                ),
                Payload::None,
            )
        }
        Action::Inspect(parcel) => {
            let inspection = profile.inspect_parcel(&parcel)?;
            let notice = if inspection.recoverable {
                "Signed envelope verified. Shards are sufficient to attempt receiving."
            } else {
                "Signed envelope verified, but at least one group exceeds its repair budget."
            };
            (notice.to_owned(), Payload::Inspected(inspection))
        }
        Action::Receive(parcel, password) => {
            let result = profile.receive_parcel(&parcel, &password)?;
            let notice = if result.duplicate {
                "Previously saved batch verified. The existing delivery is reused."
            } else {
                "Batch verified and saved. Its signed receipt is ready to carry back."
            };
            (notice.to_owned(), Payload::Received(Box::new(result)))
        }
        Action::ExportReceipt(id, carrier) => {
            let output = profile.export_receipt(&id, &carrier)?;
            (
                format!(
                    "Signed receipt exported to {}. Carry it back to the sender.",
                    output.display()
                ),
                Payload::None,
            )
        }
        Action::ImportReceipt(receipt) => {
            let record = profile.import_receipt(&receipt)?;
            (
                format!(
                    "Recipient's signed confirmation verified for {}.",
                    record.parcel_id
                ),
                Payload::None,
            )
        }
        Action::Recover(password) => {
            let summary = profile.recover(&password)?;
            let notice = format!(
                "Recovery finished: {} recovered, {} pending, {} conflicts.",
                summary.recovered,
                summary.pending,
                summary.conflicts.len()
            );
            (notice, Payload::Recovered(summary))
        }
        Action::Cancel(id) => {
            profile.cancel(&id)?;
            (
                "Future local exports cancelled. Copies already on a carrier remain valid."
                    .to_owned(),
                Payload::None,
            )
        }
        Action::DiscardIncomplete => {
            let count = profile.discard_incomplete_receives()?;
            (
                format!("Discarded {count} interrupted receive staging directories."),
                Payload::Discarded,
            )
        }
        Action::DemoStart(root) => {
            // Demo actions do not refresh the real profile; return early.
            let session = DemoSession::start(root)?;
            let snapshot = session.snapshot();
            return Ok(Completed {
                notice: "Demo started — profiles created, parcel sealed and exported.".to_owned(),
                snapshot: None,
                payload: Payload::DemoUpdate {
                    session: Box::new(session),
                    snapshot: Box::new(snapshot),
                },
            });
        }
        Action::DemoAdvance(mut session) => {
            session.advance()?;
            // If we just reached Confirmed, also export the report so
            // report_path is populated in the snapshot.
            if session.snapshot().phase == relay_core::DemoPhase::Confirmed {
                session.export_report()?;
            }
            let snapshot = session.snapshot();
            let notice = match snapshot.phase {
                relay_core::DemoPhase::Damaged => {
                    "Demo damaged: data shards 0,1 deleted; parity 20,21 XOR'd; 5-erasure receive refused; shard 2 restored."
                        .to_owned()
                }
                relay_core::DemoPhase::Received => {
                    format!(
                        "Demo received: {} shards repaired, SHA-256 hashes match.",
                        snapshot.repaired_shards.context("Received demo snapshot has no repaired-shard evidence")?
                    )
                }
                relay_core::DemoPhase::Confirmed => {
                    "Demo confirmed: Alice's delivery is Confirmed.".to_owned()
                }
                relay_core::DemoPhase::Prepared => {
                    bail!("Demo did not advance beyond Prepared. Run a new guided demo.")
                }
            };
            return Ok(Completed {
                notice,
                snapshot: None,
                payload: Payload::DemoUpdate {
                    session,
                    snapshot: Box::new(snapshot),
                },
            });
        }
        Action::DemoCopyReport(source, target) => {
            // Demo actions do not refresh the real profile; return early.
            use std::io;
            let mut src = std::fs::File::open(&source)
                .with_context(|| format!("Cannot open report: {}", source.display()))?;
            let mut dst = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .with_context(|| {
                    format!(
                        "Cannot create target (file may already exist): {}",
                        target.display()
                    )
                })?;
            io::copy(&mut src, &mut dst)
                .with_context(|| format!("Error copying report to {}", target.display()))?;
            dst.sync_all()
                .with_context(|| format!("Error syncing report to {}", target.display()))?;
            return Ok(Completed {
                notice: format!("Demo report saved to {}", target.display()),
                snapshot: None,
                payload: Payload::None,
            });
        }
    };
    Ok(Completed {
        notice,
        snapshot: if refresh {
            Some(Snapshot::load(&profile)?)
        } else {
            None
        },
        payload,
    })
}

// Presentation-only measurement of an app-owned sealed directory. Never follow
// symlinks or read the parcel format here; authentication stays in relay-core.
fn directory_file_bytes(root: &Path) -> Result<u64> {
    let mut directories = vec![(root.to_path_buf(), 0_usize)];
    let mut total = 0_u64;
    let mut entries = 0_usize;
    while let Some((directory, depth)) = directories.pop() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            entries += 1;
            if entries > 256 {
                bail!("Sealed parcel has too many entries to measure");
            }
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if metadata.is_dir() && depth < 4 {
                directories.push((entry.path(), depth + 1));
            } else if metadata.is_file() {
                total = total
                    .checked_add(metadata.len())
                    .context("Parcel file sizes overflow")?;
            } else {
                bail!(
                    "Unexpected entry in sealed parcel: {}",
                    entry.path().display()
                );
            }
        }
    }
    Ok(total)
}
