# Test bench

STA64 needs a way to put a known situation on screen or into the sim, run it
for a known number of ticks, and check what happened: one ship beside one
planet, one weapon firing, thirty ships selected. Armada II Refit (MIT, same
author) already has such a bench for the stock 32-bit game. This plan takes
its interface and what it learned, and none of its machinery.

## Goals

- Load a **scene** (map, objects, camera, seed) straight into the engine,
  headless or windowed, starting from an empty map, an installed map, a
  sim snapshot or a replay, and drive it with **commands** (orbit, pause, select,
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
- Importing stock Armada II / Fleet Ops `.sav` files as fixtures. As far as we
  know (unverified) they are close to a dump of the old engine's internal
  state, tied to that executable, and would not map onto `sta_sim`. Only worth
  doing if save import becomes a player-facing feature, which is separate from
  the bench. Starting points come from maps and from STA64's own state
  ("Starting points").
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
| Always the map the game was launched on (`-nointro a2_borg01`); objects outside its quadrant are not drawn | new key `Map=`: a BZN from the install **with its own objects**, or STA64's built-in empty map (default). The scene is an overlay on top ("Starting points"). Parity runs set `Map=a2_borg01` |
| (none: the launch map's own objects are always there) | new key `MapObjects=keep\|strip`, default `strip` when the scene file lists objects, `keep` when it lists none |
| `Delay=30`: mission ticks before the scene is built, so the engine settles | accepted; default 0. The scene is built on tick 0 after the map loads |
| `Step=<ms>`: forces the engine's variable tick to a fixed length | the sim tick is always 1/`TICK_HZ`. `Step=` becomes the **render frame** length under the bench clock (frames per tick, and so interpolation, fixed) |
| `Pause=<tick>`, `pause-at`, `tick`: counted in the old engine's ticks | counted in STA64 ticks. Cross-engine scenes use seconds: `Pause=3.2s` (see open questions) |
| `Seed=`: reseeds the engine's random sequences | seeds `Sim::new(seed)` and the cosmetic seed. Same intent, different streams: never compare random outcomes across engines |
| `#<id>`: an engine object handle | `#<id>`: the sim's `EntityId`. Ids are given in order, so `#3` names the same object on every run |
| `Hud`, `Grid`, `Cursor`, `Notices`, `Tooltips`, `Hover`, `ShieldFx`, `Music`, `Captions`, `Race` | the same switches on STA64's own systems, as each system lands. Until then the key answers `! unsupported: <system>` in the log, like Refit's stood-down hooks, and the scene still loads |
| `Enable=0` (patch nothing but music/captions) | dropped |
| Profile-dependent behaviour (a hook stands down on Roots) | none. A scene may name a content profile (`Profile=FO4`) instead |

### Starting points

A scene starts from one of four things, set by `Map=` or `Start=`:

| Start | What loads | Use |
|---|---|---|
| (none) | STA64's built-in empty map; the scene file spawns everything | unit-level tests: one ship, one weapon |
| `Map=<name>` | the BZN **and its objects**: placed units, start positions, resources, and the mission name (resolved as in `MISSIONS.md`) | "the real map at tick 0"; parity; the map corpus test |
| `Start=snapshot:<file>` | the sim state serialised at tick N | "minute 12 of a 4-player battle" without replaying 12 minutes. Arrives with player save/load (M3) |
| `Start=replay:<file>` | a map plus a command log, replayed from tick 0 | lockstep desync test; bug repro format (M5) |

The scene file is an overlay on whichever start is chosen: its `[Object.*]`
sections spawn on top, its bench edits and `Seed=` apply after load. Snapshots
and replays are STA64's own state and carry no game data beyond what the map
already names; they live under `$XDG_DATA_HOME/sta64/bench/`, not in a checkout.

**`MapObjects`.** `keep` leaves the map's contents in place; `strip` removes
them and keeps only terrain, bounds and sky, which is the old backdrop-only
behaviour. The default follows who wrote the scene: a hand-made scene that
lists its own objects wants a clean stage, so a placed station or a start
position cannot hit its shot or its count (`Expect 3 ships on team 1`); a
scene that lists none and names a map wants that map's play. The key is
explicit in every scene that is checked in. `strip` is for tests *of* a thing,
`keep` for tests *on* a map.

**Why maps help parity.** Same `Map=`, same `Seed=`, compared after N seconds
needs no coordinates to agree between engines: the map places its own objects
and the object lists are matched by name. Positions in a hand-made scene still
need Refit's quadrant to line up (open questions).

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
- **`sta-bench`** (binary in `sta_bench`, logic in `sta_bench::cli`; built):
  - `list [scenarios|scenes]`.
  - `run <scenario...>`: tier 1 in-process; `--tier 2` (or a `Tier: 2`
    header) then runs the scene in `sta64 --headless --clock bench --control
    stdio --frames ...`, requires the same final digest as the in-process run,
    and writes frames to `<case>/frames/tick-NNNNNN.png` (default ticks: 0 and
    the last). `--repeat N` runs the process N times and requires byte-identical
    frames. `--sta64`/`$STA64_BIN` names the binary.
  - `check`: every tier-1 scenario in-process, exit 1 on a failure (what CI
    calls); tier-2 scenarios are listed as skipped.
  - `session start --scene NAME [Key=Value...]` / `list` / `stop` and
    `drive --session ID "<cmd>"...`: a headless `sta64 --control stdio`
    behind a named pipe (Unix), state in `<results parent>/sessions/<id>/`.
  - `diff A B`: two report directories, per case: digests (first differing
    tick), marks, objects by name, frames byte for byte; exit 1 on a difference.
  - `pair <pair> --game a2|roots`: one description on the original game and
    on STA64, compared in sim seconds ("Paired scenes", below).
  - Not yet: isolated install clones per session, `--jobs`, `index.html`,
    frames from a session, `Take a screenshot`/`Expect shots` steps.
- **Frames** (`sta64 --headless --frames 0,100 --frame-dir D --frame-size WxH`,
  `crates/sta64/src/bench/frames.rs`): a windowless render app draws into an
  offscreen image and a `Screenshot` of it is written as PNG at each tick; the
  bench holds the sim still while a frame settles, so it is exactly that tick.
  Today the picture is stand-in shapes from the sim state (sphere for planets
  and moons, box for the rest, owner colour), camera from `Camera=`/`orbit`/
  `camera`: the bench has no install to load models from, so the game's own
  renderers join when they can run without one. Without a GPU adapter `sta64`
  exits 77 ("no GPU adapter") and `sta-bench`/the tests skip with a message.

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

## Tier-1 runner

`sta_bench::run` (`crates/sta_bench/src/run/`) runs a scenario in-process.
`cargo test -p sta_bench` runs every `bench/scenarios/*.md` whose header says
`Tier: 1` (`tests/scenes.rs`), so a new scenario needs no Rust. Fragments
(`_name.md`) are not scenarios. A scenario with no `Tier:` line is skipped.

- **Content.** `RunConfig::fake()` runs on `fake::db()`. Scenes keep stock ODF
  names (`fgalaxy`); `RunConfig::aliases` maps them onto the content in use
  (fake: Galaxy -> `fx_cruiser`, Borg cube -> `fx_frigate`, class M planet ->
  `fx_moon`). Real content needs no aliases. The runner's content also has
  the cloakers of `sta_sim::specials::fixture` (`fx_cloaker`,
  `fx_cloaker_frail`, `fx_cloaker_loud`; table in that file), and craft
  carrying real physics ODF values for the movement scenarios (SIM-15):
  `fx_a2_scout` and `fx_a2_battle` (stock `sscoutphys`, `sbattphys`),
  `fx_fo_escort`, `fx_fo_cruiser` and `fx_fo_battle` (FO 4.0
  `smooth_physics_150`, `_100`, `_60`), and `fx_fo_rig`, a construction
  ship with the FO 4.0 `fconst` values (`construction-fo`).
- **Profile.** A scene's `Profile=` (`STA2Classic`, `FO4`, ...; or the
  `Scene:` override) runs the content under that profile, which picks the
  sim's rules, the movement model among them. The `move-*` scenarios use it.
- **Heading.** `Expect "OBJ" heading ...` reads degrees clockwise from +z
  toward +x, as a scene's `Heading=` writes them.
- **Specials.** `cloak <name> on|off|now` is `Order::Cloak` (`now` is `on`:
  the engage time is the cloaking device's `effectTimer`); `special <name>
  <weapon>` is `Order::Special` for the named special weapon of the unit
  (toggles a cloak; no target yet). Check them with `Expect mark "cloak
  obj=NAME on=1|0"` and `Expect player P sees | does not see "NAME"`; see
  `bench/scenarios/cloak.md`.
- **Build.** The start (`Map=` from `RunConfig::maps`, `Start=snapshot:`,
  `Start=replay:`, else the empty map) loads, then the scene's objects are
  spawned with `Sim::spawn_def` (so ids are known: file order, `#1`...). Team
  `N>0` is `PlayerId(N)` with a seat made for it, team 0 is neutral. Scene
  keys become edits from `PlayerId::BENCH` on tick 0 (`Immortal`, `Engines=0`,
  `Weapons=0`, `Attack=` as an order from the owner). `Allies=1,2` puts those players on one team
  (each `Team=N` is otherwise a team of its own). `Heal=1` is one `Heal`
  edit a tick. `Scene: name Key=v` overrides `Seed`, `Pause`, `Map`,
  `MapObjects`, `Profile`, `Delay`, `Start`.
- **Commands** are queued and land at the next tick boundary (`at <t>` waits
  for its tick). Orders are issued as the unit's owner, so the sim's own
  ownership check is under test. An edit the sim does not apply yet comes back
  as `Rejected`; the runner logs it once in `bench.log` and goes on. Runner
  extensions: `mine <ship>... <node>` (`Order::Mine`), `goto ... queue`
  (`Move` with `queue: true`: a waypoint chain). Answers: `paused at tick
  N`, `resumed at tick N`, `tick N`, `objects N -> objects.jsonl`, `marks N ->
  marks`, `selected N`, `ok`, `! <why>`. A command answering `!` fails its step.
- **Ticks.** Tick `N` is the world after `N` steps; tick 0 is the setup. A
  mark is stamped with the tick of the step that produced it: a command
  queued at tick 0 fires `fire.start` at tick 0, and its effect is visible at
  tick 1. Every view and digest is kept, so a check may look at any tick that
  has been run (`at tick N`); a tick not reached yet is a failure, never an
  implicit run.
- **Marks** come from `SimEvent`s: `fire.start`, `fire.end`, `hit`,
  `shields.down`, `spawn`, `death`, `build.start`, `built`, `delivered`,
  `cloak`, `rejected`, `eliminated`, with the fields listed in
  `marks.rs` (`sta_bench`, shared by the runner and `sta64 --sim-only/--headless`,
  which writes `$STA64_BENCH_OUT/marks`; render marks have no source yet). `fire.start`/`fire.end` bracket a burst: shots of one
  hardpoint each no later than `FireGap` after the last ended (`fire.end` is
  the last `Fired` plus the weapon's `duration`).
- **Measuring marks** (TOOL-16; Refit's `[Marks]` keys of the same names, on
  both paths since `sta64` writes `Bench::marks()`): `Ordnance=1` adds
  `ord.new` (`id obj weapon target at`) per shot and `ord.gone` (`id obj
  weapon at life tgt_at d`) when the shot's hit lands (a shot that never hits
  has none: the sim has no expiry event; a hit is matched to the oldest open
  shot of its shooter and weapon, since hits carry no shot id; the original
  writes `ord.gone` for misses too, so a miss there is an only-in-original
  finding and shifts the `ord.gone` pairs after it), the weapon named by its ordnance
  ODF (`ordName`) as Refit names it; `OrdnanceSkip=` leaves out weapon or
  ordnance ODF prefixes. `State=<ms>` adds a `state` line per named scene
  object that often, rounded to whole ticks (`pos hull sh`, `sp` when it has
  special energy; no `q`/`ma`/`swa`); a series that ends early on one side is
  a finding.
  `bank` (a command) also writes a `bank` mark. `SimTime=1` puts `t=` (sim
  seconds) first on every mark. `sta-bench parity` compares `state` as a
  series (B resampled on A's times: position within `pos`, hull and shields
  within `hull`, special-energy slope within `slope`), the others one by one
  (`bank` resources within `hull`, `ord.gone` `d` within `pos` and `life`
  within `time`); a kind not in the name table is compared by kind and object
  and named in the notes.
- **Checks** beyond the typed steps (grammar in `run/check.rs`; `Wait ... until`
  takes any predicate about the present):

  ```text
  Expect [no] mark "KIND k=v ..." (by tick N | at tick N | between ticks A and B)
  Expect "OBJ" FIELD [at tick N] is CMP          hull hull.max shield[.fore|.aft|.port|.starboard] shield.max speed x y z
  Expect "OBJ" is within R of (X Y Z | "OBJ2") [at tick N]
  Expect "OBJ" was never within R of X Y Z            (every tick run so far)
  Expect "OBJ" exists | does not exist [at tick N]
  Expect player P resource NAME [at tick N] is CMP   dilithium latinum metal biomatter
  Expect player P is [not] eliminated [at tick N]
  Expect player P sees | does not see "OBJ" [at tick N]
  Expect digest at tick N is "HEX" | to match run "SCENARIO"      (typed)
  Expect N ships|stations|planets|objects on team T                 (typed)
  Wait [up to N seconds|ticks] until <predicate>
  Run N ticks | Run until tick N | Save snapshot as "FILE" | Save replay as "FILE"
  CMP = V [within E] | at least V | at most V | above V | below V | between A and B
  ```

  Digests: pin `Expect digest at tick N is` only for golden values a CI job
  owns (TOOL-06); scenarios that need "the same run twice" compare two
  scenarios with `to match run`.
- **Saved starts.** `Save snapshot as "F"` and `Save replay as "F"` write under
  `RunConfig::data_dir`; a scenario with `Start=snapshot:F` / `Start=replay:F`
  reads it back, and `Expect digest at tick N to match run "<saver>"` compares
  the two runs. `tests/scenes.rs` clears a private data directory and runs
  scenarios in name order, so the saver is named to sort first
  (`snapshot-save` < `snapshot-start`, `replay-save` < `replay-start`). A
  replay records every command that reached the sim, and scene and `spawn`
  placements (made with `Sim::spawn_def`) as `Edit::Spawn` on their tick;
  `Save replay as` plays the file back from tick 0 and fails on a digest
  difference. Only runs from the empty map or a `Map=` can be recorded, and
  orders given on tick 0 (before the bench's spawn edits in the sort order)
  would not replay: give them on tick 1 or later.
- **Not at tier 1:** screenshots and frame steps are skipped, and the display
  switches (`hud`, `grid`...) answer `! unsupported: <system>`. `Wait`
  without a bound is capped by the `Timeout:` header (sim seconds), default 120.
- **Report.** `run::write_report(root, &cases)` writes `<root>/<run>/{report.md,
  results.json, <case>/{log.txt, bench.log, objects.jsonl, marks, digests}}` and
  repoints `<root>/latest`. `cargo test` writes it under `target/tmp/bench-results/`.

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

**Replays.** A `Start=replay:` file is a map, a seed and the command log, with
the digest at chosen ticks. Replaying it and comparing digests is the same
check as the lockstep tests below, on a recorded run instead of a scripted
one, and a desync report from a real game is a replay a developer can run.

**Lockstep tests.** Refit's `bench-determinism` (one scene, seeded, paused at
tick 200, shot twice in one run and once in a second run, compared within 0
and 0.01) is the model. STA64's version is stronger and cheaper: two
in-process runs of one scene, digests equal at every tick; then the same
scene's digest at tick N committed as a golden value and checked on every CI
platform, which catches x86-64 / ARM64 / Apple Silicon divergence. The
render half (two `--headless` runs, frame at tick N, within 0) runs locally.

## Checks, strongest first

1. **Sim state and digest.** `objects.jsonl`, `query`, digest per tick.
   Exact, fast, cross-platform. Prefer this for every behaviour check. The
   **map corpus test** is the widest case of it: every map in the install
   loads and ticks N times without a panic, and the digest is the same on two
   runs. It covers stock and FO 4.0 maps and finds loader and sim faults on
   content nobody wrote a scene for.
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
   install and mod. With `MapObjects=keep` and no spawned objects, the same map
   and seed can be compared after N seconds without any coordinates agreeing.
3. `sta-bench parity <refit-out> <ours-out>` (TOOL-08; `--names` prints the
   Refit-to-ours mark-name table, `--tol time=0.1,pos=1,hull=1` the tolerances)
   reads Refit's `Scene.marks` and `Scene.objects.jsonl` and our `marks` and
   `objects.jsonl`: objects by name, marks in sim seconds, kinds in Refit's
   vocabulary. `sta-bench diff`: object lists by name (positions after T seconds, hull
   over time, who is dying), marks by kind on a common sim-seconds axis, and
   frames side by side with a heat map.

### Paired scenes (`sta-bench pair`, TOOL-15)

One description runs on both engines and leaves one report:

```sh
sta-bench pair move --game a2 --install "$A2_INSTALL" --refit "$REFIT_WT"
sta-bench pair firing --game roots --install "$ROOTS_INSTALL"      # Mod= from the pair file
sta-bench pair move --game a2 --refit-out <earlier>/refit --install ...   # reuse an original run
```

- **The description** is `bench/pairs/<name>.ini`: a scene file in Refit's
  format (`[Scene]`, `[Marks]`, `[Object.*]`) plus a `[Pair]` section
  Refit's `Scene.asi` never reads: `Games=a2,roots`, `Duration=` (sim seconds
  after the scene is built), `Frames=3,8` (sim seconds), `Size=WxH`,
  `Tol=time=0.25,pos=5,hull=1[,frame=8][,lowfreq=..]` (frame metrics: "Frame
  metrics" below), `At=<s> <command>` (repeatable, the timeline, in the command
  vocabulary), and per game `Launch=`, `Race=`, `Mod=`, `Map=` (see below). Any key may be prefixed by a game, in any section (`roots.Odf=`,
  `roots.Size=1280x960`, `fake.Odf=fx_cruiser`): for that game it replaces
  the plain key; other games' prefixed keys are dropped. **Armada I is not
  supported** (Refit's bench has no A1 profile, PLAN-ARMADA1); A1 keeps
  install-reading checks.
- **The original side** is `tools/pair/refit_side.py` (stdlib Python; `--dry-run`
  prints the plan). It starts one Refit bench session (stock A2: `--mod stock
  --clock bench`, 1280x720; Roots: `--game fo-roots --layers display,asiloader`,
  1280x960), installs the materialised scene with `[Marks] SimTime=1` and
  `State=250` forced on, launches, waits for `scene ready`, sends each `At=`
  command and pauses for each frame when the **sim time** read from
  `Scene.marks` reaches it (`drive shot`, of the bench's own headless display),
  and records when each actually landed. It always stops its session (the
  clone is deleted). `SimTime=`, `Ordnance=`, `State=` and `bank` need Refit's
  `bench/measure-marks` branch (`--refit` or `$REFIT_ROOT` names that
  checkout). Commands land late (one `drive` round trip, and the sim may run
  faster than the wall clock), so nothing is compared on the asked time.
- **The STA64 side** runs the same text in-process on the install's content
  (`--install`, `--mod`, or `--fake` for tests), each command on the tick
  nearest to when it **landed on the original**, frames at the original's
  frame times. It writes the original's file formats: `Scene.marks` (Refit's
  names; fire bursts merged per shooter and weapon as Refit marks them; one
  `ord.new` per shot; a `state` line per named object every tick), `run.json`,
  and the frames (next bullet).
- **The map.** The original always loads the map it was launched on, so the
  frames STA64 draws load the same: the pair's `Map=` (in `[Pair]`, per game if
  need be), else the map `Launch=` names (`-nointro a2_borg01` -> `a2_borg01`;
  `... -mod "X" fulltech.bzn` -> `fulltech`; the value after `-mod` is not a map).
  The scene the frame run uses gets `Map=<name>` and `MapObjects=keep` in `[Scene]`
  unless the file sets them itself (a hand-made scene that wants a clean stage sets
  `MapObjects=strip`). The map's own objects are therefore in the frames, **including
  ones the original does not draw** (it draws only objects in the camera's quadrant),
  so a station or a ring at the edge of a frame is not a bug. The measuring run
  (`Scene.marks`, `state`) keeps STA64's empty map: the map's own units fight and
  would change what a scene measures, and the original's marks name only the scene's
  objects. A pair with no map and no `Launch=` draws on the empty map, as before.
- **Frames from the install** (`sta64 --headless --scene S --frames T,.. --install-root R
  [--mod M] --install-content`, what `sta-bench pair` starts; `--sta64`/`$STA64_BIN`).
  The bench's sim is built on the install's content with the pair's map, and what
  is drawn is what the client draws for that map and that world, by the client's own
  systems (`sta64/src/bench/pair_view.rs` adds the map view's plugin, the unit, effect
  and hull renderers, shadows and the render profile; there is no second loader):
  the map's sky, starfield, nebulae, planets and rock fields and its lights, every
  unit of the bench's world (the map's own and the scene's) with its real hull,
  textures and team colour, weapon, impact and special effects from the bench's events,
  the hull damage state. The camera is the scene's `Camera=`/`orbit`/`camera` pose (the
  same one the stand-in frames use; commands in the `At=` timeline that move it apply).
  **Headless means headless**: a windowless Bevy app with an offscreen render target
  (wgpu on the GPU, no display server: `WAYLAND_DISPLAY` and `DISPLAY` are removed
  from the child's environment), Bevy's audio plugin disabled and no sound system
  added, nothing captured from a screen. The bench holds the sim still while a frame
  settles, so a frame is exactly its tick. `STA64_RENDER_PROFILE=stock|refit` selects
  the look (default stock; RND-22). Without an install (`--fake`) the frames are the
  bench's stand-in shapes as before. **Not yet drawn the way the original draws it**
  is what RND-23..26 and UI-19 are for: a pair now shows exactly how far the
  client's look is from the original's. The camera uses Bevy's default vertical field
  of view (45 degrees); the original's field of view and near plane are measured per game
  by the package that matches them, and a pair whose framing differs sets that first.
- **The report** (`report.md` for people, `report.json` for tools) is computed
  from the two side directories alone, so it is reproducible: commands (asked,
  landed on each side); events paired by kind, object and weapon (`fire.start`,
  `fire.end`, `destroyed`, cloak) with time deltas; ordnance counts per shooter
  and weapon; each object's `state` series sampled at the original's sample
  times (position distance, hull, shields, special energy: max, mean, first
  time over) and motion figures (start, top speed, arrival); frames side by
  side (`frames/f<i>.png`: original | STA64 | difference x4 | blurred difference x4)
  with mean, max and share of differing pixels and the frame metrics below. Frames
  are judged only when the pair sets a frame tolerance. Exit 1 when anything is
  beyond the tolerances.
- **Where it goes**: `$XDG_DATA_HOME/sta64/bench/pairs/<pair>-<game>/`
  (`refit/`, `sta64/`, `report.*`, `frames/`). Both tools refuse a path inside
  a checkout: the frames are the game's art.
- **One original game at a time.** The driver holds an exclusive `flock` on
  the bench lock for its whole run (`$STA64_BENCH_LOCK`, default
  `.claude/locks/bench.lock` of the main checkout; `--lock`), waits up to
  540 s for it (`--lock-wait`), and otherwise exits 75 `BENCH-BUSY` (so does
  `sta-bench pair`): run again. Anything else that starts a game with
  `./a2test` takes the same lock:
  `exec 8>"$STA64_BENCH_LOCK" && flock -w 540 8 || { echo BENCH-BUSY; exit 75; }`.
- **Rules for agents**: the lock above, reniced (the driver renices itself); compare in sim seconds only; a scene-script
  `attack` does not move ships on Roots (a player's right-click does), so
  write movement as `goto`. STA64's `Scene.marks` keeps `t=` beside the tick.

#### Writing a visual pair

A visual pair is a pair file whose point is its frames. Start from
`bench/pairs/firing.ini` (a Galaxy class firing at a Borg cube, side on):

1. Put what is on screen in `[Object.*]` sections (real ODF names; per-game
   `a2.Odf=` / `roots.Odf=` where the games name things differently) and the
   camera in `[Scene]` (`Camera=orbit <object|x y z> <yaw> <pitch> <distance>`,
   or `camera <ex> <ey> <ez> <object|x y z>`). Yaw 0 looks from +z, 90 from +x.
   `Anchor=world`, so coordinates are map coordinates on both sides.
2. Name the map: `a2.Launch=-nointro a2_borg01`, or `Map=` in `[Pair]`. Both
   engines then show that map's sky, nebulae and lights behind the scene.
3. Ask for frames in sim seconds: `Frames=1,3,8`. Choose them for what is in
   them: before anything fires (hull, map and lights only), while it fires
   (effects), after. The original takes each frame when its sim clock reaches
   the time (it lands late, `refit.json` says when); STA64 draws the tick
   nearest to that, not the asked time. A paused scene with a still camera is
   the least noisy.
4. `Size=1280x720` (Roots: `roots.Size=1280x960`, the size its bench session
   runs at; the sizes of the two frames are compared and reported).
5. Run it: `sta-bench pair <name> --game a2 --install "$A2" [--mod NAME]`.
   While iterating on the STA64 side, reuse the original's output:
   `--refit-out <earlier>/refit` (no game session). Read `report.md`, then the
   `frames/f<i>.png` strips (original | STA64 | difference | blurred
   difference): the strips are for your eyes, the metrics for scripts.

#### Frame metrics

Per frame, `report.json` (`frames[i].metrics`) and `report.md` carry, beside the
per-pixel `mean`, `max`, `share` (pixels that differ at all) and `share16`
(by more than 16 in some channel), figures that stay small when the two frames
are the same picture a pixel apart and large when they are not:

| Metric (tolerance key) | What it is | Reads as |
|---|---|---|
| `rms` | root-mean-square difference over all channels (0..255) | the plain figure the blurred ones are compared with |
| `midfreq`, `lowfreq` | the same after a box blur of radius `w/320` and `w/80` pixels (4 and 16 at 1280) | a one-pixel shift of an edge is a thin pulse of difference and a blur spreads it (energy falls by about `1/sqrt(2r+1)`); a wrong colour or a missing object over an area stays whole. Not the mean *absolute* difference: blurring conserves that |
| `shift` | the smallest mean absolute luminance difference over every whole-pixel offset up to `w/320`, and the offset (STA64 sampled at x+dx, y+dy) | what is left when a fixed misalignment is taken out; the offset says by how much the camera or field of view is off |
| `lum` | whole-frame mean luminance, STA64 minus original (absolute) | overall brightness: ambient and key light, nebula strength |
| `colour` | largest channel difference of the whole-frame mean colour | a colour cast |
| `bg`, `fg` | the luminance difference of the mean over the pixels the original shows dark (`bg`, luminance <= 48) or bright (`fg`) | `bg` is the sky, nebulae and stars the map draws; `fg` the objects, effects and bright gas |
| `tile` | the largest luminance difference of the mean over a 4 x 4 grid | something missing or misplaced in one part of the frame |
| `bright` | difference of the share of bright pixels | an object or a nebula that is not there |

Luminance is `0.299 R + 0.587 G + 0.114 B` over the 8-bit sRGB values. Regions are cut from
the **original's** frame and applied to both frames, so a region compares what is
drawn in the same place; the report lists mean colour and luminance of each side
per region (whole frame, `bg`, `fg`, `centre` (the middle half of both axes),
`border`). All of it is a function of the two PNGs: the same two frames give the same
numbers (`sta-bench pair --refit-out` re-computes a report without a game).

#### From a visual pair to a regression test

A pair is reported, not judged, until `Tol=` says what counts as a match
(`frame=` is the plain mean difference; the metric keys above are
`midfreq, lowfreq, shift, lum, colour, bg, fg, tile, bright`, each opt-in):

```ini
[Pair]
Tol=time=0.25,pos=5,hull=5,lowfreq=12,bg=6,fg=15,tile=10
```

Work the way a renderer package does: run the pair, change the renderer, run
again with `--refit-out` until `lowfreq`, `bg` and `fg` stop falling; then set
each limit a little above the value you reached (so a one-pixel change does not
trip it, and a lost nebula or hull does) and commit the pair file. From then on
`sta-bench pair` exits 1 and the report lists `breaches` (`lowfreq 14.20 > 12`)
when a frame leaves its limits. A frame that is not judged by any limit never
fails the pair. A regression needs the original's frames: either run the original
side each time (a game session, minutes, takes the bench lock) or keep the
original's `refit/` directory for `--refit-out`; those frames are the game's art
and stay outside the repository, so the check is a local or install-gated one, never
a CI job that reads a checked-in image. The pair file, with its limits, is what
goes in the repository.

What it is for: cull and LOD distances, weapon timing (burst length, gap,
first shot after an order, from `fire.start`/`fire.end`), ODF behaviour
(speeds, turn rates, damage per hit, shield regeneration), and visuals
(lighting, effects, team colours). Comparisons use sim seconds, never tick
counts, and tolerances of about one stock tick; random outcomes are never
compared. What a parity run finds is recorded as an observation (a number
and the scene that measured it), which `ARCHITECTURE.md`, "Legal boundary"
allows; the frames stay local.

## Playtest captures

A playtester who sees a bug presses **Ctrl+F12** in a match (`Action::Capture`
in the key bindings: F12 with the group modifier held). The game writes a
folder and shows "Capture saved: <path>" as a notice; the game goes on. The
folder is a bug report a fix agent can replay without the player's screenshot.
`sta_bench::capture` owns the layout, `sta64/src/capture.rs` the hotkey.

Where: `$STA64_CAPTURE_DIR`, else `$XDG_DATA_HOME/sta64/captures` (Windows
`%LOCALAPPDATA%\sta64\captures`, macOS `~/Library/Application Support/sta64/captures`),
one `<UTC stamp>-t<tick>/` folder per capture. Never inside a game install.

| File | What |
|---|---|
| `capture.json` | the index (`CaptureIndex`): tick, digest, map, seed, content hash, profile, seats (race, control), the mount (install root, active mod, layers in priority order), build (version, git commit, OS), selected units (id, definition, owner, race, position, hull, flags), the unit under the cursor, the cursor on the ground (map space), the camera pose |
| `capture.md` | the same for people |
| `replay.star` | a `sta_sim::replay::Replay`: the start configuration (the whole `MapDef` is in it), every command the sim was given, digests every 30 ticks |
| `snapshot.sta` | `Sim::snapshot` at the capture tick |
| `note.txt` | the player's words: `STA64_CAPTURE_NOTE` if it was set, else empty |

Not captured: a rendered frame (the match has no in-app frame dump; the
headless bench's frames draw stand-ins and are not the player's view, and a
screen grab is out of the question). HUD state beyond the selection is not in
it either. Capturing only reads the sim link's log and view, so it cannot
change a digest. The command log is kept for every match from tick 0 (each
order carries its unit ids, AI orders included: a few MB over a long game); it
is what the capture reads. The capture is written on the frame of the key
press, so that frame stalls for a moment. The git commit in the index is
whatever `STA64_GIT_COMMIT` was set to when the binary was compiled, else
unknown.

### Reproducing one

```sh
sta-bench capture list                       # newest first
sta-bench capture check latest               # or a folder; exit 0 when it reproduces
sta-bench capture check <folder> --install "<install root>" --mod "<mod folder>"
```

`check` mounts the install the capture recorded (override with `--install` and
`--mod`; the mod chain and the content hash must be the playtester's), loads
its content and races as the game does, builds the sim the way the game did
(`index.start`: a skirmish places the seats' starting forces), replays the
command log to the capture tick, comparing every recorded digest on the way,
and checks the digest at the tick against the index's. It also restores
`snapshot.sta` and compares that digest. A failure names the first tick that
differs (tick 0 means the start state differs: other content, mod or build).
`--fake` is for captures of the bench's own fake content (the tests).

A capture only reproduces on the build and content it was taken with. When the
sim has changed since, the digest drifts and `check` says FAIL (exit 1) with the
first differing tick. When it cannot be replayed at all (other content or mod:
content hash differs; a replay or snapshot format the build no longer reads) it
says ERROR and exits 3, which is not a desync. In the last case `capture.json`
and `capture.md` are still plain text for a person to read, though
`Capture::load` refuses the folder.

`--mod` defaults to the capture's recorded mod; `--install` alone keeps it.
Computer players are not recorded or recreated (D3: controllers run on the
host and their orders are in the log), so a sim replayed to the capture tick
and then stepped further has idle AI seats: it matches the player's game up to
the capture tick only.

### From a capture to a regression test

1. `sta-bench capture check <folder>` first. A capture that does not replay is
   a different bug (or a sim change): say so on the issue.
2. Read `capture.md`: the selected ids and definitions are the suspects;
   `note.txt` says what the player saw.
3. **Never commit a capture, or any file cut from one.** `replay.star` holds the
   map converted from the install's BZN, and the index and snapshot name unit
   definitions read from the install: that is game data. A regression test in
   the repo is either a hand-built scene on synthetic content (`fake::db()`)
   that reproduces the same condition, or install-gated (it reads the
   install at run time, skips when `STA64_TEST_INSTALL` is unset, and
   keeps nothing from it). Write the test against the library, not the folder:

   ```rust
   // install-gated: the capture's own content
   let (db, races) = sta_bench::capture::load_install(root, Some("Fleet Ops 4.0"))?;
   let cap = sta_bench::capture::Capture::load(folder)?;
   let mut sim = cap.replay_to_tick(db, Some(&races))?;   // the sim at the capture tick, digest checked
   sim.step(&[]);                                          // run on, then assert the fixed behaviour
   ```

4. For a scene-level scenario (`bench/scenarios/*.md`), the capture's index
   gives the unit definitions, positions and owners to place with `Object`
   sections and to `Select`, `Order` and `Expect` on. (`Start=snapshot:` and
   `Start=replay:` are the planned way to start from the capture's own files,
   see "Starting points"; the runner does not read them yet.)
5. Put the capture's tick, digest and the folder name in the issue, not the
   folder itself.

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
- CI uploads `report.md`, `results.json` and the `digests` files, never frames.

## CI

The matrix in `.github/workflows/ci.yml` has no game install, so:

- **core job**: `cargo test -p sta_bench` runs the parsers (Refit's scene and
  scenario files as fixtures), the command vocabulary, and tier-1 scenes
  built from a synthetic content fixture (our own JSON5 units and glTF
  meshes, no game data). The golden digests run here on all four platforms:
  the `golden-*` scenarios (`bench/scenarios/`, TOOL-06) are an existing
  scenario's scene and commands plus a last step `Expect digest at tick N is
  "HEX"`, so a change that moves a digest fails the job on every target (replace
  the value on purpose and say why in the PR). After the tests the job runs
  `sta-bench check --out bench-report` and uploads `report.md`, `results.json`
  and the per-tick `digests` files as `bench-report-<os>`, to compare a drifting
  platform with `sta-bench diff`; never frames.
- **Re-pinning a golden** (a PR that changes sim output, or fake content, will
  see `golden-*` scenarios fail with `digest at tick N is NEW, wanted OLD`):
  1. Run `cargo run -p sta_bench --bin sta-bench -- run golden-planet golden-firing
     ...` (all `golden-*`, `sta-bench list` names them) on your branch merged
     with current `origin/main`. Only the goldens whose digest moved need work.
  2. For each, work out which change of *yours* moves it (a content field, a
     rule, a system order). If you cannot attribute it, it is a bug or a
     determinism problem: do not re-pin.
  3. Replace the value in `bench/scenarios/golden-<name>.md` (the failing step
     prints the new one). Keep goldens on fixed ticks (`Run N ticks`, `Run until
     tick N`), not `Wait until`, whose end tick moves with behaviour.
  4. The PR body names every re-pinned golden, the old and new digest, and the
     one-line reason (e.g. "golden-mining: miners now pay the unload fee").
     A PR that moves none says so. Goldens are of STA64's own output on the
     fake content; never pin anything derived from an install.
  5. Merge `origin/main` again before asking for review: another PR may have
     moved the same golden, and the later one re-pins on top.
- **game job**: after the build, `sta64 --sim-only --scene
  bench/scenes/synthetic.ini --ticks 300` checks the scene path end to end.
- Scenarios that need the install run locally and skip when
  `STA64_TEST_INSTALL` is unset, as the format crates' optional checks do.
  The **map corpus test** is one of them: it needs the install and a mod
  (stock, or FO 4.0), and runs once per mod.
- Tier-2 frames in CI wait on a software GPU (open questions).
- **GPU frame tests are opt-in.** `crates/sta64/tests/bench_cli.rs` frame tests
  print `SKIP: set STA64_GPU_TESTS=1 ...` and pass unless `STA64_GPU_TESTS=1`
  is set, which CI never does. With it set they need a real GPU: `sta64`
  treats a software adapter (wgpu `Cpu` type, WARP, llvmpipe, SwiftShader,
  Microsoft Basic Render) as none and exits 77 (skip), and a timeout fails.

## Plan

| Milestone | Bench work | First deliverables |
|---|---|---|
| **M2** | Clock, scene loader, headless mode, minimal runner | **map corpus test, first after BZN loading lands** (every installed map loads, ticks N times without panic, digest stable across two runs; stock and FO 4.0; install-gated); `Map=` / `MapObjects=`; `AnimClock` and the three systems moved onto it; `sta_bench` parsers with Refit's `firing.ini` and `planet.ini` as fixtures; `sta64 --scene --headless --clock bench --ticks`; commands `orbit`, `camera`, `center`, `zoom`, `pause`, `resume`, `pause-at`, `tick`, `seed`, `query`, `objects`, `select`, `selection`, `goto`, `stop`, `spawn`; `Scene`/`Expect scene`/`Run N ticks`/object phrases/`Take a screenshot`/`Expect digest`; report dir; `planet`: built, two objects listed, same digest and same frame on two runs; `firing`: built, `Attack` accepted and the shooter turns to its target (fire answers `! unsupported: weapons` until M3) |
| **M3** | Combat and economy scenes; marks; parity runs | bench edits (`kill`, `damage`, `heal`, `immortal`, `energy`, `special`, `cargo`, `build`, `tech`, `colonize`); fire/death marks; `firing`, `beam-pass`, `impacts`, `shields`, `explosion`, `factions`, `mining`, `construction`, `freighters`; first parity report vs stock (weapon timing from `firing`, cull distances from `planet`); golden digests in CI; `Start=snapshot:` with player save/load (snapshot at tick N, reloaded digest equals the continued run) |
| **M4** | UI and front end | `Hud`/`Cursor`/`Tooltips`/`Hover`/`Race` keys; Bevy input-event injection for UI steps; `stations`, `ui-hud-layout`, `ui-colonize`, `zoom`; front-end screens reached by state, not clicks; UI frame baselines with tolerances |
| **M5** | Lockstep, Lua, AI | two sims in one process exchanging commands, digests compared per tick (`bench-determinism` generalised); desync injection; `Start=replay:` (replays from the command log, digests checked at chosen ticks; desync reports become replays); headless run of every installed `missions/*.lua` (`MISSIONS.md`); AI-vs-AI soak; `Players:` header |

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
- **Empty map.** Parity no longer needs it: map-based parity (`Map=` with
  `keep`) compares what the map places. It still matters for hand-made scenes
  with fixed coordinates: STA64's built-in empty map needs bounds and a sky,
  and should probably match `a2_borg01`'s quadrant so Refit's positions
  (around 2000,0,2000) mean the same on both. Only if those scenes stay.
- **Axes and units.** Confirm STA64's world axes, heading and units equal
  Refit's (Y up, heading 0 faces +z, a Galaxy class ~100 units) once BZN
  loading lands.
- **Software GPU in CI.** lavapipe/WARP for tier-2 frames on the matrix, or
  frames stay local-only?
- **Wall clock while paused.** Which cosmetic loops keep running in the
  player's `wall` mode, to match stock?
