//! UExtractor GUI — professional dark-themed asset browser built on egui/eframe.

mod app;
mod capture;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    // autoshot mode: --autoshot <out_dir> <archive> [big_archive]
    let args: Vec<String> = std::env::args().collect();
    let autoshot = args.iter().position(|a| a == "--autoshot").map(|i| {
        let out_dir = std::path::PathBuf::from(&args[i + 1]);
        std::fs::create_dir_all(&out_dir).ok();
        (out_dir, args[i + 2].clone(), args.get(i + 3).cloned())
    });

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
        "UExtractor — Unreal Engine Asset Extractor",
        options,
        Box::new(move |cc| {
            egui_extras_install(cc);
            let mut app = app::App::new();
            app.autoshot = shot;
            Ok(Box::new(app))
        }),
    )
}

fn egui_extras_install(_cc: &eframe::CreationContext<'_>) {
    // placeholder for future font/image setup
}

/// Simple generated application icon (magenta "U" on dark).
fn icon() -> eframe::egui::IconData {
    let size = 32usize;
    let mut rgba = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let i = (y * size + x) * 4;
            let in_u = (x >= 8 && x <= 23 && (y <= 10 || y >= 22))
                || (x <= 11 && y >= 10 && y <= 22)
                || (x >= 20 && y >= 10 && y <= 22);
            if in_u {
                rgba[i] = 224;
                rgba[i + 1] = 64;
                rgba[i + 2] = 176;
            } else {
                rgba[i] = 24;
                rgba[i + 1] = 24;
                rgba[i + 2] = 28;
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
