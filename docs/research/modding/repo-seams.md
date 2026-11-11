# Repo seams for the pack / profile layer

Read-only survey, 2026-11-11, worktree `modding-rfc` (base `71da97b`). Source
design: `docs/MODDING-IDEAS.md`. All paths relative to the repo root. Line
numbers are from this checkout and drift; module docs are cited by file.

## 1. Headline

A large part of the "pack" abstraction **already exists as `sta_vfs` mod
machinery that no client uses yet**. The biggest gaps are not new formats but
(a) wiring: clients mount through `Vfs::mount_install(root, Option<&str>)`
(a `ParentMod` chain over a hardcoded `Data` layer), not through the manifest
and load-order code; (b) `Profile` is a **closed 4-variant enum** selected by
`info.ini` heuristics, not by exe fingerprint, and the sim branches on it in a
few places; (c) there is no rules-layer patch format, no provenance, no
render-slot registry and no declarative trigger layer. Lua (MMM) exists and is
the natural tier-3 script escape hatch.

## 2. What exists and already matches the design

### 2.1 VFS: layered, archives, whiteouts (matches "content" layer)

- `crates/sta_vfs/src/lib.rs:1-20` (module doc), `:66` `Layer`, `:94` `Source`
  (`Loose | Packed | Zfs | Sta | Whiteout`), `:126` `Vfs`, `:171` `push_dir`,
  `:253` `resolve`, `:261` `resolve_all` (every layer that has a path: the
  hook a "why did this file win" tool needs), `:305` `list`.
- Case-insensitive normalised paths (`:119` `normalize`); within a layer loose >
  `.sta` > FPQ > ZFS (`lib.rs:14-20`).
- Whiteouts (`.sta` only) hide a path in all lower layers: the "delete" half of
  a content overlay already exists.
- **Base game = read-only layer: true today.** Nothing writes to the install.
  Flat A1/A2 roots mount as one `root` layer (`lib.rs:139-168`,
  `docs/INSTALL_ROOTS.md`, `docs/A2_INSTALL_ROOTS.md`, WORKPLAN CON-27/CON-29).

### 2.2 Manifests, dependency graph, load order (matches `extends/requires`, T0/T1)

- `crates/sta_vfs/src/manifest.rs:1-30,104`: `manifest.toml` `[mod] id/name/
  version/engine/profile/kind`, `[dependencies]` (semver reqs), `[load] after/
  before`, unknown keys kept in `Manifest::extra` (forward compatible).
- `manifest.rs:190` `Manifest::from_info`: **a legacy mod with only `info.ini`
  gets a generated manifest** (id from folder, `ParentMod` -> dependency).
  This is exactly the T0/T1 "infer manifest from folder" behaviour.
- `crates/sta_vfs/src/order.rs:129` `resolve(mods, active) -> LoadOrder`: transitive
  dependency walk, version checks, cycle refusal, `[load]` hints,
  `LoadOrder::identity()` (`:74`, SHA-256 over ids, versions, `.sta`
  `content_hash`) and `LoadOrder::mount(data)` (`:92`).
- `crates/sta_vfs/src/mods.rs:98` `ModInfo` (all `info.ini` `[Mod]` keys incl.
  `ParentMod`, `standalone`, `ignoreParentMaps`, `AssetVersion`,
  `[modules] active/required` = Roots hook module lists, `:170` `list_mods`).
- `.sta` archive (`sta.rs`, `sta_write.rs`, `sta_sign.rs`, `docs/STA_ARCHIVE.md`):
  reproducible pack, per-file SHA-256, `content_hash`, Ed25519 signatures,
  `convert.rs` FPQ/ZFS mod -> `.sta`. Covers "reversible installs",
  distribution and trust for native packs (WORKPLAN CON-16..19, EDT-08).
- `docs/STA_ARCHIVE.md:160-190`: manifest spec, "Mounting and load order".
  The pack manifest in MODDING-IDEAS should be **an extension of this file**,
  not a second format (`toml = "1.2.1"` already a workspace dep, `Cargo.toml:63`).

### 2.3 Profile detection and feature switches (matches "profile = base + quirks")

- `crates/sta_content/src/features.rs`: `enum Profile { Native, Fo4,
  Sta2Classic, Sta1Classic }`; `struct Features` (`#[non_exhaustive]`, 22 named
  bool fields, `bits()` for the hash). These are the nearest thing to "named
  quirks" and are documented per row in `docs/NATIVE_FEATURES.md`.
