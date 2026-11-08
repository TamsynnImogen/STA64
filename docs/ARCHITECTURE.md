# Architecture

## Engine choice

**Bevy** for the client. Its renderer (wgpu) covers Vulkan, DirectX 12 and
Metal from one codebase, which gives Linux, Windows, Apple Silicon and ARM64
without per-platform renderers. The ECS fits an RTS with thousands of
entities, and glTF/KTX2 loading is built in.

Bevy's API changes with every release, and its scheduling is not designed for
deterministic lockstep. Two rules keep that from becoming a problem:

1. **The simulation does not depend on Bevy.** `sta_sim` is plain Rust and
   only takes commands and returns state. Bevy reads that state to render and
   interpolate. A Bevy upgrade can never cause a desync, and the sim can run
   in a headless server, a replay verifier or a test.
2. **Formats and tools do not depend on Bevy.** `sta_vfs`, `sta_odf` and the
   future `sta_sod`, `sta_tex`, `sta_spr`, `sta_bzn` and `sta_fpq` crates are
   usable from command-line tools, the asset workbench, and other projects.

If Bevy ever stops fitting, only the `sta64` crate has to change.

## Layers

```
          user-owned install (Data, Mods, FPQ)       STA64 Data (open formats)
                          \                             /
                           sta_vfs  (layered, case-insensitive)
                              |
        sta_odf / sta_sod / sta_tex / sta_spr / sta_bzn / json5 loaders
                              |
                    content database (typed, validated)
                       /                         \
         sta_sim (fixed-point, lockstep)     sta64 client (Bevy)
         commands in, state + digest out     render / UI / audio / input
                       \                         /
                        net (lockstep command relay)
```

## Simulation rules

- Fixed-point (`Fixed`, 32.32) for all sim state. No `f32`/`f64` in state.
- Fixed tick rate (`TICK_HZ`); the renderer interpolates between ticks.
- Entities live in id-ordered storage; no hash-order iteration.
- One seeded RNG per sim.
- Per-tick state digest for desync detection and replay verification.
- Player commands are the only input. Replays are the seed plus the command
  log.

## Content profiles

One engine, several rule profiles chosen by the mounted mod:

| Profile | Purpose |
|---|---|
| `STA64` native | Main showcase. New features and formats start here. |
| `FO4` | Fleet Operations 4 compatibility. |
| `STA2 Classic` | Stock Armada II behaviour. |
| `STA1 Classic` | Armada I behaviour (e.g. `wingman`→`craft`, shields-then-death health model, `addon/` ODF layout). |

Legacy profiles exist to keep their content and behaviour working. They are
not where new native features go.

## Formats

Every legacy format stays supported. New native content uses modern open
formats. Where each legacy format came from:

| Origin | Models | Textures |
|---|---|---|
| Armada II (stock) | SOD | TGA (`Textures/RGB/`) |
| Fleet Operations | — | DDS (mostly `Textures/`) |
| A2FOExtensions | M3D (Star Trek: Legacy), converted at load | — |
| STA64 native | glTF / GLB | KTX2 (PNG fallback) |

Lookup by bare name (as ODFs and SODs reference assets): a file in a
higher-priority mod layer always wins. Within one layer the newest format
wins: KTX2 > DDS > TGA > PNG, and GLB > glTF > SOD > M3D. This matches
A2FOExtensions: an M3D in a child mod overrides a parent's SOD, but a SOD
beside an M3D in the same mod wins.

| Kind | Legacy (read) | Native (read/write) |
|---|---|---|
| Data/config | ODF, cfg, spr, techtree, `info.ini` | JSON5 |
| Models | SOD (1.4–1.93), M3D | glTF/GLB (SODX deferred) |
| Textures | TGA (incl. colour-mapped), DDS (DXT1/3/5, uncompressed), Borg `_b` variants, hand-authored `_1.._n` mip chains, height maps | KTX2 (PNG fallback) |
| Maps | BZN | JSON5 scene + glTF |
| Archives | FPQ v2 (raw/LZO) | plain folders |
| Audio | WAV, MP3 | OGG |
| Video | BIK (via FFmpeg) | WebM/AV1 |
| Scripts | FO Lua | Lua (same API, extended) |

`crates/a2fmt` is vendored from Armada II Refit (MIT, see its `VENDORED.md`)
and provides the SOD, TGA, sprite-table and ODF readers.

## Legal boundary

- No original game binaries, art, audio, data or decompiled code in this
  repository.
- Behaviour is documented from observation and from our own analysis notes,
  then reimplemented. Addresses and reverse-engineering notes stay in the
  analysis repositories, not here.
- Third-party code, such as MIT-licensed Armada II Refit code, may be reused
  with its licence notice kept.
