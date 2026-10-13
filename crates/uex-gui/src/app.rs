//! Application state and business logic. Rendering lives in `ui.rs`.

use crossbeam_channel::Receiver;
use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use uex_core::crypto::AesKey;
pub use uex_core::mount::VirtualFile;
use uex_core::mount::Mount;
use uex_core::pipeline::ProgressEvent;

use crate::theme::ic;

// ---------------------------------------------------------------------------
// persistence (same schema as v1.0 — existing configs keep working)

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Persisted {
    recent: Vec<String>,
    keys: Vec<KeyEntry>,
    active_key: Option<String>,
    export_dir: Option<String>,
    convert: bool,
    formats: ExportFormats,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct KeyEntry {
    pub name: String,
    pub hex: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ExportFormats {
    pub png: bool,
    pub dds: bool,
    pub glb: bool,
    pub obj: bool,
    pub json: bool,
    pub audio: bool,
}

impl Default for ExportFormats {
    fn default() -> Self {
        Self { png: true, dds: true, glb: true, obj: false, json: true, audio: true }
    }
}

fn config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("org", "uextractor", "UExtractor")
        .map(|d| d.config_dir().join("config.json"))
}

fn load_persisted() -> Persisted {
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// tree model

pub struct TreeNode {
    pub name: String,
    pub full_path: String,
    pub file: Option<VirtualFile>,
    pub children: Vec<TreeNode>,
}

fn build_tree(files: Vec<VirtualFile>) -> Vec<TreeNode> {
    #[derive(Default)]
    struct Node {
        children: std::collections::BTreeMap<String, Node>,
        file: Option<VirtualFile>,
    }
    let mut root = Node::default();
    for f in files {
        let parts: Vec<&str> = f.path.split('/').filter(|p| !p.is_empty()).collect();
        let mut cur = &mut root;
        for (i, part) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                cur.children.entry(part.to_string()).or_default().file = Some(f.clone());
            } else {
                cur = cur.children.entry(part.to_string()).or_default();
            }
        }
    }
    fn convert(node: &Node, prefix: &str, name: &str) -> TreeNode {
        let full_path = if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") };
        TreeNode {
            name: name.to_string(),
            full_path: full_path.clone(),
            file: node.file.clone(),
            children: node
                .children
                .iter()
                .map(|(n, c)| convert(c, &full_path, n))
                .collect(),
        }
    }
    root.children.iter().map(|(n, c)| convert(c, "", n)).collect()
}

/// Keep only nodes whose own name or any descendant matches the filter.
fn filter_tree(nodes: &mut Vec<TreeNode>, flt: &str) {
    nodes.retain_mut(|n| {
        if n.file.is_some() {
            return n.name.to_lowercase().contains(flt);
        }
        filter_tree(&mut n.children, flt);
        !n.children.is_empty()
    });
}

/// Public wrapper used by the UI when rendering a filtered tree.
pub fn filter_tree_pub(nodes: &mut Vec<TreeNode>, flt: &str) {
    filter_tree(nodes, flt);
}

// ---------------------------------------------------------------------------
// preview

pub enum Preview {
    None,
    Texture(egui::TextureHandle),
    Mesh { mesh: uex_core::assets::mesh::ExtractedMesh, rot_x: f32, rot_y: f32, zoom: f32 },
    Text(String),
    Unsupported(String),
}

#[derive(Clone, Copy, PartialEq)]
pub enum ViewMode {
    List,
    Grid,
}

#[derive(Clone, Copy, PartialEq)]
pub enum SortCol {
    Name,
    Type,
    Size,
}

// ---------------------------------------------------------------------------
// status + key prompt

#[derive(Clone, Copy, PartialEq)]
pub enum StatusKind {
    Info,
    Ok,
    Warn,
    Err,
    Busy,
}

pub struct Status {
    pub kind: StatusKind,
    pub text: String,
}

impl Status {
    pub fn new(kind: StatusKind, text: impl Into<String>) -> Self {
        Self { kind, text: text.into() }
    }
}

/// What to do after a key has been accepted.
#[derive(Clone)]
pub enum PendingAction {
    Reopen(PathBuf),
    ExtractAll,
    ExtractSelected(VirtualFile),
    Nothing,
}

/// Modal "AES key required" prompt, shown BEFORE any failing work happens.
pub struct KeyPrompt {
    pub reason: String,
    pub guid_hint: Option<String>,
    pub action: PendingAction,
    pub input: String,
    pub error: Option<String>,
}

impl KeyPrompt {
    pub fn input_valid(hex: &str) -> bool {
        AesKey::parse(hex).is_some()
    }
}

// ---------------------------------------------------------------------------

pub struct App {
    pub mount: Option<Arc<Mount>>,
    pub archive_name: String,
    pub tree: Vec<TreeNode>,
    pub flat_files: Vec<VirtualFile>,
    pub filter: String,
    pub expanded: std::collections::HashSet<String>,
    pub selected: Option<VirtualFile>,
    pub preview: Preview,
    pub view_mode: ViewMode,
    pub sort_col: SortCol,
    pub sort_asc: bool,

