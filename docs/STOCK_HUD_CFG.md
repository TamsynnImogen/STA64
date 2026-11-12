# A2/FO stock HUD from CFG and SPR (UI-18, #371)

Implements accepted modding decisions D1/D2/D3/D5. UI-17 (#370) is merged.
The stock renderer consumes the mounted local race's interface CFG and selected
GUI SPR table, falling back to gui_global.spr. LCARS restores the existing
Refit layout/grid. Gameplay/capability/command handling remains shared.

## Design before implementation

- `stock/layout.rs` is the pure view of resolved GuiLayout: case-insensitive
  access to typed panel facts and generic keys, canvas-to-screen x/y scaling,
  local/absolute element rectangles, info variants, background pieces and
  palette geometry. No minimum sizes, sorting of info heights, recentering or
  fixed LCARS margins are applied to authored stock rectangles. Drawing a zero
  width/height rectangle is disabled; offsets such as infoBarOffset still use
  x/y even when their size is zero.
- Native A2 evidence is retained in docs/UI_PROFILES.md and the private
  probes/ui17/native-a2-hud.json bundle. Background Size is a count, default
  zero: pair key_<n> with configured-sprite.<n>, never draw all discovered
  entries or synthesize a full panel for count zero. GUI resource IDs are
  0 crew, 1 officers, 2 dilithium, 3 latinum, 4 metal, 5 biomatter; live stock
  vector indices remain their existing canonical 0..3.
- Info variant 0 is no selection, 1 normal single / group up to eight, 2
  builder / larger group. The same CFG supplies names/classes, wireframes,
  bars, multi-selection cells and infoBarOffset. This is separate from
  LCARS's Low/Middle/High enum.
- Generic GuiImage nodes reuse the current GUI atlas decode/crop/cache and
  selected-table/global priority. Static panels use explicit frame indices.
  Wireframe images may explicitly use the existing shared sprite table; GUI
  button/panel lookup is not silently broadened. Missing art keeps a useful
  text/colour fallback and never crashes.
- Stock HUD children hang under the existing HudRoot, retaining game-flow
  visibility and the projecting UI camera. Replace the legacy body only when
  a supported stock profile and valid authored layout are active. Values patch
  existing nodes; structural/skin changes rebuild. Commands and group cells
  use the same GridCell/GroupTile and existing press/action handlers.
- RadarRoot keeps its own feed/state/fog/input lifecycle. Its stock branch
  builds the configured minimap pieces/display/filter areas; radar positions
  scale to the actual display size. Keep one RadarSquare; do not reparent it
  outside its owner or duplicate blips/interaction. Theme/profile/layout
  changes recreate children while retaining root metadata and radar state.
- Race CFG resolution uses mounted files, own/earlier-include precedence and
  existing aspect variants when supplied. Removing/changing a race must not
  retain another race's old layout. Authored zero-size FO cinematic panels
  stay disabled. No originals are edited to adapt resolution.
- Palette size/gap/position comes from CFG. Command ODFs (`allcommands`,
  preferredPosition, buttonName, menu) supply authored metadata for implemented
  commands. Existing action capability gates and targeting/build rules stay
  authoritative. Do not expose enabled unsupported gameplay actions.

## Scope and proof boundaries

Top-right full menu functions (UI-20), production queue views (UI-26), per-unit
current crew and future resources are not fabricated by a new presentation. Show supported live values and record unavailable
fields explicitly. A CFG can hide its own element; no gameplay is disabled
merely by choosing a HUD or platform.

Synthetic layout/art/state/input tests, installed CFG/SPR/command read checks,
real windowless GPU frames at several resolutions, desktop regressions, strict
Clippy/workspace and Android library checks precede an optimized manual build.
Native visual acceptance is separate; no full original-game equivalence, APK
or device claim is inferred from these tests. Original game data, accepted
builds and December desktop-alpha scope remain protected.

## Implemented presentation

The stock body uses CFG resource/info backgrounds and their numbered SPR crops,
resource flags and native resource IDs, single-selection name/class/hull/shields,
selectable group icons, and the authored minimap display and filter rectangles.
Race changes clear stale layouts; an unsuffixed configuration selects the closest
available canvas aspect, while explicitly suffixed configurations remain authoritative.
Numeric fields and bars update existing nodes. Switching to LCARS retains the HUD
root, its camera and visibility, current selection and armed commands.

Command ODFs are loaded through the existing layered ODF resolver. Their button,
menu, tooltip and colour fields decorate only actions the shared capability model
already supports. The static A2 bar compacts commands by command-type priority;
`preferredPosition` is a secondary ordering hint for ties. Native `qsort` does not
specify equal-priority order, so that tie policy is an explicit STA64 adaptation.
Visible rows extend upward from `popupPaletteYA`. Long lists use previous/next
pages rather than inheriting the LCARS build-list limit. Menu and paging actions
change presentation only; build, targeting, stop and formation actions still use
the shared HUD handlers.

Native evidence was checked against the read-only Armada2.exe SHA-256
`8d3ba6ec826198daf3e0c7b3435e9ce8195293e56da7c87e0af2634eaa4693b3`:

- `0x50aaa0`: StandardBackground count, `default_sprite`, `.n` / `_n` pairing.
- `0x5358f0`: independent canvas scaling for rectangles.
- `0x4eadf0`, `0x4ff450`, `0x4f2ee0`: minimap, resource IDs and selection variants.
- `0x4fa990`, `0x4fbdd0`, `0x4fc670`: palette configuration and upward static rows.
- `0x519070`, `0x44e1d0`, `0x6f45d4`, `0x44aaa0`: command metadata,
  command-name mapping and the static command-type priority.
- `0x4f7820`, `0x4f7a60`, `0x4f7e20`: five same-slot craft wireframe layers
  named `<basename>w1` through `w5`, and the noncraft `_s` form.

These are static findings, not original-engine runtime or Fleet Ops equivalence
claims. Evidence bundles and captures remain outside the repository with the
local probes; no proprietary assets or executable code are redistributed.

## Verification and remaining acceptance

Run the client tests with the repository's two-job, low-priority Cargo policy.
The optional installed-data GPU fixture is
`ui::hud::stock::gpu::installed_stock_hud_renders_on_gpu`:

```sh
export STA64_GPU_TESTS=1 STA64_HUD_TESTS=1
export STA64_TEST_INSTALL=/path/to/STA64
export STA64_TEST_MOD='STA2 Classic' STA64_HUD_PROFILE=a2
export STA64_HUD_CAPTURE_DIR=/path/to/capture-directory
# cargo test -p sta64 --lib installed_stock_hud_renders_on_gpu --jobs 2 -- --nocapture --test-threads=1
# Repeat with STA64_TEST_MOD=FO4 STA64_HUD_PROFILE=fo.
```

The fixture renders offscreen, without creating a window or injecting input. It
checks actual atlas resolution, command art, live fields and captured pixels at
4:3 and 16:9. Missing hardware/opt-in is reported as a skipped proof, not Android
or native visual acceptance. Android library checking is separate from an APK.

UI-18 remains open until visual comparison against original A2/FO Roots reference
frames and command order for representative unit classes passes. Missing shared
per-unit current crew/energy data, UI-26 production jobs, UI-20 top-right menu backends, named font
selection and per-system damage-state wireframe colouring are not fabricated. The
selection panel composes the authored wireframe layers in one slot, with an `_s`
alternative on the first layer. System damage colours remain unavailable rather
than being inferred from overall hull health. Authored
zero-size cinematic areas remain disabled; a new 3D portrait renderer is outside
this change. The LCARS option restores the existing Refit layout and grid.


## Live feedback corrections

The first optimized candidate (`6f38386`) exposed two presentation faults that
synthetic component checks did not catch: wide-screen atlas pieces were
letterboxed by Bevy's default `NodeImageMode::Auto`, and small CFG text slots
shrunk the chosen bitmap font until lowercase ink was only a few pixels high.
The follow-up uses `Stretch` for authored GUI rectangles, matching native
independent-axis drawing. The installed-data GPU check now inspects pixels at
a known resource-panel join; the old capture has a black gap at that join.

Stock resource/info/filter labels explicitly use the scalable text fallback at
a readable minimum size. Their authored position and width remain intact;
a short legacy text rectangle does not clip the line's font height. Existing
SPR fonts remain available to other consumers. The inspected Classic/FO4 CFGs
do not name individual font files, and native font parity is not claimed by
this readability correction.

The same live Classic map (`3peace`) was checked read-only: the minimap cluster
is at the expected normalized start position near (0.219, 0.219), and the large
grey world shroud matches the mod's `Gshroud` RGB (65, 71, 85). Those are separate
from the atlas/font faults; this HUD correction changes neither map projection,
sensor rules, fog rendering nor original textures.

## Starting personnel resource binding

After the shared #377 prerequisite, stock resource ID0 reads optional current
player crew from PlayerView; ID1 reads the existing live-actor officer census
and configured maximum. `resource_0`/`resource_1` rectangles, inherited
`resource_N_color`, `noCrew`/`noOfficers` and disabled rectangles govern the
rendering. No replacement positions, pools or caps are inferred by the HUD.
Unknown crew stays blank; authored zero is shown as zero. Integer formatting
uses Fixed bits rather than a float conversion, preserving large exact counts.

Setup retains Normal/Lots and the selected native MP officer row in GameOptions.
Captions use mounted `label.map` keys `sr_normal`, `sr_lots` and the authored
`ol_*` keys, with translated/literal fallbacks. These are the existing frontend
controls; full native frontend layout reproduction is outside this correction.
Unlimited is not presented as Lots. Crew production/regeneration and officer
queue reservations/admission remain separate missing gameplay systems.

The installed GPU fixture now loads actual mounted definitions and faction
starts into the shared simulation before building HudFeed. It checks crew and
officer source text against that view and native visibility flags, then captures
the real HUD. Classic Lots/Restricted adds a separate wide capture. This closes
the previous fake-player fixture gap; live gameplay/FPS acceptance is separate.

Verified follow-up: 446 client tests pass (one existing ignored), strict client
all-target Clippy, workspace/all-target and NDK29/API26 ARM64 library checks pass.
Five actual windowless NVIDIA Vulkan captures cover Classic and FO4 at 4:3 and
16:9 plus Classic Lots/Restricted. Classic source labels and pixels show
1200 /16/600 normally, 30000 /16/300 for Lots/Restricted; FO4 hides both fields
through its CFG flags. The lead inspected the captures. Private evidence lives
in `STA64-artifacts/probes/ui17/personnel-hud-*`. This is HUD rendering proof;
live gameplay readability/FPS and the broader UI-18 reference gate remain open.


## Resource text colour defaults

An authored `resource_N_color` (RGB or RGBA, including white, black or zero
alpha) wins through the existing case-insensitive CFG/include resolution.
When absent, the six native resource IDs use Armada II's built-in palette:

| ID | Resource | Default RGB |
|---|---|---|
| 0 | Crew | 0, 1, 0 |
| 1 | Officers | 1, 1, 1 |
| 2 | Dilithium | 0.6, 0.9, 1 |
| 3 | Latinum | 1, 1, 0 |
| 4 | Metal | 0.8, 0.8, 0.8 |
| 5 | Biomatter | 1, 0, 0 |

Classic does not author those overrides; previously its resource numerals
incorrectly inherited the generic theme text colour. FO4's authored palette
and per-faction overrides remain authoritative. The same widget supports both;
no game assets or simulation/resource values change.

Static evidence: clean `Armada2.exe`, SHA-256
`8d3ba6ec826198daf3e0c7b3435e9ce8195293e56da7c87e0af2634eaa4693b3`.
The initializer at `0x50a220..0x50a31c` allocates RGB triples in the pointer
table `0x764664 + ID*4`. The setter at `0x50a1f0..0x50a219` reads this
same table and copies RGB into the resource's text component at `+0x70`.
The resource-panel loop calls it at `0x4ff813` with the current resource ID.
This is original-A2 static evidence, independently checked with objdump;
it does not claim original Roots execution. Native disassembly and subsequent
local capture/build receipts are retained under
`STA64-artifacts/probes/resource-colours/`.

Regression checks cover all six defaults, explicit per-ID CFG overrides,
include/case precedence and retaining tints across value updates. The installed
windowless GPU check inspects resolved Bevy text colours and searches the
Classic crew/dilithium/latinum slots for matching framebuffer pixels, so an
all-white capture can no longer pass solely because its values are correct.


## Mouseover InfoBar

The separate InfoBar now follows the visible/selectable actor under the map
cursor without changing Selection. It uses the same body/strategic picker as
selection and orders, after fresh HUD hit testing. Hover is cleared over UI,
outside the cursor/window, in menu scenes, for hidden/dying/nonselectable or
removed actors, and when there is no live match. The bar can appear with no
selection; when nothing is hovered it falls back to a single selected actor.
The selected detail card and command panel keep their own selected actor.

The selected panel's variant determines the origin:
`infoPanelArea_0/1/2 + infoBarOffset_0/1/2`. Native CFG text rectangles are
`hull`, `shields`, `crew_amt`, `energy_amt` and `shipName`; icon rectangles
are `icon_hull`, `icon_shields`, `icon_crew_amt` and `icon_energy_amt`.
Their GUI SPR names are `hull`, `shields`, `crew_amt` and `energy_amt`, resolved
through the selected GUI table then `gui_global.spr`. Zero-area rectangles
suppress their own element. The authored CFG scales independently on both axes.

Hull, shields and energy show rounded whole percentages as bare numbers,
without a `%` suffix, following the user's stock reference correction; crew
shows actual current crew. Percentages derive from live current/max
pools, not definition capacities. Unknown
unit pools stay blank with their icons hidden; known zero stays zero. Hover
uses the authored `unitName` type caption; the selected-single fallback retains
its stable possibleCraftNames instance caption. The main card retains both
identity rows. `shipNameColor` controls the strip caption.

Native numeric/icon tints follow faction ODF `crewYellowStatus` and
`crewRedStatus` thresholds (defaults 0.5/0.25): strictly above yellow is green,
strictly above red is yellow, otherwise nonnegative is red. Negative status
is grey; a zero-capacity crew pool uses the native fully-crewed colour.
The existing A2FOCraftIdentity GUI palette commands are also honoured:
`systemIconHealthyColor`, `systemIconLowColor`, `systemIconCriticalColor` and
`systemIconDestroyedColor` for hull/shield/crew status, plus the independent
`specialEnergyIconColor`. The extension's healthy/low/critical boundaries are
strictly above 50%, above 25%, and positive through 25%; zero percent uses the
destroyed colour. Each missing palette command leaves its native fallback.
This reuses the existing extension interface and does not load its binary.
No subsystem-disabled state is fabricated from the currently exposed unit pools.

Explicit numeric text and icon tint overrides may be supplied with these
**additional STA64 GUI CFG keys** (not claims of original-A2 commands):

```cfg
hull_color = 0 1 0 1
shields_color = 0 1 0 1
crew_amt_color = 0 1 0 1
energy_amt_color = 0 1 0 1
```

RGB/RGBA, inheritance and case-insensitive lookup follow the existing GUI CFG
parser. An explicit per-field value wins over the A2FO palette and native status tint
for that field and its icon. Original data is not edited. These optional presentation fields do
not change simulation thresholds or crew/energy mechanics.

The data prerequisite is merged #381: current crew and initialized energy are
shared UnitView values. No capacity is substituted for unknown current crew;
older snapshots preserve that unknown state. Crew spending/regeneration,
casualties and boarding remain separate gameplay work.

Static evidence from clean A2 SHA-256
`8d3ba6ec826198daf3e0c7b3435e9ce8195293e56da7c87e0af2634eaa4693b3`:
InfoBar loader near `0x4ef7fc..0x4efb4a` reads native text/icon areas and
copies the name colour; percent getters `0x4ecd40`/`0x4ed000` multiply by 100;
formatting near `0x4ece4a` uses floor(percent + 0.5) and the `%` literal at
`0x70ad50`; energy uses the same format near `0x4edb9a` with `%` at `0x70af4c`.
Current crew comes from
actor state near `0x4ed3cf`. Status colour selection is `0x4ec5d0`, with race
threshold defaults loaded at `0x48b52b..0x48b558`. White icon art requires tint.
Private native receipts are in `STA64-artifacts/probes/unit-status/native-infobar*`.
This is static original-A2 evidence, not original Roots runtime acceptance.
The current bare-number presentation follows the supplied stock reference and
user direction; the original literal above is not used as a display requirement.

Stock wireframes opt into linear texture filtering when resized. The sampler
is isolated in page and resolved-art caches; other GUI pictures and sprite
fonts retain their existing sampling. CFG rectangles and SPR crops are unchanged.
World selection gauges use projected model bounds with two-pixel height and
gap defaults. Optional `selectionGaugeHeight` and `selectionGaugeGap` in the
resolved race GUI CFG are STA64-only pixel overrides (zero height disables
gauges); these are not claimed as original-A2 commands. See [display evidence](DISPLAY_FIDELITY.md).

Text/icon nodes persist across hover and value changes, following their HUD
root visibility. The integrated checks and optimized launcher are recorded in
the consumer PR; native live comparison and settled FPS remain a manual gate.
