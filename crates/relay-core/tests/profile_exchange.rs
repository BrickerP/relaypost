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
    impostor_a
        .initialize(&pair.bob_card.fingerprint, PASSWORD)
        .unwrap();
    impostor_b
        .initialize(&pair.alice_card.fingerprint, PASSWORD)
        .unwrap();
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
    assert_payload(
        &pair.bob.receive_parcel(&parcel, PASSWORD).unwrap().record,
        b"full fingerprints select the pinned identities",
    );
}

fn outgoing_record(profile: &Profile, parcel_id: &str) -> ParcelRecord {
    profile
        .deliveries()
        .unwrap()
        .into_iter()
        .find(|record| record.direction == Direction::Outgoing && record.parcel_id == parcel_id)
        .expect("known outgoing delivery")
}

fn outgoing_state(profile: &Profile, parcel_id: &str) -> DeliveryState {
    outgoing_record(profile, parcel_id).state
}

#[test]
fn outgoing_health_preserves_each_delivery_phase_and_clears_only_after_exact_restoration() {
    let pair = Pair::new(true);
    for (ordinal, phase) in [
        DeliveryState::Ready,
        DeliveryState::WaitingReceipt,
        DeliveryState::Cancelled,
    ]
    .into_iter()
    .enumerate()
    {
        let record = pair.seal(b"durable outgoing health and restoration fixture");
        match phase {
            DeliveryState::WaitingReceipt => {
                pair.export(&record, "exported-before-local-damage");
            }
            DeliveryState::Cancelled => pair.alice.cancel(&record.parcel_id).unwrap(),
            _ => {}
        }
        let original = record.parcel_path.as_ref().unwrap();
        let backup = pair.root.join(format!("synthetic-outbox-backup-{ordinal}"));
        copy_tree(original, &backup);
        for missing in [false, true] {
            if missing {
                fs::remove_dir_all(original).unwrap();
            } else {
                corrupt(&shard(original, 0, 0));
            }
            assert!(!pair.alice.recover(PASSWORD).unwrap().conflicts.is_empty());
            let damaged = outgoing_record(&pair.alice, &record.parcel_id);
            assert_eq!(damaged.state, phase);
            assert!(damaged.local_error.is_some());
            assert!(pair
                .alice
                .export_parcel(&record.parcel_id, &pair.root.join("damaged-export"))
                .is_err());
            if missing {
                copy_tree(&backup, original);
            } else {
                fs::copy(shard(&backup, 0, 0), shard(original, 0, 0)).unwrap();
            }
            assert!(pair.alice.recover(PASSWORD).unwrap().conflicts.is_empty());
            let restored = outgoing_record(&pair.alice, &record.parcel_id);
            assert_eq!(restored.state, phase);
            assert!(restored.local_error.is_none());
        }
        if phase == DeliveryState::Cancelled {
            assert!(pair
                .alice
                .export_parcel(&record.parcel_id, &pair.root.join("cancel-still-blocks"))
                .is_err());
        }
    }
}

