# Read-only model texture inspection

## Pre-implementation contract (EDT-11)

The user's [STA Viewer](https://github.com/TamsynnImogen/OpenSourced-Space-RTS)
at `d01118d86b15f0f252b56278d9606312f38cb428` exposes model materials and
their texture references in `src/features/details_panel.rs`. This is a useful
diagnostic idea for STA64, independently implemented over its existing shared
loaders. No reference code, proprietary assets, guide text or recovered
prototypes are copied. No root licence file exists in the pinned reference.

Add a headless `model_textures` example in `sta_assets`, without dependencies
or public API changes. Invocation:

```sh
cargo run -p sta_assets --example model_textures -- [--deny-missing] <install> <model> [mod]
cargo run -p sta_assets --example model_textures -- --root <asset-root> <model>
```

The first form uses `Vfs::mount_install`, including the existing mod-parent
chain and archive precedence. `--root` explicitly mounts a single asset tree
through `Vfs::push_dir`; it does not guess an installation or parent chain.
This also permits inspecting loose A2 trees before independent PR #304 merges.
`--help` exits 0; `--` terminates options for option-like paths. Invalid options
or argument counts, mount failures, missing models, unsupported model formats
and model-load failures exit 2 with a clear diagnostic, never a panic.

Find and load the selected model through `find_model` and `load_model`. Report
model-loader warnings. For each mesh node, group and explicitly present map,
in source order, report node/group indices and names, role and original texture
reference. Roles are diffuse, height, bump, specular, glow and detail. Preserve
empty or repeated authored references; each occurrence belongs to its own
group. Report the existing `find_texture` result: virtual path, format, layer
index/name, or missing. Escape strings so embedded newlines cannot forge rows.

Use existing lookup semantics without changing them: a higher-priority layer
wins before format preference, names are case-insensitive, and the shared
basename/extension lookup remains authoritative. Do not add direct filesystem
lookups, fallback images or independent archive parsers. A found path establishes
lookup only; it does not establish successful texture decoding, GPU support or
rendered material parity. An explicitly empty map is missing. No maps is a valid
empty report. Advisory reports exit 0; `--deny-missing` exits 1 for any missing
explicit map, while errors retain exit 2 precedence.

The report does not decode textures, modify inputs, recurse through ODFs, export
models, flatten includes, infer runtime bump/Borg/texture-animation fallbacks,
or validate glTF/GLB (the shared model loader currently rejects them). It is a
diagnostic of explicit SOD/M3D model references, separate from a complete asset
dependency graph or graphical editor integration. Simulation, defaults, hashes,
archives, save/replay/network formats and Bevy remain unchanged.

Before publishing, verify synthetic six-role/source-order/missing controls,
mixed-case and format lookup, higher-layer precedence, empty/no-map cases,
safe output and argument/error handling. Check the scoped example plus existing
asset consumers, strict Clippy and workspace all-target compilation. Optional
installed-model runs must preserve input hashes and remain clearly distinct
from native runtime or Android acceptance. No APK is required for this host
diagnostic. The December 2026 desktop alpha remains the priority.