- `crates/sta_content/src/profile.rs:1-41` (module doc), `:145` `Profile::detect`:
  1 explicit `[Mod] Profile=` in any layer's `info.ini` (top first);
  2 A1 `.zfs` signature; 3 FO name/folder/`mmm_loader.dll` hints (incl. `Data`
  since CON-31, `docs/FO_PROFILE_DETECTION.md`); 4 stock A2 layout; 5 Native.
  `:97` `Profile::features()` defaults table; `:317` `Overrides`; `:365-390`
  `from_info_ini` / `from_vfs` read `[Features]` of every layer, child over
  parent; `:404` `resolve_features(profile, &[mod, odf])`:
  **defaults < mod `[Features]` < per-ODF `Feature_<name>` keys**.
- This is already a three-level quirk-override system. A "profile as rules
  pack" is an *extension of an existing precedence chain*, not a replacement.
- Native ODF extension keys exist: `ODF_KEY_PREFIX` (`Feature_`), and
  `STA64_queueLimit` (TODO.md:61, SIM-20) overriding a profile default per
  producer. Precedent for "ODF key semantics are data".
- `ContentDb::load` (`crates/sta_content/src/db.rs:70`) calls
  `Profile::detect`; `load_as` (`:76`) takes a forced profile (lobby choice,
  tests): the override point for a pack-declared profile.
- `ContentDb::hash` (`db.rs:389`) folds profile + feature bits + defs: any
  rules-layer value that reaches the sim must be folded here (determinism).

### 2.4 Canonical DB seam (matches "canonical engine-neutral DB")

