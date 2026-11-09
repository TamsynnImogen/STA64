# STA64 TODO

Live working list. Milestones live in [docs/ROADMAP.md](docs/ROADMAP.md);
this file is what's being worked on now and what needs checking by eye.

Legend: `[ ]` to do · `[~]` in progress · `[x]` done · `[?]` done, needs your check in game/viewer

## Needs your check

- [?] Audio (AUD-01): `cargo run -- --audio-test` plays four rising tones (music, sfx, voice, ui buses) at startup; F9 repeats them; no sound device logs a warning and runs silent
- [?] Ship movement smooth now (no weaving); FPS shown top right (try `cargo run --release` if it's low) — 3e748b3
- [?] Planets with Refit's look: terminator, dusk band, limb haze, water glint, cloud shell, halo past the limb — 4fc4a88
- [?] Recipe planets (class D/H/J/K/L/M) switch to detailed generated ground with relief, drifting clouds and cloud shadows a few seconds after a map loads (first time only; then cached) — 7c8c421
- [?] Asteroids and dilithium moons with generated rock (craters, relief, highlights, rim); moons' cracks glow blue — 7c8c421
- [?] Movement smoothness after the sprite-material fix — 19b7300
- [?] tutorialgameplay (FO4): no more grey wash (starfield clouds were sky-sized) and no giant blue disc; FO's background planets (`map_planet_class*`) sit where the map puts them but never cover ships; suns still behind everything — b6bc4f8
- [?] FO background planets (`map_planet_class*`, e.g. banana split, tutorialgameplay): ground texture visible under a thin, lit cloud shell, not a pale solid ball — 3e24321
- [?] Strategic icons: zoom out and ships/stations turn into solid team-colour silhouettes (fade in as they get small, ships turn with heading); Tab cycles auto/always/off; click or box-select works on icons, selected icons get a green ring — a84f661
- [?] Starfield star size (estimate: ODF `.scale` = world units seen from 5k): point stars a few px, FO star clouds ~6° — b6bc4f8
- [?] Ships near a planet's day side / inside nebulae take their coloured light (planet glow, nebula lights); not overpowering? — 8a95cee
- [?] Nebulae as volumetric gas (Mutara purple/pink etc.) instead of billboards; ships inside show correctly; FPS when zoomed into a big nebula — e4eb7c7
- [?] R0-3 carve (no visual change intended): `--map` as before — ships select (click, box, Shift), right-click move keeps formation, Space stops, motion smooth, selection rings and waypoint line show, FPS readout, strategic icons green when selected

## Checked earlier (morning list)

Run with `--mod "STA2 Classic Mod" --map a2_borg01` and a couple of FO4 maps.

- [?] Nebulae no longer blown out (ScaleSOD no longer scales puff size) — `270c675`
- [?] Skybox shows for prefix skies like `mbgkl` (six faces); seams / orientation right? — `270c675`
- [?] Starfield visible; star size/brightness plausible (size is an estimate) — `270c675`
- [?] Planets textured (`<ground>1`/`<ground>2` + cloud shell); texture not mirrored/rotated — `270c675`
- [?] Ships' running lights / engine particles still sit on the hull after the ScaleSOD change (e.g. `fgalaxy`, ScaleSOD 2.78) — `270c675`
- [?] Asteroid fields show rocks, slowly orbiting/tumbling; size and spread look like the game (spread is an estimate) — aa70e99
- [?] Ships: click / drag-box select, right-click move, Space stop; they turn and arrive smoothly; speeds feel like the game (accel/turn-at-rest interpretation is ours) — a3f2076
- [?] Map camera now: middle-drag orbit, WASD/edge pan, wheel zoom (left/right are select/order); edge scroll not annoying? — a3f2076
- [?] FO4 maps now show Dominion/other ships, defence platforms, corvettes and yards that were missing (models found via ODF `basename`) — bed098d
- [?] The editor's `dlight` arrow gizmo no longer shows; lighting direction matches the game on a map with dlights — aa70e99

## Now (M2: a map you can fly in)

- [ ] Strategic view, next: stack overlapping icons into one fleet icon with a count (Sins-style); grid/height bars at strategic zoom (`cfgSTRATEGIC_GRID_RENDER`)
- [x] Custom SOD blend names mapped in `render/materials.rs` (table-driven; our reading of the effect names, not engine source; unknown names log once) — #80
- [?] Ships with weapon/effect parts (`wtractor`, `whobber`, `wtranscoil`, `xcritical05`) show them as additive glow, not plain opaque — #80

- [ ] Refit map objects, in order:
  - [x] Planet look (Lighting.asi via Refit's world_planet shader) — 4fc4a88
  - [x] Recipe surfaces (Surfaces.asi): planets PB_CLSS*, moons Mdmoon, asteroid rock Master1/2 — 7c8c421
  - [ ] Moon cracks' ravine parallax (Refit rock shader), moon glow shell and nimbus
  - [x] Volumetric nebulae (Nebulae.asi): ray-marched gas fields per class — e4eb7c7
  - [ ] Nebulae extras: lightning (mnebula7), ring/pulse (latinum), swirl (mnebula10), ship shadows, baked volume + temporal smoothing for speed
  - [x] Background objects (suns, wrecks, gates) drawn about the eye — dee4e4b
  - [x] Planet glow lights and nebula lights (Refit `lights.rs`) — 8a95cee
  - [ ] Planet city lights (need colony population)
- [ ] Nebula billboards meanwhile: near fade, occlusion intensity
- [x] Asteroid fields: rocks from the engine's table (`master*`), count/radius/speeds from the ODF
- [x] Map lights aimed by `dlight` markers; `hidden = 1` editor markers not drawn
- [x] Select units (click / drag box) in the map view
- [x] Move orders through `sta_sim` (deterministic, digest pinned), renderer interpolating between ticks
- [x] RTS camera: edge scroll (`SCROLL_BORDER_WIDTH`), focus/zoom kept to the map — bba5143
- [x] Mission resolution per docs/MISSIONS.md; `sta64 --mission-report` — 31ae0dd
- [x] MMM Lua API inventory (docs/MMM_API.md: 478 names, 141 used by installed scripts)
- [x] Lua 5.3 dry run: all 252 installed mission scripts (FO4, STA1/STA2 Classic) load with `include()` and luabind `class`, 0 errors

## Later

- [ ] Borg lattice groups (`basic_borg_alpha`): lit or self-illuminated? (your call from the game)
- [ ] Star Trek: Legacy LOD (type 11) and effect (type 2) nodes; M3D 1.93 layout
- [ ] glTF/GLB models in the viewer and maps
- [ ] Mission scripts: integrated MMM Lua 5.3 runtime, most-used API first ([docs/MISSIONS.md](docs/MISSIONS.md), [docs/MMM_API.md](docs/MMM_API.md))
- [ ] Front end on Refit's 3D menu scene (M4)
- [ ] LCARS widget gallery looks right: `cargo run -p sta64 -- --ui-gallery` (pills, elbows, hover/press, slider drag); screenshot it as the local baseline ([STYLE.md](crates/sta64/src/ui/theme/STYLE.md))

## Done recently

- [x] BZN reader, text + binary; all 517 installed maps; object counts verified — `f10cc94`
- [x] Map view: objects placed, team colours, shared parts — `f10cc94`
- [x] Native M3D loading (no SOD conversion), glow maps from STL slot — `ad27a39`, `5287d6f`
- [x] Normal-map convention verified against Bevy's tangents — `6b12c12`
- [x] Team colours from `teamcolor.odf` — `4cea39a`
- [x] Damage-state nodes hidden on healthy craft — `5287d6f`
