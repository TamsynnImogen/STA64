# Engine measurements: build payment, point defence, initial reload, attack approach and autonomy

Research record, 2026-11-12, Cedric's session. These measurements were taken on
the original games running on Refit's test bench (`armada2-refit`, `testbench/`),
in reflink clones of the installs under `/home/cedric/Games/Heroic/`. The installs
themselves were never written. Where a measurement left a mechanism unclear, the
original executables were decompiled with Ghidra, working on copies. The
repository holds no game data, frames or decompiled listings. ODF values are quoted
only as short facts, and every rule below is given as short pseudo-code.

The WORKPLAN rows that implement these rules are SIM-33 to SIM-36, and TOOL-16
covers the parity-tool additions.

**Evidence grades.**
- *Measured*: observed on the bench, with the raw marks kept.
- *Decompiled*: read from the executable's code.
- *Not measured*: neither measured nor decompiled. It is a guess and must not be
  built as fact.

**Lineages.**
- **Stock A2 1.1**: `Armada2.exe`, with Refit's `a2mod stock` applied to the
  clone.
- **Roots + Classic**: Fleet Operations: Roots `Data/ArmadaL.exe` with
  `FleetOpsHook.dll`, running the STA2 Classic mod.
- **Roots + FO 4.0**: the same executable, running the Fleet Ops 4.0 mod.
- **Roots, key absent**: the Roots engine's default when an ODF does not set a key.

## Time base (read this first)

Every time in this document is **simulation seconds**, taken from the engine's
`Get_Time` and written on each mark. On stock 1.1 at its default speed, sim time
equals wall time to three decimals.

On Roots the bench clone keeps the user's `Settings.xml`, which sets
`gameSpeed=5`. With that setting the simulation ran **1.59 to 1.86 times wall
clock**, and the ratio varied between runs. A rate taken from wall time on Roots
would be off by that factor. Parity comparisons must use sim time.

## Running Roots with a mod on the bench

Roots selects its mod from the command line: `-mod <folder under Data\Mods>`
(`/mod` also works). Settings.xml has no mod key.

```
./a2test --game fo-roots session start --res 1280x960 --layers display,asiloader --no-launch
./a2test drive --session ID launch -nointro -mod 'STA2 Classic Mod' fulltech.bzn
```

- **Quoting.** Pass the mod name as one shell argument with no extra quotes; the
  bench quotes it itself. Literal quotes get mangled.
- **FO 4.0.** It runs on the same `ArmadaL.exe` (identical file) with
  `-mod 'Fleet Ops 4.0'`.
- **Which map to use.**
  - Under Classic, `a2_borg01.bzn` runs its full mission, with hundreds of
    objects.
  - `mp02eye.bzn` is an empty stage, but its tech tree leaves special weapons
    unresearched on Roots.
  - `fulltech.bzn` (stock and Classic) is empty and has every technology
    available. Use it for anything involving specials. The stock bench's `Tech=`
    scene key does not run on Roots, because Roots answers the tech tree from its
    own tree.
- **FO 4.0 needs Instant Action.** On a bare map with FO 4.0 nothing is
  buildable: the build menu is empty. The M3 measurement therefore used Instant
  Action (Federation against the Klingon AI), with the yard spawned by the scene.
- **Scene harness on Roots.** Refit's scene harness (`Scene.asi`) runs on Roots
  with `--layers display,asiloader`.

## Bench additions used here

These additions to Refit's scene harness are on a local Refit branch, not yet
proposed upstream. They keep the existing `Scene.marks` line format: a tick, a
kind, then `key=value` fields.

| Addition | What it writes or does |
|---|---|
| `[Marks] SimTime=1` | every mark gets `t=<sim seconds>`, with millisecond resolution |
| `[Marks] Ordnance=1`, `OrdnanceSkip=<prefixes>` | `ord.new` per ordnance (`id obj weapon target at`); `ord.gone` when it leaves the list (`at life tgt_at d`, where `d` is its distance from the target at the end) |
| `[Marks] State=<ms>` | a `state` line per scene craft every N ms: `pos hull sh sp` (special energy), `q` (queue), `ma swa` (movement and special-weapon autonomy, 0 low, 1 medium, 2 high) |
| `bank [team\|object]` | a team's resources read through `Team::GetResource`; also written as a `bank` mark |
| `autonomy <name> low\|med\|high` | sets a craft's movement autonomy through `Craft::SetMovementAutonomy` and reads it back |

