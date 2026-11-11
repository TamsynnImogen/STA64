# Code-level mods: FleetOps hooks, ASIs, patched exes (research)

Feeds `docs/MODDING-IDEAS.md`. Method: read-only inspection (file, strings, objdump -h, md5sum, `.map`
files, shipped readmes and changelogs) plus the Refit repo's own measurements
(`/home/cedric/Projects/armada2-refit`). Nothing was executed. No binary was disassembled; the
behaviour column comes from shipped changelogs and from Refit's measured notes, not from reverse
engineering done here.

Install root: `/home/cedric/Games/Heroic/`. Abbreviations: FO = Fleet Operations, FOH =
`FleetOpsHook.dll`, A2 = Armada II, "stock" = A2 1.1 (GOG).

## 1. What is actually installed (corrects the brief)

| Folder | What it really is |
|---|---|
| `Star Trek Armada` (A1) | Stock A1 + compat shims only (IPXWrapper dlls, a ddraw.dll). No code mod. |
| `Star Trek Armada II (Stock)` | GOG A2 1.1, plus Refit's platform layer (widescreen patch, ASI loader, d3d8/d3d9 shims). |
| `Star Trek Armada II` | Same, plus the **Refit** layers (Render3D/UI/QOL/QOLRules/Online `.asi`, Display8 `d3d8.dll`). Refit is the user's own project, not a third-party mod. |
| `... Fleet Operations` | FO **3.2.7** (readme says so; not 4.0). Delphi launcher + `FleetOpsHook.dll` injected into the original exe. |
| `... Dominion War` | Byte-identical FO 3.2.7 engine files (same md5 for FleetOps.exe, FOH, Armada2.exe, RTS_CFG.h, ART_CFG.h). Only the data differs (`Data/` 2.4 GB vs 1.3 GB, many more `addon/` ODFs). Zero code difference. |
| `... Fleet Operations Roots` | Roots **5.0Z11** (JanB, 2026): a different engine, `ArmadaL.exe`, plus FOH 5.x. Hosts "FO 4.0", "STA2 Classic" and "STA2 Classic Enhanced Gameplay" as submods in `Data/Mods/`. |

So "FO 4.0" and "STA2 Classic" are **data mods on the Roots engine**, not separate installs. FO 4.0 ships
its own `ArmadaL.exe` but it is byte-identical to Roots' (md5 `44bf3a2f...`). There are only
three distinct code lineages: stock A2, FO 3.x (hook only), Roots (patched exe + hook).

## 2. How each lineage injects code

| Lineage | Mechanism | Evidence |
|---|---|---|
| FO 3.2.7 | `FleetOps.exe` (Delphi launcher) starts `Data/Armada2.exe` (a **Delphi loader**, 281 KB, sections CODE/DATA, not the game). The real game is the stock exe renamed `Armada2.ldl` (3,637,293 bytes = stock exe size). `Armada2Load.dll` (UPX) and `Win2kDisableTaskSwitch.dll` load `FleetOpsHook.dll`, which detours engine methods **in memory**; the exe on disk is never patched. | `Data/Armada2.exe` `objdump -h`; `Data/Armada2.ldl` size; `FleetOpsHook.map` |
| Roots 5.0Z11 | `Data/ArmadaL.exe` is stock 1.1's image **patched on disk**: same sections plus a 43 KB executable `.test` section at 0x7fa000 (FO code), about 4.2 KB changed in `.text` over 210 functions (154 jumps/calls into `.test`, 764 small constant/branch edits). FOH.dll is then loaded by Roots' own `Win2kDisableTaskSwitch.dll` and patches about 1,260 more ranges at load (550 calls, 80 jumps, 50 vtable slots). Measured by Refit with a memdiff plugin. | `Data/ArmadaL.exe` sections; `armada2-refit/PLAN-FLEETOPS.md` "The executable", "FleetOpsHook.dll at run time" |
| Refit | Ultimate ASI Loader (`winmm.dll`, MIT) loads `.asi` plugins after Display8 `d3d8.dll`; plugins patch `Armada2.exe` in memory through signature-checked "hooks" that stand down individually. | `armada2-refit/qol/README.md` "Hooks" |

Important for STA64: **all code mods are detours at engine function granularity** (class
`Craft`, `Producer`, `Shipyard`, `cOverViewImp`, ...). The FOH.map unit names are effectively an index of
engine subsystems FO rewrote (section 4). Every behavioural change below is therefore expressible as a
change to a named engine rule, not as new arbitrary code.

## 3. Inventory

Category key: **T** tunable/limit, **Q** ODF-semantics quirk (key meaning or new key), **F** new
engine feature, **B** bug-fix quirk, **R** render, **U** UI, **N** network/platform plumbing (no STA64
equivalent), **I** install/launcher.

### 3.1 Binary and loader inventory