#[test]
fn durable_acknowledgments_rebuild_stale_or_missing_indexes_without_healthy_source_bytes() {
    let pair = Pair::new(true);
    let record = pair.seal(b"confirmation is a durable recipient statement");
    let parcel = pair.export(&record, "confirmed-carrier");
    pair.bob.receive_parcel(&parcel, PASSWORD).unwrap();
    let receipt = pair
        .bob
        .export_receipt(&record.parcel_id, &pair.root.join("confirmed-return"))
        .unwrap();
    let confirmed = pair.alice.import_receipt(&receipt).unwrap();
    assert_eq!(confirmed.state, DeliveryState::Confirmed);
    assert_eq!(files(&receipt).len(), 2, "transport receipts stay minimal");
    let acknowledgment = pair
        .root
        .join("alice/acknowledgments")
        .join(&record.parcel_id);
    assert_eq!(files(&acknowledgment).len(), 4);
    for missing_index in [false, true] {
        for missing_outbox in [false, true] {
            let home = pair
                .root
                .join(format!("ack-restart-{missing_index}-{missing_outbox}"));
            copy_tree(&pair.root.join("alice"), &home);
            let restarted = Profile::open(&home).unwrap();
            let database = rusqlite::Connection::open(home.join("state.sqlite3")).unwrap();
            if missing_index {
                database
                    .execute("DELETE FROM deliveries WHERE direction = 'Outgoing'", [])
                    .unwrap();
            } else {
                // The acknowledgment directory survived, but the last database
                // update did not. A fresh WaitingReceipt row must not lose proof.
                let mut stale = confirmed.clone();
                stale.state = DeliveryState::WaitingReceipt;
                stale.receipt_path = None;
                stale.parcel_path = Some(home.join("outbox").join(&record.parcel_id));
                database
                    .execute(
                        "UPDATE deliveries SET record_json = ?1 WHERE direction = 'Outgoing'",
                        [serde_json::to_string(&stale).unwrap()],
                    )
                    .unwrap();
            }
            drop(database);
            let source = home.join("outbox").join(&record.parcel_id);
            if missing_outbox {
                fs::remove_dir_all(&source).unwrap();
            } else {
                corrupt(&shard(&source, 0, 0));
            }
            let summary = restarted.recover(PASSWORD).unwrap();
            assert!(
                !summary.conflicts.is_empty(),
                "source damage remains visible"
            );
            let recovered = outgoing_record(&restarted, &record.parcel_id);
            assert_eq!(recovered.state, DeliveryState::Confirmed);
            assert!(recovered.local_error.is_some());
            assert_eq!(
                recovered.receipt_path,
                Some(home.join("acknowledgments").join(&record.parcel_id))
            );
            restarted.recover(PASSWORD).unwrap();
            assert_eq!(
                outgoing_state(&restarted, &record.parcel_id),
                DeliveryState::Confirmed
            );
            if !missing_outbox {
                fs::remove_dir_all(&source).unwrap();
            }
            copy_tree(record.parcel_path.as_ref().unwrap(), &source);
            assert!(restarted.recover(PASSWORD).unwrap().conflicts.is_empty());
            let restored = outgoing_record(&restarted, &record.parcel_id);
            assert_eq!(restored.state, DeliveryState::Confirmed);
            assert!(restored.local_error.is_none());
        }
    }
}

#[test]
fn damaged_acknowledgment_evidence_cannot_confirm_a_missing_outbox() {
    let pair = Pair::new(true);
    let record = pair.seal(b"all four acknowledgment files must authenticate");
    let parcel = pair.export(&record, "ack-tamper-carrier");
    pair.bob.receive_parcel(&parcel, PASSWORD).unwrap();
    let receipt = pair
        .bob
        .export_receipt(&record.parcel_id, &pair.root.join("ack-tamper-return"))
        .unwrap();
    let confirmed = pair.alice.import_receipt(&receipt).unwrap();
    for name in [
        "receipt.json",
        "receipt.minisig",
        "envelope.json",
        "envelope.minisig",
    ] {
        for missing_index in [false, true] {
            let home = pair
                .root
                .join(format!("invalid-ack-{name}-{missing_index}"));
            copy_tree(&pair.root.join("alice"), &home);
            let database = rusqlite::Connection::open(home.join("state.sqlite3")).unwrap();
            if missing_index {
                database
                    .execute("DELETE FROM deliveries WHERE direction = 'Outgoing'", [])
                    .unwrap();
            } else {
                let mut stale = confirmed.clone();
                stale.state = DeliveryState::WaitingReceipt;
                stale.receipt_path = None;
                stale.parcel_path = Some(home.join("outbox").join(&record.parcel_id));
                database
                    .execute(
                        "UPDATE deliveries SET record_json = ?1 WHERE direction = 'Outgoing'",
                        [serde_json::to_string(&stale).unwrap()],
                    )
                    .unwrap();
            }
            drop(database);
            fs::remove_dir_all(home.join("outbox").join(&record.parcel_id)).unwrap();
            corrupt(
                &home
                    .join("acknowledgments")
                    .join(&record.parcel_id)
                    .join(name),
            );
            let restarted = Profile::open(&home).unwrap();
            let summary = restarted.recover(PASSWORD).unwrap();
            assert!(!summary.conflicts.is_empty());
            let rows = restarted.deliveries().unwrap();
            assert!(rows.iter().all(|row| row.state != DeliveryState::Confirmed));
            if !missing_index {
                let outgoing = outgoing_record(&restarted, &record.parcel_id);
                assert_eq!(outgoing.state, DeliveryState::WaitingReceipt);
                assert!(outgoing.local_error.is_some());
            }
        }
    }
}

