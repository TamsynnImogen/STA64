# Network protocol (lockstep)

Status: **draft for review** (NET-01, issue #401). Once accepted, this document is
the contract for NET-02 (lockstep router), NET-03 (transport and relay),
UI-11 (lobby) and the network half of CON-35 (rules digest). The executable
half is [`crates/sta_net/src/proto.rs`](../crates/sta_net/src/proto.rs): the
message types, their bounds, the framing, and the tests. Where this document
and `proto.rs` disagree, that is a bug in one of them.

Decisions this design relies on:
[MODDING_DECISIONS.md](MODDING_DECISIONS.md) D10 (peers agree on one digest of
resolved rules; pack list informational; refusal names the differing key;
replays record lineage and packs), and WORKPLAN's assumption D3 (AI and Lua
run on one machine and send commands like players).

## 1. Model

STA64 multiplayer is **deterministic lockstep**: every peer runs the whole
`sta_sim` simulation, and the only thing that crosses the network is player
input (`sta_sim::command::Command`), grouped into turns. The sim is
fixed-point and bit-identical across platforms (ARCHITECTURE.md, "Simulation
rules"), so equal input gives equal state; a per-turn state digest checks
that it did.

- **Turns.** Time is divided into turns of `ticks_per_turn` sim ticks. Every
  peer sends exactly one `TurnCommands` record per turn, empty or not, in
  turn order with no gaps. A peer runs turn *T* only when it holds every
  seated peer's record for *T*; otherwise its `CommandRouter::take_ready`
  returns `None` and the sim stalls. The commands of turn *T* apply on its
  first tick, `T × ticks_per_turn`, sorted by `(player, seq)` as S2 requires.
- **Input delay.** Input made during local turn *T* is sent as turn
  *T + delay_turns*, so it has `delay_turns` turns to reach everyone before
  it is needed. Everyone applies it on the same tick.
- **Topology: a star through a relay.** Every peer holds one connection to
  a relay room (NET-03); the relay forwards each sender's frames to the
  room's other members in the order it received them. No peer needs to be
  reachable, and there is no peer-to-peer mesh. This is the shape Refit's
  online layer arrived at for Armada II (Refit `online/README.md`: star
  through the host, then a rendezvous server and relay).
- **The host** is one seated player, the room's creator. It owns the lobby,
  sends `Start`, puts scheduled changes (pause, speed, delay, drops) into its
  own turn records, keeps the full turn log for rejoins, and runs every
  computer player and the mission (`Controller`s, assumption D3), whose
  commands travel in the host's own turn record. The host is a role, not a
  server: it runs the same sim as everyone and its turns wait like anyone's.

## 2. What the original games do

Checked in the installs on this machine (`/home/cedric/Games/Heroic/`:
Armada, Armada II stock, Fleet Operations, FO Roots) and in Refit's
DirectPlay trace, rather than guessed. Every item below either shaped a
decision or is listed out of scope.

| Behaviour | Evidence | Here |
|---|---|---|
| At most 8 players | Armada II's `Host` call asks DirectPlay for 8 (Refit trace); `PlayerId::MAX_PLAYERS = 8` | `MAX_SEATS = 8`; a map's own limit is its BZN `MPDStLoc` start count (`sta_bzn`), shown by the lobby |
| Input sent about every 233 ms | Refit trace: each player sends one ~11-byte message to each player every ~233 ms in a 2-player match (7 ticks at our 30 Hz) | Default turn of 3 ticks (100 ms); see 3.1 |
| A "High Latency" box, ticked by a player in the comms menu, and "X changed the network latency" | A1/A2 `Dynamic_Localized_Strings.h` (`GUI_POP_COMM_LATENCY`, `MP_GAME_LATENCY_CHANGED`); the button in `gui_glob16x12.cfg` / FO `gui_interface_misc.cfg` (`commLatencyButton*`) | Two delay presets, normal and high; `Action::SetDelay` |
| Game speed changes in multiplayer ("X changed the game speed") | A2 `MP_GAME_SPEED_CHANGED` | `Action::SetSpeed`, wall-clock only |
| Any player may pause ("X paused the game") | A2 `MP_GAME_PAUSED` / `UNPAUSED`, "GAME PAUSED" pop-up | `Request(Pause)`, `Action::Pause`, `Message::Resume` |
| A stalled player: "Waiting for N seconds", each player votes Eliminate or AI Takeover; reasons quit, disconnected, in the shell, unresponsive | A2 `GUI_POP_DROP_*`, `DEPARTURE_REASON_*`, `DEPARTURE_ACTION_AITAKEOVER`; A1 had a single DROP button | `DropVote`, `Action::Drop { then: Eliminate \| AiTakeover }`, `LeaveReason` |
| AI takeover uses a named AI | FO readme: "Multiplayer AI takeover now uses the Merciless AI" | `DropChoice::AiTakeover(name)` |
| Chat scopes All, Allies, Enemies, Private | A2 `GUI_POP_COMM_CHAT_*` | `ChatScope`, resolved to a recipient set |
| Out of sync is detected and the game is called ruined, not repaired | A2 `MULTIPLAYER_SYNC_ERROR`: "This network game is out of sync. You are advised to exit the game." | Desync halts the match and writes reports (section 7) |
| Mismatched versions or mods cannot join (amber in GameSpy); the check is a CRC over files, and FO once forgot `rts_cfg.h` in it | A2 `NetHelp.txt`, 1.1 patch readme; FO readme 3.2.x | D10 rules digest, named key (section 5) |
| Game closed to new joiners | FO readme ("Game closed" checkbox) | `LobbyState::closed`, `Refusal::Closed` |
| Passworded games; a ping filter on joining | A2 `NetHelp.txt`, Refit trace ("a password field when one is set"); A1 `nethelp.txt` ("stop lights") | Room password is a NET-03 room option (section 9); ping filter out of scope |
| Multiplayer save (Ctrl+S, at most every 10 minutes), map download, observers (FO), voice | A2 1.1 readme and strings; FO readme | Out of scope for v1 (section 11) |

None of the games expresses turn length, latency, speed or timeouts as
numbers in data: `RTS_CFG.h` has no network keys, `Network_CFG.h` (A2 and
STA2 Classic) holds only the message-of-the-day URL, and the gui cfgs only
place the High Latency button. So these values are STA64 settings with the
defaults below. The ones the sim's pacing depends on travel in `Start` and in
the host's turn records, so every peer and the replay agree on them; none is
hard-coded in the router.

## 3. Turns, delay and speed

### 3.1 Settings

| Setting | Default | Range | Where it lives |
|---|---|---|---|
| `ticks_per_turn` | 3 (100 ms at `TICK_HZ` 30) | 1..=15 | host setting; `Start.turn`; fixed for a game |
| `delay_turns`, normal | 2 | 1..=30 | host setting; `Start.turn`; `Action::SetDelay` |
| `delay_turns`, High Latency | 5 | 1..=30 | host setting; the comms-menu box picks it |
| `speed_percent` | 100 | 25..=400 | `Start.turn`; `Action::SetSpeed` |
| ping interval | 1 s | | host setting |
| delay adaptation window | 10 s | | host setting |
| stall before the drop dialog | 3 s | | client setting |
| connection lost after silence | 15 s (Refit's figure) | | transport setting (NET-03) |
| digests kept for comparison | 64 turns | | router setting (NET-02) |

The speed range is not from the games (their speeds are not in data); it only
bounds what a host may set. With the defaults a command applies 200 to
300 ms after the click (two turns of delay plus up to one turn of waiting for
the turn to close), about what Armada II's ~233 ms send interval felt like,
and 500 to 600 ms with High Latency. A 100 ms turn is the compromise between
that delay and the message rate: 10 frames per second per peer, so an
8-player room carries 80 frames in and 560 out of the relay per second, each
a few dozen bytes. One tick per turn would triple the frame rate for a 66 ms
gain; it remains available as a setting.

### 3.2 How delay adapts

- The delay is each sender's lead, not shared state. A peer sending turn
  *T + d* for local turn *T* is correct for any *d*, as long as its stream
  stays gap-free and sends each turn once. So a delay change never needs a
  handshake to be safe: on a larger delay the sender fills the gap with empty
  records; on a smaller one it sends nothing new until *T + d* passes the
  last turn it sent.
- The first record any peer sends is for turn `delay_turns` (from `Start`);
  turns before it are empty by definition on every peer, so play starts
  without a wait and a receiver knows where each stream begins.
- The host still sets one value for everyone (`Action::SetDelay` in its turn
  record), as Armada did, so play feels the same on every seat and the change
  is announced ("X changed the network latency").
- Automatic mode (the default): the host pings every peer once a second
  (`Ping`/`Pong`, through the relay) and sets
  `delay_turns = clamp(ceil((max RTT + one turn) / turn length), 1, 30)` over
  the last 10 seconds. It raises the delay at once and lowers it only after
  10 seconds below the lower value, so it does not oscillate.
- The High Latency box, ticked by any player (`Request(SetDelay(..))`), pins
  the delay to the High Latency preset until unticked, as in Armada.

### 3.3 Scheduled changes, speed and pause

- **Scheduled changes ride in the host's turn record** (`TurnCommands::
  schedule`), effective at the start of that record's turn, in order. A peer
  cannot run that turn without the host's record, so a change can never
  arrive after a peer has passed its turn, and the host's log, which is the
  replay, holds every change.
- A non-host asks with `Request(Action)` (pause, speed, delay); the host
  validates it and puts it in its next record with `by` set to the asker for
  the announcement. `by` is for display; authority comes from the record
  being the host's. Drops are never requested; they are voted (section 8).
- Speed is wall-clock pace only: the sim always advances in whole ticks, and
  `speed_percent` changes how many ticks per second the client asks for. The
  sim never sees it, so it cannot desync.
- Pause stops every peer before the turn whose record carries it. Nothing in
  the sim changes while paused and no peer advances, so `Resume { by }` is a
  plain host message, effective on receipt; peers resuming at slightly
  different wall times is harmless. Pings continue while paused.

## 4. Wire format

### 4.1 Frame

Every message is one frame:

```
offset size  field
0      4     magic "S64N"
4      2     protocol version, u16 little-endian (PROTOCOL_VERSION = 1)
6      4     body length, u32 little-endian, <= MAX_FRAME_BYTES - 10
10     n     body: one `Message` in postcard
```

- **The header is frozen for every version.** A peer can always read
  another build's version, even when the rest means nothing to it, and the
  relay forwards frames of any version (`peek_header`).
- **Body: postcard** (the WORKPLAN row's choice, and what replays, snapshots
  and the command seam already use; no new dependency). Postcard is
  little-endian with LEB128 varints for integers wider than a byte, `u8` as one
  byte, strings and lists as a varint length then the bytes, enums as a varint
  tag. `Fixed` is its `i64` bits. No floats are sent.
- **Exactly one frame per decode.** As in replay format 2
  ([REPLAY_FRAMING.md](REPLAY_FRAMING.md)), the decoder uses
  `postcard::take_from_bytes` and refuses leftover bytes inside the body,
  and a frame longer than its header says.
- **Stream transports** read the 10-byte header, call `frame_len`, then read
  that many bytes. `frame_len` refuses bad magic, another version and an
  oversized declared length before anything is buffered, so a reader never
  holds more than one 64 KiB frame per connection.

### 4.2 Bounds

`MAX_FRAME_BYTES` is 64 KiB. Every variable-length field has its own bound
(`proto.rs` constants): player names 32 bytes, chat 256, mission text 4096
(S2's `MAX_MISSION_TEXT_BYTES`), at most 256 commands and 8 scheduled changes
per turn record, 16 packs per peer, 256 rule entries per `Rules` part and 64
parts. `Message::validate` checks them all; `decode` runs it on every frame
and `encode` refuses to send anything `decode` would refuse.

A turn record is also bounded by bytes: `TurnCommands::fill` takes commands
from the sender's queue while they fit both the count and a frame, and leaves
the rest for the next turn. A single command too large for any frame (a Move
of tens of thousands of units) is handed back and refused locally; the UI's
selection limits keep that from happening in play. Since every record fits a
frame, a rejoin can replay any record as its own frame (section 8).

### 4.3 Versions and compatibility

- `PROTOCOL_VERSION` covers every message layout. Postcard is positional, so
  there is no "skip unknown fields" rule: any changed layout is a version
  bump, and peers on different versions do not play together. Even appending
  an enum variant bumps it: an old peer would refuse the new tag as `Corrupt`
  rather than misread it, but a version mismatch tells the player why. Since
  all peers in a game run the same build anyway (4.4), bumping costs nothing.
- Messages embed sim types (`Command`, `GameOptions`) that other lanes
  change. A golden test pins the encoding of one sample of every message, so
  such a change fails CI here and forces the bump.
- A peer receiving another version's frame (`ProtoError::Version`) sends one
  header-only `version_notice` with its own version and closes. The other
  side decodes that as `Version { found }` too, so both players learn which
  two versions met. An empty body with one's own version is an error.

### 4.4 Builds

Lockstep only works between builds whose sims are identical. `Hello.engine`
(`name`, `version`, `build`) must match the host's byte for byte, and two
builds of unknown identity (empty `build`) never match (`EngineId::matches`):
nothing says two uncommitted builds share a sim. The client fills `build`
with its commit and a dirty flag. That is stricter than necessary (a UI-only
change also refuses), which is the safe side for v1 (open question 2). Equal
builds imply equal `SEAM_REV` and an equal rules digest scheme, so neither is
checked separately.

## 5. Handshake (D10)

```
joiner                         host
  Hello  ───────────────────────▶     header version; engine; rules digest
         ◀─────────────────── Welcome   (seat, host seat, rejoin token)      or
         ◀─────────────────── Refuse    (Full, InProgress, Closed, Engine, Protocol)
  on a rules digest mismatch:
         ◀───────────────── RulesRequest
  Rules part 0..n ──────────────▶     host reassembles and compares
         ◀──────── Refuse(RulesDiffer { key, host value, joiner value })
                   or Refuse(RulesUnlisted)
```

- `Hello` carries `engine`, the **rules digest**, the content **lineage**,
  the **pack list**, and the player's name. Only engine and the rules digest
  are compared. Lineage and packs are shown in the lobby and recorded in the
  replay; peers that differ only in textures, models or render recipes play
  together (D10).
- `RulesDigest` is 32 bytes, room for SHA-256 over CON-35's resolved rules.
  Until CON-35 lands, `ContentHashRules` puts today's `ContentDb::hash()`
  (names, profile, features and every number the sim reads) in its first
  eight bytes.
- **Naming the key.** A digest cannot say what differs, so on a mismatch the
  host asks for the joiner's resolved rules (`RulesRequest`). The joiner sends
  them as `Rules` parts (`rules_parts`): sorted by key across all parts, keys
  at most 96 bytes, values over 96 bytes replaced by `#<length>:<FNV-1a-64>`
  (`hashed_value`). Nothing is truncated, so a long value that differs late
  still differs and two long keys never collide. The host feeds the parts to a
  `RulesAssembler`, which refuses parts out of order, duplicated, over 64, or
  with keys going backwards (then `Refuse(Protocol)`), renders its own list
  the same way (`wire_entries`), and walks both (`first_difference`). It
  refuses with the first differing key and both values, so both screens can
  say "rules differ: `RTS_CFG.h:cfgX` is 5 here, 7 there". If the digests
  differ but every entry agrees, `RulesUnlisted` says that a peer's entry list
  leaves out something its digest covers, which is a CON-35 bug. Only the
  joiner's list crosses the network, at most once per join, at most 64
  frames.
- **The seam for CON-35** is the `RulesIdentity` trait in `proto.rs`:
  `digest()` and `entries()` (every resolved key/value pair the digest
  covers, in a canonical rendering, sorted bytewise, keys unique and at most
  96 bytes). `ContentHashRules` implements it today with one entry,
  `content.hash`, so the whole flow works and is tested before CON-35 lands.
  CON-35 then provides the real implementation and nothing in the protocol
  changes.
- **A question for CON-35 (flagged on #401).** Commands carry `ContentKey`s,
  which index the name-sorted content table (S1). Two peers with equal rule
  values but a different set of definition names would decode each other's
  keys as different units. D10 says the digest covers "values, not names";
  one reading that stays within it is that each definition's resolved values
  are an entry keyed by the definition, so the set of definitions is
  covered. Whether that reading is right is for CON-35 and, if it changes
  D10, for a decision PR approved by Tammy and Cedric (open question 8).

## 6. Lobby, start and play

- **Lobby.** The host broadcasts `Lobby` whenever it changes (map, options,
  seats with each peer's lineage and packs, ready flags, closed). A joiner
  sends `SeatChoice` (team, race, ready). The lobby UI is UI-11; this is only
  what it sends.
- **Map.** Maps are not sent. `MapRef` is the map's name and `map_digest`, a
  64-bit FNV-1a over its parsed `MapDef` in postcard. A peer without the map,
  or with a different one, is not ready (map download is out of scope).
- **Start** carries everything a peer needs to build the same `GameSetup`
  from its own content and map files: seed (the host draws it from OS
  randomness), map, `GameOptions`, the seats (slot, team, race, occupant,
  name, and each peer's lineage and packs), the host's seat, the
  `TurnConfig`, and the rules digest everyone matched. The replay header
  takes the digest, lineages and packs from here (CON-35 adds those fields to
  the replay). `Start::player_setups(me)` maps seats to the sim's `Control` as
  seen from seat `me`: own seat `Human`, other people `Remote`, computer
  players `Ai` on the host and `Remote` elsewhere.
- **Turn records.** `TurnCommands { turn, slot, commands, schedule, digest }`,
  the first one for turn `delay_turns` (3.2).
- **Authority.** Every received message is first checked against its role
  (`Message::role`, section 9). `TurnCommands::authorize` then checks a
  record: `slot` must be the sending connection's seat; a command's `player`
  must be the sender's own seat, or, from the host only, a computer seat or
  `PlayerId::SCRIPT`; `Edit` commands only ever as `SCRIPT` from the host (S2:
  "lockstep peers reject it from players"); `PlayerId::BENCH` never appears on
  the wire; only the host's record may carry a schedule. Within one record
  commands are strictly ordered by `(player, seq)`. NET-02 also checks that
  each sender's turns arrive in order without gaps and that each player's
  `seq` keeps increasing across turns. After an AI takeover the router's seat
  table widens the host's authority to that seat (`host_speaks_for` is only
  the starting table), and the host continues that seat's `seq` after the
  last one it saw. A record that fails any check is a protocol error from that
  peer: it is disconnected (and may be dropped), never applied.

## 7. Desync detection and reports

- Every `TurnCommands` carries the sender's sim digest (`Sim::digest`) for
  the latest turn it has run: the state at that turn's first tick, before the
  tick's commands apply, as `sta_sim::replay` records it. Each peer keeps its
  own digests for the last 64 turns and compares every report it receives.
  A report for turn *T* goes out with the sender's next record, so a
  divergence at *T* is seen about one turn plus one network trip after both
  peers have run *T*. NET-02's acceptance ("detected within 1 turn") is its
  own to measure against this.
- On a mismatch the peer:
  1. sends `Desync { slot, ours, theirs }` to everyone;
  2. writes its **replay** (`sta_sim::replay`, the same `Recorder` log it has
     kept all game, recording a digest every turn, not every 30 ticks) up to
     the current tick. The replay is the report: the setup, every command
     and the digests, so a developer can rerun it on one machine and bisect;
  3. writes a sidecar beside it, `<replay>.desync.txt`: the tick, every
     peer's reported digest for it, the engine ids, lineages and packs of
     all seats (from `Start`), and the protocol version;
  4. stops the match. The host puts a final pause in its record; there is no
     resume, only leaving. As in Armada II, a desynced game is not repaired;
     unlike Armada II, it does not run on in an undefined state.
- Comparing two peers' replays of the same game shows the first digest that
  differs between machines; finding which subsystem diverged needs a state
  dump (out of scope; open question 6).

## 8. Stalls, drops, leaving and rejoining

- **Stall.** When the next turn waits on a seat for more than 3 seconds of
  wall time, every peer shows Armada II's "Waiting for N seconds" dialog with
  that seat and its state (quit, disconnected, in the shell, unresponsive) and
  can vote (`DropVote`: wait, eliminate, AI takeover). The host decides once
  every connected human has voted for one outcome, or by its own choice when
  it is the only one voting.
- **Drop, without a race.** A peer may already have run turns the host has
  not, because the stalled seat's record reached it first. So before dropping
  a seat the host has the relay close that seat's connection (if still open).
  The relay then sends every member a **departure marker** in that seat's
  stream, after the last frame it forwarded from it (NET-03's envelope
  carries it). Because the relay forwards each sender's frames to everyone in
  one order, every record any peer could have run precedes the marker at the
  host too. The host waits for the marker, takes `last_turn` as the seat's
  last record before it, and puts `Action::Drop { slot, last_turn, then }` in
  its record. Every peer counts that seat's records up to `last_turn` and has
  no others.
- **What a drop does to the sim** must be in the command log, or the replay
  will not reproduce it. AI takeover is: the host's AI sends that seat's
  commands from `last_turn + 1`. Elimination is not yet: no `Command`
  eliminates a player today (`sta_sim` eliminates a player who has lost every
  unit). NET-02 needs an S2 seam addition for it, sent by the host as
  `SCRIPT`, for example an `Edit` that eliminates a player (open question 9).
- **Leaving.** `Leave(Quit | ToShell)` then close: treated as a drop decided
  at once, with the host's default outcome.
- **The host leaving** ends the game for everyone in v1: the computer players
  and the mission die with it, and there is no host migration. Every peer
  writes its replay.
- **Rejoin.** A connection lost to the relay (15 s of silence) may come back
  while its seat is stalled and not yet dropped:
  - The client sends `Rejoin { hello, slot, token, next_turn }` instead of
    `Hello`. `hello` is checked like a joiner's (engine, rules digest), so a
    client restarted on another build cannot come back. A wrong token, or a
    dropped seat, is refused with `BadRejoin`.
  - The host answers `Rejoined { last_turn, rejoin_token }`. `last_turn` is
    the last of that seat's own records the host has; the client resends its
    kept records after it (a record it ran locally may never have left), or,
    if it lost them with its sim, empty ones. Nobody can have run a record
    the host never received, so replacing a lost record with an empty one
    is safe. The old token is spent and the new one replaces it.
  - If the client still has its sim (a network blip), `next_turn` is the
    first turn it lacks; the host sends every record from there, any seat's,
    as `Replayed` frames, then `CatchUpDone`, and live play continues.
  - If the client lost its sim (it restarted), `next_turn` is `None`: the host
    sends `Start`, then the whole log from turn 0 as `Replayed` frames; the
    client fast-forwards its sim headless, checking the digests, then joins
    live play. The others wait in the stall dialog meanwhile.
  - The host can do this because it keeps the whole turn log, which it needs
    for its replay anyway: a 30-minute game is 18,000 turns per seat, mostly
    empty, well under a megabyte.
  - Snapshot-based resync (sending state instead of history) is deferred: it
    needs a proof that a snapshot loaded on another machine continues with
    the same digests, and it is the only way to rejoin a long game quickly
    (open question 4).

## 9. Relay model (for NET-03)

- A **room** is the relay's unit: up to 8 seats plus the host flag, created by
  the host, joined by code (Refit's six-character join codes, `K7M-Q2X`, are
  the model), optionally with a password (as Armada II's games had). The relay
  forwards frames; it reads only frame headers (`peek_header`, which accepts
  every version so a version bump needs no relay update) and never decodes
  bodies.
- **The relay stamps the sender.** NET-03's envelope carries the sending
  seat and whether it is the host, as the relay knows them from the
  connection, never from what the client claims. Receivers check each message
  against its role (`Message::check_sender`), and `TurnCommands::slot` and
  `Chat::from` against the stamp. Without that stamp a peer could forge
  another seat's input or the host's messages.

  | Role | Messages |
  |---|---|
  | Host only | `Welcome`, `Refuse`, `RulesRequest`, `Lobby`, `Start`, `Resume`, `Rejoined`, `Replayed`, `CatchUpDone` |
  | Non-host, to the host only | `Hello`, `Rules`, `SeatChoice`, `Request`, `DropVote`, `Rejoin` |
  | Any seated peer | `Turn`, `Ping`, `Pong`, `Chat`, `Desync`, `Leave` |

- **Directed frames.** The envelope carries a destination mask and the relay
  delivers to it only. Everything in the handshake and rejoin goes only
  between the host and that peer, so no other member ever sees a rejoin
  token. Chat carries a recipient set (`Chat::recipients`, one bit per seat),
  resolved by the sender from its own view of alliances, which is the same
  on every peer, so private and team chat stay private from other players
  (not from the relay operator).
- **Departure markers** (section 8): after closing a member's connection, the
  relay sends every remaining member a marker in that seat's stream.
- **Limits at the relay,** after Refit's server: frames over
  `MAX_FRAME_BYTES` close the connection; a per-connection byte rate (Refit
  uses 128 KB/s; lockstep needs well under 5 KB/s per peer); a per-address
  limit on rooms and on join-code lookups.
- Direct peer-to-peer paths (hole punching) are an optimisation the protocol
  does not depend on; v1 relays everything (section 11).

## 10. Security

Every peer is untrusted, and so is anything relayed.

- **No panic on input.** `decode` returns `ProtoError` for wrong magic,
  another version, truncated, oversized and over-long frames, unknown enum
  tags, bad varints, invalid UTF-8, leftover bytes and out-of-bound fields.
  The tests cut every sample frame at every length, reassemble a stream byte
  by byte, and run 20,000 random and mutated frames through the decoder.
- **Bounded memory.** The header is checked before the body is buffered, and
  a body is at most 64 KiB. Decoding can still allocate a small multiple of
  that: serde's `Vec` decoding preallocates up to 1 MiB per list from a
  length prefix before the bytes run out, and decoded commands are larger
  than their varints. That bound is per frame and fixed. No message type is
  recursive. Before a joiner is accepted, the host holds at most one rules
  list of 64 frames for it.
- **Authority** (sections 6 and 9) keeps a peer from issuing another seat's
  orders, edits, bench commands or schedules, or the host's messages; the sim
  already rejects invalid orders deterministically (`SimEvent::Rejected`).
- **Text.** Names, chat and rule values are valid UTF-8 by construction; the UI
  strips control characters before showing them (Armada II 1.1 removed
  "atypical characters" from chat).
- **What lockstep cannot hide.** Every peer holds the whole game state, so a
  modified client can reveal fog of war. That is inherent to lockstep and out
  of scope, as is encryption: frames are plaintext between peer and relay.
  The rejoin token never reaches other players (directed frames) and is
  rotated on every rejoin, but it does not protect against someone on the
  network path.

## 11. Out of scope for v1

- Host migration; a game outlives its host only as replays.
- Observers / spectators (Fleet Operations has them). The seat model leaves
  room: an observer would be a connection with no seat that receives turns
  and sends none.
- Multiplayer save and load (Armada II 1.1's Ctrl+S), and snapshot resync.
- Map download; peers need the same map file.
- Playing on after a desync, by majority or otherwise.
- Peer-to-peer paths and NAT hole punching; the relay carries everything.
- Encryption, accounts, matchmaking, the public game list (Refit's milestone
  3 is the model for later), a ping filter, anti-cheat.
- Voice chat (Refit also declined to reimplement DirectPlay Voice).
- Play between different builds or protocol versions, and any interop with
  the original games' DirectPlay protocol.

## 12. Hand-offs

- **NET-02** builds the `CommandRouter` on these rules: turn assembly from
  turn `delay_turns`, the gap-free stream, roles and `authorize`, the mutable
  seat table, schedules applied at their turn, digest comparison, desync
  reports, delay changes, drops, and `LoopbackNet` for two sims in one
  process. It needs the S2 elimination command (section 8).
- **NET-03** builds the transport (reliable ordered frames; the envelope with
  the sender stamp, host flag and destination mask; departure markers), the
  relay rooms with codes and passwords, rejoin transport and limits.
- **CON-35** implements `RulesIdentity` for its typed rules (keys at most 96
  bytes, every covered value listed; section 5) and adds rules digest,
  lineages and packs to the replay and snapshot headers.
- **UI-11** shows the lobby, the refusal with the named key, the stall
  dialog, and the High Latency box. It will find that `GameOptions` (S7) does
  not yet express all of Armada II's multiplayer setup data: `RTS_CFG.h` has
  three starting-resource modes (`MP_RESOURCE_MODES`, mods may define more)
  and game-type parameters such as `cfgDEFEND_TIME`; `Start` carries whatever
  `GameOptions` grows into.

## 13. Open questions for review

1. **Turn length and delay defaults.** 3 ticks per turn (100 ms) with 2
   turns of delay normally and 5 with High Latency. Armada's own numbers are
   not in its data; the only measurement is the ~233 ms send interval. Keep
   these, or match Armada's feel more closely once it can be measured on the
   bench?
2. **Engine match strictness.** Peers must run byte-identical
   `name`/`version`/`build`, with unknown builds never matching. Relax it to
   "same sim" (a sim revision constant bumped by sim changes) so UI-only
   differences can play together?
3. **After a desync**, stop the match for everyone (this draft), or do as
   Armada II did and let players carry on at their own risk?
4. **Rejoin after a restart** replays the whole game headless. Is that
   acceptable for v1, or should snapshot resync (and the cross-machine
   snapshot determinism proof it needs) come first?
5. **Drops.** Default outcome when a seat leaves or times out with no votes:
   AI takeover (with which AI) or elimination? And is "every connected human
   agrees, else the host decides" the right rule?
6. **Desync diagnosis.** Should a desync also exchange a full state dump at
   the bad tick (bigger, but names the diverging entity), or are the two
   replays enough for v1?
7. **The host leaving ends the game.** Acceptable for v1, given AI and Lua
   run only on the host?
8. **D10 and the content key table.** Commands carry `ContentKey`s, so peers
   must also agree on the set of definitions, not only rule values. Is the
   reading in section 5 (each definition an entry, valued by its resolved
   values) within D10, or does D10 need a decision PR?
9. **Elimination as a command.** Agree that NET-02 adds an S2 command (a
   `SCRIPT` edit) that eliminates a player, so a dropped seat's elimination
   is in the replay?
