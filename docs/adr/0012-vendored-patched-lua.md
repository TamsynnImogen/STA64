# ADR 0012: A vendored, patched Lua for reproducible mission scripts

- **Status:** proposed
- **Date:** 2026-11-27
- **Deciders:** the collaborator who decided the direction on 2026-11-27 and
  the repository owner. It touches mission Lua (ADR 0004 D13), so both
  approve. Accepted when they approve the PR that merges this file; not
  binding before.
- **Relates to:** [ADR 0002](0002-deterministic-simulation.md);
  [ADR 0004](0004-modding-v1.md) D13; the save-and-load draft (PR #1275, V4);
  issue #1300 and its implementation, PR #1303; `vendor/lua-src/STA64-PATCH.md`;
  `docs/MISSIONS.md` ("Reproducible tables, sorting and references")

## Context

Saves that rebuild a mission by re-running its script from the start, and any
design where more than one machine runs a script, need the script to do the
same thing, given the same inputs, in every Lua state, every process and on
every machine. Stock Lua 5.3 does not: `pairs` order follows a per-state hash
seed and object addresses, `table.sort` can take a pivot from the clock, weak
tables expose collector timing, `tostring` prints addresses, and string order,
character classes and number text follow the process locale and the C
library. This was measured with authored scripts in separate processes
(#1300). Mission scripts written by mod authors are exposed to all of it.

## Decision

**L1. STA64 vendors the Lua it runs and carries a small, marked patch.**
The crates.io `lua-src` crate that mlua builds Lua 5.3 from is copied into
`vendor/lua-src` and used through `[patch.crates-io]`. The first commit is the
unchanged copy; every local change is marked `STA64` in the source and listed
in `STA64-PATCH.md`.

**L2. The patch gives these semantics, which mod authors may rely on:**
traversal in insertion order (a list kept by list operations first,
ascending), a stable `table.sort`, no weak tables, reference ids instead of
addresses in `tostring`, byte-order string comparison and C-locale string
functions, and the same number text and `^` result on every supported
platform. The exact wording for mission authors is in `docs/MISSIONS.md`.

**L3. The guarantees are tests.** A differential test against a model of the
documented semantics, authored hazard scripts compared across states and
processes, golden number texts on all four CI platforms, and a test that fails
if the unpatched Lua is ever built. An update to Lua or `lua-src` re-applies
the patch, runs these tests and one AddressSanitizer run, and carries the
`full-ci` label.

## Alternatives considered

- A layer in the sandbox only (replacement `pairs`, `next`, `table.sort`,
  `tostring`): it cannot see table constructors and raw stores, so it cannot
  know insertion order, and it leaves weak tables, the locale and number text
  alone.
- Fixed hash seeds and address-free hashing only: the order becomes
  repeatable but stays arbitrary, changes with any unrelated table operation,
  and does not cover sort, weak tables or text.
- Another Lua implementation (LuaJIT, Luau): neither orders traversal, both
  differ from the Lua 5.3 that existing mission scripts and the converter
  target, and the same patches would be needed there.
- Restricting scripts to a reproducible subset by lint: possible for
  converter output, not for hand-written missions and mods.

## Consequences

- Mods rely on these semantics; changing them later is a modding decision.
- Every Lua or `lua-src` update re-applies the patch (procedure in
  `STA64-PATCH.md`); upstream fixes do not arrive by a version bump alone.
- About 770 kB of upstream C live in the repository.
- Cost, measured on STA64's own workloads: table inserts and sorting about
  1.3 times slower, traversal and lookups unchanged, a mission tick within a
  few percent, 8 more bytes per hash entry and 24 per table.
- STA64 code must not give threads their own hooks (mlua keeps those in a
  weak table) and should not turn references into text inside a mission state.

## Open questions

- Whether to move to a later Lua (5.4) at some point, and carry the patch
  there.