fn remove_incoming_index_and_receipt(pair: &Pair, parcel_id: &str) {
    let database = rusqlite::Connection::open(pair.root.join("bob/state.sqlite3")).unwrap();
    assert_eq!(
        database
            .execute(
                "DELETE FROM deliveries WHERE direction = 'Incoming' AND parcel_id = ?1",
                [parcel_id],
            )
            .unwrap(),
        1
    );
    drop(database);
    fs::remove_dir_all(pair.root.join("bob/receipts").join(parcel_id)).unwrap();
}

// Preserve real authenticated metadata from an actual synthetic receive, then
// recreate one durable crash window without exposing production fault controls.
fn stage_received_batch(pair: &Pair, received: &ParcelRecord, keep_published: bool) -> PathBuf {
    let staging = pair.root.join("bob/inbox/.staging/batch-SYNTHETIC-crash");
    fs::create_dir(&staging).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staging, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let published = received.received_path.as_ref().unwrap();
    if keep_published {
        copy_tree(published, &staging.join("batch"));
    } else {
        fs::rename(published, staging.join("batch")).unwrap();
    }
    fs::write(
        staging.join("archive.tar"),
        b"SYNTHETIC retained plaintext archive",
    )
    .unwrap();
    fs::write(
        staging.join("ciphertext.age"),
        b"SYNTHETIC retained ciphertext",
    )
    .unwrap();
    staging
}

#[test]
fn recovery_publishes_a_fully_prepared_stage_before_regenerating_its_receipt() {
    let pair = Pair::new(true);
    let payload = b"prepared but not yet published synthetic content";
    let outgoing = pair.seal(payload);
    let parcel = pair.export(&outgoing, "prepublication-carrier");
    let received = pair.bob.receive_parcel(&parcel, PASSWORD).unwrap().record;
    let published = received.received_path.as_ref().unwrap();
    let digest = tree_digest(published);
    let stage = stage_received_batch(&pair, &received, false);
    remove_incoming_index_and_receipt(&pair, &outgoing.parcel_id);
    assert!(!published.exists());
    assert_not_committed(
        &pair.bob,
        &outgoing.parcel_id,
        &pair.root.join("early-return"),
    );
    let restarted = Profile::open(pair.root.join("bob")).unwrap();
    let summary = restarted.recover(PASSWORD).unwrap();
    assert_eq!(summary.recovered, 1);
    assert_eq!(summary.pending, 0);
    assert!(summary.conflicts.is_empty());
    assert!(!stage.exists(), "plaintext staging wrapper must be removed");
    assert_eq!(tree_digest(published), digest);
    let rows = restarted.deliveries().unwrap();
    assert_eq!(rows.len(), 1);
    assert_payload(&rows[0], payload);
    let receipt = restarted
        .export_receipt(&outgoing.parcel_id, &pair.root.join("resumed-return"))
        .unwrap();
    assert_eq!(
        pair.alice.import_receipt(&receipt).unwrap().state,
        DeliveryState::Confirmed
    );
    assert_eq!(restarted.recover(PASSWORD).unwrap().recovered, 0);
}

#[test]
fn recovery_removes_a_superseded_matching_stage_and_its_plaintext_archive() {
    let pair = Pair::new(true);
    let outgoing = pair.seal(b"already published synthetic content");
    let parcel = pair.export(&outgoing, "postpublication-carrier");
    let received = pair.bob.receive_parcel(&parcel, PASSWORD).unwrap().record;
    let published = received.received_path.as_ref().unwrap();
    let digest = tree_digest(published);
    let stage = stage_received_batch(&pair, &received, true);
    let summary = pair.bob.recover(PASSWORD).unwrap();
    assert_eq!(summary.pending, 0);
    assert!(summary.conflicts.is_empty());
    assert!(!stage.exists());
    assert_eq!(tree_digest(published), digest);
    assert_payload(
        &pair.bob.deliveries().unwrap()[0],
        b"already published synthetic content",
    );
}

