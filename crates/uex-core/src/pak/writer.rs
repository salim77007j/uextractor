//! Minimal PAK writer (V8B/V10/V11) — powers the CLI `pack` command and generates
//! byte-exact synthetic archives for the test suite.

use super::{reader::Block, PakEntry, Version, MAGIC};
use crate::compress::{self, Method};
use crate::crypto::AesKey;
use crate::error::{Error, Result};
use crate::io::{write_fstring, UexRead};
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::io::{Seek, SeekFrom, Write};

pub struct PakWriter<W: Write + Seek> {
    writer: W,
    version: Version,
    mount_point: String,
    key: Option<AesKey>,
    allowed_compression: Vec<Method>,
    entries: BTreeMap<String, PakEntry>,
    encrypt_index: bool,
}

impl<W: Write + Seek> PakWriter<W> {
    pub fn new(
        writer: W,
        version: Version,
        mount_point: &str,
        key: Option<AesKey>,
        allowed_compression: Vec<Method>,
        encrypt_index: bool,
    ) -> Self {
        Self {
            writer,
            version,
            mount_point: mount_point.to_string(),
            key,
            allowed_compression,
            entries: BTreeMap::new(),
            encrypt_index,
        }
    }

    pub fn write_file(&mut self, path: &str, data: &[u8]) -> Result<()> {
        let entry_offset = self.writer.stream_position()?;

        // compression: pick first allowed method that actually shrinks
        let mut compressed_blocks: Vec<Vec<u8>> = Vec::new();
        let mut compression_slot: Option<u32> = None;
        let mut compressed_size = 0u64;
        let block_size_limit: usize = 0x3e << 11; // 126 KiB max encodable block size

        if !data.is_empty() && !self.allowed_compression.is_empty() {
            for (slot, method) in self.allowed_compression.iter().enumerate() {
                let mut blocks = Vec::new();
                let mut ok = true;
                let mut total = 0u64;
                for chunk in data.chunks(block_size_limit) {
                    let comp = compress::compress(*method, chunk)?;
                    if comp.len() >= chunk.len() {
                        ok = false;
                        break;
                    }
                    total += comp.len() as u64;
                    blocks.push(comp);
                }
                if ok {
                    compression_slot = Some(slot as u32);
                    compressed_size = total;
                    compressed_blocks = blocks;
                    break;
                }
            }
        }

        let uncompressed = data.len() as u64;
        let (entry, payload) = if let Some(slot) = compression_slot {
            let count = compressed_blocks.len() as u32;
            let mut offset = PakEntry::serialized_size(self.version, Some(slot), count);
            let mut blocks = Vec::with_capacity(compressed_blocks.len());
            for b in &compressed_blocks {
                let start = offset;
                offset += b.len() as u64;
                blocks.push(Block { start, end: offset });
            }
            // hash = SHA1 over compressed blocks (matches repak behavior)
            let mut hasher = Sha1::new();
            for b in &compressed_blocks {
                hasher.update(b);
            }
            let hash: [u8; 20] = hasher.finalize().into();
            let mut payload = Vec::with_capacity(compressed_size as usize);
            for b in &compressed_blocks {
                payload.extend_from_slice(b);
            }
            let entry = PakEntry {
                offset: entry_offset,
                compressed: compressed_size,
                uncompressed,
                compression_slot: Some(slot),
                timestamp: None,
                hash: Some(hash),
                blocks: Some(blocks),
                flags: 0,
                compression_block_size: block_size_limit as u32,
            };
            (entry, payload)
        } else {
            let mut hasher = Sha1::new();
            hasher.update(data);
            let hash: [u8; 20] = hasher.finalize().into();
            let entry = PakEntry {
                offset: entry_offset,
                compressed: uncompressed,
                uncompressed,
                compression_slot: None,
                timestamp: None,
                hash: Some(hash),
                blocks: None,
                flags: 0,
                compression_block_size: 0,
            };
            (entry, data.to_vec())
        };

        // optional entry-level encryption (whole compressed payload, AES-ECB).
        // Each compressed block is individually padded to 16 bytes so that the
        // reader's block-index reconstruction (which aligns encrypted block sizes)
        // stays consistent.
        let (entry, payload) = if self.key.is_some() {
            let key = self.key.as_ref().unwrap();
            if compression_slot.is_some() {
                let mut aligned_blocks = Vec::with_capacity(compressed_blocks.len());
                let mut blocks = Vec::with_capacity(compressed_blocks.len());
                let mut offset = PakEntry::serialized_size(self.version, compression_slot, compressed_blocks.len() as u32);
                let mut total = 0u64;
                for b in &compressed_blocks {
                    let mut b = b.clone();
                    while b.len() % 16 != 0 {
                        b.push(0);
                    }
                    let start = offset;
                    offset += b.len() as u64;
                    blocks.push(Block { start, end: start + b.len() as u64 });
                    total += b.len() as u64;
                    aligned_blocks.push(b);
                }
                let mut payload = Vec::with_capacity(total as usize);
                for b in &aligned_blocks {
                    payload.extend_from_slice(b);
                }
                key.encrypt_ecb_in_place(&mut payload);
                let entry = PakEntry {
                    offset: entry_offset,
                    // on-disk compressed size = sum of the AES-aligned block sizes
                    compressed: total,
                    uncompressed,
                    compression_slot,
                    timestamp: None,
                    hash: entry.hash,
                    blocks: Some(blocks),
                    flags: 1,
                    compression_block_size: block_size_limit as u32,
                };
                (entry, payload)
            } else {
                let mut payload = payload;
                while payload.len() % 16 != 0 {
                    payload.push(0);
                }
                key.encrypt_ecb_in_place(&mut payload);
                let mut entry = entry;
                entry.flags |= 1;
                (entry, payload)
            }
        } else {
            (entry, payload)
        };

        write_entry_data(&mut self.writer, &entry, self.version, &payload)?;
        self.entries.insert(path.to_string(), entry);
        Ok(())
    }

