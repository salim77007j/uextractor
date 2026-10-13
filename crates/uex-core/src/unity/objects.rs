//! Unity object conversion: Texture2D/AudioClip/Mesh/Sprite/TextAsset/Material
//! etc. read from typetree UValues and exported through the existing
//! UExtractor pipelines (PNG/TGA, GLB, WAV/OGG, JSON).

use super::bundle::UnityBundle;
use super::serialized::SerializedFile;
use super::typetree::UValue;
use super::UnityVersion;
use crate::assets::mesh::{ExtractedMesh, MeshSection, Vertex};
use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Unity TextureFormat handling
// ---------------------------------------------------------------------------

/// Unity TextureFormat ids (enums/TextureFormat.py).
pub mod texture_format {
    pub const ALPHA8: i32 = 1;
    pub const ARGB4444: i32 = 2;
    pub const RGB24: i32 = 3;
    pub const RGBA32: i32 = 4;
    pub const ARGB32: i32 = 5;
    pub const ARGBFLOAT: i32 = 6;
    pub const RGB565: i32 = 7;
    pub const BGR24: i32 = 8;
    pub const R16: i32 = 9;
    pub const DXT1: i32 = 10;
    pub const DXT5: i32 = 12;
    pub const RGBA4444: i32 = 13;
    pub const BGRA32: i32 = 14;
    pub const RHALF: i32 = 15;
    pub const RGHALF: i32 = 16;
    pub const RGBAHALF: i32 = 17;
    pub const RFLOAT: i32 = 18;
    pub const RGFLOAT: i32 = 19;
    pub const RGBAFLOAT: i32 = 20;
    pub const YUY2: i32 = 21;
    pub const BC6H: i32 = 24;
    pub const BC7: i32 = 25;
    pub const BC4: i32 = 26;
    pub const BC5: i32 = 27;
    pub const DXT1_CRUNCHED: i32 = 28;
    pub const DXT5_CRUNCHED: i32 = 29;
    pub const PVRTC_RGB2: i32 = 30;
    pub const PVRTC_RGBA2: i32 = 31;
    pub const PVRTC_RGB4: i32 = 32;
    pub const PVRTC_RGBA4: i32 = 33;
    pub const ETC_RGB4: i32 = 34;
    pub const ATC_RGB4: i32 = 35;
    pub const ATC_RGBA8: i32 = 36;
    pub const EAC_R: i32 = 41;
    pub const EAC_RG: i32 = 43;
    pub const ETC2_RGB: i32 = 45;
    pub const ETC2_RGBA1: i32 = 46;
    pub const ETC2_RGBA8: i32 = 47;
    pub const ASTC_RGB_4X4: i32 = 48;
    pub const ASTC_RGB_5X5: i32 = 49;
    pub const ASTC_RGB_6X6: i32 = 50;
    pub const ASTC_RGB_8X8: i32 = 51;
    pub const ASTC_RGB_10X10: i32 = 52;
    pub const ASTC_RGB_12X12: i32 = 53;
    pub const ASTC_RGBA_4X4: i32 = 54;
    pub const ASTC_RGBA_5X5: i32 = 55;
    pub const ASTC_RGBA_6X6: i32 = 56;
    pub const ASTC_RGBA_8X8: i32 = 57;
    pub const ASTC_RGBA_10X10: i32 = 58;
    pub const ASTC_RGBA_12X12: i32 = 59;
    pub const RG16: i32 = 62;
    pub const R8: i32 = 63;
    pub const ETC_RGB4_CRUNCHED: i32 = 64;
    pub const ETC2_RGBA8_CRUNCHED: i32 = 65;
    pub const ASTC_HDR_4X4: i32 = 66;
    pub const ASTC_HDR_5X5: i32 = 67;
    pub const ASTC_HDR_6X6: i32 = 68;
    pub const ASTC_HDR_8X8: i32 = 69;
    pub const ASTC_HDR_10X10: i32 = 70;
    pub const ASTC_HDR_12X12: i32 = 71;
    pub const ASTC_RGBA_4X4_OLD: i32 = 72;
    pub const ASTC_RGBA_6X6_OLD: i32 = 73;
    pub const ASTC_RGBA_8X8_OLD: i32 = 74;
    pub const ASTC_RGBA_10X10_OLD: i32 = 75;
    pub const ASTC_RGBA_12X12_OLD: i32 = 76;
    pub const EAC_R_SIGNED: i32 = 42;
    pub const EAC_RG_SIGNED: i32 = 44;
}

