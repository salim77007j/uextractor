//! UExtractor CLI — full command-line access to the extraction engine.
//!
//! Exit codes: 0 = success, 1 = runtime error, 2 = usage error.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use uex_core::crypto::AesKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum OutputFormat {
    Table,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ConvertFormat {
    None,
    Auto,
}

#[derive(Parser)]
#[command(
    name = "uex",
    version,
    about = "UExtractor — Unreal Engine & Unity game asset extractor",
    long_about = "Fast, parallel game archive extractor with real asset conversion.\nSupports Unreal PAK v1-v11, IOStore (.ucas/.utoc), Unity bundles (.bundle/.unity3d),\nserialized files (.assets, level*, sharedassets*), AES-256 encryption and Oodle compression."
)]
struct Cli {
    /// AES-256 key (hex, 0x prefix optional)
    #[arg(short = 'k', long, global = true)]
    aes: Option<String>,

    /// AES key file (first line used as key)
    #[arg(long, global = true)]
    aes_file: Option<PathBuf>,

    /// Worker threads (0 = all cores)
    #[arg(short = 'j', long, default_value_t = 0, global = true)]
    jobs: usize,

    /// Verbose logging (-v info, -vv debug, -vvv trace)
    #[arg(short = 'v', action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show archive metadata (version, file counts, encryption, compression)
    Info {
        /// .pak / .utoc / directory containing archives
        path: PathBuf,
        #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
        format: OutputFormat,
    },
    /// List archive contents
    List {
        path: PathBuf,
        #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
        format: OutputFormat,
        #[arg(long)]
        filter: Option<String>,
    },
    /// Extract files
    Extract {
        path: PathBuf,
        #[arg(short, long, default_value = "extracted")]
        out: PathBuf,
        #[arg(long)]
        filter: Option<String>,
        /// only extract these extensions (repeatable, e.g. -e uasset -e png)
        #[arg(short = 'e', long)]
        ext: Vec<String>,
        /// convert assets (textures → PNG/DDS, meshes → GLB, audio → WAV/OGG, metadata → JSON)
        #[arg(long, value_enum, default_value_t = ConvertFormat::Auto)]
        convert: ConvertFormat,
        /// skip AES-encrypted files instead of failing on them
        #[arg(long)]
        skip_encrypted: bool,
    },
    /// Test a single AES key against an archive
    TestKey {
        path: PathBuf,
        #[arg(short = 'k', long)]
        aes: String,
    },
    /// Generate synthetic test archives (for validation without game files)
    GenTestdata {
        #[arg(short, long, default_value = "testdata")]
        out: PathBuf,
        #[arg(long, default_value_t = 64)]
        files: usize,
        #[arg(long, default_value_t = 1)]
        file_mb: usize,
        /// Also generate Unity test data (UnityFS bundle + standalone .assets)
        #[arg(long)]
        unity: bool,
    },
    /// Benchmark extraction throughput on a generated archive
    Bench {
        #[arg(long, default_value_t = 256)]
        size_mb: usize,
        #[arg(long, default_value = "oodle")]
        compression: String,
    },
}

fn main() {
    let cli = Cli::parse();
    env_logger::Builder::new()
        .filter_level(match cli.verbose {
            0 => log::LevelFilter::Warn,
            1 => log::LevelFilter::Info,
            2 => log::LevelFilter::Debug,
            _ => log::LevelFilter::Trace,
        })
        .init();

    let code = match run(&cli) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e:#}");
            1
        }
    };
    std::process::exit(code);
}

fn resolve_key(cli: &Cli) -> Option<AesKey> {
    if let Some(hex) = &cli.aes {
        match AesKey::parse(hex) {
            Some(k) => return Some(k),
            None => {
                eprintln!("warning: invalid AES key hex, ignoring");
                return None;
            }
        }
    }
    if let Some(file) = &cli.aes_file {
        if let Ok(text) = std::fs::read_to_string(file) {
            let line = text.lines().next().unwrap_or("").trim();
            return AesKey::parse(line);
        }
    }
    None
}

