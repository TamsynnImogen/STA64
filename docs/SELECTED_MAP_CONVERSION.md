# Selected-map conversion seam

This contract precedes implementation of issue #301. CON-28's format helpers
are merged in #300; this change connects them through shared content conversion.
It does not add a second gameplay implementation or change save/network formats.

## Additive API

Add `MapDef::convert_bzn_named_in(m, selected_stem, db, vfs, index) -> Conversion`.
The selected stem is the chosen VFS path relative to `bzn/`, without `.bzn`:
for `bzn/custom/renamed.bzn`, pass `custom/renamed`. It is source identity, not
an arbitrary display title. Use that name during conversion, including grouped
warnings, rather than replacing `MapDef.name` afterward. Do not clone the whole
parsed BZN or change its internal filename to provide context.

Every existing public conversion method retains its signature and behaviour.
Their lack of selected-file context must not silently trigger companion loading
from an internal `msn_filename`. No shared fields or fakes change; `SEAM_REV`
stays 1 because the API is additive. New tests verify the old APIs and A2
outputs remain compatible.

Route both production callers (map-view/skirmish `build_sim`, front-end map
discovery) through the same named entry point. Their selected stems already
come from the VFS listing. Update the existing install-gated `build_sim` test
to pass the same stem convention as production, rather than its full VFS path.

## A1 companion rules

Only the named entry point and `m.a1.is_some()` enable this adapter. Read
`bzn/<selected_stem>.mdf` through existing `Vfs::read`, preserving its normal
case/slash normalization, archive/mod/loose-file precedence and whiteouts.
Never open a physical path, use the internal BZN filename, write game data or
fall through a malformed top-layer companion to another layer.

- No companion: keep no A1 player starts. A campaign's generic camera paths
  alone must not advertise it as a multiplayer map. Do not guess from the file
  name or mission class, or invent a default count.
- Unreadable companion, invalid/unsupported canonical count or unresolved
  requested camera point: emit a `Warning` naming that selected MDF, retain
  the scene/objects, and keep no starts. No misleading partial result.
- Valid companion: use CON-28's checked count and precise `Camera_i` resolver.
  Element `i` becomes existing `StartPos.slot = i`, with the existing Fixed
  position conversion and heading facing the map's XZ centre. Preserve Y,
  coincident and off-map points; no MDF grid inversion or new clipping rule.

The MDF count controls how many precise paths participate; preview `Start1`
etc. do not move them. Native malformed-MDF/default-eight handling and the
configuration-backed missing-path fallback remain explicitly unsupported.
See [the format scope](A1_START_LOCATIONS.md). All A2 and Native conversion
continues through its existing starts path, even if a same-named MDF exists.

The existing shared simulation creates seats and loadouts from `MapDef.starts`.
It keeps its current heading/formation/loadout policy; this seam does not claim
bit-identical original A1 loadout placement or complete skirmish gameplay.

## Verification

Synthetic checks must distinguish selected and internal names, nested/case
paths, mod/archive/loose precedence, whiteouts, missing/read-error/invalid MDF,
count-limited and missing camera paths, preserved XYZ/slots/heading and unchanged
old/A2 conversion. A headless client `build_sim` check must prove the selected
name reaches shared setup rather than merely testing the format helper.

Use the real mounted A1 root to convert all 49 maps with selected VFS names:
28 MDF pairs should contribute 113 starts, while the other maps remain scenes
without inferred multiplayer starts. Retain source object accounting and hash
original inputs before/after. Keep game data out of repository fixtures.

Run scoped content/client tests, strict scoped Clippy, workspace all-targets
checking and Android checks where practical. Distinguish shared/library checks,
an actual built client, APK builds and manual rendering/gameplay acceptance.
Do not launch GUI/input automation, Android or original executables. Preserve
existing staged clients, APK receipts and playtest launchers. The December
2026 desktop alpha remains the priority; no new completion date is assigned.
