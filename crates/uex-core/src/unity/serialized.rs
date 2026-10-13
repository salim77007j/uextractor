//! Unity SerializedFile reader: the object container format used by .assets,
//! level*/sharedassets*/globalgamemanagers files and CAB-* entries inside
//! AssetBundles. Handles serialized file versions 3..=26 (Unity 3.4 → 6000.x),
//! big-endian files, 32/64-bit path ids, embedded typetrees (v10/v12/23 blob +
//! legacy) and TPK-based release trees for stripped typetrees.

use super::typetree::{self, Node, UValue};
use super::{tpk, UnityVersion};
use crate::error::{Error, Result};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Header {
    pub metadata_size: u32,
    pub file_size: u64,
    pub version: u32,
    pub data_offset: u64,
    pub big_endian: bool,
}

#[derive(Debug, Clone)]
pub struct SerializedType {
    pub class_id: i32,
    pub is_stripped: bool,
    pub script_type_index: i16,
    pub script_id: Option<[u8; 16]>,
    pub old_type_hash: [u8; 16],
    /// embedded tree when the file carries typetrees
    pub node: Option<Node>,
    // ref type extras
    pub class_name: String,
    pub namespace: String,
    pub assembly_name: String,
}

#[derive(Debug, Clone)]
pub struct ObjectInfo {
    pub path_id: i64,
    pub byte_start: u64,
    pub byte_size: u64,
    pub type_id: i32,
    pub class_id: i32,
}

#[derive(Debug, Clone)]
pub struct External {
    pub path: String,
    pub guid: [u8; 16],
    pub ext_type: i32,
}

/// A parsed Unity SerializedFile. Objects are parsed lazily on demand.
pub struct SerializedFile {
    pub header: Header,
    pub unity_version: UnityVersion,
    pub unity_version_raw: String,
    pub target_platform: i32,
    pub enable_type_tree: bool,
    pub types: Vec<SerializedType>,
    pub objects: Vec<ObjectInfo>,
    pub externals: Vec<External>,
    /// container map from the AssetBundle object (friendly path -> PPtr), filled
    /// lazily by the caller when the AssetBundle object is parsed
    pub container: Vec<(String, (i64, i64))>,
    /// full file bytes (owned; bundles extract node bytes into this)
    data: Vec<u8>,
}

