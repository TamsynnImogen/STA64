# LUA-06 Armada I mission calls (`A1` table)

The stock Armada I missions import 213 engine functions by name from
`Armada.exe` (`docs/MISSION_DLL_API.md`). 41 are the DLLs' own plumbing (hot
and timer variables, save/load, rule registration): plain Lua variables,
monitors and tables here. The other 172 imported
signatures are bound as `A1.<Name>`, one Lua function per name, overloads
selected by argument count and type. Nothing is bound that no stock DLL
imports, except two host reports (below) and `A1.paths`.

Arguments are strict: exact arity, integral handles, finite numbers, strict
booleans. A malformed call is a Lua error and queues nothing. Handles are
this runtime's entity ids; `BuildObject` returns a pending handle (`>=
0x40000000`) that all `A1` calls and the reused `A2` calls resolve once the
spawn receipt is in (before that: existence queries say no, other calls
error). Reads see the last world snapshot, as for `A2`.

## What each call does

- **Same operation as A2, reused** (the `A2` binding is called, errors say
  `A1.`): `RemoveObject CraftCannotDie IsAlive Exists DoesNotExist Cloak
  Decloak ImmediateCloak DisableWeapons DisableEngines SetSpecialEnergy
  IsVisibleToTeam GetRealTeam SetRealTeam SetPerceivedTeam
  ObjectivesDisplay_Set_Visibility`, and `GetNearestStarbase` uses
  `A2.IsStation`.
  `ObjectivesDisplay_Set_Text_From_File(file)` calls the A2 loader with
  `resetCompleted = true` (a fresh load).
- **Sim-backed**: orders as the unit's owner (`Goto Stop Attack(unit, unit)
  Mine Build StopAllShips`), SCRIPT edits (`SetImpulseSpeed`, alerts,
  `DisableShieldGenerator`, `SetDilithium`/`AddDilithium`,
  `RemoveAllObjects`, `RemoveAllCraft`, `BuildObject`), and reads of the
  view (`GetHandle GetLocation GetDistance GetShieldPercent GetCrew
  CountUnits* CountVisibleUnits* GetNearest* IsValid IsExploding GetTime`).
- **Presentation** (`Edit::Present`, no client consumer yet): bridge scenes and
  movies, cineractive mode and shots, fades, titles, audio messages, minimap
  messages, credits, grid, billboards, transporter effect, starfield. Typed
  where the seam has a variant (`Movie`, `StopMovie`, `Cineractive`, `Fade`,
  `Title`, `RemoveTitle`, `Sound`, `StopSound`, `Effect`), otherwise
  `Presentation::Call` named `A1.<Name>` with the arguments in call order.
- **Seam gaps**: see below.

## Bridge scenes (for MED-03 and the bridge display)

| call | event |
|---|---|
| `PreLoadBridgeScene(type)` | `Call A1.PreLoadBridgeScene [Int type, Text race]` |
| `SetBridgeSceneRace(type)` | `Call A1.SetBridgeSceneRace [Int type, Text race]` |
| `ShowBridgeScene(bool)` | `Call A1.ShowBridgeScene [Bool]` |
| `ShowBridgeOnScreen(bool)` | `Call A1.ShowBridgeOnScreen [Bool]` |
| `PlayBridgeMovie(name, int)` | `Movie { name, in_panel: true, skippable: true }` |
| `StopMovie()` | `StopMovie` |
| `IsMoviePlaying()` | true from `PlayBridgeMovie` until `StopMovie` or the host's `A1.MovieDone()` |

The race text is empty for type 0 (no `Text` argument is sent).

## Evidence grade

Argument shapes: read from the DLL import signatures. The bridge type to race
mapping, the title/subtitle rectangle layout and the shield fraction scale
were measured on the install's mission DLLs; details live outside the
repository. Everything under "Not measured" is a guess flagged in the code.

## Which calls do what

"Callable" in `docs/MISSION_DLL_API.md` only means the name and signature are
bound. The generated summary splits them: working (sim- or view-backed),
presentation events with no consumer yet, retained-and-counted requests with
no effect, and calls that always raise for lack of world-view data.

## Not measured (guesses are flagged in the code comments)

The third argument of `Goto`/`Stop` (the trailing `int`), the flag of
`PlayBridgeMovie`, the extra `int`/`bool` of the `GetNearestObject*` and
`CountVisible*` searches and of `RemoveAllCraft`, whether the third
argument of the `...Near...` searches is a team or a relation filter (read as a
team, -1 any), the meaning of bridge type 0, the `TeamRelation` values, the
restriction masks of `RestrictSpecialFlag`, what `IsCompromised`
and `GetLocationOfFlag` return, and the delay of `FailMission` /
`SucceedMission` (retained, not applied).

## Seams the host fills

- **`A1.paths`**: `name -> { {x=,y=,z=}, ... }`. The API holds no map data.
  `Goto(unit, path)`, `BuildObject(odf, team, path, n)`, `GetLocation(path, i)`,
  `GetDistance(unit, path, i)` and `CountUnitsNearPoint` read it; an unfilled
  path is an error naming `A1.paths`.
- **Completion**: `A1.MovieDone()` and `A1.AudioMessageDone(id)` (not DLL
  calls) tell the runtime the client finished. Until then a movie or an audio
  message counts as playing until stopped.
- **Labels**: `GetHandle(label)` uses the existing label map (`MissionConfig::labels`).

## Not available until a sim seam exists

Writes are retained as `Call A1.<Name>` and counted in `unsupported()` (also `Load_Rules` and
`Load_Action_Scripts`, which load nothing and return 0). `SetRelation` sets only the
mission-local override (provisional 0/1/2 mapping) and is counted too:
`SetPosition SetWarpSpeed ResetWarpSpeed Patrol SearchAndDestroy PointAt
Special_Attack Attack(unit, Vector) SetSpecialWeaponAttackInterval AI_Attack
AttackMainEnemyBase SetShieldPercent SetMaxShields DestroyCritical Destroy*
AllowMultipleSpecialWeaponFire SetSpecialForcesFlag TransferFlag LinkWormHoles
EnableWormholeTravel ObstacleAvoidence PauseSimulation UnpauseSimulation
EnableLight ReplaceObject RecruitSpecialForces SetRace SetWormholeTeam
SetOfficers AddOfficers SetTeamCrew AddCrew SetCrew DisableAllAITeams
EnableAllAITeams DisableTeam EnableTeam SetIsAITeam`.

Queries raise an error naming the missing data (also `strength_team_1`, `strength_team_2`, `hot_float_value`): `InStopMode InAttackMode
OutOfSpecialAttackMode InQueue GetAttackLocation UnderAttack
GetAllyShipAttacks GetAllyStationAttacks GetDilithiumOnFreighter
IsCompromised IsDerelict IsThereARomulanSpyOnShip GetTranswarpExitWormhole
GetNearestTranswarpGate`.

`AI_Team_Load_AIP` queues the file like `Player:loadAIP` for the host's AI seats (AI-06).
`CineractiveStart/End` set the shared `Cineractive.active` singleton.
Special flags (`CreateSpecialFlag` ...) are tracked in the runtime only.

No stock mission was run through this table; the tests use fake content.

`GroupNew`, `GroupDelete` and `Add_Team_To_Group` are runtime bookkeeping only.
`AddDilithium`/`SetDilithium` chain within a tick (writes are absolute; the view is a snapshot).