pub fn texture_format_name(fmt: i32) -> String {
    let name = match fmt {
        1 => "Alpha8",
        2 => "ARGB4444",
        3 => "RGB24",
        4 => "RGBA32",
        5 => "ARGB32",
        6 => "ARGBFloat",
        7 => "RGB565",
        8 => "BGR24",
        9 => "R16",
        10 => "DXT1",
        11 => "DXT3",
        12 => "DXT5",
        13 => "RGBA4444",
        14 => "BGRA32",
        15 => "RHalf",
        16 => "RGHalf",
        17 => "RGBAHalf",
        18 => "RFloat",
        19 => "RGFloat",
        20 => "RGBAFloat",
        21 => "YUY2",
        22 => "RGB9e5Float",
        23 => "RGBFloat",
        24 => "BC6H",
        25 => "BC7",
        26 => "BC4",
        27 => "BC5",
        28 => "DXT1Crunched",
        29 => "DXT5Crunched",
        30..=33 => "PVRTC",
        34 => "ETC_RGB4",
        35 => "ATC_RGB4",
        36 => "ATC_RGBA8",
        41 => "EAC_R",
        42 => "EAC_R_SIGNED",
        43 => "EAC_RG",
        44 => "EAC_RG_SIGNED",
        45 => "ETC2_RGB",
        46 => "ETC2_RGBA1",
        47 => "ETC2_RGBA8",
        48..=53 => "ASTC_RGB",
        54..=59 => "ASTC_RGBA",
        60 => "ETC_RGB4_3DS",
        61 => "ETC_RGBA8_3DS",
        62 => "RG16",
        63 => "R8",
        64 => "ETC_RGB4Crunched",
        65 => "ETC2_RGBA8Crunched",
        66..=71 => "ASTC_HDR",
        72..=76 => "ASTC_RGBA",
        _ => "Unknown",
    };
    name.to_string()
}

/// Block-compressed formats supported by texture2ddecoder.
fn block_info(fmt: i32) -> Option<(usize, usize, u32)> {
    // (block width, block height, bits per block)
    use texture_format::*;
    Some(match fmt {
        DXT1 | DXT1_CRUNCHED | ETC_RGB4 | ETC_RGB4_CRUNCHED | ETC2_RGB => (4, 4, 64),
        DXT5 | DXT5_CRUNCHED | ETC2_RGBA8 | ETC2_RGBA8_CRUNCHED | BC7 | BC6H => (4, 4, 128),
        BC4 | EAC_R | EAC_R_SIGNED => (4, 4, 64),
        BC5 | EAC_RG | EAC_RG_SIGNED => (4, 4, 128),
        ATC_RGB4 => (4, 4, 64),
        ATC_RGBA8 => (4, 4, 128),
        ASTC_RGB_4X4 | ASTC_RGBA_4X4 | ASTC_RGBA_4X4_OLD | ASTC_HDR_4X4 => (4, 4, 128),
        ASTC_RGB_5X5 | ASTC_RGBA_5X5 | ASTC_HDR_5X5 => (5, 5, 128),
        ASTC_RGB_6X6 | ASTC_RGBA_6X6 | ASTC_RGBA_6X6_OLD | ASTC_HDR_6X6 => (6, 6, 128),
        ASTC_RGB_8X8 | ASTC_RGBA_8X8 | ASTC_RGBA_8X8_OLD | ASTC_HDR_8X8 => (8, 8, 128),
        ASTC_RGB_10X10 | ASTC_RGBA_10X10 | ASTC_RGBA_10X10_OLD | ASTC_HDR_10X10 => (10, 10, 128),
        ASTC_RGB_12X12 | ASTC_RGBA_12X12 | ASTC_RGBA_12X12_OLD | ASTC_HDR_12X12 => (12, 12, 128),
        _ => return None,
    })
}

/// Bytes per pixel for uncompressed formats (None = unsupported).
fn uncompressed_bpp(fmt: i32) -> Option<u32> {
    use texture_format::*;
    Some(match fmt {
        ALPHA8 | R8 => 1,
        ARGB4444 | RGBA4444 | RGB565 | R16 | RG16 | RGHALF | RHALF => 2,
        RGB24 | BGR24 => 3,
        RGBA32 | ARGB32 | BGRA32 | RGBAHALF | RGFLOAT | YUY2 => 4,
        ARGBFLOAT | RGBAFLOAT => 16,
        RFLOAT => 4,
        _ => return None,
    })
}

fn half_to_f32(h: u16) -> f32 {
    f32::from_bits(((h as u32 & 0x8000) << 16)
        | (((h as u32 >> 10) & 0x1f).saturating_sub(15).min(0x1f) << 23)
        | ((h as u32 & 0x3ff) << 13))
    .max(0.0)
}

