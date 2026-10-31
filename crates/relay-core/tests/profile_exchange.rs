//! Independent, synthetic two-profile acceptance tests. Run only in CI.
use relay_core::{DeliveryState, Direction, ParcelRecord, PeerCard, Profile};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const PASSWORD: &str = "SYNTHETIC-TEST-ONLY-not-a-real-user-secret";
const MIB: usize = 1024 * 1024;

struct Pair {
    _temp: TempDir,
    root: PathBuf,
    alice: Profile,
    bob: Profile,
    alice_card: PeerCard,
    bob_card: PeerCard,
}

impl Pair {
    fn new(trusted: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        // macOS exposes temporary directories through /var -> /private/var;
        // use the actual directory so symlink-safety checks test our fixtures.
        let root = fs::canonicalize(temp.path()).unwrap();
        let alice = Profile::open(root.join("alice")).unwrap();
        let bob = Profile::open(root.join("bob")).unwrap();
        let alice_card = alice.initialize("Synthetic Alice", PASSWORD).unwrap();
        let bob_card = bob.initialize("Synthetic Bob", PASSWORD).unwrap();
        alice.export_card(&root.join("alice.card")).unwrap();
        bob.export_card(&root.join("bob.card")).unwrap();
        let a_contact = bob.import_contact(&root.join("alice.card")).unwrap();
        let b_contact = alice.import_contact(&root.join("bob.card")).unwrap();
        assert!(!a_contact.verified);
        assert!(!b_contact.verified);
        if trusted {
            alice.trust_contact(&bob_card.fingerprint).unwrap();
            bob.trust_contact(&alice_card.fingerprint).unwrap();
        }
        Self {
            _temp: temp,
            root,
            alice,
            bob,
            alice_card,
            bob_card,
        }
    }

    fn input(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let input = self.root.join("input").join(name);
        fs::create_dir_all(input.parent().unwrap()).unwrap();
        fs::write(&input, bytes).unwrap();
        input
    }

    fn seal(&self, bytes: &[u8]) -> ParcelRecord {
        let input = self.input("payload.bin", bytes);
        self.alice
            .seal(&[input], &self.bob_card.fingerprint, PASSWORD)
            .unwrap()
    }

    fn export(&self, record: &ParcelRecord, label: &str) -> PathBuf {
        self.alice
            .export_parcel(&record.parcel_id, &self.root.join(label))
            .unwrap()
    }

    // Clone a pristine synthetic receiver identity/index so each damage case
    // exercises reconstruction, rather than the duplicate-import fast path.
    fn isolated_receiver(&self, label: &str) -> Profile {
        let home = self.root.join(label);
        copy_tree(&self.root.join("bob"), &home);
        Profile::open(home).unwrap()
    }
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        assert!(
            !kind.is_symlink(),
            "synthetic fixture must contain no symlinks"
        );
        let target = destination.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn files(root: &Path) -> Vec<PathBuf> {
    fn walk(directory: &Path, result: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                walk(&entry.path(), result);
            } else {
                result.push(entry.path());
            }
        }
    }
    let mut result = Vec::new();
    walk(root, &mut result);
    result.sort();
    result
}

fn tree_digest(root: &Path) -> BTreeMap<PathBuf, String> {
    files(root)
        .into_iter()
        .map(|path| {
            let hash = hex::encode(Sha256::digest(fs::read(&path).unwrap()));
            (path.strip_prefix(root).unwrap().to_path_buf(), hash)
        })
        .collect()
}

fn payload_path(record: &ParcelRecord) -> PathBuf {
    let root = record
        .received_path
        .as_ref()
        .expect("committed destination");
    let matches: Vec<_> = files(root)
        .into_iter()
        .filter(|path| path.file_name().unwrap() == "payload.bin")
        .collect();
    assert_eq!(matches.len(), 1, "exactly one committed payload");
    matches[0].clone()
}

fn assert_payload(record: &ParcelRecord, bytes: &[u8]) {
    assert_eq!(fs::read(payload_path(record)).unwrap(), bytes);
    assert_eq!(record.direction, Direction::Incoming);
    assert_eq!(record.state, DeliveryState::SavedReceiptReady);
}

fn shard(parcel: &Path, group: usize, index: usize) -> PathBuf {
    parcel.join(format!("groups/g{group:03}/s{index:03}.bin"))
}

