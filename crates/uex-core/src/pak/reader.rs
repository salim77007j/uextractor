//! PAK reader: footer detection, index parsing (legacy + path-hash/encoded), entry data reading.

use super::{Version, VersionMajor, MAGIC};
use crate::compress::Method;
use crate::crypto::AesKey;
use crate::error::{Error, Result};
use crate::io::{align, UexRead};
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, Default)]
pub struct PakEntry {
    pub offset: u64,
    pub compressed: u64,
    pub uncompressed: u64,
    pub compression_slot: Option<u32>,
    pub timestamp: Option<u64>,
    pub hash: Option<[u8; 20]>,
    pub blocks: Option<Vec<Block>>,
    pub flags: u8,
    pub compression_block_size: u32,
}

impl PakEntry {
    pub fn is_encrypted(&self) -> bool {
        self.flags & 1 != 0
    }
    pub fn is_deleted(&self) -> bool {
        (self.flags >> 1) & 1 != 0
    }
    pub fn serialized_size(version: Version, compression: Option<u32>, block_count: u32) -> u64 {
        let mut size = 8 + 8 + 8; // offset, compressed, uncompressed
        size += if version == Version::V8A { 1 } else { 4 }; // compression index width
        size += if version.major() == VersionMajor::Initial { 8 } else { 0 };
        size += 20; // hash
        if compression.is_some() {
            size += 4 + (8 + 8) * block_count as u64; // block count + blocks
        }
        size += 1; // flags
        if version.major() as u32 >= VersionMajor::CompressionEncryption as u32 {
            size += 4; // compression block size
        }
        size
    }

    pub fn read(r: &mut impl Read, version: Version) -> Result<PakEntry> {
        let major = version.major();
        let offset = r.u64()?;
        let compressed = r.u64()?;
        let uncompressed = r.u64()?;
        let compression_slot = match if version == Version::V8A { r.u8()? as u32 } else { r.u32()? } {
            0 => None,
            n => Some(n - 1),
        };
        let timestamp = (major == VersionMajor::Initial).then(|| r.u64()).transpose()?;
        let mut hash = [0u8; 20];
        r.read_exact(&mut hash)?;
        let blocks = if major as u32 >= VersionMajor::CompressionEncryption as u32 && compression_slot.is_some() {
            let count = r.u32()? as usize;
            if count > 0x10000 {
                return Err(Error::malformed("too many compression blocks"));
            }
            let mut blocks = Vec::with_capacity(count);
            for _ in 0..count {
                blocks.push(Block {
                    start: r.u64()?,
                    end: r.u64()?,
                });
            }
            Some(blocks)
        } else {
            None
        };
        let flags = if major as u32 >= VersionMajor::CompressionEncryption as u32 { r.u8()? } else { 0 };
        let compression_block_size =
            if major as u32 >= VersionMajor::CompressionEncryption as u32 { r.u32()? } else { 0 };
        Ok(PakEntry {
            offset,
            compressed,
            uncompressed,
            compression_slot,
            timestamp,
            hash: Some(hash),
            blocks,
            flags,
            compression_block_size,
        })
    }

    /// UE5 path-hash-index encoded entry (used for FDI-indexed records).
    pub fn read_encoded(r: &mut impl Read, version: Version) -> Result<PakEntry> {
        let bits = r.u32()?;
        let compression_slot = match (bits >> 23) & 0x3f {
            0 => None,
            n => Some(n - 1),
        };
        let encrypted = (bits & (1 << 22)) != 0;
        let block_count: u32 = (bits >> 6) & 0xffff;
        let mut block_size = bits & 0x3f;
        if block_size == 0x3f {
            block_size = r.u32()?;
        } else {
            block_size <<= 11;
        }

        let var_flag = |bit: u32| (bits & (1 << bit)) != 0;
        let offset = if var_flag(31) { r.u32()? as u64 } else { r.u64()? };
        let uncompressed = if var_flag(30) { r.u32()? as u64 } else { r.u64()? };
        let compressed = if compression_slot.is_some() {
            if var_flag(29) { r.u32()? as u64 } else { r.u64()? }
        } else {
            uncompressed
        };

        let entry_size = Self::serialized_size(version, compression_slot, block_count);
        let blocks = if block_count == 1 && !encrypted {
            Some(vec![Block {
                start: entry_size,
                end: entry_size + compressed,
            }])
        } else if block_count > 0 {
            let mut index = entry_size;
            let mut blocks = Vec::with_capacity(block_count as usize);
            for _ in 0..block_count {
                let mut size = r.u32()? as u64;
                blocks.push(Block { start: index, end: index + size });
                if encrypted {
                    size = align(size, 16);
                }
                index += size;
            }
            Some(blocks)
        } else {
            None
        };

        Ok(PakEntry {
            offset,
            compressed,
            uncompressed,
            compression_slot,
            timestamp: None,
            hash: None,
            blocks,
            flags: encrypted as u8,
            compression_block_size: block_size,
        })
    }
}

