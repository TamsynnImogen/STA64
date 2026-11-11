# Read-only model texture inspection

## Pre-implementation contract (EDT-11)

The user's [STA Viewer](https://github.com/TamsynnImogen/OpenSourced-Space-RTS)
at `d01118d86b15f0f252b56278d9606312f38cb428` exposes model materials and
their texture references in `src/features/details_panel.rs`. This is a useful
diagnostic idea for STA64, independently implemented over its existing shared
loaders. No reference code, proprietary assets, guide text or recovered
prototypes are copied. No root licence file exists in the pinned reference.

## Wider reference assessment

The pinned repository is primarily a viewer and asset-authoring prototype,
not a second simulation implementation. Its useful contributions are tooling,
format investigation and inspector design. Existing adaptations and caveats
are recorded in [the reference audit](REFERENCE_SPACE_RTS.md).

| Area | Decision for STA64 |
|---|---|
| ODF duplicates, includes and asset links | Duplicate diagnostics already implemented; effective ODF link inspection is independently proposed in PR #314. Keep STA64's shared layered resolver. |
| A1 maps | Header investigation already led to strict A1 parsing and typed import; selected-map/start integration is pending in PR #302. Retain original-game validation rather than heuristic scans or template copying. |
| Model materials | Implement this explicit six-role texture report using shared SOD/M3D loaders and VFS lookup. |
| Hierarchy inspector | `SceneHierarchy`, `rebuild_scene_hierarchy` and `draw_hierarchy_node` in reference `src/features/details_panel.rs` offer a future selectable-node inspector. No hierarchy UI is added here; visibility/render behaviour needs separate acceptance. |
| Camera orientation | Reference `src/features/navi_cube.rs` offers face/edge/corner snapping. STA64 already has orbit/pan/zoom and EDT-01 owns planned canonical views; preserve that package rather than duplicate it. |
| OBJ/MTL export and MS3D/MAX import | Authoring candidates requiring independent axis/winding, hierarchy, material and animation tests. Not prerequisites for the desktop alpha. |
| FPQ, mod loading and runtime rules | Existing STA64 shared parsers and mod-chain lookup are richer. Keep them; the prototype supplies no authoritative simulation, Android or networking policy. |

Do not migrate its Bevy 0.13 plugins into STA64's Bevy 0.20.1 application, or
copy generated schema defaults as universal A1/A2/FO rules. Profile defaults
still need version-specific evidence and overrides in their owning data files.

## Diagnostic contract

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
model-loader warnings. For each mesh node, group and present map, in shared
model order, report node/group indices and names, role and reference as exposed
by that loader. Roles are diffuse, height, bump, specular, glow and detail.
Preserve empty or repeated references present in `Model`; each occurrence
belongs to its own group. Report the existing `find_texture` result: virtual
path, format, layer index/name, or missing. Escape strings so embedded newlines
cannot forge rows.

This is not a raw-file preservation report. The existing M3D loader normalizes
texture paths/extensions, drops empty texture slots, adapts some detail/glow
roles, synthesizes/renames hierarchy nodes and orders parents before children.
Its report therefore reflects those existing transformations; raw spelling,
empty M3D slots and original M3D node indices cannot be recovered from `Model`.
This diagnostic does not alter those shared-loader policies.

CON-09 adds static glTF/GLB reporting through the same loader. Native PBR groups
report material factors and five typed texture bindings: base color, normal,
metallic/roughness, emissive and occlusion. Images report their virtual or inline
origin, encoding, byte count, UV set and sampler. Encoded image bytes are not
dumped; this is still not a decoder or rendered-parity check. Original legacy
six-role output and exit meanings remain unchanged.

Use existing lookup semantics without changing them: a higher-priority layer
wins before format preference, names are case-insensitive, and the shared
basename/extension lookup remains authoritative. Do not add direct filesystem
lookups, fallback images or independent archive parsers. A found path establishes
lookup only; it does not establish successful texture decoding, GPU support or
rendered material parity. An empty map present in `Model` is missing. No maps is a valid
empty report. Advisory reports exit 0; `--deny-missing` exits 1 for any missing
explicit map, while errors retain exit 2 precedence.

The report does not decode textures, modify inputs, recurse through ODFs, export
models, flatten includes, infer runtime bump/Borg/texture-animation fallbacks,
or validate complete glTF specification coverage. The shared static glTF/GLB
loader performs its own geometry/resource checks before the report is built.
This is a diagnostic of explicit shared model references, separate from a complete asset
dependency graph or graphical editor integration. Simulation, defaults, hashes,
archives, save/replay/network formats and Bevy remain unchanged.

Before publishing, verify synthetic six-role/model-order/missing controls,
mixed-case and format lookup, higher-layer precedence, empty/no-map cases,
safe output and argument/error handling. Check the scoped example plus existing
asset consumers, strict Clippy and workspace all-target compilation. Optional
installed-model runs must preserve input hashes and remain clearly distinct
from native runtime or Android acceptance. No APK is required for this host
diagnostic. The December 2026 desktop alpha remains the priority.

## Implemented checks

The example and six synthetic tests are implemented. They cover all six roles,
per-group occurrences, empty/no-map cases, escaped output, mixed-case lookup,
child/parent/base priority, within-layer format preference, argument parsing
and missing/unsupported/corrupt model errors. The registered example runs in
the existing core CI test command; no new workflow or dependency is needed.

Local asset/format suites reported 147 passes, no failures and two ignored
tests, including the six example tests. Some installed-content tests return
early without their environment configured; that count is not full installed
content validation. Scoped strict all-target Clippy, workspace all-target
compilation, formatting/diff checks and an unchanged offline lock refresh
passed.

A separate headless process check exercised 12 CLI controls, including actual
0/1/2 exit codes, a synthetic valid SOD, a found but deliberately undecodable
texture, and a non-UTF-8 argument error. Four read-only local model samples
covered A1, A2, Fleet Operations and M3D. The first three resolved their
28/28/8 explicit references respectively; the M3D sample reported four missing
references with advisory exit 0. All four selected model hashes stayed equal.
Private logs and input identities are retained outside the source tree; no
game models are distributed as fixtures. These checks establish inspection
behaviour only, with no rendering, Android runtime or complete-mod claim.
