# Android support

Android ARM64 is an additional native STA64 platform. It uses the same game,
simulation, content, missions and command pipeline as desktop. The December
2026 Windows/Linux alpha remains the immediate priority; Android work is
opt-in and must not gate that milestone.

## Audit baseline

The preparation audit covers all 16 workspace crates, the client modules,
manifests, build script, packaging, tools and existing CI at main `01aae79`.
The pinned toolchain is Rust **1.97.1**, edition 2024, and Bevy **0.20.1**.
There is no reason established by this audit to upgrade either.

See [the compatibility matrix](ANDROID_COMPATIBILITY.md) for implemented,
partial and planned functionality, and [the roadmap](ANDROID_ROADMAP.md) for
follow-up acceptance gates. Source portability is not an Android build result.

## Proposed narrow platform seam

This proposal precedes the implementation in TOOL-10 / issue #250.

1. Move the current client entry implementation into the same package's Rust
   library without changing the gameplay modules. Keep a small desktop binary
   wrapper with the existing CLI. Android calls that same launch function from
   Bevy's `#[bevy_main]` entry.
2. Keep the desktop library an ordinary Rust library. Request `cdylib` only in
   the Android build command, avoiding an extra desktop shared-library release.
   Enable Bevy's `android-native-activity` feature only for the Android target.
3. Resolve Android content and writable cache locations from the activity's
   app-private data directory. Preserve desktop install-root/CLI/cache behavior.
   Reuse the existing filesystem VFS for app-owned staged content. A future
   Storage Access Framework importer can copy user-selected content there;
   `content://` URIs must not be treated as ordinary paths.
4. Gate existing input and simulation scheduling on Android lifecycle events.
   Skip the first resumed frame's wall-time delta so background time does not
   become simulation catch-up. The actual client clock reads `Time<Real>`;
   pausing Bevy virtual time alone would be insufficient. Preserve any existing
   game pause state. Surface recreation remains Bevy/winit's responsibility.
5. Package the shared native library in a small NativeActivity APK. No legacy
   game data, alternate gameplay code or unrestricted storage permission is
   included. Keep Android automation separate from mandatory desktop CI.

Touch gestures, safe-area layouts, Android audio-focus integration and SAF
installation are subsequent boundaries, not grounds for duplicating gameplay.
The initial APK must report these limitations rather than claim playability.
OS termination cannot guarantee a final save callback: mission VM persistence
is currently incomplete on desktop too, so resume-after-process-death remains
a shared save-system requirement.

## Toolchain and build contract

The version-matched [Bevy 0.20.1 Android instructions](https://github.com/bevyengine/bevy/blob/v0.20.1/examples/README.md#android)
use cargo-ndk plus Gradle and describe NativeActivity support. The initial
configuration targets `aarch64-linux-android` / `arm64-v8a`, minimum API 26,
compile/target SDK 36, NDK 29.0.14206865, JDK 21, Gradle 8.13 and Android Gradle
Plugin 8.13.2. Android tooling versions are isolated from Rust/game dependencies.
AGP's [compatibility table](https://developer.android.com/build/releases/agp-8-13-0-release-notes)
supports that SDK/Gradle combination. The Android build will request 16 KiB ELF
segment alignment and verify the resulting native artifact.

Intended commands (scripts will be added and verified before being advertised
as working):

```sh
rustup target add --toolchain 1.97.1 aarch64-linux-android
cargo install cargo-ndk --version 4.1.2 --locked --jobs 2
tools/android/doctor.sh
tools/android/build.sh debug
tools/android/build.sh release
```

Set `ANDROID_HOME` or `ANDROID_SDK_ROOT` to the SDK and `ANDROID_NDK_HOME` or
`ANDROID_NDK_ROOT` to its installed NDK. Set `JAVA_HOME` to JDK 21. Scripts must
validate these inputs and use low-priority, two-job Cargo builds. Release output
requires the developer's own signing key; never commit keys or passwords.

The initial storage convention is app-private `content/Data` for an A2/FO-style
install. Original GOG A1 roots instead use `Vfs::push_dir` because they do not
contain `Data`; do not silently rewrite their layout. A platform importer must
choose the correct existing mounting path and preserve mod inheritance, case
normalization, archives, signatures and explicit ODF/CFG/AIP overrides.

## Verification status

- Repository audit: completed; compatibility conclusions remain source based.
- SDK 36, build tools 36.0.0, NDK 29 and JDK 21: found on the preparation host.
- Android Rust target and cargo-ndk: not installed at the audit baseline.
- Android shared library, APK and device execution: not yet attempted.
- Desktop changes: none at this documentation checkpoint.

After an attempt, record exact commands, target, tool versions, artifact hashes,
desktop regression results and remaining blockers here. Shared-crate compilation,
an ARM64 ELF, an APK and an actual device run are separate evidence levels.
No device, input automation or GUI launch is part of this preparation workflow.

## Runtime and release gates

Do not describe Android as compatible until a real Android build succeeds; do
not describe it as playable until device checks pass. Vulkan/GPU limits,
compressed texture support, custom shader pipelines, pause/resume, audio output,
cutouts/DPI and content import require actual device evidence. Scale rendering
quality from measurements while preserving simulation rules, resource counts,
AI behavior, unit limits and mod semantics.

SAF is the intended user-controlled import boundary, following the
[Android storage documentation](https://developer.android.com/training/data-storage/shared/documents-files).
It is not implemented by the current filesystem VFS. Networking is also not
implemented; future cross-play must use explicit versioned formats and digest
checks, not Rust memory layout. Existing snapshot/replay formats are useful
foundations, not proof of a working multiplayer protocol.