| Install | File | What it changes | Cat | Evidence path |
|---|---|---|---|---|
| A1 | `ddraw.dll`, `dpwsockx.dll`, `wsock32.dll`, `mswsock.dll`, `ipxwrapper.dll`, `ipxconfig.exe` | Compatibility shims (IPX over UDP, DirectDraw replacement). No gameplay change. Not mods. | N | `Star Trek Armada/ipxwrapper-readme.txt` |
| A1 | `Armada.exe`, `Patches/` (empty) | Stock. No code mod found. | - | `Star Trek Armada/` |
| A2 stock | `winmm.dll` (Ultimate ASI Loader 4.68, ThirteenAG, MIT) | Generic `.asi` loader (dinput8 wrapper renamed winmm). Needs Wine override `winmm=n,b`. | I | strings: `ultimate-asi-loader`; `UltimateASILoader-license.txt` |
| A2 stock | `STA2WidescreenPatch.asi` (Ligushka v1.0, MIT, 9,216 B) | Fixes the 3D view for widescreen (Hor+ aspect). Not the UI. Resolution is written to `ARMADA.PRF` line 5. | R | `armada2-refit/platform/README.md` "Widescreen"; `platform/vendor/STA2WidescreenPatch-1.0/SOURCE.txt` |
| A2 stock | `d3d8.dll` 124 KB, `d3d9.dll` 7.8 MB (DXVK) | d3d8to9 shim then DXVK. Renderer plumbing only. | N | `strings` (dxvk symbols); Refit calls the chain "retired" |
| A2 stock | `Win2kDisableTaskSwitch.dll` (221 KB), `CKController.dll`, `imagehlp.dll`, `Dxver.dll`, `NetworkManager.dll` | Shipped with the retail game (DX version check, CD-key, lobby). Not mods. In FO/Roots, `Win2kDisableTaskSwitch.dll` is a **different, 16 KB build whose attach loads FOH**. | I | `objdump`; PLAN-FLEETOPS |
| A2 stock | `Armada II 1.1 Patch Readme.txt` | The official 1.1 patch's changes (section 3.4) are the *baseline* that FO and Roots build on. | - | file in install |
| A2 (Refit) | `QOL.asi` | Right-drag pan speed, selection/control groups up to 120, Shift+number adds to group, stations in groups, long moves go to warp, move queues, construction ship ignores own blueprint, grid hotkeys. | T/F/U | `armada2-refit/qol/README.md` |
| A2 (Refit) | `QOLRules.asi` | Pay-on-queue / refund-on-cancel (changes simulation, must-match in MP). | F (rules) | `armada2-refit/rules/README.md` |
| A2 (Refit) | `Render3D.asi`, `UI.asi`, `Display8` (`d3d8.dll`), `binkw32.dll` proxy, `Online.asi` | Renderer, 2D HUD/menus, Vulkan/D3D12 device, AV1/launch reels, UDP transport with rendezvous server. | R/U/N | `armada2-refit/MODDING.md` "What is separable". Detail belongs in `render-mods.md` |
| FO 3.2.7 and Dominion War | `FleetOps.exe` | Delphi launcher only. | I | `strings`, `file` |
| same | `Data/Armada2.exe` + `Armada2.ldl` + `Armada2Load.dll` | Loader + renamed stock exe + hook bootstrap (UPX). | I | section 2 |
| same | `FleetOpsHook.dll` (2.9 MB Delphi, madExcept) + `FleetOpsHook.map` | The whole FO engine mod: about 90 `*Enhancement`/`*Imp` units (section 4). Has a CRC list of allowed exes and checksums its own FPQ and cfg files for MP. | Q/F/B/T/U | `FleetOpsHook.map` unit list; `FOReadme.txt` |
| same | `d3d9.dll` 7.8 MB (DXVK) in root and `Data/` | DXVK; FOH imports `d3d9`/`D3DX9` but on the default path uses the engine's D3D8 device. | N | PLAN-FLEETOPS "Direct3D" |
| same | `Tunngle.dll` | Tunngle VPN API bridge for MP. | N | `FOReadme.txt` 3.2.7 |
| same | `FPQEdit.exe` | Editor for FPQ archives (container for ODFs, `ODF.fpq`). Not a hook. | I | `FOReadme.txt` 3.2.3 |
| same | `Data/RTS_CFG.h`, `ART_CFG.h`, `game_cfg.h`, `Input.map`, `label.map`, `mshell.set`, `packages.ini`, `events.dat`, `Dynamic_Localized_Strings.h` | Engine config text files (see 3.3). | T | files in `Data/` |
| Roots | `FleetOps.exe` (launcher), `FleetOpsHook.dll` (3.0 MB, different build), `.map` | Same architecture, FO 5.x. FOH.dll **checksum protection disabled** (permission from DOCa Cola). | Q/F/B | `devstuff/Engine_Changelog.txt` |
| Roots | `Data/ArmadaL.exe` (4.8 MB, patched stock + `.test` section), `ArmadaLoad2.dll` | The engine itself (section 2). Engine version "5.0.0.Bob (50000)". | Q/F/B/T | `Engine_Changelog.txt` |
| Roots | `Data/dxwrapper.dll` + `dxwrapper.ini`, `d3d8.dll` (Display8 here, DxWrapper stub in Roots as shipped), `d3d9_reshade.dll`, `d3d9_no_reshade.dll`, `D3DCompiler_43.dll`, `ReShade.ini`, `FleetOpsRootsReshadePresets.ini`, `Magnification_Roots.dll`, `msvcrt.dll` (patched, Nov 2023) | d3d8-to-9 conversion, ReShade post-FX, desktop-scaling workaround for fullscreen shell. | R/N | `Roots_ReadMe.pdf` s.2; `dxwrapper.ini` |
| Roots | `Data/QOL.asi`, `UI.asi`, `Render3D.asi`, `winmm.dll` (+ `.ini`) | **Not Roots'.** Same md5 as Refit's plugins (`ed8760f4...` UI.asi); Refit installed them into this clone. | - | md5 comparison with `Star Trek Armada II/` |
| Roots | `Data/Mods/<mod>/info.ini` | Mod manifest: `Name`, `Description`, `Author`, `Version`, `AssetVersion=50000`, `ParentMod`, `standalone`, `ignoreParentMaps`, `Screenshot`, `Icon`. Selected by `-mod <name>` or `/mod <name>`. | I | `Data/Mods/*/info.ini` |
| Roots mod "Fleet Ops 4.0" | own `ArmadaL.exe` (identical), `avatar differences.txt` | FO 4.0 gameplay on the Roots engine. | - | `Data/Mods/Fleet Ops 4.0/` |
| Roots mod "STA2 Classic" (+ Enhanced Gameplay) | `info.ini`, `Extras/Engine_Changelog.txt`, no exe | Data-only A2 port; runs on Roots' exe. Enhanced Gameplay is a pure data+rules submod (list in section 3.4). | - | `A2 Classic Submod ReadMe.pdf` |

