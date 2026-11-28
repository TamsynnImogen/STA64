# ADR 0011: Group movement: formations, pace and one route per group

- **Status:** accepted
- **Date:** 2026-11-27
- **Deciders:** Cedric stated the guidelines in "Context" on 2026-11-27 and
  approves G1 to G14. G3 and G12 read existing game and mod data under
  [ADR 0004](0004-modding-v1.md) D1, D9 and D10 and add no mod-facing file or
  key; if they are read as modding decisions, Tammy approves them too.
  Accepted when the approvers approve the PR that merges this file; not
  binding before. The file was merged in #1278 without an approving review;
  Cedric accepted G1 to G14 on 2026-11-27, recorded by the PR that set this
  status. If G3 and G12 are read as modding decisions, Tammy's approval of
  that PR confirms them.
- **Relates to:** [ADR 0002](0002-deterministic-simulation.md);
  [ADR 0003](0003-clean-room-content-policy.md); [ADR 0004](0004-modding-v1.md)
  D1, D2, D5, D9, D10; WORKPLAN SIM-02, SIM-15, SIM-21, SIM-23, SIM-32,
  SIM-50, SIM-67, UI-02, UI-13 and the packages added with this ADR: CON-59,
  SIM-76 to SIM-80, UI-54, TOOL-26, TOOL-27

## Context

Guidelines (Cedric, 2026-11-27):

- Movement feels like the original base game being played.
- Improvements are taken where they are guaranteed, that is, where no player
  of the originals loses behaviour they rely on.
- A group staying together outranks each ship's own best route: single ships
  get destroyed, groups do the destroying.
- Formations fly as in each original base game, not as in one of them.

The defect, seen in play (Cedric): a STA64 group ordered past an obstacle
splits, its ships taking different sides; he has not seen Armada II do this.
Read from the code, not yet seen in play: nothing gives a STA64 group a common
speed, so a group of mixed ships strings out, where the originals hold the
leader to a pace (G7; static, not measured).

What STA64 does today:

- SIM-02 has three hard-coded formation kinds (line, column, wedge) at a fixed
  spacing of 60. A plain group move sends every ship to the same point. Slots
  are laid out about the destination when the order is given, so a formation
  exists on arrival only, not during the trip.
- SIM-23 plans per ship: a cost-to-go field from each ship's own goal cell, on
  a grid per body radius, re-derived every tick. Ships of one group have
  different goals, positions and radii and so choose routes independently.
- Nothing couples the ships: no leader, no following, no common speed.
- No formation file and none of the keys named below is read.

What the originals do. Nothing here is measured yet; G14 says how that is
handled.

- **Armada II** (read from the executable, static): every move order to more
  than one craft is a formation move internally. Nearby craft form a cluster
  with a leader and one slot each. Only the leader plans a route. Followers
  either hold their slot relative to the leader for the whole trip or fly the
  leader's route shifted by their slot offset, depending on a move mode whose
  default is `cfgDEFAULT_TO_FORMATION_MOVE`. The leader is held to the slowest
  member's speed. The search is an A\* over quadtree cells per craft, without
  the mover's size.
- **Fleet Operations 3.2.7 and Roots** (read from the executables, static;
  partly inferred): the same engine code for clusters, slots, steering and
  pace, with their own formation files and key values, and patches that
  change what happens when a leader is lost and, in Roots, the formation
  scale and the end of a move.
- **Armada I** (read from the executable, static, without symbols; partly
  inferred): a simpler system. A square grid replaces the formation kinds,
  the slowest craft leads, offsets are not rotated, and the same two move
  modes exist under `DEFAULT_TO_FORMATION_MOVE`.
- The formation definitions of the Armada II family are data
  (`AI/formations/*.frm`), as are `FORMATION_SPACING`,
  `FORMATION_LEADER_SLOWDOWN` and the default-move key (`RTS_CFG.h`). Evidence:
  read from data.

## Decision

### The group

