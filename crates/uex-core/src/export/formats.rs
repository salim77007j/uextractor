//! OBJ exporter (geometry fallback) + PNG/TGA/DDS texture exporters + WAV path.

use crate::assets::mesh::ExtractedMesh;
use crate::error::Result;

pub fn write_obj(mesh: &ExtractedMesh) -> String {
    let mut s = String::with_capacity(mesh.vertices.len() * 40 + mesh.sections.len() * 64);
    s.push_str(&format!("# UExtractor OBJ export: {}\n", mesh.name));
    for v in &mesh.vertices {
        s.push_str(&format!("v {:.6} {:.6} {:.6}\n", v.position[0], v.position[1], v.position[2]));
    }
    for v in &mesh.vertices {
        let uv = v.uv.first().copied().unwrap_or([0.0, 0.0]);
        s.push_str(&format!("vt {:.6} {:.6}\n", uv[0], 1.0 - uv[1]));
    }
    for v in &mesh.vertices {
        let n = v.normal;
        s.push_str(&format!("vn {:.4} {:.4} {:.4}\n", n[0], n[1], n[2]));
    }
    for (si, section) in mesh.sections.iter().enumerate() {
        s.push_str(&format!("o section_{si}_mat{}\n", section.material_index));
        let idx = &section.indices;
        for tri in idx.chunks_exact(3) {
            let (a, b, c) = (tri[0] + 1, tri[1] + 1, tri[2] + 1);
            s.push_str(&format!("f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}\n"));
        }
    }
    s
}

/// Save decoded RGBA as PNG or TGA.
pub fn write_image_png(path: &std::path::Path, rgba: &[u8], w: u32, h: u32) -> Result<()> {
    let img: image::RgbaImage = image::ImageBuffer::from_raw(w, h, rgba.to_vec())
        .ok_or_else(|| crate::error::Error::asset("image buffer size mismatch"))?;
    img.save_with_format(path, image::ImageFormat::Png)?;
    Ok(())
}

pub fn write_image_tga(path: &std::path::Path, rgba: &[u8], w: u32, h: u32) -> Result<()> {
    let img: image::RgbaImage = image::ImageBuffer::from_raw(w, h, rgba.to_vec())
        .ok_or_else(|| crate::error::Error::asset("image buffer size mismatch"))?;
    img.save_with_format(path, image::ImageFormat::Tga)?;
    Ok(())
}

/// DDS writer preserving original compressed data (DXT1/3/5, ATI1/2, BC7 via DX10, Float).
pub fn write_dds(width: u32, height: u32, format: &str, data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(128 + data.len());
    out.extend_from_slice(&0x20534444u32.to_le_bytes()); // "DDS "
    out.extend_from_slice(&124u32.to_le_bytes()); // header size @4
    // flags @8: CAPS | HEIGHT | WIDTH | PIXELFORMAT (+ LINEARSIZE for compressed)
    let (bpp, block): (u32, u32) = crate::assets::texture::format_layout(format);
    let mut flags: u32 = 0x00001007; // CAPS|HEIGHT|WIDTH|PIXELFORMAT
    let fourcc: [u8; 4] = match format {
        "PF_DXT1" => *b"DXT1",
        "PF_DXT3" => *b"DXT3",
        "PF_DXT5" => *b"DXT5",
        "PF_BC4" => *b"ATI1",
        "PF_BC5" => *b"ATI2",
        "PF_BC7" | "PF_ASTC_4x4" | "PF_ASTC_6x6" | "PF_ETC2_RGBA" | "PF_ETC2_RGB" | "PF_ETC1"
        | "PF_FloatRGBA" => *b"DX10",
        _ => [0, 0, 0, 0],
    };
    if fourcc != [0, 0, 0, 0] {
        flags |= 0x00080000; // LINEARSIZE
    }
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes()); // @12
    out.extend_from_slice(&width.to_le_bytes()); // @16
    let linear = if block == 1 {
        width * height * bpp
    } else {
        let (bx, by) = crate::assets::texture::blocks_of(width, height, block);
        bx * by * bpp
    };
    out.extend_from_slice(&linear.to_le_bytes()); // pitch/linear size @20
    out.extend_from_slice(&0u32.to_le_bytes()); // depth @24
    out.extend_from_slice(&1u32.to_le_bytes()); // mip count @28
    for _ in 0..11 {
        out.extend_from_slice(&0u32.to_le_bytes()); // reserved @32..76
    }
    // pixel format @76 (32 bytes)
    out.extend_from_slice(&32u32.to_le_bytes()); // size
    if fourcc != [0, 0, 0, 0] {
        out.extend_from_slice(&4u32.to_le_bytes()); // DDPF_FOURCC
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&fourcc);
    } else {
        match format {
            "PF_G8" | "PF_A8" => {
                out.extend_from_slice(&0x00020000u32.to_le_bytes()); // LUMINANCE
                out.extend_from_slice(&8u32.to_le_bytes()); // bit count
                out.extend_from_slice(&0xFFu32.to_le_bytes()); // rmask
                out.extend_from_slice(&0u32.to_le_bytes());
                out.extend_from_slice(&0u32.to_le_bytes());
                out.extend_from_slice(&0u32.to_le_bytes()); // amask
            }
            _ => {
                out.extend_from_slice(&0x41u32.to_le_bytes()); // RGB | ALPHAPIXELS
                out.extend_from_slice(&32u32.to_le_bytes());
                out.extend_from_slice(&0x00FF0000u32.to_le_bytes()); // r
                out.extend_from_slice(&0x0000FF00u32.to_le_bytes()); // g
                out.extend_from_slice(&0x000000FFu32.to_le_bytes()); // b
                out.extend_from_slice(&0xFF000000u32.to_le_bytes()); // a
            }
        }
    }
    // caps @108
    out.extend_from_slice(&0x1000u32.to_le_bytes()); // DDSCAPS_TEXTURE
    out.extend_from_slice(&0u32.to_le_bytes()); // caps2
    out.extend_from_slice(&0u32.to_le_bytes()); // caps3
    out.extend_from_slice(&0u32.to_le_bytes()); // caps4
    out.extend_from_slice(&0u32.to_le_bytes()); // reserved2 @124

    // DX10 header for BC7/ASTC/ETC/Float
    if fourcc == *b"DX10" {
        let dxgi: u32 = match format {
            "PF_BC7" => 98,        // DXGI_FORMAT_BC7_UNORM
            "PF_ASTC_4x4" => 158,  // DXGI_FORMAT_ASTC_4X4_UNORM
            "PF_ASTC_6x6" => 159,
            "PF_FloatRGBA" => 10,  // DXGI_FORMAT_R16G16B16A16_FLOAT
            _ => 0,
        };
        out.extend_from_slice(&dxgi.to_le_bytes());
        out.extend_from_slice(&4u32.to_le_bytes()); // resource dimension: TEXTURE2D
        out.extend_from_slice(&0u32.to_le_bytes()); // misc flag
        out.extend_from_slice(&1u32.to_le_bytes()); // array size
        out.extend_from_slice(&1u32.to_le_bytes()); // misc2 (alpha mode)
    }

    out.extend_from_slice(data);
    Ok(out)
}
