# Render mods: Refit and FO Roots, mapped to effect slots

Research, read-only, 2026-11-11. Sources: `/home/cedric/Projects/armada2-refit`
(`render3d/README.md`, `MODDING.md`, `PLAN-FLEETOPS.md`, `layers.toml`, `render3d/content/`,
`render3d/mods/*/shaders`) and the Roots install
`/home/cedric/Games/Heroic/Star Trek Armada II Fleet Operations Roots/` (plus FO plain,
A2 stock beside it). Nothing was executed; binaries were only `strings`-ed, text and
ini files read. Where a claim comes from Refit's own notes about Roots rather than from my
inspection, it says "per PLAN-FLEETOPS".

## 0. Findings in ten lines

1. Refit is not a shader-override mod. It is an `.asi` that **replaces the engine's render
   steps** with its own Rust renderer; the HLSL is compiled into the DLL (SPIR-V + DXIL).
   Players cannot swap a shader. They change **ini keys and ini "recipes"**.
2. The data surface Refit exposes is already a pack-like layer: `content/sets/<set>/*.ini`
   (names: SODs, sprites, faction keys) plus `content/{sky,surfaces,nebulae,ordnance,menu}/*.ini`
   (parametric recipes), overridable by same-named files in the game folder.
3. Refit's effects are **procedural and parameterised** (ray-marched fireball, Fresnel
   shield mist, volumetric gas, weld arcs). A recipe is parameters for a fixed shader
   *kind*; the 5 "kinds" of ordnance (beam, bolt, field, ...) are the nearest thing to
   a shader-slot contract.
4. Many effects need **engine state per instance** (hit position on a shield shell, hull
   depth, shadow map, scene copy for cloak refraction, light list). That is the real
   interface of a slot, not just "a shader".
5. **FO Roots does not override shaders of effects at all.** Its visual deltas are:
   (a) data (SODs, textures, ODF keys, `ART_CFG.h`/`RTS_CFG.h` tunables), (b) a patched
   engine (`FleetOpsHook.dll` + exe edits adding new ODF keys and render behaviour), and
   (c) a **screen-space post chain**: ReShade (Deband, Pirate_Bloom, LumaSharpen, selective
   colour, gamma) via DxWrapper d3d8->d3d9. The "effects more striking" in its readme is (c).
6. Roots' new render-relevant behaviour is mostly **ODF-selected asset choice** (per-class
   shield hit SODs, `overlaySod`, `explosionSprite`, `_b` Borg textures, `_bump` DDS)
   and numeric knobs, i.e. data, but data with **new key semantics** that live in the
   profile/rules layer, not in the render layer.
7. Roots' one real code-defined look change is the bump-mapped mesh path (it draws every
   Dot3 mesh itself with its own vertex/pixel shader programs, `.nvv`) and the cloak
   alpha path (`CloakingDeviceImp`). These cannot be expressed as data.
8. The pack/slot abstraction **holds for the parametric half** (nebula, sky, surfaces,
   shield colours, ordnance recipes, post) and for Roots' asset/ODF half. It **breaks**
   for "swap this shader" as an unqualified idea: a slot must declare its input contract
   (uniform block + textures + vertex format), and a shader pack is only valid against
   one contract version.
9. Slot *selection* is not per-slot-global. Refit selects by game object identity:
   ordnance ODF name, nebula ODF name, texture name, race key. Slots need a **binding
   key** (which ODF/class/sprite) as well as a name.
10. Compat for Roots' ReShade is straightforward as a `post` chain slot with a
    declared stage; the .fx files are ReShade FX (HLSL-like), not WGSL, so they are
    T3 (reimplement the 5 shipped effects natively), not T0/T1.

## 1. Refit mechanism, per module

### 1.1 The substrate (what a "hook" is here)

- `Render3D.asi` re-points the engine's two `Armada_RenderAllOurStuff` calls and runs
  the body as its own step sequence (`frame/steps.rs`): background, terrain, objects,
  ordnance, specials, features, warp, hud prep, chunks, particles, translucent, post.
  The scene is drawn into a half-float MSAA target, bloomed and tone-mapped into the
  back buffer (`post.hlsl`).
- Modules (`world 10, particles 20, hulls 30, effects 40, ordnance 50, specials 60, sky 70,
  overlays 80`) implement a `Module` trait; the first module whose `step/object/feature`
  returns true draws that part, otherwise the engine does ("passthrough"). Each module
  can be switched off live (`[Modules] hulls=0`), falling back to the engine's draw.
- Engine patching is address-profiled per exe build: `Hooks::install(Hook{site, expect,
  chain})` with the site *checked against what stock has there* before writing;
  `profiles/<build>/render3d.<module>.toml` holds the addresses. Not portable to STA64
  (we own the engine) but it is the reason Refit can only expose what the engine
  structures expose.
- Claims: each plugin tells Display8 which categories it draws (sky, hulls, world,
  particles, effects, ordnance, specials, fog, selection...) so two plugins do not
  double-draw. A slot override in STA64 needs the same "exactly one owner per slot"
  rule; the claims table is a ready-made precedent.
