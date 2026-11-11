# Native GUI button sprites (UI-27)

This is the bounded artwork foundation for UI-18, not the stock HUD layout or
UI-profile switch. Existing grid cells, capabilities, orders, tooltips and
keyboard shortcuts stay authoritative. December 2026 desktop alpha work and
the shared Android/desktop gameplay implementation are unchanged.

## Contract

The local player's resolved race ODF selects `interfaceSprites`, independently
of `interfaceConfiguration`. Captured or selected units do not change the skin.
Load that SPR table and its includes through the existing mounted VFS. Search
the selected table before `gui_global.spr`, which is the fallback only. Never
scan other factions' tables or derive a filename from the race/config name.
An absent table or sprite leaves the global fallback available; an unavailable
image leaves a readable text button. A mount/skin change invalidates cached art.

Button artwork uses the existing SPR parser's per-sprite reference dimensions
and actual texture dimensions for crop coordinates. Textures are loaded lazily
for used buttons. Existing disabled admission, poor-stock warnings and clicks
remain in the shared widget/command systems. Native files inspected have one
image per core command, so widget states tint that image rather than inventing
hover/pressed asset names.

Verified IDs: `b_build`, `b_move`, `b_attack`, `b_stop`, `b_rally`, `b_cancel`, `b_back`, and `b_harvest` (the Mine target).
Build items try `b_` plus their resolved `basename`/model first, then ODF stem;
FO4's `fed_excelsiorA`, for example, uses `b_fed_excelsior`. Build-list ordering
does not change. Formations keep their labels with generic `b_form`: the data
alone does not establish a line/column/wedge mapping for numbered icons.
Unverified command IDs retain text. Missing artwork does not add capabilities.

## Evidence and scope

Read-only local checks: STA2 Classic `odf/other/fed.odf` selects
`gui_fed.cfg` and `gui_federation.spr`; FO4 `odf/system/federation.odf` selects
`gui_federation.cfg` and `gui_federation.spr`. Classic common command art is in
`Sprites/gui_global.spr` (128-reference atlas and individual 64-reference
textures). FO4 uses 512-reference command atlases, and faction tables define
their own `b_build`. No proprietary asset is copied into the repository.

This follows modding decisions D1/D3 (existing authored files, measured scope,
no game-data rewrite) and D2 (no original executable or hook loaded). It does
not change VFS mount policy or simulation content hashing. Full stock HUD,
animated/state-specific icon conventions, and unsupported commands remain
separate work.

## Validation

Verified locally on 2026-11-11:

- Client library: 370 tests passed, one existing test ignored. Includes selected
  table priority across candidate names, includes, crop scaling, mount/skin and
  missing-art fallback, disabled tint, local-seat metadata, basename aliases,
  command behaviour and the existing construction/render regression suite.
- Installed CPU asset checks: 22 lookups on Classic and 28 on FO4, each across
  Federation and Borg tables. All textures decoded and crop bounds resolved.
  FO4 `b_build` resolves to `(208,104)-(304,200)` for Federation and
  `(208,312)-(304,408)` for Borg on a 512-square atlas; the faction table wins.
- Strict all-target Clippy for `sta64`/`sta_content` and workspace all-target
  compilation passed. NDK29/API26 Android ARM64 client-library compilation
  passed; this is not an APK or device-run result. Dependencies are unchanged.
- Independent source review found no remaining blocker. Native visual/input
  acceptance and FPS measurement remain manual; no user input was automated.

The install audit is opt-in and ships no game assets:

```sh
STA64_TEST_INSTALL=/path/to/install STA64_TEST_MOD='mod name' \
  cargo test -p sta64 --lib ui::gui_sprites::tests::installed_button_atlases --jobs 2 -- --nocapture
```

Optional `STA64_TEST_GUI_TABLES` and `STA64_TEST_GUI_BUTTONS` are comma-separated
table names and extra sprite IDs. The default tables are Federation/Borg and
the default IDs cover build, move, attack, stop, rally, cancel, back, harvest
and generic formation. This proves asset decoding/cropping, not GPU appearance.

Logs and the pinned optimized candidate are kept outside source under
`STA64-artifacts/probes/gui-buttons/` and `STA64-artifacts/desktop/`; the
candidate receipt identifies its exact source and binary hash. Check command
and build artwork, tooltips, disabled/poor feedback, clicks and faction changes
in Classic and FO4. Formations retain distinct text labels; unknown special
art and missing/invalid assets retain the existing text fallback.
