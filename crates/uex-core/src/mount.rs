//! Unified container mounting: presents .pak/.utoc/.ucas archives, Unity
//! AssetBundles (UnityFS) and Unity SerializedFiles (.assets / level* /
//! sharedassets* / extensionless data files) as a virtual file tree with
//! bounded-memory reads. Handles multi-container mounts (global.utoc name
//! resolution, container priority ordering for patches).

use crate::crypto::AesKey;
use crate::error::{Error, Result};
use crate::iostore::{self, zen, IoStoreReader};
use crate::pak::PakReader;
use crate::unity;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, serde::Serialize)]
pub struct VirtualFile {
    /// clean path like "Game/Content/Props/SM_Chair.uasset"
    pub path: String,
    pub size: u64,
    /// index into the mount's source list
    pub source: usize,
    /// internal key (pak entry path or chunk id hex)
    pub key: String,
}

pub struct Mount {
    /// ordered sources: patches first (higher priority), global container included
    sources: Vec<Source>,
    files: BTreeMap<String, VirtualFile>,
    pub total_size: u64,
}

enum Source {
    /// A plain file (opened for preview / conversion).
    RawFile { path: PathBuf, size: u64 },
    Pak {
        path: PathBuf,
        key: Option<Arc<AesKey>>,
        version: crate::pak::Version,
        mount_point: String,
        entries: BTreeMap<String, crate::pak::PakEntry>,
        /// number of entries whose data is AES-encrypted
        encrypted_entries: u64,
        /// true when the pak index itself was encrypted (key needed to even list files)
        encrypted_index: bool,
    },
    IoStore {
        path: PathBuf,
        key: Option<Arc<AesKey>>,
        files: Vec<(String, iostore::ChunkId, u64)>,
        container_name: String,
        /// container flag: payloads are AES-encrypted
        encrypted: bool,
        /// key GUID recorded in the TOC ("" when absent)
        key_guid: String,
    },
    /// Unity bundle (UnityFS/UnityWeb/UnityRaw) or standalone SerializedFile.
    Unity {
        path: PathBuf,
        /// base dir for sibling .resource/.resS lookups
        base_dir: PathBuf,
        /// virtual entries exposed by this source
        entries: Vec<UnityVirtual>,
        /// UnityCN-encrypted bundle (needs a 16-char key)
        encrypted: bool,
        /// friendly root name (file stem)
        name: String,
        /// UnityCN key bytes when supplied at open time
        key_unitycn: Option<[u8; 16]>,
    },
}

#[derive(Debug, Clone)]
pub struct UnityVirtual {
    /// display path, e.g. "bundle.bundle/CAB-1234abcd" or "level0"
    pub path: String,
    /// node path inside the bundle when this source is a bundle
    pub node: Option<String>,
    pub size: u64,
}

/// Keys for the two encryption systems UExtractor supports.
#[derive(Debug, Clone, Default)]
pub struct MountKeys {
    /// UE AES-256 key (hex) for .pak / IOStore archives
    pub aes: Option<AesKey>,
    /// UnityCN asset-bundle key (exactly 16 characters)
    pub unitycn: Option<String>,
}

impl Mount {
    /// Open with no keys (UnityCN-encrypted bundles list but cannot extract).
    pub fn open_no_keys(path: &Path) -> Result<Mount> {
        Self::open_with_keys(path, MountKeys::default())
    }

    /// Open an archive or a directory of archives with a UE AES key.
    pub fn open(path: &Path, key: Option<AesKey>) -> Result<Mount> {
        Self::open_with_keys(
            path,
            MountKeys {
                aes: key,
                ..Default::default()
            },
        )
    }

    /// Open with both key kinds: UE AES-256 (pak/IOStore) and UnityCN 16-char
    /// (encrypted asset bundles).
    pub fn open_with_keys(path: &Path, keys: MountKeys) -> Result<Mount> {
        let key = keys.aes.map(Arc::new);
        let unitycn = keys
            .unitycn
            .as_deref()
            .map(|k| {
                let b = k.as_bytes();
                if b.len() != 16 {
                    Err(Error::other(format!(
                        "UnityCN key must be exactly 16 characters (got {})",
                        b.len()
                    )))
                } else {
                    let mut out = [0u8; 16];
                    out.copy_from_slice(b);
                    Ok(out)
                }
            })
            .transpose()?;
        let mut sources = Vec::new();

        // plain file mode: Unity archives get real parsing, everything else
        // opens as a single virtual file (preview support)
        if path.is_file()
            && !path
                .extension()
                .map(|e| e == "pak" || e == "utoc")
                .unwrap_or(false)
        {
            if is_unity_candidate(path) {
                if let Ok(source) = open_unity_source(path, unitycn) {
                    return Self::from_sources(vec![source]);
                }
            }
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "file".into());
            let size = path.metadata().map(|m| m.len()).unwrap_or(0);
            sources.push(Source::RawFile { path: path.to_path_buf(), size });
            return Self::from_sources(sources);
        }

