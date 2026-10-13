//! Zen package header parsing (UE5 / UE4.25+ IOStore packages):
//! summary, versioning info, name map, import/export maps, export bundles.

use super::{is_new, ChunkId, ChunkType, IoStoreReader, TocVersion};
use crate::error::{Error, Result};
use crate::io::UexRead;
use std::io::{Cursor, Read, Seek, SeekFrom};

// ---- container header -------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(i32)]
pub enum ContainerHeaderVersion {
    PreInitial = -1,
    Initial = 0,
    LocalizedPackages = 1,
    OptionalSegmentPackages = 2,
    NoExportInfo = 3,
    SoftPackageReferences = 4,
    SoftPackageReferencesOffset = 5,
}

#[derive(Debug, Clone, Default)]
pub struct StoreEntry {
    pub export_bundles_size: u64,
    pub load_order: u32,
    pub export_count: i32,
    pub export_bundle_count: i32,
    pub imported_packages: Vec<u64>,
}

/// Parsed container header (from the ContainerHeader chunk).
pub struct ContainerHeader {
    pub version: ContainerHeaderVersion,
    pub container_id: u64,
    /// package id → store entry
    pub packages: std::collections::BTreeMap<u64, StoreEntry>,
}

impl ContainerHeader {
    pub fn parse(data: &[u8]) -> Result<ContainerHeader> {
        let mut s = Cursor::new(data);
        let signature = s.u32()?;
        let version;
        let container_id;
        if signature != super::CONTAINER_HEADER_MAGIC {
            // PreInitial: header begins directly with container id
            version = ContainerHeaderVersion::PreInitial;
            let mut id = [0u8; 8];
            id[0..4].copy_from_slice(&signature.to_le_bytes());
            id[4..8].copy_from_slice(&UexRead::read_bytes(&mut s, 4)?);
            container_id = u64::from_le_bytes(id);
        } else {
            version = match s.i32()? {
                0 => ContainerHeaderVersion::Initial,
                1 => ContainerHeaderVersion::LocalizedPackages,
                2 => ContainerHeaderVersion::OptionalSegmentPackages,
                3 => ContainerHeaderVersion::NoExportInfo,
                4 => ContainerHeaderVersion::SoftPackageReferences,
                5 => ContainerHeaderVersion::SoftPackageReferencesOffset,
                v => return Err(Error::unsupported(format!("container header version {v}"))),
            };
            container_id = s.u64()?;
        }

        if version < ContainerHeaderVersion::OptionalSegmentPackages {
            let _package_count = s.u32()?;
        }

        if version <= ContainerHeaderVersion::Initial {
            let _names_buffer: Vec<u8> = read_len_prefixed(&mut s)?;
            let _name_hashes: Vec<u8> = read_len_prefixed(&mut s)?;
        }

        let packages = Self::parse_store_entries(&mut s, version)?;

        Ok(ContainerHeader {
            version,
            container_id,
            packages,
        })
    }

