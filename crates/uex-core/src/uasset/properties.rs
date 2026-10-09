//! Property serialization: versioned FPropertyTag reading + unversioned property
//! stream decoding (FUnversionedHeader) with per-class layouts.
//!
//! Key facts encoded here (from UE source behavior):
//! - FNames in tagged properties are (u32 index, i32 number) pairs resolved via the name map.
//! - BoolProperty's value rides bit 31 of the *type name* FName index.
//! - Unversioned streams are bit-packed: [bHasValues][bZeroPacked][7-bit count] triples;
//!   count 0 in packed mode → 32-bit escape; termination when ValueNum reaches 0 or a
//!   zero escape count of 0.

use super::UAssetPackage;
use crate::error::{Error, Result};
use crate::io::UexRead;
use std::io::Cursor;

/// Property type for unversioned layout application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutType {
    Int,     // i32
    Int64,   // i64
    Int16,   // i16
    Int8,    // i8
    UInt32,  // u32
    UInt64,  // u64
    Float,   // f32
    Double,  // f64
    Bool,    // no data (presence = value)
    Str,     // FString
    Name,    // FName (index+number)
    Object,  // i32 package index
    Byte,    // u8
    Enum,    // FName value
    Vec3F,   // 3 × f32 (pre-LWC vector)
    Vec3D,   // 3 × f64 (LWC vector)
    Vec2F,   // 2 × f32
    RotF,    // 3 × f32
    Color,   // 4 × u8
    Guid,    // 16 bytes
    SoftObj, // FString path + FString subpath + 16 guid
    Skip,    // property declared but data unknown → cannot consume; layout must not use
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum PropValue {
    Int(i32),
    Int64(i64),
    Int16(i16),
    Int8(i8),
    UInt(u32),
    UInt64(u64),
    UInt16(u16),
    Float(f32),
    Double(f64),
    Bool(bool),
    Str(String),
    Name(String),
    Object(i32),
    Byte(u8),
    Enum(String),
    Vec3([f64; 3]),
    Vec2([f32; 2]),
    Color([u8; 4]),
    Guid(String),
    SoftObject(String),
    Struct(Vec<(String, PropValue)>),
    Array(Vec<PropValue>),
    Raw(#[serde(skip)] Vec<u8>),
    None,
}

pub struct PropertyReader<'a> {
    pub pkg: &'a UAssetPackage,
    pub ue4: i32,
    pub ue5: i32,
}