/// Split the --aes value into the UE hex key and/or a UnityCN 16-char key.
fn resolve_keys(cli: &Cli) -> uex_core::mount::MountKeys {
    let mut keys = uex_core::mount::MountKeys::default();
    let mut candidate: Option<String> = None;
    if let Some(text) = &cli.aes {
        candidate = Some(text.clone());
    }
    if candidate.is_none() {
        if let Some(file) = &cli.aes_file {
            if let Ok(content) = std::fs::read_to_string(file) {
                candidate = Some(content.lines().next().unwrap_or("").trim().to_string());
            }
        }
    }
    if let Some(text) = candidate {
        if AesKey::parse(&text).is_some() {
            keys.aes = AesKey::parse(&text);
        } else if text.len() == 16 {
            keys.unitycn = Some(text);
        } else {
            eprintln!(
                "warning: key is neither 64-hex (UE AES) nor 16 characters (UnityCN), ignoring"
            );
        }
    }
    keys
}

fn run(cli: &Cli) -> Result<()> {
    match &cli.command {
        Command::Info { path, format } => cmd_info(path, *format, cli),
        Command::List { path, format, filter } => cmd_list(path, *format, filter.as_deref(), cli),
        Command::Extract { path, out, filter, ext, convert, skip_encrypted } => {
            cmd_extract(path, out, filter.as_deref(), ext, *convert, *skip_encrypted, cli)
        }
        Command::TestKey { path, aes } => cmd_test_key(path, aes),
        Command::GenTestdata { out, files, file_mb, unity } => cmd_gen_testdata(out, *files, *file_mb, *unity),
        Command::Bench { size_mb, compression } => cmd_bench(*size_mb, compression),
    }
}

fn open_mount(path: &PathBuf, cli: &Cli) -> Result<uex_core::mount::Mount> {
    let keys = resolve_keys(cli);
    let mount = uex_core::mount::Mount::open_with_keys(path, keys)
        .with_context(|| format!("failed to open {}", path.display()))?;
    if mount.has_unity_encrypted() {
        eprintln!(
            "note: this archive uses Unity's UnityCN bundle encryption;\n      pass the game's 16-character key with --aes '<key>' (validated before extraction)"
        );
    }
    Ok(mount)
}

fn cmd_info(path: &PathBuf, format: OutputFormat, cli: &Cli) -> Result<()> {
    let mount = open_mount(path, cli)?;
    let info = serde_json::json!({
        "sources": (0..mount.source_count()).map(|i| mount.source_path(i)).collect::<Vec<_>>(),
        "file_count": mount.file_count(),
        "total_bytes": mount.total_size,
    });
    match format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&info)?),
        OutputFormat::Table => {
            println!("UExtractor — archive info");
            println!("  sources:    {}", mount.source_count());
            for i in 0..mount.source_count() {
                println!("    - {}", mount.source_path(i));
            }
            if mount.has_unity_encrypted() {
                println!("  encryption: UnityCN bundle encryption detected (needs a 16-char key via --aes)");
            }
            println!("  files:      {}", mount.file_count());
            println!("  total size: {:.2} MB", mount.total_size as f64 / 1048576.0);
        }
    }
    Ok(())
}

fn cmd_list(path: &PathBuf, format: OutputFormat, filter: Option<&str>, cli: &Cli) -> Result<()> {
    let mount = open_mount(path, cli)?;
    match format {
        OutputFormat::Json => {
            let files: Vec<_> = mount
                .files()
                .filter(|f| filter.map(|flt| f.path.to_lowercase().contains(&flt.to_lowercase())).unwrap_or(true))
                .map(|f| serde_json::json!({"path": f.path, "size": f.size}))
                .collect();
            println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "files": files }))?);
        }
        OutputFormat::Table => {
            for f in mount.files() {
                if let Some(flt) = filter {
                    if !f.path.to_lowercase().contains(&flt.to_lowercase()) {
                        continue;
                    }
                }
                println!("{:>12}  {}", f.size, f.path);
            }
        }
    }
    Ok(())
}

