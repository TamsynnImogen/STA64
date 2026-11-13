# Wireframes and selection gauges

User reference correction after #383. Implements D1/D2/D5/D11. This is client
presentation only; no simulation, save, network or content-digest change.

Wireframes currently enlarge a GUI texture with a forced nearest sampler.
Use an explicit linear-filter opt-in for stock wireframe layers, retaining
authored CFG rectangles and SPR crops. Sampler policy must participate in both
page and resolved-art cache keys so shared texture pages cannot change other
buttons or sprite fonts. Do not mutate original art or scale the whole HUD.

World gauges currently scale thickness with the model's selection sphere and
sit near its origin, which puts wide station bars across the hull. Stock A2
instead projects the eight model-bounds corners and uses the resulting screen
rectangle: gauge height is a small integer, the shield gauge sits that height
plus 2 px above the rectangle top and the hull gauge that height minus 2 px
(read from the executable, static; not original-game execution or general FO
parity proof).

Retain the existing overlay lifecycle and shared view values. Supply converted,
scaled local model bounds on model attachment; project those bounds for gauge
width/anchor, with a selection-radius fallback only for missing bounds. Use
screen-pixel thickness and spacing, not radius-scaled thickness. Hide invalid,
behind-camera or offscreen projections safely. Height and gap defaults can be
overridden by optional STA64-only selectionGaugeHeight/selectionGaugeGap in
the resolved race GUI CFG (finite0..128 pixel values; zero height hides them).
LCARS switching clears stock-only overrides. Existing HUD bar rectangles remain authoritative.
The existing mesh path places gauges just ahead of the nearest model-bounds
depth to prevent self-occlusion. Current hierarchy transforms are read with
Bevy TransformHelper, explicitly after orbit_camera, before normal global
propagation; a first-frame test runs the real camera updater with stale globals.

Following the user's stock reference, numeric hull/shield/energy status values
show rounded percentages as bare integers, without a percent glyph. Current
crew stays a count. Keep existing faction health thresholds and CFG/A2FO colour
overrides, and test that changes in actual hull/shield/crew pools change tint.
No subsystem values are fabricated for backends not yet exposed by the view.

Verify sampler isolation, authored crops/rectangles, fractional scaling, camera
zoom/rotation and model translation/scale, thin separated gauges above the
bounds, invalid projection handling, live values/colours and overlay cleanup.
Run client regressions, mounted offscreen rendering, strict/workspace/ARM64
checks and stage an optimized manual candidate. Original files and prior builds
stay intact; December desktop alpha remains the priority.

## Verification

The current client suite passes468 tests (one existing ignored), including
sampler/cache isolation, bare status numbers and live colour changes, CFG
overrides/reset, bounds conversion, zoom/rotation/scaling, immediate camera
updates and existing overlay destruction/selection cleanup. Two windowless
NVIDIA captures measure both bars at2 pixels with a2-pixel gap; partial fill
widths shrink from103/52 to40/20 pixels at the farther camera. The initial
depth-occlusion failure is retained privately alongside the corrected probe.
Seven mounted Classic/FO4 HUD captures pass, including low and other-unit hover.
Strict client/all-target Clippy, workspace/all-target and NDK29/API26 ARM64
client-library checks pass. Optimized build/manual evidence follows in the PR
and artifact receipt. ARM64 compilation is not APK/device validation.
This is scoped rendering evidence, not full original-game visual parity.