    /// Finalize: write index (path-hash + full directory + encoded entries), footer.
    pub fn finalize(mut self) -> Result<W> {
        let index_offset = self.writer.stream_position()?;

        let mut index_buf = Vec::new();
        write_fstring(&mut index_buf, &self.mount_point)?;
        index_buf.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());

        let secondary = if self.version as u32 >= Version::V10 as u32 {
            let seed = 0x9E3779B97F4A7C15u64;
            index_buf.extend_from_slice(&seed.to_le_bytes());

            // encode entries
            let mut encoded = Vec::new();
            let mut offsets = Vec::with_capacity(self.entries.len());
            for entry in self.entries.values() {
                offsets.push(encoded.len() as u32);
                write_encoded_entry(&mut encoded, entry, self.version)?;
            }

            // Account for AES padding of the encrypted index. The final index buffer will
            // additionally contain: 2 flag blocks (40 bytes each) + encoded-size u32 +
            // encoded blob + non-encoded count u32.
            let final_index_len = index_buf.len() as u64
                + 40 + 40
                + 4 + encoded.len() as u64
                + 4;
            let index_padding = if self.encrypt_index { (16 - (final_index_len % 16)) % 16 } else { 0 };
            let phi_offset = index_offset + final_index_len + index_padding;

            // path hash index
            let mut phi = Vec::new();
            phi.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
            for (path, off) in self.entries.keys().zip(&offsets) {
                phi.extend_from_slice(&super::reader::fnv64_path(path, seed).to_le_bytes());
                phi.extend_from_slice(&off.to_le_bytes());
            }
            phi.extend_from_slice(&0u32.to_le_bytes());

            // full directory index
            let mut fdi = Vec::new();
            let mut dirs: BTreeMap<String, BTreeMap<String, u32>> = BTreeMap::new();
            for (path, off) in self.entries.keys().zip(&offsets) {
                let path = path.trim_start_matches('/');
                let (dir, file) = match path.rfind('/') {
                    Some(i) => (&path[..=i], &path[i + 1..]),
                    None => ("/", path),
                };
                dirs.entry(format!("/{dir}")).or_default().insert(file.to_string(), *off);
            }
            fdi.extend_from_slice(&(dirs.len() as u32).to_le_bytes());
            for (dir, files) in &dirs {
                write_fstring(&mut fdi, dir)?;
                fdi.extend_from_slice(&(files.len() as u32).to_le_bytes());
                for (file, off) in files {
                    write_fstring(&mut fdi, file)?;
                    fdi.extend_from_slice(&off.to_le_bytes());
                }
            }

            index_buf.extend_from_slice(&1u32.to_le_bytes());
            index_buf.extend_from_slice(&phi_offset.to_le_bytes());
            index_buf.extend_from_slice(&(phi.len() as u64).to_le_bytes());
            index_buf.extend_from_slice(&sha1(&phi));

            let phi_padding = if self.encrypt_index { (16 - (phi.len() % 16)) % 16 } else { 0 };
            let fdi_offset = phi_offset + phi.len() as u64 + phi_padding as u64;
            index_buf.extend_from_slice(&1u32.to_le_bytes());
            index_buf.extend_from_slice(&fdi_offset.to_le_bytes());
            index_buf.extend_from_slice(&(fdi.len() as u64).to_le_bytes());
            index_buf.extend_from_slice(&sha1(&fdi));

            index_buf.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
            index_buf.extend_from_slice(&encoded);
            index_buf.extend_from_slice(&0u32.to_le_bytes()); // non-encoded count

            Some((phi, fdi, phi_offset, fdi_offset))
        } else {
            for (path, entry) in &self.entries {
                write_fstring(&mut index_buf, path)?;
                write_entry_index(&mut index_buf, entry, self.version)?;
            }
            None
        };

