//! UTexture2D extraction: platform data parsing (classic + zen), bulk data resolution
//! (.uexp inline / .ubulk / payload-at-end), pixel format handling, decode to RGBA
//! via texture2ddecoder, and export to PNG / TGA / DDS (with original BC data).

use crate::error::{Error, Result};
use crate::io::UexRead;

#[derive(Debug, Clone)]
pub struct TextureMip {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Texture {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub format_name: String,
    pub mips: Vec<TextureMip>,
}

/// UE EPixelFormat values → canonical names (subset; unknown values pass through).
pub fn pixel_format_name(value: i32) -> String {
    let s = match value {
        1 => "PF_A32B32G32R32F",
        2 => "PF_A8R8G8B8",
        3 => "PF_G8",
        4 => "PF_G16",
        5 => "PF_DXT1",
        6 => "PF_DXT3",
        7 => "PF_DXT5",
        10 => "PF_FloatRGB",
        11 => "PF_FloatRGBA",
        16 => "PF_R32F",
        17 => "PF_G16R16",
        18 => "PF_G16R16F",
        20 => "PF_G32R32F",
        21 => "PF_A2B10G10R10",
        22 => "PF_A16B16G16R16",
        25 => "PF_R16F",
        27 => "PF_BC5",
        28 => "PF_V8U8",
        29 => "PF_A1",
        30 => "PF_FloatR11G11B10",
        31 => "PF_A8",
        32 => "PF_R32_UINT",
        33 => "PF_R32_SINT",
        34 => "PF_BC4",
        35 => "PF_ATC_RGB",
        36 => "PF_ATC_RGBA_E",
        37 => "PF_ATC_RGBA_I",
        38 => "PF_B8G8R8A8",
        39 => "PF_BC7",
        40 => "PF_R8G8B8A8",
        42 => "PF_ETC1",
        43 => "PF_ETC2_RGB",
        44 => "PF_ETC2_RGBA",
        45 => "PF_ASTC_4x4",
        46 => "PF_ASTC_6x6",
        47 => "PF_ASTC_8x8",
        48 => "PF_ASTC_10x10",
        49 => "PF_ASTC_12x12",
        _ => return format!("PF_Unknown_{value}"),
    };
    s.to_string()
}

pub fn blocks(width: u32, height: u32, block: u32) -> (u32, u32) {
    ((width + block - 1) / block, (height + block - 1) / block)
}

pub use blocks as blocks_of;

/// Bytes per pixel / per block for a format.
pub fn format_layout(format: &str) -> (u32, u32) {
    // (bytes_per_block_or_pixel, block_size) — block_size 1 = uncompressed
    match format {
        "PF_DXT1" | "PF_BC4" | "PF_ATC_RGB" | "PF_ETC1" | "PF_ETC2_RGB" => (8, 4),
        "PF_DXT3" | "PF_DXT5" | "PF_BC5" | "PF_BC7" | "PF_ATC_RGBA_E" | "PF_ATC_RGBA_I" | "PF_ETC2_RGBA" => (16, 4),
        "PF_ASTC_4x4" => (16, 4),
        "PF_ASTC_6x6" => (16, 6),
        "PF_ASTC_8x8" => (16, 8),
        "PF_ASTC_10x10" => (16, 10),
        "PF_ASTC_12x12" => (16, 12),
        "PF_A32B32G32R32F" => (16, 1),
        "PF_A8R8G8B8" | "PF_B8G8R8A8" | "PF_R8G8B8A8" => (4, 1),
        "PF_FloatRGBA" => (8, 1),
        "PF_FloatRGB" | "PF_G32R32F" => (12, 1),
        "PF_R32F" => (4, 1),
        "PF_G8" | "PF_A8" | "PF_R32_UINT" | "PF_R32_SINT" => (1, 1),
        "PF_G16" | "PF_R16F" => (2, 1),
        "PF_G16R16" | "PF_G16R16F" => (4, 1),
        "PF_A16B16G16R16" => (8, 1),
        "PF_V8U8" => (2, 1),
        "PF_A2B10G10R10" | "PF_FloatR11G11B10" => (4, 1),
        _ => (4, 1),
    }
}

/// Data size for one mip of a format.
pub fn mip_data_size(width: u32, height: u32, format: &str) -> u64 {
    let (bpp, block) = format_layout(format);
    if block == 1 {
        (width as u64) * (height as u64) * bpp as u64
    } else {
        let (bx, by) = blocks(width, height, block);
        bx as u64 * by as u64 * bpp as u64
    }
}

/// Decode a compressed/uncompressed mip to RGBA8.
pub fn decode_mip(width: u32, height: u32, format: &str, data: &[u8]) -> Result<Vec<u8>> {
    let w = width as usize;
    let h = height as usize;
    let mut decoded: Vec<u32> = vec![0u32; w * h];
    match format {
        "PF_DXT1" => texture2ddecoder::decode_bc1(data, w, h, &mut decoded),
        "PF_DXT3" => texture2ddecoder::decode_bc2(data, w, h, &mut decoded),
        "PF_DXT5" => texture2ddecoder::decode_bc3(data, w, h, &mut decoded),
        "PF_BC4" => texture2ddecoder::decode_bc4(data, w, h, &mut decoded),
        "PF_BC5" => texture2ddecoder::decode_bc5(data, w, h, &mut decoded),
        "PF_BC7" => texture2ddecoder::decode_bc7(data, w, h, &mut decoded),
        "PF_ETC1" => texture2ddecoder::decode_etc1(data, w, h, &mut decoded),
        "PF_ETC2_RGB" => texture2ddecoder::decode_etc2_rgb(data, w, h, &mut decoded),
        "PF_ETC2_RGBA" => texture2ddecoder::decode_etc2_rgba8(data, w, h, &mut decoded),
        "PF_ASTC_4x4" | "PF_ASTC_6x6" | "PF_ASTC_8x8" | "PF_ASTC_10x10" | "PF_ASTC_12x12" => {
            let dims = format.trim_start_matches("PF_ASTC_");
            let (bw, bh) = dims
                .split_once('x')
                .and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)))
                .unwrap_or((4, 4));
            texture2ddecoder::decode_astc(data, w, h, bw, bh, &mut decoded)
        }
        _ => {
            return decode_uncompressed(format, data, w, h);
        }
    }
    .map_err(|e| Error::asset(format!("{format} decode: {e}")))?;

    // texture2ddecoder stores colors as from_le_bytes([b, g, r, a])
    let mut out = vec![0u8; w * h * 4];
    for (i, px) in out.chunks_exact_mut(4).enumerate() {
        let v = decoded[i];
        px[0] = ((v >> 16) & 0xFF) as u8;
        px[1] = ((v >> 8) & 0xFF) as u8;
        px[2] = (v & 0xFF) as u8;
        px[3] = ((v >> 24) & 0xFF) as u8;
    }
    Ok(out)
}