#[test]
fn recovery_cleans_actual_postpublication_residue_only_while_the_final_batch_verifies() {
    let pair = Pair::new(true);
    let outgoing = pair.seal(b"actual postpublication wrapper has no batch child");
    let parcel = pair.export(&outgoing, "actual-postpublication-carrier");
    let received = pair.bob.receive_parcel(&parcel, PASSWORD).unwrap().record;
    let published = received.received_path.as_ref().unwrap();
    let digest = tree_digest(published);
    for changed_final in [false, true] {
        let stage = pair
            .root
            .join("bob/inbox/.staging")
            .join(format!("batch-{}-fixture", outgoing.parcel_id));
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("archive.tar"), b"SYNTHETIC remaining plaintext").unwrap();
        fs::write(
            stage.join("ciphertext.age"),
            b"SYNTHETIC remaining ciphertext",
        )
        .unwrap();
        assert!(!stage.join("batch").exists());
        let staged_digest = tree_digest(&stage);
        if changed_final {
            fs::write(payload_path(&received), b"EXTERNAL postpublication edit").unwrap();
        }
        let summary = pair.bob.recover(PASSWORD).unwrap();
        if changed_final {
            assert_eq!(summary.pending, 1);
            assert!(!summary.conflicts.is_empty());
            assert_eq!(tree_digest(&stage), staged_digest);
            assert_eq!(
                fs::read(payload_path(&received)).unwrap(),
                b"EXTERNAL postpublication edit"
            );
            assert_not_committed(
                &pair.bob,
                &outgoing.parcel_id,
                &pair.root.join("changed-postpublication-return"),
            );
        } else {
            assert_eq!(summary.pending, 0);
            assert!(summary.conflicts.is_empty());
            assert!(!stage.exists());
            assert_eq!(tree_digest(published), digest);
        }
    }
}

#[test]
fn recovery_preserves_the_only_good_stage_when_published_content_conflicts() {
    let pair = Pair::new(true);
    let outgoing = pair.seal(b"good staged evidence must remain recoverable");
    let parcel = pair.export(&outgoing, "stage-conflict-carrier");
    let received = pair.bob.receive_parcel(&parcel, PASSWORD).unwrap().record;
    let stage = stage_received_batch(&pair, &received, true);
    let staged_digest = tree_digest(&stage);
    let published_payload = payload_path(&received);
    fs::write(&published_payload, b"EXTERNAL EDIT must not be replaced").unwrap();
    let summary = pair.bob.recover(PASSWORD).unwrap();
    assert_eq!(summary.pending, 1);
    assert!(!summary.conflicts.is_empty());
    assert_eq!(tree_digest(&stage), staged_digest);
    assert_eq!(
        fs::read(published_payload).unwrap(),
        b"EXTERNAL EDIT must not be replaced"
    );
    assert_not_committed(
        &pair.bob,
        &outgoing.parcel_id,
        &pair.root.join("conflict-return"),
    );
}

#[test]
fn recovery_keeps_corrupt_prepared_stages_pending_without_publication_or_receipt() {
    let pair = Pair::new(true);
    let outgoing = pair.seal(b"a prepared marker alone grants no commit authority");
    let parcel = pair.export(&outgoing, "invalid-stage-carrier");
    let received = pair.bob.receive_parcel(&parcel, PASSWORD).unwrap().record;
    let stage = stage_received_batch(&pair, &received, false);
    remove_incoming_index_and_receipt(&pair, &outgoing.parcel_id);
    corrupt(&stage.join("batch/files/payload.bin"));
    let stage_digest = tree_digest(&stage);
    let summary = pair.bob.recover(PASSWORD).unwrap();
    assert_eq!(summary.recovered, 0);
    assert_eq!(summary.pending, 1);
    assert!(!summary.conflicts.is_empty());
    assert!(!received.received_path.as_ref().unwrap().exists());
    assert_eq!(tree_digest(&stage), stage_digest);
    assert_not_committed(
        &pair.bob,
        &outgoing.parcel_id,
        &pair.root.join("invalid-stage-return"),
    );
}

#[test]
fn incomplete_stages_remain_pending_until_explicit_discard_and_unmanaged_data_survives() {
    let pair = Pair::new(true);
    let outgoing = pair.seal(b"incomplete synthetic receive cancellation");
    let parcel = pair.export(&outgoing, "partial-stage-carrier");
    let received = pair.bob.receive_parcel(&parcel, PASSWORD).unwrap().record;
    let stage = stage_received_batch(&pair, &received, false);
    remove_incoming_index_and_receipt(&pair, &outgoing.parcel_id);
    fs::remove_file(stage.join("batch/.relaypost/prepared.json")).unwrap();
    let stage_digest = tree_digest(&stage);
    let summary = pair.bob.recover(PASSWORD).unwrap();
    assert_eq!(summary.recovered, 0);
    assert_eq!(summary.pending, 1);
    assert!(summary.conflicts.is_empty());
    assert_eq!(tree_digest(&stage), stage_digest);
    assert_not_committed(
        &pair.bob,
        &outgoing.parcel_id,
        &pair.root.join("partial-return"),
    );
    let unrelated = pair.root.join("bob/inbox/.staging/user-notes");
    fs::create_dir(&unrelated).unwrap();
    fs::write(unrelated.join("keep.txt"), b"unmanaged synthetic user data").unwrap();
    let outside = pair.root.join("outside-staging.txt");
    fs::write(&outside, b"unrelated external synthetic data").unwrap();
    assert_eq!(pair.bob.discard_incomplete_receives().unwrap(), 1);
    assert!(!stage.exists());
    assert_eq!(
        fs::read(unrelated.join("keep.txt")).unwrap(),
        b"unmanaged synthetic user data"
    );
    assert_eq!(
        fs::read(outside).unwrap(),
        b"unrelated external synthetic data"
    );
    assert_eq!(pair.bob.discard_incomplete_receives().unwrap(), 0);
    assert!(!received.received_path.as_ref().unwrap().exists());
}

