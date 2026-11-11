# Modding abstraction: ideas (pre-RFC, 2026-11-11)

Status: brainstorm, not a decision. Written to be resumed later. The
investigation findings go in `docs/research/modding/` (see bottom); the RFC
(`docs/MODDING.md`) is drafted from both.

## Goal

Find the right abstraction for "game" and "mod". The landscape is
heterogeneous: modified `Armada2.exe`, FleetOps hooks, other `.asi` runtime
hooks, replacement textures/models/ODFs/maps, Refit (shaders for nebulae,
cloaking, etc. plus more), FO Roots (effects changed in a way not yet known).
Existing mods should keep working with minimal adaptation (run as-is, or
recreate with a tiny ini/toml). Plus: native STA64 modding that feels familiar
to old modders, new things that are trivially scriptable (e.g. menu-scene
camera/effect triggers), and easy replacements for what was painful.

## Core model

**A game is a root pack with no parent; a mod is a pack on top of one.**

A *pack* = manifest + optional layers:

| Layer     | Purpose |
|-----------|---------|
| `content` | VFS overlay: textures, models, ODFs, maps, sounds |
| `rules`   | quirk/tunable overrides: limits, formulas, ODF key semantics |
| `render`  | effect-slot overrides (nebula, cloak, shield, trail); shaders are WESL (`.wesl`) |
| `script`  | triggers, timelines, scripts |

- Base game = read-only pack from the user's install, fingerprinted by exe
  hash; the fingerprint selects the profile.
- Mods, FleetOps, Refit, Roots are all packs with `extends` / `requires`.
  Roots is an *engine* (separate lineage, see below), not a pack that
  extends FleetOps; FO 4.0 / STA2 Classic are standalone packs requiring the
  Roots engine (review-opus C2: needs a separate `engine =` axis).

**Profile** = base + set of named, documented, toggleable *quirks*
(e.g. `a2.shield_regen_ignores_damage_delay`), not a per-exe monolith. A
profile is just a rules-layer pack, so exe versions and elaborate mods use the
same mechanism (answers "profiles for big mods too?": yes, same abstraction).

**Pipeline:** bytes -> codec -> lossless typed tree (unknown keys preserved) ->
profile interpretation -> canonical engine-neutral DB (the `fake::db()` seam).
The sim only sees the canonical DB. Game-specific interpretation lives in the
profile only.

## Compat tiers

Code-patching mods (rewritten exe, `.asi`, FO hooks) cannot run; no x86
emulation. Catalogue what they do, reimplement the effect as a quirk, tunable
or native feature.