### 3.2 Engine behaviours added by FO 3.2.7 (FOH.dll, in-memory detours)

Sources: `FOReadme.txt` ("Modding related engine changes", changelog) and FOH.map unit names.

| Behaviour | New ODF / cfg keys | Cat |
|---|---|---|
| New object keys on all game objects | `ignoreInterface`, `extraWeaponHardpoints`, `invincible`, `ignoredAsTarget`, `invalidAsTarget`, `clearsFog`, `selectHighlight`, `lightColor`, `lightFalloffStart`, `lightFalloffRange` | Q |
| New craft keys | `ShieldHit/Down/Add/Sub/Special1..4/HitCritical` (events), `buildableByOwnerRaceOnly`, `requiredAlliedRace`, `createFootprint`, `destroyFootprint`, `destroyMeWithResearchStation`, `avatarBoxIcon`, `worthXP`, `nextrankXP`, `nextrankClass`, `XPlevel`, `allowBuildQueuePop`, `showChoiceDialog`, `fleetCapFile`, `SpecialEnergyDisplayMode`, `overlaySod`, `IamGameToTheDeathCondition`, `constructionVoice`, `turnToFire`, `canShipyardRepair`, `perceivedNeutral` | Q/F |
| Research stations spawn modules; pods researched by yard | `buildItemXprovided`, `isResearchedByYard`; stations hold non-pod modules | F |
| Pre-queued shipyard builds | `pushBuildItemX` | F |
| Many more build items per producer | (limit lifted) | T |
| Fleet caps (build limits by class) | FleetCap files: `maxCap`, `fleetCapClassX`, `fleetCapClassXcapValue`, `capTooltip`, `capTooltipReached`, `capTooltipOver`, `affectsAI`, `simpleCapMode` | F |
| New weapon class `ReplaceWeapon` (unit transformation: promotion, mode switch, warp-in, fighters) | about 40 keys, `replacement*`, resource costs and `required*` | F |
| New `ResourceWeapon`, extended `DetectorWeapon`, `RepairWeapon`, `CloakingDeviceImp`/`Control` | `addDilithium`..., `MarkRadius`..., `standbyTime`, `eventFire`, `disableShield`, `disableWeapons`, `phaseCloak`, `decloakShieldDelay`, `energyDrainRate` | F/Q |
| `RamWeapon`, `CannonImp` extensions | `explosiondamage`, `targetTerrainOnly` | Q |
| Ordnance explosion keys; `RumbleExplosion` | `explosionHit`, `explosionShieldHit`, `explosionSizeFactor`, `explosionDuration`, `explosionSprite`; `rumbleDuration/Area/Strength` | Q/R |
| `AreaEffectCannon` 9 target modes (was 3) | key value range | Q |
| New weapon-range tier "strategic" (beyond artillery) | range class | Q |
| Race ODF keys | `allowGiveDilithium/Tritan/Metal/Supply/Crew`, `canGainXP`, `AIFreightersPerMoon`, `preloadUnitsX` | Q |
| GUI cfg dynamic values (non-4:3 aspect), per-faction colours, loadbar ODF | gui `*.cfg` keys, loadbar odf | U |
| Limits: ship-display system icons 20 -> 32; GameEvent race soundsets 16 -> 64; build-queue length; object-definition count (stock crashes or desyncs above 900 ODFs) | engine constants | T/B |
| Warp-in cap 9 -> 10 slots; troopship/carrier caps; officer cap by ODF | data via FleetCap | T |
| Experience system (XP, ranks), avatars, special energy as number | `worthXP`, `nextrankXP`, ... | F |
| ODF container: `ODF.fpq` (FPQ archive), read through `FileSystemHooks`; AIP/formation files loadable from mods | file system hook | I |
| MP file/CRC check over FPQ + `rts_cfg.h` + allowed-exe CRC list | | N |
| Map editor integrated; custom resolutions; experimental MSAA; PNG screenshots, `Ctrl+Alt+P` clean shot; FMOD Ex replaces sound engine | | U/R/N |
| Engine bug fixes (selected) | see 3.5 | B |

### 3.3 Config files as tunables (`Data/RTS_CFG.h`, `ART_CFG.h`, `game_cfg.h`)

These are stock-A2 files too, but the engine reads them at start; network games CRC `RTS_CFG.h` in FO and
stock (Refit notes this as the reason stock-compatible changes are made in memory). FO and Roots tune
them heavily. The Roots build adds 10 keys vs FO 3.2.7 and drops the `DETAIL_n_(NEAR|FAR)_CLIPPING_PLANE` set.

