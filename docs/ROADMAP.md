# Roadmap

## M0: Foundation (current)
- [x] Workspace, pinned toolchain, CI matrix (Linux/Windows/macOS arm64/Linux arm64)
- [x] `sta_vfs`: Data + ParentMod mod chain, case-insensitive lookup
- [x] `sta_odf`: parser, `#include`, typed accessors
- [x] `sta_sim`: fixed-point maths, deterministic tick + digest
- [x] `sta64`: Bevy window, mounts the install and reports what it found

## M1: See the content
- [x] `sta_fpq` (FPQ v2 archives) mounted in the VFS — byte-identical to liblzo2 on Sigma's 2,872-entry `ODF.fpq`
- ODF index across all ODF folders and A1 `addon/`, with duplicate warnings
- [x] `sta_assets`: layer-aware lookup (KTX2 > DDS > TGA > PNG; GLB > glTF > SOD), TGA decode
- [x] SOD → Bevy scene (vendored `a2fmt`), finest LOD, step animation; 926/927 FO4 SODs parse
- [x] Model viewer: browse every model, orbit, team colours
- Borg `_b` / animated `_1.._9` texture variants, named TGA mip chains, height → normal maps
- Sprite nodes, emitters, texture flipbook animation refs (`.spr`)
- M3D models (port A2FOM3D's conversion)
- glTF/GLB models in the viewer

## M2: A map you can fly in
- BZN map loading (starfield, nebulae, asteroid fields, planets)
- RTS camera, selection, move orders through `sta_sim`
- Renderer interpolation between sim ticks

## M3: Combat and economy
- Weapons/damage/shields from ODF, fire arcs, point defence
- Resources, mining, construction rigs, build queues
- FO4 profile running a skirmish against an idle opponent

## M4: Front-end and UI
- Front end built on Refit's 3D menu scene: a live map behind the menus,
  named camera views per screen, decloak and warp-departure moments (recipe
  as data, adapted from Refit's MIT `menus/map.ini`)
- Refit-style screens and LCARS glass theme; loading screen over the map's skybox
- cfg/spr-driven HUD, language packs
- Mod loader, Instant Action settings

## M5: AI, Lua, multiplayer
- FO Lua mission API
- Skirmish AI
- Lockstep netcode with desync detection
