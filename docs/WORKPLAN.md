# Work plan: seams and parallel packages

How M2–M5 get built by many agents at once. The plan fixes the contracts
between systems first (the **seams**), gives every lane a fake for each thing
it depends on, and splits the rest into **work packages** that each own their
files and end in a check an agent can run itself.

It builds on [TESTBENCH.md](TESTBENCH.md) (PR #2, not yet merged), whose
scenes, scenarios and digest checks are the done criteria for most packages,
and on PR #1 (merged), which brought `a2fmt::Odf::load_layered`.

## How to use this doc

**Assigning work.** Round 0 runs first and alone. After that, any package
whose *needs* are merged, or whose *can start against* fakes exist, is ready.
Open one GitHub issue per package (title `[SIM-06] Weapons`, label
`lane:sim`), assign it, and let the agent work on its branch. How sessions
check, claim and release packages: [AGENTS.md](../AGENTS.md).

**Picking up a package.** Read its row, the seams it names and the files it
owns. Work only in those files. Start against the listed fakes, not against
another package's unmerged branch. Finish when the *done when* check passes on
your branch, and paste the evidence (test names, scenario report lines,
digest) into the PR. If you need a seam to change, stop and open a seam PR
("Integration and merge protocol").

Status lives in the issues, not here. This file changes only when the plan
changes.

## Where main is (2026-11-08)

Most of M2 landed before this plan, inside `sta64`: `sta_bzn` reads all 517
installed maps; `sta64::map` places objects, sky, starfield, planets
(`planet.rs`, `sta_surface` recipes), volumetric nebulae, asteroid fields and
map lights; `sta64::units` runs `sta_sim` in `FixedUpdate`, interpolates by
`Time<Fixed>::overstep_fraction`, selects and orders moves; `sta_mission`
resolves scripts and dry-runs all 252 installed Lua missions; `docs/data/`
inventories every ODF/CFG/AIP key. What is missing is the separation: ODF
loading for the sim lives in `sta64::map`, the sim is driven from a Bevy
system, and seven systems read Bevy's `Time`. Round 0 carves that apart
before anything else is added.

## Round 0: the serial part

| ID | Package | Contents | Model |
|---|---|---|---|
| R0-1 | This doc | `docs/WORKPLAN.md`, pointers in ROADMAP/README | Opus |
| R0-2 | Seams as code + fakes | below | Opus |
| R0-3 | Carve `sta64` | below | Sonnet |

**R0-2 contains, and nothing else:**

1. New crate `crates/sta_core` (no deps): `Fixed` and `Vec3` moved from
   `sta_sim` (re-exported there, so no caller changes), `EntityId`,
   `PlayerId`, `TeamId`, `Tick`, `TICK_HZ`, `ContentKey`, `SEAM_REV`.
2. New crate `crates/sta_content`: the types of the content seam
   (`ContentDb`, `UnitDef`, `WeaponDef`, `Profile`, `Features`, `OdfIndex`,
   `MapDef`) with `ContentDb::builder()` working and every loader returning
   `Err(Unsupported)`; `sta_content::fake` (below).
3. In `sta_sim`: modules `command`, `event`, `view`, `setup`, `source`;
   `Sim` implements `WorldSource` with today's movement; `Sim::step` returns
   `TickOutput`; `fake::ScriptedSource`, `fake::ViewLog`. Existing tests keep
   their digests (the digest gains nothing new until SIM-01).
4. In `sta64`: `clock.rs` (`AnimClock`, wall mode working), `sim_link.rs`
   (`SimLinkPlugin`, messages, `SimEntity`, `CommandQueue`, `Selection`),
   `plugins.rs`, `app.rs` (`AppState`, `GameSet`, `LaunchConfig` with every
   planned flag parsed; unimplemented ones log `unsupported: <flag>`).
5. Workspace `Cargo.toml`: members and `[workspace.dependencies]` for every
   crate this plan creates (`sta_core`, `sta_content`, `sta_bench`,
   `sta_net`, `sta_ai`), each an empty `lib.rs` with a crate doc stating its
   owner lane; external deps `serde`, `json5`, `postcard`.
6. `clippy.toml` for `sta_core` and `sta_sim`: `disallowed-types`
   (`HashMap`, `HashSet`) and `disallowed-methods` (`f32::sin` and the other
   platform maths, `Instant::now`, `SystemTime::now`); see Determinism.
7. `docs/ARCHITECTURE.md`: `sta_core`/`sta_content` in the layer diagram.

Done when: `cargo test --workspace` and clippy pass on all four CI targets;
`cargo doc` shows every seam type; the fakes have their own tests.

**R0-3 contains:** `git mv` of the existing `sta64` modules into lane
directories (`render/` ← `model_scene`, `sprites`, `particles`, `textures`,
`planet`, `nebula`, `rock`, `space`, `surfaces`, `strategic`, `*.wgsl`;
`viewer/` ← `viewer`; `map/` ← `map`; `input/` ← selection and order code
from `units.rs`; `render/units.rs` ← placement); `units.rs`'s tick and
interpolation replaced by `sim_link` + `AnimClock`; `map.rs`'s ODF cache
replaced by a `ContentRes(Arc<ContentDb>)` built through a temporary
`sta_content::legacy` shim (CON-03 replaces it). Done when the map view
behaves as on main (TODO.md "Needs your check" items unchanged) and the
existing `units.rs` motion tests, ported to `AnimClock`, pass.

Packages that need nothing from round 0 may start while it runs: CON-01,
CON-08, CON-09, TOOL-01, LUA-02's deterministic `math.*` half.

## Seams

Each seam: owner, sketch, invariants, dependency direction, evolution.
Crate direction, enforced by `Cargo.toml`:

```
formats:   a2fmt, sta_fpq, sta_vfs, sta_odf, sta_bzn, sta_assets, sta_surface  (no sta_core)
sta_core   <- sta_content (+ sta_odf, sta_vfs, sta_bzn, json5)
sta_content <- sta_sim <- sta_mission, sta_ai, sta_net, sta_bench
all of the above <- sta64 (the only Bevy crate)
```

No Bevy-free crate depends on `sta64`; `sta_sim` never depends on
`sta_mission`, `sta_ai` or `sta_net` (they plug in through traits).

### S1. Core types — `sta_core`

```rust
pub use fixed::Fixed;                       // 32.32, moved from sta_sim
pub struct Vec3 { pub x: Fixed, pub y: Fixed, pub z: Fixed }
pub struct EntityId(pub u32);               // never reused; 0 = none
pub struct PlayerId(pub u8);                // 0..=7 players, SCRIPT = 254, BENCH = 255
pub struct TeamId(pub u8);
pub type Tick = u64;
pub const TICK_HZ: u32 = 30;
pub struct ContentKey(pub u32);             // index into ContentDb's name-sorted table
pub const SEAM_REV: u32 = 1;                // bumped on any breaking seam change
```

- Space is the maps': Direct3D left-handed, +Y up, ODF/BZN units. Headings
  are unit vectors in XZ (`(x, z)`), as `sta_sim` has them; the sim uses no
  trigonometry. Durations are ticks; ODF seconds convert once, at content
  load, rounding half up.
- `ContentKey`s are equal on every peer that loaded the same content, because
  they index names sorted bytewise. Never persist a key without the
  `ContentDb::hash()` it came from.
- Evolution: additive only. Moving or renaming a type is a breaking change.

### S2. Commands — `sta_sim::command`

```rust
pub struct Command { pub player: PlayerId, pub seq: u32, pub kind: CommandKind }
#[non_exhaustive] pub enum CommandKind { Order(Order), Edit(Edit) }
#[non_exhaustive] pub enum Order {
    Move { units: Vec<EntityId>, to: Vec3, queue: bool },
    Stop { units: Vec<EntityId> },
    Attack { units: Vec<EntityId>, target: EntityId },
    Build { builder: EntityId, what: ContentKey, at: Option<Vec3> },
    Formation { units: Vec<EntityId>, kind: u8 },
    Special { units: Vec<EntityId>, slot: u8, target: Target },
    Cloak { units: Vec<EntityId>, on: bool },
    Mine { units: Vec<EntityId>, node: EntityId },
}
#[non_exhaustive] pub enum Edit {           // TESTBENCH's Command::Bench
    Spawn { odf: ContentKey, player: PlayerId, pos: Vec3, heading: (Fixed, Fixed) },
    Kill(EntityId), Damage(EntityId, Fixed), Heal(EntityId), Immortal(EntityId, bool),
    SetEnergy(EntityId, Fixed), Tech(PlayerId, ContentKey), Cargo(PlayerId, u8, Fixed), …
}
pub trait CommandRouter {                    // local today, lockstep in sta_net
    fn submit(&mut self, cmd: Command);
    fn take_ready(&mut self, tick: Tick) -> Option<Vec<Command>>; // None = stall
}
pub struct LocalRouter;                      // everything lands on the next tick
```

- Commands are the only input to `Sim::step`. A tick applies its batch
  sorted by `(player, seq)`. Invalid commands (dead unit, wrong owner) are
  ignored deterministically and produce `SimEvent::Rejected`.
- `Edit` is accepted from `PlayerId::BENCH` (bench builds only) and
  `PlayerId::SCRIPT` (missions); lockstep peers reject it from players. Both
  are in the command log and the digest, so edited runs replay (TESTBENCH's
  open question, decided this way; see D4).
- Evolution: new variants and new fields with defaults within a milestone.
  The wire encoding (`postcard`) is versioned by `SEAM_REV`.

### S3. World view — `sta_sim::view`

```rust
pub struct WorldView {
    pub tick: Tick, pub digest: u64,
    pub units: Vec<UnitView>,       // ascending id
    pub players: Vec<PlayerView>,   // resources, officers, eliminated, relation table
}
pub struct UnitView {
    pub id: EntityId, pub def: ContentKey, pub owner: PlayerId,
    pub pos: Vec3, pub heading: (Fixed, Fixed), pub speed: Fixed,
    pub hull: [Fixed; 2], pub shields: [[Fixed; 2]; 4],   // [now, max]; fore/aft/port/stbd
    pub flags: UnitFlags,           // cloaked, dying, building, docked, selectable
    pub progress: Option<Fixed>,    // build/construction 0..1
}
pub struct TickOutput { pub view: Arc<WorldView>, pub events: Vec<SimEvent> }
pub trait WorldSource: Send {
    fn step(&mut self, commands: &[Command]) -> TickOutput;
    fn view(&self) -> Arc<WorldView>;
}
```

- The view is an immutable snapshot built at the end of each tick; readers
  never borrow `Sim`. Render, UI, audio, AI, Lua and the bench read only
  views and events.
- Everything in it is `Fixed`; readers convert with `to_f32` and never feed
  the result back.
- Per-player visibility comes later as `WorldView::visible_to(PlayerId)`
  (SIM-12); until then the view is omniscient.
- Evolution: fields are added, never removed, within a milestone.

### S4. Sim events — `sta_sim::event`

```rust
#[non_exhaustive] pub enum SimEvent {
    Spawned { id: EntityId }, Destroyed { id: EntityId, by: Option<EntityId> },
    Fired { shooter: EntityId, weapon: ContentKey, hardpoint: u8, target: Target },
    Hit { target: EntityId, by: EntityId, weapon: ContentKey, facing: u8,
          shield: Fixed, hull: Fixed, at: Vec3 },
    ShieldsDown { id: EntityId, facing: u8 },
    BuildStarted { builder: EntityId, what: ContentKey },
    Built { builder: EntityId, id: EntityId },
    Delivered { player: PlayerId, resource: u8, amount: Fixed },
    CloakChanged { id: EntityId, on: bool },
    Rejected { player: PlayerId, seq: u32 },
    PlayerEliminated { player: PlayerId },
}
```

- Emitted in deterministic order and identical on every peer, so marks and
  mission triggers can key on them; not hashed separately (they follow from
  state).
- Client-side events that the sim never sees are Bevy messages in
  `sta64::sim_link`: `OrderIssued { units, kind }` (voice acks play on input,
  not on the tick), `SelectionChanged`.
- Evolution: new variants freely (`#[non_exhaustive]`); consumers must
  ignore unknown ones.

### S5. Content database — `sta_content`

```rust
pub struct ContentDb { /* name-sorted defs, profile, features, hash */ }
impl ContentDb {
    pub fn load(vfs: &Vfs, profile: Profile, index: &OdfIndex) -> Result<(ContentDb, Vec<Warning>)>;
    pub fn builder() -> ContentBuilder;             // fakes and JSON5
    pub fn key(&self, odf: &str) -> Option<ContentKey>;
    pub fn unit(&self, k: ContentKey) -> Option<&UnitDef>;
    pub fn weapon(&self, k: ContentKey) -> Option<&WeaponDef>;
    pub fn hash(&self) -> u64;                       // folded into tick 0's digest
    pub fn profile(&self) -> Profile; pub fn features(&self) -> &Features;
}
pub struct UnitDef {
    pub key: ContentKey, pub odf: String, pub class: ClassLabel, pub race: String,
    pub model: String, pub scale: Fixed, pub movement: MoveStats,
    pub hull: Fixed, pub shields: ShieldDef, pub crew: Fixed, pub sensor: Fixed,
    pub hardpoints: Vec<Hardpoint>,                  // weapon key, node name, arc
    pub build: BuildDef,                             // cost, ticks, build list, officers
    pub sounds: SoundRefs, pub fx: FxRefs,           // names only; resolved client-side
    pub raw: Arc<sta_odf::Odf>,                      // unread keys stay reachable
}
pub enum Profile { Native, Fo4, Sta2Classic, Sta1Classic }
pub struct Features { pub directional_shields: bool, pub fire_arcs: bool,
    pub point_defense_cycles: bool, pub pay_on_queue: bool, … }  // one per NATIVE_FEATURES row
pub struct OdfIndex { /* stem -> (path, layer), duplicates */ }
```

- Every number reaches the sim as `Fixed`, converted once here
  (`Fixed::from_f64` of the parsed value, as `sta_sim` already rules). No
  other crate parses ODF numbers for the sim.
- Profile defaults first, then per-ODF keys switch features per unit.
- `ContentDb` is immutable after load and shared as `Arc`.
- Evolution: new defs and fields freely; renaming a field is breaking. Keys
  STA64 reads are marked in `docs/data/odf_keys.csv` by
  `tools/data_inventory.py`.

### S6. Maps — `sta_content::map`

```rust
pub struct MapDef {
    pub name: String, pub label: Option<String>, pub mission: Option<String>,
    pub bounds: (Vec3, Vec3),
    pub starts: Vec<StartPos>,                       // per player slot
    pub objects: Vec<MapObject>,                     // odf, owner slot, pos, heading, label
    pub regions: Vec<Region>,                        // nebula / asteroid field volumes (Fixed)
    pub scenery: Scenery,                            // f32; render only: sky, starfield,
}                                                    //  planets, lights, ambient
impl MapDef {
    pub fn from_bzn(m: &sta_bzn::Map, db: &ContentDb) -> (MapDef, Vec<Warning>);
    pub fn empty() -> MapDef;                        // the bench's built-in empty map
}
```

- `sta_bzn` stays a pure format crate; the conversion lives here because it
  needs ODF classes (`physicsFile`, `dont_load_SOD`, start markers).
- The sim spawns from `objects`, `starts` and `regions` only; `scenery`
  never enters the sim or the digest.
- JSON5 maps (native format) deserialize into the same type.

### S7. Game setup and controllers — `sta_sim::setup`

```rust
pub struct GameSetup { pub seed: u64, pub map: Arc<MapDef>, pub content: Arc<ContentDb>,
    pub players: Vec<PlayerSetup>, pub options: GameOptions }
pub struct PlayerSetup { pub id: PlayerId, pub team: TeamId, pub race: String,
    pub control: Control }                           // Human, Remote, Ai(name), Idle
impl Sim { pub fn from_setup(s: &GameSetup) -> Sim; }
pub trait Controller: Send {                         // AI, missions
    fn think(&mut self, view: &WorldView, events: &[SimEvent], out: &mut Vec<Command>);
}
```

- UI (Instant Action, lobby) produces a `GameSetup`; the sim, net and bench
  consume it. A replay is `GameSetup` + command log + digests.
- Controllers run on one host and send their commands through the
  `CommandRouter` like a player (assumption D3), so AI and Lua need not be
  bit-deterministic and replays never rerun them.

### S8. Client link — `sta64::sim_link`

```rust
#[derive(Resource)] pub struct SimLink { source: Box<dyn WorldSource>, router: Box<dyn CommandRouter>,
                                         prev: Arc<WorldView>, curr: Arc<WorldView> }
#[derive(Component)] pub struct SimEntity(pub EntityId);
#[derive(Resource)] pub struct SimEntities(BTreeMap<EntityId, Entity>);
#[derive(Message)] pub struct SimMsg(pub SimEvent);          // after each tick
#[derive(Message)] pub struct OrderIssued { pub units: Vec<EntityId>, pub kind: OrderKind }
#[derive(Resource)] pub struct CommandQueue(Vec<CommandKind>); // input pushes; link stamps player/seq
#[derive(Resource)] pub struct Selection(pub Vec<EntityId>);
#[derive(Resource)] pub struct ContentRes(pub Arc<ContentDb>);
#[derive(Message)] pub struct PlaySound { pub name: String, pub at: Option<Vec3>, pub bus: Bus }
```

- `SimLinkPlugin` decides how many ticks run this frame from `AnimClock`,
  steps the source, shifts `prev`/`curr`, and sends `SimMsg`s. Nothing else
  calls `step`.
- Interpolation: a thing is drawn at `lerp(prev, curr, AnimClock::alpha())`;
  heading lerps then renormalises. An id only in `curr` appears at `curr`; an
  id only in `prev` is gone (its `Destroyed` event drives the effect).
- Render-side entities link to the sim only by `SimEntity`; nothing keys on
  Bevy `Entity` across frames.
- `PlaySound` is how UI, render and input ask for audio without depending on
  the audio module.

### S9. Animation clock — `sta64::clock`

```rust
#[derive(Resource)] pub struct AnimClock { /* mode, sim_time, alpha, paused, seed */ }
impl AnimClock {
    pub fn now(&self) -> f64;              // sim time at last tick + alpha * tick
    pub fn alpha(&self) -> f32;
    pub fn wall(&self) -> f64;             // only for loops allowed to run while paused (wall mode)
    pub fn seed(&self) -> u64;
    pub fn entity_key(&self, id: EntityId) -> u64;
    pub const SUBSTEP: f64 = 1.0 / 120.0;  // fixed step for anything integrated per frame
}
```

TESTBENCH's rule, verbatim in effect: anything that moves on screen reads
`AnimClock`, never `Time`; cosmetic seeds come from `seed()` and
`entity_key(EntityId)`; integrated effects advance in `SUBSTEP`s. `bench`
mode advances one fixed `Step=` per `App::update`.

### S10. Plugins, sets and app modes — `sta64::{plugins, app}`

```rust
pub enum Mode { Viewer, Map, Game, FrontEnd, Bench { headless: bool, sim_only: bool } }
#[derive(States)] pub enum AppState { Boot, FrontEnd, Loading, InGame, Viewer }
#[derive(SystemSet)] pub enum GameSet { Input, SimLink, Interpolate, Render, Ui, Audio, Bench }
pub fn add_plugins(app: &mut App, cfg: &LaunchConfig) {
    // render — one line per package, append inside your lane's block
    // ui —
    // audio —
}
```

- Each package ships one `pub struct XxxPlugin` in its own module, registers
  systems only in its `GameSet`, gates them on `AppState`, and reads seams,
  not other packages' internals.
- `plugins.rs` adds render/UI/audio plugins only when the mode has a window;
  `--sim-only` adds none.
- Per-package settings: a `#[derive(Resource, Deserialize, Default)]` struct
  under the package's own section of `settings.toml` (`[render.nebula]`),
  loaded by `app.rs`.

### Determinism rules (sim crates: `sta_core`, `sta_content`, `sta_sim`)

1. No `f32`/`f64` in state or in arithmetic that feeds state; floats only at
   load (S5) and out to rendering. No platform maths.
2. `BTreeMap`/sorted `Vec` only; no `HashMap`/`HashSet` (clippy-enforced).
3. One seeded `Rng`; cosmetic randomness never touches it.
4. Commands apply at tick boundaries in `(player, seq)` order; ids are never
   reused; entities iterate by ascending id.
5. No threads, I/O or clocks inside `step`.
6. Golden digests at fixed ticks are committed and checked on all four CI
   targets (TOOL-06).

## Fakes and fixtures

Every fake ships in round 0 (R0-2) unless marked, lives with the seam it
fakes, and contains no game data.

| Fake | Where | Stands in for | Used by |
|---|---|---|---|
| `sta_content::fake::db()`: 6 units (`fx_cruiser`, `fx_frigate`, `fx_station`, `fx_yard`, `fx_miner`, `fx_moon`), 3 weapons (beam, torpedo, point defence), built in Rust | `crates/sta_content/src/fake.rs` | ODF content DB (CON-03) | SIM, AI, LUA, TOOL |
| JSON5 copies of the same units + boxy GLB models | `fixtures/content/`, `fixtures/models/` (CON-04) | real installs in CI | RND, TOOL CI scenes |
| `MapDef::empty()`, `fake::two_bases()` (2 starts, moon, nebula region, asteroid region, sky/planet scenery) | `sta_content::map`, `fake.rs` | BZN → MapDef (CON-07) | SIM-01, RND, UI, TOOL |
| `ScriptedSource`: a `WorldSource` that plays a JSON5 timeline (`at 30: Fired …`, `at 45: Hit …`, units on rails) | `crates/sta_sim/src/fake.rs` | combat/economy sim | RND FX, AUD, UI HUD |
| `ViewLog`: record a real run's `TickOutput`s to `postcard`, replay as a `WorldSource` | same | any sim | render perf, AUD |
| `NullAudio`: records `PlaySound`/positional requests | `sta64/src/audio/null.rs` (AUD-01) | audio backend | UI, tests |
| `LocalRouter`, `LoopbackNet` (two routers over a channel) | `sta_sim::command`; `sta_net` (NET-02) | relay | NET, TOOL |
| `IdleController` | `sta_ai` (AI-01) | skirmish AI | INT-M3 |
| `fixtures/odf/`: hand-written ODFs: includes, FO self-include, A1 `addon/`, `;`/`f` numbers | CON-01/02 | install ODFs | CON |

## Ownership and shared files

Every package owns the paths in its *owns* column. Hotspots and their rules:

| Hotspot | Rule |
|---|---|
| `sta64/src/main.rs`, `app.rs` | R0 and INT packages only. Others never edit them. |
| `sta64/src/plugins.rs` | Append-only: one `app.add_plugins(...)` line per package, at the end of its lane's block. Rebase conflicts are resolved by keeping both lines. |
| `sta64/src/sim_link.rs`, `clock.rs`, `sta_core`, the seam modules of `sta_content`/`sta_sim` | Seam files: changed only by seam PRs. |
| Workspace `Cargo.toml` | Members written once in R0-2. A new external dependency is one line appended to `[workspace.dependencies]`, named in the PR description. A new crate needs a seam PR. |
| `sta64/Cargo.toml`, `Cargo.lock` | Append one dependency per line. Never hand-merge `Cargo.lock`: rebase, then `cargo update -w`. |
| `.github/workflows/*` | TOOL lane. Others request a job in their PR description; TOOL-06 adds it. |
| `docs/ROADMAP.md`, `TODO.md` | Ticked only by INT packages and the user. `TODO.md` stays the user's eye-check list; packages add `[?]` lines only under "Needs your check", append-only. |
| `docs/NATIVE_FEATURES.md` | A package may change the Status cell of the rows it implements, nothing else. |
| `docs/data/*` | Generated. Never hand-edited or hand-merged; rerun `tools/data_inventory.py`. |
| `bench/scenes/`, `bench/scenarios/`, `fixtures/` | New files only; a package edits only files it added. Shared fixtures (`fixtures/content/`) are CON-04's; others append new files. |
| Existing `sta64` modules after R0-3 | Owned as listed per package below; one owner each. |

## Lanes and work packages

Columns: **needs** must be merged first; **against** lists the fakes that let
work start earlier; size S ≤ 1 day, M ≤ 1 week, L > 1 week of agent work;
model: Haiku for mechanical work, Sonnet by default, Opus for hard-to-reverse
design. `✓main` marks work already on main that the package moves behind a
seam rather than writes.

### Content and formats (CON)

| ID | Package | Owns | Needs | Against | Unlocks | Done when | Size | Model |
|---|---|---|---|---|---|---|---|---|
| CON-01 | FO self-include rule in `sta_odf::resolve` (adopt `a2fmt::Odf::load_layered` semantics: a file may include its own name from a lower layer) | `crates/sta_odf/` | — | `fixtures/odf/` | CON-02, CON-03 | Fixture test: self-include resolves, true cycle still `IncludeCycle`; install-gated: 0 `IncludeCycle` over all FO4 ODFs | S | Sonnet |
| CON-02 | ODF index: all ODF folders, recursive, A1 `addon/`, duplicate warnings (replaces `map.rs` stem cache) | `sta_content/src/index.rs` | R0-2 | synthetic `Vfs` | CON-03, CON-07 | Unit tests; `--example odf_index` prints counts and duplicates for A1/A2/FO | M | Sonnet |
| CON-03 | Content DB from ODF: `UnitDef`, `WeaponDef`, movement (from `units.rs`), build, sounds, FX refs; replaces R0-3's shim | `sta_content/src/{odf,unit,weapon}.rs` | CON-01, CON-02 | `fake::db()` | SIM-04+, AUD-02 | Every FO4 and A2 craft ODF yields a def or a listed warning (install-gated); `odf_keys.csv` ✓ column regenerated | L | Sonnet |
| CON-04 | JSON5 content + synthetic fixtures (units, weapons, GLB boxes), schema doc | `sta_content/src/json5.rs`, `fixtures/content/`, `fixtures/models/` | R0-2 | — | TOOL-06, CI scenes | `fixtures/content` loads to the same `hash()` as `fake::db()` | M | Sonnet |
| CON-05 | Profiles and feature switches: profile from mod chain, defaults table, per-ODF overrides | `sta_content/src/profile.rs` | R0-2 | — | SIM combat rules | Table test per profile; FO4 / STA2 Classic / STA1 Classic detected on installs | M | Sonnet |
| CON-06 | Tech tree and build lists (`.tt`, FO techtree `.xml`, `docs/data/techtree.md`) | `sta_content/src/techtree.rs` | CON-03 | `fake::db()` | SIM-09, AI-02, UI-04 | All installed `.tt`/`.xml` parse; prerequisites test | M | Sonnet |
| CON-07 | `MapDef::from_bzn`: objects, start positions, regions, scenery, mission; `map.rs` reads `scenery` | `sta_content/src/map.rs` (not the seam types) | R0-3, CON-02 | `two_bases()` | SIM-01, TOOL-05 | All 517 maps convert with counts equal to `sta_bzn`'s; start positions found on every multiplayer map | M | Sonnet |
| CON-08 | M3D 1.93 layout; Legacy LOD (type 11) and effect (type 2) nodes | `sta_assets/src/m3d.rs` | — | — | RND-07 | Every installed M3D parses; LOD nodes listed by `model_info` | M | Sonnet |
| CON-09 | glTF/GLB → `sta_assets::model::Model` | `sta_assets/src/gltf.rs` | — | `fixtures/models/` | native content | Fixture GLBs load; viewer shows them (`--view fx_cruiser`) | M | Sonnet |
| CON-10 | Audio data: sound lookup by bare name (OGG > WAV > MP3 per layer), WAV/MP3 decode, `audio.odf`/`events.odf` sound tables | `sta_assets/src/audio.rs` | — | — | AUD-01..04 | Every sound named by FO4 ODFs resolves or is listed; decode tests | M | Sonnet |
| CON-11 | Language packs: `Dynamic_Localized_Strings.h`, `label.map` → `StringTable` (shares `sta_mission::Labels`) | `sta_content/src/strings.rs` | R0-2 | — | UI-09 | All installed packs parse; lookup test | S | Haiku |
| CON-12 | GUI cfg (`docs/data/cfg.md`) → typed layouts for HUD/menus | `sta_content/src/gui_cfg.rs` | R0-2 | — | UI-04 | All 43 FO `.cfg` parse; layout snapshot tests | M | Sonnet |
| CON-13 | Mod discovery: list mods (`info.ini`), chain, metadata | `sta_vfs/src/mods.rs` | — | temp dirs | UI-07 | Unit tests; lists installed mods | S | Haiku |
| CON-14 | AIP files (`docs/data/aip.md`) → typed AI params | `sta_content/src/aip.rs` | R0-2 | — | AI-02 | All installed `.aip` parse | S | Haiku |
| CON-15 | Armada I `.zfs` archives: reader, mounted per layer (`.odf` → `odf/`, `.tt` → `techtree/`, rest at root; loose > FPQ > ZFS), `Source::Zfs`, `Vfs::read_from` | `sta_vfs/src/zfs.rs`, ZFS mount in `sta_vfs/src/lib.rs` | — | synthetic archives | A1 content in CON-02/03/06 | Synthetic-archive tests; install-gated: A1 retail `assets.zfs` 516 entries, `objtxt.zfs` 81; `odf_index` resolves every A1 retail ODF | S | Sonnet |

### Simulation (SIM)

| ID | Package | Owns | Needs | Against | Unlocks | Done when | Size | Model |
|---|---|---|---|---|---|---|---|---|
| SIM-01 | Entity model: kinds (craft, station, resource, projectile), id-ordered storage, `Sim::from_setup` spawning from `MapDef` | `sta_sim/src/world.rs`, `spawn.rs` | R0-2 | `fake::db()`, `two_bases()` | all SIM | `two_bases` spawns N entities; golden digest at tick 300 | M | Opus |
| SIM-02 | Movement on top of SIM-01 (✓main kinematics), queued waypoints, formations, stop | `sta_sim/src/movement.rs` | SIM-01 | `fake::db()` | UI-02, AI-02 | Bench `goto` scenario arrives within N ticks; digest golden | M | Sonnet |
| SIM-03 | Spatial grid, separation, collision with planets/regions | `sta_sim/src/spatial.rs` | SIM-01 | — | SIM-04, SIM-08 | Property tests; digest stable with 500 units | M | Sonnet |
| SIM-04 | Weapons: targeting, hardpoints, fire arcs, refire/bursts, beams vs projectiles; `Fired`/`Hit` | `sta_sim/src/combat/weapons.rs` | SIM-03 | `fake::db()` (CON-03 for FO) | SIM-05, RND-09, AUD-02 | Bench `firing`: `fire.start`/`fire.end` marks at expected ticks | L | Sonnet |
| SIM-05 | Damage, directional shields, systems damage, destruction | `sta_sim/src/combat/damage.rs` | SIM-04 | — | RND-10, SIM-13 | Bench `shields`, `explosion`: hull/shield curves match expected values | M | Sonnet |
| SIM-06 | Point-defence cycles | `sta_sim/src/combat/pd.rs` | SIM-04 | — | — | Bench torpedo-vs-PD scenario | S | Sonnet |
| SIM-07 | Resources and mining (dilithium, latinum, metal, biomatter; freighters) | `sta_sim/src/economy/` | SIM-02 | `fake::db()` | AI-02, UI-04 | Bench `mining`, `freighters`: income per minute | M | Sonnet |
| SIM-08 | Construction: rig deploy → weld → reveal, yards, build queues, multi-yard, officers, pay-on-queue | `sta_sim/src/production/` | SIM-07, CON-06 | `fake::db()` | AI-02, UI-04 | Bench `construction`: station appears at expected tick; queue tests | L | Sonnet |
| SIM-09 | Tech availability from tech tree | `sta_sim/src/tech.rs` | CON-06, SIM-08 | — | UI-04 | Build rejected until prerequisite exists | S | Sonnet |
| SIM-10 | Snapshot save/load (`postcard`, versioned) | `sta_sim/src/snapshot.rs` | SIM-01 | — | `Start=snapshot:`, saves | Reloaded digest equals continued run at tick N+300 | M | Sonnet |
| SIM-11 | Replay file (`GameSetup` + command log + digests) | `sta_sim/src/replay.rs` | SIM-01 | — | NET-02, TOOL `Start=replay:` | Record → replay → equal digests | S | Sonnet |
| SIM-12 | Sensors and per-player visibility (`visible_to`) | `sta_sim/src/sensors.rs` | SIM-03 | — | RND-13, AI | Bench visibility scenario | M | Sonnet |
| SIM-13 | Victory/elimination | `sta_sim/src/victory.rs` | SIM-05 | — | INT-M3 | Last-unit-killed scenario emits `PlayerEliminated` | S | Haiku |
| SIM-14 | Cloak and specials framework | `sta_sim/src/specials/` | SIM-05 | — | RND-11 | Bench `cloak` marks | M | Sonnet |

Later SIM backlog (rows in NATIVE_FEATURES "Simulation"): energy systems,
turrets, squadrons, breeders/infestation, collective XP, wreckage, upgrades.
Each becomes a package of the same shape when M3 closes.

### Render (RND) — all under `sta64/src/render/`

| ID | Package | Owns | Needs | Against | Unlocks | Done when | Size | Model |
|---|---|---|---|---|---|---|---|---|
| RND-01 | `AnimClock` everywhere: `model_scene`, `sprites`, `particles` (fixed sub-steps, per-emitter seed), map spin, `planet`, `nebula`, viewer light | those files | R0-3 | — | TOOL-04 frames | Grep: no `Res<Time>` in `render/`; two `--headless --clock bench` frames identical | M | Sonnet |
| RND-02 | Units from views: spawn/despawn on view diff, team colour per player, damage nodes by hull share | `render/units.rs` | R0-3 | `ScriptedSource` | RND-09..12 | Scripted spawn/destroy test; no orphan entities after 1,000 ticks | M | Sonnet |
| RND-03 | Strategic view: fleet icon stacking, grid and height bars (TODO.md) | `render/strategic.rs` | R0-3 | `ViewLog` | — | Icon count test; `[?]` check line | M | Sonnet |
| RND-04 | Custom SOD materials (~30 names; `blend_survey`) | `render/materials.rs` | R0-3 | — | — | Every listed material mapped; viewer check line | M | Sonnet |
| RND-05 | Map scenery extras: nebula lightning/ring/swirl, ship shadows, moon ravine parallax, glow shell, billboard fade | `render/{nebula,rock}.rs` | R0-3 | `two_bases()` | — | Recipe tests in `sta_surface`; check lines | L | Sonnet |
| RND-06 | Hull lighting from Refit `render3d/mods/hulls/shaders/hulls.hlsl` → WGSL | `render/hulls.rs`, `hulls.wgsl` | R0-3 | — | — | Viewer A/B toggle; frame diff vs previous within tolerance | M | Sonnet |
| RND-07 | Cull 2800, fade from 0.9×, far 20000 / near 20, LOD switch; render marks (culled, LOD) | `render/cull.rs` | CON-08 for LOD nodes | `ViewLog` | TOOL parity | Marks match Refit `planet` scene parity | S | Sonnet |
| RND-08 | Selection overlays: rings, health/shield bars, waypoint lines (`render3d/mods/overlays`) | `render/overlays.rs` | R0-3 | `ScriptedSource` | UI-04 | Frame test; overlay count marks | M | Sonnet |
| RND-09 | Weapon FX: beams, ordnance, muzzle flashes (`render3d/mods/ordnance`) from `Fired` | `render/fx/weapons.rs` | R0-3 | `ScriptedSource` | INT-M3 | Scripted `firing` timeline frames identical across runs | L | Sonnet |
| RND-10 | Impacts, shield hits, explosions, damage smoke (`mods/effects/effects_{shields,explosions}.hlsl`) | `render/fx/impacts.rs` | R0-3 | `ScriptedSource` | INT-M3 | as RND-09 | L | Sonnet |
| RND-11 | Construction, repair, warp, cloak effects (`effects_construction`, `effects_repair`, `mods/specials`) | `render/fx/specials.rs` | R0-3 | `ScriptedSource` | INT-M3 | as RND-09 | M | Sonnet |
| RND-12 | Planet city lights (needs colony population in view) | `render/planet.rs` | SIM-07 | `ScriptedSource` | — | Check line | S | Sonnet |
| RND-13 | Fog-of-war rendering | `render/fog.rs` | SIM-12 | `ScriptedSource` | — | Frame test | M | Sonnet |
| RND-14 | Opt-in post FX (Refit `postfx`) | `render/post.rs` | — | — | — | Setting toggles; frame test | S | Sonnet |

### UI and input (UI)

| ID | Package | Owns | Needs | Against | Unlocks | Done when | Size | Model |
|---|---|---|---|---|---|---|---|---|
| UI-01 | Selection QoL: control groups, double-click class, Refit `qol/README.md` rules, rebindable keys | `sta64/src/input/select.rs`, `keys.rs` | R0-3 | `ViewLog` | UI-02 | Input-injection tests (TESTBENCH M4 mechanism) | M | Sonnet |
| UI-02 | Orders from input: move, attack, stop, queue (Shift), formation; `OrderIssued` | `sta64/src/input/orders.rs` | R0-3 | `LocalRouter` | AUD-03 | Injected right-click yields the expected `Command` | M | Sonnet |
| UI-03 | LCARS theme and widget kit (Refit `ui/STYLE.md`), SDF fonts/icons via a port of `ui/fontbake` | `sta64/src/ui/theme/`, `crates/sta64/assets/ui/` (generated, ours) | R0-3 | — | UI-04..08 | Widget gallery screen; frame baselines (local) | L | Sonnet |
| UI-04 | HUD: cfg-driven layout, three info-panel heights, command grid (Refit `grid`), build buttons, resources bar (`ui/HUD.md`) | `sta64/src/ui/hud/` | UI-03, CON-12 | `ScriptedSource`, `fake::db()` | INT-M3 | Bench `ui-hud-layout` passes | L | Sonnet |
| UI-05 | Minimap / radar | `sta64/src/ui/radar.rs` | UI-03 | `ViewLog` | INT-M3 | Frame test | M | Sonnet |
| UI-06 | Front-end menu scene: live map, named views, decloak/warp moments (Refit `render3d/content/menu/map.ini`) | `sta64/src/frontend/scene.rs`, `assets/menu/` | CON-07, RND-02 | `two_bases()` | UI-07 | Scene loads headless; view-switch marks | L | Sonnet |
| UI-07 | Screens: main, single player, game setup/Instant Action, settings, mod select (`ui/MENUS.md`) → `GameSetup` | `sta64/src/frontend/screens/` | UI-03, CON-13 | fake scene | INT-M4 | Each screen reached by state; setup round-trip test | L | Sonnet |
| UI-08 | Loading screen over the map's sky (`ui/LOADING.md`) | `sta64/src/frontend/loading.rs` | UI-03 | `two_bases()` | INT-M4 | Progress marks; frame test | M | Sonnet |
| UI-09 | All UI text through `StringTable`; language switch | `sta64/src/ui/i18n.rs` | CON-11 | — | INT-M4 | No literal UI strings (lint test) | S | Haiku |
| UI-10 | Tooltips, notices, chat log | `sta64/src/ui/notices.rs` | UI-03 | `ScriptedSource` | — | Notice on `Destroyed` test | S | Sonnet |
| UI-11 | Lobby | `sta64/src/frontend/lobby.rs` | UI-07, NET-03 | `LoopbackNet` | INT-M5 | Two clients join a room in-process | M | Sonnet |

### Audio (AUD) — new; not yet on the roadmap

| ID | Package | Owns | Needs | Against | Unlocks | Done when | Size | Model |
|---|---|---|---|---|---|---|---|---|
| AUD-01 | Backend and mixer: buses (master, music, sfx, voice, ui), volumes from settings, `PlaySound`, `NullAudio` | `sta64/src/audio/{mod,mixer,null}.rs` | R0-3 | — | AUD-02..05 | `NullAudio` records the expected calls; tone plays in a manual check | M | Sonnet |
| AUD-02 | Positional SFX from `SimMsg` (fire, hit, explode, built), attenuation from camera, voice limits | `audio/sfx.rs` | AUD-01, CON-10 | `ScriptedSource` | INT-M3 | Scripted `firing` → expected `NullAudio` log | M | Sonnet |
| AUD-03 | Unit voices: select/move/attack acks, under-attack alerts | `audio/voices.rs` | AUD-01, CON-10 | `OrderIssued` injection | INT-M3 | Ack chosen per ODF; rate limit test | M | Sonnet |
| AUD-04 | Music: playlists, calm/battle switching from events, menu music | `audio/music.rs` | AUD-01 | `ScriptedSource` | INT-M4 | State-machine tests | M | Sonnet |
| AUD-05 | UI sounds | `audio/ui.rs` | AUD-01 | — | INT-M4 | Theme click names resolve | S | Haiku |

### Tooling, bench, CI and release (TOOL)

| ID | Package | Owns | Needs | Against | Unlocks | Done when | Size | Model |
|---|---|---|---|---|---|---|---|---|
| TOOL-01 | `sta_bench` parsers: `SceneFile`, `BenchCommand`, `Scenario`; Refit `firing.ini`/`planet.ini` as fixtures | `crates/sta_bench/src/parse/`, `bench/` | PR #2 merged | — | TOOL-02..04 | Parser tests on Refit fixtures | M | Sonnet |
| TOOL-02 | Tier-1 runner: scene → `GameSetup` → `Sim`; answers, `objects.jsonl`, digest steps, report dir | `sta_bench/src/run/` | TOOL-01, SIM-01 | `fake::db()` | every SIM package's scenarios | `planet` and `firing` scenes pass in-process | M | Sonnet |
| TOOL-03 | `sta64 --scene --headless --sim-only --clock bench --ticks --control`; command channel | `sta64/src/bench/` | R0-3, TOOL-01 | — | TOOL-04 | `sta64 --sim-only --scene bench/scenes/synthetic.ini --ticks 300` exits 0 with digest | M | Sonnet |
| TOOL-04 | `sta-bench` binary: run/check/list/session/drive/diff, frames | `sta_bench/src/bin/` | TOOL-03, RND-01 | — | tier-2 checks | Same frame twice for `planet` | M | Sonnet |
| TOOL-05 | Map corpus test (from `maps_report`): every map loads, ticks 300, same digest twice | `sta_bench/tests/corpus.rs` | CON-07, SIM-01 | — | CI confidence | Passes on stock and FO 4.0 (install-gated) | S | Sonnet |
| TOOL-06 | CI: golden digests on four targets, bench jobs, `--sim-only` smoke, artefact upload | `.github/workflows/` | TOOL-02, CON-04 | — | — | CI green with a deliberately broken digest failing | S | Haiku |
| TOOL-07 | Marks from `SimMsg` and render marks | `sta_bench/src/marks.rs`, `sta64/src/bench/marks.rs` | TOOL-03 | `ScriptedSource` | parity | Marks file matches Refit names | S | Sonnet |
| TOOL-08 | Parity diff vs Refit's bench (objects by name, marks in sim seconds) | `sta_bench/src/parity.rs` | TOOL-07 | Refit sample output | M3 parity report | Diff of two identical runs is empty | M | Sonnet |
| TOOL-09 | Packaging and release: tagged builds for four targets, `packaging/linux` desktop entry, Windows zip, macOS bundle | `packaging/`, `.github/workflows/release.yml` | TOOL-06 | — | release | Tag on a fork produces four artefacts | M | Sonnet |

### Lua missions (LUA)

| ID | Package | Owns | Needs | Against | Unlocks | Done when | Size | Model |
|---|---|---|---|---|---|---|---|---|
| LUA-01 | Mission runtime as a `Controller`: `sta_mission::script` from dry run to live; error isolation | `sta_mission/src/runtime.rs` | R0-2, LUA-02 | `ScriptedSource` | LUA-03 | A script's `MMM.register` callback runs per tick | M | Sonnet |
| LUA-02 | Deterministic `math.*` and `math.random` (MISSIONS.md "Determinism") | `sta_mission/src/math.rs` | — | — | LUA-01 | Bit-exact golden values on four targets | S | Sonnet |
| LUA-03 | MMM API, most used first (MMM_API.md: `Entity`, `EntityFinder`, `Monitor`, `Player` relations, objectives, `giveOrder`) | `sta_mission/src/api/` | LUA-01, SIM-01 | `fake::db()` | LUA-04 | Per-function tests; share of installed scripts that run 300 ticks without stub hits | L | Sonnet |
| LUA-04 | Every installed mission runs headless on its own map | `sta_mission/tests/missions.rs` | LUA-03, TOOL-05 | — | INT-M5 | Report: scripts run / stubbed / failed (install-gated) | S | Sonnet |

### AI (AI)

| ID | Package | Owns | Needs | Against | Unlocks | Done when | Size | Model |
|---|---|---|---|---|---|---|---|---|
| AI-01 | `sta_ai` crate: `IdleController`, controller registry by name | `crates/sta_ai/src/{lib,idle}.rs` | R0-2 | — | INT-M3 | Idle player issues no commands over 10 min | S | Haiku |
| AI-02 | Skirmish AI: economy, build order from tech tree and AIP, defence, attack waves | `sta_ai/src/skirmish/` | SIM-08, CON-06, CON-14 | `fake::db()` | AI-03, INT-M5 | Beats `IdleController` on `two_bases` within 30 sim minutes | L | Sonnet |
| AI-03 | AI-vs-AI soak | `sta_ai/tests/soak.rs` | AI-02, TOOL-02 | — | — | 10 seeded games end without panic; digests stable | S | Sonnet |

### Net (NET)

| ID | Package | Owns | Needs | Against | Unlocks | Done when | Size | Model |
|---|---|---|---|---|---|---|---|---|
| NET-01 | Lockstep protocol design: turn length, input delay, `postcard` wire format, digest exchange, desync report = replay, version/content-hash handshake | `docs/NET.md`, `sta_net/src/proto.rs` | R0-2 | — | NET-02, NET-03 | Doc reviewed by the user; proto round-trip tests | M | Opus |
| NET-02 | Lockstep `CommandRouter`; two sims in one process; desync injection | `sta_net/src/lockstep.rs` | NET-01, SIM-11 | `LoopbackNet` | NET-03, INT-M5 | Two in-process peers equal digests 10k ticks; injected desync detected within 1 turn and written as a replay | L | Sonnet |
| NET-03 | Transport and relay (design notes in Refit `online/README.md`, new protocol): rooms, NAT-free relay, reconnect | `sta_net/src/transport/`, `crates/sta_net/src/bin/relay.rs` | NET-01 | `LoopbackNet` | UI-11, INT-M5 | Two processes play 5 min over a local relay | L | Sonnet |

### Modder tools (EDT) — ported from A2FOExtensions

The four authoring tools in A2FOExtensions (`tools/A2FOArcLab`,
`A2FOIconLab`, `A2FOTooltipLab`, `ArmadaUIEditor`) are Bevy 0.13 +
`bevy_egui` apps, each with its own ODF, SOD, SPR and CFG readers. Each
package rebuilds one on STA64's crates instead (`sta_vfs` mod chain with FPQ
and ZFS, `sta_content::OdfIndex`, `sta_odf::resolve_layered`, `a2fmt`
SOD/SPR, `sta_assets` textures, and the CON loaders), on Bevy 0.20 with
`bevy_egui` 0.43. So a tool sees files exactly as the engine does, and
fixing a reader fixes both. The tools keep editing legacy files (A1/A2/FO
mods). They are standalone binaries, not `sta64` modes, so the R0-3 freeze
and the `sta64` hotspots don't apply. Behaviour and UI follow the original
READMEs; the A2FOExtensions sources are Immy's own and may be adapted.

