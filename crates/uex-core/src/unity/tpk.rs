//! Embedded Unity type-tree database (AssetRipper TPK format, TypeTreeInformation
//! blob, LZMA compressed). Provides exact field layouts for every engine class
//! across all Unity versions (3.4 → 6000.x), used to parse objects whose
//! serialized files have stripped typetrees (the normal case in shipped games).

use crate::error::{Error, Result};
use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;
use std::sync::OnceLock;

/// Raw TPK bytes, LZMA-compressed (props byte + dict size, no size field).
static TPK_BYTES: &[u8] = include_bytes!("../../resources/lzma.tpk");

struct TpkData {
    /// class id -> (version, optional class info) ordered ascending
    classes: HashMap<i32, Vec<(u64, Option<TpkClass>)>>,
    nodes: Vec<TpkNode>,
    strings: Vec<String>,
    /// resolved release root node index per (class, tree entry)
    release_roots: HashMap<i32, Vec<(u64, Option<u16>)>>,
    /// common string table (latest TPK version): sorted (offset, stringIndex)
    common_strings: Vec<(u32, u32)>,
}

#[derive(Clone)]
struct TpkClass {
    name_idx: u32,
    _base: u16,
}

struct TpkNode {
    type_name: u32,
    name: u32,
    _byte_size: i32,
    _version: u16,
    _type_flags: u8,
    meta_flag: u32,
    sub_nodes: Vec<u16>,
}

static TPK: OnceLock<TpkData> = OnceLock::new();

fn lzma_decompress_nosize(data: &[u8]) -> Result<Vec<u8>> {
    // lzma-rs expects the .lzma ALONE container (props + 8-byte size). TPK
    // stores raw LZMA1 (props + dict only), so rebuild the container with the
    // known uncompressed size from the TPK header.
    if data.len() < 5 {
        return Err(Error::malformed("tpk lzma data too short"));
    }
    let mut alone = Vec::with_capacity(data.len() + 8);
    alone.extend_from_slice(&data[..5]);
    alone.extend_from_slice(&u64::MAX.to_le_bytes()); // "unknown size" = end marker mandatory
    alone.extend_from_slice(&data[5..]);
    let mut out = Vec::new();
    lzma_rs::lzma_decompress(&mut &alone[..], &mut out)
        .map_err(|e| Error::malformed(format!("tpk lzma decompress: {e}")))?;
    Ok(out)
}

