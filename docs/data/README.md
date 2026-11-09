# Data feature inventory

What Armada I, Armada II and Fleet Operations put in their text data files,
so STA64 can track what it reads and what it still ignores.

| report | contents |
|---|---|
| [odf_keys.md](odf_keys.md) ([csv](odf_keys.csv)) | every ODF key, per game, with classes, value kind, samples, STA64 ✓ |
| [odf_classes.md](odf_classes.md) | every `classLabel` and the keys its ODFs set |
| [techtree.md](techtree.md) | `.tt` line forms and FO's techtree `.xml` schema |
| [cfg.md](cfg.md) | `.cfg` keys (GUI layouts and engine tuning) |
| [aip.md](aip.md) | `.aip` AI player variables |

## Files scanned

| type | A1 | A2 | FO |
|---|---:|---:|---:|
| .odf | 495 | 1102 | 9355 |
| .tt | 27 | 39 | 3 |
| .xml | 0 | 0 | 3 |
| .cfg | 9 | 9 | 43 |
| .aip | 86 | 91 | 41 |

## Coverage

| | A1 | A2 | FO |
|---|---:|---:|---:|
| distinct ODF keys (STA64 ✓ / all) | 75 / 572 | 107 / 970 | 104 / 893 |
| ODF class labels | 82 | 122 | 76 |
| distinct CFG keys (✓ / all) | 0 / 364 | 2 / 443 | 2 / 474 |
| distinct AIP variables (✓ / all) | 2 / 61 | 3 / 67 | 3 / 62 |

Keys found only in one game: A1 64, A2 133, FO 256.

## ODF syntax beyond `key = value`

| | A1 | A2 | FO |
|---|---:|---:|---:|
| `# comment line` | 108 | 905 | 7 |
| `#include` | 284 | 706 | · |

Unparsed ODF line shapes (`w` word, `N` number), most common first:

| shape | count | example |
|---|---:|---|
| `table rows after `key = …` (`"name" value…`)` | 3650252 | `assets.zfs:xomega.odf: damageBase = … / "zmama.odf" 2` |
| `w <w> "…"` | 1158 | `events.odf: Sound <borg> "gvobcv77.wav"` |
| `}` | 219 | `events.odf: }` |
| `w{` | 218 | `events.odf: DefaultMarkerOnly{` |
| `w` | 169 | `events.odf: Table` |
| `[w]` | 75 | `assets.zfs:aster.odf: [GameObjectClass]` |
| `w "…"` | 59 | `events.odf: Text "VTune sampling turned on."` |
| `w w` | 14 | `events.odf: SoundPriority 2` |
| `w w w w w w w w w w w` | 7 | `events.odf: Marker radius 128 shrink 0.025 time 4 color 1 0 0` |
| `/w w w w` | 2 | `assets.zfs:shieldi.odf: /energy cost per second` |
| `/ /*************************************` | 2 | `assets.zfs:fresear2.odf: / /*********************************************************` |
| `w { #w w w w w w w w w` | 1 | `events.odf: InvalidEvent { #this event should be the first in this file` |
| `w #w w w, w` | 1 | `events.odf: Marker #uses default radius, etc.` |
| `/ w w w w` | 1 | `odf/other/FluidicNebula.odf: / Runtime class of object.` |
| `/` | 1 | `odf/stations/8472_Mine.odf: /` |

## ODF keys followed by table rows

A key whose value is followed by indented `"name" value…` rows (FO's
`validtargets`/`supplyclasses` ODF tables, A1/A2 ship-name lists). Count = files.

| key | A1 | A2 | FO |
|---|---:|---:|---:|
| `damagebase` | 7 | · | 2526 |
| `hitchance` | 3 | 6 | 2521 |
| `possiblecraftnames` | 81 | 165 | 1480 |
| `validtargets` | · | · | 129 |
| `disableweaponspercent` | · | · | 49 |
| `disableweaponstime` | · | · | 49 |
| `disableenginespercent` | · | · | 34 |
| `disableenginestime` | · | · | 34 |
| `hulltargethardpoints` | 15 | 14 | · |
| `criticaltargethardpoints` | 10 | 13 | · |
| `repairrate` | · | · | 22 |
| `enginestargethardpoints` | 10 | 9 | · |
| `lifesupporttargethardpoints` | 10 | 9 | · |
| `weaponstargethardpoints` | 10 | 9 | · |
| `sensorstargethardpoints` | 10 | 8 | · |
| `shieldgeneratortargethardpoints` | 10 | 8 | · |
| `beneficialbuild` | · | · | 16 |
| `disablesensorspercent` | · | · | 15 |
| `disablesensorstime` | · | · | 15 |
| `disablelifesupportpercent` | · | · | 13 |
| `disablelifesupporttime` | · | · | 13 |
| `weaponhardpoints#` | 9 | 2 | · |
| `supplyclasses` | · | · | 3 |
| `commandhotkeys` | · | 1 | 1 |
| `commandnames` | · | 1 | 1 |
| `drainspecialenergypercent` | · | · | 1 |

## Syntax notes

- FO ODFs never `#include`; every ODF is flat. A1/A2 chain bases with `#include`.
- `#` at the start of a line is a comment in A1/A2 ODFs (only real directives count above).
- `events.odf` (A2/FO) is its own block format: `Name{ … }` with `Sound <race> "x.wav"`,
  `Text`, `Marker …`, `SoundPriority N` lines. It needs a dedicated parser.
- A1 `aster.odf`-style files carry BZ-engine `[GameObjectClass]` section headers.
- AIP build lists are C-initialiser rows (`"bor_assemblerZ", 1,0, -1, 1,`) under an array
  declaration; see the *Other line shapes* table in [aip.md](aip.md).
- FO techtrees exist twice: `.tt` (A2 style) and `.xml` (capacity lists, requirement
  trees with `mustNotHave`, ally races, caps). FO reads the XML.
- `.tt` entry = `<odf> <N> <prereq>×N`; A2 mission techtrees `#include` a base and
  set items to `-1`.

## Sources and regeneration

- Armada I: retail install; most ODFs and all techtrees are in `assets.zfs`
  (ZFS v1, 100-entry blocks, 16-char names, whole words XOR'd with the 4-byte header key,
  read by the script). `addon/` ODFs are loose.
- Armada II: retail 1.1 install, loose files.
- Fleet Operations: loose `Data/` plus `ODF.fpq` (extracted with `sta_fpq`'s
  example). `Data/Mods/` (third-party mods), backups and saves are excluded.

```sh
cargo run -p sta_fpq --release --example fpq -- "$FO/Data/ODF.fpq" extract /tmp/fpq/odf
python3 tools/data_inventory.py --out docs/data --repo . \
    --game "A1=$A1" --game "A2=$A2" --game "FO=$FO/Data,/tmp/fpq"
```