/// Decode Unity texture data to RGBA8 (top-down rows).
pub fn decode_texture(width: u32, height: u32, fmt: i32, data: &[u8]) -> Result<Vec<u8>> {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 {
        return Err(Error::asset("zero-sized texture"));
    }
    let mut out = vec![0u8; w * h * 4];

    if let Some((bw, bh, bits)) = block_info(fmt) {
        let blocks_x = (w + bw - 1) / bw;
        let blocks_y = (h + bh - 1) / bh;
        let need = blocks_x * blocks_y * (bits as usize / 8);
        if matches!(fmt, 28 | 29 | 64 | 65) {
            // crunched formats: self-contained Unity crunch streams
            let mut decoded: Vec<u32> = vec![0u32; w * h];
            texture2ddecoder::decode_unity_crunch(data, w, h, &mut decoded)
                .map_err(|e| Error::asset(format!("{} decode: {e}", texture_format_name(fmt))))?;
            let mut out = vec![0u8; w * h * 4];
            for (i, px) in out.chunks_exact_mut(4).enumerate() {
                let v = decoded[i];
                px[0] = ((v >> 16) & 0xFF) as u8;
                px[1] = ((v >> 8) & 0xFF) as u8;
                px[2] = (v & 0xFF) as u8;
                px[3] = ((v >> 24) & 0xFF) as u8;
            }
            let stride = w * 4;
            let mut flipped = vec![0u8; out.len()];
            for row in 0..h {
                let src = (h - 1 - row) * stride;
                let dst = row * stride;
                flipped[dst..dst + stride].copy_from_slice(&out[src..src + stride]);
            }
            return Ok(flipped);
        }
        if data.len() < need {
            return Err(Error::asset(format!(
                "texture data truncated: have {}, need {need}",
                data.len()
            )));
        }
        let mut decoded: Vec<u32> = vec![0u32; w * h];
        match fmt {
            10 => texture2ddecoder::decode_bc1(data, w, h, &mut decoded),
            12 => texture2ddecoder::decode_bc3(data, w, h, &mut decoded),
            26 => texture2ddecoder::decode_bc4(data, w, h, &mut decoded),
            27 => texture2ddecoder::decode_bc5(data, w, h, &mut decoded),
            25 => texture2ddecoder::decode_bc7(data, w, h, &mut decoded),
            24 => texture2ddecoder::decode_bc6_unsigned(data, w, h, &mut decoded),
            34 => texture2ddecoder::decode_etc1(data, w, h, &mut decoded),
            45 => texture2ddecoder::decode_etc2_rgb(data, w, h, &mut decoded),
            46 => texture2ddecoder::decode_etc2_rgba1(data, w, h, &mut decoded),
            47 => texture2ddecoder::decode_etc2_rgba8(data, w, h, &mut decoded),
            41 => texture2ddecoder::decode_eacr(data, w, h, &mut decoded),
            42 => texture2ddecoder::decode_eacr_signed(data, w, h, &mut decoded),
            43 => texture2ddecoder::decode_eacrg(data, w, h, &mut decoded),
            44 => texture2ddecoder::decode_eacrg_signed(data, w, h, &mut decoded),
            35 => texture2ddecoder::decode_atc_rgb4(data, w, h, &mut decoded),
            36 => texture2ddecoder::decode_atc_rgba8(data, w, h, &mut decoded),
            _ => {
                texture2ddecoder::decode_astc(data, w, h, bw, bh, &mut decoded)
            }
        }
        .map_err(|e| Error::asset(format!("{} decode: {e}", texture_format_name(fmt))))?;
        // texture2ddecoder packs BGRA
        for (i, px) in out.chunks_exact_mut(4).enumerate() {
            let v = decoded[i];
            px[0] = ((v >> 16) & 0xFF) as u8;
            px[1] = ((v >> 8) & 0xFF) as u8;
            px[2] = (v & 0xFF) as u8;
            px[3] = ((v >> 24) & 0xFF) as u8;
        }
    } else {
        let bpp = uncompressed_bpp(fmt)
            .ok_or_else(|| Error::unsupported(format!("decode for {}", texture_format_name(fmt))))?;
        let need = w * h * bpp as usize;
        if data.len() < need {
            return Err(Error::asset(format!(
                "texture data truncated: have {}, need {need}",
                data.len()
            )));
        }
        use texture_format::*;
        for (i, px) in out.chunks_exact_mut(4).enumerate() {
            match fmt {
                ALPHA8 => {
                    let a = data[i];
                    px[..3].fill(0);
                    px[3] = a;
                }
                R8 => {
                    px[0] = data[i];
                    px[1] = data[i];
                    px[2] = data[i];
                    px[3] = 255;
                }
                RGB24 => {
                    px[0] = data[i * 3];
                    px[1] = data[i * 3 + 1];
                    px[2] = data[i * 3 + 2];
                    px[3] = 255;
                }
                BGR24 => {
                    px[0] = data[i * 3 + 2];
                    px[1] = data[i * 3 + 1];
                    px[2] = data[i * 3];
                    px[3] = 255;
                }
                RGBA32 => {
                    px.copy_from_slice(&data[i * 4..i * 4 + 4]);
                }
                ARGB32 | BGRA32 => {
                    px[0] = data[i * 4 + 2];
                    px[1] = data[i * 4 + 1];
                    px[2] = data[i * 4];
                    px[3] = data[i * 4 + 3];
                }
                ARGB4444 => {
                    let v = u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
                    px[0] = (((v >> 8) & 0xF) * 17) as u8;
                    px[1] = (((v >> 4) & 0xF) * 17) as u8;
                    px[2] = ((v & 0xF) * 17) as u8;
                    px[3] = ((v >> 12) * 17) as u8;
                }
                RGBA4444 => {
                    let v = u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
                    px[0] = (((v >> 12) & 0xF) * 17) as u8;
                    px[1] = (((v >> 8) & 0xF) * 17) as u8;
                    px[2] = (((v >> 4) & 0xF) * 17) as u8;
                    px[3] = ((v & 0xF) * 17) as u8;
                }
                RGB565 => {
                    let v = u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
                    px[0] = (((v >> 11) & 0x1F) as u32 * 255 / 31) as u8;
                    px[1] = (((v >> 5) & 0x3F) as u32 * 255 / 63) as u8;
                    px[2] = ((v & 0x1F) as u32 * 255 / 31) as u8;
                    px[3] = 255;
                }
                R16 => {
                    let v = u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
                    let g = (v >> 8) as u8;
                    px[0] = g;
                    px[1] = g;
                    px[2] = g;
                    px[3] = 255;
                }
                RG16 => {
                    px[0] = data[i * 2 + 1];
                    px[1] = data[i * 2];
                    px[2] = 0;
                    px[3] = 255;
                }
                RHALF => {
                    let v = half_to_f32(u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]));
                    let b = (v * 255.0) as u8;
                    px[0] = b;
                    px[1] = b;
                    px[2] = b;
                    px[3] = 255;
                }
                RGHALF => {
                    px[0] = (half_to_f32(u16::from_le_bytes([data[i * 4 + 2], data[i * 4 + 3]])) * 255.0) as u8;
                    px[1] = (half_to_f32(u16::from_le_bytes([data[i * 4], data[i * 4 + 1]])) * 255.0) as u8;
                    px[2] = 0;
                    px[3] = 255;
                }
                RGBAHALF => {
                    px[0] = (half_to_f32(u16::from_le_bytes([data[i * 8], data[i * 8 + 1]])) * 255.0) as u8;
                    px[1] = (half_to_f32(u16::from_le_bytes([data[i * 8 + 2], data[i * 8 + 3]])) * 255.0) as u8;
                    px[2] = (half_to_f32(u16::from_le_bytes([data[i * 8 + 4], data[i * 8 + 5]])) * 255.0) as u8;
                    px[3] = (half_to_f32(u16::from_le_bytes([data[i * 8 + 6], data[i * 8 + 7]])) * 255.0) as u8;
                }
                RFLOAT => {
                    let v = f32::from_le_bytes([data[i * 4], data[i * 4 + 1], data[i * 4 + 2], data[i * 4 + 3]]);
                    px[0] = v.clamp(0.0, 1.0).mul_add(255.0, 0.5) as u8;
                    px[1] = px[0];
                    px[2] = px[0];
                    px[3] = 255;
                }
                RGFLOAT => {
                    px[0] = (f32::from_le_bytes([data[i * 8 + 8], data[i * 8 + 9], data[i * 8 + 10], data[i * 8 + 11]])
                        .clamp(0.0, 1.0)
                        * 255.0) as u8;
                    px[1] = (f32::from_le_bytes([data[i * 8], data[i * 8 + 1], data[i * 8 + 2], data[i * 8 + 3]])
                        .clamp(0.0, 1.0)
                        * 255.0) as u8;
                    px[2] = 0;
                    px[3] = 255;
                }
                ARGBFLOAT | RGBAFLOAT => {
                    let ch = |j: usize| {
                        (f32::from_le_bytes([
                            data[i * 16 + j],
                            data[i * 16 + j + 1],
                            data[i * 16 + j + 2],
                            data[i * 16 + j + 3],
                        ])
                        .clamp(0.0, 1.0)
                            * 255.0) as u8
                    };
                    px[0] = ch(0);
                    px[1] = ch(4);
                    px[2] = ch(8);
                    px[3] = ch(12);
                }
                YUY2 => {
                    // packed 2 pixels per 4 bytes
                    let pair = i & !1;
                    let base = pair * 2;
                    let y = data[base + if i % 2 == 0 { 0 } else { 2 }] as f32;
                    let u = data[base + 1] as f32 - 128.0;
                    let v = data[base + 3] as f32 - 128.0;
                    let r = y + 1.402 * v;
                    let g = y - 0.344136 * u - 0.714136 * v;
                    let b = y + 1.772 * u;
                    px[0] = r.clamp(0.0, 255.0) as u8;
                    px[1] = g.clamp(0.0, 255.0) as u8;
                    px[2] = b.clamp(0.0, 255.0) as u8;
                    px[3] = 255;
                }
                _ => return Err(Error::unsupported(format!("decode for {}", texture_format_name(fmt)))),
            }
        }
    }

    // Unity stores image data bottom-up; flip to top-down
    let stride = w * 4;
    let mut flipped = vec![0u8; out.len()];
    for row in 0..h {
        let src = (h - 1 - row) * stride;
        let dst = row * stride;
        flipped[dst..dst + stride].copy_from_slice(&out[src..src + stride]);
    }
    Ok(flipped)
}

