# Authored unit information and normal starting forces

Follow-up to UI-18 (#371), based on the user's live Classic HUD check.
Implements modding decisions D1/D7/D9/D10/D11. Original data stays read-only.

## Design before implementation

The HUD currently uses ODF stems as instance names and class labels as type
captions. Read `unitName` as the type caption, falling back to the existing
class label when absent. Read `possibleCraftNames` as a quoted list, including
continuation rows and ordinary include/parent overrides. Preserve spaces,
punctuation and authored case. With no names, use `unitName`, then the ODF
stem; undefined content retains the existing entity-ID fallback.

Names are cosmetic: select one with a fixed portable integer mixer of the
entity ID and ODF stem. This is a pseudorandom distribution over the authored
pool, stable across ticks, selection changes and save/replay reconstruction
of the same entity. It is not a new random draw on each HUD rebuild. Identical
entity IDs and definitions in separate matches can receive the same name;
uniqueness, seed-dependent variation and script-driven renaming are not
claimed. Do not consume simulation RNG, add mutable simulated name state,
change content hashes, or alter save/network formats for this display fix.
Both stock and LCARS use the shared HUD model.

Normal starting forces for a known faction come only from `standardUnitsX`,
in numeric order, preserving repeated entries. Do not append `minimalUnits`,
replace an authored ODF via `originalClass`, deduplicate, or infer builders.
A defined empty list stays empty. Unknown definitions use the existing bare
content fallback. Missing/unsupported entries are skipped with an explicit
startup warning. This exposes rather than disguises missing FO4 start-effect
and faction-object lifecycle behavior; it does not implement those backends.

Font colours will be traced from the mounted HUD CFG and sprite-font data
before changing the renderer. Explicit author values take priority over
fallback colours. Font readability changes must remain intact.

## Verification gates

Synthetic quoted-list/include/override tests; actual constructor name-pool
inspection; stable HUD names and authored type captions; duplicate/order and
no-inferred-start regression tests; startup warning coverage; authored colour
translation tests. Scoped Rust tests, strict Clippy, workspace/all-target and
Android ARM64 library compilation precede publishing. Stage an optimized
manual desktop build; native visual acceptance remains the user's check.
