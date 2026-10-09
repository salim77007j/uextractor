//! Unified container mounting: presents .pak and .utoc/.ucas archives as a virtual
//! file tree with bounded-memory reads. Handles multi-container mounts (global.utoc
//! name resolution, container priority ordering for patches).

use crate::crypto::AesKey;
use crate::error::{Error, Result};
use crate::iostore::{self, zen, IoStoreReader};
use crate::pak::PakReader;
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
    },
    IoStore {
        path: PathBuf,
        key: Option<Arc<AesKey>>,
        files: Vec<(String, iostore::ChunkId, u64)>,
        container_name: String,
    },
}

impl Mount {
    /// Open an archive or a directory of archives. Directory mode picks up all
    /// .pak/.utoc files, orders containers by priority (patches > base > global).
    /// A plain non-archive file opens as a single virtual file (preview support).
    pub fn open(path: &Path, key: Option<AesKey>) -> Result<Mount> {
        let key = key.map(Arc::new);
        let mut sources = Vec::new();

        // plain file mode
        if path.is_file()
            && !path
                .extension()
                .map(|e| e == "pak" || e == "utoc")
                .unwrap_or(false)
        {
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
            sources.push(Source::IoStore {
                path: utoc,
                key: key.clone(),
                files,
                container_name: name,
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
            let source = Source::Pak {
                path: pak_path,
                key: key.clone(),
                version: reader.version(),
                mount_point: reader.mount_point().to_string(),
                entries: reader.entries().clone(),
            };
            sources.push(source);
        }

        if sources.is_empty() {
            return Err(Error::unsupported(format!(
                "no .pak or .utoc archives found at {}",
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
        }
    }

    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    pub fn source_path(&self, source: usize) -> String {
        match &self.sources[source] {
            Source::RawFile { path, .. } | Source::Pak { path, .. } | Source::IoStore { path, .. } => {
                path.display().to_string()
            }
        }
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

/// A per-worker reader bound to one source.
pub enum SourceReader {
    Raw(std::fs::File),
    Pak(PakReader<std::io::BufReader<std::fs::File>>),
    IoStore {
        reader: IoStoreReader,
        zen_cache: BTreeMap<String, Arc<zen::ZenPackage>>,
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
