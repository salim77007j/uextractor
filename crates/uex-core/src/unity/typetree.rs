//! Unity TypeTree: node structure, embedded-typetree parsing (serialized file
//! versions 10/12 blob + legacy text form + 23 "mhtt" handler), and the
//! typetree-driven value reader that turns object bytes into a UValue tree.

use crate::error::{Error, Result};

/// Align flag in m_MetaFlag.
pub const K_ALIGN_BYTES: u32 = 0x4000;

#[derive(Debug, Clone, Default)]
pub struct Node {
    pub level: i32,
    pub type_name: String,
    pub name: String,
    pub byte_size: i32,
    pub index: i32,
    pub type_flags: u32,
    /// serialized-file version at which this field appeared
    pub version: u32,
    pub meta_flag: u32,
    pub children: Vec<Node>,
}

impl Node {
    /// Build a hierarchy from a depth-first flat node list (levels encoded in each node).
    /// Direct port of UnityPy's `TypeTreeNode.from_list` stack algorithm.
    pub fn from_flat(iter: &mut impl Iterator<Item = Node>) -> Node {
        let mut nodes: Vec<Node> = iter.collect();
        if nodes.is_empty() {
            return Node::default();
        }
        // index 0 = fake root (level -1)
        let mut all: Vec<Node> = Vec::with_capacity(nodes.len() + 1);
        all.push(Node {
            level: -1,
            ..Default::default()
        });
        all.append(&mut nodes);
        let mut children_of: Vec<Vec<usize>> = vec![Vec::new(); all.len()];
        let mut stack: Vec<usize> = Vec::new();
        let mut parent: usize = 0; // fake root
        let mut prev_idx: usize = 0;
        let mut prev_level: i32 = -1;
        for i in 1..all.len() {
            let lvl = all[i].level;
            if lvl > prev_level {
                stack.push(parent);
                parent = prev_idx;
            } else if lvl < prev_level {
                while lvl <= all[parent].level {
                    parent = stack.pop().unwrap_or(0);
                }
            }
            children_of[parent].push(i);
            prev_idx = i;
            prev_level = lvl;
        }
        fn build(all: &mut [Node], children_of: &mut [Vec<usize>], idx: usize) -> Node {
            let children = std::mem::take(&mut children_of[idx]);
            let mut node = std::mem::take(&mut all[idx]);
            node.children = children.into_iter().map(|c| build(all, children_of, c)).collect();
            node
        }
        let root_child = children_of[0].first().copied().unwrap_or(1);
        build(&mut all, &mut children_of, root_child)
    }

    pub fn find(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }
}

/// Parsed object value, in the spirit of UnityPy's dict output.
#[derive(Debug, Clone, PartialEq)]
pub enum UValue {
    Int(i64),
    UInt(u64),
    Float(f64),
    Bool(bool),
    Str(String),
    Bytes(Vec<u8>),
    Array(Vec<UValue>),
    Object(Vec<(String, UValue)>),
    PPtr { file_id: i64, path_id: i64 },
    /// reference into the shared string buffer (typetree string tables)
    Raw(String),
}

