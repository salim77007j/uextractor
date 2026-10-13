//! IOStore (zen) container support: `.utoc` table of contents + `.ucas` container data.

pub mod zen;

use crate::compress::Method;
use crate::crypto::AesKey;
use crate::error::{Error, Result};
use crate::io::UexRead;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub const TOC_MAGIC: [u8; 16] = *b"-==--==--==--==-";
pub const CONTAINER_HEADER_MAGIC: u32 = 0x496f436e; // "IoCn"

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TocVersion {
    Invalid = 0,
    Initial = 1,
    DirectoryIndex = 2,
    PartitionSize = 3,
    PerfectHash = 4,
    PerfectHashWithOverflow = 5,
    OnDemandMetaData = 6,
    RemovedOnDemandMetaData = 7,
    ReplaceIoChunkHashWithIoHash = 8,
}

impl TocVersion {
    pub const fn from_u32(v: u32) -> Option<TocVersion> {
        Some(match v {
            0 => TocVersion::Invalid,
            1 => TocVersion::Initial,
            2 => TocVersion::DirectoryIndex,
            3 => TocVersion::PartitionSize,
            4 => TocVersion::PerfectHash,
            5 => TocVersion::PerfectHashWithOverflow,
            6 => TocVersion::OnDemandMetaData,
            7 => TocVersion::RemovedOnDemandMetaData,
            8 => TocVersion::ReplaceIoChunkHashWithIoHash,
            _ => return None,
        })
    }
}

/// Chunk types; encoding differs between "old" (UE4.25-era, TOC <= PerfectHash)
/// and "new" (UE5, TOC > PerfectHash) containers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChunkType {
    ExportBundleData,
    BulkData,
    OptionalBulkData,
    MemoryMappedBulkData,
    ScriptObjects,
    ContainerHeader,
    ExternalFile,
    ShaderCodeLibrary,
    ShaderCode,
    PackageStoreEntry,
    DerivedData,
    EditorDerivedData,
    PackageResource,
    InstallManifest,
    LoaderGlobalMeta,
    LoaderInitialLoadMeta,
    LoaderGlobalNames,
    LoaderGlobalHashes,
}

impl ChunkType {
    pub fn from_raw(value: u8, is_new: bool) -> Option<ChunkType> {
        Some(if is_new {
            match value {
                1 => ChunkType::ExportBundleData,
                2 => ChunkType::BulkData,
                3 => ChunkType::OptionalBulkData,
                4 => ChunkType::MemoryMappedBulkData,
                5 => ChunkType::ScriptObjects,
                6 => ChunkType::ContainerHeader,
                7 => ChunkType::ExternalFile,
                8 => ChunkType::ShaderCodeLibrary,
                9 => ChunkType::ShaderCode,
                10 => ChunkType::PackageStoreEntry,
                11 => ChunkType::DerivedData,
                12 => ChunkType::EditorDerivedData,
                13 => ChunkType::PackageResource,
                _ => return None,
            }
        } else {
            match value {
                1 => ChunkType::InstallManifest,
                2 => ChunkType::ExportBundleData,
                3 => ChunkType::BulkData,
                4 => ChunkType::OptionalBulkData,
                5 => ChunkType::MemoryMappedBulkData,
                6 => ChunkType::LoaderGlobalMeta,
                7 => ChunkType::LoaderInitialLoadMeta,
                8 => ChunkType::LoaderGlobalNames,
                9 => ChunkType::LoaderGlobalHashes,
                10 => ChunkType::ContainerHeader,
                11 => ChunkType::ShaderCodeLibrary,
                12 => ChunkType::ShaderCode,
                _ => return None,
            }
        })
    }

