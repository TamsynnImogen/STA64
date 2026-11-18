# ADR 0006: Profile detection order for Fleet Operations installs

- **Status:** accepted, in force
- **Date:** 2026-11-12
- **Deciders:** Proposed by Cedric Haase in #443; approved by Tammy (TamsynnImogen), who merged #443 on 2026-11-12.
- **Relates to:** [FO_PROFILE_DETECTION.md](../FO_PROFILE_DETECTION.md) (CON-31 contract, implementation notes and tests), CON-43, [ADR 0004](0004-modding-v1.md) D4 and D5; supersedes the precedence list of the CON-31 contract

## Context

The CON-31 contract ruled that the authored `[Mod] Name` Fleet Operations check applies to every VFS layer, including Data, with this precedence: an explicit `[Mod] Profile`, then the A1 ZFS signature, then the Fleet Operations name matcher, then the existing A2 layout and Native fallbacks.

CON-31 made the Roots base identify itself through the FO name in its
`Data/info.ini`. Since CON-33 (#415) a `standalone` mod no longer has base
`Data` mounted, so that name is not seen any more. STA2 Classic Mod and STA2
Classic Mod Enhanced Gameplay, both on the Roots engine and relying on it,
now detect as `Sta2Classic` instead of `Fo4`. The consequences are concrete:
the Roots point-defence rule is off for them, so Classic's Nebula point-defence
weapon (its `shotDelay` was changed from 3.5 to 0.1 because the Roots engine
obeys it) never fires, and the FO resource-name fallback and other `Fo4`
defaults are chosen or skipped by the wrong rule.

The sentence of the contract this amendment replaced:

> Do not inspect executable versions, directory paths, AssetVersion or
> `[modules]` lists to invent a second identification policy.

It is now: do not inspect executable versions or hashes or directory paths;
`AssetVersion` is read as described below, and only as described below.

## Decision

First hit wins.

1. **Explicit `Profile=`** in the mod's `info.ini` or `manifest.toml`.
   Unchanged; already wins (D4).
2. **`AssetVersion` in the `[mod]` section of the mod's own `info.ini`.**
   New. The Fleet Operations guide defines it as "the Fleet Operations version
   your mod has been designed for", derived from the version as major x 10000
   + minor x 100 + patch ("if the version of Fleet Operations is 3.2.5, then
   the AssetVersion is 30205"). The original engine uses it only for a
   skippable incompatibility warning. `AssetVersion >= 50000` means the Roots
   engine lineage. It travels with the mod folder, does not depend on what is
   mounted, and is data the mod ships, not executable fingerprinting.
3. **The host install's base `Data/info.ini`, read for identity only and not
   mounted.** Fallback for a mod under a Roots install whose own `info.ini`
   has no `AssetVersion`. It reuses the existing `looks_like_fo` name matcher;
   D6 mounting is unchanged.
4. **`[FleetOps] Version=` in `Data/packages.ini`.** FO 3.2.7 and Dominion War
   installs have no `info.ini`, but both carry `Version=3.2.7`. CON-43 only
   records it; since the 2026-11-12 D5 amendment the lineage is in v1 and
   CON-44 makes this step select the FO 3.2.7 profile.
5. **The existing heuristics** (name matcher, A2 layout, ZFS archive signature,
   Native fallback). Unchanged: stock Armada II, Armada I, Native.

The ZFS rule keeps its current place relative to the name matcher (contract
precedence item 2): an install holding a `.zfs` archive is Armada I before
steps 2 to 4 are consulted. This keeps the existing guarantee that A1 wins
over FO hints.

**Tammy's review note (2026-11-12):** "probably can get rid of [modules] since
A2FOExtensions will be fully integrated". Done: module lists are not read at
all, because STA64 builds those extensions in natively. A mod that has a
`[modules]` section loads without a warning; the section is ignored.

## Alternatives considered

- The fact needed is "this mod runs on the Roots engine". The mod states it
  itself in a field the original tools already define and that the original
  engine already reads, so there is no second policy to maintain.
- It does not depend on mounting, so a future change to what is mounted cannot
  break detection again, which is exactly what CON-33 did to the name check.
- Step 3 keeps the existing check useful for mods that predate the field.

Not used:

- **Executable or hook hashes and versions.** That is executable
  fingerprinting, excluded by "Not in v1" in MODDING_DECISIONS.md. Mods may
  also ship no executable at all (Classic does not).
- **Roots-only ODF or cfg keys in a mod's data.** Unreliable: a small mod may
  use none of them, and a mod can use one by accident.
- **The mere presence of `ParentMod` or `standalone` keys.** A native mod could
  use the same file, so the keys say nothing about the engine.

## Consequences

- `AssetVersion` cannot tell Fleet Operations 4.0 from Classic (both 50000)
  and should not: it is the same engine. The differences between stock-like
  and Fleet-Operations-like data must therefore come from the data: resource
  names from the string table, the HUD and theme from the race ODF's
  `interfaceConfiguration`, rig and damage-control behaviour from the presence
  of their ODF keys. `extra_resources` and `officer_capacity`, which `Sta2Classic` leaves off
  today, should be on for stock Armada II as well, since the stock game has
  them. The per-profile review is kept outside the repository.
- `AssetVersion = 30205` and similar values below 50000 do not select the Roots
  lineage; a mod designed for Fleet Operations 3.x is not a Roots mod.
- Optional, mirroring the original: when a mod's `AssetVersion` is older than
  the lineage it loads under, emit a load **warning**. Never a refusal.
- Content hashes of Classic loads change when they move from `Sta2Classic` to
  the Roots lineage. As in CON-31, this is intentional and no old save or
  replay compatibility is claimed.

## Open questions

Should the `Fo4` variant be renamed `Roots`, keeping `fo4` and `fo` as accepted
aliases in `Profile::from_name` (and keeping the canonical name written back in
saves and manifests as it is, or versioned)? The variant now names an engine
lineage that includes Classic, which is not "Fleet Operations 4". This proposal
does not decide it. A rename touches every `Profile::Fo4` use and the profile name in any persisted header, so it deserves
its own decision.

Not answered in this document or in its history as of 2026-11-13; the implementation notes in [FO_PROFILE_DETECTION.md](../FO_PROFILE_DETECTION.md) say `Fo4` keeps its name and the rename question is still open.