Known limit: `q` (from `Producer::BuildQueueSize`) read 0 throughout, on stock
too, so build queues were followed through the bank instead.

---

## M3: when queued builds are paid

**Method.**
- A shipyard owned by the player stands alone on an empty map with ample
  resources.
- The yard is selected and three ships are queued at once: by hotkey on stock and
  Classic, by clicking the build button on FO 4.0.
- `bank` is polled about once a second until all three have started.
- The ships were the Federation destroyer `fdestroy` (dilithium 140, crew 80,
  buildTime 12) on stock and Classic, and `fed_saberZ` (dilithium 286, latinum 85,
  supplies 9, buildtime 69) on FO 4.0.

| Lineage | Charge times | Repetitions |
|---|---|---|
| Stock A2 1.1 | 1st at the click (queue was empty); then 12.1 s and 12.1 s apart | 2 launches, 6 of 6 charges |
| Roots + Classic | 1st at the click; then 12.5 / 12.7 s and 11.5 / 11.8 s apart | 2 launches, 6 of 6 |
| Roots + FO 4.0 | 57.2, 125.6, 195.8 s: about 69 to 70 s apart | 1 launch, 3 of 3 |

The bank value matched the HUD counters on all three engines. Grade: **measured**.

**Rule (all measured lineages).**
- A producer's queue is FIFO.
- Nothing is charged when an item is queued.
- The item's full ODF cost leaves the bank when it becomes the head and starts
  building.
- The next item starts, and is charged, when the previous one finishes.

```
on build order: push item (no charge)
when head starts: charge cost(head) from the owner's bank
```

Refit's "pay on queue" (QOLRules) is a Refit-only rule and not an original
lineage's.

---

## M1: point defence

**Method.**
- **Units.**
  - The player's Nebula class (`fspecial`) carries the point-defence phaser
    `gpdlaser` (`PointDefenseLaser`, `toggle = 1`, `specialEnergyCost = 40`,
    range 150), toggled with its hotkey.
  - The Nebula is healed every tick so damaged systems cannot disable the PD, and
    its engines are off.
  - Two enemy Klingon photon-torpedo turrets (`kturret2`, shotDelay 2.0,
    three hardpoints) fire at it from 600 units.
- **Sequence:**
  1. Fire with PD off.
  2. PD on, under fire.
  3. Turrets off, PD on idle.
  4. PD on under fire again.
- **Counting.**
  - A torpedo whose `ord.gone` is more than 60 units from the Nebula counts as
    intercepted.
  - Intercepts end 172 to 184 units out; hits end 20 to 60.
- **Conditions.**
  - Stock: `shotDelay = 3.5`.
  - Roots + Classic: `shotDelay = 0.1`.
  - A Roots + Classic clone with `shotDelay` set back to 3.5. The edited ODF went
    into the clone's packed `ODF.fpq` only; the install's file was unchanged.

| | Stock (3.5) | Roots + Classic (0.1) | Roots + Classic, 3.5 (2 launches) |
|---|---|---|---|
| Time with PD on | 68.5 s | 117 s | 96 s / 99 s |
| Torpedoes intercepted while on | 53/55 (96%) | 81/92 (88%) | 26/76 (34%) / 27/77 (35%) |
| PD shot interval, median (min) | 1.0 s (0.1) | 0.9 s (0.3) | 3.75 s / 3.33 s; normal windows show 4.1 s, some 2.0 to 2.3 s |
| Intercepted with PD off | 0/111 | 0/224 | 0/168 / 0/173 |
| Energy while on, net | −29.7 /s | −29.5 /s | −29.0 to −30.0 /s |

**What the numbers mean.**
- **Stock** ignores `shotDelay` for PD. The interception rate is limited only by
  how fast torpedoes arrive.
