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

Verified IDs: `b_build`, `b_move`, `b_attack`, `b_stop`, `b_rally`, `b_cancel`.
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

Implementation and validation are in progress. Automated checks and a pinned
optimized manual-test build will be recorded here before claiming completion.
