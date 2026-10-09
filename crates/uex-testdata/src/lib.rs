//! Synthetic Unreal archive builders + sample asset payloads used by the test suite.
//! These produce byte-exact PAK (v8B/v10/v11, compressed/encrypted) and IOStore
//! (TOC v2/v5) containers so the full read path can be validated end-to-end.

use std::io::Write;

pub const TEST_AES_KEY: &str = "0x11223344556677889900AABBCCDDEEFF00112233445566778899AABBCCDDEEFF";

pub struct SampleFile {
    pub path: String,
    pub data: Vec<u8>,
}

impl SampleFile {
    pub fn new(path: &str, data: Vec<u8>) -> Self {
        Self { path: path.into(), data }
    }
}

/// Deterministic pseudo-random data (half compressible runs, half noise).
pub fn pseudo_data(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(1);
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.push(if i % 1024 < 512 { 0 } else { (state >> 24) as u8 });
    }
    out
}

/// BC1 (DXT1) compressed texture payload.
pub fn bc1_texture_data(width: u32, height: u32) -> Vec<u8> {
    let (bx, by) = ((width + 3) / 4, (height + 3) / 4);
    let mut out = Vec::with_capacity((bx * by * 8) as usize);
    for i in 0..(bx * by) {
        let r = (i * 8 % 255) as u16;
        let g = (i * 13 % 255) as u16;
        let b = (i * 29 % 255) as u16;
        let c0: u16 = (r >> 3) << 11 | (g >> 2) << 5 | (b >> 3);
        let c1: u16 = c0 / 2;
        out.extend_from_slice(&c0.to_le_bytes());
        out.extend_from_slice(&c1.to_le_bytes());
        out.extend_from_slice(&0x8888888888888888u64.to_le_bytes());
    }
    out
}

