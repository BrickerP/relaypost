use crate::format::{Manifest, ManifestFile, MAX_FILES, MAX_MANIFEST, MAX_RAW};
use crate::fsutil;
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File, Metadata};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

pub(crate) const MAX_ARCHIVE: u64 = MAX_RAW + MAX_MANIFEST + 1024 * 1024;
const MANIFEST_ENTRY: &str = "manifest.json";

pub(crate) fn validate_path(path: &str) -> Result<String> {
    ensure!(!path.is_empty() && path.len() <= 1024 && !path.contains('\\'), "unsafe or overlong relative path");
    ensure!(path.nfc().collect::<String>() == path, "path must use NFC Unicode normalization");
    let components: Vec<_> = path.split('/').collect();
    ensure!(components.len() <= 16, "path exceeds depth limit");
    for component in components {
        ensure!(!component.is_empty() && component != "." && component != "..", "unsafe path component");
        ensure!(!component.starts_with('.') && !component.ends_with('.') && !component.ends_with(' '), "hidden or platform-ambiguous path component");
        ensure!(!component.chars().any(|c| c.is_control() || matches!(c, ':' | '<' | '>' | '"' | '|' | '?' | '*')), "unsafe path characters");
        let folded = component.nfkc().flat_map(char::to_lowercase).collect::<String>();
        let base = folded.split('.').next().unwrap_or("");
        let reserved = ["con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9"];
        ensure!(!reserved.contains(&base), "reserved platform filename");
    }
    // NFKC + case folding is conservative across common destination volumes.
    // The two full folds below are not supplied by char::to_lowercase.
    Ok(path.nfkc().flat_map(char::to_lowercase).collect::<String>().replace('ß', "ss").replace('ς', "σ"))
}

pub(crate) fn validate_manifest(manifest: &Manifest) -> Result<()> {
    ensure!(manifest.kind == "relaypost.files" && manifest.schema == 1, "unsupported internal manifest");
    ensure!(!manifest.files.is_empty() && manifest.files.len() <= MAX_FILES && manifest.total_bytes <= MAX_RAW, "manifest exceeds content limits");
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    for file in &manifest.files {
        let folded = validate_path(&file.path)?;
        ensure!(names.insert(folded), "colliding or duplicate file path");
        fsutil::check_id(&file.sha256)?;
        total = total.checked_add(file.bytes).context("manifest size overflow")?;
        ensure!(total <= MAX_RAW, "manifest raw size limit exceeded");
    }
    for name in &names {
        let mut prefix = String::new();
        let components: Vec<_> = name.split('/').collect();
        for part in components.iter().take(components.len().saturating_sub(1)) {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            ensure!(!names.contains(&prefix), "file/directory path collision");
        }
    }
    ensure!(total == manifest.total_bytes, "manifest total does not match files");
    Ok(())
}

fn unchanged(before: &Metadata, after: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        before.dev() == after.dev() && before.ino() == after.ino() && before.len() == after.len()
            && before.mtime() == after.mtime() && before.mtime_nsec() == after.mtime_nsec()
            && before.ctime() == after.ctime() && before.ctime_nsec() == after.ctime_nsec()
    }
    #[cfg(not(unix))]
    { before.len() == after.len() && before.modified().ok() == after.modified().ok() }
}

pub(crate) fn hash_file(path: &Path, expected_length: u64) -> Result<String> {
    let mut file = fsutil::open_read(path)?;
    let before = file.metadata()?;
    ensure!(before.len() == expected_length, "file length changed: {}", path.display());
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 { break; }
        total += n as u64;
        ensure!(total <= expected_length, "file changed while hashing");
        hash.update(&buffer[..n]);
    }
    ensure!(total == expected_length && unchanged(&before, &file.metadata()?), "file changed while hashing");
    Ok(hex::encode(hash.finalize()))
}