impl UValue {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            UValue::Int(v) => Some(*v),
            UValue::UInt(v) => Some(*v as i64),
            UValue::Bool(v) => Some(*v as i64),
            UValue::Float(v) => Some(*v as i64),
            _ => None,
        }
    }
    pub fn as_u64(&self) -> Option<u64> {
        self.as_i64().map(|v| v as u64)
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            UValue::Float(v) => Some(*v),
            UValue::Int(v) => Some(*v as f64),
            UValue::UInt(v) => Some(*v as f64),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            UValue::Str(s) | UValue::Raw(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            UValue::Bytes(b) => Some(b),
            UValue::Str(s) => Some(s.as_bytes()),
            _ => None,
        }
    }
    /// Bytes from a byte blob OR an array of small integers (TPK trees return
    /// aligned UInt8 arrays as number arrays, not byte buffers).
    pub fn as_byte_vec(&self) -> Option<Vec<u8>> {
        match self {
            UValue::Bytes(b) => Some(b.clone()),
            UValue::Str(s) => Some(s.as_bytes().to_vec()),
            UValue::Array(a) => {
                let mut out = Vec::with_capacity(a.len());
                for v in a {
                    out.push(v.as_u64()? as u8);
                }
                Some(out)
            }
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[UValue]> {
        match self {
            UValue::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn get(&self, key: &str) -> Option<&UValue> {
        match self {
            UValue::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(|v| v.as_i64())
    }
    pub fn get_u64(&self, key: &str) -> Option<u64> {
        self.get(key).and_then(|v| v.as_u64())
    }
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(|v| v.as_f64())
    }
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(|v| v.as_str())
    }

    /// Convert to serde_json for metadata output. Byte arrays become hex strings
    /// when longer than 64 bytes, arrays of numbers otherwise.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            UValue::Int(v) => serde_json::json!(v),
            UValue::UInt(v) => serde_json::json!(v),
            UValue::Float(v) => serde_json::json!(v),
            UValue::Bool(v) => serde_json::json!(v),
            UValue::Str(s) | UValue::Raw(s) => serde_json::json!(s),
            UValue::Bytes(b) => {
                if b.len() > 64 {
                    serde_json::json!(format!("<{} bytes>", b.len()))
                } else {
                    serde_json::json!(hex::encode(b))
                }
            }
            UValue::Array(a) => serde_json::Value::Array(a.iter().map(|v| v.to_json()).collect()),
            UValue::Object(fields) => {
                let mut m = serde_json::Map::new();
                for (k, v) in fields {
                    m.insert(k.clone(), v.to_json());
                }
                serde_json::Value::Object(m)
            }
            UValue::PPtr { file_id, path_id } => serde_json::json!({
                "file_id": file_id,
                "path_id": path_id,
            }),
        }
    }
}

/// A reader over big/little-endian object bytes with alignment support.
pub struct Reader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
    pub big_endian: bool,
    /// shared string buffer for name tables (from the serialized file)
    pub string_buffer: Vec<u8>,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8], big_endian: bool) -> Self {
        Reader {
            data,
            pos: 0,
            big_endian,
            string_buffer: Vec::new(),
        }
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }
    pub fn align(&mut self, to: usize) {
        let m = self.pos % to;
        if m != 0 {
            self.pos += to - m;
        }
    }
    pub fn read_bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.data.len() {
            return Err(Error::malformed(format!(
                "typetree read out of bounds at {} need {n} have {}",
                self.pos,
                self.data.len() - self.pos
            )));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn read_sized(&mut self, n: usize) -> Result<u64> {
        let b = self.read_bytes(n)?;
        let mut v = [0u8; 8];
        if self.big_endian {
            // first byte is most significant: place at the high end of the
            // low-n window so from_le_bytes yields the BE value
            for (i, byte) in b.iter().enumerate() {
                v[n - 1 - i] = *byte;
            }
        } else {
            v[..n].copy_from_slice(b);
        }
        Ok(u64::from_le_bytes(v))
    }
    pub fn read_u8(&mut self) -> Result<u8> {
        Ok(self.read_bytes(1)?[0])
    }
    pub fn read_i16(&mut self) -> Result<i16> {
        Ok(self.read_sized(2)? as i16)
    }
    pub fn read_u16(&mut self) -> Result<u16> {
        Ok(self.read_sized(2)? as u16)
    }
    pub fn read_i32(&mut self) -> Result<i32> {
        Ok(self.read_sized(4)? as i32)
    }
    pub fn read_u32(&mut self) -> Result<u32> {
        Ok(self.read_sized(4)? as u32)
    }
    pub fn read_i64(&mut self) -> Result<i64> {
        Ok(self.read_sized(8)? as i64)
    }
    pub fn read_u64(&mut self) -> Result<u64> {
        Ok(self.read_sized(8)?)
    }
    pub fn read_f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.read_sized(4)? as u32))
    }
    pub fn read_f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.read_sized(8)?))
    }
    pub fn read_bool(&mut self) -> Result<bool> {
        Ok(self.read_u8()? != 0)
    }
    pub fn read_string_to_null(&mut self) -> Result<String> {
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos] != 0 {
            self.pos += 1;
        }
        let s = String::from_utf8_lossy(&self.data[start..self.pos]).into_owned();
        if self.pos < self.data.len() {
            self.pos += 1; // skip null
        }
        Ok(s)
    }
    pub fn read_aligned_string(&mut self) -> Result<String> {
        let len = self.read_i32()? as usize;
        if len > self.remaining() {
            return Err(Error::malformed("string length out of bounds"));
        }
        let s = String::from_utf8_lossy(self.read_bytes(len)?).into_owned();
        self.align(4);
        Ok(s)
    }
}

