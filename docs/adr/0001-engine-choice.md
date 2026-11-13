# ADR 0001: Engine choice

- **Status:** accepted (foundational: in the repository since its first commit; no separate sign-off recorded)
- **Date:** 2026-11-07
- **Deciders:** not recorded
- **Relates to:** [ARCHITECTURE.md](../ARCHITECTURE.md) ("Engine choice")

## Context

From "Engine choice" in ARCHITECTURE.md, on Bevy:

Its renderer (wgpu) covers Vulkan, DirectX 12 and
Metal from one codebase, which gives Linux, Windows, Apple Silicon and ARM64
without per-platform renderers. The ECS fits an RTS with thousands of
entities, and glTF/KTX2 loading is built in.

Bevy's API changes with every release, and its scheduling is not designed for
deterministic lockstep. Two rules keep that from becoming a problem:

## Decision

**Bevy** for the client.

1. **The simulation does not depend on Bevy.** `sta_sim` is plain Rust and
   only takes commands and returns state. Bevy reads that state to render and
   interpolate. A Bevy upgrade can never cause a desync, and the sim can run
   in a headless server, a replay verifier or a test.
2. **Formats and tools do not depend on Bevy.** `sta_vfs`, `sta_odf` and the
   future `sta_sod`, `sta_tex`, `sta_spr`, `sta_bzn` and `sta_fpq` crates are
   usable from command-line tools, the asset workbench, and other projects.

## Alternatives considered

None recorded.

## Consequences

If Bevy ever stops fitting, only the `sta64` crate has to change.
