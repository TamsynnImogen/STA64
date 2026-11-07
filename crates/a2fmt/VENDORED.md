# Vendored: a2fmt

- Upstream: <https://github.com/cedrichaase/armada2-refit>, `render3d/a2fmt/`
- Commit: `37d60079169b086dba4216fe55e05cd6015d66c0` (18 October 2026)
- Licence: MIT, Copyright (c) 2026 Cedric Haase. See `LICENSE`, which must
  stay with this code.
- Local changes:
  - Added as a member of the STA64 workspace.
  - `tests/stock.rs` reads the game directory only from `A2_GAME`; Refit's
    `a2env.sh` fallback was removed.

Prefer fixing parser bugs upstream as well as here, and record each local
change in this file.
  - `src/tga.rs`: decodes colour-mapped TGAs (types 1 and 9). The GOG Armada II
    release ships one (`Textures/RGB/8472_behemoth.tga`, type 1, 8-bit index).
    `tests/stock.rs` accepts type 1 and skips it in the orientation spot check.
  - `Cargo.toml`: clippy lints allowed for this crate (keeps upstream style).
