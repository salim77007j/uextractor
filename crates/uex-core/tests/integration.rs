//! Integration tests: PAK (v8B/v10/v11, compressed, encrypted) + IOStore round-trips
//! and asset extraction using synthetic archives.

use uex_core::crypto::AesKey;
use uex_core::iostore::{ChunkType, IoStoreReader};
use uex_core::mount::Mount;
use uex_core::pak::{PakReader, PakWriter, Version};
use uex_testdata::*;

const KEY_HEX: &str = "11223344556677889900AABBCCDDEEFF00112233445566778899AABBCCDDEEFF";

fn samples(n: usize, size: usize) -> Vec<SampleFile> {
    (0..n)
        .map(|i| SampleFile::new(&format!("Props/F{i}.bin"), pseudo_data(i as u64 + 3, size)))
        .collect()
}

fn write_and_read(version: Version, files: &[SampleFile], compress: bool, key: Option<&str>) -> Vec<u8> {
    let aes = key.and_then(AesKey::parse);
    let methods = if compress {
        vec![uex_core::compress::Method::Zlib, uex_core::compress::Method::Lz4]
    } else {
        vec![]
    };
    let mut w = PakWriter::new(
        std::io::Cursor::new(Vec::new()),
        version,
        "../../../TestGame/Content/",
        aes,
        methods,
        key.is_some(),
    );
    for f in files {
        w.write_file(&f.path, &f.data).unwrap();
    }
    w.finalize().unwrap().into_inner()
}

#[test]
fn pak_v11_uncompressed_roundtrip() {
    let files = samples(4, 10_000);
    let buf = write_and_read(Version::V11, &files, false, None);
    let mut reader = PakReader::new(std::io::Cursor::new(buf), None).unwrap();
    assert_eq!(reader.version(), Version::V11);
    assert_eq!(reader.entries().len(), 4);
    for f in &files {
        let got = reader.read_entry(&f.path).unwrap();
        assert_eq!(got, f.data, "content mismatch for {}", f.path);
    }
}

#[test]
fn pak_v10_compressed_roundtrip() {
    let files = samples(4, 50_000);
    let buf = write_and_read(Version::V10, &files, true, None);
    let mut reader = PakReader::new(std::io::Cursor::new(buf), None).unwrap();
    for f in &files {
        let got = reader.read_entry(&f.path).unwrap();
        assert_eq!(got, f.data);
    }
}

#[test]
fn pak_v8b_roundtrip() {
    let files = samples(2, 5_000);
    let buf = write_and_read(Version::V8B, &files, false, None);
    let mut reader = PakReader::new(std::io::Cursor::new(buf), None).unwrap();
    for f in &files {
        assert_eq!(reader.read_entry(&f.path).unwrap(), f.data);
    }
}

#[test]
fn pak_v11_zlib_compressed_roundtrip() {
    let files = samples(2, 200_000); // multi-block (>126KiB) to exercise block splitting
    let buf = write_and_read(Version::V11, &files, true, None);
    let mut reader = PakReader::new(std::io::Cursor::new(buf), None).unwrap();
    let entry = reader.entries().values().next().unwrap();
    assert!(entry.compression_slot.is_some(), "expected compressed entry");
    if let Some(blocks) = &entry.blocks {
        assert!(blocks.len() >= 2, "expected multiple compression blocks");
    }
    for f in &files {
        assert_eq!(reader.read_entry(&f.path).unwrap(), f.data);
    }
}

#[test]
fn pak_encrypted_index_and_entries() {
    let files = samples(3, 12_000);
    let buf = write_and_read(Version::V11, &files, true, Some(KEY_HEX));
    // wrong key must fail
    let wrong = AesKey::from_bytes([9u8; 32]);
    assert!(PakReader::new(std::io::Cursor::new(buf.clone()), Some(wrong)).is_err());
    // correct key must yield identical content
    let key = AesKey::parse(KEY_HEX).unwrap();
    let mut reader = PakReader::new(std::io::Cursor::new(buf), Some(key)).unwrap();
    assert!(reader.encrypted_index());
    for f in &files {
        assert_eq!(reader.read_entry(&f.path).unwrap(), f.data);
    }
}

#[cfg(feature = "oodle_test")]
#[test]
fn pak_oodle_roundtrip() {
    let files = samples(2, 64_000);
    let mut w = PakWriter::new(
        std::io::Cursor::new(Vec::new()),
        Version::V11,
        "../../../T/",
        None,
        vec![uex_core::compress::Method::Oodle],
        false,
    );
    for f in &files {
        w.write_file(&f.path, &f.data).unwrap();
    }
    let buf = w.finalize().unwrap().into_inner();
    let mut reader = PakReader::new(std::io::Cursor::new(buf), None).unwrap();
    for f in &files {
        assert_eq!(reader.read_entry(&f.path).unwrap(), f.data);
    }
}

#[test]
fn oodle_library_available_or_graceful() {
    // The official oo2core should load (downloaded or local); if unavailable the
    // error must be informative, not a crash.
    let data = b"OODLE test payload ".repeat(1000);
    match uex_core::compress::compress(uex_core::compress::Method::Oodle, &data) {
        Ok(comp) => {
            let mut out = vec![0u8; data.len()];
            uex_core::compress::decompress(uex_core::compress::Method::Oodle, &comp, &mut out).unwrap();
            assert_eq!(out, data);
        }
        Err(e) => {
            let msg = format!("{e}");
            assert!(msg.contains("Oodle"), "unexpected error: {msg}");
        }
    }
}

