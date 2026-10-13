//! UExtractor GUI — professional dark-themed asset browser built on egui/eframe.

mod app;
mod capture;
mod theme;
mod ui;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    // autoshot mode: --autoshot <out_dir> <archive> [big_archive]
    let args: Vec<String> = std::env::args().collect();
    let autoshot = args.iter().position(|a| a == "--autoshot").map(|i| {
        let out_dir = std::path::PathBuf::from(&args[i + 1]);
        std::fs::create_dir_all(&out_dir).ok();
        (out_dir, args[i + 2].clone(), args.get(i + 3).cloned())
    });

    // positional archive path: open it on the first frame
    let positional = if autoshot.is_none() {
        args.iter().skip(1).find(|a| !a.starts_with('-')).cloned()
    } else {
        None
    };

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1000.0, 640.0])
            .with_icon(icon()),
        ..Default::default()
    };

    let shot = autoshot.map(|(out_dir, archive, big)| app::AutoShot {
        out_dir,
        archive: std::path::PathBuf::from(archive),
        big_archive: big.map(std::path::PathBuf::from),
        stage: 0,
        frames_in_stage: 0,
    });

    eframe::run_native(
        "UExtractor — Game Asset Extractor",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            let mut app = app::App::new();
            app.autoshot = shot;
            app.pending_open = positional.map(std::path::PathBuf::from);
            Ok(Box::new(app))
        }),
    )
}

/// Application icon: rounded square, blue→magenta vertical gradient, white "U"
/// with an extraction arrow notch. Drawn procedurally (no asset files needed).
fn icon() -> eframe::egui::IconData {
    let size = 64usize;
    let mut rgba = vec![0u8; size * size * 4];
    let n = size as f32;
    let rad = 14.0f32;

    // signed distance to a rounded rect (negative = inside)
    let sdf = |fx: f32, fy: f32| -> f32 {
        let cx = fx.clamp(rad, n - rad);
        let cy = fy.clamp(rad, n - rad);
        let dx = fx - cx;
        let dy = fy - cy;
        (dx * dx + dy * dy).sqrt() - rad
    };

    for y in 0..size {
        for x in 0..size {
            let i = (y * size + x) * 4;
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            if sdf(fx, fy) > 0.0 {
                rgba[i + 3] = 0; // transparent outside
                continue;
            }
            let t = fy / n;
            // gradient: top #4f8cff -> bottom #b04fd6
            let (r, g, b) = (
                (0x4f as f32 + (0xb0 - 0x4f) as f32 * t) as u8,
                (0x8c as f32 + (0x4f - 0x8c) as f32 * t) as u8,
                (0xff as f32 + (0xd6 - 0xff) as f32 * t) as u8,
            );
            // "U" glyph: two stems + bottom bar
            let in_u = (x >= 16 && x < 47 && y >= 38 && y < 48)
                || (x >= 16 && x < 26 && y >= 16 && y < 48)
                || (x >= 37 && x < 47 && y >= 16 && y < 48);
            // small down arrow inside the U gap (x 26..37)
            let dy = y - 20;
            let in_arrow_head = x >= 27
                && x < 36
                && y >= 20
                && y < 30
                && (31.5 - x as f32).abs() <= dy as f32 * 0.75;
            let in_arrow_stem = x >= 29 && x < 34 && y >= 27 && y < 36;
            if in_u || in_arrow_head || in_arrow_stem {
                rgba[i] = 255;
                rgba[i + 1] = 255;
                rgba[i + 2] = 255;
            } else {
                rgba[i] = r;
                rgba[i + 1] = g;
                rgba[i + 2] = b;
            }
            rgba[i + 3] = 255;
        }
    }
    eframe::egui::IconData {
        width: size as u32,
        height: size as u32,
        rgba: rgba.into(),
    }
}
