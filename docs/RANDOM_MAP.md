# Random maps

`sta_content::random_map` makes the map the stock Armada II game makes for
"Random map" (CON-49): a square of 10,000 to 15,000 units, start locations on
the corners and edges with their own resources, then resources, black holes
and wormholes, asteroid belts and nebulae placed at random with clearances.
The module docs list the rules and where they differ from the original.

What the resources are is data (CON-50), so a mod can change it.

## `random_map.json5`

A mod or install puts the file at its root (beside `odf/`, `bzn/`). The
highest layer that has one supplies the **whole** table; files are not merged.
With no file, the built-in table stands: `fixtures/random_map/stock.json5` is
that table as a file, and the place to start from. A file that does not parse
is reported and the built-in table used.

```json5
{
  // The sets a map can be made from. A map uses one set for all its resources.
  sets: ["normal", { name: "frozen", weight: 2 }],
  // How many resources the resource phase places, at least and at most.
  count: [10, 30],
  resources: [
    {
      name: "dilithium",
      weight: 35,               // share of the resource phase
      clearance: 1000,          // clear distance to other objects
      start: {                  // one beside every start
        inward: [1000, 1000],
        centred: [-500, -500],
        sets: { normal: ["mdmoon"] },
      },
      sets: {
        normal: [{ odf: "mmooninf", weight: 1 }, { odf: "mdmoon", weight: 4 }],
        frozen: ["my_ice_moon_a", "my_ice_moon_b", "my_ice_moon_c"],
      },
    },
    // latinum, planets, a mod's biomatter, ...
  ],
}
```

### `sets`

A list of set names, each a name or `{ name, weight }` (weight 1 when not
given). A set is one look of the map's resources: `normal`, `frozen`, a mod's
own. The front end or a caller may ask for a set by name; otherwise one is
drawn in proportion to the weights. The first set is the fallback (below).

### `count`

`[least, most]` resources placed by the resource phase, apart from those
beside the starts. Default `[10, 30]`.

### `resources`

A list, in the order the shares are laid out in. Each entry:

| Key | Default | Meaning |
|---|---|---|
| `name` | required | The resource type: `dilithium`, `latinum`, `biomatter`. Only a label; what a node yields is its ODF's business. |
| `weight` | 1 | Share of the resource phase. `0` keeps the type out of the phase. |
| `clearance` | 0 | Clear distance to other objects, in units. |
| `clearance_radii` | 0 | Clear distance in multiples of the object's own radius; the larger of the two applies. |
| `required` | true | The type is on every map: when neither the starts nor the resource phase placed one, one more placement is tried. |
| `start` | none | Puts one node beside every start (below). |
| `sets` | required | Per set name, the ODFs a node of this type may be. |

A list of ODFs holds names (with or without `.odf`, any case) or
`{ odf, weight }`. Each time a node is placed one name is drawn in proportion
to the weights, so a set with several names gives maps that differ.

A resource need not have every set. On a map of a set it lacks, it uses the
first set it lists.

### `start`

| Key | Default | Meaning |
|---|---|---|
| `inward` | required | `[x, z]`: how far towards the middle of the map the node sits from the start, on each axis where the start is at the edge. |
| `centred` | `[0, 0]` | `[x, z]`: the offset on an axis where the start is in the middle (the edge starts), so nodes there sit either side of it. |
| `sets` | the resource's | Per set, the ODFs used beside a start, when they should differ from the type's general list (the stock table keeps inexhaustible moons away from the starts this way). |

## Rules that follow

- **Every resource type is present.** A type with a `start` is beside every
  start. Any `required` type that still has no node after the resource phase
  gets one more placement, which can only fail when the map has no room left.
- **Variety.** The set, each type drawn in the phase and each node's ODF are
  drawn from the seed, so the same seed, player count, table and set give the
  same map on every machine, and another seed another mix.
- **Names are checked.** `Table::check` reports every listed ODF the content
  does not have. A name that does not exist places nothing.
- Black holes, wormholes, asteroid fields and nebulae are not in the table
  yet; they are the stock objects.

## Trying a table

```sh
cargo run -p sta_content --example random_map -- <seed> <players> out.bzn <install root> [mod] [set]
```

prints the sets, the set used and the count of every object, and writes the
map as a BZN.
