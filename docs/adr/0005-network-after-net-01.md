# ADR 0005: Network model after NET-01

- **Status:** accepted
- **Date:** 2026-11-12
- **Deciders:** Cedric Haase (requested the decisions; author and merger of #425); accepted by merge of #425
- **Relates to:** [NET.md](../NET.md) (protocol contract; open questions 3, 4 and 6), NET-01 (issue #401, PR #411), NET-02, NET-03, NET-04, NET-05, NET-06, SIM-31

This ADR records what was decided about the network model after NET.md was
written: whether lockstep stays, and what happens when peers diverge. It is
binding on the NET lane and on the packages named below. [NET.md](../NET.md)
remains the protocol contract. Where the two disagree about what a package
builds, the "Effect" line of each decision says which holds and from when.

## Context

Recorded at Cedric's request on 2026-11-12 as a follow-up to NET-01 (issue
#401, PR #411). The question that prompted it: the original games' lockstep
desyncs; should STA64 use something else? It answers NET.md's open questions
3 (what to do after a desync), 4 (rejoin after a restart) and 6 (desync
diagnosis).

## Decision

### Model

**N1. Deterministic lockstep stays.**
The original games desynced because their simulation was not deterministic
and because a divergence was neither found early nor repaired, not because
they used lockstep. STA64 already has the first two: fixed-point state,
ordered storage, one seeded RNG, a per-turn digest, and a handshake on build
and rules digest (ARCHITECTURE.md "Simulation rules"; NET.md sections 4.4, 5
and 7). The third is N2.

Considered and rejected:

| Alternative | Why not |
|---|---|
| Server-authoritative state replication (the server simulates, clients receive entity deltas) | Bandwidth grows with fleet size and needs delta compression and interest management; a replay stops being seed plus commands; it discards the determinism work already done. Its one real gain, fog of war enforced by the server, is not a goal (NET.md section 10). |
| Curve replication (the server sends keyframed curves per entity) | The same costs, and the client would have to consume sim state differently. |
| Rollback (predict locally, resimulate on late input) | It needs the same determinism, so it prevents no desync; it only removes input delay, at the price of resimulating ticks with thousands of entities. |

Reopen this only if server-enforced fog of war becomes a goal, or if
bit-identical simulation across the supported platforms cannot be held.

*Effect:* none on NET.md; it confirms section 1.

### Desync

**N2. A desync is repaired, not fatal.**
On a digest mismatch the peers still do steps 1 to 3 of NET.md section 7
(`Desync` message, replay, sidecar). Then, instead of stopping the match, the
game pauses, the authority (N3) sends a snapshot of its state, each diverged
peer loads it and checks the digest, and play resumes. This is neither the
draft's "stop for everyone" nor Armada II's "carry on at your own risk"
(open question 3).

*Effect:* NET-02 builds NET.md section 7 as written, including the stop.
NET-04 replaces the stop with the resync. Until NET-04 merges, a desync ends
the match.

**N3. The host's state wins. There is no majority vote.**
Without a referee (N5), the host is the authority and every peer whose digest
differs from the host's is resynced, even if the host is the one that
differs from everyone else. The host already holds the protocol's authority
and runs the AI and the mission (WORKPLAN decision D3), so its sim cannot be
replaced by another peer's; and a majority does not exist in a two-player
game.

Accepted consequence: resync lets a host hand out state, which plain lockstep
does not. So a resync is never silent: every player is told which seats were
resynced and at which turn, and it is in the sidecar.

**N4. Conditions on sending a snapshot.** NET-04 meets all of these.

1. **Cross-machine proof first.** SIM-10 proves that a reloaded snapshot
   continues with the same digests on one machine. Before any snapshot
   crosses the network, a committed snapshot fixture must continue to the
   same recorded digests on all four CI platforms (`full-ci`).
2. **Bounded and directed.** The 64 KiB frame bound stays. A snapshot goes as
   numbered parts in directed frames, as rules parts do (NET.md section 5),
   with a fixed limit on parts.
3. **Untrusted.** Decoding a received snapshot never panics and allocates a
   bounded amount, like every other message (NET.md section 10).
4. **Checked.** After loading, the peer's digest must equal the authority's
   for that turn. If it does not, the match ends as NET.md section 7
   describes today.
5. **Limited.** A seat is resynced at most a fixed number of times per match
   (NET-04 sets it); after that it is treated as a stalled seat (NET.md
   section 8).
6. **In the replay.** A resynced peer's replay marks the resync, so a
   verifier knows its digests before that turn may differ. The authority's
   replay is the replay of record. If this needs a replay format change, it
   is a seam PR.

**Rejoin after a restart uses the same path** (open question 4): the host
sends a snapshot and the turns after it, instead of the whole log from turn
0. Replaying the whole log (NET.md section 8) is what NET-02 and NET-03
build, and remains the fallback.

*Effect:* NET.md section 11's "snapshot resync" and "playing on after a
desync" leave the out-of-scope list when NET-04 merges. Multiplayer save and
load stay out of scope.

### Authority

**N5. An optional referee, as a peer. The relay stays a forwarder.**
An authoritative simulation outside the players is worth having, but not
inside the relay. A sim needs the same game content as the players, which
comes from a player-owned install and is agreed by rules digest (D10); the
relay has none and must not need any. NET.md section 9 is unchanged: the
relay reads frame headers only.

Instead a **referee** is a headless process with an install that joins a room
as a connection without a seat:

- It passes the same handshake as a joiner (build and rules digest).
- It receives every turn and sends none. It runs `sta_sim` and reports
  digests. It runs no AI and no mission; those stay on the host (D3).
- When a referee is in the room, its digest is the truth and snapshots come
  from it, replacing N3. A peer that differs from it is resynced, the host
  included.
- Resyncing the host is allowed only if the host's controllers (AI, mission
  Lua) can carry on across a reload of the sim. NET-05 proves that, or a host
  that differs from the referee ends the match with reports.
- It is never required. A room without one plays under N3.

Whoever runs a relay may run referees beside it; so may anyone else.

Not part of this decision: host migration, spectators, and protection against
revealing fog of war all stay out of scope (NET.md sections 10 and 11). The
seatless connection the referee introduces is the one observers would use
later.

*Effect:* a new message role and a protocol version bump, in NET-05.

### Diagnosis

**N6. Section digests name where a desync started.**
Open question 6 asked whether a desync should exchange a full state dump. It
does not. The sim instead exposes a digest per section of its state
(movement, combat, production and economy, RNG; SIM-31 fixes the list) beside
`Sim::digest`. Peers keep them for the same 64 turns as the digest, exchange
them only after a mismatch, and write the first differing sections into the
sidecar.

`Sim::digest` keeps its value and its pinned test. Section digests are
additional.

*Effect:* SIM-31 adds the sections (a seam addition to `sta_sim`); NET-06
carries them. Exchanging a state dump stays out of scope.

## Alternatives considered

- N1: server-authoritative state replication, curve replication and rollback
  were considered and rejected; the table under N1 gives the reasons.
- N2: stopping the match for everyone (the NET.md draft) and carrying on at
  your own risk (Armada II) are both set aside in favour of resync.
- N3: a majority vote is rejected; a majority does not exist in a two-player
  game, and the host's sim cannot be replaced by another peer's.
- N5: an authoritative simulation inside the relay is rejected; the relay has
  no game content and must not need any.
- N6: a full state dump on desync is not adopted; section digests instead.

## Consequences

### Packages
| Decision | Package |
|---|---|
| N2, N3, N4 | NET-04, after NET-02 and NET-03 |
| N5 | NET-05, after NET-04 |
| N6 | SIM-31, then NET-06 after NET-02 |

None of these gates INT-M5: the first two-process match is NET-02 and NET-03
as planned. Whether the first multiplayer release waits for NET-04 is for
the user to say.

### Not decided here
NET.md's open questions 1 (turn length and delay), 2 (how strict the build
match is), 5 (drop defaults), 7 (the host leaving), 8 (D10 and the content key
table) and 9 (elimination as a command) are untouched.
