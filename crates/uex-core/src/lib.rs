//! UExtractor core: Unreal Engine + Unity archive parsing, decompression and
//! asset conversion.
//!
//! Supported containers:
//! - `.pak` UE4/UE5 package files, versions 1..=11 (path hash index, Fnv64BugFix)
//! - `.utoc`/`.ucas` IOStore containers (TOC v0..=8, partitions, OnDemand)
//! - AES-256 encrypted indices and payloads (ECB, per UE FAES3)
//! - Compression: Oodle (Kraken/Mermaid/Selkie/Leviathan), Zlib, Gzip, LZ4, Zstd, None
//! - Unity AssetBundles (UnityFS/UnityWeb/UnityRaw) incl. UnityCN encryption
//! - Unity SerializedFiles: .assets, level*, sharedassets*, globalgamemanagers,
//!   CAB-* inside bundles, streamed .resource/.resS data

pub mod assets;
pub mod compress;
pub mod crypto;
pub mod error;
pub mod export;
pub mod iostore;
pub mod io;
pub mod mount;
pub mod pak;
pub mod pipeline;
pub mod uasset;
pub mod unity;
pub use error::{Error, Result};

/// Unreal package file magic (classic `.uasset`).
pub const UASSET_MAGIC: u32 = 0x9E2A83C1;
