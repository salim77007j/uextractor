//! Classic Unreal package (.uasset + .uexp + .ubulk) parsing:
//! FPackageFileSummary, name map, import/export maps, property serialization
//! (versioned tags + UE4.26+/UE5 unversioned streams).

pub mod properties;

use crate::error::{Error, Result};
use crate::io::UexRead;
use std::io::Cursor;

/// EUnrealEngineObjectUE4Version (subset relevant to parsing)
pub mod ue4ver {
    pub const VER_UE4_INITIAL_VERSION: i32 = 1;
    pub const VER_UE4_ADDED_PACKAGE_SUMMARY_LOCALIZATION_ID: i32 = 387;
    pub const VER_UE4_SUMMARY_HAS_ASSET_REGISTRY_DATA: i32 = 464;
    pub const VER_UE4_ENGINE_VERSION_OBJECT: i32 = 485;
    pub const VER_UE4_PRELOAD_DEPENDENCIES_IN_PACKAGE_SUMMARY: i32 = 504;
    pub const VER_UE4_NAMES_REFERENCED_FROM_EXPORT_DATA: i32 = 514;
    pub const VER_UE4_PAYLOAD_TOC: i32 = 517;
    pub const VER_UE4_DATA_RESOURCES: i32 = 518;
    pub const VER_UE4_ADDED_PACKAGE_OWNER: i32 = 518;
    pub const VER_UE4_NON_OUTER_PACKAGE_IMPORT: i32 = 520;
    pub const VER_UE4_ADD_SOFTOBJECTPATH_LIST: i32 = 522;
    pub const VER_UE4_CORRECT_LICENSEE_FLAG: i32 = 522;
    pub const VER_UE4_AUTOMATIC_VERSION: i32 = 523;
}

/// EUnrealEngineObjectUE5Version
pub mod ue5ver {
    pub const INITIAL_VERSION: i32 = 1000;
    pub const NAMES_REFERENCED_FROM_EXPORT_DATA: i32 = 1001;
    pub const PAYLOAD_TOC: i32 = 1002;
    pub const OPTIONAL_RESOURCES: i32 = 1003;
    pub const LARGE_WORLD_COORDINATES: i32 = 1004;
    pub const REMOVE_OBJECT_EXPORT_PACKAGE_GUID: i32 = 1005;
    pub const TRACK_OBJECT_EXPORT_IS_INHERITED: i32 = 1006;
    pub const FSOFTPBJECTPATH_REMOVE_ASSET_PATH_NAMES: i32 = 1007;
    pub const ADD_SOFTOBJECTPATH_LIST: i32 = 1008;
    pub const DATA_RESOURCES: i32 = 1009;
    pub const SCRIPT_SERIALIZATION_OFFSET: i32 = 1010;
    pub const PROPERTY_TAG_EXTENSION: i32 = 1011;
    pub const PROPERTY_TAG_COMPLETE_TYPE_NAME: i32 = 1012;
    pub const ASSET_REGISTRY_PACKAGE_BUILD_DEPENDENCIES: i32 = 1013;
    pub const METADATA_SERIALIZATION_OFFSET: i32 = 1014;
    pub const VERSE_CELLS: i32 = 1015;
    pub const PACKAGE_SAVED_HASH: i32 = 1016;
    pub const OS_SUB_OBJECT_SHADOW_SERIALIZATION: i32 = 1017;
    pub const IMPORT_TYPE_HIERARCHIES: i32 = 1018;
}

#[derive(Debug, Clone)]
pub struct FCustomVersion {
    pub key: [u8; 16],
    pub version: i32,
}

