# M1 — Asset and model support

This is the acceptance ledger for the existing M1 scope in [ROADMAP](ROADMAP.md).
Implemented code, automated checks, and manual acceptance are separate gates.
The December 2026 desktop alpha remains the priority. Android shares the loaders;
host library checks alone do not establish Android rendering or APK acceptance.

## Audit before implementation

| M1 criterion | Current implementation | Remaining gate |
| --- | --- | --- |
| FPQ v2 mounted through VFS | Implemented: sta_fpq and sta_vfs | Refresh unit/installed corpus checks; preserve original inputs |
| Recursive ODF index, A1 addon, duplicate diagnostics | Implemented: sta_content/index.rs | Refresh synthetic and installed checks |
| Layer-aware model/texture preference, TGA decode | Implemented: sta_assets | Regress precedence, case handling and decoder errors |
| SOD scene, finest LOD, step animation | Implemented: a2fmt and model_scene | Refresh corpus/render checks; resolve Sabre report #330 |
| Viewer, orbit and team colors | Implemented | Updated executable and manual viewer acceptance |
| Borg variants, authored and generated mips | Implemented | Refresh asset and material checks |
| SOD bump height maps and DDS | Implemented | CPU and optional GPU checks; device support remains distinct |
| Sprites, texture flipbooks, spray/cloud emitters | Implemented | Refresh animation/emitter checks and manual appearance |
| Native M3D parsing/material references | Implemented for 1.94–2.06 | Corpus census and errors must identify unsupported inputs |
| M3D 1.93 and Legacy LOD/effect nodes | Incomplete: 1.93 rejected, type 2/11 semantics incomplete | CON-08, verified layout/behavior and tests; do not guess unknown fields |
| glTF/GLB through shared model and viewer | CON-09 implements static scenes, typed PBR materials and VFS resources | CPU/material and offscreen GPU fixtures pass; full regressions/publication and manual viewer acceptance remain |

The read-only audit found private M3D 1.93 examples, so that row cannot be
resolved by removing the version from the requirements. Unsupported effects
must not be silently described as implemented. Private game files and recovered
binary excerpts do not become public fixtures.

## Work sequence and ownership

1. CON-09: shared glTF/GLB model loading and client integration, under the
   contract below. Parser implementation may be delegated; the lead owns model
   annotations, coordinate conventions, materials and integration.
2. CON-08: native-backed M3D layout/node completion, with a separate pre-code
   contract and narrowly scoped PR. Compatibility decisions remain with the lead.
3. Resolve #330 against the actual Sabre (`fdestroy2`) runtime evidence. Its
   isolated asset and alpha checks pass; that is not a verified gameplay fix.
4. Refresh the M1 corpus/check matrix, stage an exact executable and preserve
   manual checks. Tick the roadmap only for criteria actually accepted.

No simulation, economy, scripting, network or save format changes are authorized
by these asset packages. Existing branches, assets and visual executables remain
preserved. The user merges; agents do not push main or self-merge.

## CON-09 pre-implementation contract