| Key | Meaning | Cat | Introduced by |
|---|---|---|---|
| `OVERVIEW_MAX_HEIGHT`, `OVERVIEW_MIN_HEIGHT`, scroll/zoom coefficients, `MAX_SCROLL_SPEED`, `KEYBOARD_SCROLL_RATE` | camera limits and speeds (stock patch 1.1 raised max zoom 1500 -> 2200) | T | stock, FO |
| `EASY/MEDIUM/HARD_AI_COST_HANDICAP`, `_SPEED_HANDICAP`, `_CLOAK_PERCENT`, `*_DAMAGE`, `*_MINING_MODIFIER` | difficulty tunables | T | stock |
| `USE_SYSTEM_HITPOINTS`, `SHIELD_DAMAGE_PROTECTION`, `cfgFACING_*` | damage model | T | stock |
| `CIRCLE_IN_PLACE_DURING_COMBAT`, `MAX_PREVENTION_ROTATION`, `NEBULAE_PATHING_COST`, `ASTEROID_BELT_PATHING_COST`, `cfg_WARP_INHIBITION_*` | pathing/combat | T | stock |
| `cfgFREIGHTERS_PER_MOON/NEBULA/PLANET` | mining freighter caps | T | stock |
| `SINGLE_PLAYER_OFFICER_LIMIT`, `unitcap_*`, `sr_*` tables | officer/supply caps per preset | T | Roots |
| `MIN_TACT_ZOOM`, `NEAR_CLIPPING_PLANE` (FO 20, Roots 35), `FAR_CLIPPING_PLANE` | view planes | T/R | FO, Roots |
| `cfgDEFAULT_GAMESPEED`, `GAMESPEED_SCALING` (default 1/3), `cfgBUILDTIME_TOOLTIP` (0/1/2), `LOAD_DELAY` (500 ms), `CINE_SOUND_DAMPENING` (0.34), `MAP_EDGE_BUFFER` | UX/pacing | T | Roots |
| `CRAFT_DEFAULT_MOVEMENT_AUTONOMY`, `CRAFT_DEFAULT_SPECIALWEAPON_AUTONOMY` | default autonomy for all craft | T | Roots |
| `GAME_OPTIONS_PHYSICS`, `GAME_OPTIONS_INFINITE_RESOURCES` | default game options | T | Roots |
| `cfgDOGFIGHT_RADIUS_MODIFIER`, `cfgCLOVERLEAF_RADIUS_MODIFIER` | AI attack pattern radii | T | Roots |
| `cfgTy2Lit_R/G/B`, `cfgCombiLit_R/G/B`, `USE_ROACOMBI_LIGHT`, `SHIELD_EFFECT_DAMAGE_COLORED`, `SHIELD_EFFECT_RESCALE` | ranged-affector light colours, shield effect | R | Roots |
| `AI_BASE_GRID_SIZE`, `PRODUCER_BUILD_INFO_WIREFRAME_SIZE` | AI/UI | T/U | Roots |
| `Input.map` (`rotate_station`, `rotate_station_left`, `select_next_ship`, `alt`) | hotkeys | U | Roots |
| `misc/gui_*.cfg` (`weaponIconColor`, `systemIcon*Color`, `officerIconColor`, aspect variants) | HUD | U | Roots |

### 3.4 Official A2 1.1 patch (baseline, in `Armada II 1.1 Patch Readme.txt`)

Features that other mods assume: left-click mode, repair-ship auto-repair at high autonomy, Ctrl-I interface
cycle, formation key F, right-drag free camera, **no tactical-camera stop point** (FO and Roots both note
re-fixing this), max strategic zoom 2200, scouts flee, build-time slider, global unit-cost slider, map-depth slider,
"Slower Physics" option, "Early Upgrade" techtree, save/load MP settings and games (`Ctrl+S`), AI repair /
mining / trading behaviours, bio-pulse damage sharing, many AI fixes. The "Early Upgrade" techtree and
slower-physics option are *already* data-selectable options in stock.

### 3.5 Roots 5.0Z11 engine changes (`devstuff/Engine_Changelog.txt`, 616 lines, 2026)

Grouped by category. Keys are ODF unless stated.

**New or extended ODF keys / classlabels (Q):**

- `probetorpedo`: `detectCloak = 1` reveals cloaked ships.
- `u_ball_obj` may carry `special = 1` weapons (cheaper than craft/pod).
- `areamissile` `hitcondition` 3/4/5 (allied weakened shield / shield+hull / hull damage).
- Station `freighterSize = 0` with `footprintAligned = 1`: centre on tile cross (2x2, 4x4 footprints).
- `shotdelay` now obeyed by `pointdefenselaser` and `ordnancedefenseweapon`; the latter intercepts "bullet" ordnance (artillery).
- AI attack patterns forced by `northbuffer = 1` (circle), `westbuffer = 1` (cloverleaf), `eastbuffer = 1` (fly-by).
- `southbuffer = 0`: fast death explosion (2 s not 4 s). `downbuffer = 0`: no hit explosions render.
- `collisionRadius = 2`: excluded from AI fleets/strategic goals (fighters).
- `distanceBelowGrid` (positive, equal) marks ODFs as variants of one unit for the AI build list.
- `hitChanceMod` 0-3 on weapons (stations not auto-hit; ranged affectors ignored); when also 3 on affectors, auto-shutdown after 10 s unattacked. For `temporal` ordnance, `hitChanceMod` is repurposed as an event id (38, 44, 45, 60...).
- `lodShift` on ordnance: damage multiplier vs shields-down targets; on `borgbore`/`holding_bm` it gates target invalidation by crew count.
- `buttonslot = 9999` skips a weapon when computing engagement range; `buttonslot = -1` passive dummy weapon with system icon colour.
- `rangefactor < 0` on type-1 ranged affectors affects hit chance; `avoidanceclass` 123/124/125 stops light reflection from type-2/type-1 affectors; `avoidanceclass = 151` lets ships fire while cloaked with a classic cloak; `avoidanceclass = 99` gives construction ships unlimited build range.
- Classic cloak: `unsafeCloakDistance = 0` (cloak without dropping shields), `stepSize = 0` (cloak on creation).
- `speed_boost` with negative `speedModifier` boosts build speed.
- `unitorpedo`/`unibeam` `affectedsystems = 99`: all affects simultaneously.
- `shieldScale` (visual shield radius only; `shieldPad` still affects physics). Invalid `shieldGeometryName` no longer crashes.
- `weaponsfire backwards`: `fireeventid = "emptyevent"`, `restrictfirearc = 1`, `firearc > 0`.
- `weldinganimspeed = 0.0` enables transport attack for normal transporter. `percentNebulaWarn = 0` hides faction icon at full queue. `percentNebulaWarn = 0.12345` raises team officer cap by `maxextraweapons` while built.
- `reflectweapon` with `reflectionRatio = 0`, `reflectMissile = 1`: shipyard repeats build queue (infinite build).
- `spriteduration = 0` on `minebm`: mined resources go straight to bank. `explosionUnitsPerLatinum = 0.0123` on a freighter: auto-seek resources.
- Race ODF: `crewaccumulationrate = 0` switches biomatter strings/icons; `usesXXXX = 0` hides that resource in tooltips.
- `crewhitpercent = 0` on evolvers selects `8472_cocoon2.sod`. `trackX`/`singleplayermusic` music lists play in random order.
- Latinum nebulas honour `lifeTimer`. `forceInvalidateTarget` also stops multi-target `cannonimp`.
- `cannonimp` can auto-target terrain objects when ordnance has `specialenergycost = -1`. `regeneration` weapons lose hardcoded >100 energy / under-attack requirement.
- `deathchant` `turnModifier = 99.0` hides overlay. Corbomite reflector, `replaceweapon` self-delete when `warpInSpeed = 0.0`, and `replacementInstantPlayer/AI` always deduct energy.

