use crate::format::{self, Group, Shard, GROUP_BYTES, MIB};
use crate::{fsutil, GroupInspection};
use anyhow::{ensure, Context, Result};
use reed_solomon_erasure::galois_8::ReedSolomon;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::Path;

fn encode_group(bytes: &[u8], index: usize) -> Result<(Group, Vec<Vec<u8>>)> {
    ensure!(!bytes.is_empty() && bytes.len() as u64 <= GROUP_BYTES, "invalid group size");
    let n = bytes.len().div_ceil(MIB as usize);
    let r = n.div_ceil(5);
    let size = bytes.len().div_ceil(n);
    let mut shards = vec![vec![0u8; size]; n + r];
    for (i, chunk) in bytes.chunks(size).enumerate() {
        shards[i][..chunk.len()].copy_from_slice(chunk);
    }
    ReedSolomon::new(n, r)?.encode(&mut shards)?;
    let group = Group {
        index,
        bytes: bytes.len() as u64,
        data_shards: n,
        parity_shards: r,
        shard_bytes: size,
        shards: shards.iter().enumerate().map(|(j, data)| Shard {
            path: format::shard_path(index, j),
            sha256: fsutil::digest(data),
        }).collect(),
    };
    Ok((group, shards))
}

pub(crate) fn encode(input: &Path, parcel: &Path) -> Result<(String, u64, Vec<Group>)> {
    let mut reader = fsutil::open_read(input)?;
    let length = reader.metadata()?.len();
    ensure!(length > 0 && length <= format::MAX_CIPHERTEXT, "ciphertext exceeds limit");
    let mut remaining = length;
    let mut groups = Vec::new();
    let mut hash = Sha256::new();
    while remaining > 0 {
        let mut bytes = vec![0; remaining.min(GROUP_BYTES) as usize];
        reader.read_exact(&mut bytes)?;
        hash.update(&bytes);
        let (group, shards) = encode_group(&bytes, groups.len())?;
        let dir = parcel.join(format!("groups/g{:03}", group.index));
        fsutil::private_dir(&dir)?;
        for (shard, bytes) in group.shards.iter().zip(shards.iter()) {
            fsutil::write_new(&parcel.join(&shard.path), bytes)?;
        }
        groups.push(group);
        remaining -= bytes.len() as u64;
    }
    let mut extra = [0];
    ensure!(reader.read(&mut extra)? == 0, "ciphertext changed during encoding");
    Ok((hex::encode(hash.finalize()), length, groups))
}

fn load_group(parcel: &Path, group: &Group) -> Result<Vec<Option<Vec<u8>>>> {
    fsutil::directory(parcel)?;
    let mut shards = Vec::with_capacity(group.shards.len());
    for shard in &group.shards {
        let path = parcel.join(&shard.path);
        // Missing or malformed bytes are erasures. A link/special file is never
        // followed; its invalid content has no authority over reconstruction.
        let bytes = match fsutil::read_bounded(&path, group.shard_bytes as u64) {
            Ok(bytes) if bytes.len() == group.shard_bytes && fsutil::digest(&bytes) == shard.sha256 => Some(bytes),
            _ => None,
        };
        shards.push(bytes);
    }
    Ok(shards)
}

pub(crate) fn inspect(parcel: &Path, groups: &[Group]) -> Result<Vec<GroupInspection>> {
    groups.iter().map(|group| {
        let shards = load_group(parcel, group)?;
        let missing_shards: Vec<usize> = shards.iter().enumerate().filter_map(|(i, s)| s.is_none().then_some(i)).collect();
        Ok(GroupInspection {
            index: group.index,
            data_shards: group.data_shards,
            parity_shards: group.parity_shards,
            valid_shards: shards.len() - missing_shards.len(),
            recoverable: missing_shards.len() <= group.parity_shards,
            missing_shards,
        })
    }).collect()
}

fn reconstruct_group(group: &Group, shards: &mut [Option<Vec<u8>>]) -> Result<usize> {
    let missing = shards.iter().filter(|s| s.is_none()).count();
    ensure!(missing <= group.parity_shards, "group {} exceeds repair budget: {missing} missing/bad shards, budget {}", group.index, group.parity_shards);
    let codec = ReedSolomon::new(group.data_shards, group.parity_shards)?;
    codec.reconstruct(shards)?;
    for (i, shard) in shards.iter().enumerate() {
        let bytes = shard.as_ref().context("decoder failed to reconstruct a shard")?;
        ensure!(bytes.len() == group.shard_bytes && fsutil::digest(bytes) == group.shards[i].sha256, "reconstructed shard digest mismatch");
    }
    // Signed hashes make unknown corruption erasures. Verify code consistency
    // too, so an inconsistent signed shard set cannot silently pass.
    let refs: Vec<&[u8]> = shards.iter().map(|s| s.as_ref().unwrap().as_slice()).collect();
    ensure!(codec.verify(&refs)?, "authenticated shard set is not a valid codeword");
    Ok(missing)
}