impl SerializedFile {
    pub fn parse(data: Vec<u8>) -> Result<SerializedFile> {
        // The serialized-file HEADER (4x u32 + endian byte + reserved + v22
        // extension) is always BIG-endian (AssetStudio FileReader default and
        // UnityPy's default reader both prove this against real files). The
        // metadata after it follows the endian flag byte (LE on PC).
        let mut r = typetree::Reader::new(&data, true);
        let metadata_size = r.read_u32()?;
        let file_size32 = r.read_u32()?;
        let version = r.read_u32()?;
        let mut data_offset = if version >= 9 { r.read_u32()? as u64 } else { 0 };

        let big_endian;
        let mut file_size = file_size32 as u64;
        let mut meta_start; // reader position where the metadata begins
        if version >= 9 {
            big_endian = r.read_bool()?;
            r.align(4);
            if version >= 22 {
                let _metadata_size = r.read_u32()?;
                file_size = r.read_u64()?;
                data_offset = r.read_u64()?;
                let _unknown = r.read_u64()?;
            }
            meta_start = r.pos;
        } else {
            // metadata at the end
            let pos = file_size32 as usize - metadata_size as usize;
            let endian_byte = *data
                .get(pos)
                .ok_or_else(|| Error::malformed("serialized file truncated (endian)"))?;
            big_endian = endian_byte != 0;
            meta_start = pos + 1; // after the endian byte
        }

        // metadata reader with the actual endianness
        let mut r = typetree::Reader::new(&data, big_endian);
        r.pos = meta_start;
        if std::env::var("UEX_DEBUG_SF").is_ok() { eprintln!("[sf] meta_start={meta_start} be={big_endian} ver={version}"); }

        let mut unity_version_raw = String::from("2.5.0f5");
        if version >= 7 {
            let s = r.read_string_to_null()?;
            if !s.is_empty() && s != "0.0.0" {
                unity_version_raw = s;
            }
        }
        let mut target_platform = 0;
        if version >= 8 {
            target_platform = r.read_i32()?;
        }
        let enable_type_tree = if version >= 13 { r.read_bool()? } else { true };

        let mut types = Vec::new();
        let type_count = r.read_i32()?;
        if std::env::var("UEX_DEBUG_SF").is_ok() { eprintln!("[sf] pos={} tc={type_count}", r.pos); }
        if type_count < 0 || type_count > 100_000 {
            return Err(Error::malformed(format!("unreasonable type count {type_count}")));
        }
        for _ in 0..type_count {
            types.push(parse_serialized_type(&mut r, version, enable_type_tree, false)?);
        }

        let big_id_enabled = if (7..14).contains(&version) { r.read_i32()? } else { 0 };

        let object_count = r.read_i32()?;
        if object_count < 0 || object_count > 5_000_000 {
            return Err(Error::malformed(format!("unreasonable object count {object_count}")));
        }
        let mut objects = Vec::with_capacity(object_count.min(1 << 16) as usize);
        for _ in 0..object_count {
            let mut var_type_id: i32;
            let path_id = if big_id_enabled != 0 {
                r.read_i64()?
            } else if version < 14 {
                r.read_i32()? as i64
            } else {
                r.align(4);
                r.read_i64()?
            };
            let byte_start = if version >= 22 { r.read_u64()? } else { r.read_u32()? as u64 };
            let byte_start = byte_start + data_offset;
            let byte_size = r.read_u32()? as u64;
            let type_id = r.read_i32()?;
            let class_id;
            if version < 16 {
                class_id = r.read_u16()? as i32;
                // v<16: types are matched by class id, not by array index.
                // Remap type_id to the matching entry so later lookups work.
                let mut found = type_id;
                for (i, t) in types.iter().enumerate() {
                    if t.class_id == type_id {
                        found = i as i32;
                        break;
                    }
                }
                var_type_id = found;
            } else {
                let t = types
                    .get(type_id as usize)
                    .ok_or_else(|| Error::malformed(format!("object type index {type_id} out of range")))?;
                class_id = t.class_id;
                var_type_id = type_id;
            }
            let type_id = var_type_id;
            if version < 11 {
                let _is_destroyed = r.read_u16()?;
            }
            if (11..17).contains(&version) {
                let _script_type_index = r.read_i16()?;
            }
            if version == 15 || version == 16 {
                let _is_stripped = r.read_u8()?;
            }
            objects.push(ObjectInfo {
                path_id,
                byte_start,
                byte_size,
                type_id,
                class_id,
            });
        }

        if version >= 11 {
            let script_count = r.read_i32()?;
            for _ in 0..script_count {
                // LocalSerializedObjectIdentifier
                let _file_idx = r.read_i32()?;
                if version < 14 {
                    let _local_id = r.read_i32()?;
                } else {
                    r.align(4);
                    let _local_id = r.read_i64()?;
                }
            }
        }

        let external_count = r.read_i32()?;
        let mut externals = Vec::with_capacity(external_count.max(0) as usize);
        for _ in 0..external_count {
            let mut ext = External {
                path: String::new(),
                guid: [0u8; 16],
                ext_type: 0,
            };
            if version >= 6 {
                let _temp_empty = r.read_string_to_null()?;
            }
            if version >= 5 {
                let g = r.read_bytes(16)?;
                ext.guid.copy_from_slice(g);
                ext.ext_type = r.read_i32()?;
            }
            ext.path = r.read_string_to_null()?;
            externals.push(ext);
        }

        if version >= 20 {
            let ref_count = r.read_i32()?;
            for _ in 0..ref_count {
                parse_serialized_type(&mut r, version, enable_type_tree, true)?;
            }
        }

        if version >= 26 {
            // shared subtrees (v23 mhtt handlers)
            let sub_count = r.read_i32()?;
            for _ in 0..sub_count {
                let _hash = r.read_bytes(16)?;
                let size = r.read_u32()? as usize;
                if size >= 8 {
                    let mut sub = typetree::Reader::new(
                        r.read_bytes(size)?,
                        false,
                    );
                    let _node = typetree::parse_mhtt(&mut sub)?;
                }
            }
        }

        if version >= 5 {
            let _user_info = r.read_string_to_null()?;
        }

        let version_parsed = UnityVersion::parse(&unity_version_raw)
            .or_else(|_| UnityVersion::parse("5.0.0f4"))?;

        Ok(SerializedFile {
            header: Header {
                metadata_size,
                file_size,
                version,
                data_offset,
                big_endian,
            },
            unity_version: version_parsed,
            unity_version_raw,
            target_platform,
            enable_type_tree,
            types,
            objects,
            externals,
            container: Vec::new(),
            data,
        })
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn object_bytes(&self, obj: &ObjectInfo) -> Result<&[u8]> {
        let start = obj.byte_start as usize;
        let end = start + obj.byte_size as usize;
        if end > self.data.len() {
            return Err(Error::malformed(format!(
                "object {} data out of bounds ({}..{} > {})",
                obj.path_id, start, end,
                self.data.len()
            )));
        }
        Ok(&self.data[start..end])
    }

    /// Resolve the type tree for an object: embedded tree when available,
    /// otherwise the TPK release tree. Returns None for unknown script types.
    pub fn type_tree(&self, obj: &ObjectInfo) -> Result<Option<Node>> {
        if self.enable_type_tree {
            if let Some(t) = self.types.get(obj.type_id as usize) {
                if let Some(n) = &t.node {
                    return Ok(Some(n.clone()));
                }
            }
            return Err(Error::asset("file has typetrees but this type has none"));
        }
        // stripped: TPK release tree for the class
        tpk::release_tree(obj.class_id, self.unity_version)
    }

    /// Parse one object into a UValue via its type tree.
    pub fn read_object(&self, obj: &ObjectInfo) -> Result<(String, UValue)> {
        let class_name = super::class_name(obj.class_id);
        let bytes = self.object_bytes(obj)?;
        let Some(tree) = self.type_tree(obj)? else {
            return Err(Error::unsupported(format!(
                "no type tree available for {class_name} (class id {}) — script types require embedded trees",
                obj.class_id
            )));
        };
        let big_endian = self.header.big_endian;
        let mut r = typetree::Reader::new(bytes, big_endian);
        let value = typetree::read_value(&tree, &mut r)
            .map_err(|e| Error::asset(format!("{class_name} pid {}: {e}", obj.path_id)))?;
        Ok((class_name, value))
    }

    /// Name for an object: m_Name field if present, else path id.
    pub fn object_name(&self, obj: &ObjectInfo, value: &UValue) -> String {
        let v = value.get("m_Name").or_else(|| value.get("name"));
        match v.and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => format!("obj_{}", obj.path_id),
        }
    }

