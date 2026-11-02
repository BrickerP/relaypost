use crate::{fsutil, Contact, Direction, ParcelRecord};
use anyhow::{ensure, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub(crate) fn open(home: &Path) -> Result<Connection> {
    let path = home.join("state.sqlite3");
    if path.try_exists()? {
        fsutil::no_symlink_components(&path)?;
    }
    let connection = Connection::open(path)?;
    connection.busy_timeout(fsutil::sqlite_timeout())?;
    connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
        CREATE TABLE IF NOT EXISTS contacts (fingerprint TEXT PRIMARY KEY, card_json TEXT NOT NULL, verified INTEGER NOT NULL CHECK(verified IN (0,1)));
        CREATE TABLE IF NOT EXISTS deliveries (parcel_id TEXT NOT NULL, direction TEXT NOT NULL, record_json TEXT NOT NULL, PRIMARY KEY(parcel_id,direction));")?;
    Ok(connection)
}

fn direction(d: &Direction) -> &'static str {
    match d {
        Direction::Outgoing => "Outgoing",
        Direction::Incoming => "Incoming",
    }
}

pub(crate) fn save(connection: &Connection, record: &ParcelRecord) -> Result<()> {
    connection.execute("INSERT INTO deliveries(parcel_id,direction,record_json) VALUES (?1,?2,?3) ON CONFLICT(parcel_id,direction) DO UPDATE SET record_json=excluded.record_json", params![record.parcel_id, direction(&record.direction), serde_json::to_string(record)?])?;
    Ok(())
}

pub(crate) fn get(connection: &Connection, id: &str, d: Direction) -> Result<Option<ParcelRecord>> {
    let json: Option<String> = connection
        .query_row(
            "SELECT record_json FROM deliveries WHERE parcel_id=?1 AND direction=?2",
            params![id, direction(&d)],
            |row| row.get(0),
        )
        .optional()?;
    json.map(|s| fsutil::json(s.as_bytes())).transpose()
}

pub(crate) fn deliveries(connection: &Connection) -> Result<Vec<ParcelRecord>> {
    let mut statement =
        connection.prepare("SELECT record_json FROM deliveries ORDER BY rowid DESC")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    rows.map(|s| fsutil::json(s?.as_bytes())).collect()
}

pub(crate) fn contacts(connection: &Connection) -> Result<Vec<Contact>> {
    let mut statement =
        connection.prepare("SELECT card_json,verified FROM contacts ORDER BY fingerprint")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
    })?;
    rows.map(|r| {
        let (raw, verified) = r?;
        Ok(Contact {
            card: fsutil::json(raw.as_bytes())?,
            verified,
        })
    })
    .collect()
}

pub(crate) fn import_contact(connection: &Connection, card: &crate::PeerCard) -> Result<Contact> {
    crate::identity::verify_card(card)?;
    let existing = contacts(connection)?
        .into_iter()
        .find(|c| c.card.fingerprint == card.fingerprint);
    let verified = if let Some(existing) = existing {
        ensure!(
            existing.card.age_recipient == card.age_recipient
                && existing.card.signing_public_key == card.signing_public_key,
            "pinned identity cannot be replaced"
        );
        existing.verified
    } else {
        false
    };
    connection.execute("INSERT INTO contacts(fingerprint,card_json,verified) VALUES (?1,?2,?3) ON CONFLICT(fingerprint) DO UPDATE SET card_json=excluded.card_json", params![card.fingerprint, serde_json::to_string(card)?, verified])?;
    Ok(Contact {
        card: card.clone(),
        verified,
    })
}

pub(crate) fn trust(connection: &Connection, fingerprint: &str) -> Result<()> {
    fsutil::check_id(fingerprint)?;
    let contact = contacts(connection)?
        .into_iter()
        .find(|c| c.card.fingerprint == fingerprint)
        .ok_or_else(|| anyhow::anyhow!("contact not found; provide its complete fingerprint"))?;
    crate::identity::verify_card(&contact.card)?;
    ensure!(
        connection.execute(
            "UPDATE contacts SET verified=1 WHERE fingerprint=?1",
            [fingerprint]
        )? == 1,
        "contact not found"
    );
    Ok(())
}

pub(crate) fn resolve(connection: &Connection, fingerprint: &str) -> Result<crate::PeerCard> {
    fsutil::check_id(fingerprint)?;
    let selected = contacts(connection)?
        .into_iter()
        .find(|contact| contact.card.fingerprint == fingerprint)
        .ok_or_else(|| anyhow::anyhow!("recipient contact not found"))?;
    ensure!(
        selected.verified,
        "contact fingerprint must be explicitly trusted first"
    );
    crate::identity::verify_card(&selected.card)?;
    Ok(selected.card)
}

pub(crate) fn pinned(connection: &Connection, card: &crate::PeerCard) -> Result<()> {
    crate::identity::verify_card(card)?;
    let trusted = resolve(connection, &card.fingerprint)?;
    ensure!(
        trusted.age_recipient == card.age_recipient
            && trusted.signing_public_key == card.signing_public_key,
        "peer keys differ from pinned identity"
    );
    Ok(())
}
