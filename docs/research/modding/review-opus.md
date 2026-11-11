# Critical review of MODDING-IDEAS (2026-11-11)

Reviewer: skeptical-architect pass over `docs/MODDING-IDEAS.md` and the three
research reports beside this file. Read-only: installs under
`/home/cedric/Games/Heroic/` were inspected with `find`, `md5sum`, `file`,
`grep`, `strings`; nothing was executed. Code references are to this worktree.

Verdict in one paragraph: the layered-pack idea is sound and most of the
plumbing exists (`sta_vfs` manifests, load order, `.sta` signing). The design
is weakest exactly where STA64 is strictest: **lockstep agreement**. It never
says how two peers, a replay or a save name the pack set, profile and quirk
values they ran on, and the current content hash cannot carry that weight. It
also models "game" as a data root when the installs show two independent axes
(engine lineage and data chain). Shader authoring and the trigger DSL are
specified well ahead of the parts the sim depends on; they should wait.

## 1. Fact checks (claims tested against installs and code)

| # | Claim in MODDING-IDEAS | Result | Evidence |
|---|---|---|---|
| F1 | "Roots = `extends fleetops` = `extends a2-1.x`" | **Wrong.** FO 4.0 and STA2 Classic are `standalone=1` in their `info.ini`; they do not extend Roots' base data, let alone FO 3.2.7's. Only Classic Enhanced has a parent (`ParentMod="STA2 Classic Mod"`, `standalone=0`). | `Roots/Data/Mods/*/info.ini` |
| F2 | Roots has 55 `.tt` and 29 `.xml` tech trees; `.tt` is "pre-FO4" | Counts right, reading wrong. 1 of the 29 xml is `settings_saves_logs/Settings.xml` (28 tech-tree xml). 45 of the 55 `.tt` belong to the STA2 Classic submods (37 + 8), which use `.tt` only *on the Roots engine*. Base Roots `techtree/` has 10 `.tt`, every one paired with an `.xml`. So the dialect is chosen by the **data pack**, not the engine/profile. | `find ... -iname '*.tt'` grouped by folder |
| F3 | Menus: `.bmp`/`.png`, "no alpha, a colour key is used" | **Partly wrong.** 68 PNGs under Roots `bitmaps/` are RGBA (e.g. `bitmaps/SharedButtons/scrollTab.png`, `bitmaps/multiconnect/Internetcon.png`). Whether the engine honours that alpha is unknown, but "no alpha" is not a safe rule. Also the 342 -> 1642 PNG counts are not menu counts: in Roots 1089 are under `Mods/`, 182 are `bzn/` map previews, 9 are ReShade textures. | `file` over `*.png` |
| F4 | SPR `@tmaterial=wormhole` seen in "A1 and A2 stock" | **Wrong.** Only A1's loose SPRs use `wormhole` (2); stock A2 has none. `alpha` is absent from stock A2 too, not only A1 (FO/Roots only). `font` A2+ correct. | `grep @tmaterial` per install |
| F5 | FO 4.0 `ArmadaL.exe` identical to Roots' | Correct (md5 `44bf3a2f...`). **But** FO 4.0 also ships its own `RTS_CFG.h` and `ART_CFG.h` (md5 differ from Roots base), its own `Shaders/*.nvv` (the bump-path vertex programs), `ODF.fpq`, DLLs and a nested `Mods/`. Tunables and even engine shader programs are per data pack. | `md5sum`, `ls Mods/Fleet Ops 4.0` |
| F6 | Every `.asi` except `STA2WidescreenPatch.asi` is Refit's | Correct. | `find -iname '*.asi'` |
| F7 | `Engine_Changelog.txt` is 616 lines | Correct. | `wc -l` |
| F8 | Refit `render3d/mods/hulls/src/hooks.rs:116` `wave_lights`, 4 getters + white wave, formula | Correct, line and formula match. | file |
| F9 | `Profile` closed 4-variant enum, `Features` 22 bools, sim `match`es profile | Correct (`features.rs:7`, `kinematics.rs:88`, `boundary/state.rs:133-145`). Note `Profile::Sta2Classic` means **stock A2**, while the installed "STA2 Classic" is a Roots-engine data mod: a naming collision that will bite the RFC. | code |
| F10 | Render layer is "WGSL" | Wrong for the code: 8 `.wesl`, 0 `.wgsl` under `crates/`. (repo-seams noted it; the ideas doc's layer table still says WGSL.) | `find crates -name '*.wesl'` |
| F11 | "The sim only sees the canonical DB" (as an existing property to build on) | **Not true today.** `sta_sim/src/specials/mod.rs:78-118` reads `u.raw.f32("maxSpecialEnergy")`, `specialEnergyRate`, `effectTimer`, `energyDrainRate` and raw weapon bools straight from the ODF, although typed, hashed fields exist (`db.rs:482`, `u.special_energy_max`). | code |

## 2. Ranked findings

### C1 (critical). Lockstep agreement on "what game are we playing" is unspecified

Evidence: `ContentDb::hash` (`db.rs:389`) is a hand-written FNV-1a `u64` over
"every number the sim reads"; every new field must be remembered and added by
hand (see the `0xe6`/`0xe9` tag comments). F11 shows the sim already reads
values the hash path does not govern (raw ODF keys). The replay (`replay.rs`)
stores only `content_hash: u64` and expects "the loader supplies the
database": it cannot say *which packs* to load. `sta_net/src/lib.rs` is a
6-line stub; NET-01 owns the handshake but MODDING-IDEAS never feeds it.
Open-ended packs, quirk toggles, Lua pack scripts and hot reload all multiply
the ways two peers can differ while believing they agree.

Recommendation:
- Introduce a **session manifest** (lockfile): engine id, ordered pack list
  (id, version, per-layer byte hash, `.sta` `content_hash`), resolved quirk
  table, gameplay scripts' hashes, `SEAM_REV`. Lobby, replay header and save
  header all carry it. `LoadOrder::identity()` is most of it already.
- Two hashes, two jobs: the lock identity (cheap, explains *what* differs, a
  diff the lobby can show) and a **canonical DB digest computed from a derived
  serialisation** of the resolved DB (not hand-picked fields), so a forgotten
  field cannot silently escape. Consider 128-bit or SHA-256; `u64` is fine
  against accidents, not against a peer crafting content.
- Ban `.raw` access inside `sta_sim` (clippy `disallowed_methods` or a grep
  test). Everything the sim reads must come through typed, hashed DB fields;
  that is also the only place a profile quirk can reinterpret a key.
- Hot reload of anything the sim reads is dev-only and ends replay recording.

### C2 (critical). "Game = root pack" hides two independent axes

Evidence: F1, F2, F5. On disk, the Roots install is *one engine* hosting three
data roots, two of which are `standalone`. FO 4.0 carries its own cfg tunables
and engine shader programs. FO 3.2.7 and Dominion War share an engine and
differ only in data. The tech-tree dialect follows the data pack (F2). The
proposed chain "Roots extends fleetops extends a2-1.x" mixes engine ancestry
(Roots' exe is patched stock A2) with data ancestry (FO 4.0 data is not FO
3.2.7 data plus a delta).

Recommendation: make the manifest carry two orthogonal relations.
- `engine = "roots5"` (a requirement, one per session): selects the closed set
  of engine behaviours (quirk registry defaults), format dialect defaults and
  which cfg keys are honoured (e.g. `cfgTy2Lit_*` only on roots5).
- `extends` / `standalone`: the data overlay chain, as `info.ini` already
  encodes. A standalone pack replaces the data root but keeps the engine.
- Fingerprinting selects the *engine*, never the mod (same exe md5 across all
  three Roots submods). Data packs supply tunable values (their `RTS_CFG.h`).
- Rename the `Profile` variants by lineage (`A1`, `A2Stock`, `Fo3`, `Roots5`,
  `Native`) before the enum turns into data, so "STA2 Classic" stops meaning
  two things. Note FO 3.2.7 has no variant today; `Fo4` stands for Roots.

### H1. Quirk interaction and conflict resolution are undefined

Evidence: the doc says a profile "is just a rules-layer pack", implying any
stacked pack may set quirks with last-wins. Real quirks interact: `hitChanceMod`
is overloaded as an event id on `temporal` ordnance; pay-on-queue vs
corbomite infinite build are both "queue policy"; `avoidanceclass` sentinels
(99, 123-125, 151) are one key with five meanings. repo-seams already counts
four precedence levels (engine defaults < profile < mod `[Features]` < per-ODF
`Feature_*`), and the ideas doc adds pack quirk layers without placing them.

Recommendation:
- The quirk **registry is closed** (it is engine code; a pack cannot invent a
  sim behaviour). Packs set *values*. State this; repo-seams' "open-ended
  pack-defined quirks" contradicts it.
- One written precedence chain: engine defaults < data root < `extends` chain
  (top wins) < per-ODF key. Non-root packs may only *require* a quirk value
  (`requires.quirks = { "sim.build_queue_policy" = "pay_on_queue" }`); two
  packs requiring different values is a load error with both names, never
  silent last-wins.
- Model related flags as one enum-valued quirk (`queue_policy`,
  `cloak_rules`), not N bools. Each quirk declares phase (parse, DB, sim,
  render, UI) and scope (match-must-match vs client-local).
- No per-match player toggles of sim quirks in the UI; offer presets. Each
  free toggle doubles the test and MP matrix.

### H2. Quirk fidelity has no oracle

Evidence: fo-hooks-asi §7: no disassembly, formulas unverified, stock
`RTS_CFG` defaults "not found on disk". Section 5.4 lists ~20 bug-emulation
quirks whose exact behaviour nobody has measured. "Every quirk needs a test
fixture" gives a fixture of *our guess*, not of the original game.

Recommendation: a quirk ships only with a reference measurement from the
original engine (Refit testbench scenario, log or frame), or is marked
`provisional` in `QUIRKS.md`. Prioritise by usage in installed data (key
histogram over `ODF.fpq`, which fo-hooks-asi §7.8 already proposes). Bug
emulation for stock replays has little value without original replay import;
defer section 5.4 wholesale.

### H3. Compat tiers overstate T0/T1

Evidence: fo-hooks-asi §6 puts FO 3.2.7, Dominion War, Roots, FO 4.0 and both
Classic variants at T3. Of the installed corpus only stock-lineage data mods
are T0. T1's "auto-convert into a virtual layer" has one concrete instance
(FPQ unpack, which the VFS already reads natively). The `.asi` importer
targets a population of one third-party file (the widescreen patch, which is
"not needed").

Recommendation: replace labels with a **measured coverage report** produced
by loading the mod: unknown keys, keys with unimplemented quirk semantics,
missing assets, scripts calling absent API, plus a bench smoke scenario. The
compat matrix is that report over the corpus, regenerated in CI. Keep "T2
shim" as nothing more than a hand-written `manifest.toml`. Cut the importer.

### H4. Format-dialect resolution conflicts with existing asset lookup

Evidence: `sta_assets` ranks formats globally per layer (KTX2 > DDS > TGA >
PNG); the VFS orders loose > `.sta` > FPQ > ZFS within a layer; the doc adds a
per-profile and per-pack extension order but never says whether layer order
or extension order wins (does a `.tga` in a mod beat a `.dds` in its parent?).
`.tt` vs `.xml` precedence on Roots is explicitly unknown. F2/F3 show the
dialect is a data-pack property and the colour-key rule is not universal.

Recommendation: one rule, written down: layer order first, extension order
only within a layer (that is what modders expect: "my file wins"). Dialect
defaults come from the engine, a data pack may override for its own layer
only. Settle `.tt`/`.xml` and PNG alpha on the Roots engine with the testbench
before coding. Report same-name, different-extension pairs as a provenance
note, since `sta64 why` must explain them.

### H5. Scripting tiers ignore where scripts run (assumption D3)

Evidence: WORKPLAN D3: AI and Lua run on the host as `Controller`s; only their
commands enter the lockstep log. The tier-2 sketch mixes presentation
(`ui.hover`, camera dolly, `nebula.pulse`) with sim events (`unit.spawned`)
in one file and one bus. Tier 3 is "Lua or Rhai"; Lua 5.3 via `mlua` is
already vendored and MMM-compatible, and 252 installed missions use it. Tier 4
(WASM) has no use case.

Recommendation:
- Two script kinds with different rules. *Presentation* scripts: run on every
  client, may be non-deterministic, may not issue commands, not part of the
  lock. *Gameplay* scripts: host-only `Controller`, act only through
  `CommandRouter`, hashed into the session manifest.
- Pick Lua, close the Rhai question. Drop tier 4 from the plan.
- Keep tier 2 tiny (event -> list of presentation actions) or define it as
  sugar that compiles to the Lua API, so there is one event vocabulary. A
  bespoke TOML DSL with timelines will grow conditionals and become a third
  language.
- Audit the `mlua` sandbox before any third-party pack script runs (I found
  no explicit stdlib restriction in `sta_mission/src/runtime*`; verify
  `io`/`os`/`require`/`load` exposure, memory and instruction limits).

### H6. Shader ladder: right goal, wrong time, one blind spot

Evidence: render-mods §0.8: a shader pack is valid only against one input
contract version. The doc proposes freezing contracts "derived from Refit's
recipe formats", whose de-facto ABI is D3D9 register numbers documented in
comments (render-mods §1.1). STA64's own render plugins are still landing
(RND lane, embedded WESL, no slot registry), so any contract frozen now
freezes an unfinished renderer. "Old versions get a shim" means one shim per
slot per version, forever.

Blind spot: "render output never feeds the sim, so no desync risk" is true,
but a shader is a **fairness** risk in MP. Every lockstep client holds full
state; if slot inputs include unfiltered entities (cloak needs a scene copy
and ship data), a pack shader can draw cloaked enemies or through fog.

Recommendation:
- Ship rungs 1-2 first: make the existing `include_str!` recipes (nebula,
  ordnance, surfaces) VFS-overridable. That is most of Refit's player-facing
  modding and costs little.
- One experimental shader slot (nebula: self-contained, no gameplay
  information) behind an `unstable` contract version; freeze nothing until a
  second, independent effect has been written against it. No shim promise.
- Slot inputs come only from the local player's perception-filtered view.
  The lobby lock lists render packs; a host may require "stock render".
- Budgets: naga validation at load, compile off the main thread, a per-slot
  GPU-time warning in the bench, fallback on device loss. WGSL/WESL is memory
  safe under wgpu; hangs and pathological compile times are the real risks.

### M1. Refit as the reference pack couples two release cycles

"One source of truth for recipe formats shared by the Refit `.asi` and STA64"
means every format change needs a coordinated release of a Windows hook DLL
and a Rust engine, and invites STA64 contracts shaped by the old engine's
frame structure. Recommendation: STA64 owns its formats; Refit reads them or
gets a converter. Port Refit content as the first *consumer* of STA64 formats,
not as their definition.

### M2. Missing: saves, replays, distribution, trust, localisation, pack testing

- **Saves:** `snapshot` exists; a save must carry the session manifest, and
  loading with a different pack set must be refused or explicitly migrated.
- **Replays:** see C1; the header names packs, not just a hash.
- **Distribution / manager:** dependency fetch, version solving and
  "which version did this replay use" are unaddressed. `.sta` + `order.rs`
  give the local half.
- **Trust:** `.sta` Ed25519 signing exists, but there is no key policy (who
  signs, how keys are pinned, what an unsigned pack may do). Signatures attest
  origin, not safety: gameplay scripts and shaders need the sandbox rules
  above regardless of signature.
- **Localisation:** `Dynamic_Localized_Strings.h` is per data root; packs need
  string-table overlay and language fallback rules; nothing in the doc.
- **Pack testing:** a `sta64 check-pack` (validate, provenance, unknown-key
  report, headless smoke scenario, digest) usable in a pack author's CI. This
  is the tool that turns the compat matrix and the "file:line errors" goal into
  something real.
- **Docs:** an ODF key reference generated from the canonical DB schema and
  quirk registry (CON-04's promised schema doc never landed).

### M3. Seam leaks beyond F11

`match Profile` in sim (F9), `UnitDef::raw` exposure, ODF identity by name
across subfolders without a defined tie-break for duplicate names within one
layer, and the hardcoded `"Fleet Ops 4.0"` strings (repo-seams §3.4). Each is
small; together they decide whether "profile interpretation lives only in the
profile" is true. Treat removal as acceptance criteria of MOD-03, not
follow-ups.

### M4. Hot reload and determinism

Hot reload of ODFs changes content mid-session. Allowed in a dev single-player
session only; it invalidates the replay recorder and is refused while a lock
is shared. Shader and presentation-script reload is always safe.

### L1. Smaller points

- The exe may not be present (data-only copies, Android target): engine
  detection needs data fingerprints (`AssetVersion=50000`, `Armada2.ldl`,
  `.test` section) as first-class, not as a fallback.
- A1 is barely surveyed; fine to defer, but the engine-axis model must not
  assume the A2 `Data/` layout (already special-cased as `root`).
- Pack-layer provenance for non-ODF content (textures, cfg) can reuse
  `Vfs::resolve_all`; key-level provenance for `.h` cfg files is needed too,
  since those hold the tunables.
- `sta64 new-effect`, auto-generated sliders, contract shims: good later,
  not MVP.

## 3. MOD-01..13 sanity check

| ID | Verdict | Note |
|---|---|---|
| MOD-01 RFC | Keep, widen | Must decide: engine/data axes (C2), session manifest (C1), quirk precedence and conflict rule (H1), script kinds (H5), tier redefinition (H3). |
| MOD-02 pack mount | Keep, first | Also emits the session manifest. Keep `mount_install` as a wrapper (no freeze). |
| MOD-03 profile as data | Keep, critical path | Closed registry, typed values. Acceptance adds: no `.raw` in `sta_sim`, no `match Profile` in sim, profile variants renamed by lineage. |
| MOD-04 fingerprint | Shrink to S | Detects the engine only; data fingerprints first-class. |
| MOD-05 quirk registry | Merge into MOD-03 or run right after under the same freeze | Same seam files. Add `provisional` status and reference-evidence column. |
| MOD-06 ODF fragments + provenance | Keep | Highest modder value per cost; independent. Fold in `.h` cfg provenance. |
| MOD-07 tier loader | Cut most | T0 exists (`from_info`); "virtual layer auto-conversion" has no defined input. Replace with the coverage report. |
| MOD-08 hook importer | Cut importer | Keep the catalogue as the generated coverage matrix (part of `check-pack`). |
| MOD-09 render slots | Split | 09a: VFS-overridable recipes (cheap, now). 09b: one `unstable` shader slot, later. |
| MOD-10 triggers | Defer | `Menu/map.ini` override already works. Redesign after H5 decision. |
| MOD-11 pack Lua | Keep, after sandbox audit | Gameplay scripts host-only and locked. |
| MOD-12 pack UI | Keep minimal | Order, enable, trust, lock diff. No per-quirk toggles. |
| MOD-13 hot reload | Keep, dev-only | Shaders/recipes first; ODF reload outside locked sessions. |
| (new) Session manifest + lobby/replay/save header | Add, with NET-01 | The C1 fix; small but cross-cutting (replay format bump). |
| (new) `check-pack` + coverage report | Add | Validation, unknown keys, smoke scenario, digest. |
| (new) Strings overlay / localisation rules | Add, S | Per-pack string tables with fallback. |

## 4. Revised MVP

Goal: the three Roots data roots (FO 4.0, STA2 Classic, Classic Enhanced) and
the FO 3.2.7 install load as packs on the right engine, two peers prove they
run the same thing, and a modder can see why a value won. Nothing else.

1. **MOD-01 RFC** with the five decisions above (engine axis, session
   manifest, quirk precedence/conflicts, script kinds, coverage-not-tiers).
2. **Session manifest** (new, with NET-01): lock built by MOD-02, carried in
   replay and save headers (format bump), compared in the lobby with a
   readable diff. Canonical DB digest from derived serialisation.
3. **MOD-02** pack mount through manifests and load order, `extends` and
   `standalone` honoured, `engine` as a separate requirement.
4. **MOD-03 + MOD-05**: engine rules table with a closed registry, profiles
   renamed by lineage, the 22 `Features` and profile `match`es migrated, sim
   raw-ODF reads removed. Quirks marked `provisional` until measured.
5. **MOD-06** key-level provenance (`sta64 why`) for ODF and cfg, plus
   `check-pack` producing the coverage report for the installed corpus.
6. **MOD-09a**: existing render recipes overridable through the VFS.

Done when: `sta64 why` explains a Classic Enhanced ODF key across its parent;
the coverage report lists every installed mod; two in-process peers with
different `RTS_CFG.h` refuse to start and name the file; a replay records
and reloads its pack set without user input.

Deferred (revisit after M-milestone with NET): shader slots beyond one
experiment, tier-2 trigger DSL, pack Lua for third parties, `.asi` importer,
bug-emulation quirks, WASM, quirk toggles in UI, contract shims, Refit format
unification.
