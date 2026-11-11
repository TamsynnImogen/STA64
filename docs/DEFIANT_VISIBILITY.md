# Classic ship visibility investigation

Issue [#330](https://github.com/TamsynnImogen/STA64/issues/330) concerns the
Federation **Sabre (`fdestroy2`)** in Classic. The screenshot identifies FDESTROY2;
the user subsequently confirmed the Defiant (`fdestroy`) is fine. The Sabre hull
is invisible after construction and movement, while the selection ring remains
visible. The screenshot was taken using the older ac1b409 visual build.

## Confirmed checks

The installed definitions map `fdestroy2` to Sabre and `fdestroy` to Defiant,
both at scale 0.8. The retained Sabre LOD contains 418 triangles; Defiant has 533.
Both ships use opaque face groups. Their main hull lighting material enables
self-illumination, while the team-color group does not.

Their diffuse TGAs resolve through the Classic VFS layer and decode, with alpha
ranging from 0 to 255 and predominantly zero. Those bytes are preserved. Alpha
controls self-illumination of the main hull; neither group enables texture-alpha
cut-out or blended opacity. Forcing texture alpha to 255 would change lighting
data and is not a fix.

Direct model spawning, unit attachment followed by `show_damage`, and windowless
Vulkan rendering pass separately for both ships. Sabre draws 12,657 lit pixels
in fresh-hull, PBR and restored-hull captures; Defiant draws 13,315 in hull captures
and 13,328 with PBR. These checks establish isolated asset/material behavior, not
successful visibility in a running skirmish. The Sabre's live failure remains
unconfirmed. Original game files are unchanged.

## Opt-in local probes

These tests require privately installed Classic assets; they do not bundle them.
The separate opt-in avoids imposing a Classic-only fixture on other install tests.
`STA64_TEST_MODEL=fdestroy2` selects Sabre; its default `fdestroy` retains the
earlier Defiant control.
Use the repository's low-priority, two-job Cargo workflow:

```sh
STA64_DEFIANT_TESTS=1 STA64_TEST_MODEL=fdestroy2 \
STA64_TEST_INSTALL=/path/to/install STA64_TEST_MOD='STA2 Classic' \
STA64_GPU_TESTS=1 STA64_GPU_OUT=/path/to/output \
cargo test -p sta64 --lib --jobs 2 defiant -- --nocapture --test-threads=1
```

Without the GPU flag, the CPU probes still run. With it, the GPU test captures
fresh hull shading, PBR, and hull shading after toggling back, using an offscreen
image and no window. A real adapter and Vulkan setup are still required.

## Runtime evidence

Set `STA64_TRACE_MODEL=fdestroy2` when launching a diagnostic build. It records
three scene snapshots per matching unit root, tracking up to 16 live roots at a
time (including map-placed units): their view state,
model assignment, pending attachment, material presence, alpha mode, texture
presence, shader choice, world transforms and visibility along each part's parent
chain. It runs after render preparation and does not change gameplay or materials.
Logs identify snapshots with `unit-visual`. This trace is off unless requested.

Missing unit models now produce a warning identifying the actor, ODF and model.
`RUST_LOG=info,sta64::render::units=debug` also records attachment mesh/triangle
counts and missing textures. The diagnostic launcher enables both automatically.

The next manual check is ordinary construction of one Sabre in Classic,
followed by keeping its log. No visual fix or native renderer parity is claimed,
and #330 remains open pending diagnosis and manual acceptance.
