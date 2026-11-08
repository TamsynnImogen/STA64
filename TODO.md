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
- [?] Asteroid fields show rocks, slowly orbiting/tumbling; size and spread look like the game (spread is an estimate) — aa70e99
- [?] Ships: click / drag-box select, right-click move, Space stop; they turn and arrive smoothly; speeds feel like the game (accel/turn-at-rest interpretation is ours) — a3f2076
- [?] Map camera now: middle-drag orbit, WASD/edge pan, wheel zoom (left/right are select/order); edge scroll not annoying? — a3f2076
- [?] The editor's `dlight` arrow gizmo no longer shows; lighting direction matches the game on a map with dlights — aa70e99

## Now (M2: a map you can fly in)

- [ ] Nebula look: class colour pulse on sim time, near fade, occlusion intensity
- [x] Asteroid fields: rocks from the engine's table (`master*`), count/radius/speeds from the ODF
- [x] Map lights aimed by `dlight` markers; `hidden = 1` editor markers not drawn
- [x] Select units (click / drag box) in the map view
- [x] Move orders through `sta_sim` (deterministic, digest pinned), renderer interpolating between ticks
- [x] RTS camera: edge scroll (`SCROLL_BORDER_WIDTH`), focus/zoom kept to the map — bba5143
- [x] Mission resolution per docs/MISSIONS.md; `sta64 --mission-report` — 31ae0dd
- [x] MMM Lua API inventory (docs/MMM_API.md: 478 names, 141 used by installed scripts)

## Later

- [ ] Borg lattice groups (`basic_borg_alpha`): lit or self-illuminated? (your call from the game)
- [ ] Star Trek: Legacy LOD (type 11) and effect (type 2) nodes; M3D 1.93 layout
- [ ] glTF/GLB models in the viewer and maps
- [ ] Mission scripts: integrated MMM Lua 5.3 runtime, most-used API first ([docs/MISSIONS.md](docs/MISSIONS.md), [docs/MMM_API.md](docs/MMM_API.md))
- [ ] Front end on Refit's 3D menu scene (M4)

## Done recently

- [x] BZN reader, text + binary; all 517 installed maps; object counts verified — `f10cc94`
- [x] Map view: objects placed, team colours, shared parts — `f10cc94`
- [x] Native M3D loading (no SOD conversion), glow maps from STL slot — `ad27a39`, `5287d6f`
- [x] Normal-map convention verified against Bevy's tangents — `6b12c12`
- [x] Team colours from `teamcolor.odf` — `4cea39a`
- [x] Damage-state nodes hidden on healthy craft — `5287d6f`
