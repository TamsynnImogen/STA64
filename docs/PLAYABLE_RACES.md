# Playable races

Instant Action uses the race ODFs referenced by the effective `races.odf`.
A resolved, nonnegative integer `instantActionSlot` makes that race eligible;
zero is a valid slot. Choices follow slot order. A unit's `race` string alone
does not make a faction playable, and start-unit availability does not decide
eligibility. Missing/invalid slot fields omit a modern race; malformed explicit
values produce a diagnostic. Standard layered includes and child-mod overrides
apply before these rules.

The setup picker and direct skirmish launch use the same `skirmish_races` result.
A loaded catalogue remains authoritative even if empty, broken, or without any
playable entries. Only a bare database without `races.odf` retains the existing
unit-derived fallback for native/fake content. Launch is disabled without
eligible races; start-loadout validation remains a separate check. Campaign/NPC
race definitions remain loaded and available to their existing consumers.

## Original Armada I

Stock A1 faction ODFs omit the modern slot field. STA64's `Sta1Classic` profile
follows the user's A2FOExtensions policy at source
`7eaa2c76d3a7cd287a8f36940ad62c273cfabb76`:

- `modules/A1Compat/race_menu_policy.cpp` adapts resolved Race fields to
  `rust/a2fo_core/src/config.rs::a2fo_rs_config_race`.
- A complete explicit table of distinct slots in `0..numberOfRaces` is preserved.
- Otherwise, records defining `interfaceConfiguration` receive contiguous slots
  starting at zero, in `race0`, `race1`, … declaration order. Other records remain
  nonplayable. Field presence, including inherited presence, matches that policy.
- A fully authored valid slot table overrides this legacy default. The fallback
  applies only to the A1 profile; A2/FO content does not acquire races from GUI keys.

No original ODF is rewritten, no faction names are hardcoded, and no native
executable is run. This ports the bounded race-menu eligibility policy, not
A2FO's native memory hooks or its separate display/resource bridges.

The UI and skirmish APIs continue identifying factions by their existing unit
race keys. Multiple playable ODFs sharing one such key remain one setup choice,
as before; this change does not add a separate faction-variant identity format.
The earliest slot/stem choice supplies its start loadout and interface. Player
key lookup is separate from the existing general stem-first `Races::get`, so an
unrelated NPC ODF whose filename matches a playable key cannot replace it.