    pub keys: Vec<KeyEntry>,
    pub active_key: Option<String>,
    pub recent: Vec<String>,

    // extraction
    pub extracting: bool,
    pub progress_rx: Option<Receiver<ProgressEvent>>,
    pub last_progress: Option<ProgressEvent>,
    pub extract_cancel: Arc<AtomicBool>,
    pub report: Option<Arc<std::sync::Mutex<uex_core::pipeline::ExtractReport>>>,
    pub last_errors: Vec<String>,
    pub output_dir: String,
    pub convert: bool,
    pub formats: ExportFormats,
    pub show_settings: bool,
    pub show_keys: bool,
    pub show_errors: bool,
    pub show_progress: bool,
    pub status: Status,

    // key prompt modal
    pub key_prompt: Option<KeyPrompt>,

    // thumbnails
    pub textures: HashMap<String, egui::TextureHandle>,

    // batch queue
    pub queue: Vec<String>,

    // archive to open on the first frame (CLI argument)
    pub pending_open: Option<PathBuf>,

    // automated screenshot mode
    pub autoshot: Option<AutoShot>,
}

pub struct AutoShot {
    pub out_dir: PathBuf,
    pub archive: PathBuf,
    pub big_archive: Option<PathBuf>,
    pub stage: u32,
    pub frames_in_stage: u32,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        let persisted = load_persisted();
        let output_dir = persisted.export_dir.clone().unwrap_or_else(|| "extracted".to_string());
        App {
            mount: None,
            archive_name: String::new(),
            tree: Vec::new(),
            flat_files: Vec::new(),
            filter: String::new(),
            expanded: Default::default(),
            selected: None,
            preview: Preview::None,
            view_mode: ViewMode::List,
            sort_col: SortCol::Name,
            sort_asc: true,
            keys: persisted.keys,
            active_key: persisted.active_key,
            recent: persisted.recent,
            extracting: false,
            progress_rx: None,
            last_progress: None,
            extract_cancel: Arc::new(AtomicBool::new(false)),
            report: None,
            last_errors: Vec::new(),
            output_dir,
            convert: persisted.convert,
            formats: persisted.formats,
            show_settings: false,
            show_keys: false,
            show_errors: false,
            show_progress: false,
            status: Status::new(StatusKind::Info, "Ready — open an archive or drop it anywhere."),
            textures: HashMap::new(),
            key_prompt: None,
            queue: Vec::new(),
            pending_open: None,
            autoshot: None,
        }
    }

    pub fn save_persisted(&self) {
        let p = Persisted {
            recent: self.recent.clone(),
            keys: self.keys.clone(),
            active_key: self.active_key.clone(),
            export_dir: Some(self.output_dir.clone()),
            convert: self.convert,
            formats: self.formats.clone(),
        };
        if let Some(path) = config_path() {
            let _ = std::fs::create_dir_all(path.parent().unwrap());
            let _ = std::fs::write(path, serde_json::to_string_pretty(&p).unwrap());
        }
    }

    pub fn resolve_key(&self) -> Option<AesKey> {
        let name = self.active_key.as_ref()?;
        self.keys
            .iter()
            .find(|k| &k.name == name)
            .and_then(|k| AesKey::parse(&k.hex))
    }

    pub fn engine_badge(&self) -> Option<(&'static str, &'static str, egui::Color32)> {
        // (glyph, label, color) — determined from loaded file paths
        let files = &self.flat_files;
        if files.is_empty() {
            return None;
        }
        let any = |exts: &[&str]| {
            files.iter().any(|f| {
                let l = f.path.to_lowercase();
                exts.iter().any(|e| l.ends_with(e) || l.contains(e))
            })
        };
        if any(&[".pak", ".utoc", ".ucas", ".uasset", ".uexp", ".ubulk"]) {
            Some((ic::UNREAL, "Unreal Engine", crate::theme::Palette::ACCENT))
        } else if any(&[".bundle", ".unity3d", ".assets", "sharedassets", "level0", ".ress", ".resource", "cab-"]) {
            Some((ic::UNITY, "Unity", crate::theme::Palette::MAGENTA))
        } else {
            None
        }
    }

    // -----------------------------------------------------------------------
    // archive open

    pub fn open_path(&mut self, path: PathBuf) {
        match Mount::open(&path, self.resolve_key()) {
            Ok(mount) => {
                let mount = Arc::new(mount);
                self.flat_files = mount.files().cloned().collect();
                self.archive_name = path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                self.tree = build_tree(self.flat_files.clone());
                // auto-expand top-level folders
                self.expanded.clear();
                for node in &self.tree {
                    if !node.children.is_empty() {
                        self.expanded.insert(node.full_path.clone());
                    }
                }
                self.status = Status::new(
                    StatusKind::Ok,
                    format!(
                        "Loaded {} — {} files, {}",
                        self.archive_name,
                        self.flat_files.len(),
                        format_size(mount.total_size)
                    ),
                );
                if mount.encrypted_file_count() > 0 {
                    self.status = Status::new(
                        StatusKind::Warn,
                        format!(
                            "Loaded {} — {} files ({} AES-encrypted: a key will be requested before extraction)",
                            self.archive_name,
                            self.flat_files.len(),
                            mount.encrypted_file_count()
                        ),
                    );
                }
                let rec = path.display().to_string();
                self.recent.retain(|r| r != &rec);
                self.recent.insert(0, rec);
                self.recent.truncate(8);
                self.mount = Some(mount);
                self.preview = Preview::None;
                self.selected = None;
                self.textures.clear();
                self.save_persisted();
            }
            Err(e) => {
                // The most common failure: archive needs a key to even open.
                let text = e.to_string();
                if matches!(e, uex_core::error::Error::EncryptedNoKey)
                    || text.contains("key is most likely WRONG")
                {
                    self.key_prompt = Some(KeyPrompt {
                        reason: format!(
                            "{} is AES-encrypted (its index can only be read with the game's key).{}\nEnter the 64-character hex key to open this archive.",
                            path.file_name().and_then(|s| s.to_str()).unwrap_or("This archive"),
                            if text.contains("WRONG") {
                                "\nThe key that was loaded did NOT work — it belongs to a different game."
                            } else {
                                ""
                            }
                        ),
                        guid_hint: None,
                        action: PendingAction::Reopen(path.clone()),
                        input: String::new(),
                        error: None,
                    });
                    self.status = Status::new(
                        StatusKind::Warn,
                        if text.contains("WRONG") {
                            "Wrong AES key — enter the correct key."
                        } else {
                            "Archive is encrypted — AES key required."
                        },
                    );
                } else {
                    self.status = Status::new(StatusKind::Err, format!("Failed to open {}: {e}", path.display()));
                }
            }
        }
    }

    pub fn filtered_files(&self) -> Vec<&VirtualFile> {
        let flt = self.filter.to_lowercase();
        self.flat_files
            .iter()
            .filter(|f| flt.is_empty() || f.path.to_lowercase().contains(&flt))
            .take(20_000)
            .collect()
    }

    /// Sorted + filtered view for the asset table.
    pub fn table_files(&mut self) -> Vec<VirtualFile> {
        let mut files: Vec<VirtualFile> = self
            .filtered_files()
            .into_iter()
            .cloned()
            .collect();
        let lower_type = |p: &str| type_icon(p).0.to_string();
        match self.sort_col {
            SortCol::Name => files.sort_by(|a, b| a.path.cmp(&b.path)),
            SortCol::Size => files.sort_by_key(|f| f.size),
            SortCol::Type => files.sort_by(|a, b| {
                lower_type(&a.path).cmp(&lower_type(&b.path)).then(a.path.cmp(&b.path))
            }),
        }
        if !self.sort_asc {
            files.reverse();
        }
        files
    }

    // -----------------------------------------------------------------------
    // extraction

    /// Gate: if the archive holds encrypted payloads and no valid key is loaded,
    /// raise the key prompt instead of starting work that would fail.
    fn aes_gate(&mut self, action: PendingAction) -> bool {
        let Some(mount) = self.mount.clone() else { return false };
        let enc = mount.encrypted_file_count();
        if enc == 0 {
            return true;
        }
        match self.resolve_key() {
            None => {
                self.key_prompt = Some(KeyPrompt {
                    reason: format!(
                        "{enc} of {} files in this archive are AES-encrypted.\nExtraction would fail without the game's key. Enter the 64-character hex key now, or extract only the unencrypted files.",
                        mount.file_count()
                    ),
                    guid_hint: mount.encryption_key_guids().first().cloned(),
                    action,
                    input: String::new(),
                    error: None,
                });
                false
            }
            Some(key) => match mount.test_key(&key) {
                Ok(true) => true, // key verified against an encrypted payload
                Ok(false) => true, // nothing probeable; proceed and let per-file errors surface
                Err(e) => {
                    self.key_prompt = Some(KeyPrompt {
                        reason: format!(
                            "The active key FAILED verification: {e}\nThe loaded key is wrong for this archive. Enter the correct 64-hex-char key, or extract only the unencrypted files."
                        ),
                        guid_hint: mount.encryption_key_guids().first().cloned(),
                        action,
                        input: String::new(),
                        error: None,
                    });
                    false
                }
            },
        }
    }

    pub fn extract(&mut self, selection: Option<Vec<VirtualFile>>) {
        let action = match selection.as_deref() {
            Some([first, ..]) => PendingAction::ExtractSelected(first.clone()),
            _ => PendingAction::ExtractAll,
        };
        if !self.aes_gate(action) {
            return;
        }
        self.start_extraction(selection, false);
    }

    /// Extract while skipping encrypted files (fallback offered in the key prompt).
    pub fn extract_unencrypted_only(&mut self) {
        self.start_extraction(None, true);
    }

    pub fn start_extraction(&mut self, selection: Option<Vec<VirtualFile>>, skip_encrypted: bool) {
        let Some(mount) = self.mount.clone() else { return };
        let out = PathBuf::from(&self.output_dir);
        std::fs::create_dir_all(&out).ok();

        let opts = uex_core::pipeline::ExtractOptions {
            output_dir: out,
            convert: self.convert,
            skip_encrypted,
            cancel: Arc::new(AtomicBool::new(false)),
            ..Default::default()
        };
        self.extract_cancel = opts.cancel.clone();
        let (rx, report) = uex_core::pipeline::extract_parallel(&mount, &opts);
        let _ = selection;
        self.extracting = true;
        self.show_progress = true;
        self.progress_rx = Some(rx);
        self.report = Some(report);
        self.last_progress = None;
        self.status = Status::new(StatusKind::Busy, "Extracting…");
    }

    pub fn cancel_extraction(&mut self) {
        self.extract_cancel.store(true, Ordering::Relaxed);
        self.status = Status::new(StatusKind::Warn, "Cancelling extraction…");
    }

    pub fn update_progress(&mut self) {
        if let Some(rx) = &self.progress_rx {
            while let Ok(ev) = rx.try_recv() {
                if ev.finished {
                    self.extracting = false;
                    self.show_progress = false;
                    let (extracted_n, elapsed, bytes_n) = {
                        let r = self.report.as_ref().map(|r| r.lock().unwrap().clone());
                        match r {
                            Some(rep) => (rep.extracted, rep.elapsed, rep.bytes),
                            None => (ev.done, std::time::Duration::ZERO, ev.bytes_done),
                        }
                    };
                    let mbps = if elapsed.as_secs_f64() > 0.01 {
                        bytes_n as f64 / 1048576.0 / elapsed.as_secs_f64()
                    } else {
                        ev.mbps
                    };
                    let errors = self
                        .report
                        .as_ref()
                        .map(|r| r.lock().unwrap().errors.clone())
                        .unwrap_or_default();
                    let n_err = errors.len();
                    self.last_errors = errors;
                    if ev.current == "__cancelled__" {
                        self.status = Status::new(
                            StatusKind::Warn,
                            format!(
                                "Extraction cancelled at {}/{} files ({} errors)",
                                ev.done, ev.total, n_err
                            ),
                        );
                    } else if n_err > 0 {
                        self.status = Status::new(
                            StatusKind::Err,
                            format!(
                                "Finished with {n_err} error{} — {} files extracted, {:.1} MB/s. Click the red badge for details.",
                                if n_err == 1 { "" } else { "s" },
                                extracted_n,
                                mbps
                            ),
                        );
                    } else {
                        self.status = Status::new(
                            StatusKind::Ok,
                            format!(
                                "Extraction complete: {} files, {} in {:.1}s ({:.1} MB/s)",
                                extracted_n,
                                format_size(bytes_n),
                                elapsed.as_secs_f64(),
                                mbps
                            ),
                        );
                    }
                }
                self.last_progress = Some(ev.clone());
            }
        }
    }

    /// Try to extract one file (right-panel button). Routes through the AES gate.
    pub fn extract_selected(&mut self, sel: Option<VirtualFile>) {
        let Some(vf) = sel else { return };
        let Some(mount) = self.mount.clone() else { return };
        if mount.is_file_encrypted(&vf) {
            // single encrypted file: ask first
            if !self.aes_gate(PendingAction::ExtractSelected(vf.clone())) {
                return;
            }
        }
        self.extract_one_now(&vf);
    }

    pub fn extract_one_now(&mut self, vf: &VirtualFile) {
        let Some(mount) = self.mount.clone() else { return };
        let out = PathBuf::from(&self.output_dir);
        std::fs::create_dir_all(&out).ok();
        let mut reader = match mount.open_source_reader(vf.source) {
            Ok(r) => r,
            Err(e) => {
                self.status = Status::new(StatusKind::Err, format!("error: {e}"));
                return;
            }
        };
        let opts = uex_core::pipeline::ExtractOptions {
            output_dir: out,
            convert: self.convert,
            ..Default::default()
        };
        let result = uex_core::pipeline::convert::extract_one(&mut reader, vf, &opts);
        match result {
            Ok(converted) => {
                self.status = Status::new(
                    StatusKind::Ok,
                    if converted {
                        format!("Extracted + converted: {}", vf.path)
                    } else {
                        format!("Extracted: {}", vf.path)
                    },
                );
            }
            Err(e) => {
                self.status = Status::new(StatusKind::Err, format!("error: {e}"));
            }
        }
    }

    /// Called by the key prompt when the user saves a key.
    /// On failure returns an actionable error message (prompt stays open).
    pub fn apply_key(&mut self, hex: String) -> std::result::Result<(), String> {
        let Some(key) = AesKey::parse(&hex) else {
            return Err("Key must be 64 hex characters (32 bytes).".into());
        };
        // name it after the archive or incrementally
        let name = match &self.archive_name {
            n if !n.is_empty() => format!("Key — {n}"),
            _ => format!("Key {}", self.keys.len() + 1),
        };
        self.keys.retain(|k| k.name != name);
        self.keys.push(KeyEntry {
            name: name.clone(),
            hex: hex.trim().trim_start_matches("0x").replace(['-', ' '], "").to_lowercase(),
        });
        self.active_key = Some(name);
        self.save_persisted();

        // validate against the loaded archive when possible
        if let Some(mount) = self.mount.clone() {
            if mount.encrypted_file_count() > 0 {
                if let Err(e) = mount.test_key(&key) {
                    return Err(format!(
                        "Key was saved but FAILED verification: {e}"
                    ));
                }
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // preview

    pub fn load_preview(&mut self, ctx: &egui::Context, vf: &VirtualFile) {
        let lower = vf.path.to_lowercase();
        let Some(mount) = self.mount.clone() else { return };
        let mut reader = match mount.open_source_reader(vf.source) {
            Ok(r) => r,
            Err(e) => {
                self.preview = Preview::Unsupported(format!("open failed: {e}"));
                return;
            }
        };
        // exported images
        for ext in ["png", "tga", "jpg", "jpeg"] {
            if lower.ends_with(&format!(".{ext}")) {
                if let Ok(data) = reader.read(vf) {
                    if let Ok(img) = image::load_from_memory(&data) {
                        let rgba = img.to_rgba8();
                        let size = [rgba.width() as usize, rgba.height() as usize];
                        let handle = ctx.load_texture(
                            &vf.path,
                            egui::ColorImage::from_rgba_unmultiplied(size, &rgba),
                            Default::default(),
                        );
                        self.preview = Preview::Texture(handle);
                        return;
                    }
                }
            }
        }
        // textures from packages
        if lower.ends_with(".uasset") {
            let is_zen = matches!(reader, uex_core::mount::SourceReader::IoStore { .. });
            let tex = if is_zen {
                reader.zen_package(vf).ok().and_then(|zen| {
                    (0..zen.exports.len()).find_map(|i| {
                        if zen.exports[i].class_name == "Texture2D" {
                            uex_core::assets::texture::extract_from_zen(&zen, i).ok()
                        } else {
                            None
                        }
                    })
                })
            } else {
                let exp = self.find_sibling(&vf.path, "uexp").and_then(|p| {
                    let f = self.flat_files.iter().find(|f| f.path == p)?;
                    reader.read(f).ok()
                });
                let bulk = self.find_sibling(&vf.path, "ubulk").and_then(|p| {
                    let f = self.flat_files.iter().find(|f| f.path == p)?;
                    reader.read(f).ok()
                });
                reader.read(vf).ok().and_then(|raw| {
                    let pkg = uex_core::uasset::UAssetPackage::parse(raw, exp, bulk).ok()?;
                    (0..pkg.exports.len()).find_map(|i| {
                        if uex_core::pipeline::convert::export_class(&pkg, i) == "Texture2D" {
                            uex_core::assets::texture::extract_from_package(&pkg, i).ok()
                        } else {
                            None
                        }
                    })
                })
            };
            if let Some(tex) = tex {
                if let Ok(rgba) = tex.top_rgba() {
                    let size = [tex.width as usize, tex.height as usize];
                    let handle = ctx.load_texture(
                        &vf.path,
                        egui::ColorImage::from_rgba_unmultiplied(size, &rgba),
                        Default::default(),
                    );
                    self.preview = Preview::Texture(handle);
                    return;
                }
            }
        }
        if lower.ends_with(".glb") {
            if let Ok(data) = reader.read(vf) {
                if let Some(mesh) = parse_glb_mesh(&data, &file_name(&vf.path)) {
                    self.preview = Preview::Mesh {
                        mesh,
                        rot_x: 0.4,
                        rot_y: 0.6,
                        zoom: 1.0,
                    };
                    return;
                }
            }
            self.preview = Preview::Text("GLB exported — open in Blender or any glTF viewer.".into());
            return;
        }
        if lower.ends_with(".metadata.json") || lower.ends_with(".json") {
            if let Ok(data) = reader.read(vf) {
                if let Ok(text) = String::from_utf8(data) {
                    self.preview = Preview::Text(text);
                    return;
                }
            }
        }
        // Unity serialized files: CAB-* inside bundles, .assets, level*, …
        if let Ok(data) = reader.read(vf) {
            if uex_core::unity::looks_like_serialized_file(&data) {
                self.load_unity_preview(ctx, &vf.path, data, &mut reader);
                return;
            }
        }
        self.preview = Preview::Unsupported("No preview available for this file type.".into());
    }

    /// Unity preview: first decodable Texture2D/Sprite in the serialized file,
    /// otherwise a compact object listing.
    fn load_unity_preview(
        &mut self,
        ctx: &egui::Context,
        path: &str,
        data: Vec<u8>,
        reader: &mut uex_core::mount::SourceReader,
    ) {
        let mut sf = match uex_core::unity::serialized::SerializedFile::parse(data) {
            Ok(sf) => sf,
            Err(e) => {
                self.preview = Preview::Unsupported(format!("Unity parse failed: {e}"));
                return;
            }
        };
        sf.load_container();
        let (bundle, base_dir) = reader.unity_context();
        let res = uex_core::unity::objects::ResourceContext::new(bundle, base_dir);

        // try textures first
        for obj in &sf.objects {
            if obj.class_id == uex_core::unity::class_id::TEXTURE_2D {
                if let Ok((_cls, value)) = sf.read_object(obj) {
                    if let Ok((tex, rgba)) =
                        uex_core::unity::objects::texture2d_data(&value, &sf, &res)
                    {
                        let size = [tex.width as usize, tex.height as usize];
                        let handle = ctx.load_texture(
                            path,
                            egui::ColorImage::from_rgba_unmultiplied(size, &rgba),
                            Default::default(),
                        );
                        self.preview = Preview::Texture(handle);
                        return;
                    }
                }
            }
        }
        // sprites next (crop from atlas textures)
        let parsed: Vec<(uex_core::unity::serialized::ObjectInfo, String, uex_core::unity::typetree::UValue)> =
            sf.objects.iter().filter_map(|o| sf.read_object(o).ok().map(|(c, v)| (o.clone(), c, v))).collect();
        for (obj, _class, value) in &parsed {
            if obj.class_id == uex_core::unity::class_id::SPRITE {
                if let Ok((tex, rgba)) = uex_core::unity::objects::sprite_rgba(
                    value,
                    &sf.object_name(obj, value),
                    &sf,
                    &res,
                    &parsed,
                ) {
                    let size = [tex.width as usize, tex.height as usize];
                    let handle = ctx.load_texture(
                        path,
                        egui::ColorImage::from_rgba_unmultiplied(size, &rgba),
                        Default::default(),
                    );
                    self.preview = Preview::Texture(handle);
                    return;
                }
            }
        }
        // fallback: object listing
        let mut text = format!(
            "Unity SerializedFile\nengine: {}  |  serialized v{}  |  {} objects\n\n",
            sf.unity_version_raw,
            sf.header.version,
            sf.objects.len()
        );
        for obj in &sf.objects {
            let class = uex_core::unity::class_name(obj.class_id);
            let name = parsed
                .iter()
                .find(|(o, _, _)| o.path_id == obj.path_id)
                .map(|(o, _, v)| sf.object_name(o, v))
                .unwrap_or_else(|| "?".into());
            text.push_str(&format!("  {:>8}  {:<14} {}\n", obj.path_id, class, name));
        }
        self.preview = Preview::Text(text);
    }

    pub fn find_sibling(&self, path: &str, ext: &str) -> Option<String> {
        let base = path.trim_end_matches(".uasset");
        let candidate = format!("{base}.{ext}");
        self.flat_files.iter().find(|f| f.path == candidate).map(|_| candidate)
    }

    pub fn expand_all(&mut self) {
        fn walk(node: &TreeNode, expanded: &mut std::collections::HashSet<String>) {
            expanded.insert(node.full_path.clone());
            for c in &node.children {
                walk(c, expanded);
            }
        }
        for node in &self.tree {
            walk(node, &mut self.expanded);
        }
    }
}

// ---------------------------------------------------------------------------
// shared small helpers (used by ui.rs too)

pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

pub fn format_size(size: u64) -> String {
    if size >= 1 << 20 {
        format!("{:.1} MB", size as f64 / 1048576.0)
    } else if size >= 1 << 10 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{size} B")
    }
}

/// (glyph, color) for a file type.
pub fn type_icon(path: &str) -> (&'static str, egui::Color32) {
    use crate::theme::Palette;
    let lower = path.to_lowercase();
    if lower.ends_with(".uasset") || lower.ends_with(".uexp") {
        (ic::PACKAGE, Palette::C_PACKAGE)
    } else if lower.ends_with(".ubulk") {
        (ic::DATA, Palette::C_DATA)
    } else if lower.ends_with(".pak") || lower.ends_with(".utoc") || lower.ends_with(".ucas")
        || lower.ends_with(".bundle") || lower.ends_with(".unity3d") {
        (ic::ARCHIVE, Palette::ACCENT)
    } else if lower.ends_with(".png") || lower.ends_with(".tga") || lower.ends_with(".dds")
        || lower.ends_with(".exr") || lower.ends_with(".jpg") || lower.ends_with(".jpeg")
        || lower.ends_with(".bmp") {
        (ic::TEXTURE, Palette::C_IMAGE)
    } else if lower.ends_with(".glb") || lower.ends_with(".gltf") || lower.ends_with(".obj")
        || lower.ends_with(".fbx") || lower.ends_with(".usm") {
        (ic::MESH, Palette::C_MESH)
    } else if lower.ends_with(".ogg") || lower.ends_with(".wav") || lower.ends_with(".mp3")
        || lower.ends_with(".opus") || lower.ends_with(".bnk") || lower.ends_with(".wem") {
        (ic::AUDIO, Palette::C_AUDIO)
    } else if lower.ends_with(".json") || lower.ends_with(".csv") || lower.ends_with(".xml") {
        (ic::DATA, Palette::C_DATA)
    } else if lower.ends_with(".ttf") || lower.ends_with(".otf") || lower.ends_with(".fnt")
        || lower.ends_with(".ufo") {
        (ic::FONT, Palette::C_FONT)
    } else if lower.ends_with(".lua") || lower.ends_with(".py") || lower.ends_with(".js")
        || lower.ends_with(".cs") || lower.ends_with(".ini") || lower.ends_with(".txt")
        || lower.ends_with(".bytes") {
        (ic::CODE, Palette::C_DATA)
    } else if lower.ends_with(".uasset") {
        (ic::BLUEPRINT, Palette::C_PACKAGE)
    } else {
        (ic::FILE, Palette::C_FILE)
    }
}

pub fn file_type_name(path: &str) -> &'static str {
    let lower = path.to_lowercase();
    if lower.ends_with(".uasset") {
        "Unreal package"
    } else if lower.ends_with(".ubulk") {
        "Bulk data"
    } else if lower.ends_with(".uexp") {
        "Export data"
    } else if lower.ends_with(".pak") {
        "Unreal pak archive"
    } else if lower.ends_with(".utoc") || lower.ends_with(".ucas") {
        "Unreal IOStore container"
    } else if lower.ends_with(".bundle") || lower.ends_with(".assetbundle") {
        "Unity asset bundle"
    } else if lower.ends_with(".unity3d") {
        "Unity bundle"
    } else if lower.ends_with(".assets") {
        "Unity serialized assets"
    } else if lower.ends_with(".resource") || lower.ends_with(".ress") {
        "Unity streamed resource"
    } else {
        "File"
    }
}

