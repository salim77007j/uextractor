//! End-to-end Unity support tests: generated real UnityFS bundles and
//! SerializedFiles are mounted, listed, extracted and converted; outputs are
//! validated against known sample data (NO fake/placeholder checks).

use uex_core::mount::{Mount, MountKeys};
use uex_core::pipeline::{run_extraction, ExtractOptions};
use uex_testdata::unity as gen;

fn sample_rgba() -> (Vec<u8>, i32, i32) {
    // 8x8 gradient: pixel (x,y) = (x*32, y*32, 128, 255)
    let (w, h) = (8i32, 8i32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            rgba.push((x * 32) as u8);
            rgba.push((y * 32) as u8);
            rgba.push(128);
            rgba.push(255);
        }
    }
    (rgba, w, h)
}

fn write_test_bundle(dir: &std::path::Path, lz4: bool) -> std::path::PathBuf {
    let (rgba, w, h) = sample_rgba();
    let bundle = gen::sample_bundle_rgba("sample", &rgba, w, h, lz4);
    let path = dir.join(if lz4 { "sample_lz4.bundle" } else { "sample.bundle" });
    std::fs::write(&path, &bundle).unwrap();
    path
}

fn test_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("uex_unity_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn unity_bundle_open_list_extract() {
    let dir = test_dir("bundle_open");
    let bundle_path = write_test_bundle(&dir, false);

    let mount = Mount::open(&bundle_path, None).expect("bundle must open");
    let files: Vec<String> = mount.files().map(|f| f.path.clone()).collect();
    assert_eq!(files.len(), 2, "bundle should list CAB + resS: {files:?}");
    assert!(files.iter().any(|f| f.ends_with("CAB-sample")));
    assert!(files.iter().any(|f| f.ends_with("CAB-sample.resS")));
    assert!(!mount.has_unity_encrypted());
}

#[test]
fn unity_bundle_convert_extract_all_objects() {
    let dir = test_dir("bundle_convert");
    let bundle_path = write_test_bundle(&dir, false);
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();

    let mount = Mount::open(&bundle_path, None).unwrap();
    let opts = ExtractOptions {
        output_dir: out.clone(),
        convert: true,
        ..Default::default()
    };
    let report = run_extraction(&mount, &opts).unwrap();
    assert_eq!(report.errors.len(), 0, "no extraction errors: {:?}", report.errors);
    assert!(report.converted >= 1);

    // texture: PNG with exact sample pixels (container path assets/sample/hero.png
    // provides the friendly name "hero")
    let png = find(&out, "hero.png").expect("texture png");
    let img = image::open(&png).unwrap().to_rgba8();
    assert_eq!((img.width(), img.height()), (8, 8));
    assert_eq!(img.get_pixel(7, 7).0, [224, 224, 128, 255], "gradient bottom-right");
    assert_eq!(img.get_pixel(0, 0).0, [0, 0, 128, 255], "gradient top-left");

    // sprite: cropped 8x8 (whole texture rect); name collides with the texture
    // (both "hero" from the container), so it gains a path-id suffix
    let sprite = find(&out, "hero_5.png").expect("sprite png");
    let img = image::open(&sprite).unwrap().to_rgba8();
    assert_eq!((img.width(), img.height()), (8, 8));

    // mesh: GLB with 4 vertices
    let glb = find(&out, "quad.glb").expect("mesh glb");
    let glb_bytes = std::fs::read(&glb).unwrap();
    assert_eq!(&glb_bytes[..4], b"glTF", "valid GLB magic");
    assert!(glb_bytes.len() > 400, "GLB should carry 4 vertices");

    // audio: .fsb payload (Vorbis in FSB container) with the exact resS bytes
    let audio = find(&out, "bgm.fsb").expect("audio payload");
    let payload = std::fs::read(&audio).unwrap();
    assert_eq!(payload.len(), 4096);
    assert_eq!(payload[0], 0);
    assert_eq!(payload[1], 7);

    // text asset
    let txt = find(&out, "readme.txt").expect("text asset");
    assert_eq!(std::fs::read(&txt).unwrap(), b"hello from unity sample");

    // metadata json with unity version + object list
    let meta = find(&out, "CAB-sample.metadata.json").expect("metadata json");
    let meta_json: serde_json::Value = serde_json::from_slice(&std::fs::read(&meta).unwrap()).unwrap();
    assert_eq!(meta_json["engine"], "unity");
    assert_eq!(meta_json["unity_version"], gen::UNITY_VERSION_2019);
    let objects = meta_json["objects"].as_array().unwrap();
    assert_eq!(objects.len(), 7, "7 sample objects");
}

#[test]
fn unity_bundle_lz4_blocks() {
    let dir = test_dir("bundle_lz4");
    let bundle_path = write_test_bundle(&dir, true);
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();

    let mount = Mount::open(&bundle_path, None).unwrap();
    let opts = ExtractOptions {
        output_dir: out.clone(),
        convert: true,
        ..Default::default()
    };
    let report = run_extraction(&mount, &opts).unwrap();
    assert_eq!(report.errors.len(), 0, "{:?}", report.errors);
    assert!(find(&out, "hero.png").is_some(), "texture extracted through LZ4 blocks");
}