**New engine behaviours (F):**

- Playable factions in MP raised from 8 to **16** (limit).
- Rank skipping on large XP gains; promotion keeps formation, orders, rally point, wormhole order, recrew and mining.
- Station rotation 90/180/270 (`rotate_station`, `rotate_station_left` in `Input.map`).
- Alt+Number selects stations in numbered groups; multiple shipyards selectable together if `buildhardpoint` set.
- Movement autonomy rework: low/medium no longer assist allies; medium fleets converge on a manual target like high.
- Fire at targets of opportunity while travelling to an out-of-range target; construction ships and miners can fire while working; stations can use specials while building.
- Auto wormhole travel for cargo/mining ships; fighters follow carrier into wormholes; tractor through wormholes.
- Formation improvements: leader destroyed/replaced/split no longer stalls the fleet.
- AI: separate easy AIP `a2_easy_aggre.aip`; Hard AI switches to `a2_very_aggressive.aip` after 1800 s (was 2400); Normal switches to `a2_modified_aggressive.aip` at 1800 s (never before); cloaked fleets decloak on stop / under attack / at attack-move destination; resource moons chosen by distance; easy AI no longer explodes derelicts.
- Repair: shipyard restores special energy at end of repair; repair-queue entry distance +50; repeated priority-repair command no longer cancels.
- `select_next_ship` selects all damaged combat ships on screen (hull damage, shield < 60 %, or gravity mine).
- Team Colors graphics checkbox; mod loader with parent chain (`ParentMod`, `standalone`, `ignoreParentMaps`, `AssetVersion`).
- Settings/saves/logs in `settings_saves_logs/` beside the install (not AppData).
- Tooltips on other teams' objects; build-time tooltip (`cfgBUILDTIME_TOOLTIP`); shield bar tooltip shows strength; sensor hover circle full range (was 60 %).
- 4 GB address space (large-address-aware); cloak render performance work; Wine/Parallels compatible.

**Bug fixes (B):** unlogged ram-kill, evolver footprint leak, AI ignoring `ignoretechtree = 0` for replaceweapon and tech level of 5th techtree entry, Borg holding-beam leaving derelicts, tint leaks for Borg textures on bumpmapped models, alert status stuck yellow after cloak, AI stuck passive after wormhole, artillery XP, planet rendering, nebula rotation hardcoded off, `uxtheme.dll`/pointer crashes, FOH crash at address `...3010` during load bar.

**UI/render (U/R):** shell menus 800x600 -> 1200x900 (+50 %), fullscreen shell with dxwrapper, HUD per-aspect layout files, gui colour keys, dpi-safe fonts, cloak/shield/ECM colours, ReShade.

### 3.6 STA2 Classic mod (data, on Roots engine)

Enhanced Gameplay (all data: ODF/`RTS_CFG`-level) is a ready-made list of **rule presets**: shields absorb 90 % not 80 %
(bleedthrough 50 % lower), fewer crew casualties, subsystem repair x2 (stations x3), station/planet
shield regen x2, repair-ship range +25 %, default weapon autonomy medium, crew accumulation +33 %/+67 % on
class L/M planets, Psionic Insanity excluded from starbases/heroes, level-3 upgrade time 240 -> 180 s,
starbase sensor 800 -> 850, combat speed = impulse speed, AI permanently decloaks after losing builders,
start crew 1200 -> 1500, win condition excludes harvesters, "Early Upgrades" techtree, infinite build, transport
attack hotkey `Y`, allied shipyard repair. Both variants: station rotation, formation fixes, artillery
prefers stations, wormhole traffic, Alt+Number on stations.

## 4. FOH.map units as an index of what FO rewrote

(Delphi unit names in `FleetOpsHook.map`; the Roots map adds the bold ones.)

