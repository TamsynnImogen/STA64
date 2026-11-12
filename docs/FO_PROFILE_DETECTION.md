# Fleet Operations identity on the base Data layer

> **Which order is in force.** The precedence list in this CON-31 contract
> is the original. It has been amended (2026-11-12, approved by Tammy): the
> order that applies is the five-step list under "Amended 2026-11-12" at the
> end of this document, implemented by CON-43. The original text below is kept
> for its history; where the two differ, the amendment wins. The CON-31
> exclusion of `AssetVersion` no longer applies; `[modules]` lists are not
> read at all (the extensions are native; Tammy, 2026-11-12).

## Pre-implementation contract (CON-31)

A read-only load of the user's Roots content selected Sta2Classic although
`Data/info.ini` declares `[mod] Name=Fleet Operations: Roots`. The existing
FO-name matcher recognizes that value. The detection loop skips the check
when the layer is named Data, selecting A2 defaults instead. This is a
shared-content profile-selection bug; no original game was executed.

Move only the existing authored `[Mod] Name` FO check so it applies to every
VFS layer, including Data. Continue using the existing ASCII-insensitive
`looks_like_fo` matcher. Keep folder-name hints limited to non-Data layers,
and retain existing loader, A2-layout and empty-Native fallbacks. Do not inspect
executable versions, directory paths or AssetVersion to invent a second
identification policy. (Amended 2026-11-12: `AssetVersion` is now read, see
the end of this document; `[modules]` lists are not read at all.)

The precedence remains:

1. The first valid explicit `[Mod] Profile` in layer priority order wins over
   all heuristics, including an FO Name in a different layer.
2. Without an explicit profile, the existing A1 ZFS signature wins before FO
   hints, even if a layer also names Fleet Operations.
3. The existing FO authored-name matcher applies to all layers. Existing
   non-Data folder and loader hints retain their behaviour.
4. Existing A2 layout and Native fallback rules remain unchanged.

An otherwise plain child mod inherits its FO base's identity. A child or
base needing different semantics can supply an explicit Profile; a name
alone does not turn off a parent/base FO hint. The existing override belongs
in the applicable `info.ini`, for example:

```ini
[Mod]
Profile = Fo4
```

`Profile = Native`, `Sta2Classic` and `Sta1Classic` remain valid alternatives
through the existing parser. Do not rewrite installations or add this key
automatically. The fix makes existing authored metadata effective.

## Scope and observable effect

Only the detection-loop check placement and corresponding module docs change
in `sta_content/src/profile.rs`. Add dedicated `tests/base_profile.rs`, this
document and the CON-31 WORKPLAN row. No change to the name matcher, profile
defaults, feature readers, VFS mounts, original data, numeric parsers, class
roles, dependencies, Bevy or simulation implementation is needed.

`ContentDb::load` should now select Fo4 automatically for the demonstrated
base instead of requiring a forced profile. Existing profile defaults and
resource naming follow that selection; existing `[Features]` and per-ODF
`Feature_...` overrides keep their precedence, including explicit zero.
Choosing Fo4 does not interpret old module activation lists or establish
complete Roots/A2FO functionality. Missing content and other loader warnings
must remain visible.

The corrected profile can change content hashes and gameplay defaults for
previously misidentified installs. This is intentional; do not claim old
wrong-profile save/replay compatibility or alter frozen fixtures to mask it.
Native fake identities, explicit-profile installs and correctly identified
A1/A2/FO content must retain their existing paths. No new wire/save tag or
format version is introduced.

## Verification gates

Use synthetic fixtures for FO names on Data, names on ordinary mod layers,
plain child inheritance, stock/unknown/empty Data, matching case and section
handling, valid explicit profiles at different layers, A1 ZFS precedence,
and existing loader/folder fallbacks. Exercise `ContentDb::load` to verify
automatic profile/features and explicit feature/ODF zero overrides, rather
than testing only the enum returned by `detect`.

