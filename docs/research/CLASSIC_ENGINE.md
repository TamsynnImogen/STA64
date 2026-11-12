# STA2 Classic on the Roots engine (research)

Research record, 2026-11-12, Cedric. It supports the proposed amendment in
[FO_PROFILE_DETECTION.md](../FO_PROFILE_DETECTION.md). Method: read-only
inspection of the installs under `/home/cedric/Games/Heroic/` (ini, text and
ODF files, SHA-1 of binaries, `strings` on binaries). No game was executed and
no binary disassembled. No game data, readme text or string dumps are copied
into the repository; values are quoted only where they are short facts.

**Evidence grades.** *Read* = read from a file on this machine during this
research. *Documented* = stated in a shipped readme, changelog or guide.
*Inferred* = derived from the above by reasoning, not stated anywhere.
Claims marked *Read* about the Roots `info.ini` files and the FO 3.x files were
re-checked on 2026-11-12 immediately before writing; claims about Classic's
ODF content come from the investigation scripts' output and are graded *Read
(scripted)*.

## 1. What `info.ini` says in the Roots install

All four `[mod]` sections, *Read*:

| Folder under `Data/` | Name | Version | AssetVersion | ignoreParentMaps | standalone | ParentMod |
|---|---|---|---|---|---|---|
| (base) | Fleet Operations: Roots | 5.0Z | 50000 | 1 | 1 | none |
| `Mods/Fleet Ops 4.0` | Fleet Operations | 4.0Z | 50000 | 1 | 1 | none |
| `Mods/STA2 Classic Mod` | Star Trek Armada II: Classic | 1.2Z (Original Gameplay) | 50000 | 1 | 1 | none |
| `Mods/STA2 Classic Mod Enhanced Gameplay` | Star Trek Armada II: Classic with enhanced gameplay | 1.2Z (Enhanced Gameplay) | 50000 | 0 | 0 | STA2 Classic Mod |

No other install has an `info.ini` (*Read*): stock Armada II (both copies),
Fleet Operations 3.x, Dominion War and Armada I have none.

Two of the four names do not match the `looks_like_fo` matcher at all
("Star Trek Armada II: Classic"), and since CON-33 the base name is not seen
for the standalone ones. That is the whole problem; `AssetVersion` is present
in every one of them and is the same value (50000).

## 2. What `AssetVersion` means

- *Documented* (Fleet Operations guide): the Fleet Operations version the mod
  was designed for, derived as major x 10000 + minor x 100 + patch, for example
  3.2.5 gives 30205. The original engine uses it for an incompatibility warning
  the player can skip.
- *Documented* (`FOReadme.txt`, 3.2.5 changelog): "AssetVersion layout has been
  changed. Now also being displayed in Mod Settings".
- *Read*: `FleetOpsHook.dll` of Fleet Operations 3.x and of Dominion War are
  byte-identical (same SHA-1) and contain the string `AssetVersion`.
- *Documented* (`Data/devstuff/Engine_Changelog.txt`): the engine version
  changed "from 4.0.0.Jan (40000) to 5.0.0.Bob (50000)". The Roots engine
  number is therefore the same 50000 that the Roots mods declare.
- *Inferred*: by the formula, Fleet Operations 3.2.7 corresponds to 30207. No
  file in the FO 3.x or Dominion War installs declares it.
- *Inferred*: 50000 as the threshold for "Roots lineage". It is the only
  value seen on the Roots side, and Fleet Operations 4.0 and Classic cannot be
  told apart by it. It is not known whether a hypothetical 4.x Fleet Operations
  data pack for the old engine used 40000; none is installed.

## 3. FO 3.x and Dominion War

- *Read*: both `Data/packages.ini` files have `[FleetOps]` with `Version=3.2.7`
  (Dominion War also has `MMpack=yes`, Fleet Operations `MMpack=false`).
- Neither has an `info.ini`, so neither could be identified by `AssetVersion`.
  This is why the proposal lists `packages.ini` as step 4. D5 excludes both
  lineages from v1, so it is not needed now.

## 4. The Roots engine layout

- *Read*: one launcher `FleetOps.exe` starting `Data/ArmadaL.exe` with
  `FleetOpsHook.dll`; mods are selected with `/mod "<folder>"` (read from the
  readmes, *Documented*).
- *Read*: `Mods/Fleet Ops 4.0/ArmadaL.exe` is byte-identical to
  `Data/ArmadaL.exe` (same SHA-1).
- *Read*: `Mods/STA2 Classic Mod` ships no game executable (its folder holds
  support DLLs and the editor tool only). *Documented*: its readme says
  version 1.2Z is "fully compatible with the Roots game engine". So Classic is
  a data pack that needs the host install's engine, which is the case the
  `AssetVersion` field is designed for. Compare
  [modding/fo-hooks-asi.md](modding/fo-hooks-asi.md), which found the same
  lineage structure.

## 5. Classic's data versus stock Armada II

All *Read (scripted)* by comparing the extracted ODFs of stock Armada II with
those of Classic:

- The rules are stock Armada II rules: dilithium, metal, latinum, biomatter and
  crew; officers; stock `RTS_CFG.h` values such as
  `SHIELD_DAMAGE_PROTECTION = 0.8`; `misc/gui_fed.cfg` is byte-identical to
  stock's.
