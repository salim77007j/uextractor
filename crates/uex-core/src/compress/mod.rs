//! Compression backends: None/Zlib/Gzip/LZ4/Zstd (pure Rust) + Oodle via official
//! RAD/Epic oo2core library loaded at runtime (Kraken, Mermaid, Selkie, Leviathan, Hydra).

use crate::error::{Error, Result};
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    None,
    Zlib,
    Gzip,
    Oodle,
    Zstd,
    Lz4,
}

impl Method {
    pub fn from_name(name: &str) -> Option<Method> {
        let lower = name.trim_end_matches('\0').trim().to_ascii_lowercase();
        Some(match lower.as_str() {
            "zlib" => Method::Zlib,
            "gzip" => Method::Gzip,
            "oodle" => Method::Oodle,
            "zstd" => Method::Zstd,
            "lz4" => Method::Lz4,
            "" | "none" => Method::None,
            _ => return None,
        })
    }

    pub const fn name(&self) -> &'static str {
        match self {
            Method::None => "None",
            Method::Zlib => "Zlib",
            Method::Gzip => "Gzip",
            Method::Oodle => "Oodle",
            Method::Zstd => "Zstd",
            Method::Lz4 => "LZ4",
        }
    }
}

/// Read a streaming decoder into a fixed-size output buffer.
fn read_exact_stream(
    dec: &mut impl std::io::Read,
    output: &mut [u8],
    method: &'static str,
) -> Result<usize> {
    let mut written = 0usize;
    while written < output.len() {
        match dec.read(&mut output[written..]) {
            Ok(0) => break,
            Ok(n) => written += n,
            Err(_) => return Err(Error::DecompressionFailed { method }),
        }
    }
    Ok(written)
}

/// Decompress `input` (one compression block) into `output`.
pub fn decompress(method: Method, input: &[u8], output: &mut [u8]) -> Result<usize> {
    match method {
        Method::None => {
            if input.len() != output.len() {
                return Err(Error::DecompressionFailed { method: "None" });
            }
            output.copy_from_slice(input);
            Ok(output.len())
        }
        Method::Zlib => {
            let mut dec = flate2::read::ZlibDecoder::new(input);
            read_exact_stream(&mut dec, output, "Zlib")
        }
        Method::Gzip => {
            let mut dec = flate2::read::GzDecoder::new(input);
            read_exact_stream(&mut dec, output, "Gzip")
        }
        Method::Lz4 => lz4_flex::block::decompress_into(input, output)
            .map_err(|_| Error::DecompressionFailed { method: "LZ4" }),
        Method::Zstd => {
            let mut dec = ruzstd::decoding::StreamingDecoder::new(input)
                .map_err(|_| Error::DecompressionFailed { method: "Zstd" })?;
            read_exact_stream(&mut dec, output, "Zstd")
        }
        Method::Oodle => oodle::decompress(input, output),
    }
}

pub fn compress(method: Method, input: &[u8]) -> Result<Vec<u8>> {
    match method {
        Method::None => Ok(input.to_vec()),
        Method::Zlib => {
            let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            std::io::Write::write_all(&mut enc, input)?;
            Ok(enc.finish()?)
        }
        Method::Gzip => {
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            std::io::Write::write_all(&mut enc, input)?;
            Ok(enc.finish()?)
        }
        Method::Lz4 => Ok(lz4_flex::block::compress(input)),
        Method::Zstd => Err(Error::unsupported(
            "zstd compression (write) unsupported; use zlib/lz4/oodle for writing",
        )),
        Method::Oodle => oodle::compress(input, oodle::Compressor::Mermaid, oodle::Level::Normal),
    }
}

/// Oodle bindings via the official oo2core shared library.
pub mod oodle {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[repr(i32)]
    pub enum Compressor {
        Kraken = 8,
        Leviathan = 13,
        Mermaid = 9,
        Selkie = 11,
        Hydra = 12,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[repr(i32)]
    pub enum Level {
        SuperFast = 1,
        VeryFast = 2,
        Fast = 3,
        Normal = 4,
        Optimal1 = 5,
        Optimal2 = 6,
    }