/// A fully self-consistent classic package (uasset+uexp) with one Texture2D export
/// and an inline DXT1 mip. Versioned summary (UE5 field layout, UE4.27+ versions).
pub fn build_classic_texture_package(
    name: &str,
    width: u32,
    height: u32,
) -> (Vec<u8>, Vec<u8>) {
    let mut names: Vec<String> = Vec::new();
    fn ni(names: &mut Vec<String>, s: &str) -> u32 {
        if let Some(i) = names.iter().position(|n| n == s) {
            i as u32
        } else {
            names.push(s.to_string());
            (names.len() - 1) as u32
        }
    }
    let idx_engine_pkg = ni(&mut names, "/Script/Engine");
    let idx_texture2d = ni(&mut names, "Texture2D");
    let idx_name = ni(&mut names, name);
    let idx_format = ni(&mut names, "Format");
    let idx_pf = ni(&mut names, "PF_DXT1");
    let idx_none = ni(&mut names, "None");
    let idx_enum_prop = ni(&mut names, "EnumProperty");

    let texture_data = bc1_texture_data(width, height);

    // ---- uexp ----
    let mut exp: Vec<u8> = Vec::new();
    // tag: Format (EnumProperty, size 8, value PF_DXT1)
    exp.extend_from_slice(&idx_format.to_le_bytes());
    exp.extend_from_slice(&0i32.to_le_bytes());
    exp.extend_from_slice(&idx_enum_prop.to_le_bytes());
    exp.extend_from_slice(&0i32.to_le_bytes());
    exp.extend_from_slice(&8i64.to_le_bytes());
    exp.extend_from_slice(&0u32.to_le_bytes()); // has guid
    exp.extend_from_slice(&idx_pf.to_le_bytes());
    exp.extend_from_slice(&0i32.to_le_bytes());
    // None terminator
    exp.extend_from_slice(&idx_none.to_le_bytes());
    exp.extend_from_slice(&0i32.to_le_bytes());
    // FTexturePlatformData
    exp.extend_from_slice(&(width as i32).to_le_bytes());
    exp.extend_from_slice(&(height as i32).to_le_bytes());
    exp.extend_from_slice(&((5i32) << 1).to_le_bytes()); // packed: fmt<<1
    exp.extend_from_slice(&0i32.to_le_bytes()); // first mip
    exp.extend_from_slice(&1i32.to_le_bytes()); // num mips
    // mip
    exp.extend_from_slice(&(width as i32).to_le_bytes());
    exp.extend_from_slice(&(height as i32).to_le_bytes());
    exp.extend_from_slice(&1i32.to_le_bytes());
    exp.extend_from_slice(&4u32.to_le_bytes()); // ForceInlinePayload
    exp.extend_from_slice(&(texture_data.len() as i64).to_le_bytes());
    exp.extend_from_slice(&(texture_data.len() as i64).to_le_bytes());
    exp.extend_from_slice(&0i64.to_le_bytes());
    exp.extend_from_slice(&texture_data);

    // ---- uasset ----
    let mut asset: Vec<u8> = Vec::new();
    macro_rules! push {
        ($v:expr) => {
            asset.extend_from_slice($v)
        };
    }
    push!(&0x9E2A83C1u32.to_le_bytes());
    push!(&(-8i32).to_le_bytes());
    push!(&864i32.to_le_bytes());
    push!(&522i32.to_le_bytes());
    push!(&1008i32.to_le_bytes());
    push!(&0i32.to_le_bytes()); // licensee
    push!(&0u32.to_le_bytes()); // custom versions
    let total_header_pos = asset.len();
    push!(&0i32.to_le_bytes());
    // folder name "../../../TestGame/Content/"
    push!(&27i32.to_le_bytes());
    push!(b"../../../TestGame/Content/\0");
    push!(&0x8000_0000u32.to_le_bytes()); // package flags: FilterEditorOnly
    let name_count_pos = asset.len();
    push!(&0i32.to_le_bytes());
    let name_offset_pos = asset.len();
    push!(&0i32.to_le_bytes());
    push!(&0i32.to_le_bytes()); // gatherable count
    push!(&0i32.to_le_bytes()); // gatherable offset
    let export_count_pos = asset.len();
    push!(&0i32.to_le_bytes());
    let export_offset_pos = asset.len();
    push!(&0i32.to_le_bytes());
    let import_count_pos = asset.len();
    push!(&0i32.to_le_bytes());
    let import_offset_pos = asset.len();
    push!(&0i32.to_le_bytes());
    push!(&0i32.to_le_bytes()); // soft paths count
    push!(&0i32.to_le_bytes()); // soft paths offset
    push!(&0i32.to_le_bytes()); // thumbnail offset
    push!(&[7u8; 16]); // guid
    push!(&0i32.to_le_bytes()); // generation count
    for s in ["5.3.2.0", "5.3.2.0"] {
        push!(&((s.len() + 1) as i32).to_le_bytes());
        push!(s.as_bytes());
        push!(&[0u8][..]);
    }
    push!(&0u32.to_le_bytes()); // compression flags
    push!(&0u32.to_le_bytes()); // package source
    push!(&0i32.to_le_bytes()); // asset registry offset
    push!(&0i64.to_le_bytes()); // bulk data start offset
    push!(&0i32.to_le_bytes()); // package owner (empty FString)
    push!(&0i32.to_le_bytes()); // world tile info offset
    push!(&0i32.to_le_bytes()); // chunk ids count
    push!(&0i32.to_le_bytes()); // preload dep count
    push!(&0i32.to_le_bytes()); // preload dep offset
    push!(&4i32.to_le_bytes()); // names referenced from export data
    push!(&0i32.to_le_bytes()); // payload toc offset
    push!(&0i32.to_le_bytes()); // data resource offset

    // name map (no count field here — NameCount lives in the summary)
    let name_offset = asset.len() as i32;
    for n in &names {
        push!(&((n.len() + 1) as i32).to_le_bytes());
        push!(n.as_bytes());
        push!(&[0u8][..]);
        push!(&0i16.to_le_bytes());
    }

    // imports: 1 import → /Script/Engine.Texture2D (count lives in the summary)
    let import_offset = asset.len() as i32;
    push!(&idx_engine_pkg.to_le_bytes());
    push!(&0i32.to_le_bytes());
    push!(&idx_texture2d.to_le_bytes());
    push!(&0i32.to_le_bytes());
    push!(&0i32.to_le_bytes()); // outer
    push!(&idx_texture2d.to_le_bytes());
    push!(&0i32.to_le_bytes());
    push!(&0u32.to_le_bytes()); // optional (UE5 >= 1003)

    // exports: 1 export, class = first import → -1 (count lives in the summary)
    let export_offset = asset.len() as i32;
    push!(&(-1i32).to_le_bytes()); // class index
    push!(&0i32.to_le_bytes()); // super
    push!(&0i32.to_le_bytes()); // outer
    push!(&idx_name.to_le_bytes());
    push!(&0i32.to_le_bytes()); // name number
    push!(&0u32.to_le_bytes()); // save
    let serial_size_pos = asset.len();
    push!(&0i64.to_le_bytes());
    let serial_offset_pos = asset.len();
    push!(&0i64.to_le_bytes());
    push!(&0u32.to_le_bytes()); // forced
    push!(&0u32.to_le_bytes()); // not for client
    push!(&0u32.to_le_bytes()); // not for server
    push!(&[0u8; 16]); // guid
    push!(&0u32.to_le_bytes()); // package flags
    push!(&0u32.to_le_bytes()); // not always loaded
    push!(&1u32.to_le_bytes()); // is asset
    push!(&0i32.to_le_bytes()); // first dep
    push!(&0u32.to_le_bytes());
    push!(&0u32.to_le_bytes());
    push!(&0u32.to_le_bytes());
    push!(&0u32.to_le_bytes());

    let total_header_size = asset.len() as i32;

    asset[total_header_pos..total_header_pos + 4].copy_from_slice(&total_header_size.to_le_bytes());
    asset[name_count_pos..name_count_pos + 4].copy_from_slice(&(names.len() as i32).to_le_bytes());
    asset[name_offset_pos..name_offset_pos + 4].copy_from_slice(&name_offset.to_le_bytes());
    asset[export_count_pos..export_count_pos + 4].copy_from_slice(&1i32.to_le_bytes());
    asset[export_offset_pos..export_offset_pos + 4].copy_from_slice(&export_offset.to_le_bytes());
    asset[import_count_pos..import_count_pos + 4].copy_from_slice(&1i32.to_le_bytes());
    asset[import_offset_pos..import_offset_pos + 4].copy_from_slice(&import_offset.to_le_bytes());
    let serial_offset = 0i64;
    asset[serial_size_pos..serial_size_pos + 8].copy_from_slice(&(exp.len() as i64).to_le_bytes());
    asset[serial_offset_pos..serial_offset_pos + 8].copy_from_slice(&serial_offset.to_le_bytes());

    (asset, exp)
}

