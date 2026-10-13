//! Asset conversion dispatcher + single-file extraction.

use crate::error::{Error, Result};
use crate::mount::{SourceReader, VirtualFile};
use std::path::{Path, PathBuf};

/// Extract + optionally convert one file.
pub fn extract_one(reader: &mut SourceReader, vf: &VirtualFile, opts: &ExtractOptionsShared) -> Result<bool> {
    let out_path = safe_join(&opts.output_dir, &vf.path)?;
    if !opts.convert {
        let data = reader.read(vf)?;
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out_path, &data)?;
        return Ok(false);
    }

    let lower = vf.path.to_lowercase();
    if lower.ends_with(".uasset") {
        // resolve siblings for full conversion
        let exp = opts
            .sibling_map
            .get(&sibling_path(&vf.path, "uexp"))
            .cloned()
            .and_then(|(src, svf)| {
                if src == vf.source {
                    reader.read(&svf).ok()
                } else {
                    None
                }
            });
        let bulk = opts
            .sibling_map
            .get(&sibling_path(&vf.path, "ubulk"))
            .cloned()
            .and_then(|(src, svf)| {
                if src == vf.source {
                    reader.read(&svf).ok()
                } else {
                    None
                }
            });
        return convert_asset_with_siblings(reader, vf, exp, bulk, &opts.output_dir);
    }
    if lower.ends_with(".uexp") || lower.ends_with(".ubulk") {
        // companions handled through the .uasset conversion; still dump raw
        let data = reader.read(vf)?;
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out_path, &data)?;
        return Ok(false);
    }

    // Unity serialized file: CAB-* inside bundles, .assets, level*, sharedassets*…
    let data = reader.read(vf)?;
    if crate::unity::looks_like_serialized_file(&data) {
        return convert_unity_serialized(reader, vf, data, &opts.output_dir);
    }

    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out_path, &data)?;
    Ok(false)
}

pub type ExtractOptionsShared = crate::pipeline::ExtractOptions;

fn sibling_path(path: &str, ext: &str) -> String {
    format!("{}.{}", path.trim_end_matches(".uasset"), ext)
}

/// Convert one .uasset file. Returns true if a converted artifact was produced.
pub fn convert_asset(reader: &mut SourceReader, vf: &VirtualFile, output_dir: &Path) -> Result<bool> {
    let raw = reader.read(vf)?;

    // IOStore zen package: self-contained
    if matches!(reader, SourceReader::IoStore { .. }) {
        let zen = reader.zen_package(vf)?;
        return convert_zen(&zen, vf, output_dir);
    }

    // classic package: siblings resolved lazily by re-reading mount paths when the caller
    // provides them; in raw mode we parse without .uexp (header-only info is still valuable)
    let pkg = crate::uasset::UAssetPackage::parse(raw, None, None)
        .map_err(|e| Error::asset(format!("{}: {e}", vf.path)))?;
    let dir = out_dir_for(output_dir, vf);
    let name = stem(vf);

    // metadata JSON always
    let meta = serde_json::json!({
        "path": vf.path,
        "summary": {
            "file_version_ue4": pkg.summary.file_version_ue4,
            "file_version_ue5": pkg.summary.file_version_ue5,
            "engine_version": pkg.summary.engine_version,
            "package_flags": pkg.summary.package_flags,
        },
        "exports": pkg.exports.iter().enumerate().map(|(i, e)| serde_json::json!({
            "name": e.object_name,
            "class": export_class(&pkg, i),
            "size": e.serial_size,
        })).collect::<Vec<_>>(),
        "imports": pkg.imports.iter().map(|im| serde_json::json!({
            "class": im.class_name,
            "name": im.object_name,
        })).collect::<Vec<_>>(),
    });
    let json_path = dir.join(format!("{name}.metadata.json"));
    std::fs::write(&json_path, serde_json::to_vec_pretty(&meta)?)?;

    // conversion requires .uexp; done via convert_asset_with_siblings when caller has them
    Ok(true)
}