/// Uncompressed pixel formats decoded directly.
fn decode_uncompressed(format: &str, data: &[u8], w: usize, h: usize) -> Result<Vec<u8>> {
    let need = |n: usize| -> Result<()> {
        if data.len() < n {
            Err(Error::asset("texture data truncated"))
        } else {
            Ok(())
        }
    };
    let mut out = vec![0u8; w * h * 4];
    match format {
        "PF_A8R8G8B8" | "PF_B8G8R8A8" => {
            need(w * h * 4)?;
            for (i, px) in out.chunks_exact_mut(4).enumerate() {
                px[0] = data[i * 4 + 2];
                px[1] = data[i * 4 + 1];
                px[2] = data[i * 4];
                px[3] = data[i * 4 + 3];
            }
        }
        "PF_R8G8B8A8" => {
            need(w * h * 4)?;
            out.copy_from_slice(&data[..w * h * 4]);
        }
        "PF_G8" | "PF_A8" => {
            need(w * h)?;
            for (i, px) in out.chunks_exact_mut(4).enumerate() {
                px[0] = data[i];
                px[1] = data[i];
                px[2] = data[i];
                px[3] = 255;
            }
        }
        "PF_FloatRGBA" => {
            need(w * h * 8)?;
            for (i, px) in out.chunks_exact_mut(4).enumerate() {
                px[0] = half_to_u8(u16::from_le_bytes([data[i * 8], data[i * 8 + 1]]));
                px[1] = half_to_u8(u16::from_le_bytes([data[i * 8 + 2], data[i * 8 + 3]]));
                px[2] = half_to_u8(u16::from_le_bytes([data[i * 8 + 4], data[i * 8 + 5]]));
                px[3] = half_to_u8(u16::from_le_bytes([data[i * 8 + 6], data[i * 8 + 7]]));
            }
        }
        "PF_V8U8" => {
            need(w * h * 2)?;
            for (i, px) in out.chunks_exact_mut(4).enumerate() {
                px[0] = data[i * 2].wrapping_add(128);
                px[1] = data[i * 2 + 1].wrapping_add(128);
                px[2] = 0;
                px[3] = 255;
            }
        }
        "PF_G16R16" | "PF_G16R16F" => {
            need(w * h * 4)?;
            for (i, px) in out.chunks_exact_mut(4).enumerate() {
                let g = u16::from_le_bytes([data[i * 4], data[i * 4 + 1]]);
                let r = u16::from_le_bytes([data[i * 4 + 2], data[i * 4 + 3]]);
                px[0] = (r >> 8) as u8;
                px[1] = (g >> 8) as u8;
                px[2] = 0;
                px[3] = 255;
            }
        }
        "PF_FloatR11G11B10" | "PF_A2B10G10R10" | "PF_R32F" | "PF_G32R32F" | "PF_A32B32G32R32F"
        | "PF_A16B16G16R16" | "PF_G16" | "PF_R16F" => {
            return Err(Error::unsupported(format!("decode for {format}")));
        }
        _ => return Err(Error::unsupported(format!("decode for {format}"))),
    }
    Ok(out)
}

