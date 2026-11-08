# Test bench

STA64 needs a way to put a known situation on screen or into the sim, run it
for a known number of ticks, and check what happened: one ship beside one
planet, one weapon firing, thirty ships selected. Armada II Refit (MIT, same
author) already has such a bench for the stock 32-bit game. This plan takes
its interface and what it learned, and none of its machinery.

## Goals

- Load a **scene** (map, objects, camera, seed) straight into the engine,
  headless or windowed, and drive it with **commands** (orbit, pause, select,
  kill, heal, goto, ...).
- Run **scenarios** (plain-text steps and checks) as ordinary tests, in
  parallel, with a report per run.
- Check the **sim first** (world state, per-tick digest); pixels only where
  rendering is under test, from **a frame that repeats**.
- Keep stock as the **oracle**: one scene file runs on stock A2/FO under
  Refit's bench and on STA64, and the two are compared.

## Non-goals

- Driving STA64 through OS-level input, OCR or click-by-text.
- Running STA64 under Wine, in install clones or under a headless compositor.
- Shipping game-derived frames, baselines or scenes that embed game data.
- Replacing `cargo test`: the bench produces `cargo test` cases and a runner
  for the cases that need a process.

## What is reused from Refit, and what is not

Refit's bench is ~15k lines of Python, shell and C. Almost all of it exists
because Refit does not own its engine: it has to reach inside a closed
32-bit executable from outside. STA64 owns its engine, so that half goes.

| Refit | Here | Why |
|---|---|---|
| Per-case install clones, Wine prefixes | dropped | STA64 never writes the install; the VFS is read-only |
| Headless sway / Xwayland, noVNC | dropped | Bevy runs without a window; a windowed session is just `sta64` |
| Virtual mouse and keyboard, click-by-text, OCR waits | dropped | commands go through an in-process channel; UI tests inject Bevy input events |
| Game slots (~5 concurrent games) | dropped | one process per case; `cargo test` / the runner's job count |
| `Scene.asi` injection, symbol profiles, hooks that stand down | dropped | the scene loader is part of the engine |
| Fixed waits ("Wait 45 seconds") | dropped | steps run a number of ticks; time is the sim's |
| Scene catalogue and `scenes/*.ini` format | **kept** | `testbench/scene/README.md`, "The scenes", "Scene files" |
| Command vocabulary | **kept** | `testbench/scene/README.md`, "Commands" |
| `objects` list and marks | **kept**, reshaped | "Objects", "Marks" in the same file |
| Scenario step grammar | **kept**, subset | `testbench/scenarios/README.md` |
| Animation clock rule | **kept** | `render3d/README.md`, "The animation clock"; `testbench/README.md`, "A frame that repeats" |
| Session IDs, report layout | **kept**, thin | `testbench/README.md`, "Which session a command acts on" |
| Refit's bench itself | **kept**, as the stock oracle | see "Parity with stock" |

## Scenes

A scene is Refit's `.ini` format, read by a Bevy-free parser. Refit's
`scenes/firing.ini` and `scenes/planet.ini` are the first two; scene files are
MIT and hold only ODF names and numbers, so they can be copied into
`bench/scenes/` with Refit's licence notice, as `a2fmt` was vendored.

`[Scene]` keys keep their names and meanings. `[Object.<name>]` keeps `Odf`,
`Team`, `X`/`Y`/`Z` (Y up), `Heading` (degrees, 0 faces +z), `Immortal`,
`Heal`, `Energy`, `Tech`, `Engines`, `Weapons`, `Vent`, `Attack`,
`Population`, `Colonist`. `[Marks]` keeps `Enable`, `FireGap`, `Still`.

What is tied to the old engine and changes:

| Refit | STA64 |
|---|---|
| Always the map the game was launched on (`-nointro a2_borg01`); objects outside its quadrant are not drawn | new key `Map=`: a BZN from the install, or STA64's built-in empty map (default). Parity runs set `Map=a2_borg01` |
| `Delay=30`: mission ticks before the scene is built, so the engine settles | accepted; default 0. The scene is built on tick 0 after the map loads |
| `Step=<ms>`: forces the engine's variable tick to a fixed length | the sim tick is always 1/`TICK_HZ`. `Step=` becomes the **render frame** length under the bench clock (frames per tick, and so interpolation, fixed) |
| `Pause=<tick>`, `pause-at`, `tick`: counted in the old engine's ticks | counted in STA64 ticks. Cross-engine scenes use seconds: `Pause=3.2s` (see open questions) |
| `Seed=`: reseeds the engine's random sequences | seeds `Sim::new(seed)` and the cosmetic seed. Same intent, different streams: never compare random outcomes across engines |
| `#<id>`: an engine object handle | `#<id>`: the sim's `EntityId`. Ids are given in order, so `#3` names the same object on every run |
| `Hud`, `Grid`, `Cursor`, `Notices`, `Tooltips`, `Hover`, `ShieldFx`, `Music`, `Captions`, `Race` | the same switches on STA64's own systems, as each system lands. Until then the key answers `! unsupported: <system>` in the log, like Refit's stood-down hooks, and the scene still loads |
| `Enable=0` (patch nothing but music/captions) | dropped |
| Profile-dependent behaviour (a hook stands down on Roots) | none. A scene may name a content profile (`Profile=FO4`) instead |

## Commands

Commands are the vocabulary of `testbench/scene/README.md`, "Commands", with
the same names, arguments and, for the shared subset, the same answer text
(`paused at tick 200`, `objects N -> objects.jsonl`), so `Expect scene "..."
answers "..."` steps port unchanged.

They split into three kinds, by how they reach the sim:

- **Orders** (`goto`/`move`, `attack`, `stop`, `build`, `formation`,
  `special`, `cloak`): the same `sta_sim::Command`s a player's input makes.
  They go into the command log, so a run replays.
- **Bench edits** (`spawn`, `kill`, `damage`, `heal`, `immortal`, `engines`,
  `weapons`, `energy`, `vent`, `tech`, `cargo`, `colonize`, `team`, `seed`,
  `succeed`, `fail`): a `Command::Bench(..)` variant, also in the command log
  and the digest, so an edited run replays too. Only a bench build accepts
  them; lockstep peers reject them.
- **View and query** (`orbit`, `glide`, `spin`, `camera`, `camera on`,
  `center`, `zoom`, `select`, `select+`, `selection`, `query`, `objects`,
  `marks`, `pause`, `resume`, `pause-at`, `tick`, `hud`/`grid`/...): never
  touch sim state. Moving the camera while paused costs nothing: Refit's
  "a camera move while paused runs 3 ticks" does not exist here.

Dropped: `windowed`, `waypoint-key` (workarounds for the old engine and
Xwayland).

**When a command lands.** Refit's commands land on whatever tick is running
when `Scene.cmd` is read, which is why "nothing by hand before the pause".
STA64 applies every queued command at the next tick boundary and logs the
tick. A command may also name it: `at 120 kill target`. A scenario that only
uses `at` or runs a fixed number of ticks between commands is deterministic.

**Objects.** `objects` writes `objects.jsonl` with Refit's schema (header
line, then one object per line: `id`, `name`, `odf`, `class`, `team`, `mine`,
`kind`, `pos`, `hull`/`shields`/`crew`/`special` as `[now, max]`,
`selected`, `group`, `dying`). The header's `build` becomes STA64's version
and content profile; `label` is the BZN label. The same schema on both
engines is what makes the parity diff a plain tool.

**Marks.** Refit polls the game once a frame and stamps marks with the wall
clock and an epoch line. STA64 emits them from the sim's own events (fire
start/end per weapon burst, death, cloak state, selection) and stamps them
with the tick, plus the wall time only for footage. Kinds and fields keep
Refit's names (`fire.end obj=shooter`, `cloak.done obj=warbird`).

## Architecture

```
  bench/scenes/*.ini        bench/scenarios/*.md
          |                          |
     sta_bench (no Bevy)  ------------------------------------
     scene parser · command parser · scenario parser · report
          |                          |
   in-process runner           sta-bench runner (binary)
   sta_sim + content DB        spawns  sta64 --scene ... --headless
   (cargo test, no GPU)        one process per case, frames, render marks
```

