//! Independent on-disk acceptance checks for the shared, synthetic demo.
//! Build and execute only in CI; never reads a real user's profile.
use relay_core::{DeliveryState, DemoPhase, DemoSession, Direction, Profile};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const MIB: u64 = 1024 * 1024;

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn tree(root: &Path) -> BTreeMap<PathBuf, String> {
    fn visit(root: &Path, directory: &Path, entries: &mut BTreeMap<PathBuf, String>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink(), "synthetic evidence must not escape its root");
            if kind.is_dir() {
                visit(root, &entry.path(), entries);
            } else {
                assert!(kind.is_file());
                entries.insert(
                    entry.path().strip_prefix(root).unwrap().to_path_buf(),
                    sha256(&fs::read(entry.path()).unwrap()),
                );
            }
        }
    }
    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries);
    entries
}

fn payload_hashes(root: &Path) -> BTreeMap<String, String> {
    tree(root)
        .into_iter()
        .map(|(name, hash)| (name.to_str().unwrap().replace('\\', "/"), hash))
        .collect()
}

fn outgoing(profile: &Profile) -> relay_core::ParcelRecord {
    let mut records = profile.deliveries().unwrap();
    assert_eq!(records.len(), 1);
    let record = records.pop().unwrap();
    assert_eq!(record.direction, Direction::Outgoing);
    record
}

fn observed_erasures(parcel: &Path, group: &Value) -> Vec<usize> {
    group["shards"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .filter_map(|(index, shard)| {
            let valid = fs::read(parcel.join(shard["path"].as_str().unwrap()))
                .map(|bytes| {
                    bytes.len() as u64 == group["shard_bytes"].as_u64().unwrap()
                        && sha256(&bytes) == shard["sha256"].as_str().unwrap()
                })
                .unwrap_or(false);
            (!valid).then_some(index)
        })
        .collect()
}

fn no_secret_fields(value: &Value) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                let normalized = key.to_ascii_lowercase();
                assert!(!normalized.contains("password"));
                assert!(!normalized.contains("passphrase"));
                assert!(!normalized.contains("private_key"));
                no_secret_fields(value);
            }
        }
        Value::Array(values) => values.iter().for_each(no_secret_fields),
        Value::String(text) => {
            assert!(!text.contains("AGE-SECRET-KEY-"));
            assert!(!text.contains("synthetic-demo-alice-v1"));
            assert!(!text.contains("synthetic-demo-bob-v1"));
        }
        _ => {}
    }
}

#[cfg(unix)]
fn advance_with_external_link_checks(
    session: &mut DemoSession,
    carrier: &Path,
    pristine: &Path,
    outside: &Path,
) -> relay_core::DemoSnapshot {
    let parity = Path::new("groups/g000/s020.bin");
    let carrier_parity = carrier.join(parity);
    let original = fs::read(pristine.join(parity)).unwrap();
    fs::write(outside, &original).unwrap();

    // Between-stage changes are controlled fixtures, not a hostile race.
    // A symlink must fail closed without following it to another directory.
    fs::remove_file(&carrier_parity).unwrap();
    std::os::unix::fs::symlink(outside, &carrier_parity).unwrap();
    assert!(session.advance().is_err());
    assert_eq!(session.snapshot().phase, DemoPhase::Prepared);
    assert_eq!(fs::read(outside).unwrap(), original);
    restore_carrier_damage_targets(carrier, pristine);

    // A hardlinked carrier entry may be refused or replaced with a private
    // damaged entry, but XOR/restore must never mutate the outside inode.
    fs::remove_file(&carrier_parity).unwrap();
    fs::hard_link(outside, &carrier_parity).unwrap();
    let outcome = session.advance();
    assert_eq!(
        fs::read(outside).unwrap(),
        original,
        "demo damage must not modify a hardlinked file outside its root"
    );
    match outcome {
        Ok(snapshot) => snapshot,
        Err(_) => {
            assert_eq!(session.snapshot().phase, DemoPhase::Prepared);
            restore_carrier_damage_targets(carrier, pristine);
            session.advance().unwrap()
        }
    }
}

#[cfg(unix)]
fn restore_carrier_damage_targets(carrier: &Path, pristine: &Path) {
    for index in [0, 1, 2, 20, 21] {
        let relative = format!("groups/g000/s{index:03}.bin");
        let path = carrier.join(&relative);
        // Unlink first: writing through an existing link would invalidate this
        // test's own isolation guarantee.
        if fs::symlink_metadata(&path).is_ok() {
            fs::remove_file(&path).unwrap();
        }
        fs::write(path, fs::read(pristine.join(relative)).unwrap()).unwrap();
    }
}

