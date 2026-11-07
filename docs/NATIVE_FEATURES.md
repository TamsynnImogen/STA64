# Native feature parity

Fleet Operations is Armada II plus layers of runtime patches. STA64 builds
those layers in as native systems. Each row is a behaviour to reproduce
natively. It is not a hook to port.

Sources:
- **FOH**: Fleet Operations 4 `FleetOpsHook.dll`. Reference analysis:
  `~/FleetOpsHook-Reimplementation`.
- **A2X**: A2FOExtensions. Source: `~/A2FOHookExtensions`, `core/builtin_*.cpp`.
- **Refit**: Armada II Refit (cedrichaase/armada2-refit, MIT). Assessment:
  `~/armada2-refit-assessment/REPORT.md`.

Status: `todo` / `wip` / `done`. Profile shows which rule sets enable the
feature by default. ODF/JSON5 keys switch features per unit, so they are not
hard-coded.

## Platform and loading

| Feature | Source | STA64 system | Profile | Status |
|---|---|---|---|---|
| Mod chain (`info.ini` `[Mod]`, ParentMod, standalone) | FOH | `sta_vfs` | all | wip |
| 28 virtual directories, AppData/install/FPQ precedence | FOH | `sta_vfs` | all | todo |
| FPQ v2 archives (raw + LZO, 3 filename hashes) | FOH | `sta_fpq` | all | done |
| Recursive ODF folders | A2X `odf_variants`, `info_ini` | `sta_vfs` ODF index | all | todo |
| ODF variants | A2X `odf_variants` | content DB | all | todo |
| Large memory and object limits | FOH (RAM patches) | native 64-bit, no fixed pools | all | by design |
| Widescreen, any resolution, HUD scaling | Refit `HUD`/`UI`/`Menus`, STA2WidescreenPatch | Bevy UI, resolution-independent | all | todo |
| Startup modules, startup policy | A2X `startup_modules`, `startup_policy` | plugin registry + profile config | all | todo |
| Log mailer, radar diagnostics | A2X | `tracing` + debug overlay | dev | todo |

## Rendering

| Feature | Source | STA64 system | Status |
|---|---|---|---|
| SOD loading, node hierarchy, animation timing | stock, Refit `a2fmt` | `a2fmt` + `sod_scene` → Bevy | wip |
| Team colours, Borg `_b` textures, animated `_1.._9` | A2X `texture_variants`, `rgb_textures`, Refit | material system | todo |
| DOT3 bump maps | A2X `amd_dot3_compat` | PBR normal maps | todo |
| Texture variants and legacy texture paths | A2X `texture_variants`, A1 legacy textures | `sta_tex` | todo |
| Nebula renderer | A2X `nebula_renderer` | volumetric nebula pass | todo |
| Muzzle flashes, animated hardpoints | A2X | VFX + hardpoint animation | todo |
| Shell display | A2X `shell_display` | shield-hit VFX | todo |
| View distance | A2X `view_distance` | camera far plane, LOD | todo |
| Sprite/emitter interpretation | Refit `render3d` | `sta_spr` + particles | todo |
| Selection, fog of war rendering | Refit `render3d` | render passes | todo |
| Modern post FX (opt-in) | Refit `postfx` | Bevy post-processing | todo |

## UI and controls

