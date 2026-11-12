# JSON5 content (CON-04)

`sta_content::json5` loads unit and weapon definitions written as JSON5. There
is no JSON5 data model of our own: a content file is a set of **ODFs** in JSON5
syntax, with the games' own keys and units, and it goes through the same loader
as installed ODFs (`sta_content::odf::load_with`, CON-03). Conversion, class
handling (`classLabel`, `ordName`, `physicsFile`, `weaponN` / `weaponHardPointsN`,
`builditemN`) and the profile rules therefore cannot drift between the two.

`fixtures/content/` is the synthetic set the tests, bench scenes and CI use
when no install is present: the units and weapons of `sta_content::fake::db()`.
`sta_content::json5::load_dir("fixtures/content")` must give the same
`ContentDb::hash()` as `fake::db()`; a test pins it.

## Files

| File | Holds |
|---|---|
| `content.json5` | profile, feature switches, ODFs |
| `two_bases.json5` | a native map: `MapDef` as JSON5 (`load_map_str`) |
| `../models/fx_*.glb` | one GLB box per `basename` (`fx_cruiser` is CON-09's cube); regenerate the boxes with `python3 fixtures/models/generate_boxes.py` |

## `content.json5`

```json5
{
  profile: "native",            // native | fo4 | sta2classic | sta1classic (default native)
  features: { fire_arcs: true }, // as info.ini [Features]: over the profile's defaults
  odf: {
    fx_beam:     { classLabel: "cannon", wpnName: "Beam", ordName: "fx_beam_ord", range: 600, shotDelay: 1.5 },
    fx_beam_ord: { classLabel: "phaser", damageBase: 20, lifespan: 0.5 },
    fx_ship: {
      classLabel: "craft", unitName: "Ship", basename: "fx_cruiser",
      maxHealth: 800, maxShields: 400, shieldRate: 15,
      weapon1: "fx_beam", weaponHardPoints1: ["hp01", "hp02"],
    },
  },
}
```

- `odf` maps an ODF name to an object of ODF keys. Keys are written exactly as
  in the ODF (`docs/data/odf_keys.csv` lists them; the `sta64` column says
  which the loader reads) and values keep the ODF's units: seconds for
  `shotDelay`, `lifespan`, `buildTime`; per second for `shieldRate`; radians
  for `fireArc`.
- Value rendering: number as written, string quoted, `true`/`false` as `1`/`0`,
  array as space-separated words (`["hp01", "hp02"]` is `"hp01" "hp02"`).
- What an ODF is (unit, weapon, other) follows from its keys exactly as for
  installed ODFs: `wpnName` or a weapon class makes a weapon; a unit class or
  `unitName` a unit; ordnance and physics ODFs are not definitions but are
  found by `ordName` and `physicsFile`. `#include` between entries works.
- Unknown top-level keys are an error; ODF keys are not checked (as in ODFs).
  Problems the ODF loader would warn about come back as warnings.

### Keys beyond stock ODFs

The ODF loader reads these itself, so installed ODFs and JSON5 agree. Each is an
existing ODF key or an `STA64_` extension in the style of `STA64_queueLimit`.

| Key | Where | Meaning |
|---|---|---|
| `miner` | unit | Existing key. When present it decides whether the unit mines (`UnitDef::mines`): stock `freight.odf` says `miner = 1`, the Data Miner (`fdata.odf`, class `freighter`) says `miner = 0`. Without it a `freighter` mines. |
| `eventAttack` | unit | Existing key. Fills the attack acknowledgement sound. |
| `STA64_fireArc<N>` | unit | Half-width in radians of the fire arc of weapon slot `N` on this unit only. Stock ODFs limit a weapon wherever it is mounted (`restrictFireArc`, `fireArc`); `fake::db()` limits one beam on two units and not on a third. Wins over the weapon's own arc. |
| `STA64_duration` | weapon | Seconds the weapon stays active, when its ordnance gives no `lifespan` (point defence). Overrides the ordnance's `lifespan`. |

Not expressible, and not hashed, so not in the fixtures: the explosion sound
and the damage-effect list of `fake::db()`.

## Native maps

`MapDef` derives serde. `two_bases.json5` is `fake::two_bases()`: plain numbers
where the sim has `Fixed`, enums externally tagged (`kind: "Nebula"`,
`shape: {Sphere: {...}}`). The same encoding is the native map save format
(`json5::save_map`, CON-32); BZN bytes are written by `sta_bzn::write`.