| ID | Package | Owns | Needs | Against | Unlocks | Done when | Size | Model |
|---|---|---|---|---|---|---|---|---|
| EDT-01 | Tool shell: `sta_tools` crate (Bevy 0.20 + `bevy_egui` 0.43), install/mod picker over `Vfs::mount_install`, file browser over `OdfIndex`, atomic save with backup, window icon, shared viewport (orbit, Top/Bottom/Front/Back/Left/Right/Fit) | `crates/sta_tools/` (lib) | CON-02 (merged) | temp installs | EDT-02..05 | A demo bin opens an install and a mod, lists ODFs by root, previews a SOD; save round-trip test | M | Sonnet |
| EDT-02 | Arc Lab: weapon ↔ hardpoint discovery, fire-arc volumes at hardpoints, damage/logo decal preview, generated ODF blocks (copy/save), `--inspect <ship.odf>` headless | `crates/sta_tools/src/bin/arc_lab/` | EDT-01, SIM-04 (arc test) | the original's arc maths until SIM-04 lands | — | `--inspect` agrees with the original on its fixtures; the arc test is SIM-04's code, not a copy | M | Sonnet |
| EDT-03 | Icon Lab: `weaponXiconpos` editor previewed in the faction's info panel (`systembackgrounds.spr`, `systemimages.spr`, faction CFG `infoSingleSystemsIcon`), slots 1–128 | `crates/sta_tools/src/bin/icon_lab/` | EDT-01, CON-12 | — | — | Opens a FO4 ship, moves an icon, saves; the panel layout matches CON-12's typed CFG | M | Sonnet |
| EDT-04 | Tooltip Lab: `Dynamic_Localized_Strings.h` editor with validation and a live preview from the mod's own `FontSmall.spr` glyph map (resolved through `ParentMod`) | `crates/sta_tools/src/bin/tooltip_lab/` | EDT-01, CON-11 | — | — | Round-trips every installed strings file byte-identically when unchanged; preview uses the mod's glyph table | M | Sonnet |
| EDT-05 | UI Editor: A1/A2/FO `gui_*.cfg` + `gui_*.spr` layout editor, A1 640×480 vs A2 `screenWidth`/`screenHeight`, recursive `#include` effective values | `crates/sta_tools/src/bin/ui_editor/` | EDT-01, CON-12 | — | UI-04 (shared reading of layouts) | Opens stock A1, A2 and FO interfaces; edit → save → reload equal | L | Sonnet |
| EDT-06 | ODF formatter (`format_odfs.py` → Rust): lossless ODF syntax tree in `sta_odf` (comments, order, continuation rows kept), grouped layout after the FO Guide templates; dry run by default, `--write` atomic, `--report` JSON | `crates/sta_odf/src/cst.rs`, `crates/sta_tools/src/bin/odf_fmt.rs` | CON-01 (merged) | — | — | Every installed ODF round-trips byte-identically unformatted; formatted output resolves to the same values (`resolve_layered`) as the input | M | Sonnet |

