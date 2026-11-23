# SOL menu scenes

Place `menu.sol` and an optional `menu.lua` in the active mod's `Menu/` folder.
The engine also looks in `RefitScene/Menu/` and at the mod root. Each SOL uses
the Lua file next to it in the same mount layer. The active mod takes
precedence over its parents. Within each layer SOL takes precedence over
`Menu/map.ini` and `RefitScene/Menu/map.ini`. If a scene cannot load, STA64 tries
the next candidate. Existing INI menus still work.

Use a BinaryMode v3021 SOL and give each placement a unique instance name.
STA64 takes ship and station ODFs, positions and headings from the placements.
It can also place one planet from a planet ODF. The fixture uses the
Federation scout ODF `fscout` and supplies no game artwork.

Names are matched without regard to ASCII case. Place waypoints using named
objects with the prefixes `menu_camera_`, `menu_target_`, or `menu_point_`.
These markers stay out of the scene. Without Lua, `menu_camera_main` is the
opening camera and `menu_target_main` is its look-at point. A camera with no
matching target looks along its SOL heading. Other suffixes define named views.

`menu.lua` returns a table. See [the example](Menu/menu.lua) for one camera
route and one ship route. In `views`, give each entry a `name` and a `camera`
waypoint list. An optional `target` list supplies look-at points, and `fov`
sets the vertical field of view in degrees. In `ships`, give each moving
object's `name` and its `path` list. Use the actor's name as its first
waypoint to start the route at its original placement.

A single waypoint stays still. Each leg takes an equal share of `seconds`,
which defaults to 10. The path stops at its final point or returns to its
first point when `loop = true`. Camera paths start when their view opens.
Ship paths start with the scene. The optional top-level `sky` names a sky
texture, `stars` sets the star count, and `blend` sets the view transition
time in seconds.

STA64 evaluates the Lua file once per scene load. It supplies no mission APIs.
The sample SOL is synthetic and native editor round-tripping remains
unverified. The scene uses its own named markers for paths.