fn corrupt(path: &Path) {
    let mut bytes = fs::read(path).unwrap();
    assert!(!bytes.is_empty());
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0x80;
    fs::write(path, bytes).unwrap();
}

fn assert_not_committed(profile: &Profile, parcel_id: &str, carrier: &Path) {
    assert!(profile.export_receipt(parcel_id, carrier).is_err());
    assert!(profile.deliveries().unwrap().iter().all(|record| {
        record.parcel_id != parcel_id || record.state != DeliveryState::SavedReceiptReady
    }));
}

#[test]
fn trusted_roundtrip_preserves_files_and_requires_the_return_receipt() {
    let pair = Pair::new(true);
    let text = b"SYNTHETIC FIXTURE: offline research notes\n";
    let binary = [0, 255, 1, 128, 0, 42];
    let a = pair.input("notes.txt", text);
    let b = pair.input("measurements.bin", &binary);
    let c = pair.input("empty.txt", b"");
    let outgoing = pair
        .alice
        .seal(&[a, b, c], &pair.bob_card.fingerprint, PASSWORD)
        .unwrap();
    assert_eq!(outgoing.state, DeliveryState::Ready);
    assert_eq!(outgoing.files, 3);
    assert_eq!(outgoing.total_bytes, (text.len() + binary.len()) as u64);
    let parcel = pair.export(&outgoing, "outbound-carrier");
    let after_export = pair.alice.deliveries().unwrap();
    assert_eq!(after_export.len(), 1);
    assert_eq!(after_export[0].state, DeliveryState::WaitingReceipt);
    // The carrier must not expose source names or recognizable cleartext.
    for path in files(&parcel) {
        assert!(!path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("notes.txt"));
        let bytes = fs::read(path).unwrap();
        assert!(!bytes.windows(text.len()).any(|window| window == text));
    }
    let inspected = pair.bob.inspect_parcel(&parcel).unwrap();
    assert!(inspected.recoverable);
    assert_eq!(inspected.sender, pair.alice_card.fingerprint);
    assert_eq!(inspected.recipient, pair.bob_card.fingerprint);
    let received = pair.bob.receive_parcel(&parcel, PASSWORD).unwrap();
    assert!(!received.duplicate);
    assert_eq!(received.repaired_shards, 0);
    let destination = received.record.received_path.as_ref().unwrap();
    for (name, expected) in [
        ("notes.txt", text.as_slice()),
        ("measurements.bin", binary.as_slice()),
        ("empty.txt", b"".as_slice()),
    ] {
        let matches: Vec<_> = files(destination)
            .into_iter()
            .filter(|path| path.file_name().unwrap() == name)
            .collect();
        assert_eq!(matches.len(), 1);
        assert_eq!(fs::read(&matches[0]).unwrap(), expected);
    }
    assert_eq!(
        pair.alice.deliveries().unwrap()[0].state,
        DeliveryState::WaitingReceipt
    );
    let receipt = pair
        .bob
        .export_receipt(&outgoing.parcel_id, &pair.root.join("return-carrier"))
        .unwrap();
    let confirmed = pair.alice.import_receipt(&receipt).unwrap();
    assert_eq!(confirmed.parcel_id, outgoing.parcel_id);
    assert_eq!(confirmed.state, DeliveryState::Confirmed);
    assert_eq!(
        pair.alice.import_receipt(&receipt).unwrap().state,
        DeliveryState::Confirmed
    );
}

#[test]
fn both_peers_must_pin_trust_and_wrong_recipient_or_password_never_commits() {
    let pair = Pair::new(false);
    let input = pair.input("payload.bin", b"synthetic confidential sample");
    assert!(pair
        .alice
        .seal(
            std::slice::from_ref(&input),
            &pair.bob_card.fingerprint,
            PASSWORD
        )
        .is_err());
    pair.alice
        .trust_contact(&pair.bob_card.fingerprint)
        .unwrap();
    let record = pair
        .alice
        .seal(&[input], &pair.bob_card.fingerprint, PASSWORD)
        .unwrap();
    let parcel = pair.export(&record, "carrier");
    assert!(pair.bob.receive_parcel(&parcel, PASSWORD).is_err());
    pair.bob
        .trust_contact(&pair.alice_card.fingerprint)
        .unwrap();
    assert!(pair
        .bob
        .receive_parcel(&parcel, "wrong-synthetic-password")
        .is_err());
    assert_not_committed(
        &pair.bob,
        &record.parcel_id,
        &pair.root.join("bad-receipts"),
    );
    let charlie = Profile::open(pair.root.join("charlie")).unwrap();
    charlie.initialize("Synthetic Charlie", PASSWORD).unwrap();
    charlie
        .import_contact(&pair.root.join("alice.card"))
        .unwrap();
    charlie.trust_contact(&pair.alice_card.fingerprint).unwrap();
    assert!(charlie.receive_parcel(&parcel, PASSWORD).is_err());
    assert_not_committed(
        &charlie,
        &record.parcel_id,
        &pair.root.join("charlie-receipts"),
    );
    let valid = pair.bob.receive_parcel(&parcel, PASSWORD).unwrap();
    assert_payload(&valid.record, b"synthetic confidential sample");
}