#[derive(Debug, Clone, Default)]
pub struct PackageSummary {
    pub legacy_file_version: i32,
    pub legacy_ue3_version: i32,
    pub file_version_ue4: i32,
    pub file_version_ue5: i32,
    pub licensee_version: i32,
    pub custom_versions: Vec<FCustomVersion>,
    pub total_header_size: i32,
    pub folder_name: String,
    pub package_flags: u32,
    pub name_count: i32,
    pub name_offset: i32,
    pub gatherable_text_data_count: i32,
    pub gatherable_text_data_offset: i32,
    pub export_count: i32,
    pub export_offset: i32,
    pub import_count: i32,
    pub import_offset: i32,
    pub soft_object_paths_count: i32,
    pub soft_object_paths_offset: i32,
    pub thumbnail_table_offset: i32,
    pub guid: [u8; 16],
    pub generations: Vec<[u8; 16]>,
    pub engine_version: String,
    pub compatible_with_engine_version: String,
    pub compression_flags: u32,
    pub package_source: u32,
    pub asset_registry_data_offset: i32,
    pub bulk_data_start_offset: i64,
    pub world_tile_info_data_offset: i32,
    pub chunk_ids: Vec<i32>,
    pub preload_dependency_count: i32,
    pub preload_dependency_offset: i32,
    pub names_referenced_from_export_data_count: i32,
    pub payload_toc_offset: i32,
    pub data_resource_offset: i32,
}

impl PackageSummary {
    pub fn is_ue5(&self) -> bool {
        self.legacy_file_version <= -8 && self.file_version_ue5 > 0
    }
    pub fn has_version_info(&self) -> bool {
        self.file_version_ue4 > 0
    }
}

#[derive(Debug, Clone)]
pub struct NameEntry {
    pub name: String,
    pub number: i32,
}

#[derive(Debug, Clone, Default)]
pub struct FObjectImport {
    pub class_package: String,
    pub class_name: String,
    pub outer_index: i32,
    pub object_name: String,
    pub optional: bool,
}

#[derive(Debug, Clone, Default)]
pub struct FObjectExport {
    pub class_index: i32,
    pub super_index: i32,
    pub template_index: i32,
    pub outer_index: i32,
    pub object_name: String,
    pub save: u32,
    pub serial_size: i64,
    pub serial_offset: i64,
    pub forced_export: bool,
    pub not_for_client: bool,
    pub not_for_server: bool,
    pub package_guid: [u8; 16],
    pub package_flags: u32,
    pub not_always_loaded_for_editor_game: bool,
    pub is_asset: bool,
    pub first_export_dependency: i32,
    pub serialization_before_serialization_dependencies: bool,
    pub create_before_serialization_dependencies: bool,
    pub serialization_before_create_dependencies: bool,
    pub create_before_create_dependencies: bool,
}

/// Fully parsed classic package.
pub struct UAssetPackage {
    pub summary: PackageSummary,
    pub names: Vec<NameEntry>,
    pub imports: Vec<FObjectImport>,
    pub exports: Vec<FObjectExport>,
    pub soft_object_path_list: Vec<(String, String)>,
    /// whole .uasset file bytes
    pub asset_data: Vec<u8>,
    /// whole .uexp file bytes (if present)
    pub exp_data: Option<Vec<u8>>,
    /// whole .ubulk file bytes (if present)
    pub bulk_data: Option<Vec<u8>>,
    /// bulk data start offset (absolute position in the combined stream)
    pub bulk_data_start_offset: i64,
}

/// Read an FName (u32 index + i32 number) and resolve it against the name table.
fn read_fname_string<R: AsRef<[u8]>>(r: &mut Cursor<R>, names: &[NameEntry]) -> Result<String> {
    let index = r.u32()?;
    let number = r.i32()?;
    let mut name = names
        .get((index & 0x7FFF_FFFF) as usize)
        .map(|n| n.name.clone())
        .unwrap_or_else(|| "None".to_string());
    if number > 0 {
        name = format!("{name}_{}", number - 1);
    }
    Ok(name)
}

const PKG_FILTER_EDITOR_ONLY: u32 = 0x8000_0000;