**G1. A move order to more than one craft creates groups in the simulation.**
A group records its members, its leader, its formation, each member's slot
offset, the destination, the move mode (G4) and its pace speeds (G7). It is
simulation state: in snapshots and in the digest (ADR 0002). Craft are
clustered as the originals do: a craft joins the nearest cluster whose centre
is within 1200 and which has the same formation, otherwise it starts one.
Clusters of one order get destinations side by side, shifted by
`FORMATION_SPACING` × ceil(√n) across the mean bearing from the craft to the
target. A craft ordered alone gets no group. Evidence: read from the
executable, static; 1200 is a constant of the executable, not a config key.

**G2. The leader is a real craft.**
There is no virtual anchor point. Armada II family: the leader is the craft in
the slot with the lowest rank. Armada I: the slowest member of the cluster
(inferred; interim until measured).

**G3. Formations come from the content, per game.**

- *Armada II family.* The install's or mod's `AI/formations/*.frm` files are
  read at run time through the content layer (never committed, ADR 0003),
  together with the formations the game builds in code from
  `FORMATION_SPACING`. Slots are taken in file order; each goes to the nearest
  unassigned craft whose `shipType` letter comes first in the slot's priority
  list. Offsets are scaled by the largest member radius over the file's
  `m_shipRadius` (the radius is SIM-32's). The layout is centred on the
  cluster's centroid and faces the mean of the members' headings. Evidence:
  read from data (format, keys) and from the executable, static (assignment,
  scale, facing).
- *Roots* takes its scale from another per-class value that is not
  identified. Interim: the Armada II rule.
- *Armada I.* A square grid of ceil(√n) cells a side, `FORMATION_SPACING`
  apart, aligned to the map axes; each cell goes to the nearest remaining
  craft. There are no formation kinds. Evidence: read from the executable,
  static.
- SIM-02's three hard-coded kinds remain only as the fallback for content
  that has no formation data (`fake::db()`, a mod that ships none).
- A formation format of STA64's own, for mods, is not decided here and stays
  open.

**G4. Two move modes, as the originals offer the player.**

- *Formation move:* followers hold their slot relative to the leader for the
  whole trip.
- *Group move:* followers fly the leader's route shifted by their slot
  offset, each at its own speed.

The default is the content's `cfgDEFAULT_TO_FORMATION_MOVE`
(`DEFAULT_TO_FORMATION_MOVE` in Armada I): 1 in stock Armada II and Roots, 0
in Armada I and Fleet Operations 3.2.7 (read from data). The originals'
modifier key gives the other mode. `Order::Move` carries the mode (a change to
seam S2).

Who chooses, and how:

- *The player, per order:* the modifier key.
- *A mod, for its default:* that one key in its own `RTS_CFG.h`, as in the
  originals. Nothing else is needed: no code and no further file (ADR 0004
  D9: rules come from the legacy files; v1 has no other rules file). If
  ADR 0004 later gives mods a rules format of STA64's own, the default move
  mode is one of its values.

### The route

**G5. One route per group: only the leader plans.**
The leader plans with SIM-23's planner as it is, on the grid of the largest
body radius in the group, so the route is one every member can fly. That grid
choice is STA64's; the originals plan without the mover's size.

- *Formation move.* A follower steers at the leader's aim point plus its slot
  offset rotated by the formation heading (G8), pushed off solids by SIM-21.
  When the leader has no move order it steers at the leader's position plus
  the offset.
- *Group move.* When the order is given the leader's route is taken as a list
  of points. A follower's route is those points, each shifted by its offset
  rotated into that leg's direction; the follower's own planning only
  connects consecutive points.

In both modes every member passes an obstacle on the leader's side. This is
what removes the split. The leader's own route stays as SIM-23 has it:
re-derived each tick from the cost field and not stored, so a station built
across the way is planned round at once.

**G6. A follower that cannot hold its slot plans for itself, then returns.**
Armada II family: a follower more than 1000 from its slot point with under 100
of progress in 8 s, or more than 500 from it with none in 16 s, plans its own
route to the slot point, and plans again when that point has moved more than
1500 since. Times are counted in ticks and kept in the group record. Evidence:
read from the executable, static. Armada I shows no such fallback (weak
evidence: not found); STA64 applies the same rule there, marked as STA64's
own.

### Pace

**G7. The slowest member sets the pace.**

- *Armada II family.* The group keeps the minimum cruise speed over its
  members, updated when one joins or leaves (a warp minimum likewise, once
  warp is simulated). The leader is capped at that minimum ×
  `FORMATION_LEADER_SLOWDOWN` (1.0 in stock Armada II, Fleet Operations 3.2.7
  and Roots) in both move modes. In a formation move a follower behind its
  slot is not capped and catches up at its own speed; one level with or ahead
  of its slot flies at the leader's speed × (0.8·f + 0.2), where f falls from
  1 on the slot to 0 a full leader lead ahead of it, so never below a fifth.
  In a group move followers are not paced (inferred): only the leader is held
  to the slowest member's speed, so faster followers arrive first. A player
  or a mod that wants the group to arrive together uses the formation move
  (G4).
- *Armada I.* There is no group minimum, because the slowest craft leads
  (G2). While a follower is in a formation move the leader's speed is
  multiplied by `FORMATION_LEADER_SLOWDOWN` (0.9). A follower nearer its
  target than the leader is to its aim point is capped at the leader's speed;
  otherwise it is not capped.

Evidence: read from the executable, static; key values read from data.

### Turning, leader loss, the end of a move

**G8. Turning and arrival.**
Armada II family: the formation heading is the leader's facing, taken each
tick, and the slots rotate rigidly with it, so outer slots swing wide in a
turn. Each slot's final point is the leader's last waypoint plus its rotated
offset. Armada I: offsets are not rotated (`FORMATIONS_RELATIVE_TO_SHIP` = 0,
its stock value). What the value 1 does is not established; content that sets
it gets a logged warning and the behaviour of 0 until it is measured.

**G9. Leader loss, per profile.**

- *Armada II:* the lowest-rank surviving member becomes leader, the others
  follow it, it takes over the stored destination, and offsets are not
  recomputed. Evidence: read from the executable, static.
- *Fleet Operations 3.2.7 and Roots:* the static reading is that a new leader
  is chosen but not given the order, so the group probably stops. This
  contradicts the Roots readme (followers continue), which SIM-50 follows
  today.
- *Armada I:* followers probably stop (inferred).

Interim for the last three: as Armada II, the group continues, marked interim.
The behaviour measured per profile replaces it (SIM-50's first step is that
measurement). Keeping "continues" for a profile whose original is measured to
stop would be a deliberate deviation and needs an amendment of this ADR (open
question 2).

**G10. End of a move.**
Armada II: the group is dissolved once 75% of its members are within 700 of
the destination (constants of the executable, not config keys). Roots appears
never to dissolve it (inferred); interim: the Armada II rule. A member given
another order leaves its group, and the pace minimum is recomputed.

### Limits

**G11. What STA64 keeps as its own, and what it does not copy.**
Kept: the exact cost field on a uniform grid, the body size in planning, the
nearest reachable cell for a walled-in goal, deterministic tie-breaks. Not
copied: the originals' search and its quadtree graph, its tie order, its
leaving queued nodes unimproved, its path walk without a length limit. Not
implemented until measured: behaviour read only in part (a follower that does
not steer for a slot behind it).

**G12. The rules are data.**
`FORMATION_SPACING`, `FORMATION_LEADER_SLOWDOWN`, the default-move key and
`FORMATIONS_RELATIVE_TO_SHIP` are read from `RTS_CFG.h` into one typed rules
struct in `sta_content`, beside `PathingRules` (ADR 0004 D9). The constants of
the executables named above are that struct's per-profile defaults, and the
differences the Fleet Operations and Roots engines patch in are profile rules
(D2, D5). Rules and formation data are in the content digest (D10). The
default move mode (G4) is one of these rules: a mod sets it, and peers must
agree on it. No new mod key is introduced.

**G13. Scope.**
Player, computer-player and mission orders use the same group record. Not
decided here: the computer player's fleet stages, warp pacing beyond "the same
rule with the warp minimum", a formation format of STA64's own, and formations
in combat (the Armada II manual says a formation is kept while attacking; not
researched).

**G14. Evidence.**
Every rule above is read statically or inferred; none is measured. Each
implementing package states its rule's grade in code comments. TOOL-26 and
TOOL-27 measure the rules per game. A measured value replaces a static one of
the same shape without an amendment; a rule of a different shape is an
amendment of this ADR.

Done when: the packages listed under "Relates to" meet their WORKPLAN checks.
The first visible check is SIM-77's: a group ordered past a planet passes it
on one side and arrives in formation.

## Alternatives considered

- **Port the originals' A\* and quadtree.** Brings its defects with it and a
  tie order that depends on heap layout (against ADR 0002), and adds no feel
  that G5 does not give. Rejected.
- **A virtual anchor point instead of a leader craft.** Smoother, but it is
  not how any of the base games turns, sets its pace or loses a leader.
  Rejected.
- **A stored, committed path per ship.** More snapshot state for no gain:
  with only the leader planning, the group cannot split. Rejected.
- **Pace followers in a group move too.** Keeps groups tighter in Fleet
  Operations' default mode, but removes the choice the originals give the
  player between arriving together and arriving fast. Not taken; decided by
  Cedric on 2026-11-27 (open question 1).
- **Armada II's rules for every profile.** Rejected by Cedric: each base game
  flies as itself.
- **Keep per-ship planning and add hysteresis.** Stops ships changing sides
  mid-route, but gives neither formations in transit nor a common pace.
  Rejected.

## Consequences

**Gained**

- A group passes an obstacle on one side, flies in its formation during the
  trip where the base game does, and moves at a common pace where the base
  game does.
- Formations, spacing, default mode and leader slowdown follow each game's
  and mod's own data.
- Mixed groups are not routed through gaps their largest member cannot fly.

**Accepted**

- Inside a group a ship no longer takes its own best route. This is intended.
- One snapshot format bump and one protocol bump (group record, move mode).
- SIM-02's formation part and SIM-50 are reworked; `movement.rs`, the one
  `nav` call site and the speed cap in `kinematics.rs` are touched by several
  packages, which therefore land in the order their rows state.
- Fleet Operations 3.2.7's default stays the group move without follower
  pace, as in that game.
- Until TOOL-26 and TOOL-27 have run, the rules rest on static reading, and
  several per-profile differences are interim (G3 Roots scale, G9, G10).

## Open questions

1. Should followers be paced in a group move, or does the originals' choice
   between the two modes stand (G7)? **Decided 2026-11-27 (Cedric):** both
   modes stay as in the originals; the default is the content's key, so a mod
   selects it (G4).
2. If the measurement shows that the group stops when its leader is lost in
   Fleet Operations 3.2.7, Roots or Armada I, does STA64 follow that or keep
   "continues" as a stated improvement (G9)?
3. Does Armada I get the stuck-follower fallback (G6)?
4. Should the group's route allow for the formation's width, not only for its
   largest craft (G5)?

Raised by the first implementing package (CON-59, #1306) for the deciders;
the decisions above are unchanged:

5. The default of `FORMATION_SPACING` for content whose `RTS_CFG.h` sets
   none is carried in CON-59's code as an interim number that this ADR does
   not state. It is needed to scale formations whose header sets no spacing.
   It belongs under G12, the decision that makes the spacing a rule read from
   data with per-profile defaults, beside the key values the ADR already
   names (G7). Recording it there is for the deciders.
6. A formation file with one malformed row: is the whole file rejected, or
   only that row skipped? STA64 skips the row and warns with file and line
   (CON-59's row), so a malformed mod file can give STA64 a formation with
   fewer slots where a game that rejects the file has none, and a mod could
   rely on either. This is a fidelity question for G3; what the originals do
   is read statically in #1306 and not measured. The option #1306 names:
   reject the whole file under the classic profiles and keep skipping rows
   for STA64's own profile.
