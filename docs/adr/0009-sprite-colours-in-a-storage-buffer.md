# ADR 0009: Sprite colours in a storage buffer

- **Status:** proposed
- **Date:** 2026-11-20 (S2 and S3 amended 2026-11-23, S5 amended 2026-11-26)
- **Deciders:** Tammy and Cedric (accepted when both approve the PR that
  merges this file; not binding before). Agents do not merge it. The
  amendments of S2, S3 and S5 need the same two approvals.
- **Relates to:** [ADR 0004](0004-modding-v1.md) D12; [ADR 0003](0003-clean-room-content-policy.md);
  WORKPLAN RND-22; issue #1020; PR #1000 (open draft, the earlier
  shared-materials attempt, closed when this ADR is accepted; see "Alternatives
  considered"); PR #1048 (merged test harness); issue #1138 and PR #1183
  (step 1, whose findings led to the amendment)

## Context

Sprite nodes (nebula puffs, glows, flat sprites, moon nimbi; the code is
`crates/sta64/src/render/sprites.rs`) are billboard quads, each with its own
`StandardMaterial` asset. The material's `base_color` is the look's animated
sRGB colour multiplied by the node's `SpriteFade`, and `animate` writes it
whenever it changes. Because the fade lives in the material colour, faded nodes of a look cannot
share a material unless their fades are bit-equal, and every node whose colour or fade changes is a
material asset that changes.

In this ADR, "main" is the code on the main branch today, and a node's
"look" is what it shares with every node drawing the same sprite: texture,
blend mode, UV animation and colour animation.

Measured on STA64 itself (evidence grade: measured; one machine, an AMD RX
5700 XT under RADV, release build with `perf-trace`, 2752x1152 headless
offscreen, an Armada II map with dense nebulae, three interleaved runs each;
single-machine figures, not a general claim):

- Every sprite node has its own material on main. About seven in ten nodes
  in the scene carry a fade below 1 (nebula near fade and occlusion), and
  their look's colour cycles, so each of their materials is rewritten every
  frame; with a moving camera their fades change as well. Only about one
  entity in a hundred was a visible mesh, so nearly all rewritten materials
  belong to nodes that are not drawn. (Node shares counted in a debug build
  with a temporary counter, same scene at a lower resolution, separate from
  the release timing runs; the counter is not committed.)
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
  colour times fade differs per node (about half as many distinct
  look-and-fade pairs as faded nodes), and the colour cycles, so each is rewritten every
  frame anyway; pooling by (look, fade) could at most halve the writes.

The fade cannot be applied on the GPU from the fade value alone and stay
byte-identical: main applies the fade in sRGB and Bevy then converts to
linear on the CPU, and the shader's power
function is not bit-equal to the CPU's. Any exact design therefore ships the final
linear colour, computed on the CPU with Bevy's own conversion.

For reference (read from the executable, static; not measured): in the
original games a nebula's fade dims a puff's colour, and translucent sprites
are drawn back to front. Where two sprites are at exactly equal depth, their
order in the original follows no designed rule.

Constraints that apply:

- The Stock look must not change (WORKPLAN RND-22: Stock is the default; the
  Refit look stays). Stock mode keeps its own blend modes, phases and
  post-lighting.
- `Transparent3d` sorts by distance only, stably, over a retained list, so
  the draw order of items at exactly equal distance depends on the
  list's history: removal moves the last item into the gap and re-queued items
  are appended. On main every node whose material changed is re-queued that
  frame.
- ADR 0004 D12: embedded `.wesl` files must be shadowable by path, so any new
  shader is registered with `embedded_asset!` like the existing ones.
- Targets: desktop and Android over Vulkan. No WebGL or GLES target is
  claimed (`docs/ANDROID_SUPPORT.md`).

## Decision