        let index_hash = sha1(&index_buf);
        if self.encrypt_index {
            let key = self.key.as_ref().ok_or(Error::EncryptedNoKey)?;
            while index_buf.len() % 16 != 0 {
                index_buf.push(0);
            }
            key.encrypt_ecb_in_place(&mut index_buf);
        }
        self.writer.write_all(&index_buf)?;

        if let Some((phi, fdi, phi_offset, fdi_offset)) = secondary {
            let phi = if self.encrypt_index {
                let mut phi = phi;
                pad16(&mut phi);
                self.key.as_ref().unwrap().encrypt_ecb_in_place(&mut phi);
                phi
            } else {
                phi
            };
            let fdi = if self.encrypt_index {
                let mut fdi = fdi;
                pad16(&mut fdi);
                self.key.as_ref().unwrap().encrypt_ecb_in_place(&mut fdi);
                fdi
            } else {
                fdi
            };
            let _ = (phi_offset, fdi_offset);
            self.writer.write_all(&phi)?;
            self.writer.write_all(&fdi)?;
        }

        // footer
        let footer = FooterSnapshot {
            version: self.version,
            encrypt_index: self.encrypt_index,
            allowed_compression: self.allowed_compression.clone(),
        };
        write_fstring_placeholder_footer(&mut self.writer, &footer, index_offset, index_buf.len() as u64, index_hash)?;

        Ok(self.writer)
    }
}

fn pad16(buf: &mut Vec<u8>) {
    while buf.len() % 16 != 0 {
        buf.push(0);
    }
}

fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h = Sha1::new();
    h.update(data);
    h.finalize().into()
}

fn write_entry_data(w: &mut impl Write, entry: &PakEntry, version: Version, payload: &[u8]) -> Result<()> {
    // header with offset=0 (data location), then payload
    w.write_all(&0u64.to_le_bytes())?;
    w.write_all(&entry.compressed.to_le_bytes())?;
    w.write_all(&entry.uncompressed.to_le_bytes())?;
    let slot = entry.compression_slot.map_or(0, |n| n + 1);
    if version == Version::V8A {
        w.write_all(&(slot as u8).to_le_bytes())?;
    } else {
        w.write_all(&slot.to_le_bytes())?;
    }
    if version.major() as u32 == 1 {
        w.write_all(&entry.timestamp.unwrap_or(0).to_le_bytes())?;
    }
    let hash = entry.hash.unwrap_or([0; 20]);
    w.write_all(&hash)?;
    if version.major() as u32 >= 3 {
        if let Some(blocks) = &entry.blocks {
            w.write_all(&(blocks.len() as u32).to_le_bytes())?;
            for b in blocks {
                w.write_all(&b.start.to_le_bytes())?;
                w.write_all(&b.end.to_le_bytes())?;
            }
        }
        w.write_all(&[entry.flags])?;
        w.write_all(&entry.compression_block_size.to_le_bytes())?;
    }
    w.write_all(payload)?;
    Ok(())
}