/// Read a full object via a type tree (UnityPy read_value port).
pub fn read_value(node: &Node, reader: &mut Reader) -> Result<UValue> {
    // UnityPy: the align decision also honors an "Array" first child's align flag
    let mut align_flag = node.meta_flag & K_ALIGN_BYTES != 0;
    if !node.children.is_empty() && node.children[0].type_name == "Array" {
        if node.children[0].meta_flag & K_ALIGN_BYTES != 0 {
            align_flag = true;
        }
    }
    let value = read_value_inner(node, reader)?;
    if align_flag {
        reader.align(4);
    }
    Ok(value)
}

fn read_value_inner(node: &Node, reader: &mut Reader) -> Result<UValue> {
    match node.type_name.as_str() {
        "SInt8" => Ok(UValue::Int(reader.read_u8()? as i8 as i64)),
        "UInt8" | "char" => Ok(UValue::UInt(reader.read_u8()? as u64)),
        "short" | "SInt16" => Ok(UValue::Int(reader.read_i16()? as i64)),
        "unsigned short" | "UInt16" => Ok(UValue::UInt(reader.read_u16()? as u64)),
        "int" | "SInt32" => Ok(UValue::Int(reader.read_i32()? as i64)),
        "unsigned int" | "UInt32" | "Type*" => Ok(UValue::UInt(reader.read_u32()? as u64)),
        "long long" | "SInt64" => Ok(UValue::Int(reader.read_i64()?)),
        "unsigned long long" | "UInt64" | "FileSize" => Ok(UValue::UInt(reader.read_i64()? as u64)),
        "float" => Ok(UValue::Float(reader.read_f32()? as f64)),
        "double" => Ok(UValue::Float(reader.read_f64()?)),
        "bool" => Ok(UValue::Bool(reader.read_bool()?)),
        "string" => Ok(UValue::Str(reader.read_aligned_string()?)),
        "TypelessData" => {
            let len = reader.read_i32()? as usize;
            Ok(UValue::Bytes(reader.read_bytes(len)?.to_vec()))
        }
        "pair" => {
            let first = read_value(&node.children[0], reader)?;
            let second = read_value(&node.children[1], reader)?;
            Ok(UValue::Array(vec![first, second]))
        }
        _ => {
            // vector: node with first child "Array"
            if !node.children.is_empty() && node.children[0].type_name == "Array" {
                if node.children[0].meta_flag & K_ALIGN_BYTES != 0 {
                    // alignment handled by outer read via flag propagation below
                }
                let size = reader.read_i32()?;
                if size < 0 {
                    return Err(Error::malformed("negative array length in typetree"));
                }
                let subtype = &node.children[0].children[1];
                let align_elem = subtype.meta_flag & K_ALIGN_BYTES != 0;
                let mut out = Vec::with_capacity(size.min(1 << 20) as usize);
                if align_elem {
                    // fast array path for primitive element types
                    if let Some(vals) = read_primitive_array(subtype, reader, size as usize)? {
                        return Ok(vals);
                    }
                }
                for _ in 0..size {
                    out.push(read_value(subtype, reader)?);
                }
                return Ok(UValue::Array(out));
            }
            // class / PPtr
            if node.type_name.starts_with("PPtr<") {
                let file_id = node.find("m_FileID").map(|n| read_value(n, reader)).transpose()?;
                let path_id = node.find("m_PathID").map(|n| read_value(n, reader)).transpose()?;
                let file_id = file_id.as_ref().and_then(|v| v.as_i64()).unwrap_or(0);
                let path_id = match path_id {
                    Some(UValue::Int(v)) => v,
                    Some(UValue::UInt(v)) => v as i64,
                    _ => 0,
                };
                return Ok(UValue::PPtr { file_id, path_id });
            }
            let mut fields = Vec::with_capacity(node.children.len());
            for child in &node.children {
                let v = read_value(child, reader)?;
                fields.push((child.name.clone(), v));
            }
            Ok(UValue::Object(fields))
        }
    }
}