// ---------------------------------------------------------------------------
// 3D mesh preview (software-rendered through egui's painter)

pub fn mesh_preview_ui(ui: &mut egui::Ui, mesh: &uex_core::assets::mesh::ExtractedMesh, rot_x: &mut f32, rot_y: &mut f32, zoom: &mut f32) {
    let (rect, resp) = ui.allocate_exact_size(ui.available_size_before_wrap().min(egui::vec2(360.0, 280.0)), egui::Sense::drag());
    *rot_y += resp.drag_delta().x * 0.01;
    *rot_x += resp.drag_delta().y * 0.01;
    *rot_x = rot_x.clamp(-1.5, 1.5);
    if resp.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        *zoom = (*zoom - scroll * 0.01).clamp(0.3, 8.0);
    }

    let (center, _radius, _ignored) = mesh_center(mesh);
    let (cx, cy, cz) = (center[0], center[1], center[2]);
    let scale = *zoom * rect.width().min(rect.height()) / 2.6 / mesh_radius(mesh).max(0.001);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, egui::Color32::from_rgb(0x10, 0x11, 0x16));

    let (sy, cy_) = rot_y.sin_cos();
    let (sx, cx_) = rot_x.sin_cos();
    let mut tris: Vec<(f32, [egui::Pos2; 3], [egui::Color32; 3])> = Vec::new();
    for section in &mesh.sections {
        for tri in section.indices.chunks_exact(3) {
            let mut pts = [egui::Pos2::ZERO; 3];
            let mut cols = [egui::Color32::WHITE; 3];
            let mut zavg = 0.0;
            for (k, &idx) in tri.iter().enumerate() {
                let Some(v) = mesh.vertices.get(idx as usize) else { continue };
                let p = [
                    v.position[0] - cx,
                    v.position[1] - cy,
                    v.position[2] - cz,
                ];
                // rotate Y then X
                let x1 = p[0] * cy_ + p[2] * sy;
                let z1 = -p[0] * sy + p[2] * cy_;
                let y2 = p[1] * cx_ - z1 * sx;
                let z2 = p[1] * sx + z1 * cx_;
                zavg += z2;
                let light = (v.normal[0] * 0.3 + v.normal[1] * 0.5 + v.normal[2] * 0.4).clamp(0.05, 1.0);
                let c = egui::Color32::from_rgb(
                    (90.0 + 130.0 * light) as u8,
                    (110.0 + 120.0 * light) as u8,
                    (200.0 + 45.0 * light) as u8,
                );
                pts[k] = egui::Pos2::new(
                    rect.center().x + x1 * scale,
                    rect.center().y - y2 * scale,
                );
                cols[k] = c;
            }
            tris.push((zavg / 3.0, pts, cols));
        }
    }
    tris.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut vertices = Vec::with_capacity(tris.len() * 3);
    let indices: Vec<u32> = (0..tris.len() as u32 * 3).collect();
    for (_, pts, cols) in tris {
        for k in 0..3 {
            vertices.push(egui::epaint::Vertex { pos: pts[k], uv: egui::Pos2::ZERO, color: cols[k] });
        }
    }
    if !vertices.is_empty() {
        painter.add(egui::Shape::mesh(egui::Mesh {
            vertices,
            indices,
            texture_id: Default::default(),
        }));
    }
}

