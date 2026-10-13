//! All rendering: panels, tree, asset table, inspector, dialogs, theme usage.

use eframe::egui;
use egui::{Color32, RichText, Vec2};

use crate::app::{
    file_name, file_type_name, format_size, mesh_preview_ui, type_icon, App, KeyEntry, KeyPrompt,
    Status, PendingAction, Preview, SortCol, StatusKind, ViewMode, VirtualFile,
};
use crate::theme::{badge, ic, Palette};

/// The default egui font lacks some common glyphs (arrow, carets); route them
/// through phosphor so every label renders correctly.
fn arrow_right() -> &'static str {
    ic::ARROW
}

fn caret_up() -> &'static str {
    ic::CARET_UP
}

fn caret_down() -> &'static str {
    ic::CARET_DOWN
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
        if self.extracting {
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
        if self.show_errors {
            self.errors_window(ctx);
        }
        if self.extracting && self.show_progress {
            self.progress_window(ctx);
        }
        if self.key_prompt.is_some() {
            self.key_prompt_window(ctx);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_persisted();
    }
}

impl App {
    // ------------------------------------------------------------------ toolbar

    fn toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar")
            .frame(
                egui::Frame::default()
                    .fill(Palette::PANEL_HEADER)
                    .inner_margin(egui::Margin::symmetric(10, 7)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // brand
                    ui.label(
                        RichText::new("UExtractor")
                            .size(15.0)
                            .strong()
                            .color(Palette::ACCENT_TEXT),
                    );
                    ui.separator();

                    let open = icon_button(ui, ic::OPEN, "Open", "Open an archive (.pak, .utoc, .bundle, .assets, …)");
                    if open.clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Game archives", &["pak", "utoc", "bundle", "assetbundle", "unity3d", "assets"])
                            .add_filter("All files", &["*"])
                            .pick_file()
                        {
                            self.open_path(path);
                        }
                    }

                    let can_extract = self.mount.is_some() && !self.extracting;
                    if ui
                        .add_enabled(
                            can_extract,
                            egui::Button::new(
                                RichText::new(format!("{} Extract all", ic::EXTRACT)).size(14.0),
                            )
                            .fill(Palette::ACCENT_DIM)
                            .stroke(egui::Stroke::new(1.0, Palette::ACCENT)),
                        )
                        .on_hover_text("Extract every file in the archive")
                        .clicked()
                    {
                        self.extract(None);
                    }
                    if ui
                        .add_enabled(
                            can_extract && self.selected.is_some(),
                            egui::Button::new(RichText::new(format!("{} Selected", ic::EXTRACT_ONE)).size(14.0)),
                        )
                        .on_hover_text("Extract the file currently selected in the inspector")
                        .clicked()
                    {
                        let sel = self.selected.clone();
                        self.extract_selected(sel);
                    }

                    ui.separator();

                    // search box
                    ui.label(RichText::new(ic::SEARCH).size(15.0).color(Palette::TEXT_WEAK));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .desired_width(210.0)
                            .hint_text("Filter files…")
                            .font(egui::TextStyle::Body),
                    );
                    if !self.filter.is_empty() && ui.small_button(RichText::new(ic::CLOSE).size(12.0)).clicked() {
                        self.filter.clear();
                    }

                    ui.separator();
                    ui.toggle_value(&mut self.convert, RichText::new("Convert").size(14.0))
                        .on_hover_text("Convert assets on extract (textures → PNG/DDS, meshes → GLB, …)");