- S5 in `docs/WORKPLAN.md:239-274`: `ContentDb`, `UnitDef`, `WeaponDef`
  (`crates/sta_content/src/def.rs`), `Profile`, `Features`, `OdfIndex`;
  `UnitDef::raw: Arc<sta_odf::Odf>` keeps unread keys reachable (the "lossless
  typed tree, unknown keys preserved" requirement at the ODF level).
- `fake::db()` lives at `crates/sta_content/src/fake.rs` (6 units, 3 weapons);
  JSON5 native content is planned (CON-04, `sta_content/src/json5.rs`, **not
  present in the tree**; no `json5.rs` under `crates/sta_content/src`). The
  canonical DB therefore currently has exactly one producer: ODFs.
- Pipeline today: VFS bytes -> `sta_odf` (`parse`, `resolve`, `resolve_layered`
  `crates/sta_odf/src/lib.rs:172,179`) -> `crates/sta_content/src/odf.rs`
  (classifies by class label; typed fields + warnings) -> `ContentDb`.
  Other typed readers: `techtree.rs`, `race.rs`, `resources.rs`,
  `confinement.rs`, `strings.rs`, `gui_cfg.rs`, `aip.rs`, `map.rs`.
- Sim consumption: `sta_sim` depends on `sta_content` only through the DB
  (WORKPLAN crate-direction block, `docs/WORKPLAN.md:~100`).

### 2.5 ODF include/override semantics (matches "inherit / include")

- `sta_odf::resolve_layered` (CON-01): FO self-include = "next version down"
  across mod layers; `Odf::cut` lists cut includes. This **already is
  whole-ODF inheritance across layers**, i.e. the FO `#include "x.odf"` idiom is
  the existing key-level patch mechanism for FO mods (child file includes
  itself, overrides only the keys it writes).
- `EDT-09` duplicate-key report (`sta_odf/src/diagnostics.rs`); `Entry::line`
  (`sta_odf/src/lib.rs:36`) gives file:line for diagnostics.
- Gap: the parser is **lossy** for provenance (resolved `Odf` keeps raw text
  but `Entry.line` does not carry layer/file after include flattening, to
  be verified); EDT-06 plans a lossless syntax tree in `sta_odf`.

### 2.6 Asset codecs and lookup (matches "codec -> typed tree")

- `crates/sta_assets/src/lib.rs:1-12,108-125`: `find_texture`, `find_model`
  rank formats per layer (`KTX2 > DDS > TGA > PNG`, `GLB > glTF > SOD > M3D`);
  `load_model` (`:173`) -> one `model::Model` for SOD, M3D and glTF.
- CON-09 glTF (`sta_assets/src/gltf.rs`, `model/pbr.rs`, contract
  `docs/M1_ASSET_MODEL_COMPLETION.md`): resolves dependent files **through the
  VFS** with unsafe-URI rejection (`gltf.rs:10-40`). A pack's native models
  are served by the same overlay with no extra code.
- M3D (CON-08, `docs/M3D_NODE_SUPPORT.md`) = A2FOExtensions format; "M3D in a
  child mod overrides parent SOD" rule already in `ARCHITECTURE.md`
  (Formats section, ~line 94-98).
- Audio (CON-10), BMP (CON-21), sprite fonts through the mod chain (UI-12,
  `sta64/src/ui/sprite_font.rs`). All read via `Vfs`.

### 2.7 Data-driven rules that already behave like "quirks"

- Tech tree `NotListed` rule: `crates/sta_content/src/techtree.rs:399-460`
  (`availability_for_builder`, `unlisted_item_allowed`). Inferred from
  install data ("The engine's own rule is undocumented"); gated only by ODF
  `builditemN availability/provided` lines and an `ai` flag, **not by
  profile**. Candidate quirk: `tech.unlisted_items_buildable`. Sim gate and HUD
  share it (`sta64/src/ui/hud/model.rs:272`).
- Profile-matched tables: `resources.rs:84` (`ResourceModel::for_profile`,
  FO4 vs others, refined by `StringTable`), `confinement.rs:140-241`
  (`ConfinementInitial` by profile, map rules from ODF), `kinematics.rs:88`
  (`Model::of(profile)`: Smooth vs Native), `Profile::build_queue_limit`.
- Per-ODF `Feature_*` switches and `STA64_*` keys (see 2.3).

### 2.8 Menu scene is already data-driven (matches the menu-scene sketch)

- `crates/sta64/src/frontend/scene.rs:1-30`: `MenuMap` parsed from
  `assets/menu/map.ini` (Refit port); **"a `Menu/map.ini` in the mounted
  install takes precedence"**; named views (`MenuRequest` message),
  `CloakView`, departures; a `Director` (no Bevy, deterministic in scene time)
  and marks to `RenderMarks`. The tier-2 `[scene.main_menu]`/`on`/`timeline`
  sketch is an evolution of this ini, and the override-by-install pattern is
  already the pack mechanism for it. WORKPLAN UI-06; UI-07 (screens, mod
  select, `frontend/screens/data.rs:89` `mods_in` uses `list_mods`).

### 2.9 Scripting

- Mission Lua is MMM-compatible, Lua 5.3 via `mlua` (vendored), see
  `docs/MISSIONS.md`, `crates/sta_mission/Cargo.toml:14`, `sta_mission/src/api/`
  (`monitor.rs`, `condition_monitor.rs`: `Monitor`/`MonitorHook` event hooks
  fed by `SimEvent` and the view; `camera_intent.rs`; `a2/`). Runs as a
  `Controller` on the host (D3) so it needs not be bit-deterministic; commands
  go through `CommandRouter` like players (S2/S7).
- `SimEvent` (S4, `docs/WORKPLAN.md:211-237`, `sta_sim/src/event.rs`) is the
  natural source of `unit.spawned`, `weapon.fired` hook-style events;
  `#[non_exhaustive]`, consumers ignore unknown variants. UI-side events
  (`OrderIssued`, `SelectionChanged`) are Bevy messages in `sta64::sim_link`.
  There is **no `ui.hover`-style event surface** today.
- Tier-4 (WASM/Rust) has no precedent; no `wasm`/`rhai` dependency exists.

### 2.10 Render effect hooks (matches "effect slots", partially)

- Shaders are WESL, `embedded_asset!`-ed at build time
  (`sta64/src/render/gpu_shaders.rs:17,86-89`, `fx/shield.rs:187,208`:
  `embedded://sta64/render/...`). Note: WORKPLAN/MODDING-IDEAS say WGSL; code
  is `.wesl` (Bevy 0.20 + `wesl`). Tooling for pack shaders must accept WESL
  or WGSL consistently.
- Data-driven recipes already exist for look parameters: weapon
  `fx/ordnance.ini` (`[Ordnance.<odf>]` Refit recipes, `fx/recipe.rs`),
  `sta_surface/recipe.rs` + `sta_surface/nebulae/*.ini` (nebula, planet
  recipes; `render/nebula.rs:29-31` `include_str!`). These are **embedded, not
  VFS-overridable** in what I read (the nebula table is `include_str!`;
  I did not trace ordnance loading to the end).
- Effects are separate plugins per kind (`render/fx/{weapons,impacts,shield,
  specials,fire}.rs`, `render/{nebula,planet,rock,hulls,fog,post}.rs`)
  registered in `plugins.rs` (append-only, WORKPLAN hotspot). No registry of
  "slots" exists; each plugin hardcodes its material/shader.
- `render/post.rs` (RND-14) is opt-in per `--postfx` flag, closest to a
  user-selectable effect set.
- Refit source (`/home/cedric/Projects/armada2-refit`) is the origin of most
  shaders; see Refit notice files (`render/hulls.NOTICE.md`, `fx/NOTICE.md`,
  D10).

## 3. What conflicts with the design

1. **No exe-fingerprint detection anywhere.** `grep` for fingerprint/hash of
   executables in `sta_content`/`sta64` found nothing; detection is heuristic
   on `info.ini` names, ZFS presence, folder names, `mmm_loader.dll`
   (`profile.rs:5-29`). The design's "fingerprint by exe hash selects the
   profile" is new work and the heuristics must remain as a fallback (the
   CON-31 bug, `docs/FO_PROFILE_DETECTION.md`, shows how fragile names are).