#[test]
fn identity_and_contact_cards_reject_reinitialization_and_signed_binding_changes() {
    let pair = Pair::new(true);
    assert!(pair.alice.initialize("Replacement", PASSWORD).is_err());
    let card_path = pair.root.join("altered.card");
    let mut card = serde_json::to_value(&pair.alice_card).unwrap();
    card["age_recipient"] = serde_json::Value::String(pair.bob_card.age_recipient.clone());
    fs::write(&card_path, serde_json::to_vec(&card).unwrap()).unwrap();
    assert!(pair.bob.import_contact(&card_path).is_err());
    assert_eq!(pair.bob.contacts().unwrap().len(), 1);
    assert_eq!(pair.bob.contacts().unwrap()[0].card, pair.alice_card);
    assert!(pair.bob.contacts().unwrap()[0].verified);
}

#[test]
fn tampered_envelope_or_receipt_cannot_create_or_confirm_a_delivery() {
    let pair = Pair::new(true);
    let record = pair.seal(b"signed envelope fixture");
    let parcel = pair.export(&record, "good-carrier");
    let damaged = pair.root.join("tampered.parcel");
    copy_tree(&parcel, &damaged);
    for name in ["envelope.json", "envelope-copy.json"] {
        let path = damaged.join(name);
        let mut envelope: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        envelope["recipient"] = serde_json::Value::String(pair.alice_card.fingerprint.clone());
        fs::write(path, serde_json::to_vec(&envelope).unwrap()).unwrap();
    }
    assert!(pair.bob.inspect_parcel(&damaged).is_err());
    assert!(pair.bob.receive_parcel(&damaged, PASSWORD).is_err());
    assert_not_committed(&pair.bob, &record.parcel_id, &pair.root.join("bad-return"));
    pair.bob.receive_parcel(&parcel, PASSWORD).unwrap();
    let receipt = pair
        .bob
        .export_receipt(&record.parcel_id, &pair.root.join("good-return"))
        .unwrap();
    let changed = pair.root.join("tampered.receipt");
    copy_tree(&receipt, &changed);
    corrupt(&changed.join("receipt.json"));
    assert!(pair.alice.import_receipt(&changed).is_err());
    assert_eq!(
        pair.alice.deliveries().unwrap()[0].state,
        DeliveryState::WaitingReceipt
    );
    assert_eq!(
        pair.alice.import_receipt(&receipt).unwrap().state,
        DeliveryState::Confirmed
    );
}

#[test]
fn duplicate_import_is_idempotent_and_destination_changes_are_not_overwritten() {
    let pair = Pair::new(true);
    let record = pair.seal(b"immutable original synthetic bytes");
    let parcel = pair.export(&record, "carrier");
    let first = pair.bob.receive_parcel(&parcel, PASSWORD).unwrap();
    let before = tree_digest(first.record.received_path.as_ref().unwrap());
    let duplicate = pair.bob.receive_parcel(&parcel, PASSWORD).unwrap();
    assert!(duplicate.duplicate);
    assert_eq!(duplicate.record.received_path, first.record.received_path);
    assert_eq!(pair.bob.deliveries().unwrap().len(), 1);
    assert_eq!(
        tree_digest(duplicate.record.received_path.as_ref().unwrap()),
        before
    );
    let path = payload_path(&first.record);
    fs::write(&path, b"EXTERNAL MODIFICATION - must survive retry").unwrap();
    assert!(pair.bob.receive_parcel(&parcel, PASSWORD).is_err());
    assert_eq!(
        fs::read(&path).unwrap(),
        b"EXTERNAL MODIFICATION - must survive retry"
    );
    let summary = pair.bob.recover(PASSWORD).unwrap();
    assert!(!summary.conflicts.is_empty());
    assert_eq!(
        fs::read(&path).unwrap(),
        b"EXTERNAL MODIFICATION - must survive retry"
    );
}