#[derive(Debug)]
pub struct Footer {
    pub encrypted: bool,
    pub encryption_guid: Option<[u8; 16]>,
    pub version: Version,
    pub index_offset: u64,
    pub index_size: u64,
    pub hash: [u8; 20],
    pub frozen: bool,
    pub compression: Vec<Option<Method>>,
}

impl Footer {
    fn read(r: &mut impl Read, version: Version) -> Result<Footer> {
        let encryption_guid = if version.major() as u32 >= VersionMajor::EncryptionKeyGuid as u32 {
            Some(r.fguid()?)
        } else {
            None
        };
        let encrypted = if version.major() as u32 >= VersionMajor::IndexEncryption as u32 {
            r.u8()? == 1
        } else {
            false
        };
        let magic = r.u32()?;
        if magic != MAGIC {
            return Err(Error::Magic { found: magic });
        }
        let version_major_raw = r.u32()?;
        if version_major_raw != version.as_u32() {
            return Err(Error::Malformed(format!(
                "footer version mismatch: index version {version} vs footer major {version_major_raw}"
            )));
        }
        let index_offset = r.u64()?;
        let index_size = r.u64()?;
        let hash = r.read_bytes(20)?.try_into().unwrap();
        let frozen = if version.major() == VersionMajor::FrozenIndex { r.u8()? == 1 } else { false };

        let slot_count = if (version as u32) < Version::V8A as u32 {
            0
        } else if (version as u32) < Version::V8B as u32 {
            4
        } else {
            5
        };
        let mut compression = Vec::with_capacity(slot_count);
        for _ in 0..slot_count {
            let name = r.read_bytes(32)?;
            let s: String = name.iter().filter(|&&b| b != 0).map(|&b| b as char).collect();
            compression.push(Method::from_name(&s));
        }
        if (version.major() as u32) < VersionMajor::FNameBasedCompression as u32 {
            compression.push(Some(Method::Zlib));
            compression.push(Some(Method::Gzip));
            compression.push(Some(Method::Oodle));
        }

        Ok(Footer {
            encrypted,
            encryption_guid,
            version,
            index_offset,
            index_size,
            hash,
            frozen,
            compression,
        })
    }
}

/// Parsed .pak archive.
pub struct PakReader<R: Read + Seek> {
    file: R,
    version: Version,
    mount_point: String,
    encrypted_index: bool,
    encryption_guid: Option<[u8; 16]>,
    compression: Vec<Option<Method>>,
    index_offset: u64,
    entries: BTreeMap<String, PakEntry>,
    path_hash_seed: Option<u64>,
    key: Option<AesKey>,
}

fn sha1_hash(data: &[u8]) -> [u8; 20] {
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(data);
    h.finalize().into()
}