- Of Classic's 1097 ODFs, 1089 share a file name with stock's 1101, and 700
  have identical key and value sets.
- It also relies on Roots engine semantics: 29 keys that stock data never
  uses, for example `worthxp` (300 ODFs), `initialShotDelayModifier = 0.0` (47
  special weapons), `canShipyardRepair = 0` (14), `hitChanceMod`, `usesLatinum`,
  `replaceweapon`.
- The Nebula's point-defence weapon (`gpdlaser`) had `shotDelay` changed from
  3.5 to 0.1. *Documented* (Engine_Changelog.txt lines 81 to 83): point-defence
  weapons now obey `shotDelay` so their firing rate can be limited, naming the
  Nebula's Point Defense Phaser in the Classic submod. Under STA64's
  `Sta2Classic` profile `point_defense_cycles` is off (`profile.rs`), so this
  weapon never fires. This is the one case where the wrong profile makes a unit
  visibly useless.
- Classic Enhanced additionally uses Roots overloads: `weldinganimspeed = 0.0`,
  a `reflectweapon` "Infinite Build", `alliedrepairupgrade`,
  `CRAFT_DEFAULT_*_AUTONOMY`, `BASE_CREW_PER_SEC`.

## 6. What the profiles control today

Checked against `crates/sta_content/src/profile.rs` and its users on
`origin/main` (c9b6f12):

| Behaviour | `Fo4` | `Sta2Classic` | Finding |
|---|---|---|---|
| `point_defense_cycles` | on | off | Consumed in `sta_sim/src/combat/pd.rs`. Classic needs it on. Stock Armada II has this weapon too, so "off" is doubtful for stock as well; measured in [ENGINE_MEASUREMENTS.md](ENGINE_MEASUREMENTS.md) M1: stock PD works with no rate limit, so "off" is wrong for stock. |
| `pay_on_queue` (and `queueInstantDeduct`) | off | off | Only Native turns it on. The key is read in `sta_content/src/odf.rs`, but no Classic, FO 4.0 or Roots ODF uses it. Measured in [ENGINE_MEASUREMENTS.md](ENGINE_MEASUREMENTS.md) M3: stock, Classic and FO 4.0 all charge when the item starts. |
| Resource-name fallback | FO names | stock names | Classic follows stock and the names are derivable from the string table. |
| UI theme | FO | stock | Classic follows stock; derivable from the race ODF's `interfaceConfiguration`. |
| Turrets, fire arcs, directional shields, queue limit, kinematics, confinement | identical | identical | The two profiles do not differ. |
| `extra_resources`, `officer_capacity`, `station_rotation`, `multi_yard_builds`, `construction_rigs`, `weapon_damage_controls` | mixed | mixed | No code outside `profile.rs` and `features.rs` reads these switches yet, so choosing a profile changes nothing for them today. |

So the profile today matters for little, and what it matters for (point
defence) is the Classic failure above. The larger risk is the future: each of
the unconsumed switches will start to matter as its simulation lands, and
Classic would pick the wrong side of it.

## 7. Roots engine behaviours Classic depends on that STA64 does not model yet

Candidates only; no WORKPLAN rows are added for them by this PR. Each is a
place where the engine reads a key or constant stock Armada II never reads.

- Point-defence weapons obeying `shotDelay` (changelog; modelled as a whole by
  `point_defense_cycles`, rate unmeasured).
- `initialShotDelayModifier` on special weapons (47 in Classic).
- `worthxp` as experience award (300 ODFs).
- `canShipyardRepair = 0` (14 ODFs).
- `hitChanceMod` and `usesLatinum`.
- `replaceweapon` on upgrades.
- Station rotation when placing (Classic readme; `station_rotation`).
- `weldinganimspeed`, `reflectweapon` "Infinite Build", `alliedrepairupgrade`
  (Enhanced).
- `CRAFT_DEFAULT_*_AUTONOMY` and `BASE_CREW_PER_SEC` in `RTS_CFG.h`
  (Enhanced; part of the D9 rules struct, CON-35).
- Artillery ships preferring stations as targets; formation behaviour when
  ships are split from a fleet (Classic readme, both versions).
- The remaining of the 29 keys not listed above, from the scripted key
  comparison; the full list is not copied here.

## 8. Measurement plan (done)

A bench measurement on the Refit test bench (original game, headless) covered
four behaviours that data and changelogs do not settle. The results are in
[ENGINE_MEASUREMENTS.md](ENGINE_MEASUREMENTS.md); none are restated here.

1. Point defence: firing rate and energy use, stock Armada II and Roots.
2. Initial special-weapon delay (`initialShotDelayModifier`).
3. Pay timing: whether queued items are paid at queue time or at build start,
   per lineage.
4. Combat speed and autonomy defaults.

The results decide which switches are per-lineage and which are plain data
(`Principle: data first` in [WORKPLAN.md](../WORKPLAN.md)).

## 9. What was not examined

No Roots mission or script. No multiplayer. No Armada I mod. No executable was
disassembled, so any statement about what the engine does comes from the
shipped changelogs and from data, not from its code.