**S1. Per-sprite colour and fade live in one storage buffer, indexed by the
mesh tag.** Each sprite node gets a slot in a `ShaderBuffer` of `[f32; 4]`
and a `MeshTag` holding the slot number. `animate` computes the linear colour
with the same Bevy conversion main's material goes through, and writes it to the node's slot, only when it changed and
only while the node is shown (main's rule). The buffer is written in `Update`
inside `animate`, so it is extracted in the same frame (Bevy's own
`GpuComponentArrayBuffer` plugin is not used: its update system is unordered
and could upload a frame late). Slots come from a free list and are never
moved, so live tags do not change; the buffer grows by doubling.

**S2. One shared extended material per look, bindless where the device
supports it.** `SpriteMaterial` is `ExtendedMaterial<StandardMaterial,
SpriteTint>`. `SpriteTint` binds the colour buffer in Bevy's bindless mode:
the buffer is a storage binding array with its own small index table, so the
combined material is bindless like the standard material and all looks share
one material slab, one bind group and, for cut-out looks, one batch set, as the
per-node standard materials do on main. Where the device lacks what Bevy's
bindless mode needs for a buffer binding array (see "Back ends"), Bevy falls
back to the non-bindless form of the same material: one bind group per look and
a plain storage binding. The material asset keeps only what is per look:
texture, alpha mode, `unlit`, `double_sided`, `cull_mode` and `uv_transform`,
with `base_color` white. The look key is texture, alpha key and sprite name;
colour is no longer part of it. Only looks with animated UVs change their
asset, at most once per look per frame. Colour and fade never touch an asset.

*Amended 2026-11-23, from the step 1 findings (issue #1138, PR #1183). S2 read:
"`SpriteTint` binds the buffer with `#[storage(100, read_only)]` and is not
bindless (so the combined material is non-bindless: one bind group per look, a
plain storage binding)". That form was built (PR #1183's first head) and failed
the GPU byte-equality gate in all three camera settings: the coplanar cut-out
overlap drew in the other order, because each look's own bind group put the
cut-out looks in separate batch sets of the binned alpha-mask phase, ordered by
bind-group index (parity mode does not reach the binned phase); and one frame
per camera differed by one level in a region of additive sprites, with the same
set of re-queued nodes, the same walk order and the same sorted transparent
order as the reference (cause not established). With the bindless form all
three camera settings were byte-equal to the reference, the control run and the
negative control behaving as before; the cut-out looks then shared one batch
set, and the one-level difference was gone although the grouping of items into
draw calls still differed from the reference. Evidence grade: measured on
STA64, one machine and adapter (an AMD RX 5700 XT under RADV), on the invented
scene of the GPU test.*

**S3. A forked copy of Bevy's forward PBR fragment shader with one added
multiply.** `crates/sta64/src/render/sprites/sprite.wesl` is a verbatim copy of
the forward path of the `bevy_pbr` 0.20.1 fragment shader, plus one line after
the standard-material input is built that multiplies the base colour by the
node's slot in the colour buffer, indexed by its mesh tag. Main multiplies the
texture sample by the node colour held in the material; the fork multiplies
the same two values with the material colour set to exactly 1, which gives the
same IEEE result, so each channel is bit-equal before alpha discard, decals,
fog, tonemapping and premultiply, which stay the same code. Besides the added
line, the fork declares the colour buffer in two branches, one for the bindless
form (the binding array and its index table, read through the material slot
Bevy already keeps per instance) and one for the non-bindless fallback (the
plain storage binding); there is still exactly one added multiply. The two
shaders still compile differently (a storage operand instead of the material
uniform, a separate pipeline), so the GPU byte-equality test, not this
argument, is the proof on each adapter, and it proves only the form that adapter
runs. The file's header names the source file, Bevy version, line range, the
added line and the two-branch declaration. The prepass for cut-out (`Mask`)
looks keeps Bevy's own prepass shader.

*Amended 2026-11-23 with S2: the two-branch declaration of the colour buffer
was added; the multiply is unchanged.*

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
rule, `animate` lists the node. In `PostUpdate`, after Bevy has decided which
views see which nodes and before the specialisation checks, the node's
`MeshMaterial3d<SpriteMaterial>` is marked changed if at least one view sees
the node (a camera, or a light's shadow view). In Bevy 0.20.1
`Changed<MeshMaterial3d<M>>` feeds the same specialise, re-queue and mesh
re-extraction systems as `AssetChanged`, in the same frame. So in every view
the nodes re-queued are main's: a node no view sees is in no view's phase, so
marking it changes nothing any view draws, and a node that comes into a view is
queued as newly visible, first and once, with or without the mark. The slot,
the state and the look's uv are still written in `Update` for every node, as
before. The mark stays in the frame the state changed (it is ordered before the
system that turns a changed material into a changed mesh), so it is not a frame
late. Parity mode keeps main's specialise, queue and mesh re-extraction cost
for the nodes a view sees only, and removes the material prepare. Turning it
off is a separate, later change (open question 1).

*Amended 2026-11-26 (issue #1237). The mark was applied in `Update` to every
node whose state changed, seen or not. On a map with many nodes no view sees,
that re-extracted and re-specialised those nodes in every frame without
changing any picture; the cost is now only for nodes a view sees. Tie order:
the re-queue order is the iteration order of Bevy's set of changed entities,
and that set now holds only the nodes a view sees, so it is not the set main
iterates, and Bevy fills it from several threads. Two nodes in one view at
exactly equal distance may therefore be drawn in another order than on main,
and so may such a node and one that is not re-queued in that frame (the order
of removals from the phase moves the items left in it). This is open question
1, narrowed to nodes, and nothing makes it absent by construction. The GPU
byte-equality test covers nodes entering, leaving and re-entering a view, two
cameras that see different nodes, nodes under a hidden parent and co-located
blended and additive pairs whose entity ids a crowd of unseen nodes separates;
on the test adapter it found no difference. Evidence grade: measured on
STA64, one machine and adapter.*

**S6. Stock output stays pixel-identical.** The acceptance criterion is
byte equality of frames, main against branch, in Stock and in Refit camera
settings (see "Implementation outline"). A difference is a bug, not an
accepted change, unless open question 1 is decided otherwise.

## Alternatives considered

- **Shared materials per look (PR #1000).** Correct, exact, no measured gain
  (see Context): faded nodes still need one material each with a colour that
  changes every frame. It also adds state that this design removes (a share
  cache and private copies kept for the life of the node). Its test harness
  is kept (it is on main as #1048); its mechanism is not; #1000 is closed
  when this ADR is accepted.
- **Write the material only for nodes some view will draw this frame.**
  Large saving on the measured scene (about one entity in a hundred visible), small where
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
  but it at most halves the writes (forecast: about 4 ms in a still view);
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
- **A non-bindless sprite material everywhere (S2 as first written).** Simpler
  shader binding and the same behaviour on every device. Built in step 1 and
  rejected on its gate (amendment note under S2): it changes the draw order of
  coplanar cut-out sprites of different looks, which parity mode cannot reach,
  and it showed a one-level difference in one frame per camera whose cause was
  not found. It remains the fallback where bindless is not supported.

## Consequences

**Forecast, not measured.** From the profile, about 8 of the 8.5 ms of
material preparation goes away on the measured scene, plus about 0.5 ms of
extraction and part of 0.55 ms of bind-group preparation; the replacement
cost is a 16-byte slot per node (tens of kilobytes on the measured scene) uploaded
each frame, and material prepares for at most one per look whose UV animates.
Frame time would go from about 21 to about 13 to 14 ms on that scene, where
main-world CPU (about 9 ms) would then be close to the bound. Nothing
here has been built or run; the implementation PR must measure it (outline,
step 6) and may find less. With parity mode on, specialise, re-queue and mesh re-extraction cost (about
1 ms together) stays.

The amendment of S2 does not change this: with the bindless form a slot write
still touches no material asset (only the colour buffer, rewritten in place
while its size stays), so no material is prepared and no bind group is rebuilt
for it. A look whose UV animates re-prepares that one look's entry in the
shared slab, as a standard material does on main. Growing the colour buffer
rebuilds the slab's bind group once. The forecast stays a forecast, and step 6
still measures it.

**Forked shader.** `sprite.wesl` must be re-synced with Bevy's `pbr.wesl` on
every Bevy upgrade, and drift shows only when the GPU test runs
(`STA64_GPU_TESTS=1`, it skips without a hardware adapter). It is kept
checkable by the byte-equality harness of #1048 (`gpu_tests.rs`: reference
path against the new path, three cameras (Stock-like; tonemapped with 4x MSAA;
HDR with bloom), a control run of reference against reference, and a negative
control that a path writing a frame late must differ) and by the header and an upgrade
checklist line (re-diff against `pbr.wesl`). The check proves equality on the
adapters it runs on only.

**Tie order.** Without parity mode, nodes would no longer be re-queued every
frame, so for items at exactly equal distance (co-located sprites) the list
history differs from main and their draw order can flip. That matters for
blended sprites and, at rounding level, for additive ones. Binned phases for
cut-out sprites order bins by pipeline and bind group, so a new pipeline can
move masked sprites relative to other masked geometry, visible only at
exactly equal depth. Parity mode removes the first risk in practice (the same set is re-queued;
see S5) at about 1 ms; the second is covered by a coplanar cut-out overlap in the GPU
test and is not removed by parity mode.

Since the amendment of S2, the bindless form removes the second risk where it
runs: all looks share one slab and bind group, so cut-out looks of different
looks fall into one batch set and keep the order of their queue history, as the
per-node materials do on main (measured on the test adapter, see the note under
S2). The non-bindless fallback keeps the risk: on a device without bindless
support, coplanar cut-out sprites of different looks may draw in another order
than on main, and the one-level difference seen in step 1 may appear; neither
has been tested on such a device. Looks beyond one slab's capacity land in a
second slab and so in another batch set, so equal-depth order between looks in
different slabs may differ from main; the per-node path on main is
slab-dependent in the same way.

**Back ends.** The sprite shader needs one more fragment-stage storage buffer
than Bevy's standard material. Desktop Vulkan, Metal and DX12 limits are far
above this. On Android it depends on the device's
`max_storage_buffers_per_shader_stage`: Bevy's forward path can already use
four, the wgpu baseline, so a device at the baseline could not build the
sprite pipeline. That limit is among those `docs/ANDROID_SUPPORT.md` asks to
record per test device. If such a device, GLES or WebGL2 ever became a target,
a fallback to the per-node material path would be needed, kept as production
code and used as the test reference.

The bindless form (S2, amended) needs more than the standard material does:
Bevy enables bindless for a material with a buffer binding array only where
the device offers buffer binding arrays as well as texture binding arrays,
while the standard material needs only texture binding arrays. A device with
texture but not buffer binding arrays (Bevy's source names Metal as one such
case; Android devices are to be recorded per device) therefore draws sprites
through the non-bindless fallback while the standard material stays bindless,
with the tie-order risk above. The bindless form also binds an array of storage
buffers sized by the slab's capacity, plus a small index table; on devices
without partially bound binding arrays Bevy pads the array to that size, which
counts against the storage-buffer and binding-array limits of the fragment
stage. These limits join those `docs/ANDROID_SUPPORT.md` asks to record per
test device.

**Mods and the Stock look.** Colour, fade, blend modes and UV animation are
unchanged for content, so mods and Stock see the same frames (S6). Mods may
shadow `sprite.wesl` by path (D12), and a shadowed copy of it would be out of
step with STA64's buffer binding; the binding index and buffer layout
are therefore an internal interface, not a mod-facing one, and this ADR makes
no promise about them. For sprite nodes, Refit differs from Stock in camera settings (tonemapping,
MSAA, optional HDR and bloom, handled by the shared post-lighting code), in
which groups are shown (visibility, unchanged by this ADR) and in full-screen
post passes after the sprites are drawn. The GPU test covers the camera
settings and a group toggled hidden and shown.

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

1. May two sprites at exactly the same distance from the camera draw in the
   other order than on main (one blended sprite on top of the other instead of
   under it; for additive ones a difference at rounding level)? Yes allows
   parity mode to be dropped, saving about 1 ms on the measured scene
   (forecast). Without a decision, parity mode stays on. Background: the
   original's order at exactly equal depth follows no designed rule, and for
   additive sprites the order does not change the picture (up to rounding), so
   the question matters for the alpha-blended looks. Cut-out looks are drawn in
   Bevy's binned alpha-mask phase, whose order is not affected by parity mode
   (see "Tie order"); with the bindless form (S2, amended) it was unchanged on
   the test adapter.
2. Is carrying a forked copy of a Bevy shader (`sprite.wesl`, re-synced on
   every Bevy upgrade) acceptable? If not, the choice is between the
   unchanged per-node cost, the narrower options above (which do not remove
   the cost for faded nodes), or a custom pass.
3. Is it accepted that no fallback is built now, so a device without room for
   one more fragment-stage storage buffer (see "Back ends") cannot draw sprite
   nodes until a fallback to the per-node path is added as production code?
4. Is the same mechanism to be extended to other per-frame materials
   (flipbook parts, construction sprites, `fx/quad.rs`), or kept to sprite
   nodes until the first has been measured?
5. Is the forecast enough to proceed? The implementation PR reports frame time
   and material preparation time, main against branch, from release builds with
   `perf-trace` on a nebula-heavy map, at least three interleaved runs each. If
   the frame-time gain is within the run-to-run spread (about 1 ms on the
   measured scene), the implementation is not merged and this ADR is amended
   with the measurement before anyone builds on it.
6. Are the differences the non-bindless fallback may show on a device without
   bindless support for buffer binding arrays accepted (coplanar cut-out
   sprites of different looks in another order than on main; possibly the
   one-level difference seen in step 1, whose cause is not established)?
   Nobody has run the GPU byte-equality test on such a device. Without a
   decision, the fallback stays as it is and the question stays open.

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
   reference against new path, over the scene merged in #1048 (co-located
   blended and additive pairs, coplanar cut-out overlap, fading puffs over
   cycling colours, spawn and despawn mid-series, a parent toggled hidden and
   shown). Since the amendment of S2, the test output also states whether the
   material ran in the bindless form or the fallback on the test adapter.
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