- **Objects and ODF loading:** `GameObjectEnhancement`, `GameObjectClassEnhancement`, `CraftEnhancement`, `CraftClassEnhancement`, `CraftInstanceEnhancement`, `CraftProcessEnhancement`, `OrdnanceEnhancement`, `OrdnanceClassEnhancement`, `WeaponEnhancement`, `WeaponClassEnhancement`, `ProducerEnhancement`, `ProducerClassEnhancement`, `ShipyardEnhancement`, `ShipyardClassEnhancement`, `ResearchPodEnhancement`, `ResearchStationEnhancement`, `RaceEnhancement`, `TeamEnhancement`, `**uCustomODFLoad**`, `FleetCap`, `FPQ`, `FPQHash`.
- **Weapon classlabels:** `ReplaceWeapon`, `ResourceWeapon`, `DetectorWeapon`, `RepairWeapon`, `CannonImp`, `RamWeapon`, `UniBeam`, `UniOrdnance`, `UniPulse`, `UniTorpedo`, `UtilityWeapon`, `OrdnanceDefenseWeapon`, `FedShieldEmitterEnhancement`, `AgentSaboteur`, `SleeperAgent`, `CloakDetector`, `TimedCloakDetection`, `TimedDisableImp`, `TimedExtraWeapon`, `DisableImp`, `SpecialWarp`, `RumbleExplosion`, `CloakingDeviceImp`, `CloakingDeviceControl`, `BoardingPartyImp`, `BoardingTeam` (FO only).
- **Game rules:** `TechnologyTreeEval`, `TechnologyTreeImp`, `GameConfigurationEnhancement`, `ModificationSupport`, `moduleConf`, `ProgramEnhancement`, `ScriptInterfaceImpEnhancement`, `GameEventEnhancement`, `ShipSystems`, `Fighter`, `FighterProcess`, `CarrierEnhancement`, `RepairShipEnhancement`, `ConstructionRigEnhancement`, `**ShieldHitEnhancement**`.
- **UI/HUD:** `cOverViewImpEnhancement`, `PopupPaletteEnhancement`, `ButtonListEnhancement`, `AvatarBox`, `AvatarIcon`, `BuildBox`, `MultiHealthBar`, `ShipDisplayEnhancement`, `ShipBuffIcon`, `ShipSystemIcon`, `SelectionDisplayEnhancement`, `MiniMapImp`, `RadarComponentEnhancement`, `ResourceComponentEnhancement`, `CommDisplayEnhancement`, `DebriefingDataHook`, `FontEnhanced`, `LoadBar`, `SmoothScroller`, `uOptionsMenu`, `uAdvancedOptionsWindow`, `uModSettings`, `uMapEdit*`.
- **Render:** `ST3D_DeviceDirectX8Enhancement`, `ST3D_GraphicsEngineEnhancement`, `ST3D_TextureEnhancement`, `ST3DPreview`, `Direct3D8Hook`, `D3D8t9`, `uRenderer`, `PlaceholderFeatureEnhancement`, `WireframeIconEnhancement`.
- **Platform plumbing (not STA64 relevant):** `FileSystemHooks`, `uNetworkManagerHook`, `Tunngle`, `fmod*`, `uallHook`/`uallDisasm` (runtime code patching), `madExcept`, `ScaleMM`, `Fastcode*`, `MiniLZO`, `SynLZO`.

## 5. Proposed quirk / tunable IDs

Naming follows MODDING-IDEAS: `<game>.<area>_<behaviour>`. Quirks default OFF for stock A2 / A1 and ON in the
`fo` / `roots` profile packs. Each needs a doc line and a fixture. "Engine reads" is the point the
canonical DB or sim consumes it.

### 5.1 Tunables (profile numbers, no behaviour)

| ID | Default stock A2 / FO / Roots | Source |
|---|---|---|
| `limit.mp_factions` | 8 / 8 / 16 | Roots engine |
| `limit.odf_count` | 900 safe / lifted / lifted | FO readme "more than 900 object definition files" |
| `limit.object_defs_memory` | 3 GB / 3 GB / 4 GB | Roots (LAA) |
| `limit.ship_display_icons` | 20 / 32 / 32 | FO 3.1 |
| `limit.event_race_soundsets` | 16 / 64 / 64 | FO 3.1 |
| `limit.build_queue_items` | stock / more / more | FO 3.2 |
| `limit.control_group_size` | 16 (30 for ships in FO/Roots notes) / same / same | stock + Refit `MaxSelection` 120 |
| `limit.warp_in_slots` | 9 / 10 / data | FO |
| `cfg.camera.*` (zoom min/max, scroll) | RTS_CFG.h | stock/FO/Roots |
| `cfg.game_speed_scaling`, `cfg.default_gamespeed` | hardcoded 5 / 5 / 3-5 via cfg | Roots |
| `cfg.ai.handicaps`, `cfg.ai.aip_switch_time` (2400 / 1800 s) | stock / FO / Roots | RTS_CFG.h + Roots |
| `cfg.ai.attack_pattern_radius.*` | Roots | RTS_CFG.h |
| `cfg.default_autonomy.movement`, `.special_weapon` | none / none / medium | Roots + Classic |
| `cfg.view.near_clip`, `.far_clip`, `.min_tact_zoom` | 1-20 / 20 / 35 | ART_CFG.h |
| `cfg.ui.shell_size` | 800x600 / 800x600 / 1200x900 | Roots |
| `cfg.damage.bleedthrough` | 0.20 (stock), 0.10 (Classic Enhanced) | `SHIELD_DAMAGE_PROTECTION` data |
| `cfg.pathing.*`, `cfg.freighters_per_resource` | RTS_CFG.h | stock |