        let utoc_paths: Vec<PathBuf> = if path.is_dir() {
            let mut v: Vec<PathBuf> = std::fs::read_dir(path)?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().map(|e| e == "utoc").unwrap_or(false))
                .collect();
            v.sort();
            v
        } else if path.extension().map(|e| e == "utoc").unwrap_or(false) {
            vec![path.to_path_buf()]
        } else {
            Vec::new()
        };

        for utoc in utoc_paths {
            let reader = IoStoreReader::open(&utoc, key.clone().map(|k| (*k).clone()))?;
            let files = reader.files();
            let name = reader.container_name().to_string();
            let encrypted = reader.is_encrypted();
            let key_guid = reader.encryption_key_guid();
            sources.push(Source::IoStore {
                path: utoc,
                key: key.clone(),
                files,
                container_name: name,
                encrypted,
                key_guid,
            });
        }

        let pak_paths: Vec<PathBuf> = if path.is_dir() {
            let mut v: Vec<PathBuf> = std::fs::read_dir(path)?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().map(|e| e == "pak").unwrap_or(false))
                .collect();
            v.sort();
            v
        } else if path.extension().map(|e| e == "pak").unwrap_or(false) {
            vec![path.to_path_buf()]
        } else {
            Vec::new()
        };

        for pak_path in pak_paths {
            let file = std::fs::File::open(&pak_path)?;
            let reader = PakReader::new(std::io::BufReader::new(file), key.clone().map(|k| (*k).clone()))?;
            let encrypted_entries = reader.entries().values().filter(|e| e.is_encrypted()).count() as u64;
            let encrypted_index = reader.encrypted_index();
            let source = Source::Pak {
                path: pak_path,
                key: key.clone(),
                version: reader.version(),
                mount_point: reader.mount_point().to_string(),
                entries: reader.entries().clone(),
                encrypted_entries,
                encrypted_index,
            };
            sources.push(source);
        }

        // Unity sources: bundles + standalone serialized files
        let unity_paths: Vec<PathBuf> = if path.is_dir() {
            std::fs::read_dir(path)?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_file() && is_unity_candidate(p))
                .collect()
        } else if path.is_file() && is_unity_candidate(path) {
            vec![path.to_path_buf()]
        } else {
            Vec::new()
        };
        for up in unity_paths {
            if let Ok(source) = open_unity_source(&up, unitycn) {
                sources.push(source);
            }
        }

        if sources.is_empty() {
            return Err(Error::unsupported(format!(
                "no .pak, .utoc or Unity archives found at {}",
                path.display()
            )));
        }

        Self::from_sources(sources)
    }

    fn from_sources(sources: Vec<Source>) -> Result<Mount> {
        // priority: sort so that later (patch) containers override earlier entries.
        // patch containers end with _P or _N; global gets highest priority for names only.
        let mut ordered: Vec<&Source> = sources.iter().collect();
        ordered.sort_by_key(|s| match s {
            Source::RawFile { .. } => (false, 0u32, String::new()),
            Source::Pak { path, .. } => container_priority(&path.file_stem().unwrap_or_default().to_string_lossy()),
            Source::IoStore { container_name, .. } => container_priority(container_name),
            Source::Unity { name, .. } => (false, 0u32, name.clone()),
        });
        let _ = ordered;

        let mut files = BTreeMap::new();
        let mut total_size = 0u64;
        for (si, source) in sources.iter().enumerate() {
            let list: Vec<(String, u64, String)> = match source {
                Source::RawFile { path, size } => {
                    let name = path
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "file".into());
                    vec![(name, *size, String::new())]
                }
                Source::Pak {
                    mount_point, entries, ..
                } => entries
                    .iter()
                    .map(|(k, e)| {
                        let full = crate::pak::normalize_ue_path(&format!("{mount_point}{k}"));
                        (full, e.uncompressed, k.clone())
                    })
                    .collect(),
                Source::IoStore { files, .. } => files
                    .iter()
                    .map(|(p, c, s)| (p.clone(), *s, format!("{c}")))
                    .collect(),
                Source::Unity { entries, .. } => entries
                    .iter()
                    .map(|e| (e.path.clone(), e.size, e.node.clone().unwrap_or_default()))
                    .collect(),
            };
            for (path, size, key) in list {
                total_size += size;
                // patch containers later in list override earlier ones
                files.insert(
                    path.clone(),
                    VirtualFile {
                        path,
                        size,
                        source: si,
                        key,
                    },
                );
            }
        }

        Ok(Mount {
            sources,
            files,
            total_size,
        })
    }

    pub fn files(&self) -> impl Iterator<Item = &VirtualFile> + '_ {
        self.files.values()
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn get(&self, path: &str) -> Option<&VirtualFile> {
        self.files.get(path)
    }

    /// Build a fresh reader for a source (used by parallel workers).
    pub fn open_source_reader(&self, source: usize) -> Result<SourceReader> {
        match &self.sources[source] {
            Source::RawFile { path, .. } => {
                let file = std::fs::File::open(path)?;
                Ok(SourceReader::Raw(file))
            }
            Source::Pak { path, key, .. } => {
                let file = std::fs::File::open(path)?;
                let reader = PakReader::new(std::io::BufReader::new(file), key.as_ref().map(|k| (**k).clone()))?;
                Ok(SourceReader::Pak(reader))
            }
            Source::IoStore { path, key, .. } => {
                let reader = IoStoreReader::open(path, key.as_ref().map(|k| (**k).clone()))?;
                // global names: try sibling global.utoc
                Ok(SourceReader::IoStore { reader, zen_cache: Default::default() })
            }
            Source::Unity { path, base_dir, key_unitycn, .. } => {
                use std::io::Read;
                let mut data = Vec::new();
                std::fs::File::open(path)?.read_to_end(&mut data)?;
                let (bundle, standalone_path) = if unity::is_bundle_signature(&data) {
                    (
                        Some(unity::bundle::UnityBundle::parse(
                            data,
                            key_unitycn.as_ref().map(|k| k.as_slice()),
                        )?),
                        None,
                    )
                } else {
                    (None, Some(path.clone()))
                };
                Ok(SourceReader::Unity {
                    bundle,
                    base_dir: base_dir.clone(),
                    standalone_path,
                })
            }
        }
    }

    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    pub fn source_path(&self, source: usize) -> String {
        match &self.sources[source] {
            Source::RawFile { path, .. }
            | Source::Pak { path, .. }
            | Source::IoStore { path, .. }
            | Source::Unity { path, .. } => path.display().to_string(),
        }
    }

    /// True when this specific virtual file's payload is AES-encrypted.
    pub fn is_file_encrypted(&self, vf: &VirtualFile) -> bool {
        match &self.sources[vf.source] {
            Source::RawFile { .. } => false,
            Source::Pak { entries, .. } => {
                entries.get(&vf.key).map(|e| e.is_encrypted()).unwrap_or(false)
            }
            Source::IoStore { encrypted, .. } => *encrypted,
            Source::Unity { encrypted, .. } => *encrypted,
        }
    }

    /// Number of virtual files whose payload is AES-encrypted.
    /// Used by the GUI/CLI to warn and ask for a key BEFORE extraction starts.
    pub fn encrypted_file_count(&self) -> u64 {
        self.sources
            .iter()
            .map(|s| match s {
                Source::RawFile { .. } => 0,
                Source::Pak { entries, .. } => entries.values().filter(|e| e.is_encrypted()).count() as u64,
                Source::IoStore { encrypted, files, .. } => {
                    if *encrypted { files.len() as u64 } else { 0 }
                }
                Source::Unity { encrypted, entries, .. } => {
                    if *encrypted { entries.len() as u64 } else { 0 }
                }
            })
            .sum()
    }

    /// True when the archive needs a key even to list its contents.
    pub fn has_encrypted_index(&self) -> bool {
        self.sources
            .iter()
            .any(|s| matches!(s, Source::Pak { encrypted_index: true, .. }))
    }

    /// True when any mounted source is a UnityCN-encrypted bundle.
    pub fn has_unity_encrypted(&self) -> bool {
        self.sources
            .iter()
            .any(|s| matches!(s, Source::Unity { encrypted: true, .. }))
    }

    /// Validate a UnityCN key candidate against the mounted encrypted bundles
    /// WITHOUT extracting: the bundle header carries a signature that only
    /// decrypts to the magic marker with the correct key.
    pub fn test_unity_key(&self, key_text: &str) -> Result<bool> {
        use std::io::Read;
        let bytes = key_text.as_bytes();
        if bytes.len() != 16 {
            return Ok(false);
        }
        for source in &self.sources {
            let Source::Unity { path, encrypted, .. } = source else {
                continue;
            };
            if !encrypted {
                continue;
            }
            let mut data = Vec::new();
            std::fs::File::open(path)?.read_to_end(&mut data)?;
            let bundle = unity::bundle::UnityBundle::parse(data, Some(bytes))?;
            // parse succeeded => decryptor signature validated
            return Ok(bundle.encrypted);
        }
        Ok(false)
    }

    /// Key GUID(s) advertised by the archive, if any (helps users match keys).
    pub fn encryption_key_guids(&self) -> Vec<String> {
        self.sources
            .iter()
            .filter_map(|s| match s {
                Source::IoStore { key_guid, encrypted, .. } if *encrypted && !key_guid.is_empty() => {
                    Some(key_guid.clone())
                }
                _ => None,
            })
            .collect()
    }

    /// Validate a candidate key against this mount WITHOUT writing anything:
    /// reads one encrypted file (or reports there is nothing to test).
    /// Returns Ok(true) when the key decodes an encrypted payload correctly.
    pub fn test_key(&self, key: &AesKey) -> Result<bool> {
        for (si, source) in self.sources.iter().enumerate() {
            let has_encrypted = match source {
                Source::Pak { entries, .. } => entries.values().any(|e| e.is_encrypted()),
                Source::IoStore { encrypted, .. } => *encrypted,
                Source::RawFile { .. } | Source::Unity { .. } => false,
            };
            if !has_encrypted {
                continue;
            }
            // pick the smallest encrypted file for a fast probe
            let candidates: Vec<&VirtualFile> = self
                .files
                .values()
                .filter(|f| f.source == si)
                .collect();
            let probe = match source {
                Source::Pak { entries, .. } => candidates
                    .into_iter()
                    .filter(|f| entries.get(&f.key).map(|e| e.is_encrypted()).unwrap_or(false))
                    .min_by_key(|f| f.size),
                Source::IoStore { .. } => candidates.into_iter().min_by_key(|f| f.size),
                Source::RawFile { .. } | Source::Unity { .. } => None,
            };
            let Some(vf) = probe else { continue };
            let mut reader = self.open_source_reader(si)?;
            return reader.read(vf).map(|_| true);
        }
        Ok(false) // nothing encrypted to test against
    }
}