Reproduce the installed Roots before/after detection and compare the final
automatic content with an explicitly forced Fo4 load of the same unchanged
VFS. Keep the 117 observed content warnings visible; do not attribute old
warnings to this condition move or claim full content validity. Preserve
input metadata hashes, record the selected source and distinguish this Rust
loader proof from native runtime, visuals, hooks or complete mod acceptance.

Run scoped and shared consumer tests, genuine Native continuation/fake-hash
controls, strict scoped Clippy, workspace all-target compilation, Android
ARM64 client-library checks and an unchanged offline lock refresh. Request
the existing full four-OS CI because profile selection affects effective
content rules. An Android library check alone does not build or run an APK.
The December 2026 desktop alpha remains the priority; desktop and Android
use this same shared loader.

A single append-only TODO check is also permitted under WORKPLAN's shared-file
rule: after merge, use the normal Roots launch and confirm the existing startup
content line reports Fo4, then check skirmish loading and resource labels. No
existing user check is ticked and no client source changes.

## Implemented correction and validation

The existing authored-name check now runs before the Data-layer skip. Seven
focused tests passed, covering base/mod names, inheritance, explicit profiles,
A1 archive precedence, old hints/fallbacks and actual `ContentDb::load` feature
resolution with mod/ODF zero overrides. No matcher or default table changed.

After integration onto main `8fb9b0b`, the six shared consumer suites
reported 642 passes, no failures and twelve ignored tests, including genuine
original v8 continuation and the existing Native fake identity. Some
install-gated tests return early without configured inputs; the total is not
complete installed-content acceptance. Strict six-crate all-target Clippy,
workspace all-target compilation, the Android ARM64 client-library check
using NDK 29/API 26, scoped formatting/diff and an unchanged zero-package
offline lock refresh passed. Independent read-only source/test review found
no concrete defect.

A preserved pre-fix executable of our own Rust content probe reproduced
Sta2Classic detection for Roots. Its profile source blob is identical to the
original main audit baseline. On that original baseline, the corrected
automatic load matched both forced-Fo4 loads, including the preserved
pre-fix reference (`32b49bb1c39f7f56`).

Newly merged content/confinement metadata intentionally changes content
identity. The final updated-main check therefore uses its own forced-Fo4
reference: automatic and forced loading both select Fo4, load 660 units/1151
definitions and produce hash `c6e79d90d1253700`. Feature bits and all 117
warning records are equal. The old and new baseline hashes are not asserted
equal. Warnings remain visible; no complete-mod-validity claim is made.
A1 and stock A2 controls still select Sta1Classic and Sta2Classic.

The selected Roots `info.ini` and ODF archive hashes are unchanged across
the check. This preservation claim does not cover every model or texture.
Private probe sources, logs, baseline executable identity and comparison
receipts stay outside the repository. No original executable was run, no
installation was modified, and no APK or device/visual acceptance is claimed
for this source. The appended TODO item remains the user's manual check.

## Amended 2026-11-12 (Cedric; approved by Tammy)

**Status: approved, in force.** Proposed in #443 and approved by Tammy's merge
the same day; implemented by CON-43. The CON-31 text above stays as written
for its history; where it differs, this section wins. Evidence and its grades
are in [research/CLASSIC_ENGINE.md](research/CLASSIC_ENGINE.md).

**Tammy's review note (2026-11-12):** "probably can get rid of [modules] since
A2FOExtensions will be fully integrated". Done: module lists are not read at
all, because STA64 builds those extensions in natively. A mod that has a
`[modules]` section loads without a warning; the section is ignored.

### Problem

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
> lists to invent a second identification policy.

It is now: do not inspect executable versions or hashes or directory paths;
`AssetVersion` is read as described below, and only as described below.

### Detection order

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

### Why this and not something else

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

### Consequences

- `AssetVersion` cannot tell Fleet Operations 4.0 from Classic (both 50000)
  and should not: it is the same engine. The differences between stock-like
  and Fleet-Operations-like data must therefore come from the data: resource
  names from the string table, the HUD and theme from the race ODF's
  `interfaceConfiguration`, rig and damage-control behaviour from the presence
  of their ODF keys. `extra_resources` and `officer_capacity`, which `Sta2Classic` leaves off
  today, should be on for stock Armada II as well, since the stock game has
  them. The per-profile review is in the research record.
