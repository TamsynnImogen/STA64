# ADR 0009: Sprite colours in a storage buffer

- **Status:** proposed
- **Date:** 2026-11-20
- **Deciders:** the repo owner and Cedric. Accepted when both approve the PR that
  merges this file; not binding before. Agents do not merge it.
- **Relates to:** [ADR 0004](0004-modding-v1.md) D12; [ADR 0003](0003-clean-room-content-policy.md);
  WORKPLAN RND-22; issue #1020; PR #1000 (open draft, the earlier
  shared-materials attempt; see "Alternatives considered"); PR #1048 (merged test harness)

## Context

Sprite nodes (nebula puffs, glows, flat sprites, moon nimbi; the code is
`crates/sta64/src/render/sprites.rs`) are billboard quads, each with its own
`StandardMaterial` asset. The material's `base_color` is the look's animated
sRGB colour multiplied by the node's `SpriteFade`, and `animate` writes it
whenever it changes. Because the fade lives in the material colour, every
fading node is a material asset that changes.

Measured on STA64 itself (evidence grade: measured; one machine, an AMD RX
5700 XT under RADV, release build with `perf-trace`, 2752x1152 headless
offscreen, an Armada II map with dense nebulae, three interleaved runs each;
single-machine figures, not a general claim):

- About 4,000 sprite nodes; about 2,700 of them are faded and hold a private
  material, and their fade or their look's cycling colour changes every
  frame. Only about 170 meshes of about 16,000 entities were visible, so
  nearly all of the rewritten materials belong to nodes that are not drawn.
- `prepare_erased_assets<StandardMaterial>` takes about 8.5 ms of a frame of
  about 21 ms; the render schedule is about 19.7 ms of it, so the frame is
  bound by render-world CPU. Smaller costs scale with the same churn:
  extraction of changed materials, bind-group preparation, mesh re-extraction
  and re-specialisation.
- Cause in Bevy 0.20.1 (read from the Bevy source): a changed material asset
  runs the full render-asset prepare (bind group, new `MaterialProperties`,
  draw-function lookups), and `AssetChanged<MeshMaterial3d<M>>` makes the
  entity re-specialise, leave and re-enter the retained `Transparent3d` phase
  (`swap_remove`, then append) and extract its mesh again.
- PR #1000 shared one material per look, with a private copy only for faded
  nodes. It was correct and brought no measurable change (prepare time
  unchanged within the run-to-run spread). Faded nodes cannot share because
  colour times fade differs per node (about 1,400 distinct look-and-fade
  pairs among the 2,700), and the colour cycles, so each is rewritten every
  frame anyway; pooling by (look, fade) could at most halve the writes.

The fade cannot be applied on the GPU from the fade value alone and stay
byte-identical: main applies the fade in sRGB and Bevy then converts to
linear on the CPU (`LinearRgba::from(base_color)`), and the shader's `pow` is
not bit-equal to the CPU's `powf`. Any exact design therefore ships the final
linear colour, computed on the CPU with Bevy's own conversion.

Constraints that apply:

- The Stock look must not change (WORKPLAN RND-22: Stock is the default; the
  Refit look stays). Stock mode keeps its own blend modes, phases and
  post-lighting.
- `Transparent3d` sorts by distance only, stably, over a retained list, so
  the draw order of items at exactly equal distance depends on when each was
  last re-queued. On main every node whose material changed is re-queued that
  frame.
- ADR 0004 D12: embedded `.wesl` files must be shadowable by path, so any new
  shader is registered with `embedded_asset!` like the existing ones.
- Targets: desktop and Android over Vulkan. No WebGL or GLES target is
  claimed (`docs/ANDROID_SUPPORT.md`).

## Decision

