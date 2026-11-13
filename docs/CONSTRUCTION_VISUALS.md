# Station construction visuals

## Contract (before implementation)

The initial RND-11 candidate changed presentation only. Following live feedback,
the shared simulation also holds constructors stationary while welding and
refuses new movement/build orders until completion or explicit Stop. Stop
cancels the unfinished station using the existing non-refundable active-weld
payment rule. Approach movement and pending-deploy refunds remain available.
`UnitView.progress`, construction duration and save/wire layouts are unchanged;
the lock derives from existing weld records. An unfinished station reveals opaque geometry in discrete spatial
chunks. It does not fade its entire hull in. Cloaking still uses its existing
independent opacity path.

The same geometry plan supplies the next unrevealed chunk's triangle-edge weld
targets, transformed through the actual model hierarchy. Beam endpoints, weld
flares and spark origins use those targets. Target selection must not use a
sphere around the station. Finished instances retain their original meshes and
materials; partial instance indices never modify a shared cached mesh.

Only models attached while under construction allocate reveal data. Index
updates occur on chunk transitions, not every frame. Completion restores the
original shared mesh and releases private partial meshes. The part cache keeps
one reusable CPU-readable full mesh per construction model/material variant;
that shared source remains cached after completion, whereas per-instance
partial buffers and edge plans are released. Normal model spawning continues
using GPU-only meshes. Missing or destroyed stations/builders and cancelled
jobs must release their effects.

`constructionChunks` is an optional STA64 presentation key in the **station's
resolved ODF**, default 24, valid 1..256. It determines a deterministic spatial
partition, not native-authored build sections: no such section metadata is
currently exposed by STA64's model parsers. All original triangles are restored
at completion, preserving winding, materials, attributes and model transforms.

The **constructor's resolved ODF** provides `numberOfWorkerBees`, `workerBeeName`,
`workerBeeHardpoints`, `weldingBeamSprite`, `weldingBeamWidth`,
`timeUntilTakeoff`, `timeWelding` and `timeUntilClosure`. Explicit zero timings
and widths are respected. An absent hardpoint list falls back to the builder
origin; an authored name that cannot be found also emits a diagnostic. Named
nodes use their animated model transforms. No original
ODF or sprite file is modified. The existing procedural Refit effects remain a
fallback when the named sprite is unavailable.

Bees wait at home, leave after the takeoff delay, move between actual pending
chunk edges, dwell for `timeWelding`, and return home at completion/cancellation.
Welding is disabled during travel and return. Closure delay begins after return;
production-end model animation should wait for the returning bees. Visual
timings use `AnimClock`, including pause/replay behavior, not wall time.
`STA64_workerBeeStandOff` in the constructor ODF sets minimum outward hover
clearance (default 15 world units; explicit zero is allowed). Geometry stays
hidden until the first completed weld interval, then visual progress is rebased
over the remaining simulation progress so logical completion is not delayed.

## Native evidence and limits

Stock A2 `odf/ships/const.odf` authors six bees, `bot1` through `bot6`, `weldbeam`,
width 1 and takeoff/welding/closure delays 2/0.5/2 seconds. These are authored
defaults inherited by constructors, not proof of executable root-class defaults.
STA64 uses these three timing values as documented fallbacks where absent;
they remain overrideable in the constructor ODF. The recovered **A1** class
loader defaults the beam sprite to `phaser` and reads other fields from its
class template; this does not prove an A2 executable default. A2's mounted ODFs
explicitly author `weldbeam`.

Armada 1 construction bees (read from recovered code, static; the pseudocode
and addresses are kept outside the repository) show home/exit, travel, timed
welding, return and closure states, with movement at 80 world units per second.
The A2FOExtensions animation module defers the production-end animation until
every bee has returned; this does not extend logical station construction time.
These are static source findings, not native runtime acceptance or proof of
identical A2/FO behavior.