Use gltf 1.4.1, already present through Bevy 0.20.1, with its parsing/utils API.
Resolve buffers and images through STA64's VFS; do not use its filesystem import
helper. Reuse the installed dependency versions rather than upgrade Bevy.
Primary references: [gltf 1.4.1 API](https://docs.rs/gltf/1.4.1/gltf/) and the
[glTF 2.0 specification](https://github.com/KhronosGroup/glTF/tree/main/specification/2.0).

Implement static glTF 2.0/GLB scenes: selected/default scene, indexed or
nonindexed triangle primitives, triangle strips/fans converted explicitly,
node transforms/hierarchy, normals/tangents/colors, UV0/UV1, material factors,
base color, normal, metallic/roughness, emissive and occlusion textures. Preserve
opaque/mask/blend, double-sidedness and sampler behavior. Support external VFS
resources, GLB buffer views, and base64 data URIs, including PNG/JPEG images.

The shared Model remains Bevy-free. Add one default-empty optional PBR annotation
table for native materials, images and vertex attributes, plus a source-format
variant. Existing SOD/M3D values receive no PBR annotations and keep their
existing renderer behavior. This is an asset-model change, not a simulation seam
or serialized command change. Update the few synthetic Model literals explicitly.

glTF is right-handed. Convert it once into Model's left-handed positions and
Matrix34 convention; the existing client conversion returns it to Bevy's
right-handed space. Test handedness, parent transforms, normals and winding.
Static native glTF materials use StandardMaterial PBR, not the legacy hull look.
glTF node names are descriptive: legacy name-based LOD, Borg and damage hiding
does not apply to them. A renderer-only native-node marker and a narrow filter
in units::show_damage enforce this without changing legacy nodes or simulation.
Loading models must not rewrite files or generate proprietary texture caches.

Reject malformed lengths/offsets, indices, cycles, multiple parents, invalid
numbers and incompatible attributes with contextual errors rather than panics.
Normalize relative resource URIs within the VFS root, including legitimate `..`
references and percent escapes. Reject absolute, network/file-scheme, NUL or
root-escaping resource paths. Preserve layered lookup and case semantics.

Skins, morph targets, animation playback, compressed geometry and unsupported
required extensions are outside this static-model slice and receive explicit
unsupported errors, never silent flattening. Optional extensions may use their
defined core fallback only with a loader warning. This limitation must remain
visible in support docs; static-model completion is not full glTF-spec coverage.

Validation uses original synthetic fixtures only: glTF and GLB geometry,
external/inline/image buffer-view resources, malformed accessors/URIs/hierarchy,
materials and round-trip coordinate expectations. A windowless GPU fixture must
load through the VFS and draw real geometry/textures; library compilation alone
does not establish rendering. Existing asset/client tests, scoped strict Clippy,
workspace/all-target checks and unchanged-original receipts remain required.

The package owns the new gltf loader/tests and PBR model definitions, narrow
sta_assets dispatch/Model constructor changes, necessary dependency lines,
renderer texture/material integration and viewer dispatch, plus owning docs.
It must not alter legacy default values or take over simulation/content seams.

## Static glTF/GLB usage and limits

Place a model under a mounted content layer's `sod/` directory, for example
`sod/fx_cruiser.glb`, and select it by basename with `--view fx_cruiser`.
GLB/glTF retain the existing higher-layer-before-format lookup rules. Relative
image/buffer URIs resolve within that VFS, including its normal case handling;
they do not grant direct filesystem or network access.

The original reproducible model in `fixtures/models/` exercises this path.
Its generator and provenance are documented alongside it. Native PNG/JPEG
images are decoded to RGBA8 for GPU upload; color maps use sRGB and material
data/normal/occlusion maps use linear formats. Normal scale and occlusion
strength are applied in memory, and mipmaps are generated without rewriting
source assets. Sampler wrap/filter and UV0/UV1 selections are preserved.
This does not add KTX2/Basis or compressed glTF texture decoding.

Supported static scenes are distinct from the explicitly rejected animation,
skin, morph, point/line and unsupported-required-extension cases above.
Optional unsupported extensions report the core fallback in model warnings.
`model_textures` reports native material bindings without claiming full image
decoding or rendered parity. Automated host drawing does not establish Android
device acceptance or completion of the remaining M3D and Sabre gates.

### CON-09 local verification (2026-11-11)

- Asset crate: 35 library tests and seven inspection-example tests pass.
- Client: 343 library tests pass, one existing ignored test. GPU tests run
  separately from this default suite.
- Original GLB fixture: VFS basename lookup, CPU PBR/vertex/sampler checks and
  native-name visibility regression pass. A real Vulkan offscreen draw on the
  local NVIDIA adapter shows both red and blue checker pixels.
- Existing rock/planet/nebula GPU pipelines pass in an isolated process.
  An initial combined GPU run encountered global logger contention and a
  timing-sensitive HUD message assertion; the complete CPU suite and isolated
  pipeline test pass. Their initial logs are retained, not erased.
- Strict all-target Clippy for `sta_assets`/`sta64`, workspace all-target
  compilation and API-26 Android ARM64 client-library compilation pass.
- Offline workspace lock refresh retains all existing dependency versions;
  JPEG enables two decoder packages. Rust 1.97.1 and Bevy 0.20.1 stay pinned.

These are source/build/headless checks. Viewer interaction, original-game
appearance and Android device rendering remain separate acceptance steps.
