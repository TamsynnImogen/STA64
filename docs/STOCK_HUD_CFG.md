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

Top-right full menu functions (UI-20), production queue views (UI-26), crew
values not exposed by the shared view and future resources are not fabricated
by a new presentation. Show supported live values and record unavailable
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
crew/energy data, UI-26 production jobs, UI-20 top-right menu backends, named font
selection and per-system damage-state wireframe colouring are not fabricated. The
selection panel composes the authored wireframe layers in one slot, with an `_s`
alternative on the first layer. System damage colours remain unavailable rather
than being inferred from overall hull health. Authored
zero-size cinematic areas remain disabled; a new 3D portrait renderer is outside
this change. The LCARS option restores the existing Refit layout and grid.