The recovered corpus matches the A1 executable and does not match either local
A2 executable, so its offsets must not be applied to A2. No original game
executable was run. The user requested the movement lock; the inspected
binaries have not established A2 movement admission or cancellation refund
behavior. The FleetOpsHook cancel-build path adjusts a pending-item counter and
forwards cancellation to native code; this does not establish the forwarded
refund rule, and the bounded construction search did not establish bee hover or
weld movement rules. Historical patched-host notes are not clean A2 evidence.

A recovered class initialiser copies three class fields into the bee's takeoff,
welding and closure timers, and the building effect selects a mesh triangle
corner through an ordinal and transforms its position; neither establishes the
boundary policy above. Stock `Sprites/weapon.spr` defines `weldbeam` with
`tex1x4` animation and `xspark`/`xspark01` with `tex4x4` animation. The new path
must use the mounted mod's sprite table, not replace those authored textures.

## Verification gates

CPU tests: empty/partial/full geometry, next-chunk edge membership, transformed
targets, independent instances and cleanup; ODF overrides/zero values,
hardpoint transforms, waiting/travel/welding/return phases, paused/replayed
animation and simultaneous constructors. Existing cloak and Sabre rendering
checks must continue passing. Build an optimized desktop candidate for a manual
Classic Federation station build, including cancellation and a second build.
No Android APK or device support is established by these presentation changes.

## Mod overrides

Keep the worker-bee fields in the constructor's ODF (or its inherited
constructor base file). For example, the values requested for a six-bee ship:

```odf
numberOfWorkerBees = 6
workerBeeHardpoints = "bot1" "bot2" "bot3" "bot4" "bot5" "bot6"
weldingBeamSprite = "weldbeam"
weldingBeamWidth = 1.0
timeUntilTakeoff = 2.0
timeWelding = 0.5
timeUntilClosure = 2.0
```

Keep `workerBeeName` set to the mod's existing bee model. Put the optional
`constructionChunks = 24` on the **station** being built. These are ordinary
resolved ODF values: inherited values can be replaced in a child ODF or a mod
layer without changing the original installation.

## Headless checks

The regular client library suite includes geometry, motion and animation tests.
The optional real-asset render check requires an installed model and a working
graphics adapter, but creates no window and sends no desktop input:

```sh
STA64_GPU_TESTS=1 STA64_CONSTRUCTION_TESTS=1 \
STA64_TEST_INSTALL="<install root>" STA64_TEST_MOD="<active mod>" \
STA64_TEST_MODEL=fbase STA64_GPU_OUT="<new output directory>" \
cargo test -p sta64 --lib --jobs 2 \
  installed_station_reveals_opaque_chunks_offscreen -- --nocapture --test-threads=1
```

It advances one instance from zero to half progress after GPU extraction,
then completes it with the hull depth prepass enabled. It checks that zero
shows no hull, compares the partial hull with the full silhouette, and compares
the completed instance with an untouched original mesh. It does not by itself
verify worker movement or complete in-game construction acceptance. Existing
Sabre rendering checks remain separate.

## Validation (2026-11-11)

### Live feedback follow-up (contract before changes)

The first live candidate was rejected: bees hovered too close to the hull for
the beam to be visible, and chunks appeared before welding had taken effect.
Increase the default stand-off to 15 world units, outward from the station
centre; `STA64_workerBeeStandOff` on the constructor ODF can override it.
This minimum beam clearance is STA64 presentation policy, not a recovered native
beam-length default. Native worker positioning uses a radius-derived offset;
the previous fixed 3-unit gap did not provide adequate clearance.

For constructors with active bees, hold geometry at zero until a bee completes
its first authored `timeWelding` interval, then rebase visual progress over the
remaining simulation progress. This prevents an initial jump while keeping
logical completion at the existing simulation tick. Constructors without bees
retain progress-based chunk reveal. Completion always restores the full model,
including exceptionally short jobs that finish before bee arrival.