fn mesh_center(mesh: &uex_core::assets::mesh::ExtractedMesh) -> ([f32; 3], f32, f32) {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for v in &mesh.vertices {
        for i in 0..3 {
            min[i] = min[i].min(v.position[i]);
            max[i] = max[i].max(v.position[i]);
        }
    }
    let center = [
        (min[0] + max[0]) / 2.0,
        (min[1] + max[1]) / 2.0,
        (min[2] + max[2]) / 2.0,
    ];
    let radius = ((max[0] - min[0]).powi(2) + (max[1] - min[1]).powi(2) + (max[2] - min[2]).powi(2))
        .sqrt()
        / 2.0;
    (center, radius, 1.0)
}

fn mesh_radius(mesh: &uex_core::assets::mesh::ExtractedMesh) -> f32 {
    mesh_center(mesh).1
}

/// Minimal GLB reader for the preview: extracts positions, normals and indices
/// of the first primitive (the exporter's own output format).
fn parse_glb_mesh(data: &[u8], name: &str) -> Option<uex_core::assets::mesh::ExtractedMesh> {
    if data.len() < 20 || &data[0..4] != b"glTF" {
        return None;
    }
    let json_len = u32::from_le_bytes(data[12..16].try_into().ok()?) as usize;
    let json: serde_json::Value = serde_json::from_slice(&data[20..20 + json_len]).ok()?;
    let bin_offset = 20 + json_len;
    if bin_offset + 8 > data.len() {
        return None;
    }
    let bin_len = u32::from_le_bytes(data[bin_offset..bin_offset + 4].try_into().ok()?) as usize;
    let bin_start = bin_offset + 8;
    let bin = data.get(bin_start..bin_start + bin_len)?;

    let prim = json.get("meshes")?.get(0)?.get("primitives")?.get(0)?;
    let attrs = prim.get("attributes")?;
    let pos_acc_idx = attrs.get("POSITION")?.as_u64()? as usize;
    let idx_acc_idx = prim.get("indices")?.as_u64()? as usize;

    let accessors = json.get("accessors")?;
    let views = json.get("bufferViews")?;

    let read_accessor_f32 = |idx: usize, comps: usize| -> Option<Vec<f32>> {
        let acc = accessors.get(idx)?;
        let view = views.get(acc.get("bufferView")?.as_u64()? as usize)?;
        let off = view.get("byteOffset")?.as_u64()? as usize;
        let count = acc.get("count")?.as_u64()? as usize;
        let start = bin.get(off..off + count * comps * 4)?;
        Some(
            start
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                .collect(),
        )
    };

    let pos = read_accessor_f32(pos_acc_idx, 3)?;
    let norm = read_accessor_f32(pos_acc_idx + 1, 3).unwrap_or_else(|| vec![0.0; pos.len()]);
    let vertex_count = pos.len() / 3;
    let mut vertices = Vec::with_capacity(vertex_count);
    for i in 0..vertex_count {
        let uv: Vec<[f32; 2]> = read_accessor_f32(pos_acc_idx + 2, 2)
            .map(|u| vec![[u[i * 2], u[i * 2 + 1]]])
            .unwrap_or_else(|| vec![[0.0, 0.0]]);
        vertices.push(uex_core::assets::mesh::Vertex {
            position: [pos[i * 3], pos[i * 3 + 1], pos[i * 3 + 2]],
            normal: [norm[i * 3], norm[i * 3 + 1], norm[i * 3 + 2]],
            uv,
            ..Default::default()
        });
    }

    let acc = accessors.get(idx_acc_idx)?;
    let view = views.get(acc.get("bufferView")?.as_u64()? as usize)?;
    let off = view.get("byteOffset")?.as_u64()? as usize;
    let count = acc.get("count")?.as_u64()? as usize;
    let component = acc.get("componentType")?.as_u64()? as u32;
    let idx_start = bin.get(off..off + count * if component == 5125 { 4 } else { 2 })?;
    let indices: Vec<u32> = if component == 5125 {
        idx_start
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect()
    } else {
        idx_start
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes(c.try_into().unwrap()) as u32)
            .collect()
    };

    Some(uex_core::assets::mesh::ExtractedMesh {
        name: name.to_string(),
        vertices,
        sections: vec![uex_core::assets::mesh::MeshSection {
            material_index: 0,
            indices,
        }],
        ..Default::default()
    })
}