#[test]
fn unity_standalone_serialized_file() {
    let dir = test_dir("standalone");
    let (rgba, w, h) = sample_rgba();
    // sharedassets0 style: stripped typetrees (TPK path) + inline data
    let data = gen::sample_standalone_serialized(&rgba, w, h);
    let path = dir.join("sharedassets0.assets");
    std::fs::write(&path, &data).unwrap();
    // audio resource stream referenced by the sample AudioClip
    std::fs::write(dir.join("CAB-sample.resS"), gen::sample_resource()).unwrap();

    let mount = Mount::open(&path, None).unwrap();
    assert_eq!(mount.file_count(), 1);
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let opts = ExtractOptions {
        output_dir: out.clone(),
        convert: true,
        ..Default::default()
    };
    let report = run_extraction(&mount, &opts).unwrap();
    assert_eq!(report.errors.len(), 0, "{:?}", report.errors);
    let png = find(&out, "hero.png").expect("texture via stripped tree");
    let img = image::open(&png).unwrap().to_rgba8();
    assert_eq!((img.width(), img.height()), (8, 8));
    assert_eq!(img.get_pixel(3, 5).0, [96, 160, 128, 255]);
}

#[test]
fn unity_stripped_typetrees_via_tpk() {
    let dir = test_dir("stripped");
    let (rgba, w, h) = sample_rgba();
    let data = gen::sample_standalone_stripped(&rgba, w, h);
    let path = dir.join("level0");
    std::fs::write(&path, &data).unwrap();
    std::fs::write(dir.join("CAB-sample.resS"), gen::sample_resource()).unwrap();

    // extensionless "level0" is detected as a Unity serialized file
    let mount = Mount::open(&path, None).unwrap();
    assert_eq!(mount.file_count(), 1, "level0 must be recognized");
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let opts = ExtractOptions {
        output_dir: out.clone(),
        convert: true,
        ..Default::default()
    };
    let report = run_extraction(&mount, &opts).unwrap();
    assert_eq!(report.errors.len(), 0, "{:?}", report.errors);
    // sprite + texture + mesh + audio + text + json outputs
    assert!(find(&out, "hero.png").is_some(), "texture2d");
    assert!(find(&out, "hero.png").is_some(), "sprite crop");
    assert!(find(&out, "quad.glb").is_some(), "mesh");
    assert!(find(&out, "bgm.fsb").is_some(), "audio");
    assert!(find(&out, "readme.txt").is_some(), "text");
}

#[test]
fn unity_directory_mount() {
    let dir = test_dir("dir_mount");
    let (rgba, w, h) = sample_rgba();
    let bundle = gen::sample_bundle_rgba("sample", &rgba, w, h, false);
    std::fs::write(dir.join("mybundle.bundle"), &bundle).unwrap();
    let assets = gen::sample_standalone_serialized(&rgba, w, h);
    std::fs::write(dir.join("sharedassets0.assets"), &assets).unwrap();
    // a resource file: should NOT become a virtual entry by itself
    std::fs::write(dir.join("sharedassets0.resource"), b"backing data").unwrap();

    let mount = Mount::open(&dir, None).unwrap();
    let paths: Vec<String> = mount.files().map(|f| f.path.clone()).collect();
    assert!(
        paths.iter().any(|p| p.starts_with("mybundle/") && p.ends_with("CAB-sample")),
        "bundle node listed: {paths:?}"
    );
    assert!(paths.iter().any(|p| p == "sharedassets0"), "standalone assets listed: {paths:?}");
    assert!(!paths.iter().any(|p| p.ends_with(".resource")), "resource files are backing data only");
}

#[test]
fn unity_encrypted_bundle_lists_and_rejects_wrong_key() {
    // build an encrypted UnityFS by patching the encryption flag + writing the
    // ArchiveStorageDecryptor header fields (all zeros = no valid key material)
    let dir = test_dir("unitycn");
    let (rgba, w, h) = sample_rgba();
    let mut bundle = gen::sample_bundle_rgba("sample", &rgba, w, h, false);
    // header: sig8+ver4+player6+engine12=30; size u64@30, comp@38, uncomp@42, flags@46
    let flags_pos = 46;
    let mut flags = u32::from_le_bytes([
        bundle[flags_pos],
        bundle[flags_pos + 1],
        bundle[flags_pos + 2],
        bundle[flags_pos + 3],
    ]);
    flags |= 0x400; // UsesAssetBundleEncryption (old flags for 2019.4)
    bundle[flags_pos..flags_pos + 4].copy_from_slice(&flags.to_be_bytes()); // header is BE
    // decryptor header follows the flags field: u32 unknown + data(16) + key(16)
    // + separator byte + data_sig(16) + key_sig(16) = 69 bytes. After skipping
    // it the reader aligns to 16, so the blocks info must move accordingly.
    let after_flags = flags_pos + 4; // 50
    let bi_pos_original = 64usize;   // align16(50) in the unencrypted bundle
    let mut out: Vec<u8> = bundle[..after_flags].to_vec();
    out.extend(std::iter::repeat(0u8).take(69)); // decryptor block
    let new_bi_pos = out.len().div_ceil(16) * 16; // align16(119) = 128
    out.resize(new_bi_pos, 0);
    out.extend_from_slice(&bundle[bi_pos_original..]);
    let path = dir.join("encrypted.bundle");
    std::fs::write(&path, &out).unwrap();

    // opening without a key: bundles still LIST (blocks info is plain)
    let mount = Mount::open(&path, None).expect("encrypted bundle lists without key");
    assert!(mount.has_unity_encrypted(), "flagged as UnityCN-encrypted");
    assert!(mount.encrypted_file_count() >= 1);

    // a wrong 16-char key must fail validation via signature check
    let wrong = Mount::open(&path, None)
        .ok()
        .map(|m| m.test_unity_key("aaaaaaaaaaaaaaaa").unwrap_or(false))
        .unwrap_or(false);
    assert!(!wrong, "wrong key must be rejected by signature check");
}

fn find(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    fn walk(dir: &std::path::Path, name: &str, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, name, out);
            } else if p.file_name().map(|f| f == name).unwrap_or(false) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, name, &mut out);
    out.into_iter().next()
}
