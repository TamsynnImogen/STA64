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
- [x] Borg `_b` texture variants, hand-authored TGA mip chains (`_1.._n`), full mips everywhere
- [x] SOD 1.93 bump path: height map → normal map (`ST3D_CreateNormalMap` rule)
- [x] CPU DDS decoder (DXT1/3/5 + uncompressed); all 604 FO4 DDS decode
- [x] Sprite nodes (running lights, strobes, glows) and SOD texture flipbooks from `.spr`
- [x] Particle emitters (`@emitter`: spray and cloud)
- [x] M3D models loaded natively (A2FOM3D's parser; diffuse, bump, specular, glow kept)
- Star Trek: Legacy LOD nodes (type 11) and effect nodes (type 2); M3D 1.93 layout
- glTF/GLB models in the viewer

## M2: A map you can fly in
- [x] `sta_bzn`: text and binary BZN (typed-record stream); all 517 installed maps
  parse, binary object counts match the declared counts exactly
- [x] Map view: every object's model placed by its BZN transform, `ScaleSOD`,
  team colours, skybox (`BackgroundArt`), shared parts across copies
- Starfield (`StarfieldOdf`), asteroid fields (`dont_load_SOD` fields), planet
  bodies from `groundTextureName`, nebula volumes, map lights (`dlight`)
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