- **T0** runs as-is (pure data mods; manifest inferred from folder).
- **T1** auto-converted at load into a virtual layer (nothing written to disk).
- **T2** tiny shim `mod.toml` (~10 lines) declaring intent ("raises ship cap
  to 500, enables hook X").
- **T3** behaviour reimplemented as native pack or profile quirk.

Publish a compat matrix of top mods by tier to drive priorities. An importer
that scans known exe patches / `.asi` fingerprints and emits a draft `mod.toml`
could cover much of T2.

## Old pain points -> new answers

- Whole-file ODF copies clobber each other -> key-level `[patch]` fragments,
  plus `inherit` / `include`.
- Provenance: `sta64 why ships/foo.odf` shows which layer won each key;
  conflicts reported at load.
- Validation with file:line errors instead of crashes.
- Hot reload of ODFs, shaders, scripts.
- No exe patching ever: limits become tunables.
- Reversible installs: install never written to; uninstall = disable layer.
- Strings and IDs as data, not DLL resources.

## Native scripting, tiered

1. Data: ini/toml (familiar).
2. Declarative triggers and timelines (the easy new thing).
3. Small embedded script (Lua or Rhai) as escape hatch.
4. WASM/Rust plugins, rare.

Menu-scene sketch (tier 2):

```toml
[scene.main_menu]
[[on]]
event = "ui.hover:button.campaign"
do = [{ camera = "dolly", to = "bridge_cam", over = "1.2s", ease = "inout" },
      { effect = "nebula.pulse", intensity = 0.6 }]

[[timeline.idle]]
at = "0s";  camera = "orbit", target = "ent.station"
at = "20s"; fx = "warp_flyby"
```

Hook-style mods wanted engine events; expose the same (`unit.spawned`,
`weapon.fired`, `ui.hover`) as a supported API.

## Rendering mods

Named effect slots; a pack binds a shader + params to a slot. Refit's
volumetric nebulae = a pack overriding `slot.nebula`. How Roots does effects is
unknown: first thing to find out.

## Risks / open questions

- Quirk sprawl: every quirk needs an ID, a doc line and a test fixture.
- Game priority: A2 and FO likely before A1.
- Load order: explicit beats alphabetical; needs UI.
- Is "pack" too uniform for render vs content? Check against Refit/Roots.
- Where does canonical DB schema live and who owns it (WORKPLAN seam)?

## Investigation findings (2026-11-11)

Full reports: `docs/research/modding/{fo-hooks-asi,render-mods,repo-seams}.md`.
Summaries are the agents' own; formulas and ODF.fpq contents are unverified.

**Code lineages (fo-hooks-asi).** Four engine lineages: **stock A1** (own
exe; the survey found only IPX/ddraw shims there and did not look for A1
community patches or hooks, so A1 is the least-surveyed lineage), and the
A2 family: stock A2, FO 3.2.7 (in-memory `FleetOpsHook.dll`, stock exe renamed
`Armada2.ldl` behind a loader), Roots 5.0Z11 (patched `ArmadaL.exe` plus its
own hook). "FO 4.0" (v4.0Z, ported to the Roots engine; same `ArmadaL.exe` md5 as
Roots) and "STA2 Classic" are standalone data/rules mods under Roots
`Data/Mods/`. The original pre-Roots FO 4.0 release, if it had its own hook
build, is not installed here and is unsurveyed. FO plain and Dominion War are code-identical. Profiles
therefore key on code lineage plus a data-mod layer, matching the pack model.
Fingerprints: `.test` exe section,
`AssetVersion=50000`, `Armada2.ldl`. Every `.asi` except
`STA2WidescreenPatch.asi` is Refit's own. Roots' `devstuff/Engine_Changelog.txt`
(616 lines) is the best quirk source: sentinel ODF keys (`hitChanceMod`,
`lodShift`, `avoidanceclass`, `percentNebulaWarn`), MP factions 8 -> 16,
autonomy rework, new RTS_CFG/ART_CFG keys. All engine-rule detours, so
reimplementable as named quirks/tunables (`limit.*`, `cfg.*`, `odf.*`, `sim.*`,
`a2.*` ID families).

**Render (render-mods).** Refit is not a shader override: it is an `.asi`
replacing render steps with its own renderer, driven by ini "recipes".
Roots' visual changes are data (SODs, DDS, ODF keys, cfg numbers) plus a
ReShade post chain; only a bump-mesh path and cloak alpha are engine code.
Amendments to the slot idea:
- A slot's real contract is its *inputs* (depth, scene copy, lights, shadow
  map, hit data, sim clock): needs a versioned input contract.
- Slots need a binding key (ODF, texture, race, skybox) and a fallback to the
  default draw; post is an ordered chain.
- Pack contributions are three kinds: parameter set, asset selection, shader
  implementation. **Shader authoring is a first-class tier (owner decision,
  2026-11-11), not deferred**: not all modders will or can write shaders, so
  rungs 1-2 never require it, but rung 3 gets maximum help (see below).
  Recipe numbers are fitted per art set, so the profile supplies default params.
- Tiers: Roots data T0, Refit recipes/ini T1, ReShade preset T2, Refit
  shaders and Roots engine-code behaviour T3.

**Repo (repo-seams).**
- `sta_vfs` already has `manifest.toml` (deps, `[load]`), `order.rs` load
  order/`identity()`, `.sta` archives, `Vfs::resolve_all`. Extend it; no
  second manifest. But ~50 call sites use `Vfs::mount_install`, and mod
  identity is not wired into `ContentDb::hash`.
- Profiles are close to the design (defaults < mod `[Features]` < per-ODF
  `Feature_*`), but `Profile` is a closed 4-variant enum, `Features` a struct
  of 22 bools, and sim/content `match profile` in several places. Making
  profiles data (keyed typed quirk table) is a seam PR (proposed MOD-03).
  Quirk values must obey determinism rules (`Fixed`, no `HashMap`, hashed).
- No exe fingerprint detection yet; `Manifest::profile` is parsed but unused.
- Ready pieces: `NotListed` tech rule as first named quirk; ODF FO
  self-includes as key-level inheritance; data-driven menu scene
  (`assets/menu/map.ini`); Lua/MMM runtime (`sta_mission`) as tier-3 script;
  `SimEvent` as hook source (no `ui.hover` yet). Shaders are embedded `.wesl`,
  no slot registry.