// ---------------------------------------------------------------------------
// Resource (resS / streaming data) resolution
// ---------------------------------------------------------------------------

/// Context for resolving m_Source streams: the serialized file's own bundle
/// (for "archive:/CAB-x/CAB-x.resS" paths) plus the on-disk directory.
pub struct ResourceContext<'a> {
    pub bundle: Option<&'a UnityBundle>,
    pub base_dir: std::path::PathBuf,
    /// cache of sibling resource files read from disk
    cache: std::cell::RefCell<std::collections::HashMap<String, std::rc::Rc<Vec<u8>>>>,
}

impl<'a> ResourceContext<'a> {
    pub fn new(bundle: Option<&'a UnityBundle>, base_dir: std::path::PathBuf) -> Self {
        ResourceContext {
            bundle,
            base_dir,
            cache: std::cell::RefCell::new(Default::default()),
        }
    }

    /// Resolve a stream path (UnityPy ResourceReader.get_resource_data port).
    pub fn read(&self, res_path: &str, offset: u64, size: u64) -> Result<Vec<u8>> {
        let basename = res_path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(res_path)
            .to_string();
        let name = basename
            .rsplit_once('.')
            .map(|(n, _)| n.to_string())
            .unwrap_or_else(|| basename.clone());
        let candidates = [
            basename.clone(),
            format!("{name}.resource"),
            format!("{name}.assets.resS"),
            format!("{name}.resS"),
        ];
        // 1) inside the bundle (archive:/CAB-x/CAB-x.resS)
        if let Some(bundle) = self.bundle {
            for cand in &candidates {
                if let Some(node) = bundle.nodes.iter().find(|n| n.path == *cand || n.path.ends_with(cand)) {
                    return bundle.read_range(node.offset + offset, size);
                }
            }
        }
        // 2) sibling files on disk
        for cand in &candidates {
            let p = self.base_dir.join(cand);
            if let Ok(bytes) = std::fs::read(&p) {
                let rc = std::rc::Rc::new(bytes);
                self.cache.borrow_mut().insert(cand.clone(), rc.clone());
                let s = offset as usize;
                let e = s + size as usize;
                if e > rc.len() {
                    return Err(Error::asset(format!(
                        "resource {cand} range {s}..{e} beyond {} bytes",
                        rc.len()
                    )));
                }
                return Ok(rc[s..e].to_vec());
            }
        }
        Err(Error::NotFound(format!(
            "resource file '{basename}' not found in bundle or next to the archive"
        )))
    }
}