**S1. Per-sprite colour and fade live in one storage buffer, indexed by the
mesh tag.** Each sprite node gets a slot in a `ShaderBuffer` of `[f32; 4]`
and a `MeshTag` holding the slot number. `animate` computes the same linear
colour main's material would have produced (`LinearRgba::from(Color::srgb(col
* k)).to_vec4()`) and writes it to the node's slot, only when it changed and
only while the node is shown (main's rule). The buffer is written in `Update`
inside `animate`, so it is extracted in the same frame (Bevy's own
`GpuComponentArrayBuffer` plugin is not used: its update system is unordered
and could upload a frame late). Slots come from a free list and are never
moved, so live tags do not change; the buffer grows by doubling.

**S2. One shared extended material per look.** `SpriteMaterial` is
`ExtendedMaterial<StandardMaterial, SpriteTint>`, where `SpriteTint` binds the
buffer with `#[storage(100, read_only)]` and is not bindless (so the combined
material is non-bindless: one bind group per look, a plain storage binding).
The material asset keeps only what is per look: texture, alpha mode, `unlit`,
`double_sided`, `cull_mode` and `uv_transform`, with `base_color` white. The
look key is texture, alpha key and sprite name; colour is no longer part of
it. Only looks with animated UVs change their asset, at most once per look per
frame. Colour and fade never touch an asset.

**S3. A forked copy of Bevy's forward PBR fragment shader with one added
multiply.** `crates/sta64/src/render/sprites/sprite.wesl` is a verbatim copy of
the forward path of `bevy_pbr` 0.20.1 `pbr.wesl` `fragment`, plus one line
after `pbr_input_from_standard_material`:

```wesl
pbr_input.material.base_color = pbr_input.material.base_color * sprite_colours[get_tag(in.instance_index)];
```

Main computes `(1*c)*t`; this computes `((1*1)*t)*c`. In IEEE arithmetic
`1*x = x` and multiplication commutes, so each channel is bit-equal before
alpha discard, decals, fog, tonemapping and premultiply, which stay the same
code. The file's header names the source file, Bevy version, line range and the
added line. The prepass for cut-out (`Mask`) looks keeps Bevy's own prepass
shader (base alpha is 1 on both sides).

**S4. Cases that stay on ordinary private `StandardMaterial` copies:**

- **Cloak and build fade** (`apply_fade`): it clones the node's material,
  sets alpha and switches to `AlphaMode::Blend`. For a sprite node the code
  first builds a "demoted" `StandardMaterial` from the shared look and the
  node's stored last colour and UV (`SpriteState`), which is what main's
  private material would hold at that moment, swaps the component, then runs
  the existing edit; restoring swaps back. Reason: alpha-fade needs a
  different blend mode, which is per material, and the frozen copy must equal
  main's.
- **Ghost tint** (`tint_ghost`): copies the base into a `StandardMaterial`,
  paints colour and textures, makes it blended and unlit. Reason: it already
  replaces the whole material (the repo does the same for hulls), and the
  shader's colour multiply must not apply to a painted ghost.

Bevy supports an entity changing material type over time (a swap within one
frame is handled); hulls and rocks in the repo already do so.

**S5. Parity mode, on by default.** When a node's state changed under main's
rule, `animate` calls `set_changed()` on its `MeshMaterial3d<SpriteMaterial>`.
`Changed<MeshMaterial3d<M>>` feeds the same specialise, re-queue and mesh
re-extraction path as `AssetChanged`, in the same frame, so the set of
re-queued nodes and its order match main's, and the tie order of equal-distance
sprites is unchanged. It keeps main's specialise and queue cost (forecast
about 0.6 ms) and removes the material prepare. Turning it off is a separate,
later change (open question 1).

**S6. Stock output stays pixel-identical.** The acceptance criterion is
byte equality of frames, main against branch, in Stock and in Refit camera
settings (see "Implementation outline"). A difference is a bug, not an
accepted change, unless open question 1 is decided otherwise.

## Alternatives considered

