# UI profiles and the A2/FO stock HUD

UI-17 (#369) is the shared presentation seam; UI-18 renders stock A2/FO HUD
structure using the existing resolved `misc/gui_*.cfg` and SPR data. The LCARS
renderer stays available. No profile choice changes simulation/content rules,
saves, network formats or the mod mount. This implements modding decisions
D1/D2/D3 and supported A2/Roots scope D5.

## Contract before implementation

- `UiProfile` identifies LCARS, Armada II or Fleet Operations presentation,
  distinct from the gameplay/content `sta_content::Profile`. A2/FO mounts
  default to stock; native/no mount and unsupported A1 stock UI keep LCARS.
- A shared `UiPreference` gives an explicit LCARS override. The Options screen
  and `--lcars-ui` set it. This is in-memory, like existing Options; general
  settings persistence is separate work and is not invented here.
- Optional `[Mod] UiProfile=stock|lcars|a2|fo|<registered-name>` in `info.ini`
  selects a presentation preset. Child-before-parent mounted sources provide
  the first key. Original engines may ignore this STA64 metadata (D3).
  A small registry lets native integrations register names for existing
  renderer families; faction CFG/SPR still provides each mod's actual skin.
  Unknown names warn and fall back to the game's stock preset.
- One effective resource and one theme switch drive both HUD and menus.
  Changes happen before widget paint/HUD/menu rebuild. Existing LCARS bench
  fixtures without a mounted install retain their current look.
- UI-17 supplies profile selection, a neutral stock-inspired fallback theme
  and switch tests. It does not claim that changing colours reproduces the
  stock HUD: actual panel geometry/art and command-bar integration are UI-18.
- UI-18 follows the race's `interfaceConfiguration`, aspect-specific sibling
  CFG where supplied, existing own/earlier-include precedence and mounted VFS.
  Zero-size rectangles disable elements. Panel piece sprites use native
  indices after verification; `gui_global.spr` is the fallback. All gameplay
  buttons continue through the existing action/command path.

## Gates

Synthetic resolver/remount/profile-switch and menu/HUD tests; existing client
regressions; strict Clippy and workspace/Android library checks. Installed CFG
and GPU frame checks belong to UI-18. No original executable or GUI/input
automation is used; original data and accepted builds remain untouched.
The December desktop alpha remains the priority.

## UI-17 verification

The effective profile is resolved before widget paint and UI rebuild. Options
changes are handled before that resolution. HUD and radar retain their root
entity, visibility and camera target while replacing themed children; radar
state/map/filters/feed and fog remain intact. Notices keep active messages and
chat history, and loading-screen rebuilds preserve load progress. Ordinary
numeric HUD updates keep their controls stable.

399 client library tests passed with one existing ignored test, including
same-update Options/profile/menu changes, HUD/radar camera-target preservation,
chrome colours and dimensions, notice retention and loading progress. Strict
client Clippy across all targets, workspace/all-targets and NDK29/API26 ARM64
library checks passed. General settings persistence, stock HUD art/layout and
original-game visual equivalence are not established by these foundation tests.
UI-18's CFG/SPR renderer and current verification boundaries are documented in
[STOCK_HUD_CFG.md](STOCK_HUD_CFG.md). Its native reference acceptance remains open.

## Original behaviour followed for UI-18

Evidence grade: read from the stock A2 executable (static, Ghidra); the
function-level notes are kept outside the repository.

- A panel reads `<panel>Size` with default zero, then loops only over `0..size`,
  resolving sprite `<configured-name>.<n>` and CFG `<panel>_<n>`. Zero is not a
  request to draw a single full-panel sprite.
- Rectangles scale x/width and y/height separately by the active dimensions
  divided by the CFG canvas dimensions, with nearest-integer rounding. The
  FleetOpsHook parameter DB parser's `CacheDimensions` defaults to 1600x1200,
  overridden by `screenWidth`/`screenHeight`.
- Six resource IDs gated by `no<Name>` each load their `resource_<id>`
  rectangle, in the order Crew, Officers, Dilithium, Latinum, Metal, Biomatter.
  Those GUI IDs differ from STA64's four stock resource-vector indices.
- The info panel is 0 for no selected object, 1 for a normal single object or
  a group of at most eight, and 2 for the builder display or larger groups. This
  differs from the LCARS renderer's height enum.

These are static A2 findings and installed FO CFG/SPR observations, not
original runtime verification or a claim that all FO-specific extensions are
recovered. Original executable/data files were not changed or run.
