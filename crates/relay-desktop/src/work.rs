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
    FindTransfers(PathBuf),
    FindReceipts(PathBuf),
    Inspect(PathBuf),
    Receive(PathBuf, Zeroizing<String>),
    ExportReceipt(String, PathBuf),
    ImportReceipt(PathBuf),
    OpenFolder(PathBuf),
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
            Self::Refresh => "Checking this computer…",
            Self::Measure(_) => "Checking selected files…",
            Self::Initialize(..) => "Setting up this computer…",
            Self::ExportCard(_) => "Saving your connection card…",
            Self::ImportContact(_) => "Checking the connection card…",
            Self::Trust(_) => "Saving the checked connection…",
            Self::Seal(..) => "Protecting your files…",
            Self::ExportParcel(..) => "Copying the transfer to your drive…",
            Self::FindTransfers(_) => "Looking for transfer folders…",
            Self::FindReceipts(_) => "Looking for confirmation folders…",
            Self::Inspect(_) => "Checking this transfer…",
            Self::Receive(..) => "Restoring and saving your files…",
            Self::ExportReceipt(..) => "Copying the confirmation to your drive…",
            Self::ImportReceipt(_) => "Verifying the delivery confirmation…",
            Self::OpenFolder(_) => "Opening your saved files…",
            Self::Recover(_) => "Checking saved files and records…",
            Self::DiscardIncomplete => "Clearing unfinished temporary copies…",
            Self::Cancel(_) => "Stopping future copies…",
            Self::DemoStart(_) => "Preparing the walkthrough…",
            Self::DemoAdvance(_) => "Running the next walkthrough step…",
            Self::DemoCopyReport(..) => "Saving the walkthrough report…",
        }
    }
}