#[test]
fn retries_reuse_exact_bytes_and_cancel_stops_only_future_local_exports() {
    let pair = Pair::new(true);
    let record = pair.seal(b"the same immutable parcel");
    let first = pair.export(&record, "carrier-a");
    let second = pair.export(&record, "carrier-b");
    assert_eq!(tree_digest(&first), tree_digest(&second));
    let replacement = pair.seal(b"the same immutable parcel");
    assert_ne!(record.parcel_id, replacement.parcel_id);
    pair.alice.cancel(&record.parcel_id).unwrap();
    let cancelled = pair
        .alice
        .deliveries()
        .unwrap()
        .into_iter()
        .find(|r| r.parcel_id == record.parcel_id)
        .unwrap();
    assert_eq!(cancelled.state, DeliveryState::Cancelled);
    assert!(pair
        .alice
        .export_parcel(&record.parcel_id, &pair.root.join("after-cancel"))
        .is_err());
    assert_payload(
        &pair.bob.receive_parcel(&first, PASSWORD).unwrap().record,
        b"the same immutable parcel",
    );
    assert_eq!(
        fs::read(pair.root.join("input/payload.bin")).unwrap(),
        b"the same immutable parcel"
    );
}

#[test]
fn full_group_recovers_zero_to_four_missing_or_corrupt_source_and_parity_shards() {
    let pair = Pair::new(true);
    let payload: Vec<u8> = (0..21 * MIB)
        .map(|i| (i.wrapping_mul(31) % 251) as u8)
        .collect();
    let record = pair.seal(&payload);
    let pristine = pair.export(&record, "pristine");
    let health = pair.bob.inspect_parcel(&pristine).unwrap();
    assert_eq!(health.groups[0].data_shards, 20);
    assert_eq!(health.groups[0].parity_shards, 4);
    assert!(health.groups.len() >= 2);
    for count in 0..=4 {
        for mode in ["missing-data", "corrupt-parity", "mixed"] {
            let label = format!("{mode}-{count}");
            let parcel = pair.root.join(format!("{label}.parcel"));
            copy_tree(&pristine, &parcel);
            let receiver = pair.isolated_receiver(&format!("receiver-{label}"));
            for offset in 0..count {
                match mode {
                    "missing-data" => fs::remove_file(shard(&parcel, 0, offset)).unwrap(),
                    "corrupt-parity" => corrupt(&shard(&parcel, 0, 20 + offset)),
                    _ if offset % 2 == 0 => fs::remove_file(shard(&parcel, 0, offset)).unwrap(),
                    _ => corrupt(&shard(&parcel, 0, 20 + offset / 2)),
                }
            }
            let inspection = receiver.inspect_parcel(&parcel).unwrap();
            assert!(inspection.recoverable, "{label}");
            assert_eq!(inspection.groups[0].missing_shards.len(), count, "{label}");
            let received = receiver.receive_parcel(&parcel, PASSWORD).unwrap();
            assert_eq!(received.repaired_shards, count, "{label}");
            assert!(!received.duplicate, "{label}");
            assert_payload(&received.record, &payload);
        }
    }
    let parcel = pair.root.join("five-bad.parcel");
    copy_tree(&pristine, &parcel);
    for index in [0, 1, 2, 20, 23] {
        corrupt(&shard(&parcel, 0, index));
    }
    let receiver = pair.isolated_receiver("receiver-five-bad");
    assert!(!receiver.inspect_parcel(&parcel).unwrap().recoverable);
    assert!(receiver.receive_parcel(&parcel, PASSWORD).is_err());
    assert_not_committed(
        &receiver,
        &record.parcel_id,
        &pair.root.join("five-bad-receipts"),
    );
}

