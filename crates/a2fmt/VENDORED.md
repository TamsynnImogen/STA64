# Vendored: a2fmt

- Upstream: <https://github.com/cedrichaase/armada2-refit>, `crates/a2fmt/`
  (earlier syncs used the old path `render3d/a2fmt/`)
- Commit: `4177bd948cd31f2fbcf00ebc822021ed1a192172` (previously
  `37d60079169b086dba4216fe55e05cd6015d66c0`, 18 October 2026)
- Licence: MIT, Copyright (c) 2026 Cedric Haase. See `LICENSE`, which must
  stay with this code.
- Local changes (re-apply after every sync):
  - Added as a member of the STA64 workspace.
  - `Cargo.toml`: vendoring comment; clippy lints allowed for this crate
    (keeps upstream style).
  - `tests/stock.rs`: reads the game directory only from `A2_GAME`; Refit's
    `a2env.sh` fallback (and the `Command` import) was removed. Accepts TGA
    type 1 in the stock-kinds assertion and skips it in the orientation spot
    check.
  - `src/tga.rs`: decodes colour-mapped TGAs (types 1 and 9; new private
    `colour_map` helper, `decode` changes, doc note). The GOG Armada II
    release ships one (`Textures/RGB/8472_behemoth.tga`, type 1, 8-bit index).
    Upstream still refuses them.
- Not vendored: upstream's `Cargo.lock` (STA64 has its own workspace lock).

Prefer fixing parser bugs upstream as well as here, and record each local
change in this file.
