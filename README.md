# STA64

A native 64-bit engine for Star Trek Armada II and Fleet Operations content,
written in Rust on [Bevy](https://bevyengine.org).

STA64 is a clean-room engine. It ships **no original game content**: it reads
models, textures, ODFs, maps and sounds from a user-owned Armada II / Fleet
Operations install at run time. New content uses open formats (JSON5, glTF/GLB,
KTX2, OGG).

The features that today come from patching the old 32-bit engine —
FleetOpsHook.dll, A2FOExtensions and the Armada II Refit layers — are built
into STA64 as first-class, data-driven systems instead. See
[docs/NATIVE_FEATURES.md](docs/NATIVE_FEATURES.md).

## Platforms

| Target | Status |
|---|---|
| Linux x86-64 | primary |
| Windows x86-64 | required |
| macOS Apple Silicon (aarch64) | planned, built in CI |
| Linux ARM64 | planned, built in CI |

## Layout

| Crate | Purpose | Bevy? |
|---|---|---|
| `sta_vfs` | Install/mod mounting: `Data` + `ParentMod` chain, FPQ archives, case-insensitive lookup | no |
| `sta_fpq` | FPQ v2 archive reader + bounds-checked LZO1X decoder (`--example fpq` lists/extracts) | no |
| `sta_odf` | ODF parser, `#include` resolution, typed accessors | no |
| `sta_sim` | Deterministic fixed-point lockstep simulation | no |
| `sta64` | The game: rendering, UI, audio, input, glue | yes |

Format and simulation crates stay Bevy-free so they can be tested headless,
reused by tools, and kept bit-identical across platforms.

## Building

```sh
cargo test --workspace --exclude sta64   # fast, no Bevy
cargo run -p sta64 -- --install-root "/path/to/Fleet Ops" --mod "Fleet Ops 4.0"
```

The toolchain is pinned in `rust-toolchain.toml`. Linux builds need the usual
Bevy system packages (`alsa-lib-devel`, `systemd-devel`/`libudev`, Wayland/X11
headers).

## Docs

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — engine structure and rules
- [docs/NATIVE_FEATURES.md](docs/NATIVE_FEATURES.md) — FleetOpsHook / A2FOExtensions / Refit → native systems
- [docs/ROADMAP.md](docs/ROADMAP.md) — milestones