/// Bulk read of aligned primitive arrays (int/float vectors etc.).
fn read_primitive_array(subtype: &Node, reader: &mut Reader, size: usize) -> Result<Option<UValue>> {
    let n = match subtype.type_name.as_str() {
        "SInt8" => 1,
        "UInt8" | "char" => 1,
        "short" | "SInt16" => 2,
        "unsigned short" | "UInt16" => 2,
        "int" | "SInt32" => 4,
        "unsigned int" | "UInt32" | "Type*" => 4,
        "long long" | "SInt64" => 8,
        "unsigned long long" | "UInt64" | "FileSize" => 8,
        "float" => 4,
        "double" => 8,
        "bool" => 1,
        _ => return Ok(None),
    };
    if size.checked_mul(n).map(|v| v > reader.remaining()).unwrap_or(true) {
        return Err(Error::malformed("primitive array out of bounds"));
    }
    let bytes = reader.read_bytes(size * n)?;
    let be = reader.big_endian;
    let vals: Vec<UValue> = match subtype.type_name.as_str() {
        "float" => bytes
            .chunks_exact(4)
            .map(|c| {
                UValue::Float(if be {
                    f32::from_be_bytes([c[0], c[1], c[2], c[3]]) as f64
                } else {
                    f32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f64
                })
            })
            .collect(),
        "double" => bytes
            .chunks_exact(8)
            .map(|c| {
                let mut a = [0u8; 8];
                a.copy_from_slice(c);
                UValue::Float(if be { f64::from_be_bytes(a) } else { f64::from_le_bytes(a) })
            })
            .collect(),
        "long long" | "SInt64" => bytes
            .chunks_exact(8)
            .map(|c| {
                UValue::Int(if be {
                    i64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])
                } else {
                    i64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])
                })
            })
            .collect(),
        "unsigned long long" | "UInt64" | "FileSize" => bytes
            .chunks_exact(8)
            .map(|c| {
                UValue::UInt(if be {
                    u64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])
                } else {
                    u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])
                })
            })
            .collect(),
        _ => {
            // generic N-byte loop
            bytes
                .chunks_exact(n)
                .map(|c| {
                    let mut v: u64 = 0;
                    if be {
                        for &b in c {
                            v = (v << 8) | b as u64;
                        }
                    } else {
                        for (i, &b) in c.iter().enumerate() {
                            v |= (b as u64) << (8 * i);
                        }
                    }
                    match subtype.type_name.as_str() {
                        "SInt8" => UValue::Int(v as u8 as i8 as i64),
                        "short" | "SInt16" => UValue::Int(v as u16 as i16 as i64),
                        "int" | "SInt32" => UValue::Int(v as u32 as i32 as i64),
                        "bool" => UValue::Bool(v != 0),
                        _ => UValue::UInt(v),
                    }
                })
                .collect()
        }
    };
    Ok(Some(UValue::Array(vals)))
}

