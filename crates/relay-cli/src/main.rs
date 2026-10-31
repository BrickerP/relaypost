use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use directories::ProjectDirs;
use relay_core::{Contact, Direction, Inspection, ParcelRecord, PeerCard, Profile};
use serde::Serialize;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use zeroize::Zeroizing;

#[derive(Parser)]
#[command(
    name = "relaypost",
    version,
    about = "Carry files offline. Verify their arrival."
)]
struct Cli {
    /// Isolated profile directory (defaults to the OS application data directory)
    #[arg(long, global = true, value_name = "DIRECTORY")]
    home: Option<PathBuf>,
    /// Print only the operation's public result as JSON
    #[arg(long, global = true)]
    json: bool,
    /// Read the password from stdin instead of a masked terminal prompt
    #[arg(long, global = true, conflicts_with = "password_file")]
    password_stdin: bool,
    /// Read the password from a UTF-8 file; one trailing newline is removed
    #[arg(long, global = true, value_name = "FILE")]
    password_file: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create or export this device's public identity card
    Identity {
        #[command(subcommand)]
        command: IdentityCommand,
    },
    /// Import cards, check fingerprints, and pin trust
    Contacts {
        #[command(subcommand)]
        command: ContactsCommand,
    },
    /// Encrypt a new batch for a trusted contact
    Seal {
        /// Recipient's full pinned fingerprint
        #[arg(long, value_name = "FINGERPRINT")]
        to: String,
        /// Static regular files (directories and symlinks are rejected)
        #[arg(required = true, num_args = 1.., value_name = "FILE")]
        inputs: Vec<PathBuf>,
    },
    /// Copy an existing immutable parcel to a carrier directory
    Export {
        parcel_id: String,
        #[arg(long, value_name = "DIRECTORY")]
        carrier: PathBuf,
    },
    /// Verify a parcel's signed envelope and show each repair group's health
    Inspect { parcel: PathBuf },
    /// Repair, decrypt, validate, and commit a parcel, then sign its receipt
    Receive { parcel: PathBuf },
    /// Carry a signed receipt back to the sender
    Receipt {
        #[command(subcommand)]
        command: ReceiptCommand,
    },
    /// List local delivery facts and signed confirmation states
    Deliveries,
    /// Verify published batches and recover their index or missing receipts
    Recover {
        /// Discard interrupted receive staging instead of resuming it (no password required)
        #[arg(long)]
        discard_incomplete: bool,
    },
    /// Stop future local exports; already exported copies remain valid
    Cancel { parcel_id: String },
}

#[derive(Subcommand)]
enum IdentityCommand {
    /// Create a password-protected identity (refuses to replace an existing one)
    Init {
        #[arg(long)]
        name: String,
    },
    /// Export a PUBLIC contact card; this is not a private-key backup
    Export {
        #[arg(long, value_name = "FILE")]
        output: PathBuf,
    },
}

#[derive(Subcommand)]
enum ContactsCommand {
    /// Verify a self-signed public card, then store it as unverified
    Import {
        card: PathBuf,
    },
    /// Pin the complete fingerprint after comparing it through a trusted channel
    Trust {
        fingerprint: String,
    },
    List,
}

#[derive(Subcommand)]
enum ReceiptCommand {
    Export {
        parcel_id: String,
        #[arg(long, value_name = "DIRECTORY")]
        carrier: PathBuf,
    },
    /// Confirm an outgoing parcel only after verifying the expected recipient's signature
    Import { receipt: PathBuf },
}

fn main() {
    let cli = Cli::parse();
    if let Err(error) = run(&cli) {
        if cli.json {
            eprintln!("{}", serde_json::json!({ "error": format!("{error:#}") }));
        } else {
            eprintln!("Error: {error:#}");
        }
        std::process::exit(1);
    }
}

fn default_home() -> Result<PathBuf> {
    ProjectDirs::from("org", "RelayPost", "RelayPost")
        .map(|dirs| dirs.data_local_dir().to_path_buf())
        .context("Cannot locate application data. Pass --home DIRECTORY.")
}