pub(crate) fn reconstruct(parcel: &Path, groups: &[Group], output: &Path, expected_id: &str, expected_bytes: u64) -> Result<usize> {
    let mut file = fsutil::create_new(output)?;
    let mut total = 0;
    let mut repaired = 0;
    let mut hash = Sha256::new();
    for group in groups {
        let mut shards = load_group(parcel, group)?;
        repaired += reconstruct_group(group, &mut shards)?;
        let mut remaining = group.bytes as usize;
        for shard in shards.iter().take(group.data_shards) {
            let bytes = shard.as_ref().context("missing reconstructed data")?;
            let count = remaining.min(bytes.len());
            file.write_all(&bytes[..count])?;
            hash.update(&bytes[..count]);
            ensure!(bytes[count..].iter().all(|b| *b == 0), "nonzero authenticated padding");
            remaining -= count;
            total += count as u64;
        }
        ensure!(remaining == 0, "group output length mismatch");
    }
    ensure!(total == expected_bytes && hex::encode(hash.finalize()) == expected_id, "reconstructed ciphertext digest mismatch");
    file.sync_all()?;
    Ok(repaired)
}

pub(crate) fn copy_exact(parcel: &Path, output: &Path, groups: &[Group]) -> Result<()> {
    for group in groups {
        let dir = output.join(format!("groups/g{:03}", group.index));
        fsutil::private_dir(&dir)?;
        for shard in &group.shards {
            let bytes = fsutil::read_bounded(&parcel.join(&shard.path), group.shard_bytes as u64)?;
            ensure!(bytes.len() == group.shard_bytes && fsutil::digest(&bytes) == shard.sha256, "local immutable parcel is damaged");
            fsutil::write_new(&output.join(&shard.path), &bytes)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use proptest::prelude::*;

    fn repaired_bytes(group: &Group, shards: &mut [Option<Vec<u8>>]) -> Result<Vec<u8>> {
        reconstruct_group(group, shards)?;
        let mut bytes: Vec<u8> = shards.iter().take(group.data_shards).flat_map(|s| s.as_ref().unwrap().iter().copied()).collect();
        bytes.truncate(group.bytes as usize);
        Ok(bytes)
    }

    #[test]
    fn every_tail_budget_and_padding() -> Result<()> {
        for n in 1usize..20 {
            let bytes: Vec<u8> = (0..((n - 1) * MIB as usize + 17)).map(|i| (i % 251) as u8).collect();
            let (group, shards) = encode_group(&bytes, 0)?;
            assert_eq!(group.data_shards, n);
            assert_eq!(group.parity_shards, n.div_ceil(5));
            let mut damaged: Vec<_> = shards.iter().cloned().map(Some).collect();
            // Mix a missing data shard with parity erasures.
            damaged[0] = None;
            for j in 1..group.parity_shards {
                damaged[n + j - 1] = None;
            }
            assert_eq!(repaired_bytes(&group, &mut damaged)?, bytes);
            let mut over: Vec<_> = shards.into_iter().map(Some).collect();
            for shard in over.iter_mut().take(group.parity_shards + 1) {
                *shard = None;
            }
            assert!(reconstruct_group(&group, &mut over).is_err());
        }
        Ok(())
    }

    #[test]
    fn group_boundaries_roundtrip() -> Result<()> {
        for length in [GROUP_BYTES - 1, GROUP_BYTES, GROUP_BYTES + 1] {
            let root = tempfile::tempdir()?;
            let canonical = fs::canonicalize(root.path())?;
            let input = canonical.join("ciphertext.synthetic");
            let bytes: Vec<u8> = (0..length).map(|i| (i % 241) as u8).collect();
            fsutil::write_new(&input, &bytes)?;
            let parcel = canonical.join("parcel");
            fsutil::private_dir(&parcel)?;
            let (id, len, groups) = encode(&input, &parcel)?;
            assert_eq!(groups.len(), length.div_ceil(GROUP_BYTES) as usize);
            let first = &groups[0];
            for shard in first.shards.iter().take(4) {
                fs::remove_file(parcel.join(&shard.path))?;
            }
            let output = canonical.join("restored.synthetic");
            assert_eq!(reconstruct(&parcel, &groups, &output, &id, len)?, 4);
            assert_eq!(fs::read(output)?, bytes);
        }
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]
        #[test]
        fn small_tail_repairs_any_one_erasure(bytes in prop::collection::vec(any::<u8>(), 1..65536), missing in 0usize..2) {
            let (group, encoded) = encode_group(&bytes, 0).unwrap();
            let mut shards: Vec<_> = encoded.into_iter().map(Some).collect();
            shards[missing] = None;
            prop_assert_eq!(repaired_bytes(&group, &mut shards).unwrap(), bytes);
        }
    }
}