- **Shared materials per look (PR #1000).** Correct, exact, no measured gain
  (see Context): faded nodes still need one material each with a colour that
  changes every frame. It also adds state that this design removes (a share
  cache and private copies kept for the life of the node). Its test harness
  is kept (it is on main as #1048); its mechanism is not.
- **Write the material only for nodes some view will draw this frame.**
  Large saving on the measured scene (about 170 visible meshes), small where
  most puffs are visible and faded (camera inside a nebula field), and the
  per-node-per-frame nature of the cost stays. It is also hard to make exact:
  exact visibility is known only after `CheckVisibility`, which runs after
  asset events are turned into messages, so a write there is extracted a
  frame late and a node that comes into view is drawn once with stale
  content. Making it exact needs either a conservative visibility predictor
  over every view (including shadow and auxiliary views) or sending
  `AssetEvent::Modified` ourselves after `CheckVisibility`, which works
  against Bevy's asset bookkeeping. Risk to byte identity: high. It could be
  combined with this ADR later; it does not exclude it.
- **Pool private materials by (look, exact fade bits).** Exact, small work,
  but at most about 2,700 to about 1,400 writes (about 4 ms in a still view);
  with a moving camera every fade changes, nodes hop between pools and
  material adds, removes and prepares can make it worse than main. Quantising
  the fade would change the picture. The cost stays proportional to the
  number of distinct fades.
- **Vertex colours on a per-node mesh.** A mesh asset changing per node per
  frame costs mesh-changed marking, re-specialisation and re-upload through
  the mesh allocator, likely as much as the materials. Interpolating a
  constant varying is exact on AMD but no API guarantees it.
- **One custom instanced pass for all sprites.** It would need one sorted
  phase item per node to interleave with other transparent items, so most of
  the batching gain is lost, and it would reimplement the unlit path of
  `StandardMaterial` (fog, tonemapping, premultiplied add, MSAA, prepass for
  cut-out). Largest work and largest risk to identity.

## Consequences

**Forecast, not measured.** From the profile, about 8 of the 8.5 ms of
material preparation goes away on the measured scene, plus about 0.5 ms of
extraction and part of 0.55 ms of bind-group preparation; the replacement
cost is a 16-byte slot per node (about 64 KB for about 4,000 nodes) uploaded
each frame, and material prepares for at most one per look whose UV animates.
Frame time would go from about 21 to about 13 to 14 ms on that scene. Nothing
here has been built or run; the implementation PR must measure it (outline,
step 6) and may find less. With parity mode on, specialise and re-queue cost
(about 0.6 ms) stays.

**Forked shader.** `sprite.wesl` must be re-synced with Bevy's `pbr.wesl` on
every Bevy upgrade, and drift shows only when the GPU test runs
(`STA64_GPU_TESTS=1`, it skips without a hardware adapter). It is kept
checkable by the byte-equality harness of #1048 (`gpu_tests.rs`: reference
path against the new path, Stock-like and Refit-like cameras, with a control
run of reference against reference) and by the header and an upgrade
checklist line (re-diff against `pbr.wesl`). The check proves equality on the
adapters it runs on only.

**Tie order.** Without parity mode, nodes would no longer be re-queued every
frame, so for items at exactly equal distance (co-located sprites) the list
history differs from main and their draw order can flip. That matters for
blended sprites and, at rounding level, for additive ones. Binned phases for
cut-out sprites order bins by pipeline and bind group, so a new pipeline can
move masked sprites relative to other masked geometry, visible only at
exactly equal depth. Parity mode removes the first risk at about 0.6 ms of
forecast cost; the second is covered by a coplanar cut-out overlap in the GPU
test and is not removed by parity mode.

**Back ends.** Storage buffers in the fragment stage are needed. Fine on
desktop and on Vulkan Android. If GLES or WebGL2 ever became a target, a
fallback to the per-node material path would be needed, kept as production
code and used as the test reference. Not needed for the targets claimed
today.

**Mods and the Stock look.** Colour, fade, blend modes and UV animation are
unchanged for content, so mods and Stock see the same frames (S6). Mods may
shadow `sprite.wesl` by path (D12), and a shadowed copy of it would be out of
step with the engine's buffer binding; the binding index and buffer layout
are therefore an internal interface, not a mod-facing one, and this ADR makes
no promise about them. Refit differs from Stock only in camera settings
(tonemapping, MSAA, optional HDR and bloom), which the shared post-lighting
code handles the same way; both are in the GPU test.

**Harder to reverse as code grows.**

- Sprite nodes stop being plain `StandardMaterial`. Every system that edits a
  sprite node's material (a future hit flash, selection tint or new fade) must
  handle `SpriteMaterial` or demote the node through the helper.
- `MeshTag` is taken on sprite node entities, so Bevy's
  `GpuComponentArrayBuffer` cannot be used on them later.
- It is the first `ExtendedMaterial` in the repo.
- It opens the same channel for other per-frame materials (flipbook parts,
  construction sprites, `fx/quad.rs`); that is not decided here.

## Open questions

1. Is a different draw order for sprites at exactly the same distance
   acceptable, or must parity mode stay the default? Yes would allow dropping
   it for about 0.6 ms (forecast). Without a decision, parity mode stays on.
2. Is carrying a forked copy of a Bevy shader (`sprite.wesl`, re-synced on
   every Bevy upgrade) acceptable? If not, the choice is between the
   unchanged per-node cost, the narrower options above (which do not remove
   the cost for faded nodes), or a custom pass.
3. Should a device without fragment-stage storage buffers fall back to the
   per-node path as production code? Not needed for the claimed targets;
   decide when a new target is considered.
4. Is the same mechanism to be extended to other per-frame materials
   (flipbook parts, construction sprites, `fx/quad.rs`), or kept to sprite
   nodes until the first has been measured?
5. Is the forecast enough to proceed? The implementation PR is to report the
   measured result with the method of the 2026-11-20 measurement (same scene
   kind, interleaved runs, several runs each); if the gain is under the
   run-to-run spread, the change is reverted.

## Implementation outline

Each step is a PR or part of one; the gate is what must pass before merge.

1. **Harness and order** (merged as #1048): reference path, GPU
   byte-equality scaffold, `animate` ordered before `apply_fade` and
   `tint_ghost`. Gate: control run (reference against reference) byte-equal.
2. **Sprite material and slots** (`sprites/material.rs`, `SpriteTint`, the
   `SpriteSlots` resource, `MaterialPlugin` in map and viewer plugins). Gate:
   CPU slot tests: unique tags, reuse after despawn, buffer growth.
3. **Shader** (`sprites/sprite.wesl`, registered with `embedded_asset!`, D12,
   header and upgrade-checklist line). Gate: the GPU byte-equality test,
   reference against new path, Stock-like and Refit-like cameras, including
   co-located blended and additive pairs, a coplanar cut-out overlap, a
   fade-changing puff over a cycling colour, spawn and despawn mid-series and a
   parent toggled hidden/inherited.
4. **Adoption** by an observer on adding the sprite component, so no frame has
   an unadopted node; slot freed on removal. Gate: CPU parity test (the
   demoted material equals what the reference writes; the slot value is
   bit-equal to the linear colour of the reference's `base_color`).
5. **`animate`, cloak and build, ghost** (`apply_fade`, `tint_ghost`
   branches, parity mode). Gate: CPU guard (in steady state, zero
   `StandardMaterial` modified events from sprite nodes; `SpriteMaterial`
   events at most the number of looks per frame); fade and ghost copies equal
   main's field by field, including a node that was blinking off when the
   cloak began; the tag unchanged across demote and restore.
6. **End to end and measurement.** Gate: headless frames of main against
   branch, byte for byte, with `STA64_RENDER_PROFILE=stock` and `refit`, in an
   install-gated test or run (it reads the player's install at run time and
   skips without one; control run main against main first, on a scene
   without beams); then the before/after frame time with
   `sta-bench perf`, reported as a rule and a result in the PR, with the data
   kept outside the repo (ADR 0003).
7. **Optional, only with a decision on open question 1:** turn parity mode
   off. Gate: the same byte tests, with the tie cases expected to differ or
   shown not to.