#[test]
fn adaptive_tail_budget_is_real_and_cannot_be_borrowed_from_other_groups() {
    let pair = Pair::new(true);
    // These payloads produce tails well away from ciphertext boundary overhead.
    for (ordinal, payload_len) in [17, 5 * MIB + 17, 11 * MIB + 17, 17 * MIB + 17, 21 * MIB]
        .into_iter()
        .enumerate()
    {
        let payload: Vec<u8> = (0..payload_len).map(|i| (i % 251) as u8).collect();
        let record = pair.seal(&payload);
        let parcel = pair.export(&record, &format!("tail-{ordinal}"));
        let inspection = pair.bob.inspect_parcel(&parcel).unwrap();
        let tail = inspection.groups.last().unwrap();
        assert!((1..=19).contains(&tail.data_shards));
        assert_eq!(tail.parity_shards, tail.data_shards.div_ceil(5));
        let budget = tail.parity_shards;
        let group_index = tail.index;
        for excess in [0, 1] {
            let case = pair
                .root
                .join(format!("tail-{ordinal}-excess-{excess}.parcel"));
            copy_tree(&parcel, &case);
            let receiver = pair.isolated_receiver(&format!("tail-receiver-{ordinal}-{excess}"));
            // Lose every parity shard and optionally one data shard, validating
            // that parity is part of the same per-group erasure budget.
            for index in tail.data_shards..tail.data_shards + budget {
                corrupt(&shard(&case, group_index, index));
            }
            if excess == 1 {
                fs::remove_file(shard(&case, group_index, 0)).unwrap();
            }
            let damaged = receiver.inspect_parcel(&case).unwrap();
            assert_eq!(damaged.recoverable, excess == 0);
            if excess == 0 {
                let result = receiver.receive_parcel(&case, PASSWORD).unwrap();
                assert_eq!(result.repaired_shards, budget);
                assert_payload(&result.record, &payload);
            } else {
                assert!(receiver.receive_parcel(&case, PASSWORD).is_err());
                assert_not_committed(
                    &receiver,
                    &record.parcel_id,
                    &pair.root.join("tail-failed-return"),
                );
            }
        }
    }
}

#[test]
fn truncated_or_extended_shards_are_erasures_and_missing_envelopes_fail_closed() {
    let pair = Pair::new(true);
    let payload = vec![73; 2 * MIB];
    let record = pair.seal(&payload);
    let parcel = pair.export(&record, "carrier");
    for extended in [false, true] {
        let case = pair.root.join(format!("length-{extended}.parcel"));
        copy_tree(&parcel, &case);
        let path = shard(&case, 0, 0);
        let mut bytes = fs::read(&path).unwrap();
        if extended {
            bytes.push(1);
        } else {
            bytes.pop();
        }
        fs::write(&path, bytes).unwrap();
        let receiver = pair.isolated_receiver(&format!("length-receiver-{extended}"));
        let inspection = receiver.inspect_parcel(&case).unwrap();
        assert_eq!(inspection.groups[0].missing_shards, vec![0]);
        let result = receiver.receive_parcel(&case, PASSWORD).unwrap();
        assert_eq!(result.repaired_shards, 1);
        assert_payload(&result.record, &payload);
    }
    for name in ["envelope.json", "envelope-copy.json"] {
        fs::remove_file(parcel.join(name)).unwrap();
    }
    assert!(pair.bob.inspect_parcel(&parcel).is_err());
    assert!(pair.bob.receive_parcel(&parcel, PASSWORD).is_err());
    assert_not_committed(
        &pair.bob,
        &record.parcel_id,
        &pair.root.join("no-envelope-return"),
    );
}