### Integration (INT)

One package per milestone owns `main.rs`, `app.rs`, `sta64/src/game/` (the
match flow) and the roadmap ticks. It writes no feature code; it wires plugins
and fixes seams through seam PRs.

| ID | Package | Needs | Done when | Size | Model |
|---|---|---|---|---|---|
| INT-M2 | Map you can fly in, on seams | R0-3, RND-01, CON-07, SIM-01, SIM-02, TOOL-03, TOOL-05 | `--map` plays as today from `MapDef` + `GameSetup`; corpus test green; ROADMAP M2 ticked | M | Sonnet |
| INT-M3 | FO4 skirmish vs idle opponent | SIM-04..08, SIM-13, CON-03, CON-06, RND-09..11, UI-04, AUD-02, AUD-03, AI-01 | Bench skirmish scenario: player destroys idle opponent; first parity report (TOOL-08) | M | Sonnet |
| INT-M4 | Front end → Instant Action → game → back | UI-06..09, AUD-04, CON-13, TOOL-09 | Scenario reaches each screen by state and starts a match from setup | M | Sonnet |
| INT-M5 | Multiplayer with AI and missions | NET-02, NET-03, UI-11, AI-02, LUA-04 | Two-process match with an AI and a Lua mission, no desync over 30 min | M | Sonnet |