impl<'a> PropertyReader<'a> {
    pub fn new(pkg: &'a UAssetPackage) -> PropertyReader<'a> {
        PropertyReader {
            ue4: pkg.summary.file_version_ue4,
            ue5: pkg.summary.file_version_ue5,
            pkg,
        }
    }

    pub fn read_fname(&self, r: &mut Cursor<&[u8]>) -> Result<String> {
        let index = r.u32()?;
        let number = r.i32()?;
        let mut name = self
            .pkg
            .name((index & 0x7FFF_FFFF) as i32)
            .unwrap_or("None")
            .to_string();
        if number > 0 {
            name = format!("{name}_{}", number - 1);
        }
        Ok(name)
    }

    /// Read a tagged property list (versioned packages) until "None".
    pub fn read_tagged(&self, r: &mut Cursor<&[u8]>) -> Result<Vec<(String, PropValue)>> {
        let mut out = Vec::new();
        let mut guard = 0usize;
        loop {
            guard += 1;
            if guard > 1_000_000 {
                return Err(Error::asset("property list too long"));
            }
            let name = self.read_fname(r)?;
            if name == "None" {
                return Ok(out);
            }
            let type_index_raw = r.u32()?;
            let type_number = r.i32()?;
            let type_name = {
                let base = self
                    .pkg
                    .name((type_index_raw & 0x7FFF_FFFF) as i32)
                    .unwrap_or("None")
                    .to_string();
                let _ = type_number;
                base
            };
            let size = r.i64()?;
            let struct_name = if matches!(type_name.as_str(), "StructProperty" | "ArrayProperty" | "MapProperty" | "SetProperty") {
                self.read_fname(r)?
            } else {
                String::new()
            };
            let has_guid = if self.pkg.summary.is_ue5() && self.ue5 >= 1011 {
                r.u8()? == 1
            } else {
                r.u32()? == 1
            };
            if has_guid {
                let _guid = r.read_bytes(16)?;
            }

            let body_start = r.position() as usize;
            let data_len = r.get_ref().len();
            let body_end = (body_start + size.max(0) as usize).min(data_len);

            let value = match type_name.as_str() {
                "IntProperty" => PropValue::Int(r.i32()?),
                "Int64Property" => PropValue::Int64(r.i64()?),
                "Int16Property" => PropValue::Int16(r.i16()?),
                "Int8Property" => PropValue::Int8(r.i8()?),
                "UInt32Property" => PropValue::UInt(r.u32()?),
                "UInt64Property" => PropValue::UInt64(r.u64()?),
                "UInt16Property" => PropValue::UInt16(r.u16()?),
                "FloatProperty" => PropValue::Float(r.f32()?),
                "DoubleProperty" => PropValue::Double(r.f64()?),
                "BoolProperty" => PropValue::Bool(type_index_raw & 0x8000_0000 != 0),
                "StrProperty" => PropValue::Str(r.fstring()?),
                "NameProperty" => PropValue::Name(self.read_fname(r)?),
                "ObjectProperty" | "ClassProperty" | "WeakObjectProperty" | "LazyObjectProperty" => {
                    PropValue::Object(r.i32()?)
                }
                "SoftObjectProperty" => {
                    let path = r.fstring()?;
                    let _sub = r.fstring()?;
                    let _guid = r.read_bytes(16)?;
                    PropValue::SoftObject(path)
                }
                "EnumProperty" => PropValue::Enum(self.read_fname(r)?),
                "ByteProperty" => {
                    if struct_name.is_empty() {
                        PropValue::Byte(r.u8()?)
                    } else {
                        // byte with enum: serialized as FName
                        PropValue::Enum(self.read_fname(r)?)
                    }
                }
                "StructProperty" => {
                    let sub = self.read_tagged(r)?;
                    PropValue::Struct(sub)
                }
                "ArrayProperty" => {
                    let count = r.i32()?;
                    let inner = struct_name.as_str();
                    let mut arr = Vec::new();
                    for _ in 0..count.clamp(0, 0x1_0000) {
                        arr.push(match inner {
                            "IntProperty" => PropValue::Int(r.i32()?),
                            "FloatProperty" => PropValue::Float(r.f32()?),
                            "UInt32Property" => PropValue::UInt(r.u32()?),
                            "NameProperty" => PropValue::Name(self.read_fname(r)?),
                            "StrProperty" => PropValue::Str(r.fstring()?),
                            "ObjectProperty" => PropValue::Object(r.i32()?),
                            "ByteProperty" => PropValue::Byte(r.u8()?),
                            "DoubleProperty" => PropValue::Double(r.f64()?),
                            _ => {
                                // unknown inner: bail out of array body
                                break;
                            }
                        });
                    }
                    // position at declared end when possible
                    r.set_position(body_end as u64);
                    PropValue::Array(arr)
                }
                _ => {
                    // unknown/complex: skip declared body
                    let raw = r.get_ref()[body_start..body_end].to_vec();
                    r.set_position(body_end as u64);
                    PropValue::Raw(raw)
                }
            };
            out.push((name, value));
        }
    }
}

/// Decode an unversioned property header. `value_num` = total property count of the class layout.
/// Returns fragments in stream order: (count, has_values).
pub fn decode_unversioned_fragments(r: &mut Cursor<&[u8]>, mut value_num: i32) -> Result<Vec<(i32, bool)>> {
    struct BitReader<'b, R> {
        r: &'b mut R,
        buf: u64,
        count: u32,
    }
    impl<'b, R: UexRead> BitReader<'b, R> {
        fn bit(&mut self) -> Result<u32> {
            if self.count == 0 {
                self.buf = self.r.u8()? as u64;
                self.count = 8;
            }
            let b = (self.buf & 1) as u32;
            self.buf >>= 1;
            self.count -= 1;
            Ok(b)
        }
        fn bits32(&mut self) -> Result<u32> {
            let mut v = 0u32;
            for i in 0..32 {
                v |= self.bit()? << i;
            }
            Ok(v)
        }
    }
    let mut br = BitReader { r, buf: 0, count: 0 };
    let mut fragments = Vec::new();
    while value_num > 0 {
        let has_values = br.bit()? != 0;
        let mut zero_num: i32;
        if br.bit()? != 0 {
            // zero packed: 7 bits
            zero_num = 0;
            for i in 0..7 {
                zero_num |= (br.bit()? as i32) << i;
            }
            if zero_num == 0 {
                zero_num = br.bits32()? as i32;
                if zero_num == 0 {
                    return Ok(fragments);
                }
            }
        } else {
            zero_num = 1;
        }
        value_num -= zero_num;
        fragments.push((zero_num, has_values));
    }
    Ok(fragments)
}

/// Apply a class layout to an unversioned property stream, producing name→value pairs
/// for properties that have values.
pub fn apply_unversioned_layout(
    r: &mut Cursor<&[u8]>,
    layout: &[(&str, LayoutType)],
    pkg: &UAssetPackage,
) -> Result<Vec<(String, PropValue)>> {
    let fragments = decode_unversioned_fragments(r, layout.len() as i32)?;
    let mut out = Vec::new();
    let mut cursor = 0usize;
    for (count, has_values) in fragments {
        if has_values {
            for i in cursor..(cursor + count as usize).min(layout.len()) {
                let (name, ty) = layout[i];
                let value = read_unversioned_value(r, ty, pkg)?;
                out.push((name.to_string(), value));
            }
        }
        cursor += count as usize;
    }
    Ok(out)
}