- **Roots** obeys `shotDelay`, but through a timer that can go into debt. In both
  3.5 launches, the one fire window that followed PD activations with nothing to
  shoot fired pairs 0.1 to 0.2 s apart every 2 s. That is the debt being paid off.
- **Energy** (measured, both lineages):
  - Energy is charged at `specialEnergyCost` per second, continuously, also with
    nothing to shoot. Recharge of about +10/s keeps running alongside.
  - There is no charge per intercept and no lump sum on activation.
  - At 0 energy the PD switches itself off.

**Mechanism** (decompiled, consistent with every measured window).

```
stock PD tick (no rate limit):
    if active: energy -= cost * dt
        shoot every eligible incoming ordnance in range (ignores shotDelay)
        if energy <= 0: active = false

Roots PD tick (ArmadaL's patched PointDefenseLaser):
    if active: energy -= cost * dt; timer -= dt          // never clamped at 0
        for each eligible ordnance: if timer > 0: stop
                                    fire; timer += shotDelay * (1 + U[0, 0.05])
        if energy <= 0: active = false
    activation needs energy >= cost
```

- The timer is one value per weapon instance. It is not per hardpoint and not per
  target.
- Toggling does not reset it.
- `U[…]` is the engine's synced random generator.
- **FO's `ordnancedefenseweapon` class** is a separate implementation, graded
  *decompiled, not measured*:
  - Same energy and switch-off logic.
  - On each shot the timer is set to the raw `shotDelay`, so there is no debt and
    no jitter.
  - It keeps a list of ordnance already engaged, cleared when it is switched off.

**Rule for STA64, per lineage.**
- All lineages:
  - PD drains `specialEnergyCost` per second while toggled on, and switches off
    at 0.
  - Roots: activation needs energy ≥ `specialEnergyCost` (decompiled).
- Stock A2 1.1: no rate limit.
- Roots (Classic and the key-absent default): the debt timer above.
- FO `ordnancedefenseweapon`: a reset-to-`shotDelay` timer and an engaged list
  (decompiled only).

---

## M2: initial reload of a special weapon, and `initialShotDelayModifier`

**Method.**
- Klingon Negh'Var (`kbattle`) ships with the ion cannon `gplasmac` (launcher,
  shotDelay 20, 600 special energy, needs a target) are placed 500 units from an
  enemy target.
- Fresh ships are spawned, then given the fire order repeatedly until the first
  shot. The time from spawn to the first `plasmac` ordnance is measured.
- Ships older than 20 s are also ordered, as a control.
- The order is the hotkey on stock and hotkey plus target click on Roots, every
  2.5 s on stock and every 4 s on Roots. About 4.4 s of the Roots times is input
  latency.

| Lineage | Spawn → first shot (fresh ships) |
|---|---|
| Stock A2 1.1 | 4.6, 8.9, 10.8, 12.9, 15.0, 19.1 s |
| Roots + Classic (`initialShotDelayModifier = 0.0`) | 4.4 to 4.7 s for 5 of 6 (the first order fires; this is latency only); the sixth 9.4 s |
| Roots, key removed (clone) | 4.7, 4.9, 13.1, 13.1, 21.4, 21.6 s |

- Ships older than 20 s fired within input latency on every lineage.
- A fire order given while the weapon is not ready is **dropped, not queued**: the
  ship does not fire later on its own.
- Grade: **measured**, with the mechanism **decompiled**:

```
at weapon creation (stock 1.1):     timer = U[0, 1) * shotDelay
Roots, m = initialShotDelayModifier (ODF; absent → -1, children inherit):
    m == 0: timer = 0
    m >  0: timer = shotDelay * m
    m <  0: timer = U[0, -m) * shotDelay
each shot (all lineages): reload = shotDelay * (1 + U[0, 0.05])
```

(The team shot-delay modifier and the crew penalty also apply; FO's
`ignoreShotDelayEffects` switches the jitter and the team modifier off.)

So **0.0 is not stock**. Classic's 0.0 makes specials ready at once. The Roots
default, key absent or −1, reproduces stock's random initial reload.

