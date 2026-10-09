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
    let data = reader.read(vf)?;
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