### 5.2 ODF-semantics quirks (parser/DB level: the DB learns the key or its meaning)

| ID | Effect |
|---|---|
| `odf.fo_extended_keys` | accept the FO object/craft/race key set (3.2) into the canonical DB, with unknown-key preservation as per the pipeline |
| `odf.replace_weapon` | the `ReplaceWeapon` class and its `replacement*` keys (transformation/warp-in/promotion engine feature) |
| `odf.fleet_cap` | FleetCap files and `fleetCapFile` key |
| `odf.fpq_container` | read `ODF.fpq` (and AIP/formation inside) as a VFS layer |
| `odf.resource_weapon`, `odf.detector_weapon`, `odf.repair_weapon_costs` | FO weapon-class extensions |
| `odf.cloak_flags` | `disableShield`, `disableWeapons`, `phaseCloak`, `unsafeCloakDistance=0`, `stepSize=0`, `avoidanceclass=151` |
| `odf.hit_chance_mod` | Roots `hitChanceMod` 0-3 (and event-id overload for `temporal`) |
| `odf.lod_shift_damage` | Roots `lodShift` hull/shield damage multiplier; borg-beam crew gate |
| `odf.avoidanceclass_magic` | 99, 123-125, 151 sentinel values (document as quirk, not as general `avoidanceclass`) |
| `odf.ai_pattern_buffers` | `north/west/eastbuffer` AI attack patterns, `southbuffer`, `downbuffer` |
| `odf.distance_below_grid_variant` | AI treats equal positive `distanceBelowGrid` as same unit |
| `odf.officer_cap_bonus` | `percentNebulaWarn = 0.12345` + `maxextraweapons` raises team officer cap |
| `odf.minebm_direct_bank`, `odf.freighter_auto_mine` | `spriteduration = 0`, `explosionUnitsPerLatinum` |
| `odf.ranged_affector_flags` | `rangefactor<0`, `hitChanceMod=3` auto-shutdown, `rangedObjectAffector` autonomy |
| `odf.infinite_build_reflector` | corbomite `reflectionRatio=0` repeats queue |
| `odf.shield_scale` | `shieldScale` vs `shieldPad` |

(The sentinel-value keys are exactly the "painful" mod conventions; propose exposing each as a
named pack-level feature with a friendly key, and keep the sentinel as an import alias only.)

### 5.3 New engine features (native sim or pack behaviour; ordered by value)

| ID | Why it matters |
|---|---|
| `sim.autonomy_rework` | Roots movement autonomy (medium usable); the single biggest AI/UX change |
| `sim.targets_of_opportunity` | fire while travelling, miners/construction ships fire while working |
| `sim.formation_survival` | formation continues without its leader/after promotion or replacement |
| `sim.promotion_keeps_state` | orders, rally, recrew, mining, formation preserved across replace/promote |
| `sim.rank_skipping` | multi-rank promotion on big XP |
| `sim.station_rotation` | build rotated stations (hotkeys + footprint) |
| `sim.wormhole_traffic` | cargo/miners/fighters/tractor through wormholes |
| `sim.build_queue_policy` | pay-on-queue (Refit QOL-7) / infinite build, as selectable queue rule |
| `sim.ai_cloak_policy` | decloak on stop / under attack / at attack-move end |
| `sim.ai_aip_schedule` | easy/medium/hard AIP names and switch times as data |
| `sim.sensor_range_full_hover` and similar HUD facts | UI-level |
| `sim.special_energy_scalar` | special energy as a precise number scaling with system value (FO) |
| `sim.xp_ranks` | XP ladder with `worthXP`/`nextrankXP`/`nextrankClass` |
| `sim.fleet_caps` | FleetCap evaluation incl. AI |
| `sim.research_permanent` | research not lost with the station (FO) |
| `sim.strategic_range` | strategic range tier beyond artillery |
| `ui.mod_loader` | `info.ini` manifests with `ParentMod`/`standalone`/`ignoreParentMaps` (maps directly to `extends`) |

### 5.4 Bug-fix quirks (engine bug emulation: STA64 default = "fixed", stock profile = "buggy")

Stock A2 profile should be able to reproduce the bug when fidelity matters (replays, MP determinism);
FO/Roots profiles turn the fix on.

| ID | Bug |
|---|---|
| `a2.camera_no_tactical_stop` | 1.1 patch removed the tactical camera stop point; FO restored it (profile flip) |
| `a2.cloak_alert_yellow` | classic cloak sets yellow alert forever |
| `a2.ram_kill_unlogged` | ram destruction not in Admiral's log / no refund |
| `a2.holding_beam_derelict` | beam leaves target at zero crew when crew divisible by party size |
| `a2.ai_ignores_techlevel_5th_entry`, `a2.ai_ignores_ignoretechtree_replace` | AI tech handling |
| `a2.ai_passive_after_wormhole` | AI ships idle after exiting wormhole |
| `a2.ai_can_attack_cloaked`, `a2.cloaked_target_acquisition` | AI/cloak targeting leaks |
| `a2.weapons_hit_root_hardpoint` | weapons hit root hardpoint, not designated hit hardpoint |
| `a2.slowdown_near_target_value`, `a2.bump_into_target` | movement bugs fixed by FO |
| `a2.shield_upgrade_level2_noop` | L2 shield upgrade did nothing |
| `a2.burst_shot_cycler_reset_latency` | burst-fire timer reset under latency (MP) |
| `a2.object_defs_over_900` | desync/crash above 900 ODFs |
| `a2.evolver_footprint_leak`, `a2.nebula_rotation` | misc |

