use crate::{fsutil, PeerCard};
use age::secrecy::{ExposeSecret, SecretString};
use anyhow::{ensure, Context, Result};
use minisign::{KeyPair, PublicKey, SecretKey, SignatureBox};
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read, Write};
use std::path::Path;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const IDENTITY_LIMIT: u64 = 32 * 1024;

// Secret DTOs are deliberately private and never implement Debug.
#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
struct SecretBundle {
    age_identity: String,
    signing_secret_key: String,
}

pub(crate) struct Unlocked {
    pub age: age::x25519::Identity,
    pub signing: SecretKey,
}

#[derive(Serialize)]
struct CardBinding<'a> {
    kind: &'static str,
    version: u32,
    name: &'a str,
    fingerprint: &'a str,
    age_recipient: &'a str,
    signing_public_key: &'a str,
}

fn binding(card: &PeerCard) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&CardBinding {
        kind: "relaypost.peer",
        version: card.version,
        name: &card.name,
        fingerprint: &card.fingerprint,
        age_recipient: &card.age_recipient,
        signing_public_key: &card.signing_public_key,
    })?)
}

fn fingerprint(age: &str, signing: &PublicKey) -> String {
    // Fixed domain + length-delimited canonical keys. Display names are labels.
    let mut bytes = b"RelayPost peer fingerprint v1\0".to_vec();
    bytes.extend_from_slice(&(age.len() as u32).to_be_bytes());
    bytes.extend_from_slice(age.as_bytes());
    let signing = signing.to_bytes();
    bytes.extend_from_slice(&(signing.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&signing);
    fsutil::digest(&bytes)
}

pub(crate) fn verify_card(card: &PeerCard) -> Result<()> {
    ensure!(card.version == 1, "unsupported peer-card version");
    ensure!(
        !card.name.trim().is_empty()
            && card.name.len() <= 128
            && !card.name.chars().any(char::is_control),
        "invalid peer name"
    );
    ensure!(
        card.age_recipient.len() <= 128
            && card.signing_public_key.len() <= 128
            && card.signature.len() <= 2048,
        "peer card exceeds limits"
    );
    let recipient: age::x25519::Recipient = card
        .age_recipient
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid age recipient"))?;
    ensure!(
        recipient.to_string() == card.age_recipient,
        "noncanonical age recipient"
    );
    let pk = PublicKey::from_base64(&card.signing_public_key)?;
    ensure!(
        pk.to_base64() == card.signing_public_key,
        "noncanonical Minisign public key"
    );
    ensure!(
        fingerprint(&card.age_recipient, &pk) == card.fingerprint,
        "peer fingerprint mismatch"
    );
    verify(&card.signing_public_key, &binding(card)?, &card.signature)
}

pub(crate) fn sign(sk: &SecretKey, bytes: &[u8]) -> Result<String> {
    Ok(minisign::sign(
        None,
        sk,
        Cursor::new(bytes),
        Some("RelayPost v1"),
        Some("RelayPost authenticated record"),
    )?
    .into_string())
}

pub(crate) fn verify(public: &str, bytes: &[u8], signature: &str) -> Result<()> {
    ensure!(signature.len() <= 2048, "signature exceeds limit");
    let pk = PublicKey::from_base64(public)?;
    let sig = SignatureBox::from_string(signature)?;
    minisign::verify(&pk, &sig, Cursor::new(bytes), true, false, false)
        .context("Minisign authentication failed")?;
    Ok(())
}

pub(crate) fn initialize(identity_dir: &Path, name: &str, password: &str) -> Result<PeerCard> {
    ensure!(!password.is_empty(), "identity password cannot be empty");
    fsutil::assert_absent(identity_dir)?;
    let parent = identity_dir.parent().context("identity needs parent")?;
    let staging = tempfile::Builder::new()
        .prefix(".identity-")
        .tempdir_in(parent)?;
    let age_key = age::x25519::Identity::generate();
    let pair = KeyPair::generate_unencrypted_keypair()?;
    let mut card = PeerCard {
        version: 1,
        name: name.to_owned(),
        fingerprint: fingerprint(&age_key.to_public().to_string(), &pair.pk),
        age_recipient: age_key.to_public().to_string(),
        signing_public_key: pair.pk.to_base64(),
        signature: String::new(),
    };
    card.signature = sign(&pair.sk, &binding(&card)?)?;
    verify_card(&card)?;
    let secret_bytes = Zeroizing::new(pair.sk.to_bytes());
    let bundle = SecretBundle {
        age_identity: age_key.to_string().expose_secret().to_owned(),
        signing_secret_key: hex::encode(&*secret_bytes),
    };
    let plaintext = Zeroizing::new(serde_json::to_vec(&bundle)?);
    let recipient = age::scrypt::Recipient::new(SecretString::from(password.to_owned()));
    let encrypted = age::encrypt(&recipient, &plaintext)?;
    fsutil::write_new(&staging.path().join("secrets.age"), &encrypted)?;
    fsutil::write_new(
        &staging.path().join("card.json"),
        &serde_json::to_vec_pretty(&card)?,
    )?;
    fsutil::sync_tree(staging.path())?;
    fsutil::publish_noreplace(staging.path(), identity_dir)?;
    Ok(card)
}

pub(crate) fn public_card(identity_dir: &Path) -> Result<PeerCard> {
    fsutil::directory(identity_dir)?;
    let card: PeerCard = fsutil::json(&fsutil::read_bounded(
        &identity_dir.join("card.json"),
        IDENTITY_LIMIT,
    )?)?;
    verify_card(&card)?;
    Ok(card)
}

pub(crate) fn unlock(identity_dir: &Path, password: &str) -> Result<Unlocked> {
    let card = public_card(identity_dir)?;
    let encrypted = fsutil::read_bounded(&identity_dir.join("secrets.age"), IDENTITY_LIMIT)?;
    let mut scrypt = age::scrypt::Identity::new(SecretString::from(password.to_owned()));
    scrypt.set_max_work_factor(20);
    let decryptor = age::Decryptor::new(&encrypted[..]).context("invalid encrypted identity")?;
    let reader = decryptor
        .decrypt(std::iter::once(&scrypt as &dyn age::Identity))
        .context("cannot unlock identity (wrong password or damaged identity)")?;
    let mut plaintext = Zeroizing::new(Vec::new());
    reader
        .take(IDENTITY_LIMIT + 1)
        .read_to_end(&mut plaintext)?;
    ensure!(
        plaintext.len() as u64 <= IDENTITY_LIMIT,
        "identity plaintext exceeds limits"
    );
    let bundle: SecretBundle = fsutil::json(&plaintext)?;
    let age: age::x25519::Identity = bundle
        .age_identity
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid encrypted age identity"))?;
    let signing_bytes = Zeroizing::new(hex::decode(&bundle.signing_secret_key)?);
    let signing = SecretKey::from_bytes(&signing_bytes)?;
    ensure!(
        !signing.is_encrypted(),
        "unexpected nested secret-key encryption"
    );
    ensure!(
        age.to_public().to_string() == card.age_recipient
            && PublicKey::from_secret_key(&signing)?.to_base64() == card.signing_public_key,
        "private identity does not match public card"
    );
    Ok(Unlocked { age, signing })
}

pub(crate) fn encrypt_file(input: &Path, output: &Path, recipient: &str) -> Result<()> {
    let recipient: age::x25519::Recipient = recipient
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid age recipient"))?;
    let encryptor =
        age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))?;
    let file = fsutil::create_new(output)?;
    let mut writer = encryptor.wrap_output(file)?;
    std::io::copy(&mut fsutil::open_read(input)?, &mut writer)?;
    let file = writer.finish()?;
    file.sync_all()?;
    Ok(())
}