fn half_to_u8(half: u16) -> u8 {
    let f = f32::from_bits(((half as u32 & 0x8000) << 16)
        | (((half as u32 >> 10) & 0x1f).saturating_sub(15).min(0x1f) << 23)
        | ((half as u32 & 0x3ff) << 13));
    let v = if half >> 10 == 0 { 0.0 } else { f };
    (v.clamp(0.0, 1.0) * 255.0) as u8
}

/// Bulk data flags (UE EBulkDataFlags subset).
pub const BULKDATA_PayloadAtEndOfFile: u32 = 0x0001;
pub const BULKDATA_SerializeCompressedZLIB: u32 = 0x0002;
pub const BULKDATA_ForceInlinePayload: u32 = 0x0004;
pub const BULKDATA_ForceNOTInlinePayload: u32 = 0x0008;
pub const BULKDATA_PayloadInSeperateFile: u32 = 0x0010;
pub const BULKDATA_SerializeCompressedBZIP2: u32 = 0x0100;
pub const BULKDATA_PayloadStreamed: u32 = 0x0400;
pub const BULKDATA_MemoryMappedPayload: u32 = 0x2000;
pub const BULKDATA_SizeOnDisk: u32 = 0x4000;
pub const BULKDATA_SerializeCompressedZSTD: u32 = 0x0080;
pub const BULKDATA_SerializeCompressedOodle: u32 = 0x8000;

/// Parse a .ubulk file: [flags u32][element count i64][size on disk i64][payload]
pub fn parse_ubulk(ubulk: &[u8]) -> Result<Vec<u8>> {
    let mut r = std::io::Cursor::new(ubulk);
    let flags = r.u32()?;
    let _element_count = r.i64()?;
    let size_on_disk = r.i64()? as usize;
    let payload = r.read_bytes(size_on_disk.min(ubulk.len()))?;
    decompress_bulk_payload(&payload, flags, size_on_disk)
}

fn decompress_bulk_payload(payload: &[u8], flags: u32, _size_on_disk: usize) -> Result<Vec<u8>> {
    if flags & BULKDATA_SerializeCompressedZLIB != 0 {
        let mut dec = flate2::read::ZlibDecoder::new(payload);
        let mut out = Vec::new();
        std::io::Read::read_to_end(&mut dec, &mut out)?;
        Ok(out)
    } else if flags & BULKDATA_SerializeCompressedZSTD != 0 {
        let mut dec = crate::compress::Method::Zstd;
        let mut out = vec![0u8; _size_on_disk.max(1)];
        let n = crate::compress::decompress(dec, payload, &mut out)?;
        out.truncate(n);
        let _ = &mut dec;
        Ok(out)
    } else if flags & BULKDATA_SerializeCompressedOodle != 0 {
        // Oodle-compressed bulk: [u32 uncompressed size][data]
        let mut r = std::io::Cursor::new(payload);
        let uncompressed = r.u32()? as usize;
        let data = &payload[4..];
        let mut out = vec![0u8; uncompressed];
        crate::compress::decompress(crate::compress::Method::Oodle, data, &mut out)?;
        Ok(out)
    } else {
        Ok(payload.to_vec())
    }
}

