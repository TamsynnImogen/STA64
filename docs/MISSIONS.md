# Mission scripts

## Background

| Source | Files | What it is |
|---|---|---|
| Armada II (stock) | `missions/<name>R.drl` + `<name>S.dsl` | 32-bit Windows DLLs holding the compiled mission rules and script |
| Fleet Operations | `missions/mmm_loader.dll` + `missions/*.lua` | the MMM framework: Lua (luabind) bindings such as `mmm::Entity`, `mmm::Matrix` and `mmm::Vector3`, loaded by a 32-bit DLL |
| BZN / SOL maps | mission name inside the map (e.g. `EmptyMission`) | selects which mission the map runs |

A native 64-bit STA64 build cannot load 32-bit Windows DLLs on any platform,
including Windows. So STA64 never loads `.drl`, `.dsl`, `.dll` or
`mmm_loader.dll`.

## Design

MMM is built into the engine as a native Lua runtime (`mlua`). It exposes the
same Lua API that `mmm_loader.dll` gives FO scripts, so existing `.lua`
missions run unchanged. Native STA64 missions use the same runtime with
extended APIs.

Mission resolution for a map's mission name `N`, highest-priority mod layer
first:

1. `missions/N.lua` exists: run it in the integrated MMM runtime.
2. Only legacy DLL files exist (`NR.drl`, `NS.dsl`, `N.dll`): no script runs.
   The map loads as a plain scenario. One info-level log line says the
   mission needs a Lua conversion, and the name is added to the
   conversion report (see below).
3. Nothing exists, or the name is a known placeholder such as
   `EmptyMission`: no script runs, and only a debug-level log line is written.

Missing or unloadable mission scripts never stop a map from loading. They
never show an error dialog, and they never count as a script error. That
covers every case where a BZN or SOL map names a script that isn't there.

Errors inside a Lua script that does exist are a separate case. They are
reported, the failing callback is disabled, and the match continues.

## Conversion report

`sta64 --mission-report` (planned) lists every map whose mission resolves to
case 2. That gives the Lua conversion work (STA2 Classic Mod campaign, A1
missions) a checklist straight from the installed content.

## To do

- Inventory the full `mmm` Lua API from `mmm_loader.dll`'s luabind
  registrations and the stock `.lua` scripts' usage. Record names and
  signatures only, then reimplement.
- `sta_mission` crate: resolution rules above, plus the Lua runtime bound to
  `sta_sim` commands so scripts stay deterministic in lockstep.
- Run every installed `missions/*.lua` headless against stub maps as a
  regression test.