/// Parse an embedded typetree blob (serialized file versions >= 12, and == 10).
/// `version` is the serialized file version.
pub fn parse_blob(reader: &mut Reader, version: u32) -> Result<Node> {
    let node_count = reader.read_i32()? as usize;
    let stringbuffer_size = reader.read_i32()? as usize;
    if node_count > 1_000_000 {
        return Err(Error::malformed("typetree node count unreasonable"));
    }
    // node struct: hBBIIiii (+Q for v>=19)
    #[derive(Clone, Copy)]
    struct RawNode {
        version: u16,
        level: u8,
        type_flags: u8,
        type_str: u32,
        name_str: u32,
        byte_size: i32,
        index: i32,
        meta_flag: i32,
        ref_hash: u64,
    }
    let mut raws = Vec::with_capacity(node_count);
    let file_version = version; // records carry the ref hash only for FILE version >= 19
    for _ in 0..node_count {
        let node_version = reader.read_u16()?;
        let level = reader.read_u8()?;
        let type_flags = reader.read_u8()?;
        let type_str = reader.read_u32()?;
        let name_str = reader.read_u32()?;
        let byte_size = reader.read_i32()?;
        let index = reader.read_i32()?;
        let meta_flag = reader.read_i32()?;
        let ref_hash = if file_version >= 19 { reader.read_i64()? as u64 } else { 0 };
        raws.push(RawNode {
            version: node_version,
            level,
            type_flags,
            type_str,
            name_str,
            byte_size,
            index,
            meta_flag,
            ref_hash,
        });
    }
    let mut sb = Vec::with_capacity(stringbuffer_size);
    sb.extend_from_slice(reader.read_bytes(stringbuffer_size)?);

    let mut sb_reader = Reader::new(&sb, false);
    let mut read_string = |value: u32| -> Result<String> {
        if value & 0x8000_0000 == 0 {
            // offset into this blob's string buffer
            let save = sb_reader.pos;
            sb_reader.pos = value as usize;
            let s = sb_reader.read_string_to_null();
            sb_reader.pos = save;
            s
        } else {
            let offset = (value & 0x7FFF_FFFF) as usize;
            Ok(crate::unity::tpk::common_string_at(offset).unwrap_or_else(|| format!("{offset}")))
        }
    };

    // build hierarchy (UnityPy stack algorithm)
    let mut nodes: Vec<Node> = Vec::with_capacity(node_count);
    for r in &raws {
        nodes.push(Node {
            level: r.level as i32,
            type_name: read_string(r.type_str)?,
            name: read_string(r.name_str)?,
            byte_size: r.byte_size,
            index: r.index,
            type_flags: r.type_flags as u32,
            version: r.version as u32,
            meta_flag: r.meta_flag as u32,
            children: Vec::new(),
        });
    }
    Ok(build_hierarchy(nodes))
}