Counts: CON 15, SIM 14, RND 14, UI 11, AUD 5, TOOL 9, LUA 4, AI 3, NET 3,
EDT 6, INT 4 (98), plus 3 in round 0.

## Dependency graph

```mermaid
graph LR
  R0[R0-2 seams + fakes] --> R03[R0-3 carve sta64]
  CON01 --> CON03; CON02 --> CON03; R0 --> CON02
  CON02 --> CON07; R03 --> CON07
  R0 --> SIM01 --> SIM02 & SIM03 & SIM10 & SIM11
  SIM03 --> SIM04 --> SIM05 --> SIM13 & SIM14
  SIM04 --> SIM06
  SIM02 --> SIM07 --> SIM08 --> SIM09
  CON03 --> CON06 --> SIM08
  R03 --> RND01 & RND02 & RND09 & RND10 & UI01 & UI02 & UI03 & AUD01 & TOOL03
  UI03 --> UI04 & UI07; CON12 --> UI04
  TOOL01 --> TOOL02 & TOOL03; SIM01 --> TOOL02
  CON07 --> TOOL05; SIM01 --> TOOL05
  SIM08 --> AI02; CON14 --> AI02
  NET01 --> NET02 & NET03; SIM11 --> NET02
  CON02 --> EDT01 --> EDT02 & EDT03 & EDT04 & EDT05; SIM04 --> EDT02; CON12 --> EDT03 & EDT05; CON11 --> EDT04; CON01 --> EDT06
  LUA02 --> LUA01 --> LUA03 --> LUA04
  CON07 & SIM02 & RND01 & TOOL05 --> INTM2[INT-M2]
  SIM05 & SIM08 & RND09 & UI04 & AI01 --> INTM3[INT-M3]
  UI07 & UI06 --> INTM4[INT-M4]
  NET02 & NET03 & AI02 & LUA04 --> INTM5[INT-M5]
```

