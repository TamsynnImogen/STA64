# ADR 0004: Modding v1

- **Status:** accepted
- **Date:** 2026-11-11 (D5 and D10 amended 2026-11-12)
- **Deciders:** Tammy and Cedric (merged in #352)
- **Relates to:** [MODDING-IDEAS.md](../MODDING-IDEAS.md) (background and evidence); WORKPLAN packages CON-33..36, CON-38..42, CON-44, CON-45, SIM-32, RND-28, LUA-09, TOOL-12

This ADR covers how STA64 treats existing Armada mods and what the first
modding release contains. It is binding on all agent and human work in those
areas. D5 and D10 were amended on 2026-11-12 and approved by Tammy; see the
amendment notes under each.

## Context

The owner adopted the Fable review's recommendations on 2026-11-11 (see the
"ADOPTED PLAN" section of [MODDING-IDEAS.md](../MODDING-IDEAS.md)). That plan
supersedes compat tiers T0-T3 in favour of a measured coverage report, the
TOML `[patch]` format for ODFs, the keyed quirk map, `engine =` as lineage key,
the MOD-01..13 list and shader contracts, shims and `new-effect`. Background
and evidence are in MODDING-IDEAS.md and in research notes kept outside the
repository; this ADR records decisions only.

## Decision

### Scope of "mod"

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

### Games and profiles

**D4. Axes.**
- `profile` = the engine lineage, a closed list.
- `engine` keeps its existing meaning: the STA64 version a mod requires.
- A mod's data chain comes from `kind` and `ParentMod`.

**D5. v1 lineages: stock Armada II, stock Armada I, Fleet Operations 3.2.7 and Roots.**
FO 4.0, STA2 Classic and STA2 Classic Enhanced run on the Roots engine and are
covered through it. Stock Armada I runs as its own lineage (the `Sta1Classic`
profile); mods of Armada I are not covered in v1. Fleet Operations 3.2.7 is its
own lineage (the Fleet Operations 3.x engine: no `info.ini`, `[FleetOps]
Version=3.2.7` in `Data/packages.ini`, `AssetVersion` scheme 30207, below the
Roots threshold of 50000) and Dominion War, a mod built on it, is the test mod
for that lineage and in scope with it.

*Amended 2026-11-12: Armada I moved into v1 (Cedric; approved by
Tammy). It read "stock Armada II and Roots only" and listed Armada I with
FO 3.2.7 and Dominion War as not covered. Already on main for it: the
`Sta1Classic` profile (`sta_content`), the ZFS archive mount (`sta_vfs`, CON-15
and CON-27), A1 BZN headers, binary bodies and MDF start locations (`sta_bzn`,
CON-25, CON-26 and CON-28) and the A1 queue-capacity rule (SIM-20). What is
still missing is CON-38..42 in WORKPLAN.*

