# M1 asset validation — 2026-11-11

This report supplements the [M1 acceptance ledger](M1_ASSET_MODEL_COMPLETION.md).
It distinguishes installed-content checks, offscreen rendering and user acceptance.
The December 2026 desktop alpha target is unchanged. No original assets, gameplay
rules, dependency versions, simulation or serialization formats change here.

## Installed content

The same VFS, ODF index and shared model loaders serve every profile below.
`STA64_TEST_INSTALL` selects the root. `STA64_TEST_MOD` is optional: absent or empty
selects the base install or a recognized original flat A1/A2 root; a nonempty value
selects that mod and its parent chain. This changes the installed test harness,
not production mounting. Each ODF test requires a nonempty enumeration.

| Mounted content | Indexed ODF names | Resolved | Missing include | Include cycles | Same-root duplicates |
| --- | ---: | ---: | ---: | ---: | ---: |
| Roots base | 1,696 | 1,695 | 1 | 0 | 0 |
| Fleet Ops 4.0 | 10,663 | 10,663 | 0 | 0 | 0 |
| Armada II Classic | 1,097 | 1,097 | 0 | 0 | 0 |
| Original GOG Armada I | 489 | 489 | 0 | 0 | 0 |

The remaining base-only reference is `bor_effectss.odf -> allR_technology.obf`.
No include or replacement file is invented. Synthetic index tests also cover
recursive folders, A1 `addon/` priority, duplicates, parent overrides and self-includes.
The A1 VFS check actually reads all 489 ODFs from ZFS. Empty ODF contents are legal:
FO4 contains one empty `basic_rotate_combat.odf`; readability is checked without
rejecting that authored file.

| Mounted content | Models loaded | Explicit model refusals | Resolved texture references | Distinct missing texture references | DDS decoded / errors |
| --- | ---: | ---: | ---: | ---: | ---: |
| Roots base | 617 | 1 old SOD | 1,936 | 38 | 449 / 0 |
| Fleet Ops 4.0 | 926 | 1 old SOD | 3,837 | 9 | 604 / 0 |
| Armada II Classic | 456 | 11 old SOD + 8 required-extension glTF | 1,946 | 38 | 0 / 0 |
| Original GOG Armada I | 255 | 12 old SOD | 2,063 | 92 | 0 / 0 |

The model corpus test reports refusals and missing references rather than requiring
zero. A green test alone therefore does **not** mean every installed file loaded.
Every failed model is now printed, including failures beyond the previous first ten.
Missing texture counts establish lookup gaps, not their native runtime significance;
no placeholder texture or source rewrite is introduced.

Roots/FO4's refused file is `wshldinv_old.sod` (1.2). The A1 refusals are
`mcomet`, `mgalaxy`, `mgate`, `mplasma`, `mstars`, `wcobdet`, `wmastran`, `wprobe`,
`wshldinv`, `wtetrion`, `wuitrbm` (1.2), and `wviracod` (1.0). Classic has that set
except `wshldinv`. These retain the existing explicit pre-1.4 SOD refusal, also
accounted separately in the original Armada II corpus test; this report does not
extend native compatibility claims to those obsolete streams.