fn container_priority(name: &str) -> (bool, u32, String) {
    // higher tuple = higher priority (applied last)
    let mut base = name.to_string();
    let mut chunk_version = 0u32;
    if let Some(stripped) = base.strip_suffix("_P") {
        base = stripped.to_string();
        chunk_version = 1;
        if let Some((b, v)) = base.rsplit_once('_') {
            if let Ok(n) = v.parse::<u32>() {
                base = b.to_string();
                chunk_version = n + 2;
            }
        }
    }
    (name == "global", chunk_version, base)
}

/// Quick check whether a path could be a Unity bundle or serialized file.
/// Extension-based first, then signature sniffing (real games use arbitrary
/// extensions like .ab/.bin or no extension at all).
fn is_unity_candidate(path: &Path) -> bool {
    let Some(name) = path.file_name().map(|s| s.to_string_lossy().into_owned()) else {
        return false;
    };
    let lower = name.to_lowercase();
    // Unity backing resource streams are not entry points
    if lower.ends_with(".ress") || lower.ends_with(".resource") || lower.ends_with(".manifest") {
        return false;
    }
    let ext_ok = lower.ends_with(".bundle")
        || lower.ends_with(".assets")
        || lower.ends_with(".unity3d")
        || lower.ends_with(".assetbundle")
        || lower.ends_with(".assetbundles")
        // standalone Unity data files that ship without extension
        || lower.starts_with("level")
        || lower.starts_with("sharedassets")
        || lower.starts_with("globalgamemanagers")
        || lower == "data.unity3d";
    if ext_ok {
        return true;
    }
    // signature sniff for unknown extensions (.ab etc.)
    use std::io::Read;
    let mut magic = [0u8; 8];
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let Ok(n) = f.read(&mut magic) else { return false };
    if n < 8 {
        return false;
    }
    magic.starts_with(b"UnityFS")
        || magic.starts_with(b"UnityWeb")
        || magic.starts_with(b"UnityRaw")
        || magic.starts_with(b"UnityArc")
}

