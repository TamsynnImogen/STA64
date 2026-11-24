# STA64 closed alpha — Testbed

Thank you for testing. This is a **closed alpha**: please don't share the files
or post footage without asking first.

STA64 is a new engine for Armada II / Fleet Operations style games. This bundle
runs on its own: **no game install is needed**. It contains the engine and the
*Testbed*, a test mod with two factions and one test map per group of engine
systems.

## Start

- **Linux:** `./run-testbed.sh [Group]`
- **Windows:** `run-testbed.bat [Group]`

`Group` picks which systems are switched on (default `Sandbox`, everything):

| Group | What it switches on |
|---|---|
| `Combat` | weapons: damage, fire arcs, turrets, shot timing, hit rolls, target masks |
| `Defence` | shields and point defence |
| `Behaviour` | how units pick targets and help each other without orders |
| `Economy` | resources, officers, paying for builds |
| `Production` | yards, construction rigs, upgrade pods, station rotation |
| `Fleet` | breeders, experience, squadrons, wrecks, ship names |
| `Sandbox` | all of the above |

Then **Single Player → Testbed Scenarios** and pick the map for your group.
Each briefing says what to try and lists the group's systems. Systems marked
**"not expected to work yet"** have no engine code behind them so far: no need
to report that they do nothing.

You play the **Terran Union**; the enemy is the **Halcyon Accord**. Instant
Action / skirmish works too, with either faction.

## Cheats (single player only)

Press **Enter**, type the code, **Enter** again: `showmethemoney` (resources),
`expl` (destroy selected), `elim`, `dis` (drop shields of selected), `fps`,
`version`. **Alt+F** / **Alt+V** lift fog / shroud.

## Reporting a bug

1. When you see it, press **Ctrl+F12** in the match. A notice says
   "Capture saved: <folder>". The folder holds a replay, a snapshot and what was
   selected, so the bug can be replayed exactly.
   - Linux: `~/.local/share/sta64/captures/`
   - Windows: `%LOCALAPPDATA%\sta64\captures\`
2. Zip that folder and send it with: the group you ran, the map, what you
   expected and what happened. A screenshot helps, too.
3. Crashes: send the terminal output (Linux) or the console window text.

## Signed files

The `.sta` archives are signed with the testbed's key; its public half is in
`STA64-Testbed-signing-key.txt`. Every file inside an archive also carries a
SHA-256 checksum, and STA64 refuses a file whose checksum does not match.
(STA64 does not show signatures in game yet; the developers check them with
the `sta-archive` tool.)

## Credits

See `CREDITS.md`. Star Ruler 2 art © Blind Mind Studios, CC-BY-NC 2.0
(non-commercial). Earth Conflict models by Immy. Sounds and skies made for the
testbed (CC0).
