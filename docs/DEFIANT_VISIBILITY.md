# Classic Defiant visibility investigation

Issue [#330](https://github.com/TamsynnImogen/STA64/issues/330) records a user
report from the Linux visual build at ac1b409 (Rust source matches merged main
0d979bf): the Federation Defiant finishes construction but its hull is invisible.
Its selection ring remains visible, including after ordering it away from the yard.
Other ship classes and close-zoom behavior have not been systematically checked.

## Confirmed checks

The installed Classic `fdestroy` definition resolves its own model and a scale of
0.8. Its SOD contains two LODs; the retained LOD contains 533 triangles. Both
face groups use `opaque`. The main hull lighting material enables
self-illumination; the small team-color group does not.

The referenced diffuse TGA resolves through the mod's VFS layer and decodes.
Its alpha ranges from 0 to 255 and is predominantly zero. The renderer preserves
those bytes. On this asset, alpha controls self-illumination of the main hull;
neither group enables texture-alpha cut-out or blended opacity. Changing the
texture to force alpha to 255 would change its lighting data and is not a fix.

The direct model spawn and the unit attachment path (`attach_models` followed by
`show_damage`) produce textured opaque hull parts with no hidden hull ancestor.
A windowless Vulkan render on the local NVIDIA adapter draws the real installed
Defiant under both Bevy PBR and the hull shader. These checks establish the
isolated asset/material path, not successful visibility in a running skirmish.
The live failure's cause remains unconfirmed. Original game files are unchanged.

## Opt-in local probes

These tests require privately installed Classic assets; they do not bundle them.
The separate opt-in avoids imposing a Classic-only fixture on other install tests.
Use the repository's low-priority, two-job Cargo workflow:

```sh
STA64_DEFIANT_TESTS=1 \
STA64_TEST_INSTALL=/path/to/install STA64_TEST_MOD='STA2 Classic' \
STA64_GPU_TESTS=1 STA64_GPU_OUT=/path/to/output \
cargo test -p sta64 --lib --jobs 2 defiant -- --nocapture --test-threads=1
```

Without the GPU flag, the CPU probes still run. With it, the GPU test captures
fresh hull shading, PBR, and hull shading after toggling back, using an offscreen
image and no window. A real adapter and Vulkan setup are still required.

## Runtime evidence

Set `STA64_TRACE_MODEL=fdestroy` when launching a diagnostic build. It records
three scene snapshots per matching unit root, tracking up to 16 live roots at a
time (including map-placed units): their view state,
model assignment, pending attachment, material presence, alpha mode, texture
presence, shader choice, world transforms and visibility along each part's parent
chain. It runs after render preparation and does not change gameplay or materials.
Logs identify snapshots with `unit-visual`. This trace is off unless requested.

Missing unit models now produce a warning identifying the actor, ODF and model.
`RUST_LOG=info,sta64::render::units=debug` also records attachment mesh/triangle
counts and missing textures. The diagnostic launcher enables both automatically.

The next manual check is ordinary construction of one Defiant in Classic,
followed by keeping its log. No visual fix or native renderer parity is claimed,
and #330 remains open pending diagnosis and manual acceptance.