pub(crate) fn decrypt_file(
    input: &Path,
    output: &Path,
    key: &age::x25519::Identity,
    max_plaintext: u64,
) -> Result<()> {
    let decryptor = age::Decryptor::new(fsutil::open_read(input)?)?;
    let reader = decryptor.decrypt(std::iter::once(key as &dyn age::Identity))?;
    let mut file = fsutil::create_new(output)?;
    let copied = std::io::copy(&mut reader.take(max_plaintext + 1), &mut file)?;
    ensure!(
        copied <= max_plaintext,
        "decrypted archive exceeds resource limit"
    );
    file.flush()?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_identity_binding_and_password() -> Result<()> {
        let home = tempfile::tempdir()?;
        let id = std::fs::canonicalize(home.path())?.join("identity");
        let card = initialize(&id, "Synthetic Alice", "synthetic-fixture-password")?;
        verify_card(&card)?;
        let unlocked = unlock(&id, "synthetic-fixture-password")?;
        let bytes = b"synthetic signed record";
        let sig = sign(&unlocked.signing, bytes)?;
        verify(&card.signing_public_key, bytes, &sig)?;
        assert!(verify(&card.signing_public_key, b"changed", &sig).is_err());
        assert!(unlock(&id, "wrong synthetic password").is_err());
        let mut tampered = card;
        tampered.name = "Different label".into();
        assert!(verify_card(&tampered).is_err());
        Ok(())
    }
}