/// Snapshot every input before archive/encryption. The MVP accepts static
/// regular files; selecting a directory or a link requires explicit expansion
/// by the user, and never silently traverses another filesystem subtree.
pub(crate) fn snapshot(inputs: &[PathBuf], root: &Path) -> Result<(Manifest, Vec<PathBuf>)> {
    ensure!(!inputs.is_empty() && inputs.len() <= MAX_FILES, "select 1–100 regular files");
    let mut manifest = Manifest { kind: "relaypost.files".into(), schema: 1, files: Vec::new(), total_bytes: 0 };
    let mut snapshots = Vec::new();
    let mut names = BTreeSet::new();
    for (i, input) in inputs.iter().enumerate() {
        let label = input.file_name().and_then(|s| s.to_str()).context("input filename is not UTF-8")?.nfc().collect::<String>();
        ensure!(names.insert(validate_path(&label)?), "input filenames collide");
        let metadata = fs::symlink_metadata(input)?;
        ensure!(metadata.is_file() && !metadata.file_type().is_symlink(), "input must be a static regular file");
        manifest.total_bytes = manifest.total_bytes.checked_add(metadata.len()).context("raw size overflow")?;
        ensure!(manifest.total_bytes <= MAX_RAW, "batch exceeds 64 MiB raw data limit");
        let mut source = fsutil::open_read(input)?;
        let before = source.metadata()?;
        ensure!(unchanged(&metadata, &before), "input changed before snapshot");
        let output = root.join(format!("snapshot-{i:03}"));
        let mut destination = fsutil::create_new(&output)?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        let mut copied = 0;
        loop {
            let n = source.read(&mut buffer)?;
            if n == 0 { break; }
            copied += n as u64;
            ensure!(copied <= before.len(), "input grew while taking snapshot");
            hash.update(&buffer[..n]);
            destination.write_all(&buffer[..n])?;
        }
        ensure!(copied == before.len() && unchanged(&before, &source.metadata()?) && unchanged(&before, &fs::symlink_metadata(input)?), "input changed while taking snapshot");
        destination.sync_all()?;
        manifest.files.push(ManifestFile { path: label, bytes: copied, sha256: hex::encode(hash.finalize()) });
        snapshots.push(output);
    }
    validate_manifest(&manifest)?;
    Ok((manifest, snapshots))
}

fn header(path: &str, bytes: u64) -> Result<tar::Header> {
    let mut h = tar::Header::new_ustar();
    h.set_path(path)?;
    h.set_entry_type(tar::EntryType::Regular);
    h.set_mode(0o600);
    h.set_uid(0);
    h.set_gid(0);
    h.set_mtime(0);
    h.set_size(bytes);
    h.set_cksum();
    Ok(h)
}

pub(crate) fn pack(manifest: &[u8], snapshots: &[PathBuf], output: &Path) -> Result<()> {
    ensure!(manifest.len() as u64 <= MAX_MANIFEST, "manifest exceeds 256 KiB");
    let mut archive = tar::Builder::new(fsutil::create_new(output)?);
    archive.append(&header(MANIFEST_ENTRY, manifest.len() as u64)?, manifest)?;
    for (i, path) in snapshots.iter().enumerate() {
        let file = fsutil::open_read(path)?;
        archive.append(&header(&format!("data/{i:03}"), file.metadata()?.len())?, file)?;
    }
    archive.finish()?;
    archive.into_inner()?.sync_all()?;
    Ok(())
}

// Only a deliberately small standard USTAR subset is accepted. Numbered tar
// entries separate archive metadata from user paths: the manifest alone maps
// content order to validated destination names. No GNU/PAX extensions, links,
// sparse files or implicit archive paths are interpreted.
fn read_header(reader: &mut File, expected: &str, maximum: u64) -> Result<u64> {
    let mut block = [0u8; 512];
    reader.read_exact(&mut block)?;
    let h = tar::Header::from_byte_slice(&block);
    let sum: u32 = block.iter().enumerate().map(|(i, b)| if (148..156).contains(&i) { 32 } else { *b as u32 }).sum();
    ensure!(h.cksum()? == sum, "archive header checksum mismatch");
    ensure!(h.entry_type() == tar::EntryType::Regular && h.path_bytes().as_ref() == expected.as_bytes(), "unexpected, unsafe or special archive entry");
    ensure!(&block[257..263] == b"ustar\0" && &block[263..265] == b"00", "archive must use USTAR v1 subset");
    ensure!(block[157..257].iter().all(|b| *b == 0) && block[345..500].iter().all(|b| *b == 0), "archive links and path prefixes are forbidden");
    let size = h.size()?;
    ensure!(size <= maximum, "archive entry exceeds authenticated resource bound");
    Ok(size)
}

fn padding(reader: &mut File, bytes: u64) -> Result<()> {
    let pad = (512 - bytes % 512) % 512;
    let mut buffer = [0u8; 512];
    reader.read_exact(&mut buffer[..pad as usize])?;
    ensure!(buffer[..pad as usize].iter().all(|b| *b == 0), "nonzero archive padding");
    Ok(())
}