// ---------------------------------------------------------------------------
// Texture2D
// ---------------------------------------------------------------------------

pub struct UnityTexture {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub format_id: i32,
    pub format: String,
}

/// Extract a Texture2D object's image data (inline or via StreamData).
pub fn texture2d_data(
    value: &UValue,
    sf: &SerializedFile,
    res: &ResourceContext,
) -> Result<(UnityTexture, Vec<u8>)> {
    let name = value.get_str("m_Name").unwrap_or("texture").to_string();
    let width = value.get_i64("m_Width").unwrap_or(0) as u32;
    let height = value.get_i64("m_Height").unwrap_or(0) as u32;
    let fmt = value.get_i64("m_TextureFormat").unwrap_or(0) as i32;
    if width == 0 || height == 0 {
        return Err(Error::asset(format!("texture '{name}' has zero size")));
    }

    // modern: m_StreamData { path, offset, size }
    let stream_path = value.get("m_StreamData").and_then(|s| s.get_str("path")).unwrap_or("");
    let data: Vec<u8> = if !stream_path.is_empty() {
        let offset = value
            .get("m_StreamData")
            .and_then(|s| s.get("offset"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let size = value
            .get("m_StreamData")
            .and_then(|s| s.get("size"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        res.read(stream_path, offset, size)?
    } else if let Some(image_data) = value.get("image_data").and_then(|v| v.as_byte_vec()) {
        image_data
    } else if let Some(img) = value.get("image data").and_then(|v| v.as_byte_vec()) {
        img
    } else {
        Vec::new()
    };
    if data.is_empty() {
        return Err(Error::asset(format!("texture '{name}' has no image data")));
    }
    let rgba = decode_texture(width, height, fmt, &data)?;
    Ok((
        UnityTexture {
            name,
            width,
            height,
            format_id: fmt,
            format: texture_format_name(fmt),
        },
        rgba,
    ))
}

// ---------------------------------------------------------------------------
// AudioClip
// ---------------------------------------------------------------------------

/// Compression format ids (AudioCompressionFormat).
pub fn audio_extension(compression_format: i32, version: UnityVersion) -> &'static str {
    match compression_format {
        0 => if version.major() < 5 { ".ogg" } else { ".fsb" },
        1 => ".fsb",  // Vorbis (FSB5 container)
        2 => ".fsb",  // ADPCM
        3 => ".mp3",
        4 | 5 => ".vag",
        6 => ".wav",
        7 => ".m4a",
        8 => ".fsb",
        9 => ".at9",
        _ => ".audioclip",
    }
}

/// Extract an AudioClip's raw audio payload.
pub fn audio_data(value: &UValue, sf: &SerializedFile, res: &ResourceContext) -> Result<(String, Vec<u8>)> {
    let version = sf.unity_version;
    let load_type = value.get_i64("m_LoadType").unwrap_or(-1);
    let compression_format = value.get_i64("m_CompressionFormat").unwrap_or(-1);
    let channels = value.get_i64("m_Channels").unwrap_or(0);
    let frequency = value.get_i64("m_Frequency").unwrap_or(0);

    // resource reference: m_Resource (AudioData/ResourceInfo 2022+) or m_Resource (ResourceInfo 5.x)
    let resource = value.get("m_Resource");
    let (src, offset, size) = if let Some(r) = resource {
        let src = r.get_str("m_Source").or_else(|| r.get_str("m_SourcePath")).unwrap_or("").to_string();
        let offset = r.get("m_Offset").and_then(|v| v.as_u64()).unwrap_or(0);
        let size = r.get("m_Size").and_then(|v| v.as_u64()).unwrap_or(0);
        (src, offset, size)
    } else {
        (String::new(), 0, 0)
    };

    if !src.is_empty() && size > 0 {
        let data = res.read(&src, offset, size)?;
        let ext = if compression_format >= 0 {
            audio_extension(compression_format as i32, version)
        } else {
            ".fsb"
        };
        return Ok((ext.to_string(), data));
    }

    // legacy inline data (Unity 4.x): m_CompressedData / m_Data
    for key in ["m_CompressedData", "m_Data"] {
        if let Some(b) = value.get(key).and_then(|v| v.as_bytes()) {
            let ext = match load_type {
                1 => ".ogg",
                2 => ".wav",
                _ => ".fsb",
            };
            return Ok((ext.to_string(), b.to_vec()));
        }
    }
    Err(Error::asset(format!(
        "audio clip has no data (channels={channels}, freq={frequency})"
    )))
}

// ---------------------------------------------------------------------------
// Mesh -> ExtractedMesh (GLB path)
// ---------------------------------------------------------------------------

/// Vertex channel format -> component byte size (per MeshHelper maps).
fn vertex_component_size(fmt: i32, version: UnityVersion) -> usize {
    if version.major() < 2017 {
        // VertexChannelFormat
        match fmt {
            0 => 4, // float
            1 => 2, // float16
            2 => 1, // color (u8)
            3 => 1, // byte
            4 => 4, // u32
            _ => 4,
        }
    } else if version.major() < 2019 {
        // VertexFormat2017
        match fmt {
            0 => 4,
            1 => 2,
            2 => 1,
            3 => 1,
            4 => 1,
            5 => 2,
            6 => 2,
            7 => 1,
            8 => 1,
            9 => 2,
            10 => 2,
            11 => 4,
            _ => 4,
        }
    } else {
        // VertexFormat (2019+)
        match fmt {
            0 => 4,
            1 => 2,
            2 => 1,
            3 => 1,
            4 => 2,
            5 => 2,
            6 => 1,
            7 => 1,
            8 => 2,
            9 => 2,
            10 => 4,
            11 => 4,
            _ => 4,
        }
    }
}

fn read_channel_values(
    data: &[u8],
    stream_offset: usize,
    stride: usize,
    channel_offset: usize,
    dimension: usize,
    comp_size: usize,
    count: usize,
) -> Result<Vec<f32>> {
    let mut out = Vec::with_capacity(count * dimension);
    let mut voff = stream_offset + channel_offset;
    for _ in 0..count {
        for c in 0..dimension {
            let s = voff + c * comp_size;
            let e = s + comp_size;
            if e > data.len() {
                return Err(Error::asset("vertex data truncated"));
            }
            let v = match comp_size {
                1 => data[s] as f32,
                2 => {
                    let u = u16::from_le_bytes([data[s], data[s + 1]]);
                    u as f32
                }
                4 => f32::from_le_bytes([data[s], data[s + 1], data[s + 2], data[s + 3]]),
                _ => 0.0,
            };
            out.push(v);
        }
        voff += stride;
    }
    Ok(out)
}

/// Parse a Mesh object into an ExtractedMesh for GLB export.
pub fn mesh_to_extracted(value: &UValue, name: &str, sf: &SerializedFile, res: &ResourceContext) -> Result<ExtractedMesh> {
    let version = sf.unity_version;
    let vertex_data = value
        .get("m_VertexData")
        .ok_or_else(|| Error::asset("mesh has no m_VertexData"))?;
    let vertex_count = vertex_data.get_i64("m_VertexCount").unwrap_or(0) as usize;
    if vertex_count == 0 {
        return Err(Error::asset("mesh has zero vertices"));
    }

    // vertex buffer: m_DataSize inline, or m_StreamData external
    let mut vdata: Vec<u8> = Vec::new();
    if let Some(sd) = vertex_data.get("m_StreamData") {
        let path = sd.get_str("path").unwrap_or("");
        if !path.is_empty() {
            let offset = sd.get("offset").and_then(|v| v.as_u64()).unwrap_or(0);
            let size = sd.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
            vdata = res.read(path, offset, size)?;
        }
    }
    if vdata.is_empty() {
        if let Some(d) = vertex_data.get("m_DataSize").and_then(|v| v.as_byte_vec()) {
            vdata = d;
        }
    }
    if vdata.is_empty() {
        return Err(Error::asset("mesh has no vertex buffer"));
    }

    // channels
    let channels = vertex_data
        .get("m_Channels")
        .and_then(|c| c.as_array())
        .ok_or_else(|| Error::asset("mesh has no m_Channels"))?;

    // compute streams (MeshHelper.get_streams)
    struct Stream {
        offset: usize,
        stride: usize,
        mask: u32,
    }
    let mut streams: Vec<Stream> = Vec::new();
    {
        let mut offset = 0usize;
        let stream_count = 1 + channels
            .iter()
            .filter_map(|c| c.get_i64("stream"))
            .max()
            .unwrap_or(0) as usize;
        for s in 0..stream_count {
            let mut mask = 0u32;
            let mut stride = 0usize;
            for (chn, c) in channels.iter().enumerate() {
                let st = c.get_i64("stream").unwrap_or(0) as usize;
                if st == s {
                    let dim = c.get_i64("dimension").unwrap_or(0) as usize;
                    if dim > 0 {
                        mask |= 1 << chn;
                        let fmt = c.get_i64("format").unwrap_or(0) as i32;
                        stride += (dim & 0xF) * vertex_component_size(fmt, version);
                    }
                }
            }
            streams.push(Stream { offset, stride, mask });
            offset += vertex_count * stride;
            offset = (offset + 15) & !15; // 16-byte align between streams
        }
    }

    // vertex positions (channel 0) + uv0 (channel 3 pre-2018 / 4 in 2018+)
    let get_channel = |chn: usize| -> Option<(usize, i32, usize, usize, usize)> {
        channels.get(chn).and_then(|c| {
            let dim = c.get_i64("dimension").unwrap_or(0) as usize;
            if dim == 0 {
                return None;
            }
            let st = c.get_i64("stream").unwrap_or(0) as usize;
            let fmt = c.get_i64("format").unwrap_or(0) as i32;
            let off = c.get_i64("offset").unwrap_or(0) as usize;
            let stream = streams.get(st)?;
            Some((stream.offset, fmt, off, stream.stride, dim))
        })
    };
    let decode_channel = |chn: usize| -> Result<Option<Vec<f32>>> {
        let Some((stream_off, fmt, ch_off, stride, dim)) = get_channel(chn) else {
            return Ok(None);
        };
        let comp = vertex_component_size(fmt, version);
        Ok(Some(read_channel_values(
            &vdata,
            stream_off,
            stride,
            ch_off,
            dim & 0xF,
            comp,
            vertex_count,
        )?))
    };

    let positions = decode_channel(0)?.ok_or_else(|| Error::asset("mesh has no position channel"))?;
    let normals = decode_channel(1)?;
    let uv0 = decode_channel(if version.major() >= 2018 { 4 } else { 3 })?;

    // index buffer
    let index_buffer = value
        .get("m_IndexBuffer")
        .and_then(|v| v.as_byte_vec())
        .ok_or_else(|| Error::asset("mesh has no index buffer"))?;
    let index_format = value.get_i64("m_IndexFormat").unwrap_or(0);
    // UnityPy: m_Use16BitIndices field exists on old versions; 2017.4+ uses
    // m_IndexFormat (0 = 16 bit, 1 = 32 bit).
    let use16 = match value.get("m_Use16BitIndices") {
        Some(v) => v.as_i64().map(|x| x != 0).unwrap_or(true),
        None => index_format == 0,
    };
    let indices: Vec<u32> = if use16 {
        index_buffer
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]) as u32)
            .collect()
    } else {
        index_buffer
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    };

    // submeshes
    let submeshes = value
        .get("m_SubMeshes")
        .and_then(|s| s.as_array())
        .ok_or_else(|| Error::asset("mesh has no submeshes"))?;
    let mut face_ranges: Vec<(usize, usize)> = Vec::new();
    for sm in submeshes {
        let first_byte = sm.get_i64("firstByte").unwrap_or(0) as usize;
        let index_count = sm.get_i64("indexCount").unwrap_or(0) as usize;
        let topology = sm.get_i64("topology").unwrap_or(0);
        if topology != 0 {
            continue; // lines/points/quads skipped for GLB
        }
        let start = first_byte / if use16 { 2 } else { 4 };
        face_ranges.push((start, start + index_count));
    }
    if face_ranges.is_empty() {
        face_ranges.push((0, indices.len()));
    }

    let mut mesh = ExtractedMesh {
        name: name.to_string(),
        vertices: Vec::with_capacity(vertex_count),
        sections: Vec::new(),
        bones: Vec::new(),
        root_bones: Vec::new(),
        bone_children: Vec::new(),
        is_skeletal: false,
        materials: Vec::new(),
    };
    for vi in 0..vertex_count {
        let mut v = Vertex {
            position: [positions[vi * 3], positions[vi * 3 + 1], positions[vi * 3 + 2]],
            ..Default::default()
        };
        if let Some(n) = &normals {
            v.normal = [n[vi * 3], n[vi * 3 + 1], n[vi * 3 + 2]];
        }
        if let Some(uv) = &uv0 {
            v.uv = vec![[uv[vi * 2], uv[vi * 2 + 1]]];
        }
        mesh.vertices.push(v);
    }
    for (si, (s, e)) in face_ranges.iter().enumerate() {
        let start = *s;
        let end = (*e).min(indices.len());
        mesh.sections.push(MeshSection {
            material_index: si as i32,
            indices: indices[start..end].to_vec(),
        });
    }

    // Unity uses a left-handed Y-up system; GLTF is right-handed Y-up.
    // Flip Z of positions and normals, and V of UVs (standard conversion).
    for v in &mut mesh.vertices {
        v.position[2] = -v.position[2];
        v.normal[2] = -v.normal[2];
        for uv in &mut v.uv {
            uv[1] = 1.0 - uv[1];
        }
    }
    for s in &mut mesh.sections {
        for tri in s.indices.chunks_exact_mut(3) {
            tri.swap(0, 1);
        }
    }
    Ok(mesh)
}