#[test]
fn guided_demo_reports_real_bytes_and_receipt_gated_delivery() {
    let temporary = tempfile::tempdir().unwrap();
    let parent = fs::canonicalize(temporary.path()).unwrap();
    let root = parent.join("fresh-demo");
    // A separate synthetic normal profile must survive the whole demo untouched.
    let normal_home = parent.join("normal-profile");
    let normal = Profile::open(&normal_home).unwrap();
    normal
        .initialize("Synthetic untouched identity", "SYNTHETIC-sentinel-only")
        .unwrap();
    let normal_before = tree(&normal_home);

    let mut session = DemoSession::start(root.clone()).unwrap();
    let prepared = session.snapshot();
    assert_eq!(prepared.phase, DemoPhase::Prepared);
    assert_eq!(prepared.root, root);
    assert_eq!(prepared.sender_state, Some(DeliveryState::WaitingReceipt));
    assert!(prepared.received_sha256.is_empty());
    assert!(prepared.repaired_shards.is_none());
    assert!(session.export_report().is_err());
    assert!(!root.join("demo-report.json").exists());
    let source = root.join("source_files");
    let total: u64 = fs::read_dir(&source)
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .sum();
    assert_eq!(total, 21 * MIB);
    assert_eq!(prepared.source_sha256, payload_hashes(&source));

    let alice = Profile::open(root.join("alice")).unwrap();
    let bob = Profile::open(root.join("bob")).unwrap();
    assert_ne!(
        alice.public_card().unwrap().fingerprint,
        bob.public_card().unwrap().fingerprint
    );
    let outgoing_before = outgoing(&alice);
    assert_eq!(outgoing_before.state, DeliveryState::WaitingReceipt);
    assert!(bob.deliveries().unwrap().is_empty());
    let pristine = outgoing_before.parcel_path.as_ref().unwrap();
    let carrier = prepared.carrier_parcel_path.as_ref().unwrap();
    assert!(carrier.starts_with(&root));
    assert_eq!(tree(pristine), tree(carrier));
    let raw_envelope = fs::read(carrier.join("envelope.json")).unwrap();
    let envelope: Value = serde_json::from_slice(&raw_envelope).unwrap();
    assert_eq!(envelope["total_bytes"].as_u64(), Some(21 * MIB));
    let groups = envelope["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0]["data_shards"], 20);
    assert_eq!(groups[0]["parity_shards"], 4);
    assert!(groups[1]["data_shards"].as_u64().unwrap() < 20);
    let inspected = bob.inspect_parcel(carrier).unwrap();
    assert!(inspected.recoverable);
    assert_eq!(
        serde_json::to_value(prepared.initial_inspection.as_ref().unwrap()).unwrap(),
        serde_json::to_value(&inspected).unwrap()
    );

    // Reassemble the pristine DATA bytes independently of the demo and codec.
    // The parcel ID must really identify these age ciphertext bytes.
    let mut ciphertext = Vec::new();
    for group in groups {
        assert!(observed_erasures(pristine, group).is_empty());
        let mut data = Vec::new();
        for shard in group["shards"].as_array().unwrap().iter().take(
            group["data_shards"].as_u64().unwrap() as usize,
        ) {
            data.extend(fs::read(pristine.join(shard["path"].as_str().unwrap())).unwrap());
        }
        data.truncate(group["bytes"].as_u64().unwrap() as usize);
        ciphertext.extend(data);
    }
    assert_eq!(ciphertext.len() as u64, inspected.ciphertext_bytes);
    assert!(ciphertext.starts_with(b"age-encryption.org/v1"));
    assert_eq!(sha256(&ciphertext), outgoing_before.parcel_id);

    #[cfg(unix)]
    let damaged = advance_with_external_link_checks(
        &mut session,
        carrier,
        pristine,
        &parent.join("SYNTHETIC-outside-sentinel.bin"),
    );
    #[cfg(not(unix))]
    let damaged = session.advance().unwrap();
    assert_eq!(damaged.phase, DemoPhase::Damaged);
    let actual_erasures = observed_erasures(carrier, &groups[0]);
    assert_eq!(actual_erasures.len(), 4);
    assert_eq!(actual_erasures.iter().filter(|&&index| index < 20).count(), 2);
    assert_eq!(actual_erasures.iter().filter(|&&index| index >= 20).count(), 2);
    assert_eq!(actual_erasures, damaged.damaged_shard_indices);
    for &index in &actual_erasures {
        let path = carrier.join(groups[0]["shards"][index]["path"].as_str().unwrap());
        if index < 20 {
            assert!(!path.exists(), "the two damaged data shards must be absent");
        } else {
            assert!(path.is_file(), "bad parity shards remain present for hash checking");
        }
    }
    let four = damaged.inspection_after_damage.as_ref().unwrap();
    assert!(four.recoverable);
    assert_eq!(four.groups[0].missing_shards, actual_erasures);
    let five = damaged.five_erasure_refusal_evidence.as_ref().unwrap();
    assert!(!five.recoverable);
    assert_eq!(five.groups[0].missing_shards.len(), 5);
    assert!(bob.deliveries().unwrap().is_empty());
    assert!(!root.join("bob/inbox").join(&inspected.parcel_id).exists());
    assert!(bob
        .export_receipt(&inspected.parcel_id, &root.join("forbidden-receipt"))
        .is_err());
    let damaged_json = serde_json::to_value(&damaged).unwrap();
    assert!(damaged_json["five_erasure_receive_error"]
        .as_str()
        .is_some_and(|error| !error.is_empty()));

    // A failed real receive cannot advance the phase or manufacture a receipt.
    let extra_index = (0..20).find(|index| !actual_erasures.contains(index)).unwrap();
    let extra_relative = groups[0]["shards"][extra_index]["path"].as_str().unwrap();
    let extra_carrier = carrier.join(extra_relative);
    let correct_extra = fs::read(&extra_carrier).unwrap();
    fs::remove_file(&extra_carrier).unwrap();
    assert!(session.advance().is_err());
    assert_eq!(session.snapshot().phase, DemoPhase::Damaged);
    assert_eq!(serde_json::to_value(session.snapshot()).unwrap(), damaged_json);
    assert!(bob.deliveries().unwrap().is_empty());
    assert_eq!(outgoing(&alice).state, DeliveryState::WaitingReceipt);
    fs::write(&extra_carrier, correct_extra).unwrap();

    let received = session.advance().unwrap();
    assert_eq!(received.phase, DemoPhase::Received);
    assert_eq!(received.repaired_shards, Some(4));
    assert_eq!(received.sender_state, Some(DeliveryState::WaitingReceipt));
    assert_eq!(outgoing(&alice).state, DeliveryState::WaitingReceipt);
    let incoming = bob.deliveries().unwrap();
    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0].state, DeliveryState::SavedReceiptReady);
    let received_files = incoming[0].received_path.as_ref().unwrap();
    assert!(received_files.starts_with(&root));
    assert_eq!(payload_hashes(received_files), prepared.source_sha256);
    assert_eq!(received.received_sha256, payload_hashes(received_files));
    assert_eq!(payload_hashes(&source), prepared.source_sha256);

    let confirmed = session.advance().unwrap();
    assert_eq!(confirmed.phase, DemoPhase::Confirmed);
    assert_eq!(confirmed.sender_state, Some(DeliveryState::Confirmed));
    assert_eq!(outgoing(&alice).state, DeliveryState::Confirmed);
    let receipt_dir = incoming[0].receipt_path.as_ref().unwrap();
    let receipt = json(&receipt_dir.join("receipt.json"));
    assert_eq!(receipt["parcel_id"], inspected.parcel_id);
    assert_eq!(receipt["envelope_sha256"], sha256(&raw_envelope));
    assert_eq!(receipt["manifest_sha256"], envelope["manifest_sha256"]);
    assert_eq!(receipt["recipient"], bob.public_card().unwrap().fingerprint);
    assert_eq!(receipt["result"], "committed");
    assert!(receipt_dir.join("receipt.minisig").is_file());

    let report_path = session.export_report().unwrap();
    assert_eq!(report_path, root.join("demo-report.json"));
    let report = json(&report_path);
    assert_eq!(report, serde_json::to_value(session.snapshot()).unwrap());
    no_secret_fields(&report);
    let final_tree = tree(&root);
    assert!(session.advance().is_err());
    assert!(session.export_report().is_err());
    assert!(DemoSession::start(root.clone()).is_err());
    assert_eq!(tree(&root), final_tree, "rejected reuse must preserve all evidence");
    assert_eq!(tree(&normal_home), normal_before);

    // Starting again means a fresh run, never resuming or replacing old output.
    let restarted = DemoSession::start(parent.join("second-demo")).unwrap();
    assert_eq!(restarted.snapshot().phase, DemoPhase::Prepared);
    assert_eq!(restarted.snapshot().source_sha256, prepared.source_sha256);
    let fresh_alice = Profile::open(parent.join("second-demo/alice")).unwrap();
    assert_ne!(
        fresh_alice.public_card().unwrap().fingerprint,
        alice.public_card().unwrap().fingerprint
    );
    assert_eq!(tree(&root), final_tree);
    assert_eq!(tree(&normal_home), normal_before);
}

#[test]
fn guided_demo_refuses_all_existing_output_paths_without_modification() {
    let temporary = tempfile::tempdir().unwrap();
    let parent = fs::canonicalize(temporary.path()).unwrap();
    let empty = parent.join("empty");
    fs::create_dir(&empty).unwrap();
    assert!(DemoSession::start(empty.clone()).is_err());
    assert_eq!(fs::read_dir(&empty).unwrap().count(), 0);
    let occupied = parent.join("occupied");
    fs::create_dir(&occupied).unwrap();
    fs::write(occupied.join("sentinel"), b"SYNTHETIC preserve me").unwrap();
    let before = tree(&occupied);
    assert!(DemoSession::start(occupied.clone()).is_err());
    assert_eq!(tree(&occupied), before);
    let file = parent.join("file");
    fs::write(&file, b"SYNTHETIC existing file").unwrap();
    assert!(DemoSession::start(file.clone()).is_err());
    assert_eq!(fs::read(&file).unwrap(), b"SYNTHETIC existing file");
    #[cfg(unix)]
    {
        let link = parent.join("link");
        std::os::unix::fs::symlink(&occupied, &link).unwrap();
        assert!(DemoSession::start(link).is_err());
        assert_eq!(tree(&occupied), before);
    }
}