- Proposed packages MOD-01..MOD-13 (RFC, pack mount, profile-as-data,
  exe fingerprint, quirk registry, ODF fragments + provenance, tier loaders,
  hook importer, render slots, triggers, script layer, pack UI, hot reload).
  Check `origin/main` for free IDs before claiming.

## Shader authoring ladder

Render output never feeds the deterministic sim, so shader mods carry no
desync risk; that is why this can be opened early.
1. Parameters (ini recipe numbers). 2. Asset selection (textures, models).
3. Shader: replace the slot's effect code.
Making rung 3 easy:
- Modder writes only the effect body; the slot defines signature + versioned
  input struct (depth, scene copy, lights, hit data, sim clock); engine owns
  bindings, passes, fallback. Shared WESL helpers (`import sta::lighting`).
- `sta64 new-effect <slot>` ejects the built-in default shader into the pack;
  Refit's ported effects ship as annotated example packs.
- Shader uniform declarations (range, default) auto-become recipe keys and
  in-game sliders, linking rung 3 back to rung 1.
- Validate at load; compile error -> fall back to default draw + error with
  file:line. Hot reload. Slot-isolation preview scene via the testbench harness.
- Pack names the contract version it targets; old versions get a shim.

## Refit is first-party (author = project owner)

The user develops Armada 2 Refit and can extract, change and repackage parts
of it for STA64. So Refit is not a foreign mod to be converted (the T1/T3
tiers above apply to third-party mods only). Implications:
- Refit is the **reference pack**: repackage its recipes (sky, surfaces,
  nebulae, ordnance, menu scene), content sets and QOL rules to the STA64 pack
  manifest; derive and freeze slot input contracts from its recipe formats.
- One source of truth for recipe/param formats and ported shaders shared by
  the Refit `.asi` and STA64; consider splitting Refit into data/recipes vs
  the Windows renderer.
