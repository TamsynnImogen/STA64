# Android compatibility assessment

This is a source-based compatibility assessment and preparation checklist. It
does not claim an Android build, installation, or runtime test. For current
Android entry-point, build, and validation evidence, see
[ANDROID_SUPPORT.md](ANDROID_SUPPORT.md). The staged work is in
[ANDROID_ROADMAP.md](ANDROID_ROADMAP.md).

## Status key

- **Implemented** — present in the repository's current source or configuration.
- **Partial** — a useful part exists, but required Android or product behavior is
  missing or unverified.
- **Planned** — required work has not been implemented.
- **Android-unverified** — desktop or headless source exists, but no Android
  build/device proof is recorded.

These labels describe different things; source presence alone is not Android
compatibility evidence.

## Compatibility matrix

| Area | Status | Evidence and Android gap |
|---|---|---|
| Rust and engine versions | **Implemented / Android-unverified** | `rust-toolchain.toml` pins Rust 1.97.1. The workspace pins Bevy 0.20.1. Rust Android target installation and a successful device build are not yet evidenced. |
| Deterministic simulation | **Implemented / Android-unverified** | `crates/sta_core`, `crates/sta_sim`, and `crates/sta_ai` are Bevy-free; `sta_sim` uses fixed-point state, ordered entities/commands, and deterministic tick processing. Keep platform APIs out of this seam. |
| Snapshot and replay formats | **Implemented / Android-unverified** | `crates/sta_sim/src/snapshot.rs` uses snapshot format 8 with little-endian version/seam/content-hash header and postcard body; `crates/sta_sim/src/replay.rs` uses replay format 1 and carries `SEAM_REV`. These formats are not an Android save-compatibility guarantee across builds. |
| Network crate | **Planned** | `crates/sta_net/src/lib.rs` remains a documented placeholder. There is no transport or lockstep network implementation to claim as Android-ready. |
| Client separation | **Implemented / Android-unverified** | `crates/sta64` is the Bevy client; `crates/sta64/src/sim_link.rs` routes existing commands into the shared simulation. The shared command path is the intended input seam for touch controls. |
| Android app entry and lifecycle | **Planned / Android-unverified** | No Android NativeActivity entry or app lifecycle integration is proven by the current desktop source review. Initial device work must gate simulation startup on a usable resumed frame and use `SimLink`/`GameSet::SimLink`; pausing virtual time alone does not establish correct lifecycle behavior. |
| Native build dependencies | **Partial / Android-unverified** | `crates/sta_mission/Cargo.toml` enables mlua 0.12.2 with `lua53` and `vendored`, so the Lua VM requires an NDK C build. Host SDK 36, NDK 29, and JDK 21 are reported installed; Rust Android target setup and the cargo-ndk/Gradle path are still being prepared. No Android build result is recorded here. |
| Lua state and saves | **Partial** | `sta_mission` integrates the Lua runtime, but VM state is not part of `sta_sim` snapshots. Android lifecycle pause/resume must not imply mission VM serialization. |
| Data parsing | **Implemented / Android-unverified** | `crates/a2fmt`, `sta_odf`, `sta_bzn`, `sta_fpq`, and `sta_vfs` contain the current SOD/TGA/ODF/BZN/FPQ/ZFS/STA readers. VFS also implements `.sta` signatures and manifests. Parser presence does not prove all real install data or archive layouts on device. |
| Filesystem and install access | **Partial / Android-unverified** | `crates/sta_vfs/src/lib.rs` builds mounts from directories and enumerates them with filesystem paths and `read_dir`. Android needs app-private paths and a user-selected import path; do not assume desktop install-root discovery works in app storage. GOG A1 roots have no `Data` directory, so mount their content with `push_dir`; `mount_install` expects `Data`. |
| Models and textures | **Implemented / Android-unverified** | `crates/sta_assets` handles SOD, M3D, glTF/GLB and the listed image formats. `crates/sta64/src/render/textures.rs` uploads BC1/2/3 texture data when the GPU supports it and decodes it on CPU otherwise. KTX2 lookup and dispatch exist, but its Bevy feature and device path are not established for Android. GPU format support, memory pressure, and driver behavior need device checks. |
| Surface baking | **Partial / Android-unverified** | `crates/sta_surface` uses `available_parallelism` and full CPU buffers for baking. Mobile thread counts, peak allocation, latency, and cache behavior need measurement before choosing mobile limits. |
| Input | **Partial** | `crates/sta64/src/input` currently maps desktop mouse and keyboard into existing commands. Touch and gamepad translation are absent. Gesture arbitration must keep camera gestures from committing game orders. |
| UI scaling and safe areas | **Partial** | Existing UI/config code handles a fixed 1080-based layout/scaling path, but Android safe-area insets and density-specific behavior are not implemented or verified. |
| HUD resources | **Partial** | `crates/sta64/src/ui/hud` currently exposes dilithium, latinum, metal, and biomatter. The requested ten-resource interface is future work; it must remain data-driven rather than imply those slots are already available. |
| Audio | **Implemented / Android-unverified** | `crates/sta_assets/src/audio.rs` decodes WAV, Ogg Vorbis, and MP3; `crates/sta64/Cargo.toml` configures Kira with CPAL and the client has a null fallback. Android audio initialization, focus, routing, and device behavior remain untested. |
| Video | **Planned** | Video playback is absent. BIK or other cutscene playback must not be inferred from audio or asset decoding. |
| Map browser and viewer | **Implemented / Android-unverified** | `crates/sta64/src/map` and `crates/sta64/src/viewer` provide map browsing/viewing and the model viewer on the desktop client. Their controls, performance, and layouts need device validation. |
| Editor and campaign features | **Partial / Planned** | A map editor and galactic strategy are absent; campaign progression is incomplete. Android work must not present these as parity features. |

## Android-specific constraints

- Keep `sta_core`, `sta_sim`, and AI decision logic independent of Bevy and Android
  APIs. Platform integration belongs at the client boundary.
- Use the existing `SimLink` and command stream for touch/gamepad actions. Camera
  gestures and map navigation must not accidentally submit orders.
- Keep game data in app-private storage by default. For user-selected imports,
  use Android's [Storage Access Framework](https://developer.android.com/training/data-storage/shared/documents-files)
  and copy/validate selected content into managed app storage as appropriate;
  do not request broad `MANAGE_EXTERNAL_STORAGE` access.
- A successful cross-compile is only a build result. GPU formats, Lua's native
  build, input, audio, lifecycle, memory, and frame pacing require on-device
  verification.

The pinned [Bevy 0.20.1 examples](https://github.com/bevyengine/bevy/blob/v0.20.1/examples/README.md)
are a version-matched reference for Bevy APIs. They do not constitute evidence
that this application runs on Android.
