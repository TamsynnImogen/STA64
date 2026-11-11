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

## Investigation plan (in progress)

Sonnet subagents, read-only, findings written to `docs/research/modding/`:

1. `fo-hooks-asi.md`: what FleetOps hooks, other `.asi` files and exe patches
   actually change (A2 install, FO install).
2. `render-mods.md`: Refit (`/home/cedric/Projects/armada2-refit`) and FO Roots
   effect/shader approaches, mapped to candidate effect slots.
3. `repo-seams.md`: current STA64 VFS, ODF/format loading, db seam, WORKPLAN
   ownership; where a pack/profile layer would attach.

Next: read the three reports, revise this doc, draft `docs/MODDING.md`.