impl UAssetPackage {
    /// True when exports live in the .uexp file (cooked / filter-editor-only packages).
    pub fn exports_in_uexp(&self) -> bool {
        self.summary.package_flags & PKG_FILTER_EDITOR_ONLY != 0
    }

    pub fn parse(asset: Vec<u8>, exp: Option<Vec<u8>>, bulk: Option<Vec<u8>>) -> Result<UAssetPackage> {
        let mut r = Cursor::new(&asset);
        let magic = r.u32()?;
        if magic != crate::UASSET_MAGIC {
            return Err(Error::Magic { found: magic });
        }
        let mut summary = PackageSummary::default();
        summary.legacy_file_version = r.i32()?;
        summary.legacy_ue3_version = r.i32()?;
        if summary.legacy_file_version <= -4 {
            summary.file_version_ue4 = r.i32()?;
        }
        if summary.legacy_file_version <= -8 {
            summary.file_version_ue5 = r.i32()?;
            summary.licensee_version = r.i32()?;
        }
        if summary.legacy_file_version <= -2 {
            let custom_count = r.i32()? as usize;
            if custom_count < 0x10000 {
                for _ in 0..custom_count {
                    let key = r.read_bytes(16)?;
                    let ver = r.i32()?;
                    summary.custom_versions.push(FCustomVersion { key: key.try_into().unwrap(), version: ver });
                }
            }
        }
        summary.total_header_size = r.i32()?;
        summary.folder_name = r.fstring()?;
        summary.package_flags = r.u32()?;
        summary.name_count = r.i32()?;
        summary.name_offset = r.i32()?;
        if summary.legacy_file_version <= -1 && summary.file_version_ue4 >= 387 {
            // gatherable text (Localization in summary: VER_UE4_ADDED_PACKAGE_SUMMARY_LOCALIZATION_ID)
            summary.gatherable_text_data_count = r.i32()?;
            summary.gatherable_text_data_offset = r.i32()?;
        }
        summary.export_count = r.i32()?;
        summary.export_offset = r.i32()?;
        summary.import_count = r.i32()?;
        summary.import_offset = r.i32()?;
        if summary.legacy_file_version <= -8 && summary.file_version_ue5 >= ue5ver::ADD_SOFTOBJECTPATH_LIST {
            summary.soft_object_paths_count = r.i32()?;
            summary.soft_object_paths_offset = r.i32()?;
        }
        if summary.legacy_file_version <= -7 {
            summary.thumbnail_table_offset = r.i32()?;
        }
        summary.guid = r.read_bytes(16)?.try_into().unwrap();
        let generation_count = r.i32()?;
        for _ in 0..generation_count.clamp(0, 0x1000) {
            summary.generations.push(r.read_bytes(16)?.try_into().unwrap());
        }
        if summary.legacy_file_version <= -8 && summary.file_version_ue5 >= ue5ver::REMOVE_OBJECT_EXPORT_PACKAGE_GUID {
            // persistent guid dropped in 1005
        }
        if summary.file_version_ue4 >= ue4ver::VER_UE4_ENGINE_VERSION_OBJECT {
            summary.engine_version = r.fstring()?;
            summary.compatible_with_engine_version = r.fstring()?;
        } else {
            let _engine_changelist = r.i32()?;
        }
        summary.compression_flags = r.u32()?;
        if summary.file_version_ue4 >= 387 {
            // ADDED_PACKAGE_SUMMARY_LOCALIZATION_ID → package source as i32
            summary.package_source = r.u32()?;
        }
        if summary.file_version_ue4 >= ue4ver::VER_UE4_SUMMARY_HAS_ASSET_REGISTRY_DATA {
            summary.asset_registry_data_offset = r.i32()?;
        }
        if summary.legacy_file_version <= -7 {
            summary.bulk_data_start_offset = r.i64()?;
        }
        if summary.file_version_ue4 >= ue4ver::VER_UE4_ADDED_PACKAGE_OWNER {
            let _package_owner = r.fstring()?;
        }
        if summary.file_version_ue4 >= ue4ver::VER_UE4_NON_OUTER_PACKAGE_IMPORT
            && summary.file_version_ue4 < ue4ver::VER_UE4_CORRECT_LICENSEE_FLAG
        {
            let _cook_info_header_offset = r.i32()?;
        }
        if summary.file_version_ue4 >= 516 {
            summary.world_tile_info_data_offset = r.i32()?;
        }
        if summary.file_version_ue4 >= ue4ver::VER_UE4_PAYLOAD_TOC || summary.file_version_ue5 >= ue5ver::PAYLOAD_TOC {
            let chunk_count = r.i32()?;
            for _ in 0..chunk_count.clamp(0, 0x1000) {
                summary.chunk_ids.push(r.i32()?);
            }
        }
        if summary.file_version_ue4 >= ue4ver::VER_UE4_PRELOAD_DEPENDENCIES_IN_PACKAGE_SUMMARY {
            summary.preload_dependency_count = r.i32()?;
            summary.preload_dependency_offset = r.i32()?;
        }
        if summary.file_version_ue4 >= ue4ver::VER_UE4_NAMES_REFERENCED_FROM_EXPORT_DATA
            || summary.file_version_ue5 >= ue5ver::NAMES_REFERENCED_FROM_EXPORT_DATA
        {
            summary.names_referenced_from_export_data_count = r.i32()?;
        }
        if summary.file_version_ue4 >= ue4ver::VER_UE4_PAYLOAD_TOC || summary.file_version_ue5 >= ue5ver::PAYLOAD_TOC {
            summary.payload_toc_offset = r.i32()?;
        }
        if summary.file_version_ue4 >= ue4ver::VER_UE4_DATA_RESOURCES || summary.file_version_ue5 >= ue5ver::DATA_RESOURCES {
            summary.data_resource_offset = r.i32()?;
        }

        // name map
        let mut names = Vec::new();
        if summary.name_count > 0 {
            r.set_position(summary.name_offset as u64);
            let count = summary.name_count as usize;
            for ni in 0..count {
                let name = r.fstring()?;
                let number = if summary.file_version_ue4 >= 514 || summary.is_ue5() {
                    r.i16()? as i32
                } else {
                    0
                };
                names.push(NameEntry { name, number });
            }
        }

        // soft object path list (UE5)
        let mut soft_object_path_list = Vec::new();
        if summary.soft_object_paths_count > 0 && summary.soft_object_paths_offset > 0 {
            r.set_position(summary.soft_object_paths_offset as u64);
            for _ in 0..summary.soft_object_paths_count {
                let object_path = r.fstring()?;
                let class_path = r.fstring()?;
                soft_object_path_list.push((object_path, class_path));
            }
        }
        // imports (FName = index+number pairs resolved via the name map)
        let mut imports = Vec::new();
        if summary.import_count > 0 {
            r.set_position(summary.import_offset as u64);
            for _ in 0..summary.import_count {
                let class_package = read_fname_string(&mut r, &names)?;
                let class_name = read_fname_string(&mut r, &names)?;
                let outer_index = r.i32()?;
                let object_name = read_fname_string(&mut r, &names)?;
                let optional = if summary.is_ue5() && summary.file_version_ue5 >= ue5ver::OPTIONAL_RESOURCES {
                    r.u32()? != 0
                } else {
                    false
                };
                imports.push(FObjectImport {
                    class_package,
                    class_name,
                    outer_index,
                    object_name,
                    optional,
                });
            }
        }
        // exports
        let mut exports = Vec::new();
        if summary.export_count > 0 {
            r.set_position(summary.export_offset as u64);
            for _ in 0..summary.export_count {
                let mut e = FObjectExport::default();
                e.class_index = r.i32()?;
                e.super_index = r.i32()?;
                if summary.is_ue5() && summary.file_version_ue5 >= ue5ver::VERSE_CELLS {
                    e.template_index = r.i32()?;
                }
                e.outer_index = r.i32()?;
                e.object_name = read_fname_string(&mut r, &names)?;
                e.save = r.u32()?;
                e.serial_size = r.i64()?;
                e.serial_offset = r.i64()?;
                e.forced_export = r.bool32()?;
                e.not_for_client = r.bool32()?;
                e.not_for_server = r.bool32()?;
                if !summary.is_ue5() || summary.file_version_ue5 < ue5ver::REMOVE_OBJECT_EXPORT_PACKAGE_GUID {
                    e.package_guid = r.read_bytes(16)?.try_into().unwrap();
                }
                e.package_flags = r.u32()?;
                if !summary.is_ue5() || summary.file_version_ue5 < ue5ver::TRACK_OBJECT_EXPORT_IS_INHERITED {
                    e.not_always_loaded_for_editor_game = r.bool32()?;
                }
                e.is_asset = r.bool32()?;
                if summary.file_version_ue4 >= ue4ver::VER_UE4_PRELOAD_DEPENDENCIES_IN_PACKAGE_SUMMARY {
                    e.first_export_dependency = r.i32()?;
                    e.serialization_before_serialization_dependencies = r.bool32()?;
                    e.create_before_serialization_dependencies = r.bool32()?;
                    e.serialization_before_create_dependencies = r.bool32()?;
                    e.create_before_create_dependencies = r.bool32()?;
                }
                exports.push(e);
            }
        }

        Ok(UAssetPackage {
            summary,
            names,
            imports,
            exports,
            soft_object_path_list,
            asset_data: asset,
            exp_data: exp,
            bulk_data: bulk,
            bulk_data_start_offset: 0,
        })
    }