#[test]
fn self_cards_and_self_send_are_rejected_without_any_delivery_records() {
    let pair = Pair::new(true);
    assert!(pair
        .alice
        .import_contact(&pair.root.join("alice.card"))
        .is_err());
    assert_eq!(pair.alice.contacts().unwrap().len(), 1);
    // Also model an old database that already contains a trusted self-contact.
    let database = rusqlite::Connection::open(pair.root.join("alice/state.sqlite3")).unwrap();
    database
        .execute(
            "INSERT INTO contacts (fingerprint, card_json, verified) VALUES (?1, ?2, 1)",
            rusqlite::params![
                pair.alice_card.fingerprint,
                serde_json::to_string(&pair.alice_card).unwrap()
            ],
        )
        .unwrap();
    drop(database);
    let source = pair.input("self-send.txt", b"SYNTHETIC self-send must fail closed");
    assert!(pair
        .alice
        .seal(&[source], &pair.alice_card.fingerprint, PASSWORD)
        .is_err());
    assert!(pair.alice.deliveries().unwrap().is_empty());
    assert!(pair.bob.deliveries().unwrap().is_empty());
    assert!(files(&pair.root.join("alice/outbox")).is_empty());
    assert!(files(&pair.root.join("alice/inbox")).is_empty());
}

#[cfg(unix)]
#[test]
fn explicit_stage_discard_rejects_linked_contents_and_preserves_external_targets() {
    use std::os::unix::fs::symlink;
    let pair = Pair::new(true);
    let staging_root = pair.root.join("bob/inbox/.staging");
    let outside = pair.root.join("outside-cancellation-scope");
    fs::create_dir(&outside).unwrap();
    let protected = outside.join("precious.txt");
    fs::write(&protected, b"SYNTHETIC external target must survive").unwrap();
    let linked_stage = staging_root.join("batch-SYNTHETIC-linked-root");
    symlink(&outside, &linked_stage).unwrap();
    assert_eq!(pair.bob.discard_incomplete_receives().unwrap(), 0);
    assert!(fs::symlink_metadata(&linked_stage)
        .unwrap()
        .file_type()
        .is_symlink());
    fs::remove_file(&linked_stage).unwrap();
    let stage = staging_root.join("batch-SYNTHETIC-linked-content");
    fs::create_dir(&stage).unwrap();
    symlink(&protected, stage.join("archive.tar")).unwrap();
    assert!(pair.bob.discard_incomplete_receives().is_err());
    assert!(fs::symlink_metadata(stage.join("archive.tar"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read(&protected).unwrap(),
        b"SYNTHETIC external target must survive"
    );
    // The refused cleanup must release the profile lock for the next operation.
    assert!(pair.bob.deliveries().unwrap().is_empty());
}

#[cfg(unix)]
fn fifo_operation_returns_promptly(home: &Path, source: &Path, fifo: &Path, receipt: bool) {
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::mpsc;
    use std::time::Duration;
    let home = home.to_path_buf();
    let source = source.to_path_buf();
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let profile = Profile::open(home).unwrap();
        let rejected = if receipt {
            profile.import_receipt(&source).is_err()
        } else {
            profile.inspect_parcel(&source).is_err()
        };
        sender.send(rejected).unwrap();
    });
    let rejected = match receiver.recv_timeout(Duration::from_secs(5)) {
        Ok(rejected) => rejected,
        Err(error) => {
            // Rescue a regressed blocking open so CI reports a failed assertion
            // instead of indefinitely retaining the profile lock or test process.
            let rescue = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(fifo)
                .unwrap();
            std::thread::sleep(Duration::from_millis(100));
            drop(rescue);
            let _ = receiver.recv_timeout(Duration::from_secs(5));
            panic!("FIFO operation waited for a writer: {error}");
        }
    };
    assert!(rejected, "special-file metadata must be refused");
    worker.join().unwrap();
}

