//! Unreal Engine .pak parser — versions 1..=11 (UE4.0 .. UE5.x), byte-exact per FPakInfo/FPakEntry.

mod reader;
mod writer;

pub use reader::*;
pub use writer::*;

pub const MAGIC: u32 = 0x5A6F12E1;
pub const AES_BLOCK: u64 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Version {
    V0,
    V1,
    V2,
    V3,
    V4,
    V5,
    V6,
    V7,
    V8A,
    V8B,
    V9,
    V10,
    V11,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VersionMajor {
    Unknown = 0,
    Initial = 1,
    NoTimestamps = 2,
    CompressionEncryption = 3,
    IndexEncryption = 4,
    RelativeChunkOffsets = 5,
    DeleteRecords = 6,
    EncryptionKeyGuid = 7,
    FNameBasedCompression = 8,
    FrozenIndex = 9,
    PathHashIndex = 10,
    Fnv64BugFix = 11,
}

impl Version {
    pub const ALL: [Version; 13] = [
        Version::V0,
        Version::V1,
        Version::V2,
        Version::V3,
        Version::V4,
        Version::V5,
        Version::V6,
        Version::V7,
        Version::V8A,
        Version::V8B,
        Version::V9,
        Version::V10,
        Version::V11,
    ];

    pub const fn from_u32(v: u32) -> Option<Version> {
        Some(match v {
            0 => Version::V0,
            1 => Version::V1,
            2 => Version::V2,
            3 => Version::V3,
            4 => Version::V4,
            5 => Version::V5,
            6 => Version::V6,
            7 => Version::V7,
            8 => Version::V8A,
            9 => Version::V9,
            10 => Version::V10,
            11 => Version::V11,
            // some games write 8 twice (V8B stored as major 8); handled by caller heuristics
            _ => return None,
        })
    }

    pub const fn as_u32(self) -> u32 {
        match self {
            Version::V0 => 0,
            Version::V1 => 1,
            Version::V2 => 2,
            Version::V3 => 3,
            Version::V4 => 4,
            Version::V5 => 5,
            Version::V6 => 6,
            Version::V7 => 7,
            Version::V8A | Version::V8B => 8,
            Version::V9 => 9,
            Version::V10 => 10,
            Version::V11 => 11,
        }
    }

    pub const fn major(self) -> VersionMajor {
        match self {
            Version::V0 => VersionMajor::Unknown,
            Version::V1 => VersionMajor::Initial,
            Version::V2 => VersionMajor::NoTimestamps,
            Version::V3 => VersionMajor::CompressionEncryption,
            Version::V4 => VersionMajor::IndexEncryption,
            Version::V5 => VersionMajor::RelativeChunkOffsets,
            Version::V6 => VersionMajor::DeleteRecords,
            Version::V7 => VersionMajor::EncryptionKeyGuid,
            Version::V8A | Version::V8B => VersionMajor::FNameBasedCompression,
            Version::V9 => VersionMajor::FrozenIndex,
            Version::V10 => VersionMajor::PathHashIndex,
            Version::V11 => VersionMajor::Fnv64BugFix,
        }
    }

    /// Serialized footer size for this version.
    pub const fn footer_size(self) -> i64 {
        let mut size = 4 + 4 + 8 + 8 + 20; // magic, version, offset, size, hash
        if self.major() as u32 >= VersionMajor::EncryptionKeyGuid as u32 {
            size += 16; // encryption key guid
        }
        if self.major() as u32 >= VersionMajor::IndexEncryption as u32 {
            size += 1; // encrypted index flag
        }
        if self.major() as u32 == VersionMajor::FrozenIndex as u32 {
            size += 1; // frozen index flag
        }
        if self as u32 >= Version::V8A as u32 {
            size += 32 * 4; // compression names (4)
        }
        if self as u32 >= Version::V8B as u32 {
            size += 32; // 5th compression name
        }
        size
    }

    pub fn iter() -> impl Iterator<Item = Version> {
        Version::ALL.iter().copied().rev()
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_u32())
    }
}