The user also requires the constructor to stop and refuse movement once station
welding starts. The approach to a chosen construction site is still mobile;
Stop/cancellation and completion release the lock. Use the existing active weld
record rather than adding save/wire state. Enforce this in shared simulation,
including queued orders and already-running movement, with desktop/Android
using the same rule. This is an intentional gameplay change requested after
the first candidate only. The lock is visible through the existing `BUILDING`
view flag, so Move/Mine/Build controls are unavailable while Stop remains.
Skirmish AI uses that same view flag to choose idle constructors and avoids
issuing replacement builds to a welding rig.
Collision separation treats a welding constructor as fixed; old waypoints and
residual velocity are cleared. Explicit Stop removes its unfinished child
without refunding the active weld. Stop on an unfinished station cancels that
site and releases its constructor too. Engines running different gameplay
versions should not be mixed merely because the wire layout is unchanged.

### Initial candidate evidence (live acceptance rejected)

- Client library: 361 passed, zero failed, one existing ignored test. Includes
  geometry partition/edge/restore checks, exact quoted ODF hardpoints, explicit
  zero overrides, waiting/travel/welding/return phases, late geometry, parallel
  constructor binding and home transforms, bay closure and consecutive jobs.
  Existing cloak and five-minute FX cleanup checks pass.
- Production regression subset: 33 passed. No simulation, save or wire code
  changed.
- Strict client all-target Clippy and workspace/all-target compilation pass.
- Real Classic `fbase`, Vulkan/headless depth prepass: 0 / 6,228 / 10,188 lit
  pixels at zero / half / complete progress. Completed and untouched reference
  silhouettes match exactly. This exercises partial index updates **after**
  renderer extraction, then restoration of the original mesh.
- Sabre `fdestroy2`: 12,657 lit pixels with and without the depth prepass,
  12,915 with MSAA4; no regression of the accepted invisible-hull fix.
- Android NDK 29, API 26, `aarch64-linux-android` client library compilation
  passes. This is not a full APK build or Android/Waydroid runtime test.
- Seven inspected stock ODF/sprite and native-source hashes remain unchanged.

Remaining acceptance: the optimized desktop build needs a live Classic station
build, cancellation and second-build check, including FPS during construction.
Stock A1/FO visual equivalence has not been established. Chunk order is the
STA64 spatial policy, not recovered native section ordering. The previous
24-bee presentation cap remains; this does not alter simulation unit limits.
No desktop milestone or December 2026 alpha date was changed.

### Live-feedback follow-up verification

- All-target client/simulation tests pass: 363 client library tests (one existing
  ignored), 286 simulation library tests and their integration suites. Fourteen
  AI tests pass, including the busy-constructor positive/negative controls.
- Lock tests cover all four content profiles, direct/queued moves, replacement
  builds, mixed groups, residual velocity, craft/field displacement, explicit
  Stop on either constructor or site, destruction and snapshot continuation.
  Existing construction completion ticks are unchanged.
- Strict all-target Clippy for client, simulation and AI, workspace/all-target
  compilation and Android ARM64 client-library compilation pass.
- `construction` (3,900 ticks), `construction-fo` (4,000) and `rally` (1,700)
  scenarios pass all 67 steps, with zero failures or skips.
- The real Classic shipyard GPU capture is zero pixels while waiting and at
  the first-weld latch, 3,135 pixels at the rebased midpoint (identical to the
  unblocked midpoint), and 5,467 at completion/reference. The final silhouettes
  match exactly. The Sabre prepass/MSAA regression remains unchanged.

The first live candidate remains rejected. On the replacement optimized
`construction-a1666d0` build, the user confirmed "All three look correct":
visible beams, later chunk reveal and a fixed constructor under normal/queued
movement. This accepts those three requested fixes. FPS was not reported;
broader native visual parity and Android runtime remain unverified. Original
assets and reference executables were inspected read-only.