fn run(cli: &Cli) -> Result<()> {
    let home = match &cli.home {
        Some(home) => home.clone(),
        None => default_home()?,
    };
    let profile = Profile::open(home)?;
    match &cli.command {
        Command::Identity { command } => match command {
            IdentityCommand::Init { name } => {
                let password = password(cli, true)?;
                let card = profile.initialize(name, &password)?;
                emit(cli.json, &card, card_text(&card))?;
            }
            IdentityCommand::Export { output } => {
                let card = profile.export_card(output)?;
                emit(
                    cli.json,
                    &card,
                    format!("Public card: {}\n{}", output.display(), card_text(&card)),
                )?;
            }
        },
        Command::Contacts { command } => match command {
            ContactsCommand::Import { card } => {
                let contact = profile.import_contact(card)?;
                emit(
                    cli.json,
                    &contact,
                    format!(
                        "{}\nCompare the complete fingerprint before trusting.",
                        contact_text(&contact)
                    ),
                )?;
            }
            ContactsCommand::Trust { fingerprint } => {
                profile.trust_contact(fingerprint)?;
                let contact = profile
                    .contacts()?
                    .into_iter()
                    .find(|contact| contact.card.fingerprint == *fingerprint)
                    .context("Contact missing after trust update")?;
                emit(cli.json, &contact, contact_text(&contact))?;
            }
            ContactsCommand::List => {
                let contacts = profile.contacts()?;
                let text = if contacts.is_empty() {
                    "No contacts. Import a public card first.".to_owned()
                } else {
                    contacts
                        .iter()
                        .map(contact_text)
                        .collect::<Vec<_>>()
                        .join("\n\n")
                };
                emit(cli.json, &contacts, text)?;
            }
        },
        Command::Seal { to, inputs } => {
            let password = password(cli, false)?;
            let record = profile.seal(inputs, to, &password)?;
            emit(cli.json, &record, record_text(&record))?;
        }
        Command::Export { parcel_id, carrier } => {
            let path = profile.export_parcel(parcel_id, carrier)?;
            emit(
                cli.json,
                &path,
                format!(
                    "Exported: {}\nAwaiting a verified signed receipt.",
                    path.display()
                ),
            )?;
        }
        Command::Inspect { parcel } => {
            let inspection = profile.inspect_parcel(parcel)?;
            emit(cli.json, &inspection, inspection_text(&inspection))?;
        }
        Command::Receive { parcel } => {
            let password = password(cli, false)?;
            let result = profile.receive_parcel(parcel, &password)?;
            let text = format!(
                "{}\nRepaired shards: {}\nAlready saved: {}",
                record_text(&result.record),
                result.repaired_shards,
                result.duplicate
            );
            emit(cli.json, &result, text)?;
        }
        Command::Receipt { command } => match command {
            ReceiptCommand::Export { parcel_id, carrier } => {
                let path = profile.export_receipt(parcel_id, carrier)?;
                emit(
                    cli.json,
                    &path,
                    format!(
                        "Signed receipt exported: {}\nCarry it back to the sender.",
                        path.display()
                    ),
                )?;
            }
            ReceiptCommand::Import { receipt } => {
                let record = profile.import_receipt(receipt)?;
                emit(cli.json, &record, record_text(&record))?;
            }
        },
        Command::Deliveries => {
            let records = profile.deliveries()?;
            let text = if records.is_empty() {
                "No deliveries yet.".to_owned()
            } else {
                records
                    .iter()
                    .map(record_text)
                    .collect::<Vec<_>>()
                    .join("\n\n")
            };
            emit(cli.json, &records, text)?;
        }
        Command::Recover { discard_incomplete } => {
            if *discard_incomplete {
                let count = profile.discard_incomplete_receives()?;
                emit(
                    cli.json,
                    &serde_json::json!({ "discarded_staging": count }),
                    format!("Discarded {count} interrupted receive staging directories."),
                )?;
            } else {
                let password = password(cli, false)?;
                let summary = profile.recover(&password)?;
                let mut text = format!(
                    "Recovered: {}\nPending: {}",
                    summary.recovered, summary.pending
                );
                for conflict in &summary.conflicts {
                    text.push_str(&format!("\nConflict: {conflict}"));
                }
                emit(cli.json, &summary, text)?;
            }
        }
        Command::Cancel { parcel_id } => {
            profile.cancel(parcel_id)?;
            let record = profile
                .deliveries()?
                .into_iter()
                .find(|record| {
                    record.parcel_id == *parcel_id && record.direction == Direction::Outgoing
                })
                .context("Delivery missing after cancellation")?;
            emit(
                cli.json,
                &record,
                format!(
                    "{}\nAlready exported copies cannot be recalled.",
                    record_text(&record)
                ),
            )?;
        }
    }
    Ok(())
}

