//! All rendering: panels, tree, asset table, inspector, dialogs, theme usage.
//!
//! Design rules (v1.3):
//! * one primary accent (UE blue); semantic colors only for status
//! * standard egui text styles only — no custom named styles (panic-safe)
//! * flat surfaces, hairline separators, calm hierarchy

use eframe::egui;
use egui::{Align2, Color32, FontId, RichText, Sense, Vec2};

use crate::app::{
    file_name, file_type_name, format_size, mesh_preview_ui, type_icon, App, KeyEntry, KeyPrompt,
    PendingAction, Preview, SortCol, Status, StatusKind, ViewMode, VirtualFile,
};
use crate::theme::{badge, counter, ic, Palette};

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
        // archive passed as CLI argument / queued on launch
        if let Some(p) = self.pending_open.take() {
            self.open_path(p);
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
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(2.0);
                    // brand mark: rounded accent square with "U"
                    let (mark, _) = ui.allocate_exact_size(Vec2::new(20.0, 20.0), Sense::hover());
                    ui.painter().rect_filled(mark, 5.0, Palette::ACCENT);
                    ui.painter().text(
                        mark.center(),
                        Align2::CENTER_CENTER,
                        "U",
                        FontId::proportional(12.5),
                        Color32::WHITE,
                    );
                    ui.label(
                        RichText::new("UExtractor")
                            .size(13.5)
                            .strong()
                            .color(Palette::TEXT_STRONG),
                    );
                    ui.add_space(6.0);

                    // Open + recents chevron
                    if ui
                        .button(format!("{} Open", ic::OPEN))
                        .on_hover_text("Open an archive (.pak, .utoc, .bundle, .assets, …)")
                        .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter(
                                "Game archives",
                                &["pak", "utoc", "bundle", "assetbundle", "unity3d", "assets"],
                            )
                            .add_filter("All files", &["*"])
                            .pick_file()
                        {
                            self.open_path(path);
                        }
                    }
                    if !self.recent.is_empty() {
                        ui.menu_button(RichText::new(ic::CARET_DOWN).size(11.0), |ui| {
                            ui.set_min_width(260.0);
                            ui.label(
                                RichText::new("RECENT ARCHIVES")
                                    .size(10.5)
                                    .strong()
                                    .color(Palette::TEXT_FAINT),
                            );
                            for r in self.recent.clone() {
                                let (icon, color) = type_icon(&r);
                                if ui
                                    .button(
                                        RichText::new(format!("{icon} {}", file_name(&r)))
                                            .size(12.5)
                                            .color(color),
                                    )
                                    .on_hover_text(&r)
                                    .clicked()
                                {
                                    self.open_path(std::path::PathBuf::from(&r));
                                    ui.close();
                                }
                            }
                        })
                        .response
                        .on_hover_text("Recently opened archives");
                    }

                    ui.add_space(4.0);

                    // primary action
                    let can_extract = self.mount.is_some() && !self.extracting;
                    if ui
                        .add_enabled(
                            can_extract,
                            egui::Button::new(
                                RichText::new(format!("{} Extract all", ic::EXTRACT)).strong(),
                            )
                            .fill(Palette::ACCENT),
                        )
                        .on_hover_text("Extract every file in the archive")
                        .clicked()
                    {
                        self.extract(None);
                    }
                    if ui
                        .add_enabled(
                            can_extract && self.selected.is_some(),
                            egui::Button::new(format!("{} Selected", ic::EXTRACT_ONE)),
                        )
                        .on_hover_text("Extract the file selected in the inspector")
                        .clicked()
                    {
                        let sel = self.selected.clone();
                        self.extract_selected(sel);
                    }

                    ui.add_space(6.0);

                    // search
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .desired_width(230.0)
                            .hint_text("Search files…"),
                    );
                    if !self.filter.is_empty()
                        && ui
                            .small_button(RichText::new(ic::CLOSE).size(11.0))
                            .on_hover_text("Clear search")
                            .clicked()
                    {
                        self.filter.clear();
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_button(ui, ic::SETTINGS, "Settings", "Export destination, formats and batch queue")
                            .clicked()
                        {
                            self.show_settings = !self.show_settings;
                        }
                        if icon_button(ui, ic::KEYS, "Keys", "AES key manager — add or activate decryption keys")
                            .clicked()
                        {
                            self.show_keys = !self.show_keys;
                        }
                        // segmented view toggle
                        let list = self.view_mode == ViewMode::List;
                        let seg = egui::Frame::default()
                            .fill(Palette::CARD)
                            .corner_radius(5.0)
                            .inner_margin(egui::Margin::same(2));
                        seg.show(ui, |ui| {
                            if ui
                                .selectable_label(list, RichText::new(ic::LIST).size(13.5))
                                .on_hover_text("List view")
                                .clicked()
                            {
                                self.view_mode = ViewMode::List;
                            }
                            if ui
                                .selectable_label(!list, RichText::new(ic::GRID).size(13.5))
                                .on_hover_text("Grid view")
                                .clicked()
                            {
                                self.view_mode = ViewMode::Grid;
                            }
                        });
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
                    .inner_margin(egui::Margin::symmetric(12, 4)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // status dot
                    let color = match self.status.kind {
                        StatusKind::Info => Palette::TEXT_FAINT,
                        StatusKind::Ok => Palette::GREEN,
                        StatusKind::Warn => Palette::AMBER,
                        StatusKind::Err => Palette::RED,
                        StatusKind::Busy => Palette::ACCENT,
                    };
                    let (dot, _) = ui.allocate_exact_size(Vec2::new(8.0, 8.0), Sense::hover());
                    ui.painter()
                        .circle_filled(dot.center(), 3.0, color);
                    ui.label(
                        RichText::new(&self.status.text)
                            .size(12.0)
                            .color(match self.status.kind {
                                StatusKind::Err => Palette::RED,
                                StatusKind::Warn => Palette::AMBER,
                                StatusKind::Ok => Palette::GREEN,
                                _ => Palette::TEXT_WEAK,
                            }),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                                .size(10.5)
                                .color(Palette::TEXT_FAINT),
                        );
                        if n_errors > 0 {
                            let chip = egui::Button::new(
                                RichText::new(format!(
                                    "{} {n_errors}",
                                    ic::ERROR
                                ))
                                .size(10.5)
                                .color(Palette::RED),
                            )
                            .fill(Palette::RED.gamma_multiply(0.14))
                            .corner_radius(4.0);
                            if ui
                                .add(chip)
                                .on_hover_text("Show extraction errors")
                                .clicked()
                            {
                                self.show_errors = true;
                            }
                        }
                        if let Some(ev) = &self.last_progress {
                            if self.extracting {
                                let pct = if ev.total > 0 {
                                    ev.done as f32 / ev.total as f32
                                } else {
                                    0.0
                                };
                                ui.add(
                                    egui::ProgressBar::new(pct)
                                        .desired_width(150.0)
                                        .desired_height(10.0),
                                );
                                ui.label(
                                    RichText::new(format!("{:.1} MB/s", ev.mbps))
                                        .size(10.5)
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
            .default_width(280.0)
            .width_range(200.0..=420.0)
            .frame(egui::Frame::default().fill(Palette::PANEL))
            .show(ctx, |ui| {
                if self.mount.is_none() {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new("No archive loaded")
                                    .size(12.5)
                                    .color(Palette::TEXT_FAINT),
                            );
                            ui.label(
                                RichText::new(
                                    "Supported: .pak · .utoc/.ucas · .bundle · data.unity3d · .assets",
                                )
                                .size(11.0)
                                .color(Palette::TEXT_FAINT),
                            );
                        });
                    });
                    return;
                }

                // header block
                egui::Frame::default()
                    .fill(Palette::PANEL)
                    .inner_margin(egui::Margin::symmetric(12, 9))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            RichText::new(&self.archive_name)
                                .size(13.0)
                                .strong()
                                .color(Palette::TEXT_STRONG),
                        )
                        .on_hover_text(&self.archive_name);
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            if let Some((glyph, label, color)) = self.engine_badge() {
                                ui.label(RichText::new(glyph).size(12.0).color(color));
                                ui.label(
                                    RichText::new(label).size(11.0).color(Palette::TEXT_WEAK),
                                );
                            }
                            counter(ui, &format!("{} files", self.flat_files.len()));
                        });
                    });
                ui.separator();

                let mut to_select: Option<VirtualFile> = None;
                let mut to_toggle: Option<String> = None;
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let selected_path = self.selected.as_ref().map(|s| s.path.clone());
                    let flt = self.filter.to_lowercase();
                    let mut tree = std::mem::take(&mut self.tree);
                    if !flt.is_empty() {
                        crate::app::filter_tree_pub(&mut tree, &flt);
                    }
                    egui::Frame::default()
                        .inner_margin(egui::Margin::symmetric(8, 6))
                        .show(ui, |ui| {
                            render_tree_nodes(
                                ui,
                                &tree,
                                &mut self.expanded,
                                selected_path.as_deref(),
                                &mut to_select,
                                &mut to_toggle,
                            );
                        });
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
            .frame(egui::Frame::default().fill(Palette::BG).inner_margin(egui::Margin::same(12)))
            .show(ctx, |ui| {
                if self.mount.is_none() {
                    self.welcome(ui);
                    return;
                }
                // slim header row
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Assets").size(14.5).strong().color(Palette::TEXT_STRONG));
                    let total: u64 = self.flat_files.iter().map(|f| f.size).sum();
                    counter(
                        ui,
                        &format!("{} files · {}", self.flat_files.len(), format_size(total)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let list = self.view_mode == ViewMode::List;
                        let seg = egui::Frame::default()
                            .fill(Palette::CARD)
                            .corner_radius(5.0)
                            .inner_margin(egui::Margin::same(2));
                        seg.show(ui, |ui| {
                            if ui
                                .selectable_label(list, RichText::new(ic::LIST).size(13.5))
                                .on_hover_text("List view")
                                .clicked()
                            {
                                self.view_mode = ViewMode::List;
                            }
                            if ui
                                .selectable_label(!list, RichText::new(ic::GRID).size(13.5))
                                .on_hover_text("Grid view")
                                .clicked()
                            {
                                self.view_mode = ViewMode::Grid;
                            }
                        });
                    });
                });
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);

                match self.view_mode {
                    ViewMode::List => self.asset_table(ui),
                    ViewMode::Grid => self.asset_grid(ctx, ui),
                }
            });
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(56.0);
            // logo mark
            let (mark, _) = ui.allocate_exact_size(Vec2::new(56.0, 56.0), Sense::hover());
            ui.painter().rect_filled(mark, 14.0, Palette::ACCENT);
            ui.painter().text(
                mark.center(),
                Align2::CENTER_CENTER,
                "U",
                FontId::proportional(30.0),
                Color32::WHITE,
            );
            ui.add_space(10.0);
            ui.label(
                RichText::new("UExtractor")
                    .size(24.0)
                    .strong()
                    .color(Palette::TEXT_STRONG),
            );
            ui.label(
                RichText::new("Browse, preview and extract Unreal Engine & Unity game assets")
                    .size(13.5)
                    .color(Palette::TEXT_WEAK),
            );
            ui.add_space(22.0);

            // drop card
            egui::Frame::default()
                .fill(Palette::CARD)
                .corner_radius(8.0)
                .stroke(egui::Stroke::new(1.0, Palette::LINE))
                .inner_margin(egui::Margin::symmetric(26, 13))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(ic::DROP).size(18.0).color(Palette::ACCENT_TEXT),
                        );
                        ui.label(
                            RichText::new("Drop an archive here, or click Open")
                                .size(13.0)
                                .color(Palette::TEXT),
                        );
                    });
                });

            ui.add_space(20.0);
            // three feature hints in a row
            let hints: [(&str, &str, Color32); 3] = [
                (ic::UNREAL, "UE4 .pak & UE5 IOStore — models, textures, audio, data", Palette::ACCENT),
                (ic::UNITY, "Unity bundles & serialized files — all engine versions", Palette::MAGENTA),
                (ic::LOCK, "Encrypted archives? The AES key is requested before extraction", Palette::AMBER),
            ];
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 32.0) / 3.0;
                for (glyph, text, color) in hints {
                    ui.allocate_ui(Vec2::new(w, 0.0), |ui| {
                        ui.vertical(|ui| {
                            ui.add_space(2.0);
                            ui.label(RichText::new(glyph).size(16.0).color(color));
                            ui.add_space(2.0);
                            ui.label(RichText::new(text).size(11.5).color(Palette::TEXT_WEAK));
                        });
                    });
                }
            });

            if !self.recent.is_empty() {
                ui.add_space(26.0);
                ui.label(
                    RichText::new("RECENT")
                        .size(10.5)
                        .strong()
                        .color(Palette::TEXT_FAINT),
                );
                ui.add_space(2.0);
                let recents: Vec<String> = self.recent.iter().take(5).cloned().collect();
                for r in &recents {
                    let (glyph, color) = type_icon(r);
                    let path = r.clone();
                    if ui
                        .link(
                            RichText::new(format!("{glyph} {}", file_name(r)))
                                .size(12.5)
                                .color(color),
                        )
                        .on_hover_text(r)
                        .clicked()
                    {
                        self.open_path(std::path::PathBuf::from(path));
                    }
                }
            }
        });
    }

    fn asset_table(&mut self, ui: &mut egui::Ui) {
        // local sort copies so header + body closures don't fight over `self`
        let mut sort_col = self.sort_col;
        let mut sort_asc = self.sort_asc;

        let files = self.table_files();
        let sel_path = self.selected.as_ref().map(|s| s.path.clone());
        let mut to_select: Option<VirtualFile> = None;

        // zero horizontal cell spacing so the row highlight is continuous
        ui.allocate_ui(ui.available_size(), |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            egui_extras::TableBuilder::new(ui)
                .striped(false)
                .vscroll(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(egui_extras::Column::remainder().resizable(true).clip(true))
                .column(egui_extras::Column::auto())
                .column(egui_extras::Column::auto())
                .header(26.0, |mut header| {
                    // Name
                    header.col(|ui| {
                        if sort_header(ui, "Name", sort_col == SortCol::Name, sort_asc, false)
                            .clicked()
                        {
                            if sort_col == SortCol::Name { sort_asc = !sort_asc } else { sort_col = SortCol::Name; sort_asc = true }
                        }
                    });
                    // Type
                    header.col(|ui| {
                        if sort_header(ui, "Type", sort_col == SortCol::Type, sort_asc, false)
                            .clicked()
                        {
                            if sort_col == SortCol::Type { sort_asc = !sort_asc } else { sort_col = SortCol::Type; sort_asc = true }
                        }
                    });
                    // Size
                    header.col(|ui| {
                        if sort_header(ui, "Size", sort_col == SortCol::Size, sort_asc, true)
                            .clicked()
                        {
                            if sort_col == SortCol::Size { sort_asc = !sort_asc } else { sort_col = SortCol::Size; sort_asc = true }
                        }
                    });
                })
                .body(|mut body| {
                    body.rows(26.0, files.len(), |mut row| {
                        let f = &files[row.index()];
                        let selected = sel_path.as_ref() == Some(&f.path);
                        row.set_selected(selected);
                        // name cell: icon + filename
                        row.col(|ui| {
                            ui.add_space(8.0);
                            let (icon, color) = type_icon(&f.path);
                            ui.label(RichText::new(icon).size(14.0).color(color));
                            ui.add_space(7.0);
                            let name = RichText::new(file_name(&f.path))
                                .size(12.5)
                                .color(if selected { Palette::TEXT_STRONG } else { Palette::TEXT });
                            ui.add(egui::Label::new(name).truncate()).on_hover_text(&f.path);
                        });
                        // type cell
                        row.col(|ui| {
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new(file_type_name(&f.path))
                                    .size(12.0)
                                    .color(Palette::TEXT_WEAK),
                            );
                        });
                        // size cell (right-aligned)
                        row.col(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add_space(12.0);
                                ui.label(
                                    RichText::new(format_size(f.size))
                                        .size(12.0)
                                        .color(Palette::TEXT_WEAK),
                                );
                            });
                        });
                        if row.response().clicked() {
                            to_select = Some(f.clone());
                        }
                    });
                });
        });

        self.sort_col = sort_col;
        self.sort_asc = sort_asc;
        if let Some(vf) = to_select {
            self.selected = Some(vf.clone());
            self.load_preview(ui.ctx(), &vf);
        }
    }

    fn asset_grid(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let files = self.table_files();
        let card_w = 132.0;
        let cols = ((ui.available_width() / card_w).max(1.0) as usize).max(1);
        let sel_path = self.selected.as_ref().map(|s| s.path.clone());
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
            egui::Grid::new("assets_cards").num_columns(cols).show(ui, |ui| {
                for f in &files {
                    ui.vertical(|ui| {
                        let selected = sel_path.as_ref() == Some(&f.path);
                        let (icon, color) = type_icon(&f.path);
                        let thumb: Option<&egui::TextureHandle> = self.textures.get(&f.path);
                        let cell = ui.allocate_ui(Vec2::new(card_w - 8.0, 112.0), |ui| {
                            let frame = egui::Frame::default()
                                .fill(if selected { Palette::ROW_SEL } else { Palette::CARD })
                                .corner_radius(5.0)
                                .inner_margin(4.0);
                            frame.show(ui, |ui| {
                                if let Some(tex) = thumb {
                                    ui.image(egui::load::SizedTexture::new(
                                        tex.id(),
                                        Vec2::new(card_w - 16.0, 78.0),
                                    ));
                                } else {
                                    ui.vertical_centered(|ui| {
                                        ui.add_space(20.0);
                                        ui.label(RichText::new(icon).size(30.0).color(color));
                                        ui.add_space(12.0);
                                    });
                                }
                                ui.add_sized(
                                    [card_w - 14.0, 14.0],
                                    egui::Label::new(
                                        RichText::new(file_name(&f.path))
                                            .small()
                                            .color(Palette::TEXT),
                                    )
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
            .default_width(360.0)
            .width_range(280.0..=520.0)
            .frame(egui::Frame::default().fill(Palette::PANEL))
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new("INSPECTOR")
                            .size(10.5)
                            .strong()
                            .color(Palette::TEXT_FAINT),
                    );
                });
                ui.add_space(4.0);
                ui.separator();

                let Some(vf) = self.selected.clone() else {
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new("Select an asset to inspect and preview.")
                                .size(12.0)
                                .color(Palette::TEXT_FAINT),
                        );
                    });
                    return;
                };

                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    egui::Frame::default()
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            let (icon, color) = type_icon(&vf.path);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(icon).size(19.0).color(color));
                                ui.add_space(2.0);
                                ui.label(
                                    RichText::new(file_name(&vf.path))
                                        .size(14.0)
                                        .strong()
                                        .color(Palette::TEXT_STRONG),
                                );
                            });
                            ui.label(
                                RichText::new(&vf.path)
                                    .monospace()
                                    .size(10.5)
                                    .color(Palette::TEXT_FAINT),
                            );

                            ui.add_space(6.0);
                            egui::Grid::new("meta")
                                .num_columns(2)
                                .spacing([16.0, 2.0])
                                .show(ui, |ui| {
                                    meta_row(ui, "Type", file_type_name(&vf.path).to_string());
                                    meta_row(ui, "Size", format_size(vf.size));
                                    if let Some(m) = &self.mount {
                                        meta_row(
                                            ui,
                                            "Source",
                                            file_name(&m.source_path(vf.source)).to_string(),
                                        );
                                    }
                                    let encrypted = self
                                        .mount
                                        .as_ref()
                                        .map(|m| m.is_file_encrypted(&vf))
                                        .unwrap_or(false);
                                    if encrypted {
                                        ui.label(
                                            RichText::new("Encryption")
                                                .small()
                                                .color(Palette::TEXT_FAINT),
                                        );
                                        ui.label(
                                            RichText::new(format!("{} AES-256", ic::LOCK))
                                                .small()
                                                .color(Palette::AMBER),
                                        );
                                        ui.end_row();
                                    }
                                });
                        });

                    // preview section
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new("PREVIEW")
                                .size(10.5)
                                .strong()
                                .color(Palette::TEXT_FAINT),
                        );
                    });
                    ui.add_space(4.0);
                    egui::Frame::default()
                        .inner_margin(egui::Margin::symmetric(12, 2))
                        .show(ui, |ui| {
                            match &mut self.preview {
                                Preview::None => {
                                    ui.label(
                                        RichText::new("Loading…")
                                            .size(12.0)
                                            .color(Palette::TEXT_FAINT),
                                    );
                                }
                                Preview::Texture(tex) => {
                                    let avail = ui.available_width() - 24.0;
                                    egui::Frame::default()
                                        .fill(Palette::PREVIEW_BG)
                                        .corner_radius(5.0)
                                        .inner_margin(6.0)
                                        .show(ui, |ui| {
                                            ui.image(egui::load::SizedTexture::new(
                                                tex.id(),
                                                Vec2::new(avail, avail * 0.75),
                                            ));
                                        });
                                    ui.caption("Decoded texture");
                                }
                                Preview::Mesh { mesh, rot_x, rot_y, zoom } => {
                                    mesh_preview_ui(ui, mesh, rot_x, rot_y, zoom);
                                    ui.caption("Drag to rotate · scroll to zoom");
                                }
                                Preview::Text(t) => {
                                    egui::ScrollArea::vertical()
                                        .auto_shrink(false)
                                        .id_salt("preview_text")
                                        .max_height(ui.available_height() - 50.0)
                                        .show(ui, |ui| {
                                            ui.monospace(
                                                RichText::new(t.as_str())
                                                    .size(11.0)
                                                    .color(Palette::TEXT),
                                            );
                                        });
                                }
                                Preview::Unsupported(t) => {
                                    ui.label(
                                        RichText::new(t.as_str())
                                            .size(12.0)
                                            .color(Palette::TEXT_FAINT),
                                    );
                                }
                            }
                        });
                });

                // action footer
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    egui::Frame::default()
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            let w = ui.available_width();
                            let btn = egui::Button::new(
                                RichText::new(format!("{} Extract file", ic::EXTRACT_ONE))
                                    .size(13.0)
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(Palette::ACCENT)
                            .corner_radius(5.0)
                            .min_size(Vec2::new(w, 30.0));
                            if ui.add(btn).clicked() {
                                let sel = self.selected.clone();
                                self.extract_selected(sel);
                            }
                        });
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
        .default_width(560.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.add_space(2.0);
            ui.label(RichText::new(&prompt.reason).size(13.0));
            if let Some(guid) = &prompt.guid_hint {
                ui.label(
                    RichText::new(format!("Archive key GUID: {guid}"))
                        .size(11.0)
                        .color(Palette::TEXT_FAINT),
                );
            }
            ui.add_space(8.0);
            ui.label(
                RichText::new("Encryption key (64 hex characters)")
                    .size(11.0)
                    .strong()
                    .color(Palette::TEXT_WEAK),
            );
            ui.add(
                egui::TextEdit::singleline(&mut prompt.input)
                    .desired_width(ui.available_width())
                    .font(egui::TextStyle::Monospace)
                    .hint_text("0x… paste the game's AES-256 key"),
            );
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
                            .size(11.0)
                            .color(Palette::GREEN),
                    );
                } else {
                    ui.label(
                        RichText::new(format!("{len}/64 hex characters"))
                            .size(11.0)
                            .color(if len == 64 { Palette::RED } else { Palette::TEXT_FAINT }),
                    );
                }
            });
            if let Some(err) = &prompt.error {
                ui.add_space(2.0);
                ui.label(
                    RichText::new(format!("{} {err}", ic::ERROR))
                        .size(11.0)
                        .color(Palette::RED),
                );
            }
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                cancel = ui.button("Cancel").clicked();
                // "extract unencrypted only" only makes sense when the archive is
                // open and only PART of it is encrypted
                if let Some(mount) = &self.mount {
                    let enc = mount.encrypted_file_count();
                    if enc > 0 && enc < mount.file_count() as u64 {
                        extract_unenc = ui
                            .button(format!("{} Unencrypted only", ic::EXTRACT))
                            .on_hover_text("Skip encrypted files — extract what is readable now")
                            .clicked();
                    }
                }
                save = ui
                    .add(
                        egui::Button::new(
                            RichText::new(format!("{} Save key & continue", ic::OK))
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(Palette::ACCENT)
                        .corner_radius(5.0),
                    )
                    .clicked();
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
            self.status = Status::new(StatusKind::Info, "Skipping encrypted files.");
            self.start_extraction(None, true);
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
        egui::Window::new(RichText::new(format!("{} Extracting", ic::EXTRACT)).heading().strong())
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
                            .desired_height(16.0),
                    );
                    ui.add_space(4.0);
                    egui::Grid::new("prog_stats")
                        .num_columns(2)
                        .spacing([16.0, 2.0])
                        .show(ui, |ui| {
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
                        .size(10.5)
                        .color(Palette::TEXT_FAINT),
                    );
                }
                ui.add_space(6.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    cancel = ui
                        .add(
                            egui::Button::new(
                                RichText::new(format!("{} Cancel", ic::CANCEL))
                                    .color(Palette::RED),
                            )
                            .corner_radius(5.0),
                        )
                        .clicked();
                });
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
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "Keys decrypt encrypted archives. The active key is used when opening and extracting.",
                    )
                    .size(12.0)
                    .color(Palette::TEXT_WEAK),
                );
                ui.separator();
                if keys.is_empty() {
                    ui.label(
                        RichText::new("No keys saved yet.")
                            .size(12.0)
                            .color(Palette::TEXT_FAINT),
                    );
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
                        ui.monospace(RichText::new(shown).size(10.5).color(Palette::TEXT_FAINT));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .small_button(RichText::new(ic::TRASH).size(11.0))
                                .on_hover_text("Remove key")
                                .clicked()
                            {
                                remove = Some(key.name.clone());
                            }
                            if active {
                                badge(ui, "ACTIVE", Palette::GREEN);
                            }
                        });
                    });
                }
                ui.separator();
                ui.label(
                    RichText::new("ADD KEY")
                        .size(10.5)
                        .strong()
                        .color(Palette::TEXT_FAINT),
                );
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.add_sized([140.0, 24.0], egui::TextEdit::singleline(&mut add_name).hint_text("Name"));
                    ui.add_sized(
                        [ui.available_width(), 24.0],
                        egui::TextEdit::singleline(&mut add_hex)
                            .hint_text("64 hex characters")
                            .font(egui::TextStyle::Monospace),
                    );
                });
                ui.add_space(4.0);
                add = ui
                    .add(
                        egui::Button::new(format!("{} Add key", ic::PLUS)).corner_radius(5.0),
                    )
                    .clicked();
                if !self.archive_name.is_empty() {
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
            keys.push(KeyEntry {
                name: add_name,
                hex: add_hex
                    .trim()
                    .trim_start_matches("0x")
                    .replace(['-', ' '], "")
                    .to_lowercase(),
            });
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
                self.open_path(std::path::PathBuf::from(&rec));
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

        egui::Window::new(
            RichText::new(format!("{} Export Settings", ic::SETTINGS))
                .heading()
                .strong(),
        )
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(520.0)
        .show(ctx, |ui| {
            ui.label(
                RichText::new("DESTINATION")
                    .size(10.5)
                    .strong()
                    .color(Palette::TEXT_FAINT),
            );
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut output_dir)
                        .desired_width(ui.available_width() - 74.0)
                        .font(egui::TextStyle::Monospace),
                );
                browse = ui.button("Browse…").clicked();
            });
            ui.add_space(6.0);
            ui.separator();
            ui.label(
                RichText::new("EXPORT FORMATS")
                    .size(10.5)
                    .strong()
                    .color(Palette::TEXT_FAINT),
            );
            ui.add_space(2.0);
            egui::Grid::new("formats").num_columns(2).spacing([16.0, 3.0]).show(ui, |ui| {
                ui.checkbox(&mut formats.png, format!("Textures {} PNG (decoded)", ic::ARROW_RIGHT));
                ui.checkbox(&mut formats.dds, format!("Textures {} DDS (original compression)", ic::ARROW_RIGHT));
                ui.end_row();
                ui.checkbox(&mut formats.glb, format!("Meshes {} GLTF 2.0 (.glb)", ic::ARROW_RIGHT));
                ui.checkbox(&mut formats.obj, format!("Meshes {} OBJ (fallback)", ic::ARROW_RIGHT));
                ui.end_row();
                ui.checkbox(&mut formats.json, format!("Metadata {} JSON", ic::ARROW_RIGHT));
                ui.checkbox(&mut formats.audio, format!("Audio {} WAV / OGG", ic::ARROW_RIGHT));
                ui.end_row();
            });
            ui.add_space(4.0);
            ui.checkbox(&mut convert, "Enable conversion on extract");
            ui.add_space(6.0);
            ui.separator();
            ui.label(
                RichText::new("BATCH QUEUE")
                    .size(10.5)
                    .strong()
                    .color(Palette::TEXT_FAINT),
            );
            if queue.is_empty() {
                ui.label(
                    RichText::new("Empty — add archives to process them one after another.")
                        .size(12.0)
                        .color(Palette::TEXT_FAINT),
                );
            }
            for (i, item) in queue.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{}. {}", i + 1, file_name(item)))
                            .size(12.0),
                    );
                    if ui.small_button(RichText::new(ic::CLOSE).size(10.0)).clicked() {
                        remove_idx = Some(i);
                    }
                });
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                add_current = ui.button(format!("{} Add current", ic::PLUS)).clicked();
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
                .size(12.0)
                .color(Palette::TEXT_WEAK),
            );
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                if self.last_errors.is_empty() {
                    ui.label(
                        RichText::new("No errors recorded.")
                            .size(12.0)
                            .color(Palette::TEXT_FAINT),
                    );
                }
                for (i, e) in self.last_errors.iter().enumerate() {
                    ui.label(
                        RichText::new(format!("{}. {e}", i + 1))
                            .monospace()
                            .size(10.5)
                            .color(Palette::TEXT),
                    );
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

fn icon_button(ui: &mut egui::Ui, glyph: &str, _label: &str, tip: &str) -> egui::Response {
    ui.button(RichText::new(glyph).size(15.0).color(Palette::TEXT_WEAK))
        .on_hover_text(tip)
}

/// Flat, professional table header cell: small caps text + caret when active.
/// Returns the cell's click response.
fn sort_header(ui: &mut egui::Ui, label: &str, active: bool, asc: bool, right_align: bool) -> egui::Response {
    let text = label.to_uppercase();
    let color = if active { Palette::ACCENT_TEXT } else { Palette::TEXT_FAINT };
    let caret = if !active {
        ""
    } else if asc {
        ic::CARET_UP
    } else {
        ic::CARET_DOWN
    };
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 16.0),
        Sense::click(),
    );
    let galley_text = format!("{text} {caret}");
    let font = FontId::proportional(10.5);
    let anchor = if right_align { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER };
    let pos = if right_align { rect.right_center() } else { rect.left_center() };
    ui.painter().text(pos, anchor, galley_text, font, color);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

fn meta_row(ui: &mut egui::Ui, k: &str, v: String) {
    ui.label(RichText::new(k).size(11.0).color(Palette::TEXT_FAINT));
    ui.label(RichText::new(v).size(12.0).color(Palette::TEXT));
    ui.end_row();
}

fn prog_row(ui: &mut egui::Ui, k: &str, v: String) {
    ui.label(RichText::new(k).size(11.0).color(Palette::TEXT_FAINT));
    ui.label(RichText::new(v).size(12.0).color(Palette::TEXT));
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
            let caret = if open { ic::CARET_DOWN } else { ic::CARET_RIGHT };
            let glyph = if open { ic::FOLDER_OPEN } else { ic::FOLDER };
            let resp = ui.selectable_label(
                false,
                RichText::new(format!("{caret} {glyph} {}", node.name))
                    .size(12.5)
                    .color(Palette::TEXT),
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
                        .size(12.5)
                        .color(if selected { Palette::ACCENT_TEXT } else { Palette::TEXT }),
                )
                .on_hover_text(&node.full_path)
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
        self.label(RichText::new(text).size(10.5).color(Palette::TEXT_FAINT));
    }
}