**Start right after round 0** (needs met or fakes enough): CON-02, CON-04,
CON-05, CON-08, CON-09, CON-10, CON-11, CON-12, CON-13, CON-14; SIM-01;
RND-01..06, RND-08..11, RND-14; UI-01, UI-02, UI-03; AUD-01; TOOL-01,
TOOL-03; LUA-02 (LUA-01 right after); AI-01; NET-01. That is about 30 packages at once;
the user's review bandwidth, not the graph, is the limit.

**Critical path.** R0-2 → R0-3 → SIM-01 → SIM-03 → SIM-04 → SIM-05 → INT-M3,
with SIM-02 → SIM-07 → SIM-08 → AI-02 → INT-M5 as the longest chain overall
(three L packages). Content feeds it: CON-01/02 → CON-03 → CON-06 → SIM-08.
Everything in RND, UI and AUD runs beside it on `ScriptedSource` and joins at
INT-M3/INT-M4. NET is short but its design (NET-01) should be early because
it constrains `Command` and replay formats.

**Join points.** INT-M2 (map on seams), INT-M3 (combat + economy + FX + HUD
+ audio), INT-M4 (front end), INT-M5 (net + AI + Lua). Each is the only place
where lanes meet in one PR.

## Integration and merge protocol

- **Branches:** `wp/<ID>-<slug>` (`wp/SIM-04-weapons`); seam changes
  `seam/<slug>`. Never commit to `main` directly.