    /// Build the container map (AssetBundle m_Container) if the object exists.
    pub fn load_container(&mut self) {
        let Some(ab) = self.objects.iter().find(|o| o.class_id == super::class_id::ASSET_BUNDLE).cloned() else {
            return;
        };
        let Ok((_cls, value)) = self.read_object(&ab) else {
            return;
        };
        let Some(container) = value.get("m_Container") else {
            return;
        };
        let Some(entries) = container.as_array() else {
            return;
        };
        let mut map = Vec::new();
        for e in entries {
            // pair(first: string, second: AssetInfo { preloadIndex, preloadSize, asset: PPtr })
            let Some(arr) = e.as_array() else { continue };
            if arr.len() != 2 {
                continue;
            }
            let Some(path) = arr[0].as_str() else { continue };
            let pptr = arr[1].get("asset");
            if let Some(p) = pptr {
                if let UValue::PPtr { file_id, path_id } = p {
                    map.push((path.to_string(), (*file_id, *path_id)));
                }
            }
        }
        self.container = map;
    }

    /// Friendly container path for a (file_id, path_id) pair.
    pub fn container_path(&self, file_id: i64, path_id: i64) -> Option<&str> {
        self.container
            .iter()
            .find(|(_, (fid, pid))| *fid == file_id && *pid == path_id)
            .map(|(p, _)| p.as_str())
    }
}

fn parse_serialized_type(
    r: &mut typetree::Reader,
    version: u32,
    enable_type_tree: bool,
    is_ref_type: bool,
) -> Result<SerializedType> {
    let class_id = r.read_i32()?;
    let is_stripped = if version >= 16 { r.read_bool()? } else { false };
    let script_type_index = if version >= 17 { r.read_i16()? } else { -1 };
    let mut script_id = None;
    let mut old_type_hash = [0u8; 16];
    if version >= 13 {
        let want_script_id = (is_ref_type && script_type_index >= 0)
            || (version < 16 && class_id < 0)
            || (version >= 16 && class_id == 114);
        if want_script_id {
            let b = r.read_bytes(16)?;
            let mut id = [0u8; 16];
            id.copy_from_slice(b);
            script_id = Some(id);
        }
        let h = r.read_bytes(16)?;
        old_type_hash.copy_from_slice(h);
    }
    let mut node = None;
    let (mut class_name, mut namespace, mut assembly_name) = (String::new(), String::new(), String::new());
    if enable_type_tree {
        if version >= 23 {
            let hash = r.read_bytes(16)?.to_vec();
            let size = r.read_u32()? as usize;
            if size >= 8 {
                let blob = r.read_bytes(size)?;
                let mut sub = typetree::Reader::new(blob, false);
                node = Some(typetree::parse_mhtt(&mut sub)?);
            }
            let _ = hash;
        } else if version >= 12 || version == 10 {
            node = Some(typetree::parse_blob(r, version)?);
        } else {
            node = Some(typetree::parse_legacy(r, version)?);
        }
        if version >= 21 {
            if is_ref_type {
                class_name = r.read_string_to_null()?;
                namespace = r.read_string_to_null()?;
                assembly_name = r.read_string_to_null()?;
            } else {
                let dep_count = r.read_i32()?;
                let deps = r.read_bytes(dep_count.max(0) as usize * 4)?;
                let _ = deps;
            }
        }
    }
    Ok(SerializedType {
        class_id,
        is_stripped,
        script_type_index,
        script_id,
        old_type_hash,
        node,
        class_name,
        namespace,
        assembly_name,
    })
}

/// Lookup helpers used by converters.
impl SerializedFile {
    pub fn find_object(&self, path_id: i64) -> Option<&ObjectInfo> {
        self.objects.iter().find(|o| o.path_id == path_id)
    }

    /// Resolve a PPtr within this file (file_id 0). Returns (info, value).
    pub fn resolve_pptr(&self, file_id: i64, path_id: i64) -> Option<(&ObjectInfo, String)> {
        if file_id != 0 {
            return None;
        }
        self.find_object(path_id).map(|o| (o, super::class_name(o.class_id)))
    }

    pub fn type_tree_map(&self) -> HashMap<i32, &Node> {
        self.types
            .iter()
            .filter_map(|t| t.node.as_ref().map(|n| (t.class_id, n)))
            .collect()
    }
}