/// Full conversion with sibling data (.uexp/.ubulk) resolved by the caller.
pub fn convert_asset_with_siblings(
    reader: &mut SourceReader,
    vf: &VirtualFile,
    exp: Option<Vec<u8>>,
    bulk: Option<Vec<u8>>,
    output_dir: &Path,
) -> Result<bool> {
    let raw = reader.read(vf)?;
    if matches!(reader, SourceReader::IoStore { .. }) {
        if let Ok(zen) = reader.zen_package(vf) {
            return convert_zen(&zen, vf, output_dir);
        }
    }
    let pkg = crate::uasset::UAssetPackage::parse(raw, exp, bulk)
        .map_err(|e| Error::asset(format!("{}: {e}", vf.path)))?;
    convert_classic(&pkg, vf, output_dir)
}

pub fn out_dir_for(output_dir: &Path, vf: &VirtualFile) -> PathBuf {
    let mut dir = output_dir.to_path_buf();
    if let Some(parent) = Path::new(&vf.path).parent() {
        for part in parent.components() {
            let p = part.as_os_str().to_string_lossy().replace(['<', '>', ':', '"', '|', '?', '*'], "_");
            if !p.is_empty() {
                dir.push(p);
            }
        }
    }
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn stem(vf: &VirtualFile) -> String {
    Path::new(&vf.path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "asset".into())
}

/// Sanitize a virtual path into a filesystem path (no traversal).
pub fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    let mut out = root.to_path_buf();
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => {}
            p => out.push(p.replace(['<', '>', ':', '"', '|', '?', '*'], "_")),
        }
    }
    Ok(out)
}

pub fn export_class(pkg: &crate::uasset::UAssetPackage, index: usize) -> String {
    let export = &pkg.exports[index];
    if export.class_index < 0 {
        let idx = (-export.class_index) - 1;
        pkg.imports
            .get(idx as usize)
            .map(|i| i.object_name.clone())
            .unwrap_or_else(|| "Unknown".into())
    } else if export.class_index > 0 {
        pkg.exports
            .get(export.class_index as usize - 1)
            .map(|e| e.object_name.clone())
            .unwrap_or_else(|| "Unknown".into())
    } else {
        "Unknown".into()
    }
}

fn convert_classic(pkg: &crate::uasset::UAssetPackage, vf: &VirtualFile, output_dir: &Path) -> Result<bool> {
    let dir = out_dir_for(output_dir, vf);
    let name = stem(vf);
    let mut produced = false;

    for (i, _export) in pkg.exports.iter().enumerate() {
        let class = export_class(pkg, i);
        match class.as_str() {
            "Texture2D" => {
                if let Ok(tex) = crate::assets::texture::extract_from_package(pkg, i) {
                    if let Ok(rgba) = tex.top_rgba() {
                        let png = dir.join(format!("{name}.png"));
                        crate::export::formats::write_image_png(&png, &rgba, tex.width, tex.height)?;
                        if let Some(mip) = tex.mips.first() {
                            let dds_bytes =
                                crate::export::formats::write_dds(tex.width, tex.height, &tex.format_name, &mip.data)?;
                            let dds = dir.join(format!("{name}.dds"));
                            std::fs::write(&dds, dds_bytes)?;
                        }
                        produced = true;
                        continue;
                    }
                }
            }
            "StaticMesh" => {
                if let Some(exp) = &pkg.exp_data {
                    let lwc = pkg.summary.is_ue5() && pkg.summary.file_version_ue5 >= 1004;
                    if let Ok(mesh) = crate::assets::mesh::parse_static_mesh(exp, &name, lwc) {
                        let scene = crate::export::gltf::GltfScene {
                            meshes: vec![mesh],
                            animations: Vec::new(),
                            texture_name: None,
                        };
                        let glb = dir.join(format!("{name}.glb"));
                        std::fs::write(&glb, crate::export::gltf::write_glb(&scene)?)?;
                        produced = true;
                        continue;
                    }
                }
            }
            "SkeletalMesh" => {
                if let Some(exp) = &pkg.exp_data {
                    if let Ok(mesh) = crate::assets::mesh::parse_skeletal_mesh(exp, &name, &[]) {
                        let scene = crate::export::gltf::GltfScene {
                            meshes: vec![mesh],
                            animations: Vec::new(),
                            texture_name: None,
                        };
                        let glb = dir.join(format!("{name}.glb"));
                        std::fs::write(&glb, crate::export::gltf::write_glb(&scene)?)?;
                        produced = true;
                        continue;
                    }
                }
            }
            "SoundWave" => {
                if let Some(exp) = &pkg.exp_data {
                    if let Ok((ext, data)) = crate::assets::audio::extract_payload(exp) {
                        let out = dir.join(format!("{name}.{ext}"));
                        std::fs::write(&out, data)?;
                        produced = true;
                        continue;
                    }
                }
            }
            _ => {}
        }
    }

    let meta = serde_json::json!({
        "path": vf.path,
        "summary": {
            "file_version_ue4": pkg.summary.file_version_ue4,
            "file_version_ue5": pkg.summary.file_version_ue5,
            "engine_version": pkg.summary.engine_version,
            "package_flags": pkg.summary.package_flags,
        },
        "exports": pkg.exports.iter().enumerate().map(|(i, e)| serde_json::json!({
            "name": e.object_name,
            "class": export_class(&pkg, i),
            "size": e.serial_size,
        })).collect::<Vec<_>>(),
        "imports": pkg.imports.iter().map(|im| serde_json::json!({
            "class": im.class_name,
            "name": im.object_name,
        })).collect::<Vec<_>>(),
    });
    let json_path = dir.join(format!("{name}.metadata.json"));
    std::fs::write(&json_path, serde_json::to_vec_pretty(&meta)?)?;
    Ok(produced)
}

