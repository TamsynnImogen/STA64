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

| Kind | Legacy (read) | Native (read/write) |
|---|---|---|
| Data/config | ODF, cfg, spr, techtree, `info.ini` | JSON5 |
| Models | SOD | glTF/GLB (SODX deferred) |
| Textures | TGA, DDS (incl. Borg `_b`, animated `_1.._9`) | KTX2 (PNG fallback) |
| Maps | BZN | JSON5 scene + glTF |
| Archives | FPQ v2 (raw/LZO) | plain folders |
| Audio | WAV, MP3 | OGG |
| Video | BIK (via FFmpeg) | WebM/AV1 |
| Scripts | FO Lua | Lua (same API, extended) |

## Legal boundary

- No original game binaries, art, audio, data or decompiled code in this
  repository.
- Behaviour is documented from observation and from our own analysis notes,
  then reimplemented. Addresses and reverse-engineering notes stay in the
  analysis repositories, not here.
- Third-party code, such as MIT-licensed Armada II Refit code, may be reused
  with its licence notice kept.