- Open: how far to change Refit formats for STA64 vs keeping them stable for
  existing Refit users (owner's decision).

## Format dialects per profile (from Tammy, verified against installs)

Formats differ by lineage, not only interpretation, so a profile also owns a
*format resolution table* (the codec axis is per-profile, not global):
- Tech trees: `.tt` pre-FO4; `.xml` for FO4/Roots engine, which also accepts
  `.tt`. Roots install has 55 `.tt` and 29 `.xml`, often same-named pairs
  (`devtech.tt` + `devtech.xml`); FO plain has 3 `.tt`, 0 xml. Changelog
  mentions "caplists in xml techtrees". Need a precedence rule when both exist
  (open: which wins in the Roots engine?).
- Textures: `.tga` pre-FO4; `.dds` for most of FO4 (Roots: 1064 dds vs 4205
  tga; bump maps accept both per changelog; FO plain: 0 dds).
- Menus: `.bmp` or `.png`; mostly no alpha, a *colour mask/key* is used for
  transparency (png count 342 in FO plain -> 1642 in Roots).
Design: per-profile ordered extension search (`texture: [dds, tga]`,
`techtree: [xml, tt]`, `menu_image: [png, bmp]` + colour-key rule), with
packs able to override the order. Codecs stay game-independent; the profile
only says which to try and how to interpret alpha/colour-key. Tech-tree xml
and tt should both parse to the same canonical tree (xml adds features such
as caplist tooltips, so the canonical form needs those fields optional).

## File-type glossary (from Tammy; semantics unverified unless noted)

| Ext / file | Meaning |
|---|---|
| `.odf` | text object definition: weapons, build lists, ship stats, most gameplay data |
| `.sod` | model |
| `.spr` | sprite definition: tables of sprites over an atlas |
| `.cfg` / `.h` | mostly UI; `dynamic_localized_strings.h` holds most strings; `RTS_CFG.h`/`ART_CFG.h` are tunables (see fo-hooks-asi.md) |
| `.aip` | AI files |
| `.tt` / `.xml` | tech trees (see dialects above) |

**ODF folder layout.** `odf/` has subfolders `commands`, `other`, `ships`,
`special_weapons`, `stations`, `system`, `weapons` (with nested `Phasers`,
`Photons`, `Pulse` in the Classic Enhanced mod). Consequence for packs: a
pack's ODF may sit in a different subfolder than the base file it overrides,
so ODF identity should be the *name*, resolved recursively across subfolders,
not the path. Check how the engine resolves (unverified); matters for
key-level `[patch]` fragments and `sta64 why`.

**SPR tables.** Each table starts with `@` headings. `@reference=64` = atlas
size (64x64 px); `@tmaterial=` = blend/material mode for the atlas. Values
observed by grep across installs (meaning of each not yet checked):
`interface`, `additive`, `default`, `alphathreshold`, `add_nomipf`,
`wormhole` (A1 only; review-opus found it absent in A2 stock sprites); `font` (A2+); `alpha` (FO/Roots, absent in A1).
Other directives seen: `@referencewidth`/`@referenceheight` (non-square
atlases, up to 2048 in FO/Roots), `@auto=square|column|row|scalar`,
`@skip=off|(r,g,b)`. `@tmaterial` is a prime *profile-keyed enum*: the
canonical form is a named blend mode, each profile maps its accepted
spellings, and FO/Roots-only values must be rejected or mapped on A1.
Next: have an agent pin down each `@tmaterial`'s semantics and the
`@auto`/`@skip` rules from the installs and the existing `sta_content` spr
parser.

## Worked example: rangedobjectaffector wave lights (from JanB, Roots author)

`rangedobjectaffector` weapons (type 1 weaponenhancer, type 2 sensor
scrambler = "ECM" in Roots, type 3 cloak enhancer, per JanB) tint a craft's
hull with a moving **wave light**. One feature, four dependencies, which is a
good stress test for the pack/profile/slot model:
- **Rendering path depends on profile x model.** Stock A2 and stock FO use
  vertex lighting (the "wavy pattern"; only some Borg parts have bump maps in
  A2). FO/Roots ships bump maps on all ships and the Roots engine lights
  bump-mapped hulls as a reflection tint instead (screenshots from JanB show
  both, incl. cloaked Romulans falling back to the wavy look because cloaked
  craft are not drawn through the bump path). So the slot variant is chosen
  by (profile feature, "model has bump map", cloak state), not by profile
  alone.
- **Roots-engine-only tunables** (ART_CFG.h; not in stock A2/FO):
  `cfgTy2Lit_R/G/B` (type 2 colour, default 0.0/0.5/1.0),
  `USE_ROACOMBI_LIGHT` (0 = engine mixes, 1 = force custom),
  `cfgCombiLit_R/G/B` (type 1+2 combined, default 1.0/0.25/0.0). Originally
  types 2/3 ignored colour and any combination went white. These are
  `cfg.*` tunables owned by a Roots-lineage profile; stock profiles must not
  honour them.
- **ODF semantics:** type 1 obeys the cycle-colour ODF keys (guide:
  guide.fleetops.net/guide/modding/fo-odf-commands/weapon/rangedobjectaffector);
  types 2/3 do not. Also (changelog) negative `rangefactor` on type 1 affects
  hit chance (`odf.*` quirk, sim-side).
- **Refit already models it** (`render3d/mods/hulls/src/hooks.rs:116`,
  `wave_lights`): reads five engine WaveLight objects per draw (weapon
  enhancer, sensor scrambler, speed enhancer, psionic getters, plus a white
  wave on crafts while a craft field is > 0), each with colour, wavelength,
  speed, direction; legacy lighting is
  `colour x 0.5 sin(2 pi (p.dir/wavelength + t x speed))` per vertex.
  Naming mismatch to resolve: Refit calls the third getter "speed enhancer",
  JanB calls type 3 "cloak enhancer".
Slot sketch: `hull.affector_light` with inputs {active affector set, per-type
colour/wavelength/speed/dir, cycle colours, model has bump, cloaked} and
params from the profile (ART_CFG keys mapped to pack tunables). The sim
exposes only the affector state; colour mixing and wave maths stay render-side
(no determinism risk).

## Opus review (docs/research/modding/review-opus.md)

Corrections above applied. Open decisions from the review, not yet adopted:
session manifest/lock for peers+replays+saves (C1); `engine =` axis (C2);
quirk registry with conflict rule; replace tiers with a coverage report and
cut the `.asi` importer; split scripts into presentation vs host-only gameplay
(Lua, no WASM); trim MOD-01..13. Conflict with owner decision: review says
defer shaders; owner decided shader authoring is first-class (see ladder),
so reconcile as "contracts marked `unstable`, ship recipes-via-VFS first".

## Open unknowns

- **A1 survey** (exe fingerprint, community patches/hooks, A1-specific
  quirks vs A2: not covered by the first pass); Roots `ODF.fpq` key survey; FO 4.0 effect SODs vs FO plain; Dominion War
  data diff; widescreen patch internals; - No disassembly yet: quirk formulas unverified; stock RTS_CFG defaults not
  found on disk.

## Next

Draft `docs/MODDING.md` from this doc and the three reports; then claim MOD-01
(RFC) per AGENTS.md if wanted.