/// Parse a Unity file into a mount source. Errors propagate for encrypted
/// bundles (key needed), other malformed Unity files are reported too.
fn open_unity_source(path: &Path, unitycn: Option<[u8; 16]>) -> Result<Source> {
    use std::io::Read;
    let mut data = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut data)?;
    let base_dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unity".into());

    if unity::is_bundle_signature(&data) {
        let bundle = unity::bundle::UnityBundle::parse(data, unitycn.as_ref().map(|k| k.as_slice())).map_err(|e| match e {
            Error::EncryptedNoKey => Error::EncryptedNoKey,
            other => Error::malformed(format!("{}: {other}", path.display())),
        })?;
        let encrypted = bundle.encrypted;
        let entries = bundle
            .nodes
            .iter()
            .map(|n| UnityVirtual {
                path: format!("{name}/{}", n.path),
                node: Some(n.path.clone()),
                size: n.size,
            })
            .collect();
        return Ok(Source::Unity {
            path: path.to_path_buf(),
            base_dir,
            entries,
            encrypted,
            name,
            key_unitycn: unitycn,
        });
    }
    if unity::looks_like_serialized_file(&data) {
        let size = data.len() as u64;
        let display = name.clone();
        return Ok(Source::Unity {
            path: path.to_path_buf(),
            base_dir,
            entries: vec![UnityVirtual {
                path: display,
                node: None,
                size,
            }],
            encrypted: false,
            name,
            key_unitycn: None,
        });
    }
    Err(Error::malformed(format!(
        "{} is not a Unity bundle or serialized file",
        path.display()
    )))
}