    fn parse_store_entries(s: &mut Cursor<&[u8]>, version: ContainerHeaderVersion) -> Result<std::collections::BTreeMap<u64, StoreEntry>> {
        let count = s.u32()? as usize;
        let mut package_ids = Vec::with_capacity(count);
        for _ in 0..count {
            package_ids.push(s.u64()?);
        }
        let buffer = read_len_prefixed(s)?;
        let (member_offset, entry_size): (usize, usize) = match version {
            ContainerHeaderVersion::PreInitial => (8, 16),
            ContainerHeaderVersion::Initial => (24, 32),
            ContainerHeaderVersion::LocalizedPackages | ContainerHeaderVersion::OptionalSegmentPackages => (8, 24),
            _ => (0, 16),
        };
        let mut entries_raw = Vec::with_capacity(count);
        {
            let mut cur = Cursor::new(&buffer);
            for _ in 0..count {
                let mut raw = vec![0u8; entry_size];
                cur.read_exact(&mut raw)?;
                entries_raw.push(raw);
            }
        }

        let mut packages = std::collections::BTreeMap::new();
        for (i, package_id) in package_ids.into_iter().enumerate() {
            let raw = &entries_raw[i];
            let mut cur = Cursor::new(raw);
            let entry = match version {
                ContainerHeaderVersion::Initial => {
                    let export_bundles_size = cur.u64()?;
                    let export_count = cur.i32()?;
                    let export_bundle_count = cur.i32()?;
                    let load_order = cur.u32()?;
                    let _pad = cur.u32()?;
                    let imported = read_imported_array(&buffer, i * entry_size + member_offset)?;
                    StoreEntry {
                        export_bundles_size,
                        load_order,
                        export_count,
                        export_bundle_count,
                        imported_packages: imported,
                    }
                }
                ContainerHeaderVersion::PreInitial
                | ContainerHeaderVersion::LocalizedPackages
                | ContainerHeaderVersion::OptionalSegmentPackages => {
                    let export_count = cur.i32()?;
                    let export_bundle_count = cur.i32()?;
                    let imported_view = ImportedView {
                        offset_to_data: cur.i32()?,
                        array_num: cur.i32()?,
                    };
                    let _shader_view = ImportedView {
                        offset_to_data: cur.i32()?,
                        array_num: cur.i32()?,
                    };
                    let imported = if imported_view.array_num > 0 {
                        let base = i * entry_size + member_offset + imported_view.offset_to_data as usize;
                        let mut bc = Cursor::new(&buffer);
                        bc.seek(SeekFrom::Start(base as u64))?;
                        let n = imported_view.array_num as usize;
                        let mut v = Vec::with_capacity(n);
                        for _ in 0..n {
                            v.push(bc.u64()?);
                        }
                        v
                    } else {
                        vec![]
                    };
                    StoreEntry {
                        export_bundles_size: 0,
                        load_order: 0,
                        export_count,
                        export_bundle_count,
                        imported_packages: imported,
                    }
                }
                ContainerHeaderVersion::NoExportInfo
                | ContainerHeaderVersion::SoftPackageReferences
                | ContainerHeaderVersion::SoftPackageReferencesOffset => {
                    let export_bundles_size = cur.u64()?;
                    let load_order = cur.u32()?;
                    let imported_view = ImportedView {
                        offset_to_data: cur.i32()?,
                        array_num: cur.i32()?,
                    };
                    let _shader_view = ImportedView {
                        offset_to_data: cur.i32()?,
                        array_num: cur.i32()?,
                    };
                    let imported = if imported_view.array_num > 0 {
                        let base = i * entry_size + imported_view.offset_to_data as usize;
                        let mut bc = Cursor::new(&buffer);
                        bc.seek(SeekFrom::Start(base as u64))?;
                        let n = imported_view.array_num as usize;
                        let mut v = Vec::with_capacity(n);
                        for _ in 0..n {
                            v.push(bc.u64()?);
                        }
                        v
                    } else {
                        vec![]
                    };
                    StoreEntry {
                        export_bundles_size,
                        load_order,
                        export_count: -1,
                        export_bundle_count: -1,
                        imported_packages: imported,
                    }
                }
            };
            packages.insert(package_id, entry);
        }
        Ok(packages)
    }

    pub fn get(&self, package_id: u64) -> Option<&StoreEntry> {
        self.packages.get(&package_id)
    }
}

#[derive(Debug, Clone, Copy)]
struct ImportedView {
    offset_to_data: i32,
    array_num: i32,
}

fn read_imported_array(buffer: &[u8], base: usize) -> Result<Vec<u64>> {
    let mut bc = Cursor::new(buffer);
    bc.seek(SeekFrom::Start(base as u64))?;
    let count = bc.i32()?;
    if count <= 0 {
        return Ok(vec![]);
    }
    let mut v = Vec::with_capacity(count as usize);
    for _ in 0..count {
        v.push(bc.u64()?);
    }
    Ok(v)
}

fn read_len_prefixed<R: AsRef<[u8]>>(s: &mut Cursor<R>) -> Result<Vec<u8>> {
    let len = s.u32()? as usize;
    UexRead::read_bytes(s, len)
}

// ---- zen package ------------------------------------------------------------

/// Parsed zen package (the ExportBundleData chunk of a package).
pub struct ZenPackage {
    pub package_name: String,
    pub has_versioning_info: bool,
    pub file_version_ue4: i32,
    pub file_version_ue5: i32,
    pub licensee_version: i32,
    pub custom_versions: Vec<([u8; 16], i32)>,
    pub is_unversioned: bool,
    pub import_map: Vec<u64>,
    pub exports: Vec<ZenExport>,
    pub export_bundle_entries: Vec<(u32, u32)>, // (local_export_index, command_type)
    /// raw chunk bytes (exports reference into this)
    pub data: Vec<u8>,
    pub local_names: Vec<String>,
    pub container_header_version: i32,
}