- **One package per PR.** Title `[SIM-04] Weapons and fire arcs`. Body: the
  done-when evidence, files touched outside *owns* (should be none besides
  `plugins.rs` and appended deps), new `TODO.md` check lines.
- **Seam changes** go in their own small PR touching only seam files, with
  the reason, the callers affected, and updated fakes. Within a milestone
  seams change additively only (new variants, new fields with defaults, new
  methods). A breaking change waits for the milestone boundary, bumps
  `SEAM_REV`, and the seam PR fixes every caller on main.
- **Merging:** rebase on `main`, CI green, done-when shown. The user merges.
  Squash merge, one commit per package.
- **Stale fakes:** when the real thing lands (CON-03, SIM-04), fakes stay
  and keep working; tests that used them keep running in CI.
- **Conflicts on hotspots** follow the ownership table; if two packages need
  the same non-hotspot file, the plan was wrong: split the file in a seam PR.

## Risks and open questions

Decisions for the user (**A** = assumed in this plan):

- **D1. `sta_core` crate.** (A) Move `Fixed`, `Vec3`, `EntityId`, `TICK_HZ`
  out of `sta_sim` so `sta_content` can use them while `sta_sim` depends on
  `sta_content`. Alternative: content DB inside `sta_sim`.
- **D2. One Bevy crate.** (A) Bevy code stays in `sta64`, split by lane
  directory, which keeps ARCHITECTURE's "only `sta64` changes" rule. Splitting
  into `sta_render`/`sta_ui`/`sta_audio` crates would isolate builds and
  dependencies further at the cost of that rule.