                    ui.separator();
                    if icon_button(ui, ic::KEYS, "Keys", "AES key manager — add or activate decryption keys").clicked() {
                        self.show_keys = !self.show_keys;
                    }
                    if icon_button(ui, ic::SETTINGS, "Settings", "Export destination, formats and batch queue").clicked() {
                        self.show_settings = !self.show_settings;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // engine badge
                        if let Some((glyph, label, color)) = self.engine_badge() {
                            badge(ui, &format!("{glyph} {label}"), color);
                        }
                        if !self.recent.is_empty() {
                            egui::ComboBox::from_id_salt("recent")
                                .selected_text(RichText::new(format!("{} Recent", ic::RECENT)).size(13.5))
                                .show_ui(ui, |ui| {
                                    for r in self.recent.clone() {
                                        if ui
                                            .selectable_label(false, RichText::new(file_name(&r)).size(13.0))
                                            .on_hover_text(&r)
                                            .clicked()
                                        {
                                            self.open_path(std::path::PathBuf::from(&r));
                                        }
                                    }
                                });
                        }
                    });
                });
            });
    }

    // --------------------------------------------------------------- status bar

    fn status_bar(&mut self, ctx: &egui::Context) {
        let n_errors = self.last_errors.len();
        egui::TopBottomPanel::bottom("status")
            .frame(
                egui::Frame::default()
                    .fill(Palette::PANEL_HEADER)
                    .inner_margin(egui::Margin::symmetric(10, 5)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // status dot + text
                    let (glyph, color) = match self.status.kind {
                        StatusKind::Info => (ic::INFO, Palette::TEXT_WEAK),
                        StatusKind::Ok => (ic::OK, Palette::GREEN),
                        StatusKind::Warn => (ic::WARNING, Palette::AMBER),
                        StatusKind::Err => (ic::ERROR, Palette::RED),
                        StatusKind::Busy => (ic::REFRESH, Palette::ACCENT),
                    };
                    ui.label(RichText::new(glyph).size(15.0).color(color));
                    ui.label(RichText::new(&self.status.text).size(13.0).color(match self.status.kind {
                        StatusKind::Err => Palette::RED,
                        StatusKind::Warn => Palette::AMBER,
                        StatusKind::Ok => Palette::GREEN,
                        _ => Palette::TEXT_WEAK,
                    }));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new("v1.2.0")
                                .small()
                                .color(Palette::TEXT_FAINT),
                        );
                        if n_errors > 0 {
                            let btn = egui::Button::new(
                                RichText::new(format!(
                                    "{} {n_errors} error{}",
                                    ic::ERROR,
                                    if n_errors == 1 { "" } else { "s" }
                                ))
                                .small()
                                .color(Palette::RED),
                            )
                            .fill(Palette::RED.gamma_multiply(0.12))
                            .stroke(egui::Stroke::new(1.0, Palette::RED.gamma_multiply(0.6)));
                            if ui.add(btn).clicked() {
                                self.show_errors = true;
                            }
                        }
                        if let Some(ev) = &self.last_progress {
                            if self.extracting {
                                let pct = if ev.total > 0 { ev.done as f32 / ev.total as f32 } else { 0.0 };
                                ui.add(
                                    egui::ProgressBar::new(pct)
                                        .show_percentage()
                                        .desired_width(200.0)
                                        .desired_height(14.0),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "{:.1} MB/s · ETA {:.0}s",
                                        ev.mbps, ev.eta_secs
                                    ))
                                    .small()
                                    .color(Palette::TEXT_WEAK),
                                );
                            }
                        }
                    });
                });
            });
    }

    // --------------------------------------------------------------- left panel

    fn left_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("tree")
            .default_width(300.0)
            .width_range(220.0..=420.0)
            .frame(egui::Frame::default().fill(Palette::PANEL).inner_margin(egui::Margin::same(8)))
            .show(ctx, |ui| {
                // header
                ui.horizontal(|ui| {
                    ui.label(RichText::new(ic::TREE).size(14.0).color(Palette::ACCENT_TEXT));
                    ui.label(RichText::new("ARCHIVE").size(13.0).strong().color(Palette::TEXT_WEAK));
                    if !self.archive_name.is_empty() {
                        badge(ui, &format!("{} files", self.flat_files.len()), Palette::ACCENT_DIM);
                    }
                });
                ui.add_space(4.0);
                if self.mount.is_none() {
                    ui.label(RichText::new("No archive loaded.").color(Palette::TEXT_FAINT));
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Supported: .pak · .utoc/.ucas · .bundle · .assets · data.unity3d")
                            .small()
                            .color(Palette::TEXT_FAINT),
                    );
                    return;
                }
                ui.label(
                    RichText::new(&self.archive_name)
                        .size(13.0)
                        .color(Palette::TEXT)
                        .strong(),
                )
                .on_hover_text(&self.archive_name);
                ui.separator();

                let mut to_select: Option<VirtualFile> = None;
                let mut to_toggle: Option<String> = None;
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    let selected_path = self.selected.as_ref().map(|s| s.path.clone());
                    let flt = self.filter.to_lowercase();
                    let mut tree = std::mem::take(&mut self.tree);
                    if !flt.is_empty() {
                        crate::app::filter_tree_pub(&mut tree, &flt);
                    }
                    render_tree_nodes(
                        ui,
                        &tree,
                        &mut self.expanded,
                        selected_path.as_deref(),
                        &mut to_select,
                        &mut to_toggle,
                    );
                    self.tree = tree;
                });
                if let Some(p) = to_toggle {
                    if self.expanded.contains(&p) {
                        self.expanded.remove(&p);
                    } else {
                        self.expanded.insert(p);
                    }
                }
                if let Some(vf) = to_select {
                    self.selected = Some(vf.clone());
                    self.load_preview(ctx, &vf);
                }
            });
    }

    // ------------------------------------------------------------- center panel

    fn center_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(Palette::BG).inner_margin(egui::Margin::same(10)))
            .show(ctx, |ui| {
                if self.mount.is_none() {
                    self.welcome(ui);
                    return;
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Assets").heading().strong());
                    ui.separator();
                    let list_sel = self.view_mode == ViewMode::List;
                    if ui
                        .selectable_label(list_sel, RichText::new(format!("{} List", ic::LIST)).size(14.0))
                        .clicked()
                    {
                        self.view_mode = ViewMode::List;
                    }
                    if ui
                        .selectable_label(!list_sel, RichText::new(format!("{} Grid", ic::GRID)).size(14.0))
                        .clicked()
                    {
                        self.view_mode = ViewMode::Grid;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let total: u64 = self.flat_files.iter().map(|f| f.size).sum();
                        badge(ui, &format!("{} files", self.flat_files.len()), Palette::ACCENT_DIM);
                        badge(ui, &format_size(total), Palette::CARD_HOVER);
                    });
                });
                ui.add_space(2.0);

                match self.view_mode {
                    ViewMode::List => self.asset_table(ui),
                    ViewMode::Grid => self.asset_grid(ctx, ui),
                }
            });
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(48.0);
            ui.label(RichText::new(ic::UNREAL).size(64.0).color(Palette::ACCENT));
            ui.add_space(6.0);
            ui.label(RichText::new("UExtractor").size(38.0).strong().color(Palette::TEXT));
            ui.label(
                RichText::new("Browse, preview and extract Unreal Engine & Unity game assets")
                    .size(15.0)
                    .color(Palette::TEXT_WEAK),
            );
            ui.add_space(18.0);

            // drop card
            let card = egui::Frame::default()
                .fill(Palette::CARD)
                .corner_radius(10.0)
                .stroke(egui::Stroke::new(1.0, Palette::ACCENT_DIM))
                .inner_margin(egui::Margin::symmetric(24, 14));
            card.show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(ic::DROP).size(22.0).color(Palette::ACCENT_TEXT));
                    ui.label(RichText::new("Drop an archive here — .pak · .utoc/.ucas · .bundle · data.unity3d · .assets").size(14.0));
                });
            });

            ui.add_space(18.0);
            let hint = |ui: &mut egui::Ui, glyph: &str, color: Color32, text: &str| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(glyph).size(15.0).color(color));
                    ui.label(RichText::new(text).size(13.5).color(Palette::TEXT_WEAK));
                });
            };
            hint(ui, ic::OPEN, Palette::ACCENT, "Open an archive with the toolbar button, or drag & drop");
            hint(ui, ic::LOCK, Palette::AMBER, "Encrypted archive? UExtractor asks for the AES key before extracting");
            hint(ui, ic::EXTRACT, Palette::GREEN, "Extract everything, or pick files in the browser and extract selectively");
            hint(ui, ic::MESH, Palette::C_MESH, "Preview textures and 3D meshes live before export");

            if !self.recent.is_empty() {
                ui.add_space(20.0);
                ui.label(RichText::new("RECENT").size(12.0).strong().color(Palette::TEXT_FAINT));
                for r in self.recent.clone() {
                    let (glyph, color) = type_icon(&r);
                    if ui
                        .link(RichText::new(format!("{glyph} {}", file_name(&r))).size(13.0))
                        .on_hover_text(&r)
                        .clicked()
                    {
                        self.open_path(std::path::PathBuf::from(r));
                    }
                }
            }
        });
    }

    fn asset_table(&mut self, ui: &mut egui::Ui) {
        // header row
        ui.horizontal(|ui| {
            let sort_btn = |ui: &mut egui::Ui, label: &str, col: SortCol, w: f32| -> bool {
                let active = self.sort_col == col;
                let arrow = if active {
                    if self.sort_asc { format!(" {}", caret_up()) } else { format!(" {}", caret_down()) }
                } else {
                    String::new()
                };
                ui.add_sized(
                    [w, 20.0],
                    egui::Button::new(
                        RichText::new(format!("{label}{arrow}"))
                            .size(12.5)
                            .strong()
                            .color(if active { Palette::ACCENT_TEXT } else { Palette::TEXT_WEAK }),
                    )
                    .fill(if active { Palette::CARD } else { Color32::TRANSPARENT }),
                ).clicked()
            };
            let click_name = sort_btn(ui, "NAME", SortCol::Name, 380.0);
            let click_type = sort_btn(ui, "TYPE", SortCol::Type, 110.0);
            let click_size = sort_btn(ui, "SIZE", SortCol::Size, 90.0);
            if click_name {
                if self.sort_col == SortCol::Name { self.sort_asc = !self.sort_asc } else { self.sort_col = SortCol::Name; self.sort_asc = true }
            }
            if click_type {
                if self.sort_col == SortCol::Type { self.sort_asc = !self.sort_asc } else { self.sort_col = SortCol::Type; self.sort_asc = true }
            }
            if click_size {
                if self.sort_col == SortCol::Size { self.sort_asc = !self.sort_asc } else { self.sort_col = SortCol::Size; self.sort_asc = true }
            }
        });
        ui.separator();

        let files = self.table_files();
        let sel_path = self.selected.as_ref().map(|s| s.path.clone());
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            egui::Grid::new("assets_grid")
                .num_columns(3)
                .striped(true)
                .min_col_width(90.0)
                .start_row(0)
                .show(ui, |ui| {
                    let mut clicked: Option<VirtualFile> = None;
                    for f in &files {
                        let (icon, color) = type_icon(&f.path);
                        let selected = sel_path.as_ref() == Some(&f.path);
                        if ui
                            .selectable_label(
                                selected,
                                RichText::new(icon).size(15.0).color(color),
                            )
                            .clicked()
                        {
                            clicked = Some(f.clone());
                        }
                        if ui
                            .selectable_label(
                                selected,
                                RichText::new(file_name(&f.path)).monospace().size(12.5),
                            )
                            .on_hover_text(&f.path)
                            .clicked()
                        {
                            clicked = Some(f.clone());
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format_size(f.size))
                                    .small()
                                    .color(Palette::TEXT_WEAK),
                            );
                            ui.label(RichText::new(file_type_name(&f.path)).small().color(Palette::TEXT_FAINT));
                        });
                        ui.end_row();
                    }
                    if let Some(vf) = clicked {
                        self.selected = Some(vf.clone());
                        self.load_preview(ui.ctx(), &vf);
                    }
                });
        });
    }

    fn asset_grid(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let files = self.table_files();
        let card_w = 136.0;
        let cols = (ui.available_width() / card_w).max(1.0) as usize;
        let sel_path = self.selected.as_ref().map(|s| s.path.clone());
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            egui::Grid::new("assets_cards").num_columns(cols).show(ui, |ui| {
                for f in &files {
                    ui.vertical(|ui| {
                        let selected = sel_path.as_ref() == Some(&f.path);
                        let (icon, color) = type_icon(&f.path);
                        let thumb: Option<&egui::TextureHandle> = self.textures.get(&f.path);
                        let cell = ui.allocate_ui(Vec2::new(card_w - 8.0, 118.0), |ui| {
                            let frame = egui::Frame::default()
                                .fill(if selected { Palette::ACCENT_DIM } else { Palette::CARD })
                                .corner_radius(6.0)
                                .stroke(egui::Stroke::new(
                                    1.0,
                                    if selected { Palette::ACCENT } else { Color32::TRANSPARENT },
                                ))
                                .inner_margin(4.0);
                            frame.show(ui, |ui| {
                                if let Some(tex) = thumb {
                                    ui.image(egui::load::SizedTexture::new(tex.id(), Vec2::new(116.0, 84.0)));
                                } else {
                                    ui.vertical_centered(|ui| {
                                        ui.add_space(22.0);
                                        ui.label(RichText::new(icon).size(34.0).color(color));
                                        ui.add_space(14.0);
                                    });
                                }
                                ui.add_sized(
                                    [card_w - 12.0, 14.0],
                                    egui::Label::new(RichText::new(file_name(&f.path)).small().color(Palette::TEXT))
                                        .truncate(),
                                );
                            });
                        });
                        if cell.response.clicked() {
                            self.selected = Some(f.clone());
                            self.load_preview(ctx, f);
                        }
                    });
                    ui.end_row();
                }
            });
        });
    }

    // ------------------------------------------------------------- right panel

    fn right_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("details")
            .default_width(380.0)
            .width_range(280.0..=520.0)
            .frame(egui::Frame::default().fill(Palette::PANEL).inner_margin(egui::Margin::same(10)))
            .show(ctx, |ui| {
                ui.label(RichText::new("INSPECTOR").size(13.0).strong().color(Palette::TEXT_WEAK));
                ui.separator();

                let Some(vf) = self.selected.clone() else {
                    ui.add_space(8.0);
                    ui.label(RichText::new("Select an asset to inspect and preview.").color(Palette::TEXT_FAINT));
                    return;
                };

                let (icon, color) = type_icon(&vf.path);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon).size(22.0).color(color));
                    ui.label(RichText::new(file_name(&vf.path)).size(15.0).strong());
                });
                ui.label(RichText::new(&vf.path).monospace().small().color(Palette::TEXT_WEAK));

                ui.add_space(6.0);
                // metadata grid
                egui::Grid::new("meta").num_columns(2).striped(true).show(ui, |ui| {
                    meta_row(ui, "Type", file_type_name(&vf.path).to_string());
                    meta_row(ui, "Size", format_size(vf.size));
                    if let Some(m) = &self.mount {
                        meta_row(ui, "Source", file_name(&m.source_path(vf.source)).to_string());
                    }
                    let encrypted = self.mount.as_ref().map(|m| m.is_file_encrypted(&vf)).unwrap_or(false);
                    if encrypted {
                        ui.label(RichText::new("Encryption").small().color(Palette::TEXT_FAINT));
                        ui.label(RichText::new(format!("{} AES-256", ic::LOCK)).small().color(Palette::AMBER));
                        ui.end_row();
                    }
                });
                ui.separator();

                match &mut self.preview {
                    Preview::None => {
                        ui.add_space(4.0);
                        ui.label(RichText::new("No preview loaded.").color(Palette::TEXT_FAINT));
                    }
                    Preview::Texture(tex) => {
                        let avail = ui.available_width();
                        let frame = egui::Frame::default()
                            .fill(Color32::from_rgb(0x10, 0x11, 0x16))
                            .corner_radius(6.0)
                            .inner_margin(6.0);
                        frame.show(ui, |ui| {
                            ui.image(egui::load::SizedTexture::new(tex.id(), Vec2::new(avail - 16.0, (avail - 16.0) * 0.75)));
                        });
                        ui.caption("Texture preview (decoded)");
                    }
                    Preview::Mesh { mesh, rot_x, rot_y, zoom } => {
                        let (rx_, ry_, z_) = (rot_x, rot_y, zoom);
                        mesh_preview_ui(ui, mesh, rx_, ry_, z_);
                        ui.caption("Drag to rotate · scroll to zoom");
                    }
                    Preview::Text(t) => {
                        egui::ScrollArea::vertical()
                            .auto_shrink(false)
                            .max_height(ui.available_height() - 60.0)
                            .show(ui, |ui| {
                                ui.monospace(RichText::new(t.as_str()).size(11.5).color(Palette::TEXT));
                            });
                    }
                    Preview::Unsupported(t) => {
                        ui.label(RichText::new(t.as_str()).weak());
                    }
                }

                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(6.0);
                    let btn = egui::Button::new(
                        RichText::new(format!("{} Extract this file", ic::EXTRACT_ONE)).size(14.0).strong(),
                    )
                    .fill(Palette::ACCENT)
                    .min_size(Vec2::new(ui.available_width(), 30.0));
                    if ui.add(btn).clicked() {
                        let sel = self.selected.clone();
                        self.extract_selected(sel);
                    }
                });
            });
    }

    // ------------------------------------------------------------------ dialogs

    fn key_prompt_window(&mut self, ctx: &egui::Context) {
        let mut prompt = match self.key_prompt.take() {
            Some(p) => p,
            None => return,
        };
        let mut open = true;
        let valid = KeyPrompt::input_valid(&prompt.input);
        let mut save = false;
        let mut cancel = false;
        let mut extract_unenc = false;

        egui::Window::new(
            RichText::new(format!("{} AES Key Required", ic::LOCK))
                .heading()
                .strong()
                .color(Palette::AMBER),
        )
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.add_space(4.0);
            ui.label(RichText::new(&prompt.reason).size(14.0));
            if let Some(guid) = &prompt.guid_hint {
                ui.label(
                    RichText::new(format!("Archive key GUID: {guid}"))
                        .small()
                        .color(Palette::TEXT_FAINT),
                );
            }
            ui.add_space(8.0);
            ui.label(RichText::new("Encryption key (64 hex characters): ").size(13.0));
            let input = egui::TextEdit::singleline(&mut prompt.input)
                .desired_width(560.0)
                .font(egui::TextStyle::Monospace)
                .hint_text("0x… paste the game's AES-256 key");
            ui.add(input);
            ui.horizontal(|ui| {
                let len = prompt
                    .input
                    .trim()
                    .trim_start_matches("0x")
                    .replace(['-', ' '], "")
                    .len();
                if valid {
                    ui.label(
                        RichText::new(format!("{} Valid 32-byte key", ic::OK))
                            .small()
                            .color(Palette::GREEN),
                    );
                } else {
                    ui.label(
                        RichText::new(format!("{len}/64 hex characters"))
                            .small()
                            .color(if len == 64 { Palette::RED } else { Palette::TEXT_FAINT }),
                    );
                }
            });
            if let Some(err) = &prompt.error {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("{} {err}", ic::ERROR))
                        .small()
                        .color(Palette::RED),
                );
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                save = ui
                    .add(
                        egui::Button::new(RichText::new(format!("{} Save key & continue", ic::OK)).strong())
                            .fill(Palette::ACCENT)
                            .min_size(Vec2::new(190.0, 28.0)),
                    )
                    .clicked();
                // "extract unencrypted only" is only meaningful when the archive is
                // already open and only PART of it is encrypted
                if let Some(mount) = &self.mount {
                    let enc = mount.encrypted_file_count();
                    if enc > 0 && enc < mount.file_count() as u64 {
                        extract_unenc = ui
                            .button(format!("{} Extract unencrypted only", ic::EXTRACT))
                            .on_hover_text("Skip encrypted files — extract what is readable now")
                            .clicked();
                    }
                }
                cancel = ui.button("Cancel").clicked();
            });
        });

        if !open || cancel {
            self.status = Status::new(StatusKind::Info, "Cancelled — no key applied.");
            return; // prompt dropped
        }
        if save && valid {
            let hex = prompt.input.clone();
            match self.apply_key(hex) {
                Ok(()) => {
                    let action = prompt.action;
                    self.perform_pending(action);
                    self.status = Status::new(
                        StatusKind::Ok,
                        "AES key accepted and verified against an encrypted file.",
                    );
                    return;
                }
                Err(err) => {
                    prompt.error = Some(err);
                }
            }
        } else if save && !valid {
            prompt.error = Some("Key must be exactly 64 hex characters (32 bytes).".into());
        }
        if extract_unenc {
            let action = prompt.action;
            self.status = Status::new(StatusKind::Info, "Skipping encrypted files.");
            self.start_extraction(None, true);
            let _ = action;
            return;
        }
        self.key_prompt = Some(prompt);
    }

    fn perform_pending(&mut self, action: PendingAction) {
        match action {
            PendingAction::Reopen(path) => {
                self.open_path(path);
            }
            PendingAction::ExtractAll => {
                self.start_extraction(None, false);
            }
            PendingAction::ExtractSelected(vf) => {
                self.extract_one_now(&vf);
            }
            PendingAction::Nothing => {}
        }
    }

    fn progress_window(&mut self, ctx: &egui::Context) {
        let mut cancel = false;
        egui::Window::new(RichText::new(format!("{} Extracting…", ic::EXTRACT)).heading().strong())
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -40.0])
            .show(ctx, |ui| {
                if let Some(ev) = &self.last_progress {
                    let pct = if ev.total > 0 { ev.done as f32 / ev.total as f32 } else { 0.0 };
                    ui.add(
                        egui::ProgressBar::new(pct)
                            .show_percentage()
                            .desired_width(420.0)
                            .desired_height(18.0),
                    );
                    ui.add_space(4.0);
                    egui::Grid::new("prog_stats").num_columns(2).show(ui, |ui| {
                        prog_row(ui, "Files", format!("{} / {}", ev.done, ev.total));
                        prog_row(ui, "Speed", format!("{:.1} MB/s", ev.mbps));
                        prog_row(ui, "ETA", format!("{:.0}s", ev.eta_secs));
                        prog_row(ui, "Errors", ev.errors.to_string());
                    });
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(if ev.current.len() > 60 {
                            format!("…{}", &ev.current[ev.current.len() - 59..])
                        } else {
                            ev.current.clone()
                        })
                        .monospace()
                        .small()
                        .color(Palette::TEXT_WEAK),
                    );
                }
                ui.add_space(6.0);
                cancel = ui
                    .add(
                        egui::Button::new(RichText::new(format!("{} Cancel extraction", ic::CANCEL)).color(Palette::RED))
                            .stroke(egui::Stroke::new(1.0, Palette::RED.gamma_multiply(0.6))),
                    )
                    .clicked();
            });
        if cancel {
            self.cancel_extraction();
        }
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
        let mut reload_archive = false;

        egui::Window::new(RichText::new(format!("{} AES Key Manager", ic::KEYS)).heading().strong())
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("Keys decrypt encrypted archives. The active key is used when opening and extracting.")
                        .color(Palette::TEXT_WEAK),
                );
                ui.separator();
                if keys.is_empty() {
                    ui.label(RichText::new("No keys saved yet.").color(Palette::TEXT_FAINT));
                }
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
                        ui.monospace(RichText::new(shown).small().color(Palette::TEXT_WEAK));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button(RichText::new(ic::TRASH).size(12.0)).on_hover_text("Remove key").clicked() {
                                remove = Some(key.name.clone());
                            }
                            if active {
                                badge(ui, "ACTIVE", Palette::GREEN);
                            }
                        });
                    });
                }
                ui.separator();
                ui.label(RichText::new("Add key").strong());
                ui.horizontal(|ui| {
                    ui.add_sized([160.0, 22.0], egui::TextEdit::singleline(&mut add_name).hint_text("Name"));
                    ui.add_sized([380.0, 22.0], egui::TextEdit::singleline(&mut add_hex).hint_text("64 hex characters").font(egui::TextStyle::Monospace));
                });
                add = ui
                    .add(egui::Button::new(format!("{} Add key", ic::PLUS)).min_size(Vec2::new(110.0, 24.0)))
                    .clicked();
                if !self.archive_name.is_empty() {
                    ui.add_space(4.0);
                    reload_archive = ui
                        .button(format!("{} Reload archive with active key", ic::REFRESH))
                        .clicked();
                }
            });

        if let Some(n) = activate {
            active_key = Some(n);
            reload_archive = self.mount.is_some();
        }
        if let Some(n) = remove {
            keys.retain(|k| k.name != n);
            if active_key.as_deref() == Some(&n) {
                active_key = None;
            }
        }
        if add && !add_name.is_empty() && KeyPrompt::input_valid(&add_hex) {
            keys.push(KeyEntry { name: add_name, hex: add_hex.trim().trim_start_matches("0x").replace(['-', ' '], "").to_lowercase() });
            if active_key.is_none() {
                active_key = keys.last().map(|k| k.name.clone());
            }
        }
        self.show_keys = open;
        self.keys = keys;
        self.active_key = active_key;
        self.save_persisted();
        if reload_archive {
            // reopen the most recent archive with the (possibly new) active key
            if let Some(rec) = self.recent.first().cloned() {
                let path = std::path::PathBuf::from(&rec);
                self.open_path(path);
            }
        }
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

        egui::Window::new(RichText::new(format!("{} Export Settings", ic::SETTINGS)).heading().strong())
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(RichText::new("Destination").strong());
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut output_dir)
                            .desired_width(340.0)
                            .font(egui::TextStyle::Monospace),
                    );
                    browse = ui.button("Browse…").clicked();
                });
                ui.separator();
                ui.label(RichText::new("Export formats").strong());
                egui::Grid::new("formats").num_columns(2).show(ui, |ui| {
                    ui.checkbox(&mut formats.png, format!("Textures {} PNG (decoded)", arrow_right()));
                    ui.checkbox(&mut formats.dds, format!("Textures {} DDS (original compression)", arrow_right()));
                    ui.end_row();
                    ui.checkbox(&mut formats.glb, format!("Meshes {} GLTF 2.0 (.glb)", arrow_right()));
                    ui.checkbox(&mut formats.obj, format!("Meshes {} OBJ (fallback)", arrow_right()));
                    ui.end_row();
                    ui.checkbox(&mut formats.json, format!("Metadata {} JSON", arrow_right()));
                    ui.checkbox(&mut formats.audio, format!("Audio {} WAV / OGG", arrow_right()));
                    ui.end_row();
                });
                ui.separator();
                ui.checkbox(&mut convert, format!("{} Enable conversion on extract", ic::ROCKET));
                ui.separator();
                ui.label(RichText::new("Batch queue").strong());
                for (i, item) in queue.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}. {}", i + 1, file_name(item)));
                        if ui.small_button(RichText::new(ic::CLOSE).size(11.0)).clicked() {
                            remove_idx = Some(i);
                        }
                    });
                }
                ui.horizontal(|ui| {
                    add_current = ui.button(format!("{} Add current archive", ic::PLUS)).clicked();
                    process = ui
                        .add_enabled(!queue.is_empty(), egui::Button::new(format!("{} Process next", ic::PLAY)))
                        .clicked();
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
            self.open_path(std::path::PathBuf::from(&first));
            self.extract(None);
        }
        self.show_settings = open;
        self.output_dir = output_dir;
        self.formats = formats;
        self.convert = convert;
        self.queue = queue;
        self.save_persisted();
    }

    fn errors_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_errors;
        egui::Window::new(
            RichText::new(format!("{} Extraction Errors", ic::ERROR))
                .heading()
                .strong()
                .color(Palette::RED),
        )
        .open(&mut open)
        .collapsible(false)
        .default_width(640.0)
        .show(ctx, |ui| {
            ui.label(
                RichText::new(
                    "Most failures are encrypted files without a valid key, or damaged archives.",
                )
                .color(Palette::TEXT_WEAK),
            );
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                if self.last_errors.is_empty() {
                    ui.label("No errors recorded.");
                }
                for (i, e) in self.last_errors.iter().enumerate() {
                    ui.label(RichText::new(format!("{}. {e}", i + 1)).monospace().small().color(Palette::TEXT));
                    ui.add_space(2.0);
                }
            });
        });
        self.show_errors = open;
    }

    // ---------------------------------------------------------------- autoshot

    fn autoshot_tick(&mut self, ctx: &egui::Context) {
        let Some(shot) = &self.autoshot else { return };
        let stage = shot.stage;
        let out_dir = shot.out_dir.clone();
        let archive = shot.archive.clone();
        let big = shot.big_archive.clone();
        match stage {
            0 => {
                self.open_path(archive);
                self.next_stage();
            }
            1 => {
                // if opening failed for lack of a key, capture the prompt instead
                if self.key_prompt.is_some() {
                    self.tick_and_capture(ctx, &out_dir, "05_key_prompt.png", 40);
                } else {
                    self.expand_all();
                    self.next_stage();
                }
            }
            2 => {
                if self.flat_files.is_empty() {
                    // key prompt flow: we're done after capturing it
                    self.save_persisted();
                    std::process::exit(0);
                }
                self.tick_and_capture(ctx, &out_dir, "01_main_window.png", 40);
            }
            3 => {
                let target = self
                    .flat_files
                    .iter()
                    .find(|f| {
                        f.path.ends_with(".png")
                            || f.path.ends_with(".uasset")
                            || f.path.ends_with(".glb")
                            || f.path.contains("CAB-")
                            || f.path.ends_with(".assets")
                            || f.path.starts_with("level")
                            || f.path.starts_with("sharedassets")
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
                self.save_persisted();
                std::process::exit(0);
            }
        }
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
}

// ---------------------------------------------------------------------------
// small ui helpers

fn icon_button(ui: &mut egui::Ui, glyph: &str, label: &str, tip: &str) -> egui::Response {
    ui.button(RichText::new(format!("{glyph} {label}")).size(14.0))
        .on_hover_text(tip)
}

fn meta_row(ui: &mut egui::Ui, k: &str, v: String) {
    ui.label(RichText::new(k).small().color(Palette::TEXT_FAINT));
    ui.label(RichText::new(v).small().color(Palette::TEXT));
    ui.end_row();
}

fn prog_row(ui: &mut egui::Ui, k: &str, v: String) {
    ui.label(RichText::new(k).small().color(Palette::TEXT_FAINT));
    ui.label(RichText::new(v).small().color(Palette::TEXT));
    ui.end_row();
}

fn render_tree_nodes(
    ui: &mut egui::Ui,
    nodes: &[crate::app::TreeNode],
    expanded: &mut std::collections::HashSet<String>,
    selected_path: Option<&str>,
    to_select: &mut Option<VirtualFile>,
    to_toggle: &mut Option<String>,
) {
    for node in nodes {
        let has_children = !node.children.is_empty();
        if has_children {
            let open = expanded.contains(&node.full_path);
            let glyph = if open { ic::FOLDER_OPEN } else { ic::FOLDER };
            let resp = ui.selectable_label(
                false,
                RichText::new(format!("{glyph} {}", node.name))
                    .size(13.5)
                    .color(Palette::ACCENT_TEXT),
            );
            if resp.clicked() {
                *to_toggle = Some(node.full_path.clone());
            }
            if open {
                ui.indent(node.full_path.as_str(), |ui| {
                    render_tree_nodes(ui, &node.children, expanded, selected_path, to_select, to_toggle);
                });
            }
        } else if node.file.is_some() {
            let (icon, color) = type_icon(&node.name);
            let selected = selected_path.map(|p| node.full_path.ends_with(p)).unwrap_or(false);
            if ui
                .selectable_label(
                    selected,
                    RichText::new(format!("{icon} {}", node.name))
                        .size(13.0)
                        .color(if selected { Palette::ACCENT_TEXT } else { Palette::TEXT }),
                )
                .clicked()
            {
                *to_select = node.file.clone();
            }
        }
    }
}

trait UiCaptionExt {
    fn caption(&mut self, text: &str);
}

impl UiCaptionExt for egui::Ui {
    fn caption(&mut self, text: &str) {
        self.label(RichText::new(text).small().color(Color32::from_rgb(0x9a, 0xa0, 0xb0)));
    }
}

// referenced helper so `ic::TREE` etc stay private-friendly
pub(crate) fn _noop() {}