- **`sta_bench`** (new crate, Bevy-free): `SceneFile`, `BenchCommand` and
  `Scenario` types and their parsers; building a scene into a `Sim` through
  the content database; the answer text; `objects.jsonl` and marks writers;
  report writers. Tests that need only the sim run here, in-process, as
  ordinary `cargo test` cases.
- **`sta64 --scene <ini>`**: loads the map and the scene, then runs. Flags:
  `--headless` (no window; renders to an image target only when a frame is
  asked for), `--sim-only` (no renderer: `MinimalPlugins`), `--clock bench`,
  `--ticks N` (run N ticks and exit), `--control stdio|socket` (the command
  channel). A `bench` cargo feature gates the bench edits.
- **Command channel**: a Bevy resource holding a queue of `BenchCommand`s,
  filled by the scene file at load, by stdin or a local socket in a live
  session, and by the runner. Answers go back one line per command and into
  `bench.log`.
- **`sta-bench`** (binary in `sta_bench`): `run <scenario...>`, `check`
  (parse and show steps, no launch), `list`, `session start|stop|list`,
  `drive --session ID "<cmd>"`, `diff` (frames, object lists).

**Two tiers.** Most checks (orders, combat, economy, AI, missions) are
**tier 1**: `sta_bench` + `sta_sim` in-process, milliseconds per case, run by
`cargo test`. Only rendering checks (frames, visibility, culling,
interpolation) are **tier 2**: a `sta64 --headless` process per case,
scheduled by `sta-bench run --jobs N`. Parallelism is processes and test
threads; there are no slots to manage.

## Scenarios

Scenarios keep Refit's plain-text form (`testbench/scenarios/README.md`): a
markdown file, a `# Title`, header lines, prose as description, one step per
numbered line, `## ` phases, `Include "_fragment"`, `Note "..."`.

| Kept | Changed | Dropped |
|---|---|---|
| `Scene "CMD"`, `Expect scene "CMD" answers "TEXT"` / `does not answer` | `Setup: .../bench-setup.sh <scene> Key=v` → header `Scene: firing Seed=7 Pause=200` | `Launch`, `Quit`, `Stop the game` (the runner owns the process) |
| object phrases: `Expect 3 ships on team 1`, `Wait until no ship is on team 2`, `Select the ships of class "Galaxy"`, `Order ... to attack ...` | `Wait up to N seconds until ...` → bounded in sim seconds or ticks | OCR waits, `Click`, `Drag`, `Press`, `Type`, focus, bar, window steps |
| `Take a screenshot called "NAME"`, `Dump the frame called "NAME"`, `Expect shots "A" and "B" to match within X` | `Wait N seconds` → `Run N ticks` / `Run N seconds` (sim time, no wall clock) | `Resolutions` aspect sets (a single `Size: 1280x720` for tier 2) |
| `Mod:`, `Timeout:`, `Clock:` headers | `Mod:` → `--mod`; `Clock:` defaults to `bench` | `Stock shell`, `Assets`, `Overlay`, `Layers`, `Players` (until M5) |
| `Expect no crash` | | |

New steps: `Expect digest at tick 200 is "<hex>"`, `Expect digest at tick 200
to match run "<scenario>"`, `Expect mark "fire.start obj=shooter" by tick 90`,
`Expect "bench.log" contains "TEXT"`.

## Determinism and the animation clock

`sta_sim` is already deterministic: fixed point, ordered ids, one seeded RNG,
a digest per tick. The renderer is not yet, and that is the part Refit had to
retrofit at a cost (`testbench/README.md`, "A frame that repeats": three
rounds of fixes before `showcase` reached 0 differing pixels between
launches). STA64 adopts the rule now, before M2 adds interpolation and
effects.

**The rule.** Anything that moves on screen reads `AnimClock`, never Bevy's
`Time`. A cosmetic random sequence takes its seed from `AnimClock::seed()`.
A per-object cosmetic seed comes from `AnimClock::entity_key(EntityId)`, the
sim's id, never a Bevy `Entity`, a pointer or spawn order of render-only
entities. Bevy's `Time` is for what is not seen: logs, file polling,
profiling.