2. **`Profile` is a closed enum used in `match` by the sim.** `sta_sim`
   branches on it: `kinematics.rs:88` (`Model::of`), `boundary/state.rs:133-135`
   (callback counts per profile), `production/mod.rs:455`; `sta_content` in
   `confinement.rs`, `resources.rs`. This contradicts "the sim only sees the
   canonical DB; game-specific interpretation lives in the profile only" and
   blocks a data-defined profile (a pack cannot add a variant). Replacing
   these with resolved values on `ContentDb` (as `build_queue_limit` and
   `Features` already do) is a **seam change** (S5 + sim callers), additive
   first, removal at a milestone boundary.
3. **`Features` is a fixed struct of bools.** Quirks need IDs, docs, tests and
   non-boolean values (limits, formulas). The struct is `#[non_exhaustive]` and
   hashed via `bits() -> [bool; 22]` (`features.rs:84`): adding a quirk means
   editing a seam file and the hash layout; open-ended pack-defined quirks need
   a keyed, typed map (`BTreeMap<QuirkId, QuirkValue>`, determinism rules:
   no `HashMap`) folded into `hash()`.
4. **Clients bypass the manifest/load-order code.** ~50 call sites call
   `Vfs::mount_install(root, Some("Fleet Ops 4.0"))` or `cfg.active_mod`
   (`sta64/src/lib.rs:34`, `game/flow.rs:365`, `frontend/scene.rs:1459`,
   `audio/mod.rs:85`, `bench/mod.rs:203`, plus tests). `order::resolve`/
   `LoadOrder::mount`/`identity` have **no non-test caller outside `sta_vfs`**
   and the CON-18 claim "mod identity into `ContentDb::hash`" is not wired
   (no `identity()` use in `sta_content`). Also several non-test render
   helpers hardcode `"Fleet Ops 4.0"` as the mod
   (`render/fog.rs:643`, `render/moons.rs:253`, `render/space.rs:399`,
   `ui/sprite_font.rs:767,822`; test/diagnostic paths, but shows the
   single-active-mod assumption). A pack layer needs one `mount(pack
   selection) -> Vfs + Profile + identity` entry point and a pass over these.
5. **`Data` is hardcoded as the base layer** (`lib.rs:166`, `order.rs:99`).
   "A game is a root pack with no parent" requires a base that is not
   necessarily `Data` (A1 flat root already special-cased as `root`; Legacy
   STL `.sol` maps, CON-20, imply a third game family). Roots = `extends
   fleetops` = `extends a2-1.x` conflicts with today's flat
   mod/`ParentMod`->`Data` model, where FO's own content *is* the active mod
   (FO is `Data/Mods/Fleet Ops 4.0`, selected as `active_mod`; Roots is another
   mod, or the base `Data` per CON-31). Manifest `dependencies` can express it
   but the "stock A2 base pack" must be synthesized from the install.
6. **`manifest.toml`'s `profile` key is parsed but unused by detection.**
   `Profile::detect` reads only `info.ini` `[Mod] Profile`; `manifest.rs` has
   `profile: Option<String>` and `profile.rs`'s name table is documented as
   "as written in `info.ini` and `.sta` manifests", yet no code path feeds
   `Manifest::profile` into `detect`. Two sources for one setting must be
   unified (manifest wins, info.ini fallback).
