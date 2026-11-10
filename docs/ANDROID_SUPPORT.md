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

## Narrow platform seam

The proposal was recorded before source changes in merged #251. TOOL-10 /
issue #250 implements this boundary, with build evidence tracked below.

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
   game pause state. Cover HUD command producers in `GameSet::Ui` and the plain
   viewer/map update chains, including their idle clock, as well as Input and
   SimLink. Clear held input and uncommitted drag/placement/target state while
   blocked; retain commands already committed before suspension. Surface
   recreation remains Bevy/winit's responsibility. The existing theme click
   dispatcher also observes this gate, since pointer observers run outside
   `GameSet::Ui`; stale pointer press tracking and theme messages are discarded.
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

The checked-in scripts validate prerequisites before building. Install the
Android SDK components and the Rust target/helper once:

```sh
sdkmanager --install 'platforms;android-36' 'build-tools;36.0.0' 'ndk;29.0.14206865'
rustup target add --toolchain 1.97.1 aarch64-linux-android
renice -n 19 -p $$ >/dev/null
cargo install cargo-ndk --version 4.1.2 --locked --jobs 2
```

Then, from the checkout root:

```sh
tools/android/doctor.sh
tools/android/build.sh debug
tools/android/build.sh release
```

Set `ANDROID_HOME` or `ANDROID_SDK_ROOT` to the SDK and `ANDROID_NDK_HOME` or
`ANDROID_NDK_ROOT` to its installed NDK. Set `JAVA_HOME` to JDK 21. Scripts must
validate these inputs and use low-priority, two-job Cargo builds. Release output
requires the developer's own signing key; never commit keys or passwords.

Debug output is `packaging/android/app/build/outputs/apk/debug/app-debug.apk`.
Release output is `packaging/android/app/build/outputs/apk/release/app-release-unsigned.apk`.
The build script requests a `cdylib` only for Android and checks ELF64/AArch64,
activity exports, every packaged library's 16 KiB LOAD alignment and shared
dependencies against NDK public API-26 libraries. It extracts the actual APK
native payload and repeats these checks after Gradle stripping/packaging, then
checks APK ABI/ZIP alignment and the debug signature. These are artifact checks, not device tests.
Do not enable the desktop `dev` / dynamic-linking feature for Android.

For a release, build the unsigned APK, use SDK `apksigner sign` with your own
keystore and output path, then `apksigner verify --verbose` on the signed APK.
Let the signer prompt for passwords. Keep the key and passwords outside the
repository; the build and CI intentionally contain no signing credentials.
Release optimization, signing, store submission and device acceptance are
separate from the initial debug build.

The manual `Android APK (manual)` workflow packages and uploads a debug APK.
It is dispatched explicitly and adds no required Android job to desktop CI.

The initial storage convention is app-private `content/Data` for an A2/FO-style
install. Original GOG A1 roots instead use `Vfs::push_dir` because they do not
contain `Data`; do not silently rewrite their layout. A platform importer must
choose the correct existing mounting path and preserve mod inheritance, case
normalization, archives, signatures and explicit ODF/CFG/AIP overrides.

## Verification status

Preparation verified on 2026-11-10, native sources at `3fd2615e61caed2546803b9b2958d832febf8635`:

- Repository audit: completed across all 16 crates and build/client tooling.
- Rust 1.97.1 / Bevy 0.20.1: retained without dependency upgrades.
- SDK36/build-tools36.0.0, NDK29.0.14206865, Temurin21.0.11 and cargo-ndk4.1.2:
  doctor passes.
- `JAVA_HOME=<JDK21> tools/android/build.sh debug`: succeeds, producing the full
  shared ARM64 client and installable, debug-signed NativeActivity APK.
- Both staged and actual APK ELFs pass ELF64/AArch64/DYN, activity exports,
  16 KiB LOAD alignment and shared-dependency checks. APK ZIP alignment, ABI
  contents and v2 debug-signature verification pass.
- All **291 client unit tests pass**, including four platform tests covering
  direct Running/WillResume recovery, background/resume clock isolation,
  preserved player pause/commands, stale input/widget cleanup and desktop's
  absent-gate behavior. Optional GPU/external-install probes were not enabled.
- Strict client all-targets Clippy and workspace all-targets check pass. Commands
  use two jobs, low priority, debug info/incremental disabled and dependency
  optimization overrides for the local check build.
- Native Linux desktop build, `--help`, and the 300-tick synthetic sim-only smoke
  pass: `tick 300 objects 3 digest 31d49d4f2ab3de17`.
- Device execution, Waydroid execution and release APK/signing: **not performed**.
  Other desktop OS validation is delegated to the existing full CI matrix.

Artifact SHA-256:

```text
APK:             839b776cdb9aee3039355af2b63391ee67e1f5da486388926fede950c071d8d5
APK ELF payload: 790fa20cb05f0aac326f39c92279b22c47801bf676381f291c0ae240680bcf68
Prepackage ELF:  6e133b219a0380658d795b2f242ba5a3bc037be9f61422b41e1d772e9abdf454
```

The actual APK payload differs from the prepackage library because Gradle strips
symbols. The debug APK is about 176 MiB; this is an unoptimized development
artifact, not a release-size or performance result. No original game data is
included. Build logs and a source/toolchain/hash receipt are retained with the
local artifact.

Shared-crate compilation, an ARM64 ELF, an APK and an actual device run remain
separate evidence levels. No device, input automation or GUI launch is part of
this preparation workflow.

## Renderer requirements

The pinned Bevy render settings use wgpu 30, automatic available-backend
selection, functionality priority and normal wgpu device limits. An Android
API level or the presence of a Vulkan driver alone does not establish that
these requirements or STA64's custom shaders/3D textures can run. No GLES or
low-end-device fallback is claimed by this preparation. On each test device,
record the selected backend, adapter/driver, features and limits, shader
pipeline success, BC upload versus CPU texture fallback, and surface recreation.
Choose graphics presets from that evidence. Do not change simulation rules to
work around rendering limits.

## Waydroid smoke testing

Waydroid is useful for initial application checks, with its results recorded
separately from physical ARM64 devices. The preparation host currently has a
stopped x86_64 Waydroid session whose configuration advertises `arm64-v8a`
through `libhoudini.so`. This is configuration evidence only: the running ABI,
Android version, bridge loading, Vulkan behavior and APK launch are unverified.
The standard [Waydroid installation documentation](https://docs.waydro.id/usage/install-and-run-android-applications)
warns that ARM-only applications do not run on ordinary x86_64 images.

For a manual test, start Waydroid normally, install the debug APK with
`waydroid app install /absolute/path/to/app-debug.apk`, then open STA64 from
the application menu yourself. Record the APK hash, Android version, active
ABI list, first-screen result and application logs. An empty content catalogue
is expected until user-owned content can be staged/imported; the APK contains
no original game assets. Check background/resume and mouse/keyboard behavior
separately. Report any ABI/native-loader error before altering the bridge.

A translated Waydroid run does not validate native ARM64 execution, a mobile
GPU driver, touch hardware, display cutouts, battery use or thermal behavior.
No Waydroid session, activity or GUI is started or automated by these scripts.

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