#[test]
fn recovery_reindexes_verified_published_content_and_regenerates_the_receipt() {
    let pair = Pair::new(true);
    let payload = b"published before index: synthetic crash-window fixture";
    let record = pair.seal(payload);
    let parcel = pair.export(&record, "carrier");
    let received = pair.bob.receive_parcel(&parcel, PASSWORD).unwrap();
    let published = received.record.received_path.clone().unwrap();
    let digest = tree_digest(&published);
    // Reproduce the durable state after directory publication but before the
    // index and receipt are durable. This is not a physical power-loss test.
    let connection = rusqlite::Connection::open(pair.root.join("bob/state.sqlite3")).unwrap();
    assert_eq!(
        connection
            .execute("DELETE FROM deliveries WHERE direction = 'Incoming'", [])
            .unwrap(),
        1
    );
    drop(connection);
    let receipts = pair.root.join("bob/receipts").join(&record.parcel_id);
    fs::remove_dir_all(&receipts).unwrap();
    let restarted = Profile::open(pair.root.join("bob")).unwrap();
    assert!(restarted.deliveries().unwrap().is_empty());
    let summary = restarted.recover(PASSWORD).unwrap();
    assert_eq!(summary.recovered, 1);
    assert_eq!(summary.pending, 0);
    assert!(summary.conflicts.is_empty());
    let recovered = restarted.deliveries().unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].received_path.as_ref().unwrap(), &published);
    assert_payload(&recovered[0], payload);
    assert_eq!(tree_digest(&published), digest);
    let receipt = restarted
        .export_receipt(&record.parcel_id, &pair.root.join("return"))
        .unwrap();
    assert_eq!(
        pair.alice.import_receipt(&receipt).unwrap().state,
        DeliveryState::Confirmed
    );
    let repeated = restarted.recover(PASSWORD).unwrap();
    assert_eq!(repeated.recovered, 0);
    assert!(repeated.conflicts.is_empty());
    assert_eq!(restarted.deliveries().unwrap().len(), 1);
}

#[test]
fn recovery_never_signs_success_for_externally_modified_unindexed_content() {
    let pair = Pair::new(true);
    let record = pair.seal(b"verified content before simulated crash");
    let parcel = pair.export(&record, "carrier");
    let received = pair.bob.receive_parcel(&parcel, PASSWORD).unwrap();
    let path = payload_path(&received.record);
    let connection = rusqlite::Connection::open(pair.root.join("bob/state.sqlite3")).unwrap();
    connection
        .execute("DELETE FROM deliveries WHERE direction = 'Incoming'", [])
        .unwrap();
    drop(connection);
    fs::remove_dir_all(pair.root.join("bob/receipts").join(&record.parcel_id)).unwrap();
    fs::write(&path, b"externally changed after publication").unwrap();
    let restarted = Profile::open(pair.root.join("bob")).unwrap();
    let summary = restarted.recover(PASSWORD).unwrap();
    assert_eq!(summary.recovered, 0);
    assert!(!summary.conflicts.is_empty());
    assert_eq!(
        fs::read(&path).unwrap(),
        b"externally changed after publication"
    );
    assert_not_committed(
        &restarted,
        &record.parcel_id,
        &pair.root.join("must-not-exist-return"),
    );
    assert!(restarted.receive_parcel(&parcel, PASSWORD).is_err());
}

#[test]
fn recovery_detects_unexpected_files_in_a_previously_committed_batch() {
    let pair = Pair::new(true);
    let record = pair.seal(b"exact file set is committed");
    let parcel = pair.export(&record, "carrier");
    let received = pair.bob.receive_parcel(&parcel, PASSWORD).unwrap();
    let destination = received.record.received_path.as_ref().unwrap();
    let unexpected = destination.join("external-extra.txt");
    fs::write(&unexpected, b"not in authenticated manifest").unwrap();
    assert!(pair.bob.receive_parcel(&parcel, PASSWORD).is_err());
    let recovery = pair.bob.recover(PASSWORD).unwrap();
    assert!(!recovery.conflicts.is_empty());
    assert_eq!(
        fs::read(unexpected).unwrap(),
        b"not in authenticated manifest"
    );
}

#[cfg(unix)]
#[test]
fn unsafe_inputs_are_rejected_before_an_outgoing_delivery_exists() {
    use std::os::unix::fs::symlink;
    let pair = Pair::new(true);
    let source = pair.input("source.txt", b"regular synthetic fixture");
    let link = pair.root.join("input/link.txt");
    symlink(&source, &link).unwrap();
    assert!(pair
        .alice
        .seal(&[link], &pair.bob_card.fingerprint, PASSWORD)
        .is_err());
    assert!(pair
        .alice
        .seal(
            &[pair.root.join("input")],
            &pair.bob_card.fingerprint,
            PASSWORD
        )
        .is_err());
    assert!(pair
        .alice
        .seal(&[], &pair.bob_card.fingerprint, PASSWORD)
        .is_err());
    let mut inputs = Vec::new();
    for index in 0..101 {
        inputs.push(pair.input(&format!("limit-{index}.txt"), b""));
    }
    assert!(pair
        .alice
        .seal(&inputs, &pair.bob_card.fingerprint, PASSWORD)
        .is_err());
    let oversized = pair.root.join("input/oversized.bin");
    fs::File::create(&oversized)
        .unwrap()
        .set_len((64 * MIB + 1) as u64)
        .unwrap();
    assert!(pair
        .alice
        .seal(&[oversized], &pair.bob_card.fingerprint, PASSWORD)
        .is_err());
    let same_name = pair.root.join("another/source.txt");
    fs::create_dir_all(same_name.parent().unwrap()).unwrap();
    fs::write(&same_name, b"different bytes, colliding archive name").unwrap();
    assert!(pair
        .alice
        .seal(&[source, same_name], &pair.bob_card.fingerprint, PASSWORD)
        .is_err());
    assert!(pair.alice.deliveries().unwrap().is_empty());
}