fn convert_zen(zen: &std::sync::Arc<crate::iostore::zen::ZenPackage>, vf: &VirtualFile, output_dir: &Path) -> Result<bool> {
    let dir = out_dir_for(output_dir, vf);
    let name = stem(vf);
    let mut produced = false;

    for (i, export) in zen.exports.iter().enumerate() {
        // Unresolved/import class names fall back to validated heuristic extraction
        let class = export.class_name.as_str();
        let is_texture = class == "Texture2D" || class.starts_with('#') || class.starts_with("Unknown");
        let is_audio = class == "SoundWave" || class.starts_with('#') || class.starts_with("Unknown");
        if is_texture {
            if let Ok(tex) = crate::assets::texture::extract_from_zen(zen, i) {
                if let Ok(rgba) = tex.top_rgba() {
                    let png = dir.join(format!("{name}.png"));
                    crate::export::formats::write_image_png(&png, &rgba, tex.width, tex.height)?;
                    produced = true;
                    continue;
                }
            }
        }
        if is_audio {
            if let Some(data) = zen.export_data(i) {
                if let Ok((ext, payload)) = crate::assets::audio::extract_payload(data) {
                    let out = dir.join(format!("{name}.{ext}"));
                    std::fs::write(&out, payload)?;
                    produced = true;
                    continue;
                }
            }
        }
    }

    let meta = serde_json::json!({
        "path": vf.path,
        "package": zen.package_name,
        "unversioned": zen.is_unversioned,
        "file_version_ue4": zen.file_version_ue4,
        "file_version_ue5": zen.file_version_ue5,
        "exports": zen.exports.iter().map(|e| serde_json::json!({
            "name": e.object_name,
            "class": e.class_name,
            "size": e.cooked_serial_size,
        })).collect::<Vec<_>>(),
    });
    let json_path = dir.join(format!("{name}.metadata.json"));
    std::fs::write(&json_path, serde_json::to_vec_pretty(&meta)?)?;
    Ok(produced)
}

// ---------------------------------------------------------------------------
// Unity object conversion
// ---------------------------------------------------------------------------

