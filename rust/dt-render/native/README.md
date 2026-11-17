# dt-render native sources

The archives contain source code, not binaries or media. They are kept here because the published
`libaom-sys` build does not expose the generic-CPU switch required on machines without NASM.
`build.rs` extracts them into a content-keyed directory under Cargo's build output. CMake builds
static libraries with `AOM_TARGET_CPU=generic`; it never downloads tools or dependencies.

- `aom.tar.gz`: the unmodified `vendor/` source tree from crates.io
  `libaom-sys 0.18.1+libaom.3.15.3` (libaom 3.15.3).
  SHA-256: `dfb6374b75d16dbb1c0611faae60b7f7d2362b79d11ee142b8a53de107dde734`.
- `avif.tar.gz`: the unmodified `libavif/` source tree from crates.io
  `libavif-sys 0.18.0+libavif.1.4.2` (libavif 1.4.2).
  SHA-256: `011abc2a6387a0faa2458e5be363f0e759b7ad6c48104aaba9f9af5aa5d46fc0`.

Both trees include upstream license, patent, and third-party notices. libaom's BSD license and
patent grant are in `aom/LICENSE` and `aom/PATENTS`; libavif's BSD license is in `avif/LICENSE`.
Upstream sources: <https://aomedia.googlesource.com/aom/> and
<https://github.com/AOMediaCodec/libavif>.

Only `codec.c` and the top-level `CMakeLists.txt` are project-specific. The C shim owns each
libavif image and encoder, borrows Rust RGB buffers for one synchronous call, and returns an owned
byte allocation. `src/native.rs` is the sole unsafe Rust boundary and frees that allocation once.
The codec uses PSNR tuning, 8-bit full-range BT.601 YUV420, a 1000-unit timebase, and explicit
single-play or infinite repetition semantics. Installed Segoe fonts are read at runtime, never
embedded or redistributed.

MSVC and CMake are required. libaom also uses Perl for generated configuration; the existing Git
for Windows copy is detected at `C:/Program Files/Git/usr/bin/perl.exe`. No NASM, YASM, clang,
Python, or external media encoder is needed to build the crate or encode a theme clip. ffmpeg is
needed only for source decoding and the explicitly selected experimental ffmpeg backends.
