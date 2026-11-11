# Transactional objective reload completion policy

Pre-code private-helper seam for the remaining A2 objective-file adapter, tracked
in #311 under the unfinished LUA-05 work. Based on main `14ce742`, independently of
the open objective-completion #310 and map-confinement #309. This seam introduces
no Lua callable and grants no inventory credit.

## Native gate and shared implementation

The pinned stock A2 file-loader saves its completion-byte vector before reading
and parsing replacement text. Its explicit boolean is tested after successful
reading: false reapplies the old bytes with zero-based list ordinals; true skips
that restoration. The parser clears the old list and inserts a zero bool for
each new goal. The matching original map identifies the insertion as the bool
vector's insert method. The independently captured own-image FO loader provides
corroborating control flow. Missing files skip parsing/restoration. Original
images were read statically; they were not executed or modified.

Private captures, source/image identities and earlier open questions remain in
`STA64-artifacts/native/LUA-05-objective-completion/future-file-loader`. No native
code or game text belongs in the repository. This is bounded completion-policy
evidence, not native GUI, file-resolution, malformed-input, ABI or persistence
parity. No A1 default is inferred from A2's explicit second argument.

STA64 already reads injected bytes, parses metadata/goals, checks ID allocation
and replaces the host list atomically in `api/objectives.rs::load`. Apply the
policy inside that transaction. Do not infer load success from changed IDs,
normalize old states after a failed read, or duplicate reader/parser logic in
the future A2 adapter.

Keep `load(ctx, name) -> Result<()>` and its existing Lua surface unchanged.
Introduce one `pub(super) load_with_completion(ctx, name, reset_completed: bool)
-> Result<()>`; `load` delegates with true. No new Context fields, serialized
records, commands, dependencies or user-facing settings are needed.

## Replacement semantics

After a successful read, parse and checked ID reservation:

- True creates every replacement goal as Active, matching today's generic load.
- False preserves Complete only at the same old list position; other/new slots
  are Active. Persistent IDs and matching text are not selectors.
- A previously Failed goal becomes Active on successful replacement: the native
  completion byte has no distinct failure value to retain. This does not erase
  Failed state when a load fails or is missing.
- Shorter files discard extra old slots; longer files start unmatched goals
  Active; a valid empty-goal file clears the old list.
- All replacement goals receive fresh monotonic IDs. Old handles must not bind
  to new goals. Heading, briefing, notes and source metadata come from the new
  file; visibility and unrelated host state remain unchanged.

Retain the existing path normalization/validation, injected reader order,
UTF-8 parser policy, missing-asset `Objectives.load` diagnostic and error text.
These are established STA64 loader policies, not native filesystem claims.
Missing, malformed, invalid-path and ID-exhaustion cases must preserve the
entire prior list, states, IDs, briefing, visibility and ID allocator. A missing
asset still records its established diagnostic; no other command/event is added.

The generic Lua load keeps exact accepted call forms/arity, void return,
default reset and missing-file behavior. No new Lua boolean argument is exposed
by this seam. The future exact A2 file adapter will validate its two explicit
arguments and translate the boolean only after the prerequisite merges.

## Scope and checks

Own only the narrow generic loader/helper, a new dedicated reload-policy test
module and this contract. Leave the parser, A2 binding/README, inventory/report,
runtime/player/context, core/content/client, serialization and dependencies
unchanged. There is no overlap with #310's A2 completion implementation.

Use synthetic injected files to exercise reset/preserve, changed titles/IDs,
growth/shrink/empty lists, Failed reactivation only on success, all failure
transactions and unchanged generic Lua behavior. Run full mission tests,
strict mission Clippy, workspace and Android ARM64 library checks, formatting,
diff and unchanged-lock checks. A library compile is not APK/device acceptance.
No in-game visual test applies until an actual caller is integrated.

The parent LUA-05 goal and exact file adapter remain unfinished. This adds no
desktop release gate or completion date; December 2026 desktop alpha remains the
priority. User merges; no automatic merge or original-content rewrite.

## Implemented helper and checks

The helper is implemented inside the existing replacement transaction. Four
new tests and the existing objective/parser tests report 11 focused passes.
The full mission suite reports 184 passes, zero failures and 10 ignored;
ignored installed-port probes are not complete-campaign evidence. Independent
read-only review found no confirmed defect. Strict mission all-target Clippy,
workspace all-target checking and an Android ARM64 client-library check through
cargo-ndk/NDK 29 API 26 pass. Root loader formatting uses its existing
120-column style through stdin without traversing unrelated test modules; the
new tests use default Rust formatting. Diff checks pass.

This does not implement the A2 file callable or change its inventory status.
The existing Lua load's exact arity, zero return values, reset default, stale
handle behavior and missing-asset diagnostic are covered by regressions. No
new APK or runtime acceptance is implied by the library check.
