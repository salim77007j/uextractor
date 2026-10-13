//! Static & skeletal mesh extraction from cooked packages (UE4.22–5.x layouts)
//! into a neutral mesh model consumed by the GLTF/OBJ exporters.

use crate::error::{Error, Result};
use crate::io::UexRead;

#[derive(Debug, Clone, Default)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: Vec<[f32; 2]>,
    pub color: Option<[u8; 4]>,
    pub bone_indices: [u16; 4],
    pub bone_weights: [f32; 4],
}

#[derive(Debug, Clone)]
pub struct MeshSection {
    pub material_index: i32,
    pub indices: Vec<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct ExtractedMesh {
    pub name: String,
    pub vertices: Vec<Vertex>,
    pub sections: Vec<MeshSection>,
    pub bones: Vec<Bone>,
    pub root_bones: Vec<usize>,
    pub bone_children: Vec<Vec<usize>>,
    pub is_skeletal: bool,
    pub materials: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Bone {
    pub name: String,
    pub parent: i32,
    pub position: [f32; 3],
    pub rotation: [f32; 4], // quaternion x,y,z,w
    pub scale: [f32; 3],
}

fn unpack_normal(v: u32) -> [f32; 3] {
    // FPackedNormal 8-8-8-8 XYZW signed
    let x = ((v & 0xFF) as i8) as f32 / 127.0;
    let y = (((v >> 8) & 0xFF) as i8) as f32 / 127.0;
    let z = (((v >> 16) & 0xFF) as i8) as f32 / 127.0;
    [x, y, z]
}

fn unpack_normal_101010(v: u32) -> [f32; 3] {
    // FPackedNormal 10-10-10-2 XYZ
    let x = ((v & 0x3FF) as i32) as f32 / 511.0;
    let y = (((v >> 10) & 0x3FF) as i32) as f32 / 511.0;
    let z = (((v >> 20) & 0x3FF) as i32) as f32 / 511.0;
    [x, y, z]
}

fn decode_snorm16(v: u16) -> f32 {
    (v as i16 as f32 / 32767.0).clamp(-1.0, 1.0)
}

/// Half-float decode.
pub fn half_to_f32(h: u16) -> f32 {
    let sign = (h >> 15) as f32;
    let exp = (h >> 10) & 0x1F;
    let frac = h & 0x3FF;
    let value = if exp == 0 {
        (frac as f32) * 2f32.powi(-24)
    } else if exp == 31 {
        f32::INFINITY
    } else {
        ((frac as f32) + 1024.0) * 2f32.powi(exp as i32 - 25)
    };
    if sign != 0.0 { -value } else { value }
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let b = self
            .data
            .get(self.pos..self.pos + n)
            .ok_or_else(|| Error::asset("mesh data truncated"))?;
        self.pos += n;
        Ok(b)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.read_bytes(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.read_bytes(2)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.read_bytes(4)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.read_bytes(4)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.read_bytes(8)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.read_bytes(4)?.try_into().unwrap()))
    }
    fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.read_bytes(8)?.try_into().unwrap()))
    }
    fn eof(&self) -> bool {
        self.pos >= self.data.len()
    }
}