/// Build a full synthetic .pak using uex-core's writer.
pub fn build_pak_v11(
    files: &[SampleFile],
    compress: bool,
    key: Option<&str>,
) -> anyhow::Result<Vec<u8>> {
    use uex_core::crypto::AesKey;
    use uex_core::pak::{PakWriter, Version};
    let aes = key.and_then(AesKey::parse);
    let methods = if compress {
        vec![uex_core::compress::Method::Zlib, uex_core::compress::Method::Lz4]
    } else {
        vec![]
    };
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = PakWriter::new(
        cursor,
        Version::V11,
        "../../../TestGame/Content/",
        aes,
        methods,
        key.is_some(),
    );
    for f in files {
        writer.write_file(&f.path, &f.data)?;
    }
    let out = writer.finalize()?;
    Ok(out.into_inner())
}

// ---- IOStore builder --------------------------------------------------------

use uex_core::iostore::{
    CompressedBlockEntry, ChunkId, ChunkType, OffsetAndLength, TocVersion, CONTAINER_FLAG_COMPRESSED,
    CONTAINER_FLAG_INDEXED,
};

pub struct CasBuilder {
    pub chunks: Vec<ChunkId>,
    pub offsets: Vec<OffsetAndLength>,
    pub blocks: Vec<CompressedBlockEntry>,
    pub metas: Vec<u32>,
    pub hashes: Vec<[u8; 32]>,
    pub data: Vec<u8>,
    files: Vec<(String, u32)>,
}

impl CasBuilder {
    pub fn new() -> Self {
        Self {
            chunks: Vec::new(),
            offsets: Vec::new(),
            blocks: Vec::new(),
            metas: Vec::new(),
            hashes: Vec::new(),
            data: Vec::new(),
            files: Vec::new(),
        }
    }

    /// Add an uncompressed chunk; returns its index.
    pub fn add_chunk(&mut self, package_id: u64, chunk_index: u16, chunk_type: ChunkType, data: &[u8], is_new: bool) -> u32 {
        let id = ChunkId::new(package_id, chunk_index, chunk_type, is_new);
        let idx = self.chunks.len() as u32;
        let offset = self.data.len() as u64;
        self.data.extend_from_slice(data);
        self.chunks.push(id);
        self.offsets.push(OffsetAndLength::set(offset, data.len() as u64));
        // single uncompressed block
        self.blocks.push(CompressedBlockEntry::set(offset, data.len() as u32, data.len() as u32, 0));
        self.metas.push(0);
        self.hashes.push([0u8; 32]);
        idx
    }