pub enum Payload {
    None,
    Initialized,
    ContactImported(Box<Contact>),
    Trusted,
    ParcelExported,
    ReceiptExported(String),
    ReceiptImported(Box<ParcelRecord>),
    /// Name-only candidates. These have not been authenticated by discovery.
    TransferCandidates(Vec<PathBuf>),
    /// Name-only candidates. Import must verify their signed confirmations.
    ReceiptCandidates(Vec<PathBuf>),
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
    let refresh = !matches!(
        &action,
        Action::Measure(_)
            | Action::Inspect(_)
            | Action::FindTransfers(_)
            | Action::FindReceipts(_)
            | Action::OpenFolder(_)
    );
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
                "This computer is ready. Add the other computer to exchange files.".to_owned(),
                Payload::Initialized,
            )
        }
        Action::ExportCard(output) => {
            profile.export_card(&output)?;
            (
                "Connection card saved. Give it to the other person to add this computer."
                    .to_owned(),
                Payload::None,
            )
        }
        Action::ImportContact(input) => {
            let contact = profile.import_contact(&input)?;
            (
                format!(
                    "Connection card added for {}. Compare the safety code before exchanging files.",
                    contact.card.name
                ),
                Payload::ContactImported(Box::new(contact)),
            )
        }
        Action::Trust(fingerprint) => {
            profile.trust_contact(&fingerprint)?;
            (
                "Connection checked. The other computer must also check your connection card."
                    .to_owned(),
                Payload::Trusted,
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
                "Your transfer is prepared. Choose a drive or folder to copy it.".to_owned(),
                Payload::Sealed(Box::new(SealedDetails {
                    record,
                    inspection,
                    file_bytes,
                })),
            )
        }
        Action::ExportParcel(id, carrier) => {
            profile.export_parcel(&id, &carrier)?;
            (
                "Transfer copied. Take the drive to the other computer; delivery is not confirmed yet."
                    .to_owned(),
                Payload::ParcelExported,
            )
        }
        Action::FindTransfers(root) => {
            let candidates = discover_folders(&root, "parcel")?;
            (
                "Transfer folders found. Choose a transfer to check.".to_owned(),
                Payload::TransferCandidates(candidates),
            )
        }
        Action::FindReceipts(root) => {
            let candidates = discover_folders(&root, "receipt")?;
            (
                "Confirmation folders found. Choose a confirmation to verify.".to_owned(),
                Payload::ReceiptCandidates(candidates),
            )
        }
        Action::Inspect(parcel) => {
            let inspection = profile.inspect_parcel(&parcel)?;
            if inspection.recipient != profile.public_card()?.fingerprint {
                bail!("This transfer was prepared for another computer. Ask the sender to prepare it for this computer.");
            }
            let notice = if inspection.recoverable {
                "Ready to receive. Any repairable damage will be restored while saving."
            } else {
                "This copy is too damaged. Ask the sender to copy the prepared transfer again."
            };
            (notice.to_owned(), Payload::Inspected(inspection))
        }
        Action::Receive(parcel, password) => {
            let result = profile.receive_parcel(&parcel, &password)?;
            let notice = if result.duplicate {
                "These files were already saved. RelayPost checked them again."
            } else {
                "Files saved and checked. Put the confirmation on the drive to send back."
            };
            (notice.to_owned(), Payload::Received(Box::new(result)))
        }
        Action::ExportReceipt(id, carrier) => {
            profile.export_receipt(&id, &carrier)?;
            (
                "Confirmation copied to the drive. Take it back to the sender.".to_owned(),
                Payload::ReceiptExported(id),
            )
        }
        Action::ImportReceipt(receipt) => {
            let record = profile.import_receipt(&receipt)?;
            (
                "Delivery confirmed. The other computer confirmed saving the complete transfer."
                    .to_owned(),
                Payload::ReceiptImported(Box::new(record)),
            )
        }
        Action::OpenFolder(folder) => {
            open_saved_folder(&profile, &folder)?;
            ("Your saved files are open.".to_owned(), Payload::None)
        }
        Action::Recover(password) => {
            let summary = profile.recover(&password)?;
            let notice = format!(
                "Record check complete: {} restored, {} unfinished, {} need review.",
                summary.recovered,
                summary.pending,
                summary.conflicts.len()
            );
            (notice, Payload::Recovered(summary))
        }
        Action::Cancel(id) => {
            profile.cancel(&id)?;
            (
                "Future copies are stopped. Copies already on a drive still work.".to_owned(),
                Payload::None,
            )
        }
        Action::DiscardIncomplete => {
            let count = profile.discard_incomplete_receives()?;
            (
                format!(
                    "Cleared {count} unfinished receive folders. Saved files were not changed."
                ),
                Payload::Discarded,
            )
        }
        Action::DemoStart(root) => {
            // Demo actions do not refresh the real profile; return early.
            let session = DemoSession::start(root)?;
            let snapshot = session.snapshot();
            return Ok(Completed {
                notice: "Walkthrough ready. Two example computers and their files are set up."
                    .to_owned(),
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
                    "The example drive copy is damaged. The app also checked that excessive damage is refused."
                        .to_owned()
                }
                relay_core::DemoPhase::Received => {
                    format!(
                        "Example files saved. Repaired {} pieces and checked the contents.",
                        snapshot.repaired_shards.context("Received demo snapshot has no repaired-shard evidence")?
                    )
                }
                relay_core::DemoPhase::Confirmed => {
                    "Walkthrough complete. The sender verified the returned confirmation."
                        .to_owned()
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
                notice: "Walkthrough report saved.".to_owned(),
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

// Discovery is a name-only presentation step. It does not open envelopes,
// decrypt data, import confirmations, or establish trust. The selected folder
// still goes through the corresponding authenticated relay-core operation.
fn discover_folders(root: &Path, extension: &str) -> Result<Vec<PathBuf>> {
    const MAX_ENTRIES: usize = 512;
    const MAX_CANDIDATES: usize = 64;

    let metadata = std::fs::symlink_metadata(root)
        .with_context(|| format!("Cannot read the selected folder: {}", root.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("Choose a drive or an actual folder, not a file or a folder shortcut.");
    }
    if root.extension() == Some(std::ffi::OsStr::new(extension)) {
        return Ok(vec![root.to_path_buf()]);
    }

    let mut candidates = Vec::new();
    for (index, entry) in std::fs::read_dir(root)
        .with_context(|| format!("Cannot list the selected folder: {}", root.display()))?
        .enumerate()
    {
        if index >= MAX_ENTRIES {
            bail!("This drive has too many items to check at once. Choose the folder containing the RelayPost transfer or confirmation.");
        }
        let entry = entry.context("Cannot read an item in the selected folder")?;
        let file_type = entry
            .file_type()
            .context("Cannot check an item in the selected folder")?;
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        let path = entry.path();
        if path.extension() == Some(std::ffi::OsStr::new(extension)) {
            if candidates.len() >= MAX_CANDIDATES {
                bail!("There are too many RelayPost folders here. Choose the individual transfer or confirmation you need.");
            }
            candidates.push(path);
        }
    }
    candidates.sort();
    if candidates.is_empty() {
        let kind = if extension == "parcel" {
            "transfer"
        } else {
            "confirmation"
        };
        bail!("No RelayPost {kind} folder was found here. Choose the drive or the folder that directly contains it.");
    }
    Ok(candidates)
}

fn open_saved_folder(profile: &Profile, folder: &Path) -> Result<()> {
    let saved_here = profile.deliveries()?.iter().any(|record| {
        matches!(&record.direction, relay_core::Direction::Incoming)
            && record.received_path.as_deref() == Some(folder)
    });
    if !saved_here {
        bail!("Choose a saved transfer in Activity before opening its files.");
    }
    let metadata = std::fs::symlink_metadata(folder)
        .with_context(|| format!("Cannot find the saved folder: {}", folder.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "The saved folder is no longer available as a regular folder. Review its local record."
        );
    }
    // Canonicalization gives the OS opener an absolute filesystem path, never
    // an option, URL, shell command, or executable selected by the UI.
    let folder = std::fs::canonicalize(folder).context("Cannot locate the saved folder")?;
    let program = match std::env::consts::OS {
        "macos" => "open",
        "linux" => "xdg-open",
        _ => bail!("Opening a saved folder is not supported on this operating system."),
    };
    let status = std::process::Command::new(program)
        .arg(&folder)
        .status()
        .context("Cannot open the saved folder in the file manager")?;
    if !status.success() {
        bail!("The file manager could not open the saved folder. Review its location in the transfer details.");
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::discover_folders;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct SyntheticDirectory(PathBuf);

    impl SyntheticDirectory {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "relaypost-synthetic-discovery-{}-{nonce}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).expect("create synthetic test directory");
            Self(path)
        }
    }

    impl Drop for SyntheticDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn direct_folder_and_drive_selection_use_the_same_candidates() {
        let root = SyntheticDirectory::new();
        let first = root.0.join("a.parcel");
        let second = root.0.join("b.parcel");
        std::fs::create_dir(&second).unwrap();
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(root.0.join("a.receipt")).unwrap();
        std::fs::write(root.0.join("not-a-folder.parcel"), b"synthetic").unwrap();

        assert_eq!(
            discover_folders(&root.0, "parcel").unwrap(),
            vec![first.clone(), second]
        );
        assert_eq!(discover_folders(&first, "parcel").unwrap(), vec![first]);
        assert_eq!(
            discover_folders(&root.0, "receipt").unwrap(),
            vec![root.0.join("a.receipt")]
        );
    }

    #[test]
    fn discovery_does_not_search_nested_folders() {
        let root = SyntheticDirectory::new();
        std::fs::create_dir(root.0.join("unrelated")).unwrap();
        std::fs::create_dir(root.0.join("unrelated/nested.parcel")).unwrap();

        let error = discover_folders(&root.0, "parcel").unwrap_err();
        assert!(error.to_string().contains("No RelayPost transfer folder"));
    }

    #[cfg(unix)]
    #[test]
    fn discovery_refuses_selected_shortcuts_and_does_not_follow_child_links() {
        let root = SyntheticDirectory::new();
        let outside = SyntheticDirectory::new();
        let link = root.0.join("shortcut.parcel");
        std::os::unix::fs::symlink(&outside.0, &link).unwrap();

        assert!(discover_folders(&link, "parcel").is_err());
        assert!(discover_folders(&root.0, "parcel").is_err());
    }

    #[test]
    fn discovery_reports_entry_limit_instead_of_a_partial_list() {
        let root = SyntheticDirectory::new();
        std::fs::create_dir(root.0.join("valid.parcel")).unwrap();
        for index in 0..512 {
            std::fs::write(root.0.join(format!("synthetic-{index}.txt")), b"").unwrap();
        }

        let error = discover_folders(&root.0, "parcel").unwrap_err();
        assert!(error.to_string().contains("too many items"));
    }

    #[test]
    fn discovery_reports_candidate_limit_instead_of_a_partial_list() {
        let root = SyntheticDirectory::new();
        for index in 0..65 {
            std::fs::create_dir(root.0.join(format!("synthetic-{index}.parcel"))).unwrap();
        }

        let error = discover_folders(&root.0, "parcel").unwrap_err();
        assert!(error.to_string().contains("too many RelayPost folders"));
    }
}