fn read_unversioned_value(r: &mut Cursor<&[u8]>, ty: LayoutType, pkg: &UAssetPackage) -> Result<PropValue> {
    Ok(match ty {
        LayoutType::Int => PropValue::Int(r.i32()?),
        LayoutType::Int64 => PropValue::Int64(r.i64()?),
        LayoutType::Int16 => PropValue::Int16(r.i16()?),
        LayoutType::Int8 => PropValue::Int8(r.i8()?),
        LayoutType::UInt32 => PropValue::UInt(r.u32()?),
        LayoutType::UInt64 => PropValue::UInt64(r.u64()?),
        LayoutType::Float => PropValue::Float(r.f32()?),
        LayoutType::Double => PropValue::Double(r.f64()?),
        LayoutType::Bool => PropValue::Bool(true),
        LayoutType::Str => PropValue::Str(r.fstring()?),
        LayoutType::Name => {
            let index = r.u32()?;
            let number = r.i32()?;
            let mut name = pkg.name(index as i32).unwrap_or("None").to_string();
            if number > 0 {
                name = format!("{name}_{}", number - 1);
            }
            PropValue::Name(name)
        }
        LayoutType::Object => PropValue::Object(r.i32()?),
        LayoutType::Byte => PropValue::Byte(r.u8()?),
        LayoutType::Enum => {
            let index = r.u32()?;
            let number = r.i32()?;
            let mut name = pkg.name(index as i32).unwrap_or("None").to_string();
            if number > 0 {
                name = format!("{name}_{}", number - 1);
            }
            PropValue::Enum(name)
        }
        LayoutType::Vec3F => PropValue::Vec3([
            r.f32()? as f64,
            r.f32()? as f64,
            r.f32()? as f64,
        ]),
        LayoutType::Vec3D => PropValue::Vec3([r.f64()?, r.f64()?, r.f64()?]),
        LayoutType::Vec2F => PropValue::Vec2([r.f32()?, r.f32()?]),
        LayoutType::RotF => PropValue::Vec3([
            r.f32()? as f64,
            r.f32()? as f64,
            r.f32()? as f64,
        ]),
        LayoutType::Color => {
            let b = r.read_bytes(4)?;
            PropValue::Color([b[0], b[1], b[2], b[3]])
        }
        LayoutType::Guid => PropValue::Guid(hex::encode(r.read_bytes(16)?)),
        LayoutType::SoftObj => {
            let path = r.fstring()?;
            let _sub = r.fstring()?;
            let _guid = r.read_bytes(16)?;
            PropValue::SoftObject(path)
        }
        LayoutType::Skip => PropValue::None,
    })
}

/// Dump properties of a versioned export (tagged serialization).
pub fn dump_export_properties(pkg: &UAssetPackage, export_index: usize) -> Result<Vec<(String, PropValue)>> {
    let data = pkg
        .export_bytes(export_index)
        .ok_or_else(|| Error::asset("export out of range"))?;
    let reader = PropertyReader::new(pkg);
    let mut r = Cursor::new(data);
    reader.read_tagged(&mut r)
}

/// Convert parsed properties to a JSON value map.
pub fn props_to_json(props: &[(String, PropValue)]) -> serde_json::Value {
    use serde_json::Value;
    let mut map = serde_json::Map::new();
    for (k, v) in props {
        let json = match v {
            PropValue::Int(x) => Value::from(*x),
            PropValue::Int64(x) => Value::from(*x),
            PropValue::Int16(x) => Value::from(*x),
            PropValue::Int8(x) => Value::from(*x),
            PropValue::UInt(x) => Value::from(*x),
            PropValue::UInt64(x) => Value::from(*x),
            PropValue::UInt16(x) => Value::from(*x),
            PropValue::Float(x) => Value::from(*x),
            PropValue::Double(x) => Value::from(*x),
            PropValue::Bool(x) => Value::from(*x),
            PropValue::Str(x) | PropValue::Name(x) | PropValue::Enum(x) | PropValue::SoftObject(x) => Value::from(x.clone()),
            PropValue::Object(x) => serde_json::json!({ "$object": x }),
            PropValue::Byte(x) => Value::from(*x),
            PropValue::Vec3(v) => serde_json::json!(v),
            PropValue::Vec2(v) => serde_json::json!(v),
            PropValue::Color(c) => serde_json::json!(c),
            PropValue::Guid(g) => Value::from(g.clone()),
            PropValue::Struct(fields) => {
                let inner = props_to_json(fields);
                inner
            }
            PropValue::Array(items) => {
                let fake: Vec<(String, PropValue)> = items
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (i.to_string(), v.clone()))
                    .collect();
                let obj = props_to_json(&fake);
                Value::Array(obj.as_object().map(|o| o.values().cloned().collect()).unwrap_or_default())
            }
            PropValue::Raw(bytes) => Value::String(format!("<{} raw bytes>", bytes.len())),
            PropValue::None => Value::Null,
        };
        map.insert(k.clone(), json);
    }
    Value::Object(map)
}