// ---------------------------------------------------------------------------
// Sprite
// ---------------------------------------------------------------------------

/// Crop a Sprite from its referenced Texture2D.
pub fn sprite_rgba(
    value: &UValue,
    name: &str,
    sf: &SerializedFile,
    res: &ResourceContext,
    objects: &[(super::serialized::ObjectInfo, String, UValue)],
) -> Result<(UnityTexture, Vec<u8>)> {
    let rect = value
        .get("m_Rect")
        .ok_or_else(|| Error::asset("sprite has no m_Rect"))?;
    let x = rect.get_i64("x").unwrap_or(0) as u32;
    let y = rect.get_i64("y").unwrap_or(0) as u32;
    let w = rect.get_i64("width").unwrap_or(0) as u32;
    let h = rect.get_i64("height").unwrap_or(0) as u32;
    if w == 0 || h == 0 {
        return Err(Error::asset(format!("sprite '{name}' has zero size")));
    }

    // texture reference from m_RD.texture
    let tex_pptr = value
        .get("m_RD")
        .and_then(|rd| rd.get("texture"))
        .cloned()
        .or_else(|| value.get("m_AtlasTags").map(|_| UValue::Array(vec![])));
    let Some(UValue::PPtr { file_id, path_id }) = tex_pptr else {
        return Err(Error::asset(format!("sprite '{name}' texture reference missing")));
    };
    if file_id != 0 {
        return Err(Error::asset(format!(
            "sprite '{name}' references an external texture atlas (file_id {file_id}); open the atlas archive together with this file"
        )));
    }
    let Some((_, _, tex_value)) = objects.iter().find(|(o, _, _)| o.path_id == path_id) else {
        return Err(Error::asset(format!("sprite '{name}' texture object not found")));
    };
    let (tex, tex_rgba) = texture2d_data(tex_value, sf, res)?;
    if tex.width < x + w || tex.height < y + h {
        return Err(Error::asset(format!(
            "sprite '{name}' rect {x},{y}+{w}x{h} exceeds texture {}x{}",
            tex.width, tex.height
        )));
    }
    // crop; texture2d_data returns a TOP-DOWN image while m_Rect uses
    // bottom-up UV coordinates: the rect's top edge on the top-down image
    // is at tex.height - (y + h), and rows go downward from there
    let stride = tex.width as usize * 4;
    let mut out = vec![0u8; (w * h * 4) as usize];
    for row in 0..h as usize {
        let src_row = tex.height as usize - (y as usize + h as usize) + row;
        let src = src_row * stride + x as usize * 4;
        let dst = row * w as usize * 4;
        out[dst..dst + w as usize * 4].copy_from_slice(&tex_rgba[src..src + w as usize * 4]);
    }
    Ok((
        UnityTexture {
            name: name.to_string(),
            width: w,
            height: h,
            format_id: 4,
            format: "RGBA32".into(),
        },
        out,
    ))
}

// ---------------------------------------------------------------------------
// Text / misc
// ---------------------------------------------------------------------------

pub fn text_asset_bytes(value: &UValue) -> Result<Vec<u8>> {
    if let Some(b) = value.get("m_Script").and_then(|v| v.as_byte_vec()) {
        return Ok(b);
    }
    Err(Error::asset("text asset has no m_Script"))
}
