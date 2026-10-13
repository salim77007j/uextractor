//! Unity Engine archive parsing: AssetBundles (UnityFS/UnityWeb/UnityRaw),
//! SerializedFiles (.assets, level*, sharedassets*, globalgamemanagers, CAB-*
//! inside bundles), streamed .resource/.resS data, and object conversion
//! (Texture2D, AudioClip, Mesh, Sprite, TextAsset, Material, MonoBehaviour).
//!
//! Field layouts for stripped typetrees come from an embedded type-tree
//! database (AssetRipper TPK, release nodes) covering Unity 3.4 → 6000.x,
//! so no per-version hand-written field layouts are needed.

pub mod bundle;
pub mod objects;
pub mod serialized;
pub mod tpk;
pub mod typetree;

use crate::error::{Error, Result};

/// Unity object class ids (subset used by extraction).
pub mod class_id {
    pub const GAME_OBJECT: i32 = 1;
    pub const TRANSFORM: i32 = 4;
    pub const MATERIAL: i32 = 21;
    pub const TEXTURE_2D: i32 = 28;
    pub const MESH: i32 = 43;
    pub const SHADER: i32 = 48;
    pub const TEXT_ASSET: i32 = 49;
    pub const AUDIO_CLIP: i32 = 83;
    pub const MONOBEHAVIOUR: i32 = 114;
    pub const ASSET_BUNDLE: i32 = 142;
    pub const SPRITE_ATLAS: i32 = 162;
    pub const SPRITE: i32 = 213;
    pub const VIDEO_CLIP: i32 = 329;
}

pub fn class_name(id: i32) -> String {
    // Names for classes we special-case or that are common; numbers otherwise.
    let name = match id {
        1 => "GameObject",
        4 => "Transform",
        14 => "Material",
        15 => "Mesh",
        18 => "Animation",
        19 => "AnimationClip",
        21 => "Material",
        23 => "MeshFilter",
        28 => "Texture2D",
        29 => "Cubemap",
        31 => "Shader",
        33 => "Cubemap",
        34 => "AudioClip",
        41 => "AudioMixer",
        43 => "Mesh",
        48 => "Shader",
        49 => "TextAsset",
        54 => "Rigidbody",
        61 => "BoxCollider2D",
        63 => "AudioClip",
        65 => "BoxCollider",
        72 => "ComputeShader",
        74 => "AnimationClip",
        81 => "AudioListener",
        82 => "AudioSource",
        83 => "AudioClip",
        84 => "RenderTexture",
        89 => "Cubemap",
        90 => "Avatar",
        91 => "AnimatorController",
        92 => "RuntimeAnimatorController",
        93 => "Animator",
        95 => "Animator",
        96 => "TrailRenderer",
        102 => "TextMesh",
        104 => "RenderSettings",
        108 => "Light",
        110 => "AnimationClip",
        111 => "Animation",
        113 => "MonoScript",
        114 => "MonoBehaviour",
        117 => "Texture3D",
        118 => "Sprite",
        120 => "SpriteRenderer",
        121 => "Sprite",
        125 => "PhysicMaterial",
        128 => "Font",
        129 => "PlayerSettings",
        133 => "RuntimeAnimatorController",
        135 => "SphereCollider",
        136 => "CapsuleCollider",
        137 => "SkinnedMeshRenderer",
        142 => "AssetBundle",
        146 => "OcclusionCullingSettings",
        150 => "PreloadData",
        152 => "MovieTexture",
        155 => "AnimatorOverrideController",
        157 => "LightmapSettings",
        158 => "QualitySettings",
        159 => "Avatar",
        162 => "SpriteAtlas",
        164 => "AudioMixerSnapshot",
        168 => "AudioMixerGroup",
        171 => "PhysicsSettings",
        174 => "TimeManager",
        177 => "SpriteAtlas",
        185 => "ProceduralMaterial",
        186 => "ProceduralTexture",
        195 => "AssetBundleManifest",
        196 => "UnityConnectSettings",
        202 => "GraphicsSettings",
        204 => "AudioManager",
        209 => "AudioMixerGroup",
        210 => "TerrainCollider",
        213 => "Sprite",
        224 => "RectTransform",
        225 => "NavMeshAgent",
        226 => "NavMeshSettings",
        235 => "CanvasRenderer",
        249 => "Texture2D",
        251 => "VideoClip",
        258 => "LightingSettings",
        290 => "VideoPlayer",
        329 => "VideoClip",
        330 => "VideoClip",
        _ => "",
    };
    if name.is_empty() {
        format!("Class{id}")
    } else {
        name.to_string()
    }
}

/// Packed Unity engine version (AssetRipper VersionUtilities layout):
/// major<<48 | minor<<32 | build<<16 | type<<8 | type_number.
/// type: 0=a, 1=b, 2=c, 3=f, 4=p, 5=x, 255=unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnityVersion(pub u64);

impl UnityVersion {
    pub fn from_list(major: u16, minor: u16, build: u16, vtype: u8, type_number: u8) -> Self {
        UnityVersion(
            (major as u64) << 48
                | (minor as u64) << 32
                | (build as u64) << 16
                | (vtype as u64) << 8
                | type_number as u64,
        )
    }