#[test]
fn iostore_roundtrip_and_texture() {
    let (pid, pkg) = build_zen_texture_package("T_Rock", 64, 64);
    let mut cas = CasBuilder::new();
    let chunk_idx = cas.add_chunk(pid, 0, ChunkType::ExportBundleData, &pkg, true);
    cas.add_file("Props/T_Rock.uasset", chunk_idx);
    let (utoc, ucas) = cas.build().unwrap();

    let dir = std::env::temp_dir().join(format!("uex_it_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("test.utoc"), &utoc).unwrap();
    std::fs::write(dir.join("test.ucas"), &ucas).unwrap();

    let reader = IoStoreReader::open(&dir.join("test.utoc"), None).unwrap();
    let files = reader.files();
    assert_eq!(files.len(), 1);
    assert!(files[0].0.ends_with("Props/T_Rock.uasset"), "got {}", files[0].0);

    // read the chunk back and parse the zen package
    let data = reader.read_chunk_id(&files[0].1).unwrap();
    assert_eq!(data, pkg);
    let zen = uex_core::iostore::zen::ZenPackage::parse_data(data, 2, None, &reader, pid).unwrap();
    assert_eq!(zen.exports.len(), 1);
    assert_eq!(zen.exports[0].object_name, "T_Rock");

    // extract the texture from the zen export
    let tex = uex_core::assets::texture::extract_from_zen(&zen, 0).unwrap();
    assert_eq!(tex.width, 64);
    assert_eq!(tex.height, 64);
    let rgba = tex.top_rgba().unwrap();
    assert_eq!(rgba.len(), 64 * 64 * 4);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn classic_texture_package_roundtrip() {
    let (ua, ue) = build_classic_texture_package("T_Wood", 64, 64);
    let files = vec![
        SampleFile::new("Props/T_Wood.uasset", ua),
        SampleFile::new("Props/T_Wood.uexp", ue),
    ];
    let buf = write_and_read(Version::V11, &files, false, None);
    let mut reader = PakReader::new(std::io::Cursor::new(buf), None).unwrap();
    let ua_data = reader.read_entry("Props/T_Wood.uasset").unwrap();
    let ue_data = reader.read_entry("Props/T_Wood.uexp").unwrap();

    let pkg = uex_core::uasset::UAssetPackage::parse(ua_data, Some(ue_data), None).unwrap();
    assert_eq!(pkg.exports.len(), 1);
    assert_eq!(pkg.exports[0].object_name, "T_Wood");

    let tex = uex_core::assets::texture::extract_from_package(&pkg, 0).unwrap();
    assert_eq!(tex.width, 64);
    assert_eq!(tex.format_name, "PF_DXT1");
    let rgba = tex.top_rgba().unwrap();
    assert_eq!(rgba.len(), 64 * 64 * 4);
    // decoded BC1 gradient: some pixel must be non-trivial
    assert!(rgba.iter().any(|&b| b != 0), "decoded texture is all black");
}

#[test]
fn mount_open_pak_and_extract() {
    let files = samples(8, 128_000);
    let buf = write_and_read(Version::V11, &files, true, None);
    let dir = std::env::temp_dir().join(format!("uex_mount_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pak = dir.join("test_plain.pak");
    std::fs::write(&pak, &buf).unwrap();

    let mount = Mount::open(&pak, None).unwrap();
    assert_eq!(mount.file_count(), 8);
    let out = dir.join("out");
    let report = uex_core::pipeline::run_extraction(
        &mount,
        &uex_core::pipeline::ExtractOptions {
            output_dir: out.clone(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(report.extracted, 8);
    assert_eq!(report.errors.len(), 0);
    for f in &files {
        let p = out.join("TestGame").join("Content").join(&f.path);
        let got = std::fs::read(&p).unwrap_or_else(|_| panic!("missing {}", p.display()));
        assert_eq!(got, f.data);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn gltf_export_valid_structure() {
    use uex_core::assets::mesh::{ExtractedMesh, MeshSection, Vertex};
    let mut mesh = ExtractedMesh {
        name: "Test".into(),
        ..Default::default()
    };
    mesh.vertices = vec![
        Vertex { position: [0.0, 0.0, 0.0], normal: [0.0, 0.0, 1.0], uv: vec![[0.0, 0.0]], ..Default::default() },
        Vertex { position: [1.0, 0.0, 0.0], normal: [0.0, 0.0, 1.0], uv: vec![[1.0, 0.0]], ..Default::default() },
        Vertex { position: [0.0, 1.0, 0.0], normal: [0.0, 0.0, 1.0], uv: vec![[0.0, 1.0]], ..Default::default() },
    ];
    mesh.sections = vec![MeshSection {
        material_index: 0,
        indices: vec![0, 1, 2],
    }];
    let scene = uex_core::export::gltf::GltfScene {
        meshes: vec![mesh],
        animations: Vec::new(),
        texture_name: None,
    };
    let glb = uex_core::export::gltf::write_glb(&scene).unwrap();
    // GLB magic + version 2
    assert_eq!(&glb[0..4], b"glTF");
    assert_eq!(u32::from_le_bytes(glb[4..8].try_into().unwrap()), 2);
    // parse the JSON chunk
    let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let json: serde_json::Value =
        serde_json::from_slice(&glb[20..20 + json_len]).unwrap();
    assert!(json["meshes"][0]["primitives"][0]["indices"].is_u64());
    assert_eq!(json["accessors"][0]["count"], 3);
}