#[cfg(unix)]
fn replace_with_fifo(path: &Path) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    fs::remove_file(path).unwrap();
    let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: c_path is a live, NUL-terminated fixture path; mode is private.
    assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
}

#[cfg(unix)]
#[test]
fn fifo_envelopes_and_receipts_return_without_writers_and_leave_the_profile_usable() {
    let pair = Pair::new(true);
    let outgoing = pair.seal(b"special-file denial-of-service synthetic fixture");
    let parcel = pair.export(&outgoing, "fifo-pristine-carrier");
    for name in ["envelope.json", "envelope.minisig"] {
        let case = pair.root.join(format!("fifo-{name}.parcel"));
        copy_tree(&parcel, &case);
        fs::remove_file(case.join("envelope-copy.json")).unwrap();
        fs::remove_file(case.join("envelope-copy.minisig")).unwrap();
        let fifo = case.join(name);
        replace_with_fifo(&fifo);
        fifo_operation_returns_promptly(&pair.root.join("bob"), &case, &fifo, false);
        assert!(pair.bob.inspect_parcel(&parcel).unwrap().recoverable);
    }
    pair.bob.receive_parcel(&parcel, PASSWORD).unwrap();
    let receipt = pair
        .bob
        .export_receipt(&outgoing.parcel_id, &pair.root.join("fifo-pristine-return"))
        .unwrap();
    for name in ["receipt.json", "receipt.minisig"] {
        let case = pair.root.join(format!("fifo-{name}.receipt"));
        copy_tree(&receipt, &case);
        let fifo = case.join(name);
        replace_with_fifo(&fifo);
        fifo_operation_returns_promptly(&pair.root.join("alice"), &case, &fifo, true);
        assert_eq!(
            outgoing_state(&pair.alice, &outgoing.parcel_id),
            DeliveryState::WaitingReceipt
        );
    }
    assert_eq!(
        pair.alice.import_receipt(&receipt).unwrap().state,
        DeliveryState::Confirmed
    );
}

#[test]
fn exports_cannot_write_into_any_managed_profile_tree() {
    let pair = Pair::new(true);
    let record = pair.seal(b"managed files must remain immutable on export");
    let parcel = pair.export(&record, "external-carrier");
    let received = pair.bob.receive_parcel(&parcel, PASSWORD).unwrap();
    let input = pair.input("return-payload.txt", b"independent reverse direction");
    let reverse = pair
        .bob
        .seal(&[input], &pair.alice_card.fingerprint, PASSWORD)
        .unwrap();
    let bob_home = pair.root.join("bob");
    for directory in [
        bob_home.clone(),
        bob_home.join("identity"),
        bob_home.join("outbox"),
    ] {
        let identity_before = tree_digest(&bob_home.join("identity"));
        let outbox_before = tree_digest(&bob_home.join("outbox"));
        let target = directory.join("forbidden-export.card");
        assert!(pair.bob.export_card(&target).is_err());
        assert!(!target.exists());
        assert_eq!(tree_digest(&bob_home.join("identity")), identity_before);
        assert_eq!(tree_digest(&bob_home.join("outbox")), outbox_before);
    }
    for carrier in [
        received.record.received_path.as_ref().unwrap().clone(),
        bob_home.join("receipts").join(&record.parcel_id),
        bob_home.join("outbox"),
    ] {
        let before = tree_digest(&carrier);
        assert!(pair
            .bob
            .export_parcel(&reverse.parcel_id, &carrier)
            .is_err());
        assert!(pair
            .bob
            .export_receipt(&record.parcel_id, &carrier)
            .is_err());
        assert_eq!(tree_digest(&carrier), before);
    }
    assert_payload(
        &pair.bob.receive_parcel(&parcel, PASSWORD).unwrap().record,
        b"managed files must remain immutable on export",
    );
    let receipt = pair
        .bob
        .export_receipt(&record.parcel_id, &pair.root.join("valid-external-return"))
        .unwrap();
    assert_eq!(
        pair.alice.import_receipt(&receipt).unwrap().state,
        DeliveryState::Confirmed
    );
}