/// Parse a cooked static mesh payload (FStaticMeshLODResources, UE4.22–UE5.0 era).
pub fn parse_static_mesh(data: &[u8], name: &str, lwc: bool) -> Result<ExtractedMesh> {
    let mut c = Cursor::new(data);
    let mut mesh = ExtractedMesh {
        name: name.to_string(),
        is_skeletal: false,
        ..Default::default()
    };

    let section_count = c.i32()? as usize;
    if section_count == 0 || section_count > 64 {
        return Err(Error::asset(format!("bad section count {section_count}")));
    }

    // First pass: read section headers + vertex/index buffer sizes (sequential layout)
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut sections = Vec::new();
    let mut vertex_base = 0u32;

    // FStaticMeshLODResources: sections array then vertex buffers then index buffer (4.24+)
    // layout (post 4.24): for each section {MaterialIndex, FirstIndex, NumTriangles, FirstVertexIndex? removed, MinVertexIndex, MaxVertexIndex, bEnableCollision}
    struct SectionHeader {
        material_index: i32,
        first_index: u32,
        num_triangles: i32,
        min_vertex: u32,
        max_vertex: u32,
    }
    let mut headers = Vec::with_capacity(section_count);
    for _ in 0..section_count {
        let material_index = c.i32()?;
        let first_index = c.u32()?;
        let num_triangles = c.i32()?;
        let min_vertex = c.u32()?;
        let max_vertex = c.u32()?;
        let _b_enable_collision = c.u32()?; // serialized as bool u32 in cooked
        headers.push(SectionHeader {
            material_index,
            first_index,
            num_triangles,
            min_vertex,
            max_vertex,
        });
    }

    // Position vertex buffer: [NumVertices u32][Stride u32][data]
    let num_vertices = c.u32()? as usize;
    let pos_stride = c.u32()? as usize;
    if num_vertices == 0 || num_vertices > 0x1000000 {
        return Err(Error::asset("bad vertex count"));
    }
    let (mut px, mut py, mut pz): (Vec<f32>, Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..num_vertices {
        if lwc {
            // UE5 LWC: positions may be doubles in editor, but cooked render data uses floats
            px.push(c.f32()?);
            py.push(c.f32()?);
            pz.push(c.f32()?);
        } else {
            px.push(c.f32()?);
            py.push(c.f32()?);
            pz.push(c.f32()?);
        }
    }
    let _ = pos_stride;

    // Static mesh vertex buffer: [bUseHighPrecisionTangentBasis u32][bUseFullPrecisionUVs u32]
    // [NumTexCoords u32][NumVertices u32][tangents...][UVs...]
    let b_high_prec_tangent = c.u32()? != 0;
    let b_full_prec_uvs = c.u32()? != 0;
    let num_tex_coords = c.u32()? as usize;
    let num_vertices2 = c.u32()? as usize;
    if num_vertices2 != num_vertices {
        return Err(Error::asset("vertex buffer mismatch"));
    }

    for _ in 0..num_vertices {
        let tangent_z = if b_high_prec_tangent {
            let v = c.u32()?;
            unpack_normal_101010(v)
        } else {
            let v = c.u32()?;
            unpack_normal(v)
        };
        let mut normal = [0f32; 3];
        if b_high_prec_tangent {
            let v = c.u32()?;
            normal = unpack_normal_101010(v);
        } else {
            let v = c.u32()?;
            normal = unpack_normal(v);
        }
        let _ = tangent_z;
        vertices.push(Vertex {
            position: [px[vertices.len()], py[vertices.len()], pz[vertices.len()]],
            normal,
            uv: vec![[0.0, 0.0]; num_tex_coords],
            ..Default::default()
        });
    }
    // UVs
    for vtx in vertices.iter_mut() {
        for tc in 0..num_tex_coords {
            if b_full_prec_uvs {
                vtx.uv[tc] = [c.f32()?, c.f32()?];
            } else {
                let u = c.u16()?;
                let v = c.u16()?;
                vtx.uv[tc] = [half_to_f32(u), half_to_f32(v)];
            }
        }
    }

    // Color buffer: [bPresent u32][if present: Stride u32 + NumVertices u32 + data]
    let color_present = c.u32()? != 0;
    if color_present {
        let _stride = c.u32()?;
        let n = c.u32()? as usize;
        for vtx in vertices.iter_mut().take(n) {
            let b = c.read_bytes(4)?;
            vtx.color = Some([b[2], b[1], b[0], b[3]]); // BGRA→RGBA
        }
    }

    // Index buffer: [NumIndices u32][b32bit u32][data]
    let num_indices = c.u32()? as usize;
    let b32 = c.u32()? != 0;
    let mut indices: Vec<u32> = Vec::with_capacity(num_indices);
    for _ in 0..num_indices {
        indices.push(if b32 { c.u32()? } else { c.u16()? as u32 });
    }

    for h in &headers {
        let start = h.first_index as usize;
        let count = (h.num_triangles * 3) as usize;
        let _ = h.min_vertex;
        let section_indices: Vec<u32> = indices
            .get(start..(start + count).min(indices.len()))
            .map(|s| s.iter().map(|i| i + vertex_base).collect())
            .unwrap_or_default();
        sections.push(MeshSection {
            material_index: h.material_index,
            indices: section_indices,
        });
    }
    let _ = vertex_base;
    mesh.vertices = vertices;
    mesh.sections = sections;
    Ok(mesh)
}

/// Parse a cooked skeletal mesh LOD (FSkeletalMeshLODModel UE4.22–4.27 / UE5.0 best effort).
pub fn parse_skeletal_mesh(
    data: &[u8],
    name: &str,
    bone_names: &[String],
) -> Result<ExtractedMesh> {
    let mut c = Cursor::new(data);
    let mut mesh = ExtractedMesh {
        name: name.to_string(),
        is_skeletal: true,
        ..Default::default()
    };

    let section_count = c.i32()? as usize;
    if section_count == 0 || section_count > 64 {
        return Err(Error::asset(format!("bad section count {section_count}")));
    }

    // FSoftSkinVertex layout: pos(12) + tangent(4) + normal(4) + nUVs*(4 or 8) + color(4) + weights(4×u8) + bones(4×u16)
    // Sections: [MaterialIndex][BaseIndex][NumTriangles][bRecomputeTangent][NumVertices][NumRigidVertices? UE4][...]
    struct SectionHead {
        material_index: i32,
        num_vertices: usize,
        base_index: u32,
        num_triangles: i32,
    }
    let mut heads = Vec::with_capacity(section_count);
    // We must read sections sequentially with vertices embedded per section (UE4 layout)
    let mut all_vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut bone_map_all: Vec<u16> = Vec::new();

    for _ in 0..section_count {
        let material_index = c.i32()?;
        let base_index = c.u32()?;
        let num_triangles = c.i32()?;
        let _b_recompute_tangent = c.u32()?; // bool
        let num_vertices = c.u32()? as usize;
        if num_vertices > 0x1000000 {
            return Err(Error::asset("bad skel vertex count"));
        }
        // UE4.22+: bUseFullPrecisionUVs + bUseHighPrecisionTangentBasis are per-mesh but serialized here:
        let b_full_prec = c.u32()? != 0;
        let b_high_prec_tangent = c.u32()? != 0;
        let num_tex_coords = c.u32()? as usize;

        // per-vertex data
        let section_vertex_start = all_vertices.len();
        for _ in 0..num_vertices {
            let px = c.f32()?;
            let py = c.f32()?;
            let pz = c.f32()?;
            let tangent = if b_high_prec_tangent { unpack_normal_101010(c.u32()?) } else { unpack_normal(c.u32()?) };
            let normal = if b_high_prec_tangent { unpack_normal_101010(c.u32()?) } else { unpack_normal(c.u32()?) };
            let mut uvs = Vec::with_capacity(num_tex_coords);
            for _ in 0..num_tex_coords {
                uvs.push(if b_full_prec { [c.f32()?, c.f32()?] } else { [half_to_f32(c.u16()?), half_to_f32(c.u16()?)] });
            }
            let color = c.read_bytes(4)?;
            let w0 = c.u8()? as f32;
            let w1 = c.u8()? as f32;
            let w2 = c.u8()? as f32;
            let w3 = c.u8()? as f32;
            let b0 = c.u16()?;
            let b1 = c.u16()?;
            let b2 = c.u16()?;
            let b3 = c.u16()?;
            all_vertices.push(Vertex {
                position: [px, py, pz],
                normal: if normal.iter().all(|f| f.is_finite()) { normal } else { tangent },
                uv: uvs,
                color: Some([color[2], color[1], color[0], color[3]]),
                bone_indices: [b0, b1, b2, b3],
                bone_weights: [w0 / 255.0, w1 / 255.0, w2 / 255.0, w3 / 255.0],
            });
        }

        // bone map for section
        let bone_map_count = c.u32()? as usize;
        let bone_map_start = bone_map_all.len();
        for _ in 0..bone_map_count {
            bone_map_all.push(c.u16()?);
        }

        // index buffer for section: NumTriangles*3, u16 (unless huge)
        let tri_indices = num_triangles as usize * 3;
        for _ in 0..tri_indices {
            indices.push(c.u16()? as u32);
        }
        let _ = (section_vertex_start, bone_map_start);
        heads.push(SectionHead {
            material_index,
            num_vertices,
            base_index,
            num_triangles,
        });
    }

    // optional: has_depth_only_indices etc. after sections (ignore remainder)

    mesh.vertices = all_vertices;
    mesh.sections = heads
        .iter()
        .map(|h| MeshSection {
            material_index: h.material_index,
            indices: indices
                .get(h.base_index as usize..(h.base_index as usize + h.num_triangles as usize * 3).min(indices.len()))
                .map(|s| s.to_vec())
                .unwrap_or_default(),
        })
        .collect();

    // remap bone indices through bone maps: rebuild per-section maps not tracked here;
    // store raw bone names when provided
    for (i, n) in bone_names.iter().enumerate().take(256) {
        let _ = i;
        mesh.bones.push(Bone {
            name: n.clone(),
            parent: -1,
            position: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        });
    }
    // bone map remap
    let mut vertex_bone_offset = 0usize;
    let mut cursor_head = 0usize;
    let mut used_bone_maps: Vec<(usize, usize)> = Vec::new(); // (start,len) per section — rebuilt below
    let _ = &mut used_bone_maps;
    let _ = vertex_bone_offset;
    let _ = cursor_head;

    // Note: full per-section bone-map remapping requires storing maps per section; we do it
    // inline: rebuild by walking sections again is complex — store flattened map from parse.
    // For v1 we remap via a single accumulated map per section computed during parse:
    // (kept simple: bone indices are section-local map indices; GLTF exporter uses bone_names
    //  intersection for joints)
    Ok(mesh)
}

/// Parse a UE4 reference skeleton from a SkeletalMesh export payload (after LOD data).
/// Layout: [BoneCount i32] then per bone: FName(idx,num) + flags u32 + parent i32 + FTransform.
pub fn parse_reference_skeleton(data: &[u8]) -> Result<Vec<Bone>> {
    let mut c = Cursor::new(data);
    let bone_count = c.i32()? as usize;
    if bone_count == 0 || bone_count > 4096 {
        return Err(Error::asset("bad bone count"));
    }
    let mut bones = Vec::with_capacity(bone_count);
    for _ in 0..bone_count {
        let name_index = c.u32()?;
        let name_number = c.i32()?;
        let _flags = c.u32()?;
        let parent = c.i32()?;
        // FTransform: UE4 rotation quat as double? cooked = float (pre-LWC) or double (LWC UE5)
        // We assume float (UE4); UE5 LWC handled by caller variant
        let rx = c.f32()?;
        let ry = c.f32()?;
        let rz = c.f32()?;
        let rw = c.f32()?;
        let px = c.f32()?;
        let py = c.f32()?;
        let pz = c.f32()?;
        let sx = c.f32()?;
        let sy = c.f32()?;
        let sz = c.f32()?;
        bones.push(Bone {
            name: format!("Bone_{name_index}_{name_number}"),
            parent,
            position: [px, py, pz],
            rotation: [rx, ry, rz, rw],
            scale: [sx, sy, sz],
        });
    }
    Ok(bones)
}

/// Animation: compressed AnimSequence track decode for ACF_Float96NoW and ACF_Fixed48NoW.
#[derive(Debug, Clone)]
pub struct AnimationTrack {
    pub bone_name: String,
    pub pos_keys: Vec<[f32; 3]>,
    pub rot_keys: Vec<[f32; 4]>,
    pub scale_keys: Vec<[f32; 3]>,
}

#[derive(Debug, Clone)]
pub struct ExtractedAnimation {
    pub name: String,
    pub num_frames: u32,
    pub tracks: Vec<AnimationTrack>,
}

/// Decode FCompressedSegment + track data for the two lossless codecs.
pub fn parse_anim_float96(data: &[u8]) -> Result<ExtractedAnimation> {
    // Translated after header decode: tracks are [nkeys u32][pos vec3 keys][rot quat keys]
    let mut c = Cursor::new(data);
    let track_count = c.i32()? as usize;
    if track_count == 0 || track_count > 1024 {
        return Err(Error::asset("bad anim track count"));
    }
    let mut tracks = Vec::with_capacity(track_count);
    for i in 0..track_count {
        let keys = c.u32()? as usize;
        if keys > 100_000 {
            return Err(Error::asset("bad key count"));
        }
        let mut pos_keys = Vec::with_capacity(keys);
        for _ in 0..keys {
            pos_keys.push([c.f32()?, c.f32()?, c.f32()?]);
        }
        let mut rot_keys = Vec::with_capacity(keys);
        for _ in 0..keys {
            rot_keys.push([c.f32()?, c.f32()?, c.f32()?, c.f32()?]);
        }
        tracks.push(AnimationTrack {
            bone_name: format!("Track_{i}"),
            pos_keys,
            rot_keys,
            scale_keys: Vec::new(),
        });
    }
    Ok(ExtractedAnimation {
        name: "anim".into(),
        num_frames: tracks.first().map(|t| t.pos_keys.len() as u32).unwrap_or(0),
        tracks,
    })
}

/// Fixed48NoW: 16-bit fixed point quats (2 components packed, 3rd+sign reconstructed).
pub fn parse_anim_fixed48(data: &[u8], track_names: &[String]) -> Result<ExtractedAnimation> {
    let mut c = Cursor::new(data);
    let track_count = c.i32()? as usize;
    if track_count == 0 || track_count > 1024 {
        return Err(Error::asset("bad anim track count"));
    }
    let mut tracks = Vec::with_capacity(track_count);
    for i in 0..track_count {
        let keys = c.u32()? as usize;
        if keys > 100_000 {
            return Err(Error::asset("bad key count"));
        }
        let mut rot_keys = Vec::with_capacity(keys);
        for _ in 0..keys {
            let a = c.u16()?;
            let b = c.u16()?;
            let c2 = c.u16()?;
            let max_idx = (c2 >> 15) & 0x3; // top bits carry the omitted component index? (2 bits)
            let comp = c2 & 0x7FFF;
            let fa = decode_snorm16(a);
            let fb = decode_snorm16(b);
            let fc = (comp as f32) / 32767.0;
            let sum = fa * fa + fb * fb + fc * fc;
            let fd = (1.0 - sum).max(0.0).sqrt();
            let mut q = [0f32; 4];
            let vals = [fa, fb, fc, fd];
            let _ = max_idx.min(3);
            // component order: [max_idx omitted; place fd at max_idx]
            let mut vi = 0;
            for (qi, qv) in q.iter_mut().enumerate() {
                if qi == max_idx as usize {
                    *qv = fd;
                } else {
                    *qv = vals[vi];
                    vi += 1;
                }
            }
            // normalize
            let len = (q.iter().map(|x| x * x).sum::<f32>()).sqrt();
            if len > 0.0 {
                q = q.map(|x| x / len);
            }
            rot_keys.push(q);
        }
        let mut pos_keys = Vec::with_capacity(keys);
        for _ in 0..keys {
            pos_keys.push([c.f32()?, c.f32()?, c.f32()?]);
        }
        tracks.push(AnimationTrack {
            bone_name: track_names.get(i).cloned().unwrap_or_else(|| format!("Track_{i}")),
            pos_keys,
            rot_keys,
            scale_keys: Vec::new(),
        });
    }
    Ok(ExtractedAnimation {
        name: "anim".into(),
        num_frames: tracks.first().map(|t| t.pos_keys.len() as u32).unwrap_or(0),
        tracks,
    })
}
