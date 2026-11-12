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