fn password(cli: &Cli, confirm: bool) -> Result<Zeroizing<String>> {
    let mut password = if cli.password_stdin {
        read_password(io::stdin().lock())?
    } else if let Some(path) = &cli.password_file {
        read_password(File::open(path).context("Cannot open password file")?)?
    } else {
        let password =
            Zeroizing::new(rpassword::prompt_password("Identity password: ").context(
                "Cannot read a masked password. Use --password-stdin or --password-file.",
            )?);
        if confirm {
            let confirmation = Zeroizing::new(rpassword::prompt_password("Confirm password: ")?);
            if *password != *confirmation {
                bail!("Passwords do not match");
            }
        }
        password
    };
    if password.ends_with('\n') {
        password.pop();
        if password.ends_with('\r') {
            password.pop();
        }
    }
    if password.is_empty() {
        bail!("Password must not be empty");
    }
    if password
        .chars()
        .any(|character| matches!(character, '\n' | '\r' | '\0'))
    {
        bail!("Password must be a single line without NUL bytes");
    }
    Ok(password)
}

fn read_password(reader: impl Read) -> Result<Zeroizing<String>> {
    let mut password = Zeroizing::new(String::new());
    reader
        .take(65_537)
        .read_to_string(&mut password)
        .context("Cannot read a UTF-8 password")?;
    if password.len() > 65_536 {
        bail!("Password input exceeds 64 KiB");
    }
    Ok(password)
}

fn emit(json: bool, value: &impl Serialize, text: String) -> Result<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    if json {
        serde_json::to_writer(&mut output, value)?;
        writeln!(output)?;
    } else {
        writeln!(output, "{text}")?;
    }
    Ok(())
}

fn card_text(card: &PeerCard) -> String {
    format!(
        "{}\nFingerprint: {}\nPublic age recipient: {}",
        card.name, card.fingerprint, card.age_recipient
    )
}

fn contact_text(contact: &Contact) -> String {
    format!(
        "{}\nTrust: {}",
        card_text(&contact.card),
        if contact.verified {
            "pinned"
        } else {
            "unverified"
        }
    )
}

fn record_text(record: &ParcelRecord) -> String {
    let mut text = format!(
        "Parcel: {}\nDirection: {:?}\nState: {:?}\nFrom: {}\nTo: {}\nFiles: {} ({} bytes)",
        record.parcel_id,
        record.direction,
        record.state,
        record.sender,
        record.recipient,
        record.files,
        record.total_bytes
    );
    if let Some(path) = &record.received_path {
        text.push_str(&format!("\nSaved batch: {}", path.display()));
    }
    if let Some(path) = &record.receipt_path {
        text.push_str(&format!("\nSigned receipt: {}", path.display()));
    }
    if let Some(error) = &record.local_error {
        text.push_str(&format!("\nLocal copy needs attention: {error}"));
    }
    text
}

fn inspection_text(inspection: &Inspection) -> String {
    let mut text = format!(
        "Parcel: {}\nFrom: {}\nTo: {}\nCiphertext: {} bytes\nRecoverable: {}",
        inspection.parcel_id,
        inspection.sender,
        inspection.recipient,
        inspection.ciphertext_bytes,
        inspection.recoverable
    );
    for group in &inspection.groups {
        text.push_str(&format!(
            "\nGroup {}: {} data + {} parity; {} valid; missing/bad {:?}; recoverable {}",
            group.index,
            group.data_shards,
            group.parity_shards,
            group.valid_shards,
            group.missing_shards,
            group.recoverable
        ));
    }
    text
}