- **D3. Controllers on the host.** (A) AI and Lua run on one machine and
  send commands like players; replays never rerun them. MISSIONS.md's
  "deterministic `math.*`" then matters for single-player repeatability and
  tests, not for lockstep. Alternative: run them on every peer, which needs
  the Lua runtime and AI to be bit-deterministic.
- **D4. Bench edits in the command log** (TESTBENCH's open question): (A)
  yes, as `CommandKind::Edit`, also used by missions.
- **D5. Audio backend.** (A) `kira` used directly from an `sta64` plugin,
  avoiding a third-party Bevy plugin's release lag; alternative `bevy_audio`.
  Audio is also a new roadmap line: add it to M3 (SFX, voices) and M4 (music,
  UI)?
- **D6. Units and axes.** (A) The maps' own space (left-handed, +Y up, BZN
  units) is the sim's, as `sta_sim` states; Refit's parity scenes must agree
  (TESTBENCH open question).
- **D7. `TICK_HZ = 30`.** (A) unchanged; any change before M5 breaks all
  golden digests at once.
- **D8. Serialization.** (A) `serde` + `postcard` for snapshots, replays and
  the wire; `json5` for native content.
- **D9. TESTBENCH (PR #2)** must merge before TOOL-01; it is still a draft.
- **D10. Refit shader ports** (HLSL → WGSL, MIT): (A) each ported shader
  carries Refit's notice next to it, as `sta_surface/recipes/NOTICE.md` does.
- **D11. Concurrency.** How many agents at once, and who reviews seam PRs.
  The graph allows ~30 after round 0.
- **D12. `TODO.md` vs issues.** (A) Issues track packages; `TODO.md` stays the
  user's by-eye checklist.

Risks:

- **R0-3 is a big move on a moving `main`.** Freeze other `sta64` work while
  it runs (one or two days).
- **Content DB scope (CON-03).** FO has 9,355 ODFs and 893 keys; the package
  must stop at what M3 needs and list the rest from `docs/data/odf_keys.csv`.
- **Interpretation drift.** Movement and weapon timings are our reading of ODF
  keys (`units.rs` notes "our reading"); TOOL-08 parity runs are the check,
  and they need Refit's bench and an install, so they run locally.
- **Frame determinism** is unproven until RND-01 and TOOL-04 land; tier-2
  checks stay advisory until then.
