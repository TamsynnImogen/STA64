# Fleet Operations identity on the base Data layer

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
executable versions, directory paths, AssetVersion or `[modules]` lists to
invent a second identification policy.

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

The six shared consumer suites reported 564 passes, no failures and ten
ignored tests, including genuine original v8 continuation and the existing
Native fake identity. Some install-gated tests return early without configured
inputs; the total is not complete installed-content acceptance. Strict
six-crate all-target Clippy, workspace all-target compilation, the Android
ARM64 client-library check using NDK 29/API 26, scoped formatting/diff and
an unchanged zero-package offline lock refresh passed. Independent read-only
source/test review found no concrete defect.

A preserved pre-fix executable of our own Rust content probe reproduced
Sta2Classic detection for Roots. Its profile source blob is identical to the
main audit baseline. The corrected loader selects Fo4 automatically, loads
660 units/1151 definitions and produces content hash `32b49bb1c39f7f56`.
This matches both the corrected forced-Fo4 load and the preserved pre-fix
forced-Fo4 result. Automatic/forced feature bits and all 117 warning records
are equal. These warnings remain visible; no complete-mod-validity claim is
made. A1 and stock A2 controls still select Sta1Classic and Sta2Classic.

The selected Roots `info.ini` and ODF archive hashes are unchanged across
the check. This preservation claim does not cover every model or texture.
Private probe sources, logs, baseline executable identity and comparison
receipts stay outside the repository. No original executable was run, no
installation was modified, and no APK or device/visual acceptance is claimed
for this source. The appended TODO item remains the user's manual check.