    pub fn add_file(&mut self, path: &str, chunk_idx: u32) {
        self.files.push((path.to_string(), chunk_idx));
    }

    /// Serialize a .utoc (v5, new chunk types) + .ucas pair.
    /// Serialize a .utoc (new layout: version u8 @16, header_size @20, 0x90 total) + .ucas pair.
    pub fn build(self) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
        let mut utoc: Vec<u8> = Vec::new();
        let mut w = &mut utoc;
        w.write_all(b"-==--==--==--==-")?;
        w.write_all(&[TocVersion::PerfectHashWithOverflow as u8])?; // version u8
        w.write_all(&[0u8])?; // reserved0
        w.write_all(&[0u8, 0u8])?; // reserved1 u16
        w.write_all(&0x90u32.to_le_bytes())?; // toc header size @20
        w.write_all(&(self.chunks.len() as u32).to_le_bytes())?;
        w.write_all(&(self.blocks.len() as u32).to_le_bytes())?;
        w.write_all(&12u32.to_le_bytes())?;
        w.write_all(&0u32.to_le_bytes())?; // compression method count
        w.write_all(&32u32.to_le_bytes())?; // method name length
        w.write_all(&0x10000u32.to_le_bytes())?; // compression block size
        let dir_index = self.build_directory_index();
        w.write_all(&(dir_index.len() as u32).to_le_bytes())?;
        w.write_all(&1u32.to_le_bytes())?; // partition count
        w.write_all(&0x1234_5678_9ABC_DEF0u64.to_le_bytes())?; // container id
        w.write_all(&[0u8; 16])?; // encryption key guid
        w.write_all(&[CONTAINER_FLAG_COMPRESSED | CONTAINER_FLAG_INDEXED])?; // flags u8
        w.write_all(&[0u8])?; // reserved3 u8
        w.write_all(&[0u8, 0u8])?; // reserved4 u16
        w.write_all(&0u32.to_le_bytes())?; // perfect hash seeds count
        w.write_all(&0u64.to_le_bytes())?; // partition size
        w.write_all(&0u32.to_le_bytes())?; // chunks without perfect hash
        w.write_all(&0u32.to_le_bytes())?; // reserved7
        w.write_all(&[0u8; 40])?; // reserved8 [u64;5]
        debug_assert_eq!(w.len(), 0x90);

        for c in &self.chunks {
            w.write_all(&c.id)?;
        }
        for o in &self.offsets {
            w.write_all(&o.data)?;
        }
        for b in &self.blocks {
            w.write_all(&b.data)?;
        }
        w.write_all(&dir_index)?;
        // metas: version < ReplaceIoChunkHashWithIoHash → full 32-byte hash + flags u32
        for h in &self.hashes {
            w.write_all(h)?;
        }
        for m in &self.metas {
            w.write_all(&m.to_le_bytes())?;
        }
        Ok((utoc, self.data))
    }

    fn build_directory_index(&self) -> Vec<u8> {
        // FIoDirectoryIndexResource: mount point, dirs, files, strings
        let mut out: Vec<u8> = Vec::new();
        // mount point "../../../TestGame/Content/"
        let mount = "../../../TestGame/Content/";
        out.extend_from_slice(&((mount.len() + 1) as i32).to_le_bytes());
        out.extend_from_slice(mount.as_bytes());
        out.push(0);
        // dirs: single root
        out.extend_from_slice(&1u32.to_le_bytes());
        // root dir entry: name=None(-1), first_child=-1, next_sibling=-1, first_file=0
        out.extend_from_slice(&(-1i32).to_le_bytes());
        out.extend_from_slice(&(-1i32).to_le_bytes());
        out.extend_from_slice(&(-1i32).to_le_bytes());
        if self.files.is_empty() {
            out.extend_from_slice(&(-1i32).to_le_bytes());
        } else {
            out.extend_from_slice(&0i32.to_le_bytes());
        }
        // files
        out.extend_from_slice(&(self.files.len() as u32).to_le_bytes());
        for (i, (_, user_data)) in self.files.iter().enumerate() {
            let name_idx = i as u32;
            out.extend_from_slice(&name_idx.to_le_bytes());
            if i + 1 < self.files.len() {
                out.extend_from_slice(&((i + 1) as i32).to_le_bytes());
            } else {
                out.extend_from_slice(&(-1i32).to_le_bytes());
            }
            out.extend_from_slice(&user_data.to_le_bytes());
        }
        // strings: one per file (file names only, path relative)
        out.extend_from_slice(&(self.files.len() as u32).to_le_bytes());
        for (path, _) in &self.files {
            out.extend_from_slice(&((path.len() + 1) as i32).to_le_bytes());
            out.extend_from_slice(path.as_bytes());
            out.push(0);
        }
        out
    }
}

