# STA64 TODO

Live working list. Milestones live in [docs/ROADMAP.md](docs/ROADMAP.md);
this file is what's being worked on now and what needs checking by eye.

Legend: `[ ]` to do · `[~]` in progress · `[x]` done · `[?]` done, needs your check in game/viewer

## Needs your check (morning)

Run with `--mod "STA2 Classic Mod" --map a2_borg01` and a couple of FO4 maps.

- [?] Nebulae no longer blown out (ScaleSOD no longer scales puff size) — `270c675`
- [?] Skybox shows for prefix skies like `mbgkl` (six faces); seams / orientation right? — `270c675`
- [?] Starfield visible; star size/brightness plausible (size is an estimate) — `270c675`
- [?] Planets textured (`<ground>1`/`<ground>2` + cloud shell); texture not mirrored/rotated — `270c675`
- [?] Ships' running lights / engine particles still sit on the hull after the ScaleSOD change (e.g. `fgalaxy`, ScaleSOD 2.78) — `270c675`
- [?] Asteroid fields show rocks, slowly orbiting/tumbling; size and spread look like the game (spread is an estimate) — ASTEROIDS
- [?] The editor's `dlight` arrow gizmo no longer shows; lighting direction matches the game on a map with dlights — ASTEROIDS

## Now (M2: a map you can fly in)

- [ ] Nebula look: class colour pulse on sim time, near fade, occlusion intensity
- [x] Asteroid fields: rocks from the engine's table (`master*`), count/radius/speeds from the ODF
- [x] Map lights aimed by `dlight` markers; `hidden = 1` editor markers not drawn
- [ ] Select units (click / drag box) in the map view
- [ ] Move orders through `sta_sim`, renderer interpolating between ticks
- [ ] RTS camera (edge scroll, tactical zoom limits from `RTS_CFG.h`)

## Later

- [ ] Borg lattice groups (`basic_borg_alpha`): lit or self-illuminated? (your call from the game)
- [ ] Star Trek: Legacy LOD (type 11) and effect (type 2) nodes; M3D 1.93 layout
- [ ] glTF/GLB models in the viewer and maps
- [ ] Mission scripts: integrated MMM Lua runtime ([docs/MISSIONS.md](docs/MISSIONS.md))
- [ ] Front end on Refit's 3D menu scene (M4)

## Done recently

- [x] BZN reader, text + binary; all 517 installed maps; object counts verified — `f10cc94`
- [x] Map view: objects placed, team colours, shared parts — `f10cc94`
- [x] Native M3D loading (no SOD conversion), glow maps from STL slot — `ad27a39`, `5287d6f`
- [x] Normal-map convention verified against Bevy's tangents — `6b12c12`
- [x] Team colours from `teamcolor.odf` — `4cea39a`
- [x] Damage-state nodes hidden on healthy craft — `5287d6f`
