# Original menu choreography showcase

This original SOL menu scene uses the local test ODF identifiers `fscout`,
`fgalaxy`, and `fbase`. The escort starts at `(-150, 0, -40)`, the flagship
at `(80, 0, 35)`, and the bastion at `(5, 0, 205)`. Most cameras sit within
a few hundred map units of the group. The campaign view starts farther out.

## Mounting

Mount `Menu/menu.sol` and `Menu/menu.lua` together in the active mod's `Menu/`
directory. They must resolve from the same mount layer. The fixture uses a
BinaryMode version-3021 SOL. Route marker placements are named SOL objects and
are hidden by the menu-scene loader; the three non-marker ODF placements are
the actors.

The `screens` table maps stable menu keys to authored view names. The sample
opens on `opening`, uses `planning` for Instant Action, and shows `dock` in the
mod manager. The `campaigns` table binds a campaign ID to a view. The
Federation binding selects `campaign` for its mission and briefing screens.

Screen keys are `main`, `single_player`, `setup`, `advanced`, `map_select`,
`settings`, `mods`, `missions`, `briefing`, `debrief`, `multiplayer`,
`host_game`, `join_game`, and `lobby`. A view named after the screen also
works without a binding. Advanced and map selection inherit their parent
shot by default. Briefings and debriefs can inherit the mission shot, and
the lobby and room screens can inherit the multiplayer shot.

## Editing and regenerating routes

Edit the placement coordinates and headings in `author_menu.py`'s `placements`
and `marker_rows` lists. The escort and flagship routes begin at their SOL
positions, then visit their `menu_point_*` markers. The bastion is static.
Each camera has a three-point orbit and a parallel three-point look-at route.
Change the corresponding name lists, `seconds`, `loop`, or `easing` in
`Menu/menu.lua` to alter choreography. Use `easing = 'smooth'` to ease in and
out of each leg. The default `easing = 'linear'` keeps the speed constant
along each leg.

Run `python3 fixtures/menu-showcase/author_menu.py` from the repository root,
or run `python3 author_menu.py` from this directory. It writes only
`Menu/menu.sol` beside the Lua file.