    pub fn to_raw(self, is_new: bool) -> u8 {
        if is_new {
            match self {
                ChunkType::ExportBundleData => 1,
                ChunkType::BulkData => 2,
                ChunkType::OptionalBulkData => 3,
                ChunkType::MemoryMappedBulkData => 4,
                ChunkType::ScriptObjects => 5,
                ChunkType::ContainerHeader => 6,
                ChunkType::ExternalFile => 7,
                ChunkType::ShaderCodeLibrary => 8,
                ChunkType::ShaderCode => 9,
                ChunkType::PackageStoreEntry => 10,
                ChunkType::DerivedData => 11,
                ChunkType::EditorDerivedData => 12,
                ChunkType::PackageResource => 13,
                _ => 0,
            }
        } else {
            match self {
                ChunkType::InstallManifest => 1,
                ChunkType::ExportBundleData => 2,
                ChunkType::BulkData => 3,
                ChunkType::OptionalBulkData => 4,
                ChunkType::MemoryMappedBulkData => 5,
                ChunkType::LoaderGlobalMeta => 6,
                ChunkType::LoaderInitialLoadMeta => 7,
                ChunkType::LoaderGlobalNames => 8,
                ChunkType::LoaderGlobalHashes => 9,
                ChunkType::ContainerHeader => 10,
                ChunkType::ShaderCodeLibrary => 11,
                ChunkType::ShaderCode => 12,
                _ => 0,
            }
        }
    }
}

/// 12-byte chunk id: 8-byte package id + 2-byte chunk index + 1 type byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkId {
    pub id: [u8; 12],
}

impl ChunkId {
    pub fn new(package_id: u64, chunk_index: u16, chunk_type: ChunkType, is_new: bool) -> Self {
        let mut id = [0u8; 12];
        id[0..8].copy_from_slice(&package_id.to_le_bytes());
        id[8..10].copy_from_slice(&chunk_index.to_le_bytes());
        id[11] = chunk_type.to_raw(is_new);
        Self { id }
    }

    pub fn package_id(&self) -> u64 {
        u64::from_le_bytes(self.id[0..8].try_into().unwrap())
    }

    pub fn chunk_index(&self) -> u16 {
        u16::from_le_bytes(self.id[8..10].try_into().unwrap())
    }

    /// The raw type byte. Note: callers must know the container's `is_new` mode
    /// to interpret this correctly.
    pub fn type_raw(&self) -> u8 {
        self.id[11]
    }

    pub fn chunk_type(&self, is_new: bool) -> Option<ChunkType> {
        ChunkType::from_raw(self.id[11], is_new)
    }
}

impl std::fmt::Display for ChunkId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", hex::encode(self.id))
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct OffsetAndLength {
    pub data: [u8; 10],
}