The eight inherited glTF files are `fbonchune`, `ffargo`, `ffarragut`, `fgalaxyx`,
`fincurs`, `fmelb`, `fnova`, and `fphoenix`. They require `KHR_texture_basisu`, outside
CON-09's PNG/JPEG static slice. They are not silently flattened or substituted.
One inspected export's KTX2 payload (`fabonchune`) stores RGBA8 sRGB with Zstandard,
despite its glTF declaring Basis Universal. The [Khronos extension specification](https://github.com/KhronosGroup/glTF/blob/main/extensions/2.0/Khronos/KHR_texture_basisu/README.md)
requires ETC1S/BasisLZ or UASTC, so that sample also needs exporter investigation.
Its generator string is not proof that the absent converter source is correct.
The files remain unchanged. Basis texture support is not claimed by M1's static slice.

## Archives, textures and animation data

- Roots' FPQ v2: all 1,698 entries read, hash-lookup and in-memory repack checks pass;
  1,696 ODFs parse. Its archive-only include check reports one unresolved reference.
- Sigma's FPQ v2: all 2,872 entries read, hash-lookup and in-memory repack checks pass;
  all 2,872 ODFs parse. Its archive-only include check reports five unresolved references.
  This is not a full mounted Sigma mod audit.
- Original Armada II: 447 supported SOD files parse, with 12 old-version refusals
  counted separately; all 2,116 TGA files decode; 723 authored mip links validate.
  All eight installed stock-corpus tests pass.
- Original Armada II sprite data: 32 tables, 2,388 sprites, 225 sprite nodes,
  127 animations and 32 emitters parse. SOD references retain seven unresolved
  sprite names and four unresolved flipbook names; none are fabricated.
- Mounted SPR tables load with zero missing includes: Roots has 286 animations /
  50 emitters; FO4 342 / 59; Classic 329 / 59; A1 127 / 28. Parser warning counts
  are respectively 9, 8, 8 and 6; those warnings are not suppressed.

The scoped parser/asset/VFS/ODF suites and seven ODF-index tests pass. Client
regressions cover model transforms, height-to-normal conversion, team materials,
native PBR image factors/samplers and deterministic spray/cloud animation.
These checks establish data/translation behavior, not every effect's visual fidelity.

## New model support and live rendering

CON-09 (#334, merged) loads static glTF/GLB through the VFS and shared model,
including PNG/JPEG PBR resources. CPU/material checks and a real checker-texture
Vulkan offscreen draw pass. The reproducible public fixture lives under
`fixtures/models/`. Advanced animation/skins/morph/compression and unsupported
required extensions retain explicit errors. Manual orbit of this new GLB fixture
has been offered; the user's latest response is **Not tested yet**.

CON-08 (#336, ready for user merge) accounts for 2,412 private M3D paths:
606 complete scenes, 594 material libraries and 1,196 geometry variants decode;
16 historical 1.0/1.2 streams retain explicit native-version refusals. Twelve
geometry files preserve non-finite UV attributes with warnings. Four declared
variant paths are absent from the original content. See that package's
`M3D_NODE_SUPPORT.md` and private corpus receipt for the evidence and limits.
Typed light/LOD data and inspection are distinct from native light-shader,
destruction playback and distance-LOD rendering work.

The Sabre's depth-prepass fault is reproduced and fixed in #337, with matching
single-sided culling in forward/depth passes. The user confirmed **Hull stays
visible** after construction and movement in Classic using source 31d20cd's
staged executable. The [Sabre report](DEFIANT_VISIBILITY.md) records its hash,
before/after GPU counts and unchanged input identities. Issue #330 is closed.

The Sabre branch passes 346 client tests (one ignored), strict client all-target
Clippy, workspace all-target compilation, Linux executable build and NDK29/API26
Android ARM64 client-library compilation. Isolated Vulkan GPU regressions pass.
This is not a new APK build or an Android/Waydroid device run.

## Reproducing and preserving evidence

Use the repository's low-priority, two-job Cargo workflow. For example:

```sh
export STA64_TEST_INSTALL='/path/to/install'
# Optional: export STA64_TEST_MOD='Fleet Ops 4.0'
cargo test -p sta_content --test odf_index_install --jobs 2 -- --nocapture
cargo test -p sta_odf --test install --jobs 2 -- --nocapture
cargo test -p sta_vfs --lib tests::real_install_if_available --jobs 2 -- --exact --nocapture
cargo test -p sta_assets --lib real_install_if_available --jobs 2 -- --nocapture
```

Use the exact VFS test selector: the separate mod-list/order fixtures require a
Data/Mods layout and do not apply to a flat A1 installation. FPQ checks use
`STA64_TEST_FPQ` for `sta_fpq::real_archive_if_available`, and `A2_FO_ODF_FPQ` with
`a2fmt --test fpq real_odf_fpq_decompresses_and_parses -- --ignored --nocapture`.
The original A2 corpus uses `A2_GAME` with `a2fmt --test stock`.

Private logs and receipts are preserved under `STA64-artifacts/probes/M1-refresh/`.
30,739 input hashes (5,276,614,069 bytes) match after the audit. The Classic parent
layer's additional receipt was taken before its final rerun. Earlier logs include
the too-strict empty-ODF assertion, an overly broad A1 test selector and an
intermediate test compilation error; corrected exact reruns pass. They remain
available rather than being replaced by the successful results.

M1 #332 remains open for integration of the ready packages and the new GLB viewer
acceptance. No device rendering, missing content or unsupported extension is
marked complete merely because a host library compiles. The low-FPS report also
needs comparison with an optimized executable: the diagnostic build overrides
dependency optimization to zero, unlike the repository's normal level two.