/// Build a sample zen package (ExportBundleData chunk) with one Texture2D export.
pub fn build_zen_texture_package(name: &str, width: u32, height: u32) -> (u64, Vec<u8>) {
    let package_name = format!("/Game/Props/{name}");
    let package_id = uex_core::iostore::package_id_from_name(&package_name);

    let mut local_names: Vec<String> = Vec::new();
    fn ni(names: &mut Vec<String>, s: &str) -> u32 {
        if let Some(i) = names.iter().position(|n| n == s) {
            i as u32
        } else {
            names.push(s.to_string());
            (names.len() - 1) as u32
        }
    }
    let idx_name = ni(&mut local_names, name);
    let idx_texture2d = ni(&mut local_names, "Texture2D");
    let idx_engine_pkg = ni(&mut local_names, "/Script/Engine");

    let texture_data = bc1_texture_data(width, height);

    // export payload: (zen packages are unversioned → no property tags) platform data only
    let mut export_payload: Vec<u8> = Vec::new();
    export_payload.extend_from_slice(&(width as i32).to_le_bytes());
    export_payload.extend_from_slice(&(height as i32).to_le_bytes());
    export_payload.extend_from_slice(&((5i32) << 1).to_le_bytes());
    export_payload.extend_from_slice(&0i32.to_le_bytes());
    export_payload.extend_from_slice(&1i32.to_le_bytes());
    export_payload.extend_from_slice(&(width as i32).to_le_bytes());
    export_payload.extend_from_slice(&(height as i32).to_le_bytes());
    export_payload.extend_from_slice(&1i32.to_le_bytes());
    export_payload.extend_from_slice(&4u32.to_le_bytes());
    export_payload.extend_from_slice(&(texture_data.len() as i64).to_le_bytes());
    export_payload.extend_from_slice(&(texture_data.len() as i64).to_le_bytes());
    export_payload.extend_from_slice(&0i64.to_le_bytes());
    export_payload.extend_from_slice(&texture_data);

    // Build package header
    let mut pkg: Vec<u8> = Vec::new();
    macro_rules! zp {
        ($v:expr) => {
            pkg.extend_from_slice($v)
        };
    }
    // summary (container header v2 = OptionalSegmentPackages)
    zp!(&1u32.to_le_bytes()); // has_versioning_info = 1
    let header_size_pos = pkg.len();
    zp!(&0u32.to_le_bytes()); // header size (patched)
    // name mapped: local, index idx_name, number 0
    zp!(&idx_name.to_le_bytes()); // index_and_type: type 0 = Package
    zp!(&0u32.to_le_bytes());
    zp!(&0u32.to_le_bytes()); // package flags
    zp!(&0u32.to_le_bytes()); // cooked header size
    zp!(&0i32.to_le_bytes()); // imported public export hashes offset
    let import_map_off_pos = pkg.len();
    zp!(&0i32.to_le_bytes());
    let export_map_off_pos = pkg.len();
    zp!(&0i32.to_le_bytes());
    let export_bundle_off_pos = pkg.len();
    zp!(&0i32.to_le_bytes());
    // chv=2 (< NoExportInfo): single graph_data_offset field; filled after bundle entries below
    let graph_data_off_pos = pkg.len();
    zp!(&0i32.to_le_bytes());
    // versioning info
    zp!(&3u32.to_le_bytes()); // zen package version (ExportDependencies)
    zp!(&522i32.to_le_bytes()); // ue4
    zp!(&1008i32.to_le_bytes()); // ue5
    zp!(&0i32.to_le_bytes()); // licensee
    zp!(&0u32.to_le_bytes()); // custom versions count
    // local name map
    let names_blob_pos = pkg.len();
    zp!(&(local_names.len() as u32).to_le_bytes());
    let mut name_bytes = 0u32;
    for n in &local_names {
        name_bytes += n.len() as u32;
    }
    zp!(&name_bytes.to_le_bytes());
    zp!(&0xC164_0000u64.to_le_bytes()); // hash version
    for _ in &local_names {
        zp!(&0u64.to_le_bytes()); // hashes
    }
    for n in &local_names {
        zp!(&((n.len() as i16) as u16).to_be_bytes().to_vec());
    }
    for n in &local_names {
        zp!(n.as_bytes());
    }
    let names_end = pkg.len();

    // import map (0 imports)
    let import_map_off = names_end as i32;
    // export map (1 export)
    let export_map_off = import_map_off;
    let cooked_offset_pos = pkg.len();
    zp!(&0u64.to_le_bytes()); // cooked serial offset (patched to header_size below)
    let exp_size = export_payload.len() as u64;
    zp!(&exp_size.to_le_bytes());
    zp!(&idx_name.to_le_bytes()); // object name
    zp!(&0u32.to_le_bytes());
    zp!(&[0xFFu8; 8]); // outer = null index
    // class index: package import (0) pointing to import map — use export 0? class import idx 0
    // package import: (pkg_idx << 32 | hash_idx); with 0 imports we store a global-style null
    zp!(&u64::from(idx_texture2d).to_le_bytes().iter().copied().chain(std::iter::repeat(0)).take(8).collect::<Vec<u8>>());
    // ^ placeholder patched below: class index should point to import map entry (we have none),
    // so instead we write package-import style value referencing imported package 0/hash 0.
    let class_pos = pkg.len() - 8;
    // write: package import (imported_package_index=0, hash=0)
    pkg[class_pos..class_pos + 8].copy_from_slice(&0u64.to_le_bytes());
    zp!(&[0xFFu8; 8]); // super
    zp!(&[0xFFu8; 8]); // template
    zp!(&1u64.to_le_bytes()); // public export hash
    zp!(&0u32.to_le_bytes()); // object flags
    zp!(&[0u8, 0, 0, 0]); // filter flags + padding
    let export_bundle_off = pkg.len() as i32;
    // export bundle entries: create + serialize
    zp!(&0u32.to_le_bytes()); // local export index
    zp!(&0u32.to_le_bytes()); // Create
    zp!(&0u32.to_le_bytes());
    zp!(&1u32.to_le_bytes()); // Serialize
    let graph_data_off = pkg.len() as i32;
    pkg[graph_data_off_pos..graph_data_off_pos + 4].copy_from_slice(&graph_data_off.to_le_bytes());
    let header_size = pkg.len() as u32;
    // exports live after the header within the chunk
    pkg[cooked_offset_pos..cooked_offset_pos + 8].copy_from_slice(&(header_size as u64).to_le_bytes());

    pkg[header_size_pos..header_size_pos + 4].copy_from_slice(&header_size.to_le_bytes());
    pkg[import_map_off_pos..import_map_off_pos + 4].copy_from_slice(&import_map_off.to_le_bytes());
    pkg[export_map_off_pos..export_map_off_pos + 4].copy_from_slice(&export_map_off.to_le_bytes());
    pkg[export_bundle_off_pos..export_bundle_off_pos + 4].copy_from_slice(&export_bundle_off.to_le_bytes());
    let _ = idx_engine_pkg;

    // exports data follows header
    pkg.extend_from_slice(&export_payload);

    (package_id, pkg)
}