    type DecompressFn = unsafe extern "system" fn(
        *const u8, usize, *mut u8, usize, u32, u32, u32, u64, usize, u64, u64, *mut u8, usize, u32,
    ) -> isize;

    type CompressFn = unsafe extern "system" fn(
        i32, *const u8, usize, *mut u8, i32, *const (), *const (), *const (), *mut u8, usize,
    ) -> isize;

    type GetBufSizeFn = unsafe extern "system" fn(i32, usize) -> usize;

    struct OodleLib {
        _lib: libloading::Library,
        decompress: DecompressFn,
        compress: CompressFn,
        get_buf_size: GetBufSizeFn,
    }

    unsafe impl Send for OodleLib {}
    unsafe impl Sync for OodleLib {}

    static OODLE: OnceLock<std::result::Result<OodleLib, String>> = OnceLock::new();

    fn sha256(data: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(data);
        hex::encode(h.finalize())
    }

    /// Known-good official Oodle 2.9.10 binaries (WorkingRobot/OodleUE mirror of the
    /// EpicGames.Oodle SDK, same mechanism repak uses). SHA-256 verified before load.
    fn platform_info() -> (&'static str, &'static str, &'static str) {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        return (
            "linux/lib",
            "liboo2corelinux64.so.9",
            "ed7e98f70be1254a80644efd3ae442ff61f854a2fe9debb0b978b95289884e9c",
        );
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        return (
            "linuxarm/lib",
            "liboo2corelinuxarm64.so.9",
            "161a8ecca8cc2d4ea6469779c2cc529ed5bb2765d99466273c29fdbef4657374",
        );
        #[cfg(target_os = "macos")]
        return (
            "mac/lib",
            "liboo2coremac64.2.9.10.dylib",
            "b09af35f6b84a61e2b6488495c7927e1cef789b969128fa1c845e51a475ec501",
        );
        #[cfg(windows)]
        return (
            "win/redist",
            "oo2core_9_win64.dll",
            "6f5d41a7892ea6b2db420f2458dad2f84a63901c9a93ce9497337b16c195f457",
        );
    }

