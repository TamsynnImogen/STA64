# Station construction visuals

## Contract (before implementation)

This RND-11 follow-up changes presentation only. `UnitView.progress`, simulation
ticks, payment, completion, save formats and multiplayer rules stay shared and
unchanged. An unfinished station reveals opaque geometry in discrete spatial
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

## Native evidence and limits

Stock A2 `odf/ships/const.odf` authors six bees, `bot1` through `bot6`, `weldbeam`,
width 1 and takeoff/welding/closure delays 2/0.5/2 seconds. These are authored
defaults inherited by constructors, not proof of executable root-class defaults.
STA64 uses these three timing values as documented fallbacks where absent;
they remain overrideable in the constructor ODF. Stock class loading defaults
the beam sprite to `phaser` and reads the other fields from its class template.

The local A2FOExtensions stock-A2 pseudocode (`_inferred/WorkerBee.cpp`, update
at 0x00518250) shows home/exit, travel, timed welding, return and closure states,
and movement at 80 world units per second. `modules/A2FOAnimations/module.cpp`
defers the production-end animation until every bee has returned; this does not
extend logical station construction time. These are static source findings,
not native runtime acceptance or proof of identical A1/FO behavior.

`_unattributed/110000.cpp` at 0x00519010 copies class fields +0x570,
+0x578 and +0x574 into the bee's takeoff, welding and closure timers. Recovered
`Game Feature/BuildingEffect.cpp` at 0x0050d2f0 selects a mesh triangle corner
through an ordinal and transforms its position; it does not establish the
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
