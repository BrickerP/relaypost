//! Transport-boundary acceptance tests. Execution belongs to remote CI only.
use anyhow::Result;
use relay_core::{
    DeliveryState, DirectOfferArtifact, Direction, PeerCard, Profile, MAX_DIRECT_BUNDLE,
};
use std::fs;
use std::io::Read;
use std::path::PathBuf;

const PASSWORD: &str = "SYNTHETIC-direct-test-only-not-a-real-secret";

struct Pair {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    alice: Profile,
    bob: Profile,
    alice_card: PeerCard,
    bob_card: PeerCard,
}

impl Pair {
    fn new(trust_bob: bool) -> Result<Self> {
        let temporary = tempfile::tempdir()?;
        let root = fs::canonicalize(temporary.path())?;
        let alice = Profile::open(root.join("alice"))?;
        let bob = Profile::open(root.join("bob"))?;
        let alice_card = alice.initialize("Synthetic Direct Alice", PASSWORD)?;
        let bob_card = bob.initialize("Synthetic Direct Bob", PASSWORD)?;
        assert!(!alice.import_contact_card(&bob_card)?.verified);
        alice.trust_contact(&bob_card.fingerprint)?;
        assert!(!bob.import_contact_card(&alice_card)?.verified);
        if trust_bob {
            bob.trust_contact(&alice_card.fingerprint)?;
        }
        Ok(Self {
            _temporary: temporary,
            root,
            alice,
            bob,
            alice_card,
            bob_card,
        })
    }

    fn prepare(&self) -> Result<(String, DirectOfferArtifact, Vec<u8>)> {
        let input = self.root.join("SYNTHETIC-private-filename.txt");
        fs::write(&input, b"Synthetic direct transfer content\n")?;
        let record = self
            .alice
            .seal(&[input], &self.bob_card.fingerprint, PASSWORD)?;
        let prepared = self.alice.prepare_direct_handoff(&record.parcel_id)?;
        let mut bytes = Vec::new();
        prepared.open_bundle()?.read_to_end(&mut bytes)?;
        assert_eq!(prepared.bundle_len(), bytes.len() as u64);
        assert!(!prepared
            .offer
            .envelope_json
            .contains("SYNTHETIC-private-filename"));
        Ok((record.parcel_id, prepared.offer.clone(), bytes))
    }

    fn file(&self, name: &str, bytes: &[u8]) -> Result<PathBuf> {
        let path = self.root.join(name);
        fs::write(&path, bytes)?;
        Ok(path)
    }

    fn assert_no_stage(&self) -> Result<()> {
        assert_eq!(
            fs::read_dir(self.root.join("bob/direct/incoming"))?.count(),
            0
        );
        assert!(self.bob.deliveries()?.is_empty());
        Ok(())
    }
}

fn shard_offset(bytes: &[u8]) -> usize {
    bytes
        .as_chunks::<512>()
        .0
        .iter()
        .position(|block| block.starts_with(b"groups/g000/s000.bin\0"))
        .expect("canonical shard header")
        * 512
        + 512
}