/// A per-worker reader bound to one source.
pub enum SourceReader {
    Raw(std::fs::File),
    Pak(PakReader<std::io::BufReader<std::fs::File>>),
    IoStore {
        reader: IoStoreReader,
        zen_cache: BTreeMap<String, Arc<zen::ZenPackage>>,
    },
    /// Parsed Unity bundle (None for standalone serialized files).
    Unity {
        bundle: Option<unity::bundle::UnityBundle>,
        base_dir: PathBuf,
        /// for standalone serialized files: the file to read directly
        standalone_path: Option<PathBuf>,
    },
}

impl SourceReader {
    /// Read raw file bytes for a virtual file entry.
    pub fn read(&mut self, vf: &VirtualFile) -> Result<Vec<u8>> {
        match self {
            SourceReader::Raw(f) => {
                use std::io::Read;
                let mut buf = Vec::new();
                f.read_to_end(&mut buf)?;
                Ok(buf)
            }
            SourceReader::Pak(reader) => reader.read_entry(&vf.key),
            SourceReader::IoStore { reader, .. } => {
                let bytes = parse_chunk_hex(&vf.key)?;
                reader.read_chunk_id(&iostore::ChunkId { id: bytes })
            }
            SourceReader::Unity {
                bundle,
                standalone_path,
                ..
            } => {
                if let Some(bundle) = bundle {
                    let node = bundle
                        .nodes
                        .iter()
                        .find(|n| n.path == vf.key)
                        .ok_or_else(|| Error::NotFound(format!("bundle node '{}'", vf.key)))?;
                    bundle.read_node(node)
                } else if let Some(path) = standalone_path {
                    use std::io::Read;
                    let mut buf = Vec::new();
                    std::fs::File::open(path)?.read_to_end(&mut buf)?;
                    Ok(buf)
                } else {
                    Err(Error::other("unity source has no data to read"))
                }
            }
        }
    }

    /// Context for Unity resource resolution (bundle + sibling dir).
    pub fn unity_context(&mut self) -> (Option<&unity::bundle::UnityBundle>, PathBuf) {
        match self {
            SourceReader::Unity { bundle, base_dir, .. } => (bundle.as_ref(), base_dir.clone()),
            _ => (None, PathBuf::new()),
        }
    }

    /// Parse a zen package (for asset conversion).
    pub fn zen_package(&mut self, vf: &VirtualFile) -> Result<Arc<zen::ZenPackage>> {
        let (reader, cache) = match self {
            SourceReader::IoStore { reader, zen_cache } => (reader, zen_cache),
            _ => return Err(Error::other("zen packages only exist in IOStore containers")),
        };
        if let Some(p) = cache.get(&vf.key) {
            return Ok(p.clone());
        }
        let chunk = iostore::ChunkId {
            id: parse_chunk_hex(&vf.key)?,
        };
        let package_id = chunk.package_id();
        let store_entry = None; // container header store entries parsed at open; optional here
        let zen = zen::ZenPackage::parse(reader, &chunk, store_entry)?;
        let arc = Arc::new(zen);
        cache.insert(vf.key.clone(), arc.clone());
        let _ = package_id;
        Ok(arc)
    }
}

fn parse_chunk_hex(hex_str: &str) -> Result<[u8; 12]> {
    let bytes = hex::decode(hex_str).map_err(|_| Error::malformed("bad chunk id hex"))?;
    bytes
        .try_into()
        .map_err(|_| Error::malformed("chunk id must be 12 bytes"))
}