| Feature | Source | STA64 system | Status |
|---|---|---|---|
| **Front end = Refit's 3D menu scene**: a live map behind every menu (main view, per-campaign views with camera blends, Klingon group decloaking on its view, warp departure into the loading screen) | Refit `render3d/src/menu`, `menus/map.ini`, `ui/src/menus` | Bevy menu scene + UI panels over it; scene recipe in data | todo |
| Refit screen layouts (main, single player, campaign/mission select, game setup, lobby, settings, saves, admiral's log, tech tree) and LCARS glass panel style | Refit `ui/src/menus/screens`, `ui/STYLE.md` | Bevy UI, per-race themes | todo |
| Loading screen: map's own skybox backdrop, briefing, objectives, players, progress | Refit `ui/LOADING.md` | loading state | todo |
| cfg/spr-driven HUD, three info-panel heights | FO, Refit `ui` | data-driven UI | todo |
| Build tooltips | A2X `build_tooltips` | UI | todo |
| Command grid layout | Refit `grid` | UI | todo |
| Camera controls | A2X `camera_controls`, Refit `qol` | RTS camera | todo |
| Selection groups, command generation QoL | Refit `QOL` | input layer | todo |
| Pay-on-queue rule | Refit `QOLRules` | sim rule flag | todo |
| Edit menu, mission selector, Instant Action settings | A2X | front-end | todo |
| Refit menus and yards UI | A2X `refit_menus`, `refit_yards` | UI + sim | todo |
| Language packs (`Dynamic_Localized_Strings.h`, `label.map`) | FO | localisation | todo |
| Video (BIK intro/cutscenes) | A2X `video` | FFmpeg-backed player | todo |

## Simulation

| Feature | Source | STA64 system | Status |
|---|---|---|---|
| Weapons, damage controls, normal weapon tech | FOH, A2X `weapon_damage_controls`, `normal_weapon_tech` | combat | todo |
| Point-defence cycles (numbered firing cycles) | FOH, A2X `point_defense_cycles` | combat | todo |
| Fire arcs | A2X `fire_arcs` | combat | todo |
| Directional shields (fore/aft, port/starboard) | A2X `directional_shields`, `always_shields` | defence | todo |
| Energy systems | A2X `energy_systems` | ship systems | todo |
| Turrets, linked hull turrets | A2X `turrets` | hardpoints | todo |
| Team-change weapons | A2X | combat | todo |
| Resources (data-driven, incl. biomatter, latinum, metal) | FO, A2X `resources` | economy | todo |
| Officer capacity per profile | FO, A1 | economy | todo |
| Multi-yard builds, build queues | A2X `multi_yard_build`, feature pack queue | production | todo |
| Construction rig: deploy → weld → reveal | FO | production | todo |
| Upgrade pods (to level 4), cocoon `EvolverClass` | A2X feature pack | production | todo |
| Hybrid research station / yard | A2X hybrid build/runtime | production | todo |
| Breeders, swarm system, infestation | A2X | species mechanics | todo |
| Collective XP | A2X `collective_xp` | veterancy | todo |
| Squadrons | A2X `squadrons` | unit groups | todo |
| Station rotation | A2X `station_rotation` | movement | todo |
| Wreckage | A2X `wreckage` | world | todo |
| Craft identity | A2X `craft_identity` | units | todo |
| Cheats | A2X `cheats` | dev/sandbox commands | todo |
| A1 compatibility (classlabels, health model, fallbacks) | A2X `a1_compat`, `a1_fallbacks` | STA1 Classic profile | todo |

## Content and scripting

| Feature | Source | STA64 system | Status |
|---|---|---|---|
| FO Lua mission scripting (MMM, `mmm_loader.dll`) | FO, A2X `missions` | integrated MMM Lua runtime, see [MISSIONS.md](MISSIONS.md) | todo |
| Stock DLL missions (`*R.drl` / `*S.dsl`) | stock | not loadable natively; Lua conversion or silent fallback | design |
| SOL maps | A2X `sol_maps` | map loader | todo |
| BZN maps | stock | `sta_bzn` | todo |
| M3D (Star Trek: Legacy) models; child-mod M3D overrides parent SOD | A2X `m3d` (`modules/A2FOM3D`) | `sta_assets` M3D reader | todo |
| Scene command vocabulary, event marks | Refit `Scene.asi` | headless scenario tests | todo |

## Multiplayer

| Feature | Source | STA64 system | Status |
|---|---|---|---|
| Internet play without DirectPlay | Refit `Online` | native lockstep relay (new protocol) | todo |

Not a goal: wire compatibility with stock Armada II or FO multiplayer.