/// A cube static mesh (24 verts, 12 tris) serialized as cooked FStaticMeshLODResources
/// (UE4.22+ layout) inside a classic package with a StaticMesh export.
pub fn build_classic_static_mesh_package(name: &str) -> (Vec<u8>, Vec<u8>) {
    let mut names: Vec<String> = Vec::new();
    fn ni(names: &mut Vec<String>, s: &str) -> u32 {
        if let Some(i) = names.iter().position(|n| n == s) {
            i as u32
        } else {
            names.push(s.to_string());
            (names.len() - 1) as u32
        }
    }
    let idx_engine_pkg = ni(&mut names, "/Script/Engine");
    let idx_static_mesh = ni(&mut names, "StaticMesh");
    let idx_name = ni(&mut names, name);

    // build the cube mesh payload
    let mut exp: Vec<u8> = Vec::new();
    // 1 section
    exp.extend_from_slice(&1i32.to_le_bytes());
    // section header: material, first index, num triangles, min vertex, max vertex, bEnableCollision
    exp.extend_from_slice(&0i32.to_le_bytes()); // material index
    exp.extend_from_slice(&0u32.to_le_bytes()); // first index
    exp.extend_from_slice(&12i32.to_le_bytes()); // 12 triangles
    exp.extend_from_slice(&0u32.to_le_bytes()); // min vertex
    exp.extend_from_slice(&23u32.to_le_bytes()); // max vertex
    exp.extend_from_slice(&1u32.to_le_bytes()); // bEnableCollision

    // position vertex buffer
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let s = 1.0f32;
    for &sz in &[s, -s] {
        for &sy in &[s, -s] {
            for &sx in &[s, -s] {
                positions.push([sx, sy, sz]);
            }
        }
    }
    // corner permutation to make a proper cube (24 verts)
    let corners: [[f32; 3]; 8] = [
        [-s, -s, -s], [s, -s, -s], [-s, s, -s], [s, s, -s],
        [-s, -s, s], [s, -s, s], [-s, s, s], [s, s, s],
    ];
    positions.clear();
    // 4 verts per face, 6 faces
    let faces: [[usize; 4]; 6] = [
        [0, 1, 3, 2], // -Z
        [4, 5, 7, 6], // +Z
        [0, 1, 5, 4], // -Y
        [2, 3, 7, 6], // +Y
        [0, 2, 6, 4], // -X
        [1, 3, 7, 5], // +X
    ];
    for f in &faces {
        for &c in f {
            positions.push(corners[c]);
        }
    }

    exp.extend_from_slice(&(positions.len() as u32).to_le_bytes());
    exp.extend_from_slice(&12u32.to_le_bytes()); // stride
    for p in &positions {
        exp.extend_from_slice(&p[0].to_le_bytes());
        exp.extend_from_slice(&p[1].to_le_bytes());
        exp.extend_from_slice(&p[2].to_le_bytes());
    }

    // static mesh vertex buffer: bUseHighPrecisionTangentBasis, bUseFullPrecisionUVs, numTexCoords, numVertices
    exp.extend_from_slice(&0u32.to_le_bytes()); // high prec tangents
    exp.extend_from_slice(&1u32.to_le_bytes()); // full precision uvs
    exp.extend_from_slice(&1u32.to_le_bytes()); // 1 uv channel
    exp.extend_from_slice(&(positions.len() as u32).to_le_bytes());
    for i in 0..positions.len() {
        let v = i as u32;
        let _ = v;
        // tangent + normal packed normals (up)
        exp.extend_from_slice(&0x00008080u32.to_le_bytes()); // tangent (x+)
        exp.extend_from_slice(&0x00008080u32.to_le_bytes()); // normal (z+)
        exp.extend_from_slice(&0.0f32.to_le_bytes());
        exp.extend_from_slice(&0.0f32.to_le_bytes());
    }

    // color buffer: not present
    exp.extend_from_slice(&0u32.to_le_bytes());

    // index buffer
    let mut indices: Vec<u32> = Vec::new();
    for f in 0..6 {
        let base = (f * 4) as u32;
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    exp.extend_from_slice(&(indices.len() as u32).to_le_bytes());
    exp.extend_from_slice(&0u32.to_le_bytes()); // 16-bit
    for i in &indices {
        exp.extend_from_slice(&(*i as u16).to_le_bytes());
    }

    // build asset (same structure as texture package but StaticMesh class)
    let mut asset: Vec<u8> = Vec::new();
    macro_rules! push {
        ($v:expr) => {
            asset.extend_from_slice($v)
        };
    }
    push!(&0x9E2A83C1u32.to_le_bytes());
    push!(&(-8i32).to_le_bytes());
    push!(&864i32.to_le_bytes());
    push!(&522i32.to_le_bytes());
    push!(&1008i32.to_le_bytes());
    push!(&0i32.to_le_bytes());
    push!(&0u32.to_le_bytes());
    let total_header_pos = asset.len();
    push!(&0i32.to_le_bytes());
    push!(&27i32.to_le_bytes());
    push!(b"../../../TestGame/Content/\0");
    push!(&0x8000_0000u32.to_le_bytes());
    let name_count_pos = asset.len();
    push!(&0i32.to_le_bytes());
    let name_offset_pos = asset.len();
    push!(&0i32.to_le_bytes());
    push!(&0i32.to_le_bytes()); // gatherable
    push!(&0i32.to_le_bytes());
    let export_count_pos = asset.len();
    push!(&0i32.to_le_bytes());
    let export_offset_pos = asset.len();
    push!(&0i32.to_le_bytes());
    let import_count_pos = asset.len();
    push!(&0i32.to_le_bytes());
    let import_offset_pos = asset.len();
    push!(&0i32.to_le_bytes());
    push!(&0i32.to_le_bytes()); // soft paths
    push!(&0i32.to_le_bytes());
    push!(&0i32.to_le_bytes()); // thumbnail
    push!(&[7u8; 16]);
    push!(&0i32.to_le_bytes());
    for s in ["5.3.2.0", "5.3.2.0"] {
        push!(&((s.len() + 1) as i32).to_le_bytes());
        push!(s.as_bytes());
        push!(&[0u8][..]);
    }
    push!(&0u32.to_le_bytes()); // compression flags
    push!(&0u32.to_le_bytes()); // package source
    push!(&0i32.to_le_bytes()); // asset registry
    push!(&0i64.to_le_bytes()); // bulk start
    push!(&0i32.to_le_bytes()); // package owner
    push!(&0i32.to_le_bytes()); // world tile
    push!(&0i32.to_le_bytes()); // chunk ids
    push!(&0i32.to_le_bytes()); // preload count
    push!(&0i32.to_le_bytes()); // preload offset
    push!(&4i32.to_le_bytes()); // names referenced
    push!(&0i32.to_le_bytes()); // payload toc
    push!(&0i32.to_le_bytes()); // data resources

    let name_offset = asset.len() as i32;
    for n in &names {
        push!(&((n.len() + 1) as i32).to_le_bytes());
        push!(n.as_bytes());
        push!(&[0u8][..]);
        push!(&0i16.to_le_bytes());
    }

    let import_offset = asset.len() as i32;
    push!(&idx_engine_pkg.to_le_bytes());
    push!(&0i32.to_le_bytes());
    push!(&idx_static_mesh.to_le_bytes());
    push!(&0i32.to_le_bytes());
    push!(&0i32.to_le_bytes()); // outer
    push!(&idx_static_mesh.to_le_bytes());
    push!(&0i32.to_le_bytes());
    push!(&0u32.to_le_bytes()); // optional

    let export_offset = asset.len() as i32;
    push!(&(-1i32).to_le_bytes()); // class = first import
    push!(&0i32.to_le_bytes());
    push!(&0i32.to_le_bytes());
    push!(&idx_name.to_le_bytes());
    push!(&0i32.to_le_bytes());
    push!(&0u32.to_le_bytes());
    let serial_size_pos = asset.len();
    push!(&0i64.to_le_bytes());
    let serial_offset_pos = asset.len();
    push!(&0i64.to_le_bytes());
    push!(&0u32.to_le_bytes());
    push!(&0u32.to_le_bytes());
    push!(&0u32.to_le_bytes());
    push!(&[0u8; 16]);
    push!(&0u32.to_le_bytes());
    push!(&0u32.to_le_bytes());
    push!(&1u32.to_le_bytes());
    push!(&0i32.to_le_bytes());
    push!(&0u32.to_le_bytes());
    push!(&0u32.to_le_bytes());
    push!(&0u32.to_le_bytes());
    push!(&0u32.to_le_bytes());

    let total_header_size = asset.len() as i32;
    asset[total_header_pos..total_header_pos + 4].copy_from_slice(&total_header_size.to_le_bytes());
    asset[name_count_pos..name_count_pos + 4].copy_from_slice(&(names.len() as i32).to_le_bytes());
    asset[name_offset_pos..name_offset_pos + 4].copy_from_slice(&name_offset.to_le_bytes());
    asset[export_count_pos..export_count_pos + 4].copy_from_slice(&1i32.to_le_bytes());
    asset[export_offset_pos..export_offset_pos + 4].copy_from_slice(&export_offset.to_le_bytes());
    asset[import_count_pos..import_count_pos + 4].copy_from_slice(&1i32.to_le_bytes());
    asset[import_offset_pos..import_offset_pos + 4].copy_from_slice(&import_offset.to_le_bytes());
    asset[serial_size_pos..serial_size_pos + 8].copy_from_slice(&(exp.len() as i64).to_le_bytes());
    asset[serial_offset_pos..serial_offset_pos + 8].copy_from_slice(&0i64.to_le_bytes());

    (asset, exp)
}
