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
| `render`  | effect-slot overrides (nebula, cloak, shield, trail) via WGSL |
| `script`  | triggers, timelines, scripts |

- Base game = read-only pack from the user's install, fingerprinted by exe
  hash; the fingerprint selects the profile.
- Mods, FleetOps, Refit, Roots are all packs with `extends` / `requires`.
  Roots = `extends fleetops` = `extends a2-1.x`.

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
own hook). "FO 4.0" and "STA2 Classic" are data submods under Roots
`Data/Mods/`. FO plain and Dominion War are code-identical. Profiles
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

## Open unknowns

- **A1 survey** (exe fingerprint, community patches/hooks, A1-specific
  quirks vs A2: not covered by the first pass); Roots `ODF.fpq` key survey; FO 4.0 effect SODs vs FO plain; Dominion War
  data diff; widescreen patch internals; - No disassembly yet: quirk formulas unverified; stock RTS_CFG defaults not
  found on disk.

## Next

Draft `docs/MODDING.md` from this doc and the three reports; then claim MOD-01
(RFC) per AGENTS.md if wanted.
