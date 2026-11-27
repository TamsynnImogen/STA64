# ADR 0010: Save and load

- **Status:** proposed
- **Date:** 2026-11-27
- **Deciders:** the project lead and the repository owner (the pull request that records it).
  The file was merged as proposed in #1275. The recommendations of that pull
  request were accepted by the project lead on 2026-11-27; the repository
  owner's decision is pending. Accepted when both have approved (the pull
  request that records the decisions); not binding before.
- **Relates to:** issue #1032; [ADR 0002](0002-deterministic-simulation.md); [ADR 0003](0003-clean-room-content-policy.md); [ADR 0004](0004-modding-v1.md) D10; [ADR 0005](0005-network-after-net-01.md) N2, N5; WORKPLAN SIM-10, SIM-11, SIM-61, UI-20, LUA-15, CON-40, CON-57

## Context

Players ask for Save game and Load game in the in-game menu (#1032). A save
is a file players keep on disk and pass around, so what it promises, what it
contains and how it restores a mission are hard to change once saves exist.

What STA64 has: the sim snapshot (SIM-10) holds the whole simulation and
refuses any format but its current one and the one before; the replay
(SIM-11) holds the setup, every command and digests, and is checked by
re-running the sim; network resync loads a host snapshot and the turns after
it. Controllers (the skirmish AI, the mission AI hub and the mission's Lua
script) run on the host only and are in neither: replays record their
commands and never run them again (WORKPLAN assumption D3).

What a save needs beyond the snapshot:

- **Game identity:** map, mod chain, seats, options, seed, local seat. In the
  replay header and the capture index; not in the snapshot.
- **AI state:** plain Rust data (queues, waves, its own generator, the mission
  plans and switches). Serialisable with work.
- **Client state:** explored fog, camera, control groups. Small and plain.
- **Mission script state:** the Lua VM (globals, upvalues, class instances,
  coroutines), callbacks held as Lua function handles, and the runtime's own
  records (objectives, timers, monitors, cineractive state, the script's
  random stream). Nothing of it can be serialised today, and Lua has no
  state persistence of its own. This is the hard part.
- **Identity the content digest does not cover:** the map file, the mission
  script and its includes, the AI plans, the converter version of a converted
  script.

The formats move fast: the snapshot format went from 1 to 51 in its first
19 days (49 commits changed the number), the network protocol number changed
in 22 commits and the replay format in 7. The one migration
that exists (snapshot 50 read by 51) needed its own content-hash variant.

The original games save single-player games from the in-game menu and load
them from there and from the single-player menu (stated in the Armada II
manual); their missions save their own variables through save callbacks
(read from the executable, static).

## Decision

**Scope: skirmish saves first.** The first saves are single-player skirmish
saves (Instant Action against computer players), saved and loaded by hand;
mission saves follow once V4's conditions hold (see "Not in v1"). The
container, the controller state and the client's load path are built for the
skirmish and reused by missions.
*Accepted by the project lead on 2026-11-27; the repository owner's decision
is pending.*

**V1. Saves are valid only for the formats and content they were made with.**
A save loads in a build that reads its container version, its snapshot format
and seam revision, with a content database of the same hash, the same map
file and, for a mission, the same script identity. Anything else is refused
before loading, with a message that names the first mismatch (older STA64,
other content, map changed, script changed, damaged file). No migration code
is written for any format. During development most updates will therefore
make older saves unloadable; the load list says so instead of failing.
*Rejected:* migration per format (one reader and fixture per bump, at the
current rate several a week, for a feature that is not the product);
loading on a content mismatch (D10; the sim reads content by index).
Revisited by a later ADR when the formats are declared stable.
*No migration, refusal with the reason: accepted by the project lead on
2026-11-27; the repository owner's decision is pending.*

**V2. A save is its own container with its own version number.**
A fixed header, readable for the load list without parsing the rest: magic,
container version, the snapshot and replay formats and seam revision it
holds, an informational build id, the content hash and provenance, profile,
install and mod key, the map path and a hash of the map file, game kind,
seats, options, seed, local seat, tick, play time, the time it was made and
the player's description; for a mission also the campaign book digest, the
mission, the script path, one hash over the script and every file it
included, and the converter version of a converted script. Sections follow:
the sim snapshot, the controllers' state, the client's state and, for a
mission, the journal (V4). The container number moves independently of the
snapshot, replay and protocol numbers.
*Rejected:* appending fields to the snapshot (every resync and bench file
would carry them); a JSON sidecar beside a snapshot (two files that can be
separated).

**V3. Where saves live and what they may contain.**
One folder, `saves/` in the user data directory; every save is listed, those
this install cannot load greyed out with the reason. File names are
generated; the description lives in the header. Writes are atomic. A save
holds game state only (positions, labels, orders, command arguments, as
snapshots and captures already do), never script source, text files,
strings read from game files, map data, movies, images or any other asset;
those are referenced by path and hash and read from the player's install at
load (ADR 0003). No thumbnail in v1.
*Rejected:* a folder per mod (a save already names its content, and the list
can explain a refusal); a thumbnail (an image rendered from game art in a
file players share).
*No thumbnail in v1: accepted by the project lead on 2026-11-27; the
repository owner's decision is pending.*

**V4. A mission's script state is restored by re-running it, not by serialising Lua.**
A mission save carries the game's full journal from tick 0 (the replay, plus
every input the client gives the script). Load builds the game from tick 0
without drawing it, runs the sim with the mission script and the AI attached,
checks every command they issue against the journal and every digest against
the recorded one, and hands over to the player at the saved tick. Any
difference refuses the load; it never continues on a state that differs.
Mission saves are not enabled until (a) every client answer to a script
travels in the command stream (SIM-61), and (b) a measurement shows the
installed and converted missions issue the same commands in two separate
processes (Lua seeds its string hashing per process).
*Rejected:* serialising the Lua state (no persistence in Lua 5.3; would need
a patched VM and a persistence table for every binding, and would tie saves
to the VM's internals); a state contract declared by the script, as the
original missions do (possible later as a faster load for converted
missions, since STA64 writes the converter; it cannot cover hand-written
mission scripts).
*Accepted cost:* load time grows with the time played.
*Re-run over serialising Lua: accepted by the project lead on 2026-11-27; the
repository owner's decision is pending. Open questions 1 and 2 below were
decided with it.*

**V5. Skirmish controllers are saved as data.**
The skirmish AI and the mission AI hub (per-seat flags, loaded plans by file
name, the pending replan) are serialised into the controller section and
restored so the game continues as an uninterrupted one would.
*Rejected:* new controllers after a load (they plan afresh, so the game
diverges from the one saved).

**V6. Multiplayer save is not part of this decision.**
ADR 0005 N2 already keeps it out of scope. It would need every peer to agree
on the save's content and the host's controllers to survive a reload (the
problem N5 names for resyncing the host), which V4 and V5 solve for one
machine first.

**V7. Saving is refused where the state cannot be restored.**
In a networked match, in replay playback, as an observer, and while a
cineractive or movie plays or a script waits for a player answer, the menu
shows Save as not allowed with the reason. The originals have a "save not
allowed" state (read from data: a menu caption key).

## Consequences

- A loaded game must continue the replay log from tick 0 (for a skirmish save
  the journal is kept as well, so a capture of a loaded game still replays).
- A save of a game that was itself loaded is just a later save of the same
  journal.
- Round-trip tests become part of the done-when of every save step: save at
  tick n, load, run to tick m; the digest and the controllers' commands equal
  those of an uninterrupted run. Refusal tests for each mismatch named in V1.
- Every container, snapshot or replay change that touches saves needs the
  `full-ci` label (file format).
- Importing the original games' saved games (WORKPLAN CON-40, CON-57) is a
  separate track; nothing here depends on it.
- Campaign progress keeps its own file; finishing a loaded mission applies its
  result as finishing it unloaded would.

## Not in v1

- Mission (campaign) saves: after SIM-61 and the reproducibility measurement (V4).
- Autosave, quicksave and save slots bound to keys.
- Loading a save in another STA64 version or with other content (V1).
- Multiplayer save and load (V6).
- Thumbnails (V3).
- Saving during a cineractive, movie or scripted wait (V7).
- Reading the original games' saved games.
- Cloud or cross-machine sync of saves.

## Open questions

1. V4 (b): if the measurement shows commands depend on per-process hashing,
   is fixing the seed in the bundled Lua build acceptable, or do mission saves
   wait for the script-declared state of the rejected alternative?
   **Decided 2026-11-27 (the project lead; the repository owner's decision is
   pending):** fix the seed in the bundled Lua build first, measure again, then
   decide.
2. Is a load time of minutes for a long mission acceptable for v2, with a
   progress display, or is a bound on it a condition for enabling mission saves?
   **Decided 2026-11-27 (the project lead; the repository owner's decision is
   pending):** acceptable for v2 with a progress display; a bound is revisited
   once the load time has been measured end to end.
3. V5 and V4 on the mission AI hub: V5 serialises the hub (per-seat flags,
   loaded plans by file name, the pending replan) into the controller
   section, while V4's re-run from tick 0 rebuilds it with the script. Is the
   hub's record kept (and checked against the re-run state at the hand-over),
   or is V5's hub clause amended to "rebuilt by the re-run"? Until decided,
   the WORKPLAN rows hold to V5 as written. Raised by the review of the save
   packages' WORKPLAN rows (#1315).
4. Matches whose map has no file (a generated random map, an unsaved map
   started from the editor) have no map path and file hash for V1 and V2, and
   V3 keeps their map data out of a save. Until decided, saving them is
   refused with that reason. One option that fits V3: record the generator's
   inputs as game state in the header and regenerate the map at load.
   Raised with question 3.

## Evidence since the first draft

Facts gathered after this draft was written; the decisions above are unchanged.

- **Sim state (PR #1307, merged 2026-11-27).** A tests-only change
  (`crates/sta_sim/tests/save_round_trip.rs`) runs authored scenarios
  (navigation round obstacles, a formation group, queued orders, combat across
  the save point, random-number state, production, resources, the stuck-order
  watchdog, visibility, one long mixed run) three ways: uninterrupted, by
  snapshot into a fresh sim, and through a file. It found no sim state lost
  (its own runs, about 15 s for the file). It does not cover controller state,
  mission Lua state or client state.
- **Mission Lua (issue #1300, open PR #1303, in review).** An experiment with
  authored scripts showed mission Lua is not reproducible from one Lua state to
  the next (table traversal order, sort ties, weak tables, printed references).
  PR #1303 makes it reproducible by vendoring and patching the Lua runtime
  (ordered traversal, stable sort, no weak tables, stable reference printing).
  This is a precondition for re-running a mission on load (V4 b). Whether the
  vendored runtime needs an ADR of its own is for the deciders.
- **Skirmish AI.** Its state has no serialisation today, and its folder is under
  an open claim of the repository owner. Saving a skirmish with AI seats (V5)
  is blocked until that claim is released or shared.
