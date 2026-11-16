# STA64 TODO

Live working list. Milestones live in [docs/ROADMAP.md](docs/ROADMAP.md);
this file is what's being worked on now and what needs checking by eye.

Legend: `[ ]` to do · `[~]` in progress · `[x]` done · `[?]` done, needs your check in game/viewer

## Needs your check
- [?] LUA-40 read-only MMM paths: check an external converted mission's optional camera-path lookup with populated and absent paths. Headless tests cover `Path.exists`, `.count`, copied `.points` and authored-label camera handoff; full cinematic playback and mission fidelity still need acceptance.
- [?] CFG multi-selection tiers: check selections of 2, 16, 17 and 30 units in Classic and FO4. The normal wireframe slots show names through 16; the compact wireframe slots show no captions from 17. Wireframes follow their cfg rectangles and retain full hover tooltips. This supersedes the earlier over-eight small-tile fallback note.
- [?] Multi-selection names: select two construction ships in Classic and FO4 (build them first if needed). Captions should use a compact sprite font in their cfg name areas, with full names retained and clear of the hull bars. Check at 1080p and 720p, resize, and compare a selection of more than eight ships (small tiles retain their existing icon-area fallback).
- [?] Graphics Options layout: compare the display-device/resolution/anti-aliasing row, renderer and monitor rows, checkbox columns and detail row against the supplied reference. Check compact and ultrawide windows and the in-game panel; MSAA, shadows, bloom and nebula detail should apply and persist. F8 effects changes must appear in Bloom and remain intact when changing MSAA. GPU/backend/monitor fields report the active setup; gamma stays a system display setting.
- [?] Engine measurements open point: on stock A2 1.1, an idle ship on **low** movement autonomy assisted an attacked ally when the autonomy was set through the engine setter. Set low autonomy with the HUD's autonomy buttons and check whether it still moves in to help; SIM-36 waits on this
- [x] Crisp menu text (fix/ui-sprite-font-crisp): on the 3440x1440 monitor open Single player in a Fleet Ops / Roots match and in Classic. The button labels and the title should be in the mod's sprite font with even letter spacing ("Federation campaign"), the title inside its bar, and on the stock skins the labels centred and light on the dark pills. Check also at 1920x1080 and with a larger window scale. Kit text now picks a font and a whole multiple so glyph texels land on whole screen pixels; HUD text is unchanged.
- [?] Engine measurements open point: on stock A2 1.1, an idle ship on **low** movement autonomy assisted an attacked ally when the autonomy was set through the engine setter. Set low autonomy with the HUD's autonomy buttons and check whether it still moves in to help; SIM-36 waits on this ([ENGINE_MEASUREMENTS.md](docs/research/ENGINE_MEASUREMENTS.md))