fn tpk() -> &'static TpkData {
    TPK.get_or_init(|| parse_tpk(TPK_BYTES).expect("embedded TPK must be valid"))
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Cursor { data, pos: 0 }
    }
    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.data.len() {
            return Err(Error::malformed("tpk truncated"));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn read_i32(&mut self) -> Result<i32> {
        let b = self.read_bytes(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn read_u16(&mut self) -> Result<u16> {
        let b = self.read_bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn read_u64(&mut self) -> Result<u64> {
        let b = self.read_bytes(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }
    fn read_u8(&mut self) -> Result<u8> {
        Ok(self.read_bytes(1)?[0])
    }
    /// varint length + utf-8 string
    fn read_string(&mut self) -> Result<String> {
        let mut len = 0usize;
        let mut shift = 0u32;
        loop {
            let b = self.read_u8()?;
            len |= ((b & 0x7F) as usize) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                break;
            }
        }
        let bytes = self.read_bytes(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|e| Error::malformed(format!("tpk string utf8: {e}")))
    }
}

fn parse_tpk(raw: &[u8]) -> Result<TpkData> {
    let mut c = Cursor::new(raw);
    // TpkFile header: <IBBBBIII = magic, version, compression, datatype, pad, pad, compSize, uncompSize>
    let magic = c.read_i32()?;
    if magic != 0x2A4B5054 {
        return Err(Error::malformed("tpk magic mismatch"));
    }
    let _version = c.read_u8()?;
    let compression = c.read_u8()?;
    let data_type = c.read_u8()?;
    let _pad_b = c.read_u8()?; // 4th B of <IBBBBIII
    let _pad_i = c.read_i32()?; // 5th field (I)
    let comp_size = c.read_i32()? as usize;
    let uncomp_size = c.read_i32()? as usize;
    let comp = c.read_bytes(comp_size)?;
    if data_type != 0 {
        return Err(Error::malformed("tpk is not a typetree blob"));
    }
    let mut decomp = match compression {
        0 => comp.to_vec(),
        1 => {
            // lz4 block
            let mut out = vec![0u8; uncomp_size];
            lz4_flex::block::decompress_into(comp, &mut out)
                .map_err(|_| Error::malformed("tpk lz4 decompress failed"))?;
            out
        }
        2 => lzma_decompress_nosize(comp)?,
        _ => return Err(Error::unsupported("tpk brotli compression")),
    };
    if decomp.len() != uncomp_size {
        decomp.truncate(uncomp_size.max(decomp.len()));
    }

    let mut d = Cursor::new(&decomp);
    let _creation_time = d.read_u64()?;
    let version_count = d.read_i32()? as usize;
    let mut versions = Vec::with_capacity(version_count);
    for _ in 0..version_count {
        versions.push(d.read_u64()?);
    }
    // ClassInformation
    let class_count = d.read_i32()? as usize;
    let mut classes: HashMap<i32, Vec<(u64, Option<TpkClass>)>> = HashMap::with_capacity(class_count);
    let mut release_roots: HashMap<i32, Vec<(u64, Option<u16>)>> = HashMap::with_capacity(class_count);
    for _ in 0..class_count {
        let id = d.read_i32()?;
        let entry_count = d.read_i32()? as usize;
        let mut entries = Vec::with_capacity(entry_count);
        let mut roots = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            let v = d.read_u64()?;
            let has_class = d.read_u8()? != 0;
            if has_class {
                // TpkUnityClass: <HHB name, base, flags>
                let name = d.read_u16()? as u32;
                let base = d.read_u16()?;
                let flags = d.read_u8()?;
                let editor_root = if flags & 64 != 0 { Some(d.read_u16()?) } else { None };
                let release_root = if flags & 128 != 0 { Some(d.read_u16()?) } else { None };
                let _ = editor_root;
                entries.push((v, Some(TpkClass { name_idx: name, _base: base })));
                roots.push((v, release_root));
            } else {
                entries.push((v, None));
                roots.push((v, None));
            }
        }
        classes.insert(id, entries);
        release_roots.insert(id, roots);
    }
    // CommonString (v2 format: version -> (offset, stringIndex) pairs).
    // Keep the LAST version's entries (UnityPy BuildMap(None) semantics).
    let cs_version_count = d.read_i32()? as usize;
    let mut common_strings: Vec<(u32, u32)> = Vec::new();
    for i in 0..cs_version_count {
        let _v = d.read_u64()?;
        let entry_count = d.read_i32()? as usize;
        let mut entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            let offset = d.read_u16()? as u32;
            let sidx = d.read_u16()? as u32;
            entries.push((offset, sidx));
        }
        if i + 1 == cs_version_count {
            common_strings = entries;
        }
    }
    // NodeBuffer
    let node_count = d.read_i32()? as usize;
    let mut nodes = Vec::with_capacity(node_count);
    for _ in 0..node_count {
        let type_name = d.read_u16()? as u32;
        let name = d.read_u16()? as u32;
        let byte_size = d.read_i32()?;
        let _version = d.read_u16()?;
        let type_flags = d.read_u8()?;
        let meta_flag = d.read_i32()? as u32;
        let sub_count = d.read_u16()? as usize;
        let mut sub_nodes = Vec::with_capacity(sub_count);
        for _ in 0..sub_count {
            sub_nodes.push(d.read_u16()?);
        }
        nodes.push(TpkNode {
            type_name,
            name,
            _byte_size: byte_size,
            _version,
            _type_flags: type_flags,
            meta_flag,
            sub_nodes,
        });
    }
    // StringBuffer
    let string_count = d.read_i32()? as usize;
    let mut strings = Vec::with_capacity(string_count);
    for _ in 0..string_count {
        strings.push(d.read_string()?);
    }

    Ok(TpkData {
        classes,
        nodes,
        strings,
        release_roots,
        common_strings,
    })
}

/// Get the release type-tree for `class_id` at engine `version`, converted to
/// the typetree module's node tree. Returns None when the class has no entry
/// for that version (unknown/script classes).
pub fn release_tree(class_id: i32, version: crate::unity::UnityVersion) -> Result<Option<super::typetree::Node>> {
    let data = tpk();
    let entries = match data.classes.get(&class_id) {
        Some(e) => e,
        None => return Ok(None),
    };
    // get_item_for_version: last entry with entry_version <= requested version
    let idx = match entries.iter().rposition(|(v, _)| *v <= version.0) {
        Some(i) => i,
        None => return Ok(None),
    };
    if entries[idx].1.is_none() {
        return Ok(None);
    }
    let roots = &data.release_roots[&class_id];
    let Some(Some(root)) = roots.get(idx).map(|(_, r)| r) else {
        return Ok(None);
    };
    Ok(Some(build_tree(data, *root)))
}

fn build_tree(data: &TpkData, root: u16) -> super::typetree::Node {
    // UnityPy generate_node: pop-front + prepend children = DFS preorder,
    // which is exactly the order Node::from_flat expects.
    let mut out: Vec<super::typetree::Node> = Vec::new();
    let mut queue: std::collections::VecDeque<(u16, i32)> = std::collections::VecDeque::new();
    queue.push_back((root, 0));
    while let Some((id, level)) = queue.pop_front() {
        let node = &data.nodes[id as usize];
        out.push(super::typetree::Node {
            level,
            type_name: data.strings.get(node.type_name as usize).cloned().unwrap_or_default(),
            name: data.strings.get(node.name as usize).cloned().unwrap_or_default(),
            byte_size: 0,
            index: 0,
            type_flags: 0,
            version: 0,
            meta_flag: node.meta_flag,
            children: Vec::new(),
        });
        // prepend children in order so the first child is popped next
        for &sub in node.sub_nodes.iter().rev() {
            queue.push_front((sub, level + 1));
        }
    }
    super::typetree::Node::from_flat(&mut out.into_iter())
}

pub fn common_string_count() -> usize {
    tpk().strings.len()
}

/// Common string at a buffer offset (used by typetree name tables: the value
/// has the 0x80000000 bit set and the low bits index into Unity's common
/// string table). Falls back to None for unknown offsets.
pub fn common_string_at(offset: usize) -> Option<String> {
    let data = tpk();
    let idx = data
        .common_strings
        .binary_search_by_key(&(offset as u32), |e| e.0)
        .ok()?;
    let sidx = data.common_strings[idx].1 as usize;
    data.strings.get(sidx).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unity::UnityVersion;

    #[test]
    fn tpk_loads_and_resolves() {
        let v2021 = UnityVersion::parse("2021.3.16f1").unwrap();
        let tree = release_tree(28, v2021).unwrap().expect("Texture2D tree for 2021");
        // root is "Texture2D"; the second level should include m_Name / m_Width
        assert_eq!(tree.type_name, "Texture2D");
        let names: Vec<&str> = tree.children.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"m_Name"), "fields: {names:?}");

        let v34 = UnityVersion::parse("3.4.0f1").unwrap();
        let tree34 = release_tree(28, v34).unwrap().expect("Texture2D tree for 3.4");
        let names34: Vec<&str> = tree34.children.iter().map(|c| c.name.as_str()).collect();
        assert!(names34.contains(&"m_Width"));

        // AudioClip for Unity 5+ must have m_Resource
        let v5 = UnityVersion::parse("5.6.0f3").unwrap();
        let audio = release_tree(83, v5).unwrap().unwrap();
        let anames: Vec<&str> = audio.children.iter().map(|c| c.name.as_str()).collect();
        assert!(anames.contains(&"m_Resource"), "audio fields: {anames:?}");

        // unknown class -> None
        assert!(release_tree(99999, v2021).unwrap().is_none());
    }
}