`AnimClock` holds the anim time: sim time at the last tick plus the
interpolation alpha times the tick length. It stands while the sim is paused.
Two modes, one API:

- **`wall`** (players): cosmetic loops that stock keeps running while paused
  (running lights, glows) may follow real time.
- **`bench`**: everything follows sim time; the cosmetic seed is fixed; each
  `App::update` advances exactly one render frame of `Step=` milliseconds, so
  the interpolation alpha of every frame is the same on every run.

**What is summed per frame steps on fixed sub-steps.** Refit's last
differences came from values added up once a frame, which depend on how ticks
fell between frames. Particle emission and anything that integrates over
frame time advances in fixed steps of anim time, independent of the frame
rate.

Today `model_scene::animate`, `sprites::animate` and `particles::simulate`
read `Res<Time>`, and particles share one RNG advanced per frame. Moving them
onto `AnimClock` is the first deliverable.

**Lockstep tests.** Refit's `bench-determinism` (one scene, seeded, paused at
tick 200, shot twice in one run and once in a second run, compared within 0
and 0.01) is the model. STA64's version is stronger and cheaper: two
in-process runs of one scene, digests equal at every tick; then the same
scene's digest at tick N committed as a golden value and checked on every CI
platform, which catches x86-64 / ARM64 / Apple Silicon divergence. The
render half (two `--headless` runs, frame at tick N, within 0) runs locally.

## Checks, strongest first

1. **Sim state and digest.** `objects.jsonl`, `query`, digest per tick.
   Exact, fast, cross-platform. Prefer this for every behaviour check.
2. **Structured logs and events.** Marks and `bench.log` answers, keyed by
   tick. For timing (weapon bursts, cloak duration, build time) and for
   render-side facts that are not pixels (what was culled, which LOD was
   drawn, how many particles were alive), emitted by the renderer as marks.