#[derive(Debug, Clone)]
pub struct ZenExport {
    pub cooked_serial_offset: u64,
    pub cooked_serial_size: u64,
    pub object_name: String,
    pub class_name: String,
    pub outer_index: u64,
    pub super_index: u64,
    pub object_flags: u32,
    /// data range within the package chunk
    pub data: (u64, u64),
}

fn resolve_idx_name(
    reader: &IoStoreReader,
    index: u64,
    local_names: &[String],
) -> String {
    // FPackageObjectIndex: top 2 bits = kind (Export=0, ScriptImport=1, PackageImport=2, Null=3)
    let kind = (index >> 62) as u64;
    if kind != 0 {
        return format!("<{:?}>", match kind {
            1 => "ScriptImport",
            2 => "PackageImport",
            _ => "Null",
        });
    }
    // export index → export map lookup happens in caller; here treat as export index
    let _ = reader;
    let _ = local_names;
    format!("#export_{}", index)
}

impl ZenPackage {
    /// Parse the ExportBundleData chunk of a package.
    pub fn parse(reader: &IoStoreReader, chunk: &ChunkId, store_entry: Option<&StoreEntry>) -> Result<ZenPackage> {
        let data = reader.read_chunk_id(chunk)?;
        let container_header_version = reader.container_header_version();
        Self::parse_data(data, container_header_version, store_entry, reader, chunk.package_id())
    }