impl<R: Read + Seek> PakReader<R> {
    /// Detect version + parse the index. `key` is used for encrypted indices/entries.
    pub fn new(mut file: R, key: Option<AesKey>) -> Result<PakReader<R>> {
        let mut last_err: Option<Error> = None;
        for version in Version::iter() {
            if version.footer_size() < 0 {
                continue;
            }
            let footer_res = (|| -> Result<Footer> {
                let file_len = file.seek(SeekFrom::End(0))?;
                if file_len < version.footer_size() as u64 {
                    return Err(Error::malformed("file smaller than footer"));
                }
                file.seek(SeekFrom::End(-version.footer_size()))?;
                Footer::read(&mut file, version)
            })();
            match footer_res {
                Ok(footer) => {
                    if footer.index_offset == 0 || footer.index_size == 0 {
                        last_err = Some(Error::malformed("empty index"));
                        continue;
                    }
                    return Self::from_footer(file, footer, key);
                }
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or(Error::malformed("no pak footer found")))
    }

    fn from_footer(mut file: R, footer: Footer, key: Option<AesKey>) -> Result<PakReader<R>> {
        // main index
        file.seek(SeekFrom::Start(footer.index_offset))?;
        let mut index_buf = file.read_bytes(footer.index_size as usize)?;
        if footer.encrypted {
            let key = key.as_ref().ok_or(Error::EncryptedNoKey)?;
            key.decrypt_ecb_in_place(&mut index_buf);
        }

        // secondary indexes live outside the main index buffer
        let mut secondary: Vec<(String, Vec<u8>)> = Vec::new();
        let mut cursor = std::io::Cursor::new(&index_buf);
        let mount_point = cursor.fstring()?;
        let entry_count = cursor.u32()?;
        let mut path_hash_seed = None;

        let (entries, phi_present, fdi_present) = if footer.version as u32 >= Version::V10 as u32 {
            let seed = cursor.u64()?;
            path_hash_seed = Some(seed);
            let phi_flag = cursor.u32()?;
            if phi_flag != 0 {
                let phi_offset = cursor.u64()?;
                let phi_size = cursor.u64()?;
                let _phi_hash = cursor.read_bytes(20)?;
                file.seek(SeekFrom::Start(phi_offset))?;
                let phi_read = if footer.encrypted { align(phi_size, 16) } else { phi_size };

                let mut phi_buf = file.read_bytes(phi_read as usize)?;
                if footer.encrypted {
                    key.as_ref().ok_or(Error::EncryptedNoKey)?.decrypt_ecb_in_place(&mut phi_buf);
                }
                secondary.push(("phi".into(), phi_buf));
            }
            let fdi_flag = cursor.u32()?;
            if fdi_flag != 0 {
                let fdi_offset = cursor.u64()?;
                let fdi_size = cursor.u64()?;
                let _fdi_hash = cursor.read_bytes(20)?;
                file.seek(SeekFrom::Start(fdi_offset))?;
                let fdi_read = if footer.encrypted { align(fdi_size, 16) } else { fdi_size };
                let mut fdi_buf = file.read_bytes(fdi_read as usize)?;
                if footer.encrypted {
                    key.as_ref().ok_or(Error::EncryptedNoKey)?.decrypt_ecb_in_place(&mut fdi_buf);
                }
                secondary.push(("fdi".into(), fdi_buf));
            }
            let encoded_size = cursor.u32()? as usize;

            let encoded = cursor.read_bytes(encoded_size)?;
            let non_encoded_count = cursor.u32()? as usize;
            let mut non_encoded = Vec::with_capacity(non_encoded_count);
            for _ in 0..non_encoded_count {
                non_encoded.push(PakEntry::read(&mut cursor, footer.version)?);
            }

            // Build path → entry from the full directory index
            let fdi_buf = secondary
                .iter()
                .find(|(k, _)| k == "fdi")
                .map(|(_, v)| v.clone())
                .ok_or_else(|| Error::malformed("pak has no full directory index"))?;
            let mut fdi = std::io::Cursor::new(fdi_buf);
            let dir_count = fdi.u32()? as usize;
            let mut entries = BTreeMap::new();
            let mut encoded_reader = std::io::Cursor::new(encoded);
            for di in 0..dir_count {
                let dir_name = fdi.fstring()?;
                let file_count = fdi.u32()? as usize;
                for fi in 0..file_count {
                    let file_name = fdi.fstring()?;
                    let encoded_offset = fdi.i32()?;
                    if encoded_offset == i32::MIN {
                        continue; // deleted sentinel
                    }
                    let entry = if encoded_offset >= 0 {
                        encoded_reader.set_position(encoded_offset as u64);
                        PakEntry::read_encoded(&mut encoded_reader, footer.version)?
                    } else {
                        let idx = (-encoded_offset) as usize - 1;
                        non_encoded
                            .get(idx)
                            .cloned()
                            .ok_or_else(|| Error::malformed("non-encoded entry index out of range"))?
                    };
                    let path = format!(
                        "{}{}",
                        dir_name.strip_prefix('/').unwrap_or(&dir_name),
                        file_name
                    );
                    entries.insert(path, entry);
                }
            }
            (entries, secondary.iter().any(|(k, _)| k == "phi"), true)
        } else {
            let mut entries = BTreeMap::new();
            if entry_count > 0x100_0000 {
                return Err(Error::malformed("entry count out of range"));
            }
            for _ in 0..entry_count {
                let path = cursor.fstring()?;
                let entry = PakEntry::read(&mut cursor, footer.version)?;
                entries.insert(path, entry);
            }
            (entries, false, false)
        };
        let _ = (phi_present, fdi_present);

        Ok(PakReader {
            file,
            version: footer.version,
            mount_point,
            encrypted_index: footer.encrypted,
            encryption_guid: footer.encryption_guid,
            compression: footer.compression,
            index_offset: footer.index_offset,
            entries,
            path_hash_seed,
            key,
        })
    }

    pub fn version(&self) -> Version {
        self.version
    }
    pub fn mount_point(&self) -> &str {
        &self.mount_point
    }
    pub fn encrypted_index(&self) -> bool {
        self.encrypted_index
    }
    pub fn path_hash_seed(&self) -> Option<u64> {
        self.path_hash_seed
    }
    pub fn compression_methods(&self) -> &[Option<Method>] {
        &self.compression
    }
    pub fn entries(&self) -> &BTreeMap<String, PakEntry> {
        &self.entries
    }
    pub fn index_offset(&self) -> u64 {
        self.index_offset
    }

    /// Files listed as mount-point-resolved virtual paths (e.g. "Game/Maps/Test.umap").
    pub fn files(&self) -> Vec<String> {
        self.entries.keys().map(|k| self.full_path(k)).collect()
    }

    pub fn full_path(&self, entry_path: &str) -> String {
        normalize_ue_path(&format!("{}{}", self.mount_point, entry_path))
    }

    /// Decompress + decrypt one entry fully into memory (bounded by entry size).
    pub fn read_entry(&mut self, entry_path: &str) -> Result<Vec<u8>> {
        let entry = self
            .entries
            .get(entry_path)
            .ok_or_else(|| Error::NotFound(entry_path.to_string()))?
            .clone();
        let mut out = Vec::with_capacity(entry.uncompressed.min(1 << 30) as usize);
        self.read_entry_into(&entry, &mut out)?;
        Ok(out)
    }

    pub fn read_entry_into(&mut self, entry: &PakEntry, out: &mut Vec<u8>) -> Result<()> {
        if entry.is_deleted() {
            return Err(Error::malformed("entry is deleted"));
        }
        self.file.seek(SeekFrom::Start(entry.offset))?;
        // Re-read the on-disk entry header to find the exact data start.
        let disk_entry = PakEntry::read(&mut self.file, self.version)?;
        let data_offset = self.file.stream_position()?;

        let read_len = if entry.is_encrypted() {
            align(entry.compressed, 16)
        } else {
            entry.compressed
        } as usize;

        let mut data = self.file.read_bytes(read_len)?;
        if entry.is_encrypted() {
            let key = self.key.as_ref().ok_or(Error::EncryptedNoKey)?;
            key.decrypt_ecb_in_place(&mut data);
            data.truncate(entry.compressed as usize);
        }

        let ranges: Vec<(usize, usize)> = match &disk_entry.blocks {
            Some(blocks) => blocks
                .iter()
                .map(|b| {
                    let rel = |v: u64| -> usize {
                        (v - (data_offset - entry.offset)) as usize
                    };
                    // encrypted blocks are padded to AES boundaries; clamp to real data
                    let start = rel(b.start).min(data.len());
                    let end = rel(b.end).min(data.len());
                    (start, end)
                })
                .collect(),
            None => vec![(0, data.len())],
        };

        match entry.compression_slot.and_then(|s| self.compression.get(s as usize).copied().flatten()) {
            None => out.write_all(&data)?,
            Some(method) => {
                let chunk_size = if ranges.len() == 1 {
                    entry.uncompressed as usize
                } else {
                    entry.compression_block_size as usize
                };
                let start_len = out.len();
                out.resize(start_len + entry.uncompressed as usize, 0);
                let mut dst_off = start_len;
                for (start, end) in ranges {
                    let src = &data[start..end];
                    let dst = &mut out[dst_off..];
                    let n = if dst.len() < chunk_size { dst.len() } else { chunk_size };
                    let written = crate::compress::decompress(method, src, &mut dst[..n])?;
                    dst_off += written;
                }
                out.truncate(dst_off);
            }
        }
        Ok(())
    }

    /// Verify the entry's SHA-1 hash against the recorded value (compressed content hash for
    /// compressed entries, raw content for uncompressed ones — matches UE writer behavior).
    pub fn verify_entry_hash(&mut self, entry_path: &str) -> Result<bool> {
        let entry = self
            .entries
            .get(entry_path)
            .ok_or_else(|| Error::NotFound(entry_path.to_string()))?
            .clone();
        let Some(recorded) = entry.hash else {
            return Ok(false);
        };
        let data = self.read_entry(entry_path)?;
        let _ = &entry;
        Ok(sha1_hash(&data) == recorded || {
            // UE hashes compressed blocks for compressed entries; fall back to raw file slice
            let mut buf = Vec::new();
            let _ = self.read_entry_raw_compressed(&entry, &mut buf);
            sha1_hash(&buf) == recorded
        })
    }

    fn read_entry_raw_compressed(&mut self, entry: &PakEntry, out: &mut Vec<u8>) -> Result<()> {
        self.file.seek(SeekFrom::Start(entry.offset))?;
        let disk_entry = PakEntry::read(&mut self.file, self.version)?;
        let data_offset = self.file.stream_position()?;
        let read_len = if entry.is_encrypted() { align(entry.compressed, 16) } else { entry.compressed } as usize;
        let mut data = self.file.read_bytes(read_len)?;
        if entry.is_encrypted() {
            self.key.as_ref().ok_or(Error::EncryptedNoKey)?.decrypt_ecb_in_place(&mut data);
            data.truncate(entry.compressed as usize);
        }
        if let Some(blocks) = &disk_entry.blocks {
            for b in blocks {
                let start = (b.start - (data_offset - entry.offset)) as usize;
                let end = (b.end - (data_offset - entry.offset)) as usize;
                out.extend_from_slice(&data[start..end]);
            }
        } else {
            out.extend_from_slice(&data);
        }
        Ok(())
    }
}

/// Convert a UE virtual path (mount + "../../Proj/Content/..." style) into a clean relative path.
pub fn normalize_ue_path(path: &str) -> String {
    let path = path.replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

/// UE5 Fnv64 path hash (used by path hash index; implemented for verification purposes).
pub fn fnv64_path(path: &str, seed: u64) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x00000100000001b3;
    let mut hash = OFFSET.wrapping_add(seed);
    for b in path.to_lowercase().encode_utf16().flat_map(u16::to_le_bytes) {
        hash ^= b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_entry_layout_v11() {
        // sanity: single-block uncompressed encoded entry
        let entry = PakEntry {
            offset: 1000,
            compressed: 100,
            uncompressed: 100,
            compression_slot: None,
            timestamp: None,
            hash: None,
            blocks: None,
            flags: 0,
            compression_block_size: 0,
        };
        let size = PakEntry::serialized_size(Version::V11, None, 0);
        assert_eq!(size, 8 + 8 + 8 + 4 + 20 + 1 + 4);
        let _ = entry;
    }
}
