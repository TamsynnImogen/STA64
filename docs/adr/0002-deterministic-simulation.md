# ADR 0002: Deterministic simulation

- **Status:** accepted (foundational: in the repository since its first commit; no separate sign-off recorded)
- **Date:** 2026-11-07
- **Deciders:** not recorded
- **Relates to:** [ARCHITECTURE.md](../ARCHITECTURE.md) ("Simulation rules"); `docs/WORKPLAN.md`, "Determinism rules (sim crates: `sta_core`, `sta_content`, `sta_sim`)", the operative restatement; [ADR 0001](0001-engine-choice.md)

## Context

None recorded beyond the source section, which states the rules without giving reasons. Its stated purposes are desync detection and replay verification (per-tick state digest).

## Decision

From "Simulation rules" in ARCHITECTURE.md:

- Fixed-point (`Fixed`, 32.32) for all sim state. No `f32`/`f64` in state.
- Fixed tick rate (`TICK_HZ`); the renderer interpolates between ticks.
- Entities live in id-ordered storage; no hash-order iteration.
- One seeded RNG per sim.
- Per-tick state digest for desync detection and replay verification.
- Player commands are the only input. Replays are the seed plus the command
  log.

From "Determinism rules" in `docs/WORKPLAN.md`, which restates and extends them:

1. No `f32`/`f64` in state or in arithmetic that feeds state; floats only at
   load (S5) and out to rendering. No platform maths.
2. `BTreeMap`/sorted `Vec` only; no `HashMap`/`HashSet` (clippy-enforced).
3. One seeded `Rng`; cosmetic randomness never touches it.
4. Commands apply at tick boundaries in `(player, seq)` order; ids are never
   reused; entities iterate by ascending id.
5. No threads, I/O or clocks inside `step`.
6. Golden digests at fixed ticks are committed and checked on all four CI
   targets (TOOL-06).

## Alternatives considered

None recorded.

## Consequences

None recorded.
