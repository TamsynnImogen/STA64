# CON-08 — Legacy M3D layouts and model nodes

## Pre-implementation contract

This package completes the format/inspection gate in WORKPLAN, under M1 issue
#332. Every valid installed M3D must parse; an unsupported valid stream remains
a closure blocker. Invalid, misnamed or unrelated `.m3d` files must be reported
separately with contextual errors. Counts alone are insufficient evidence.

The existing 1.94–2.06 reader is retained. Version 1.93 must use its actual old
layout, not merely pass the newer version gate. Two private 1.93 streams parse
completely with the existing strict SOD reader when only their ten-byte magic
is normalized in a temporary byte buffer. Their original bytes stay untouched.
The installed Legacy reader independently accepts `Mad3D_SW`, `Storm3D_SW`
and `StarTrekDB` before the same version/layout dispatch. This supports reuse
of the existing 1.93 reader in memory; it does not prove all later layouts equal.
Identify the result as M3D and retain old-layout face winding/material meaning.

Native type 2 is a light payload: a subtype byte, RGB floats and two falloff
floats. Static reader/writer and RTTI evidence identifies subtype 2 as point,
and the other constructor as directional. Keep the authored subtype and values;
do not infer its meaning from Direct3D enum constants. Preserve this payload in
the shared Bevy-free model instead of skipping 21 bytes. Existing a2fmt already
has the matching light data structure, which can be reused without claiming
the Armada and Legacy light-class policies are identical.

Preserve type-11 controls as explicit shared LOD nodes. Retain hierarchy and
child geometry. M1 keeps the existing finest-LOD presentation policy; use typed
control identity as well as existing geometry/LOD name conventions, and test
that exactly the chosen branch remains visible. RND-07 owns distance switching,
cull/fade thresholds and render marks. Native dynamic LOD is not inferred from
successful parsing or from a plain-transform fallback.

The shared Model node enum may gain typed LOD and light variants, with narrow
constructor and exhaustive client-match changes. SOD light data may be retained
through the same representation. No Bevy dependency enters the asset crate,
and no simulation, command, content hash, save or network representation changes.
This package must not invent physical light intensities or substitute Bevy's
inverse-square falloff for Legacy's authored shader behavior. Model inspection
must state whether fields are parsed or actually consumed by the renderer.
Light playback remains an explicit rendering limitation until a faithful
adapter exists; preserving data must not be called rendered-light parity.

Extend `model_info` narrowly to report node kind, parent/children, LOD choice
and parsed light fields, retaining its existing model/material/bump report.
Add a read-only `--file <path>` mode for corpus inspection; use shared parsers,
contextual errors and stable nonzero exits. Do not copy proprietary examples
into fixtures or bypass game/mod VFS resolution for normal engine loading.

Use original synthetic binary tests for 1.93 and newer versions, hierarchy,
LOD/light preservation, truncation and invalid fields. Run the installed corpus
read-only, bind results to input hashes and refresh legacy asset/client checks,
strict Clippy and workspace/ARM64 compilation. Source/headless/build proof,
manual model appearance and original-native runtime acceptance remain distinct.
Private decompiler output and game bytes stay outside the repository.

Ownership: M3D loader/tests; narrow shared Model annotations and client enum
matches; model_info/tests; this document, WORKPLAN and an appended TODO check.
CON-09 and Sabre diagnostic PRs remain independent. No original assets, defaults,
major dependencies, input devices or native original executables are modified
or executed. The December 2026 desktop alpha target remains unchanged.

## Stage-one evidence and remaining gate

The initial changes pass 32 asset library tests, three model-info tests and six
existing texture-inspection tests. Two-crate strict all-target Clippy passes.
The new synthetic streams cover 1.93, 1.94, 1.96 and 2.06, including every
truncated prefix, typed fields, hierarchy and winding. These checks are not
native runtime or visual acceptance.

The read-only installed scan contains 2,412 paths, including duplicate installs:
606 complete models parse; 1,790 files under `M3D/chunks/` have separate native
chunk-material/geometry layouts; 16 `StarTrekDB` 1.0/1.2 files are unsupported.
All input hashes match before and after. None of the latter files is labelled
corrupt merely because it is outside the complete-model reader's layout.
The native Legacy header routine rejects versions below 1.4, so the old
database files also need separate historical-format evidence.

The native chunk readers use the same magic/version prefix, followed by
different records. They are not complete scene models. Format/role discovery,
decoding and the original corpus gate remain open: this stage does not close
CON-08 or M1. Private input identities and native evidence stay in the preserved
artifact tree outside the source checkout.