impl OffsetAndLength {
    pub fn offset(&self) -> u64 {
        let d = &self.data;
        u64::from_be_bytes([0, 0, 0, d[0], d[1], d[2], d[3], d[4]])
    }
    pub fn length(&self) -> u64 {
        let d = &self.data;
        u64::from_be_bytes([0, 0, 0, d[5], d[6], d[7], d[8], d[9]])
    }
    pub fn set(offset: u64, length: u64) -> Self {
        let mut s = Self::default();
        s.data[0..5].copy_from_slice(&offset.to_be_bytes()[3..]);
        s.data[5..10].copy_from_slice(&length.to_be_bytes()[3..]);
        s
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CompressedBlockEntry {
    pub data: [u8; 12],
}

impl CompressedBlockEntry {
    pub fn offset(&self) -> u64 {
        u64::from_le_bytes([self.data[0], self.data[1], self.data[2], self.data[3], self.data[4], 0, 0, 0])
    }
    pub fn compressed_size(&self) -> u32 {
        u32::from_le_bytes([self.data[5], self.data[6], self.data[7], 0])
    }
    pub fn uncompressed_size(&self) -> u32 {
        u32::from_le_bytes([self.data[8], self.data[9], self.data[10], 0])
    }
    pub fn method_index(&self) -> u8 {
        self.data[11]
    }
    pub fn set(offset: u64, compressed: u32, uncompressed: u32, method_index: u8) -> Self {
        let mut s = Self::default();
        s.data[0..5].copy_from_slice(&offset.to_le_bytes()[0..5]);
        s.data[5..8].copy_from_slice(&compressed.to_le_bytes()[0..3]);
        s.data[8..11].copy_from_slice(&uncompressed.to_le_bytes()[0..3]);
        s.data[11] = method_index;
        s
    }
}

pub const CONTAINER_FLAG_COMPRESSED: u8 = 0b0001;
pub const CONTAINER_FLAG_ENCRYPTED: u8 = 0b0010;
pub const CONTAINER_FLAG_SIGNED: u8 = 0b0100;
pub const CONTAINER_FLAG_INDEXED: u8 = 0b1000;

const META_FLAG_COMPRESSED: u32 = 1;
const META_FLAG_MEMORY_MAPPED: u32 = 2;

#[derive(Debug, Default)]
pub struct DirectoryIndex {
    pub mount_point: String,
    /// file path components joined → chunk index in TOC
    pub files: Vec<(String, u32)>,
}

/// Parsed .utoc + ability to read chunks from the paired .ucas file(s).
pub struct Toc {
    pub version: TocVersion,
    pub container_id: u64,
    pub container_flags: u8,
    pub encryption_key_guid: [u8; 16],
    pub compression_block_size: u32,
    pub partition_size: u64,
    pub partition_count: u32,
    pub chunks: Vec<ChunkId>,
    pub offset_lengths: Vec<OffsetAndLength>,
    pub compression_blocks: Vec<CompressedBlockEntry>,
    pub compression_methods: Vec<Method>,
    pub chunk_metas: Vec<(u32)>, // flags per chunk
    pub chunk_hashes: Vec<[u8; 32]>,
    pub directory_index: DirectoryIndex,
    pub on_demand: Option<Vec<u8>>,
    /// chunk index → ucas file index (for multi-partition containers)
    cas_paths: Vec<PathBuf>,
}

fn is_new(version: TocVersion) -> bool {
    version > TocVersion::PerfectHash
}

pub(crate) fn is_new_version(version: TocVersion) -> bool {
    is_new(version)
}

impl Toc {
    /// Parse a .utoc. Handles both header layouts:
    /// - "new" (UE5.4+ / retoc): version u8 @16, header_size u32 @20
    /// - "old" (UE4.25-5.3): version u32 @16, header_size u32 @24
    /// Both headers total 0x90 bytes; chunk data starts at 0x90.
    pub fn read(mut utoc: impl Read, key: Option<&AesKey>) -> Result<Toc> {
        let header: [u8; 0x90] = utoc.read_bytes(0x90)?.try_into().unwrap();
        if header[0..16] != TOC_MAGIC {
            let found = u32::from_le_bytes(header[0..4].try_into().unwrap());
            return Err(Error::Magic { found });
        }
        let be = |off: usize| -> u32 { u32::from_le_bytes(header[off..off + 4].try_into().unwrap()) };
        let ble = |off: usize| -> u64 { u64::from_le_bytes(header[off..off + 8].try_into().unwrap()) };

        // layout detection
        let new_layout = be(20) == 0x90;
        let old_layout = be(24) == 0x90 && !new_layout;
        if !new_layout && !old_layout {
            return Err(Error::malformed("unrecognized utoc header layout"));
        }
        // field offsets per layout
        let (o_entries, o_bec, o_bes, o_mnc, o_mnl, o_cbs, o_dis, o_pc, o_cid, o_guid, o_flags, o_seeds, o_psize, o_wc) =
            if new_layout {
                (24, 28, 32, 36, 40, 44, 48, 52, 56, 64, 80, 84, 88, 96)
            } else {
                (28, 32, 36, 40, 44, 48, 52, 56, 60, 68, 84, 88, 92, 100)
            };
        let version_raw = if new_layout { header[16] as u32 } else { be(16) };
        let version = TocVersion::from_u32(version_raw)
            .ok_or_else(|| Error::unsupported(format!("utoc version {version_raw}")))?;
        let toc_entry_count = be(o_entries) as usize;
        let compressed_block_entry_count = be(o_bec) as usize;
        let compressed_block_entry_size = be(o_bes);
        if compressed_block_entry_size != 12 {
            return Err(Error::malformed(format!(
                "unexpected compressed block entry size {compressed_block_entry_size}"
            )));
        }
        let compression_method_count = be(o_mnc) as usize;
        let compression_method_length = be(o_mnl) as usize;
        let compression_block_size = be(o_cbs);
        let directory_index_size = be(o_dis) as usize;
        let partition_count = be(o_pc);
        let container_id = ble(o_cid);
        let encryption_key_guid: [u8; 16] = header[o_guid..o_guid + 16].try_into().unwrap();
        let container_flags = header[o_flags];
        let perfect_hash_seeds_count = be(o_seeds) as usize;
        let partition_size = ble(o_psize);
        let chunks_without_perfect_hash_count = be(o_wc) as usize;

        let new = is_new(version);

        let mut chunks = Vec::with_capacity(toc_entry_count);
        for _ in 0..toc_entry_count {
            let mut id = [0u8; 12];
            utoc.read_exact(&mut id)?;
            if let Some(ct) = ChunkType::from_raw(id[11], new) {
                id[11] = ct.to_raw(new);
            }
            chunks.push(ChunkId { id });
        }

        let mut offset_lengths = Vec::with_capacity(toc_entry_count);
        for _ in 0..toc_entry_count {
            let mut ol = OffsetAndLength::default();
            utoc.read_exact(&mut ol.data)?;
            offset_lengths.push(ol);
        }

        for _ in 0..perfect_hash_seeds_count {
            utoc.i32()?;
        }
        for _ in 0..chunks_without_perfect_hash_count {
            utoc.i32()?;
        }

        let mut compression_blocks = Vec::with_capacity(compressed_block_entry_count);
        for _ in 0..compressed_block_entry_count {
            let mut be_entry = CompressedBlockEntry::default();
            utoc.read_exact(&mut be_entry.data)?;
            compression_blocks.push(be_entry);
        }

        let mut compression_methods = Vec::with_capacity(compression_method_count);
        for _ in 0..compression_method_count {
            let name = utoc.read_bytes(compression_method_length)?;
            let s: String = name.iter().take_while(|&&b| b != 0).map(|&b| b as char).collect();
            let method = Method::from_name(&s)
                .ok_or_else(|| Error::unsupported(format!("unknown compression method {s:?}")))?;
            compression_methods.push(method);
        }

        if container_flags & CONTAINER_FLAG_SIGNED != 0 {
            let sig_size = utoc.u32()? as usize;
            let _toc_sig = utoc.read_bytes(sig_size)?;
            let _block_sig = utoc.read_bytes(sig_size)?;
            let _chunk_sigs = utoc.read_bytes(compressed_block_entry_count * 20)?;
        }

        let mut dir_index_buf = utoc.read_bytes(directory_index_size)?;
        if container_flags & CONTAINER_FLAG_ENCRYPTED != 0 {
            let key = key.ok_or(Error::EncryptedNoKey)?;
            key.decrypt_ecb_in_place(&mut dir_index_buf);
        }
        let directory_index = parse_directory_index(&dir_index_buf)?;

        let mut chunk_hashes = Vec::with_capacity(toc_entry_count);
        let mut chunk_metas = Vec::with_capacity(toc_entry_count);
        for _ in 0..toc_entry_count {
            if version >= TocVersion::ReplaceIoChunkHashWithIoHash {
                let mut h = [0u8; 32];
                utoc.read_exact(&mut h[0..20])?;
                let flags = utoc.u32()?;
                utoc.read_bytes(3)?;
                chunk_hashes.push(h);
                chunk_metas.push(flags);
            } else {
                let h = utoc.read_bytes(32)?.try_into().unwrap();
                let flags = utoc.u32()?;
                chunk_hashes.push(h);
                chunk_metas.push(flags);
            }
        }

        let on_demand = if version == TocVersion::OnDemandMetaData {
            let size = utoc.u64()? as usize;
            Some(utoc.read_bytes(size)?)
        } else {
            None
        };

        Ok(Toc {
            version,
            container_id,
            container_flags,
            encryption_key_guid,
            compression_block_size,
            partition_size,
            partition_count,
            chunks,
            offset_lengths,
            compression_blocks,
            compression_methods,
            chunk_metas,
            chunk_hashes,
            directory_index,
            on_demand,
            cas_paths: Vec::new(),
        })
    }

    /// Attach the .ucas file(s).
    pub fn open_cas(&mut self, ucas_path: &Path) -> Result<()> {
        if !ucas_path.exists() {
            return Err(Error::NotFound(ucas_path.display().to_string()));
        }
        self.cas_paths = vec![ucas_path.to_path_buf()];
        Ok(())
    }

    fn block_range(&self, toc_index: usize) -> Result<(usize, usize)> {
        let offset = self.offset_lengths[toc_index].offset();
        let size = self.offset_lengths[toc_index].length();
        let cbs = self.compression_block_size as u64;
        let first = (offset / cbs) as usize;
        let last = (((offset + size + cbs - 1) / cbs) - 1) as usize;
        Ok((first, last))
    }

    /// Read + decrypt + decompress a chunk by TOC index.
    pub fn read_chunk(&self, toc_index: usize, key: Option<&AesKey>) -> Result<Vec<u8>> {
        if self.cas_paths.is_empty() {
            return Err(Error::other("no .ucas attached"));
        }
        let offset = self.offset_lengths[toc_index].offset();
        let size = self.offset_lengths[toc_index].length() as usize;
        let (first, last) = self.block_range(toc_index)?;
        let encrypted = self.container_flags & CONTAINER_FLAG_ENCRYPTED != 0
            && self.chunk_metas[toc_index] & META_FLAG_COMPRESSED == 0;
        let compressed_meta = self.chunk_metas[toc_index] & META_FLAG_COMPRESSED != 0
            && self.container_flags & CONTAINER_FLAG_COMPRESSED != 0;

        let mut f = std::fs::File::open(&self.cas_paths[0])?;
        let mut out = vec![0u8; (size + 15) & !15];
        let mut cur = 0usize;

        if !compressed_meta {
            // single uncompressed run (possibly encrypted whole)
            f.seek(SeekFrom::Start(offset))?;
            if encrypted {
                let key = key.ok_or(Error::EncryptedNoKey)?;
                let read = (size + 15) & !15;
                let mut buf = vec![0u8; read];
                f.read_exact(&mut buf)?;
                key.decrypt_ecb_in_place(&mut buf);
                out[..size].copy_from_slice(&buf[..size]);
            } else {
                f.read_exact(&mut out[..size])?;
            }
            out.truncate(size);
            return Ok(out);
        }

        for block in &self.compression_blocks[first..=last] {
            let method_index = block.method_index() as usize;
            let comp_size = block.compressed_size() as usize;
            let uncomp_size = block.uncompressed_size() as usize;
            let dst = &mut out[cur..];
            let method = if method_index == 0 {
                None
            } else {
                Some(
                    self.compression_methods
                        .get(method_index - 1)
                        .copied()
                        .ok_or_else(|| Error::malformed("bad compression method index"))?,
                )
            };
            match method {
                None => {
                    f.seek(SeekFrom::Start(block.offset()))?;
                    if self.container_flags & CONTAINER_FLAG_ENCRYPTED != 0 {
                        let key = key.ok_or(Error::EncryptedNoKey)?;
                        let read = (uncomp_size + 15) & !15;
                        let mut buf = vec![0u8; read];
                        f.read_exact(&mut buf)?;
                        key.decrypt_ecb_in_place(&mut buf);
                        dst[..uncomp_size].copy_from_slice(&buf[..uncomp_size]);
                    } else {
                        f.read_exact(&mut dst[..uncomp_size])?;
                    }
                }
                Some(method) => {
                    f.seek(SeekFrom::Start(block.offset()))?;
                    if self.container_flags & CONTAINER_FLAG_ENCRYPTED != 0 {
                        let key = key.ok_or(Error::EncryptedNoKey)?;
                        let read = (comp_size + 15) & !15;
                        let mut buf = vec![0u8; read];
                        f.read_exact(&mut buf)?;
                        key.decrypt_ecb_in_place(&mut buf);
                        let tmp = &mut buf[..comp_size];
                        crate::compress::decompress(method, tmp, &mut dst[..uncomp_size])?;
                    } else {
                        let mut buf = vec![0u8; comp_size];
                        f.read_exact(&mut buf)?;
                        crate::compress::decompress(method, &buf, &mut dst[..uncomp_size])?;
                    }
                }
            }
            cur += uncomp_size;
        }
        out.truncate(size);
        Ok(out)
    }

    pub fn has_chunk(&self, id: &ChunkId) -> bool {
        self.chunks.iter().any(|c| c == id)
    }

    pub fn chunk_index(&self, id: &ChunkId) -> Option<usize> {
        self.chunks.iter().position(|c| c == id)
    }

    /// Read a chunk by ID.
    pub fn read_by_id(&self, id: &ChunkId, key: Option<&AesKey>) -> Result<Vec<u8>> {
        let idx = self
            .chunk_index(id)
            .ok_or_else(|| Error::NotFound(format!("chunk {id}")))?;
        self.read_chunk(idx, key)
    }
}

fn parse_directory_index(buf: &[u8]) -> Result<DirectoryIndex> {
    let mut idx = DirectoryIndex::default();
    if buf.is_empty() {
        return Ok(idx);
    }
    let mut r = std::io::Cursor::new(buf);
    idx.mount_point = r.fstring()?;

    let dir_count = r.u32()? as usize;
    #[derive(Default)]
    struct DirEntry {
        name: Option<u32>,
        first_child: Option<u32>,
        next_sibling: Option<u32>,
        first_file: Option<u32>,
    }
    let mut dirs = Vec::with_capacity(dir_count);
    for _ in 0..dir_count {
        let name = r.i32()?;
        let first_child = r.i32()?;
        let next_sibling = r.i32()?;
        let first_file = r.i32()?;
        dirs.push(DirEntry {
            name: (name >= 0).then_some(name as u32),
            first_child: (first_child >= 0).then_some(first_child as u32),
            next_sibling: (next_sibling >= 0).then_some(next_sibling as u32),
            first_file: (first_file >= 0).then_some(first_file as u32),
        });
    }

    let file_count = r.u32()? as usize;
    #[derive(Default)]
    struct FileEntry {
        name: u32,
        next: Option<u32>,
        user_data: u32,
    }
    let mut files = Vec::with_capacity(file_count);
    for _ in 0..file_count {
        let name = r.u32()?;
        let next = r.i32()?;
        let user_data = r.u32()?;
        files.push(FileEntry {
            name,
            next: (next >= 0).then_some(next as u32),
            user_data,
        });
    }

    let str_count = r.u32()? as usize;
    let mut strings = Vec::with_capacity(str_count);
    for _ in 0..str_count {
        strings.push(r.fstring()?);
    }

    // walk from root
    fn walk(
        dirs: &[DirEntry],
        files: &[FileEntry],
        strings: &[String],
        dir_idx: usize,
        prefix: &mut Vec<String>,
        out: &mut Vec<(String, u32)>,
    ) {
        let dir = &dirs[dir_idx];
        if let Some(n) = dir.name {
            prefix.push(strings.get(n as usize).cloned().unwrap_or_default());
        }
        let mut fi = dir.first_file;
        while let Some(i) = fi {
            let f = &files[i as usize];
            let name = strings.get(f.name as usize).cloned().unwrap_or_default();
            let mut path = prefix.join("/");
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(&name);
            out.push((path, f.user_data));
            fi = f.next;
        }
        let mut ci = dir.first_child;
        while let Some(i) = ci {
            walk(dirs, files, strings, i as usize, prefix, out);
            ci = dirs[i as usize].next_sibling;
        }
        if dir.name.is_some() {
            prefix.pop();
        }
    }

    if !dirs.is_empty() {
        let mut prefix = Vec::new();
        walk(&dirs, &files, &strings, 0, &mut prefix, &mut idx.files);
    }
    Ok(idx)
}

/// CityHash64 of the lowercase UTF-16 bytes of a package path → FPackageId.
pub fn package_id_from_name(name: &str) -> u64 {
    let bytes: Vec<u8> = name
        .to_ascii_lowercase()
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    cityhasher::hash(bytes)
}

/// Global name batch (LoaderGlobalNames / ScriptObjects era name table).
pub fn read_name_batch(buf: &[u8]) -> Result<Vec<String>> {
    let mut r = std::io::Cursor::new(buf);
    let num = r.u32()? as usize;
    if num == 0 {
        return Ok(vec![]);
    }
    let _num_string_bytes = r.u32()?;
    let _hash_version = r.u64()?;
    let _hash_bytes = r.read_bytes(num * 8)?;
    let mut lengths = Vec::with_capacity(num);
    for _ in 0..num {
        lengths.push(r.i16()?);
    }
    let mut names = Vec::with_capacity(num);
    for l in lengths {
        let len = if l < 0 { (i16::MIN as i32) - l as i32 } else { l as i32 };
        let data = r.read_bytes(len as usize)?;
        if l < 0 {
            let units: Vec<u16> = data
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
            names.push(String::from_utf16_lossy(&units[..end]));
        } else {
            let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
            names.push(String::from_utf8_lossy(&data[..end]).into_owned());
        }
    }
    Ok(names)
}

/// A mounted IOStore container set (a container + optional global container for names).
pub struct IoStoreReader {
    pub toc: Toc,
    key: Option<AesKey>,
    /// global names resolved from the "global" container (chunk 0/0 LoaderGlobalNames)
    global_names: Vec<String>,
    /// local (container) name map from PackageResource chunks, resolved lazily per package
    container_name: String,
    container_header_version_i32: i32,
}

impl IoStoreReader {
    pub fn open(utoc_path: &Path, key: Option<AesKey>) -> Result<IoStoreReader> {
        let utoc_file = std::fs::File::open(utoc_path)?;
        let mut toc = Toc::read(std::io::BufReader::new(utoc_file), key.as_ref())?;
        let ucas_path = utoc_path.with_extension("ucas");
        toc.open_cas(&ucas_path)?;
        let container_name = utoc_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        // try to parse container header eagerly to learn header version
        let mut container_header_version_i32: i32 = -1;
        let new = is_new(toc.version);
        let ct = if new { ChunkType::ContainerHeader } else { ChunkType::ContainerHeader };
        let header_id = ChunkId::new(0, 0, ct, new);
        if toc.chunk_index(&header_id).is_some() {
            if let Ok(data) = toc.read_by_id(&header_id, key.as_ref()) {
                if let Ok(header) = zen::ContainerHeader::parse(&data) {
                    container_header_version_i32 = header.version as i32;
                }
            }
        }
        Ok(IoStoreReader {
            toc,
            key,
            global_names: Vec::new(),
            container_name,
            container_header_version_i32,
        })
    }

    pub fn container_name(&self) -> &str {
        &self.container_name
    }

    /// True when this container's payloads are AES-encrypted (container flag).
    pub fn is_encrypted(&self) -> bool {
        self.toc.container_flags & CONTAINER_FLAG_ENCRYPTED != 0
    }

    /// The encryption key GUID recorded in the TOC (empty when absent).
    pub fn encryption_key_guid(&self) -> String {
        let g = self.toc.encryption_key_guid;
        format!("{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            g[0], g[1], g[2], g[3], g[4], g[5], g[6], g[7], g[8], g[9], g[10], g[11], g[12], g[13], g[14], g[15])
    }

    pub fn has_global_names(&self) -> bool {
        !self.global_names.is_empty()
    }

    /// Load global names from a separate "global" container (UE4.25+ zen / UE5).
    pub fn load_global_names(&mut self, global: &IoStoreReader) -> Result<()> {
        let new = is_new(global.toc.version);
        let type_ = if new { ChunkType::ScriptObjects } else { ChunkType::LoaderGlobalNames };
        let id = ChunkId::new(0, 0, type_, new);
        let data = global.toc.read_by_id(&id, global.key.as_ref())?;
        self.global_names = read_name_batch(&data)?;
        Ok(())
    }

    pub fn resolve_global_name(&self, index: u32) -> Option<String> {
        self.global_names.get(index as usize).cloned()
    }

    pub fn container_header_version(&self) -> i32 {
        self.container_header_version_i32
    }

    /// Resolve an FMappedName (index_and_type, number) to "Name" or "Name_N".
    pub fn resolve_name(&self, index_and_type: u32, number: u32, local_names: &[String]) -> Option<String> {
        const INDEX_BITS: u32 = 30;
        let index = index_and_type & ((1 << INDEX_BITS) - 1);
        let ty = (index_and_type & !((1 << INDEX_BITS) - 1)) >> INDEX_BITS;
        // 0 = global, 1 = container/local
        let table = match ty {
            0 => &self.global_names,
            _ => return local_names.get(index as usize).cloned(),
        };
        let name = table.get(index as usize)?;
        Some(if number == 0 {
            name.clone()
        } else {
            format!("{name}_{number}")
        })
    }

    pub fn read_chunk(&self, idx: usize) -> Result<Vec<u8>> {
        self.toc.read_chunk(idx, self.key.as_ref())
    }

    pub fn read_chunk_id(&self, id: &ChunkId) -> Result<Vec<u8>> {
        self.toc.read_by_id(id, self.key.as_ref())
    }

    /// Files (from the directory index) with their resolved chunk ids.
    pub fn files(&self) -> Vec<(String, ChunkId, u64)> {
        let new = is_new(self.toc.version);
        self.toc
            .directory_index
            .files
            .iter()
            .filter_map(|(path, user_data)| {
                let idx = *user_data as usize;
                let chunk = self.toc.chunks.get(idx)?;
                let size = self.toc.offset_lengths.get(idx).map(|o| o.length()).unwrap_or(0);
                let mount = self.toc.directory_index.mount_point.trim_end_matches('/');
                let full = format!("{mount}/{}", path.trim_start_matches('/'));
                let _ = new;
                Some((normalize(&full), *chunk, size))
            })
            .collect()
    }

    /// All chunks with sizes (fallback listing when no directory index exists).
    pub fn all_chunks(&self) -> Vec<(ChunkId, u64)> {
        self.toc
            .chunks
            .iter()
            .zip(&self.toc.offset_lengths)
            .map(|(c, o)| (*c, o.length()))
            .collect()
    }
}

fn normalize(path: &str) -> String {
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

/// Map of package id → export bundle chunk (for package-level access).
pub fn build_package_map(toc: &Toc) -> HashMap<u64, Vec<ChunkId>> {
    let mut map: HashMap<u64, Vec<ChunkId>> = HashMap::new();
    for c in &toc.chunks {
        map.entry(c.package_id()).or_default().push(*c);
    }
    map
}
