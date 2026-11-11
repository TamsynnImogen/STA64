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

## Executed checks

The six content seam tests passed with their original A1 and A2 corpus checks
explicitly enabled. They cover selected/internal name conflicts, case/slash and
nested paths, precise slots/XYZ/heading, scene preservation, missing/unreadable/
invalid MDFs, no partial paths, packed/loose/mod precedence, whiteouts and an
invalid child over a valid parent. Existing unnamed APIs and full A2 map
results remain compatible.

All 49 original A1 maps account for 17,595 source objects. Their 28 MDF pairs
produce 113 starts; the other maps do not infer starts from camera paths.
Control conversion preserves every `MapDef`, object count and warning count
for all 72 installed A2 maps (213 converted starts) and all 190 FO4 maps
(855 converted starts), after the same selected-name assignment the client
previously performed afterward. The loose A2 control uses `push_dir`; it is
not proof that the client recognizes that install layout through `mount_install`.

Both actual client tests passed without constructing an `App`: synthetic renamed
MDF selection reaches shared skirmish seats/loadout spawning; the original A1
front-end path lists 28 maps/113 starts, and `2blue` creates a two-seat setup
with 18 initial units. This is headless setup evidence, not rendering or a
completed game. The 90 existing content library tests also passed.

Independent source/test review found no confirmed defect. Strict content/client
all-targets Clippy, workspace all-targets checking and the Android ARM64 client
library check through cargo-ndk/NDK API 26 passed. The Android check is not an APK
or device-runtime result. All 79 original A1 BZN/MDF/ZFS hashes remained unchanged.
The actual Linux client also built and its early-exit mission reports read 49
A1 maps and 190 FO4 maps without unreadable entries. That report exits before
the window/audio/input pipeline and does not itself exercise starts; the client
tests above cover those paths. A separate hashed client is staged privately for
manual checks. An offline lock refresh changed zero packages; the lock stayed
unchanged. Existing playtest builds and original game data are preserved.

For the opt-in A1 checks (in addition to ordinary scoped tests):

```sh
renice -n 19 -p $$ >/dev/null
STA64_TEST_A1_INSTALL="/path/to/Star Trek Armada" cargo test -p sta_content --test selected_map original_a1_install --jobs 2 -- --ignored --nocapture
STA64_TEST_A1_INSTALL="/path/to/Star Trek Armada" cargo test -p sta64 --lib map::a1_start_tests --jobs 2 -- --include-ignored --nocapture
```

The installed tests require their environment explicitly and remain ignored in
ordinary CI. Synthetic coverage uses only generated data and owned temporary
directories. Manual front-end/skirmish checks are appended to `TODO.md`.