    fn candidate_paths() -> Vec<PathBuf> {
        let (_, lib_name, _) = platform_info();
        let mut paths = Vec::new();
        if let Ok(p) = std::env::var("UEX_OODLE_LIB") {
            paths.push(PathBuf::from(p));
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                paths.push(dir.join(lib_name));
                paths.push(dir.join("oodle").join(lib_name));
            }
        }
        if let Some(cfg) = dirs_config() {
            paths.push(cfg.join(lib_name));
        }
        paths.push(PathBuf::from(lib_name)); // CWD + system search path
        paths
    }

    fn dirs_config() -> Option<PathBuf> {
        directories::ProjectDirs::from("org", "uextractor", "UExtractor")
            .map(|d| d.data_local_dir().to_path_buf())
    }

    fn download_oodle() -> std::result::Result<PathBuf, String> {
        let (sub, lib_name, expected_hash) = platform_info();
        let url = format!(
            "https://github.com/WorkingRobot/OodleUE/raw/refs/heads/main/Engine/Source/Programs/Shared/EpicGames.Oodle/Sdk/2.9.10/{sub}/{lib_name}"
        );
        let resp = curl_get(&url)?;
        if sha256(&resp) != expected_hash {
            return Err("downloaded Oodle library hash mismatch — refusing to load".into());
        }
        let dir = dirs_config().ok_or("cannot resolve config dir")?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(lib_name);
        std::fs::write(&path, &resp).map_err(|e| e.to_string())?;
        log::info!("downloaded official Oodle library to {}", path.display());
        Ok(path)
    }

    fn curl_get(url: &str) -> std::result::Result<Vec<u8>, String> {
        let out = std::process::Command::new("curl")
            .args(["-sSLf", "--max-time", "120", url])
            .output()
            .map_err(|e| format!("curl spawn: {e}"))?;
        if !out.status.success() {
            return Err(format!("curl failed ({}): {}", out.status, String::from_utf8_lossy(&out.stderr)));
        }
        Ok(out.stdout)
    }

    fn load() -> std::result::Result<OodleLib, String> {
        let mut errors = Vec::new();
        for path in candidate_paths() {
            if !path.exists() {
                errors.push(format!("{}: not found", path.display()));
                continue;
            }
            unsafe {
                match libloading::Library::new(&path) {
                    Ok(lib) => match lib.get::<DecompressFn>(b"OodleLZ_Decompress") {
                        Ok(sym) => {
                            let decompress: DecompressFn = *sym;
                            let compress: CompressFn = *lib.get(b"OodleLZ_Compress").map_err(|e| e.to_string())?;
                            let get_buf_size: GetBufSizeFn =
                                *lib.get(b"OodleLZ_GetCompressedBufferSizeNeeded").map_err(|e| e.to_string())?;
                            return Ok(OodleLib { _lib: lib, decompress, compress, get_buf_size });
                        }
                        Err(e) => errors.push(format!("{}: symbol lookup failed: {e}", path.display())),
                    },
                    Err(e) => errors.push(format!("{}: {e}", path.display())),
                }
            }
        }
        match download_oodle() {
            Ok(path) => unsafe {
                let lib = libloading::Library::new(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                let decompress: DecompressFn = *lib.get(b"OodleLZ_Decompress").map_err(|e| e.to_string())?;
                let compress: CompressFn = *lib.get(b"OodleLZ_Compress").map_err(|e| e.to_string())?;
                let get_buf_size: GetBufSizeFn =
                    *lib.get(b"OodleLZ_GetCompressedBufferSizeNeeded").map_err(|e| e.to_string())?;
                Ok(OodleLib { _lib: lib, decompress, compress, get_buf_size })
            },
            Err(e) => {
                let detail = errors.join("; ");
                Err(format!(
                    "Oodle library not available ({detail}). Place the official oo2core library next to the executable, set UEX_OODLE_LIB, or enable network access for automatic download. Download failure: {e}"
                ))
            }
        }
    }

    fn oodle() -> Result<&'static OodleLib> {
        let res = OODLE.get_or_init(|| load());
        match res {
            Ok(lib) => Ok(lib),
            Err(e) => Err(Error::Oodle(e.clone())),
        }
    }

    /// Decompress an Oodle stream into `output`; returns bytes written.
    pub fn decompress(input: &[u8], output: &mut [u8]) -> Result<usize> {
        let lib = oodle()?;
        unsafe {
            let n = (lib.decompress)(
                input.as_ptr(),
                input.len(),
                output.as_mut_ptr(),
                output.len(),
                1, // fuzz safe
                1, // check CRC
                0, // verbosity
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                0,
                3, // thread phase: both
            );
            if n <= 0 {
                return Err(Error::Oodle(format!(
                    "OodleLZ_Decompress failed ({n}) for {} input bytes / {} output bytes",
                    input.len(),
                    output.len()
                )));
            }
            Ok(n as usize)
        }
    }

    pub fn compress(input: &[u8], compressor: Compressor, level: Level) -> Result<Vec<u8>> {
        let lib = oodle()?;
        unsafe {
            let cap = (lib.get_buf_size)(compressor as i32, input.len());
            let mut buf = vec![0u8; cap];
            let n = (lib.compress)(
                compressor as i32,
                input.as_ptr(),
                input.len(),
                buf.as_mut_ptr(),
                level as i32,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
            );
            if n == -1 {
                return Err(Error::Oodle("OodleLZ_Compress failed".into()));
            }
            buf.truncate(n as usize);
            Ok(buf)
        }
    }

    /// Check availability without failing (for UI hints).
    pub fn available() -> bool {
        matches!(OODLE.get_or_init(|| load()), Ok(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_zlib_lz4() {
        let data = b"hello compression world".repeat(100);
        for m in [Method::Zlib, Method::Lz4] {
            let comp = compress(m, &data).unwrap();
            let mut out = vec![0u8; data.len()];
            decompress(m, &comp, &mut out).unwrap();
            assert_eq!(out, data);
        }
    }
}
