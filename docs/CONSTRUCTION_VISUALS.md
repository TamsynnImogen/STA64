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

Only models attached while under construction retain CPU reveal data. Index
updates occur on chunk transitions, not every frame. Completion restores the
original shared mesh and releases private partial meshes. Missing or destroyed
stations/builders and cancelled jobs must release their effects.

`constructionChunks` is an optional STA64 presentation key in the **station's
resolved ODF**, default 24, valid 1..256. It determines a deterministic spatial
partition, not native-authored build sections: no such section metadata is
currently exposed by STA64's model parsers. All original triangles are restored
at completion, preserving winding, materials, attributes and model transforms.

The **constructor's resolved ODF** provides `numberOfWorkerBees`, `workerBeeName`,
`workerBeeHardpoints`, `weldingBeamSprite`, `weldingBeamWidth`,
`timeUntilTakeoff`, `timeWelding` and `timeUntilClosure`. Explicit zero timings
and widths are respected. Missing hardpoints fall back to the builder origin
with a diagnostic; named nodes use their animated model transforms. No original
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

## Verification gates

CPU tests: empty/partial/full geometry, next-chunk edge membership, transformed
targets, independent instances and cleanup; ODF overrides/zero values,
hardpoint transforms, waiting/travel/welding/return phases, paused/replayed
animation and simultaneous constructors. Existing cloak and Sabre rendering
checks must continue passing. Build an optimized desktop candidate for a manual
Classic Federation station build, including cancellation and a second build.
No Android APK or device support is established by these presentation changes.
