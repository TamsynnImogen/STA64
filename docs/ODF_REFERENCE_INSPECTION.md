# Effective ODF reference inspection

EDT-10 adapts the diagnostic idea from the user's earlier STA Viewer
[`odf_check`](https://github.com/TamsynnImogen/OpenSourced-Space-RTS/blob/d01118d86b15f0f252b56278d9606312f38cb428/src/bin/odf_check.rs).
Implement it independently using STA64's existing shared loaders. No source,
schema distributions, game assets or recovered prototypes are copied.

## Pre-implementation contract

Add a headless `sta_content` example, `odf_references`, and its dedicated
tests. A `[[example]]` entry with `test = true` in sta_content/Cargo.toml makes
the existing workspace CI execute its tests; no workflow changes are needed.
No library, parser, simulation, asset loader, default, dependency or
Bevy changes. The tool does not write inputs or output files.

Usage: `odf_references <root> <odf> [--mod <name> | --loose-root]`.
`--help` alone prints usage and exits 0. Usage errors exit 2. Otherwise mount
with `Vfs::mount_install`; an unsuccessful install/mod mount is an error,
never a reason to silently inspect a different root. `--loose-root` explicitly
mounts a loose directory using `Vfs::push_dir`, for unpacked fixtures and
retail roots not recognized by the current install detector. It cannot be
combined with `--mod`. Mount/input/include errors exit 1.

Build `OdfIndex`, print its warnings, select the requested ODF by existing
index rules, and resolve it with `sta_odf::resolve_layered` and
`OdfIndex::load`. Report the selected source's winning layer and VFS path.
Inspect only effective `weapon<u32>`, `buildItem<u32>` and `ordName` values,
case insensitive. Numbered-key suffix parsing follows the existing unit
reader. Sort weapon slots numerically, then build slots numerically, then
ordName; break equal-number ties by canonical key. Preserve the effective
reference text after ordinary surrounding quote/whitespace removal. Skip
empty values; skip `NULL` only for numbered weapon/build slots, matching
their existing unit-reader gap behavior. A literal ordName NULL is still
reported. Do not confuse weaponHardPointsN or buildItemNavailability with
ODF references.

For each link report its canonical key, reference and indexed winning
target layer/path, or `missing`. Escape externally supplied text in the
report so a value containing a newline cannot manufacture a report row.
Missing links are advisory and exit 0. They are not proof of an invalid mod.
A present indexed file is reported as `indexed`, not parsed or runtime valid;
the target's own includes, class and semantics are not validated. Do not
recurse into a dependency graph or count this as a full asset/mod validator.

Existing index/version/include behavior remains authoritative, including
self-includes and same-root duplicate warnings. The tool does not recover
original assignment provenance: the source header identifies the root ODF,
not the defining file/line of every inherited key. It uses the existing
index's text decoding policy without claiming additional encoding support.

## Validation gate

Synthetic tests must cover layered self-includes, inherited and locally
overridden links, target layer/case/extension/path lookup, numeric ordering,
NULL/empty gaps, missing targets, excluded lookalike keys, missing root or
include and include cycles. Test CLI argument errors and deterministic,
escaped report rows. Verify fixture bytes remain unchanged. Run example and
content tests, strict scoped Clippy, workspace all-target and Android ARM64
library compilation where practical. A shared library compile is not an APK
or device test. The reference checkout stays clean at its pinned revision.

This optional author diagnostic adds no gameplay or Android release gate;
the December 2026 desktop alpha remains the immediate priority.

## Implemented usage and checks

From the workspace root:

```sh
cargo run -p sta_content --example odf_references -- /path/to/install ship --mod MyMod
cargo run -p sta_content --example odf_references -- /path/to/unpacked/root ship --loose-root
cargo test -p sta_content --example odf_references
```

Reports contain tab-separated `source`, `warning` and `link` rows, with escaped
field values. A successful link row may look like:

```text
link    key=weapon1    ref=MyGun    status=indexed    layer=MyMod    path=odf/weapons/mygun.odf
```

The example and manifest test opt-in are implemented. Five focused tests pass;
the content suite reports 132 passes and no failures, including these five.
Some existing installed-content tests return early without their environment
variables, so that count is not proof of installed-mod validation. Strict
content Clippy, workspace all-target checking, scoped formatting/diff checks
and an offline zero-package lock refresh pass; Cargo.lock is unchanged. The
Android ARM64 client-library check passes with NDK29/API26. No new APK or
device test was requested for this author diagnostic.

A separate real CLI sample inspected four original installed A2 ODFs:
fconst, fcruise1, fbase and fyard. It reported 37 indexed links, no missing
targets and exit 0. All 1,089 installed ODF hashes remain unchanged. Synthetic
subprocess checks verify help0, usage2, operational1 and advisory-missing0.
An actual before/after CLI probe exposed and then verified a source-name fix:
the inspector now resolves the index winner's exact filename, so a leading
space cannot redirect its values to a different file. The reference checkout
remains clean at the pinned source. These checks do not run an original game
or establish target-file validity or full campaign/mod compatibility.