    pub fn parse_data(
        data: Vec<u8>,
        container_header_version: i32,
        store_entry: Option<&StoreEntry>,
        reader: &IoStoreReader,
        package_id: u64,
    ) -> Result<ZenPackage> {
        let mut s = Cursor::new(&data[..]);
        let chv = match container_header_version {
            0 => ContainerHeaderVersion::Initial,
            1 => ContainerHeaderVersion::LocalizedPackages,
            2 => ContainerHeaderVersion::OptionalSegmentPackages,
            3 => ContainerHeaderVersion::NoExportInfo,
            4 => ContainerHeaderVersion::SoftPackageReferences,
            5 => ContainerHeaderVersion::SoftPackageReferencesOffset,
            _ => ContainerHeaderVersion::OptionalSegmentPackages,
        };
        let mut s = Cursor::new(&data);

        // summary
        let mut has_versioning_info = 0u32;
        let mut header_size = 0u32;
        if chv > ContainerHeaderVersion::Initial {
            has_versioning_info = s.u32()?;
            header_size = s.u32()?;
        }
        let name_map_entry = s.u32()?;
        let name_map_number = s.u32()?;
        if chv <= ContainerHeaderVersion::Initial {
            let _source_name_index = s.u32()?;
            let _source_name_number = s.u32()?;
        }
        let package_flags = s.u32()?;
        let _cooked_header_size = s.u32()?;
        let mut imported_public_export_hashes_offset = -1i32;
        let mut name_map_names_offset = -1i32;
        let mut name_map_names_size = -1i32;
        let mut _name_map_hashes_offset = -1i32;
        let mut _name_map_hashes_size = -1i32;
        if chv <= ContainerHeaderVersion::Initial {
            name_map_names_offset = s.i32()?;
            name_map_names_size = s.i32()?;
            _name_map_hashes_offset = s.i32()?;
            _name_map_hashes_size = s.i32()?;
        } else {
            imported_public_export_hashes_offset = s.i32()?;
        }
        let import_map_offset = s.i32()?;
        let export_map_offset = s.i32()?;
        let export_bundle_entries_offset = s.i32()?;
        let mut dependency_bundle_headers_offset = -1i32;
        let mut dependency_bundle_entries_offset = -1i32;
        let mut imported_package_names_offset = -1i32;
        let mut graph_data_offset = -1i32;
        if chv >= ContainerHeaderVersion::NoExportInfo {
            dependency_bundle_headers_offset = s.i32()?;
            dependency_bundle_entries_offset = s.i32()?;
            imported_package_names_offset = s.i32()?;
        } else {
            graph_data_offset = s.i32()?;
        }
        let mut graph_data_size = -1i32;
        if chv <= ContainerHeaderVersion::Initial {
            graph_data_size = s.i32()?;
            let _pad = s.i32()?;
            header_size = (graph_data_offset + graph_data_size) as u32;
        }

        // versioning info
        let mut file_version_ue4 = 0i32;
        let mut file_version_ue5 = 0i32;
        let mut licensee_version = 0i32;
        let mut custom_versions = Vec::new();
        let mut is_unversioned = true;
        if has_versioning_info != 0 {
            let _zen_version = s.u32()?;
            file_version_ue4 = s.i32()?;
            file_version_ue5 = s.i32()?;
            licensee_version = s.i32()?;
            let cv_count = s.u32()? as usize;
            for _ in 0..cv_count {
                let key: [u8; 16] = s.read_bytes(16)?.try_into().unwrap();
                let ver = s.i32()?;
                custom_versions.push((key, ver));
            }
            is_unversioned = false;
        }

        // cell maps for UE5.6+ (SoftPackageReferencesOffset header version)
        let (cell_import_map_offset, cell_export_map_offset) = if chv >= ContainerHeaderVersion::SoftPackageReferencesOffset {
            (s.i32()?, s.i32()?)
        } else {
            (export_bundle_entries_offset, export_bundle_entries_offset)
        };

        // local name map (UE5: right after summary/versioning/cell offsets; UE4.27: at offsets in summary)
        let local_names: Vec<String> = if chv > ContainerHeaderVersion::Initial {
            read_name_batch(&mut s)?
        } else {
            let start = name_map_names_offset as u64;
            s.seek(SeekFrom::Start(start))?;
            let names_buffer = read_len_prefixed(&mut s)?;
            read_name_batch_parts(&names_buffer)?
        };

        // bulk data map (UE5.4+ with padding)
        let version_for_bulk = if file_version_ue5 > 0 { file_version_ue5 } else { i32::MAX };
        let has_bulk_data = if file_version_ue5 > 0 {
            file_version_ue5 >= 1009 // DataResources
        } else {
            chv >= ContainerHeaderVersion::OptionalSegmentPackages
        };
        if has_bulk_data {
            if version_for_bulk >= 1012 {
                // padding count + bytes (UE5.4+)
                let pad_count = s.u64()?;
                for _ in 0..pad_count {
                    s.u8()?;
                }
            }
            let bulk_map_size = s.i64()?;
            let n = bulk_map_size as usize / 36;
            for _ in 0..n {
                let _e = s.read_bytes(36)?;
            }
        }

        // imported public export hashes
        let _hashes_count = (import_map_offset - imported_public_export_hashes_offset).max(0) as usize / 8;

        // import map
        s.seek(SeekFrom::Start(import_map_offset as u64))?;
        let import_count = ((export_map_offset - import_map_offset) as usize) / 8;
        let mut import_map = Vec::with_capacity(import_count);
        for _ in 0..import_count {
            import_map.push(s.u64()?);
        }

        // export map
        let export_count = ((cell_import_map_offset - export_map_offset) as usize) / 72;
        s.seek(SeekFrom::Start(export_map_offset as u64))?;
        let mut exports = Vec::with_capacity(export_count);
        for _ in 0..export_count {
            let cooked_serial_offset = s.u64()?;
            let cooked_serial_size = s.u64()?;
            let object_name_idx = s.u32()?;
            let object_name_num = s.u32()?;
            let outer_index = s.u64()?;
            let class_index = s.u64()?;
            let _super_index = s.u64()?;
            let _template_index = s.u64()?;
            let _public_export_hash = s.u64()?;
            let object_flags = s.u32()?;
            let _filter_flags = s.u8()?;
            let _padding = s.read_bytes(3)?;

            let object_name = resolve_mapped(reader, &local_names, object_name_idx, object_name_num)
                .unwrap_or_else(|| format!("Unknown_{object_name_idx}"));
            let class_name: String = if class_index >> 62 == 0 && (class_index as u32) < export_count as u32 {
                exports
                    .get(class_index as u32 as usize)
                    .map(|e: &ZenExport| e.object_name.clone())
                    .unwrap_or_else(|| "Unknown".to_string())
            } else if class_index >> 62 == 2 {
                // package import: [package_index:32 | hash_index:32]
                let pkg_i = ((class_index & ((1u64 << 62) - 1)) >> 32) as usize;
                format!("#pkg_import_{pkg_i}")
            } else {
                resolve_idx_name(reader, class_index, &local_names)
            };

            exports.push(ZenExport {
                cooked_serial_offset,
                cooked_serial_size,
                object_name,
                class_name,
                outer_index,
                super_index: _super_index,
                object_flags,
                data: (cooked_serial_offset, cooked_serial_offset + cooked_serial_size),
            });
        }

        // export bundle entries
        s.seek(SeekFrom::Start(export_bundle_entries_offset as u64))?;
        let entry_count = if chv >= ContainerHeaderVersion::LocalizedPackages {
            let end = if dependency_bundle_headers_offset > 0 {
                dependency_bundle_headers_offset
            } else {
                graph_data_offset
            };
            ((end - export_bundle_entries_offset) as usize) / 8
        } else {
            store_entry.map(|se| se.export_bundle_count).unwrap_or(0) as usize * 2
        };
        let mut export_bundle_entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            let local_export_index = s.u32()?;
            let command_type = s.u32()?;
            export_bundle_entries.push((local_export_index, command_type));
        }

