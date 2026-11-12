# Collision geometry: how the originals size a unit, and what peers compare

Research, 2026-11-12. Evidence for the D10 amendment in
[MODDING_DECISIONS.md](../MODDING_DECISIONS.md) (issue #436) and for
WORKPLAN SIM-32 and the geometry part of CON-35.

Method: a data survey of every installed game and mod (ODFs resolved through
`#include`, SODs parsed), plus static analysis of `Armada2.exe` 1.1 with its
symbol map, a comparison with the Roots engine executable, and a string and
readme reading for multiplayer. Nothing was run. No decompiled game code and
no game data are in the repository: functions are cited by name and their logic
is described in words. Per-unit lists are in [../data/collision/](../data/collision/);
the survey scripts are in `tools/research/collision/` (usage at the end).

## Evidence grades

Every claim carries one grade, as the research stated it. A grade is never
raised by a later document without new evidence.

| Grade | Meaning |
|---|---|
| decompiled | read from the disassembly or decompilation of the executable |
| documented | stated in a shipped readme, changelog or string table |
| data-implied | follows from the installed data, not from code |
| unverified | suspected, not shown |

## 1. No ship gets its collision size from rules data

Ship and station ODFs with no explicit size key (`shieldPad` or
`collisionRadius`) after `#include` resolution. Armada II (Stock) matches
Armada II 1.1. Grade: data-implied.

| Install / mod | Ships | Without a key | Stations | Without a key |
|---|---|---|---|---|
| Armada I | 71 | 71 (100%) | 55 | 54 (98.2%) |
| Armada II 1.1 | 123 | 123 (100%) | 90 | 90 (100%) |
| Fleet Ops 3.x | 1754 | 1754 (100%) | 367 | 367 (100%) |
| Roots base | 355 | 155 (43.7%) | 109 | 49 (45.0%) |
| Roots / Fleet Ops 4.0 | 1754 | 1753 (99.9%) | 367 | 367 (100%) |
| Roots / STA2 Classic and Enhanced | 127 | 127 (100%) | 90 | 90 (100%) |
| Dominion War | 2036 | 2036 (100%) | 567 | 567 (100%) |

Where a key exists it is `shieldPad`, a signed offset on a model-derived size,
not an absolute size. The only `collisionRadius` values found: one Armada I
station, one Roots base ship, one Roots / Fleet Ops 4.0 ship.

## 2. Size-related ODF keys

Counts are ODF files that carry the key in their own text (before include
resolution), from `survey.py`; they show reach, not use.

| Key | What it does | Armada I | Armada II 1.1 | Roots / FO 3.x / DW | Grade |
|---|---|---|---|---|---|
| `scaleSOD` | Scales the model when it loads; every later size derives from the scaled model | 29 files | 114 | Roots base 441, FO 3.x 35, DW 1162 | decompiled (A2) |
| `shieldPad` | Signed offset added to the largest ellipsoid semi-axis, giving the bounding radius | not present | not present | Roots base only in practice (199 ships, 60 stations) | decompiled (A2) |
| `collisionRadius` | A2 1.1: read, but the only reader is never called, so no effect. Armada I: live, overrides the model radius. Roots: an "exclude from AI fleets" flag, not a size | 1 | 0 | Roots base 1, FO 4.0 1 | decompiled |
| `shieldScale` | Roots-only key (55 ODFs); meaning not traced | none | none | Roots base | unverified |
| `avoidanceClass`, `avoidMe` | Mark what steering treats as an obstacle and how; they select, they do not size | 13 / 9 | 34 / 19 | FO 3.x 2485 / 398, Roots base 710 / 281, DW 3038 / 470 | data-implied |
| `footprintBuffer` (and Roots `footprintAligned`, 99 ODFs) | Spacing and alignment around a build footprint on top of the size | 4 | 7 | FO 3.x 476, Roots base 233, DW 714 | unverified |
| `physicsFile`, `trekPhysicsFile`, `combatPhysicsFile` | Name physics parameter files (movement tuning). Not part of the size path as far as it was traced | 21 | 49 | FO 3.x 1821, Roots base 370 | unverified |

## 3. The Armada II 1.1 size rule

All steps grade: decompiled. Routine names are from the shipped symbol map.

1. `scaleSOD` is applied when the model loads.
2. `ST3D_Node::CalculateHierarchyBoundingVolumes` builds an axis-aligned box
   over every mesh vertex of the whole node tree, transformed by each node's
   matrix. All LOD meshes and damage meshes count, as do sprite and emitter
   points.
3. `GameObjectClass::InitShieldMatrices` takes the half-extents of that box,
   multiplies each by the square root of 3 for the ellipsoid semi-axes, then
   rescales the three semi-axes together by the factor that makes the ellipsoid
   enclose every vertex. The largest semi-axis plus `shieldPad` is the radius of
   the bounding sphere.
4. The sphere and the ellipsoid are centred on the box centre. The object's
   simulation position is that centre, not the model origin.

Systems that read the shape:

| System | How it uses it |
|---|---|
| Ship collision prevention | broad phase on spheres: contact when the centre distance is under 1.5 x (r1 + r2); narrow phase on the ellipsoids |
| Static obstacle avoidance | the same shape against planets, stations and fields |
| Weapon hits | ellipsoid test |
| Selection | the shape |
| Build footprint | a square whose half-size is the radius |
| Formation scale | the largest radius in the group, with a floor of 28 |

Roots: `ArmadaL.exe` is byte-identical to A2 1.1 in these routines (decompiled,
by comparison). Fleet Operations 3.x uses the same rule; its exe is packed and
was not analysed (data-implied only). Armada I computes size differently from
the model and its formula is not known (unverified); there `collisionRadius`
overrides it.

## 4. Multiplayer in the originals

- Armada II 1.1: `TransportNetwork::GetCrcPacket` builds checksums over all
  `sod/*.sod`, `odf/*.odf`, `techtree/*.tt`, `ai/*.aip`, `*.frm`, the exe with
  `RTS_CFG.h`, and the map. Per-category mismatch strings exist ("SOD files do
  not match", and the same for ODF, TT, AIP/FRM and EXE/RTS_CFG.h).
  Grade: decompiled for the checksum inputs, documented for the strings.
- The readmes say games with mismatching files cannot be joined (documented).
- Roots checksums its whole FPQ file system (documented).
- That a SOD mismatch actually blocks the launch is inferred from the strings
  and readmes, not traced or measured (unverified).

Consequence for STA64: the originals treat models as rules for two reasons
that hold independently. Size comes from the model, and the models are
compared. D10 therefore compares the derived collision geometry per unit, which
is exactly what the rule above consumes, instead of whole-file checksums: a
texture-only pack or a geometry-identical re-export still matches.

## 5. How STA64's radius compares

STA64 (`crates/sta_content/src/launch.rs`, `attach_radii`) uses a different
formula: the maximum over mesh nodes of the XZ distance of the node bounds
centre from the model origin plus the 3D bounding-sphere radius of that node,
times `scaleSOD`, centred on the model origin, and the simulation then takes
three quarters of it. Grade: data-implied (a script approximation of the
decompiled rule, run over the installs' SODs; `orig.py`).

Ratio STA64 radius (before the three-quarter factor) to the original rule
(scale applied, `shieldPad` 0 where absent):

| Install | Units | min | p10 | median | p90 | max |
|---|---|---|---|---|---|---|
| Armada I | 114 | 0.66 | 0.75 | 0.89 | 1.06 | 1.46 |
| Armada II 1.1 | 189 | 0.67 | 0.75 | 0.88 | 1.11 | 1.46 |
| Fleet Ops 3.x | 2104 | 0.66 | 0.73 | 0.85 | 1.02 | 1.66 |
| Roots base | 448 | 0.65 | 0.78 | 0.88 | 1.00 | 2.66 |
| Dominion War | 2586 | 0.66 | 0.73 | 0.88 | 1.01 | 1.66 |

With the three-quarter factor the simulation radius is about 0.65 of the
original on median (0.64 to 0.67 across installs), and the ratio varies per
model, so no single scale factor repairs it. The original also centres the
shape on the box centre: the offset from the model origin has median 0.35 to
1.74 units, p90 7 to 16, and up to 150 for some stations (data-implied).

Bug found on the way (STA64 code reading, not run): when a pack supplies a glTF
for a unit, `find_model` prefers it, `attach_radii` then fails to read it as a
SOD and leaves the radius at 0. The unit loses its collision body and the
digest changes. The size rule must always read the profile's SOD.

## 6. Decisions this supports

D10 as amended: the digest holds the derived geometry per unit (ellipsoid
axes, bounding radius, centre offset, from the profile's SOD with `scaleSOD` and
`shieldPad`), definition keys are compared and display strings are not, physics
reads the profile's SOD only, and a refusal names the unit and the differing
value. Implementation: CON-35 (digest) and SIM-32 (the size rule).

## 7. Still unverified

- Armada I's model formula (only `collisionRadius`'s role is known).
- The packed Fleet Operations 3.x exe: it is assumed to match A2 1.1 from the
  data alone.
- Whether a SOD mismatch really blocks a launch in the originals.
- Sprite and emitter points count in the original bounds; the derived geometry
  as D10 defines it may leave them out of the comparison, and differing points
  alone would then not block a match.
- Whether units of different classes share one cached model, so a `scaleSOD`
  on one unit could change another's shape.
- Meaning of `shieldScale` and the exact role of the buffer keys.

## 8. Bench measurement plan

Estimated 10 to 13 hours on the running originals through the test bench
([../TESTBENCH.md](../TESTBENCH.md)); the measurements are SIM-32's *needs*.

| Measurement | Confirms |
|---|---|
| Sphere readout: read the bounding sphere (radius, centre) from memory for a sample of units per install | the rule in section 3; the tolerance SIM-32 states |
| `shieldPad` effect: the same unit with and without the key, in Roots | the signed offset on the largest semi-axis |
| SOD swap: replace one unit's SOD by a larger or smaller model | size follows the model, not the ODF |
| `collisionRadius` in A2: set a large value on a unit | no effect in A2 1.1 |
| Multiplayer refusal: two instances with one differing SOD | whether the launch is refused and with which message |

## Lists and scripts

`../data/collision/*.tsv`: one row per ship, station and other object per
install: kind, ODF, class label, race, model, `scaleSOD`, `shieldPad`,
`collisionRadius`, source layer. File names and numeric values only. They are
not produced by `tools/data_inventory.py`, so the rule "generated, never
hand-edited" for the other files in `docs/data/` does not apply to this
folder; they are a one-off listing of 2026-11-12.

`tools/research/collision/` (Python 3, standard library only, no game content;
set `STA_GAMES` to the folder holding the installs and `STA_EXTRACT` to the
extracted archive contents):

| Script | Does |
|---|---|
| `arc.py` | extracts FPQ and ZFS archives (a standalone port of `sta_fpq` and `sta_vfs`' ZFS reader) |
| `odf.py` | ODF loading with layered `#include` resolution |
| `survey.py` | the install and mod layer sets; lists size-adjacent keys per set |
| `count.py` | the counts table of section 1 and `units.json` |
| `sodbound.py` | SOD parser and STA64's bounding circle |
| `radii.py` | per-unit STA64 radius into `radii.json` |
| `orig.py` | the approximation of the original rule and the ratios of section 5 |

The decompilation project (`part2/`) and the extracted game files are not in
the repository.
