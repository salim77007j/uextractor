<div align="center">

# UExtractor

**Fast, parallel Unreal Engine `.pak` / `.ucas` / `.utoc` asset extractor — written entirely in Rust.**

GUI + CLI · UE4 & UE5 · AES-256 · Oodle (Kraken/Mermaid/Selkie/Leviathan) · GLTF · PNG/DDS · JSON

</div>

---

## What it does

UExtractor mounts Unreal Engine archive files as a virtual filesystem, extracts them in
parallel across all CPU cores, and converts game assets into industry-standard formats:

| Input | Output |
|---|---|
| `UTexture2D` (DXT1/3/5, BC4, BC5, BC7, ASTC, ETC2, BGRA, FloatRGBA, G8…) | **PNG** (decoded), **DDS** (original compression preserved) |
| `UStaticMesh` / `USkeletalMesh` (cooked vertex buffers, LODs) | **GLTF 2.0 (.glb)**, OBJ fallback |
| `SoundWave` (OGG / FLAC / WAV / ADPCM payloads) | **WAV / OGG** (stream copy or ADPCM→PCM) |
| `DataTable`, `Curve`, `StringTable`, materials, blueprints, any object | **JSON** (properties, imports, exports, metadata) |

## Supported formats

- **`.pak`** — UE4.0 → UE5.x, versions 1–11 (path hash index, Fnv64BugFix), full & partial
  directory indexes, encoded entries, encrypted indices, per-entry encryption, patch archives
- **`.utoc` + `.ucas`** — IOStore (zen) containers, TOC versions 1–8, both header layouts
  (UE4.25–5.3 and UE5.4+), partitions, perfect-hash maps, encrypted directory indices,
  container headers v−1…5, package store entries
- **Compression** — Oodle (official oo2core 2.9.10, hash-verified runtime download),
  Zlib, Gzip, LZ4, Zstd, None
- **Encryption** — AES-256 ECB (UE FAES3) for indices and payloads

## Quick start (CLI)

```sh
# inspect an archive
uex infopakchunk0-WindowsNoEditor.pak

# list contents
uex list pakchunk0-WindowsNoEditor.pak --filter textures

# extract everything (parallel, saturating storage)
uex extract pakchunk0-WindowsNoEditor.pak --out extracted

# extract + convert (PNG/DDS/GLB/JSON/WAV)
uex extract pakchunk0-WindowsNoEditor.pak --convert auto --out out

# AES-encrypted game
uex extract Game.pak -k 0x0123456789ABCDEF... --convert auto --out out

# IOStore containers (UE5)
uex list global.utoc
uex extract . --convert auto --out out        # directory mode: mounts all paks+utocs
```

Exit codes: `0` success · `1` runtime error · `2` usage error. `--format json` gives
pipeline-friendly machine output.

## Quick start (GUI)

```sh
uex-gui
```

Dark-themed professional UI: archive tree browser, asset list/grid, live previews
(decoded textures, software-rendered 3D mesh viewport with drag-rotate), batch
extraction queue with progress/speed/ETA, AES key manager, export settings,
drag-and-drop, recent archives.

## Building

```sh
cargo build --release
# binaries: target/release/uex (CLI) and target/release/uex-gui
```

Requires Rust 1.75+. No external C dependencies. The Oodle library is loaded at runtime
(placed next to the executable, found via `UEX_OODLE_LIB`, or auto-downloaded with
SHA-256 verification from the official EpicGames.Oodle SDK mirror). Without Oodle the
tool still fully supports Zlib/Gzip/LZ4/Zstd archives.

## Testing

```sh
cargo test --workspace
uex gen-testdata --out testdata   # byte-exact synthetic archives (PAK v11, encrypted, IOStore v5)
uex bench --size-mb 512 --compression oodle
```

The test suite builds synthetic PAK/IOStore archives from scratch and round-trips them
through the real parser — covering all compression methods, AES-encrypted indices and
entries, multi-block entries, zen package headers, name maps, and asset conversion.

## Performance

Measured on a 2-core cloud VM (hypervisor-limited; scales further on real hardware):

| Workload | Throughput |
|---|---|
| Oodle decompression (Kraken payload) | ~1.7 GB/s |
| LZ4 extraction | ~1.3 GB/s |
| Zlib extraction | ~0.43 GB/s |
| Memory (1 GB archive) | **10 MB peak RSS** |

## Architecture

```
uex-core     archive parsers (PAK/IOStore), crypto, compression, uasset/uexp/uubulk,
             property serialization (versioned + unversioned), converters, exporters
uex-cli      full CLI (info/list/extract/bench/test-key/gen-testdata)
uex-gui      eframe/egui desktop app (tree, previews, 3D viewport, queue, key manager)
uex-testdata synthetic archive builders used by the test suite
```

## Legal

UExtractor parses published game archives for interoperability and archival purposes.
The Oodle SDK binary is Epic's property and is loaded (or downloaded with hash
verification) at runtime rather than redistributed by this repository. Respect the EULA
of any game you extract assets from; do not redistribute copyrighted content.

## License

MIT
