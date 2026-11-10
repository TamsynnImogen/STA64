# STA64 TODO

Live working list. Milestones live in [docs/ROADMAP.md](docs/ROADMAP.md);
this file is what's being worked on now and what needs checking by eye.

Legend: `[ ]` to do · `[~]` in progress · `[x]` done · `[?]` done, needs your check in game/viewer

## Needs your check

- [?] Launch points (SIM-16): `cargo run -p sta_bench --bin sta-bench -- run launch` passes; `STA64_TEST_INSTALL=<root> STA64_TEST_MOD=<mod or empty> cargo test -p sta_content --test launch_install -- --nocapture` (and `STA64_TEST_DIR=<dir>` for A1/A2) prints how many builders launch from their `buildHardpoint` node and how many take the fallback; in a skirmish a built ship appears at the yard's build bay facing out and drives clear of the yard
- [?] Context orders (UI-14): start a skirmish (command above), select a freighter and right-click a moon: it flies there, mines and delivers to the outpost (cursor turns to a pointing hand over the moon); select a warship and right-click an enemy ship: it attacks (cursor is a crosshair); warships on a moon just move. Rally from builders waits for SIM-17.
- [?] UI text (UI-09): `cargo test -p sta64 no_literal_ui_strings` fails on any literal handed to a UI text helper; the language is the `Language` resource (`english` default), tables come from `Dynamic_Localized_Strings.h` plus `sta64_<language>.strings` (`key<TAB>text`) in the install or mod
- [?] Front-end menu scene (UI-06), until INT-M4 puts it under the screens: `STA64_MENU_SCENE=1 STA64_MENU_VIEWS=klingon@6 cargo run -- --headless --clock bench --install-root <FO install> --frames 150,600 --frame-size 960x540 --frame-dir /tmp/menu`: tick 150 is the `main` view (a Federation fleet in orbit over a lit planet, stars), tick 600 the `klingon` view with five Klingon ships decloaked (they are invisible in the main view); without `--install-root` the same scene is boxes on the fake content. `STA64_MENU_DEPART=<seconds>` sends the view's ships to warp (front ship first; no tunnel or flash drawn yet). `STA64_BENCH_OUT=<dir>` writes the `render.menu.*` marks. `STA64_GPU_TESTS=1 STA64_TEST_INSTALL=<FO install> cargo test -p sta64 --test menu_scene`
- [?] Skirmish (INT-M3): `cargo run --release -- --install-root <FO install> --mod "Fleet Ops 4.0" --skirmish blueamber --race federation --opponent-race klingon` opens the map with your outpost at your start and the HUD on the bottom edge (info panel, 5x3 grid, resources strip, radar); fog shows only what your outpost sees; select the outpost: the grid has Build, the build page lists what the tech tree allows (locked buttons say why); Federation music plays and clicking units plays their select and order voices; the AI opponent starts mining and building ships within a minute (log lines `ai: command ...`); destroying every enemy craft and station shows a VICTORY banner, a notice and the log line `skirmish over`; `--opponent idle` leaves the other seat doing nothing; a mobile builder's build asks for a click on the map for the site (Esc cancels)
- [?] Skirmish bench scenario (INT-M3): `cargo run -p sta_bench --bin sta-bench -- run skirmish` passes (the scripted fleet destroys the idle opponent and player 2 is eliminated at tick 4368)
- [?] Start units (INT-M3): `STA64_TEST_INSTALL=<root> STA64_TEST_MOD=<mod> cargo test -p sta64 install_skirmish -- --nocapture` prints each seat's start units and stock and what the AI built in five minutes
- [?] Map on seams (INT-M2): `cargo run -- --install-root <FO install> --map <name>` plays as before (ships move on right-click, fight when ordered to attack); the info panel's `sim:` line shows the `MapDef` counts. `--seat 0` plays seat 0 (orders for its units only, its fog and shroud drawn). `STA64_TEST_INSTALL=<root> STA64_TEST_MOD=<mod or empty> cargo test -p sta64 map::tests -- --nocapture`
- [?] Map conversion (CON-07): `STA64_TEST_INSTALL=<root> STA64_TEST_MOD=<mod or empty> cargo test -p sta_content --test map_install -- --nocapture` (and `STA64_TEST_DIR=<dir>` for a bare game folder) prints, per install, maps converted, objects placed or counted as regions/planets/lights/props, and multiplayer maps with starts; no `PROBLEM` lines
- [?] Map corpus (TOOL-05): `STA64_TEST_INSTALL=<root> STA64_TEST_MOD=<mod or empty> cargo test -p sta_bench --test corpus -- --nocapture` (and `STA64_TEST_DIR=<dir>` for a bare game folder) prints maps loaded, ticked 300 and repeated with the same digest; no `PROBLEM` lines
- [?] Skirmish AI (AI-02): `cargo test -p sta_ai skirmish -- --nocapture` passes and prints `skirmish beats idle on two_bases at 21.84 sim minutes`; no in-game view yet (INT-M3 seats it with `Control::Ai("skirmish")`)
- [?] Nebula extras (RND-05), `cargo run -- --map <name>` on a map with the class: latinum's core pulses and its ring about the core tints in patches that go round teal, violet, pink and green; mnebula7 (crimson) flickers with lightning, a flash lighting the cloud about it for under a second; mnebula10's gas wanders slow loops (swirl); other nebulae look as before
- [?] Hull lighting with Refit's look (RND-06): in the model viewer or a map, ships are lit by the key and fill with a highlight, rim light, team colour, glowing windows and (Borg) normal maps; **H** switches to the previous Bevy lighting and back, to compare; T still changes the team colour in the viewer
- [?] Team colour on the right parts (RND-17), `cargo run -- --map <name>` as Federation with a construction ship (FO4): its `team*` hull groups (80% of the faces) take the player colour and the lit windows keep theirs; stations of every race are almost untinted (0-2%); T in the viewer changes the colour. If the Federation ship should be grey instead, the team colours (built-in when `teamcolor.odf` is absent) are what to change
- [?] Post FX (RND-14), opt-in: `cargo run -- --map <name> --postfx bloom,smaa` (names: bloom smaa fxaa vignette chromatic exposure aces reinhard cinematic); F8 cycles off / bloom / cinematic; with no flag the look is exactly as before
- [?] Selection overlays (RND-08): `cargo run -- --map <name>`, select ships: team-coloured ring, green hull bar (and blue shield bar if any) above, faint line to the move target after a move order; all gone on deselect
- [?] Impact FX (RND-10): in a map fight, shield hits flash a blue shell on the struck side, hull hits throw sparks, destroyed ships explode (flash, shell, debris), ships under half hull trail grey smoke (two streams under a quarter), none when cloaked; all fades and nothing lingers
- [?] Effects touching despawned entities (RND-19): play a skirmish for a few minutes of combat; the log shows no `Entity despawned: The entity with ID ... is invalid` warnings
- [?] Bay doors and build arms by event (RND-18): in a skirmish as Federation (FO4), the construction ship (`fed_mediterranean`) holds its two bay panels shut while idle and plays them open and shut once when it starts a build and again when a ship leaves; rotating dishes and lights keep turning
- [?] Weapon FX in the map view once a sim with weapons is linked: beams fade over their duration, torpedo spheres fly to the target, short muzzle flash at the shooter (centre of the ship for now, no hardpoint positions yet); all gone shortly after firing stops
- [?] Audio (AUD-01): `cargo run -- --audio-test` plays four rising tones (music, sfx, voice, ui buses) at startup; F9 repeats them; no sound device logs a warning and runs silent
- [?] UI sounds (AUD-05): `STA64_TEST_INSTALL=<root> STA64_TEST_MOD=<mod> cargo test -p sta64 audio::ui` passes (every click/hover/error/chat name resolves in the install); clicks are audible with `--install-root` (CON-10 resolver). Every installed WAV/MP3/OGG decodes: `STA64_TEST_INSTALL=<root> STA64_TEST_MOD=<mod> cargo test -p sta_assets audio -- --nocapture`
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
- [?] Radar (UI-05), until the HUD wires it in: `STA64_UI_RADAR=1 cargo run -- --headless --clock bench --scene crates/sta64/tests/fixtures/radar.ini --frames 0 --frame-size 1280x720 --frame-dir /tmp/radar`; the LCARS panel sits bottom left with Ships / Stations filter dots above a dark square; own ships green, enemies red, moon dim grey, bases square, only what the player's sensors see
- [?] HUD (UI-04), until INT-M3 wires it into a game mode: `STA64_UI_HUD=1 STA64_UI_HUD_SELECT=yard cargo run -- --headless --clock bench --scene bench/scenes/hud.ini --frames 0 --frame-size 1280x720 --frame-dir /tmp/hud` (add `STA64_UI_HUD_PAGE=build` for the build page, `STA64_UI_HUD_SELECT=flagship,escort,escort2` for a group, `STA64_UI_HUD_CFG=fixtures/gui/fo/gui_federation16x9.cfg` for a race cfg, `--frame-size 1024x768` for 4:3): resources strip top centre; radar bottom left; the 5x3 command grid beside it (above it at 4:3); the info panel at the bottom in three heights (one object low, a yard middle, a group high); build buttons are the yard's list, greyed when the tech tree locks them; the layout is printed as a `hud layout:` line
- [?] HUD frame test and bench (UI-04): `STA64_GPU_TESTS=1 cargo test -p sta64 --test hud_frame`, and `STA64_UI_HUD=1 STA64_UI_HUD_SELECT=yard sta-bench run ui-hud-layout --tier 2 --repeat 2`; the radar's blips are back in `STA64_UI_RADAR=1` frames (the bench link spoke for player 0 and hid them)
- [?] Front-end screens (UI-07), until INT-M4 wires them in: `STA64_UI_SCREEN=setup cargo run -- --headless --clock bench --scene bench/scenes/hud.ini --frames 0 --frame-size 1280x720 --frame-dir /tmp/screen` (also `main`, `single`, `map`, `settings`, `mods`): a centred LCARS panel over the scene; Instant action lists a row per start slot (Player / Computer / Idle, side, team), the map with Select map, Launch and Previous menu; the screen is printed as a `screen:` line
- [?] Screens frame test (UI-07): `STA64_GPU_TESTS=1 cargo test -p sta64 --test screens_frame`; lavender buttons and a gold title in the middle half of the frame, the scene untouched either side, each screen its own picture
- [?] Radar clicks and filters: dots pick all / allies / enemies / neutral, the name cycles; a click on the square sends `RadarClick` (camera move / `GO_WARP` wiring is the HUD's, INT-M3)

- [?] Rally controls (UI-14 follow-up): select a finished stationary yard/outpost and right-click empty ground or a unit to set its rally target; new ships use that target. A mobile constructor still moves on right-click, as do mixed yard/ship groups. Clicking allied or neutral ships with a warship moves rather than attacking. Rally currently has no voice acknowledgement.

- [?] Capability grid (UI-13): select a stationary yard (Build/Rally, no movement/formation buttons), a warship (Move/Attack/Stop/formations), or a freighter (Move/Mine); mixed groups show shared actions. Click a target button then the map/unit; Esc or right-click cancels. Grid Move keeps ship spacing and waypoint lines. Known allies/neutral relations cannot be targeted by Attack. Cell placement is a fallback; native visual layout still needs checking.

- [?] Stop capability: select a stationary yard/station or an engine-disabled ship and check that Stop remains in the top grid while Move and formations stay unavailable; Stop clears the selected actors' orders. Mixed owned craft/station groups retain Stop; resources and dying units do not gain it.

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
- [?] Fog of war and shroud (RND-13): the draw is ready but the map view does not insert `FogBounds` or a `LocalPlayer` yet, so nothing shows in `--map`; `sta64 --headless --clock bench --frames 0,60 --frame-dir out --scene bench/scenes/fog.ini` (needs a GPU) shows unseen ground dimmed, never-seen ground black, round edges, and the discs following the sensors (compare `fog-off.ini`)

- [ ] Refit map objects, in order:
  - [x] Planet look (Lighting.asi via Refit's world_planet shader) — 4fc4a88
  - [x] Recipe surfaces (Surfaces.asi): planets PB_CLSS*, moons Mdmoon, asteroid rock Master1/2 — 7c8c421
  - [?] Moon cracks' ravine parallax (Refit rock shader), moon glow shell (dark, Refit's default) and nimbus (off, Refit's default) — RND-05, #141
    - [?] Check in `--map` on a dilithium moon: tilting the view, the cracks' walls should hide the blue light at an angle; no black or banded cracks; the moon shows no round halo
    - [?] Check the moon's glow shell: it should not show as a black or blue ball around the moon
  - [x] Volumetric nebulae (Nebulae.asi): ray-marched gas fields per class — e4eb7c7
  - [?] Nebulae extras: lightning (mnebula7), ring/pulse (latinum), swirl (mnebula10) — RND-05, #141
  - [ ] Nebulae extras still to do: ship shadows (needs the key light's shadow map in the march; proposed as its own package on #141), baked volume + temporal smoothing for speed
  - [x] Background objects (suns, wrecks, gates) drawn about the eye — dee4e4b
  - [x] Planet glow lights and nebula lights (Refit `lights.rs`) — 8a95cee
  - [ ] Planet city lights (need colony population)
- [?] Nebula billboards meanwhile: near fade, occlusion intensity — RND-05, #141
  - [?] Check on a map with a stock nebula (no gas recipe): puffs fade out as the camera flies into them (no hard near-plane cut) and a nebula seen through another is dimmer, not doubly bright. The fade and occlusion rules are our reading, not stock's formula
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
- [ ] Notices and tooltips look right: in a bench match a destroyed own unit shows a red "<Unit> destroyed" line at the top centre that fades after 5 s, an enemy loss a plain one, and a `Tooltip` node shows its text 0.35 s after the pointer rests ([notices.rs](crates/sta64/src/ui/notices.rs), UI-10)
- [ ] In-match text (UI-15): start a skirmish as a non-default race and kill a ship; the notice appears top centre, clear of the info text at the top left, and the info panel's race field names the race you play, not the map's default ([notices.rs](crates/sta64/src/ui/notices.rs), [map/mod.rs](crates/sta64/src/map/mod.rs))

## Done recently

- [x] BZN reader, text + binary; all 517 installed maps; object counts verified — `f10cc94`
- [x] Map view: objects placed, team colours, shared parts — `f10cc94`
- [x] Native M3D loading (no SOD conversion), glow maps from STL slot — `ad27a39`, `5287d6f`
- [x] Normal-map convention verified against Bevy's tangents — `6b12c12`
- [x] Team colours from `teamcolor.odf` — `4cea39a`
- [x] Damage-state nodes hidden on healthy craft — `5287d6f`
- [ ] Construction, cloak FX: a station under construction welds and firms up, `Built` flashes; a cloaked own unit goes translucent and back (RND-11)
- [?] Resource moons (RND-16), `cargo run -- --map <name>` on an FO4, FO or DW map with `trigger_dilithium` / `trigger_tritanium` markers: each draws a dilithium or tritanium moon model (one of four variants, as the game's replace weapon lists them), never the editor letter; it is neutral, shows no selection ring and plays no ship voice; A2 maps (`mdmoon`) look as before
- [?] Language packs (CON-11): `cargo test -p sta_content --lib strings -- --nocapture` lists a string and label count for every installed game and mod (A1, A2, A2 stock, Dominion War, FO, FO Roots, Fleet Ops 4.0, STA2 Classic) and passes; the UI tooltip "Double click to return to strategic mode." still shows in-game
