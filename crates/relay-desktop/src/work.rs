use anyhow::{bail, Context, Result};
use relay_core::{
    Contact, Inspection, ParcelRecord, PeerCard, Profile, ReceiveResult, RecoverySummary,
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
    Cancel(String),
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
            Self::Cancel(_) => "Cancelling future local exports…",
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