#[test]
fn unverified_display_names_cannot_shadow_a_pinned_full_fingerprint() {
    let pair = Pair::new(true);
    let impostor_a = Profile::open(pair.root.join("impostor-a")).unwrap();
    let impostor_b = Profile::open(pair.root.join("impostor-b")).unwrap();
    // These cards are valid and self-signed, but have never been trusted.
    impostor_a.initialize(&pair.bob_card.fingerprint, PASSWORD).unwrap();
    impostor_b.initialize(&pair.alice_card.fingerprint, PASSWORD).unwrap();
    let card_a = pair.root.join("impostor-a.card");
    let card_b = pair.root.join("impostor-b.card");
    impostor_a.export_card(&card_a).unwrap();
    impostor_b.export_card(&card_b).unwrap();
    assert!(!pair.alice.import_contact(&card_a).unwrap().verified);
    assert!(!pair.bob.import_contact(&card_b).unwrap().verified);
    let record = pair.seal(b"full fingerprints select the pinned identities");
    assert_eq!(record.recipient, pair.bob_card.fingerprint);
    let parcel = pair.export(&record, "carrier-with-name-spoofs");
    let inspection = pair.bob.inspect_parcel(&parcel).unwrap();
    assert_eq!(inspection.sender, pair.alice_card.fingerprint);
    assert_eq!(inspection.recipient, pair.bob_card.fingerprint);
    assert_payload(&pair.bob.receive_parcel(&parcel, PASSWORD).unwrap().record,
        b"full fingerprints select the pinned identities");
}

fn outgoing_state(profile: &Profile, parcel_id: &str) -> DeliveryState {
    profile.deliveries().unwrap().into_iter()
        .find(|record| record.direction == Direction::Outgoing && record.parcel_id == parcel_id)
        .expect("known outgoing delivery").state
}

#[test]
fn outgoing_recovery_marks_active_damage_failed_and_restores_only_verified_parcels() {
    let pair = Pair::new(true);
    let record = pair.seal(b"durable outgoing failure and restoration fixture");
    let original = record.parcel_path.as_ref().unwrap();
    let backup = pair.root.join("synthetic-outbox-backup");
    copy_tree(original, &backup);
    corrupt(&shard(original, 0, 0));
    assert!(!pair.alice.recover(PASSWORD).unwrap().conflicts.is_empty());
    assert_eq!(outgoing_state(&pair.alice, &record.parcel_id), DeliveryState::Failed);
    fs::copy(shard(&backup, 0, 0), shard(original, 0, 0)).unwrap();
    assert!(pair.alice.recover(PASSWORD).unwrap().conflicts.is_empty());
    assert_eq!(outgoing_state(&pair.alice, &record.parcel_id), DeliveryState::Ready);
    fs::remove_dir_all(original).unwrap();
    assert!(!pair.alice.recover(PASSWORD).unwrap().conflicts.is_empty());
    assert_eq!(outgoing_state(&pair.alice, &record.parcel_id), DeliveryState::Failed);
    copy_tree(&backup, original);
    assert!(pair.alice.recover(PASSWORD).unwrap().conflicts.is_empty());
    assert_eq!(outgoing_state(&pair.alice, &record.parcel_id), DeliveryState::Ready);

    pair.alice.cancel(&record.parcel_id).unwrap();
    corrupt(&shard(original, 0, 0));
    pair.alice.recover(PASSWORD).unwrap();
    assert_eq!(outgoing_state(&pair.alice, &record.parcel_id), DeliveryState::Cancelled);
    fs::remove_dir_all(original).unwrap();
    pair.alice.recover(PASSWORD).unwrap();
    assert_eq!(outgoing_state(&pair.alice, &record.parcel_id), DeliveryState::Cancelled);
    copy_tree(&backup, original);
    pair.alice.recover(PASSWORD).unwrap();
    assert_eq!(outgoing_state(&pair.alice, &record.parcel_id), DeliveryState::Cancelled);
    assert!(pair.alice.export_parcel(&record.parcel_id, &pair.root.join("cancel-still-blocks")).is_err());
}