fn cmd_extract(
    path: &PathBuf,
    out: &PathBuf,
    filter: Option<&str>,
    ext: &[String],
    convert: ConvertFormat,
    skip_encrypted: bool,
    cli: &Cli,
) -> Result<()> {
    let mount = open_mount(path, cli)?;
    // pre-flight: encrypted files need a key — say so BEFORE extracting
    let enc = mount.encrypted_file_count();
    if enc > 0 && !skip_encrypted && cli.aes.is_none() {
        eprintln!(
            "warning: {enc} file(s) in this archive are AES-encrypted and NO key was supplied (--aes)."
        );
        eprintln!("         Those files will fail to extract. Pass --aes <hex64> with the game key,");
        eprintln!("         or use --skip-encrypted to extract only the unencrypted files.");
    }
    let opts = uex_core::pipeline::ExtractOptions {
        filter: filter.map(|s| s.to_string()),
        extensions: ext.iter().map(|e| format!(".{}", e.trim_start_matches('.'))).collect(),
        output_dir: out.clone(),
        threads: cli.jobs,
        convert: convert == ConvertFormat::Auto,
        skip_encrypted,
        ..Default::default()
    };
    let planned = uex_core::pipeline::plan(&mount, &opts);
    let planned_bytes: u64 = planned.iter().map(|f| f.size).sum();
    drop(planned);
    eprintln!(
        "Extracting {} files ({:.2} MB) to {} with {} threads...",
        mount.file_count(),
        planned_bytes as f64 / 1048576.0,
        out.display(),
        if cli.jobs == 0 { std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) } else { cli.jobs }
    );
    let report = uex_core::pipeline::run_extraction(&mount, &opts)?;
    println!(
        "done: {} extracted, {} converted, {} errors, {:.2} MB in {:.2}s ({:.1} MB/s)",
        report.extracted,
        report.converted,
        report.errors.len(),
        report.bytes as f64 / 1048576.0,
        report.elapsed.as_secs_f64(),
        report.bytes as f64 / 1048576.0 / report.elapsed.as_secs_f64().max(0.001)
    );
    for e in report.errors.iter().take(20) {
        eprintln!("  error: {e}");
    }
    if report.errors.len() > 20 {
        eprintln!("  ... and {} more errors", report.errors.len() - 20);
    }
    Ok(())
}

fn cmd_test_key(path: &PathBuf, aes: &str) -> Result<()> {
    if let Some(key) = AesKey::parse(aes) {
        let mount = uex_core::mount::Mount::open(path, Some(key)).context("failed to open with key")?;
        println!("key accepted: {} files, {:.2} MB", mount.file_count(), mount.total_size as f64 / 1048576.0);
        return Ok(());
    }
    // UnityCN keys are 16 characters, not hex
    if aes.len() == 16 {
        let mount = uex_core::mount::Mount::open_no_keys(path).context("failed to open archive")?;
        if !mount.has_unity_encrypted() {
            anyhow::bail!("archive has no UnityCN-encrypted bundles to test against");
        }
        let ok = mount.test_unity_key(aes)?;
        if ok {
            println!("UnityCN key accepted (signature verified)");
            Ok(())
        } else {
            anyhow::bail!("UnityCN key rejected: signature mismatch — wrong key for this bundle");
        }
    } else {
        anyhow::bail!("invalid key: expected 64 hex chars (UE AES-256) or 16 characters (UnityCN)");
    }
}

