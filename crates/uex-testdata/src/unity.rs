//! Synthetic Unity archive builders: real SerializedFiles (v20 / "2019.4.40f1"
//! layouts) and UnityFS bundles (none/LZ4 blocks). The embedded typetrees and
//! object field layouts mirror the AssetRipper TPK 2019.4 release trees
//! exactly, so the same bytes validate both the embedded-tree path and the
//! stripped-tree path (TPK lookup).

#![allow(dead_code)]

use std::io::Write;

// ---------------------------------------------------------------------------
// binary writer helpers (little endian, Unity alignment semantics)
// ---------------------------------------------------------------------------

pub struct W(pub Vec<u8>);

impl W {
    pub fn new() -> Self {
        W(Vec::new())
    }
    pub fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    pub fn bool(&mut self, v: bool) {
        self.0.push(v as u8);
    }
    pub fn align(&mut self, to: usize) {
        let m = self.0.len() % to;
        if m != 0 {
            self.0.extend(std::iter::repeat(0).take(to - m));
        }
    }
    pub fn i16(&mut self, v: i16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i64(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i16_be(&mut self, v: i16) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    pub fn u16_be(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    pub fn i32_be(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    pub fn u32_be(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    pub fn i64_be(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    pub fn u64_be(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    /// null-terminated string (no alignment)
    pub fn s(&mut self, v: &str) {
        self.0.extend_from_slice(v.as_bytes());
        self.0.push(0);
    }
    /// aligned string: i32 length + bytes + align4 (Unity "string" type)
    pub fn astring(&mut self, v: &str) {
        self.i32(v.len() as i32);
        self.0.extend_from_slice(v.as_bytes());
        self.align(4);
    }
    /// aligned byte blob: i32 length + bytes + align4 (TypelessData / vector<UInt8>)
    pub fn blob(&mut self, v: &[u8]) {
        self.i32(v.len() as i32);
        self.0.extend_from_slice(v);
        self.align(4);
    }
    /// vector count prefix
    pub fn count(&mut self, n: usize) {
        self.i32(n as i32);
    }
}

// ---------------------------------------------------------------------------
// typetree definitions (flattened, level-based) — 2019.4 release layouts
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct TNode {
    pub level: i32,
    pub type_name: &'static str,
    pub name: &'static str,
    pub meta: u32,
}

const ALIGN: u32 = 0x4000;

fn n(level: i32, type_name: &'static str, name: &'static str) -> TNode {
    TNode {
        level,
        type_name,
        name,
        meta: 0,
    }
}
fn na(level: i32, type_name: &'static str, name: &'static str) -> TNode {
    TNode {
        level,
        type_name,
        name,
        meta: ALIGN,
    }
}

/// aligned string field: 3 nodes (string + Array + size/char)
fn string_field(level: i32, name: &'static str) -> Vec<TNode> {
    vec![
        n(level, "string", name),
        n(level + 1, "Array", "Array"),
        n(level + 2, "int", "size"),
        n(level + 2, "char", "data"),
    ]
}

fn blob_field(level: i32, name: &'static str) -> Vec<TNode> {
    vec![
        na(level, "TypelessData", name),
        n(level + 1, "int", "size"),
        n(level + 1, "UInt8", "data"),
    ]
}

fn pptr_field(level: i32, type_name: &'static str, name: &'static str) -> Vec<TNode> {
    vec![
        na(level, type_name, name),
        n(level + 1, "int", "m_FileID"),
        n(level + 1, "SInt64", "m_PathID"),
    ]
}

/// vector of a struct: vector node + Array + size + struct children
fn vector_struct(level: i32, name: &'static str, item: &'static str, children: Vec<Vec<TNode>>) -> Vec<TNode> {
    let mut out = vec![n(level, "vector", name), na(level + 1, "Array", "Array"), n(level + 2, "int", "size")];
    out.push(TNode {
        level: level + 2,
        type_name: item,
        name: "data",
        meta: 0,
    });
    for c in children {
        for mut node in c {
            node.level += level + 3 - node.level.min(level + 3);
            // children were authored at absolute level 3 for the item; shift is fixed below
            out.push(node);
        }
    }
    out
}

fn texture2d_tree() -> Vec<TNode> {
    let mut v = vec![n(0, "Texture2D", "Base")];
    v.extend(string_field(1, "m_Name"));
    v.push(n(1, "int", "m_ForcedFallbackFormat"));
    v.push(na(1, "bool", "m_DownscaleFallback"));
    v.push(n(1, "int", "m_Width"));
    v.push(n(1, "int", "m_Height"));
    v.push(n(1, "int", "m_CompleteImageSize"));
    v.push(n(1, "int", "m_TextureFormat"));
    v.push(n(1, "int", "m_MipCount"));
    v.push(n(1, "bool", "m_IsReadable"));
    v.push(n(1, "bool", "m_IgnoreMasterTextureLimit"));
    v.push(n(1, "bool", "m_IsPreProcessed"));
    v.push(na(1, "bool", "m_StreamingMipmaps"));
    v.push(n(1, "int", "m_StreamingMipmapsPriority"));
    v.push(n(1, "int", "m_ImageCount"));
    v.push(n(1, "int", "m_TextureDimension"));
    // GLTextureSettings
    v.push(n(1, "GLTextureSettings", "m_TextureSettings"));
    v.push(n(2, "int", "m_FilterMode"));
    v.push(n(2, "int", "m_Aniso"));
    v.push(n(2, "float", "m_MipBias"));
    v.push(n(2, "int", "m_WrapU"));
    v.push(n(2, "int", "m_WrapV"));
    v.push(n(2, "int", "m_WrapW"));
    v.push(n(1, "int", "m_LightmapFormat"));
    v.push(n(1, "int", "m_ColorSpace"));
    v.extend(blob_field(1, "image data"));
    // StreamingInfo m_StreamData
    v.push(n(1, "StreamingInfo", "m_StreamData"));
    v.push(n(2, "unsigned int", "offset"));
    v.push(n(2, "unsigned int", "size"));
    v.extend(string_field(2, "path"));
    v
}

fn text_asset_tree() -> Vec<TNode> {
    let mut v = vec![n(0, "TextAsset", "Base")];
    v.extend(string_field(1, "m_Name"));
    v.extend(string_field(1, "m_Script"));
    v
}

fn audio_clip_tree() -> Vec<TNode> {
    let mut v = vec![n(0, "AudioClip", "Base")];
    v.extend(string_field(1, "m_Name"));
    v.push(n(1, "int", "m_LoadType"));
    v.push(n(1, "int", "m_Channels"));
    v.push(n(1, "int", "m_Frequency"));
    v.push(n(1, "int", "m_BitsPerSample"));
    v.push(n(1, "float", "m_Length"));
    v.push(n(1, "bool", "m_IsTrackerFormat"));
    v.push(na(1, "bool", "m_Ambisonic"));
    v.push(n(1, "int", "m_SubsoundIndex"));
    v.push(n(1, "bool", "m_PreloadAudioData"));
    v.push(n(1, "bool", "m_LoadInBackground"));
    v.push(na(1, "bool", "m_Legacy3D"));
    // StreamedResource
    v.push(n(1, "StreamedResource", "m_Resource"));
    v.extend(string_field(2, "m_Source"));
    v.push(n(2, "UInt64", "m_Offset"));
    v.push(n(2, "UInt64", "m_Size"));
    v.push(na(1, "int", "m_CompressionFormat"));
    v
}

fn submesh_nodes(level: i32) -> Vec<TNode> {
    let mut v = vec![
        n(level, "SubMesh", "data"),
        n(level + 1, "unsigned int", "firstByte"),
        n(level + 1, "unsigned int", "indexCount"),
        n(level + 1, "int", "topology"),
        n(level + 1, "unsigned int", "baseVertex"),
        n(level + 1, "unsigned int", "firstVertex"),
        n(level + 1, "unsigned int", "vertexCount"),
        n(level + 1, "AABB", "localAABB"),
        n(level + 2, "Vector3f", "m_Center"),
        n(level + 3, "float", "x"),
        n(level + 3, "float", "y"),
        n(level + 3, "float", "z"),
        n(level + 2, "Vector3f", "m_Extent"),
        n(level + 3, "float", "x"),
        n(level + 3, "float", "y"),
        n(level + 3, "float", "z"),
    ];
    for t in &mut v {
        if t.level > level {
            t.meta = 1; // meta 0x1-ish is irrelevant for reading; keep 0
        }
        t.meta = 0;
    }
    v
}

fn vector_of(level: i32, name: &'static str, item_nodes: Vec<TNode>) -> Vec<TNode> {
    let mut out = vec![n(level, "vector", name), na(level + 1, "Array", "Array"), n(level + 2, "int", "size")];
    let base = item_nodes.iter().map(|t| t.level).min().unwrap_or(0);
    for t in item_nodes {
        out.push(TNode {
            level: t.level - base + level + 2,
            type_name: t.type_name,
            name: t.name,
            meta: t.meta,
        });
    }
    out
}

fn packed_bit_vector(level: i32, with_range: bool) -> Vec<TNode> {
    let mut v = vec![
        n(level, "PackedBitVector", "PBV"),
        n(level + 1, "unsigned int", "m_NumItems"),
    ];
    if with_range {
        v.push(n(level + 1, "float", "m_Range"));
        v.push(n(level + 1, "float", "m_Start"));
    }
    v.extend(vector_of(level + 1, "m_Data", vec![n(level + 2, "UInt8", "data")]));
    v.push(na(level + 1, "UInt8", "m_BitSize"));
    // rename PBV node to the field name later
    v
}

fn mesh_tree() -> Vec<TNode> {
    let mut v = vec![n(0, "Mesh", "Base")];
    v.extend(string_field(1, "m_Name"));
    v.extend(vector_of(1, "m_SubMeshes", submesh_nodes(3)));
    // m_Shapes
    v.push(n(1, "BlendShapeData", "m_Shapes"));
    v.extend(vector_of(
        2,
        "vertices",
        vec![
            n(4, "BlendShapeVertex", "data"),
            n(5, "Vector3f", "vertex"),
            n(6, "float", "x"),
            n(6, "float", "y"),
            n(6, "float", "z"),
            n(5, "Vector3f", "normal"),
            n(6, "float", "x"),
            n(6, "float", "y"),
            n(6, "float", "z"),
            n(5, "Vector3f", "tangent"),
            n(6, "float", "x"),
            n(6, "float", "y"),
            n(6, "float", "z"),
            n(5, "unsigned int", "index"),
        ],
    ));
    v.extend(vector_of(
        2,
        "shapes",
        vec![
            n(4, "MeshBlendShape", "data"),
            n(5, "unsigned int", "firstVertex"),
            n(5, "unsigned int", "vertexCount"),
            n(5, "bool", "hasNormals"),
            na(5, "bool", "hasTangents"),
        ],
    ));
    v.extend(vector_of(
        2,
        "channels",
        vec![
            n(4, "MeshBlendShapeChannel", "data"),
            n(5, "string", "name"),
            n(6, "Array", "Array"),
            n(7, "int", "size"),
            n(7, "char", "data"),
            n(5, "unsigned int", "nameHash"),
            n(5, "int", "frameIndex"),
            n(5, "int", "frameCount"),
        ],
    ));
    v.extend(vector_of(2, "fullWeights", vec![n(4, "float", "data")]));
    // bindpose: vector<Matrix4x4f>
    let mut matrix_children: Vec<TNode> = (0..16)
        .map(|i| {
            let names = [
                "e00", "e01", "e02", "e03", "e10", "e11", "e12", "e13", "e20", "e21", "e22", "e23", "e30", "e31",
                "e32", "e33",
            ];
            n(1, "float", names[i])
        })
        .collect();
    for t in &mut matrix_children {
        t.level += 1; // under the Matrix4x4f data node
    }
    let mut matrix_item = vec![n(0, "Matrix4x4f", "data")];
    matrix_item.extend(matrix_children);
    v.extend(vector_of(1, "m_BindPose", matrix_item));
    v.extend(vector_of(1, "m_BoneNameHashes", vec![n(4, "unsigned int", "data")]));
    v.push(n(1, "unsigned int", "m_RootBoneNameHash"));
    v.extend(vector_of(
        1,
        "m_BonesAABB",
        vec![
            n(4, "MinMaxAABB", "data"),
            n(5, "Vector3f", "m_Min"),
            n(6, "float", "x"),
            n(6, "float", "y"),
            n(6, "float", "z"),
            n(5, "Vector3f", "m_Max"),
            n(6, "float", "x"),
            n(6, "float", "y"),
            n(6, "float", "z"),
        ],
    ));
    // VariableBoneCountWeights
    v.push(n(1, "VariableBoneCountWeights", "m_VariableBoneCountWeights"));
    v.extend(vector_of(2, "m_Data", vec![n(4, "unsigned int", "data")]));
    v.push(n(1, "UInt8", "m_MeshCompression"));
    v.push(n(1, "bool", "m_IsReadable"));
    v.push(n(1, "bool", "m_KeepVertices"));
    v.push(na(1, "bool", "m_KeepIndices"));
    v.push(n(1, "int", "m_IndexFormat"));
    v.extend(vector_of(1, "m_IndexBuffer", vec![n(4, "UInt8", "data")]));
    // VertexData
    v.push(n(1, "VertexData", "m_VertexData"));
    v.push(n(2, "unsigned int", "m_VertexCount"));
    v.extend(vector_of(
        2,
        "m_Channels",
        vec![
            n(4, "ChannelInfo", "data"),
            n(5, "UInt8", "stream"),
            n(5, "UInt8", "offset"),
            n(5, "UInt8", "format"),
            n(5, "UInt8", "dimension"),
        ],
    ));
    v.extend(blob_field(2, "m_DataSize"));
    // CompressedMesh (7 PBVs + UVInfo)
    v.push(n(1, "CompressedMesh", "m_CompressedMesh"));
    let mut pbv_fields = [
        ("m_Vertices", true),
        ("m_UV", true),
        ("m_Normals", true),
        ("m_Tangents", true),
        ("m_Weights", false),
        ("m_NormalSigns", false),
        ("m_TangentSigns", false),
        ("m_FloatColors", true),
        ("m_BoneIndices", false),
        ("m_Triangles", false),
    ];
    for (name, with_range) in pbv_fields.iter_mut() {
        let mut block = packed_bit_vector(2, *with_range);
        block[0].name = match *name {
            "m_Vertices" => "m_Vertices",
            "m_UV" => "m_UV",
            "m_Normals" => "m_Normals",
            "m_Tangents" => "m_Tangents",
            "m_Weights" => "m_Weights",
            "m_NormalSigns" => "m_NormalSigns",
            "m_TangentSigns" => "m_TangentSigns",
            "m_FloatColors" => "m_FloatColors",
            "m_BoneIndices" => "m_BoneIndices",
            _ => "m_Triangles",
        };
        v.extend(block);
    }
    v.push(n(2, "unsigned int", "m_UVInfo"));
    // local AABB
    v.push(n(1, "AABB", "m_LocalAABB"));
    v.push(n(2, "Vector3f", "m_Center"));
    v.push(n(3, "float", "x"));
    v.push(n(3, "float", "y"));
    v.push(n(3, "float", "z"));
    v.push(n(2, "Vector3f", "m_Extent"));
    v.push(n(3, "float", "x"));
    v.push(n(3, "float", "y"));
    v.push(n(3, "float", "z"));
    v.push(n(1, "int", "m_MeshUsageFlags"));
    v.extend(vector_of(1, "m_BakedConvexCollisionMesh", vec![n(4, "UInt8", "data")]));
    v.extend(vector_of(1, "m_BakedTriangleCollisionMesh", vec![n(4, "UInt8", "data")]));
    v.push(n(1, "float", "m_MeshMetrics[0]"));
    v.push(na(1, "float", "m_MeshMetrics[1]"));
    // m_StreamData
    v.push(n(1, "StreamingInfo", "m_StreamData"));
    v.push(n(2, "unsigned int", "offset"));
    v.push(n(2, "unsigned int", "size"));
    v.extend(string_field(2, "path"));
    v
}

fn sprite_tree() -> Vec<TNode> {
    let mut v = vec![n(0, "Sprite", "Base")];
    v.extend(string_field(1, "m_Name"));
    // Rectf
    v.push(n(1, "Rectf", "m_Rect"));
    v.push(n(2, "float", "x"));
    v.push(n(2, "float", "y"));
    v.push(n(2, "float", "width"));
    v.push(n(2, "float", "height"));
    // Vector2f
    v.push(n(1, "Vector2f", "m_Offset"));
    v.push(n(2, "float", "x"));
    v.push(n(2, "float", "y"));
    // Vector4f
    v.push(n(1, "Vector4f", "m_Border"));
    for f in ["x", "y", "z", "w"] {
        v.push(n(2, "float", static_float_name(f)));
    }
    v.push(n(1, "float", "m_PixelsToUnits"));
    v.push(n(1, "Vector2f", "m_Pivot"));
    v.push(n(2, "float", "x"));
    v.push(n(2, "float", "y"));
    v.push(n(1, "unsigned int", "m_Extrude"));
    v.push(na(1, "bool", "m_IsPolygon"));
    // pair m_RenderDataKey
    v.push(n(1, "pair", "m_RenderDataKey"));
    v.push(n(2, "GUID", "first"));
    for i in 0..4 {
        v.push(n(3, "unsigned int", static_data_index(i)));
    }
    v.push(n(2, "SInt64", "second"));
    v.extend(vector_of(1, "m_AtlasTags", vec![n(3, "string", "data"), n(4, "Array", "Array"), n(5, "int", "size"), n(5, "char", "data")]));
    v.extend(pptr_field(1, "PPtr<SpriteAtlas>", "m_SpriteAtlas"));
    // SpriteRenderData m_RD
    v.push(n(1, "SpriteRenderData", "m_RD"));
    v.extend(pptr_field(2, "PPtr<Texture2D>", "texture"));
    v.extend(pptr_field(2, "PPtr<Texture2D>", "alphaTexture"));
    v.extend(vector_of(
        2,
        "secondaryTextures",
        vec![
            n(4, "SecondarySpriteTexture", "data"),
            n(5, "PPtr<Texture2D>", "texture"),
            n(6, "int", "m_FileID"),
            n(6, "SInt64", "m_PathID"),
            n(5, "string", "name"),
            n(6, "Array", "Array"),
            n(7, "int", "size"),
            n(7, "char", "data"),
        ],
    ));
    v.extend(vector_of(2, "m_SubMeshes", submesh_nodes(4)));
    v.extend(vector_of(2, "m_IndexBuffer", vec![n(4, "UInt8", "data")]));
    // VertexData
    v.push(n(2, "VertexData", "m_VertexData"));
    v.push(n(3, "unsigned int", "m_VertexCount"));
    v.extend(vector_of(
        3,
        "m_Channels",
        vec![
            n(5, "ChannelInfo", "data"),
            n(6, "UInt8", "stream"),
            n(6, "UInt8", "offset"),
            n(6, "UInt8", "format"),
            n(6, "UInt8", "dimension"),
        ],
    ));
    v.extend(blob_field(3, "m_DataSize"));
    // m_Bindpose (empty): vector<Matrix4x4f>
    let mut matrix_children: Vec<TNode> = (0..16)
        .map(|i| n(1, "float", static_matrix_name(i)))
        .collect();
    for t in &mut matrix_children {
        t.level += 1;
    }
    let mut matrix_item = vec![n(0, "Matrix4x4f", "data")];
    matrix_item.extend(matrix_children);
    v.extend(vector_of(2, "m_Bindpose", matrix_item));
    // textureRect / offsets / settings / uvTransform / downscaleMultiplier
    v.push(n(2, "Rectf", "textureRect"));
    v.push(n(3, "float", "x"));
    v.push(n(3, "float", "y"));
    v.push(n(3, "float", "width"));
    v.push(n(3, "float", "height"));
    v.push(n(2, "Vector2f", "textureRectOffset"));
    v.push(n(3, "float", "x"));
    v.push(n(3, "float", "y"));
    v.push(n(2, "Vector2f", "atlasRectOffset"));
    v.push(n(3, "float", "x"));
    v.push(n(3, "float", "y"));
    v.push(n(2, "unsigned int", "settingsRaw"));
    v.push(n(2, "Vector4f", "uvTransform"));
    for f in ["x", "y", "z", "w"] {
        v.push(n(3, "float", static_float_name(f)));
    }
    v.push(n(2, "float", "downscaleMultiplier"));
    // physics shape / bones
    v.extend(vector_of(
        1,
        "m_PhysicsShape",
        vec![n(3, "vector", "data"), n(4, "Array", "Array"), n(5, "int", "size"), n(5, "Vector2f", "data"), n(6, "float", "x"), n(6, "float", "y")],
    ));
    v.extend(vector_of(
        1,
        "m_Bones",
        vec![
            n(3, "SpriteBone", "data"),
            n(4, "string", "name"),
            n(5, "Array", "Array"),
            n(6, "int", "size"),
            n(6, "char", "data"),
            n(4, "Vector3f", "position"),
            n(5, "float", "x"),
            n(5, "float", "y"),
            n(5, "float", "z"),
            n(4, "Quaternionf", "rotation"),
            n(5, "float", "x"),
            n(5, "float", "y"),
            n(5, "float", "z"),
            n(5, "float", "w"),
            n(4, "float", "length"),
            n(4, "int", "parentId"),
        ],
    ));
    v
}

fn static_float_name(s: &'static str) -> &'static str {
    match s {
        "x" => "x",
        "y" => "y",
        "z" => "z",
        _ => "w",
    }
}
fn static_matrix_name(i: usize) -> &'static str {
    const NAMES: [&str; 16] = [
        "e00", "e01", "e02", "e03", "e10", "e11", "e12", "e13", "e20", "e21", "e22", "e23", "e30",
        "e31", "e32", "e33",
    ];
    NAMES[i]
}

fn static_data_index(i: u32) -> &'static str {
    match i {
        0 => "data[0]",
        1 => "data[1]",
        2 => "data[2]",
        _ => "data[3]",
    }
}

fn game_object_tree() -> Vec<TNode> {
    let mut v = vec![n(0, "GameObject", "Base")];
    v.extend(vector_of(
        1,
        "m_Component",
        vec![
            n(4, "ComponentPair", "data"),
            n(5, "PPtr<Component>", "component"),
            n(6, "int", "m_FileID"),
            n(6, "SInt64", "m_PathID"),
        ],
    ));
    v.push(n(1, "unsigned int", "m_Layer"));
    v.extend(string_field(1, "m_Name"));
    v.push(n(1, "UInt16", "m_Tag"));
    v.push(n(1, "bool", "m_IsActive"));
    v
}

fn asset_bundle_tree() -> Vec<TNode> {
    let mut v = vec![n(0, "AssetBundle", "Base")];
    v.extend(string_field(1, "m_Name"));
    v.extend(vector_of(
        1,
        "m_PreloadTable",
        vec![
            n(4, "PPtr<Object>", "data"),
            n(5, "int", "m_FileID"),
            n(5, "SInt64", "m_PathID"),
        ],
    ));
    // map m_Container
    v.push(n(1, "map", "m_Container"));
    v.push(n(2, "Array", "Array"));
    v.push(n(3, "int", "size"));
    v.push(n(3, "pair", "data"));
    v.push(n(4, "string", "first"));
    v.push(n(5, "Array", "Array"));
    v.push(n(6, "int", "size"));
    v.push(n(6, "char", "data"));
    v.push(n(4, "AssetInfo", "second"));
    v.push(n(5, "int", "preloadIndex"));
    v.push(n(5, "int", "preloadSize"));
    v.push(n(5, "PPtr<Object>", "asset"));
    v.push(n(6, "int", "m_FileID"));
    v.push(n(6, "SInt64", "m_PathID"));
    // m_MainAsset
    v.push(n(1, "AssetInfo", "m_MainAsset"));
    v.push(n(2, "int", "preloadIndex"));
    v.push(n(2, "int", "preloadSize"));
    v.push(n(2, "PPtr<Object>", "asset"));
    v.push(n(3, "int", "m_FileID"));
    v.push(n(3, "SInt64", "m_PathID"));
    v.push(n(1, "unsigned int", "m_RuntimeCompatibility"));
    v.extend(string_field(1, "m_AssetBundleName"));
    v.extend(vector_of(
        1,
        "m_Dependencies",
        vec![n(3, "string", "data"), n(4, "Array", "Array"), n(5, "int", "size"), n(5, "char", "data")],
    ));
    v.push(na(1, "bool", "m_IsStreamedSceneAssetBundle"));
    v.push(n(1, "int", "m_ExplicitDataLayout"));
    v.push(n(1, "int", "m_PathFlags"));
    // m_SceneHashes map (empty)
    v.push(n(1, "map", "m_SceneHashes"));
    v.push(n(2, "Array", "Array"));
    v.push(n(3, "int", "size"));
    v.push(n(3, "pair", "data"));
    v.push(n(4, "string", "first"));
    v.push(n(5, "Array", "Array"));
    v.push(n(6, "int", "size"));
    v.push(n(6, "char", "data"));
    v.push(n(4, "string", "second"));
    v.push(n(5, "Array", "Array"));
    v.push(n(6, "int", "size"));
    v.push(n(6, "char", "data"));
    v
}

// ---------------------------------------------------------------------------
// typetree blob writer (serialized file version 20 => records carry refHash)
// ---------------------------------------------------------------------------

/// Serialize a flattened tree in the v12+ blob format (node table + string buffer).
pub fn write_tree_blob(nodes: &[TNode], version: u32) -> Vec<u8> {
    let mut strings: Vec<String> = Vec::new();
    let mut string_offset: std::collections::HashMap<String, u32> = Default::default();
    let mut intern = |strings: &mut Vec<String>,
                      map: &mut std::collections::HashMap<String, u32>,
                      s: &str|
     -> u32 {
        if let Some(off) = map.get(s) {
            return *off;
        }
        let off = strings.iter().map(|x| x.len() + 1).sum::<usize>() as u32;
        strings.push(s.to_string());
        map.insert(s.to_string(), off);
        off
    };

    // pre-intern all strings to compute buffer size
    let mut string_map: std::collections::HashMap<String, u32> = Default::default();
    for t in nodes {
        intern(&mut strings, &mut string_map, t.type_name);
        intern(&mut strings, &mut string_map, t.name);
    }
    let mut sb: Vec<u8> = Vec::new();
    for s in &strings {
        sb.extend_from_slice(s.as_bytes());
        sb.push(0);
    }

    let mut w = W::new();
    w.i32(nodes.len() as i32);
    w.i32(sb.len() as i32);
    for t in nodes {
        w.u16(1); // m_Version
        w.u8(t.level as u8);
        w.u8(0); // type flags
        w.u32(string_map[t.type_name]);
        w.u32(string_map[t.name]);
        w.i32(0); // byte size (unused for reading)
        w.i32(0); // index
        w.i32(t.meta as i32);
        if version >= 19 {
            w.i64(0); // ref type hash
        }
    }
    w.0.extend_from_slice(&sb);
    w.0
}

// ---------------------------------------------------------------------------
// object writers (field layouts = 2019.4 release trees above)
// ---------------------------------------------------------------------------

pub fn write_texture2d(
    name: &str,
    width: i32,
    height: i32,
    format_id: i32,
    image_data: &[u8],
    stream: Option<(&str, u32, u32)>,
) -> Vec<u8> {
    let mut w = W::new();
    w.astring(name);
    w.i32(0); // m_ForcedFallbackFormat
    w.bool(false);
    w.align(4); // m_DownscaleFallback align
    w.i32(width);
    w.i32(height);
    w.i32(image_data.len() as i32); // m_CompleteImageSize
    w.i32(format_id);
    w.i32(1); // m_MipCount
    w.bool(false); // m_IsReadable
    w.bool(false); // m_IgnoreMasterTextureLimit
    w.bool(false); // m_IsPreProcessed
    w.bool(false);
    w.align(4); // m_StreamingMipmaps align
    w.i32(0); // m_StreamingMipmapsPriority
    w.i32(1); // m_ImageCount
    w.i32(2); // m_TextureDimension (Tex2D)
    w.i32(1); // filter
    w.i32(1); // aniso
    w.f32(0.0); // mip bias
    w.i32(2); // wrap U
    w.i32(2); // wrap V
    w.i32(2); // wrap W
    w.i32(0); // lightmap format
    w.i32(1); // color space
    w.blob(image_data); // "image data"
    match stream {
        Some((path, off, size)) => {
            w.u32(off);
            w.u32(size);
            w.astring(path);
        }
        None => {
            w.u32(0);
            w.u32(0);
            w.astring("");
        }
    }
    w.0
}

pub fn write_text_asset(name: &str, script: &[u8]) -> Vec<u8> {
    let mut w = W::new();
    w.astring(name);
    // m_Script: i32 length + raw bytes + align4
    w.i32(script.len() as i32);
    w.0.extend_from_slice(script);
    w.align(4);
    w.0
}

pub fn write_audio_clip(
    name: &str,
    channels: i32,
    frequency: i32,
    source: &str,
    offset: u64,
    size: u64,
    compression_format: i32,
) -> Vec<u8> {
    let mut w = W::new();
    w.astring(name);
    w.i32(1); // m_LoadType compressed
    w.i32(channels);
    w.i32(frequency);
    w.i32(16); // bits per sample
    w.f32(size as f32 / (frequency * channels * 2) as f32); // length secs
    w.bool(false); // tracker
    w.bool(false);
    w.align(4); // m_Ambisonic align
    w.i32(0); // subsound index
    w.bool(true); // preload
    w.bool(false); // background
    w.bool(false); // legacy 3d
    w.align(4); // m_Legacy3D align
    w.astring(source); // m_Resource.m_Source
    w.u64(offset);
    w.u64(size);
    w.i32(compression_format);
    w.align(4); // m_CompressionFormat align
    w.0
}


pub struct MeshBuilder {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uv0: Vec<[f32; 2]>,
    pub indices: Vec<u16>,
}

/// 2019.4 mesh: stream 0 = positions(0) + normals(12) + uv0(24), stride 32, f32.
/// Channels array (2018+ indexing): 0=pos, 1=normal, 2=tangent(empty), 3=color(empty), 4=uv0.
pub fn write_mesh(name: &str, m: &MeshBuilder, stream_data: Option<(&str, u32, u32)>) -> Vec<u8> {
    let vertex_count = m.positions.len() as u32;
    let mut vbuf: Vec<u8> = Vec::with_capacity(vertex_count as usize * 32);
    for i in 0..m.positions.len() {
        let p = m.positions[i];
        let nrm = m.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]);
        let uv = m.uv0.get(i).copied().unwrap_or([0.0, 0.0]);
        vbuf.extend_from_slice(&p[0].to_le_bytes());
        vbuf.extend_from_slice(&p[1].to_le_bytes());
        vbuf.extend_from_slice(&p[2].to_le_bytes());
        vbuf.extend_from_slice(&nrm[0].to_le_bytes());
        vbuf.extend_from_slice(&nrm[1].to_le_bytes());
        vbuf.extend_from_slice(&nrm[2].to_le_bytes());
        vbuf.extend_from_slice(&uv[0].to_le_bytes());
        vbuf.extend_from_slice(&uv[1].to_le_bytes());
    }
    let index_bytes: Vec<u8> = m.indices.iter().flat_map(|i| i.to_le_bytes()).collect();

    let mut w = W::new();
    w.astring(name);
    // m_SubMeshes: one triangle-list submesh covering all indices
    w.count(1);
    w.u32(0); // firstByte
    w.u32(m.indices.len() as u32);
    w.i32(0); // topology triangles
    w.u32(0); // baseVertex
    w.u32(0); // firstVertex
    w.u32(vertex_count);
    w.f32(0.0);
    w.f32(0.0);
    w.f32(0.0);
    w.f32(1.0);
    w.f32(1.0);
    w.f32(1.0);
    // m_Shapes: vertices/shapes/channels/fullWeights empty
    w.count(0);
    w.count(0);
    w.count(0);
    w.count(0);
    // m_BindPose empty
    w.count(0);
    // m_BoneNameHashes empty
    w.count(0);
    w.u32(0); // m_RootBoneNameHash
    // m_BonesAABB empty
    w.count(0);
    // m_VariableBoneCountWeights.m_Data empty
    w.count(0);
    w.u8(0); // m_MeshCompression
    w.bool(true); // m_IsReadable
    w.bool(true); // m_KeepVertices
    w.bool(true); // m_KeepIndices
    w.align(4);
    w.i32(0); // m_IndexFormat = 16 bit
    // m_IndexBuffer
    w.blob(&index_bytes);
    // m_VertexData
    w.u32(vertex_count);
    w.count(5);
    let mut ch = |stream: u8, offset: u8, format: u8, dim: u8| {
        w.u8(stream);
        w.u8(offset);
        w.u8(format);
        w.u8(dim);
    };
    ch(0, 0, 0, 3); // 0 positions: float3
    ch(0, 12, 0, 3); // 1 normals: float3
    ch(0, 0, 0, 0); // 2 tangent: unused
    ch(0, 0, 0, 0); // 3 color: unused
    ch(0, 24, 0, 2); // 4 uv0: float2
    // m_DataSize: inline vertex buffer (streamed variant would leave this
    // empty and point m_StreamData at a CAB-.resS node)
    let _ = stream_data;
    w.blob(&vbuf);
    // m_CompressedMesh: 10 PBVs + UVInfo (all empty)
    let mut pbv = |with_range: bool| {
        w.u32(0); // num items
        if with_range {
            w.f32(0.0);
            w.f32(0.0);
        }
        w.count(0); // data
        w.u8(0); // bit size
        w.align(4);
    };
    pbv(true); // m_Vertices
    pbv(true); // m_UV
    pbv(true); // m_Normals
    pbv(true); // m_Tangents
    pbv(false); // m_Weights
    pbv(false); // m_NormalSigns
    pbv(false); // m_TangentSigns
    pbv(true); // m_FloatColors
    pbv(false); // m_BoneIndices
    pbv(false); // m_Triangles
    w.u32(0); // m_UVInfo
    // m_LocalAABB
    w.f32(0.0);
    w.f32(0.0);
    w.f32(0.0);
    w.f32(1.0);
    w.f32(1.0);
    w.f32(1.0);
    w.i32(0); // m_MeshUsageFlags
    w.blob(&[]); // m_BakedConvexCollisionMesh
    w.blob(&[]); // m_BakedTriangleCollisionMesh
    w.f32(0.0); // m_MeshMetrics[0]
    w.f32(0.0); // m_MeshMetrics[1]
    w.align(4); // m_MeshMetrics[1] align flag
    // m_StreamData
    match stream_data {
        Some((path, off, size)) => {
            w.u32(off);
            w.u32(size);
            w.astring(path);
        }
        None => {
            w.u32(0);
            w.u32(0);
            w.astring("");
        }
    }
    w.0
}

pub fn write_sprite(
    name: &str,
    rect: (f32, f32, f32, f32),
    texture_path_id: i64,
) -> Vec<u8> {
    let mut w = W::new();
    w.astring(name);
    // m_Rect
    w.f32(rect.0);
    w.f32(rect.1);
    w.f32(rect.2);
    w.f32(rect.3);
    // m_Offset
    w.f32(0.0);
    w.f32(0.0);
    // m_Border
    w.f32(0.0);
    w.f32(0.0);
    w.f32(0.0);
    w.f32(0.0);
    w.f32(100.0); // m_PixelsToUnits
    // m_Pivot
    w.f32(0.5);
    w.f32(0.5);
    w.u32(0); // m_Extrude
    w.bool(false); // m_IsPolygon
    w.align(4);
    // m_RenderDataKey
    for i in 0..4 {
        w.u32(0x12345670 + i);
    }
    w.i64(0);
    // m_AtlasTags empty
    w.count(0);
    // m_SpriteAtlas PPtr (null)
    w.i32(0);
    w.i64(0);
    // m_RD
    w.i32(0); // texture m_FileID (same file)
    w.i64(texture_path_id);
    w.i32(0); // alphaTexture null
    w.i64(0);
    w.count(0); // secondaryTextures
    // m_SubMeshes: one quad submesh
    w.count(1);
    w.u32(0); // firstByte
    w.u32(6); // indexCount
    w.i32(0); // topology
    w.u32(0); // baseVertex
    w.u32(0); // firstVertex
    w.u32(4); // vertexCount
    w.f32(0.0);
    w.f32(0.0);
    w.f32(0.0);
    w.f32(1.0);
    w.f32(1.0);
    w.f32(0.0);
    // m_IndexBuffer: 2 triangles (u32? sprites use 16-bit) — 6 u16 indices
    let idx: [u16; 6] = [0, 1, 2, 2, 1, 3];
    let ib: Vec<u8> = idx.iter().flat_map(|i| i.to_le_bytes()).collect();
    w.blob(&ib);
    // m_VertexData: 4 verts, pos(12)+uv(8) = stride 20, stream 0
    w.u32(4);
    w.count(5);
    let mut ch = |stream: u8, offset: u8, format: u8, dim: u8| {
        w.u8(stream);
        w.u8(offset);
        w.u8(format);
        w.u8(dim);
    };
    ch(0, 0, 0, 3); // positions
    ch(0, 0, 0, 0); // normals unused
    ch(0, 0, 0, 0); // tangent unused
    ch(0, 0, 0, 0); // color unused
    ch(0, 12, 0, 2); // uv0
    let (rx, ry, rw, rh) = rect;
    let mut vb: Vec<u8> = Vec::new();
    for (px, py, uu, vv) in [
        (rx, ry, 0.0f32, 0.0f32),
        (rx, ry + rh, 0.0f32, 1.0f32),
        (rx + rw, ry, 1.0f32, 0.0f32),
        (rx + rw, ry + rh, 1.0f32, 1.0f32),
    ] {
        vb.extend_from_slice(&px.to_le_bytes());
        vb.extend_from_slice(&py.to_le_bytes());
        vb.extend_from_slice(&0f32.to_le_bytes());
        vb.extend_from_slice(&uu.to_le_bytes());
        vb.extend_from_slice(&vv.to_le_bytes());
    }
    w.blob(&vb);
    // m_Bindpose empty
    w.count(0);
    // textureRect (defaults to the sprite rect)
    w.f32(rx);
    w.f32(ry);
    w.f32(rw);
    w.f32(rh);
    // textureRectOffset
    w.f32(0.0);
    w.f32(0.0);
    // atlasRectOffset
    w.f32(0.0);
    w.f32(0.0);
    w.u32(0); // settingsRaw
    // uvTransform (identity-ish)
    w.f32(1.0);
    w.f32(0.0);
    w.f32(0.0);
    w.f32(1.0);
    w.f32(1.0); // downscaleMultiplier
    // m_PhysicsShape empty
    w.count(0);
    // m_Bones empty
    w.count(0);
    w.0
}

pub fn write_game_object(name: &str) -> Vec<u8> {
    let mut w = W::new();
    w.count(0); // m_Component empty
    w.u32(0); // m_Layer
    w.astring(name);
    w.u16(0); // m_Tag
    w.bool(true); // m_IsActive
    w.0
}

pub fn write_asset_bundle(name: &str, container: &[(&str, i64)]) -> Vec<u8> {
    let mut w = W::new();
    w.astring(name);
    w.count(0); // m_PreloadTable
    // m_Container
    w.count(container.len());
    for (path, pid) in container {
        w.astring(path);
        w.i32(0); // preloadIndex
        w.i32(0); // preloadSize
        w.i32(0); // asset m_FileID
        w.i64(*pid);
    }
    // m_MainAsset
    w.i32(0);
    w.i32(0);
    w.i32(0);
    w.i64(0);
    w.u32(1); // m_RuntimeCompatibility
    w.astring(""); // m_AssetBundleName
    w.count(0); // m_Dependencies
    w.bool(false); // m_IsStreamedSceneAssetBundle
    w.align(4);
    w.i32(1); // m_ExplicitDataLayout
    w.i32(0); // m_PathFlags
    w.count(0); // m_SceneHashes (map, empty)
    w.0
}

// ---------------------------------------------------------------------------
// SerializedFile builder
// ---------------------------------------------------------------------------

pub struct UnityObjectSpec {
    pub class_id: i32,
    pub path_id: i64,
    pub data: Vec<u8>,
}

pub const UNITY_VERSION_2019: &str = "2019.4.40f1";
pub const SERIALIZED_VERSION_2019: u32 = 20;

/// Build a real Unity SerializedFile (v20, little endian) with either embedded
/// typetrees or stripped ones (reader falls back to the TPK).
pub fn build_serialized_file(
    enable_type_tree: bool,
    objects: Vec<UnityObjectSpec>,
    externals: &[&str],
) -> Vec<u8> {
    let version = SERIALIZED_VERSION_2019;
    // distinct types in first-use order
    let mut type_ids: Vec<i32> = Vec::new();
    for o in &objects {
        if !type_ids.contains(&o.class_id) {
            type_ids.push(o.class_id);
        }
    }

    // tree per class id
    let tree_for = |cid: i32| -> Vec<TNode> {
        match cid {
            28 => texture2d_tree(),
            49 => text_asset_tree(),
            83 => audio_clip_tree(),
            43 => mesh_tree(),
            213 => sprite_tree(),
            1 => game_object_tree(),
            142 => asset_bundle_tree(),
            _ => vec![n(0, "Unknown", "Base")],
        }
    };

    // layout: header(32) + metadata + align16 + object data
    let mut meta = W::new();
    meta.s(UNITY_VERSION_2019); // version >= 7
    meta.i32(19); // target platform: iPhone? 19 = WIIU... value irrelevant
    meta.bool(enable_type_tree); // version >= 13
    meta.count(type_ids.len());
    for cid in &type_ids {
        meta.i32(*cid); // class id
        meta.bool(false); // is stripped (v>=16)
        meta.i16(-1); // script type index (v>=17)
        // old type hash (v>=13): 16 bytes (no script id: class != 114)
        meta.0.extend_from_slice(&[0xAA; 16]);
        if enable_type_tree {
            let blob = write_tree_blob(&tree_for(*cid), version);
            // v12-22 blob: raw (node count is the first field); no length prefix
            meta.0.extend_from_slice(&blob);
        }
        // v>=21 type deps omitted (version 20)
    }
    // objects
    meta.count(objects.len());
    // object data writer
    let mut data = W::new();
    let mut entries: Vec<(i64, u32, u32, usize)> = Vec::new(); // (path_id, start, size, type_index)
    for o in &objects {
        data.align(8);
        let start = data.0.len() as u32;
        data.0.extend_from_slice(&o.data);
        let ti = type_ids.iter().position(|c| *c == o.class_id).unwrap();
        entries.push((o.path_id, start, o.data.len() as u32, ti));
    }
    for (pid, start, size, ti) in &entries {
        meta.align(4); // v>=14 path id align
        meta.i64(*pid);
        meta.u32(*start); // byte start (u32 for v<22)
        meta.u32(*size);
        meta.i32(*ti as i32);
        // v>=16: no class id / destroyed / script index / stripped fields
    }
    // script types (v>=11): 0
    meta.count(0);
    // externals
    meta.count(externals.len());
    for ext in externals {
        meta.s(""); // temp empty (v>=6)
        meta.0.extend_from_slice(&[0xEE; 16]); // guid (v>=5)
        meta.i32(0); // type (v>=5)
        meta.s(ext);
    }
    // v>=20 ref types: 0
    meta.count(0);
    // v>=5 user information
    meta.s("");

    let header_size = 20u32; // 4x u32 + endian bool + 3 reserved
    let metadata_size = meta.0.len() as u32;
    let data_offset = (header_size + metadata_size).div_ceil(16) * 16;
    let file_size = data_offset + data.0.len() as u32;

    let mut w = W::new();
    // header fields are BIG-endian (verified against real Unity files)
    w.u32_be(metadata_size);
    w.u32_be(file_size);
    w.u32_be(version);
    w.u32_be(data_offset);
    w.bool(false); // little endian metadata
    w.align(4); // 3 reserved bytes
    w.0.extend_from_slice(&meta.0);
    w.align(16);
    debug_assert_eq!(w.0.len() as u32, data_offset);
    w.0.extend_from_slice(&data.0);
    w.0
}

// ---------------------------------------------------------------------------
// UnityFS builder
// ---------------------------------------------------------------------------

/// Build a real UnityFS bundle (format v7, aligned) with one data block.
/// `lz4` selects LZ4 block compression, otherwise uncompressed.
pub fn build_unityfs(nodes: &[(&str, Vec<u8>)], lz4: bool) -> Vec<u8> {
    // concatenated node data
    let mut payload: Vec<u8> = Vec::new();
    let mut dir: Vec<(String, u64, u64)> = Vec::new();
    for (name, data) in nodes {
        let off = payload.len() as u64;
        dir.push((name.to_string(), off, data.len() as u64));
        payload.extend_from_slice(data);
    }

    // compress payload block first so the blocks info records the real size
    let payload_uncompressed = payload.len() as u32;
    let payload_comp = if lz4 { lz4_block_compress(&payload) } else { payload };

    // blocks info (BIG-endian): hash + 1 block + directory
    let mut bi = W::new();
    bi.0.extend_from_slice(&[0u8; 16]); // uncompressed data hash
    bi.i32_be(1); // block count
    bi.u32_be(payload_uncompressed); // uncompressed size
    bi.u32_be(payload_comp.len() as u32); // compressed size
    bi.u16_be(if lz4 { 2 } else { 0 }); // block flags
    bi.i32_be(dir.len() as i32);
    for (name, off, size) in &dir {
        bi.u64_be(*off);
        bi.u64_be(*size);
        bi.u32_be(0); // node flags
        bi.s(name);
    }
    let bi_raw = bi.0;

    // compress blocks info
    let (flags, bi_comp) = if lz4 {
        (0x40 | 2, lz4_block_compress(&bi_raw)) // 0x40 = blocks+directory combined
    } else {
        (0x40, bi_raw.clone())
    };

    // header: sig + version + strings + size/comp/uncomp/flags (no align between),
    // then 16-byte alignment before the blocks info (format v7).
    let mut w = W::new();
    w.s("UnityFS");
    w.u32(7); // bundle format version
    w.s("5.x.x"); // version player
    w.s(UNITY_VERSION_2019); // version engine
    let size_pos = w.0.len(); // patch target for the total size field
    w.u64_be(0); // bundle size placeholder
    w.u32_be(bi_comp.len() as u32); // compressed blocks info size
    w.u32_be(bi_raw.len() as u32); // uncompressed blocks info size
    w.u32_be(flags);
    w.align(16); // v7 alignment BEFORE blocks info
    w.0.extend_from_slice(&bi_comp);
    w.0.extend_from_slice(&payload_comp);
    let total = w.0.len() as u64;
    w.0[size_pos..size_pos + 8].copy_from_slice(&total.to_le_bytes());
    w.0
}

fn lz4_block_compress(input: &[u8]) -> Vec<u8> {
    lz4_flex::block::compress(input)
}

/// Standard sample project: texture + text + audio + mesh + sprite + gameobject
/// + assetbundle (container map), returning the ordered object specs.
pub fn sample_objects(
    tex_name: &str,
    tex_rgba: &[u8],
    tex_w: i32,
    tex_h: i32,
    resource_cab: &str,
) -> Vec<UnityObjectSpec> {
    use crate::unity::texture_rgba32_bytes;
    let mut objects = Vec::new();

    // Texture2D (path id 1): inline RGBA32
    objects.push(UnityObjectSpec {
        class_id: 28,
        path_id: 1,
        data: write_texture2d(tex_name, tex_w, tex_h, 4, &texture_rgba32_bytes(tex_rgba), None),
    });

    // TextAsset (path id 2)
    objects.push(UnityObjectSpec {
        class_id: 49,
        path_id: 2,
        data: write_text_asset("readme", b"hello from unity sample"),
    });

    // AudioClip (path id 3): streamed from CAB-xxx.resS at offset 0
    objects.push(UnityObjectSpec {
        class_id: 83,
        path_id: 3,
        data: write_audio_clip("bgm", 2, 44100, "archive:/CAB-sample/CAB-sample.resS", 0, 4096, 1),
    });
    let _ = resource_cab;

    // Mesh (path id 4): 4 vertices, 2 triangles, inline vertex data
    let mesh = MeshBuilder {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, -1.0]; 4],
        uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
        indices: vec![0, 1, 2, 2, 1, 3],
    };
    objects.push(UnityObjectSpec {
        class_id: 43,
        path_id: 4,
        data: write_mesh("quad", &mesh, None),
    });

    // Sprite (path id 5): references the texture (path id 1)
    objects.push(UnityObjectSpec {
        class_id: 213,
        path_id: 5,
        data: write_sprite("hero", (0.0, 0.0, 8.0, 8.0), 1),
    });

    // GameObject (path id 6)
    objects.push(UnityObjectSpec {
        class_id: 1,
        path_id: 6,
        data: write_game_object("HeroObject"),
    });

    // AssetBundle (path id 7): container map with friendly paths
    objects.push(UnityObjectSpec {
        class_id: 142,
        path_id: 7,
        data: write_asset_bundle(
            "samplebundle",
            &[
                ("assets/sample/hero.png", 1),
                ("assets/sample/hero.plist", 5),
            ],
        ),
    });

    objects
}

/// Resource payload appended to a bundle as CAB-xxx.resS.
pub fn sample_resource() -> Vec<u8> {
    // 4096 bytes of pseudo audio-ish payload
    (0..4096u32)
        .map(|i| ((i * 7) % 251) as u8)
        .collect()
}

/// Flip an RGBA image top-down -> bottom-up (Unity's on-disk row order).
pub fn texture_rgba32_bytes(rgba_topdown: &[u8]) -> Vec<u8> {
    // caller provides w/h via closure-free convention: we can't know w/h here,
    // so this helper is called with the already-flipped buffer by sample_objects;
    // see sample_bundle_rgba which performs the flip.
    rgba_topdown.to_vec()
}

/// Convenience: build a complete UnityFS bundle with a texture (given as
/// top-down RGBA), the standard sample objects, and a CAB-sample.resS payload.
pub fn sample_bundle_rgba(name: &str, rgba: &[u8], w: i32, h: i32, lz4: bool) -> Vec<u8> {
    // flip rows to Unity's bottom-up order
    let stride = (w as usize) * 4;
    let mut flipped: Vec<u8> = Vec::with_capacity(rgba.len());
    for row in (0..h as usize).rev() {
        flipped.extend_from_slice(&rgba[row * stride..(row + 1) * stride]);
    }
    let cab = build_serialized_file(true, sample_objects(name, &flipped, w, h, "CAB-sample"), &["archive:/CAB-sample/CAB-sample.resS"]);
    let cab_stripped = build_serialized_file(false, sample_objects(name, &flipped, w, h, "CAB-sample"), &["archive:/CAB-sample/CAB-sample.resS"]);
    let res = sample_resource();
    let _ = cab_stripped;
    build_unityfs(
        &[
            ("CAB-sample", cab),
            ("CAB-sample.resS", res),
        ],
        lz4,
    )
}

/// Standalone .assets-style serialized file (level0/sharedassets0 equivalent).
pub fn sample_standalone_serialized(rgba: &[u8], w: i32, h: i32) -> Vec<u8> {
    let stride = (w as usize) * 4;
    let mut flipped: Vec<u8> = Vec::with_capacity(rgba.len());
    for row in (0..h as usize).rev() {
        flipped.extend_from_slice(&rgba[row * stride..(row + 1) * stride]);
    }
    build_serialized_file(false, sample_objects("standalone", &flipped, w, h, ""), &["sharedassets0.resource"])
}

/// Stripped-typetree standalone serialized file (typical modern game data).
pub fn sample_standalone_stripped(rgba: &[u8], w: i32, h: i32) -> Vec<u8> {
    let stride = (w as usize) * 4;
    let mut flipped: Vec<u8> = Vec::with_capacity(rgba.len());
    for row in (0..h as usize).rev() {
        flipped.extend_from_slice(&rgba[row * stride..(row + 1) * stride]);
    }
    build_serialized_file(false, sample_objects("stripped", &flipped, w, h, ""), &[])
}