### 5.5 Out of scope (do not model)

Platform/network plumbing: DXVK/d3d8to9/DxWrapper/ReShade chains, Tunngle, Heroic/Wine overrides, FMOD
Ex swap, exception handler, memory managers, launcher/AV workarounds, CRC/checksum guards. Render
changes belong to `render-mods.md`.

## 6. Import tiers (for the compat matrix)

| Mod | Tier | Why |
|---|---|---|
| Pure data mods, STA2 Classic (Original), FO data | T0/T1 | ODFs + textures; FPQ unpacked into a layer |
| FO 3.2.7 (+ Dominion War) | T3 | needs `odf.fo_extended_keys`, `odf.replace_weapon`, `odf.fleet_cap`, XP and fleet-cap features, `fo` profile; identical code in DW, data differs |
| Roots 5.0 / FO 4.0 / Classic Enhanced | T3 on top of the above | needs `autonomy_rework`, formation/promotion features and the Roots quirks; submods become `extends` packs |
| Refit QOL | not a mod to load: native STA64 features (`limit.control_group_size`, warp-on-long-move, move queue) | |
| Refit QOLRules `PayOnQueue` | T2 flag `sim.build_queue_policy = "pay_on_queue"` | MP must-match, as Refit documents |
| `STA2WidescreenPatch.asi` | not needed (native aspect handling) | |

A importer could fingerprint a fork by `.map`/`info.ini` (`AssetVersion=50000` = Roots engine; `Version=3.2.7`
in `mshell.set` = FO 3.x; presence of `Armada2.ldl` = FO 3.x loader; `.test` section = Roots exe).

## 7. Open unknowns

1. **No disassembly done.** Behaviours in 3.2 to 3.5 are the developers' changelog claims plus Refit's
   measured diff counts. We have not verified exact formulas (e.g. how shield bleedthrough 80 % becomes 90 %,
   hit-chance maths with `hitChanceMod`, XP curve, what "normalized reload 5 s" does). Needs either bench
   measurement or reading the Armada2.map/ArmadaL symbols.
2. **FOH.dll source is closed** and Delphi; unit names tell where, not what. `FleetOpsHook.map` of Roots is 3.1 MB;
   a diff of the two maps (not done) would show exactly which units Roots added (found by name:
   `ModificationSupport`, `ProducerEnhancement`, `ShieldHitEnhancement`, `TechnologyTreeEval`,
   `uCustomODFLoad`, `SmoothScroller`).
3. **"FO 4.0" as the brief names it** is a Roots submod (engine identical to Roots). There is no standalone FO
   4.0 engine on disk; the FO 3.2.7 plain install is the only pre-Roots code.
4. **Dominion War** has no readme of its own; its code is FO 3.2.7. Which ODFs/techtree it changes is a data-diff
   task (2.4 GB vs 1.3 GB) not done here.
5. **Where stock A2 reads `RTS_CFG.h`** from: the stock folders have no loose `RTS_CFG.h` (it lives in the packed
   exe/ldl or Refit's list says `rts-cfg-check.py`). Stock default values in 5.1 are therefore unverified; Refit's
   `qol/rts-cfg-check.py` and `profiles/gog-1.1/*.toml` hold stock values.
6. **STA2WidescreenPatch.asi's exact patches** (FOV formula, UI behaviour) were not read; only its role.
7. **A1** has no code mods installed to catalogue. Whether A1 community exe patches exist is unknown from the
   disk alone; would need web research.
8. **ODF key semantic coverage:** the readmes list names, not defaults or ranges. A key survey over the actual FO/Roots
   `ODF.fpq` (unpack with FPQ reader, which STA64 needs anyway) would give an authoritative key list and value
   histogram for the profile schema.
9. **Overlap with Refit hooks:** Roots detours four functions Refit's QOL sits in (`mCheckGroupSelect`,
   `mProcessKeyboardInput`, `ComputerVoice::Say`, part of `cOverViewImp`); on Roots the equivalent behaviour is
   Roots' own version. STA64 must pick one canonical behaviour per quirk, not stack them.
10. **Multiplayer determinism:** FO/Roots check CRCs of FPQ, `RTS_CFG.h` and exe lists. STA64 MP compatibility
    (if ever) would need the same fingerprints, or a distinct protocol; out of scope here.

## 8. Evidence index

- `/home/cedric/Games/Heroic/Star Trek Armada II Fleet Operations/FOReadme.txt` (3,307 lines, FO 3.2.7 changelog)
- `/home/cedric/Games/Heroic/Star Trek Armada II Fleet Operations/FleetOpsHook.map`, `.../Data/RTS_CFG.h`, `ART_CFG.h`
- `/home/cedric/Games/Heroic/Star Trek Armada II Fleet Operations Roots/Data/devstuff/Engine_Changelog.txt` (616 lines)
- `/home/cedric/Games/Heroic/Star Trek Armada II Fleet Operations Roots/Roots_ReadMe.pdf`, `A2 Classic Submod ReadMe.pdf`
- `/home/cedric/Games/Heroic/Star Trek Armada II Fleet Operations Roots/Data/Mods/*/info.ini`
- `/home/cedric/Games/Heroic/Star Trek Armada II/` (Refit-installed `.asi` plugins and `.ini`)
- `/home/cedric/Games/Heroic/Star Trek Armada II (Stock)/Armada II 1.1 Patch Readme.txt`
- `/home/cedric/Projects/armada2-refit/PLAN-FLEETOPS.md` (Roots/FOH measured), `qol/README.md`, `rules/README.md`, `MODDING.md`, `layers.toml`, `platform/README.md`
