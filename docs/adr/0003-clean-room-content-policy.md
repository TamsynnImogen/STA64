# ADR 0003: Clean-room content policy

- **Status:** accepted
- **Date:** 2026-11-07 (first "Legal boundary" text, foundation commit e25f1c1); extended 2026-11-13 (#469, research and game assets) and 2026-11-13 (#636, mission conversion); clarification on engine lookup names, decision of Cedric as an easily reversible one, in use from 2026-11-28
- **Deciders:** "Legal boundary": not recorded. Research and game-asset rules (#469): not recorded (the commit message names no approver). Mission conversion rule (#636): decision of Cedric, agreed by Tammy on 2026-11-13 (commit message of #636 and the WORKPLAN note under the LUA table)
- **Relates to:** [AGENTS.md](../../AGENTS.md) ("Research and game files", the operative wording for agents), [ARCHITECTURE.md](../ARCHITECTURE.md) ("Legal boundary"), [MISSIONS.md](../MISSIONS.md), WORKPLAN LUA-15

## Context

The README states: "STA64 is a clean-room engine. It ships **no original game content**: it reads models, textures, ODFs, maps and sounds from a user-owned Armada II / Fleet Operations install at run time."

The "Legal boundary" section of ARCHITECTURE.md was part of the first commit of that file (2026-11-07).

#469 (2026-11-13) added the AGENTS.md section on research and game assets: no research write-ups, no game assets in any form.

#636 (2026-11-13) recorded that stock missions are converted on the player's machine and that no mission content is in the repository. A follow-up commit in the same PR records Tammy's answer: she agreed to conversion on the player's machine provided it converts the missions at a sensible rate, and considers distributing Lua mission scripts acceptable as the fallback.

## Decision

From ARCHITECTURE.md, "Legal boundary":

- No original game binaries, art, audio, data or decompiled code in this
  repository.
- Behaviour is documented from observation and from our own analysis notes,
  then reimplemented. Addresses and reverse-engineering notes stay in the
  analysis repositories, not here.
- Third-party code, such as MIT-licensed Armada II Refit code, may be reused
  with its licence notice kept.

From AGENTS.md, "Research and game files":

- Decompilation findings and measurements of the original games are not
  committed unless STA64 needs them to work (a data table the code reads, a
  test fixture you authored yourself). Keep them outside the repository, in a
  separate research directory on your machine. Code comments, WORKPLAN rows and
  PR bodies state the rule STA64 follows, the data keys it reads and the
  evidence grade (measured / read from data / read from the executable, static / not measured), in a line or two:
  no `docs/research/` write-ups, no measurement or bench run tables, no
  pseudo-code, addresses or disassembly, no bench run logs.
- Never commit game assets or anything copied or converted from them, in any
  form: no textures, models, sounds, movies, maps, ODF/cfg/sprite/tech-tree
  files or excerpts beyond a key name and a value, no frames showing game art,
  no re-encoded copies. Tests that need them are install-gated and read the
  install at run time.
- Stock mission scripts, whether converted from the mission DLLs or ported by
  hand, count as game content and are never committed or released. Conversion
  happens on the player's machine from their own install into the user data
  directory (WORKPLAN LUA-15 and the note under the LUA table).

AGENTS.md carries the operative wording; where the two texts ever differ, AGENTS.md is followed until this ADR is superseded.

### Clarification 2026-11-28: engine lookup names and fixed file paths

*Records a decision taken by Cedric (`cedrichaase`) on 2026-11-28 as an easily reversible one (AGENTS.md, "Decisions: ADRs and experiments": one revert removes the constants, and no save, replay, protocol, content or mod format depends on them). It is in use from that date and is recorded here so that this ADR stays true. Tammy (`TamsynnImogen`) is informed through the pull request that records it and is free to object; if she does, the constants are reverted. Reversal: revert the constants and the AGENTS.md sentence that goes with this note, and mark this note withdrawn.*

Context: under the Stock look STA64 must draw the original's effects. Some effects are defined in the player's own install, but the name by which the original engine selects the definition exists only in the engine, not in any data file. Without that name the code cannot find the entry. The first case is the spark drawn when a weapon hits a bare hull (issue #1153). `main` already carries one such name as the constant `IMPACT_SPRITE` in `crates/sta64/src/render/fx/stock_flares.rs`, for the beam impact effect.

Clarification: a fixed lookup name by which the original engine selects an entry that the install's own data defines may be a named constant in STA64's code, where the code needs it to find that entry at run time. Each such constant has a one-line comment with the rule and the evidence grade, and a mod may define the entry under that name (as the comment on `IMPACT_SPRITE` does).

Limits:

- This covers lookup names and fixed file paths only (see the extension below). It does not cover the definitions themselves, values, or any other names from game assets (units, maps, missions, files, textures); those stay under the rules above.
- The names and paths appear in code only: not in issues, pull request bodies, review comments, commit messages, docs, WORKPLAN or TODO.md.

Extension, same day (the same decider): the same exception holds for a fixed file path by which the original engine opens data that the player's install provides (by kind: a shell background picture, an intro movie). It may be a named constant in STA64's code only, where the code needs it to open that file at run time. Each such constant has a one-line comment with the rule and the evidence grade. Where the path is specific to one game or profile, it must be identifiable which one, by the constant's name, its module or the comment. The file itself and anything read from it are never committed, and a mod may provide its own file under that path. `main` already carries such constants for the intro movies, and the campaign-screen work (draft PR #1374) depends on this extension.

Second extension, same day (the same decider, issue #1377): a description, in STA64's own words, of how a stock mission plays (what the player is asked to do, in what order, what ends it) is allowed in the repository, as hand-written playthrough plans for tests and as descriptive lines in docs. Still excluded: names from game files (units, maps, files, objects), measured thresholds and timings, map positions, caption and dialogue texts, and anything copied or converted from the mission files. A plan describes the play; it does not reproduce the script. The converted scripts themselves stay out as before.

Alternatives considered and not chosen:

- Finding the entries by a name pattern at run time: the pattern itself would come from the engine, and it is fragile.
- A mod-facing key written on the player's machine at first-run import: clean, but a new mechanism for a handful of names.

Consequences: a follow-up package draws the hull-hit effect under the Stock look once the dying phase (#1100) is in; it waits for nothing else. AGENTS.md, "Research and game files", carries one matching sentence with the same limits.

## Alternatives considered

- Research and game-asset rules: None recorded.
- Mission scripts: the WORKPLAN note under the LUA table (which AGENTS.md cites) names as rejected executing the mission DLLs directly (it needs an x86 interpreter plus the engine call interface on every platform, little gain over conversion) and shipping hand-written scripts made from decompiled logic. Hand-written ports remain possible only as a separate download maintained outside the repository and the release. The fallback if conversion does not prove out is distributing Lua mission scripts, which Tammy considers acceptable (commit message of #636); the WORKPLAN note adds that shipping Lua ports would need a new decision by Tammy and Cedric.

## Consequences

- Tests that need game files are install-gated and read the install at run time.
- Converted mission scripts are written to the user data directory (`<user data dir>/sta64/converted/<key>/`, MISSIONS.md), not to the repository or an install; the tool refuses output inside either.
- STA64 never loads `.drl`, `.dsl`, `.dll` or `mmm_loader.dll` (MISSIONS.md, "Background"); legacy compiled scripts reach the player by conversion to Lua on their own machine from their own install, never committed or shipped.
- Stock-mission conversion is currently an interim Python route; the release target is a Rust port run by the game (LUA-15).
