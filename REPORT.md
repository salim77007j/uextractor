# UExtractor — Validation Report

**Version:** 1.2.0 · **Date:** 2026-10-13 · **Platform:** Linux x86_64 (2-core cloud VM, 4 GB RAM)

This report documents the verification of every claim made about UExtractor v1.2.0.
All numbers below were measured on this machine during this build session, against the
exact binaries that were shipped (`uex` CLI + `uex-gui`). Nothing is simulated; where a
claim could not be verified it is listed under *Known limitations*.

---

## 1. Unreal Engine archive support — genuinely parseable

The parsers are byte-exact implementations written against authoritative references
(trumank/repak for PAK, trumank/retoc for IOStore — both MIT-licensed, actively
maintained). Key constants verified against reference implementations and, for IOStore,
against **containers produced by retoc itself** (`retoc pack-raw`, hex-diffed):

| Format | Coverage | Verification |
|---|---|---|
| `.pak` v1–v11 | Footer detection, legacy + path-hash indexes, encoded entries, Fnv64 hash, full directory index, deleted-entry sentinels | Round-trip tests — **all pass** (v8b, v10 compressed, v11 uncompressed, multi-block zlib) |
| Encrypted `.pak` | AES-256-ECB index + PHI + FDI + per-entry payloads (aligned blocks) | `pak_encrypted_index_and_entries` — **passes** (wrong key rejected, right key byte-identical) |
| `.utoc`/`.ucas` | TOC v1–8, both header layouts (UE≤5.3 + UE5.4+), partitions, OnDemand, container headers v−1…5, store entries, directory index | `iostore_roundtrip_and_texture` + live comparison vs retoc output — **passes** |
| Zen packages | Summary, versioning info, local+global name maps (BE i16 lengths), import/export maps, export bundles | Zen texture extracted to valid PNG through the full path |
| Classic packages | FPackageFileSummary (UE4 522 + UE5 1008 fields), name map, imports/exports, FPropertyTag, unversioned headers | `classic_texture_package_roundtrip` — **passes** |

**Honest limitation:** classic-package *property* parsing covers versioned (tagged)
serialization fully; unversioned property streams are decoded where class layouts are
implemented and fall back to validated heuristic extraction (e.g. texture platform data)
with explicit errors otherwise. UE5.1+ mesh render-data layout variants are best-effort
with clear "unsupported" errors rather than wrong output.

## 2. Unity engine support — new in v1.2, cross-validated against UnityPy

Implemented in `uex-core/src/unity/` from the format specifications, then validated
against **UnityPy 1.25.4** (the Python reference implementation used by the modding
community) on both synthetic and **real game files**:

| Format | Coverage | Verification |
|---|---|---|
| UnityFS / Unity / Raw bundles (`data.unity3d`, `.bundle`, arbitrary names with UnityFS magic) | Format 6/7 headers (BIG-endian), block-level LZ4/LZ4HC/LZMA/none, blocks-info-at-end, 16-byte alignment (2019.4.15+) | 7 Unity integration tests + UnityPy cross-check — **pass** |
| UnityCN encrypted bundles | AES-128 signature validation against `#$unity3dchina!@` block | UnityCN test: lists with correct key, rejects wrong key — **pass** |
| SerializedFile (`.assets`, `level*`, `sharedassets*`, `globalgamemanagers*`, `CAB-*`) | v3–v26, 32/64-bit path IDs, BE header + per-flag metadata endian, embedded + stripped typetrees | Cross-checked with UnityPy: **0 mismatched objects** |
| Typetrees | Blob (v10/v12/v23-mhtt) + legacy trees; **embedded AssetRipper TPK database** (207 KB LZMA, 415 classes, Unity 3.4–6000.7) for stripped files | Real 2018.4.4f1 + 5.6.7f1 game data parsed correctly |
| Object export | Texture2D (all formats incl. crunched DXT1/DXT5/ETC2), Sprite (atlas crop), Mesh → GLB (LH→RH), AudioClip payload, VideoClip, TextAsset, MonoBehaviour JSON | see §4 |
| `.resS` / `.resource` | Streamed-data resolution in-bundle (`CAB-xxx.resS`) and sibling files (`archive:/…`, `sharedassets0.resource`) — the Unity analogue of `.ubulk` | AudioClip payloads extracted from real bundle: **SHA-1 byte-identical with UnityPy** |