*Amended 2026-11-12 (second amendment): FO 3.2.7 and Dominion War moved into
v1 (Cedric confirmed that FO 3.2.7 is in scope; Tammy agreed to the general
idea of including everything from the start: "might aswell get what we can as
soon as possible so we dont have to rebuild or make janky additions later").
The sentence read "FO 3.2.7 and Dominion War are not covered in v1", and the
title read "stock Armada II, stock Armada I and Roots". Dominion War is an FO
3.2.7 mod installed on this machine; it is named here as the lineage's test
mod and in scope as a consequence (the author's reading, to be confirmed by
Cedric and Tammy). Armada I mods stay out (Cedric: "armada 1 stock only for
now, a1 mods later"). What was already on main for it: detection records
`[FleetOps] Version=` (CON-43, #447), such installs currently detect as
`Sta2Classic`, and `sta_fpq` reads the FPQ v1 archives they ship. What is
missing is CON-44 and CON-45 (and TOOL-12) in WORKPLAN. D2 applies unchanged:
`FleetOpsHook.dll` is never loaded and what 3.2.7 data needs from it becomes
native rules.*

**D6. Mounting must follow the mod's own rules.**
`standalone` and `ignoreParentMaps` are honoured. Today FO 4.0 mounts over base
Roots data and base ODFs, tech trees and maps leak in; this is fixed first.

### Verification

**D7. Coverage report.**
A `check-pack` tool reports, per mod: how many of its keys STA64 understands
(weighted by use), cfg keys it changes that STA64 ignores, unresolved map
references, and which layer and line supplied a given file or key.

**D8. Acceptance target.**
STA2 Classic Enhanced (a parent mod with changed `RTS_CFG.h`, tech trees, ODFs
and maps, no binaries) is the compatibility reference, and its author is asked
to try the result. Refit is the reference for rendering only.

### Rules and multiplayer

**D9. Rules are typed and come from the legacy files.**
A typed `Rules` struct is filled from `RTS_CFG.h` and ODF keys, starting with
the keys Classic Enhanced changes. No new rules file and no keyed "quirk map"
in v1. The simulation stops reading raw ODF keys directly.

**D10. Peers must agree on one digest of resolved rules, model geometry included.**
Multiplayer start, replays and saves compare a digest of the resolved
sim-visible rules (values, not names), including the tech tree, the resource
model and the derived collision geometry of every unit (ellipsoid axes,
bounding radius and centre offset, computed from the profile's SOD with
`scaleSOD` and `shieldPad`; not whole-file checksums of SODs). Definition keys
are part of the comparison; display strings are not. Physics always reads
geometry from the profile's SOD, never from a render-only replacement (glTF,
M3D). The pack list is shown but informational: peers that differ only in
textures, render-only models, geometry-identical re-exports or render recipes
can play together; a model whose simulation geometry differs blocks the match.
Two peers on Classic and Classic Enhanced refuse to start and name the
differing key; for geometry the refusal names the unit and the differing
value. Replays and snapshots record the lineage and pack names.

*Amended 2026-11-12: model geometry is part of the rules (Cedric; approved by
Tammy). It read "peers that differ only in textures, models or
render recipes can play together", meaning that peers whose installs differ
only in art play together. That cannot hold for models: in every install
99-100% of ships have no explicit size key (Roots base 43.7%), so the size of a
unit comes from its SOD, as in the originals, and Armada II and Roots checksum
the models between peers. The original size rule is stated in WORKPLAN SIM-32; the evidence notes are
kept outside the repository.
Implemented by CON-35 (digest) and SIM-32 (the original size rule).*

**D11. Conflicts are solved by inheritance, not a new patch format.**
A file in an upper layer may include its own name from the layer below (already
the rule for ODFs) and override only what it changes. This is extended to `.h`
cfg files and `.tt` tech trees, with provenance. No TOML `[patch]` format.

### Rendering and scripting

**D12. Render recipes are overridable from mods.**
Ordnance recipes join nebula and surface recipes as files a mod can replace.
Shader authoring is a developer-only option: a mod file can shadow an embedded
`.wesl` shader by path, labelled unstable, with no versioned contract. A real
shader slot is built when a second author asks for one.

**D13. Mission Lua is restricted.**
Mission scripts run with an explicit standard-library set and memory and
instruction limits (no `io`, `os`, `require`, `load`). This is done regardless
of the rest of the plan.

## Alternatives considered

Rejected by the decisions themselves:

- Compatibility tier labels (D1): mod support is reported by the coverage tool (D7) instead.
- A hook or `.asi` importer, or loading modified executables and FleetOps hooks (D2): behaviour is reimplemented as native rules.
- A new rules file or a keyed "quirk map" (D9): rules come from `RTS_CFG.h` and ODF keys.
- A TOML `[patch]` format (D11): conflicts are solved by inheritance.
- A versioned shader contract (D12): shadowing an embedded `.wesl` shader by path, labelled unstable, instead.

## Consequences

### Not in v1

Executable fingerprinting; mods of Armada I (the stock Armada I lineage is in
v1, see D5; FO 3.2.7 and Dominion War are in v1 too, see D5); mods of FO 3.2.7
other than Dominion War, beyond what the coverage report (D7) shows they need;
emulating engine bugs as toggles; the TOML ODF patch format; trigger and
timeline scripting for menu scenes; Lua in third-party packs; WASM; shader
contracts and shims; pack manager UI and load-order editor; dependency
download; signing policy; string-table overlays; hot reload beyond the shader
shadowing; per-match rule toggles.

### Work packages

Added to `docs/WORKPLAN.md` (PR #352). Done-when is in WORKPLAN.

| ID | Delivers | Decision |
|---|---|---|
| CON-33 | Mount fix | D6 |
| CON-34 | Coverage report and `why` | D7 |
| CON-35 | Typed rules and digest, collision geometry included (seam PR, needs NET-01) | D9, D10 |
| CON-36 | Inheritance for `.h` and `.tt` | D11 |
| SIM-32 | Original size rule: collision geometry from the SOD (proposed 2026-11-12, pending Tammy's approval) | D10 |
| RND-28 | Recipe overrides, shader shadowing | D12 |
| LUA-09 | Lua restriction | D13 |
| CON-38..42 | Armada I map writer, text bodies, saved state, light colours, MDF parity (added 2026-11-12 with the D5 amendment, approved by Tammy) | D5 |
| CON-44 | `Fo3` profile for the FO 3.2.7 lineage: variant, detection, defaults (second D5 amendment, 2026-11-12) | D4, D5 |
| CON-45 | FO 3.2.7 and Dominion War content coverage and install-gated golden tests (second D5 amendment) | D5, D7 |
| TOOL-12 | Bench skirmish scenario and digest golden on the FO 3.2.7 and Dominion War installs (second D5 amendment) | D5 |

v1 is done when: Classic Enhanced loads over its parent; FO 4.0 loads with no
base Roots leaking in; the report prints coverage for every installed mod;
peers on Classic and Classic Enhanced refuse to start and name the differing
key while peers differing only by a texture or recipe folder are accepted and a
model whose collision geometry differs is refused with the unit named; a
replay names its packs; a recipe ini dropped in a mod folder changes a headless
frame; every stock Armada I map reads, writes and reads back equal (CON-38); the FO
3.2.7 install and Dominion War detect as the FO 3.2.7 lineage, load, and the
report prints their coverage (CON-44, CON-45).

## Changing these decisions

A decision changes only through a PR that edits this file, states which
decision and why, and is approved by both Tammy and Cedric. An agent that finds
a decision wrong or blocking comments on its package issue and stops; it does
not work around the decision. Number decisions are never reused: a replaced
decision is marked "Superseded by Dn" and kept.