/// Legacy (version < 12, != 10) typetree: text strings + explicit child counts.
/// Direct port of UnityPy's `TypeTreeNode.parse` (non-blob form).
pub fn parse_legacy(reader: &mut Reader, version: u32) -> Result<Node> {
    let read_node = |reader: &mut Reader| -> Result<(Node, i32)> {
        let type_name = reader.read_string_to_null()?;
        let name = reader.read_string_to_null()?;
        let byte_size = reader.read_i32()?;
        if version == 2 {
            let _variable_count = reader.read_i32()?;
        }
        let index = if version != 3 { reader.read_i32()? } else { 0 };
        let type_flags = reader.read_i32()? as u32;
        let ver = reader.read_i32()? as u32;
        let meta_flag = if version != 3 { reader.read_i32()? as u32 } else { 0 };
        let children_count = reader.read_i32()?;
        Ok((
            Node {
                level: 0,
                type_name,
                name,
                byte_size,
                index,
                type_flags,
                version: ver,
                meta_flag,
                children: Vec::new(),
            },
            children_count,
        ))
    };

    // arena of nodes + child links by index
    let mut arena: Vec<Node> = Vec::new();
    arena.push(Node {
        level: -1,
        ..Default::default()
    }); // fake root
    let mut children_of: Vec<Vec<usize>> = vec![Vec::new()];
    let mut frames: Vec<(usize, i32)> = Vec::new(); // (arena parent idx, remaining children)
    frames.push((0, 1)); // the fake root has exactly one child (the real root)
    while let Some(&(pid, count)) = frames.last() {
        // pop-or-decrement BEFORE reading the child (UnityPy semantics)
        if count == 1 {
            frames.pop();
        } else if let Some(f) = frames.last_mut() {
            f.1 -= 1;
        }
        let (mut node, children_count) = read_node(reader)?;
        node.level = arena[pid].level + 1;
        let my = arena.len();
        arena.push(node);
        children_of.push(Vec::new());
        children_of[pid].push(my);
        if children_count > 0 {
            frames.push((my, children_count));
        }
        if arena.len() > 1_000_000 {
            return Err(Error::malformed("legacy typetree too large"));
        }
    }
    // materialize: return the real root (first child of fake root)
    let root_idx = children_of[0].first().copied().unwrap_or(1);
    let mut children_of = children_of;
    fn build(arena: &mut [Node], children_of: &mut [Vec<usize>], idx: usize) -> Node {
        let children = std::mem::take(&mut children_of[idx]);
        let mut node = std::mem::take(&mut arena[idx]);
        node.children = children.into_iter().map(|c| build(arena, children_of, c)).collect();
        node
    }
    Ok(build(&mut arena, &mut children_of, root_idx))
}

/// Assemble a hierarchy from a flat depth-first list ordered by traversal.
pub fn build_hierarchy(nodes: Vec<Node>) -> Node {
    Node::from_flat(&mut nodes.into_iter())
}

/// v23+ "mhtt" TypeTreeNodeInfo: blob of the same format as parse_blob.
pub fn parse_mhtt(reader: &mut Reader) -> Result<Node> {
    let mut magic = [0u8; 4];
    magic.copy_from_slice(reader.read_bytes(4)?);
    if &magic != b"mhtt" {
        return Err(Error::malformed("invalid type tree blob magic (mhtt)"));
    }
    let version = reader.read_i32()? as u32;
    parse_blob(reader, version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_flat_builds_hierarchy() {
        // A(0) -> B(1) -> C(2); D(1); E(0)
        let mk = |level: i32, name: &str| Node {
            level,
            type_name: "int".into(),
            name: name.into(),
            ..Default::default()
        };
        let flat = vec![mk(0, "A"), mk(1, "B"), mk(2, "C"), mk(1, "D"), mk(0, "E")];
        let root = Node::from_flat(&mut flat.into_iter());
        assert_eq!(root.name, "A");
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children[0].name, "B");
        assert_eq!(root.children[0].children.len(), 1);
        assert_eq!(root.children[0].children[0].name, "C");
        assert_eq!(root.children[1].name, "D");
        assert_eq!(root.children[1].children.len(), 0);
        assert_eq!(root.children[0].children[0].children.len(), 0);
    }

    #[test]
    fn value_reader_primitives() {
        // node: int array (aligned)
        let mk = |level: i32, type_name: &str, name: &str, meta: u32| Node {
            level,
            type_name: type_name.into(),
            name: name.into(),
            meta_flag: meta,
            ..Default::default()
        };
        let node = mk(0, "vector", "m_Data", 0);
        let node = Node {
            children: vec![Node {
                children: vec![
                    mk(2, "int", "size", 0),
                    mk(2, "int", "data", K_ALIGN_BYTES),
                ],
                ..mk(1, "Array", "Array", K_ALIGN_BYTES)
            }],
            ..node
        };
        // simulate buffer: size=2, values 1, -2
        let buf = vec![2u8, 0, 0, 0, 1, 0, 0, 0, 0xFE, 0xFF, 0xFF, 0xFF];
        let mut r = Reader::new(&buf, false);
        let v = read_value(&node, &mut r).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0].as_i64(), Some(1));
        assert_eq!(arr[1].as_i64(), Some(-2));
    }
}