- Shaders: HLSL per crate, constants at fixed registers (`CREG(n)`), samplers at fixed
  stages (`STAGE(n)`), loops fixed-count, no branches (kept from a Direct3D 9 history).
  So the de-facto shader ABI is "register file + stage numbers", documented only in
  comments beside each shader.
- Config: `Render3D.ini` sections per module; a descriptor table (`A2SET_ABI 1.1`, rows
  with kind/apply class/scope/label/help) exported by the plugin feeds a settings UI;
  overlay file `RefitPlayer.ini` layers over the ini (`a2ini` overlay).
- Content sets: `profiles/<build>/content.toml` -> `[set] render3d = "fleetops"` picks
  a folder of small ini tables. A new game/mod's names are data; "a new set is a copy of
  `sets/stock/`, no code change".
- Recipes in the game folder win over built-in ones: `Sky\<name>.ini`, `Surfaces\<name>.ini`,
  `Nebulae\<name>.ini`, `Menu\map.ini`. Missing recipe = engine behaviour for that object.
  Hot reload for recipes (`[World] Reload=1`, read again each second) and for several keys.

### 1.2 Module by module

| Module | Engine step replaced | What the look is | How it is selected / parameterised |
|---|---|---|---|
| `hulls` | `GameObject::Render` / `ST3D_Instance::RenderInternal` | Clustered-light hull shading, specular, rim, bump from height map, shadows (shadow map from Key light), team colours, Borg profile, scrap glow, **cloak** (glass over a scene copy: refraction with colour split, rim, shimmer, travelling wave bands + vertex sway, front line), warp stretch | `[Hulls]` ~40 keys (`Ambient, Specular, SpecularPower, RimLight, Shadows, BumpStrength, Cloak*`...); content set `hulls.ini` (`ProfileBy = race`, `ProfileRaces = borg,freeborg`, SOD node names, resource colours). Shaders `hulls.hlsl` (`ps_hull_cl`, `ps_bump_cl`, `ps_cloak`, `vs_hull`) |
| `effects` | `ExplosionFeature::Render`, `ShieldEffect::Render`, `BuildingEffect::Render`, `NodeParticleEffect::Render`, `BeeColony::Render`, `ST3D_Instance::Fragment` (debris) | Ray-marched fireball + shockwave band (HDR core, per-explosion point light); shield hit = lit mist over a sphere shell (4-octave gradient noise on unit sphere, Gaussian falloff from hit, Klingon rays, per-hit hull light); weld arcs and cooling seams for repair and construction; sparks; bees/drones; own debris chunks | `[Explosions] [Shields] [Repair] [Debris] [Construction] [Bees] [Sparks] [Shell.<name>]`. `effects.ini` content set: `[Factions]` key -> RGB, `[Races]` race -> faction key, `Sites` (which weapons' CreateShieldHit are tapped), cocoon SOD name |
| `ordnance` | `Ordnance::RenderAll`, `RenderWarpSpriteEffects` | Per-shot cascade: (1) a **recipe** by ordnance ODF name (`kind=beam/bolt/field/...`: Colour, CoreColour, Width, Taper, Flicker, FadeIn, Light, LightReach), (2) stock's sprite look re-drawn with HDR "hot texels" lift (`Hot`, `HotFrom`), (3) engine's own `Submit`. Warp flash replaces stock `warp_sprite` (core, star, streak, ring, afterglow, light) | `content/ordnance/ordnance.ini` built in, overridden by `[Ordnance.<odf>]` in `Render3D.ini`; `ordnance.ini` set table `[Names] Ordnance.<odf> = <recipe>` maps *Roots ODF names onto stock recipes* |
| `specials` | `SpecialObjectEffect::RenderAll` | Overlay on hull + light for Computer Override, Energy Dissipator, Psionic Disruption; cloud emitters of the effect SOD fed to particles | `[Specials]`; `specials.ini` kinds |
| `particles` | `ST3D_DeviceDirectX8::RenderParticle` etc. | Own particle simulation on sim clock for every SOD emitter node (damage vents, drone steam, gates, chain pulsar, Manheim, nebula puffs), spray/cloud modes, `alpha` vs additive material | `[Particles]`; `particles.ini` names vent sprites; materials come from stock sprite tables |
| `sky` | background step | Parametric skybox recipe fitted to the stock skybox by measurement (`sky-shape.py`, `sky-grade.py`): fbm gas, cores, stretch, grade, dither; starfield, sun, background planets, warp tunnel | `content/sky/<prefix>.ini` (`Seed, Scale, Warp, Detail, Coverage, GasA/B, Glow, CoreN, Stretch, GradeGain, GradeCurve...`), mapped by the stock skybox prefix; game folder `Sky\<name>.ini` wins |
| `world` | planets, moons, rocks, nebulae | Planet/moon/rock surface shaders from a surface recipe; **volumetric nebula**: view-perpendicular slices, additive, depth-tested vs the scene, density = envelope texture (baked top-down from nebula positions) x vertical profile x domain-warped 3D noise; lit by the Key with ship shadows (shadow map lent by hulls); pulses on sim time | `content/nebulae/<odf>.ini` (`Seed, Tile, Warp, Coverage, Softness, Gamma, GasA/B, Glow, Extent, Height, Lumps, Knots, Lanes, Light, Shading, Filaments, Flow, FlowSpeed, EdgeOnDim, Opacity, NearFade, Fringe, Fuzz`), `content/surfaces/<texture>.ini`; `world.ini` maps ODF class -> recipe + glow hue |
| `overlays` | fog/shroud, grid, selection bubbles, ghosts, move paths | Own drawing; Roots grid colour, "EngineMoves" | `[Overlays]`; `overlays.ini` |
| core post | frame end | HDR resolve, bloom by energy (`Bloom, BloomThreshold, BloomKnee`), `Exposure`, tone-map `Shoulder`, MSAA | `[Render3D]` |
| menu scene | front end | Recipe `menu/map.ini`: groups of ships, nebula fields, warp departure, cloak/decloak choreography, camera | `Menu\map.ini` |

### 1.3 What the knobs look like

- Almost every look parameter is a **scalar/colour/vector in an ini key** with a stated
  default and unit in the README (colours 0..255, distances in bounding radii or world
  units, times in seconds, `...MinPixels` clamps).
- Colour inputs by **faction key** (shields) or by recipe (nebula glow).
- Hot-reloadable: recipe files, `[World] Reload`, `[Hulls] Reload`, several rows live.
- Settings are *described* (descriptor table: kind bool/enum, apply class
  live/next-mission/restart, scope local/match, advanced flag): the UI is generated.
  This is directly reusable for STA64's slot params UI.

### 1.4 Specific limits relevant to the abstraction

- **Shader code is not data.** No runtime shader loading; changing the look beyond the
  keys means editing HLSL and rebuilding the DLL. MODDING.md says so ("Shaders" section).
- **Recipes are kind-bound.** A nebula recipe is only a parameter set for the one gas
  shader; there is no "write a new nebula algorithm" path.
- **Selection is by game identity**, not by slot name: recipe file name = stock skybox
  prefix / texture name / nebula ODF; ordnance by ODF name; shield colour by race.
- **Menu and mission share modules** but `in_menu()` keeps some modules alive when off.
- **Slots are coupled by services**: effects asks hulls to draw construction and bees;
  world borrows hulls' shadow map; ordnance feeds particles; module-to-module calls go
  through `r3d_core::services`. A pack replacing `hulls` silently breaks `construction`
  unless the contract says so ("construction and the bees need the hulls module").
- **Determinism**: the cosmetic sim runs on the sim clock, never feeds back (one noted
  exception, a write the engine itself makes in its render pass). STA64's
  render-must-not-affect-sim rule matches.

## 2. FO Roots: what it changed and how

### 2.1 What Roots ships (from the install)

Roots `5.0Z11` (Roots_Download_Version.txt): a Fleet Ops total conversion with a modified
engine, its mod loader (`Mods/Fleet Ops 4.0`, `STA2 Classic Mod`, `Classic Enhanced`),
and a graphics stack:

| Component | File(s) | Kind |
|---|---|---|
| Engine patches | `FleetOps.exe`, `FleetOpsHook.dll` (+`.map`) | code; adds ODF keys, per-object draw wrapper, Dot3 mesh draw, cloak path, shield hit model pick |
| Direct3D 8 -> 9 | `dxwrapper.dll` + `dxwrapper.ini`, `d3d9.dll` | wrapper (the installed `d3d9.dll` hashes as DXVK in this machine's Refit chain; Roots' own ships a ReShade `d3d9_reshade.dll`, 4.1 MB, and `d3d9_no_reshade.dll`) |
| **Post chain** | `ReShade.ini`, `FleetOpsRootsReshadePresets.ini`, `reshade-shaders/` | screen-space only |
| Tunables | `Data/ART_CFG.h`, `RTS_CFG.h`, `game_cfg.h`, `misc/gui_*.cfg` | data |
| Art | `sod/` (625 SODs; FO has 794), `Textures/` (548 DDS + 754 TGA; FO plain has ~2350 TGA), `Sprites/*.spr`, `Shaders/dot3_*.nvv` | data |
| Game rules | `ODF.fpq` (4 MB; FO 32 MB), `techtree`, `AI` | data (packed, not opened here) |
| Refit-owned files in the same folder | `Render3D.asi`, `Render3D.ini`, `UI.asi`, `QOL.asi`, `Nebulae/`, `Surfaces/`, `winmm.dll` | **not Roots**: Refit was installed on top of this copy (`Render3D.ini` dated 2026-11-06). The `Nebulae/*.ini` and `Surfaces/*.ini` there are Refit's recipes. Do not read them as Roots' |

### 2.2 The post chain (the "effects more striking" part)

Roots readme: "ReShade shader/filter technology, complete with presets optimized for Fleet
Ops ... slightly improve image quality and make effects and engine glows more striking".
The preset in `FleetOpsRootsReshadePresets.ini`:

```
Techniques = Linear_Gamma, Linear_Gamma_2, LumaBlur, prod80_04_SelectiveColor_v2,
             LumaSharpen, Deband, Pirate_Bloom
```

| Effect | Knobs in the preset | What it stands in for |
|---|---|---|
| Linear gamma (x2, types 7 and 8) | `Gamma_type`, `Gamma_type_2` | brighten; gamma-correct the following passes |
| LumaBlur | `sharp_strength=-0.4`, `sharp_clamp`, `pattern`, `offset_bias` | a soft blur (negative sharpen) |
| Selective colour (PD80) | per-hue `*_adj_*` sliders (lightness of blacks/greys/etc.) | grade |
| LumaSharpen | `sharp_strength=0.6` | sharpen |
| Deband | `iterations=2`, `range`, `threshold_preset`, Weber/sdev flags | removes banding in space gradients, shield hits, engine glows (readme: "space backgrounds, shield impacts and engine glows look more smooth") |
| Pirate_Bloom | `BLOOM_STRENGTH=0.55`, `BLOOM_RADIUS=2.5`, `BLOOM_THRESHOLD=0.65` | bloom, i.e. the "striking glows" |

`ReShade.ini`: search paths, `PerformanceMode=1`, depth-buffer addon disabled,
`KeyOverlay=45` (Insert). Order is the technique order; all passes are full-screen and read
only the 8-bit back buffer. No depth, no per-object information. This is entirely
a **post slot** with a list of passes and per-pass float params; Refit's own `post`
(HDR bloom by energy) replaces it and is why bloom there is physically based.

Mechanism summary: **injected after the engine, outside the game's own render code,
via the D3D wrapper chain.** Removing is renaming `d3d9.dll` (documented in the readme).

### 2.3 Engine-side render behaviour (FleetOpsHook), known partly via Refit's analysis

`FleetOpsHook.dll` strings (`strings`, no execution) show these render-relevant ODF/keys:
`overlaySod, renderIgnoreCameraDistance, selectHighlight, explosionChunks, explosionSprite,
explosionSizeFactor, explosionDuration, explosionHit, explosionShieldHit, shieldHit,
shieldHitCritical, shieldDown, ShieldAdd, ShieldSub, shield_hit/shield_down/shield_add/shield_sub,
logoFileNames, bump_mapping, BumpMappingCheck, SpecialWarpEffect, CloakingDeviceImp`,
`dot3_directional9.nvv` and `Textures\env_*0.dds` lookups. PLAN-FLEETOPS (Refit's code reading,
cited, not re-verified by me) establishes:

| Hook | Behaviour | Data or code |
|---|---|---|
| object loop (0x598297) | draws via object's own `Render` unless culled, or ODF `renderIgnoreCameraDistance` | code + 1 ODF key (unused in 5.0Z11) |
| `GameObject::Render` | `selectHighlight`: ambient raised to team colour while selected/hovered | code + ODF key (unused) |
| `GameObject::RenderInternal` | cloak (`CloakingDeviceImp`) alpha path; wave-light detours | code |
| `CraftInstance::RenderInternal` | per-craft node shown with per-ship texture or hidden | code + data |
| **`ST3D_Dot3_MeshVB::Render`** | **replaces the Dot3 bump mesh draw**: per-light passes, base texture, glow; Roots' `.nvv` vertex shaders; no reflection pass | code + shader programs |
| `Craft::RenderInternal` (post-draw) | `overlaySod` second instance (sensor arrays etc.), weapon range rings | code + ODF key |
| `Craft::Explode` | `explosionChunks` toggle | code + key |
| **`CreateShieldHit` model pick** | per-class from ODF: `shieldHit, shieldHitCritical, shieldDown, ShieldAdd, ShieldSub, ShieldSpecial1..4`; sized sets (`shield_fed_hitL`...) | **data selection, code pick** |
| `ShieldHit::DrawShield` | `SHIELD_EFFECT_RESCALE` (x2 in Roots), `SHIELD_EFFECT_DAMAGE_COLORED` (0) from `RTS_CFG.h` | code + 2 cfg keys |
| `ART_CFG.h` | adds `A2FO_SPECULAR_SUFFIX "_spec"`, `cfgTy2Lit_R/G/B` (ECM hull-light colour), `USE_ROACOMBI_LIGHT` + `cfgCombiLit_*` (ECM+fleet-coord colour), `NEAR/FAR_CLIPPING_PLANE`, starfield counts, chunk manager counts/lifetime/velocity | data |
| Engine changelog (`devstuff/Engine_Changelog.txt`) | `downbuffer = 0` hides ordnance explosions on tiny hulls; `shieldScale` scales shield bubble without changing collision; invalid `shieldGeometryName` falls back to "shield down"; planets rendered "smoother spheres" in Classic submod; Borgified textures (`_b` suffix) usable by any craft; team colours checkbox; latinum nebulas obey `lifeTimer`; nebula objects hardcoded not to rotate; render of cloaked ships cheaper | each = **an engine behaviour switched by an ODF key** |

### 2.4 What Roots did to *effects* specifically, by effect

| Effect | Roots mechanism | Compare FO plain / A2 stock |
|---|---|---|
| Shield hit/down/add/sub | Per-class **SOD meshes**, sized (S/M/L/XL/XS, "crit", Borg/Fed/Dom variants), picked by ODF keys; x2 rescale; no damage colouring; Borg/8472 greenish tint (readme) | A2 stock: one global `shield_hit` family. FO plain: 59 `shield_*` SODs (per race). Roots: 29 (trimmed, re-sized). Pure data + the ODF-key pick the hook adds |
| Weapon sprites, explosions | Texture/SOD replacement; `explosionSprite` ODF; FO "explosion effects ... higher quality textures, optimized not to reduce performance"; chunk counts reduced in `ART_CFG.h` (`MAX_CHUNKS` 9000 -> 2000, lifetime 4 -> 2.3, max velocity 90 -> 15) | Data only |
| Engine glow, hull glow | `*_glow.dds` textures; ReShade bloom/debanding makes them "striking" | Textures + post |
| Cloak | Engine/ODF: `CloakingDevice` (classic) vs `CloakingDeviceImp`; `unsafeCloakDistance`, `stepSize`; engine fix keeps transparency through promotion; "framerate impact much less than original FO"; `ST3D_NUM_ZSORT_BUCKETS` doc comment about cloak cost | Rendering of the cloak itself = engine alpha path (code); parameters in ODF |
| Nebulae | Class behaviour in ODF (`nebulaType`, `lifeTimer`), SOD/TGA billboards (`env_*_nebula*.tga`, `mnebulaN.sod`); engine: no rotation | Data + engine |
| Sky / starfield | `mbg*.sod` + DDS faces, starfield counts in `ART_CFG.h` (`DETAIL_n_STARFIELD_*`), clip planes | Data (tunables) |
| Bump/specular | `_bump.dds` normal data for almost every hull, `A2FO_SPECULAR_SUFFIX` (`_spec`), Dot3 `.nvv` programs, per-light passes | Data + engine code (Dot3 replacement) |
| ECM / fleet-coordination lighting | Hull light colours from `cfgTy2Lit_*` / `cfgCombiLit_*` | Data in ART_CFG.h, engine reads |
| Post | ReShade (above) | none |
| UI/HUD colours | `gui_*.cfg` colour keys (e.g. `officerIconColor`) | Data |

Net: **Roots has no shader-defined effects of its own beyond the Dot3 pass and the
ReShade chain.** Everything else is SOD/DDS/ODF/cfg data, plus engine code giving those
data new meaning. If STA64 ingests Roots data, the effect slots are mostly **asset
slots with ODF-selected keys**, not shader slots.

### 2.5 The part I could not open

`ODF.fpq` (Roots 4 MB, FO 32 MB) is an `FPQ` container with a hashed/compressed index
(header `46 50 51 00 02 00 00 00 ...`). I did not write an extractor in this read-only
pass. So concrete per-effect ODF keys beyond those the hook `strings` and the engine
changelog name (`explosionSprite`, `shieldHit*`, `overlaySod`, `shieldScale`, `downbuffer`,
...) are not enumerated. Refit's `hosttest` reads `ODF.fpq` (`a2fmt::vfs`), so a
follow-up could dump the effect-related key set from there. Related open item: whether
Roots' Fleet Ops 4.0 submod changes any effect SODs relative to FO plain was not diffed.

## 3. Candidate effect slots

Legend: **D** = data only (asset/ini; no engine support beyond key reading), **P** =
parametric (a pack gives numbers to an engine-owned shader *kind*), **S** = needs the
engine to expose a shader contract (a pack may supply WGSL), **E** = engine behaviour
(cannot be a pack at all; a pack toggles a quirk).

Per-slot *inputs* are what the engine must give the shader; *params* are what the pack
gives.

| Slot | Replaces (stock/Refit module) | Engine-provided inputs | Pack params (Refit knobs) | Class |
|---|---|---|---|---|
| `post.tonemap_bloom` | core post | HDR scene colour, depth (optional), exposure, frame size | bloom amount/threshold/knee, exposure, shoulder, MSAA | P (S if free-form) |
| `post.chain` (Roots ReShade) | ReShade presets | LDR or HDR frame, frame size, time; optional depth | ordered list of passes, each with float params (gamma type, sharpen strength, bloom radius...) | S (list of shaders over a fixed fullscreen contract) |
| `sky.background` | sky | camera orientation, skybox id/prefix, sim time | seed, scale, warp, detail, coverage, gas colours, cores, stretch, grade, dither | P (recipe) |
| `sky.starfield` | sky | count scale, camera | star counts, scale | D |
| `world.nebula` | world | depth buffer, view-proj, eye, nebula placement envelope (baked texture), noise volume, Key light, hull shadow map, sim time | gas A/B/glow colours, tile, warp, coverage, softness, extent, height, lumps, knots, lanes, light, shading, filaments, flow, opacity, near-fade, fringe, fuzz, lit/unlit | P now; S if a pack wants another algorithm |
| `world.planet` / `world.moon` / `world.rock` | world | normal/height, Key light, atmosphere flags, surface texture name | surface recipe (colours, noise, glow) | P |
| `hull.shade` | hulls | albedo, normal map (from height), spec map (`_spec`), glow map, team colour, light list (clustered), shadow map, instance scale | ambient, specular, power, rim, bump strength, Borg profile, near-fade, shadows | S (one contract for every hull) + P |
| `hull.cloak` | hulls (`ps_cloak`, `vs_hull`) | **copy of the scene colour behind the hull**, hull depth, cloak phase 0..1, state (cloaking/decloaking/cloaked), hull bounding length/axis, owner (player/other), sim time | wave bands, wave amplitude, shimmer, front glow, hull floor for own vs others, colour split | S + P; needs scene-copy resource |
| `hull.warp_stretch` | hulls | warp phase, velocity axis | stretch factor | S/P |
| `fx.shield_hit` | effects (`effects_shields`) | hit position on shell, shell transform/radius (engine's per-class scale, x2 rescale), shell mesh (SOD), faction colour, hit strength, hit age, sim time, point-light slot | mist, reach, share, contrast, scale, drift, edge, rays (+faction list), hit-light, glow, lattice, ring, rim | S + P; **mesh-based in Roots** (SOD per class) |
| `fx.shield_down/add/sub` | effects | as above, event type | per-type colour/duration | D (SOD choice) or P |
| `fx.explosion` | effects (`effects_explosions`) | position, radius, age/life, class (fireball/shockwave/omega), light list | fireball and shockwave look, HdrFrom/Hdr, light colour/brightness/range | P (S for new class) |
| `fx.debris` | effects | SOD, instance transform, anim clock, team colour, `ART_CFG` chunk numbers | counts, lifetime, velocity, size factor | D (cfg) |
| `fx.weld` (repair, construction) | effects (`effects_repair`, `effects_construction`) | hardpoint/edge endpoints, progress, builder colour | arc share, flicker, hot spot, seam, cool time, sparks | P |
| `fx.sparks` / `fx.hit_flash` | effects | sprite flipbook, size, point | burst, spread, life, light | P |
| `ordnance.beam/bolt/field` | ordnance (`ordnance.hlsl` kinds) | start/end, width, owner/target, sim time, nebula-reach light data | Colour, CoreColour, Width, CoreWidth, Taper, Flicker, FadeIn, Light, LightReach, kind | P (kind-keyed) |
| `ordnance.sprite` (stock look) | ordnance | sprite flipbook texture, material blend factors, UV rect, colour | Hot, HotFrom (HDR lift) | D + P |
| `ordnance.warp_flash` | ordnance | warp point, craft closing, time | brightness, core, star, streak, ring, afterglow, light | P |
| `particles.emitter` | particles | SOD emitter node transform, spray/cloud mode, count, sprite, material (additive/alpha) | speed, life, spread, drag, colour curve | D (SOD) + engine sim |
| `special.overlay` (override, psionic...) | specials | target hull instance, SOD cloud | overlay on/off, light | D/P |
| `light.cluster` / `light.dynamic` | core lights | per-effect point lights | tile size, slices, count | E (engine perf) |
| `overlay.fog/grid/selection/ghost` | overlays | fog map, selection set, grid params, team colours | colours, line styles, move-path mode | P |
| `ui.shield_bar` etc. | not render | | | out of scope |
| `menu.scene` | menu | ships, nebula fields, camera, cloak choreography | `menu/map.ini` (groups, cameras, choreography) | D (script) |
| `hull.dot3` (Roots) | FleetOpsHook Dot3 | bump normal map, light directions | per-light passes | E (it is just stock hull shading in STA64) |

### 3.1 Inputs that recur (the real contract vocabulary)

These show up as inputs to many slots; defining them once is the abstraction.

| Input | Used by |
|---|---|
| `scene.depth` (readable) | nebula, cloak, shield, explosions (soft particles) |
| `scene.colour_copy` (behind-hull capture) | cloak, any refraction |
| `light.list` (clustered, per-view) and `light.register()` (effects add lights) | hull, ordnance, effects, planets |
| `light.key` / `shadow.map` (Key light depth) | hulls, nebula, planets |
| `time.sim` (sim clock; never wall time) | all animated looks |
| `instance.transform / scale / owner / team_colour / faction` | hulls, shields, debris |
| `hit.position / strength / age / type` | shields |
| `sprite.flipbook + material (blend)` | ordnance sprites, sparks, particles |
| `recipe` (named, per object identity) | sky, nebula, surface, ordnance, menu |
| `hdr` (the scene is HDR; effects may exceed 1.0) | everything bloomed |

## 4. Data-only versus engine support

**Data-only (a pack can ship these with no new engine code):**
- SOD/DDS/TGA replacements (hull, glow, bump, spec, env, shield-hit meshes, sprites).
- `ART_CFG.h` / `RTS_CFG.h` / `game_cfg.h` / `gui_*.cfg` numerics (chunk counts, starfield
  counts, clip planes, ECM light colours, shield rescale).
- Refit recipes: sky, surface, nebula, ordnance, menu scene; content-set name tables.
- Faction colour tables, race->faction mapping.
- Post chain **parameters** for an already-known pass set (bloom/threshold/gamma/sharpen).

**Needs engine support (STA64 must implement once, per slot):**
- Each slot's input contract (depth, scene copy, lights, shadow map, hit data).
- Draw ownership per slot (claims), with fallback to the default draw.
- Per-slot recipe parser + validation (file:line errors).
- ODF keys Roots added (`overlaySod`, `shieldScale`, `explosionSprite`, `downbuffer`,
  `shieldHit*`, `selectHighlight`, `renderIgnoreCameraDistance`, `explosionChunks`): these
  belong in the profile/rules layer; the render layer then only *consumes* the canonical
  DB fields (`shield_models[type]`, `overlay_model`, `scale`...).
- Light registration from effects (a feedback path from effects back into hull lighting
  for the same frame).
- The scene-copy resource for refraction slots (cost; must be only allocated when a
  cloaked hull is on screen, as Refit does).

**Cannot be a pack (engine behaviour; expose as quirk/tunable):**
- Dot3 draw (Roots replaced it; STA64 just has its own hull shading).
- Cloak AI/decloak rules, `CloakingDeviceImp` vs `CloakingDevice` gameplay semantics.
- Nebula movement/no-rotate, latinum nebula life timer (rules), chunk manager limits
  (perf knobs = tunables).

## 5. Implications for the pack/slot abstraction

### 5.1 Does it hold?

**Yes, with three amendments.**

1. `render` as a pack layer maps cleanly onto what Refit already does: recipes +
   content-set tables + ini knobs + a settings descriptor. The Refit precedent proves
   "named slot, parameter table, game folder wins over built-in, missing = default draw."
2. Roots fits **`content` + `rules` + `post`** and almost nothing of its own in `render`.
   The layer split in MODDING-IDEAS holds: Roots is "extends fleetops" with new content
   and a post chain. Its effect changes are rules-layer key semantics (which SOD for
   which hit) plus content.
3. A `render` pack for Refit-in-STA64 is T3 by nature (Refit's shaders are baked in
   code, not data), but the *recipes* are T1 (auto-convert: read `Nebulae\*.ini`,
   `Sky\*.ini`, `Surfaces\*.ini`, `[Ordnance.<odf>]`, `[Shields]` into slot params).

### 5.2 What breaks, and what to change

1. **"Override a slot with a shader" is too coarse.** A slot is `contract + implementation +
   params`. Without a versioned contract (declared uniform block, textures, vertex format,
   outputs, blend state) a third-party WGSL cannot be validated or hot reloaded safely.
   Refit's "constants keep their registers, samplers keep their stages" is the informal
   version; STA64 needs it formal (e.g. `slot.nebula@1`). Pack manifest says which
   contract version it targets.
2. **Slots need a binding key, not just a name.** Refit binds by game identity: sky by
   skybox prefix, nebula by ODF, surface by texture name, ordnance by ODF, shield colour
   by race. So `[render.world.nebula]` alone is not enough; the pack needs
   `bind = { odf = "mnebula6" }` or a default. Suggest: each slot declares its key kind
   (`none`, `odf`, `texture`, `race`, `class`, `skybox`).
3. **Three kinds of pack contribution, not one**: (a) *parameter set* for an engine
   shader kind (most of Refit), (b) *new implementation* of a contract (a WGSL file),
   (c) *asset selection* (SOD/DDS by ODF key: most of Roots). Treat as separate
   manifest forms; (a) and (c) are T0/T1-friendly and what 90% of mods need.
   (b) is the only one that needs the shader contract machinery.
4. **Slots are coupled.** Effects borrow hulls' draw; nebula borrows the shadow map;
   ordnance feeds particles. Replacing one slot can break another. The manifest needs a
   `requires_slot`/`provides_service` edge, and the engine must **fall back to the default
   implementation** rather than draw nothing (Refit's "a module that is off hands the
   part back"). Also "exactly one owner per slot per view" (claims).
5. **Post is a chain, not a slot.** ReShade presets (Roots) and Refit's HDR bloom both
   live there. Make `post` an ordered list of stages with a fixed stage position
   (pre-tonemap HDR, post-tonemap LDR), and give each pass a typed param block. Roots'
   preset is an ordered list of 7 named passes with params; this is directly a TOML array.
6. **Per-object overrides inside a slot**: Roots picks shield meshes **per ship class**
   (and Refit per race). Keep this as data in the slot's param table keyed by class/race,
   not as separate packs.
7. **Mesh-based vs shader-based effects.** Roots' shield hits are SOD meshes drawn by the
   stock path; Refit's are a procedural shell. A `fx.shield_hit` slot must accept a *mesh
   + material* implementation as well as a *procedural* one. So the contract includes
   "an optional mesh asset per type".
8. **Quirk/key sprawl lands in rules, not render.** New ODF keys (`shieldScale`,
   `downbuffer`, `overlaySod`...) need canonical DB fields; the render slot reads those,
   not raw ODF. This keeps Roots' key semantics in its profile.
9. **Compat tiers for these mods**:
   - Refit recipes -> T1 (convert at load).
   - Refit `Render3D.ini` keys -> T1 into slot params; key names stay as aliases.
   - Refit's baked shaders -> T3: STA64 ships its own slot implementations; Refit's HLSL is
     a design reference (and not copyable if derived from game files; Refit's own rule:
     recipes may hold names/params/hashes, not derived content).
   - Roots data -> T0 (SOD/DDS/cfg) via the fleetops profile.
   - Roots ReShade preset -> T2: a manifest-level mapping of the 7 pass names to native
     STA64 post passes (gamma, blur, sharpen, selective colour, deband, bloom). Free-form
     ReShade `.fx` loading is not worth it; 5 shipped effects can be native, and unknown
     `.fx` is "unsupported".
   - FleetOpsHook render code (Dot3, cloak alpha path, shield pick) -> T3 / quirk.
10. **Hot reload**: Refit already reloads recipes and some rows live; recipes are the best
    candidate. WGSL hot reload requires contract validation at reload.
11. **Settings UI**: a slot param table should carry the same descriptor metadata Refit's
    `A2SET` rows carry (kind, apply class live/next-mission/restart, scope local/match,
    advanced, help). `scope = match` matters: a visual slot is local, but a gameplay-visible
    one (e.g. cloak opacity for others' ships, `CloakHullOthers`) may be a fairness issue
    and should be tagged.
12. **HDR assumption.** Refit's scene is HDR and effects emit past 1.0; Roots' ReShade
    chain assumes 8-bit LDR. The post contract must state its input encoding; converting
    a Roots preset implies re-tuning thresholds (Pirate_Bloom threshold 0.65 on LDR is
    not the same as `BloomThreshold=1.0` on HDR).

### 5.3 Suggested slot manifest shape (illustrative)

```toml
[render.world.nebula]                 # slot = a named contract version
contract = "nebula@1"
bind.odf = "mnebula6"                 # binding key (slot declares its key kind)
params = { seed = 11, tile = 650, gas_a = [255,195,30], coverage = 0.2, ... }

[render.fx.shield_hit]
contract = "shield_hit@1"
mode = "procedural"                   # or "mesh"
bind.faction = "klingon"
params = { colour = [12,255,6], rays = 1.2, mist = 0.45 }

[render.post]
stage = "ldr"                         # or "hdr"
chain = [
  { pass = "gamma", type = 7 }, { pass = "sharpen", strength = -0.4 },
  { pass = "deband", iterations = 2 }, { pass = "bloom", strength = 0.55, radius = 2.5, threshold = 0.65 },
]
```

### 5.4 Open questions for the RFC

- Contract versioning policy: a minor only adds (as Display8's `display8v1_*`) is the
  proven model; adopt it.
- Which slots are mission-visible to other players (scope `match`)? Cloak is the main one.
- Is a user-supplied WGSL slot implementation worth the validation burden before v1?
  Refit shipped 2+ years of look without it. Recommend: v1 = parameters + asset
  selection + post chain; WGSL replacement behind a flag after contracts are stable.
- Do we convert recipes at load or ship STA64-native recipes with an importer? Refit's
  recipes are *fitted to stock art by measurement* (sky, surfaces); they are not valid
  for Roots' art (PLAN-FLEETOPS decision 7: "names and keys only, never numbers fitted to
  Roots' art"). A pack's look params are therefore **per art set**, not per slot. A game
  profile (a2, fo, roots) should pick the default param set for each slot.
- ODF key survey from `ODF.fpq` (Roots vs FO vs A2) is still outstanding (section 2.5).

## 6. Evidence pointers

- Refit: `render3d/README.md` (Modules, Content sets, Claims, Hooks), `MODDING.md`
  (Content sets, Content recipes, Shaders), `PLAN-FLEETOPS.md` ("What Roots draws, and in
  which order", "FO-B: Roots' shield hits", decisions 6, 7, 22, 30, 31, 32, 40, 43),
  `render3d/content/{sky,nebulae,surfaces,ordnance,sets}/`, `ART-DIRECTION.md`
  ("Cloaking", "Shield hits"), `layers.toml`.
- Roots: `Data/FleetOpsRootsReshadePresets.ini`, `Data/ReShade.ini`,
  `Data/reshade-shaders/Shaders/`, `Data/ART_CFG.h`, `Data/RTS_CFG.h` (lines 558-559),
  `Data/devstuff/Engine_Changelog.txt`, `Roots_ReadMe.pdf` (text extracted to /tmp, not
  kept), `FleetOpsHook.map`, strings of `FleetOpsHook.dll`, `Data/sod/shield_*`.
- Diffs vs FO plain: `ART_CFG.h` (clip planes, chunk numbers, spec suffix, ECM colours),
  `RTS_CFG.h` (shield keys), SOD/texture counts.
