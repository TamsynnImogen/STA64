# Modding v1: decisions for sign-off

Status: **accepted** (Tammy and Cedric, 2026-11-11; merged in #352; D5 amended
2026-11-12 and approved by Tammy, see D5). Binding on
all agent and human work in the areas below. Covers how STA64 treats existing Armada
mods and what the first modding release contains. Background and evidence are
in [MODDING-IDEAS.md](MODDING-IDEAS.md) and `docs/research/modding/`; this
document records decisions only.

## Scope of "mod"

**D1. Existing mods keep working, measured, not promised.**
STA64 reads existing mod folders (`info.ini`, `ParentMod`, `standalone`,
`ignoreParentMaps`, ODFs, SPR, tech trees, cfg files, textures, models, maps).
How well a mod works is reported by a coverage tool (D7), not by a compatibility
tier label.

**D2. Code-patch mods are not loaded.**
Modified executables, FleetOps hooks and `.asi` files are never run. Their
behaviour is reimplemented as native rules, driven by what the coverage report
shows mods actually need. No hook or `.asi` importer in v1.

**D3. An adapted mod must still run on its original engine.**
Anything STA64-specific that a mod adds (extra `info.ini` keys, files) must be
ignorable by Armada II, Fleet Operations and Roots. `info.ini` stays the "tiny
ini" for adapting a mod.

## Games and profiles

**D4. Axes.**
- `profile` = the engine lineage, a closed list.
- `engine` keeps its existing meaning: the STA64 version a mod requires.
- A mod's data chain comes from `kind` and `ParentMod`.

**D5. v1 lineages: stock Armada II, stock Armada I and Roots.**
FO 4.0, STA2 Classic and STA2 Classic Enhanced run on the Roots engine and are
covered through it. Stock Armada I runs as its own lineage (the `Sta1Classic`
profile); mods of Armada I are not covered in v1. FO 3.2.7 and Dominion War are not covered in v1.

*Amended 2026-11-12: Armada I moved into v1 (Cedric; approved by
Tammy). It read "stock Armada II and Roots only" and listed Armada I with
FO 3.2.7 and Dominion War as not covered. Already on main for it: the
`Sta1Classic` profile (`sta_content`), the ZFS archive mount (`sta_vfs`, CON-15
and CON-27), A1 BZN headers, binary bodies and MDF start locations (`sta_bzn`,
CON-25, CON-26 and CON-28) and the A1 queue-capacity rule (SIM-20). What is
still missing is CON-38..42 in WORKPLAN.*

**D6. Mounting must follow the mod's own rules.**
`standalone` and `ignoreParentMaps` are honoured. Today FO 4.0 mounts over base
Roots data and base ODFs, tech trees and maps leak in; this is fixed first.

## Verification

**D7. Coverage report.**
A `check-pack` tool reports, per mod: how many of its keys STA64 understands
(weighted by use), cfg keys it changes that STA64 ignores, unresolved map
references, and which layer and line supplied a given file or key.

**D8. Acceptance target.**
STA2 Classic Enhanced (a parent mod with changed `RTS_CFG.h`, tech trees, ODFs
and maps, no binaries) is the compatibility reference, and its author is asked
to try the result. Refit is the reference for rendering only.

## Rules and multiplayer

**D9. Rules are typed and come from the legacy files.**
A typed `Rules` struct is filled from `RTS_CFG.h` and ODF keys, starting with
the keys Classic Enhanced changes. No new rules file and no keyed "quirk map"
in v1. The simulation stops reading raw ODF keys directly.

**D10. Peers must agree on one digest of resolved rules.**
Multiplayer start, replays and saves compare a digest of the resolved
sim-visible rules (values, not names), including the tech tree and resource
model. The pack list is shown but informational: peers that differ only in
textures, models or render recipes can play together. Two peers on Classic and
Classic Enhanced refuse to start and name the differing key. Replays and
snapshots record the lineage and pack names.

**D11. Conflicts are solved by inheritance, not a new patch format.**
A file in an upper layer may include its own name from the layer below (already
the rule for ODFs) and override only what it changes. This is extended to `.h`
cfg files and `.tt` tech trees, with provenance. No TOML `[patch]` format.

## Rendering and scripting

**D12. Render recipes are overridable from mods.**
Ordnance recipes join nebula and surface recipes as files a mod can replace.
Shader authoring is a developer-only option: a mod file can shadow an embedded
`.wesl` shader by path, labelled unstable, with no versioned contract. A real
shader slot is built when a second author asks for one.

**D13. Mission Lua is restricted.**
Mission scripts run with an explicit standard-library set and memory and
instruction limits (no `io`, `os`, `require`, `load`). This is done regardless
of the rest of the plan.

## Not in v1

Executable fingerprinting; mods of Armada I (the stock Armada I lineage is in
v1, see D5); FO 3.2.7 and Dominion War lineages;
emulating engine bugs as toggles; the TOML ODF patch format; trigger and
timeline scripting for menu scenes; Lua in third-party packs; WASM; shader
contracts and shims; pack manager UI and load-order editor; dependency
download; signing policy; string-table overlays; hot reload beyond the shader
shadowing; per-match rule toggles.

## Work packages

Added to `docs/WORKPLAN.md` (PR #352). Done-when is in WORKPLAN.

| ID | Delivers | Decision |
|---|---|---|
| CON-33 | Mount fix | D6 |
| CON-34 | Coverage report and `why` | D7 |
| CON-35 | Typed rules and digest (seam PR, needs NET-01) | D9, D10 |
| CON-36 | Inheritance for `.h` and `.tt` | D11 |
| RND-28 | Recipe overrides, shader shadowing | D12 |
| LUA-09 | Lua restriction | D13 |
| CON-38..42 | Armada I map writer, text bodies, saved state, light colours, MDF parity (added 2026-11-12 with the D5 amendment, approved by Tammy) | D5 |

v1 is done when: Classic Enhanced loads over its parent; FO 4.0 loads with no
base Roots leaking in; the report prints coverage for every installed mod;
peers on Classic and Classic Enhanced refuse to start and name the differing
key while peers differing only by a texture or recipe folder are accepted; a
replay names its packs; a recipe ini dropped in a mod folder changes a headless
frame; every stock Armada I map reads, writes and reads back equal (CON-38).

## Changing these decisions

A decision changes only through a PR that edits this file, states which
decision and why, and is approved by both Tammy and Cedric. An agent that finds
a decision wrong or blocking comments on its package issue and stops; it does
not work around the decision. Number decisions are never reused: a replaced
decision is marked "Superseded by Dn" and kept.