    /// Parse "2021.3.16f1", "5.6.7", "6000.0.32f1" etc. Falls back to
    /// major.minor.build with type=f when the suffix can't be parsed.
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        let mut it = s.split('.');
        let major = it.next().and_then(|v| v.parse::<u16>().ok()).unwrap_or(0);
        let minor = it.next().and_then(|v| v.parse::<u16>().ok()).unwrap_or(0);
        let rest = it.next().unwrap_or("0");
        // split build number from type suffix: "16f1" -> build=16, type=f, number=1
        let build_end = rest
            .find(|c: char| c.is_ascii_alphabetic())
            .unwrap_or(rest.len());
        let build = rest[..build_end].parse::<u16>().unwrap_or(0);
        let suffix = &rest[build_end..];
        let (vtype, tnum) = if suffix.is_empty() {
            (3u8, 0u8)
        } else {
            let type_ch = suffix.as_bytes()[0];
            let vtype = match type_ch {
                b'a' => 0u8,
                b'b' => 1,
                b'c' => 2,
                b'f' => 3,
                b'p' => 4,
                b'x' => 5,
                _ => 255,
            };
            let num: u8 = suffix[1..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0);
            (vtype, num)
        };
        if major == 0 && minor == 0 && build == 0 {
            return Err(Error::malformed(format!("invalid unity version: {s}")));
        }
        Ok(Self::from_list(major, minor, build, vtype, tnum))
    }

    pub fn major(&self) -> u16 {
        ((self.0 >> 48) & 0xFFFF) as u16
    }
    pub fn minor(&self) -> u16 {
        ((self.0 >> 32) & 0xFFFF) as u16
    }
    pub fn build(&self) -> u16 {
        ((self.0 >> 16) & 0xFFFF) as u16
    }
    pub fn vtype(&self) -> u8 {
        ((self.0 >> 8) & 0xFF) as u8
    }
    pub fn type_number(&self) -> u8 {
        (self.0 & 0xFF) as u8
    }

    pub fn as_tuple(&self) -> (u16, u16, u16, u8, u8) {
        (
            self.major(),
            self.minor(),
            self.build(),
            self.vtype(),
            self.type_number(),
        )
    }

    pub fn as_string(&self) -> String {
        let t = match self.vtype() {
            0 => "a",
            1 => "b",
            2 => "c",
            3 => "f",
            4 => "p",
            5 => "x",
            _ => "u",
        };
        format!(
            "{}.{}.{}{}{}",
            self.major(),
            self.minor(),
            self.build(),
            t,
            self.type_number()
        )
    }
}

/// Detect a UnityFS/UnityWeb/UnityRaw bundle by signature.
pub fn is_bundle_signature(data: &[u8]) -> bool {
    for sig in ["UnityFS\0", "UnityWeb\0", "UnityRaw\0"] {
        if data.starts_with(sig.as_bytes()) {
            return true;
        }
    }
    // UnityArchive (rare legacy) is bundle-ish too; detected but unsupported.
    data.starts_with(b"UnityArchive\0")
}

/// Heuristic detection of a Unity SerializedFile. There is no magic: verify
/// the header fields are internally consistent (the standard approach used
/// by AssetStudio/UnityPy for level0/sharedassets0/extensionless files).
/// NOTE: the header ints are BIG-endian in real Unity files (both AssetStudio
/// and UnityPy read them BE); the LE reading is kept as a fallback.
pub fn looks_like_serialized_file(data: &[u8]) -> bool {
    fn check(data: &[u8], be: bool) -> bool {
        let rd = |off: usize| -> u32 {
            let b = [data[off], data[off + 1], data[off + 2], data[off + 3]];
            if be {
                u32::from_be_bytes(b)
            } else {
                u32::from_le_bytes(b)
            }
        };
        let metadata_size = rd(0);
        let file_size = rd(4);
        let version = rd(8);
        let data_offset = rd(12);
        if version < 3 || version > 30 {
            return false;
        }
        if file_size as usize != data.len() {
            // header claims an exact size; a mismatch means this is not a serialized file
            return false;
        }
        if version >= 9 {
            let min_header = if version >= 22 { 60u32 } else { 32 };
            if data_offset < min_header || data_offset as usize > data.len() {
                return false;
            }
            if metadata_size == 0 || metadata_size as usize > data.len() {
                return false;
            }
            true
        } else {
            // v<9: metadata is stored at the end of the file
            metadata_size > 0 && (metadata_size as usize) < data.len()
        }
    }
    if data.len() < 20 {
        return false;
    }
    check(data, true) || check(data, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parse() {
        assert_eq!(
            UnityVersion::parse("2021.3.16f1").unwrap().as_string(),
            "2021.3.16f1"
        );
        assert_eq!(UnityVersion::parse("5.6.7f1").unwrap().as_tuple(), (5, 6, 7, 3, 1));
        assert_eq!(UnityVersion::parse("6000.0.32f1").unwrap().major(), 6000);
        assert_eq!(
            UnityVersion::parse("2019.4.40f1").unwrap().as_tuple(),
            (2019, 4, 40, 3, 1)
        );
        assert_eq!(UnityVersion::parse("3.4.0").unwrap().as_tuple(), (3, 4, 0, 3, 0));
    }

    #[test]
    fn version_ordering() {
        let a = UnityVersion::parse("2017.1.0f1").unwrap();
        let b = UnityVersion::parse("2020.3.34f1").unwrap();
        assert!(a < b);
        let c = UnityVersion::parse("2020.3.34f1").unwrap();
        assert_eq!(b, c);
    }

    #[test]
    fn serialized_detect() {
        assert!(!looks_like_serialized_file(&[0u8; 64]));
        assert!(is_bundle_signature(b"UnityFS\0\0\0\x06"));
        assert!(!is_bundle_signature(b"PK\x03\x04"));
    }
}
