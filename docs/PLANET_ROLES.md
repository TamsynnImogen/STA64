# Resolved planet role metadata (CON-30)

## Contract before implementation

The [planet ownership proposal in PR #318](https://github.com/TamsynnImogen/STA64/pull/318)
records the reproduced background mining/neutralization fault and the native
distinction between background objects and producer-backed planets. This
content-only prerequisite makes that distinction explicit in the immutable
S5 seam. It does not implement the subsequent simulation behaviour.

Add `PlanetRole`, a non-exhaustive enum with `Background` and `Producer`, and
`UnitDef::planet_role: Option<PlanetRole>`. `UnitDef::new` initializes it to
`None`; programmatically built definitions supply metadata explicitly. The
profile-aware ODF loader resolves it once after creating each unit. Standalone
`unit_def`, which has no profile argument, leaves it unset. No simulation or
client may infer new behaviour from this field in this package.

| Detected/selected profile | `planet_obj` | `planet` | Other classes |
|---|---|---|---|
| Sta1Classic | Background | None | None |
| Sta2Classic | Background | Producer | None |
| Fo4 | Background | Producer | None |
| Native | None | None | None |

These defaults follow the pinned own-image class/team-dispatch distinctions
in #318. Producer means the native planet rule family, not that a colony
exists, `isColonizable` is true, a job is active or the entity should have
Station kind. Neither role by itself supplies ownership, collision, damage,
mining or victory policy. Those require the separately specified shared
consumers and represented state.

## Authored override and admission

The STA64-owned ODF extension `STA64_planetRole` overrides the table for
`planet_obj` and `planet` units only:

```text
STA64_planetRole = "none"       // opt out of the resolved planet metadata
STA64_planetRole = "background"
STA64_planetRole = "producer"
```

Keys and values compare ASCII case-insensitively; trim surrounding whitespace
after normal ODF string decoding. Absence takes the table's default. Explicit
`none` is a real override and must survive includes and child-mod inheritance.
It does not mean neutral, zero hull, non-mineable or non-colonizable. Existing
include/VFS precedence decides the effective value; no second resolver or
`info.ini` feature flag is added.

An empty/unknown value warns with the file/key and falls back to the table.
On a non-planet unit any authored key warns that its class is unsupported and
is ignored; its role stays None. Weapon definitions are excluded from this
unit-only reader, with no new weapon warnings or fields. Preserve raw ODF data.
An explicit Native or cross-profile override selects STA64 metadata; it does
not assert support for that role in the original game.

Add `planet_obj` to `UNIT_CLASSES` so a genuine known background class need
not happen to carry `unitName` to load. This does not change the existing
unknown-class-with-unitName fallback, class spelling, ODF inheritance or the
craft-required-definition list. Admission and role are separate: Native
`planet_obj` loads but retains its existing None metadata default.

## Content identity and compatibility

Hash effective Some roles with a distinct domain-separated extension record,
using explicit Background/Producer tags rather than the Rust enum's memory
layout. None adds no bytes, preserving the original Native fake hash and the
genuine old Native fixtures. Equivalent absent/explicit defaults and include
spellings must have equal hashes; role changes must change hashes. Explicit
None and unresolved/default None are equivalent metadata.

Reserve marker `0xe9`, followed by the length-prefixed domain string
`sta64.planet-role.v1` and one byte: Background 0, Producer 1. Pending #309
already reserves `0xe7`/`0xe8` for its independent confinement additions;
neither is reused. This is an internal content-hash record, not a wire tag.

Newly recognized legacy roles change those databases' content hashes, even
before simulation consumes the metadata. This is intentional content-seam
identity, not an old legacy-save migration claim. The Native fake moon is
class `planet` with no resource field and keeps None; it must remain a
resource-like moon under the existing simulation. Do not edit frozen fixtures
or their expected hashes to conceal a change.

This adds no snapshot/entity/wire tag, save version, SEAM_REV change,
dependency, Bevy change or platform branch. Pending confinement PR #309 has
independent additions in the same content containers; preserve both at
integration and keep the role extension record distinct from its hash data.

## Scope and checks

Implementation owns the new `sta_content::planet` module and type export;
only the UnitDef field/constructor, one loader assignment, class-list entry
and conditional hash block in existing source; dedicated planet-role tests;
this document and the CON-30 work-plan row. No existing consumer is changed.

Verify every profile/class combination and explicit Native opt-in, default
constructor behaviour, valid case/whitespace/None overrides, inherited and
layered self-include overrides, invalid diagnostics, unsupported-class/weapon
exclusion, unitName-free admission and effective/no-op hash differences.
Check original fake identity and genuine v8 continuation, downstream shared
consumers, strict scoped Clippy, workspace all-target compilation, formatting
and an unchanged offline lock refresh. Optional installed-data probes must
stay read-only and separate from native runtime acceptance.

Native numeric planet fields (`crewModifier`, `isColonizable`,
`civilianPopulationModifier`, `maxCrewGain`) remain raw data here. Their
typed values, native parser semantics, zero/default rules and accounting
need a later contract; do not manufacture universal numeric defaults in this
role-only seam. The known background mining/Team bugs remain until the
simulation integration is implemented and verified. The December 2026
desktop alpha remains the priority; all future consumers share one engine
across desktop and Android.

## Implemented metadata and verification

The typed field, resolver, known-class admission and optional hash record are
implemented. Nine focused tests cover the contract, including zero-valued
colonization/crew keys leaving the Producer family intact. No simulation,
input, mission or rendering consumer uses the new field yet.

Local six-crate shared suites reported 566 passes, zero failures and ten
ignored tests, including genuine original v8 mining continuation and the
unchanged fixed Native fake-content hash. Some installed-content tests return
early without configured inputs, so this is not complete legacy validation.
Strict six-crate all-target Clippy, workspace all-target compilation, the
Android ARM64 client-library check (NDK 29/API 26), scoped formatting/diff and
an unchanged zero-package offline lock refresh passed. A library check is not
an APK build or Android runtime acceptance. Independent source/test review
found no remaining defect after correcting quoted-raw-value test assertions.

A separate read-only installed-content probe confirmed 15 Background roles
in GOG A1 and one Background/seven Producer roles in stock A2. The actual
`mearth` and `pb_clssh` definitions resolve as Background and Producer; the
same A2 tree forced to Native has all 395 unit roles unset. No new role
warnings occurred in those loads. All 1098 checked metadata-source hashes
stayed equal (A1/A2 loose ODFs and A1 archives, plus the selected Roots ODF
archive); models and textures are outside that preservation claim.

The Roots sample was explicitly forced to Fo4 after the existing detector
selected Sta2Classic. Its 660 units contained neither planet role and its
117 content warnings remain visible; this sample does not validate FO role
defaults or correct profile detection. Synthetic tests cover the Fo4 table.
The detector's treatment of the base layer's authored mod name needs separate
investigation. No original game execution, assets or private input paths are
distributed with this metadata prerequisite.
