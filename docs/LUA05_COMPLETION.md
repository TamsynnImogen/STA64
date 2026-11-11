# LUA-05 completion audit and next bindings

Baseline main `d2c72c9`: 24 exact A2 callables, 108 missing signatures. The
original workplan's zero-missing gate remains authoritative. A partial binding
or an error-only/no-op stub does not close the parent issue.

## Pre-implementation contract

The now-merged ownership and confinement simulation operations allow these
adapters without changing Core, content, wire formats or gameplay defaults:

- `A2.SetForceOntoMap(id, boolean)`: exactly two arguments, strict unsigned
  integral handle and Boolean. Require a current non-dying object, then queue
  the existing SCRIPT ForceOntoMap edit. Actual capability, map geometry,
  zero/false requests and rearming remain Core decisions. Add no getter or
  guessed flag to WorldView.
- `Entity.forceOntoMap = boolean`: strict Boolean; use a typed deferred action
  for pending entity receipts, retaining actor-local action ordering and failed
  spawn handling. A resolved non-dying entity uses the same SCRIPT edit. This
  fixes the missing setter used by Blockade and other external ports.
- `A2.SetRealTeam(id, team)`: exactly two arguments and unsigned integral values;
  decode the existing configured numbering mode, accepting real slots and
  neutral only. Queue the existing SCRIPT Team edit. Core retains destination,
  target, unfinished-child, production, cargo and victory guards. Do not
  mutate the cached mission view or implement a second ownership algorithm.
  Reads see the previous snapshot until the next host tick.
- `A2.ObjectivesDisplay_Set_Text_From_File(path, resetCompleted)`: exactly two
  arguments, UTF-8 string and strict Boolean. Reuse the merged atomic
  `load_with_completion` helper: true resets, false preserves only completed
  ordinal slots. Reuse its content-reader paths, parse/error/missing-file policy,
  fresh IDs and metadata. No direct filesystem access or duplicate parser.

Native stock A2 wrappers support the confinement Boolean write and real-team
dispatch on existing/non-dying objects. The pinned objective loader and own-image
FO observations support reset-versus-preserve completion. Strict Lua validation,
pending scheduling and diagnosed failures are explicit bridge policies; native
invalid-input/ABI and full runtime parity are not implied.

## Rejected command feedback

Only these new reported edits and the existing Entity.player transfer are
tracked. Each record retains its SCRIPT sequence, operation and an initially
unsubmitted state. Mark submission only when its actual command is drained to
the host. On the next strictly newer view, match SCRIPT rejection events by
sequence, emit a detailed runtime error containing sequence and edit, count a
fixed operation-specific unsupported key, and retire all settled records.
Unsubmitted, unrelated-player and unrelated-sequence events must not generate
false failures; repeated views/events must not duplicate settled diagnostics.
Use the runtime's existing 100-error cap. Do not change spawn receipts, general
command semantics, sim events, saves or replay encoding. The host must deliver
the complete events of its submitted batch with the next view, as already
required for deferred spawns.

Owned source additions are the narrow mission api/a2 modules and tests;
Entity/EntityAction/spawn integration; private edit_feedback state, api/mod and
runtime processing; exact inventory signatures/tests and regenerated report.
No Core/content/client/network/dependency changes are authorized by this slice.

## Closure blockers to retain

The remaining missing signatures include orders and AI, mutable crew/shields,
unsupported actor edits, path movement and object/fleet APIs. Several require
unimplemented shared systems. Existing presentation intents can record requests,
but recording alone is not playback. Inventory counts, native semantics and
campaign progression must remain separate evidence. The full closure audit
must identify the owning prerequisite for every remaining group rather than
rename missing operations as implemented.

Verification includes strict argument failures without queued commands, real
sim confinement/ownership effects, pending spawn order, correlated rejection
feedback, host objective transactions, inventory source coverage, standard and
installed 1800-tick mission probes, Clippy/workspace and ARM64 checks. Keep all
other campaign diagnostics visible and preserve original reference inputs.
