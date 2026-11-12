# Start helpers: hidden pods and faction objects

Playtest report (Cedric, 2026-11-12, Roots with the Fleet Ops HUD, Federation):
grey squares round the start that can be selected, named "AI Pod",
"Species 8472 Starting Pod" and "Federation Faction" (hull 100, shields 100).

## Where they come from

Roots' race ODFs list engine helpers among their `standardUnits`. Federation
(`federation.odf`): `all_ai_pod`, `all_start`, `fbase`, `fminer`, `fcorvette`,
`fed_effectsS`, `fconst`, `fconst`. Every Roots race lists `all_ai_pod`,
`all_start` (Species 8472: `8472start`) and its `<race>_effectsS`; Romulan
also `allu_detect`. Fleet Ops 4.0, FO 3.x and Dominion War list their
`<race>_effectsS` (and Borg `borr_connectionsS`; FO 4.0 also `effect_rm`).
Stock A2, STA2 Classic and Armada I list none.

`all_start` is a copy of `8472start` that keeps its `unitName = "Species
8472 Starting Pod"`; it is every race's generic start pod, not a Species 8472
unit, so it is right that a Federation seat gets one.

Before #375 (UI-18 follow-up, merged 2026-11-12: "honour authored craft
names, HUD colours and standard starts") `Races::loadout` kept only craft and
station classes, so `classLabel = "pod"` entries were dropped. #375 made the
start the authored list exactly (docs/UNIT_INFO_STARTS.md), which placed the
pods as ordinary props: drawn (no model, so a placeholder), on the minimap,
selectable. The regression is #375; #412, #414/#424, #415 and #416/#426 do not
touch starts.

## What the data says (Roots `Data/ODF.fpq`)

All of them, and everything their weapons spawn, carry `hidden = 1`,
`basename = "effect_empty"`, `mapicon = "mapicon_empty"`, `invincible = 1`,
`invalidAsTarget = 1`, `perceivedNeutral = 1`, `clearsFog = 0`,
`officerCost = 0`, `iAmGameToTheDeathCondition = 0`. No model, no minimap icon,
not a target, never counts for victory.

| object | lifecycle keys | reading |
|---|---|---|
| `all_ai_pod` "AI Pod" | `autoExpireTime = 0.5`; `weapon1..4 = convert_ai_pod_{random,steamroller,balanced,expansionist}`: `classLabel = "replaceweapon"`, `replacementInstantAI = 1`, `replacementInstantPlayer = 0`, `replacementMode = 2`, condition `all_ai_pod` → `ai_pod_*` (random: weights 35/35/25) | AI seat: replaced at once by a personality pod (`ai_pod_balanced` etc., hidden, `anti_cloakA` weapon, no expiry). Human seat: nothing fires; it expires after 0.5 s |
| `all_start` "Species 8472 Starting Pod" | `weapon1 = spawn_effect_rmai` (`InstantAI = 1`, delay 0.1 s), `weapon2 = spawn_effect_rm` (`InstantPlayer = 1`, delay 0.2 s), `replacementMode = 0`, `concurrentChilds = 2` → `effect_rm[ai]` + `all_basei_pod` | spawns the moon-reveal effect (`effect_rm`: `u_ball_obj`, `lifeTimer = 1`, weapons `all_revealmoonsA[1]`) and an `all_basei_pod` ("AI Pod", `autoExpireTime = 600`) |
| `8472start` | as `all_start`, plus `8472corvette`, `8472active` ×2, `8472passive` at authored offsets | Species 8472's real starting ships come from this pod |
| `fed_effectsS` "Federation Faction" | `weapon1 = spawn_unitcap_pod` (`requiredOfficers = 499999`, delay 1.0 s), `weapon2 = spawn_unitcap_d_pod` (`requiredOfficers = 299999`, delay 0.5 s), all three `replacementInstant*` = 1, `replacementKeepOwner = 1`, `replacementMode = 2` | replaced by `unitcap_pod` or `unitcap_d_pod` (hidden, persistent) by the officer setting |
| FO 4.0 `fed_effectsS` "Starfleet Faction" | no weapons | a persistent hidden faction marker |

Grades: placement, keys and chains are **data**. That `hidden = 1` means "not
shown to or selectable by the player" is **data by convergence** (every helper
and every object they spawn has it, together with an empty model and minimap
icon; STA64's map loader already treats `hidden = 1` map objects as editor
markers), not a documented engine rule: the Engine_Changelog and readmes do
not define `hidden`. Hidden objects with real models exist (Roots `8472gate`
"Fluidic Rift", `fed_warpinE2`), so `hidden` does not by itself prove the
engine skips drawing; for start helpers the empty model makes that moot.
The timings, which of the four AI conversions wins, `replacementMode` 0 vs 2
and the officer test of the unit-cap pods are **unverified**.

## What STA64 does now

The sim's view hides every unit whose ODF has `hidden = 1`
(`EntityKind::hidden`): it is not `SELECTABLE` and is in no player's visible
set, so the client neither draws it, puts it on the radar, nor lets it be
hovered or picked (all of those follow `visible_to`). It stays in
`WorldView::units` and in the sim, so missions find it by id and SIM-28 can
run its lifecycle. State, digest, snapshot and replay are unchanged.

## Left for SIM-28 (needs SIM-27, unmerged)

`autoExpireTime`, instant replace weapons (`replacementInstantPlayer/AI/
Derelict`, `replacementInstantDelay`, `replacementMode`, conditions, random
weights, `requiredOfficers`), `lifeTimer` of `u_ball_obj` effects, the moon
reveal, and the 8472 starting ships spawned by `8472start` (a Species 8472
seat today starts without its corvette and active/passive ships).

### Measurement plan (Refit bench, `./a2test --game fo-roots`)

One scenario per seat type, on a 2-player Instant Action map, seat 0 the
bench's player and seat 1 an AI, both Federation, then both Species 8472;
log every object create/destroy with its ODF, owner and game time for the
first 15 s, and again at 601 s:

1. Human seat: does `all_ai_pod` vanish at 0.5 s with nothing created? Does
   `all_start` create `effect_rm` and `all_basei_pod` at 0.2 s, and is
   `all_start` itself removed (mode 0)? When do `effect_rm` (1 s) and
   `all_basei_pod` (600 s) go?
2. AI seat: which `ai_pod_*` replaces `all_ai_pod`, at what time, and does it
   vary with the seed (random weights) or the AI personality setting?
3. `fed_effectsS`: which of `unitcap_pod` / `unitcap_d_pod` appears at each
   officer setting (normal / restricted / more / max), and when.
4. Species 8472: the four ships' positions relative to `8472start`.
5. Click and box-drag over each helper's position, and look at the minimap:
   confirm nothing is selectable or shown.
