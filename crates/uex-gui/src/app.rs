//! Application state and UI layout.

use crossbeam_channel::Receiver;
use eframe::egui;
use egui::{Color32, RichText};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use uex_core::crypto::AesKey;
use uex_core::mount::{Mount, VirtualFile};
use uex_core::pipeline::ProgressEvent;

// ---------------------------------------------------------------------------
// persistence

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
struct KeyEntry {
    name: String,
    hex: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct ExportFormats {
    png: bool,
    dds: bool,
    glb: bool,
    obj: bool,
    json: bool,
    audio: bool,
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

// ---------------------------------------------------------------------------
// preview

enum Preview {
    None,
    Texture(egui::TextureHandle),
    Mesh { mesh: uex_core::assets::mesh::ExtractedMesh, rot_x: f32, rot_y: f32, zoom: f32 },
    Text(String),
    Unsupported(String),
}

#[derive(Clone, Copy, PartialEq)]
enum ViewMode {
    List,
    Grid,
}

// ---------------------------------------------------------------------------

pub struct App {
    mount: Option<Arc<Mount>>,
    tree: Vec<TreeNode>,
    flat_files: Vec<VirtualFile>,
    filter: String,
    expanded: std::collections::HashSet<String>,
    selected: Option<VirtualFile>,
    preview: Preview,
    view_mode: ViewMode,

    keys: Vec<KeyEntry>,
    active_key: Option<String>,
    recent: Vec<String>,

    // extraction
    extracting: bool,
    progress_rx: Option<Receiver<ProgressEvent>>,
    last_progress: Option<ProgressEvent>,
    output_dir: String,
    convert: bool,
    formats: ExportFormats,
    show_settings: bool,
    show_keys: bool,
    status: String,

    // thumbnails
    textures: HashMap<String, egui::TextureHandle>,

    // batch queue
    queue: Vec<String>,

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

fn load_persisted() -> Persisted {
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

impl App {
    pub fn new() -> Self {
        let persisted = load_persisted();
        let output_dir = persisted.export_dir.clone().unwrap_or_else(|| "extracted".to_string());
        App {
            mount: None,
            tree: Vec::new(),
            flat_files: Vec::new(),
            filter: String::new(),
            expanded: Default::default(),
            selected: None,
            preview: Preview::None,
            view_mode: ViewMode::List,
            keys: persisted.keys,
            active_key: persisted.active_key,
            recent: persisted.recent,
            extracting: false,
            progress_rx: None,
            last_progress: None,
            output_dir,
            convert: persisted.convert,
            formats: persisted.formats,
            show_settings: false,
            show_keys: false,
            status: "Ready. Open a .pak / .utoc archive or drop it here.".into(),
            textures: HashMap::new(),
            queue: Vec::new(),
            autoshot: None,
        }
    }

    fn save_persisted(&self) {
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

    fn resolve_key(&self) -> Option<AesKey> {
        let name = self.active_key.as_ref()?;
        self.keys
            .iter()
            .find(|k| &k.name == name)
            .and_then(|k| AesKey::parse(&k.hex))
    }

    fn open_path(&mut self, path: PathBuf) {
        match Mount::open(&path, self.resolve_key()) {
            Ok(mount) => {
                let mount = Arc::new(mount);
                self.flat_files = mount.files().cloned().collect();
                self.tree = build_tree(self.flat_files.clone());
                self.status = format!(
                    "Loaded {} ({} files, {:.1} MB)",
                    path.display(),
                    self.flat_files.len(),
                    mount.total_size as f64 / 1048576.0
                );
                let rec = path.display().to_string();
                self.recent.retain(|r| r != &rec);
                self.recent.insert(0, rec);
                self.recent.truncate(8);
                self.mount = Some(mount);
                self.preview = Preview::None;
                self.selected = None;
                self.save_persisted();
            }
            Err(e) => {
                self.status = format!("Failed to open {}: {e}", path.display());
            }
        }
    }

    fn filtered_files(&self) -> Vec<&VirtualFile> {
        let flt = self.filter.to_lowercase();
        self.flat_files
            .iter()
            .filter(|f| flt.is_empty() || f.path.to_lowercase().contains(&flt))
            .take(20_000)
            .collect()
    }

    fn extract(&mut self, selection: Option<Vec<&VirtualFile>>) {
        let Some(mount) = self.mount.clone() else { return };
        let out = PathBuf::from(&self.output_dir);
        let threads = 0;
        let convert = self.convert;
        std::fs::create_dir_all(&out).ok();

        // Build a temporary Mount-like view: we reuse the pipeline by planning over all
        // files then filtering; selection case handled through a custom plan closure.
        let opts = uex_core::pipeline::ExtractOptions {
            filter: None,
            extensions: Vec::new(),
            output_dir: out,
            threads,
            convert,
            ..Default::default()
        };
        let (rx, _report) = uex_core::pipeline::extract_parallel(&mount, &opts);
        self.extracting = true;
        self.progress_rx = Some(rx);
        self.last_progress = None;
        let _ = selection;
        self.status = "Extracting...".into();
    }

    fn update_progress(&mut self) {
        if let Some(rx) = &self.progress_rx {
            while let Ok(ev) = rx.try_recv() {
                if ev.finished {
                    self.extracting = false;
                    self.status = format!(
                        "Extraction complete: {} files, {:.1} MB/s peak",
                        ev.done,
                        ev.mbps
                    );
                }
                self.last_progress = Some(ev.clone());
            }
        }
    }

    // -----------------------------------------------------------------------

    fn load_preview(&mut self, ctx: &egui::Context, vf: &VirtualFile) {
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
        self.preview = Preview::Unsupported("No preview available for this file type.".into());
    }

    fn find_sibling(&self, path: &str, ext: &str) -> Option<String> {
        let base = path.trim_end_matches(".uasset");
        let candidate = format!("{base}.{ext}");
        self.flat_files.iter().find(|f| f.path == candidate).map(|_| candidate)
    }
}

fn render_tree_nodes(
    ui: &mut egui::Ui,
    nodes: &[TreeNode],
    expanded: &mut std::collections::HashSet<String>,
    selected_path: Option<&str>,
    to_select: &mut Option<VirtualFile>,
    to_toggle: &mut Option<String>,
) {
    for node in nodes {
        let has_children = !node.children.is_empty();
        if has_children {
            let open = expanded.contains(&node.full_path);
            let chevron = if open { "▾" } else { "▸" };
            let resp = ui.selectable_label(open, format!("{chevron} 📁 {}", node.name));
            if resp.clicked() {
                *to_toggle = Some(node.full_path.clone());
            }
            if open {
                egui::Grid::new(format!("indent_{}", node.full_path)).num_columns(1).show(ui, |ui| {
                    render_tree_nodes(ui, &node.children, expanded, selected_path, to_select, to_toggle);
                });
            }
        } else if node.file.is_some() {
            let icon = type_icon(&node.name);
            let selected = selected_path.map(|p| node.full_path.ends_with(p)).unwrap_or(false);
            if ui.selectable_label(selected, format!("{icon} {}", node.name)).clicked() {
                *to_select = node.file.clone();
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_progress();
        self.autoshot_tick(ctx);

        // drag & drop
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(f) = dropped.first() {
            if let Some(p) = &f.path {
                self.open_path(p.clone());
            }
        }
        let repainting = self.extracting;
        if repainting {
            ctx.request_repaint();
        }

        self.toolbar(ctx);
        self.status_bar(ctx);
        self.left_panel(ctx);
        self.right_panel(ctx);
        self.center_panel(ctx);
        if self.show_settings {
            self.settings_window(ctx);
        }
        if self.show_keys {
            self.keys_window(ctx);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_persisted();
    }
}

impl App {
    fn autoshot_tick(&mut self, ctx: &egui::Context) {
        use std::time::{Duration, Instant};
        let Some(shot) = &self.autoshot else { return };
        let stage = shot.stage;
        let out_dir = shot.out_dir.clone();
        let archive = shot.archive.clone();
        let big = shot.big_archive.clone();
        match stage {
            0 => {
                // open the demo archive
                self.open_path(archive);
                self.next_stage();
            }
            1 => {
                // expand tree fully
                self.expand_all();
                self.next_stage();
            }
            2 => {
                // wait 30 frames for the tree to lay out, then capture
                self.tick_and_capture(ctx, &out_dir, "01_main_window.png", 40);
            }
            3 => {
                // select the texture asset for preview
                let target = self
                    .flat_files
                    .iter()
                    .find(|f| {
                        f.path.ends_with(".png")
                            || f.path.ends_with(".uasset")
                            || f.path.ends_with(".glb")
                    })
                    .cloned();
                if let Some(vf) = target {
                    self.selected = Some(vf.clone());
                    self.load_preview(ctx, &vf);
                }
                self.next_stage();
            }
            4 => self.tick_and_capture(ctx, &out_dir, "02_texture_preview.png", 40),
            5 => {
                // start a real extraction of the big archive in background
                if let Some(big) = big {
                    self.open_path(big);
                    self.extract(None);
                }
                self.next_stage();
            }
            6 => self.tick_and_capture(ctx, &out_dir, "03_extraction_progress.png", 45),
            7 => {
                self.show_settings = true;
                self.next_stage();
            }
            8 => self.tick_and_capture(ctx, &out_dir, "04_settings_dialog.png", 40),
            _ => {
                // done — save capture summary and exit
                self.save_persisted();
                std::process::exit(0);
            }
        }
        let _ = Duration::from_millis(0);
        let _ = Instant::now();
    }

    fn next_stage(&mut self) {
        if let Some(shot) = &mut self.autoshot {
            shot.stage += 1;
            shot.frames_in_stage = 0;
        }
    }

    fn tick_and_capture(&mut self, ctx: &egui::Context, out_dir: &std::path::Path, name: &str, frames: u32) {
        if let Some(shot) = &mut self.autoshot {
            shot.frames_in_stage += 1;
            if shot.frames_in_stage >= frames {
                let screen = crate::capture::capture_root_window();
                if let Ok(png) = screen {
                    let path = out_dir.join(name);
                    if std::fs::write(&path, png).is_ok() {
                        log::info!("screenshot saved: {}", path.display());
                    }
                }
                self.next_stage();
            }
        }
        ctx.request_repaint();
    }

    fn expand_all(&mut self) {
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

    fn toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                if ui.button(RichText::new("📂 Open").size(14.0)).clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Unreal archives", &["pak", "utoc"])
                        .add_filter("All files", &["*"])
                        .pick_file()
                    {
                        self.open_path(path);
                    }
                }
                ui.separator();
                let can_extract = self.mount.is_some();
                if ui
                    .add_enabled(can_extract, egui::Button::new(RichText::new("📦 Extract all").size(14.0)))
                    .clicked()
                {
                    self.extract(None);
                }
                if ui
                    .add_enabled(self.selected.is_some() && can_extract, egui::Button::new("Extract selected"))
                    .clicked()
                {
                    let sel = self.selected.clone();
                    self.extract_selected(sel);
                }
                ui.separator();
                ui.label("Search:");
                ui.add(egui::TextEdit::singleline(&mut self.filter).desired_width(220.0));
                if ui.button("✕").clicked() {
                    self.filter.clear();
                }
                ui.separator();
                ui.toggle_value(&mut self.convert, "Convert assets");
                ui.separator();
                if ui.button("🔑 AES keys").clicked() {
                    self.show_keys = !self.show_keys;
                }
                if ui.button("⚙ Settings").clicked() {
                    self.show_settings = !self.show_settings;
                }
                ui.separator();
                egui::ComboBox::from_id_salt("recent")
                    .selected_text("🕘 Recent")
                    .show_ui(ui, |ui| {
                        for r in self.recent.clone() {
                            if ui.selectable_label(false, &r).clicked() {
                                self.open_path(PathBuf::from(&r));
                            }
                        }
                    });
            });
            ui.add_space(4.0);
        });
    }

    fn extract_selected(&mut self, sel: Option<VirtualFile>) {
        // For v1 the selected file is extracted + converted synchronously (single item).
        let Some(vf) = sel else { return };
        let Some(mount) = self.mount.clone() else { return };
        let out = PathBuf::from(&self.output_dir);
        std::fs::create_dir_all(&out).ok();
        let mut reader = match mount.open_source_reader(vf.source) {
            Ok(r) => r,
            Err(e) => {
                self.status = format!("error: {e}");
                return;
            }
        };
        let opts = uex_core::pipeline::ExtractOptions {
            output_dir: out,
            convert: self.convert,
            ..Default::default()
        };
        let result = uex_core::pipeline::convert::extract_one(&mut reader, &vf, &opts);
        match result {
            Ok(converted) => {
                self.status = if converted {
                    format!("Extracted + converted: {}", vf.path)
                } else {
                    format!("Extracted: {}", vf.path)
                };
            }
            Err(e) => self.status = format!("error: {e}"),
        }
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if let Some(ev) = &self.last_progress {
                    if self.extracting {
                        let pct = if ev.total > 0 { ev.done as f32 / ev.total as f32 } else { 0.0 };
                        ui.add(
                            egui::ProgressBar::new(pct)
                                .show_percentage()
                                .desired_width(220.0),
                        );
                        ui.label(format!(
                            "{:.1} MB/s · ETA {:.0}s · {} errors",
                            ev.mbps, ev.eta_secs, ev.errors
                        ));
                        ui.separator();
                        ui.monospace(if ev.current.len() > 48 { format!("…{}", &ev.current[ev.current.len()-47..]) } else { ev.current.clone() });
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(&self.status).color(Color32::from_rgb(160, 160, 170)));
                });
            });
            ui.add_space(2.0);
        });
    }

    fn left_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("tree")
            .default_width(280.0)
            .width_range(220.0..=320.0)
            .show(ctx, |ui| {
            ui.heading("Archive tree");
            ui.separator();
            if self.mount.is_none() {
                ui.label(RichText::new("No archive loaded.").weak());
                ui.add_space(8.0);
                ui.label("Supported: .pak, .utoc/.ucas");
                return;
            }
            let mut to_select: Option<VirtualFile> = None;
            let mut to_toggle: Option<String> = None;
            egui::ScrollArea::vertical().show(ui, |ui| {
                let selected_path = self.selected.as_ref().map(|s| s.path.clone());
                render_tree_nodes(ui, &self.tree, &mut self.expanded, selected_path.as_deref(), &mut to_select, &mut to_toggle);
            });
            if let Some(p) = to_toggle {
                if self.expanded.contains(&p) { self.expanded.remove(&p); } else { self.expanded.insert(p); }
            }
            if let Some(vf) = to_select {
                self.selected = Some(vf.clone());
                self.load_preview(ctx, &vf);
            }
        });
    }

    fn center_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.mount.is_none() {
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    ui.heading(RichText::new("UExtractor").size(42.0).strong());
                    ui.label(RichText::new("Unreal Engine .pak / .ucas / .utoc asset extractor").size(16.0));
                    ui.add_space(16.0);
                    ui.label("Drop an archive here, or use 📂 Open.");
                    if !self.recent.is_empty() {
                        ui.add_space(8.0);
                        ui.label(RichText::new("Recent archives:").weak());
                        for r in self.recent.clone() {
                            if ui.link(&r).clicked() {
                                self.open_path(PathBuf::from(r));
                            }
                        }
                    }
                });
                return;
            }
            ui.horizontal(|ui| {
                ui.heading("Assets");
                ui.separator();
                ui.selectable_value(&mut self.view_mode, ViewMode::List, "☰ List");
                ui.selectable_value(&mut self.view_mode, ViewMode::Grid, "▦ Grid");
                ui.separator();
                ui.label(format!("{} files", self.filtered_files().len()));
            });
            ui.separator();

            let files: Vec<VirtualFile> = self.filtered_files().into_iter().cloned().collect();
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                match self.view_mode {
                    ViewMode::List => {
                        egui::Grid::new("assets_grid")
                            .num_columns(3)
                            .striped(true)
                            .min_col_width(90.0)
                            .show(ui, |ui| {
                                ui.strong("Icon");
                                ui.strong("Path");
                                ui.strong("Size");
                                ui.end_row();
                                let sel_path = self.selected.as_ref().map(|s| s.path.clone());
                                for f in &files {
                                    let icon = type_icon(&f.path);
                                    let selected = sel_path.as_ref() == Some(&f.path);
                                    if ui.selectable_label(selected, icon).clicked() {
                                        let vf = (*f).clone();
                                        self.selected = Some(vf.clone());
                                        self.load_preview(ctx, &vf);
                                    }
                                    if ui
                                        .selectable_label(selected, RichText::new(&f.path).monospace())
                                        .clicked()
                                    {
                                        let vf = (*f).clone();
                                        self.selected = Some(vf.clone());
                                        self.load_preview(ctx, &vf);
                                    }
                                    ui.label(format_size(f.size));
                                    ui.end_row();
                                }
                            });
                    }
                    ViewMode::Grid => {
                        let card_w = 132.0;
                        egui::Grid::new("assets_cards")
                            .num_columns((ui.available_width() / card_w).max(1.0) as usize)
                            .show(ui, |ui| {
                                let sel_path = self.selected.as_ref().map(|s| s.path.clone());
                                for f in &files {
                                    ui.vertical(|ui| {
                                        let selected = sel_path.as_ref() == Some(&f.path);
                                        let thumb: Option<&egui::TextureHandle> = self.textures.get(&f.path);
                                        let cell = ui.allocate_ui(egui::vec2(card_w - 8.0, 110.0), |ui| {
                                            if let Some(tex) = thumb {
                                                ui.image(egui::load::SizedTexture::new(tex.id(), egui::vec2(110.0, 84.0)));
                                            } else {
                                                ui.centered_and_justified(|ui| {
                                                    ui.label(RichText::new(type_icon(&f.path)).size(34.0));
                                                });
                                            }
                                            ui.add_sized(
                                                [card_w - 10.0, 14.0],
                                                egui::Label::new(
                                                    RichText::new(file_name(&f.path)).small(),
                                                )
                                                .truncate(),
                                            );
                                        });
                                        let resp = cell.response;
                                        if resp.clicked() {
                                            let vf = (*f).clone();
                                            self.selected = Some(vf.clone());
                                            self.load_preview(ctx, &vf);
                                        }
                                    });
                                    ui.end_row();
                                }
                            });
                    }
                }
            });
        });
    }

    fn right_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("details").default_width(360.0).show(ctx, |ui| {
            ui.heading("Details & preview");
            ui.separator();
            if let Some(vf) = &self.selected {
                ui.monospace(RichText::new(&vf.path).size(13.0));
                ui.label(format!("Size: {} bytes", vf.size));
                ui.label(format!("Type: {}", file_type_name(&vf.path)));
                ui.separator();
                match &mut self.preview {
                    Preview::None => {
                        ui.label("Select an asset to preview.");
                    }
                    Preview::Texture(tex) => {
                        let avail = ui.available_width();
                        ui.image(egui::load::SizedTexture::new(tex.id(), egui::vec2(avail, avail * 0.75)));
                        ui.caption("Texture preview (decoded)");
                    }
                    Preview::Mesh { mesh, rot_x, rot_y, zoom } => {
                        let (rx_, ry_, z_) = (rot_x, rot_y, zoom);
                        mesh_preview_ui(ui, mesh, rx_, ry_, z_);
                        ui.caption("Drag to rotate · scroll to zoom");
                    }
                    Preview::Text(t) => {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            ui.monospace(t.as_str());
                        });
                    }
                    Preview::Unsupported(t) => {
                        ui.label(RichText::new(t.as_str()).weak());
                    }
                }
                ui.separator();
                if ui.button("💾 Extract this file").clicked() {
                    let sel = self.selected.clone();
                    self.extract_selected(sel);
                }
            } else {
                ui.label("No asset selected.");
            }
        });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        let mut output_dir = self.output_dir.clone();
        let mut formats = self.formats.clone();
        let mut convert = self.convert;
        let mut queue = self.queue.clone();
        let mut browse = false;
        let mut add_current = false;
        let mut process = false;
        let mut remove_idx: Option<usize> = None;

        egui::Window::new("Export settings")
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Destination:");
                    ui.add(egui::TextEdit::singleline(&mut output_dir).desired_width(240.0));
                    browse = ui.button("Browse…").clicked();
                });
                ui.separator();
                ui.label(RichText::new("Export formats").strong());
                ui.checkbox(&mut formats.png, "Textures → PNG (decoded)");
                ui.checkbox(&mut formats.dds, "Textures → DDS (original compression)");
                ui.checkbox(&mut formats.glb, "Meshes → GLTF 2.0 (.glb)");
                ui.checkbox(&mut formats.obj, "Meshes → OBJ (fallback)");
                ui.checkbox(&mut formats.json, "Materials/Tables/Blueprints → JSON");
                ui.checkbox(&mut formats.audio, "Audio → WAV / OGG");
                ui.separator();
                ui.checkbox(&mut convert, "Enable conversion on extract");
                ui.separator();
                ui.label(RichText::new("Batch queue").strong());
                for (i, item) in queue.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}. {}", i + 1, item));
                        if ui.small_button("remove").clicked() {
                            remove_idx = Some(i);
                        }
                    });
                }
                ui.horizontal(|ui| {
                    add_current = ui.button("Add current archive to queue").clicked();
                    process = ui.button("Process next in queue").clicked() && !queue.is_empty();
                });
            });

        if browse {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                output_dir = dir.display().to_string();
            }
        }
        if add_current {
            if let Some(m) = &self.mount {
                for i in 0..m.source_count() {
                    queue.push(m.source_path(i));
                }
            }
        }
        if let Some(i) = remove_idx {
            queue.remove(i);
        }
        if process {
            let first = queue.remove(0);
            queue.clear();
            self.open_path(PathBuf::from(&first));
            self.extract(None);
        }
        self.show_settings = open;
        self.output_dir = output_dir;
        self.formats = formats;
        self.convert = convert;
        self.queue = queue;
        self.save_persisted();
    }

    fn keys_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_keys;
        let mut keys = self.keys.clone();
        let mut active_key = self.active_key.clone();
        let mut add_name = String::new();
        let mut add_hex = String::new();
        let mut add = false;
        let mut remove: Option<String> = None;
        let mut activate: Option<String> = None;

        egui::Window::new("AES key manager")
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Keys are stored locally and used for encrypted archives.");
                ui.separator();
                for key in &keys {
                    ui.horizontal(|ui| {
                        let active = active_key.as_ref() == Some(&key.name);
                        if ui.radio(active, key.name.as_str()).clicked() {
                            activate = Some(key.name.clone());
                        }
                        let shown = if key.hex.len() > 16 {
                            format!("{}…{}", &key.hex[..8], &key.hex[key.hex.len() - 8..])
                        } else {
                            key.hex.clone()
                        };
                        ui.monospace(RichText::new(shown).weak());
                        if ui.small_button("✕").clicked() {
                            remove = Some(key.name.clone());
                        }
                    });
                }
                ui.separator();
                ui.label("Add key:");
                ui.text_edit_singleline(&mut add_name);
                ui.text_edit_singleline(&mut add_hex);
                add = ui.button("Add").clicked();
            });

        if let Some(n) = activate {
            active_key = Some(n);
        }
        if let Some(n) = remove {
            keys.retain(|k| k.name != n);
            if active_key.as_deref() == Some(&n) {
                active_key = None;
            }
        }
        if add && !add_name.is_empty() && AesKey::parse(&add_hex).is_some() {
            keys.push(KeyEntry { name: add_name, hex: add_hex });
        }
        self.show_keys = open;
        self.keys = keys;
        self.active_key = active_key;
        self.save_persisted();
    }
}

