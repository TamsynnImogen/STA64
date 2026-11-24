# Closed alpha testbed mod

The December 2026 desktop alpha ships with a test mod: a standalone,
`Profile=native` mod that runs with no game install, carries only assets we may
distribute, and gives testers one map per group of systems so a report points
at one area. Release target: **24 December 2026** (closed alpha, invited
testers only). Packages TOOL-20..25 in [WORKPLAN.md](WORKPLAN.md).

## Shape

```
STA64Testbed.sta                 base mod: standalone=1, Profile=native
                                 units, weapons, tech tree, effects, sounds,
                                 every test map, scenario Lua, credits
STA64Testbed_Combat.sta          child mods: ParentMod=STA64Testbed and a
STA64Testbed_Defence.sta         [Features] section that sets all 42 switches
STA64Testbed_Behaviour.sta       (on for the group, off otherwise), plus the
STA64Testbed_Economy.sta         group's maps in front of the list
STA64Testbed_Production.sta
STA64Testbed_Fleet.sta
STA64Testbed_Sandbox.sta         every switch on except a1_compat
```

Why child mods and not per-map switches: switches resolve when the content
database loads (`resolve_features`: profile < mod `[Features]` < per-ODF
`Feature_*` keys), before a map is chosen. A child mod per group uses that
machinery unchanged, needs no engine or digest change (D10), and the selected
group is part of the mod set that replays and captures already record.

One mod per `.sta` ([STA_ARCHIVE.md](STA_ARCHIVE.md)), so testers get a zip of
seven `.sta` files signed with the project key, to drop into `Data/Mods/` of an
empty folder (no install needed: D6 mounts nothing under a standalone mod).

## Switch groups

`cheats` is on in every group (sandbox commands, [CHEATS.md](CHEATS.md));
`a1_compat` is off in every group.

| Group | Switches |
|---|---|
| Combat | `weapon_damage_controls`, `fire_arcs`, `turrets`, `team_change_weapons`, `weapon_shot_timing`, `start_fire_hold`, `weapon_hit_roll`, `weapon_target_damage`, `weapon_target_masks`, `weapon_cloak_fire`, `crew_weapon_delay` |
| Defence | `directional_shields`, `point_defense_cycles`, `pd_energy`, `pd_debt_timer`, `energy_systems`, `shield_zero_hold`, `crew_loss_on_hit` |
| Behaviour | `attack_buffer_roots`, `attack_combat_speed`, `attack_opportunity_fire`, `builder_hold_fire`, `target_score`, `idle_assist`, `assist_high_autonomy_only`, `assist_on_ally_fire`, `transport_attack_by_weld` |
| Economy | `extra_resources`, `officer_capacity`, `pay_on_queue` |
| Production | `multi_yard_builds`, `construction_rigs`, `upgrade_pods`, `hybrid_yards`, `station_rotation` |
| Fleet | `breeders`, `collective_xp`, `squadrons`, `wreckage`, `craft_identity` |
| Sandbox | all of the above |

The `[Features]` sections are generated from this table by
`tools/testbed/gen_children.py`; a test fails if a switch in `FEATURE_NAMES` is
in no group, so a new switch cannot be forgotten.

Each scenario map's briefing lists its switches and their state on main
(from [NATIVE_FEATURES.md](NATIVE_FEATURES.md)): a switch whose system is still
`todo` is listed as "not expected to work yet", so testers do not report it.

## Assets: distributable only

Nothing from Armada, Armada II, Fleet Operations or Legacy (ADR 0003). Every
file in the mod has a row in `CREDITS.md` inside the archive (path, source,
author, licence). Sources, in order of preference:

| Source | Use | Licence | Obligation |
|---|---|---|---|
| Earth Conflict HD (Immy's own 2008 mod, rebuilt <6k tris, 1K PBR, SOD with hardpoints) | Faction 1 "Terran Union": 19 hulls incl. yards, miner, constructor | Immy's own | none |
| Star Ruler 2 shipset + stations (Blind Mind Studios) | Faction 2 (one shipset, recoloured in-material), shared stations, asteroids, debris | CC-BY-NC 2.0 | credit in-game and in CREDITS.md; non-commercial (a free closed alpha is fine) |
| Kenney / other CC0 packs | weapon, shield, explosion sprites; UI and weapon sounds | CC0 | none (credited anyway) |
| Made for the mod (procedural sprites, synthesised SFX, fixture GLBs) | gaps | CC0 (ours) | none |

Atolm's Hellions stay out of the alpha until the written permission is in the
repository's records.

The mod's source (assets are binaries of tens of MB) lives in its own
repository, `STA64-Testbed`, not in the engine repository; the engine repository
keeps this plan, the generator and the smoke test. Every mod key the engine does
not read is reported by `check-pack` (D7) and must be zero before release.

## Schedule

| By | Package | Result |
|---|---|---|
| 28 Nov | TOOL-20 | Skeleton mod mounts standalone; empty map starts; licence ledger; build → signed `.sta` |
| 5 Dec | TOOL-21, TOOL-22 | Two factions buildable and fighting; effects and sounds; tech tree |
| 12 Dec | TOOL-23, TOOL-24 | Seven child mods, one map per group plus Sandbox; scenario Lua with briefings and set-ups |
| 19 Dec | TOOL-25 | Release candidate: zip, tester README, bug-report flow (TOOL-14 capture), headless smoke over every map |
| 20–23 Dec | — | Buffer, fixes from a dry run |
| 24 Dec | — | Closed alpha out |
