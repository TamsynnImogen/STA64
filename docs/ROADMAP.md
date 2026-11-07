# Roadmap

## M0: Foundation (current)
- [x] Workspace, pinned toolchain, CI matrix (Linux/Windows/macOS arm64/Linux arm64)
- [x] `sta_vfs`: Data + ParentMod mod chain, case-insensitive lookup
- [x] `sta_odf`: parser, `#include`, typed accessors
- [x] `sta_sim`: fixed-point maths, deterministic tick + digest
- [x] `sta64`: Bevy window, mounts the install and reports what it found

## M1: See the content
- `sta_fpq` (FPQ v2 archives) mounted in the VFS
- ODF index across all ODF folders and A1 `addon/`, with duplicate warnings
- `sta_tex`: TGA/DDS (+ Borg/animated variants), KTX2
- `sta_sod`: SOD → Bevy mesh/scene, node animation
- Model viewer scene: orbit a selected ship with team colours

## M2: A map you can fly in
- BZN map loading (starfield, nebulae, asteroid fields, planets)
- RTS camera, selection, move orders through `sta_sim`
- Renderer interpolation between sim ticks

## M3: Combat and economy
- Weapons/damage/shields from ODF, fire arcs, point defence
- Resources, mining, construction rigs, build queues
- FO4 profile running a skirmish against an idle opponent

## M4: Front-end and UI
- cfg/spr-driven shell menus and HUD, language packs
- Mod loader, Instant Action settings

## M5: AI, Lua, multiplayer
- FO Lua mission API
- Skirmish AI
- Lockstep netcode with desync detection