- `AssetVersion = 30205` and similar values below 50000 do not select the Roots
  lineage; a mod designed for Fleet Operations 3.x is not a Roots mod.
- Optional, mirroring the original: when a mod's `AssetVersion` is older than
  the lineage it loads under, emit a load **warning**. Never a refusal.
- Content hashes of Classic loads change when they move from `Sta2Classic` to
  the Roots lineage. As in CON-31, this is intentional and no old save or
  replay compatibility is claimed.

### Open question for Tammy

Should the `Fo4` variant be renamed `Roots`, keeping `fo4` and `fo` as accepted
aliases in `Profile::from_name` (and keeping the canonical name written back in
saves and manifests as it is, or versioned)? The variant now names an engine
lineage that includes Classic, which is not "Fleet Operations 4". This proposal
does not decide it. A rename touches every `Profile::Fo4` use and the profile name in any persisted header, so it deserves
its own decision.

### Implementation notes (CON-43, 2026-11-12)

- `Profile::detect_with_source` returns a `Detection`: the profile, the step
  (`Explicit`, `Zfs`, `AssetVersion`, `HostBaseInfo`, `Heuristic`), the file
  that decided, what was read there, the `packages.ini` version and the
  warnings. `Profile::detect` is its `.profile`. `ContentDb::load` puts the
  warnings first in its warning list.
- Step 2 reads the `AssetVersion` of the top-most layer that states one,
  `Data` included (the Roots base `Data/info.ini` says 50000). A child mod
  without one inherits its mod parent's. A value below 50000 does not select
  the lineage and the later steps go on, so a 30205 mod under a Roots host
  still loads as Roots (step 3) and gets the warning below.
- Step 3 finds the host `Data` as the mounted `Data` layer or, for a mod
  folder under `Data/Mods`, that mod's grandparent. It reads `info.ini` and
  `packages.ini` there and nothing else; D6 mounting is not touched.
- Step 4 records `[FleetOps] Version=` (3.2.7 on the FO 3.x and Dominion War
  installs; the Roots install's own file says 4.0.0). In CON-43 it selects
  nothing; the profile variant for the 3.x engine is CON-44.
- The warning: when the detected profile is the Roots lineage, every layer
  whose `AssetVersion` is below 50000 gets one ("older than the Roots
  lineage this mod loads under"). A malformed value (not a whole number
  in `0..=4294967295`) is ignored with a warning, whatever step decided.
- `Fo4` keeps its name; the rename question below is still open.

### Tests an implementation must add

Install-gated tests (skip cleanly without the install), using the Roots install:

- Base Roots, `Mods/Fleet Ops 4.0`, `Mods/STA2 Classic Mod` and
  `Mods/STA2 Classic Mod Enhanced Gameplay` each detect as the Roots lineage
  (today `Fo4`).
- Stock Armada II detects as `Sta2Classic`; Armada I as `Sta1Classic`.

Synthetic tests:

- A mod with `AssetVersion=30205` does not detect as Roots; `50000` does;
  `49999` does not.
- A mod with no `AssetVersion` under a host whose base `Data/info.ini` names
  Fleet Operations detects as Roots, with the base not mounted.
- An explicit `Profile=` (manifest or `info.ini`) still wins over every step.
- A `.zfs` archive still wins over the new steps.
- A `packages.ini` with `[FleetOps] Version=3.2.7` is read (and, for v1,
  selects nothing new), and a missing or unparsable file is ignored.
- An older-than-lineage `AssetVersion` produces a warning and no failure.
- Detection results are unchanged for the existing `tests/base_profile.rs`
  cases.

The implementation is the WORKPLAN package CON-43 (now unblocked). MODDING_DECISIONS.md D4 (lineage order) and D5 would
need a follow-up wording change by the normal route; this PR does not touch it.