#[test]
fn direct_handoff_preserves_authorities_and_survives_restart() -> Result<()> {
    let pair = Pair::new(false)?;
    assert!(pair.alice.inspect_contact_card(&pair.alice_card).is_err());
    let mut changed_card = pair.bob_card.clone();
    changed_card.name.push_str(" changed");
    assert!(pair.alice.inspect_contact_card(&changed_card).is_err());
    let (id, offer, pristine) = pair.prepare()?;
    assert!(
        pair.bob.inspect_direct_offer(&offer).is_err(),
        "import alone cannot trust a sender"
    );
    assert!(
        pair.alice.inspect_direct_offer(&offer).is_err(),
        "own outgoing package is not incoming"
    );
    pair.bob.trust_contact(&pair.alice_card.fingerprint)?;
    let inspected = pair.bob.inspect_direct_offer(&offer)?;
    assert_eq!(inspected.sender, pair.alice_card.fingerprint);
    assert_eq!(inspected.recipient, pair.bob_card.fingerprint);
    let mut changed_offer = offer.clone();
    changed_offer.bundle_bytes = u64::MAX;
    assert!(pair.bob.inspect_direct_offer(&changed_offer).is_err());
    changed_offer = offer.clone();
    changed_offer.envelope_json.push(' ');
    assert!(pair.bob.inspect_direct_offer(&changed_offer).is_err());
    let mut damaged = pristine.clone();
    let offset = shard_offset(&damaged);
    damaged[offset] ^= 0x5a;
    let bundle = pair.file("damaged-direct.tar", &damaged)?;
    let staged = pair.bob.stage_direct_bundle(&offer, &bundle)?;
    assert!(staged.inspection.recoverable);
    assert_eq!(staged.inspection.groups[0].missing_shards, vec![0]);
    assert!(
        pair.bob.deliveries()?.is_empty(),
        "encrypted staging is not a saved delivery"
    );
    assert!(pair.bob.receipt_artifact(&id).is_err());
    assert!(!pair.bob.finalize_direct_offer(&id)?);
    pair.bob.stage_direct_bundle(&offer, &bundle)?;
    assert_eq!(pair.bob.direct_offers()?.len(), 1);
    let conflicting = pair.file("different-retry.tar", &pristine)?;
    assert!(
        pair.bob.stage_direct_bundle(&offer, &conflicting).is_err(),
        "retry cannot overwrite different staged bytes"
    );
    assert_eq!(pair.alice.deliveries()?[0].state, DeliveryState::Ready);
    let sent = pair.alice.record_direct_handoff(&id)?;
    assert_eq!(sent.state, DeliveryState::WaitingReceipt);
    let restarted_bob = Profile::open(pair.root.join("bob"))?;
    let offers = restarted_bob.direct_offers()?;
    assert_eq!(offers.len(), 1);
    assert_eq!(offers[0].offer.files, 1);
    assert_eq!(
        offers[0].offer.total_bytes,
        b"Synthetic direct transfer content\n".len() as u64
    );
    assert_eq!(offers[0].offer.sender, pair.alice_card.fingerprint);
    assert_eq!(offers[0].offer.recipient, pair.bob_card.fingerprint);
    assert_eq!(offers[0].offer.bundle_bytes, offer.bundle_bytes);
    let saved = restarted_bob.receive_parcel(&offers[0].parcel_path, PASSWORD)?;
    assert_eq!(saved.repaired_shards, 1);
    assert_eq!(saved.record.state, DeliveryState::SavedReceiptReady);
    assert_eq!(
        fs::read(
            saved
                .record
                .received_path
                .as_ref()
                .unwrap()
                .join("SYNTHETIC-private-filename.txt")
        )?,
        b"Synthetic direct transfer content\n"
    );
    let receipt = restarted_bob.receipt_artifact(&id)?;
    let mut forged = receipt.clone();
    forged.receipt_json.push(' ');
    assert!(pair.alice.import_receipt_artifact(&forged).is_err());
    assert_eq!(
        pair.alice.deliveries()?[0].state,
        DeliveryState::WaitingReceipt
    );
    let restarted_alice = Profile::open(pair.root.join("alice"))?;
    let confirmed = restarted_alice.import_receipt_artifact(&receipt)?;
    assert_eq!(confirmed.state, DeliveryState::Confirmed);
    assert_eq!(confirmed.direction, Direction::Outgoing);
    assert_eq!(
        restarted_alice.import_receipt_artifact(&receipt)?.state,
        DeliveryState::Confirmed
    );
    assert_eq!(
        restarted_alice.record_direct_handoff(&id)?.state,
        DeliveryState::Confirmed
    );
    assert!(restarted_bob.finalize_direct_offer(&id)?);
    assert!(!restarted_bob.finalize_direct_offer(&id)?);
    assert!(restarted_bob.direct_offers()?.is_empty());
    fs::write(
        saved
            .record
            .received_path
            .unwrap()
            .join("SYNTHETIC-private-filename.txt"),
        b"externally changed",
    )?;
    assert!(
        restarted_bob.receipt_artifact(&id).is_err(),
        "changed saved bytes cannot authorize another receipt export"
    );
    assert_eq!(
        restarted_alice.deliveries()?[0].state,
        DeliveryState::Confirmed
    );
    Ok(())
}