/// A resolved texture platform data parse over a byte stream (uexp tail or zen export).
struct PlatformDataReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> PlatformDataReader<'a> {
    fn i32(&mut self) -> Result<i32> {
        let b = self
            .data
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| Error::asset("platform data truncated"))?;
        self.pos += 4;
        Ok(i32::from_le_bytes(b.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64> {
        let b = self
            .data
            .get(self.pos..self.pos + 8)
            .ok_or_else(|| Error::asset("platform data truncated"))?;
        self.pos += 8;
        Ok(i64::from_le_bytes(b.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(self.i32()? as u32)
    }
    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let b = self
            .data
            .get(self.pos..self.pos + n)
            .ok_or_else(|| Error::asset("platform data truncated"))?;
        self.pos += n;
        Ok(b)
    }
}

#[derive(Debug, Clone)]
struct RawMip {
    width: u32,
    height: u32,
    bulk_flags: u32,
    element_count: i64,
    size_on_disk: i64,
    file_offset: i64,
    inline_data: Option<Vec<u8>>,
}

/// Try to parse FTexturePlatformData at `offset` within `data` (export payload).
/// Validates aggressively; returns None when the layout doesn't check out.
pub fn try_parse_platform_data(
    data: &[u8],
    offset: usize,
    ubulk: Option<&[u8]>,
    combined: Option<&[u8]>,
) -> Option<(i32, i32, String, Vec<TextureMip>, usize)> {
    let mut r = PlatformDataReader { data, pos: offset };
    // UE5.4+: optional padding before SizeX? (TexturePlatformData: has optional "bIsVirtual" u32 in 5.x)
    // We try multiple candidate layouts:
    for has_virtual_prefix in [false, true] {
        let mut rr = PlatformDataReader { data, pos: offset };
        if has_virtual_prefix {
            let _virtual_flag = rr.u32().ok()?; // VT data offset / bIsVirtual marker
        }
        let size_x = rr.i32().ok()?;
        let size_y = rr.i32().ok()?;
        let packed = rr.i32().ok()?;
        let first_mip = rr.i32().ok()?;
        let num_mips = rr.i32().ok()?;
        if !(1..=16384).contains(&size_x) || !(1..=16384).contains(&size_y) {
            continue;
        }
        if !(1..=32).contains(&num_mips) || first_mip < -1 || first_mip > 16 {
            continue;
        }
        let pixel_format_value = (packed >> 1) as i32;
        let format = pixel_format_name(pixel_format_value);

        let mut mips = Vec::new();
        let mut ok = true;
        for _ in 0..num_mips {
            match parse_mip(&mut rr, ubulk, combined, num_mips as usize) {
                Some(m) => mips.push(m),
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok || mips.is_empty() {
            continue;
        }
        // validate the mip chain sizes
        let (mut w, mut h) = (size_x as u32, size_y as u32);
        for mip in &mips {
            if mip.width != w || mip.height != h {
                // top mip must match; others may differ slightly — enforce chain halving
                ok = false;
                break;
            }
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
        if !ok {
            continue;
        }
        let consumed = rr.pos;
        let mips_out: Vec<TextureMip> = mips;
        return Some((size_x, size_y, format, mips_out, consumed));
    }
    None
}

fn parse_mip(
    r: &mut PlatformDataReader,
    ubulk: Option<&[u8]>,
    combined: Option<&[u8]>,
    _num_mips: usize,
) -> Option<TextureMip> {
    let size_x = r.i32().ok()?;
    let size_y = r.i32().ok()?;
    let _size_z = r.i32().ok()?;
    if !(1..=16384).contains(&size_x) || !(1..=16384).contains(&size_y) {
        return None;
    }
    let bulk_flags = r.u32().ok()?;
    let element_count = r.i64().ok()?;
    let size_on_disk = r.i64().ok()?;
    if size_on_disk < 0 || size_on_disk > (1 << 30) {
        return None;
    }
    let mut file_offset = 0i64;
    if bulk_flags & (BULKDATA_PayloadAtEndOfFile | BULKDATA_ForceInlinePayload) != 0
        && bulk_flags & BULKDATA_PayloadInSeperateFile == 0
    {
        file_offset = r.i64().ok()?;
    }
    if bulk_flags & 0x10000 != 0 {
        // BULKDATA_Force_ALIGN_16 payload marker (UE5): alignment chunk
        let _align_chunk = r.u32().ok()?;
    }

    let data: Vec<u8> = if bulk_flags & BULKDATA_ForceInlinePayload != 0 {
        let bytes = r.read_bytes(size_on_disk as usize).ok()?;
        decompress_bulk_payload(bytes, bulk_flags, size_on_disk as usize).ok()?
    } else if bulk_flags & BULKDATA_PayloadInSeperateFile != 0 {
        let ubulk = ubulk?;
        let parsed = parse_ubulk(ubulk).ok()?;
        if parsed.len() < size_on_disk as usize {
            return None;
        }
        parsed[..size_on_disk as usize].to_vec()
    } else if bulk_flags & BULKDATA_PayloadAtEndOfFile != 0 {
        let combined = combined?;
        let off = file_offset as usize;
        if off + size_on_disk as usize > combined.len() {
            // offsets may be relative to .uexp; combined already covers both files
            return None;
        }
        let bytes = &combined[off..off + size_on_disk as usize];
        decompress_bulk_payload(bytes, bulk_flags, size_on_disk as usize).ok()?
    } else {
        return None;
    };

    let _ = element_count;
    Some(TextureMip {
        width: size_x as u32,
        height: size_y as u32,
        data,
    })
}

/// Extract texture from a classic UAssetPackage (finds Texture2D exports).
pub fn extract_from_package(
    pkg: &crate::uasset::UAssetPackage,
    export_index: usize,
) -> Result<Texture> {
    use crate::uasset::properties::{dump_export_properties, PropValue};
    let export = &pkg.exports[export_index];
    let name = export.object_name.clone();
    let data = pkg
        .export_bytes(export_index)
        .ok_or_else(|| Error::asset("export data out of range"))?;

    // combined stream for payload-at-end resolution
    let mut combined = pkg.asset_data.clone();
    combined.extend_from_slice(pkg.exp_data.as_deref().unwrap_or(&[]));

    // Determine format from properties (versioned or unversioned layout)
    let mut format_from_props: Option<String> = None;
    let is_unversioned = pkg.summary.package_flags & 0x2000_0000 != 0 || !pkg.summary.has_version_info(); // PKG_UnversionedProperties
    let props_end;
    if !is_unversioned {
        let props = dump_export_properties(pkg, export_index)?;
        for (k, v) in &props {
            if k == "Format" {
                if let PropValue::Enum(val) = v {
                    format_from_props = Some(val.clone());
                }
            }
        }
        // position: after reading tagged props we need the reader's offset; recompute
        let reader = crate::uasset::properties::PropertyReader::new(pkg);
        let mut r = std::io::Cursor::new(data);
        let _ = reader.read_tagged(&mut r)?;
        props_end = r.position() as usize;
    } else {
        // unversioned: we don't know the class layout for Texture2D reliably;
        // scan for the platform data with validation
        props_end = 0;
    }

    // Try parse platform data after properties (versioned), else scan
    let parsed = if !is_unversioned {
        try_parse_platform_data(data, props_end, pkg.bulk(), Some(&combined))
            .ok_or_else(|| Error::asset("texture platform data parse failed"))?
    } else {
        // heuristic scan: try offsets stepping by 4 within first 4 KiB
        let mut found = None;
        for off in (0..data.len().min(4096)).step_by(4) {
            if let Some(p) = try_parse_platform_data(data, off, pkg.bulk(), Some(&combined)) {
                found = Some(p);
                break;
            }
        }
        found.ok_or_else(|| Error::asset("texture platform data not located (unversioned)") )?
    };
    let (w, h, fmt, mips, _) = parsed;
    let format_name = format_from_props.unwrap_or(fmt);
    // decode mips eagerly kept raw; decode on demand
    let _ = is_unversioned;
    Ok(Texture {
        name,
        width: w as u32,
        height: h as u32,
        format_name,
        mips,
    })
}

/// Extract texture from a zen (IOStore) package export.
pub fn extract_from_zen(zen: &crate::iostore::zen::ZenPackage, export_index: usize) -> Result<Texture> {
    let export = &zen.exports[export_index];
    let data = zen
        .export_data(export_index)
        .ok_or_else(|| Error::asset("export data out of range"))?;
    let name = export.object_name.clone();

    // heuristic scan for platform data (unversioned zen packages)
    let mut found = None;
    for off in (0..data.len().min(4096)).step_by(4) {
        if let Some(p) = try_parse_platform_data(data, off, None, None) {
            found = Some(p);
            break;
        }
    }
    let (w, h, fmt, mips, _) = found.ok_or_else(|| Error::asset("texture platform data not located in zen export"))?;
    Ok(Texture {
        name,
        width: w as u32,
        height: h as u32,
        format_name: fmt,
        mips,
    })
}

impl Texture {
    /// Decode the largest mip to RGBA.
    pub fn top_rgba(&self) -> Result<Vec<u8>> {
        let mip = self.mips.first().ok_or_else(|| Error::asset("no mips"))?;
        decode_mip(mip.width, mip.height, &self.format_name, &mip.data)
    }
}