pub(crate) fn unpack(input: &Path, files_root: &Path, expected_manifest: &str, expected_files: usize, expected_total: u64) -> Result<(Manifest, Vec<u8>)> {
    let mut file = fsutil::open_read(input)?;
    ensure!(file.metadata()?.len() <= MAX_ARCHIVE, "archive exceeds size bound");
    let manifest_size = read_header(&mut file, MANIFEST_ENTRY, MAX_MANIFEST)?;
    let mut raw = vec![0; manifest_size as usize];
    file.read_exact(&mut raw)?;
    padding(&mut file, manifest_size)?;
    ensure!(fsutil::digest(&raw) == expected_manifest, "internal manifest is not bound to signed envelope");
    let manifest: Manifest = fsutil::json(&raw)?;
    validate_manifest(&manifest)?;
    ensure!(manifest.files.len() == expected_files && manifest.total_bytes == expected_total, "manifest conflicts with signed resource bounds");
    fsutil::private_dir(files_root)?;
    for (i, entry) in manifest.files.iter().enumerate() {
        let size = read_header(&mut file, &format!("data/{i:03}"), entry.bytes)?;
        ensure!(size == entry.bytes, "archive file length disagrees with manifest");
        let target = files_root.join(&entry.path);
        fsutil::private_dir(target.parent().context("file needs parent")?)?;
        let mut output = fsutil::create_new(&target)?;
        let mut hash = Sha256::new();
        let mut remaining = size;
        let mut buffer = [0u8; 64 * 1024];
        while remaining > 0 {
            let count = remaining.min(buffer.len() as u64) as usize;
            file.read_exact(&mut buffer[..count])?;
            hash.update(&buffer[..count]);
            output.write_all(&buffer[..count])?;
            remaining -= count as u64;
        }
        ensure!(hex::encode(hash.finalize()) == entry.sha256, "decrypted file digest mismatch");
        output.sync_all()?;
        padding(&mut file, size)?;
    }
    let mut trailer = [0u8; 1024];
    file.read_exact(&mut trailer)?;
    ensure!(trailer.iter().all(|b| *b == 0), "unexpected undeclared archive entries");
    let mut extra = [0u8];
    ensure!(file.read(&mut extra)? == 0, "trailing archive data refused");
    Ok((manifest, raw))
}

pub(crate) fn verify_files(root: &Path, manifest: &Manifest) -> Result<()> {
    validate_manifest(manifest)?;
    fsutil::directory(root)?;
    let mut expected_files = BTreeSet::new();
    let mut expected_dirs = BTreeSet::new();
    for file in &manifest.files {
        ensure!(hash_file(&root.join(&file.path), file.bytes)? == file.sha256, "published content was externally modified: {}", file.path);
        expected_files.insert(PathBuf::from(&file.path));
        let mut parent = Path::new(&file.path).parent();
        while let Some(p) = parent {
            if !p.as_os_str().is_empty() { expected_dirs.insert(p.to_path_buf()); }
            parent = p.parent();
        }
    }
    fn walk(root: &Path, here: &Path, files: &BTreeSet<PathBuf>, dirs: &BTreeSet<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(here)? {
            let entry = entry?;
            let path = entry.path();
            let relative = path.strip_prefix(root)?;
            let meta = fs::symlink_metadata(&path)?;
            ensure!(!meta.file_type().is_symlink(), "symlink in published content");
            if meta.is_dir() {
                ensure!(dirs.contains(relative), "undeclared directory in published batch");
                walk(root, &path, files, dirs)?;
            } else {
                ensure!(meta.is_file() && files.contains(relative), "undeclared or special file in published batch");
            }
        }
        Ok(())
    }
    walk(root, root, &expected_files, &expected_dirs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_paths_and_collisions_are_refused() {
        for bad in ["../escape", "/root", "a/../b", "a\\b", "a//b", "a/./b", ".relaypost/key", "CON", "a:", "a ", "e\u{301}.txt"] {
            assert!(validate_path(bad).is_err(), "accepted {bad}");
        }
        let mut manifest = Manifest { kind: "relaypost.files".into(), schema: 1, total_bytes: 0, files: vec![ManifestFile { path: "A.txt".into(), bytes: 0, sha256: fsutil::digest(b"") }, ManifestFile { path: "a.txt".into(), bytes: 0, sha256: fsutil::digest(b"") }] };
        assert!(validate_manifest(&manifest).is_err());
        manifest.files[0].path = "a".into();
        manifest.files[1].path = "a/file".into();
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn archive_refuses_symlink_and_traversal_entries() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = fs::canonicalize(temp.path())?;
        for (name, entry_type) in [("../escape", tar::EntryType::Regular), ("data/000", tar::EntryType::Symlink)] {
            let path = root.join(format!("bad-{}.tar", entry_type.as_byte()));
            let mut builder = tar::Builder::new(fsutil::create_new(&path)?);
            let manifest = Manifest { kind: "relaypost.files".into(), schema: 1, total_bytes: 0, files: vec![ManifestFile { path: "safe.txt".into(), bytes: 0, sha256: fsutil::digest(b"") }] };
            let raw = serde_json::to_vec(&manifest)?;
            builder.append(&header(MANIFEST_ENTRY, raw.len() as u64)?, raw.as_slice())?;
            let mut h = header("data/000", 0)?;
            // Build hostile raw bytes without asking tar's path sanitizer.
            h.as_mut_bytes()[..100].fill(0);
            h.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
            h.set_entry_type(entry_type);
            h.set_cksum();
            builder.append(&h, std::io::empty())?;
            builder.finish()?;
            drop(builder);
            assert!(unpack(&path, &root.join(format!("files-{}", entry_type.as_byte())), &fsutil::digest(&raw), 1, 0).is_err());
        }
        Ok(())
    }
}
