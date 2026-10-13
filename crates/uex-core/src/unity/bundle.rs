//! Unity AssetBundle reader: UnityFS (block-based), UnityWeb (LZMA whole-file)
//! and UnityRaw (uncompressed legacy). Handles Unity 5.x → 6000.x header
//! variants, block-info-at-end layout, 16-byte block alignment (2019.4.15+),
//! and Unity's official asset-bundle encryption (UnityCN key + signature
//! validation) so the GUI/CLI can prompt for a key BEFORE extraction.

use crate::error::{Error, Result};
use aes::cipher::{BlockEncrypt, KeyInit};
use aes::Aes128;
use aes::cipher::generic_array::GenericArray;

/// Compression type of a block (flags & 0x3F).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockCompression {
    None,
    Lzma,
    Lz4,
    Lz4Hc,
    Lzham,
}

impl BlockCompression {
    fn from_flags(flags: u32) -> Result<Self> {
        Ok(match flags & 0x3F {
            0 => BlockCompression::None,
            1 => BlockCompression::Lzma,
            2 => BlockCompression::Lz4,
            3 => BlockCompression::Lz4Hc,
            4 => BlockCompression::Lzham,
            other => return Err(Error::unsupported(format!("bundle compression flag {other}"))),
        })
    }
    pub fn name(&self) -> &'static str {
        match self {
            BlockCompression::None => "None",
            BlockCompression::Lzma => "LZMA",
            BlockCompression::Lz4 | BlockCompression::Lz4Hc => "LZ4",
            BlockCompression::Lzham => "LZHAM",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BundleNode {
    /// path inside the bundle, e.g. "CAB-1234abcd" or "CAB-xxx.resS"
    pub path: String,
    pub offset: u64,
    pub size: u64,
    pub flags: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleSignature {
    UnityFs,
    UnityWeb,
    UnityRaw,
}

impl BundleSignature {
    pub fn as_str(&self) -> &'static str {
        match self {
            BundleSignature::UnityFs => "UnityFS",
            BundleSignature::UnityWeb => "UnityWeb",
            BundleSignature::UnityRaw => "UnityRaw",
        }
    }
}

pub struct UnityBundle {
    pub signature: BundleSignature,
    /// bundle format version (6/7)
    pub version: u32,
    pub unity_version: String,
    pub unity_revision: String,
    pub nodes: Vec<BundleNode>,
    /// true when the bundle uses Unity's official UnityCN encryption
    pub encrypted: bool,
    data: Vec<u8>,
    blocks: Vec<Block>,
    decryptor: Option<UnityCnDecryptor>,
}

struct Block {
    /// offset of compressed bytes within the file
    file_offset: u64,
    compressed_size: u64,
    uncompressed_size: u64,
    flags: u32,
    /// offset within the concatenated uncompressed stream
    stream_offset: u64,
}

fn lzma_decompress_alone(input: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    lzma_rs::lzma_decompress(&mut &input[..], &mut out)
        .map_err(|e| Error::malformed(format!("LZMA decompress: {e}")))?;
    Ok(out)
}

/// Raw LZMA1 stream (props + dict, no size field) as used by bundle block info.
fn lzma_decompress_raw(input: &[u8]) -> Result<Vec<u8>> {
    if input.len() < 5 {
        return Err(Error::malformed("lzma data too short"));
    }
    let mut alone = Vec::with_capacity(input.len() + 8);
    alone.extend_from_slice(&input[..5]);
    alone.extend_from_slice(&u64::MAX.to_le_bytes()); // "unknown size" = end marker mandatory
    alone.extend_from_slice(&input[5..]);
    lzma_decompress_alone(&alone)
}

/// Unity CN asset bundle encryption (ArchiveStorageDecryptor port).
/// Requires the game's 16-byte key; validity is verified via a signature
/// check against b"#$unity3dchina!@" so wrong keys fail cleanly.
pub struct UnityCnDecryptor {
    index: [u8; 16],
    substitute: [u8; 16],
}

const UNITY3D_SIGNATURE: [u8; 16] = *b"#$unity3dchina!@";

impl UnityCnDecryptor {
    /// Parse the decryptor header at `pos` and validate `key`.
    pub fn parse(data: &[u8], pos: &mut usize, key: &[u8]) -> Result<UnityCnDecryptor> {
        if key.len() != 16 {
            return Err(Error::EncryptedNoKey);
        }
        let cipher = Aes128::new(GenericArray::from_slice(key));
        let encrypt_block = |cipher: &Aes128, input: &[u8; 16]| -> [u8; 16] {
            let mut block = GenericArray::from(*input);
            cipher.encrypt_block(&mut block);
            block.into()
        };
        let mut read16 = |p: &mut usize| -> Result<[u8; 16]> {
            if *p + 16 > data.len() {
                return Err(Error::malformed("bundle decryptor truncated"));
            }
            let mut a = [0u8; 16];
            a.copy_from_slice(&data[*p..*p + 16]);
            *p += 16;
            Ok(a)
        };
        if *pos + 4 > data.len() {
            return Err(Error::malformed("bundle decryptor truncated"));
        }
        let unknown = u32::from_le_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
        *pos += 4;
        let _ = unknown;
        let data_vec = read16(pos)?;
        let key_vec = read16(pos)?;
        *pos += 1; // separator byte
        let data_sig = read16(pos)?;
        let key_sig = read16(pos)?;

        // validate: AES-ECB-encrypt(key_sig, key) XOR data_sig must equal signature
        let enc_key_sig = encrypt_block(&cipher, &key_sig);
        let mut signature = [0u8; 16];
        for i in 0..16 {
            signature[i] = enc_key_sig[i] ^ data_sig[i];
        }
        if signature != UNITY3D_SIGNATURE {
            return Err(Error::other(
                "Unity asset bundle key INVALID (signature mismatch) — this bundle needs its own 16-character UnityCN key",
            ));
        }

        let enc = encrypt_block(&cipher, &key_vec);
        let mut dec = [0u8; 16];
        for i in 0..16 {
            dec[i] = enc[i] ^ data_vec[i];
        }
        // nibble expand: 16 bytes -> 32 nibbles
        let mut expanded = [0u8; 32];
        for (i, b) in dec.iter().enumerate() {
            expanded[i * 2] = b >> 4;
            expanded[i * 2 + 1] = b & 0xF;
        }
        let mut index = [0u8; 16];
        index.copy_from_slice(&expanded[..16]);
        let mut substitute = [0u8; 16];
        for j in 0..4 {
            for i in 0..4 {
                substitute[i + j * 4] = expanded[0x10 + i * 4 + j];
            }
        }
        Ok(UnityCnDecryptor { index, substitute })
    }

    fn decrypt_byte(&self, data: &mut [u8], offset: usize, index: u32) -> (u8, usize, u32) {
        let idx = index as usize;
        let b: i32 = self.substitute[((index >> 2) & 3) as usize + 4] as i32
            + self.substitute[idx & 3] as i32
            + self.substitute[((index >> 4) & 3) as usize + 8] as i32
            + self.substitute[((index % 256) >> 6) as usize + 12] as i32;
        let cur = data[offset];
        let a = self.index[(cur & 0xF) as usize] as i32 - b;
        let c = self.index[(cur >> 4) as usize] as i32 - b;
        // Python-compatible: two's-complement nibbles, result modulo 256
        data[offset] = ((a & 0xF) | (0x10 * c)).rem_euclid(256) as u8;
        (data[offset], offset + 1, index.wrapping_add(1))
    }

    /// Decrypt one chunk; returns bytes consumed (UnityPy ArchiveStorageDecryptor.decrypt).
    fn decrypt_chunk(&self, data: &mut [u8], mut index: u32, remaining: usize) -> usize {
        let mut offset = 0usize;
        let (cur_byte, mut off, mut idx) = self.decrypt_byte(data, offset, index);
        let mut byte_high: u32 = (cur_byte >> 4) as u32;
        let byte_low = cur_byte & 0xF;
        if byte_high == 0xF {
            loop {
                let (b, o, i) = self.decrypt_byte(data, off, idx);
                off = o;
                idx = i;
                byte_high += b as u32;
                if b != 0xFF {
                    break;
                }
            }
        }
        off += byte_high as usize;
        if off < remaining {
            let (_, o, i) = self.decrypt_byte(data, off, idx);
            let (_, o, i) = self.decrypt_byte(data, o, i);
            off = o;
            idx = i;
            if byte_low == 0xF {
                loop {
                    let (b, o, i) = self.decrypt_byte(data, off, idx);
                    off = o;
                    idx = i;
                    if b != 0xFF {
                        break;
                    }
                }
            }
        }
        off
    }

    /// Decrypt one block in place (UnityPy ArchiveStorageDecryptor.decrypt_block).
    pub fn decrypt_block(&self, data: &mut [u8], mut index: u32) {
        let mut offset = 0usize;
        let size = data.len();
        while offset < size {
            let remaining = size - offset;
            let advanced = {
                let slice = &mut data[offset..];
                self.decrypt_chunk(slice, index, remaining)
            };
            offset += advanced;
            index = index.wrapping_add(1);
        }
    }
}

/// Cursor over the bundle header. NOTE: Unity AssetBundle headers (and the
/// compressed blocks-info structure) are BIG-endian, unlike most of the
/// payload (SerializedFiles carry their own endianness flag).
struct Ctx<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Ctx<'a> {
    fn u8(&mut self) -> u8 {
        let b = self.data[self.pos];
        self.pos += 1;
        b
    }
    fn bytes(&mut self, n: usize) -> &'a [u8] {
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        s
    }
    fn u16(&mut self) -> u16 {
        let b = self.bytes(2);
        u16::from_be_bytes([b[0], b[1]])
    }
    fn u32(&mut self) -> u32 {
        let b = self.bytes(4);
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    }
    fn u64(&mut self) -> u64 {
        let b = self.bytes(8);
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        u64::from_be_bytes(a)
    }
    fn string_to_null(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos] != 0 {
            self.pos += 1;
        }
        let s = String::from_utf8_lossy(&self.data[start..self.pos]).into_owned();
        if self.pos < self.data.len() {
            self.pos += 1;
        }
        s
    }
    fn align(&mut self, to: usize) {
        let m = self.pos % to;
        if m != 0 {
            self.pos += to - m;
        }
    }
}

