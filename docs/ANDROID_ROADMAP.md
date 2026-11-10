# Android roadmap

This roadmap stages Android work without changing the desktop release priority.
The December 2026 desktop alpha remains the immediate target; no Android dates
are committed. See [ANDROID_COMPATIBILITY.md](ANDROID_COMPATIBILITY.md) for the
source-based feature matrix and [ANDROID_SUPPORT.md](ANDROID_SUPPORT.md) for the
Android entry point, build steps, and current verification evidence.

## Guardrails

- Keep `sta_core`, `sta_sim`, AI decisions, snapshots, and replay commands shared
  across platforms. Do not fork the engine, upgrade Bevy for Android, or rewrite
  core protocols as part of initial bring-up.
- Use the existing `GameSet::SimLink` and `SimLink` command path. Android
  lifecycle handling must pause and resume simulation work deliberately; pausing
  virtual time alone is not a lifecycle policy.
- Treat cross-compilation, APK installation, device launch, sustained play, and
  feature parity as separate evidence gates.
- Preserve all supported desktop features. An Android-specific quality preset
  may be introduced only from measurements, not from AI or simulation feature
  caps.

## Stages and gates

| Stage | Status | Work and exit evidence |
|---|---|---|
| 1. Compatibility inventory | **Complete** | Audit the 16 Rust crates and their platform-sensitive seams. Keep implemented code distinct from planned behavior and Android-unverified behavior in the compatibility matrix. |
| 2. Minimal app shell and build path | **Local build verified; device acceptance pending** | Add a shared-library/desktop-compatible entry arrangement, Android NativeActivity entry, app-private writable/cache paths, and an opt-in cargo-ndk plus Gradle build path. Gate simulation startup until the activity is resumed and the first usable frame can reach the existing SimLink schedule. The full ARM64 client and signed debug APK now build from the committed source with pinned tooling; desktop unit, Clippy, workspace and CLI checks pass. See ANDROID_SUPPORT.md for the exact revision and hashes. Remote CI and device acceptance remain separate. |
| 3. First-device bring-up | **Planned** | Build, install, launch, resume, background, and resume again on a physical device. Record device/OS, toolchain, artifact hash, logs, and lifecycle results. The NDK vendored-Lua build already passes; test Lua initialization/mission execution, asset loading, renderer startup, audio initialization, and clean shutdown at runtime. Waydroid is available for an initial translated smoke test but does not replace physical ARM64/mobile-GPU acceptance. |
| 4. Touch and gesture mapping | **Planned** | Map selection, move, attack, and other supported actions through existing commands. Define gesture arbitration so camera pan/zoom/orbit does not submit orders. Validate multi-touch, cancellation, rotation, and input focus on-device. |
| 5. Safe-area and density adaptation | **Planned** | Add safe-area insets and DPI-aware layout for phones and tablets. Preserve all existing controls and HUD information, including a data-driven path toward the requested ten-resource display. Check minimum readable scale, clipping, and landscape/portrait behavior. |
| 6. User-selected content import | **Planned** | Add an import flow using Android's [Storage Access Framework](https://developer.android.com/training/data-storage/shared/documents-files). Import only user-selected files/directories into managed app storage, validate paths and content, and expose that copy to the existing VFS. Avoid broad storage permissions. Include cancellation, insufficient-space, malformed-archive, and interrupted-copy recovery tests. |
| 7. Performance and graphics quality | **Planned** | Measure startup, frame time, memory high-water mark, CPU surface baking, asset decode, and thermal behavior on representative devices. Test compressed texture/device support, including fallback behavior. Base any quality presets on those measurements while keeping simulation and AI behavior unchanged. |
| 8. Feature breadth and parity review | **Planned** | Track remaining UI, editor, campaign, video, and gameplay gaps explicitly. Map viewer and model viewer presence do not establish full game parity. Do not mark campaign progression, galactic strategy, map editing, video playback, or ten-resource HUD support complete until implemented and validated. |
| 9. Networked play | **Future / separately gated** | `sta_net` is currently a placeholder. Any Android network work follows a separately versioned protocol design and interoperability plan; it is not part of the first mobile shell. |

## Evidence to record

For each stage, record:

- Exact source revision and build command, plus Android SDK/NDK/JDK versions.
- Target ABI, device model, Android release, and graphics/audio backend.
- Artifact identity and whether the evidence is compile-only, installed launch,
  manual interaction, sustained runtime, or parity validation.
- Reproducible failure logs and the smallest device matrix used.

The [Bevy 0.20.1 examples](https://github.com/bevyengine/bevy/blob/v0.20.1/examples/README.md)
are the version-pinned API reference for client work. They are not a substitute
for Android device evidence.
