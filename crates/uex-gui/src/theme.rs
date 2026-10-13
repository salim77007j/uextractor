//! UExtractor visual theme — professional dark palette inspired by modern game-dev
//! tools (Unreal Editor / Figma dark). Single source of truth for colors + styles.

use eframe::egui::{self, Color32, CornerRadius, Margin, Stroke, Vec2};

pub struct Palette;

impl Palette {
    // base surfaces
    pub const BG: Color32 = Color32::from_rgb(0x13, 0x14, 0x1a);
    pub const PANEL: Color32 = Color32::from_rgb(0x1a, 0x1c, 0x24);
    pub const PANEL_HEADER: Color32 = Color32::from_rgb(0x20, 0x23, 0x2d);
    pub const CARD: Color32 = Color32::from_rgb(0x22, 0x25, 0x30);
    pub const CARD_HOVER: Color32 = Color32::from_rgb(0x2a, 0x2e, 0x3b);
    pub const WIDGET: Color32 = Color32::from_rgb(0x26, 0x29, 0x34);
    pub const WIDGET_HOVER: Color32 = Color32::from_rgb(0x30, 0x34, 0x42);
    pub const WIDGET_ACTIVE: Color32 = Color32::from_rgb(0x38, 0x3d, 0x4e);

    // accents
    pub const ACCENT: Color32 = Color32::from_rgb(0x4f, 0x8c, 0xff); // UE blue
    pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x2c, 0x4a, 0x80);
    pub const ACCENT_TEXT: Color32 = Color32::from_rgb(0x8a, 0xb8, 0xff);
    pub const MAGENTA: Color32 = Color32::from_rgb(0xd4, 0x5f, 0xd6);
    pub const GREEN: Color32 = Color32::from_rgb(0x4f, 0xd6, 0x7e);
    pub const AMBER: Color32 = Color32::from_rgb(0xff, 0xb4, 0x54);
    pub const RED: Color32 = Color32::from_rgb(0xff, 0x5c, 0x6c);

    // text
    pub const TEXT: Color32 = Color32::from_rgb(0xe8, 0xea, 0xf0);
    pub const TEXT_WEAK: Color32 = Color32::from_rgb(0x9a, 0xa0, 0xb0);
    pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x66, 0x6c, 0x7c);

    // type-icon colors
    pub const C_PACKAGE: Color32 = Color32::from_rgb(0x7e, 0x9c, 0xff);
    pub const C_IMAGE: Color32 = Color32::from_rgb(0x5f, 0xd6, 0xb2);
    pub const C_MESH: Color32 = Color32::from_rgb(0xff, 0xa9, 0x5f);
    pub const C_AUDIO: Color32 = Color32::from_rgb(0xe0, 0x7a, 0x9c);
    pub const C_DATA: Color32 = Color32::from_rgb(0x9c, 0xc0, 0x6a);
    pub const C_FILE: Color32 = Color32::from_rgb(0x8d, 0x93, 0xa5);
    pub const C_FONT: Color32 = Color32::from_rgb(0xc0, 0x9c, 0x6a);
}

/// Icon glyphs (Phosphor fill). Registered as a fallback font so any RichText
/// can embed them inline.
pub mod ic {
    use egui_phosphor::fill;

    pub const OPEN: &str = fill::FOLDER_OPEN;
    pub const EXTRACT: &str = fill::DOWNLOAD_SIMPLE;
    pub const EXTRACT_ONE: &str = fill::EXPORT;
    pub const CANCEL: &str = fill::PROHIBIT;
    pub const SEARCH: &str = fill::MAGNIFYING_GLASS;
    pub const KEYS: &str = fill::KEY;
    pub const SETTINGS: &str = fill::GEAR_SIX;
    pub const RECENT: &str = fill::CLOCK_COUNTER_CLOCKWISE;
    pub const LIST: &str = fill::LIST_BULLETS;
    pub const GRID: &str = fill::SQUARES_FOUR;
    pub const FOLDER: &str = fill::FOLDER;
    pub const FOLDER_OPEN: &str = fill::FOLDER_OPEN;
    pub const FILE: &str = fill::FILE;
    pub const ARCHIVE: &str = fill::ARCHIVE;
    pub const PACKAGE: &str = fill::PACKAGE;
    pub const TEXTURE: &str = fill::IMAGE;
    pub const MESH: &str = fill::CUBE;
    pub const AUDIO: &str = fill::SPEAKER_HIGH;
    pub const DATA: &str = fill::FILE_TEXT;
    pub const CODE: &str = fill::FILE_CODE;
    pub const FONT: &str = fill::TEXT_T;
    pub const TREE: &str = fill::TREE_VIEW;
    pub const BLUEPRINT: &str = fill::PUZZLE_PIECE;
    pub const WARNING: &str = fill::WARNING_CIRCLE;
    pub const ERROR: &str = fill::X_CIRCLE;
    pub const OK: &str = fill::CHECK_CIRCLE;
    pub const INFO: &str = fill::INFO;
    pub const CARET_DOWN: &str = fill::CARET_DOWN;
    pub const CARET_RIGHT: &str = fill::CARET_RIGHT;
    pub const CARET_UP: &str = fill::CARET_UP;
    pub const CARET_SORT: &str = fill::CARET_UP;
    pub const CLOSE: &str = fill::X;
    pub const UNREAL: &str = fill::CUBE_TRANSPARENT;
    pub const UNITY: &str = fill::SPHERE;
    pub const ROCKET: &str = fill::ROCKET_LAUNCH;
    pub const DROP: &str = fill::DOWNLOAD;
    pub const LOCK: &str = fill::LOCK_KEY;
    pub const REFRESH: &str = fill::ARROW_CLOCKWISE;
    pub const TRASH: &str = fill::TRASH_SIMPLE;
    pub const TABLE: &str = fill::TABLE;
    pub const TERMINAL: &str = fill::TERMINAL_WINDOW;
    pub const PLUS: &str = fill::PLUS;
    pub const ARROW: &str = fill::ARROW_RIGHT;
    pub const PLAY: &str = fill::PLAY;
}

