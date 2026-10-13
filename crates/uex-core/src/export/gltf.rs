//! GLTF 2.0 (.glb) exporter for extracted meshes, skins and animations.

use crate::assets::mesh::{ExtractedMesh, ExtractedAnimation};
use crate::error::{Error, Result};
use serde_json::{json, Value};

pub struct GltfScene {
    pub meshes: Vec<ExtractedMesh>,
    pub animations: Vec<ExtractedAnimation>,
    pub texture_name: Option<String>,
}

struct BufferBuilder {
    data: Vec<u8>,
}

impl BufferBuilder {
    fn new() -> Self {
        Self { data: Vec::new() }
    }
    fn add(&mut self, bytes: &[u8]) -> (usize, usize) {
        while self.data.len() % 4 != 0 {
            self.data.push(0);
        }
        let off = self.data.len();
        self.data.extend_from_slice(bytes);
        (off, bytes.len())
    }
    fn len(&self) -> usize {
        self.data.len()
    }
}

/// Write a .glb for the scene. Returns file bytes.
pub fn write_glb(scene: &GltfScene) -> Result<Vec<u8>> {
    let mut buf = BufferBuilder::new();
    let mut buffer_views = Vec::new();
    let mut accessors = Vec::new();
    let mut materials = Vec::new();
    let mut images = Vec::new();
    let mut samplers = Vec::new();
    let mut textures = Vec::new();

    if scene.texture_name.is_some() {
        samplers.push(json!({"magFilter": 9729, "minFilter": 9987, "wrapS": 33071, "wrapT": 33071}));
    }

    let mut add_view = |buf: &mut BufferBuilder, bytes: &[u8], target: u32| -> usize {
        let (off, len) = buf.add(bytes);
        buffer_views.push(json!({"buffer": 0, "byteOffset": off, "byteLength": len, "target": target}));
        buffer_views.len() - 1
    };

    let mut nodes = Vec::new();
    let mut skins = Vec::new();
    let mut scenes_meshes = Vec::new();

    for mesh in &scene.meshes {
        let vert_count = mesh.vertices.len();
        if vert_count == 0 {
            continue;
        }

        // positions
        let positions: Vec<u8> = mesh
            .vertices
            .iter()
            .flat_map(|v| v.position.iter().flat_map(|f| f.to_le_bytes()))
            .collect();
        let min: Vec<f32> = (0..3)
            .map(|i| mesh.vertices.iter().map(|v| v.position[i]).fold(f32::INFINITY, f32::min))
            .collect();
        let max: Vec<f32> = (0..3)
            .map(|i| mesh.vertices.iter().map(|v| v.position[i]).fold(f32::NEG_INFINITY, f32::max))
            .collect();
        let pos_view = add_view(&mut buf, &positions, 34962);
        accessors.push(json!({
            "bufferView": pos_view, "componentType": 5126, "count": vert_count,
            "type": "VEC3", "min": min, "max": max
        }));
        let pos_acc = accessors.len() - 1;

        // normals
        let normals: Vec<u8> = mesh
            .vertices
            .iter()
            .flat_map(|v| v.normal.iter().flat_map(|f| f.to_le_bytes()))
            .collect();
        let norm_view = add_view(&mut buf, &normals, 34962);
        accessors.push(json!({"bufferView": norm_view, "componentType": 5126, "count": vert_count, "type": "VEC3"}));
        let norm_acc = accessors.len() - 1;

        // UV0
        let uv_view = if mesh.vertices.first().map(|v| !v.uv.is_empty()).unwrap_or(false) {
            let mut uvs: Vec<u8> = Vec::with_capacity(vert_count * 8);
            for v in &mesh.vertices {
                let uv = v.uv.first().copied().unwrap_or([0.0, 0.0]);
                uvs.extend_from_slice(&uv[0].to_le_bytes());
                uvs.extend_from_slice(&uv[1].to_le_bytes());
            }
            add_view(&mut buf, &uvs, 34962)
        } else {
            0
        };
        accessors.push(json!({"bufferView": uv_view, "componentType": 5126, "count": vert_count, "type": "VEC2"}));
        let uv_acc = accessors.len() - 1;

        let mut attributes = json!({"POSITION": pos_acc, "NORMAL": norm_acc, "TEXCOORD_0": uv_acc});

        // skin joints + weights for skeletal meshes
        if mesh.is_skeletal {
            let joints: Vec<u8> = mesh
                .vertices
                .iter()
                .flat_map(|v| v.bone_indices.iter().flat_map(|b| b.to_le_bytes()))
                .collect();
            let jv = add_view(&mut buf, &joints, 34962);
            accessors.push(json!({"bufferView": jv, "componentType": 5123, "count": vert_count, "type": "VEC4"}));
            let joints_acc = accessors.len() - 1;

            let weights: Vec<u8> = mesh
                .vertices
                .iter()
                .flat_map(|v| v.bone_weights.iter().flat_map(|w| w.to_le_bytes()))
                .collect();
            let wv = add_view(&mut buf, &weights, 34962);
            accessors.push(json!({"bufferView": wv, "componentType": 5126, "count": vert_count, "type": "VEC4"}));
            let weights_acc = accessors.len() - 1;
            attributes["JOINTS_0"] = json!(joints_acc);
            attributes["WEIGHTS_0"] = json!(weights_acc);

            // build skin with joints + inverse bind matrices
            let bone_count = mesh.bones.len().max(1);
            let mut joint_list = Vec::new();
            for i in 0..bone_count {
                joint_list.push(json!(i));
            }
            let mut ibms: Vec<u8> = Vec::new();
            for i in 0..bone_count {
                let m = bind_matrix(mesh, i);
                for row in m.iter() {
                    for f in row.iter() {
                        ibms.extend_from_slice(&f.to_le_bytes());
                    }
                }
            }
            let iv = add_view(&mut buf, &ibms, 34962);
            accessors.push(json!({"bufferView": iv, "componentType": 5126, "count": bone_count, "type": "MAT4"}));
            let ibm_acc = accessors.len() - 1;

            // skeleton node hierarchy
            let node_base = nodes.len();
            for bone in &mesh.bones {
                nodes.push(json!({
                    "name": bone.name,
                    "translation": bone.position,
                    "rotation": [bone.rotation[0], bone.rotation[1], bone.rotation[2], bone.rotation[3]],
                    "scale": bone.scale,
                }));
            }
            // wire children
            for (i, bone) in mesh.bones.iter().enumerate() {
                if bone.parent >= 0 && (bone.parent as usize) < mesh.bones.len() {
                    let p = node_base + bone.parent as usize;
                    let c = node_base + i;
                    if let Some(obj) = nodes.get_mut(p) {
                        obj["children"] = match obj.get("children") {
                            Some(Value::Array(arr)) => {
                                let mut a = arr.clone();
                                a.push(json!(c));
                                json!(a)
                            }
                            _ => json!([c]),
                        };
                    }
                }
            }
            let joint_node_indices: Vec<Value> = (0..bone_count).map(|i| json!(node_base + i)).collect();
            skins.push(json!({
                "joints": joint_node_indices,
                "inverseBindMatrices": ibm_acc,
                "skeleton": node_base,
            }));
            // root mesh node = skeleton root
            scenes_meshes.push(json!({
                "name": mesh.name,
                "mesh": 0,
                "skin": 0,
                "translation": [0.0, 0.0, 0.0],
            }));
        } else {
            // static: single node
            scenes_meshes.push(json!({"name": mesh.name, "mesh": 0}));
        }

        // materials
        let mat_count = mesh.sections.iter().map(|s| s.material_index).max().unwrap_or(0) + 1;
        for mi in 0..mat_count.max(1) {
            let mut m = json!({"name": mesh.materials.get(mi as usize).unwrap_or(&format!("Material_{mi}")), "pbrMetallicRoughness": {"metallicFactor": 0.0, "roughnessFactor": 0.9}});
            if mi == 0 && scene.texture_name.is_some() {
                images.push(json!({"name": scene.texture_name.clone().unwrap(), "mimeType": "image/png"}));
                textures.push(json!({"sampler": 0, "source": images.len() - 1}));
                m["pbrMetallicRoughness"]["baseColorTexture"] = json!({"index": textures.len() - 1});
                m["doubleSided"] = json!(true);
            }
            materials.push(m);
        }
        // only handle first mesh in this v1 writer (multi-mesh merging kept simple)
        break;
    }

    // mesh primitives (first mesh)
    let mut mesh_json = Value::Null;
    for mesh in &scene.meshes {
        let vert_count = mesh.vertices.len();
        let mut primitives = Vec::new();
        for (si, section) in mesh.sections.iter().enumerate() {
            if section.indices.is_empty() {
                continue;
            }
            let use32 = vert_count > 65535;
            let idx_bytes: Vec<u8> = section
                .indices
                .iter()
                .flat_map(|i| if use32 { i.to_le_bytes().to_vec() } else { (*i as u16).to_le_bytes().to_vec() })
                .collect();
            let iv = add_view(&mut buf, &idx_bytes, 34963);
            accessors.push(json!({
                "bufferView": iv,
                "componentType": if use32 { 5125 } else { 5123 },
                "count": section.indices.len(),
                "type": "SCALAR"
            }));
            let idx_acc = accessors.len() - 1;
            primitives.push(json!({
                "attributes": {
                    "POSITION": 0,
                    "NORMAL": 1,
                    "TEXCOORD_0": 2,
                },
                "indices": idx_acc,
                "material": section.material_index.clamp(0, materials.len() as i32 - 1).max(0),
                "mode": 4,
                "_section": si
            }));
        }
        mesh_json = json!({"primitives": primitives, "name": mesh.name});
        break;
    }
    if mesh_json.is_null() {
        return Err(Error::asset("no mesh data to export"));
    }

    // animations
    let mut animations_json = Vec::new();
    for (ai, anim) in scene.animations.iter().enumerate() {
        let mut channels = Vec::new();
        let mut samplers_a = Vec::new();
        for (ti, track) in anim.tracks.iter().enumerate() {
            if track.rot_keys.is_empty() {
                continue;
            }
            // rotation sampler
            let rot: Vec<u8> = track
                .rot_keys
                .iter()
                .flat_map(|q| q.iter().flat_map(|f| f.to_le_bytes()))
                .collect();
            let rv = add_view(&mut buf, &rot, 34962);
            accessors.push(json!({"bufferView": rv, "componentType": 5126, "count": track.rot_keys.len(), "type": "VEC4"}));
            let rot_acc = accessors.len() - 1;
            let times: Vec<u8> = (0..track.rot_keys.len())
                .flat_map(|i| ((i as f32 / 30.0)).to_le_bytes())
                .collect();
            let tv = add_view(&mut buf, &times, 34962);
            accessors.push(json!({"bufferView": tv, "componentType": 5126, "count": track.rot_keys.len(), "type": "SCALAR"}));
            let time_acc = accessors.len() - 1;
            samplers_a.push(json!({"input": time_acc, "output": rot_acc, "interpolation": "LINEAR"}));
            channels.push(json!({"sampler": samplers_a.len() - 1, "target": {"node": ti, "path": "rotation"}}));

            if !track.pos_keys.is_empty() {
                let pos: Vec<u8> = track
                    .pos_keys
                    .iter()
                    .flat_map(|p| p.iter().flat_map(|f| f.to_le_bytes()))
                    .collect();
                let pv = add_view(&mut buf, &pos, 34962);
                accessors.push(json!({"bufferView": pv, "componentType": 5126, "count": track.pos_keys.len(), "type": "VEC3"}));
                let pos_acc = accessors.len() - 1;
                samplers_a.push(json!({"input": time_acc, "output": pos_acc, "interpolation": "LINEAR"}));
                channels.push(json!({"sampler": samplers_a.len() - 1, "target": {"node": ti, "path": "translation"}}));
            }
        }
        if !channels.is_empty() {
            animations_json.push(json!({"name": anim.name, "samplers": samplers_a, "channels": channels, "_index": ai}));
        }
    }
    let _ = &scenes_meshes;

    let mut gltf = json!({
        "asset": {"version": "2.0", "generator": "UExtractor 1.0"},
        "scene": 0,
        "scenes": [{"nodes": [if skins.is_empty() { json!(0) } else { json!({"children": scenes_meshes, "name": "Armature"}) }]}],
        "nodes": nodes,
        "meshes": [mesh_json],
        "materials": materials,
        "accessors": accessors,
        "bufferViews": buffer_views,
        "buffers": [{"byteLength": buf.len()}],
    });
    if let Some(root) = gltf.as_object_mut() {
        if !skins.is_empty() {
            root["skins"] = json!(skins);
        }
        if !animations_json.is_empty() {
            root["animations"] = json!(animations_json);
        }
        if !images.is_empty() {
            root["images"] = json!(images);
        }
        if !samplers.is_empty() {
            root["samplers"] = json!(samplers);
        }
        if !textures.is_empty() {
            root["textures"] = json!(textures);
        }
    }

    let json_bytes = serde_json::to_vec(&gltf)?;
    // GLB container
    let json_len = json_bytes.len();
    let json_pad = (4 - (json_len % 4)) % 4;
    let bin_pad = (4 - (buf.data.len() % 4)) % 4;
    let total = 12 + 8 + json_len + json_pad + 8 + buf.data.len() + bin_pad;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&0x46546C67u32.to_le_bytes()); // glTF
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&((json_len + json_pad) as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F534Au32.to_le_bytes()); // JSON
    out.extend_from_slice(&json_bytes);
    for _ in 0..json_pad {
        out.push(0x20);
    }
    out.extend_from_slice(&((buf.data.len() + bin_pad) as u32).to_le_bytes());
    out.extend_from_slice(&0x004E494Bu32.to_le_bytes()); // BIN
    out.extend_from_slice(&buf.data);
    for _ in 0..bin_pad {
        out.push(0);
    }
    Ok(out)
}