- [x] Hidden start helpers (fix for the "AI Pod" / "Species 8472 Starting Pod" / "Federation Faction" report): start a Roots skirmish as Federation. No grey squares near the starbase, nothing extra on the minimap, and clicking or box-dragging round the start selects only the starbase, freighter, corvette and two construction ships (FO 4.0: only the outpost). A Roots AI opponent still builds a starbase (`fbase` needs its hidden `fed_effectsS`). `STA64_TEST_INSTALL=<FO Roots dir> cargo test -p sta_sim --test start_helpers_install -- --nocapture` passes (base, Fleet Ops 4.0, STA2 Classic). What the pods do after tick 0 (expiry, AI personality and unit-cap pods, Species 8472's starting ships) waits for SIM-28; [notes](docs/START_HELPERS.md).

- [x] Command bar on ultrawide (fix/hud-command-bar-ultrawide): on the 3440x1440 monitor in a Fleet Ops / Roots match, select a ship: the row of command buttons should now start at the minimap panel's right edge instead of overlapping it. Check with and without a selection, and that 16:9 and Classic look as before. At exactly 16:9 the Roots sheet still puts the first button column about 87 canvas units inside the minimap panel's right part (its authored `popupPaletteXA = 264` against a 351 wide panel); that is left as authored until compared with the original game.
- [x] TOOL-14 playtest capture: start a real skirmish (any install), give an order or two, select a few ships, then press Ctrl+F12. A "Capture saved: ..." notice should appear and the game carry on; the folder is under `~/.local/share/sta64/captures/` (or `$STA64_CAPTURE_DIR`) with `capture.md` listing your selection, the mod mount and the build commit. Then `sta-bench capture check latest` (after `cargo build -p sta_bench`) should print PASS with the tick and digest. Try it after a few minutes of play with the AI fighting, and on an FO 4.0 and a Classic mount; report a FAIL with the folder name. F12 alone must do nothing.

- [?] SIM-23 pathfinding: in a fresh skirmish on a map with nebulae and planets (e.g. a stock A2 map with a Metreon/fluidic nebula, an FO map with blocking-field walls), order a ship straight across a nebula, through a planet, and into a walled-off spot. It should fly round the nebula and planet (never entering), go through gaps in barriers, and stop at the nearest reachable point when the goal is walled in; an order *into* a nebula should still fly straight in. Compare the routes with stock A2/FO behaviour, and watch settled FPS with a large fleet ordered across a nebula.

- [?] UI-18 display fidelity: use the new optimized display-fidelity launcher on Classic/Federation and FO4. Compare enlarged station/ship wireframes with the supplied stock reference; zoom, pan and rotate while selected and check that thin gauges stay above model bounds without lagging or disappearing. Hover values should use bare numbers and live health/crew tints. Try LCARS switching and report settled FPS. Existing original texture resolution still limits fine detail.

- [?] AI-03 soak: `cargo test -p sta_ai --test soak -- --nocapture` (about 40 s) plays ten seeded skirmish-vs-skirmish games on the fake `two_bases` map. Check the digests repeat across two of your own runs. Finding to judge: no game is decided in 20 sim minutes; all end at tick 64899 (about 36 minutes) with seat 1 the winner, and the seed does not change the course of the game, only the digest.
- [?] EDT-06 `odf_fmt`: `tools/agent-cargo --light run -p sta_tools --bin odf_fmt -- --check --grouped <a copy of a mod's odf folder>` passes; `--grouped` dry run on the copy lists files; try `--write` and diff a few (see docs/ODF_FMT.md). Group order is ours, not the FO guide's.
- [?] Menu widget kit (UI-21): `sta64 --ui-gallery`, then press the three profile tabs at the top (LCARS, Armada II, Fleet Operations). Click the dropdown (list opens under it, a click on a row chooses, a click elsewhere or Escape closes), scroll the list with the wheel, tick the check boxes, type in the text field (caret shows; Enter leaves it), switch the inner tabs. With the keyboard: Tab / Shift+Tab walk the widgets (a gold ring follows), arrows work in dropdown, list, slider and tabs, Enter or Space press buttons and tick boxes. Check that the stock skins' button labels are light on dark, not black. Frames without a window: `STA64_GPU_TESTS=1 STA64_UI_CAPTURE_DIR=/tmp/ui cargo test -p sta64 gallery_renders_on_gpu` (add `STA64_TEST_INSTALL=<root> STA64_TEST_MOD=<mod>` for the mod's fonts).
- [?] Stock front end by eye (UI-22, #496): `cargo run -p sta64 -- --help` for the front-end entry, or the bench frame run `STA64_UI_SCREEN=setup|main|single|settings` (`frontend/screens/frame.rs`) at 3440x1440. Check: main menu shows the successfully mounted mod's display name (STA64 without a mod), a gold divider and nine left-aligned text choices (Single Player, Instant Action, Multiplayer, Map Editor, Mod Manager, Options, Replay Intro, Credits, Exit); Mod Manager opens the existing picker, and unavailable Map Editor / Replay Intro / Credits entries stay inactive; Instant Action has the player table at the left (player, side, team dropdowns per seat, the side list holds the install's playable factions), the map name above a preview of start positions at the right, Select map / resources / officer limit beside it, Previous menu and Launch along the bottom; no text clipped in a button; a dropdown list opens over the other rows. Layout tests cover 21:9, 16:9, 16:10 and 4:3 at UiScale 1.0, 1.25 and 1.333. The main menu also has inspected windowless GPU captures at 16:9, 4:3 and ultrawide, plus installed A2 fonts; other screen appearance still needs manual review. (#502, #705)
- [?] Sprite fonts everywhere (UI-16): in a skirmish on FO 4 / Classic, notice lines at the top, chat down the left and tooltips (hover a command icon) are drawn in the mod's own font, and the gallery text too once an install is mounted; a tooltip wraps at its box. Resource symbols inside text keep their own colours. The HUD itself is UI-18's (Tammy); the follow-up list is in the UI-16 issue.
- [?] A2 initial hull/shield pools: in the new optimized initial-pools candidate, start a fresh Classic match and hover a constructor. With no shield strength authored, both health-derived pools should be present; build another constructor/ship and compare. Partial curHealth/curShield values must begin below maximum, explicit zero must remain zero at spawn, and save/load must preserve live values. Check settled FPS; original ODFs remain untouched.

- [?] UI-18 mouseover InfoBar: in the new optimized consumer build, hover a constructor, ship and station with nothing selected; then keep one selected while hovering another. The strip should show the hovered type and current hull/shield percentages, crew and energy with native icons/tints, while the main card/commands stay selected. Move off, over UI or into fog and check clearing/fallback, resizing, LCARS restoration and settled FPS. Use a fresh match for initialized crew; old saves preserve unknown unit crew.

- [?] UI-18 live unit-status prerequisite: current/max crew is now canonical per craft/station and existing live special energy is exposed in UnitView. Snapshot 13 preserves initialized pools; older accepted saves keep unit crew unknown. The mouseover-bar consumer and its new optimized launcher follow separately; do not treat this prerequisite as a completed visual check.

- [?] UI-18 resource colour correction: compare Classic resource numbers with original Armada II (crew green, officers white, dilithium pale cyan, latinum yellow, metal grey); explicit `resource_N_color` overrides still win. Check the new candidate launcher and settled FPS; parent #371 remains open.

- [?] Claimed bug #340: with artillery, check that a target inside weapon range but outside all allied sensors receives no new shots. Bring an allied scout into sight: fire should start; remove the scout: new fire should stop. Already launched projectiles continue.
- [?] Claimed bugs #341/#344: while stock, hull, shields or build progress change, check that the HUD stays continuously visible. Mobile builders have no Rally; yards do. Unarmed, point-defence-only and authored `canAttack=0` actors have no Attack. Open Formations: registered SET_FORMATION ODFs use b_<ODF stem> icons without overlaid captions; missing art falls back to the ODF name. Native choices stay disabled until their movement mapping is implemented; Back remains active. Without native metadata, the generic Line/Column/Wedge fallback still works. Selection changes must clear an unavailable page. Multi-selection wireframe tiles have no button fill; full ship names wrap inside the cfg tile rectangle using recognized InformationFont roles (Small fallback), and clicking a tile still narrows selection.
- [?] Claimed bug #342: watch individual worker bees during a station build. Each welding beam must be off while waiting/travelling/returning and on during its stationary welding phase; check textured beams as well as sparks.
- [?] Claimed bug #343: trigger selections/orders/advisor alerts while a long voice line plays. Only that line should be audible; conflicting lines are discarded and never played later. A fresh request after completion should play normally; music and effects remain independent.
- [?] Claimed bug #356: arm a station placement in Classic/FO4 and move over clear and blocked sites. The ghost should remain a readable translucent green/red blueprint, including custom hull-material models. Cancel and check that real units retain their original textures and lighting.

- [?] Native GUI button artwork (UI-27): in Classic and FO4, select a constructor, yard and ordinary ship. Check command/build icons, tooltips, hover/press feedback, disabled tech and poor-stock warning; build/back/cancel and map-target commands should work as before. Change faction between matches (especially Federation/Borg in FO4): the authored `interfaceSprites` table must win, with `gui_global.spr` as fallback. Formation labels remain visible; unavailable artwork and unverified special IDs retain text. [Scope and checks](docs/GUI_BUTTON_SPRITES.md).

- [?] Movement models (SIM-15): `cargo run -p sta_bench --bin sta-bench -- run move-turn-90-fo move-waypoints-fo move-start-stop-a2` passes. In a skirmish on FO 4 (or the STA2 Classic mod), order a ship to a point behind it: it sets off at full speed while turning instead of slowing to pivot; queue two waypoints: it takes the corners without stopping and comes to rest near, not exactly on, the last point; a point just behind it (inside `tooCloseToTurn`) leaves it where it is. Native-profile content moves as before. Not yet measured against the running games (proposed follow-up in the PR).
- [?] Launch points (SIM-16): `cargo run -p sta_bench --bin sta-bench -- run launch` passes; `STA64_TEST_INSTALL=<root> STA64_TEST_MOD=<mod or empty> cargo test -p sta_content --test launch_install -- --nocapture` (and `STA64_TEST_DIR=<dir>` for A1/A2) prints how many builders launch from their `buildHardpoint` node and how many take the fallback; in a skirmish a built ship appears at the yard's build bay facing out and drives clear of the yard
- [?] Context orders (UI-14): start a skirmish (command above), select a freighter and right-click a moon: it flies there, mines and delivers to the outpost (cursor turns to a pointing hand over the moon); select a warship and right-click an enemy ship: it attacks (cursor is a crosshair); warships on a moon just move. Rally from builders waits for SIM-17.
- [?] UI text (UI-09): `cargo test -p sta64 no_literal_ui_strings` fails on any literal handed to a UI text helper; the language is the `Language` resource (`english` default), tables come from `Dynamic_Localized_Strings.h` plus `sta64_<language>.strings` (`key<TAB>text`) in the install or mod
- [?] Front-end menu scene (UI-06), until INT-M4 puts it under the screens: `STA64_MENU_SCENE=1 STA64_MENU_VIEWS=klingon@6 cargo run -- --headless --clock bench --install-root <FO install> --frames 150,600 --frame-size 960x540 --frame-dir /tmp/menu`: tick 150 is the `main` view (a Federation fleet in orbit over a lit planet, stars), tick 600 the `klingon` view with five Klingon ships decloaked (they are invisible in the main view); without `--install-root` the same scene is boxes on the fake content. `STA64_MENU_DEPART=<seconds>` sends the view's ships to warp (front ship first; no tunnel or flash drawn yet). `STA64_BENCH_OUT=<dir>` writes the `render.menu.*` marks. `STA64_GPU_TESTS=1 STA64_TEST_INSTALL=<FO install> cargo test -p sta64 --test menu_scene`
- [?] Skirmish (INT-M3): `cargo run --release -- --install-root <FO install> --mod "Fleet Ops 4.0" --skirmish blueamber --race federation --opponent-race klingon` opens the map with your outpost at your start and the HUD on the bottom edge (info panel, 5x3 grid, resources strip, radar); fog shows only what your outpost sees; select the outpost: the grid has Build, the build page lists what the tech tree allows (locked buttons say why); Federation music plays and clicking units plays their select and order voices; the AI opponent starts mining and building ships within a minute (log lines `ai: command ...`); destroying every enemy craft and station shows a VICTORY banner, a notice and the log line `skirmish over`; `--opponent idle` leaves the other seat doing nothing; a mobile builder's build asks for a click on the map for the site (Esc cancels)
- [?] Skirmish bench scenario (INT-M3): `cargo run -p sta_bench --bin sta-bench -- run skirmish` passes (the scripted fleet destroys the idle opponent and player 2 is eliminated at tick 4368)
- [?] Start units (INT-M3): `STA64_TEST_INSTALL=<root> STA64_TEST_MOD=<mod> cargo test -p sta64 install_skirmish -- --nocapture` prints each seat's start units and stock and what the AI built in five minutes
- [?] Map on seams (INT-M2): `cargo run -- --install-root <FO install> --map <name>` plays as before (ships move on right-click, fight when ordered to attack); the info panel's `sim:` line shows the `MapDef` counts. `--seat 0` plays seat 0 (orders for its units only, its fog and shroud drawn). `STA64_TEST_INSTALL=<root> STA64_TEST_MOD=<mod or empty> cargo test -p sta64 map::tests -- --nocapture`
- [?] Map conversion (CON-07): `STA64_TEST_INSTALL=<root> STA64_TEST_MOD=<mod or empty> cargo test -p sta_content --test map_install -- --nocapture` (and `STA64_TEST_DIR=<dir>` for a bare game folder) prints, per install, maps converted, objects placed or counted as regions/planets/lights/props, and multiplayer maps with starts; no `PROBLEM` lines
- [?] BMP decode (CON-21): `STA64_TEST_BMP_DIRS="<A1 root>:<A2 root>:<FO root>" cargo test -p sta_assets --lib bmp -- --nocapture` prints `decoded N of N BMPs` (every `.bmp` below the roots, including `splash.bmp` and the ScrnThms/bitmaps art); nothing is drawn with them yet (UI-06..08)
- [?] JSON5 content (CON-04): `cargo test -p sta_content --lib json5` passes; `fixtures/content/content.json5` loads to the same `ContentDb::hash()` as `fake::db()`, `two_bases.json5` equals `fake::two_bases()`, and every unit's `basename` has a GLB box in `fixtures/models/`. Open the boxes in the model viewer (`--view fx_station`) if you want to see them; schema in docs/CONTENT_JSON5.md
- [?] Star Trek: Legacy SOL maps (CON-20): `STA64_TEST_SOL=<path>/mpbajor_all.sol cargo test -p sta_bzn --lib sol -- --nocapture` prints 8 starts and 19 objects (needs a Star Trek: Legacy install; not available on the dev machine, so only synthetic fixtures ran). Copy the SOL into an install's `bzn/`: the skirmish map list (and `--map <name>`) shows it, and its objects appear (those whose ODFs the install has); its Legacy `.drl` script is only named, not run
- [?] Map writer (CON-32): `STA64_TEST_BZN_DIRS="<A2 bzn dir>:<FO bzn dir>:<A1 bzn dir>" cargo test -p sta_bzn --lib installed_maps_round_trip -- --nocapture` prints how many maps read, write and read again to an equal `Map` (A2/FO), and how many Armada 1 maps are skipped (A1 bodies are not written yet)
- [?] Armada I map writer and lights (CON-38, CON-41): `STA64_TEST_A1_BZN_DIR="<A1 root>/bzn" cargo test -p sta_bzn --lib every_stock_a1_map_round_trips -- --nocapture` prints `49 maps equal, 106 lights, 49 headers byte-equal, 49 maps record-equal to the original`; `STA64_TEST_DIR=<A1 root> cargo test -p sta_content --test a1_lights -- --nocapture` prints 106 lights in the files and in the converted maps, 72 not white. In `--map <A1 map>` the lights now take the file's colours (RND lights for A1 are not wired yet)
- [?] Map corpus (TOOL-05): `STA64_TEST_INSTALL=<root> STA64_TEST_MOD=<mod or empty> cargo test -p sta_bench --test corpus -- --nocapture` (and `STA64_TEST_DIR=<dir>` for a bare game folder) prints maps loaded, ticked 300 and repeated with the same digest; no `PROBLEM` lines
- [?] Skirmish AI (AI-02): `cargo test -p sta_ai skirmish -- --nocapture` passes and prints `skirmish beats idle on two_bases at 21.84 sim minutes`; no in-game view yet (INT-M3 seats it with `Control::Ai("skirmish")`)
- [?] Nebula extras (RND-05), `cargo run -- --map <name>` on a map with the class: latinum's core pulses and its ring about the core tints in patches that go round teal, violet, pink and green; mnebula7 (crimson) flickers with lightning, a flash lighting the cloud about it for under a second; mnebula10's gas wanders slow loops (swirl); other nebulae look as before
- [?] Hull lighting with Refit's look (RND-06): in the model viewer or a map, ships are lit by the key and fill with a highlight, rim light, team colour, glowing windows and (Borg) normal maps; **H** switches to the previous Bevy lighting and back, to compare; T still changes the team colour in the viewer
- [?] Team colour on the right parts (RND-17), `cargo run -- --map <name>` as Federation with a construction ship (FO4): its `team*` hull groups (80% of the faces) take the player colour and the lit windows keep theirs; stations of every race are almost untinted (0-2%); T in the viewer changes the colour. If the Federation ship should be grey instead, the team colours (built-in when `teamcolor.odf` is absent) are what to change
- [?] Team palette (RND-20), `cargo run -- --map <name>` as Federation (FO4, with a construction ship): the hull is grey with the default player colour (white 0.9, from the stock `odf/other/teamcolor.odf` palette, now found there and built in when absent); red, blue, green and the rest tint the team faces fully, as in game (multiply); the lit windows stay untinted; T in the model viewer cycles the colours
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

- [?] Scripted ownership (SIM-19, #256; updated builds only): at a mission handoff, check new-owner control and rejection of former-owner commands; old movement, attack and actor rally stop. A loaded freighter retains cargo and delivers to its new owner after a new Mine order. End-of-batch last Craft/Station loss eliminates a configured seat; recovery in that same batch survives. Older pinned builds predate these changes.

- [?] Station queue limits (SIM-20): in A1 content admit five total jobs per yard, and in stock A2/FO admit ten, counting the active head; the next order is refused without spending resources. In a local test mod set the producer ODF `STA64_queueLimit = 12` to admit twelve, or inherit a nonzero value and override it with `0` to disable new jobs. Existing jobs continue; this does not add the A2FOExtensions grouped queue UI. Native content keeps five unless authored otherwise.

- [?] Construction/planet handoff (SIM-19 completion; updated builds only): station or producer-planet capture clears queued work; active prepaid work refunds, active deferred work forfeits its charge. A pending rig reservation refunds if paid; a welding rig loses its unfinished child. Direct unfinished-child capture remains unsupported. Invasion's planet handoff and Cavalry transfers need in-game acceptance. Background planets retain ownership but offer no Mine; ordinary deposits remain neutral. Full colonies, research attachments and orbital/multi-resource behavior remain planned. [Contract and limits](docs/OWNERSHIP_TRANSFER.md).

- [?] ODF duplicate report (EDT-09): `cargo run -p sta_odf --bin sta-odf-duplicates -- path/to/unit.odf` prints repeated command locations without rewriting the file. Later assignments remain legal; add `--deny-duplicates` only when a duplicate should fail a mod-author check. Includes and parent-mod overrides are not compared. [Usage and validation](docs/REFERENCE_SPACE_RTS.md#implemented-report-and-validation).

- [?] Original flat A1 install (CON-27, build including CON-26): manually run an updated `sta64 --install-root "<A1 install>" --map 2blue`. The map view should load placed actors/scenery directly from the existing game folder; record missing models, lighting or selection issues. The headless `--mission-report` path already enumerates all 49 maps. Do not move game files; A1 multiplayer starts still need MDF/precise-path integration. [Mount rules and checks](docs/INSTALL_ROOTS.md).

- [?] Map confinement (SIM-22 core, updated builds): in an A2/FO skirmish, order a ship toward a map edge and past an asteroid or planet near that edge. An enabled ordinary actor should stay in its configured playable volume and take an available inward route; an impossible route should retain its order for retry. Build ships with an outward rally point: each product follows its own confinement policy. Terrain keeps its spatial bypass. Owning-unit ODF overrides and their explicit-zero behavior are documented in [the core contract](docs/MAP_CONFINEMENT_CORE.md); Native content defaults off. Manual visual/runtime acceptance is pending, and the mission Lua adapter is still a separate step.

- [?] Standalone FO base detection (CON-31): after merging and building the updated engine, choose Roots through the normal STA64 install/mod entry without a forced profile. The startup `content:` line should report `profile Fo4`; confirm a skirmish loads and resource labels match the Roots content.
- [?] Standalone mods mount without base Data (CON-33, D6): in a build with this change, start a skirmish on Roots `Fleet Ops 4.0` and on `STA2 Classic Mod`: the map list shows only the mod's own maps (no base Roots maps), the unit and build lists are the mod's own, and no ship is missing a model or sound (the base `Data` layer is no longer mounted under a standalone mod; Data's `nebulae/` and `surfaces/` recipe inis are not visible to it, so check the backdrop and planet surfaces still look right). `STA2 Classic Mod Enhanced Gameplay` still shows Classic's maps and ODFs under its own.
- [?] Coverage report (CON-34, D7): `cargo run -p sta_content --example check_pack -- "<Roots install>" --all` prints a section per mod; `... "STA2 Classic Mod Enhanced Gameplay" --why RTS_CFG.h SHIELD_DAMAGE_PROTECTION` names the Enhanced layer and line, and the Enhanced section lists its 8 changed cfg lines as IGNORED. Check that the ignored list matches what you expect the engine to read (the read-keys list is a seed list).
- [?] Self-include inheritance (CON-36, D11): the installed Classic Enhanced ships full copies, so it does not exercise self-include. Ask the mod author for a mod whose `RTS_CFG.h` or `.tt` starts with `#include` of its own name; its changed keys should show in `check_pack --why` with their layer and the parent's keys inherited.

- [?] Lua shared controls (LUA-05, updated builds): run Blockade and check that its off-map staging/arrival callbacks no longer fail on forceOntoMap. Also check Invasion and Cavalry ownership handoffs. Lua writes use shared simulation guards; any rejected operation should identify its SCRIPT sequence in mission errors rather than silently appearing successful. Full campaigns, native playback and unimplemented A2 calls remain separate checks.

- [?] LUA-05 perceived-team prerequisite: after its separate seam and Lua adapter merge, check Blockade's disguise/reveal sequence. Confirm real ownership still controls selection/resources, automatic enemy acquisition follows perceived identity, and save/load preserves the disguise. Source/headless proof does not replace this campaign check; use the current build's revision-2 replay/view logs.

- [?] LUA-05 perception adapter: run Blockade beyond its fleet-isolation callback on a current campaign build. Confirm no unknown `perceivedTeam` field warnings, check the isolated ships retain their intended real owners, and keep any AIP/other unsupported diagnostics visible. A clean 1800-tick headless startup is a bounded check, not full campaign acceptance.

- [?] M1 corpus refresh: installed ODF/VFS checks now cover an original flat A1 ZFS install as well as base/modded Roots and Classic. See [the acceptance report](docs/M1_VALIDATION.md) for the exact counts, old-version/required-extension refusals and remaining GLB viewer check. Sabre construction/movement is accepted on the staged fix; M1 integration of #336/#337/#338 and the optimized-build FPS comparison are done; the GLB orbit check is waived (see below).

- [x] Static glTF/GLB (CON-09; orbit check waived by Cedric on 2026-11-13 ("we'll notice when it becomes relevant"), not tested): with an updated build, place the original `fixtures/models/fx_cruiser.glb` in a mounted layer's `sod/` directory and run `--view fx_cruiser`; orbit the red/blue checker model and check its surface remains visible. Parser, material, VFS lookup and windowless GPU checks are separate automated evidence. Animation, skins, morph targets and unsupported required extensions currently report explicit errors. [Scope and limits](docs/M1_ASSET_MODEL_COMPLETION.md).

- [?] M1 M3D formats (CON-08, updated builds): the normal viewer list should contain resolvable scene models, without nested chunk-library entries. Inspect known M3D models with their original geometry/materials; `model_info --file`, `--chunk-materials` and `--chunk-geometry` provide read-only diagnostics. Typed light fields and raw chunk decoding do not yet provide native light/destruction playback. The installed corpus and unchanged-byte checks are automated; remaining renderer limits and known content gaps are in [M3D support](docs/M3D_NODE_SUPPORT.md).

- [?] Playable race selection: on an updated build, the Instant Action picker should offer only the effective `races.odf` slot entries, including slot 0; NPC unit race names must not add choices. Original A1 uses the A2FOExtensions interface-based fallback. Confirm the selected race’s interface and start units match. [Rule and overrides](docs/PLAYABLE_RACES.md).

- [?] Station construction (#349, updated builds): build a Federation station in Armada 2 Classic. Its solid hull should appear in chunks; bees should leave the constructor's named bays after the takeoff delay, weld with sparks on the next chunk's edges, return home and then let the bays close. Cancel a second station and start another build to check returning bees and cleanup. Constructor timing/beam overrides and the station's optional `constructionChunks` are documented in [construction visuals](docs/CONSTRUCTION_VISUALS.md). Automated geometry/phase and windowless render checks are separate from this live acceptance check.

- [?] Station construction live follow-up (#349): on the updated candidate, confirm the wider outward bee stand-off makes `weldbeam` visible and no station chunks appear until the first welding interval has completed. During welding, Move and queued Move must leave the constructor fixed; Stop cancels the unfinished station (the active weld remains non-refundable) and permits a new order. Check the same with a second build and note FPS. `STA64_workerBeeStandOff` is overrideable on the constructor ODF. This follows the rejected first candidate; it does not mark that earlier visual check accepted.

- [?] UI-18 authored labels/colours and starts (#371): in Classic/Federation, check both starting constructors, select/reselect a new ship and confirm a stable full craft name plus its `unitName` type. Check selected and builder labels, the top name, and FO4 separate name/class colours against the mounted CFG. Known factions use only their standardUnits list; FO4 start-effect/faction lifecycle remains unsupported, without synthetic constructor injection. See [UNIT_INFO_STARTS.md](docs/UNIT_INFO_STARTS.md).

- [?] Faction display names (#371): the race picker should show the faction ODF `displayKey` through `Dynamic_Localized_Strings.h`, falling back to the literal display key when absent. Check Classic and FO4 captions and cycle/launch a race; simulation IDs must remain unchanged. See [RACE_DISPLAY_NAMES.md](docs/RACE_DISPLAY_NAMES.md).

- [?] UI-18 crew/officers (#371): use the new personnel HUD build in Classic/Federation. Normal should show crew1200 and officers16/600 at the fresh start; Lots should show crew30000, and Restricted/More/Max should use the configured300/800/999 maxima. Check resource text stays in CFG slots/colours after resizing; noCrew/noOfficers and zero-area slots must stay hidden. Confirm FPS. Crew spending/regeneration and officer queue admission are separate work; this checks starting values and live-actor census.

- [?] Captain/registry companions (UI-35, #599): in an isolated test mod with `possibleCraftNames`, `possibleCaptainNames` and `possibleCraftRegistry`, select a ship and a producer. Check the row-aligned values and authored medium/tall CFG areas/colours, including a blank companion and a disabled rectangle. No/group selection and the mouseover strip should show none. [Keys and scope](docs/CRAFT_IDENTITY_COMPANIONS.md).

- [?] Movie decoding (MED-01/MED-02, `sta_media`): nothing plays in game until MED-03. To check the decoders, run `STA64_TEST_MOVIE_DIRS="<A1>:<A2>:<FO Roots>" cargo test -p sta_media --release --test bink_install -- --nocapture` (every installed `.bik` decodes to its stated frames and audio; 7 golden hashes), and dump a movie with `cargo run -p sta_media --release --example media_dump -- <movie> --yuv out.yuv --pcm out` to view `out.yuv` (`ffplay -f rawvideo -pixel_format yuv420p -video_size WxH out.yuv`) and listen to `out.0.s16le`. MP4 decodes H.264 + AAC-LC; the installed Armada II `Intro.mp4` is AV1 and is reported as unsupported.
- [?] Movie playback (MED-03): `cargo run -p sta64 -- --install-root "<Armada II>"` shows the logo reels (Activision, Paramount, MadDoc) and the title movie full screen, picture fitted to the window with black bars, sound in step with the picture (watch lips against voice in the title movie; the sound is on the Voice bus), music silent while a movie plays and back after. Esc, Space, Enter or a click skips one movie and the next starts. A movie that cannot play (Armada II's `animations/Intro.mp4` is AV1; remove `Intro.bik` to see it) is skipped with a `movie ...` log line and the front end comes up. Check the window stays responsive (drag it) and CPU use is one core.
- [?] Intro cutscenes (MED-05): `--nointro` (also `-nointro`) goes straight to the front end with no movie. The sequence is `cfgINTRO_MOVIE_NAME` from `RTS_CFG.h` after the logo reels the install has (`intro:` log line lists them); Armada I plays `STIntro.bik`. The setting has no settings-screen toggle yet: `IntroSettings` is the resource a toggle will drive. Mission movies (`Media:playMovie`, in-panel on `PanelScreen`) are covered by tests only until UI-30 and the bridge display exist.
- `[?]` A1 campaign mission with bridge scenes (e.g. Federation 1): the bridge movie plays on its story event and the log reports it done; ending the mission stops a running one. (#531)

- [?] Render culling and LOD switch (RND-07, RND-27; #397, #399): in `--map`, zoom out and in over ships and stations of every size in Classic and FO4, and tilt the camera so far objects sit past 2800 units. No object should ever show its hull and its icon together, and none should vanish; a model past the culling distance (`cfgOBJECT_CULLING_DISTANCE`, height counted a quarter) becomes its icon and returns only once it is back inside 0.9 of it. The model does not yet fade by alpha across the band: it changes to the icon in one frame. `ART_CFG.h` far/near planes are read and marked, not applied to the camera.
- [?] Render profile toggle (RND-22, #395): in a running `--map`, F7 flips Stock/Refit (or start with `STA64_RENDER_PROFILE=refit`). Until RND-23..26 land there is no stock look, so the picture must not change in either profile; this checks only that nothing breaks (a log line `render profile: Stock -> Refit` appears).
- [?] Mod recipe and developer shader shadowing (RND-28, #393): a mod folder with `ordnance/ordnance.ini` holding `[Ordnance.<weapon odf>]` changes how that weapon's beam/bolt draws in a skirmish (nothing else needs to change). With `STA64_DEV_SHADER_SHADOW=1`, a mod's `shaders/render/<path>.wesl` (for example `shaders/render/fx/quad.wesl`) replaces the embedded shader; developer-only and unstable. `STA64_GPU_TESTS=1 cargo test -p sta64 --test recipe_override_frame` checks both in headless frames.
- [?] Beams end with their shooter and target (fix/render-beams-outlive-shooter): in a skirmish with phasers, destroy an enemy ship while it is being fired on. The beams to it should fade out within about a quarter second and no beam or glowing impact dot should stay hanging in empty space. Destroy the firing ship: its beams should vanish at once. Cedric to confirm in game (reported 2026-11-12: beams stayed after the ships were destroyed). (#483)
- [?] Mining ships (RND-29, #345): in a skirmish order a freighter to mine a moon next to a mining station. While mining it should circle the moon at about 180 units, firing a blue mining beam at it; when full it should leave, glide onto the station while unloading, then go back. Orbit radius and beam come from the mining weapon's ODF; the berth is the station centre (no game file names a dock point). `STA64_TEST_INSTALL=<root> cargo test -p sta64 --lib render::mining -- --nocapture` lists each miner. (#524)
- [?] `STA64_TEST_INSTALL=.. STA64_TEST_MAP=.. cargo test -p sta64 --lib the_bench_sim_of_a_map` passes on Roots + STA2 Classic (`fulltech`): not run. (#527)
- [?] CON-35 rules digest (D9, D10): `STA64_TEST_INSTALL="<Roots install>" cargo test -p sta_net --test rules_handshake -- --nocapture` prints the refusal between STA2 Classic and STA2 Classic Enhanced (first key `RTS_CFG.h:BASE_CREW_PER_SEC: unset here, 30 there`) and each installed mod's rule count; judge whether the named keys and values read well for the lobby's refusal text (UI-11). Replays (format 4) and saves (format 15) record lineage and pack names; older ones are refused with "from an older STA64". (#465)

- [?] Persistent shields (RND-32, #589): on a build containing this change, set `alwaysShowShields = 1` in an isolated test mod's ship or station ODF. Confirm the continuous blue outline follows the craft, disappears at zero shields, returns on recharge, and remains separate from hit flashes. Check cloak/uncloak, losing sight of an enemy, death and returning to the menu. Default and explicit zero stay off. Bounds/colour are documented approximations; [scope and automated checks](docs/PERSISTENT_SHIELDS.md).

- [?] Mission-driven faction appearance (RND-35): change a ship's race and its owner's race separately, capture it with and without a unit override, and save/reload. Check textures and faction subtrees under Stock/Refit, unknown/missing variants, and two instances sharing one model. Unit override wins, then current owner's override, then setup faction; team colour and ownership follow the existing simulation.
- [?] FO 3.2.7 / Dominion War detect as `Fo3` (CON-44, #582): `STA64_TEST_FO327_INSTALL=<FO 3.2.7 dir> STA64_TEST_DW_INSTALL=<Dominion War dir> cargo test -p sta_content --test profile_install --test fo3_install`
- [?] `sta-bench golden --install <FO 3.2.7 dir> --record g.json` then `--against g.json` matches, exit 0; the same against Dominion War's golden exits 1 (TOOL-12, #584)
- [?] Freighters (RND-29, #345): in Roots with the Mandril (and Classic/A2 freighters) mining a dilithium moon, the freighter should circle the moon and keep pulsing its beam for the whole harvest, then go to the station.
- [?] Freighters (RND-29, #345): unloading at a station, a freighter with a mining beam flies in and only slides the last short stretch to the berth; one without a beam (e.g. an A1/Classic freight ship) slides to the berth in about a second, never jumping when pushed by another ship.
- [?] In game at 21:9 and 32:9 the stock HUD is centred and undistorted, the margins beside it take world clicks (select, order, scroll), and no cutscene is pillarboxed.
- [?] UI-16 playtest: skirmish on A1, A2, FO 3.2.7, Roots, Classic: stock HUD info panel, mouse-over bar, command and PREV/NEXT buttons, Refit grid captions are in the mod's font and readable; no clipped or doubled text.
- [?] UI-16 playtest: A2 stock at 1024x768: info panel lines unclipped, minimap filter buttons without captions; FO resource numbers not doubled at 4:3.

- [?] SIM-70 station Recrew: in a fresh match, reduce an owned station's crew, activate Recrew and check that station crew rises while the player pool pays the same amount. Check full/empty pool, toggle/Stop, damage pause, owner changes and save/reload. Native timing is backed by static rules and headless checks; graphical acceptance remains manual. Yard recrewing remains separate.

- [?] FO4 AI startup (#806): start a fresh Federation versus Klingon skirmish. The computer's avatar starter should change into its authored productive station, then begin building. Check that the human starter remains available for manual avatar selection. The automatic AI owner gate is backed by static original-hook evidence and headless tests; graphical acceptance remains manual.

### Playtest checks from the 2026-11-14/15 merges

- [?] Planets (#732): in a map view a planet shows its near, camera-side hemisphere lit toward the key light, with the cloud shell and halo over the ground; check that dense clouds do not dim the halo.
- [?] Planet and backdrop scenery (#757): play the first Armada I campaign mission from the start: the background planet stays visible at every zoom, at low camera angles and in the opening cineractive; clicking it while no own ship is near selects nothing. Check other Armada I missions with a planet object the same way.
- [?] Armada I hull rims (#758): `cargo run -- --map fed1` as Federation: the saucer rim and nacelle caps are the owner's team colour, not red or pink; the Armada II Galaxy strut stays neutral.
- [?] Armada I movement (LUA-30, #740): orders given with `Attack(unit, Vector)` fly to weapon range of the point without firing on the way; `ReplaceObject` leaves a new unit at the old place under the same owner.
- [?] Armada I warp speed (LUA-31, #745): an Armada I mission that calls `SetWarpSpeed` makes the ship fly its warp leg at that speed and turn at impulse; a cineractive end restores normal warp speed.
- [?] Wormholes (LUA-24, #763): in the Roots gameplay tutorial, right-click the first gate with the flagship, go out through it and back.
- [?] Fleet Operations miners (#750): in a Fleet Operations Roots game, a miner with a long beam circles its resource instead of spiralling into it, and unloads at the station.
- [?] a2_fed01 first contact (#751): the first-contact scene plays at the first real Borg sighting after the briefing, not at the end of the intro; the cinematic video shows in its own window with play continuing.
- [?] Cutscenes (#755): in an installed Roots campaign mission with a cineractive intro, selection and orders do nothing and no event voices play while the cineractive runs. (install-gated)
- [?] Mission openings (#772): in installed Roots campaign missions that open on a cineractive the first frame is the script's opening shot with the HUD hidden and no explosion of the replaced craft.
- [?] Armada I HUD text (#756): in an Armada I campaign mission with the LCARS HUD, the resource bar shows numbers and labels and event lines show unit captions, not ODF names.
- [?] Armada I fonts (UI-41, #802): in an Armada I skirmish the resource strip and unit captions use the stock font.
- [?] Fleet Operations HUD (#760): a Roots 16:9 match (2752x1152 and 1920x1080): select a ship; the command buttons start right of the minimap panel; STA2 Classic and Armada II look as before.
- [?] HUD layout (#773): in a wide window "HUD layout: Centred" is shown in Graphics options; Stretched fills the window at once, Centred restores the centred HUD, and the choice survives a restart.
- [?] Campaign stock (#765): the first mission of each Armada II race starts with stock in the top bar and ships can be built; a Roots campaign mission starts with its map's stock; the gameplay tutorial still has none.
- [?] Orbital stations (SIM-65, #759): in Armada II stock or STA2 Classic a finished orbital processing facility circles its planet once per `orbitTime` and pulses a mining beam at it.
- [?] Armada I resource moons (CON-53, #798): in a running Armada I game the moon is clickable, a freighter mines and delivers, and the moon glows.
- [?] AI plan switching (LUA-38, #776): a converted Armada II mission with AI-plan switch rules loads one AI plan after the opening and another at most once per switch interval when the relative strength crosses a band.
- [?] Transport content keys (SIM-46 P0, #753): with every install present, `cargo test -p sta_content --test transport_keys` passes (general rules over the install).

- [?] Generic AI armed production (#818): in a fresh Classic skirmish with a Borg computer opponent on mp02twin, let it establish mining, construct a shipyard, and produce armed ships that attack the opponent. Check that it builds one useful producer, keeps its mining economy, and continues to honour explicit mission build plans. Headless combat probes supplement this manual check.
- [?] RND-36 construction presentation: check valid/blocked placement footprints, the faint station blueprint after confirmation and while its hull builds, cancellation and constructor loss; queue two ships and build at two yards together. Ships should reveal opaque chunks with sparks on the mesh edges and no worker bees, then launch once. Check pause, save/load, fog and a new match; windowless proof is separate from in-game acceptance.

### Playtest checks from the 2026-11-15/16 merges

- [?] Weapon base keys (CON-54, #858): `STA64_TEST_INSTALL=<A1, A2 or FO install> cargo test -p sta_content --test weapon_base_keys -- --nocapture` prints how many weapons set a base key, the `hitChance` and `damageBase` table rows read and `row warnings 0`. Nothing changes in play yet: the keys are typed, and SIM-73 applies them one rule at a time. Judge the load cost on Fleet Operations (about 160 MB more peak memory and 3 s more in a debug build for its 3 million table rows).

- [?] Armada I music (#762): in an Armada I campaign mission the intro music and voices play and the gameplay music follows (not heard by ear, no audio run).
- [?] Armada I health (#790): in an Armada I skirmish a damaged ship regenerates hull at the stock rule's pace; a ship at zero hull dies (the bench saw death at an empty pool, the static read says otherwise: see the follow-up).
- [?] Armada I stock HUD (#795): in an Armada I campaign mission the stock-sheet HUD shows the command, build and info panels; orders and build slots work by click; stations show their own entries.
- [?] Bridge and cinematic clips (#799, #815): in a2_fed01 the first-contact clip plays in the bottom-right cinematic window while the game runs; in a1 fed1 the bridge clip plays in the framed viewscreen (top right with the generic HUD, the sheet's cinematic rectangle with the stock sheet) over the cineractive shot, the rest of the HUD hidden, and the panel vanishes when the clip stops. Centred and Stretched both.
- [?] Armada I text spacing (#817): the resource strip numbers and unit captions have a gap between letters and digits and no touching glyphs.
- [?] Tooltips (#819): in Roots at 21:9 hover a button near the bottom right of the command grid: the tooltip sits beside the cursor, above it near the bottom edge, left of it near the right edge, fully visible; same at 16:9 in the Armada II stock HUD.
- [?] Mission audio (#820): in a2_fed01 only the mission's own music plays under the voice during the intro (no second race track), and the race music starts when the intro ends; in a2_kling01 the first script track stops cleanly when the next starts; clicking ships during a mission message makes no unit acknowledgements. Listening check, install-gated.
- [?] Alert and assist (LUA-32, #777): in a2_kling01 the staged craft of the opening hold their positions until hand-over instead of flying off on idle-assist attack orders.
- [?] Armada I movement model (#805): in an Armada I skirmish ships cruise, lag into turns, brake and stop short of the goal by the general rules; a mod can layer its own play-physics file.
- [?] Armada I transport (SIM-46, #823): in an Armada I game a transporter beams crew onto an enemy or own-team unit under the rule read from the executable; nothing about it was measured or run.
- [?] Orbital income (SIM-72, #813, #803): in Armada II stock or STA2 Classic a finished orbital station on its ring earns metal while its beam pulses, and its planet starts from the data's amount.
- [?] Armada I campaign win (#825): win the first Armada I campaign mission: the screen fades, the debrief opens within a few seconds and the next mission is offered, instead of staying black.
- [?] Roots supply (#826): in the first Roots Federation mission the supply readout at mission start is non-zero and equals the original's; in `tutorialgameplay` it stays zero by its table.
- [?] Script guard (#827): give a ship an Armada II script guard on a moving friendly craft (a2_borg04 once its other gap is closed): it trails the ward, fires at what attacks it and stops when the ward dies.
- [?] Armada I crew and officers (#828): in an Armada I Federation campaign mission after the first, select the construction ship: the strip shows a crew value and officers as used/cap from the install header, not an empty crew field and `0/0`.
- [?] Cursors (UI-43, #830): in a skirmish the stock arrow animates and the OS cursor is gone; the attack cursor over an enemy, mining over a moon for a freighter, rally over a friendly for a builder, wormhole entry over a wormhole for a mover; the cursor follows the pointer exactly at UI scales other than one and its hotspot sits on the click point; in Roots the larger cursor.
- [?] Armada II title and fade (#835): in an Armada II mission with a delayed fade out the held fade clears as intended and the title appears at the script's placement.
- [?] Boarding data (#800): with the stock Armada II install `cargo test -p sta_sim` boarding scenarios still pass after the data differences the PR lists.


## Checked earlier (morning list)

- [x] Construction live-feedback fixes accepted (2026-11-11, `construction-a1666d0`): the user confirmed visible beams, later chunk reveal and the constructor staying fixed under normal/queued movement. Automated Stop/cancellation and snapshot checks pass. FPS was not reported; this does not complete broader native/Android checks above.

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

- [?] Android / Waydroid first APK (TOOL-10, #253): manually install the ARM64 debug APK and open STA64; record its hash, Android version/active ABI, first-screen result and any native-loader/renderer errors. The current APK contains no original game data and has no SAF importer yet; an empty catalogue is expected. A translated x86_64 Waydroid run is separate from physical ARM64 evidence ([steps and limits](docs/ANDROID_SUPPORT.md#waydroid-smoke-testing)).
- [?] Android lifecycle (TOOL-10, #253): background and resume the app; the frontend should recover without stale clicks or a crash. With staged user-owned content, also check that a running match does not catch up background time or lose the player's existing pause state. Record audio/surface-recreation failures separately; audio focus remains a follow-up.

- [?] Loaded freighters (SIM-19, #256; updated builds only): interrupt mining with Stop or a new Move, then order mining again; the carried resource should be delivered before the next load. When changing resource types, watch that the old resource credits its own counter rather than disappearing or becoming the new resource. The older pinned rally build predates this change.

## Now (M2: a map you can fly in)

- [?] Strategic view (#365): zoom out/in in Classic and FO4. Each ship/station should switch exclusively between its hull and an icon at the hull centre; small zoom changes near the handoff should not flicker. Check resized windows/HUD scaling, icon clicks, fleet count/folding, Tab Auto/Always/Off and constructor/station selection. Grid/height bars and stacking already exist; broader RND-27 distance-LOD/culling remains planned. See [handoff contract](docs/STRATEGIC_HANDOFF.md).
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
- [ ] A2 `SetImpulseSpeed`/alert/`SetCanBeDecommissioned`/`DisableShieldGenerator` edits are rejected by the sim until SIM applies them; re-check the counted rejections then. (#532)
- [ ] A2 nearest searches: skip derelicts once the view carries derelict state; measure mask and tie order on the bench. (#532)
- [ ] A2 `BuildObject` object form: measure the rotation convention to accept non-zero offsets. (#532)
- [ ] Client consumer (UI-30/MED) answers `A2.EnterKeyDone`, `AudioMessageDone`, `MovieDone`, `CineractiveStartDone` and handles the new `Call` events. (#532)
- [ ] A converted-mission step machine polls `A2.ScriptWaits(script)`. (#532)
- [ ] Seams still needed (ranked list in the follow-up docs PR): Patrol, SearchAndDestroy, Scout, Colonize, warp Goto, PointAt, SetCrew, SetRace, SetPosition, AllowAsTarget, shield edits, targeted specials, unit command record. (#532)
- [ ] Front end on Refit's 3D menu scene (M4)
- [ ] LCARS widget gallery looks right: `cargo run -p sta64 -- --ui-gallery` (pills, elbows, hover/press, slider drag); screenshot it as the local baseline ([STYLE.md](crates/sta64/src/ui/theme/STYLE.md))
- [ ] Notices and tooltips look right: in a bench match a destroyed own unit shows a red "<Unit> destroyed" line at the top centre that fades after 5 s, an enemy loss a plain one, and a `Tooltip` node shows its text 0.35 s after the pointer rests ([notices.rs](crates/sta64/src/ui/notices.rs), UI-10)
- [ ] In-match text (UI-15): start a skirmish as a non-default race and kill a ship; the notice appears top centre, clear of the info text at the top left, and the info panel's race field names the race you play, not the map's default ([notices.rs](crates/sta64/src/ui/notices.rs), [map/mod.rs](crates/sta64/src/map/mod.rs))
- [ ] Engine measurements, not measured yet: where stock and Roots choose impulse vs combat speed for an attack order (only the effect is measured; SIM-36)
- [ ] Engine measurements, not measured yet: M1, M2 and M4 on Roots + FO 4.0 (only M3 measured; FO `ordnancedefenseweapon` decompiled only; SIM-34)
- [ ] LUA-11: bind `GetHandle`, `AudioMessage`, `PlayMusic`, `CineractiveStart/End`, `ShowTitle`/`RemoveTitle`, `SetPosition`, `BuildObject`, `Stop`, `Goto` on `A2` so converted a2_fed01 passes its intro (LUA-05) (#534)
- [ ] LUA-11: script waits (FadeIn/FadeOut/Cineractive/AudioMessage/PlayMusic/WaitForEnterKey hold the calling script) in the converted runtime once the bindings report completion (#534)
- [ ] LUA-11: engine-owned variables strength/moons/tried_warp_team_N written from the sim (AIP switching rules) (#534)
- [ ] LUA-11: A1 front end for the lifter (stack-depth argument tracking, DllMain registration/init), 20 campaign missions (#534)
- [ ] RND-23: map sky dome blend names (`mbgdom1.N`) draw opaque grey on `fulltech`; stock nebula strength on `a2_borg01`. (#527)
- [ ] UI-29: with no `settings.cfg`, a Refit install's `RefitPlayer.ini` `MaxSelection` seeds the cap (install-gated check on a Refit install) (#526)
- [ ] UI-29: Windows and macOS: the file appears under `%APPDATA%\sta64` / `~/Library/Application Support/sta64` after a rebind (needs a windowed run on each OS) (#526)
- [ ] UI-30: a converted A2 mission hosted in the client shows its opening camera and titles in a headless frame (needs the mission host) (#533)
- [ ] UI-30: `ScriptReply` becomes a command in the shared stream (seam) so movie/message/skip answers replay (#533)
- [ ] UI-30: `SetCameraZoom`, `CineractiveCameraOffset`, `CineractiveShot` meanings measured on the bench, then consumed (#533)
- [ ] UI-30: map loader fills `ScenePaths` so path-point shots resolve (#533)
- [ ] LUA-11: read the A2 BZN path list (sta_bzn) and fill `A2.paths`, then `Goto`/`BuildObject`/`GetDistance` by path run (19-22 missions) (#550)
- [ ] LUA-11: engine-owned team variables (strength/moons/tried_warp) written from the sim for the AIP switching rules; fleet objects (`CreateNew*Fleet`) (#550)
- [ ] LUA-11: measure what releases a cineractive start and Enter waits in the original (stand-in values today) (#550)
- [ ] LUA-11: `IsCompromised` meaning (27 missions) and `AI_Team_Load_AIP` (30) once SIM rows land (#550)
- [ ] INT-M6 campaign: from a running client, Single player -> a campaign card -> mission -> Start plays the mission with its script (a human checks the first minute of FO Roots mission 1 on a real GPU). (#551)
- [ ] UI-25: a human checks the mission list, briefing and debrief regions on the user's monitor at 21:9. (#551)
- [ ] SIM-61: presentation-done command (the host-side reply function goes). (#551)
- [ ] `sta-bench desync` on two real peers' desync replays from an A2/FO install game (install-gated, not measured here) (#549)
- [ ] NET-03 transport carries `Sections` end to end (relay forwards tag 21) (#549)
- [ ] RND-23: stock nebula puffs are sharper and more saturated than the original's smooth haze (lowfreq 40..62); test the occlusion-cull and gamma-space additive hypotheses. (#557)
- [ ] RND-23: planet in view (`pb_clss*`): original draws none at a2_borg01 1527 292 1748; find why before matching. (#557)
- [ ] RND-23: switching the profile at run time does not rebuild nebulae (next map build). (#557)
- [ ] CON-47: measure what the original does for a path name the map lacks (10 names in 8 stock missions) (#554)
- [ ] CON-47: meaning of `pathType` 2 (patrol-like in fed03 `pat_1`, `pat_2`) once SIM-56 patrol lands (#554)
- [ ] LUA-11: a2_tutorial6 `Run_Tutorial` holds on a wait the stand-in host does not report; find which (#554)
- [ ] SIM-39 hull-hit crew law, low-hull life-support drain and boarding losses measured on stock A2 (paired scene), then implemented (#548)
- [ ] SIM-29 unfinished-child capture and research attachments measured per profile (#548)
- [ ] SIM-54 Player/Entity race and crew clamps (MAX_CREW, officer cap) measured on stock A2 and Roots, then applied (#548)
- [ ] SIM-55 effect of a disabled/damaged shield generator, life support and sensors measured on stock A2 (#548)
- [ ] `cargo test -p sta_net --test rejoin` stays green 5 times in a row on a loaded machine (socket tests wait on conditions, never on sleeps). (#564)
- [ ] After SIM changes to movement or the roster: `STA64_BLESS_NET_FIXTURE=1 cargo test -p sta_net --test snapshot_fixture` and say so in the PR. (#564)
- [ ] Lobby (UI-11/INT) must: store the `rejoin_token` from `Welcome` and the new one from every rejoin; call `Peer::serve_rejoins` and `set_departure_source` on the host; show `ResyncEvent`s; not drop a seat while `rejoin_in_progress`; kick on `SeatAbuse`; fill `Start` seat lineage and pack names from the host's `ContentDb` (the snapshot's provenance is checked against them). (#564)
- [ ] LUA-12: bind A2 `FailMission` / `SucceedMission` (unbound; missions cannot end). (#567)
- [ ] SIM-59: derelict latch and original owner as view fields; then `IsCompromised` is exact. (#567)
- [ ] AI-06 / INT-M6: the client's mission host takes `take_aip_requests` and seats `MissionAiHub` controllers. (#565)
- [ ] AI-06: watch a Roots mission AI (build list then priorities file) build and attack on a real run. (#565)
- [ ] UI-11: INT-M5 starts the map view from `NetLaunch` (`NetMatch::into_peer`), and turns `RouterEvent::Protocol` into a kick. (#566)
- [ ] UI-11: rejoin (enable the button): `NetTransport::rejoin` + `Rejoiner`, host `Peer::serve_rejoins`, `set_departure_source`, resync messages, `SeatAbuse` kick, `last_seq`. (#566)
- [ ] UI-11: decide kick policy (`Session::after_kick`) and AI seating (`place_computers_around_humans`). (#566)
- [ ] UI-11: measure Armada I's and Fleet Operations' multiplayer Game Setup columns. (#566)
- [ ] SIM-57: measure `SetPosition`/`PointAt` by path name on a bench set-up that loads the map's named paths (a2_borg01 `BORG_CRASH_PATH`), and the y clamp limits. (#570)
- [ ] SIM-57: bench-check what a zero-length `PointAt` does on the original (borg01). (#570)
- [ ] SIM-57: measure `PointAt` on Armada I and `SetAngularVelocity` on Roots. (#570)
- [ ] LUA-11: converter passes `Vector3 const&` slots as `Ref` tables (`SetAngularVelocity`); unresolved `G[0x...]` operands to `PointAt`. (#570)
- [ ] Check `Start.map.digest` against each peer's `map_digest` at start (NET). (#569)
- [ ] sta64 networked start: `Control::RemoteAi` must stay an AI seat (flow.rs). (#569)
- [ ] INT-M6/AI-06: a mission whose script loads an AIP at tick N has its AI seat act only from N (headless `game::mission_tests`); install-gated Roots and converted-A2 starts show no `Player.loadAIP` in `unsupported()`. (#575)
- [ ] SIM-61: a save after a scripted AIP load keeps the plan (the plan is outside the snapshot today). (#575)
- [ ] LUA-13: with a stock install, `tools/campaign_readiness.py --converted --ticks 9000` shows 0 wins, 0 losses that rest on a gap, and the blocked reasons fall as gaps close. (#577)
- [ ] LUA-13: `A2.FailMission` / `SucceedMission` in a converted mission end the campaign mission once and a blocked end keeps the mission running. (#577)
- [ ] LUA-13: a converted mission that reads a handle a failed call named never reports a loss; `readiness` lists it as blocked with the reason. (#585)
- [ ] `python3 tools/campaign_readiness.py --converted --a2 <stock> --ticks 9000`: no first-problem call is a converter defect (SetAngularVelocity, GetNearestObjectNearPosition, G[...] operands gone); `engine_reads` non-empty for AIP missions. (#586)
- [ ] A host binds `A2.EngineVariable(name)` for team strength / moons / warp tries. (#586)
- [ ] SIM seam: synchronous spawn so queries (GetMaxHealth) work on a BuildObject handle in the same tick (tutorial4). (#586)
- [ ] LUA-14 follow-up: model `isCineractiveDone` (camera-shot completion) so A1 openings end (fed1, fed2, finale5) (#591)
- [ ] LUA-14 follow-up: A1 `BuildObject` "neutral ownership requires a resource definition" (team mapping) (#591)
- [ ] LUA-14 follow-up: engine behaviour of A1 calls on an invalid or dead handle (read from Armada.exe), as A2's object guard (#591)
- [ ] SIM-36 follow-up: join SIM-58's per-team targetable mask into `combat::targeting::candidate`.
- [ ] SIM-36 follow-up: run `sta-bench pair attack-approach` both sides (STA64 side on install content) and set its tolerances from the report.
- [ ] SIM-36 follow-up: measure the Roots construction ship in transit, and the stock assist through the HUD autonomy buttons.
- [ ] Referee: run `relay`, `netplay host --referee`, `netplay join` and `referee` as in the recipe on a second machine over a real network.
- [ ] Referee against real content: wire `Referee::new` into the game's referee entry point once INT has a headless content loader.
- [ ] Mission Lua across a host resync (`RefereePolicy::resync_host`), then flip the default.
- [ ] Late referee (needs the relay to keep a room open to `RefereeHello` and a snapshot kind for it).
- [ ] check: `STA64_TEST_A2_STOCK_INSTALL=... STA64_TEST_A1_INSTALL=... python3 tools/test_mission_oracle.py` passes
- [ ] check: converted a2_fed02 RaidCargo_step exercised in a running mission (decoded, not run in the campaign runner)
- [ ] check: a confined craft resting on a map-border field obeys a new move order (`avoid::tests::a_confined_craft_resting_on_a_border_field_leaves_it`).
- [ ] check: a move order to a point just beyond a map-border field edge ends (no orbit).
- [ ] INT-M5: rejoin from the Join form: keep token + relay + seat after launch, `NetTransport::rejoin` + `Rejoiner` in the app, `SimLink` from `RejoinDone`, seq from `router().last_seq`; then enable the entry.
- [ ] INT-M5: a stalled-but-connected seat has no host action yet (drop now / wait / AI takeover vote per NET.md section 8).
- [ ] INT-M5: decide what happens to a dropped player's forces (NET.md open question 9); `DropChoice::Eliminate` has no sim effect today.
- [ ] INT-M5: Lua mission in a networked match; install-gated two-process run.
- [ ] Classic: with a Roots install started without `--mod`, choose STA2 Classic Mod in the front end, start an Instant Action skirmish as Federation, select a ship and move it: voice lines play (also Borg, and after switching back to the base game).
- [ ] check: a move order that gets no nearer for a minute ends with `OrderBlocked` (`an_order_into_a_wall_is_reported_at_once_even_from_rest_and_ends_after_a_minute`).
- [ ] Stock A2 menu: start each campaign's first mission from the menu after `tools/convert_install_missions.py` and play 5 minutes
- [ ] Mod chains: convert/mount per mod chain (D6) if wanted
- [ ] Front end: select Fleet Ops 4.0 then back to the base, no crash; menu scene shows ships as models, no grid or icons; after switching to Classic the overlay says Classic and `PA_ECFR0` resolves in a match.
- [ ] Play an Armada I campaign mission from the menu after conversion
- [ ] check: a slow craft flying round a long wall keeps its order (`a_slow_craft_flying_round_a_long_wall_is_not_cut_off`).
- [ ] `STA64_TEST_A1_INSTALL=<A1 root> cargo test -p sta_mission --test a1_objective_text` passes (67 texts resolve, 0 unresolved).
- [ ] Playtest: on STA2 Classic `4alliance`, click each visible dilithium moon, right-click it with a miner selected, and use the Mine command's target click; if one fails, press Ctrl+F12 and read `Click pick` / `Pointer over HUD` in capture.md.
- [ ] Planet ghosts (Classic tritanium planets) that start in sight are selectable and right-click Mine targets.
- [ ] RND-24: Borg cube is 1.1-1.2x too bright and its dark faces show detail the original does not; check the engine's vertex normals (smoothing at hard edges).
- [ ] RND-24: A1, FO 3.2.7 and Dominion War hulls are not measured against the original (the bench has no A1 profile); `alpha`/`atmosphere`/`phong`/`lambert` SOD blends are drawn as `default`.
- [ ] RND-24: the original's player colour in the A2 bench is white; STA64's pair colour is not read from the install's settings.
- [ ] Manual (user): with the game running and music playing, switch the PipeWire/default output device (and unplug/replug headphones). Expect at most a short dropout, then sound and music return, and the log shows at most a handful of `audio:` lines, never a flood.
- [ ] Manual (user): `grep -c rodio` in the log during a play session is 0; `cargo tree -p sta64 -i rodio` finds nothing.
- [ ] In a real match (windowed, 21:9 and 16:9): Esc opens the menu over the HUD, the match is frozen, Resume continues; Options sliders change the music volume audibly and survive a restart.
- [ ] Exit to main menu from Instant Action, then Instant Action again, a campaign mission, and a mod switch: no leftover ships, fog, particles or titles.
- [ ] Multiplayer (after INT-M5 merges): the menu does not pause; Exit leaves the session cleanly.
- [ ] Armada I, Fleet Operations, Roots: Esc menu captions come from the install's `label.map` (`return`, `options`).
- [ ] Roots "Politics by Other Means": with `STA64_TEST_ROOTS_INSTALL`, `install_gated_roots_politics_by_other_means_plays_its_intro_and_never_holds_the_camera` passes.
- [ ] Cineractive hold: Space/Esc skips a skippable one, a non-skippable one after 10 s, any after 180 s; verify in a real client on a Roots intro.
- [ ] A loss after an unsupported call shows "cannot be finished yet" and changes no progress (debrief "Mission abandoned").
- [ ] Unify the Roots (this PR) and A2 (LUA-13 #577) blocked-end paths in one outcome type.
- [ ] RND-23 planets: lock-in pairs `bench/pairs/planet.ini`, `planet-far.ini` (`sta-bench pair bench/pairs/planet.ini --game a2|roots --install ...`), exit 0 needs the original's frames (local).
- [ ] RND-23 planets: match the original's planet lighting and cloud bands (second PR); compare background `map_planet_*` planets on FO/DW/FO4.
- [ ] Bench planets hold shield 10000 where the original's hold 0 (hence `hull=10000` in the planet pairs).
- [ ] After a front-end mod switch the map view's texture cache must not keep misses from the base mount (`TextureCache`).
- [ ] Bench: measure the original's orbital placement ghost (snap position, bearing source, facing side, `orbitTime`) and implement orbiting (SIM-62 follow-up).
- [ ] NET: a stopped peer that cannot reach the differing tick (records missing) writes its report after 10 s, but the match then still waits for `RepairOverdue`; consider the host ending the match with a reason when no repair request comes.
- [ ] Armada I fed1 in a real window: intro plays its shots, Space jumps to the zoom-out, camera control returns, start view at the original's height.
- [ ] Wire `SetCameraZoom` and `OVERVIEW_MIN/MAX_HEIGHT` zoom limits to the orbit camera.
- [ ] Audio lane: play cineractive voice/music so A1 intros keep their original length in the client.
- [ ] Graphics section in the options screen for `RenderQuality` (shadows, MSAA, nebula steps), persisted in the UI-29 store.
- [ ] Options -> Graphics on a real desktop: change Windowed size, toggle Fullscreen (monitor resolution), Vertical sync and Show FPS, then restart and confirm the choices persist. Repeat from an in-game Options menu, including a match launched directly, and check returning from fullscreen after manually resizing/maximizing the window. Window-component tests and windowless GPU captures cover these display controls; compositor/HiDPI behavior still needs manual acceptance. Android retains its native window size/mode.
- [ ] Nebula march at half resolution with depth-aware upsample (RND-31 follow-up); measure with `sta-bench perf ... --variants base,no-nebula`.
- [ ] Animated sprite materials (stock nebula billboards) rewritten every frame: share or throttle.
- [ ] Shroud art: original games draw unexplored space black; `Gshroud.tga` is drawn as-is (fog owner).
- [ ] check: a stock A2 mission with `Patrol` on a real path loops it from the first point (needs the A2 path reader).
- [ ] check: `AllowAsTarget` blocks automatic fire at a unit but not an explicit attack, on the bench (static only today).
- [ ] check: measure scout and search-and-destroy on stock A2 (Refit `measure/sim57-cmds` has `setpos`/`paths`).
- [ ] Roots "Politics by Other Means": with `STA64_TEST_ROOTS_INSTALL` the test passes; confirm in a real client that the "N Minutes Remaining" subtitle shows a countdown that drops once a minute.
- [ ] A mission that renames a unit with `Entity.name` shows it in the HUD (the view carries `attrs.names`; the HUD does not read it yet).
- [ ] Politics by Other Means (Roots, tutorials 4): the mission opens black and fades in on the Federation fleet (camera on a path, no HUD, no shroud), a voice line plays at 3 s, cuts to the Dominion relay at 26 s, fades out and the game appears with HUD and the objectives panel (never black, never an empty panel)
- [ ] A1 converter: resolve string arguments held in registers (`CountUnits(team, R.eax)` in rom5/finale4, `CountUnitsNearPoint` in borg3/borg5/rom3)
- [ ] Carry the LUA-17 converter-runtime guard (`is not alive` answers 0) into the LUA-15 Rust port
- [ ] View order state (moving / attacking) to replace the A1 `InQueue`/`InAttackMode` approximations
- [ ] Wormhole links: A2 map pairing and `LinkWormholes`/`LinkWormHoles`, `StuffIntoWormhole`
- [ ] Playthrough plan and reference for A2 a2_fed01 once SetRelation and Colonize land
- [ ] Smooth the quarter-size gas (a small blur or temporal accumulation as Refit has) and a graphics-menu choice of nebula resolution.
- [ ] Skip march steps outside the envelope before the flow/swirl reads (per-step cost, ~20 texture reads).
- [ ] RND-23 planets: confirm in a real window whether planet lighting/texture still misbehaves (see Not verified).
- [ ] `bench_cli`: `STA64_GPU_TESTS=1 cargo test -p sta64 --test bench_cli session_frames_equal_the_frames_option shot_steps_run_at_tier_two`
- [ ] Classic 3peace: hover and click a latinum nebula, right-click it with a freighter (mines).
- [ ] Check at 3440x1440 in game: minimap shroud edge follows the blips/background across the whole display area, and the camera outline sits on the same box.
- [ ] A1 Instant Action: with stock A1 the HUD shows dilithium 3500, crew 4000 and officers 0/100 for the player; the build menu is not empty.
- [ ] A1 player move: ships go to warp on long moves as stock A1 does (measure first).
- [ ] SIM-39: station RECREW toggle (`recrewRate`, 10 s pause after damage, off when full; needs an order seam) and shipyard recrew 20/s from the team pool (with SIM-40). Not measured on the bench.
- [ ] SIM-46: boarding strength rules (`BoardingParty`, `boardingStrength`, derelict capture, transporter) per the research notes.
- [ ] SIM-39: facing-weight hardpoint blend when target hardpoint positions reach the sim.
- [ ] SIM-39: expose derelict/system state in `UnitView` (view seam) for the HUD.
- [ ] SIM-39: bench check that Classic inside Roots hits like stock (one scene on Roots with `STA2 Classic Mod`).
- [ ] SIM-39: yard crew refill consuming `yard_crew_per_sec`; `Uni*` hook weapon effects are separate.
- [ ] check: stock A2 `CenterCamera(path, point)` centres the strategy camera on the path point in the client.
- [ ] check: a siege fleet and a move fleet of a stock A2 mission reach their destination on the bench (static only today).
- [ ] check: A1 `fed1` playthrough still 0 differences against its reference.
- [ ] A1 playtest: scout across a 2deadzone-size map takes about 30-40 s; battleship about 50 s; a short hop stays slow.
- [ ] A1 playtest: a ship turning to a far goal turns at impulse first, warps once within 30 degrees of the goal.
- [ ] Bench: confirm the stock A1 objectives panel shows a single OBJECTIVES heading and no BRIEFING SUMMARY heading; A2 shows both.
- [ ] In game: placing an orbital station by click snaps to the planet's orbit and is refused in open space.
- [ ] Refit look: infinite moons (purple nimbus) still get blue cracks; colour them purple (not done here).
- [ ] Stock look (A2 stock and Classic): dilithium moons show a blue pulsing halo and blue glowing cracks; infinite moons purple.
- [ ] Refit look: moons unchanged (no round halo, light in the cracks).
- [ ] Stock look: asteroids look exactly as before (stock texture).
- [ ] A1 fed1 opening: title 'PREMONITIONS' fades out at 10 s over 1 s; voice line plays out before the zoom-out; control returns after the 5 s zoom-out (playtest with sound).
- [ ] A1 title font (title.spr) and size: measure on the original-game bench.
- [ ] Shipyard repair-dock animation (`repairAnimation`, flags 2/4 at `SetRepairDock`) once the sim has a repair dock.
- [ ] A windowless GPU frame of a stock mining station docked and undocked.
- [ ] Units with `animation` off that are not transfer stations: the engine rests their ambient tracks; decide after a visual check.
- [ ] check: repair time, mining rate modifier, recycle fractions and crew-status thresholds read the unit's effective race (`Sim::unit_race`) when those systems land
- [ ] check: `latinumCostModifier` documented as an STA64 race ODF key in the modding docs
- [ ] check: research pod attachments (design on #539)
- [ ] check: refund of a queued item uses the latinum factor paid under; AI and HUD apply the seat's latinum factor
- [ ] A1 HUD: dilithium, crew and officers (used/cap) readouts appear top right, update live, for all four races.
- [ ] SIM-63: A1/A2 `SetRelation` and Roots `setRelation` honoured in a playthrough (a2_fed01 opening loses no ship)
- [ ] SIM-63 follow-up: silent removal vs destroyed for scripted `Entity.remove` / `RemoveObject` (monitors' view unmeasured)
- [ ] SIM-63 follow-up: point-defence cover, repair, trade and boarding by relation
- [ ] LUA-18: install-gated check that every path name the converted stock missions pass to Goto/Patrol/PointAt/GetLocation/BuildObject is either in the mission's map or reported as absent (counts), so a map-fill regression is told from an absent name
- [ ] Armada I `Goto`/`Patrol`/`PointAt`/`GetLocation` on an absent name: read the A1 executable's script functions (now the A2 rule, not measured)
- [ ] `GetLocation` default vector value (not measured, zero)
- [ ] `SetPosition`, `GetDistance`, `CountUnitsNearPoint`, A1 `BuildObject` on an absent path: engine behaviour is a no-op / 0 for SetPosition and BuildObject (A2 static); still raise here
- [ ] A1 `finale6`: transwarp-link query (`GetTranswarpExitWormhole`), and make a failed overload parse report the closest overload
- [ ] SIM-30: Borg colonizing beam (`colonybm`, `cleansingWeaponIndex`, ion beam on an owned planet, 25 colonists/s, full special energy, ship survives); order is rejected for those ships.
- [ ] SIM-30: ship crew moves to the planet on finish (planets have no crew pool), planet max crew from population level (1000/2000/3000/5000).
- [ ] SIM-30: pod flight time and pods in flight on STOP; landing spiral and `Destroyed` event; pod vs planet shields (`ignoreShield`).
- [ ] SIM-30: shield and crew interaction of the 0.1 x damage rule on planets not measured; planet hull value when no row exists is unchanged (takes hull damage).
- [ ] SIM-30: planets' shields/atmosphere persistence and orbital links; AI planet choice; SimEvents for colonization start/finish/cancel; map-load owner without population; forced cancel when a script changes the planet owner mid-colonization.
- [ ] SIM-30: bench scenes of research note (pod timeline, growth curve, Borg beam, stop mid-run, steal attempt, SetPlanetCivPop extremes) to move grades from static to measured.
- [ ] SIM-30: a derelict (crewless) colony ship can still colonize: main has no derelict gate on orders in general (known gap, no gate added).
- [ ] SIM-30: a people-lost field on the Hit event for planets (the event now reports hull 0).
- [ ] SIM-30: populated planets show no hull impact spark because the hit event reports hull 0.
- [ ] `AllStationsDestroyed`/`CountStations` engine parity: team -1 matches nothing and derelicts are not counted in the engine; STA64's CountStations differs (not hit by stock). Bench-check or fix with the derelict latch.
- [ ] `CommandAllShips` first bool (per-craft state byte not identified) and commands other than STOP/ATTACK: identify the byte; the others need targets or seams.
- [ ] `GetNextCraft`/`GetFirstCraft` list membership of dying and unfinished objects: not measured.
- [ ] LUA-20: a disabled seat in a stock mission (A2 borg07 mid-mission `DisableAllAITeams`) stops building and sending waves but still defends its base; a re-enable or plan load brings it back (game check, original not measured).
- [ ] LUA-20 follow-up: defend-only / wave-upkeep subset for a disabled seat once the distress-call rule is read (known deviation).
- [ ] LUA-20 / SIM-61: mission save must carry per seat `ai`, `enabled` and the plan.
- [ ] UI-38: right-click colonize (default-mode vote, priority 5; refusal red cursor and voice) in `sta_sim::context` and the bench `rclick`.
- [ ] UI-38: stock command hotkeys from `Input.map` (colonize = GreyInsert, harvest = H, rally = P, ...), none wired in the HUD today.
- [ ] UI-38: `c_colonize` / `c_nocolonize` sprite cursors while a COLONIZE pick is armed.
- [ ] UI-38: planet readout of population, cap and colonizer in the info bar (`WorldView.colonies`), after #681.
- [ ] UI-38: `OrderKind::Colonize` and the colony ship's `eventAcknowledge` voice for the HUD order.
- [ ] UI-38: COLONIZE for the Borg beam colony ship once the simulation models the beam (stock shows the button).
- [ ] UI-38: bench check of the original's COLONIZE availability for mixed selections and of which planets the cursor accepts.
- [ ] Player check: converted A1 rom3 no longer fires the four wormhole-switch rules every tick once wormhole objects exist; A1 fed5's opening survives Worf (currently ends lost at ~45 s in the stand-in run).
- [ ] `GetDistance` is 2D (x, z) in the original; the binding is 3D.
- [ ] RND-12: a bench verb for planet population, then a pair of a colonized planet against the original (Stock look, both hemispheres).
- [ ] RND-12: whether the original keeps drawing a planet's old cities when its owner becomes neutral (not established; they stay drawn here).
- [ ] RND-12 (see #545): the planet hemisphere meshes face inward, so a planet draws the far wall of its sphere; check the cloud shell and halo against the original.
- [ ] RND-12: measure the 16x16 alloc picture's filtering, the drawn population at map load and the pass's night-side lighting on the original.
- [ ] LUA-20: a2_borg01 / a2_borg10 end near a `DisableAllAITeams`; check in the original whether a disabled team's distress responses and running waves change what `IsCompromised`/`CountStations` see there (not measured).
- [ ] LUA-20: player check in game: a mission with `DisableAllAITeams` in a cineractive (borg01) and its AI waves afterwards.
- [ ] SIM-61: mission-save blob carries per seat `ai`, `enabled`, plan; host-side requests outside `think` (replies) in the command stream.

### Follow-ups from the 2026-11-14/15 merges

- [ ] Render (#732): `render::gpu_shaders::rock_planet_and_nebula_pipelines_build` fails when run in one process with the other GPU tests (pre-existing on main), passes alone: isolate the test.
- [ ] Render (#732): the planet halo (additive) and the cloud shell (blended) sort-tie at the same origin and depth bias: fix the bias if dense clouds dim the halo.
- [ ] LUA-24 (#734): wormhole props are selectable and tinted in the UI for map gates seated to a team (`map/mod.rs` adds a select radius from the BZN team): key the select radius and the tint on the sim owner, or exclude `wormhole_obj` as the neighbouring code does. The study wanted them "not counted" by `CountUnits`; not filtered.
- [ ] LUA-24 (#734): id-order gate pairing over live gates is reachable on stock maps (a script-built gate pairs with an odd leftover map gate; removing a gate shifts later pairs): fix with the gate link package; re-record the install-derived goldens.
- [ ] LUA-24 (#763): maps with a gate count other than two get travel only after LUA-33 reads the partner pointers; recheck then, and re-check `a2_borg05`, `a2_kling05` and `a2_kling06` when the team travel switch and gate routing land.
- [ ] LUA-24 (#763): HUD cursor for a right-click on a wormhole.
- [ ] LUA-24 (#763): queued, patrol, escort and follow orders onto a gate are not carried through, a destroyed map gate ends travel for the game, and a craft with no qualifying exit spot may rest on the gate's centre: revisit with LUA-34.
- [ ] LUA-24 (#763): `EnableWormholeTravel`, `LinkWormholes`, `StuffIntoWormhole` and `SetWormholeDeleteOnTransport` are still gaps while travel is always on for two-gate maps: a mission that keeps a gate closed gets a teleport instead.
- [ ] LUA-25 (#737): do NOT wire `map_label_gaps` into the client host while `handle_gap` is global; first narrow the taint to the gapped label's own handle uses.
- [ ] LUA-25 (#737): add the no-gap control to the label-gap test for the never-existing handle read, and rename the scenery-light test and its assertion message to "light-named prop".
- [ ] LUA-30 (#740): the A2 `DestroyCritical` has no kind gate (it kills a prop or resource with hull, the A1 one skips those): check it against the stock A2 scripts.
- [ ] LUA-30 (#740): `ReplaceObject` and `RemoveObject` emit Destroyed events, the original removes silently: a silent-retire edit tag in the sim (protocol bump); decide for the other remove calls (`RemoveAllObjects`, `RemoveAllCraft`, the A2 `RemoveObject`) once the docs or a measurement say so (#772).
- [ ] LUA-31 (#745): `A1.GetWarpSpeed` can read a stale value for one tick (it ignores an edit the host has taken but the sim has not applied); implement the pending step if a mission calls it.
- [ ] LUA-31 (#745): a script dropping a craft's warp below impulse mid-move is handled two ways (plain moves versus the attack approach); unify if it matters in play.
- [ ] Test (#742): find the root cause of the Windows stall in `injected_latency_stalls_but_never_diverges` once the next failure shows the per-peer report (check the Windows read-timeout residual risk in `transport/wire.rs`).
- [ ] Test (#741): the model_info temp-file collision on macOS was not reproduced; confirm the unique names cure it.
- [ ] Miners (#750): measure on the original bench how fast and which way a miner with a long beam circles its resource and at what radius; Armada I sits exactly on the new branch's threshold, so a rounding-level position difference is possible there.
- [ ] a2_fed01 (#751): `CountVisibleUnits` / `CountVisibleUnitsNearObject` honour the bool argument (real versus perceived team; read from the executable, static; STA64 ignores it today).
- [ ] a2_fed01 (#751): measure in the original when a freshly created object becomes visible to other teams; if it is a sensor-pass cadence, replace the one-tick rule (needs a spawn tick in the sim).
- [ ] HUD (#751): cinematic panel from the stock cfg `cinematic*Area` keys shows `PlayCinematicMovie` videos while play continues (`media::PanelScreen`).
- [ ] SIM-46 P0 (#753): the AI and presentation keys (`crewRetreatRatio`, `transportToEnemyOnHard`, `transportToFriendsOnNotEasy`, `transportSprite`, `eventTransport`) are typed only and have no consumer yet; AI-02 and the renderer read them or they are removed.
- [ ] SIM-46 P0 (#753): the boarding weapon keys (boarding size, delay and interval, commando team keys, torpedo boarding keys, `borgbore`) and the new transport `Features` switches (`derelict_latch`, `transport_approach`, `transport_attack_by_weld`, `transport_cloak_abort`, `transport_ally_hold_fire`) belong to the later SIM-46 packages.
- [ ] SIM-46 (#774): switch the transporter seam (`TransportRules::of` and the four functions in `combat/boarding.rs`, stub values) to the #753 content keys, digest them (D10), apply race `boardingStrength` and `canCommandeer` from data, and update `TYPED_CFG_KEYS` for `TRANSPORTER_*`.
- [ ] SIM-46 (#774): carrier shield restore at the end of a hold is not skipped when the carrier changed owner during the hold, and fills shields while a target hold on the same unit still runs: not measured; decide with the assault step.
- [ ] SIM-46 (#774): fix the doc comment of `a_scripted_capture_run_is_pinned` (the first party captures), add a test that an invalid hold-kind byte is refused on load, and an end-to-end test of a captured yard (queue lost, no refund, cargo and special energy kept).
- [ ] SIM-46 (#774): paired bench scene of stock A2 enemy boarding and derelict capture within tolerance of the unit tests; measure allied (different-team) derelict transfer, then fix the rule.
- [ ] SIM-46 (#774): `Captured` and refusal events and the `UnitView` transport fields for the HUD.
- [ ] UI-40 (#796): switch `family_can_transport` to the sim's rule when the Armada I transport rule lands, and `transport_state` to a sim accessor; stock `T` hotkey for TRANSPORT once the client has command hotkeys; transport cursors, refusal voice and event; acknowledgement voice (`OrderKind::of` has no Transport case); the capture button (`TRANSPORT_ATTACK`, assault units) once the sim takes `attack: true`; Armada I places transport on its speed rail (belongs to the Armada I stock HUD).
- [ ] Cutscenes (#755): measure on the original (A2 stock, A1, Roots) which voices, advisor lines, alerts and the minimap under-attack ping are silenced during a cineractive, whether events are dropped or queued, whether selection and orders are locked, and what happens to a selection, a drag or an armed build held when it starts.
- [ ] Cutscenes (#755): test that recalling an assigned control group does nothing during a cutscene (assign group 1 before the cutscene starts).
- [ ] Armada I HUD (#756): the LCARS strip still lists Metal, Latinum and Biomatter for Armada I and omits crew; check whether other Armada I units author `unitName` as a string key.
- [ ] Scenery picking (#757): right-click order targets (`input/orders.rs`), the order cursor (`input/context.rs`) and HUD targeting (`ui/hud/mod.rs`) are not filtered for kept scenery; settle them with the planet/moon picking fix as one shared "pickable" check.
- [ ] Render (#769): measure on the original bench whether resources in explored space outside sensor range can be clicked and ordered; add a HUD-targeting test for `target_seen`.
- [ ] Render (#769, #763): the doc comment above the wormhole pick-body test in `map/mod.rs` belongs to the test after it, and one comment line is over width: fix with the next change to that file.
- [ ] Render (#758): Borg glow (`team_Bglow`, lit groups, A1): compare their colour under a player colour with the original and whether it also ignores their own colour.
- [ ] Render (#758): the team material's specular colour is still added after the tint (tinted highlight regardless of owner colour); measure a team-face highlight against the original (A2 bench) before changing it.
- [ ] Render (#761): measure on the original which colour team-material faces of unowned objects (derelicts, neutral stations) show, and the player colour of the first lobby slot and of a mission's BZN team 1; debris from a destroyed subsystem draws team faces white whatever the owner.
- [ ] SIM-65 (#759): keep orbital stations out of the path planner as the original does (they still count as solid obstacles at their current position); freighters chase a circling station and dock within arrival distance: add the original's `resourcesCanHandle` filter for which station takes the cargo.
- [ ] SIM-65 (#759): the orbital beam uses the orbit owner (falls back to pick), so an off-ring station in reach of a planet fires without circling: decide with the income package; assert the freighter docking test per freighter; trace `a2_borg06` once Special_Attack lands.
- [ ] SIM-65 (#759): a station on two crossing rings orbits only the planet whose circle it is nearest to; an enemy orbital beam is drawn without a fog check of its own (not checked on screen).
- [ ] UI-33 (#760): measure on the original bench the command palette's top-left position on the 16:9 sheet and the 5:4 sheet and settle the rule (reference-canvas versus derived from the minimap).
- [ ] LUA-36 (#764): Armada II scripted music: a music override of the playlist with a per-sound stop and the music volume (a scripted track follows the UI volume today); one shared sound-id counter for `A2.AudioMessage`/`PlayMusic` and `Media:playSound` (ids can collide).
- [ ] SIM-61 (#764): the A2 opening hand-over tick depends on a client clock and the decoded clip length, outside the command stream; not replay-safe until then.
- [ ] Missions (#764): `AllowWarp`, `CollisionPrevention` and `ObstacleAvoidance` as sim state (no-ops today); `SetGreenAlert` / `SetRedAlert` (alert not modelled; alert calls: see #777).
- [ ] Campaign start (#765): measure what a literal 0 for crew or officer limit means in the original on a map whose team table has zeros (STA64 leaves such a seat without a crew pool or officer limit); the AI seats' first-tick orders are applied before the start grants (an AI first build order may be refused and retried a tick later).
- [ ] SIM-66 (#770): `A1_GOLDEN_QUEUED` ends before the final-leg brake (extend the run or shorten the course); an install-gated test asserting the same general rules (cruise, lag, no pivot, braking, stop short) on shipped data; a goal inside the turning-circle diameter ends the order without moving (decide whether to turn onto it).
- [ ] SIM-66 (#770): when the A1 model is switched on by content, rerun `tools/campaign_readiness.py --a1` before and after, the fed1 win and lose playthroughs and the opening camera timings; bench the Borg-physics ship lag and spin, a 90 degree order at cruise, and escort and attack approaches under the model.
- [ ] Mission openings (#772): `OPENING_TICKS` fits a set-up one script update late; one more update late still shows the default camera first. A removal is listed only for a unit alive at the call, and the removal log is bounded if no host takes it.
- [ ] HUD (#773): add an "Anchored" third HUD layout choice once UI-37's anchored form (ADR 0008 H5/H7 layout file) exists.
- [ ] A1 `InQueue` (#775): bind to a gate-travel queue state (ordered onto, waiting at, or being carried through a gate) once the sim exposes one; replace the speed approximation.
- [x] A1 bridge panel (#775, done by #799 and #815): a panel for bridge-scene calls and the clip viewscreen (clips play fullscreen today).
- [x] LUA-36 (#778, done by #827): guard order: a script guard command with a parameter (`a2_borg04`) still ends as a counted gap after the build receipt because the sim has no guard order; a new `Order` tag (protocol bump) is a ledger decision.
- [ ] a2_kling03 (#778): fails early on an empty team 1 in the idle run: cause not investigated.
- [ ] LUA-36 (#778): `IsVisibleToTeam(obj, n)` raises for an `n` that is no seated player (the original wraps and never raises); movement autonomy answers differ for an absent config key versus no process component.
- [ ] SIM-67 (#780): model the original's post-collision height return (pitch turning back, needs a saved pitch) if the playtest still shows the difference; replace the interim bounding sphere by the shield ellipsoid (SIM-32) and refit the bias constant.
- [ ] SIM-67 (#780): check on the bench the three-body head-on sides and the tie rule for equal heights; large craft in formation (slots overlap their own formation spacing) and the small residual height after arrival: revisit with SIM-32.
- [ ] CON-53 (#798): the `infinite` key is not read (needs an infinite-node flag in the sim); the Armada I freighter declares no capacity key in the form the sim reads (falls back to the default); the moon rule tests the presence of the scrap keys, not their values; check from the executable that Armada I keys a moon on the same scrap-key absence.
- [ ] UI-41 (#802): Armada I font: measure inter-glyph spacing and line pitch on the original; load only the fonts the game's own name pattern would load; a font strip with a comment line directly followed by a `page`/`char` line is not covered.
- [ ] WORKPLAN owed (#734): LUA-24 row is stale after review: it must say wormhole objects are placed as neutral props when the ODF has a `unitName`, the `a1_compat` teleport now acts on map gates, id-order pairing is the known gap for the link package, and no partner pointers or format number are carried.
- [ ] WORKPLAN owed (#737): LUA-25 row must say unresolved map labels are a handle gap (listed in `unsupported()`, recorded as `handle_gap`), the taint is global (known over-block, pinned by a test), and the client host must not pass the list until the taint is narrowed.
- [ ] WORKPLAN owed (#753): SIM-46 row *owns* cell must add the typed-key files in `sta_content` (`def`, `unit`, `race`, `rules`, `coverage`) for P0, and *done when* must add "typed keys expose the data values and move no content digest until the sim reads them".
- [ ] WORKPLAN owed (#759): add the SIM-65 row (orbital mining stations); text in the PR body's requested-row section.
- [ ] WORKPLAN owed (#776): add the LUA-38 row (team strength engine variables); text in the PR body's requested-row section.
- [ ] WORKPLAN owed (#770): SIM-66 row still says "with the stock numbers" and "assert the measured targets": reword to the general rules, and fix its "unblocks" (SIM-67 is Armada II separation, not the stopgap removal; the SIM-68 reference is not a WORKPLAN row).

### Follow-ups from the 2026-11-15/16 merges

- [ ] Armada I music (#762): ambient versus battle selection in the original (which key or event; STA64 plays the track list as calm only); Armada I menu music (`flow.rs` reads only the single-player key); the mission script `Music`/`StopMusic` calls are still unhandled in `ui/presentation.rs`.
- [ ] Armada I health (#790): the bench and the static read disagree on whether death comes at an empty pool; mods must later be able to choose another health model (shield plus hull) per profile or content; not modelled: zero-pool shield delay, repair-facility regeneration, the unread Armada I target-score rule.
- [ ] Armada I stock HUD (#795): remaining are the speed rail (specials, transport, tractor beam, merged across a selection), multi-selection and system icons, dialogs and tooltips, the officers readout and dilithium bar, build-list paging, locked orders (alert states, stand down, patrol, guard, decommission, self-destruct, repair), transport and upgrade slots (UI-40 and later); stock behaviour of the Orders, Repair and Stand Down entries for stations is not measured.
- [ ] Armada I stock HUD: resource readout text alignment (fix in work).
- [ ] Clips (#799, #815): measure where the original's bridge scene puts its clip during play and during a cineractive, then replace `HudLayout::viewscreen`; draw the bridge backdrop and layers if wanted. Accepted for now: a clip keeps playing unseen while the HUD briefly has no panel, and keeps playing while paused; `BridgeScene::from_call` runs twice per call.
- [ ] Clips (#815): the fed1 wide shot is held too long and the clip starts late because `A1.InQueue` is speed-based; fix with the gate-queue seam (see the #775 line).
- [ ] SIM-46 (#800, #823): remaining are planets, assault capture (transport attack), the Roots and Fleet Operations deltas, the HUD transport button and hotkey, the Armada I speed-rail button and mission bindings.
- [ ] SIM-46 (#823): the Armada I transport rule is read from the executable (static) and not measured (needs an Armada I scene harness): range, shield gate and kills; what follows an Armada I capture (production queue, officers, unit cap) is not established; the shield drop on an own-team target is assumed equal to the measured allied case; the gate reads the hull pool when a unit has no shield maximum and must move with any change to the single-pool representation; an allied target captured by an enemy before the hold ends gets full shields back (fix with the captured-carrier shield restore); no test covers planets or refused capture on Armada I, nor a save and reload mid-transfer.
- [ ] Tooltips (#819): the first frame of a tooltip uses the previous frame's box size, so changed text may sit for the old size for one frame.
- [ ] Mission audio (#820): event voice priority and queue rules are not built (only the suppression is); `MuteSounds` is handled (#843: a multiplier on the music, effect and voice buses; script streams and movie sound exempt, and movie sound follows only the master volume until a UI or movie slider exists; its level on the original and the movie exemption are not measured); script streams are on the UI bus and ignore the music volume; on the original, check that two messages one tick apart mix, that an event voice during a script message is dropped and what the ambient track does across a cineractive while script music plays; the suppression and hold were read from the Armada II executable only (Armada I, Fleet Operations not read).
- [ ] a2_kling01 (#820): a shot aimed at a dead handle is dropped and its `CineractiveStart` wait returns at once, so late steps can start together; a staged ship dies before its shot: why the object is already gone is a sim divergence.
- [ ] LUA-32 (#777): measure on the original whether assist at yellow moves to the ally or fires in place (read: issues an attack order as at red); whether automatic target pickup counts toward the ally attack counters; add the `defaultGreenAlert` and `canAttack` class keys to `sta_content` (gate assist on `canAttack`) and the profile bit for the human cloak red-to-yellow rule; the pursue branch (which campaign teams count as AI-controlled is not established); trace the a2_kling07 and a2_kling01 readiness rows (causes inferred, not traced); a hit whose shooter is already removed never stamps the under-attack state (original not established); the queue-command reset of the last-hit stamp has no STA64 order yet; Armada I green flees when hit (not modelled).
- [ ] SIM-68 (#805): a mod manifest key naming the play-physics file (today a layered file of the same name); the Armada I model also reads `maxRoll` and `leftRightInertia` when the roll and skid follow-up lands; `rom2` and `finale6` end lost sooner in the no-input probe (only `fed5` was traced, same cause); regenerate `docs/odf_keys.csv` in the docs batch if it tracks keys read; headless skirmish run under the model.
- [ ] Orbital income (#803, #813): the node's second resource has typed data but the sim's node table holds one amount per node; the empty-node behaviour, the single-player difficulty modifier, the absent-key defaults, the beam phase against the drawn pulse and the freighter's use of the node's mining rate are not established; Armada I's inexhaustible-moon key is typed nowhere and not read; freighters ignore the weapons-off flag (decide whether it should stop them as it stops an orbital station); the family content digests moved, so older saves and replays of those profiles are refused.
- [ ] Orbital mining: earlier income tests built a weapon shape the real loader never produces; income tests must go through the loader (fix in an open PR).
- [ ] Pathfinding: an orbiting station changes the obstacle layout every tick and defeats the navigation grid cache (fix in work).
- [ ] Armada I campaign win (#825): the original's screen sequence after a win (score or debrief screen, next briefing) is not known; replace the interim debrief rule.
- [ ] Roots supply (#826): confirm from the Armada II executable that the table's supply slot is applied for mission maps; ask the Roots author whether the tutorial's first map really starts with zero supply or something else grants it.
- [ ] Script guard (#827): bench measurement of the original's guard geometry; no readiness run was repeated after the later main merges.
- [ ] Armada I crew and officers (#828): model the team crew capacity and the executable's crew growth (SIM); distinguish a map that cannot be read from a map with no team table (both get the header start today); charge the crew cost from the team pool when a build starts if the original does (not established); confirm in the executable which setups use the header start and what the team multiplier is; starbase officer upgrades raise the officer cap.
- [ ] Cursors (UI-43, #830): UI-44 (armed picks, placement valid and invalid) and UI-45 (race variants, size sets) are not built; the hover-select cursor and the move cursor over ground are not established; cursor size in stock mode against the original at native resolution is not measured.
- [ ] Armada II titles and fade (#835): decide between "delayed fade out after a fade out" and "a new shot clears the fade" on a second Armada II mission; title fade-in flag, title fade-out, font and colour are not measured and not applied (colour in work); title-over-fade draw order is not measured (a caption shown while black is held may be hidden); wrapped lines of a centred title are not re-centred; map-given instance names of objects are not shown (fix in work).
- [ ] Missions: mission winnability is proven end to end (scripted player run) for one Armada I mission only; no Armada II, Fleet Operations or Roots mission has a proven natural win, and the no-input readiness runs do not show winnability. Add scripted-player playthrough tests per profile.
- [ ] a2_kling01: stations and turrets honour the alert gate (read from the executable, static); turrets set green by the script stay silent after the intro: confirm on the original that they are silent there too, or find what sets them red again.
- [ ] a2_kling01: with green craft holding position the intro's scripted destruction steps are no longer reached; whether a craft with engines disabled coasts is not established.
- [ ] a2_borg04: a nil handle in a call argument stops the mission early (separate from the guard order).
- [ ] Scripted shield percentage: one test scene shows no hull change after a shield-percentage call; cause not established.
- [ ] Roots mission scripts reuse timer ids: the id of an expired timer must be reusable (fix in work); a script error inside a hook must be visible to the player.
- [ ] Cutscenes: units outside the local player's sight are not drawn during a cineractive (fix in work).
- [ ] Reveal cheats (UI-46): the chords are not rebindable; whether cloaked enemies are revealed is to be stated; which original key drives which layer is not established.
- [ ] WORKPLAN owed (missions lane): the order after LUA-31 is SIM-46 packages and LUA-33/LUA-34, then LUA-32, then LUA-35, and the snapshot, Edit and protocol numbers written in rows LUA-32 to LUA-35 are stale: the coordinator re-assigns them when the SIM-46 needs are stated.
- [ ] WORKPLAN owed (LUA-37): the row says a forced end is refused outside dev or cheat mode, while LUA-29 found the original needs no dev mode in single-player missions: settle when LUA-37 starts.

## Backward compatibility gaps (Armada I, Armada II, Fleet Operations)

What the original games read or do that STA64 does not yet, so original
content and mods keep working. Compared on 2026-11-16 against `main` at
`37283fe8`.

Source tags, on every line:

- `[A1]` Armada I only.
- `[A2]` Armada II only (new in Armada II).
- `[A1 A2]` both stock games.
- `[FO]` Fleet Operations addition (its hook library).

Fleet Operations runs on the Armada II executable, so every `[A2]` and
`[A1 A2]` line also applies to Fleet Operations content.

How to read it:

- Evidence grade for the key and name lists: read from the executable, static,
  not measured. Usage counts behind the ordering are read from data (`docs/data/`).
- A name counts as covered when it is a string literal in `crates/**/*.rs` (the
  rule `tools/data_inventory.py` uses). That shows the name is mentioned, not
  that the behaviour matches. Names read through a generic loader (`cfg_h.rs`,
  `gui_cfg.rs`) show as missing even where a mod's value would be picked up.
- "Not named" for a file means its name does not occur in the source; a file
  reached through a pattern or another file's reference can still load. Those
  lines are things to verify.
- Nothing here was run in game. Each item needs a WORKPLAN row and a claim
  before code; `[plan X]` marks a row that exists whose keys are not in code yet.

### Decided (Tammy, 2026-11-16)

- [ ] ODF key lookup is reproduced per profile. `[A1]`: a key matches the first
  line in file order whose start equals the key (case-insensitive) and which
  contains `=`; there is no word boundary, so an earlier `keyX = ...` line
  shadows `key`, and a file's own lines come before its includes. `[A2]` and
  `[FO]`: the whole key must match, case-insensitive. `sta_odf` is exact for
  every profile today; prefix lookup belongs under `Profile::Sta1Classic` only.
  Still to read for `[A2]`: which of two duplicate keys wins. Fixtures: `keyX`
  before `key`, own line against included line, a line without `=`.
- [ ] `[A2]` Armada II saved games must be readable. No reader and no WORKPLAN
  row yet (Armada I saved state is CON-40). First step: an inspection tool that
  walks a save and reports every record, before any import into the sim.

### Loader and parser rules

- [ ] `[A1 A2 FO]` Class defaults: each object class has defaults that apply when
  an ODF omits a key. Audit STA64's defaults class by class.
- [ ] `[A2]` Subsystem key families built from the subsystem name:
  `<system>SparkTime` is unread (`HitPoints`, `HitPercent`, `RepairTime`,
  `CrewLoss` and `TargetHardpoints` are read).
- [ ] `[A1 A2]` Physics cascade: Armada I movement keys come from the shared
  physics file first, then the ship ODF; Armada II reads 48 keys in the same
  order. Check `sta_content` applies that order per profile.
- [ ] `[A1 A2]` `events.odf` / `events.dat`: sounds and `SoundPriority` are parsed;
  `Marker` (radius, shrink, time, colour) and `Text` lines are not.
- [ ] `[A1 A2 FO]` Table values (`"name" value` rows after a key): check a reader
  for each of `hitChance`, `damageBase`, `possibleCraftNames`,
  `<system>TargetHardpoints`, `weaponHardpoints#`, `commandHotkeys`,
  `commandNames`; `[FO]` `validTargets`, `disable<System>Percent/Time`,
  `repairRate`, `beneficialBuild`, `supplyClasses`, `drainSpecialEnergyPercent`.
- [ ] `[A2]` Starfield ODF keys `bstar.#`, `mstar.#`, `cstar.#` and their `.scale`
  forms are built at run time; check a reader exists.
- [ ] `[FO]` Virtual directories and file precedence (`docs/NATIVE_FEATURES.md`
  row is `todo`, no WORKPLAN row). Rule read from the hook: entries order by
  name (ASCII case fold), then owning mod (an entry with a mod before one
  without; two mods by priority), then primary root, then loose before packed.
  Two different mods of equal priority have no tie-break. Compare with `sta_vfs`.
- [ ] `[FO]` Producer queue accounting (start, cancel, finish, tech check at
  start): compare with SIM-08/20/33 and file only the differences.

### Fixed-name and per-format files

ODF:
- [ ] `[A1 A2]` `light.odf`, `component.odf`, `techlvl.odf` not named.
- [ ] `[A2]` `editmenu.odf` (map-editor menus: `menuTitle`, `menuName<n>`, `item<n>`, `forceToNeutral`) not named.
- [ ] `[FO]` `loadscreen.odf` (load bar and tips) not named.
- [ ] Check depth of the named ones: `[A1 A2]` `teamcolor.odf`, `button.odf`, `audio.odf`, `races.odf`; `[A2]` `allcommands.odf`; `[FO]` `odf/system/stars_*.odf`.

CFG:
- [ ] `[A1]` `cca.cfg`, `collision.cfg`, `logfile.cfg` not named (`woncfg.cfg` is the retired online service: out of scope).
- [ ] `[A2]` `gui_cardassian.cfg`, `gui_species8472.cfg` not named (`gamespy.cfg`: out of scope).
- [ ] `[FO]` per-aspect sets `gui_<race>{4x3,5x4,16x10,16x9}.cfg` for Borg, Dominion, Federation, Klingon and Romulan (only `gui_16x9.cfg` and `gui_federation16x9.cfg` are named), `gui_4x3.cfg`, `gui_5x4.cfg`, `gui_16x10.cfg`, `gui_defaultrace.cfg`, `gui_interface_misc.cfg`, `gui_observer*.cfg`, `gui_mapedit*.cfg`.
- [ ] `[A2]` cfg keys built at run time: `background_<n>`, `tabBmp_<n>`, `tabBmpH_<n>`, `playerButtonRect<n>`, `<button>_click.wav`, `<button>_rollover.wav`.
- [ ] `[A1 A2]` value kinds: rectangle, sprite name, colour, cursor, point pair. Check `gui_cfg.rs` has a typed reader for each.

SPR (syntax is covered: every directive, material, track kind, interpolation, row flag and emitter parameter in the three installs has a name in source):
- [ ] `[A1 A2 FO]` an unknown emitter type silently becomes `spray`; warn instead.
- [ ] `[A1]` `@tmaterial=wormhole`, `[A1 A2]` `alphathreshold`: named; check they are drawn, not only parsed.
- [ ] Sprite files not named: `[A1 A2]` `gui_map.spr`; `[A2]` `gui_cardassian.spr`, `gui_species8472.spr`; `[A2 FO]` `gui_credits.spr`; `[FO]` `gui_dominion.spr`, `systembackgrounds.spr`, `systemimages.spr`, `miniimages.spr`, `fleetops_comp.spr`; `[A1 A2 FO]` `tex_anim.spr`, `tex_anim_gui.spr` (normally `@include`d).
- [ ] Font sets: `[A2]` the font file name is built from a name and a size; only `fontfinal4_10/13/16` are named. Unnamed: `[A2 FO]` `fontfinal4_12/15/19/20/24`; `[FO]` `fontfinal4_9/14/17/18/18_large/18_small/30/31/38/48`, `fontsmall_5/6/8/10`.
- [ ] Not compared at all: animation timing, cursor `@origin` hot spots, `@snap_to_pixel` and `@chromakey` drawing, emitter physics.

AIP:
- [ ] `docs/data/aip.md` marks 3 of the 77 variables stock files set as read.
- [ ] `[A1]` `UNIT_CONSTRUCTION_PROGRAM` rows and the `aipdef.h` constants (`NUMBER_TO_HAVE`, `NUMBER_TO_BUILD`, `RATIO_TO_BUILD`, `RATIO_TO_HAVE`; placement `CENTER_OF_BASE`, `PERIMETER`, `OUTSIDE_OF_BASE`, `NEAR_ENEMY_BASE`, `NEAR_ENEMY_TROOPS`) have no reader.
- [ ] Build-list row widths in stock data are 5, 4, 3 and 2 fields; check each maps to the right fields.
- [ ] File selection not named: `[A1]` `<race>_offensive_list.aip`, `<race>_defensive_list.aip`, `default_offensive.aip`, `default_defensive.aip`; `[A1 A2]` `default.aip`, `newdefault.aip`; `[A2]` `_endgame_list.aip`.
- [ ] `[A1 A2]` `strategic_ai_config.txt` not named.
- [ ] Set by stock files but not confirmed as read by the executables (verify before implementing): `MAX_EVAL_*`, `MAX_EXEC_ATTACK_BASE`, `MAX_EXEC_ESCORT`, `MAX_EXEC_EXPLORE`, `MAX_EXEC_RESOURCE`, `max_attack_force`, `min_attack_force`, `min_defense_force`, `max_perimeter_force`, `min_perimeter_force`, `min_resource_force`, `patrol_one_in_n`, `recompute_strategy_period`, `threat_proximity_modifier`; `[A2]` `MAX_SCOUTING`, `recycle_units`.

TT (line forms are covered: `<odf> 0`, `<odf> N <prereq>...`, `-1`, `-2`, `#include`, names without `.odf`):
- [ ] `[A2]` tech flags (`gTech...`) parse as withheld items; check that finishing the research makes the flag available. Not verified either way.
- [ ] `[A2]` game-option techtrees not named: `earlytech.tt`, `nosci.tt`, `nospweap.tt`, `nosuper.tt`, `demotech.tt`. `[FO]` `devtech.tt`.
- [ ] `[A2]` mission techtrees `a2_<race>02..10.tt`: load each once.

XML (Fleet Operations techtree):
- [ ] `[FO]` not in `techtree.rs`: `customEval` with requirement `ID`, `numSatisfied`, `countConstructions` (default true) on `cap` and `capacityList`, `contextODF` on `capList`, the `<enemy>` condition, `<parentInclude/>`. Stock FO 4 data uses none of them; mods can.
- [ ] `[FO]` rules to verify: names compare without case; booleans are `true`/`false` or a number (non-zero true); an ODF name that resolves to nothing skips the element; an include of the file's own name reads the next mod level down; a capacity pool sums negative weights first, then fails as soon as the running total passes `maxCapValue`; pool names are matched without case, newest definition first.
- [ ] `[FO]` `race` on `<object>` is used twice in FO data; find what the game does with it.
- [ ] `[FO]` `devtech.xml`, `freetech.xml` not named; `TECHTREE_DEFAULT_FILE` (header) unread.

Other text files opened by fixed name:
- [ ] `[A1]` `gamekey.map`; `[A1 A2]` `lang.ini`, `texture.dat`; `[A1]` `lang.txt` not named.

### Game modes, saves, missions, network

- [ ] `[A2]` Capture the Flag (`ctfUnits#`, race `canPlayCaptureTheFlag`) and Collect Latinum (`canPlayCollectLatinum`): absent.
- [ ] `[A1]` Instant-4X keys `myScrap`, `myPilots`, `hisScrap`, `hisPilots`: absent.
- [ ] `[A2]` Admiral's Log: `ADLOG_CFG.h` (28 names) and its screens (build, battles, ships, economy, military, score, tally, timeline): none read.
- [ ] `[A1]` Saved-game state per object is not imported (`docs/A1_BZN_SUPPORT.md`; CON-40). Most state: craft orders and alert status, per-object flags, AI team state, timed effects (disable, drain, hyper, invincible, change team, give vision, scramble interface).
- [ ] `[A1]` Mission calls (`crates/sta_mission/src/api/a1/README.md`): of 172 bound calls, 81 change the game, 35 raise an event nothing consumes, 42 are retained with no effect, 14 always raise. No stock mission has been run through the table.
- [ ] `[A2]` Mission calls: `DisableLifeSupport(int, bool)` and `DisableSensors(int, bool)` lack the original signature. Of the methods no stock mission calls, 98 have no binding (community missions can call them):
  `AddBillboardEffect`, `AddToAdLog`, `AddToLoad`, `AddToSave`, `AllCapitalShipsDestroyed`, `CineractiveShotTrackLeft`, `CineractiveTargetOffset`, `ClearFog`, `CountUnitsInRectangle`, `CountUnitsInRectangle2`, `CountVisibleUnitsNearPoint`, `CreateNewDefendFleet`, `EnginesActive`, `EnginesDestroyed`, `EnginesDisabled`, `FindGameEvent`, `GetBiomatter`, `GetCombatSpeed`, `GetCurrentCommand`, `GetInputControlPointer`, `GetMaxDilithium`, `GetMaxLatinum`, `GetMaxMetal`, `GetNearestObject`, `GetNearestObjectNearPath`, `GetNearestRepairFacility`, `GetNearestVehicle`, `GetRace`, `GetSpecialEnergy`, `IsAITeam`, `IsConfirmedAlly`, `IsConfirmedEnemy`, `IsConfirmedNeutral`, `IsEnabled`, `IsRaceOfFlag`, `IsUnconfirmedAlly`, `IsUnconfirmedEnemy`, `IsUnconfirmedNeutral`, `LifeSupportActive`, `LifeSupportDestroyed`, `LifeSupportDisabled`, `MonitorInputControls`, `ObjectHasFlag`, `RemoveFromLoad`, `RemoveFromSave`, `ScriptCreate_dirty_int`, `ScriptCreate_hot_float`, `ScriptCreate_hot_int`, `ScriptCreate_timer_int`, `ScriptDestroy_dirty_int`, `ScriptDestroy_hot_float`, `ScriptGet_ALWAYS_EVALUATE`, `ScriptGet_WORLD`, `ScriptGet_isCineractiveCancel`, `ScriptGet_isCineractiveDone`, `ScriptGet_moons_team_1`, `ScriptGet_moons_team_2`, `ScriptGet_moons_team_3`, `ScriptGet_moons_team_4`, `ScriptGet_moons_team_5`, `ScriptGet_moons_team_6`, `ScriptGet_moons_team_7`, `ScriptGet_moons_team_8`, `ScriptGet_strength_team_1`, `ScriptGet_strength_team_2`, `ScriptGet_strength_team_3`, `ScriptGet_strength_team_4`, `ScriptGet_strength_team_5`, `ScriptGet_strength_team_6`, `ScriptGet_strength_team_7`, `ScriptGet_strength_team_8`, `ScriptGet_tried_warp_team_1`, `ScriptGet_tried_warp_team_2`, `ScriptGet_tried_warp_team_3`, `ScriptGet_tried_warp_team_4`, `ScriptGet_tried_warp_team_5`, `ScriptGet_tried_warp_team_6`, `ScriptGet_tried_warp_team_7`, `ScriptGet_tried_warp_team_8`, `SensorsActive`, `SensorsDestroyed`, `SensorsDisabled`, `SetCombatSpeed`, `SetPathSegmentSpeedModifier`, `SetPathType`, `SetStarbaseUpgrades`, `SetTeamText`, `SetVelocity`, `SetWormholeExpireOnClose`, `ShieldGeneratorActive`, `ShieldGeneratorDestroyed`, `ShieldGeneratorDisabled`, `TeamAttackAll`, `TeamKamikaze`, `TriggerGameEvent`, `WeaponsActive`, `WeaponsDestroyed`, `WeaponsDisabled`
- [ ] `[A2]` Random map (CON-49): the class position check and the exact distribution table are not reproduced, so a seed does not give the original's map.
- [ ] `[A1 A2]` Original multiplayer wire compatibility is not planned (new lockstep protocol). State it as a deliberate break in `docs/NET.md`.
- [ ] `[A2]` Resolution tiers and font groups: which art and font set a resolution selects.

### Suggested order

1. `[A1]` prefix lookup per profile (decided); class defaults audit.
2. `[A1 A2]` weapon and ordnance base keys (`hitChance`, to-hit modifiers, target masks, `ignoreShield`): every weapon in all three games.
3. `[FO]` high-volume keys: `extraWeaponHardpoints`, `explosionHit`, `explosionShieldHit`, cannon cycle keys `[plan SIM-48]`, experience `[plan SIM-42]`, replace-weapon remainder `[plan SIM-43]`, ordnance effects.
4. `[A1 A2]` map-object effects (nebula, wormhole, black hole).
5. Special weapon classes by data weight.
6. `[A2]` saved games (decided), stations, trade, recycling `[plan SIM-45]`, Admiral's Log; hotkeys; AIP.

### ODF keys (762 names the originals read that STA64 does not)

Grouped by the object class that reads the key, most-used first; within a line, most-used key first.

- [ ] [A1] craft (3): `criticalHitPercent`, `dockingSpeedFactor`, `yawRotationRate`
- [ ] [A1 A2] craft (8): `intrinsicValue`, `criticalTargetHardpoints`, `hullTargetHardpoints`, `damagedScan`, `isHero`, `selfDestruct`, `defaultGreenAlert`, `alwaysUseSpecialWeapons`
- [ ] [A2] craft (28): `maxExtraWeapons`, `healthRate`, `shipType`, `trekPhysicsFile`, `footprintBuffer`, `northBuffer`, `physics`, `southBuffer`, `shipIsBuiltByEvolution`, `recyclesInPlace`, `addToPlanner`, `canAttackPlanets`, `eastBuffer`, `canOnlyAttackPlanets`, `downBuffer`, `missileDetonator`, `percentNebulaWarn`, `upBuffer`, `westBuffer`, `%sCrewLoss`, `%sHitPercent`, `%sHitPoints`, `%sRepairTime`, `%sSparkTime`, `D4p`, `shipIsBuiltByShips`, `xfireBal`, `xselfbal`
- [ ] [FO] craft (47): `SpecialEnergyDisplayMode`, `recycleBiomatter`, `recycleLatinum`, `recycleTime`, `worthXP`, `originalClass`, `XPlevel`, `nextrankXP`, `nextrankClass`, `giveIsAllowed`, `overlaySod`, `perceivedNeutral`, `destroyedBiomatterCost`, `rankupBiomatter`, `IamGameToTheDeathCondition`, `canRepairshipRepair`, `destroyMeWithResearchStation`, `avatarBoxIcon`, `avatarBoxTooltip`, `destroyFootprint`, `turnToFire`, `preferredSelect`, `constructionVoice`, `allowBuildQueuePop`, `createFootprint`, `beneficialBuild`, `beneficialBuildRadius`, `showChoiceDialog`, `freighterSize`, `beneficialBuildForce`, `beneficialBuildInvert`, `defaultAlertStatus`, `defaultMovementAutonomy`, `defaultSpecialWeaponAutonomy`, `destroyedCrewCost`, `destroyedDilithiumCost`, `destroyedLatinumCost`, `destroyedMetalCost`, `explosionChunks`, `footprintAligned`, `healthBeamRatio`, `moduleGroup`, `moduleType`, `rankupCrew`, `rankupDilithium`, `rankupLatinum`, `rankupMetal`
- [ ] [A1] weapon (5): `buttonPriority`, `slowToHitModifier`, `toHitSlowModifier`, `toHitWarpModifier`, `warpToHitModifier`
- [ ] [A1 A2] weapon (19): `hitChance`, `stopToHitModifier`, `toHitStopModifier`, `needTarget`, `buttonSlot`, `switchToAttack`, `targetLocation`, `beneficial`, `buttonBorder`, `AItargetLocation`, `targetAlliedCraft`, `cloakEnable`, `targetEnemyBuildings`, `targetEnemyCraft`, `targetAlliedBuildings`, `offSound`, `crewAffectsWeaponDelay`, `regularWeaponDelay`, `fireEventID`
- [ ] [A2] weapon (10): `impulseToHitModifier`, `toHitImpulseModifier`, `speakOnAttack`, `attackSpeech`, `hotkeyLabel`, `ignoreValidAsTarget`, `assimilatable`, `targetEnemyPlanets`, `targetEnemyPlanetsOnly`, `warpEnable`
- [ ] [FO] weapon (10): `overrideSubSystem`, `showRange`, `validTargets`, `overrideCeaseFire`, `visibleRange`, `preserveStatus`, `showInMultiSelect`, `showEffectIcon`, `hasNegativeEffectOnOwner`, `showAsEffectIcon`
- [ ] [A1] ordnance (1): `shotSound`
- [ ] [A1 A2] ordnance (10): `spriteDuration`, `shotColor`, `ignoreShield`, `hitSound`, `shieldDuration`, `Length`, `specialValue`, `shotInfo`, `lodShift`, `shieldGeometryName`
- [ ] [A2] ordnance (1): `forceInvalidateTarget`
- [ ] [A1 A2 FO] ordnance (2): `lightFalloffRange`, `lightFalloffStart`
- [ ] [FO] ordnance (6): `explosionHit`, `explosionShieldHit`, `registerTargetHit`, `explosionSprite`, `explosionDuration`, `explosionSizeFactor`
- [ ] [A1] game object (3): `keymapLabel`, `decommissionValue`, `mapColor`
- [ ] [A1 A2] game object (12): `avoidanceClass`, `weldingRadius`, `aiName`, `mapIcon`, `distanceBelowGrid`, `mapSize`, `mapSprite`, `moveBelowCombatArea`, `collisionRadius`, `shieldPad`, `weldingSpeed`, `weldingSpriteName`
- [ ] [A2] game object (5): `hotkeyLabel`, `ambientMinimumFactor`, `ambientMaximumFactor`, `directorRadiusScale`, `Odf_Names`
- [ ] [A1 A2 FO] game object (2): `lightFalloffRange`, `lightFalloffStart`
- [ ] [FO] game object (9): `extraWeaponHardpoints`, `clearsFog`, `ignoreInterface`, `selectHighlight`, `ignoredAsTarget`, `priorityBuild`, `curSpecialEnergy`, `hullTooltip`, `renderIgnoreCameraDistance`
- [ ] [A1 A2] mine (3): `omegaTurn`, `shotAccel`, `detectRadius`
- [ ] [A2] mine (1): `canAttackStations`
- [ ] [A1 A2] missile (2): `omegaTurn`, `seekTime`
- [ ] [A1 A2] cannon (3): `maxShots`, `minimumShotInterval`, `saveFire`
- [ ] [A2 FO] cannon (1): `eventFire`
- [ ] [FO] cannon (15): `shotCycleResetTime`, `saveFireCyclePoint`, `usePrimaryTarget`, `baseTargets`, `hitMode`, `targetLoop`, `autoTargetClosest`, `hitCloaked`, `HardpointShift`, `maxExtraTargets`, `hitInvalid`, `targetSelf`, `targetTerrainOnly`, `forceAudioVisual`, `alwaysDeductValues`
- [ ] [A2] producer (1): `alert`
- [ ] [A1 A2] phaser (1): `flareSprite`
- [ ] [A2] phaser (1): `flareRadiusMultiplier`
- [ ] [FO] custom odfload (1): `XPlevel`
- [ ] [FO] replace weapon (29): `replacementMaintainOrders`, `replacementCreationType`, `replacementRegisterAsNew`, `replacementkeepName`, `replacementKeepOwner`, `transferStats`, `replacementEvent`, `selectMode`, `isToggledOn`, `replacementDontRemoveFootprint`, `replacementInstantCycle`, `ignoreTechTree`, `replacementResetSpecialEnergy`, `concurrentChilds`, `replacementResetCrew`, `replacementFindOpenSpot`, `warpIn`, `warpInSpeed`, `replacementEnemyOfAll`, `replacementAnimate`, `replacementFractionSOD`, `transferExtraWeapons`, `ignoreSameClass`, `replacementUseLabelAsClass`, `requiredBiomatter`, `requiredCrew`, `requiredDilithium`, `requiredLatinum`, `requiredMetal`
- [ ] [A1 A2] toggle weapon (1): `needTarget`
- [ ] [FO] uni ordnance (32): `affectedSystems`, `damageMode`, `disableWeaponsPercent`, `disableWeaponsTime`, `extraWeapon`, `extraWeaponPercent`, `extraWeaponTime`, `disableEnginesPercent`, `disableEnginesTime`, `disableSensorsPercent`, `disableSensorsTime`, `disableLifeSupportPercent`, `disableLifeSupportTime`, `boardingPercent`, `boardingRate`, `boardingTime`, `drainSpecialEnergyPercent`, `drainSpecialEnergyRate`, `drainSpecialEnergyTime`, `boardingCrewSubtract`, `disableShieldsPercent`, `disableShieldsTime`, `drainCrewPercent`, `drainCrewRate`, `drainCrewTime`, `drainHullPercent`, `drainHullRate`, `drainHullTime`, `drainShieldPercent`, `drainShieldRate`, `drainShieldTime`, `extraWeaponTimerReset`
- [ ] [A1] race (4): `displayName`, `officerUpgradeODF`, `officerName`, `officerOperate`
- [ ] [A1 A2] race (6): `weaponRed`, `weaponYellow`, `retreatStrength`, `officerRes`, `officerTooltip`, `officerVerboseTooltip`
- [ ] [A2] race (11): `creationLabel`, `insufficientOfficersEvent`, `recycleDilithiumFraction`, `recycledLabel`, `recycleMetalFraction`, `atmosphereTint`, `canPlayCollectLatinum`, `affectedByPsionicInsanity`, `canPlayCaptureTheFlag`, `canPlayColonizePlanets`, `canTrade`
- [ ] [FO] race (17): `allowGiveBiomatter`, `allowGiveCrew`, `allowGiveDilithium`, `allowGiveLatinum`, `allowGiveMetal`, `canGainXP`, `AIFreightersPerMoon`, `giveToOtherRaceBiomatterRatio`, `giveToOtherRaceCrewRatio`, `giveToOtherRaceDilithiumRatio`, `giveToOtherRaceLatinumRatio`, `giveToOtherRaceMetalRatio`, `usesBiomatter`, `usesCrew`, `usesDilithium`, `usesLatinum`, `usesMetal`
- [ ] [A1 A2] holding beam (6): `lengthScale`, `radiusScale`, `boardingDelay`, `boardingInterval`, `boardingSize`, `targetRange`
- [ ] [A1] mining station (1): `dockingBuffer`
- [ ] [A1 A2] mining station (1): `freighterName`
- [ ] [A2] mining station (1): `checkValidResourceInterval`
- [ ] [A2] smooth (8): `backwardAccel`, `pitchAlpha`, `pitchOmega`, `rollCoupling`, `pitchAlphaFractionAtRest`, `pitchOmegaFractionAtRest`, `pitchDefault`, `pitchDefaultSpeed`
- [ ] [A1 A2] fireball explosion (5): `Length`, `dbName`, `shockwave`, `shockwaveDelay`, `shockwaveMinimum`
- [ ] [A2] fireball explosion (1): `blameTheShooter`
- [ ] [A1 A2] nebula (19): `blue_glow_range`, `green_glow_range`, `one_in_n_chance_of_light_source`, `red_glow_range`, `damageValue`, `min_speed_change`, `speed_change_period_divisor`, `speed_change_range`, `disableCloak`, `disableCrew`, `disableShields`, `hideContents`, `craft_speed_coefficient`, `craft_speed_limit`, `regenValue`, `repairValue`, `clearEffects`, `damageCrewModifier`, `damageHullModifier`
- [ ] [A2] nebula (3): `canWeBuildInHere`, `nebulaType`, `craft_max_speed_fraction`
- [ ] [A1] shipyard (1): `dockingBuffer`
- [ ] [A1 A2] shipyard (1): `buildAnimation`
- [ ] [A2] shipyard (7): `decommissionFacility`, `repairAnimation`, `alwaysRecomputeRallyPoint`, `dilithiumRecyclingFraction`, `metalRecyclingFraction`, `recycleExchangeRate`, `fix`
- [ ] [FO] shipyard (1): `alliedRepairUpgrade`
- [ ] [A1 A2] tractor beam (5): `lengthScale`, `radiusScale`, `targetRange`, `omegaType`, `targetSpeed`
- [ ] [A1 A2] utritium beam (3): `objectName`, `scaleCount`, `scaleValue`
- [ ] [A1 A2] tech assim beam (3): `lengthScale`, `radiusScale`, `targetRange`
- [ ] [A2] tech assim beam (1): `assimilationTime`
- [ ] [A1 A2] resource extraction beam (3): `lengthScale`, `radiusScale`, `drainRate`
- [ ] [A2] resource extraction beam (1): `conversionRate`
- [ ] [A1 A2] borg bore (3): `lengthScale`, `radiusScale`, `attackRange`
- [ ] [A1 A2] research pod (1): `isUpgrade`
- [ ] [A2] research pod (4): `isShipUpgrade`, `upgradeLevel`, `upgradeMultiplier`, `upgradeSystem`
- [ ] [FO] research pod (1): `isResearchedByYard`
- [ ] [A1 A2] mining beam (2): `lengthScale`, `radiusScale`
- [ ] [A1 A2] holo generator (2): `lengthScale`, `radiusScale`
- [ ] [A1 A2] shield disruptor (2): `lengthScale`, `radiusScale`
- [ ] [A1 A2] edit menu (1): `forceToNeutral`
- [ ] [A2] edit menu (3): `menuTitle`, `item%d`, `menuName%d`
- [ ] [A1 A2] artillery (2): `objectName`, `blastRadius`
- [ ] [A1 A2] wormhole (13): `Rotation`, `ClosedScale`, `AnimationTime`, `DisableTimer`, `MaxScaleFactor`, `MaxSpeedFactor`, `OpenScale`, `ScaleEffectRange`, `SpeedEffectRange`, `openSound`, `closeSound`, `AnimationRadius`, `TransportRadius`
- [ ] [A2] wormhole (1): `expireOnClose`
- [ ] [A1 A2] worm hole (13): `Rotation`, `ClosedScale`, `AnimationTime`, `DisableTimer`, `MaxScaleFactor`, `MaxSpeedFactor`, `OpenScale`, `ScaleEffectRange`, `SpeedEffectRange`, `openSound`, `closeSound`, `AnimationRadius`, `TransportRadius`
- [ ] [A1] hover craft (15): `maxVerticalSpeed`, `shp_pitchAngleCoeff`, `shp_pitchVelocCoeff`, `shp_verticalPosCoeff`, `shp_verticalVelocCoeff`, `sp_pitchVelocCoeff`, `spl_lvelocAngleCoeff`, `spl_pitchAngleCoeff`, `spl_pitchVelocCoeff`, `spl_pvelocAngleCoeff`, `spl_verticalVelocCoeff`, `sy_rollAngleCoeff`, `sy_rollVelocCoeff`, `sy_velocAngleCoeff`, `maxForwardSpeed`
- [ ] [A1 A2] hover craft (7): `maxRoll`, `leftRightInertia`, `maxPitch`, `pitchRotationSpeed`, `sp_pitchAngleCoeff`, `sp_velocAngleCoeff`, `upDownInertia`
- [ ] [A1 A2] ion storm torpedo (1): `objectName`
- [ ] [A1 A2] transient rift (1): `objectName`
- [ ] [A1 A2] area effect cannon (4): `hitCondition`, `waveName`, `maxTargets`, `waveEffect`
- [ ] [A2] area effect cannon (1): `waveRange`
- [ ] [A1] micro organism (2): `specialEnergyDelay`, `specialEnergyDrainRate`
- [ ] [A1 A2] micro organism (2): `particleArt`, `drainRate`
- [ ] [A2] micro organism (2): `affectedSystem`, `DrainTime`
- [ ] [A1 A2] shockwave explosion (2): `fadeFactor`, `radiusFactor`
- [ ] [A1 A2] area effect missile (2): `hitCondition`, `maxTargets`
- [ ] [A1 A2] hover (7): `maxRoll`, `leftRightInertia`, `maxPitch`, `pitchRotationSpeed`, `sp_pitchAngleCoeff`, `sp_velocAngleCoeff`, `upDownInertia`
- [ ] [A1 A2] death chant (6): `SpeedModifier`, `DamageModifier`, `FireModifier`, `HitModifier`, `RepairModifier`, `TurnModifier`
- [ ] [A1 A2] plasma overdrive (6): `SpeedModifier`, `DamageModifier`, `FireModifier`, `HitModifier`, `RepairModifier`, `TurnModifier`
- [ ] [A1 A2] nanites (7): `rollRate`, `effectTexture`, `maxTime`, `minTime`, `movementSpeed`, `startReturnTime`, `AIeffectTime`
- [ ] [A2] nanites (1): `wphoton`
- [ ] [A2] technology tree (1): `isShipUpgrade`
- [ ] [A1 A2] explosion (1): `explSound`
- [ ] [A2] explosion (1): `blameTheExploder`
- [ ] [A1 A2] ion storm (4): `damageValue`, `Rotation`, `ScalingCount`, `ScalingStepSize`
- [ ] [A1 A2] utritium ball (4): `damageValue`, `Rotation`, `ScalingCount`, `ScalingStepSize`
- [ ] [A2] build screen (27): `betweenPlayerSpace`, `bottomY`, `buildArrowDownRect`, `buildArrowUpRect`, `buildScrollAreaRect`, `buildTopY`, `builtHeaderRect`, `builtHeaderX`, `classHeaderRect`, `classHeaderX`, `lostHeaderRect`, `lostHeaderX`, `SCROLL_DOWN_BUTTON_DOWN`, `SCROLL_DOWN_BUTTON_NORMAL`, `SCROLL_DOWN_BUTTON_OVER`, `SCROLL_TAB_HEIGHT`, `SCROLL_TAB_NORMAL`, `SCROLL_TAB_WIDTH`, `SCROLL_UP_BUTTON_DOWN`, `SCROLL_UP_BUTTON_NORMAL`, `SCROLL_UP_BUTTON_OVER`, `sortButton`, `sortButtonH`, `sortButtonRect`, `stardateHeaderRect`, `stardateHeaderX`, `ship1ClassData`
- [ ] [A1 A2] research station (1): `podHardpoints`
- [ ] [A2] research station (2): `reserveItem%d`, `secondaryBuildItem%d`
- [ ] [A2] command info (6): `needsTarget`, `displayTrade`, `isBuy`, `sourceNot`, `sourceTypeOr`, `isToggle`
- [ ] [A1] freighter (1): `maxScrap`
- [ ] [A2] freighter (5): `explosionUnitsPerLatinum`, `latinum_spray_emitter`, `speedFractionCarryingLatinum`, `latinum_spray_hardpoint`, `weaponIndexForShips`
- [ ] [FO] utility weapon (2): `isToggledOn`, `linkWeapon`
- [ ] [A1] control panel (1): `buildName`
- [ ] [A1 A2] control panel (1): `forceToNeutral`
- [ ] [A2] battles screen (21): `barHeight`, `battleAlliedHeaderRect`, `battleEnemyHeaderRect`, `battleFleetsHeaderRect`, `battlesArrowDownRect`, `battlesArrowUpRect`, `battlesScrollAreaRect`, `battleTitleHeaderRect`, `battleTopY`, `bottomY`, `individualBarThin`, `SCROLL_DOWN_BUTTON_DOWN`, `SCROLL_DOWN_BUTTON_NORMAL`, `SCROLL_DOWN_BUTTON_OVER`, `SCROLL_TAB_HEIGHT`, `SCROLL_TAB_NORMAL`, `SCROLL_TAB_WIDTH`, `SCROLL_UP_BUTTON_DOWN`, `SCROLL_UP_BUTTON_NORMAL`, `SCROLL_UP_BUTTON_OVER`, `battle1Data`
- [ ] [A2] ships screen (22): `barHeight`, `bottomY`, `filterButton`, `filterButtonH`, `filterButtonSpace`, `individualBarThin`, `SCROLL_DOWN_BUTTON_DOWN`, `SCROLL_DOWN_BUTTON_NORMAL`, `SCROLL_DOWN_BUTTON_OVER`, `SCROLL_TAB_HEIGHT`, `SCROLL_TAB_NORMAL`, `SCROLL_TAB_WIDTH`, `SCROLL_UP_BUTTON_DOWN`, `SCROLL_UP_BUTTON_NORMAL`, `SCROLL_UP_BUTTON_OVER`, `shipFilterButtonRect`, `shipsArrowDownRect`, `shipsArrowUpRect`, `shipsScrollAreaRect`, `topBarY`, `ship1Data`, `ship1Image`
- [ ] [A2] beam (1): `travelTimeFraction`
- [ ] [A1 A2] wormhole generator (3): `drainRate`, `wormholeID`, `lifetime`
- [ ] [A2] ranged object affector (9): `affectorType`, `timeBetween`, `wave_Effect`, `cycleColorBlue`, `cycleColorGreen`, `cycleColorRed`, `damageFactor`, `rangeFactor`, `percentReduction`
- [ ] [A1 A2] computer override (4): `particleArt`, `ownerSystemDuration`, `targetSystemDuration`, `timeBetweenSystems`
- [ ] [A1 A2] fusing weapon (1): `initialDistance`
- [ ] [A2] fusing weapon (6): `finalDistance`, `fusedClass`, `eventCancel`, `fusingMetalCost`, `saucerFormation`, `disbandTime`
- [ ] [A2 FO] fusing weapon (1): `eventFire`
- [ ] [A1 A2] detect cloak (2): `detectClass`, `detectTimer`
- [ ] [A2 FO] repair ship (1): `repairRate`
- [ ] [FO] repair ship (1): `repairsShields`
- [ ] [A1 A2] klingon commando team (5): `fallDuration`, `fallRate`, `rotationRate`, `stayDuration`, `timeDelay`
- [ ] [A1] starbase (1): `maximumUpgrades`
- [ ] [A1 A2] starbase (1): `officerGain`
- [ ] [A1 A2] regeneration weapon (1): `regenModifier`
- [ ] [A1 A2] psionic disruption (1): `particleArt`
- [ ] [A2] admiral's log dialog (16): `adLogBG`, `betweenTabSpace`, `buttonTextColor`, `dialogWinRect`, `doneBmp`, `doneBmpH`, `doneButtonRect`, `generalTextColor`, `interPlayerHeight`, `playerButton`, `playerButtonH`, `saveBmp`, `saveBmpH`, `saveButtonRect`, `playerButtonRect1`, `tabRect1`
- [ ] [A1 A2] black hole (5): `Rotation`, `ScaleSize`, `EventRadius`, `GravityRadius`, `GravityForce`
- [ ] [A1 A2] shield inversion (1): `drainRate`
- [ ] [A1 A2] flux wave (6): `expansionTime`, `initialScale`, `maxScale`, `retractionTime`, `timeAtMaxSize`, `visualFactor`
- [ ] [A2] flux wave (2): `propSpeed`, `spinRate`
- [ ] [A1 A2] black hole creator (6): `expansionTime`, `initialScale`, `maxScale`, `retractionTime`, `timeAtMaxSize`, `visualFactor`
- [ ] [A2] black hole creator (1): `centerEffectOnTarget`
- [ ] [A1 A2] repulsion wave (9): `repulsionForce`, `waveArc`, `amplitude`, `frequency`, `segments`, `spreadSpeed`, `waveLength`, `waveSpeed`, `explosionRadius`
- [ ] [A1 A2] cloaking device (3): `stepSize`, `transparency`, `unsafeCloakDistance`
- [ ] [FO] cloaking device (4): `decloakShieldDelay`, `disableShield`, `initialCloak`, `phaseCloak`
- [ ] [A1 A2] officer display (2): `officerTooltip`, `officerVerboseTooltip`
- [ ] [A1 A2] planet (1): `Rotation`
- [ ] [A2] planet (3): `groundDistance`, `turbulence`, `relativeAtmosphereSpeed`
- [ ] [A1 A2] probe torpedo (2): `detectCloak`, `periodScan`
- [ ] [A1 A2] upgrade (1): `officerGain`
- [ ] [A1 A2] officer upgrade (1): `officerGain`
- [ ] [A1 A2] transwarp drive (2): `initialDistance`, `ScaleSize`
- [ ] [A1 A2] shockwave cannon (3): `repulsionForce`, `disableTime`, `waveArc`
- [ ] [A1 A2] background planet (1): `Rotation`
- [ ] [FO] load bar (17): `loadbarColorEnd`, `loadbarColorStart`, `loadbarHeight`, `loadbarInvert`, `loadbarPositionX`, `loadbarPositionY`, `loadbarTransparencyEnd`, `loadbarVertical`, `loadbarWidth`, `numberOfTips`, `tipPositionX`, `tipPositionY`, `tipSpriteSize`, `tipSpriteX`, `tipSpriteY`, `tipWidth`, `loadbarTransparencyStart`
- [ ] [A1 A2] chain pulsar (3): `chainDamageModifier`, `chainRadius`, `maxHits`
- [ ] [A2 FO] agent saboteur (1): `eventFire`
- [ ] [FO] agent saboteur (2): `destructionProbability`, `ownerGainsXP`
- [ ] [FO] rumble explosion (5): `rootNode`, `createObject`, `rumbleArea`, `rumbleDuration`, `rumbleStrength`
- [ ] [A2] economy screen (8): `betweenPlayerSpace`, `crewHeaderRect`, `dilithiumHeaderRect`, `latinumHeaderRect`, `metalHeaderRect`, `totalEconHeaderRect`, `tradeHeaderRect`, `player1DilData`
- [ ] [A2] timeline screen (8): `timeLineArea`, `timeLineBattlesArea`, `timelineBattlesIcon`, `timeLineBattlesIconArea`, `timeLineTextArea`, `timeLinePlanetArea`, `timelinePlanetIcon`, `timeLinePlanetIconArea`
- [ ] [A2] score screen (7): `betweenPlayerSpace`, `economyHeaderRect`, `gamePtsHeaderRect`, `militaryHeaderRect`, `scienceHeaderRect`, `totalScoreHeaderRect`, `player1ScoreData`
- [ ] [A2] tally screen (7): `tallyBuiltHeaderRect`, `tallyClassHeaderRect`, `tallyDestroyedHeaderRect`, `tallyEffectivenessRect`, `tallyKillsHeaderRet`, `tallyLostHeaderRect`, `tally1Data`
- [ ] [A1 A2] speed boost (1): `SpeedModifier`
- [ ] [A1 A2] ghost (3): `ghostAmbient`, `ghostDiffuse`, `ghostSpecular`
- [ ] [A2] ghost (1): `ghostTeamColorFactor`
- [ ] [A1 A2] quantum effect (1): `wormholeID`
- [ ] [A2] quantum effect (5): `deltaRadius`, `maxRadius`, `minRadius`, `numSections`, `shipID`
- [ ] [A2 FO] repair weapon (1): `eventFire`
- [ ] [FO] repair weapon (1): `standbyTime`
- [ ] [A2 FO] sleeper agent (1): `eventFire`
- [ ] [FO] sleeper agent (1): `ownerGainsXP`
- [ ] [FO] module conf (1): `numberOfModules`
- [ ] [FO] build yard (2): `moduleConf`, `vanishEffect`
- [ ] [A2] military screen (6): `betweenPlayerSpace`, `shipsHeaderRect`, `stationsHeaderRect`, `totalMilHeaderRect`, `planetsHeaderRect`, `player1ShipData`
- [ ] [A2 FO] separating weapon (1): `eventFire`
- [ ] [A1] instant 4X mission (4): `hisPilots`, `hisScrap`, `myPilots`, `myScrap`
- [ ] [A1 A2] repulsion field (1): `repulsionForce`
- [ ] [A2] trading station (4): `cargoTransferRate`, `percentTradeProfit`, `cargoKey%d`, `cargoValue%d`
- [ ] [A2] bio pulse (8): `battleOrdnance`, `chargeDelay`, `damageRate`, `formationBehind`, `formationRadius`, `frigateOrdnance`, `mainDelay`, `specialEnergyLossRate`
- [ ] [FO] detector weapon (4): `MarkColor`, `MarkRadius`, `MarkShrink`, `MarkTime`
- [ ] [FO] cloaking device control (3): `disableShield`, `initialCloak`, `phaseCloak`
- [ ] [A1 A2] standard button (3): `colorBorderDisabled`, `colorBorderMouseOver`, `colorBorderNormal`
- [ ] [A1 A2] psychonic blast (1): `percentage`
- [ ] [A1 A2] reflect weapon (2): `reflectBeam`, `reflectMissile`
- [ ] [A2] reflect weapon (2): `reflectHitOwnerRatio`, `reflectionRatio`
- [ ] [A1 A2] plasma cannon (1): `disableTime`
- [ ] [A2] plasma cannon (1): `disableWeapon`
- [ ] [A2] nebula creator (5): `nebulaID`, `growTime`, `maxSize`, `shrinkTime`, `stableTime`
- [ ] [FO] ram weapon (1): `successChance`
- [ ] [A1 A2] asteroid field (1): `repelIfWithin`
- [ ] [A2] mine array (6): `acceleration`, `destructRadius`, `detectRange`, `returnSpeed`, `slowdownDist`, `accelCont`
- [ ] [A1 A2] standard component (2): `tooltipDelay`, `verboseTooltipDelay`
- [ ] [A1 A2] gravity mine (1): `slowPercent`
- [ ] [A1 A2] omega explosion (2): `topEdgeModifier`, `waveHeight`
- [ ] [A1 A2] cursor interface (2): `tooltipDelay`, `verboseTooltipDelay`
- [ ] [A2] accel cannon (2): `MaxCiviliansKilled`, `MaxCrewMembersKilled`
- [ ] [A2] individual screen (6): `horizontalOverlay`, `verticalOverlay`, `horzOverlayRect_0`, `horzOverlayRect_1`, `vertOverlayRect_0`, `vertOverlayRect_1`
- [ ] [A1 A2] sensor array (1): `isTranswarpGate`
- [ ] [FO] fighter (2): `supplyClasses`, `operationRange`
- [ ] [A1 A2] fed repair team (1): `shieldsReturned`
- [ ] [A2] cargo ship (1): `m_maxResource`
- [ ] [A2] planet shield (2): `hitPercent`, `hitPoints`
- [ ] [A2] popup palette (2): `commandHotkeys`, `controlDefaultCursor`
- [ ] [A2] colonizing beam (2): `transferAmount`, `transferTime`
- [ ] [A2] nebula flux (3): `damage_range`, `wave_name`, `wFluxWav`
- [ ] [A2] nebula generation (1): `nebulaID`
- [ ] [A1] background object (1): `streamChunkLength`
- [ ] [A1 A2] background object (1): `minimum3DSounds`
- [ ] [A2] colony (1): `cleansingWeaponIndex`
- [ ] [A2] construction rig (2): `animateBees`, `fbee`
- [ ] [A2] drones (2): `numDrones`, `fbee`
- [ ] [A2] resource transfer data (1): `conversionRate`
- [ ] [FO] fed shield emitter (1): `shieldAmount`
- [ ] [A1] ship display (1): `infoSystem`
- [ ] [A1] speed rail (1): `speedSingleButtonGap`
- [ ] [A1] selection display (1): `selectSystem`
- [ ] [A1] comm display (3): `commBarNumberOfPlayers`, `commIndicatorAnchor`, `commIndicatorWidth`
- [ ] [A1 A2] target override (4): `segmentLength`, `segmentRadius`, `segmentTexture`, `segmentVariance`
- [ ] [A1 A2] audio default (1): `minimum3DSounds`
- [ ] [A2] clairvoyant link (1): `effectDuration`
- [ ] [A2] tech assim weapon (1): `assignClass%d`
- [ ] [FO] command info bar (1): `CommandInfoBarButtonDistance`
- [ ] [FO] fleet ops functions hook (1): `groupTextColor`
- [ ] [FO] resource weapon (5): `addBiomatterThreshold`, `addCrewThreshold`, `addDilithiumThreshold`, `addLatinumThreshold`, `addMetalThreshold`
- [ ] [FO] uni beam (1): `missLifetime`

### ODF class labels with no code (82)

Tag = which games ship ODFs of the class. Most-used first.

- [ ] [A1 A2] (25): `detectcloak`, `fireballexplode`, `wormholegen`, `reflectweapon`, `blackholecreator`, `myotron`, `shieldh`, `shieldinv`, `polerontorpedo`, `shockwavecannon`, `override`, `warpcoreoverload`, `omegaexplode`, `manheimeffect`, `ionstorm_tp`, `containbreach`, `fedrepairteam`, `remodulation`, `phasecloakdevice`, `selfdestruct`, `holoemitter`, `grenade`, `klingoncommandoteam`, `repulsionwave`, `posijammer`
- [ ] [A1 A2 FO] (17): `shockwavexplode`, `regeneration`, `micro`, `plasmacannon`, `minelayer`, `deathchant`, `antimine`, `psychonicblast`, `transwarpconduit`, `transwarpdrive`, `chainpulsar`, `esconverter`, `rfield`, `gravmine`, `nanitesordnance`, `romulanspy`, `temporal`
- [ ] [A2] (22): `insanity`, `nebcreator`, `resourceextractionbeam`, `separatingweapon`, `transriftweapon`, `speed_boost`, `shield_disruptor`, `targneb`, `biopulse`, `clairvoyance`, `shieldremovingtorpedo`, `fluxwave`, `cvoyantlink`, `tech_assim_beam`, `quantumeffect`, `techassim`, `trans_rift`, `drones_weapon`, `nebgeneration`, `nebulaflux`, `hologen_weapon`, `hologenerator`
- [ ] [A2 FO] (5): `borgenergyemitter`, `rangedobjectaffector`, `fedshieldemitter`, `fusingweapon`, `psidisrupt`
- [ ] [FO] (10): `unitorpedo`, `rumbleexplode`, `probetorpedo`, `ramweapon`, `agentsaboteur`, `cloakdetector`, `detectorweapon`, `cloakingdevicecontrol`, `repairweapon`, `sleeperagent`
- [ ] [A1 (executable only, no stock ODF)] (3): `plasmaoverdrive`, `shieldgendisruptor`, `targetov`

### Header config names (`*.h`) the originals read

- [ ] [A1] `ART_CFG.h` (16 of 18): `SMOOTH_CAMERA_TIME`, `MOVIE_CAMERA`, `CAMERA_MIN_ZOOM`, `CAMERA_MAX_ZOOM`, `ST3D_CHUNK_MANAGER_MAX_CHUNKS`, `ST3D_CHUNK_DENSITY`, `ST3D_CHUNK_MAX_LIFETIME`, `ST3D_CHUNK_MAX_ANGULAR_VELOCITY`, `ST3D_CHUNK_MAX_VELOCITY`, `STARFIELD_GEOMETRY_NAME`, `STARFIELD_GEOMETRY_SCALE`, `CINEMATIC_NEAR_CLIPPING_PLANE`, `CINEMATIC_FAR_CLIPPING_PLANE`, `STAR_FAR_CLIP_PLANE`, `GRID_FAR_CLIP_PLANE`, `STARFIELD_BACKGROUND_STAR_NUM_REPRESENTATIONS`
- [ ] [A1] `game_cfg.h` (4 of 29): `MISSIONS_DIRECTORY`, `AI_DIRECTORY`, `WATCH_LOCATION`, `WATCH_VERSION`
- [ ] [A1] `RTS_CFG.h` (114 of 141): `OVERVIEW_MAX_ZOOM`, `OVERVIEW_INIT_ZOOM`, `OVERVIEW_MIN_ZOOM`, `OVERVIEW_MAX_HEIGHT`, `OVERVIEW_MIN_HEIGHT`, `SCROLL_COEFFICIENT`, `FASTSCROLL_COEFFICIENT`, `ZOOM_SCROLL_COEFFICIENT`, `ROTATION_SCROLL_COEFFICIENT`, `CINEMATIC_ROTATION_COEFFECIENT`, `KEYBOARD_SCROLL_RATE`, `SCROLL_ACCELERATION`, `TIME_SCALE`, `MAX_SCROLL_SPEED`, `INITIAL_SCROLL_SPEED`, `FAST_SCROLL_ACCELERATION`, `FAST_TIME_SCALE`, `FAST_MAX_SCROLL_SPEED`, `FAST_INITIAL_SCROLL_SPEED`, `USE_LIGHTING_FOR_SELECTION`, `SELECTION_RECTANGLE_COMMIT_AREA`, `RESOURCE_PANEL_PRIORITY`, `CINEMATIC_PANEL_PRIORITY`, `CONTROL_PANEL_PRIORITY`, `CURSOR_SIMULATE_PRIORITY`, `CURSOR_RENDER_PRIORITY`, `RADAR_PANEL_PRIORITY`, `OVERVIEW_PANEL_PRIORITY`, `VIDEO_PANEL_PRIORITY`, `SELECTION_PRIORITY`, `INFO_PANEL_PRIORITY`, `SPEED_PANEL_PRIORITY`, `OBJECTIVES_PANEL_PRIORITY`, `SNAP_TO_DISTANCE`, `UNHIDE_DISTANCE`, `SCROLL_INTERVAL`, `DEFAULT_CHAT_TEXT_HEIGHT`, `GRID_FX_GROWTH_RATE`, `GRID_FX_MAX_SIZE`, `GRID_FX_MIN_SIZE`, `GRID_FX_GROWTH_INTERVAL`, `GRID_FX_ROTATION_SPEED`, `GRID_FX_MIN_SEGMENTS`, `GRID_FX_MAX_SEGMENTS`, `SHOW_QUADTREE`, `SHOW_FOOTPRINTS_IN_EDIT_MODE`, `DEBUG_VECTORS_3D`, `DEBUG_SHOW_AI_PATHS`, `DEBUG_SHOW_AI_PATHS_RUNNING`, `DEBUG_SHOW_CURVES`, `DEBUG_SHOW_OBJ_POSTS`, `DEBUG_SHOW_OBJ_POSTS_RUNNING`, `DEBUG_SHOW_LOCAL_BOUNDING_BOX`, `DEBUG_SHOW_OBJECT_BOUNDING_BOX`, `DEBUG_SHOW_LOCAL_BOUNDING_SPHERE`, `DEBUG_SHOW_OBJECT_BOUNDING_SPHERE`, `DEBUG_SHOW_BOUNDING_SQUARE`, `DEBUG_DRAW`, `SELECTION_CIRCLE_VERTICAL_OFFSET`, `SELECTION_CIRCLE_SIZE_MULTIPLIER`, `EASY_SPECIAL_WEAPON_FIRE_MODIFIER`, `DEFAULT_SPECIAL_WEAPON_FIRE_MODIFIER`, `HARD_SPECIAL_WEAPON_FIRE_MODIFIER`, `HARD_AI_COST_AND_SPEED_HANDICAP`, `MEDIUM_AI_COST_AND_SPEED_HANDICAP`, `HARD_AI_CLOAK_PERCENT`, `MEDIUM_AI_CLOAK_PERCENT`, `EASY_AI_CLOAK_PERCENT`, `cfg_MIN_MOVE_DISTANCE_FOR_CLOAK`, `EASY_DAMAGE`, `HARD_DAMAGE`, `ST3D_NUM_ZSORT_BUCKETS`, `ST3D_SHOW_LIGHT_SOURCE_INFO`, `ST3D_PRELOAD_TEXTURES`, `USE_SYSTEM_HITPOINTS`, `SYSTEM_HITPOINTS_BONUS`, `DEMAND_LOAD_ODFS`, `FRAMERATE_DISPLAY`, `MORASS_SIZE`, `MORASS_SHIP_LIMIT`, `MORASS_FRIENDS_LIMIT`, `CIRCLE_IN_PLACE_DURING_COMBAT`, `CIRCLE_RADIUS_DIVISOR`, `MAX_PREVENTION_ROTATION`, `CANNOT_PATH_INTO_COST`, `PERFORM_RUBBER_BANDING`, `FORMATIONS_RELATIVE_TO_SHIP`, `COMBAT_SLOWDOWN`, `FORMATION_LEADER_SLOWDOWN`, `ASTEROID_BELT_PATHING_COST`, `ASTEROID_BELT_PATHING_DISTANCE`, `BLACKHOLE_PATHING_COST`, `BLACKHOLE_PATHING_SAFETY_COEFFICIENT`, `DEFAULT_TO_FORMATION_MOVE`, `AI_GRID_SIZE`, `EASY_AI`, `MEDIUM_AI`, `ANCHOR_RADIUS`, `ANCHOR_PURSUIT_FORCE`, `CENTER_OF_COMBAT_PERSISTENCE_CUTOFF`, `DOGFIGHT_RANGE`, `FORMATION_SPACING`, `OLD_AVOIDANCE_FORCE_WEIGHT`, `COLLISION_LOOK_AHEAD_TIME`, `COLLISION_SAFETY_MARGIN`, `Y_REPULSION`, `PATH_POINT_RADIUS_SQUARED`, `SCRIPTED_ATTACK_SPEED_UP`, `SCRIPTED_ATTACK_TARGET_SLOW_DOWN`, `MAP_EDGE_EPSILON`, `MAP_EDGE_BUFFER`, `DECONSTRUCT_PERCENT_TIME`, `BEAM_MISS_LIFETIME`, `NORMAL_WEAPON_TEAM_COLOR`
- [ ] [A1] names read that no stock header declares (26 of 27): `AVOIDANCE_USING_RANGE_SEARCH`, `CLIFF_REPULSION_COEFFICIENT`, `COLLISION_CHEAP_2D_COLLIDE`, `COLLISION_CONFIG_FILE_REVISION`, `COLLISION_DIRECT_REPULSION`, `COLLISION_FULL_POTENTIAL_FIELD`, `COLLISION_LATERAL_REPULSION`, `COLLISION_MARTIN_REPULSION`, `COLLISION_REAL_COLLISIONS`, `death_map_resolution`, `DEFAULT_REPULSION_CUTOFF`, `DISTANCE_OF_INFLUENCE`, `empire_map_resolution`, `exploration_map_resolution`, `EXTRA_GOAL_ATTRACTION`, `EXTRA_STUCK_TIME`, `goal_map_resolution`, `hunt_map_resolution`, `LATERAL_AVOIDANCE_MULTIPLIER`, `LATERAL_DANGER_WIDTH`, `output_file_name`, `relaxation_cycles`, `REPULSIVE_COEFFICIENT`, `start_up_script`, `SWITCH_TO_NEXT_POINT_DISTANCE`, `threat_map_resolution`
- [ ] [A2] `ADLOG_CFG.h` (28 of 28): `cfgShipsKilled`, `cfgShipsLost`, `cfgShipsBuilt`, `cfgStationsKilled`, `cfgStationsLost`, `cfgStationsBuilt`, `cfgPlanetsColonized`, `cfgPlanetsLost`, `cfgDilithiumGathered`, `cfgDilithiumSpent`, `cfgDilithiumBought`, `cfgDilithiumSold`, `cfgLatinumGathered`, `cfgLatinumSpent`, `cfgMetalGathered`, `cfgMetalSpent`, `cfgMetalBought`, `cfgMetalSold`, `cfgBiomatterGathered`, `cfgBiomatterSpent`, `cfgCrewEnlisted`, `cfgCrewAssigned`, `cfgCrewLost`, `cfgTradeStations`, `cfgCargoShips`, `cfgShipsSent`, `cfgShipsReceived`, `cfgTradeProfit`
- [ ] [A2] `ART_CFG.h` (21 of 25): `SCROLL_INTERVAL`, `DEFAULT_CHAT_TEXT_HEIGHT`, `ST3D_CHUNK_MANAGER_MAX_CHUNKS`, `ST3D_CHUNK_SIZE_FACTOR`, `ST3D_CHUNK_LIFETIME_AVG`, `ST3D_CHUNK_LIFETIME_DEVIATION`, `ST3D_CHUNK_MAX_ANGULAR_VELOCITY`, `ST3D_CHUNK_MAX_VELOCITY`, `ST3D_NUM_ZSORT_BUCKETS`, `ST3D_SHOW_LIGHT_SOURCE_INFO`, `ST3D_PRELOAD_TEXTURES`, `cfgST3D_ALWAYS_FLUSH_D3D_BUFFERS`, `STARFIELD_GEOMETRY_NAME`, `STARFIELD_ODF_NAME`, `CINEMATIC_NEAR_CLIPPING_PLANE`, `CINEMATIC_FAR_CLIPPING_PLANE`, `STAR_FAR_CLIP_PLANE`, `STARFIELD_GEOMETRY_SCALE`, `cfgSTARFIELD_NUM_BSTAR`, `cfgSTARFIELD_NUM_MSTAR`, `cfgSTARFIELD_NUM_CSTAR`
- [ ] [A2] `game_cfg.h` (2 of 2): `MISSIONS_DIRECTORY`, `AI_DIRECTORY`
- [ ] [A2] `Network_CFG.h` (1 of 1): `MOTD_ADDRESS`
- [ ] [A2] `RTS_CFG.h` (131 of 177): `cfg_DEFAULT_TACTICAL_ZOOM`, `OVERVIEW_MAX_HEIGHT`, `OVERVIEW_MIN_HEIGHT`, `SCROLL_COEFFICIENT`, `FASTSCROLL_COEFFICIENT`, `ZOOM_SCROLL_COEFFICIENT`, `ROTATION_SCROLL_COEFFICIENT`, `CINEMATIC_ROTATION_COEFFECIENT`, `KEYBOARD_SCROLL_RATE`, `cfgMOUSE_HOLD_LEVEL`, `SCROLL_ACCELERATION`, `MAX_SCROLL_SPEED`, `INITIAL_SCROLL_SPEED`, `cfgTACTICAL_WHERE_MODE`, `SELECTION_RECTANGLE_COMMIT_AREA`, `cfgPATH_POINT_SELECTION_DISTANCE`, `SNAP_TO_DISTANCE`, `UNHIDE_DISTANCE`, `GRID_FX_GROWTH_RATE`, `GRID_FX_MAX_SIZE`, `GRID_FX_MIN_SIZE`, `GRID_FX_GROWTH_INTERVAL`, `GRID_FX_ROTATION_SPEED`, `SHOW_QUADTREE`, `SHOW_FOOTPRINTS_IN_EDIT_MODE`, `DEBUG_VECTORS_3D`, `DEBUG_SHOW_AI_PATHS`, `DEBUG_SHOW_AI_PATHS_RUNNING`, `DEBUG_SHOW_CURVES`, `DEBUG_SHOW_OBJ_POSTS`, `DEBUG_SHOW_LOCAL_BOUNDING_BOX`, `DEBUG_SHOW_OBJECT_BOUNDING_BOX`, `DEBUG_SHOW_OBJECT_BOUNDING_SPHERE`, `DEBUG_SHOW_BOUNDING_SQUARE`, `DEBUG_DRAW`, `cfgINITIAL_STARDATE`, `cfgSTARDATE_PER_SECOND`, `cfgDEFEND_TIME`, `cfgLATINUM_COLLECT`, `EASY_DAMAGE`, `HARD_DAMAGE`, `EASY_MINING_MODIFIER`, `HARD_MINING_MODIFIER`, `EASY_SPECIAL_WEAPON_FIRE_MODIFIER`, `DEFAULT_SPECIAL_WEAPON_FIRE_MODIFIER`, `HARD_SPECIAL_WEAPON_FIRE_MODIFIER`, `MEDIUM_AI_COST_HANDICAP`, `MEDIUM_AI_SPEED_HANDICAP`, `EASY_AI_COST_HANDICAP`, `EASY_AI_SPEED_HANDICAP`, `HARD_AI_CLOAK_PERCENT`, `MEDIUM_AI_CLOAK_PERCENT`, `EASY_AI_CLOAK_PERCENT`, `cfgRESOURCE_LATINUM_SPACING`, `cfg8472_RESOURCE_DISTANCE`, `cfg8472_RESOURCE_INSIDE_DISTANCE`, `cfgFREIGHTERS_PER_MOON`, `cfgRESOURCE_THREAT_PENALTY_COEFFICIENT`, `cfgPLANET_BASE_BUILD_BUFFER`, `cfg_MIN_MOVE_DISTANCE_FOR_CLOAK`, `SYSTEM_HITPOINTS_BONUS`, `cfgDEMAND_LOAD_ODFS`, `cfgRELOAD_OBJECT_CLASSES`, `cfgFLIP_TO_WINDOWED_ON_ASSERT`, `cfgTACTICAL_GRID_RENDER`, `cfgSTRATEGIC_GRID_RENDER`, `FRAMERATE_DISPLAY`, `cfgMAX_STRATEGIC_VIEW_OBJECT_SCALE`, `cfg_AI_SHOW_GOALS_COUNT`, `CIRCLE_IN_PLACE_DURING_COMBAT`, `CIRCLE_RADIUS_DIVISOR`, `MAX_PREVENTION_ROTATION`, `cfg_WARP_INHIBITION_BUFFER`, `cfg_WARP_INHIBITION_COST`, `cfg_ALMOST_INTRAVERSIBLE`, `PERFORM_RUBBER_BANDING`, `COMBAT_SLOWDOWN`, `FORMATION_LEADER_SLOWDOWN`, `ASTEROID_BELT_PATHING_COST`, `BLACKHOLE_PATHING_COST`, `BLACKHOLE_PATHING_SAFETY_COEFFICIENT`, `cfgDEFAULT_TO_FORMATION_MOVE`, `cfgBLACKHOLE_DEWARP_RADIUS_MULTIPLIER`, `PLANET_MINING_RADIUS_OFFSET`, `AI_GRID_SIZE`, `EASY_AI`, `MEDIUM_AI`, `SKINNY_SHIP_COMBAT_FLOOR`, `SKINNY_SHIP_COMBAT_CEILING`, `DEEP_SPACE_SHIP_COMBAT_FLOOR`, `DEEP_SPACE_SHIP_COMBAT_CEILING`, `ANCHOR_RADIUS`, `ANCHOR_PURSUIT_FORCE`, `CENTER_OF_COMBAT_PERSISTENCE_CUTOFF`, `DOGFIGHT_RANGE`, `cfgDOGFIGHT_RADIUS_MODIFIER`, `cfgCLOVERLEAF_RADIUS_MODIFIER`, `FORMATION_SPACING`, `OLD_AVOIDANCE_FORCE_WEIGHT`, `COLLISION_LOOK_AHEAD_TIME`, `COLLISION_SAFETY_MARGIN`, `Y_REPULSION`, `PATH_POINT_RADIUS_SQUARED`, `MAP_EDGE_EPSILON`, `MAP_EDGE_BUFFER`, `DECONSTRUCT_PERCENT_TIME`, `BEAM_MISS_LIFETIME`, `NORMAL_WEAPON_TEAM_COLOR`, `MAX_LATINUM`, `MAX_METAL`, `MAX_BIOMATTER`, `STARTING_DILITHIUM`, `STARTING_LATINUM`, `STARTING_METAL`, `MP_RESOURCE_MODES`, `DRAW_PIZZA_BOX`, `cfgSCREEN_WIDTH`, `cfgSCREEN_HEIGHT`, `cfgRESIZEABLE_WINDOW`, `cfgTACTICAL_SNAP_DISTANCE`, `cfgFADE_OUT_ENABLED`, `cfgFADE_OUT_MIN_FOV`, `cfgFADE_OUT_MAX_FOV`, `cfgFADE_OUT_MAX`, `cfgWINNERS_BONUS`, `cfgTIMELINE_FREQ`, `cfgTSTATION_TIME_BETWEEN_FERENGI`, `cfgKILL_FERENGI_TIME_ADD`, `cfgBIOMATTER_RECYCLE_FRACTION`, `cfgSOUND_VOICE_SAMPLE`, `cfgSOUND_EFFECTS_SAMPLE`
- [ ] [A2] names read that no stock header declares (25 of 25): `danger_diminishment`, `death_map_resolution`, `distance_from_goal_match_modifier`, `empire_map_resolution`, `exploration_map_resolution`, `fleetAIRatio`, `formationName`, `goal_map_resolution`, `m_formationID`, `m_shipRadius`, `max_matching_force_ratio`, `min_matching_force_ratio`, `relaxation_coefficient`, `relaxation_cycles`, `resource_biomatter_tendency`, `resource_dilithium_tendency`, `resource_distance_multiplier`, `resource_enemy_penalty`, `resource_health_multiplier`, `resource_lack_multiplier`, `resource_latinum_tendency`, `resource_metal_tendency`, `scoutRandomlyRatio`, `threat_map_resolution`, `threatModifier`
- [ ] [FO] names the Fleet Operations hook reads (23 of 29): `cfgBUTTON_COLOR`, `cfgCINEMATIC_WINDOW_ENABLED`, `cfgDEFAULT_MAP`, `cfgDEFAULT_MAP_TITLE`, `cfgSOUND_INGAME_CHEAT`, `cfgSOUND_LOADING_MUSIC`, `cfgSOUND_SHELL_MUSIC`, `cfgTEXT_COLOR`, `cfgTITLE_COLOR`, `CRAFT_DEFAULT_ALERT_STATUS`, `DISABLE_HEIGHT_CHANGE`, `GAME_OPTIONS_INFINITE_RESOURCES`, `GAME_OPTIONS_MAP_DEPTH`, `GAME_OPTIONS_MOTHERSHIP`, `GAME_OPTIONS_OFFICERS`, `GAME_OPTIONS_PHYSICS`, `GAME_OPTIONS_RANDOM_MAP`, `GAME_OPTIONS_WARP`, `PRODUCER_BUILD_INFO_WIREFRAME_SIZE`, `SHIELD_EFFECT_DAMAGE_COLORED`, `SHIELD_EFFECT_RESCALE`, `SYSTEM_REPAIR_SPARK_SPRITE`, `TECHTREE_DEFAULT_FILE`

### CFG keys

- [ ] [A1] GUI, cinematic view (10 of 13): `cinematicDefaultCursor`, `cinematicDefaultBackground`, `cinematicBackgroundPanelArea`, `cinematicMinimizedPanelArea`, `cinematicMenuButtonArea`, `cinematicMenuButton`, `cinematicMenuBorder`, `cinematicCommButtonArea`, `cinematicCommButton`, `cinematicCommBorder`
- [ ] [A1] GUI, control panel (2 of 4): `controlDefaultCursor`, `controlBackgroundPanelArea`
- [ ] [A1] GUI, map radar (2 of 7): `minimapDefaultCursor`, `minimapBackgroundPanelArea`
- [ ] [A1] GUI, ship display (13 of 30): `infoDefaultCursor`, `infoBackgroundPanelArea`, `infoSingleBackgroundArea`, `infoSingleCrewTextArea`, `infoSingleOfficerTextArea`, `infoSingleRaceIconBackground`, `infoSingleShieldBarDisableColor`, `infoSingleEnergyBarDisableColor`, `infoSingleConstructionBarDisableColor`, `infoSingleDilithiumBarArea`, `infoSingleDilithiumBar`, `infoSingleDilithiumBarDisableColor`, `infoSingleDilithiumBarColor`
- [ ] [A1] GUI, energy bar (10 of 10): `infoMultiShieldBarArea`, `infoMultiShieldBar`, `infoMultiShieldBarDisableColor`, `infoMultiEnergyBarArea`, `infoMultiEnergyBar`, `infoMultiEnergyBarDisableColor`, `infoMultiEnergyBarColor`, `infoMultiCrewDotArea`, `infoMultiCrewDot`, `infoMultiWireframeIconArea`
- [ ] [A1] GUI, other panels (2 of 2): `bridgePanelRect`, `bridgeBlackRect`
- [ ] [A1] GUI, speed rail (6 of 6): `speedDefaultCursor`, `speedPanelArea`, `speedBackgroundPanelArea`, `speedSingleButtonArea`, `speedSeparatorArea`, `speedSeparator`
- [ ] [A1] GUI, objectives display (3 of 4): `objectivesDefaultCursor`, `objectivesBlackArea`, `objectivesBackgroundPanelArea`
- [ ] [A1] GUI, comm display (7 of 7): `commDefaultCursor`, `commPanelArea`, `commBlackArea_0`, `commBlackArea_1`, `commBlackArea_2`, `commBackgroundPanelArea`, `commIndicatorSprite`
- [ ] [A1] GUI, drop player box (3 of 4): `dropPlayerDefaultCursor`, `dropPlayerBlackArea`, `dropPlayerBackgroundPanelArea`
- [ ] [A1] GUI, pause box (4 of 4): `pauseGameDefaultCursor`, `pauseGamePanelArea`, `pauseGameBlackArea`, `pauseGameBackgroundPanelArea`
- [ ] [A1] engine config names, including `collision.cfg` and `cca.cfg` (160 of 216): `output_file_name`, `start_up_script`, `threat_map_resolution`, `relaxation_cycles`, `empire_map_resolution`, `death_map_resolution`, `exploration_map_resolution`, `hunt_map_resolution`, `goal_map_resolution`, `COLLISION_CONFIG_FILE_REVISION`, `AVOIDANCE_USING_RANGE_SEARCH`, `COLLISION_LATERAL_REPULSION`, `COLLISION_FULL_POTENTIAL_FIELD`, `COLLISION_REAL_COLLISIONS`, `COLLISION_DIRECT_REPULSION`, `COLLISION_MARTIN_REPULSION`, `LATERAL_AVOIDANCE_MULTIPLIER`, `LATERAL_DANGER_WIDTH`, `COLLISION_CHEAP_2D_COLLIDE`, `EXTRA_GOAL_ATTRACTION`, `REPULSIVE_COEFFICIENT`, `DEFAULT_REPULSION_CUTOFF`, `DISTANCE_OF_INFLUENCE`, `CLIFF_REPULSION_COEFFICIENT`, `SWITCH_TO_NEXT_POINT_DISTANCE`, `EXTRA_STUCK_TIME`, `MISSIONS_DIRECTORY`, `AI_DIRECTORY`, `WATCH_LOCATION`, `WATCH_VERSION`, `EASY_SPECIAL_WEAPON_FIRE_MODIFIER`, `DEFAULT_SPECIAL_WEAPON_FIRE_MODIFIER`, `HARD_SPECIAL_WEAPON_FIRE_MODIFIER`, `EASY_DAMAGE`, `HARD_DAMAGE`, `HARD_AI_COST_AND_SPEED_HANDICAP`, `MEDIUM_AI_COST_AND_SPEED_HANDICAP`, `HARD_AI_CLOAK_PERCENT`, `MEDIUM_AI_CLOAK_PERCENT`, `EASY_AI_CLOAK_PERCENT`, `cfg_MIN_MOVE_DISTANCE_FOR_CLOAK`, `SCROLL_INTERVAL`, `DEFAULT_CHAT_TEXT_HEIGHT`, `SELECTION_CIRCLE_VERTICAL_OFFSET`, `SELECTION_CIRCLE_SIZE_MULTIPLIER`, `FRAMERATE_DISPLAY`, `SHOW_QUADTREE`, `CANNOT_PATH_INTO_COST`, `PERFORM_RUBBER_BANDING`, `FORMATIONS_RELATIVE_TO_SHIP`, `COMBAT_SLOWDOWN`, `FORMATION_LEADER_SLOWDOWN`, `ASTEROID_BELT_PATHING_COST`, `ASTEROID_BELT_PATHING_DISTANCE`, `BLACKHOLE_PATHING_COST`, `BLACKHOLE_PATHING_SAFETY_COEFFICIENT`, `DEFAULT_TO_FORMATION_MOVE`, `SHOW_FOOTPRINTS_IN_EDIT_MODE`, `MORASS_SIZE`, `MORASS_SHIP_LIMIT`, `MORASS_FRIENDS_LIMIT`, `CIRCLE_IN_PLACE_DURING_COMBAT`, `CIRCLE_RADIUS_DIVISOR`, `GRID_FX_GROWTH_RATE`, `GRID_FX_MAX_SIZE`, `GRID_FX_MIN_SIZE`, `GRID_FX_GROWTH_INTERVAL`, `GRID_FX_ROTATION_SPEED`, `GRID_FX_MIN_SEGMENTS`, `GRID_FX_MAX_SEGMENTS`, `AI_GRID_SIZE`, `EASY_AI`, `MEDIUM_AI`, `OVERVIEW_MIN_ZOOM`, `OVERVIEW_MAX_ZOOM`, `OVERVIEW_INIT_ZOOM`, `OVERVIEW_MAX_HEIGHT`, `OVERVIEW_MIN_HEIGHT`, `RESOURCE_PANEL_PRIORITY`, `CINEMATIC_PANEL_PRIORITY`, `CONTROL_PANEL_PRIORITY`, `CURSOR_SIMULATE_PRIORITY`, `CURSOR_RENDER_PRIORITY`, `RADAR_PANEL_PRIORITY`, `OVERVIEW_PANEL_PRIORITY`, `VIDEO_PANEL_PRIORITY`, `SELECTION_PRIORITY`, `INFO_PANEL_PRIORITY`, `SPEED_PANEL_PRIORITY`, `OBJECTIVES_PANEL_PRIORITY`, `SCROLL_COEFFICIENT`, `FASTSCROLL_COEFFICIENT`, `ZOOM_SCROLL_COEFFICIENT`, `ROTATION_SCROLL_COEFFICIENT`, `CINEMATIC_ROTATION_COEFFECIENT`, `KEYBOARD_SCROLL_RATE`, `SNAP_TO_DISTANCE`, `UNHIDE_DISTANCE`, `USE_LIGHTING_FOR_SELECTION`, `SELECTION_RECTANGLE_COMMIT_AREA`, `SCROLL_ACCELERATION`, `TIME_SCALE`, `MAX_SCROLL_SPEED`, `INITIAL_SCROLL_SPEED`, `FAST_SCROLL_ACCELERATION`, `FAST_TIME_SCALE`, `FAST_MAX_SCROLL_SPEED`, `FAST_INITIAL_SCROLL_SPEED`, `SMOOTH_CAMERA_TIME`, `MOVIE_CAMERA`, `DEBUG_VECTORS_3D`, `DEBUG_SHOW_AI_PATHS`, `DEBUG_SHOW_AI_PATHS_RUNNING`, `DEBUG_SHOW_CURVES`, `DEBUG_SHOW_OBJ_POSTS`, `DEBUG_SHOW_OBJ_POSTS_RUNNING`, `DEBUG_SHOW_LOCAL_BOUNDING_BOX`, `DEBUG_SHOW_OBJECT_BOUNDING_BOX`, `DEBUG_SHOW_LOCAL_BOUNDING_SPHERE`, `DEBUG_SHOW_OBJECT_BOUNDING_SPHERE`, `DEBUG_SHOW_BOUNDING_SQUARE`, `DEBUG_DRAW`, `CAMERA_MIN_ZOOM`, `CAMERA_MAX_ZOOM`, `STARFIELD_GEOMETRY_NAME`, `STARFIELD_GEOMETRY_SCALE`, `CINEMATIC_NEAR_CLIPPING_PLANE`, `CINEMATIC_FAR_CLIPPING_PLANE`, `STAR_FAR_CLIP_PLANE`, `GRID_FAR_CLIP_PLANE`, `STARFIELD_BACKGROUND_STAR_NUM_REPRESENTATIONS`, `DECONSTRUCT_PERCENT_TIME`, `BEAM_MISS_LIFETIME`, `NORMAL_WEAPON_TEAM_COLOR`, `ST3D_NUM_ZSORT_BUCKETS`, `ST3D_SHOW_LIGHT_SOURCE_INFO`, `ST3D_PRELOAD_TEXTURES`, `ST3D_CHUNK_MANAGER_MAX_CHUNKS`, `ST3D_CHUNK_DENSITY`, `ST3D_CHUNK_MAX_LIFETIME`, `ST3D_CHUNK_MAX_ANGULAR_VELOCITY`, `ST3D_CHUNK_MAX_VELOCITY`, `USE_SYSTEM_HITPOINTS`, `SYSTEM_HITPOINTS_BONUS`, `ANCHOR_RADIUS`, `ANCHOR_PURSUIT_FORCE`, `CENTER_OF_COMBAT_PERSISTENCE_CUTOFF`, `FORMATION_SPACING`, `PATH_POINT_RADIUS_SQUARED`, `SCRIPTED_ATTACK_SPEED_UP`, `SCRIPTED_ATTACK_TARGET_SLOW_DOWN`, `DOGFIGHT_RANGE`, `OLD_AVOIDANCE_FORCE_WEIGHT`, `MAX_PREVENTION_ROTATION`, `COLLISION_LOOK_AHEAD_TIME`, `COLLISION_SAFETY_MARGIN`, `Y_REPULSION`, `MAP_EDGE_EPSILON`, `MAP_EDGE_BUFFER`, `DEMAND_LOAD_ODFS`
- [ ] [A2] button panel (32 of 35): `bb_standard`, `buttonBackgroundPanelArea`, `comm_button`, `commBorder`, `commButton`, `commButtonArea`, `help_button`, `helpBorder`, `helpButton`, `helpButtonArea`, `idle_button`, `idleBorder`, `idleButton`, `idleButtonArea`, `menuBorder`, `menuButtonArea`, `more_button`, `moreBorder`, `moreButton`, `moreButtonArea`, `objectives_button`, `objectivesBorder`, `objectivesButton`, `objectivesButtonArea`, `palette_button`, `paletteBorder`, `paletteButton`, `paletteButtonArea`, `toggle_button`, `toggleBorder`, `toggleButton`, `toggleButtonArea`
- [ ] [A2] cinematic view (3 of 6): `cinematic_background`, `cinematicBackgroundPanelArea`, `cinematicDefaultBackground`
- [ ] [A2] comm bar (4 of 4): `comm_standard_marker`, `comm_standard_toggle`, `comm_standard_border`, `comm_standard_button`
- [ ] [A2] comm display (11 of 11): `comm_standard_toggle`, `comm_standard_border`, `comm_standard_button`, `comm_indicator`, `commBackgroundPanelArea`, `commIndicatorSprite`, `commPanelArea`, `commBackgroundPanel`, `commBarNumberOfPlayers`, `commIndicatorAnchor`, `commIndicatorWidth`
- [ ] [A2] cursor interface (14 of 15): `frameLocation`, `tooltipBackgroundColor`, `tooltipFrameBottom.0`, `tooltipFrameBottom.1`, `tooltipFrameBottomRect_0`, `tooltipFrameBottomRect_1`, `tooltipFrameLeft`, `tooltipFrameLeftRect`, `tooltipFrameRight`, `tooltipFrameRightRect`, `tooltipFrameTop.0`, `tooltipFrameTop.1`, `tooltipFrameTopRect_0`, `tooltipFrameTopRect_1`
- [ ] [A2] drop player box (4 of 5): `drop_standard_border`, `drop_standard_button`, `dropPlayerBackgroundPanelArea`, `dropPlayerBlackArea`
- [ ] [A2] load percentage box (3 of 3): `loadingPlayerBackgroundPanelArea`, `loadingPlayerBlackArea`, `loadingPlayerPanelArea`
- [ ] [A2] map radar (4 of 18): `bb_standard`, `minimapBackgroundPanelArea`, `minimapShipFilterBorder`, `minimapStationFilterBorder`
- [ ] [A2] message display (4 of 5): `pause_standard_border`, `pause_standard_button`, `objectivesBackgroundPanelArea`, `objectivesBlackArea`
- [ ] [A2] objectives display (4 of 5): `pause_standard_border`, `pause_standard_button`, `objectivesBackgroundPanelArea`, `objectivesBlackArea`
- [ ] [A2] pause box (5 of 5): `pause_standard_border`, `pause_standard_button`, `pauseGameBackgroundPanelArea`, `pauseGameBlackArea`, `pauseGamePanelArea`
- [ ] [A2] popup palette (4 of 12): `popupPaletteXB`, `popupPaletteYB`, `staticPaletteHeightB`, `staticPaletteWidthB`
- [ ] [A2] replay panel (23 of 23): `bb_standard`, `fast_button`, `fastBorder`, `fastButton`, `fastButtonArea`, `normal_button`, `normalBorder`, `normalButton`, `normalButtonArea`, `pause_button`, `pauseBorder`, `pauseButton`, `pauseButtonArea`, `replayBackgroundPanelArea`, `replayPanelArea`, `restart_button`, `restartBorder`, `restartButton`, `restartButtonArea`, `slow_button`, `slowBorder`, `slowButton`, `slowButtonArea`
- [ ] [A2] ship display (33 of 50): `infoBackgroundPanelArea`, `infoBuildCrew`, `infoBuildCrewIcon`, `infoBuildEnergy`, `infoBuildEnergyIcon`, `infoBuildOfficers`, `infoBuildOfficersIcon`, `infoCrewNum`, `infoCrewNumIcon`, `infoEnergy`, `infoEnergyIcon`, `infoLowBackgroundPanelArea`, `infoMiddleBackgroundPanelArea`, `infoOfficers`, `infoOfficersIcon`, `infoProgressBar`, `infoSingleConstructionArea`, `infoSingleConstructionBarDisableColor`, `infoSingleDilithiumBar`, `infoSingleDilithiumBarArea`, `infoSingleDilithiumBarDisableColor`, `infoSinglePlanetMetalBarArea`, `infoSinglePlanetMetalBarColor`, `infoSinglePlanetMetalBarDisableColor`, `infoSinglePlanetTeamTextArea`, `infoSingleRaceIconBackground`, `infoSingleShieldBarDisableColor`, `large_construction_bar`, `large_dilithium_bar`, `large_metal_bar`, `large_shield_bar`, `race_icon_bar`, `infoAreaYOffset`
- [ ] [A2] ship icon (3 of 3): `infoMultiShieldBar`, `infoMultiShieldBarDisableColor`, `small_shield_bar`
- [ ] [A2] race (5 of 12): `ADM_COMMISSIONED`, `ADM_DECOMMISSIONED`, `clickSound`, `GUI_RD_OFFICERS_TOOLTIP`, `rolloverSound`
- [ ] [A2] selection display (4 of 5): `group_background`, `select_crew_dot`, `select_energy_bar`, `select_system`
- [ ] [A2] admiral's log tabs (3 of 5): `playerButtonRect`, `tabBmp_`, `tabBmpH_`
- [ ] [FO] `gui_*.cfg`, comm panel (184): `CommandInfoBarButtonDistance`, `commallianceheadingtextarea`, `commallianceheadingtextlabel`, `commallianceheadingtextlabelarea`, `commalliedvictorybuttonarea`, `commalliedvictorybuttonimage`, `commalliedvictorybuttontoggle`, `commalliedvictorybuttontooltip`, `commalliedvictorytextarea`, `commalliedvictorytextcolor`, `commalliedvictorytextlabel`, `commalliedvictorytextlabelarea`, `commandinfobarbackgroundpanel`, `commandinfobarbackgroundpanelarea`, `commandinfobarbackgroundpanelsize`, `commandinfobarbuttonrect`, `commandinfobardefaultcursor`, `commandinfobarpanelarea`, `commbackgroundpanel`, `commbackgroundpanel_#`, `commbackgroundpanelarea`, `commbackgroundpanelsize`, `commbarallybuttonarea`, `commbarallybuttonimage`, `commbarallybuttonmarker`, `commbarallybuttontoggle`, `commbarallybuttontooltip`, `commbarchatbuttonarea`, `commbarchatbuttonimage`, `commbarchatbuttontoggle`, `commbarchatbuttontooltip`, `commbarenemybuttonarea`, `commbarenemybuttonimage`, `commbarenemybuttonmarker`, `commbarenemybuttontooltip`, `commbarnamebuttonarea`, `commbarnamebuttonimage`, `commbarnamebuttontooltip`, `commbarnametextarea`, `commbarnametextcolor`, `commbarnametextinformationarea`, `commbarneutralbuttonarea`, `commbarneutralbuttonimage`, `commbarneutralbuttonmarker`, `commbarneutralbuttontooltip`, `commbarnumberofplayers`, `commbarplayer_#`, `commbarselectbuttonarea`, `commbarselectbuttonimage`, `commbarselectbuttontoggle`, `commbarselectbuttontooltip`, `commbarteamtextarea`, `commbarteamtextbackground`, `commbarteamtextbackgroundarea`, `commbarteamtextcolor`, `commbarteamtextinformationarea`, `commbiomatteramounttextarea`, `commbiomatteramounttextbackground`, `commbiomatteramounttextbackgroundarea`, `commbiomatteramounttextinformationarea`, `commbiomatterdownbuttonarea`, `commbiomatterdownbuttonimage`, `commbiomatterdownbuttontooltip`, `commbiomatterupbuttonarea`, `commbiomatterupbuttonimage`, `commbiomatterupbuttontooltip`, `commbutton`, `commbuttonarea`, `commcrewamounttextarea`, `commcrewamounttextbackground`, `commcrewamounttextbackgroundarea`, `commcrewamounttextinformationarea`, `commcrewdownbuttonarea`, `commcrewdownbuttonimage`, `commcrewdownbuttontooltip`, `commcrewupbuttonarea`, `commcrewupbuttonimage`, `commcrewupbuttontooltip`, `commdefaultcursor`, `commdilithiumamounttextarea`, `commdilithiumamounttextbackground`, `commdilithiumamounttextbackgroundarea`, `commdilithiumamounttextinformationarea`, `commdilithiumdownbuttonarea`, `commdilithiumdownbuttonimage`, `commdilithiumdownbuttontooltip`, `commdilithiumupbuttonarea`, `commdilithiumupbuttonimage`, `commdilithiumupbuttontooltip`, `commgivebiomatterbuttonarea`, `commgivebiomatterbuttonimage`, `commgivebiomatterbuttontext`, `commgivebiomatterbuttontooltip`, `commgivecrewbuttonarea`, `commgivecrewbuttonimage`, `commgivecrewbuttontext`, `commgivecrewbuttontooltip`, `commgivedilithiumbuttonarea`, `commgivedilithiumbuttonimage`, `commgivedilithiumbuttontext`, `commgivedilithiumbuttontooltip`, `commgivegrouptextarea`, `commgivegrouptextlabel`, `commgivegrouptextlabelarea`, `commgivelatinumbuttonarea`, `commgivelatinumbuttonimage`, `commgivelatinumbuttontext`, `commgivelatinumbuttontooltip`, `commgivemetalbuttonarea`, `commgivemetalbuttonimage`, `commgivemetalbuttontext`, `commgivemetalbuttontooltip`, `commgiveunitsbuttonarea`, `commgiveunitsbuttonimage`, `commgiveunitsbuttontext`, `commgiveunitsbuttontooltip`, `commindicatoranchor`, `commindicatorcolor`, `commindicatorsprite`, `commindicatorwidth`, `commlatencybuttonarea`, `commlatencybuttonimage`, `commlatencybuttontoggle`, `commlatencybuttontooltip`, `commlatencytextarea`, `commlatencytextcolor`, `commlatencytextlabel`, `commlatencytextlabelarea`, `commlatinumamounttextarea`, `commlatinumamounttextbackground`, `commlatinumamounttextbackgroundarea`, `commlatinumamounttextinformationarea`, `commlatinumdownbuttonarea`, `commlatinumdownbuttonimage`, `commlatinumdownbuttontooltip`, `commlatinumupbuttonarea`, `commlatinumupbuttonimage`, `commlatinumupbuttontooltip`, `commmessageallbuttonarea`, `commmessageallbuttonimage`, `commmessageallbuttontext`, `commmessageallbuttontooltip`, `commmessagealliesbuttonarea`, `commmessagealliesbuttonimage`, `commmessagealliesbuttontext`, `commmessagealliesbuttontooltip`, `commmessageenemiesbuttonarea`, `commmessageenemiesbuttonimage`, `commmessageenemiesbuttontext`, `commmessageenemiesbuttontooltip`, `commmessagegrouptextarea`, `commmessagegrouptextlabel`, `commmessagegrouptextlabelarea`, `commmessageheadingtextarea`, `commmessageheadingtextlabel`, `commmessageheadingtextlabelarea`, `commmessageprivatebuttonarea`, `commmessageprivatebuttonimage`, `commmessageprivatebuttontext`, `commmessageprivatebuttontooltip`, `commmetalamounttextarea`, `commmetalamounttextbackground`, `commmetalamounttextbackgroundarea`, `commmetalamounttextinformationarea`, `commmetaldownbuttonarea`, `commmetaldownbuttonimage`, `commmetaldownbuttontooltip`, `commmetalupbuttonarea`, `commmetalupbuttonimage`, `commmetalupbuttontooltip`, `commokaybuttonarea`, `commokaybuttonimage`, `commokaybuttontext`, `commpanelarea`, `commplayerheadingtextarea`, `commplayerheadingtextlabel`, `commplayerheadingtextlabelarea`, `commteamheadingtextarea`, `commteamheadingtextlabel`, `commteamheadingtextlabelarea`, `commtitletextarea`, `commtitletextcolor`, `commtitletextlabel`, `commtitletextlabelarea`
- [ ] [FO] `gui_*.cfg`, other (17): `groupTextColor`, `fastbuttonarea`, `helpbutton`, `helpbuttonarea`, `idlebutton`, `idlebuttonarea`, `menubuttonarea`, `morebuttonarea`, `normalbuttonarea`, `pausebuttonarea`, `replaypanelarea`, `selectsystem_#`, `slowbuttonarea`, `togglebutton`, `togglebuttonarea`, `xtooltip`, `ytooltip`
- [ ] [FO] `gui_*.cfg`, avatar box (6): `avatarboxbackgroundpanel`, `avatarboxbackgroundpanelarea`, `avatarboxbackgroundpanelsize`, `avatarboxdefaultcursor`, `avatarboxiconarea`, `avatarboxpanelarea`
- [ ] [FO] `gui_*.cfg`, bridge (32): `b_bridgebackground`, `b_bridgebackground_#`, `b_bridgebackgroundrect`, `b_bridgebackgroundsize`, `b_bridgelayer#rect`, `b_bridgelayer#sprite`, `bridgeblackrect`, `bridgepanelrect`, `d_bridgebackground`, `d_bridgebackground_#`, `d_bridgebackgroundrect`, `d_bridgebackgroundsize`, `d_bridgelayer#rect`, `d_bridgelayer#sprite`, `f_bridgebackground`, `f_bridgebackground_#`, `f_bridgebackgroundrect`, `f_bridgebackgroundsize`, `f_bridgelayer#rect`, `f_bridgelayer#sprite`, `k_bridgebackground`, `k_bridgebackground_#`, `k_bridgebackgroundrect`, `k_bridgebackgroundsize`, `k_bridgelayer#rect`, `k_bridgelayer#sprite`, `r_bridgebackground`, `r_bridgebackground_#`, `r_bridgebackgroundrect`, `r_bridgebackgroundsize`, `r_bridgelayer#rect`, `r_bridgelayer#sprite`
- [ ] [FO] `gui_*.cfg`, build box (6): `buildboxbackgroundpanel`, `buildboxbackgroundpanelarea`, `buildboxbuildbutton`, `buildboxdefaultcursor`, `buildboxpanelarea`, `buildboxresourcetext`
- [ ] [FO] `gui_*.cfg`, button panel (5): `buttonbackgroundpanel`, `buttonbackgroundpanel_#`, `buttonbackgroundpanelarea`, `buttonbackgroundpanelsize`, `buttondefaultcursor`
- [ ] [FO] `gui_*.cfg`, cinematic (3): `cinematicbackgroundpanelarea`, `cinematicdefaultbackground`, `cinematicdefaultcursor`
- [ ] [FO] `gui_*.cfg`, drop-player box (16): `dropplayeraitakeoverbuttonarea`, `dropplayeraitakeoverbuttonimage`, `dropplayeraitakeoverbuttontext`, `dropplayeraitakeoverbuttontooltip`, `dropplayerbackgroundpanel`, `dropplayerbackgroundpanel_#`, `dropplayerbackgroundpanelarea`, `dropplayerbackgroundpanelsize`, `dropplayerdefaultcursor`, `dropplayerdropbuttonarea`, `dropplayerdropbuttonimage`, `dropplayerdropbuttontext`, `dropplayerdropbuttontooltip`, `dropplayertextarea`, `dropplayertextcolor`, `dropplayertextinformationarea`
- [ ] [FO] `gui_*.cfg`, tooltip and cursor (6): `framelocation`, `tooltipbackgroundcolor`, `tooltipframebottomrect_#`, `tooltipframeleftrect`, `tooltipframerightrect`, `tooltipframetoprect_#`
- [ ] [FO] `gui_*.cfg`, info panel, other (31): `infobackgroundpanel_#`, `infobackgroundpanelarea`, `infobaroffset_#`, `infoblackarea_#`, `infobuildcrew`, `infobuildcrewicon`, `infobuildenergy`, `infobuildenergyicon`, `infobuildofficers`, `infobuildofficersicon`, `infobuildqueue_#`, `infobuildqueueslot_#`, `infobuildsystem_#`, `infobuildsystemicon_#`, `infocrewnum`, `infocrewnumicon`, `infodefaultcursor`, `infoenergy`, `infoenergyicon`, `infolowbackgroundpanelarea`, `infolowbackgroundpanelsize`, `infomiddlebackgroundpanel_#`, `infomiddlebackgroundpanelarea`, `infomiddlebackgroundpanelsize`, `infoofficers`, `infoofficersicon`, `infopanelarea_#`, `infoprogressbar`, `infosystem_#`, `infosystemicon_#`, `infoteam`
- [ ] [FO] `gui_*.cfg`, info panel, multi selection (12): `infomultihullbar`, `infomultishieldbar`, `infomultishieldbardisablecolor`, `infomultishiphull_#`, `infomultishiphullsm_#`, `infomultishipicon_#`, `infomultishipiconsm_#`, `infomultishipname_#`, `infomultishipshield_#`, `infomultishipshieldsm_#`, `infomultishipwireframe_#`, `infomultishipwireframesm_#`
- [ ] [FO] `gui_*.cfg`, info panel, single selection (17): `infosingleconstructionarea`, `infosingleconstructionbardisablecolor`, `infosinglecrewbarcolor`, `infosingledilithiumbar`, `infosingledilithiumbararea`, `infosingledilithiumbarcolor`, `infosingledilithiumbardisablecolor`, `infosinglelatinumbarcolor`, `infosinglemetalbarcolor`, `infosingleplanetmetalbar`, `infosingleplanetmetalbararea`, `infosingleplanetmetalbarcolor`, `infosingleplanetmetalbardisablecolor`, `infosingleplanetteamtextarea`, `infosingleshieldbardisablecolor`, `infosinglesystemsdisplay`, `infosinglesystemsicon`
- [ ] [FO] `gui_*.cfg`, loading box (9): `loadingplayerbackgroundpanel`, `loadingplayerbackgroundpanel_#`, `loadingplayerbackgroundpanelarea`, `loadingplayerbackgroundpanelsize`, `loadingplayerdefaultcursor`, `loadingplayerpanelarea`, `loadingplayertextarea`, `loadingplayertextcolor`, `loadingplayertextinformationarea`
- [ ] [FO] `gui_*.cfg`, minimap (3): `minimapbackgroundpanel_#`, `minimapbackgroundpanelarea`, `minimapdefaultcursor`
- [ ] [FO] `gui_*.cfg`, objectives (24): `objectivesbackgroundpanel`, `objectivesbackgroundpanel_#`, `objectivesbackgroundpanelarea`, `objectivesbackgroundpanelsize`, `objectivesbutton`, `objectivesbuttonarea`, `objectivesdefaultcursor`, `objectivesheaderarea`, `objectivesheadercolor`, `objectivesheaderinformationarea`, `objectivesokayborderimage`, `objectivesokaybuttonarea`, `objectivesokaybuttonimage`, `objectivesokaybuttontext`, `objectivesokaybuttontooltip`, `objectivessummarycolor`, `objectivessummaryheaderarea`, `objectivessummaryinformationarea`, `objectivestextarea`, `objectivestextcolor`, `objectivestextinformationarea`, `objectivestitlearea`, `objectivestitlecolor`, `objectivestitleinformationarea`
- [ ] [FO] `gui_*.cfg`, command palette (6): `palettebutton`, `palettebuttonarea`, `popuppalettexb`, `popuppaletteyb`, `staticpaletteheightb`, `staticpalettewidthb`
- [ ] [FO] `gui_*.cfg`, pause box (14): `pausegamebackgroundpanel`, `pausegamebackgroundpanelarea`, `pausegamebackgroundpanelsize`, `pausegamedefaultcursor`, `pausegamepanelarea`, `pausegametextarea`, `pausegametextcolor`, `pausegametextlabel`, `pausegametextlabelarea`, `pausegameunpauseborderimage`, `pausegameunpausebuttonarea`, `pausegameunpausebuttonimage`, `pausegameunpausebuttontext`, `pausegameunpausebuttontooltip`
- [ ] [FO] `gui_*.cfg`, resource bar (5): `resource_#`, `resource_#_color`, `resourcedefaultcursor`, `resourcepanel_#`, `restartbuttonarea`

FO total: 396 of 474 key names in its `gui_*.cfg` files and hook reads are not mentioned in STA64.

### AIP names

- [ ] [A1] read by the executable (26 of 31): `attack_troops_priority`, `attack_region_priority`, `defend_priority`, `attack_enemy_base_priority`, `intrinsic_value_bonus`, `exploration_priority`, `scripted_priority`, `perimeter_priority`, `resource_priority`, `escort_priority`, `min_escort_force`, `max_escort_force`, `max_resource_force`, `max_defense_force`, `min_exploration_force`, `max_exploration_force`, `relaxation_cycles`, `max_danger_raw_bonus`, `max_threat_raw_bonus`, `distance_from_home_priority_modifier`, `distance_from_enemy_priority_modifier`, `avoid_danger_priority_modifier`, `human_target_raw_bonus`, `max_misc_bid_bonus`, `persistence_modifier`, `patrol_one_in_n`
- [ ] [A2] read by the executable (26 of 33): `planetThreatBonus`, `attack_troops_priority`, `attack_region_priority`, `defend_priority`, `attack_enemy_base_priority`, `intrinsic_value_bonus`, `exploration_priority`, `explorationFailurePenalty`, `scripted_priority`, `perimeter_priority`, `resource_priority`, `escort_priority`, `min_escort_force`, `max_escort_force`, `max_resource_force`, `max_defense_force`, `min_exploration_force`, `max_exploration_force`, `relaxation_cycles`, `distance_from_home_priority_modifier`, `distance_from_enemy_priority_modifier`, `avoid_danger_priority_modifier`, `human_target_raw_bonus`, `max_misc_bid_bonus`, `persistence_modifier`, `patrol_one_in_n`

## Done recently

- [x] Map-confinement geometry prerequisite (SIM-22 stage 1): checked Fixed boundary volumes, strict edge comparisons, asymmetric upper projection and inspect-only reporting pass five focused tests, including a 405-point containment/idempotence grid. This is a pure shared helper; entity state, movement enforcement, profile/ODF defaults, save/replay integration and Lua ForceOntoMap remain unfinished. No new in-game test applies until that integration is delivered.
- [x] BZN reader, text + binary; all 517 installed maps; object counts verified — `f10cc94`
- [x] Map view: objects placed, team colours, shared parts — `f10cc94`
- [x] Native M3D loading (no SOD conversion), glow maps from STL slot — `ad27a39`, `5287d6f`
- [x] Normal-map convention verified against Bevy's tangents — `6b12c12`
- [x] Team colours from `teamcolor.odf` — `4cea39a`
- [x] Damage-state nodes hidden on healthy craft — `5287d6f`
- [ ] Construction, cloak FX: a station under construction welds and firms up, `Built` flashes; a cloaked own unit goes translucent and back (RND-11)
- [?] Resource moons (RND-16), `cargo run -- --map <name>` on an FO4, FO or DW map with `trigger_dilithium` / `trigger_tritanium` markers: each draws a dilithium or tritanium moon model (one of four variants, as the game's replace weapon lists them), never the editor letter; it is neutral, shows no selection ring and plays no ship voice; A2 maps (`mdmoon`) look as before
- [?] Language packs (CON-11): `cargo test -p sta_content --lib strings -- --nocapture` lists a string and label count for every installed game and mod (A1, A2, A2 stock, Dominion War, FO, FO Roots, Fleet Ops 4.0, STA2 Classic) and passes; the UI tooltip "Double click to return to strategic mode." still shows in-game
- [?] Asteroid fields block ships: `STA64_TEST_INSTALL=<FO Roots dir> STA64_TEST_MOD="Fleet Ops 4.0" cargo test -p sta_content --test asteroid_install` passes. In a skirmish on FO 4 `AsteroidZone`, order a ship straight across one of the hidden `trigger_blocking` walls: it stops at the edge or slides round it, never inside (no path search yet, so it may stall behind a wall it is aimed squarely at; proposed follow-up in the PR). Maps without fields fly as before.
- [?] Ships and obstacles (SIM-21): `cargo run -p sta_bench --bin sta-bench -- run avoid-planet avoid-field crossing construction` passes; `STA64_TEST_INSTALL=<FO Roots dir> STA64_TEST_MOD="Fleet Ops 4.0" cargo test -p sta_bench --test avoid_install` passes. In a skirmish on FO 4 `AsteroidZone`, order a ship straight through an asteroid field or past a planet: it flies round and arrives instead of stopping at the edge; two groups ordered across each other do not pass through one another; a point inside a planet ends at its edge. Order a rig to build: the structure goes up on the chosen point and the rig parks beside it, outside its footprint. Own stations are not solid to their owner (freighters still dock). Unit radii come from the model bounds, so a unit without a readable model takes no room.

- [x] UI-17 profile foundation (#369): A2/FO stock family is the mounted-game default; `--lcars-ui` and Options -> LCARS interface override it, and optional `[Mod] UiProfile` metadata/registered preset aliases are resolved through the mount. 399 client tests (one existing ignored) plus strict Clippy/workspace/ARM64 checks pass. Persistent chrome, root camera targets and UI state survive switching. Actual stock CFG/SPR HUD rendering is UI-18 and remains the active follow-up; no stock visual-parity claim from this foundation.

- [x] UI-18 CFG/SPR renderer pass (#371): authored A2/FO resource/info/minimap backgrounds, upward command bars, ODF artwork/menu/tints, layered wireframes, full build-list paging and retained LCARS/root/state paths. 426 client tests (one existing ignored), strict Clippy, workspace/all-targets and ARM64 library checks pass; four installed-data NVIDIA Vulkan HUD captures cover Classic/FO4 at 4:3 and 16:9. This is rendering and shared-command evidence, not native parity acceptance.
- [ ] UI-18 live/reference acceptance (#371): compare Classic, FO Roots and FO4 resource/info/minimap/bar geometry and supported command order for ships, yards, constructors and groups; check long build lists, stop/target/build/selection, LCARS restoration and FPS. Keep #371 open until the stock-reference gate passes; crew/energy view data, per-system wireframe colours, named fonts, UI-20 menu backends and UI-26 jobs remain separate work. See [STOCK_HUD_CFG.md](docs/STOCK_HUD_CFG.md).
- [ ] UI-18 tooltip localization (#371): hover a native command, then a build icon until its expanded tooltip appears. Compare text with the mounted `Dynamic_Localized_Strings.h` entries; missing entries retain ODF/source text, and deliberately empty entries stay blank. Source/headless checks do not establish native hover rendering acceptance.

- [x] UI-18 live-feedback correction: authored atlas pieces now stretch to the CFG rectangle instead of letterboxing on wide screens; stock labels use a readable scalable-text fallback with an 18-unit minimum, while tiny icon-only radar filters keep tooltips. 427 client tests (one pre-existing ignored), strict Clippy/workspace/ARM64 checks and four refreshed installed-data Vulkan captures pass; a pixel assertion guards the previously broken Classic resource-panel join. Native/live retest remains open under #371.
- [?] A2 presentation requests (LUA-05 slice, #149): `python3 tools/test_mission_dll_inventory.py` passes and `tools/mission_dll_inventory.py` reports 43 exact A2 callables / 89 missing; `cargo test -p sta_mission native_presentation native_camera` passes. In a mission, `A2.MinimapMessage(1,2,0.5)` or `A2.GridVisible(false)` queues a presentation request in the command log (no client consumer yet, so nothing is drawn)
- [?] Lua sandbox and installed missions (LUA-09, LUA-04): `cargo test -p sta_mission sandbox` passes (no `io`/`os`/`require`/`load`, runaway loop and allocation stopped); `STA64_TEST_INSTALL=<root> STA64_TEST_MOD=<mod or empty> cargo test -p sta_mission --test missions -- --nocapture` (and `STA64_TEST_DIR=<dir>` for a bare game folder) prints scripts run / stubbed / failed per install; no `PROBLEM` lines
- [?] Mission object gaps (LUA-05 slice, #149): `STA64_TEST_INSTALL=<FO Roots> STA64_TEST_MOD= cargo test -p sta_mission --test missions -- --nocapture` reports 3 run / 16 stubbed / 17 failed (was 2 / 0 / 34); `cargo test -p sta_mission` passes. The stubbed ones name each assignment the sim cannot apply yet (`Entity.shields = x (needs a sim shield edit)`, ...)
- [?] Roots lineage by `AssetVersion` (CON-43, #446): `STA64_TEST_INSTALL=<FO Roots dir> STA64_TEST_STOCK_A2=<stock A2 dir> STA64_TEST_A1=<A1 dir> STA64_TEST_FO3=<FO 3.x dir>:<Dominion War dir> cargo test -p sta_content --test profile_install -- --nocapture` passes and prints `fo4 (AssetVersion ...)` for base Roots, Fleet Ops 4.0, STA2 Classic Mod and its Enhanced Gameplay; `sta2classic` for stock A2, `sta1classic` (Zfs) for A1. Then start a skirmish on `--mod "STA2 Classic Mod"`: the startup content line reports `fo4` and Classic's Nebula point-defence weapon fires.
- [?] Unit voices follow the player's race (voices regression, Cedric 2026-11-12): in a base Roots skirmish as Federation, select and order several ships and stations; they must speak Federation voice lines, never Borg ones (the start loadout opens with the shared `all_ai_pod` unit of race `borg`, which used to name the player's race). Repeat as Klingon and Borg, and in Fleet Ops 4.0 and STA2 Classic. Listen by ear; `STA64_TEST_INSTALL=<FO Roots dir> cargo test -p sta_content --test voices_install` checks the data side.
- [?] Starfield without a texture (Classic report, Cedric 2026-11-12): in a STA2 Classic skirmish at a wide resolution the stars still look like flat light squares? That is Classic's own star art (`Mstars.TGA` bstar.0, a 16 px blocky disc, 750 of them at 4 to 14 px). If a star texture ever cannot be found the sky now logs `starfield: texture ... not found` and draws nothing instead of white squares. Please send a Classic screenshot next to a Fleet Ops one so the star size and filtering can be measured.
- [?] Dropdown list not clipped (UI popup rooting): open a dropdown inside a scrolling page (the options screen of UI-20 or EDT-12 once they have one; today only the tests build one); the list must show whole over the page edge, row clicks choose once, a click outside closes it, and it opens above the field near the bottom of the window. Layout is tested; the look is not (no rendered frames on the agent machine).
- [?] Ship shadows (RND-15, #432): in a skirmish on a map with a gas nebula (Refit profile, `STA64_RENDER_PROFILE=refit`), fly a ship into the gas with the Key light behind it: the gas on the far side should be darker in the ship's shadow, soft at the edges. Hulls under another ship (or station) should be shaded too, and no hull should shadow itself. `STA64_GPU_TESTS=1 cargo test -p sta64 --lib render::shadows_gpu -- --test-threads=1` checks both on a GPU.
- [?] Event-track triggers (RND-21, #431): in a skirmish, a shipyard's bay doors open when it builds or launches, a turret or platform weapon animation plays when it fires, and nothing hangs open. In the model viewer a model with open-ended tracks (e.g. a worker ship) plays them by itself every few seconds. Docking, workers at work and deploy still have no sim event.

- [ ] RND-33 hull customization: with an authored test ship, compare two faction owners and capture, missing texture fallback, faction subtree switching, destroyed/repaired subsystem parts and their effects; inspect projected scorch, existing registry images and optional registry text on curved/animated hulls under Stock and Refit. Check cloak/fog, save/load and second-match cleanup. Automated synthetic evidence is separate from this native/manual visual check.
- [?] Reveal cheats (UI-46, #831): in a campaign mission press Alt+F: the dim over unseen ground goes and enemy units show everywhere on the map and the minimap; Alt+V: the black over never-seen ground goes; each press says so in a notice and a second press restores it with what was explored kept. `--reveal all` starts lifted. In a multiplayer match the keys do nothing. The mission and the AI play on exactly as without (a Ctrl+F12 capture replays to the same digest).
- [ ] Reveal chords are not rebindable (UI-46): Alt+F and Alt+V read the keyboard directly, not through `KeyBindings`; add actions when the settings screen should list them.

- [?] RND-36 construction correction: while placing a station, check the square green/red footprint grid; after confirming, the faint station ghost remains during constructor travel and disappears when welding begins. Cancel placement/build and return to the menu to check cleanup. Repeat with a different mounted game/mod grid size.

- [?] RND-36 all footprints: arm station placement and inspect footprints under existing ships/stations, moons/planets/wormholes, nebulae and asteroid fields; move a ship and verify its grid follows. Cancel or confirm placement: all grids disappear, while a confirmed station blueprint stays only until welding starts. Check hidden objects stay hidden and a second match starts clean.
- [?] Single-player cheat input (UI-47, #848): Enter opens, exact code then Enter submits, Escape cancels; check typing never issues map orders, moves the camera or opens the game menu. Check the A1/A2/FO money amounts, selected-ship FO codes, forced campaign win/loss and refusal in multiplayer. See docs/CHEATS.md; no native/graphical acceptance yet.
- [ ] Cheat prerequisites (UI-47): A1 `canofwhoopass` needs original tactical-process modes; FO `levelup` needs rank/replacement progression. Both currently explain the missing system and apply no substitute. Optional A2FOCheats configurable five-resource grants and timed `dis` recovery are also absent.