**Rule for STA64.** Set the reload timer once, at weapon creation:
- by default, U[0, 1) × shotDelay;
- with the key present, as in the pseudo-code above.

Each shot reloads with +0 to 5 % jitter. A special fire order arriving while the
weapon is reloading is refused, not queued.

---

## M4: attack approach speed, targets of opportunity, assist by autonomy, construction fire

**Method.**
- Federation destroyers (`fdestroy`: impulseSpeed 140, combatSpeed 70,
  warpSpeed 400, weapon range 400).
- `state` marks every 250 ms give positions, and speed is taken from those.
- Attack orders are the player's right-click on the target. Refit's scripted
  `attack` (through the ScriptInterface) did not move ships on Roots, so it was not
  used.

**(a) Attack on a target 3800 units away** (two launches per lineage):

| | Stock A2 1.1 | Roots + Classic |
|---|---|---|
| Plain move (goto), same distance | 140 u/s throughout | 140 u/s throughout |
| Attack, far target | ~2400 units at 400 u/s (warp), then 70 u/s (combatSpeed) for the last ~1050; stops ~350 from the target and fires | 140 u/s (impulseSpeed) all the way, no warp, no combat speed; closes right onto the target |
| Enemy 250 off the path | not fired at | fired at en route (11 shots in each launch), still at 140 u/s |

**(b) Assist.**
- **Setup.**
  - The player's idle destroyer sits 600 units from an ally that an enemy attacks
    from 300 units away; the enemy is 900 from the idle ship.
  - There are three groups, with the idle ship at low, medium and high movement
    autonomy, set with the `autonomy` command.
  - Spawned ships start at medium on both engines.

| Autonomy | Stock A2 1.1 (2 launches) | Roots + Classic (2 launches) |
|---|---|---|
| Low | moves in at 70 u/s, engages | stays |
| Medium | moves in at 70 u/s, engages | stays |
| High | moves in at 70 u/s, engages | moves in at 140 u/s, engages |

**(c) Construction ship** (Klingon `kconst`, phaser shotDelay 8, with an enemy in
range of the build site):
- **Stock** stopped firing while it flew to the site and built; there was a gap of
  about 35 s, after which it fired again.
- **Roots** fired every ~8.3 s throughout the build.

Grade: **measured**. For (b) the autonomy was set through the engine setter, not
through the HUD (see "Not measured").

**Rules for STA64.**

Stock A2 1.1:
- **Far attack.** Warp toward the target, then drop to `combatSpeed` for the
  approach. Stop at weapon range.
- **En route.** No targets of opportunity are engaged.
- **Assist.** An idle ship near an attacked ally moves in at `combatSpeed`
  whatever its movement autonomy (as measured).
- **Construction.** A construction ship holds fire while building.

Roots:
- **Far attack.** Use `impulseSpeed` while the target is beyond weapon range.
- **En route.** Engage targets of opportunity.
- **Assist.** Only high movement autonomy moves in to assist, at `impulseSpeed`.
- **Construction.** A construction ship fires while building.

Default autonomies come from `RTS_CFG.h`.
- Decompiled defaults: movement 1, special weapon 0.
- Roots and FO 4.0 set movement 1, special 2.
- Classic Enhanced sets 1 and 1.

---

## Not measured

- **Stock low-autonomy assist through the HUD.** The assist was measured with the
  engine setter (`Craft::SetMovementAutonomy`, read back through the getter). The
  player's autonomy keys did not change the value on the bench. Stock assisting on
  low is unexpected. It must be confirmed with the HUD's autonomy buttons, and the
  player-ship assist code found.
- **The impulse-versus-combat speed decision** in the movement code was not
  decompiled. Only its effect was measured.
- **FO 4.0 was measured for M3 only.** M1, M2 and M4 on FO 4.0 are not measured,
  and FO's `ordnancedefenseweapon` is decompiled only.
- **M2 timing precision** is bounded by input latency: about 0.35 s on stock and
  1.3 s on Roots. A key-event mark on the bench would tighten it.
- **Roots, `initialShotDelayModifier` > 0** (e.g. 1.0) was not run. It is
  decompiled only.