7. **ODF patch fragments have no home.** `sta_odf` resolves whole files and
   FO-style self-includes; there is no `[patch]` key-level fragment, no
   per-key provenance output (`sta64 why ...`), and the `sta_odf` crate's
   lossless tree is only planned (EDT-06). `UnitDef::raw` exposes the
   flattened `Odf`, so provenance would need `resolve_layered` to report
   `(layer, file, line)` per surviving key.
8. **Render slots are not overridable by packs.** Embedded WESL + `include_str!`
   recipes; no shader/param binding by name, no hot reload evidence
   (`bevy` asset hot reload not wired for embedded). Pack shader overlays would
   need a loader from the VFS plus a slot registry in `sta64/src/render/`
   (all of it is a single-owner area per package; see section 4).
9. **Determinism rules constrain the rules layer.** `sta_core`/`sta_content`/
   `sta_sim` forbid floats in state and `HashMap`; a pack's `rules` values
   (limits, formulas) must be parsed to `Fixed`/integers once at load,
   sorted, and hashed. User-written formulas ("formulas" in the Rules layer)
   would need a deterministic evaluator: this is the highest-risk item for
   lockstep and replays (D3, S2 digest, `SEAM_REV`).
10. **Hash/replay compatibility.** CON-31 states a profile change "can change
    content hashes ... do not claim old wrong-profile save/replay
    compatibility". A pack identity folded into `ContentDb::hash` will
    re-record all goldens once; schedule it with other golden re-records
    (SIM-15's note, WORKPLAN "Risks").
11. **Native vs legacy policy.** `ARCHITECTURE.md:75`: "Legacy profiles exist
    to keep their content working. They are not where new native features go."
    A pack/quirk system makes it possible for a legacy pack to switch on
    native features (`Feature_*`, `[Features]` already allow it); the RFC
    should state whether that is intended (it already is per
    `NATIVE_FEATURES.md` header: "ODF/JSON5 keys switch features per unit").

## 4. WORKPLAN packages and seams affected

| Package / seam | Where | Effect of a pack/profile layer |
|---|---|---|
| S5 content DB | `sta_content/src/{db,features,def}.rs` (seam files: seam PR only) | New resolved-rules table; `Profile` match removal; `hash()` additions. Additive first. |
| S6 maps | `sta_content/src/map.rs` | Pack-provided maps (`MapDef`) already served via VFS; `ignoreParentMaps` (`ModInfo`) is a pack rule. |
| S10 plugins/app modes | `sta64/src/{app,plugins}.rs` (R0/INT only, append-only) | A pack selection in `LaunchConfig` (`active_mod: Option<String>` at `app.rs:93-95`) and per-pack settings under `settings.toml` sections; `plugins.rs` one line per new plugin. |
| S2/S4 command/event | `sta_sim/src/{command,event}.rs` | Hook-style events for scripts: new `SimEvent` variants are free; new `Edit` variants additive. |
| CON-05 profiles (merged) | `sta_content/src/profile.rs` | Extend detection (exe fingerprint, manifest `profile`), quirk table. Owner file. |
| CON-13/16/17/18/19 (merged) | `sta_vfs/src/*` | Reused as the pack container, manifest, ordering, signing. Pack manifest extends `Manifest` (`extra` keeps unknown keys). Wiring package missing. |
| CON-01/CON-03 | `sta_odf`, `sta_content/src/odf.rs` | Fragment/`[patch]` semantics, provenance output. |
| CON-04 (JSON5/fixtures) | planned, not on main | Second canonical-DB producer; pack `content` in JSON5 and the "canonical DB without ODFs" test route. |
| CON-06 techtree | `sta_content/src/techtree.rs` | `NotListed` rule -> a named quirk; `.tt`/`.xml` pack overrides. |
| CON-09/CON-08 model/texture | `sta_assets` | No change needed for content overlay (VFS-resolved); a gap is manifest-declared format priority if ever required. |
| CON-11 strings, CON-12 gui cfg | `sta_content/src/{strings,gui_cfg}.rs` | Strings/IDs as data already; pack-level overrides ride on VFS. |
| UI-06 menu scene, UI-07 screens | `sta64/src/frontend/` | Tier-2 triggers extend `MenuMap`; mod select consumes pack list (`mods_in`). |
| RND-* (fx, nebula, planet, hulls) | `sta64/src/render/**` (one owner each) | Slot registry touches every effect plugin; do as a seam-ish PR first. |
| RND-14 post | `render/post.rs` | Closest existing user-selectable effect set. |
| LUA-01..07 | `sta_mission` | Tier-3 scripting reuses runtime; pack `script` layer = `missions/*.lua` already resolved via VFS. |
| EDT-01/06/07/08/09 | `sta_tools`, `sta_odf` | Importer/validator/`sta64 why` live naturally beside the other modder tools. |
| TOOL-01..06 (bench) | `sta_bench`, `sta64/src/bench` | Compat matrix + per-quirk fixtures run as bench scenarios/tests; `STA64_TEST_INSTALL/STA64_TEST_MOD` pattern already used for install-gated tests. |
| AGENTS/WORKPLAN hotspots | `docs/WORKPLAN.md:420-436` | `sta_core`, seam modules of `sta_content`/`sta_sim`, `app.rs`: seam PRs or INT only. New crate needs a seam PR. |
| `docs/NATIVE_FEATURES.md` | Status cells only | Quirk docs must not live here (packages may edit only Status cells); a new `docs/QUIRKS.md` (generated or hand-owned by the quirk package) is needed. |

Freeze note: any package that rewrites `Vfs::mount_install` call sites in
`sta64/` would touch ~15 files across lanes; AGENTS.md "Freezes" says to declare
`**Freeze:** crates/sta64/src/ ...` or, better, keep `mount_install` as a
thin wrapper over the new entry point so no caller changes (recommended).

## 5. Suggested new WORKPLAN packages

Per AGENTS.md "Packages not in the plan": next free number in lane, added by a
small PR to the lane table. CON currently ends at CON-31 on this branch
(`docs/WORKPLAN.md:499`); check `origin/main` for newer IDs before claiming. IDs
below are proposals. Sizes: S/M/L. Model suggestion in parentheses.

Order: P0 RFC -> P1 -> (P2, P3, P4 parallel) -> P5..P9.

| ID | Package | Owns | Needs | Against | Done when | Size |
|---|---|---|---|---|---|---|
| MOD-01 | **RFC and seam decision**: `docs/MODDING.md`; decides pack manifest extension of `manifest.toml`, quirk ID scheme, where the canonical DB schema is documented, compat tiers; seam-PR text for MOD-03 | `docs/MODDING.md`, `docs/MODDING-IDEAS.md` (retire) | this research | — | User sign-off; open questions of MODDING-IDEAS each answered or deferred in a table (Opus) | S |
| MOD-02 | **Pack mount entry point**: `PackSelection -> (Vfs, LoadOrder, Profile)`; wires `order::resolve`/`LoadOrder::mount`/`identity()` behind `Vfs::mount_install` (kept as wrapper); `LaunchConfig.active_mod` maps to a selection; mod identity folded into `ContentDb::hash` | `crates/sta_vfs/src/pack.rs` (new), narrow edits to `lib.rs`/`order.rs`; hash fold = seam PR to `sta_content/src/db.rs` | CON-18 (merged), MOD-01 | synthetic temp dirs, `fake::db()` | Existing 50 call sites unchanged and tests green; a two-mod manifest chain mounts in dependency order; identity changes when a `.sta` content_hash changes; goldens re-recorded once and listed in the PR (Sonnet, seam part Lead) | M |
| MOD-03 | **Profile as data (seam PR)**: additive `ContentDb` rules table (`QuirkId` -> typed value, `BTreeMap`, hashed), `Features` stays; replace `match Profile` in `sta_sim/{kinematics,boundary/state,production}` and `sta_content/{confinement,resources}` with resolved values; keep the 4 `Profile` constants as built-in rule packs | seam files: `sta_content/src/{db,features,profile}.rs`; sim callers listed in the PR | MOD-01 | `fake::db()` with a test quirk | No `match profile` left outside the built-in packs' definitions; digests of existing scenarios unchanged for the 4 profiles; `SEAM_REV` unchanged if purely additive (Opus design, Sonnet execution) | L |
| MOD-04 | **Game fingerprint and base pack synthesis**: exe/`Data` fingerprint (A1, A2, FO3/FO4, Roots) -> profile and base-pack id; manifest `profile` read by `detect` (manifest > `info.ini` > heuristic); keeps CON-31 precedence | `sta_content/src/profile.rs` (owner), `sta_vfs/src/fingerprint.rs` (new) | MOD-02, FO/A2 installs | synthetic exes (hash table), hash-preserved install | Table test per known exe hash (listed in a fixture file, no binaries committed); stock A2, FO4, Roots, A1 select as today; unknown exe falls back to heuristics with a log line (Sonnet) | M |
| MOD-05 | **Quirk registry and docs**: every quirk has ID, doc line, default per profile, and a test fixture; migrate the 22 `Features` and the `NotListed` rule, `resources`, kinematics/boundary profile values into it; generate `docs/QUIRKS.md` | `crates/sta_content/src/quirks.rs`, `docs/QUIRKS.md`, `docs/data/` generator in `tools/` | MOD-03 | `fake::db()` | Lint test: a quirk without doc or fixture fails; `sta64 --list-quirks`; `[Features]` and `Feature_*` still work (Sonnet) | M |
| MOD-06 | **ODF fragments and provenance**: `[patch]`/`inherit` key-level fragment files, per-key `(layer, file, line)` provenance through `resolve_layered`; `sta64 why <odf>` and load-time conflict report | `crates/sta_odf/src/{patch,provenance}.rs`, `sta_content/examples/odf_why.rs` or `sta_tools` bin | CON-01, EDT-06 (lossless tree; or provide a minimal one) | `fixtures/odf/` | Fixture: three layers, each key reports its winning layer; fragment applies without copying the file; FO self-include behaviour unchanged (all 0 `IncludeCycle`/same hash on an install-gated run) (Sonnet) | M |
| MOD-07 | **Compat tier loader (T0/T1/T2)**: manifest inference from folder (T0, exists via `from_info`), `mod.toml` shim (~10 lines: limits, enabled quirks, effect bindings), auto-conversion into a virtual layer in memory | `sta_vfs/src/shim.rs`, `sta_vfs/src/virtual_layer.rs` | MOD-02, MOD-05 | temp dirs | A pure-data mod runs unchanged; a shim raising a limit changes only that quirk value and the content hash; nothing written to disk (Sonnet) | M |
| MOD-08 | **Hook catalogue and importer**: catalogue what FleetOps hooks / `.asi` / exe patches do (from the parallel `fo-hooks-asi.md` research) mapped to quirk, tunable or native feature; importer scans exe patch/`.asi` fingerprints and emits a draft `mod.toml` | `docs/COMPAT_MATRIX.md`, `crates/sta_tools/src/bin/mod_import/` | MOD-05, MOD-07, EDT-01 | recorded fingerprints | Matrix lists top mods by tier; importer emits a draft for FO Roots from its `info.ini`+`[modules]` and flags unknown `.asi` (Sonnet) | L |
| MOD-09 | **Render effect slots**: `slot.<name>` registry (nebula, cloak, shield, trail, planet...), a pack binds a WESL/WGSL shader + params; loader reads from the VFS; fall back to built-in; reload on change in dev; maps current embedded materials onto slots | `sta64/src/render/slots.rs` (new), then one small PR per effect plugin | MOD-02, `render-mods.md` research, RND-05/10/11 landed | built-in shaders as slot defaults | Replacing `slot.nebula` from a test pack changes the headless frame; with no pack frames are byte-identical to today; shader compile errors give file:line and keep the built-in (Opus design, Sonnet execution) | L |
| MOD-10 | **Declarative scene/trigger layer (tier 2)**: `[scene.*]`, `[[on]]`, `[[timeline.*]]` TOML on top of `MenuMap` (UI-06) and a `ui.*`/`sim.*` event bus; engine events exposed as a supported API (`unit.spawned`, `weapon.fired`, `ui.hover`) | `sta64/src/frontend/triggers.rs`, `sta_content/src/triggers.rs` (parse, no Bevy) | UI-06, UI-07, MOD-02 | `ScriptedSource`, `two_bases()` | The main-menu scene of the sketch runs from a pack file; parser errors carry file:line; bench scenario checks marks (Sonnet) | M |
| MOD-11 | **Script escape hatch (tier 3)**: pack `script` layer entries run in the existing `mlua` runtime with a pack-scoped API subset beside MMM; decide Lua vs Rhai in MOD-01 (Lua already vendored) | `sta_mission/src/pack.rs`, `sta_mission/src/api/pack/` | LUA-03, MOD-10 | `fake::db()`, `ScriptedSource` | A pack script reacts to `unit.spawned` and issues a command through `CommandRouter`; sandboxed, errors isolated (Sonnet) | M |
| MOD-12 | **Pack UI and load-order screen**: mod select shows manifest chain, tier, trust status, quirk toggles; explicit order editor | `sta64/src/frontend/screens/packs.rs` | UI-07, MOD-02, CON-19 | `ScreenData::fake()` | Screen test: reorder, toggle quirk, see signature status (Sonnet) | M |
| MOD-13 | **Hot reload and validation**: watch pack folders (ODF, shaders, scripts), re-resolve provenance, file:line errors instead of crashes | `sta64/src/dev/reload.rs` | MOD-06, MOD-09 | temp dirs | Edit an ODF and a shader in a running bench session; frame/marks change; invalid edit reports and keeps previous state (Sonnet) | M |

Dependency sketch: MOD-01 -> MOD-02 -> {MOD-03, MOD-04, MOD-07}; MOD-03 -> MOD-05
-> MOD-07/08; MOD-06 independent after CON-01; MOD-09 after MOD-02;
MOD-10 after UI-06/07; MOD-11 after MOD-10. MOD-03 is the critical seam PR: it
touches sim and content seam files together, so it needs a freeze on
`crates/sta_content/src/{db,features,profile}.rs` and the sim callers listed
in section 3 item 2.

Alternative lane naming: AGENTS.md says "next free number in its lane"; if the
user prefers no new lane, split into CON-3x (MOD-02, 04, 05, 06, 07), RND-2x
(MOD-09), UI-1x (MOD-10, 12), LUA-0x (MOD-11), EDT-1x (MOD-08, 13), with
labels `lane:con` etc. A new `lane:mod` needs `gh label create lane:mod`.

## 6. Answers to the open questions, from the repo

- **"Where does the canonical DB schema live and who owns it?"** S5 in
  `docs/WORKPLAN.md:239-274`; types in `crates/sta_content/src/{def,db,features}.rs`;
  owner is the CON lane with changes through seam PRs only (hotspot table,
  WORKPLAN:420-436). No separate schema document exists; CON-04 promised a
  "schema doc" for JSON5 and has not landed.
- **"Is 'pack' too uniform for render vs content?"** Content is VFS overlay
  (fits). Rules are per-`ContentDb` values (fits, needs MOD-03). Render slots
  are code-registered plugins with embedded shaders (does not fit without
  MOD-09). Script layer fits the `Controller` model (S7). Recommend the pack
  stays uniform at the manifest level, with each layer owned by its own
  loader.
- **"Profiles for big mods too?"** Yes, mechanically: `Overrides` already lets
  a mod's `[Features]` and an explicit `[Mod] Profile` choose a base, and
  CON-31 shows a child inherits its base's identity. What is missing is
  non-boolean values and a pack-defined (non-enum) base.
- **Existing mods keep working?** Already true for ODF/texture/model/map/sound/
  string/menu content (the loaders accept legacy layouts). Not covered:
  anything that patches `Armada2.exe`, `.asi`, `mmm_loader.dll`, `.drl/.dsl`
  (`docs/MISSIONS.md`: never loaded). Roots `[modules] activeN/requiredN` are
  parsed (`mods.rs`) but unused.
- **Provenance** (`sta64 why`): `Vfs::resolve_all` (`lib.rs:261`) already lists
  every layer holding a path; key-level provenance needs MOD-06.
- **Install never written to:** true for reading; the `convert`/`sta_write`
  tools write only where asked. `EDT-01` plans "atomic save with backup" for
  tool edits: ensure the pack layer writes only to a separate pack folder.

## 7. Facts I could not confirm (quick checks for the RFC author)

- Whether `sta_odf::Odf` after `resolve_layered` retains per-key
  (layer, file, line); `Entry::line` exists on the raw tree only.
- Whether `fx/ordnance.ini` and `sta_surface` recipes are overridable by an
  install's files (nebula recipes are `include_str!`; ordnance load path not
  traced). Refit let ini files override; check `render/fx/weapons.rs`.
- Whether `ContentDb::hash` should include `LoadOrder::identity()` or only
  resolved values (cross-peer agreement argues for content-derived hash plus
  identity for the lobby handshake, NET).
- `docs/research/modding/` held no other reports when this was written; the
  `fo-hooks-asi.md` and `render-mods.md` findings are needed to settle
  MOD-08 and MOD-09 scopes.