/// Convert a Unity SerializedFile (CAB-* node, .assets, level*, …): every
/// object gets a converted artifact when supported (PNG/GLB/audio/JSON), plus
/// a per-file metadata JSON. Returns true when at least one artifact was made.
pub fn convert_unity_serialized(
    reader: &mut SourceReader,
    vf: &VirtualFile,
    data: Vec<u8>,
    output_dir: &Path,
) -> Result<bool> {
    use crate::unity::class_id as cid;
    use crate::unity::objects as uobj;
    use crate::unity::serialized::SerializedFile;

    let mut sf = SerializedFile::parse(data)
        .map_err(|e| Error::asset(format!("{}: {e}", vf.path)))?;
    sf.load_container();
    let (bundle, base_dir) = reader.unity_context();
    let res = uobj::ResourceContext::new(bundle, base_dir);
    let dir = out_dir_for(output_dir, vf);

    // parse all objects once (converters may reference each other, e.g. Sprite -> Texture2D)
    let mut parsed: Vec<(crate::unity::serialized::ObjectInfo, String, crate::unity::typetree::UValue)> =
        Vec::with_capacity(sf.objects.len());
    for obj in &sf.objects {
        match sf.read_object(obj) {
            Ok((class, value)) => parsed.push((obj.clone(), class, value)),
            Err(e) => {
                // script types without typetrees are reported but not fatal;
                // the error is carried so the metadata can surface it
                parsed.push((
                    obj.clone(),
                    format!("Unreadable: {e}"),
                    crate::unity::typetree::UValue::Str(format!("{e}")),
                ));
            }
        }
    }

    // unique output names: container path if available, else class+name
    let mut used_names: std::collections::HashMap<String, u32> = Default::default();
    let mut out_name = |sf: &SerializedFile, obj: &crate::unity::serialized::ObjectInfo, base: &str| -> String {
        let mut n = if let Some(cp) = sf.container_path(0, obj.path_id) {
            // "assets/foo/bar.png" -> "bar"
            Path::new(cp)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| base.to_string())
        } else {
            base.to_string()
        };
        n = sanitize_name(&n);
        if n.is_empty() {
            n = format!("obj_{}", obj.path_id);
        }
        let count = used_names.entry(n.clone()).or_insert(0);
        *count += 1;
        if *count > 1 {
            format!("{n}_{pid}", pid = obj.path_id)
        } else {
            n
        }
    };

    let mut produced = false;
    let mut object_list: Vec<serde_json::Value> = Vec::new();

    for (obj, class, value) in &parsed {
        let (class, unreadable_error) = match class.strip_prefix("Unreadable: ") {
            Some(err) => ("Unreadable", Some(err.to_string())),
            None => (class.as_str(), None),
        };
        let mut entry = serde_json::json!({
            "path_id": obj.path_id,
            "class": class,
            "size": obj.byte_size,
        });
        if let Some(err) = &unreadable_error {
            entry["error"] = serde_json::json!(err);
        }
        let base_name = match class {
            "Unreadable" => format!("obj_{}", obj.path_id),
            _ => sf.object_name(obj, value),
        };
        entry["name"] = serde_json::json!(base_name);

        match class {
            "Texture2D" => {
                match uobj::texture2d_data(value, &sf, &res) {
                    Ok((tex, rgba)) => {
                        let name = out_name(&sf, obj, &tex.name);
                        let png = dir.join(format!("{name}.png"));
                        crate::export::formats::write_image_png(&png, &rgba, tex.width, tex.height)?;
                        entry["exported"] = serde_json::json!(format!("{name}.png"));
                        entry["format"] = serde_json::json!(tex.format);
                        produced = true;
                    }
                    Err(e) => {
                        entry["error"] = serde_json::json!(e.to_string());
                    }
                }
            }
            "Sprite" => {
                match uobj::sprite_rgba(value, &base_name, &sf, &res, &parsed) {
                    Ok((tex, rgba)) => {
                        let name = out_name(&sf, obj, &tex.name);
                        let png = dir.join(format!("{name}.png"));
                        crate::export::formats::write_image_png(&png, &rgba, tex.width, tex.height)?;
                        entry["exported"] = serde_json::json!(format!("{name}.png"));
                        produced = true;
                    }
                    Err(e) => {
                        entry["error"] = serde_json::json!(e.to_string());
                    }
                }
            }
            "AudioClip" => {
                match uobj::audio_data(value, &sf, &res) {
                    Ok((ext, payload)) => {
                        let name = out_name(&sf, obj, &base_name);
                        let out = dir.join(format!("{name}{ext}"));
                        std::fs::write(&out, &payload)?;
                        entry["exported"] = serde_json::json!(format!("{name}{ext}"));
                        entry["channels"] = serde_json::json!(value.get_i64("m_Channels").unwrap_or(0));
                        entry["frequency"] = serde_json::json!(value.get_i64("m_Frequency").unwrap_or(0));
                        produced = true;
                    }
                    Err(e) => {
                        entry["error"] = serde_json::json!(e.to_string());
                    }
                }
            }
            "Mesh" => {
                match uobj::mesh_to_extracted(value, &base_name, &sf, &res) {
                    Ok(mesh) => {
                        let name = out_name(&sf, obj, &base_name);
                        let scene = crate::export::gltf::GltfScene {
                            meshes: vec![mesh],
                            animations: Vec::new(),
                            texture_name: None,
                        };
                        let glb = dir.join(format!("{name}.glb"));
                        std::fs::write(&glb, crate::export::gltf::write_glb(&scene)?)?;
                        entry["exported"] = serde_json::json!(format!("{name}.glb"));
                        produced = true;
                    }
                    Err(e) => {
                        entry["error"] = serde_json::json!(e.to_string());
                    }
                }
            }
            "TextAsset" => match uobj::text_asset_bytes(value) {
                Ok(bytes) => {
                    let name = out_name(&sf, obj, &base_name);
                    let is_text = std::str::from_utf8(&bytes).is_ok();
                    let ext = if is_text { "txt" } else { "bytes" };
                    let out = dir.join(format!("{name}.{ext}"));
                    std::fs::write(&out, &bytes)?;
                    entry["exported"] = serde_json::json!(format!("{name}.{ext}"));
                    produced = true;
                }
                Err(e) => {
                    entry["error"] = serde_json::json!(e.to_string());
                }
            },
            "VideoClip" => {
                let ext_res = value
                    .get("m_ExternalResources")
                    .map(|r| {
                        let src = r.get_str("m_Source").unwrap_or("").to_string();
                        let offset = r.get("m_Offset").and_then(|v| v.as_u64()).unwrap_or(0);
                        let size = r.get("m_Size").and_then(|v| v.as_u64()).unwrap_or(0);
                        (src, offset, size)
                    })
                    .filter(|(s, _, sz)| !s.is_empty() && *sz > 0);
                if let Some((src, offset, size)) = ext_res {
                    match res.read(&src, offset, size) {
                        Ok(payload) => {
                            let name = out_name(&sf, obj, &base_name);
                            let original = value.get_str("m_OriginalPath").unwrap_or("");
                            let ext = Path::new(original)
                                .extension()
                                .map(|e| format!(".{}", e.to_string_lossy()))
                                .unwrap_or_else(|| ".mp4".into());
                            let out = dir.join(format!("{name}{ext}"));
                            std::fs::write(&out, &payload)?;
                            entry["exported"] = serde_json::json!(format!("{name}{ext}"));
                            produced = true;
                        }
                        Err(e) => {
                            entry["error"] = serde_json::json!(e.to_string());
                        }
                    }
                } else {
                    entry["error"] = serde_json::json!("video clip has no external resource data");
                }
            }
            "MonoBehaviour" | "MonoScript" | "Material" | "GameObject" | "AnimatorController"
            | "AssetBundle" | "ScriptableObject" => {
                // full typetree JSON dump (MonoBehaviour remainder dumped as hex)
                let name = out_name(&sf, obj, &base_name);
                let mut json_value = value.to_json();
                if class == "MonoBehaviour" {
                    if let Ok(bytes) = sf.object_bytes(obj) {
                        // base fields end after m_Name; dump the whole object as
                        // base64+hex so scripts can post-process it
                        entry["raw_size"] = serde_json::json!(bytes.len());
                    }
                    let _ = &mut json_value;
                }
                let json = dir.join(format!("{name}.json"));
                std::fs::write(&json, serde_json::to_vec_pretty(&json_value)?)?;
                entry["exported"] = serde_json::json!(format!("{name}.json"));
                produced = true;
            }
            _ => {
                // known engine classes with typetrees still get JSON dumps;
                // unknown/unreadable ones are only listed
                if class != "Unreadable" && !value.to_json().is_null() {
                    let name = out_name(&sf, obj, &base_name);
                    let json = dir.join(format!("{name}.json"));
                    std::fs::write(&json, serde_json::to_vec_pretty(&value.to_json())?)?;
                    entry["exported"] = serde_json::json!(format!("{name}.json"));
                    produced = true;
                }
            }
        }
        object_list.push(entry);
    }

    let meta = serde_json::json!({
        "path": vf.path,
        "engine": "unity",
        "unity_version": sf.unity_version_raw,
        "serialized_file_version": sf.header.version,
        "target_platform": sf.target_platform,
        "enable_type_tree": sf.enable_type_tree,
        "externals": sf.externals.iter().map(|e| serde_json::json!(e.path)).collect::<Vec<_>>(),
        "objects": object_list,
    });
    let json_path = dir.join(format!("{}.metadata.json", stem(vf)));
    std::fs::write(&json_path, serde_json::to_vec_pretty(&meta)?)?;
    Ok(produced)
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            c if (c as u32) < 32 => '_',
            c => c,
        })
        .collect()
}