fn bind_matrix(mesh: &ExtractedMesh, bone_index: usize) -> [[f32; 4]; 4] {
    // inverse bind = inverse(world transform). v1: local transform inverse (good enough for
    // root-anchored skeletons produced by our extractor).
    let bone = mesh.bones.get(bone_index);
    let (pos, rot) = match bone {
        Some(b) => (b.position, b.rotation),
        None => ([0.0; 3], [0.0, 0.0, 0.0, 1.0]),
    };
    let m = quat_to_mat4(rot, pos);
    invert_mat4(m)
}

fn quat_to_mat4(q: [f32; 4], pos: [f32; 3]) -> [[f32; 4]; 4] {
    let [x, y, z, w] = q;
    let xx = x * x;
    let yy = y * y;
    let zz = z * z;
    let xy = x * y;
    let xz = x * z;
    let yz = y * z;
    let wx = w * x;
    let wy = w * y;
    let wz = w * z;
    [
        [1.0 - 2.0 * (yy + zz), 2.0 * (xy - wz), 2.0 * (xz + wy), pos[0]],
        [2.0 * (xy + wz), 1.0 - 2.0 * (xx + zz), 2.0 * (yz - wx), pos[1]],
        [2.0 * (xz - wy), 2.0 * (yz + wx), 1.0 - 2.0 * (xx + yy), pos[2]],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn invert_mat4(m: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    // Affine inverse: the matrix is orthonormal rotation + translation (no scale),
    // so inverse(R|t) = (R^T | -R^T t).
    let mut o = [[0f32; 4]; 4];
    for (i, row) in m.iter().enumerate().take(3) {
        for (j, col) in o.iter_mut().enumerate().take(3) {
            col[i] = row[j]; // transpose of 3x3 block
        }
    }
    // -R^T * t
    for (j, row) in o.iter_mut().enumerate().take(3) {
        row[3] = -(m[0][j] * m[0][3] + m[1][j] * m[1][3] + m[2][j] * m[2][3]);
    }
    o[3][3] = 1.0;
    o
}