#[test]
fn verified_acknowledgments_override_failed_indexes_and_confirmation_survives_outbox_loss() {
    let pair = Pair::new(true);
    let record = pair.seal(b"confirmation is a durable recipient statement");
    let original = record.parcel_path.as_ref().unwrap();
    let parcel = pair.export(&record, "confirmed-carrier");
    pair.bob.receive_parcel(&parcel, PASSWORD).unwrap();
    let receipt = pair.bob.export_receipt(&record.parcel_id, &pair.root.join("confirmed-return")).unwrap();
    let mut confirmed = pair.alice.import_receipt(&receipt).unwrap();
    assert_eq!(confirmed.state, DeliveryState::Confirmed);
    // Reproduce an index update lost after the verified acknowledgment became
    // durable. Recovery must derive confirmation from the acknowledgment.
    confirmed.state = DeliveryState::Failed;
    let database = rusqlite::Connection::open(pair.root.join("alice/state.sqlite3")).unwrap();
    database.execute("UPDATE deliveries SET record_json = ?1 WHERE parcel_id = ?2 AND direction = 'Outgoing'",
        rusqlite::params![serde_json::to_string(&confirmed).unwrap(), record.parcel_id]).unwrap();
    drop(database);
    assert_eq!(outgoing_state(&pair.alice, &record.parcel_id), DeliveryState::Failed);
    pair.alice.recover(PASSWORD).unwrap();
    assert_eq!(outgoing_state(&pair.alice, &record.parcel_id), DeliveryState::Confirmed);
    corrupt(&shard(original, 0, 0));
    pair.alice.recover(PASSWORD).unwrap();
    assert_eq!(outgoing_state(&pair.alice, &record.parcel_id), DeliveryState::Confirmed);
    fs::remove_dir_all(original).unwrap();
    pair.alice.recover(PASSWORD).unwrap();
    assert_eq!(outgoing_state(&pair.alice, &record.parcel_id), DeliveryState::Confirmed);
}

#[test]
fn exports_cannot_write_into_any_managed_profile_tree() {
    let pair = Pair::new(true);
    let record = pair.seal(b"managed files must remain immutable on export");
    let parcel = pair.export(&record, "external-carrier");
    let received = pair.bob.receive_parcel(&parcel, PASSWORD).unwrap();
    let input = pair.input("return-payload.txt", b"independent reverse direction");
    let reverse = pair.bob.seal(&[input], &pair.alice_card.fingerprint, PASSWORD).unwrap();
    let bob_home = pair.root.join("bob");
    for directory in [bob_home.clone(), bob_home.join("identity"), bob_home.join("outbox")] {
        let identity_before = tree_digest(&bob_home.join("identity"));
        let outbox_before = tree_digest(&bob_home.join("outbox"));
        let target = directory.join("forbidden-export.card");
        assert!(pair.bob.export_card(&target).is_err());
        assert!(!target.exists());
        assert_eq!(tree_digest(&bob_home.join("identity")), identity_before);
        assert_eq!(tree_digest(&bob_home.join("outbox")), outbox_before);
    }
    for carrier in [received.record.received_path.as_ref().unwrap().clone(),
        bob_home.join("receipts").join(&record.parcel_id), bob_home.join("outbox")] {
        let before = tree_digest(&carrier);
        assert!(pair.bob.export_parcel(&reverse.parcel_id, &carrier).is_err());
        assert!(pair.bob.export_receipt(&record.parcel_id, &carrier).is_err());
        assert_eq!(tree_digest(&carrier), before);
    }
    assert_payload(&pair.bob.receive_parcel(&parcel, PASSWORD).unwrap().record,
        b"managed files must remain immutable on export");
    let receipt = pair.bob.export_receipt(&record.parcel_id, &pair.root.join("valid-external-return")).unwrap();
    assert_eq!(pair.alice.import_receipt(&receipt).unwrap().state, DeliveryState::Confirmed);
}
