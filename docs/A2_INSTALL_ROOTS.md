# Original loose-file A2 roots (CON-29)

This contract precedes implementation. The observed original A2 folder has
`Armada2.exe`, `odf/`, `bzn/` and other conventional content directories, but
no case-insensitive Data child or root ZFS/FPQ/STA archive. Existing parsers and
`push_dir` can read it; the client's shared `mount_install` selector rejects it.

## Conservative root recognition

Preserve the existing Data-first branch, its mod chain, errors and ordering.
Preserve the flat A1 ZFS rule. Only with Data absent and no requested mod,
also recognize a root containing all of:

- a case-insensitive `Armada2.exe` **file**;
- a case-insensitive `odf` **directory**;
- a case-insensitive `bzn` **directory**.

Use the existing case-insensitive child lookup and type checks. A directory
named Armada2.exe or regular files named odf/bzn do not qualify. Partial
marker combinations and ordinary folders still report `NoDataDir`. An explicit
mod, including an empty string, must not be silently discarded. An invalid Data
child continues to report its existing error; no flat fallback can mask it.

On recognition, call the same `push_dir("root", install_root)` used for A1.
Do not change loaders, archives, recursive indexing, case/slash normalization,
profile detection, mod inheritance or loose > STA > FPQ > ZFS precedence. The
existing content profile detector still decides the effective rules after
mounting; this selector does not force a gameplay profile.

These markers recognize the observed layout, not binary authenticity or a
complete/healthy installation. No executable is read or launched to select it.
Tests use inert synthetic marker bytes, never original executable fixtures.
This is not an arbitrary-folder fallback, a generic flat-mod convention or a
claim about other uninspected A2 distributions/packed layouts.

The selector is shared Rust code for Windows, Linux and Android. It adds no
platform branch, permission, Android SAF/storage importer, dependency, Bevy
upgrade, simulation default or serialized field. Android still requires a
user-controlled supported content location; desktop mount success is not
device-storage/runtime proof.

## Validation

Synthetic tests cover the complete mixed-case marker combination, missing and
wrong-type markers, an ordinary root, explicit mods, Data priority/errors and
unchanged loose-file access. Use owned unique temporary directories; compare
directory identity rather than spelling where Data casing differs. Existing
A1 archive/mod/precedence tests remain authoritative for their paths.

Build the actual client and use only its early-exit `--mission-report` path:
the same original A2 root should change from NoData/zero maps to a mounted root
enumerating 72 maps. Source tracing confirms this path returns before App,
window, audio and input initialization. Repeat A1/FO4 controls. Hash original
inputs before/after; no data moves, rewrites, deployment or GUI/input automation.

Run VFS tests, relevant content/profile consumers, strict scoped Clippy,
workspace checking and Android checking where practical. Refresh the lock
through Cargo; do not edit it. Distinguish library checking, actual client
builds, APKs and manual runtime acceptance. The open selected-map seam #302 is
independent; do not branch from or depend on its unmerged implementation.

The December 2026 desktop alpha remains the priority. No new completion date
or release dependency is assigned.

## Executed checks

The actual baseline client reported NoData and zero maps for this root. Its
VFS source file is byte-identical to the pre-change main version; its exact
source/binary hashes are retained with the private probe receipt.

All 59 VFS library tests and three CLI tests passed, including four new tests
for complete/incomplete/wrong-type markers, case handling, loose-file access,
explicit mods and Data priority/errors. Independent source/test review found
no confirmed defect.

The 90 content library tests passed with the real A2 root and expected
`sta2classic` detection. For this profile test, leave `STA64_TEST_MOD` **unset**;
it passes a set empty value as the explicit `Some("")`, which correctly fails
flat mounting. An initial combined invocation exposed this test-harness
distinction; production behaviour was not weakened to satisfy it.

The separate `map_install` harness filters its empty mod variable into None.
Through `mount_install` it converted all 72 original A2 maps and accounted for
34,695 objects: 5,123 actors, 28,944 regions, 148 lights, 397 uncarried props and
83 skipped objects. All 35 multiplayer maps have their announced starts, 213
in total. Its 75 existing background-prop/wormhole warnings remain; no ODF was
unrecognized. This does not establish rendering or complete gameplay.

Strict VFS Clippy, workspace all-targets checking and the full Android ARM64
client-library check through cargo-ndk/NDK API 26 passed. This library check is
not an APK or device-runtime result. Original source hashes and built-client
checks are bound to the published implementation in the PR/private receipts.

The changed Linux client built and mounted the same A2 root directly: 72 maps,
36 with legacy DLL scripts needing Lua conversion and 36 without a script,
with no unreadable entries. The identical command previously reported NoData
and zero maps. A1 and FO4 control reports still enumerate 49 and 190 maps.
These early-exit reports do not start App/window/audio/input or execute the
missions. A separate private binary and receipts preserve exact source/hash
identity; prior playtest builds are untouched. All 1,239 original A2 input
hashes stayed unchanged. An offline workspace lock refresh changed zero
packages and left the lock unchanged.