    pub fn name(&self, index: i32) -> Option<&str> {
        if index < 0 || index as usize >= self.names.len() {
            return None;
        }
        Some(&self.names[index as usize].name)
    }

    /// Object path for an import/export package index (FPropertyTag-style index).
    /// Positive = export (index-1), negative = import (-index-1).
    pub fn object_path(&self, index: i32) -> Option<String> {
        if index == 0 {
            return None;
        }
        if index > 0 {
            let e = self.exports.get(index as usize - 1)?;
            Some(e.object_name.clone())
        } else {
            let i = self.imports.get((-index as usize) - 1)?;
            Some(format!("{}.{}", i.class_package, i.object_name))
        }
    }

    pub fn import_display(&self, index: i32) -> Option<String> {
        let i = self.imports.get((-index as usize).checked_sub(1)?)?;
        Some(format!("{}/{}", i.class_name, i.object_name))
    }

    /// Combined stream: .uasset then .uexp (serial_offset is relative to the start of uexp data
    /// for packages with "editor only data stripped"; in cooked packages exports live in .uexp).
    /// Returns the byte slice for an export's serialized data.
    /// For cooked (filter-editor-only) packages exports live in .uexp at serial_offset;
    /// otherwise they are embedded in the .uasset at serial_offset.
    pub fn export_bytes(&self, export_index: usize) -> Option<&[u8]> {
        let e = self.exports.get(export_index)?;
        let offset = e.serial_offset as usize;
        let size = e.serial_size as usize;
        if self.exports_in_uexp() {
            let exp = self.exp_data.as_ref()?;
            return exp.get(offset..(offset + size).min(exp.len()));
        }
        let total = self.asset_data.len();
        if let Some(exp) = &self.exp_data {
            if offset >= total {
                let rel = offset - total;
                return exp.get(rel..(rel + size).min(exp.len()));
            }
        }
        self.asset_data.get(offset..(offset + size).min(total))
    }

    /// Access to .ubulk bytes
    pub fn bulk(&self) -> Option<&[u8]> {
        self.bulk_data.as_deref()
    }
}
