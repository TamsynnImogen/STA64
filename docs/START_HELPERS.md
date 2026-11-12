# Start helpers: hidden pods and faction objects

Playtest report (Cedric, 2026-11-12, Roots with the Fleet Ops HUD, Federation):
grey squares round the start that can be selected, named "AI Pod",
"Species 8472 Starting Pod" and "Federation Faction" (hull 100, shields 100).

## Where they come from

Evidence grade: read from data (Roots, FO 4.0, FO 3.x, Dominion War, stock A2,
STA2 Classic and Armada I race ODFs; the per-ODF survey is kept outside the
repository).

Roots' race ODFs list engine helpers among their `standardUnits` (`all_ai_pod`,
`all_start` or a race's own start pod, and a `<race>_effectsS` faction object).
FO 4.0, FO 3.x and Dominion War list at least their `<race>_effectsS`. Stock
A2, STA2 Classic and Armada I list none. `all_start` is every race's generic
start pod, not a Species 8472 unit, although it keeps that unit's display name.

Before #375 (UI-18 follow-up, merged 2026-11-12: "honour authored craft
names, HUD colours and standard starts") `Races::loadout` kept only craft and
station classes, so `classLabel = "pod"` entries were dropped. #375 made the
start the authored list exactly (docs/UNIT_INFO_STARTS.md), which placed the
pods as ordinary props: drawn (no model, so a placeholder), on the minimap,
selectable.

## What the data says

Every helper, and everything its weapons spawn, carries `hidden = 1`,
`basename = "effect_empty"`, `mapicon = "mapicon_empty"`, `invincible = 1`,
`invalidAsTarget = 1`, `perceivedNeutral = 1`, `clearsFog = 0`,
`officerCost = 0`, `iAmGameToTheDeathCondition = 0`: no model, no minimap icon,
not a target, never counts for victory. Their lifecycle is driven by
`autoExpireTime` and instant replace/spawn weapons (`replacementInstantPlayer`,
`replacementInstantAI`, `replacementInstantDelay`, `replacementMode`,
`requiredOfficers`).

Grades: the keys above are **read from data**. That `hidden = 1` means "not
shown to or selectable by the player" is **data by convergence** (every helper
and every object they spawn has it, together with an empty model and minimap
icon; STA64's map loader already treats `hidden = 1` map objects as editor
markers), not a documented engine rule. Hidden objects with real models exist,
so `hidden` does not by itself prove the engine skips drawing; for start
helpers the empty model makes that moot. The timings, which AI conversion
wins, `replacementMode` 0 vs 2 and the officer test of the unit-cap pods are
**not measured**.

## What STA64 does now

The sim's view hides every unit whose ODF has `hidden = 1`
(`EntityKind::hidden`): it is not `SELECTABLE` and is in no player's visible
set. In a seated match the client draws, radars, hovers and picks only what
`visible_to` returns, so the helpers are gone from the screen. A view with no
local seat (the `--map` viewer, the menu scene) or with no visibility data
still draws everything. The helpers stay in `WorldView::units` and in the sim:
missions find them by id (though `canPlayerSee` / `A2.IsVisibleToTeam` now
answer false for them; what the original answers is unmeasured), the sim's
tech tree counts them, and the skirmish AI takes its own units from `units`
so their tech-tree role (Roots `tech1.tt`: `fbase.odf 1 fed_effectsS.odf`)
holds for it too. Sim state, digest and snapshot are unchanged; AI decisions
on installed FO/Roots/DW content can change, because enemy helpers left the
AI's visible set (an AI could target an invincible enemy pod before).

`hidden = 1` is not "never drawn" in general: FO/DW effects such as
`effect_missilestormL` (`u_ball_obj`, model `effect_mstorm`) are hidden and
clearly visual. None reach the sim yet (only replace/spawn weapons create
them). When SIM-28 lands, drawing should follow the object's model and
class, and `hidden` keep meaning "not selectable, not on the minimap".

Not handled here: the sim does not read `invalidAsTarget`, so ships can still
auto-fire at an enemy helper near its base.

## Left for SIM-28 (needs SIM-27, unmerged)

`autoExpireTime`, instant replace weapons (`replacementInstantPlayer/AI/
Derelict`, `replacementInstantDelay`, `replacementMode`, conditions, random
weights, `requiredOfficers`), `lifeTimer` of `u_ball_obj` effects, the moon
reveal, and the 8472 starting ships spawned by `8472start` (a Species 8472
seat today starts without its corvette and active/passive ships).

What the originals do with these objects at run time is not measured; the
measurement plan is kept outside the repository.