fn replace_header(bytes: &mut [u8], index: usize, path: &str, kind: u8, size: Option<u64>) {
    let offset = index * 512;
    let mut header = tar::Header::new_ustar();
    if bytes[offset..offset + 512].iter().any(|b| *b != 0) {
        header
            .as_mut_bytes()
            .copy_from_slice(&bytes[offset..offset + 512]);
    }
    header.as_mut_bytes()[..100].fill(0);
    header.as_mut_bytes()[..path.len()].copy_from_slice(path.as_bytes());
    header.as_mut_bytes()[156] = kind;
    if let Some(size) = size {
        header.set_size(size);
    }
    header.set_cksum();
    bytes[offset..offset + 512].copy_from_slice(header.as_bytes());
}

#[test]
fn hostile_direct_archives_never_publish_or_escape() -> Result<()> {
    let pair = Pair::new(true)?;
    let (_, offer, pristine) = pair.prepare()?;
    for (index, (path, kind)) in [
        ("../escape", b'0'),
        ("/absolute", b'0'),
        ("envelope.json", b'1'),
        ("envelope.json", b'2'),
        ("envelope.json", b'3'),
        ("envelope.json", b'6'),
        ("envelope.json", b'5'),
    ]
    .into_iter()
    .enumerate()
    {
        let mut hostile = pristine.clone();
        replace_header(&mut hostile, 0, path, kind, None);
        let path = pair.file(&format!("hostile-{index}.tar"), &hostile)?;
        assert!(pair.bob.stage_direct_bundle(&offer, &path).is_err());
        pair.assert_no_stage()?;
    }
    // A repeated entry cannot hide behind tar's permissive overwrite semantics.
    let mut repeated = pristine.clone();
    let second = (512 + offer.envelope_json.len().div_ceil(512) * 512) / 512;
    replace_header(&mut repeated, second, "envelope.json", b'0', None);
    assert!(pair
        .bob
        .stage_direct_bundle(&offer, &pair.file("repeated.tar", &repeated)?)
        .is_err());
    let mut oversized_entry = pristine.clone();
    replace_header(
        &mut oversized_entry,
        0,
        "envelope.json",
        b'0',
        Some(offer.envelope_json.len() as u64 + 1),
    );
    assert!(pair
        .bob
        .stage_direct_bundle(&offer, &pair.file("oversized-entry.tar", &oversized_entry)?)
        .is_err());
    let mut extra = pristine.clone();
    let trailer = extra.len() / 512 - 2;
    replace_header(&mut extra, trailer, "unexpected", b'0', Some(0));
    assert!(pair
        .bob
        .stage_direct_bundle(&offer, &pair.file("extra.tar", &extra)?)
        .is_err());
    assert!(pair
        .bob
        .stage_direct_bundle(
            &offer,
            &pair.file("truncated.tar", &pristine[..pristine.len() - 1])?
        )
        .is_err());
    let mut trailing = pristine.clone();
    trailing.push(0);
    assert!(pair
        .bob
        .stage_direct_bundle(&offer, &pair.file("trailing.tar", &trailing)?)
        .is_err());
    let huge = pair.root.join("oversized.tar");
    fs::File::create(&huge)?.set_len(MAX_DIRECT_BUNDLE + 1)?;
    assert!(pair.bob.stage_direct_bundle(&offer, &huge).is_err());
    pair.assert_no_stage()?;
    assert!(!pair.root.join("bob/direct/escape").exists());
    Ok(())
}

#[test]
fn cancelled_delivery_cannot_be_prepared_or_recorded_as_handed_off() -> Result<()> {
    let pair = Pair::new(true)?;
    let (id, _, _) = pair.prepare()?;
    pair.alice.cancel(&id)?;
    assert!(pair.alice.prepare_direct_handoff(&id).is_err());
    assert!(pair.alice.record_direct_handoff(&id).is_err());
    assert_eq!(pair.alice.deliveries()?[0].state, DeliveryState::Cancelled);
    Ok(())
}
