# Strategic symbol/model handoff

Issue #365 reports early and displaced strategic icons. This repairs the
existing renderer; RND-27's broader distance-LOD/culling package is not claimed
complete, and its missing `render/cull.rs` prerequisite is not invented.

## Causes verified before editing

The icon currently fades at projected radii 14 to 7 pixels, independently of
the model-root hide rule at camera height 2500. Both can therefore draw at
once. Hiding also requires `SimEntity`, missing static roots with strategic
artwork. `Camera::world_to_viewport` returns logical target pixels, but the
icon writes those as UI node pixels without undoing `UiScale`; the HUD's
height/1080 scale displaces and resizes them. Root pivots also need not equal
the visual centre of a model with offset geometry.

## Bounded implementation contract

- Auto mode uses a single per-object projected-radius decision: enter symbol
  mode at 7 pixels or smaller and return to model mode at 10 pixels or larger.
  The interval provides hysteresis. Model bounds/scale naturally determine the
  corresponding distance. There is no simultaneous alpha crossfade. Explicit
  Tab Always/Off overrides remain available; default is Auto.
- The same decision gates the model root and symbol, including static roots
  with no simulation ID. Child selection rings/health bars inherit the model
  gate; symbol selection outlines remain. Save and restore only visibility
  changed by this system, preserving independent fog/cloak/mesh-node controls.
  A model already hidden by another owner must not gain a visible symbol.
- Project the transformed model-bound centre, with the same ScaleSOD and
  handedness conversion as its rendered hierarchy. Missing bounds keep the
  existing root-anchor fallback. Simulation positions never change.
- Aim the symbol UI at the projecting scene camera. Convert absolute logical
  viewport coordinates into that viewport's local UI coordinates, undoing
  `UiScale` for both placement and fixed logical-pixel size. Keep picking
  coordinates in logical target pixels so cursor and rendered symbol agree.
- Fleet folding considers only units actually in symbol mode. Hidden/folded
  symbols must not leave ghost icon hitboxes; no hidden model sphere should
  steal a click from a drawn symbol. Grid/height-bar presentation remains.
- Apply the handoff after current transforms/camera updates and before
  visibility propagation/UI preparation, preventing a one-frame overlap.

No simulation, save/wire, dependencies, mod/VFS rules or original assets change.
Defaults here are STA64 presentation policy, not newly claimed native constants.
The December desktop alpha and one shared cross-platform engine remain priorities.

## Verification

Required: zoom sweeps and threshold jitter; static and linked roots; hidden
visibility preservation; centre/scale/viewport conversion; symbol picking;
headless real GPU captures showing exclusive near/far representation and
centred icons at multiple UI scales; desktop regressions, scoped strict Clippy,
workspace check and Android library compilation. Stage an optimized manual
candidate and keep in-game acceptance distinct from these checks.

## Implementation evidence

The implementation uses one post-transform/post-camera-update pass before model
visibility propagation and UI preparation. Both static and linked model builders
supply the scaled, mirrored bounds centre; input uses the same transformed centre.
An icon's `shown` state is zero when folded, offscreen, or independently hidden,
and replaced model spheres are excluded from click targeting. Root visibility
has no other writer in the reviewed paths; damage-node and cloak-material control
remain separate. A future writer of that same root would need explicit coordination.

Verified on Linux with Bevy 0.20.1:

- Client library suite: 388 passed, one existing ignored test. This includes
  mixed-size static-root zoom/hysteresis, camera loss, pre-hidden visibility,
  offset-centre picking/box selection and folded-symbol hitbox regressions.
- Strict client Clippy across all targets and workspace/all-targets check passed.
- NDK 29/API 26 `aarch64-linux-android` client library check passed. This is
  compilation evidence only; this change has no new APK or device run.
- Real NVIDIA GTX 1660 Ti Max-Q / Vulkan, with Winit disabled and no window:
  six near/far/return captures at `UiScale` 0.66 and 1.5, an offset/scaled model,
  and viewport origin (32, 24). Near/return frames contain 784 magenta model
  pixels and zero green icon pixels; far frames contain 100 green icon pixels
  and zero magenta model pixels. All measured centroids are exactly (192, 152),
  matching the projected visual centre. Corresponding captures at both scales
  are byte-identical. First transition-frame state checks also pass.

Reproduce the GPU regression with:

```sh
renice -n 19 -p $$ >/dev/null && STA64_GPU_TESTS=1 STA64_STRATEGIC_TESTS=1 \
  cargo test -p sta64 --lib static_offset_model_and_symbol_are_exclusive_on_gpu \
  --jobs 2 -- --nocapture
```

`STA64_GPU_OUT` saves frames, including failed frames. The fixture classifies
magenta by channel dominance because default scene tonemapping raises its green
channel above the initial absolute cutoff; the engine's tonemapping is unchanged.
A software adapter prints an explicit skip and is not hardware-render evidence.

Classic/FO4 native appearance, zoom feel and FPS still require the user's manual
check in the optimized candidate. This bounded repair does not complete RND-27's
unimplemented distance-LOD/culling dependencies or change December alpha scope.