pub fn install(ctx: &egui::Context) {
    // ---- fonts: phosphor icons as fallback for all text styles ----
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Fill);
    ctx.set_fonts(fonts);

    // ---- spacing ----
    let mut style = (*ctx.style()).clone();
    let s = &mut style;
    s.spacing.item_spacing = Vec2::new(8.0, 6.0);
    s.spacing.button_padding = Vec2::new(9.0, 5.0);
    s.spacing.menu_margin = Margin::same(6);
    s.spacing.window_margin = Margin::same(12);
    s.spacing.indent = 18.0;

    // ---- text sizes ----
    use egui::FontId;
    s.text_styles = [
        (egui::TextStyle::Heading, FontId::proportional(21.0)),
        (egui::TextStyle::Body, FontId::proportional(14.0)),
        (egui::TextStyle::Button, FontId::proportional(14.0)),
        (egui::TextStyle::Small, FontId::proportional(11.5)),
        (egui::TextStyle::Monospace, FontId::monospace(12.5)),
        (egui::TextStyle::Name("Tab".into()), FontId::proportional(13.5)),
        (egui::TextStyle::Name("Badge".into()), FontId::proportional(11.0)),
    ].into_iter().collect();

    // ---- visuals ----
    let mut v = egui::Visuals::dark();
    v.panel_fill = Palette::PANEL;
    v.window_fill = Palette::PANEL;
    v.extreme_bg_color = Palette::BG; // scroll areas, text edit bg
    v.faint_bg_color = Color32::from_rgb(0x1f, 0x21, 0x2a); // striped rows
    v.override_text_color = Some(Palette::TEXT);

    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(6);

    v.widgets.noninteractive.bg_fill = Palette::PANEL_HEADER;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Palette::TEXT);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x30, 0x33, 0x40));
    v.widgets.noninteractive.corner_radius = CornerRadius::same(4);

    v.widgets.inactive.bg_fill = Palette::WIDGET;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, Palette::TEXT_WEAK);
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.inactive.corner_radius = CornerRadius::same(5);

    v.widgets.hovered.bg_fill = Palette::WIDGET_HOVER;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Palette::TEXT);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Palette::ACCENT_DIM);
    v.widgets.hovered.corner_radius = CornerRadius::same(5);

    v.widgets.active.bg_fill = Palette::WIDGET_ACTIVE;
    v.widgets.active.fg_stroke = Stroke::new(1.0, Palette::ACCENT_TEXT);
    v.widgets.active.bg_stroke = Stroke::new(1.0, Palette::ACCENT);
    v.widgets.active.corner_radius = CornerRadius::same(5);

    v.widgets.open.bg_fill = Palette::WIDGET;
    v.widgets.open.fg_stroke = Stroke::new(1.0, Palette::TEXT);

    v.selection.bg_fill = Palette::ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0, Palette::ACCENT_TEXT);

    v.window_stroke = Stroke::new(1.0, Color32::from_rgb(0x33, 0x37, 0x45));
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 28,
        spread: 2,
        color: Color32::from_black_alpha(120),
    };
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(90),
    };
    v.resize_corner_size = 12.0;
    style.visuals = v;
    ctx.set_style(style);
}

/// Small rounded badge (chips like "1 024 files", engine tags).
pub fn badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    let galley_text = format!(" {text} ");
    let font = egui::TextStyle::Name("Badge".into()).resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(galley_text, font, color);
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(galley.size().x + 8.0, galley.size().y + 4.0),
        egui::Sense::hover(),
    );
    ui.painter().rect_filled(rect, 4.0, color.gamma_multiply(0.16));
    ui.painter()
        .rect_stroke(rect, 4.0, Stroke::new(1.0, color.gamma_multiply(0.55)), egui::StrokeKind::Inside);
    ui.painter().galley(rect.left_top() + Vec2::new(4.0, 2.0), galley, color);
}

/// Icon painted in a specific color (type icons in lists/trees).
pub fn icon_colored(ui: &mut egui::Ui, glyph: &str, color: Color32, size: f32) {
    ui.label(egui::RichText::new(glyph).size(size).color(color));
}