fn cmd_gen_testdata(out: &PathBuf, files: usize, file_mb: usize, unity: bool) -> Result<()> {
    std::fs::create_dir_all(out)?;
    // plain pak
    let samples: Vec<uex_testdata::SampleFile> = (0..files)
        .map(|i| {
            uex_testdata::SampleFile::new(
                &format!("Props/SM_File{i}.bin"),
                uex_testdata::pseudo_data(i as u64 + 1, file_mb * 1024 * 1024),
            )
        })
        .collect();
    let pak = uex_testdata::build_pak_v11(&samples, true, None)?;
    std::fs::write(out.join("test_plain.pak"), &pak)?;
    // encrypted pak
    let pak_enc = uex_testdata::build_pak_v11(&samples, true, Some(uex_testdata::TEST_AES_KEY))?;
    std::fs::write(out.join("test_encrypted.pak"), &pak_enc)?;
    // iostore
    let (pid, pkg) = uex_testdata::build_zen_texture_package("T_Rock", 64, 64);
    let mut cas = uex_testdata::CasBuilder::new();
    let is_new = true;
    let chunk_idx = cas.add_chunk(pid, 0, uex_core::iostore::ChunkType::ExportBundleData, &pkg, is_new);
    cas.add_file("Props/T_Rock.uasset", chunk_idx);
    let (utoc, ucas) = cas.build()?;
    std::fs::write(out.join("test_container.utoc"), &utoc)?;
    std::fs::write(out.join("test_container.ucas"), &ucas)?;
    // classic texture package
    let (ua, ue) = uex_testdata::build_classic_texture_package("T_Wood", 64, 64);
    let classic = uex_testdata::build_pak_v11(
        &[
            uex_testdata::SampleFile::new("Props/T_Wood.uasset", ua),
            uex_testdata::SampleFile::new("Props/T_Wood.uexp", ue),
        ],
        false,
        None,
    )?;
    std::fs::write(out.join("test_textures.pak"), &classic)?;
    // classic static mesh package
    let (ma, me) = uex_testdata::build_classic_static_mesh_package("SM_Cube");
    let meshpak = uex_testdata::build_pak_v11(
        &[
            uex_testdata::SampleFile::new("Props/SM_Cube.uasset", ma),
            uex_testdata::SampleFile::new("Props/SM_Cube.uexp", me),
        ],
        false,
        None,
    )?;
    std::fs::write(out.join("test_mesh.pak"), &meshpak)?;
    if unity {
        // UnityFS bundle (LZ4, 2019.4 layout) with the full sample object set:
        // Texture2D + TextAsset + AudioClip(streamed from .resS) + Mesh + Sprite
        // + GameObject + AssetBundle container map.
        let (w, h) = (128i32, 128i32);
        let rgba: Vec<u8> = (0..(w * h) as usize)
            .flat_map(|i| {
                let x = (i % w as usize) as u8;
                let y = (i / w as usize) as u8;
                [x * 2, y * 2, (x as u16 * y as u16 % 255) as u8, 255]
            })
            .collect();
        let bundle = uex_testdata::unity::sample_bundle_rgba("T_UnitySample", &rgba, w, h, true);
        std::fs::write(out.join("test_unity.bundle"), &bundle)?;
        // standalone serialized file (level0/sharedassets0 equivalent, stripped trees)
        let standalone = uex_testdata::unity::sample_standalone_serialized(&rgba, w, h);
        std::fs::write(out.join("test_unity_standalone.assets"), &standalone)?;
        println!("unity test data: test_unity.bundle, test_unity_standalone.assets");
    }
    println!("test archives written to {}", out.display());
    Ok(())
}

fn cmd_bench(size_mb: usize, compression: &str) -> Result<()> {
    let dir = std::env::temp_dir().join("uex_bench");
    std::fs::create_dir_all(&dir)?;
    let files: Vec<uex_testdata::SampleFile> = (0..16)
        .map(|i| {
            uex_testdata::SampleFile::new(
                &format!("Bench/F{i}.bin"),
                uex_testdata::pseudo_data(i as u64 + 7, size_mb * 1024 * 1024 / 16),
            )
        })
        .collect();
    let method = match compression.to_lowercase().as_str() {
        "zlib" => vec![uex_core::compress::Method::Zlib],
        "lz4" => vec![uex_core::compress::Method::Lz4],
        "oodle" => vec![uex_core::compress::Method::Oodle],
        _ => vec![],
    };
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = uex_core::pak::PakWriter::new(
        cursor,
        uex_core::pak::Version::V11,
        "../../../Bench/",
        None,
        method.clone(),
        false,
    );
    for f in &files {
        writer.write_file(&f.path, &f.data)?;
    }
    let pak_bytes = writer.finalize()?.into_inner();
    let pak_path = dir.join("bench.pak");
    std::fs::write(&pak_path, &pak_bytes)?;
    eprintln!("archive: {:.1} MB, compression: {compression}", pak_bytes.len() as f64 / 1048576.0);

    let mount = uex_core::mount::Mount::open(&pak_path, None)?;
    let out = dir.join("out");
    let _ = std::fs::remove_dir_all(&out);
    let opts = uex_core::pipeline::ExtractOptions {
        output_dir: out,
        threads: 0,
        ..Default::default()
    };
    let start = std::time::Instant::now();
    let report = uex_core::pipeline::run_extraction(&mount, &opts)?;
    let secs = report.elapsed.as_secs_f64();
    println!(
        "extracted {} files ({:.1} MB raw) in {:.2}s = {:.1} MB/s compressed, {:.1} MB/s raw",
        report.extracted,
        report.bytes as f64 / 1048576.0,
        secs,
        mount.total_size as f64 / 1048576.0 / secs,
        report.bytes as f64 / 1048576.0 / secs
    );
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