3. **Frames.** Frame dumps and pixel diffs with explicit tolerances (mean
   absolute difference, largest difference, share of pixels), as Refit's
   `Expect shots ... to match within`. Only for rendering work. A frame is
   game-derived: frames and baselines live under
   `$XDG_DATA_HOME/sta64/bench/`, never in a checkout, and the tool refuses a
   path inside one (Refit's hard rule, `testbench/README.md`, "Baselines").

## Parity with stock

Refit's bench stays the oracle for stock behaviour. One scene file runs on
both engines:

1. Refit: `SCENE_INI=<file>` (or `Setup: testbench/scene/bench-setup.sh`)
   with `Mod: stock`, `Clock: bench`, `Seed=`, `Step=`, `Pause=`. It writes
   `Scene.objects.jsonl`, `Scene.marks` and frames.
2. STA64: `sta-bench run` on the same file with `Map=a2_borg01`, the same
   install and mod.
3. `sta-bench diff`: object lists by name (positions after T seconds, hull
   over time, who is dying), marks by kind on a common sim-seconds axis, and
   frames side by side with a heat map.

What it is for: cull and LOD distances, weapon timing (burst length, gap,
first shot after an order, from `fire.start`/`fire.end`), ODF behaviour
(speeds, turn rates, damage per hit, shield regeneration), and visuals
(lighting, effects, team colours). Comparisons use sim seconds, never tick
counts, and tolerances of about one stock tick; random outcomes are never
compared. What a parity run finds is recorded as an observation (a number
and the scene that measured it), which `ARCHITECTURE.md`, "Legal boundary"
allows; the frames stay local.

## Sessions and reports

A thin layer over Refit's conventions (`testbench/README.md`, "Which session
a command acts on"):

- `sta-bench session start --scene planet` prints an ID
  (`20261108-142501-48213`) and the `drive` and `stop` lines for it. `drive`
  and `stop` take `--session ID` or `STA64_BENCH_SESSION`; there is no
  default when more than one runs, and an ended ID is an error. An agent
  passes its ID on every call and stops only sessions it started.
- Each run writes `$XDG_DATA_HOME/sta64/bench/results/<run>/` with `latest`
  pointing at it: `index.html`, `report.md`, `results.json`, and per case
  `log.txt`, `bench.log`, `objects.jsonl`, `marks`, `frames/`.
- CI uploads `report.md` and `results.json`, never frames.

## CI

The matrix in `.github/workflows/ci.yml` has no game install, so:

- **core job**: `cargo test -p sta_bench` runs the parsers (Refit's scene and
  scenario files as fixtures), the command vocabulary, and tier-1 scenes
  built from a synthetic content fixture (our own JSON5 units and glTF
  meshes, no game data). The golden digests run here on all four platforms.
- **game job**: after the build, `sta64 --sim-only --scene
  bench/scenes/synthetic.ini --ticks 300` checks the scene path end to end.
- Scenarios that need the install run locally and skip when
  `STA64_TEST_INSTALL` is unset, as the format crates' optional checks do.
- Tier-2 frames in CI wait on a software GPU (open questions).

## Plan

| Milestone | Bench work | First deliverables |
|---|---|---|
| **M2** | Clock, scene loader, headless mode, minimal runner | `AnimClock` and the three systems moved onto it; `sta_bench` parsers with Refit's `firing.ini` and `planet.ini` as fixtures; `sta64 --scene --headless --clock bench --ticks`; commands `orbit`, `camera`, `center`, `zoom`, `pause`, `resume`, `pause-at`, `tick`, `seed`, `query`, `objects`, `select`, `selection`, `goto`, `stop`, `spawn`; `Scene`/`Expect scene`/`Run N ticks`/object phrases/`Take a screenshot`/`Expect digest`; report dir; `planet`: built, two objects listed, same digest and same frame on two runs; `firing`: built, `Attack` accepted and the shooter turns to its target (fire answers `! unsupported: weapons` until M3) |
| **M3** | Combat and economy scenes; marks; parity runs | bench edits (`kill`, `damage`, `heal`, `immortal`, `energy`, `special`, `cargo`, `build`, `tech`, `colonize`); fire/death marks; `firing`, `beam-pass`, `impacts`, `shields`, `explosion`, `factions`, `mining`, `construction`, `freighters`; first parity report vs stock (weapon timing from `firing`, cull distances from `planet`); golden digests in CI |
| **M4** | UI and front end | `Hud`/`Cursor`/`Tooltips`/`Hover`/`Race` keys; Bevy input-event injection for UI steps; `stations`, `ui-hud-layout`, `ui-colonize`, `zoom`; front-end screens reached by state, not clicks; UI frame baselines with tolerances |
| **M5** | Lockstep, Lua, AI | two sims in one process exchanging commands, digests compared per tick (`bench-determinism` generalised); desync injection; replays from the command log; headless run of every installed `missions/*.lua` (`MISSIONS.md`); AI-vs-AI soak; `Players:` header |

## Open questions

- **Ticks across engines.** Refit's `Pause=200` with `Step=16` is 3.2 s of
  stock sim; STA64's tick 200 is 6.67 s. Add a seconds form (`Pause=3.2s`)
  and have parity scenes use only that, or convert in the parity tool?
- **One file, two engines.** Does Refit's scene parser ignore keys it does
  not know (`Map=`, `Profile=`, `Pause=3.2s`)? If not, Refit needs a small
  change, or STA64-only keys go in a `[STA64]` section.
- **Where scenes live.** Copy Refit's catalogue into `bench/scenes/` (MIT,
  drifts) or read it from a Refit checkout (`REFIT_ROOT`, never drifts, not in
  CI)?
- **Bench edits in the sim.** A `Command::Bench` variant in the command log,
  or a separate debug API outside the digest? The plan assumes the former so
  edited runs replay.
- **Empty map.** STA64's built-in empty map needs bounds and a sky; should
  its coordinates match `a2_borg01`'s quadrant so Refit's positions
  (around 2000,0,2000) mean the same on both?
- **Axes and units.** Confirm STA64's world axes, heading and units equal
  Refit's (Y up, heading 0 faces +z, a Galaxy class ~100 units) once BZN
  loading lands.
- **Software GPU in CI.** lavapipe/WARP for tier-2 frames on the matrix, or
  frames stay local-only?
- **Wall clock while paused.** Which cosmetic loops keep running in the
  player's `wall` mode, to match stock?
