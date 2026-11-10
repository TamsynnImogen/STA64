# LUA-05 presentation milestone

`Cineractive`, `Media` and `UI` now queue `Edit::Present` commands from
`PlayerId::SCRIPT`. The sim raises the matching events; these bindings do
not render a shot/title or play a sound/movie themselves.

Call-shape evidence: Immy's clean-room `armada_missions` A2 ports (including
*Along the Neutral Zone*, *Into the Breach* and *UI Tutorial*) and the local
`A2FOMissions/missions/Recovery.lua`. No MMM implementation was copied.

- `Cineractive.active` and `.skippable` are readable/writable booleans.
- `:lookFrom(entity | Vector | Path(name), ...)` retains the source and
  `:lookAt(...)` completes a simple entity/vector pair with a zero-duration
  `Shot`. A new source does not produce a shot toward an earlier target.
  Other requests, including paths and extra arguments, stay generic `Call`
  events without adding an approximate shot. The number seen with paths is not
  interpreted as a duration or speed. `Path` currently keeps only a name.
- `:setCameraOffset(Vector)` produces `CameraOffset(relative=false)`;
  `:fadeIn(seconds)` / `:fadeOut(seconds)` fade from/to black. These are
  explicit bridge defaults. Unobserved overloads return errors.
- `Media:playSound(name)` returns a bridge sound id; `:stopSounds()` stops
  all sounds this runtime requested, in request order. `:playMusic(name)` /
  `:stopMusic()` and `:playMovie(name)` / `:stopMovie()` send typed events.
  Music defaults to looping and movies to skippable/fullscreen; native
  defaults and return-value compatibility have not been verified.
- `UI:addTitle(text)` / `:addSubtitle(text)` send typed titles. The observed
  `(text, x, y, seconds, boolean)` forms send `Call("UI.addTitle" |
  "UI.addSubtitle", [id, text, x, y, seconds, boolean])`. Placement and the
  final boolean are retained without guessing their interpretation. UI
  returns no handle. There is no evidenced title-removal API in this slice.
- Unknown methods retain permissive stubs and increment `unsupported()`;
  malformed calls to implemented methods return Lua errors.

The asset-free intro test drives a real `Sim`, reaches a synthetic first
objective, and replays its recorded presentation commands. It does not run
a full converted stock mission. Remaining LUA-05 gates include the
DLL-named `A2` table, mission-object lifecycle beyond the setup/update bridge,
unverified Timer overloads and other missing MMM APIs, full converted-mission
tests, and client-side presentation/completion feedback. The package remains in progress.


## File-backed objectives and external validation

`Objectives:load(name)` (or `Objectives.load(name)`) reads through the same
caller-supplied content reader as the mission and its includes. The bridge
tries `objectives/name` and then the relative name. The provisional parser
accepts UTF-8 text
with an optional BOM, normalizes line endings, reads the first nonempty
line as the heading and each `*` line as a goal, and retains the briefing
and non-goal notes for host inspection. Missing files record an unsupported
call and preserve the previous list; malformed files return a Lua error
without partly replacing it. Assets remain outside the source tree.

Goals support one-based indexing and `.state = true` for completion, as
used by the converted ports; explicit active/complete/failed state names
are also accepted. Reloading replaces the list with fresh IDs, so handles
from an earlier file cannot accidentally modify a new goal. The runtime's
objective list and briefing remain host-only state; client display,
replication and objective presentation are still pending. `Player:loadAIP`
records the requested file and an unsupported diagnostic; it does not run AI.

The ignored `tests/a2_port_intro.rs` probes accept `STA64_A2_PORTS` pointing
to the external ports' `a2/missions` folder and `STA64_A2_OBJECTIVES`
pointing to an installed objectives directory. The first-goal probe loads
Into the Breach unmodified, uses synthetic craft, and places the Enterprise
inside the script's discovery region to exercise its actual trigger.
It proves Lua scheduling, file goals and state changes, not native map
movement, complete mission play, or presentation playback. A separate
ignored probe loads every text file in the supplied objectives directory
without embedding any game text in the repository.