        // package name
        let package_name = resolve_mapped(reader, &local_names, name_map_entry, name_map_number)
            .unwrap_or_else(|| format!("Package_{package_id:x}"));
        let _ = package_flags;

        Ok(ZenPackage {
            package_name,
            has_versioning_info: has_versioning_info != 0,
            file_version_ue4,
            file_version_ue5,
            licensee_version,
            custom_versions,
            is_unversioned,
            import_map,
            exports,
            export_bundle_entries,
            data,
            local_names,
            container_header_version,
        })
    }

    /// Byte range of an export's serialized data within this package chunk.
    pub fn export_data(&self, export_index: usize) -> Option<&[u8]> {
        let e = self.exports.get(export_index)?;
        let start = e.cooked_serial_offset as usize;
        let end = start + e.cooked_serial_size as usize;
        self.data.get(start..end)
    }
}

fn resolve_mapped(reader: &IoStoreReader, local_names: &[String], index_and_type: u32, number: u32) -> Option<String> {
    const INDEX_BITS: u32 = 30;
    let index = index_and_type & ((1 << INDEX_BITS) - 1);
    let ty = index_and_type >> INDEX_BITS;
    let name = match ty {
        2 => reader.resolve_global_name(index),
        _ => {
            if let Some(n) = local_names.get(index as usize) {
                Some(n.clone())
            } else {
                reader.resolve_global_name(index)
            }
        }
    }?;
    Some(if number == 0 {
        name
    } else {
        format!("{name}_{}", number - 1)
    })
}

pub fn read_name_batch<R: AsRef<[u8]>>(s: &mut Cursor<R>) -> Result<Vec<String>> {
    let num = s.u32()? as usize;
    if num == 0 {
        return Ok(vec![]);
    }
    let _num_string_bytes = s.u32()?;
    let _hash_version = s.u64()?;
    let _hash_bytes = s.read_bytes(num * 8)?;
    let mut lengths = Vec::with_capacity(num);
    for _ in 0..num {
        // name lengths are big-endian i16 in the UE global name batch format
        let b = s.read_bytes(2)?;
        lengths.push(i16::from_be_bytes([b[0], b[1]]));
    }
    let mut names = Vec::with_capacity(num);
    for l in lengths {
        let len = if l < 0 { (i16::MIN as i32).wrapping_sub(l as i32) } else { l as i32 };
        let data = s.read_bytes(len as usize)?;
        if l < 0 {
            let units: Vec<u16> = data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
            names.push(String::from_utf16_lossy(&units[..end]));
        } else {
            let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
            names.push(String::from_utf8_lossy(&data[..end]).into_owned());
        }
    }
    Ok(names)
}

fn read_name_batch_parts(buf: &[u8]) -> Result<Vec<String>> {
    let mut names = vec![];
    let mut s = Cursor::new(buf);
    while (s.position() as usize) < buf.len() {
        let l = s.i16()?;
        let len = if l < 0 { (i16::MIN as i32).wrapping_sub(l as i32) } else { l as i32 };
        if l < 0 && s.position() % 2 != 0 {
            s.u8()?;
        }
        let data = s.read_bytes(len as usize)?;
        if l < 0 {
            let units: Vec<u16> = data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
            names.push(String::from_utf16_lossy(&units[..end]));
        } else {
            let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
            names.push(String::from_utf8_lossy(&data[..end]).into_owned());
        }
    }
    Ok(names)
}

/// Convenience: find the ExportBundleData chunk for a package.
pub fn export_bundle_chunk(reader: &IoStoreReader, package_id: u64) -> Option<ChunkId> {
    let new = is_new(reader.toc.version);
    let t = ChunkType::ExportBundleData;
    let id = ChunkId::new(package_id, 0, t, new);
    reader.toc.chunk_index(&id).map(|_| id)
}