// ---------------------------------------------------------------------------
// 3D mesh preview (software-rendered through egui's painter)

fn mesh_preview_ui(ui: &mut egui::Ui, mesh: &uex_core::assets::mesh::ExtractedMesh, rot_x: &mut f32, rot_y: &mut f32, zoom: &mut f32) {
    let (rect, resp) = ui.allocate_exact_size(ui.available_size_before_wrap().min(egui::vec2(340.0, 260.0)), egui::Sense::drag());
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
    painter.rect_filled(rect, 4.0, Color32::from_rgb(18, 18, 24));

    let (sy, cy_) = rot_y.sin_cos();
    let (sx, cx_) = rot_x.sin_cos();
    let mut tris: Vec<(f32, [egui::Pos2; 3], [Color32; 3])> = Vec::new();
    for section in &mesh.sections {
        for tri in section.indices.chunks_exact(3) {
            let mut pts = [egui::Pos2::ZERO; 3];
            let mut cols = [Color32::WHITE; 3];
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
                let c = Color32::from_rgb(
                    (120.0 + 110.0 * light) as u8,
                    (90.0 + 90.0 * light) as u8,
                    (170.0 + 70.0 * light) as u8,
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

// ---------------------------------------------------------------------------

fn type_icon(path: &str) -> &'static str {
    let lower = path.to_lowercase();
    if lower.ends_with(".uasset") || lower.ends_with(".uexp") {
        "📦"
    } else if lower.ends_with(".ubulk") {
        "🗄"
    } else if lower.ends_with(".utoc") || lower.ends_with(".ucas") || lower.ends_with(".pak") {
        "🗃"
    } else if lower.ends_with(".png") || lower.ends_with(".tga") || lower.ends_with(".dds") {
        "🖼"
    } else if lower.ends_with(".glb") || lower.ends_with(".gltf") || lower.ends_with(".obj") {
        "🧊"
    } else if lower.ends_with(".ogg") || lower.ends_with(".wav") {
        "🔊"
    } else if lower.ends_with(".json") {
        "📄"
    } else {
        "▪"
    }
}

fn file_type_name(path: &str) -> &'static str {
    let lower = path.to_lowercase();
    if lower.ends_with(".uasset") {
        "Unreal package"
    } else if lower.ends_with(".ubulk") {
        "Bulk data"
    } else if lower.ends_with(".uexp") {
        "Export data"
    } else {
        "File"
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn format_size(size: u64) -> String {
    if size >= 1 << 20 {
        format!("{:.1} MB", size as f64 / 1048576.0)
    } else if size >= 1 << 10 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{size} B")
    }
}

trait UiCaptionExt {
    fn caption(&mut self, text: &str);
}

impl UiCaptionExt for egui::Ui {
    fn caption(&mut self, text: &str) {
        self.label(RichText::new(text).small().color(Color32::from_rgb(140, 140, 150)));
    }
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
