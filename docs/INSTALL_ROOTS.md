# Install roots (CON-27)

This contract precedes implementation. The client and tools use the same
`Vfs::mount_install` entry point. Extend its root selection to original flat A1
installs; reuse `push_dir` and all existing archive/index/mod implementations.
No new loading path, filesystem writes, platform conditionals or dependencies.

## Root-selection rules

1. A case-insensitive `Data` child remains the first choice. Use the existing
   active-mod/ParentMod chain and base Data layer without changes. Root-level
   archives or loose files outside Data cannot override this choice. An invalid
   Data path must report its existing error, not fall back to the parent root.
2. Only when Data is absent and **no mod was requested**, recognize an original
   flat A1 install if at least one root-level `.zfs` **file** exists. Extension
   matching is ASCII case-insensitive. Mount that folder as the one `root`
   layer using `push_dir`, exactly as existing bare-folder diagnostic tests do.
3. Ordinary/missing roots, nested-only ZFS files and directories named `.zfs`
   remain `NoDataDir`. Do not turn arbitrary selected folders into installs.
   A selected mod on a flat root still returns `NoDataDir`; never silently
   ignore a requested mod or invent a flat mod-chain convention.
4. Recognized roots use the existing decoders and return their actual archive
   or I/O errors. A corrupt ZFS is not an empty successful mount or a reason
   to try another layout. Archive precedence and path normalization stay
   unchanged: loose > STA > FPQ > ZFS within each layer.

Existing original-A1 profile detection already recognizes a ZFS-backed layer.
Keep it unchanged. There is no Android-specific gameplay/storage shortcut;
the same VFS API is shared by Windows, Linux and Android. The Android packaged
Data layout remains supported. This does not add unrestricted-storage access,
SAF import, mod installation or a new permission.

## Validation

Use the existing synthetic ZFS builder in `zfs::tests`; no native game bytes
in fixtures. New tests own unique temporary directories and remove only those
they successfully created. Check:

- A flat root with case-mixed ZFS/paths reads archived ODF, tech tree and root
  data alongside a loose BZN; a loose ODF override still wins.
- Data wins even when a valid flat archive is present; case-mixed Data and
  existing active/parent mods retain their order and errors.
- Missing/ordinary roots, nested-only archives, `.zfs` directories, an explicit
  flat-root mod and corrupt recognized archives refuse as specified.
- A Data child that is a regular file reports its existing I/O error rather
  than exposing files from the flat parent.

Run scoped VFS tests and strict Clippy, relevant content/profile consumers,
workspace all-targets checking and an Android ARM64 library compile where
available. Build the actual client and run its `--mission-report` path against
the user-owned flat A1 install. Source tracing establishes that this path
mounts/resolves data and returns before `App::new`, window/audio/input setup;
do not start GUI/input automation. A VFS-only test is not a client smoke.
Hash original source maps/archives before and after. A library compile is not
an APK or device/runtime result. GUI and Waydroid acceptance remain manual.

## Manual checks after integration

On a build containing CON-26's A1 binary body reader and this mounting change:

```sh
sta64 --install-root "/path/to/Star Trek Armada" --mission-report
sta64 --install-root "/path/to/Star Trek Armada" --map 2blue
```

The first command should report the root as mounted and enumerate the A1 maps.
The second is a **manual** map-view check for placed actors, terrain and
lighting; it is not automated runtime proof. Multiplayer starts still require
MDF/precise Camera_i path integration. Background props/wormholes and other
known importer/converter limits are recorded in [A1 support](A1_BZN_SUPPORT.md).
Do not move, rename, normalize or rewrite the original game data.

Lead owns layout decisions, the narrow mount selection/module addition, docs
and integration. A bounded worker may own only `src/flat_install_tests.rs`;
independent source review follows the frozen implementation. December 2026's
desktop alpha stays the immediate priority; no completion date or new release
dependency is assigned.
