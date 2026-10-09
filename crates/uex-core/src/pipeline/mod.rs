//! Parallel extraction pipeline: rayon worker pool over per-worker source readers,
//! bounded memory (each worker holds at most one entry payload), progress events
//! with speed and ETA.

pub mod convert;

use crate::error::Result;
use crate::mount::{Mount, SourceReader, VirtualFile};
use crossbeam_channel::{bounded, Receiver};
use rayon::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProgressEvent {
    pub done: u64,
    pub total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current: String,
    pub mbps: f64,
    pub eta_secs: f64,
    pub errors: u64,
    pub finished: bool,
}

/// Map of ".uexp"/".ubulk" sibling path → (source index, virtual file).
pub type SiblingMap = std::collections::HashMap<String, (usize, VirtualFile)>;

#[derive(Clone)]
pub struct ExtractOptions {
    pub filter: Option<String>,
    pub extensions: Vec<String>,
    pub output_dir: std::path::PathBuf,
    pub threads: usize,
    pub convert: bool,
    /// populated by extract_parallel for sibling resolution during conversion
    pub sibling_map: std::sync::Arc<SiblingMap>,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            filter: None,
            extensions: Vec::new(),
            output_dir: std::path::PathBuf::from("extracted"),
            threads: 0,
            convert: false,
            sibling_map: std::sync::Arc::new(std::collections::HashMap::new()),
        }
    }
}

#[derive(Clone)]
pub struct ExtractReport {
    pub total_files: u64,
    pub extracted: u64,
    pub converted: u64,
    pub errors: Vec<String>,
    pub bytes: u64,
    pub elapsed: Duration,
}

pub fn plan<'a>(mount: &'a Mount, opts: &ExtractOptions) -> Vec<&'a VirtualFile> {
    mount
        .files()
        .filter(|f| {
            if let Some(flt) = &opts.filter {
                if !f.path.to_lowercase().contains(&flt.to_lowercase()) {
                    return false;
                }
            }
            if !opts.extensions.is_empty() {
                let lower = f.path.to_lowercase();
                if !opts.extensions.iter().any(|e| lower.ends_with(&e.to_lowercase())) {
                    return false;
                }
            }
            true
        })
        .collect()
}

/// Extract files in parallel; emits progress events on the returned receiver.
pub fn extract_parallel(
    mount: &Mount,
    opts: &ExtractOptions,
) -> (Receiver<ProgressEvent>, Arc<Mutex<ExtractReport>>) {
    let items = plan(mount, opts);
    let total = items.len() as u64;
    let bytes_total: u64 = items.iter().map(|f| f.size).sum();
    let (tx, rx) = bounded::<ProgressEvent>(64);
    let threads = if opts.threads == 0 {
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
    } else {
        opts.threads
    };
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build_global()
        .ok();

    let done = Arc::new(AtomicU64::new(0));
    let bytes_done = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(AtomicU64::new(0));
    let error_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let extracted_count = Arc::new(AtomicU64::new(0));
    let converted_count = Arc::new(AtomicU64::new(0));
    let bytes_count = Arc::new(AtomicU64::new(0));
    let start = Instant::now();
    let report = Arc::new(Mutex::new(ExtractReport {
        total_files: total,
        extracted: 0,
        converted: 0,
        errors: Vec::new(),
        bytes: 0,
        elapsed: Duration::ZERO,
    }));

    // build sibling map for conversion (uexp/ubulk companions of uassets)
    let mut sibling_map: SiblingMap = std::collections::HashMap::new();
    for f in mount.files() {
        let lower = f.path.to_lowercase();
        if lower.ends_with(".uexp") || lower.ends_with(".ubulk") {
            sibling_map.insert(f.path.clone(), (f.source, f.clone()));
        }
    }
    let opts2 = ExtractOptions {
        sibling_map: std::sync::Arc::new(sibling_map),
        ..opts.clone()
    };
    let opts_arc2 = std::sync::Arc::new(opts2);

    let work: Vec<(usize, VirtualFile)> = items.into_iter().map(|f| (f.source, f.clone())).collect();

    // Safety: Mount is immutable after open; we only read from it concurrently.
    let mount_static: &'static Mount = unsafe { std::mem::transmute::<&Mount, &'static Mount>(mount) };

    let opts_arc = Arc::new(opts.clone());
    let report2 = report.clone();

    std::thread::spawn(move || {
        let mut by_source: std::collections::BTreeMap<usize, Vec<VirtualFile>> = Default::default();
        for (src, vf) in work {
            by_source.entry(src).or_default().push(vf);
        }
        let src_vec: Vec<(usize, Vec<VirtualFile>)> = by_source.into_iter().collect();

        src_vec.into_par_iter().for_each(|(source_idx, files)| {
            let mut reader = match mount_static.open_source_reader(source_idx) {
                Ok(r) => r,
                Err(e) => {
                    errors.fetch_add(files.len() as u64, Ordering::Relaxed);
                    error_log.lock().unwrap().push(format!("source {}: {e}", source_idx));
                    return;
                }
            };
            for vf in files {
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    convert::extract_one(&mut reader, &vf, &opts_arc2)
                }));
                match res {
                    Ok(Ok(converted)) => {
                        extracted_count.fetch_add(1, Ordering::Relaxed);
                        bytes_count.fetch_add(vf.size, Ordering::Relaxed);
                        if converted {
                            converted_count.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Ok(Err(e)) => {
                        errors.fetch_add(1, Ordering::Relaxed);
                        error_log.lock().unwrap().push(format!("{}: {e}", vf.path));
                    }
                    Err(_) => {
                        errors.fetch_add(1, Ordering::Relaxed);
                        error_log
                            .lock()
                            .unwrap()
                            .push(format!("{}: panicked during extraction", vf.path));
                    }
                }
                let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                let b = bytes_done.fetch_add(vf.size, Ordering::Relaxed) + vf.size;
                let elapsed = start.elapsed().as_secs_f64();
                let mbps = b as f64 / 1024.0 / 1024.0 / elapsed.max(0.001);
                let eta = if b > 0 {
                    elapsed * ((bytes_total - b) as f64 / b as f64)
                } else {
                    0.0
                };
                let _ = tx.send(ProgressEvent {
                    done: d,
                    total,
                    bytes_done: b,
                    bytes_total,
                    current: vf.path.clone(),
                    mbps,
                    eta_secs: eta,
                    errors: errors.load(Ordering::Relaxed),
                    // only the explicit final event after the report update carries `finished`
                    finished: false,
                });
            }
        });
        {
            let mut rep = report2.lock().unwrap();
            rep.extracted = extracted_count.load(Ordering::Relaxed);
            rep.converted = converted_count.load(Ordering::Relaxed);
            rep.bytes = bytes_count.load(Ordering::Relaxed);
            rep.elapsed = start.elapsed();
            rep.errors = error_log.lock().unwrap().clone();
        }
        let _ = tx.send(ProgressEvent {
            done: total,
            total,
            bytes_done: bytes_done.load(Ordering::Relaxed),
            bytes_total,
            current: String::new(),
            mbps: 0.0,
            eta_secs: 0.0,
            errors: errors.load(Ordering::Relaxed),
            finished: true,
        });
    });

    (rx, report)
}

pub fn run_extraction(mount: &Mount, opts: &ExtractOptions) -> Result<ExtractReport> {
    let (rx, report) = extract_parallel(mount, opts);
    for ev in rx {
        if ev.finished {
            break;
        }
    }
    let snapshot = report.lock().unwrap().clone();
    Ok(snapshot)
}