/// version tuple comparison helper: "2020.3.34" style
fn ver_tuple(s: &str) -> (u16, u16, u16) {
    match crate::unity::UnityVersion::parse(s) {
        Ok(v) => (v.major(), v.minor(), v.build()),
        Err(_) => (0, 0, 0),
    }
}

fn ver_lt(a: (u16, u16, u16), b: (u16, u16, u16)) -> bool {
    a < b
}

impl UnityBundle {
    pub fn parse(data: Vec<u8>, key: Option<&[u8]>) -> Result<UnityBundle> {
        let sig = if data.starts_with(b"UnityFS\0") {
            BundleSignature::UnityFs
        } else if data.starts_with(b"UnityWeb\0") {
            BundleSignature::UnityWeb
        } else if data.starts_with(b"UnityRaw\0") {
            BundleSignature::UnityRaw
        } else {
            return Err(Error::malformed("not a Unity bundle"));
        };

        let (version, version_player, version_engine, header_pos) = {
            let mut c = Ctx { data: &data, pos: 0 };
            let _sig_str = c.string_to_null();
            let version = c.u32();
            let version_player = c.string_to_null();
            let version_engine = c.string_to_null();
            (version, version_player, version_engine, c.pos)
        };

        match sig {
            BundleSignature::UnityFs => {
                Self::read_fs(data, header_pos, sig, version, &version_player, &version_engine, key)
            }
            BundleSignature::UnityWeb | BundleSignature::UnityRaw => {
                Self::read_web_raw(data, header_pos, sig, version, &version_player, &version_engine)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn read_web_raw(
        data: Vec<u8>,
        header_pos: usize,
        sig: BundleSignature,
        version: u32,
        version_player: &str,
        version_engine: &str,
    ) -> Result<UnityBundle> {
        let mut c = Ctx { data: &data, pos: header_pos };
        if version >= 4 {
            let _hash = c.bytes(16);
            let _crc = c.u32();
        }
        let _minimum_streamed_bytes = c.u32();
        let header_size = c.u32() as usize;
        let _n_download = c.u32();
        let level_count = c.u32() as usize;
        c.bytes(level_count.saturating_sub(1) * 8);
        let compressed_size = c.u32() as usize;
        let _uncompressed_size = c.u32();
        if version >= 2 {
            let _complete_file_size = c.u32();
        }
        if version >= 3 {
            let _file_info_header_size = c.u32();
        }
        if header_size > data.len() {
            return Err(Error::malformed("bundle header size out of bounds"));
        }
        let payload = &data[header_size..(header_size + compressed_size).min(data.len())];
        let raw = if sig == BundleSignature::UnityWeb {
            lzma_decompress_alone(payload)?
        } else {
            payload.to_vec()
        };
        let mut bc = Ctx { data: &raw, pos: 0 };
        let node_count = bc.u32() as usize;
        let mut nodes = Vec::with_capacity(node_count);
        for _ in 0..node_count {
            let path = bc.string_to_null();
            let offset = bc.u32() as u64;
            let size = bc.u32() as u64;
            nodes.push(BundleNode { path, offset, size, flags: 0 });
        }
        Ok(UnityBundle {
            signature: sig,
            version,
            unity_version: version_player.to_string(),
            unity_revision: version_engine.to_string(),
            nodes,
            encrypted: false,
            data,
            blocks: Vec::new(),
            decryptor: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn read_fs(
        data: Vec<u8>,
        header_pos: usize,
        sig: BundleSignature,
        version: u32,
        version_player: &str,
        version_engine: &str,
        key: Option<&[u8]>,
    ) -> Result<UnityBundle> {
        let mut c = Ctx { data: &data, pos: header_pos };
        let _size = c.u64();
        let compressed_size = c.u32() as usize;
        let uncompressed_size = c.u32() as usize;
        let dataflags = c.u32();

        // UnityWeb version 6 has one extra byte
        if sig != BundleSignature::UnityFs {
            let _pad = c.u8();
        }

        let v = ver_tuple(&version_engine);
        // ArchiveFlags vs ArchiveFlagsOld selection (UnityPy read_fs)
        let uses_new_flags = !(
            ver_lt(v, (2020, 0, 0))
                || (v.0 == 2020 && ver_lt(v, (2020, 3, 34)))
                || (v.0 == 2021 && ver_lt(v, (2021, 3, 2)))
                || (v.0 == 2022 && ver_lt(v, (2022, 1, 1)))
        );

        let enc_flag: u32 = if uses_new_flags { 0x1400 } else { 0x400 };
        let encrypted = dataflags & enc_flag != 0;
        let mut decryptor: Option<UnityCnDecryptor> = None;
        if encrypted {
            // without a key the bundle can still be LISTED (blocks info is
            // plain); the key is validated lazily when blocks are read or
            // when the caller calls UnityCnDecryptor::parse explicitly
            if let Some(key) = key {
                decryptor = Some(UnityCnDecryptor::parse(c.data, &mut c.pos, key)?);
            } else {
                // skip the decryptor header fields to keep positions correct
                // (u32 unknown + 16 + 16 + 1 + 16 + 16)
                c.bytes(4 + 16 + 16 + 1 + 16 + 16);
            }
        }

        // alignment for format version >= 7 or engine >= 2019.4.15
        let aligned = version >= 7 || (v.0 == 2019 && v >= (2019, 4, 15));
        if aligned {
            c.align(16);
        }

        let blocks_info_at_end = dataflags & 0x80 != 0;
        let start = c.pos;
        let blocks_info: Vec<u8> = if blocks_info_at_end {
            if compressed_size > data.len() {
                return Err(Error::malformed("bundle blocks info size out of bounds"));
            }
            data[data.len() - compressed_size..].to_vec()
        } else {
            if start + compressed_size > data.len() {
                return Err(Error::malformed("bundle blocks info out of bounds"));
            }
            data[start..start + compressed_size].to_vec()
        };

        // decompress blocks info
        let comp = BlockCompression::from_flags(dataflags)?;
        let decompressed: Vec<u8> = match comp {
            BlockCompression::None => blocks_info,
            BlockCompression::Lzma => lzma_decompress_raw(&blocks_info)?,
            BlockCompression::Lz4 | BlockCompression::Lz4Hc => {
                let mut out = vec![0u8; uncompressed_size];
                lz4_flex::block::decompress_into(&blocks_info, &mut out)
                    .map_err(|_| Error::DecompressionFailed { method: "LZ4" })?;
                out
            }
            BlockCompression::Lzham => return Err(Error::unsupported("LZHAM block info")),
        };

        let mut bi = Ctx {
            data: &decompressed,
            pos: 0,
        };
        let _hash = bi.bytes(16);
        let block_count = bi.u32() as usize;
        let mut blocks: Vec<Block> = Vec::with_capacity(block_count);
        let mut stream_off = 0u64;
        for _ in 0..block_count {
            let uncompressed = bi.u32() as u64;
            let _compressed = bi.u32() as u64;
            let flags = bi.u16() as u32;
            blocks.push(Block {
                file_offset: 0,
                compressed_size: _compressed,
                uncompressed_size: uncompressed,
                flags,
                stream_offset: stream_off,
            });
            stream_off += uncompressed;
        }
        let node_count = bi.u32() as usize;
        let mut nodes = Vec::with_capacity(node_count);
        for _ in 0..node_count {
            let offset = bi.u64();
            let size = bi.u64();
            let flags = bi.u32();
            let path = bi.string_to_null();
            nodes.push(BundleNode { path, offset, size, flags });
        }

        // where block data begins in the file
        let mut base = if blocks_info_at_end { start } else { start + compressed_size };
        if dataflags & 0x200 != 0 {
            base = base.div_ceil(16) * 16;
        }
        if base > data.len() {
            return Err(Error::malformed("bundle block data out of bounds"));
        }
        let mut file_off = base as u64;
        for b in blocks.iter_mut() {
            b.file_offset = file_off;
            file_off += b.compressed_size;
        }
        // end all borrows of `data` before moving it into the result
        drop(bi);
        drop(c);

        Ok(UnityBundle {
            signature: sig,
            version,
            unity_version: version_player.to_string(),
            unity_revision: version_engine.to_string(),
            nodes,
            encrypted,
            data,
            blocks,
            decryptor,
        })
    }

    /// Read a byte range from the uncompressed block stream.
    pub fn read_range(&self, offset: u64, size: u64) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(size as usize);
        let mut remaining = size;
        let mut cur = offset;
        while remaining > 0 {
            let Some(idx) = self.block_index_for(cur) else {
                return Err(Error::malformed(format!(
                    "bundle read at {offset}+{size} beyond block stream"
                )));
            };
            let b = &self.blocks[idx];
            let in_block = cur - b.stream_offset;
            let take = remaining.min(b.uncompressed_size - in_block);
            let start = b.file_offset as usize;
            let end = start + b.compressed_size as usize;
            if end > self.data.len() {
                return Err(Error::malformed("bundle block range out of bounds"));
            }
            let compressed = &self.data[start..end];
            let comp = BlockCompression::from_flags(b.flags)?;
            let mut block_buf: Vec<u8> = match comp {
                BlockCompression::None => compressed.to_vec(),
                BlockCompression::Lzma => lzma_decompress_raw(compressed)?,
                BlockCompression::Lz4 | BlockCompression::Lz4Hc => {
                    let mut out = vec![0u8; b.uncompressed_size as usize];
                    lz4_flex::block::decompress_into(compressed, &mut out)
                        .map_err(|_| Error::DecompressionFailed { method: "LZ4" })?;
                    out
                }
                BlockCompression::Lzham => return Err(Error::unsupported("LZHAM block decompression")),
            };
            // per-block UnityCN decryption (flag 0x100), keyed by block index
            if let Some(dec) = &self.decryptor {
                if b.flags & 0x100 != 0 && comp != BlockCompression::None {
                    dec.decrypt_block(&mut block_buf, idx as u32);
                }
            }
            if (in_block + take) as usize > block_buf.len() {
                return Err(Error::malformed("bundle block content shorter than expected"));
            }
            out.extend_from_slice(&block_buf[in_block as usize..(in_block + take) as usize]);
            cur += take;
            remaining -= take;
        }
        Ok(out)
    }

    fn block_index_for(&self, offset: u64) -> Option<usize> {
        let mut acc = 0u64;
        for (i, b) in self.blocks.iter().enumerate() {
            let next = acc + b.uncompressed_size;
            if offset >= acc && offset < next {
                return Some(i);
            }
            acc = next;
        }
        None
    }

    /// Read a whole node (serialized file / resS stream).
    pub fn read_node(&self, node: &BundleNode) -> Result<Vec<u8>> {
        self.read_range(node.offset, node.size)
    }
}
