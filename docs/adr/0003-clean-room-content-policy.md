# ADR 0003: Clean-room content policy

- **Status:** accepted
- **Date:** 2026-11-07 (first "Legal boundary" text, foundation commit e25f1c1); extended 2026-11-13 (#469, research and game assets) and 2026-11-13 (#636, mission conversion)
- **Deciders:** "Legal boundary": not recorded. Research and game-asset rules (#469): not recorded (the commit message names no approver). Mission conversion rule (#636): decision of Cedric, agreed by Tammy on 2026-11-13 (commit message of #636 and the WORKPLAN note under the LUA table)
- **Relates to:** [AGENTS.md](../../AGENTS.md) ("Research and game files", the operative wording for agents), [ARCHITECTURE.md](../ARCHITECTURE.md) ("Legal boundary"), [MISSIONS.md](../MISSIONS.md), WORKPLAN LUA-15

## Context

The README states: "STA64 is a clean-room engine. It ships **no original game content**: it reads models, textures, ODFs, maps and sounds from a user-owned Armada II / Fleet Operations install at run time."

The "Legal boundary" section of ARCHITECTURE.md was part of the first commit of that file (2026-11-07).

#469 (2026-11-13) removed decompilation findings and original-game measurements (`docs/research/`, `docs/data/collision/`, `tools/research/`) from the repository; they now live in a separate local research directory. Kept docs and two code comments were trimmed to the rule, the data keys and an evidence grade, and AGENTS.md gained the section on research and game assets: no research write-ups, no game assets in any form.

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

## Alternatives considered

- Research and game-asset rules: None recorded.
- Mission scripts: the WORKPLAN note under the LUA table (which AGENTS.md cites) names as rejected executing the mission DLLs directly (it needs an x86 interpreter plus the engine call interface on every platform, little gain over conversion) and shipping hand-written scripts made from decompiled logic. Hand-written ports remain possible only as a separate download maintained outside the repository and the release. The fallback if conversion does not prove out is distributing Lua mission scripts, which Tammy considers acceptable (commit message of #636); the WORKPLAN note adds that shipping Lua ports would need a new decision by Tammy and Cedric.

## Consequences

- Tests that need game files are install-gated and read the install at run time.
- Converted mission scripts are written to the user data directory (`<user data dir>/sta64/converted/<key>/`, MISSIONS.md), not to the repository or an install; the tool refuses output inside either.
- STA64 never loads `.drl`, `.dsl`, `.dll` or `mmm_loader.dll` (MISSIONS.md, "Background"); legacy compiled scripts reach the player by conversion to Lua on their own machine from their own install, never committed or shipped.
- Stock-mission conversion is currently an interim Python route; the release target is a Rust port run by the game (LUA-15).