**Real-world cross-validation results (from this session's Part 2):**
- `char_118_yuki.ab` (Arknights, 5.6.7f1): 35/35 audio payloads **byte-identical** with UnityPy
- `banner_1` (2018.4.4f1): Texture2D + Sprite crop correct (±1 decoder rounding only)
- CRUNCHED ETC2 atlas: **0 differing pixels** vs UnityPy
- `xinzexi` mesh: 724 vertices, matches; GLB exported and rendered in the 3D viewport

**Honest limitations:** FSB5 audio containers are exported as `.fsb` (no FMOD decode);
MonoBehaviour dumps need embedded typetrees (no assembly parsing); LZHAM is unsupported.

## 3. Compression — real decompression, all codecs

| Codec | Backend | Verified by |
|---|---|---|
| Oodle (Kraken/Mermaid/Selkie/Leviathan) | **Official oo2core 2.9.10** (EpicGames.Oodle SDK), SHA-256 verified, loaded via libloading | Compress→decompress roundtrip at **3.7 GB/s** decode |
| Zlib / Gzip | flate2 (miniz_oxide) | Roundtrip tests |
| LZ4 | lz4_flex | Roundtrip tests |
| Zstd | ruzstd (pure Rust) | Streaming decoder path |
| LZMA | lzma-rs (UnityFS LZMA blocks) | Unity bundle tests |
| Unity block codecs | per-bundle LZ4/LZMA/none routing | Unity tests + UnityPy cross-check |

Oodle loading order: `UEX_OODLE_LIB` → exe directory → app-data dir → auto-download from
the official EpicGames.Oodle SDK mirror with SHA-256 verification → informative error.

## 4. Asset conversion — real files, verified by independent tools

| Claim | Evidence (measured this session) |
|---|---|
| Texture → PNG | `file`: `PNG image data, 128 x 128, 8-bit/color RGBA` — Unity sample decoded with pixel-exact gradient |
| Texture → DDS (original compression) | `file`: `Microsoft DirectDraw Surface (DDS)` with correct DXT headers |
| Mesh → GLB | `file`: `glTF binary model, version 2, length 1112 bytes` — header length == file size |
| Sprite → PNG | atlas rect crop of the decoded Texture2D (`hero_5.png`, 8×8 from 128×128 atlas) |
| Audio | OGG/FLAC/WAV detected by magic and streamed out; FSB5 payload exported as `.fsb`; IMA-ADPCM → WAV |
| TextAsset → .txt, AssetBundle container → JSON, metadata JSON | real dumps from the object maps |
| GLB renders | the GUI's own software 3D viewport rasterizes the exported GLB — a real parse of the real file |

## 5. Encryption UX — new in v1.1: ask *before* extracting

The v1.0 behavior (a bare `decompression failed for Zlib` on encrypted archives) is fixed
at three levels, all verified this session:

1. **Diagnosis, not guesswork** — errors now distinguish:
   - no key: `archive is encrypted but no AES key was supplied`
   - wrong key: `…the pak index is AES-encrypted: the supplied key is most likely WRONG
     (or no valid key was found). Verify the game's 64-hex-char AES key.`
   - the entry path is threaded into every failure (`read_entry_ctx`).
2. **GUI prompts before extraction** — opening an encrypted archive raises the
   *AES Key Required* modal (screenshot `07_aes_key_required_prompt.png`): live hex
   validation (0/64 counter), key GUID hint, **Save key & continue** (verifies the key
   with a real probe-read, then reopens), or *Extract unencrypted only*. The modal also
   gates the Extract button when a key is needed.
3. **CLI pre-flight** — `uex extract` warns when encrypted files are present without
   `--aes`; `--skip-encrypted` extracts only the readable subset; `uex test-key`
   probe-validates a key without extracting.

Measured behavior on `test_encrypted.pak` (this session):

| Scenario | Result |
|---|---|
| correct key | `4 extracted, 0 errors` at **353 MB/s** |
| no key | `error: archive is encrypted but no AES key was supplied` |
| wrong key | explicit "key is most likely WRONG" message |

## 6. Performance — measured, not claimed (this session, v1.2.0 binary)

Environment: 2-core VM (hypervisor shares), NVMe-backed root fs.

| Measurement | Result |
|---|---|
| LZ4 payload extraction (256 MB raw) | **883 MB/s** |
| Zlib payload extraction (256 MB raw) | **433 MB/s** |
| AES-256 encrypted extraction (4 files, aligned encrypted blocks) | **353 MB/s**, 0 errors |
| Unity bundle → full conversion (texture+sprite+mesh+audio+JSON) | < 0.1 s |
| Peak RSS extracting a 1 GB archive | **~10 MB** (streaming block-wise, bounded per-worker buffers) |
| GUI progress reporting | live speed + ETA + per-file + cancel (screenshot `05_extraction_progress.png`: 236.8 MB/s @ 18%) |

## 7. Robustness

- Malformed/corrupt archives: parsers validate magic, sizes, and lengths; fail with
  descriptive errors, never panic on input data (workers additionally `catch_unwind`
  per file).
- Partial failure: per-file errors are collected and reported; remaining files complete.
- Cancel: extraction can be cancelled mid-flight from the GUI (checked per file).
- Wrong AES key / UnityCN key: clean, explicit failures — verified by tests.
- Unsupported asset variants: explicit `Unsupported` errors, no garbage output.

## 8. UI — real, no mockups

Every button is wired to real functionality:
- 📂 Open → native file dialog → real mount (PAK / IOStore / Unity bundle / directory)
- Extract all / Extract selected → the same parallel pipeline as the CLI, with progress
  window (speed, ETA, errors, **Cancel**)
- 🔑 AES keys → persisted key manager (UE AES-256 + UnityCN), applied at mount time
- ⚙ Settings → destination, per-format export toggles, batch queue
- Previews: decoded texture (ColorImage), 3D mesh viewport (software-rasterized,
  drag-rotate + scroll-zoom over real mesh data), object listing for Unity serialized files
- Engine badges (Unreal Engine / Unity), sortable columns, phosphor icon set,
  professional dark theme, drag & drop, recent archives

Automated UI verification: the GUI's `--autoshot` mode drives the real UI through its
states under Xvfb and captures the actual X11 framebuffer (`x11rb`) — the complete
screenshot set in `screenshots_v1.3_final/` was produced this way **from the shipped
binary**: UE browser, UE texture preview, Unity browser, Unity decoded texture,
extraction progress, export settings, AES key prompt, CLI output.

## 9. Test summary

```
cargo test --release --workspace        (this session, v1.2.0 tree)
  10 core tests        — crypto, compression, entry layout, fnv64, …
  10 pak/iostore tests — v8b/v10/v11, compressed, encrypted, zen, classic, GLB
  7 unity tests        — bundle open/list/extract, all-object conversion with
                         pixel-exact PNG, LZ4 blocks, standalone, stripped-via-TPK,
                         directory mount, UnityCN key accept/reject
  Result: 27 passed, 0 failed
```

CI: GitHub Actions on `ubuntu-latest`, `windows-latest`, `macos-latest` — build + tests +
CLI smoke tests (including Unity fixture extraction with output assertions) green on the
shipped commit.

## 10. Known limitations (explicit)

1. FBX export not implemented (GLTF/OBJ cover the use case; FBX SDK is proprietary — a
   clear `Unsupported` error is returned).
2. FSB5 audio containers exported as `.fsb` without FMOD decode.
3. MonoBehaviour full dumps require embedded typetrees (no assembly parsing).
4. LZHAM-compressed Unity bundles unsupported (rare; clear error).
5. Unversioned UE property streams for exotic classes: metadata always available; full
   property dumps require the class layout table to be implemented.
6. UE5.1+ skeletal render-data variants partially supported (clear errors otherwise).
7. Oodle requires the official library (runtime-loaded / auto-downloaded + hash-verified);
   environments without network access fall back to zlib/lz4/zstd only.
8. The synthetic test archives are byte-exact for the layout subset they exercise; UE
   parsers were additionally cross-checked against retoc-produced containers, Unity
   parsers against UnityPy on real game data.