fn write_entry_index(w: &mut impl Write, entry: &PakEntry, version: Version) -> Result<()> {
    w.write_all(&entry.offset.to_le_bytes())?;
    w.write_all(&entry.compressed.to_le_bytes())?;
    w.write_all(&entry.uncompressed.to_le_bytes())?;
    let slot = entry.compression_slot.map_or(0, |n| n + 1);
    if version == Version::V8A {
        w.write_all(&(slot as u8).to_le_bytes())?;
    } else {
        w.write_all(&slot.to_le_bytes())?;
    }
    if version.major() as u32 == 1 {
        w.write_all(&entry.timestamp.unwrap_or(0).to_le_bytes())?;
    }
    let hash = entry.hash.unwrap_or([0; 20]);
    w.write_all(&hash)?;
    if version.major() as u32 >= 3 {
        if let Some(blocks) = &entry.blocks {
            w.write_all(&(blocks.len() as u32).to_le_bytes())?;
            for b in blocks {
                w.write_all(&b.start.to_le_bytes())?;
                w.write_all(&b.end.to_le_bytes())?;
            }
        }
        w.write_all(&[entry.flags])?;
        w.write_all(&entry.compression_block_size.to_le_bytes())?;
    }
    Ok(())
}

fn write_encoded_entry(w: &mut Vec<u8>, entry: &PakEntry, version: Version) -> Result<()> {
    let block_count = entry.blocks.as_ref().map_or(0, |b| b.len()) as u32;
    let mut bs_flag = (entry.compression_block_size >> 11) & 0x3f;
    if (bs_flag << 11) != entry.compression_block_size {
        bs_flag = 0x3f;
    }
    let encrypted = entry.is_encrypted();
    let slot = entry.compression_slot.map_or(0, |n| n + 1);
    let bits: u32 = bs_flag
        | (block_count << 6)
        | ((encrypted as u32) << 22)
        | (slot << 23)
        | (1 << 29) // 32-bit compressed
        | (1 << 30) // 32-bit uncompressed
        | (1 << 31); // 32-bit offset
    w.extend_from_slice(&bits.to_le_bytes());
    if bs_flag == 0x3f {
        w.extend_from_slice(&entry.compression_block_size.to_le_bytes());
    }
    w.extend_from_slice(&(entry.offset as u32).to_le_bytes());
    w.extend_from_slice(&(entry.uncompressed as u32).to_le_bytes());
    if entry.compression_slot.is_some() {
        w.extend_from_slice(&(entry.compressed as u32).to_le_bytes());
        let blocks = entry.blocks.as_ref().unwrap();
        if blocks.len() > 1 || encrypted {
            for b in blocks {
                w.extend_from_slice(&((b.end - b.start) as u32).to_le_bytes());
            }
        }
    }
    let _ = version;
    Ok(())
}

struct FooterSnapshot {
    version: Version,
    encrypt_index: bool,
    allowed_compression: Vec<crate::compress::Method>,
}

fn write_fstring_placeholder_footer<W: Write + Seek>(
    w: &mut W,
    pw: &FooterSnapshot,
    index_offset: u64,
    index_size: u64,
    hash: [u8; 20],
) -> Result<()> {
    let version = pw.version;
    if version.major() as u32 >= 7 {
        w.write_all(&[0u8; 16])?; // encryption guid
    }
    if version.major() as u32 >= 4 {
        w.write_all(&[pw.encrypt_index as u8])?;
    }
    w.write_all(&MAGIC.to_le_bytes())?;
    w.write_all(&version.as_u32().to_le_bytes())?;
    w.write_all(&index_offset.to_le_bytes())?;
    w.write_all(&index_size.to_le_bytes())?;
    w.write_all(&hash)?;
    if version.major() as u32 == 9 {
        w.write_all(&[0])?; // frozen
    }
    let slot_count = if (version as u32) < Version::V8A as u32 {
        0
    } else if (version as u32) < Version::V8B as u32 {
        4
    } else {
        5
    };
    for i in 0..5 {
        let mut name = [0u8; 32];
        if i < slot_count {
            if let Some(m) = pw.allowed_compression.get(i) {
                for (j, b) in m.name().as_bytes().iter().enumerate() {
                    name[j] = *b;
                }
            }
        }
        w.write_all(&name)?;
    }
    Ok(())
}
