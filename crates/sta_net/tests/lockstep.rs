//! NET-02: two (or three) sims in one process, fed only through the lockstep
//! router over `LoopbackNet`. Built on the public API alone.

use std::path::PathBuf;
use std::sync::Arc;

use sta_content::ContentDb;
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Tick, Vec3};
use sta_net::lockstep::{
    admit, Fault, HandshakeError, LinkFault, LockstepRouter, LoopbackNet, Peer, RouterError,
    RouterEvent, StepOutcome,
};
use sta_net::proto::{
    Action, DropChoice, EngineId, Hello, MapRef, Message, Occupant, PackInfo,
    ProtoError, Refusal, RuleEntry, RulesDigest, RulesIdentity, Scheduled, Seat, Start, TickDigest,
    TurnCommands, TurnConfig,
};
use sta_sim::command::{Command, CommandKind, CommandRouter, Edit, Order, Target};
use sta_sim::replay::Replay;
use sta_sim::setup::GameOptions;
use sta_sim::WorldSource;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn packs() -> Vec<PackInfo> {
    vec![PackInfo {
        name: "Test pack".into(),
        version: "1".into(),
    }]
}

fn seat(slot: u8, occupant: Occupant) -> Seat {
    Seat {
        slot: PlayerId(slot),
        team: TeamId(slot),
        race: "fx".into(),
        occupant,
        name: format!("p{slot}"),
        lineage: "Test lineage".into(),
        packs: packs(),
    }
}

fn cfg(ticks_per_turn: u8, delay_turns: u8) -> TurnConfig {
    TurnConfig {
        ticks_per_turn,
        delay_turns,
        speed_percent: 100,
    }
}

fn start(peers: u8, turn: TurnConfig) -> Start {
    Start {
        seed: 0x5eed,
        map: MapRef {
            name: "fx_two_bases".into(),
            digest: 0,
        },
        options: GameOptions::default(),
        seats: (0..peers).map(|s| seat(s, Occupant::Peer)).collect(),
        host: PlayerId(0),
        turn,
        rules: RulesDigest([0; 32]),
    }
}

struct Room {
    net: LoopbackNet,
    peers: Vec<Peer>,
    db: Arc<ContentDb>,
    seq: [u32; 256],
    last_order_tick: Vec<Option<Tick>>,
    rng: Rng,
}

fn room_with(start: &Start, latency: u64, jitter: u64, seed: u64) -> Room {
    let db = Arc::new(sta_content::fake::db());
    let map = Arc::new(sta_content::fake::two_bases());
    let ids: Vec<PlayerId> = start.seats.iter().map(|s| s.slot).collect();
    let (net, links) = LoopbackNet::with_delay(&ids, latency, jitter, seed ^ 0xabcd);
    let peers = links
        .into_iter()
        .map(|l| {
            let me = l.seat();
            Peer::start(start, me, db.clone(), map.clone(), Box::new(l)).unwrap()
        })
        .collect::<Vec<_>>();
    let n = peers.len();
    Room {
        net,
        peers,
        db,
        seq: [0; 256],
        last_order_tick: vec![None; n],
        rng: Rng(seed),
    }
}

fn room(peers: u8, turn: TurnConfig) -> Room {
    room_with(&start(peers, turn), 0, 0, 1)
}

impl Room {
    fn next_seq(&mut self, player: PlayerId) -> u32 {
        let s = &mut self.seq[usize::from(player.0)];
        let v = *s;
        *s += 1;
        v
    }

    /// The host spawns forces for every seat, as a mission script would.
    fn spawn_forces(&mut self, per_seat: u32) {
        let cruiser = self.db.key("fx_cruiser").unwrap();
        let frigate = self.db.key("fx_frigate").unwrap();
        let n = self.peers.len() as i32;
        for p in 0..n {
            for k in 0..per_seat as i32 {
                let seq = self.next_seq(PlayerId::SCRIPT);
                let odf = if k % 2 == 0 { cruiser } else { frigate };
                let x = if p % 2 == 0 { -150 + 40 * p } else { 150 - 40 * p };
                self.peers[0].submit(Command {
                    player: PlayerId::SCRIPT,
                    seq,
                    kind: CommandKind::Edit(Edit::Spawn {
                        odf,
                        player: PlayerId(p as u8),
                        pos: Vec3::from_f32([x as f32, 0.0, (k * 70 - 100) as f32]),
                        heading: (Fixed::ZERO, Fixed::ONE),
                    }),
                });
            }
        }
    }

    /// A random order from seat `i`, from its own view of the world.
    fn random_order(&mut self, i: usize) -> Option<Command> {
        let me = PlayerId(i as u8);
        let view = self.peers[i].sim().view();
        let mine: Vec<EntityId> = view.units.iter().filter(|u| u.owner == me).map(|u| u.id).collect();
        let theirs: Vec<EntityId> = view.units.iter().filter(|u| u.owner != me).map(|u| u.id).collect();
        if mine.is_empty() {
            return None;
        }
        let take = 1 + self.rng.below(mine.len() as u64) as usize;
        let units: Vec<EntityId> = mine.iter().copied().take(take).collect();
        let kind = match self.rng.below(9) {
            0..=2 => {
                let to = Vec3::from_f32([
                    self.rng.below(600) as f32 - 300.0,
                    0.0,
                    self.rng.below(600) as f32 - 300.0,
                ]);
                Order::Move {
                    units,
                    to,
                    queue: self.rng.below(4) == 0,
                }
            }
            3 => Order::Stop { units },
            4 | 5 if !theirs.is_empty() => Order::Attack {
                units,
                target: theirs[self.rng.below(theirs.len() as u64) as usize],
            },
            6 => Order::Formation {
                units,
                kind: self.rng.below(4) as u8,
            },
            7 => Order::Cloak {
                units,
                on: self.rng.below(2) == 0,
            },
            8 => Order::Special {
                units,
                slot: self.rng.below(3) as u8,
                target: Target::None,
            },
            _ => Order::Stop { units },
        };
        let seq = self.next_seq(me);
        Some(Command {
            player: me,
            seq,
            kind: if self.rng.below(15) == 0 {
                CommandKind::MissionText {
                    text: format!("hello {seq}"),
                }
            } else {
                CommandKind::Order(kind)
            },
        })
    }

    /// Steps the peers until every one is at `target`, in a pseudo-random
    /// interleaving, submitting random orders when `orders`.
    fn run_to(&mut self, target: Tick, orders: bool) {
        let mut idle = 0u32;
        while self.peers.iter().any(|p| p.tick() < target) {
            self.net.advance(1);
            let mut order: Vec<usize> = (0..self.peers.len()).collect();
            for k in (1..order.len()).rev() {
                let j = self.rng.below(k as u64 + 1) as usize;
                order.swap(k, j);
            }
            let mut progress = false;
            for i in order {
                let burst = 1 + self.rng.below(4);
                for _ in 0..burst {
                    let t = self.peers[i].tick();
                    if t >= target {
                        break;
                    }
                    if orders
                        && t.is_multiple_of(7)
                        && self.last_order_tick[i] != Some(t)
                        && self.rng.below(3) == 0
                    {
                        self.last_order_tick[i] = Some(t);
                        if let Some(c) = self.random_order(i) {
                            self.peers[i].submit(c);
                        }
                    }
                    match self.peers[i].step() {
                        StepOutcome::Ran { .. } => progress = true,
                        StepOutcome::Stalled { .. } | StepOutcome::Halted => break,
                    }
                }
            }
            if progress {
                idle = 0;
            } else {
                idle += 1;
                assert!(idle < 5_000, "no peer can advance: deadlock");
            }
        }
    }

    /// One tick for every peer in turn, until all are at `target`.
    fn round_robin_to(&mut self, target: Tick, mut each: impl FnMut(&mut Room, Tick)) {
        let mut idle = 0;
        while self.peers.iter().any(|p| p.tick() < target) {
            self.net.advance(1);
            let mut progress = false;
            for i in 0..self.peers.len() {
                if self.peers[i].tick() < target {
                    let t = self.peers[i].tick();
                    if i == 0 {
                        each(self, t);
                    }
                    if matches!(self.peers[i].step(), StepOutcome::Ran { .. }) {
                        progress = true;
                    }
                }
            }
            idle = if progress { 0 } else { idle + 1 };
            assert!(idle < 5_000, "deadlock");
        }
    }

    /// Steps everyone until none advances, for at most `rounds`.
    fn settle(&mut self, rounds: u32) {
        for _ in 0..rounds {
            self.net.advance(1);
            let mut progress = false;
            for p in &mut self.peers {
                if matches!(p.step(), StepOutcome::Ran { .. }) {
                    progress = true;
                }
            }
            if !progress {
                return;
            }
        }
    }

    fn events(&mut self, i: usize) -> Vec<RouterEvent> {
        self.peers[i].router_mut().take_events()
    }
}

fn first_divergence(a: &Replay, b: &Replay) -> Option<Tick> {
    a.digests
        .iter()
        .zip(&b.digests)
        .find(|(x, y)| x.0 != y.0 || x.1 != y.1)
        .map(|(x, _)| x.0)
}

/// All peers at one tick, one state, one command log, and the replay of the
/// log reproduces that state on a single machine.
fn assert_in_sync(room: &Room) -> u64 {
    let first = &room.peers[0];
    let digest = first.sim().digest();
    for p in &room.peers {
        assert_eq!(p.tick(), first.tick());
        assert_eq!(p.sim().digest(), digest, "peer {} diverged", p.router().me().0);
        assert_eq!(p.replay().digests, first.replay().digests);
        assert_eq!(p.replay().commands, first.replay().commands);
        assert!(
            p.router().desync().is_none() && p.router().peer_desync().is_none(),
            "desync reported in a healthy game"
        );
    }
    let sim = first.replay().play(room.db.clone()).expect("replay verifies");
    assert_eq!(sim.digest(), digest);
    digest
}

/// The turn a halted peer says first differed: its own finding, or the one
/// it was told of (a peer that had not reached that turn yet).
fn noticed_tick(p: &Peer) -> Tick {
    let r = p.router();
    r.desync()
        .map(|n| n.theirs.tick)
        .or_else(|| r.peer_desync().map(|(_, n)| n.ours.tick))
        .expect("the peer knows of the desync")
}

fn tmp_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------------------
// Equal digests
// ---------------------------------------------------------------------------

#[test]
fn scripted_two_peers_stay_identical_for_10k_ticks() {
    let run = |orders: bool| {
        let mut r = room(2, cfg(3, 2));
        r.spawn_forces(3);
        r.run_to(10_000, orders);
        let digest = assert_in_sync(&r);
        (digest, r.peers[0].replay().commands.len())
    };
    let (with, n) = run(true);
    let (without, _) = run(false);
    // Non-vacuous: orders were released, and they changed the outcome.
    assert!(n > 200, "only {n} commands in 10k ticks");
    assert_ne!(with, without, "orders had no effect on the state");
}

#[test]
fn randomised_streams_stay_identical_across_turn_shapes_and_delivery() {
    // (peers, ticks per turn, delay, latency, jitter, seed)
    let cases = [
        (2, 1, 1, 0, 0, 11),
        (2, 3, 2, 3, 5, 12),
        (3, 3, 2, 2, 9, 13),
        (2, 5, 5, 6, 12, 14),
        (4, 3, 3, 0, 4, 15),
        (3, 15, 1, 1, 1, 16),
        (2, 2, 30, 10, 40, 17),
    ];
    for (peers, tpt, delay, latency, jitter, seed) in cases {
        let mut r = room_with(&start(peers, cfg(tpt, delay)), latency, jitter, seed);
        r.spawn_forces(2);
        r.run_to(2_500, true);
        assert_in_sync(&r);
        assert!(
            r.peers[0].replay().commands.len() > 40,
            "case {seed}: too few commands"
        );
        // The shuffled delivery really did interleave: the host's events
        // show no protocol errors.
        for i in 0..r.peers.len() {
            let bad: Vec<_> = r
                .events(i)
                .into_iter()
                .filter(|e| matches!(e, RouterEvent::Protocol { .. }))
                .collect();
            assert!(bad.is_empty(), "case {seed}: {bad:?}");
        }
    }
}

#[test]
fn long_randomised_stream_with_jitter_10k_ticks() {
    let mut r = room_with(&start(2, cfg(3, 2)), 2, 7, 99);
    r.spawn_forces(4);
    r.run_to(10_000, true);
    assert_in_sync(&r);
    assert!(r.peers[0].replay().commands.len() > 200);
}

// ---------------------------------------------------------------------------
// Input delay
// ---------------------------------------------------------------------------

fn applied_tick(p: &Peer, player: PlayerId, seq: u32) -> Option<Tick> {
    p.replay()
        .commands
        .iter()
        .find(|l| l.command.player == player && l.command.seq == seq)
        .map(|l| l.tick)
}

#[test]
fn commands_apply_exactly_delay_turns_after_the_turn_they_were_given_in() {
    for (tpt, delay) in [(3u8, 2u8), (5, 4), (1, 1)] {
        let mut r = room(2, cfg(tpt, delay));
        r.spawn_forces(2);
        r.run_to(60, false);
        let mine = r.peers[1]
            .sim()
            .view()
            .units
            .iter()
            .find(|u| u.owner == PlayerId(1))
            .map(|u| u.id)
            .unwrap();
        let mv = |seq| Command {
            player: PlayerId(1),
            seq,
            kind: CommandKind::Order(Order::Stop { units: vec![mine] }),
        };
        let tpt = Tick::from(tpt);
        let delay = Tick::from(delay);
        // Given at a turn's first tick, before it runs: part of that turn's
        // send, applied `delay` turns on.
        let t0 = r.peers[1].tick();
        assert_eq!(t0 % tpt, 0);
        r.peers[1].submit(mv(0));
        r.run_to(t0 + 1, false);
        assert_eq!(applied_tick(&r.peers[1], PlayerId(1), 0), None);
        // Given one tick later, mid-turn (if turns have one): next turn's.
        let t1 = r.peers[1].tick();
        r.peers[1].submit(mv(1));
        r.run_to(t0 + 2 * tpt * (delay + 3), false);
        let a0 = t0;
        let want0 = a0 + delay * tpt;
        let turn_of_t1 = t1 / tpt;
        let want1 = if t1.is_multiple_of(tpt) {
            t1 + delay * tpt
        } else {
            (turn_of_t1 + 1 + delay) * tpt
        };
        for p in &r.peers {
            assert_eq!(applied_tick(p, PlayerId(1), 0), Some(want0), "tpt {tpt} delay {delay}");
            assert_eq!(applied_tick(p, PlayerId(1), 1), Some(want1), "tpt {tpt} delay {delay}");
        }
        assert_in_sync(&r);
    }
}

#[test]
fn scheduled_delay_changes_keep_the_stream_gap_free() {
    let mut r = room(2, cfg(3, 2));
    r.spawn_forces(3);
    r.run_to(120, true);
    r.peers[1].router_mut().request(Action::SetDelay(6));
    r.run_to(400, true);
    for p in &r.peers {
        assert_eq!(p.router().config().delay_turns, 6);
    }
    r.peers[0].router_mut().schedule(Action::SetDelay(1));
    r.peers[0].router_mut().schedule(Action::SetSpeed(250));
    r.run_to(900, true);
    for p in &r.peers {
        assert_eq!(p.router().config().delay_turns, 1);
        assert_eq!(p.router().config().speed_percent, 250);
    }
    for i in 0..2 {
        assert!(!r.events(i).iter().any(|e| matches!(e, RouterEvent::Protocol { .. })));
    }
    assert_in_sync(&r);
}

// ---------------------------------------------------------------------------
// Stalls
// ---------------------------------------------------------------------------

#[test]
fn a_silent_peer_stalls_the_others_and_nothing_diverges() {
    let mut r = room(2, cfg(3, 2));
    r.spawn_forces(2);
    r.run_to(30, true);
    r.net.inject(LinkFault::Hold(PlayerId(1)));
    // Peer 0 can run only the turns it already has records for.
    let mut last = r.peers[0].tick();
    for _ in 0..400 {
        r.net.advance(1);
        r.peers[1].step();
        match r.peers[0].step() {
            StepOutcome::Ran { tick, .. } => last = tick,
            StepOutcome::Stalled { waiting } => {
                assert_eq!(waiting, vec![PlayerId(1)]);
            }
            StepOutcome::Halted => panic!("a late turn is not a desync"),
        }
    }
    let frozen = r.peers[0].sim().digest();
    assert_eq!(r.peers[0].tick(), last);
    assert!(last <= 30 + 2 * 3 + 3, "ran past the missing record: {last}");
    for _ in 0..100 {
        assert!(matches!(r.peers[0].step(), StepOutcome::Stalled { .. }));
    }
    assert_eq!(r.peers[0].sim().digest(), frozen, "a stalled sim must not change");
    assert!(r.peers[0].router().desync().is_none());
    assert_eq!(r.peers[0].router().waiting_on(), vec![PlayerId(1)]);

    // The turn arrives late: play resumes and the peers agree.
    r.net.release(PlayerId(1));
    r.run_to(300, true);
    assert_in_sync(&r);
}

#[test]
fn a_swapped_pair_from_one_sender_is_refused_and_stalls() {
    let mut r = room(2, cfg(3, 2));
    // Peer 1 sends two records before peer 0 polls.
    r.net.inject(LinkFault::SwapNextPair(PlayerId(1)));
    for _ in 0..8 {
        r.peers[1].step();
    }
    let mut stalled = false;
    for _ in 0..50 {
        stalled |= matches!(r.peers[0].step(), StepOutcome::Stalled { .. });
    }
    assert!(stalled);
    let events = r.events(0);
    assert!(
        events.iter().any(|e| matches!(
            e,
            RouterEvent::Protocol {
                error: RouterError::OutOfOrder { got: 3, expected: 2, .. },
                ..
            }
        )),
        "{events:?}"
    );
    // The bad record was not applied: the peer waits on seat 1 for turn 2.
    assert_eq!(r.peers[0].router().waiting_on(), vec![PlayerId(1)]);
}

#[test]
fn dropping_a_silent_seat_lets_the_rest_play_on() {
    for then in [DropChoice::Eliminate, DropChoice::AiTakeover("normal".into())] {
        let mut r = room(3, cfg(3, 2));
        r.spawn_forces(2);
        r.run_to(90, true);
        r.net.inject(LinkFault::Hold(PlayerId(2)));
        r.net.advance(1);
        // Run until both others stall on seat 2.
        for _ in 0..200 {
            r.net.advance(1);
            for i in 0..3 {
                r.peers[i].step();
            }
        }
        assert_eq!(r.peers[0].router().waiting_on(), vec![PlayerId(2)]);
        assert_eq!(r.peers[1].router().waiting_on(), vec![PlayerId(2)]);
        // The host reads the seat's last record (here, from what it holds;
        // the relay's departure marker in a real game) and drops it. It
        // sends the change without advancing.
        let last = r.peers[0].router().last_record(PlayerId(2)).unwrap();
        r.peers[0].router_mut().schedule(Action::Drop {
            slot: PlayerId(2),
            last_turn: last,
            then: then.clone(),
        });
        if let DropChoice::AiTakeover(_) = then {
            // The host's AI now speaks for the seat from the next record on.
            let seq = r.next_seq(PlayerId(2));
            let unit = r.peers[0]
                .sim()
                .view()
                .units
                .iter()
                .find(|u| u.owner == PlayerId(2))
                .map(|u| u.id)
                .unwrap();
            r.peers[0].submit(Command {
                player: PlayerId(2),
                seq,
                kind: CommandKind::Order(Order::Move {
                    units: vec![unit],
                    to: Vec3::from_f32([5.0, 0.0, 5.0]),
                    queue: false,
                }),
            });
        }
        r.peers[0].router_mut().extend_lead();
        // The seat is gone: peers 0 and 1 continue, and agree.
        let target = r.peers[0].tick() + 600;
        let mut rr = 0;
        while r.peers[0].tick() < target || r.peers[1].tick() < target {
            r.net.advance(1);
            for i in 0..2 {
                if r.peers[i].tick() < target {
                    r.peers[i].step();
                }
            }
            rr += 1;
            assert!(rr < 10_000, "still stalled after the drop");
        }
        assert_eq!(r.peers[0].sim().digest(), r.peers[1].sim().digest());
        assert_eq!(r.peers[0].replay().commands, r.peers[1].replay().commands);
        for i in 0..2 {
            let dropped = r.events(i).into_iter().any(|e| {
                matches!(e, RouterEvent::SeatDropped { slot, last_turn, then: ref t }
                    if slot == PlayerId(2) && last_turn == last && *t == then)
            });
            assert!(dropped, "peer {i} did not see the drop");
        }
        if let DropChoice::AiTakeover(_) = then {
            let applied = r.peers[1]
                .replay()
                .commands
                .iter()
                .any(|l| l.command.player == PlayerId(2) && l.tick > u64::from(last) * 3);
            assert!(applied, "the taken-over seat's command was not applied");
        }
    }
}

#[test]
fn a_scheduled_pause_stops_every_peer_before_the_same_turn() {
    let mut r = room(3, cfg(3, 2));
    r.spawn_forces(2);
    r.run_to(60, true);
    // A non-host asks; the host schedules it.
    r.peers[2].router_mut().request(Action::Pause);
    for _ in 0..300 {
        r.net.advance(1);
        for p in &mut r.peers {
            p.step();
        }
    }
    let stop = r.peers[0].tick();
    assert_eq!(stop % 3, 0);
    for p in &r.peers {
        assert_eq!(p.tick(), stop, "everyone stops before the same turn");
        assert!(p.router().paused());
    }
    assert_eq!(r.peers[0].sim().digest(), r.peers[1].sim().digest());
    r.peers[0].router_mut().resume();
    for _ in 0..50 {
        r.net.advance(1);
        for p in &mut r.peers {
            p.step();
        }
    }
    for p in &r.peers {
        assert!(p.tick() > stop);
        assert!(!p.router().paused());
    }
    r.run_to(500, true);
    assert_in_sync(&r);
}

// ---------------------------------------------------------------------------
// Desync detection
// ---------------------------------------------------------------------------

/// Runs a game, makes peer `victim` miss one order, and returns the room
/// stopped by the desync plus the tick that order applied on.
fn run_with_dropped_order(
    peers: u8,
    victim: usize,
    latency: u64,
    jitter: u64,
    seed: u64,
) -> (Room, Tick) {
    let mut r = room_with(&start(peers, cfg(3, 2)), latency, jitter, seed);
    r.spawn_forces(3);
    r.run_to(150, true);
    r.settle(0);
    // Peer 0 orders one unit across the map: a command that changes the
    // digest within a turn.
    let unit = r.peers[0]
        .sim()
        .view()
        .units
        .iter()
        .find(|u| u.owner == PlayerId(0))
        .map(|u| u.id)
        .unwrap();
    let seq = r.next_seq(PlayerId(0));
    r.peers[0].submit(Command {
        player: PlayerId(0),
        seq,
        kind: CommandKind::Order(Order::Move {
            units: vec![unit],
            to: Vec3::from_f32([-400.0, 0.0, 300.0]),
            queue: false,
        }),
    });
    // The victim never applies it.
    r.peers[victim].inject(Fault::DropCommand { player: PlayerId(0), seq });
    // Run on until a peer reports the desync.
    let mut guard = 0;
    while !r.peers.iter().any(|p| p.router().halted()) {
        r.net.advance(1);
        for i in 0..r.peers.len() {
            r.peers[i].step();
        }
        guard += 1;
        assert!(guard < 5_000, "an injected desync was never detected");
    }
    // Let the notices reach everyone.
    for _ in 0..200 {
        r.net.advance(1);
        for p in &mut r.peers {
            p.step();
        }
    }
    let applied = applied_tick(&r.peers[0], PlayerId(0), seq).expect("peer 0 applied the order");
    // The victim never did.
    assert_eq!(applied_tick(&r.peers[victim], PlayerId(0), seq), None);
    (r, applied)
}

#[test]
fn an_injected_desync_is_detected_in_one_turn_and_written_as_a_replay() {
    let (r, applied) = run_with_dropped_order(2, 1, 0, 0, 7);
    // Where do the two replays first part?
    let first = first_divergence(r.peers[0].replay(), r.peers[1].replay())
        .expect("the replays of a desynced game differ");
    // The order applies at `applied`; its effect first shows in the digest
    // of the next turn's first tick.
    assert_eq!(first, applied + 3, "first differing digest");
    for p in &r.peers {
        assert_eq!(noticed_tick(p), first, "the report names the first differing turn");
        if let Some(d) = p.router().desync() {
            assert_ne!(d.ours.digest, d.theirs.digest);
            assert_eq!(d.ours.tick, d.theirs.tick);
        }
        assert!(p.router().halted());
        // Detected at that turn's boundary, one turn after the order applied.
        assert!(p.tick() <= first + 3, "peer ran on after the desync");
    }
    // Stopped: no further progress.
    let ticks: Vec<_> = r.peers.iter().map(Peer::tick).collect();
    let mut r = r;
    for _ in 0..50 {
        for p in &mut r.peers {
            assert!(matches!(p.step(), StepOutcome::Halted));
        }
    }
    assert_eq!(r.peers.iter().map(Peer::tick).collect::<Vec<_>>(), ticks);

    // Reports: a replay and a sidecar each.
    let dir = tmp_dir("desync-two");
    let files: Vec<_> = (0..2)
        .map(|i| r.peers[i].write_desync_report(&dir, &format!("seat{i}")).unwrap())
        .collect();
    let replays: Vec<Replay> = files
        .iter()
        .map(|f| Replay::decode(&std::fs::read(&f.replay).unwrap()).unwrap())
        .collect();
    // At least one peer found it by its own comparison.
    assert!(r.peers.iter().any(|p| p.router().desync().is_some()));
    // Each replay verifies on one machine (it records what its peer ran) ...
    for (i, rep) in replays.iter().enumerate() {
        let sim = rep.play(r.db.clone()).expect("a desync replay replays");
        assert_eq!(sim.digest(), r.peers[i].sim().digest());
    }
    // ... and comparing the two names the first differing turn.
    assert_eq!(first_divergence(&replays[0], &replays[1]), Some(first));
    for (i, f) in files.iter().enumerate() {
        assert_eq!(f.sidecar.file_name().unwrap().to_str().unwrap(), format!("seat{i}.star.desync.txt"));
        let text = std::fs::read_to_string(&f.sidecar).unwrap();
        assert!(text.contains(&format!("first differing tick: {first}")), "{text}");
        assert!(text.contains("lineage \"Test lineage\""), "{text}");
        assert!(text.contains(&format!("protocol version: {}", sta_net::proto::PROTOCOL_VERSION)), "{text}");
        assert!(text.contains("digest at that tick, seat 0"), "{text}");
        assert!(text.contains("digest at that tick, seat 1"), "{text}");
    }
}

#[test]
fn an_injected_desync_is_found_with_latency_and_three_peers() {
    for (latency, jitter, victim, seed) in [(0, 0, 2, 21), (2, 5, 1, 22), (4, 9, 2, 23)] {
        let (r, applied) = run_with_dropped_order(3, victim, latency, jitter, seed);
        let reference = (0..3).find(|&i| i != victim).unwrap();
        let first = first_divergence(r.peers[reference].replay(), r.peers[victim].replay()).unwrap();
        assert_eq!(first, applied + 3);
        for p in &r.peers {
            assert!(p.router().halted(), "every peer halts");
        }
        // The victim, and at least the host, name the first differing turn.
        assert_eq!(noticed_tick(&r.peers[victim]), first);
        assert!(r.peers.iter().any(|p| p.router().desync().is_some()));
        // Healthy peers agree with each other up to then.
        let others: Vec<_> = (0..3).filter(|&i| i != victim).collect();
        assert_eq!(
            first_divergence(r.peers[others[0]].replay(), r.peers[others[1]].replay()),
            None
        );
    }
}

#[test]
fn a_local_only_command_is_a_desync_too() {
    let mut r = room(2, cfg(3, 2));
    r.spawn_forces(2);
    r.round_robin_to(90, |_, _| {});
    // Damage one unit on one side only: nothing in the command stream differs.
    let unit = r.peers[1].sim().view().units.iter().find(|u| u.owner == PlayerId(1)).unwrap().id;
    r.peers[1].inject(Fault::Inject(Command {
        player: PlayerId::SCRIPT,
        seq: 9_999,
        kind: CommandKind::Edit(Edit::Damage(unit, Fixed::from_int(7))),
    }));
    for _ in 0..40 {
        for p in &mut r.peers {
            p.step();
        }
    }
    let first = first_divergence(r.peers[0].replay(), r.peers[1].replay()).unwrap();
    assert_eq!(noticed_tick(&r.peers[0]), first);
    assert!(r.peers[0].tick() <= first + 3);
}

#[test]
fn a_healthy_game_never_reports_a_desync_across_many_seeds() {
    for seed in 100..112 {
        let mut r = room_with(&start(3, cfg(3, 2)), seed % 3, seed % 7, seed);
        r.spawn_forces(2);
        r.run_to(600, true);
        assert_in_sync(&r);
    }
}

// ---------------------------------------------------------------------------
// Router rules, on single messages
// ---------------------------------------------------------------------------

fn bare_router(me: u8) -> LockstepRouter {
    let st = start(3, cfg(3, 2));
    let ids = [PlayerId(0), PlayerId(1), PlayerId(2)];
    let (_net, mut links) = LoopbackNet::new(&ids);
    let link = links.remove(usize::from(me));
    LockstepRouter::new(&st, PlayerId(me), Box::new(link)).unwrap()
}

fn rec(turn: u32, slot: u8, commands: Vec<Command>) -> Message {
    Message::Turn(TurnCommands {
        turn,
        slot: PlayerId(slot),
        commands,
        schedule: vec![],
        digest: None,
    })
}

fn stop(player: u8, seq: u32) -> Command {
    Command {
        player: PlayerId(player),
        seq,
        kind: CommandKind::Order(Order::Stop {
            units: vec![EntityId(1)],
        }),
    }
}

#[test]
fn the_router_refuses_what_the_protocol_forbids_and_never_applies_it() {
    // Gap, repeat and early start.
    let mut r = bare_router(0);
    assert_eq!(
        r.receive(PlayerId(1), rec(4, 1, vec![])),
        Err(RouterError::OutOfOrder { slot: PlayerId(1), expected: 2, got: 4 })
    );
    // A refused seat is closed: even the right record is not taken now.
    assert_eq!(
        r.receive(PlayerId(1), rec(2, 1, vec![])),
        Err(RouterError::SeatClosed(PlayerId(1)))
    );
    let mut r = bare_router(0);
    assert_eq!(r.receive(PlayerId(1), rec(2, 1, vec![stop(1, 5)])), Ok(()));
    assert_eq!(
        r.receive(PlayerId(1), rec(2, 1, vec![])),
        Err(RouterError::OutOfOrder { slot: PlayerId(1), expected: 3, got: 2 })
    );
    // seq must keep increasing across turns.
    let mut r = bare_router(0);
    assert_eq!(r.receive(PlayerId(1), rec(2, 1, vec![stop(1, 5)])), Ok(()));
    assert_eq!(
        r.receive(PlayerId(1), rec(3, 1, vec![stop(1, 5)])),
        Err(RouterError::SeqNotIncreasing { player: PlayerId(1), seq: 5, last: 5 })
    );
    // A command for a seat the sender does not hold.
    let mut r = bare_router(0);
    assert!(matches!(
        r.receive(PlayerId(1), rec(2, 1, vec![stop(2, 0)])),
        Err(RouterError::Proto(ProtoError::Authority(_)))
    ));
    // A record sent for another seat.
    let mut r = bare_router(0);
    assert!(matches!(
        r.receive(PlayerId(1), rec(2, 2, vec![])),
        Err(RouterError::Proto(ProtoError::Authority(_)))
    ));
    // A schedule from a non-host.
    let mut r = bare_router(0);
    let m = Message::Turn(TurnCommands {
        turn: 2,
        slot: PlayerId(1),
        commands: vec![],
        schedule: vec![Scheduled { by: PlayerId(1), action: Action::Pause }],
        digest: None,
    });
    assert!(matches!(
        r.receive(PlayerId(1), m),
        Err(RouterError::Proto(ProtoError::Authority(_)))
    ));
    // An edit from a player, and from the bench.
    let mut r = bare_router(0);
    let edit = Command {
        player: PlayerId(1),
        seq: 0,
        kind: CommandKind::Edit(Edit::Kill(EntityId(1))),
    };
    assert!(r.receive(PlayerId(1), rec(2, 1, vec![edit])).is_err());
    let mut r = bare_router(0);
    let bench = Command {
        player: PlayerId::BENCH,
        seq: 0,
        kind: CommandKind::Edit(Edit::Kill(EntityId(1))),
    };
    assert!(r.receive(PlayerId(0), rec(2, 0, vec![bench])).is_err());
    // A host-only message from a non-host, and a joiner message from the host.
    let mut r = bare_router(2);
    assert!(matches!(
        r.receive(PlayerId(1), Message::Resume { by: PlayerId(1) }),
        Err(RouterError::Proto(ProtoError::Authority(_)))
    ));
    // A sender with no seat.
    let mut r = bare_router(0);
    assert_eq!(
        r.receive(PlayerId(6), rec(2, 6, vec![])),
        Err(RouterError::UnknownSeat(PlayerId(6)))
    );
    // Nothing above released a turn.
    assert_eq!(r.take_ready(0), Some(vec![]));
}

#[test]
fn local_submissions_are_checked_before_they_are_sent() {
    let mut r = bare_router(1);
    r.submit(stop(1, 3));
    r.submit(stop(1, 3)); // seq does not increase
    r.submit(stop(2, 0)); // not this seat's
    r.submit(Command {
        player: PlayerId::SCRIPT,
        seq: 0,
        kind: CommandKind::Edit(Edit::Seed(1)),
    }); // only the host speaks for the script
    let why: Vec<_> = r
        .take_events()
        .into_iter()
        .filter_map(|e| match e {
            RouterEvent::LocalRejected { player, seq, .. } => Some((player.0, seq)),
            _ => None,
        })
        .collect();
    assert_eq!(why, vec![(1, 3), (2, 0), (254, 0)]);
}

#[test]
fn a_released_turn_is_the_same_on_every_ask_and_sorted() {
    let mut host = bare_router(0);
    host.submit(stop(0, 4));
    host.submit(stop(0, 2));
    let mut r = bare_router(0);
    // Turns 0 and 1 are empty by definition; turn 2 needs every seat.
    assert_eq!(r.take_ready(0), Some(vec![]));
    assert_eq!(r.take_ready(1), Some(vec![]));
    assert_eq!(r.take_ready(3), Some(vec![]));
    assert_eq!(r.take_ready(6), None, "turn 2 waits for the others");
    assert_eq!(r.waiting_on(), vec![PlayerId(1), PlayerId(2)]);
    r.receive(PlayerId(2), rec(2, 2, vec![stop(2, 1)])).unwrap();
    r.receive(PlayerId(1), rec(2, 1, vec![stop(1, 9), stop(1, 10)])).unwrap();
    let a = r.take_ready(6).unwrap();
    let keys: Vec<_> = a.iter().map(|c| (c.player.0, c.seq)).collect();
    assert_eq!(keys, vec![(1, 9), (1, 10), (2, 1)]);
    assert_eq!(r.take_ready(6), Some(a), "asking again repeats the answer");
    assert_eq!(r.take_ready(7), Some(vec![]));
}

// ---------------------------------------------------------------------------
// Handshake (D9, D10)
// ---------------------------------------------------------------------------

struct Rules(Vec<(&'static str, &'static str)>, u8);

impl RulesIdentity for Rules {
    fn digest(&self) -> RulesDigest {
        let mut d = [self.1; 32];
        for (k, v) in &self.0 {
            for (i, b) in k.bytes().chain(v.bytes()).enumerate() {
                d[i % 32] = d[i % 32].wrapping_mul(31).wrapping_add(b);
            }
        }
        RulesDigest(d)
    }
    fn entries(&self) -> Vec<RuleEntry> {
        self.0
            .iter()
            .map(|(k, v)| RuleEntry { key: (*k).into(), value: (*v).into() })
            .collect()
    }
}

fn engine() -> EngineId {
    EngineId { name: "sta64".into(), version: "0.1.0".into(), build: "abc".into() }
}

fn hello(rules: &dyn RulesIdentity, lineage: &str, packs: Vec<PackInfo>) -> Hello {
    Hello { engine: engine(), rules: rules.digest(), lineage: lineage.into(), packs, name: "j".into() }
}

#[test]
fn peers_on_different_rules_refuse_to_start_and_name_the_key() {
    let classic = Rules(vec![("RTS_CFG.h:cfgA", "5"), ("RTS_CFG.h:cfgB", "7"), ("unit.fx:hull", "100")], 0);
    let enhanced = Rules(vec![("RTS_CFG.h:cfgA", "5"), ("RTS_CFG.h:cfgB", "9"), ("unit.fx:hull", "100")], 0);
    let h = hello(&enhanced, "STA2 Classic Enhanced", vec![]);
    let err = admit(&engine(), &classic, &h, &enhanced).unwrap_err();
    match &err {
        HandshakeError::Refused(Refusal::RulesDiffer(d)) => {
            assert_eq!(d.key, "RTS_CFG.h:cfgB");
            assert_eq!(d.host.as_deref(), Some("7"));
            assert_eq!(d.joiner.as_deref(), Some("9"));
        }
        other => panic!("{other:?}"),
    }
    assert!(err.to_string().contains("RTS_CFG.h:cfgB"));
    // A definition one side lacks is named too.
    let extra = Rules(vec![("RTS_CFG.h:cfgA", "5"), ("unit.fx:hull", "100"), ("unit.fx_new:hull", "1")], 0);
    let h = hello(&extra, "x", vec![]);
    match admit(&engine(), &classic, &h, &extra).unwrap_err() {
        HandshakeError::Refused(Refusal::RulesDiffer(d)) => assert_eq!(d.key, "RTS_CFG.h:cfgB"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn peers_differing_only_in_display_data_play_together() {
    let rules = Rules(vec![("RTS_CFG.h:cfgA", "5")], 0);
    let h = hello(
        &rules,
        "FO 4.0 with HD textures",
        vec![PackInfo { name: "HD textures".into(), version: "2".into() }],
    );
    assert_eq!(admit(&engine(), &rules, &h, &rules), Ok(()));
}

#[test]
fn another_build_and_unlisted_differences_are_refused() {
    let rules = Rules(vec![("k", "1")], 0);
    let mut h = hello(&rules, "x", vec![]);
    h.engine.build = "other".into();
    assert!(matches!(
        admit(&engine(), &rules, &h, &rules),
        Err(HandshakeError::Refused(Refusal::Engine { .. }))
    ));
    // Same listed entries, different digest: the digest covers more than it lists.
    let hidden = Rules(vec![("k", "1")], 9);
    let h = hello(&hidden, "x", vec![]);
    assert_eq!(
        admit(&engine(), &rules, &h, &hidden),
        Err(HandshakeError::Refused(Refusal::RulesUnlisted))
    );
    // The content databases' keyed digests (CON-35) work end to end.
    let a = sta_content::fake::db();
    let rules = sta_content::Rules::default().with_autonomy(sta_content::AutonomyRules { movement: 2, special_weapon: 2 });
    let b = sta_content::fake::db().with_rules(rules);
    let h = hello(b.digest(), "x", vec![]);
    match admit(&engine(), a.digest(), &h, b.digest()).unwrap_err() {
        HandshakeError::Refused(Refusal::RulesDiffer(d)) => assert_eq!(d.key, "RTS_CFG.h:CRAFT_DEFAULT_MOVEMENT_AUTONOMY"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn tick_digest_type_is_what_the_records_carry() {
    // Guards the wire shape the router relies on.
    let t = TickDigest { tick: 3, digest: 4 };
    assert_eq!((t.tick, t.digest), (3, 4));
}

// ---------------------------------------------------------------------------
// Review findings
// ---------------------------------------------------------------------------

#[test]
fn a_flood_of_requests_cannot_stall_the_game() {
    let mut r = room(3, cfg(3, 2));
    r.spawn_forces(2);
    r.run_to(30, true);
    for _ in 0..30 {
        r.peers[1].router_mut().request(Action::SetSpeed(100));
        r.peers[0].router_mut().schedule(Action::SetSpeed(100));
    }
    r.run_to(400, true);
    for i in 0..3 {
        assert!(!r.events(i).iter().any(|e| matches!(
            e,
            RouterEvent::Protocol { .. } | RouterEvent::Transport(_)
        )));
    }
    assert_in_sync(&r);
}

#[test]
fn a_takeover_record_arriving_before_the_seats_own_records_is_accepted() {
    let mut r = bare_router(1);
    let drop = Message::Turn(TurnCommands {
        turn: 2,
        slot: PlayerId(0),
        commands: vec![stop(2, 100)],
        schedule: vec![Scheduled {
            by: PlayerId(0),
            action: Action::Drop {
                slot: PlayerId(2),
                last_turn: 3,
                then: DropChoice::AiTakeover("x".into()),
            },
        }],
        digest: None,
    });
    assert_eq!(r.receive(PlayerId(0), drop), Ok(()));
    // The seat's own earlier records, with lower seqs, arrive later.
    assert_eq!(r.receive(PlayerId(2), rec(2, 2, vec![stop(2, 5)])), Ok(()));
    assert_eq!(r.receive(PlayerId(2), rec(3, 2, vec![stop(2, 6)])), Ok(()));
}

// ---------------------------------------------------------------------------
// NET-06: desync diagnosis. Section digests on the wire, then the offline
// bisect of the two peers' artefacts.
// ---------------------------------------------------------------------------

use sta_net::diagnose::{self, diagnose, DiagnoseError, Options, Report, Side};
use sta_net::proto::{SectionReport, MAX_SECTIONS};
use sta_sim::digest::SectionDigests;
use sta_sim::Sim;

/// Writes every peer's report, reads the files back and diagnoses peers `a`
/// and `b`, as the command line does.
fn diagnose_peers(r: &Room, dir: &str, a: usize, b: usize) -> (Report, [PathBuf; 2]) {
    let dir = tmp_dir(dir);
    let files: Vec<_> = [a, b]
        .iter()
        .map(|&i| r.peers[i].write_desync_report(&dir, &format!("seat{i}")).unwrap())
        .collect();
    let side = |f: &sta_net::lockstep::DesyncFiles| {
        let replay = diagnose::read_replay("replay", &std::fs::read(&f.replay).unwrap()).unwrap();
        let sim = Sim::from_setup(&replay.setup(r.db.clone()).unwrap());
        Side {
            label: f.replay.file_name().unwrap().to_string_lossy().into_owned(),
            replay,
            sim,
            sidecar: Some(std::fs::read_to_string(&f.sidecar).unwrap()),
        }
    };
    let report = diagnose(side(&files[0]), side(&files[1]), &Options::default()).unwrap();
    (report, [files[0].sidecar.clone(), files[1].sidecar.clone()])
}

#[test]
fn a_dropped_order_is_localised_to_tick_section_entity_and_command() {
    let (r, applied) = run_with_dropped_order(2, 1, 0, 0, 7);
    let unit = r.peers[0]
        .replay()
        .commands
        .iter()
        .find_map(|l| match &l.command.kind {
            CommandKind::Order(Order::Move { units, .. }) if l.tick == applied => Some(units[0]),
            _ => None,
        })
        .unwrap();
    let (report, sidecars) = diagnose_peers(&r, "diag-dropped", 0, 1);
    let d = report.divergence.as_ref().expect("the peers diverged");
    // The order applies at `applied`; its first effect is in the state at the next tick;
    // the routers notice at the next turn boundary.
    assert_eq!(d.tick, applied + 1);
    assert_eq!(d.detected_at, Some(applied + 3));
    assert_eq!(d.sections.iter().map(|s| s.section).collect::<Vec<_>>(), ["movement"]);
    let rows = &d.sections[0].rows;
    assert!(rows.iter().all(|x| x.entity == Some(unit)), "{rows:?}");
    // Peer 0 (a) heads for the order's destination (-400, 300); the victim (b) does not.
    let row = rows.iter().find(|x| x.field == "target").expect("the target differs");
    assert!(row.values[0].as_deref().is_some_and(|v| v.starts_with("-400.0000") && v.contains("300.0000 ")), "{row:?}");
    assert!(row.values[1].is_some(), "{row:?}");
    // The command that preceded it: only peer 0 applied the Move, in the last turns before.
    let only = &d.commands.only_in;
    assert!(only[1].is_empty());
    assert_eq!(only[0].len(), 1);
    assert_eq!((only[0][0].tick, only[0][0].player), (applied, 0));
    assert!(only[0][0].text.contains("Move"), "{:?}", only[0][0]);
    assert!(d.commands.window[0].iter().any(|c| c.tick == applied));
    assert!(!d.commands.window[1].iter().any(|c| c.tick == applied && c.player == 0 && c.text.contains("Move")));
    // Both sidecars named the same section, and the tool agrees with them.
    for (i, p) in sidecars.iter().enumerate() {
        let text = std::fs::read_to_string(p).unwrap();
        assert!(text.contains("section digests at that tick, seat 0: roster=0x"), "{text}");
        assert!(text.contains("section digests at that tick, seat 1: roster=0x"), "{text}");
        let other = 1 - i;
        assert!(text.contains(&format!("first differing sections, seat {i} against seat {other}: movement\n")), "{text}");
        let rep = report.reported[i].as_ref().unwrap();
        assert_eq!((rep.tick, rep.sections.as_slice()), (Some(applied + 3), ["movement".to_owned()].as_slice()));
        assert_eq!(diagnose::sidecar_agrees(rep, d), Some(true));
    }
    // Output is stable and carries the answer.
    let json = report.to_json();
    assert_eq!(json, report.to_json());
    assert!(json.contains("\"verdict\":\"diverged\""));
    assert!(json.contains(&format!("\"first_differing_tick\":{}", applied + 1)));
    assert!(json.contains(&format!("\"entity\":{},\"field\":\"target\"", unit.0)));
    let md = report.to_markdown();
    assert!(md.contains(&format!("Diverged at tick {}", applied + 1)) && md.contains("Section `movement`"), "{md}");
}

#[test]
fn the_peers_exchange_section_digests_for_the_mismatching_turn_only() {
    let (r, applied) = run_with_dropped_order(3, 2, 2, 5, 22);
    let first = applied + 3;
    for (i, p) in r.peers.iter().enumerate() {
        let ours = SectionDigests::NAMES.len();
        let at = p.router().sections_at(first);
        // The victim and the host halt on the mismatch and announce their sections;
        // every peer holds its own, and the ones announced.
        assert!(at.iter().any(|(s, d)| s.0 as usize == i && d.len() == ours), "peer {i}: {at:?}");
        assert!(at.len() >= 2, "peer {i} received no section digests: {at:?}");
        // Quiet turns before the mismatch were not announced.
        assert!(p.router().sections_at(first - 30).iter().all(|(s, _)| s.0 as usize == i));
    }
    // The victim's section digests differ from the others' in movement only.
    let victim = r.peers[2].router().sections_at(first);
    let host = r.peers[0].router().sections_at(first);
    let get = |v: &[(PlayerId, Vec<u64>)], s: u8| v.iter().find(|(p, _)| p.0 == s).map(|(_, d)| d.clone());
    let (a, b) = (get(&host, 0).unwrap(), get(&victim, 2).unwrap());
    assert_eq!(get(&victim, 0), Some(a.clone()), "the victim holds the host's digests");
    let differing: Vec<_> = (0..a.len()).filter(|&i| a[i] != b[i]).map(|i| SectionDigests::NAMES[i]).collect();
    assert_eq!(differing, ["movement"]);
}

#[test]
fn a_local_only_state_edit_is_localised_to_tick_section_and_entity() {
    let mut r = room(2, cfg(3, 2));
    r.spawn_forces(2);
    r.round_robin_to(90, |_, _| {});
    let unit = r.peers[1].sim().view().units.iter().find(|u| u.owner == PlayerId(1)).unwrap().id;
    r.peers[1].inject(Fault::Inject(Command {
        player: PlayerId::SCRIPT,
        seq: 9_999,
        kind: CommandKind::Edit(Edit::Damage(unit, Fixed::from_int(7))),
    }));
    for _ in 0..60 {
        r.net.advance(1);
        for p in &mut r.peers {
            p.step();
        }
    }
    let applied = applied_tick(&r.peers[1], PlayerId::SCRIPT, 9_999).expect("applied on peer 1");
    assert_eq!(applied_tick(&r.peers[0], PlayerId::SCRIPT, 9_999), None);
    let (report, _) = diagnose_peers(&r, "diag-edit", 0, 1);
    let d = report.divergence.as_ref().expect("diverged");
    assert_eq!(d.tick, applied + 1);
    assert_eq!(d.sections.iter().map(|s| s.section).collect::<Vec<_>>(), ["combat"]);
    let rows = &d.sections[0].rows;
    assert!(!rows.is_empty() && rows.iter().all(|x| x.entity == Some(unit)), "{rows:?}");
    assert!(rows.iter().any(|x| x.field == "hull" || x.field == "shields"), "{rows:?}");
    // Only the edited peer applied the command.
    assert_eq!(d.commands.only_in[0], []);
    assert_eq!((d.commands.only_in[1][0].tick, d.commands.only_in[1][0].seq), (applied, 9_999));
    assert!(d.commands.only_in[1][0].text.contains("Damage"));
    // Both sidecars agree with the replays about the section at the turn that noticed.
    assert!(report.reported.iter().flatten().all(|rep| diagnose::sidecar_agrees(rep, d) == Some(true)), "{:?}", report.reported);
}

#[test]
fn identical_peers_report_no_divergence() {
    let mut r = room(2, cfg(3, 2));
    r.spawn_forces(3);
    r.run_to(300, true);
    let (report, _) = diagnose_peers(&r, "diag-same", 0, 1);
    assert!(!report.diverged() && report.divergence.is_none());
    assert_eq!(report.compared_through, r.peers[0].replay().end_tick);
    assert!(report.setup_differences.is_empty());
    assert!(report.to_json().contains("\"verdict\":\"identical\""));
    assert!(report.to_markdown().contains("No divergence"));
    // The same replay twice is identical too, and the tool is repeatable.
    let again = diagnose_peers(&r, "diag-same", 0, 1).0;
    assert_eq!(report.to_json(), again.to_json());
}

#[test]
fn damaged_artefacts_are_errors_not_panics() {
    let mut r = room(2, cfg(3, 2));
    r.spawn_forces(2);
    r.run_to(60, true);
    let bytes = r.peers[0].replay().encode();
    let bad = |b: &[u8]| matches!(diagnose::read_replay("x.star", b), Err(DiagnoseError::Replay { .. }));
    // Every truncation (sampled), garbage, wrong magic, and trailing bytes.
    for n in (0..bytes.len()).step_by(bytes.len() / 97 + 1) {
        assert!(bad(&bytes[..n]), "truncated to {n}");
    }
    assert!(bad(&[]) && bad(b"not a replay at all") && bad(&bytes[1..]));
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(bad(&trailing));
    let mut state = 12345u64;
    for _ in 0..50 {
        let mut flipped = bytes.clone();
        for _ in 0..4 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let at = (state >> 33) as usize % flipped.len();
            flipped[at] ^= 1 << (state & 7);
        }
        // Either rejected or readable: never a panic.
        let _ = diagnose::read_replay("x.star", &flipped);
    }
    let msg = diagnose::read_replay("seat0.star", &bytes[..bytes.len() / 2]).err().unwrap().to_string();
    assert!(msg.starts_with("seat0.star: not a readable replay"), "{msg}");

    // A replay whose recorded digest is not what its commands give: not the game that peer ran.
    let mut tampered = Replay::decode(&bytes).unwrap();
    tampered.digests[2].1 ^= 1;
    let sim = || Sim::from_setup(&tampered.setup(r.db.clone()).unwrap());
    let fine = Replay::decode(&bytes).unwrap();
    let side = |label: &str, replay: &Replay, sim: Sim| Side { label: label.into(), replay: replay.clone(), sim, sidecar: None };
    let err = diagnose(side("good", &fine, sim()), side("bad", &tampered, sim()), &Options::default()).err().unwrap();
    match &err {
        DiagnoseError::NotReproducible { label, tick, .. } => assert_eq!((label.as_str(), *tick), ("bad", tampered.digests[2].0)),
        other => panic!("{other:?}"),
    }
    assert!(err.to_string().contains("not the game that peer ran"), "{err}");
    // A sim that is not at tick 0.
    let mut late = sim();
    late.step(&[]);
    assert!(matches!(
        diagnose(side("a", &fine, late), side("b", &fine, sim()), &Options::default()),
        Err(DiagnoseError::NotAtStart { tick: 1, .. })
    ));
}

#[test]
fn section_reports_from_a_peer_are_bounded() {
    let mut r = bare_router(0);
    let report = |tick: Tick, n: usize| Message::Sections(SectionReport { tick, digests: vec![7; n] });
    // Wrong size, empty, and off a turn boundary (3 ticks per turn) are refused.
    assert!(matches!(r.receive(PlayerId(1), report(3, MAX_SECTIONS + 1)), Err(RouterError::Proto(_))));
    assert!(matches!(r.receive(PlayerId(1), report(3, 0)), Err(RouterError::Proto(_))));
    assert!(matches!(r.receive(PlayerId(1), report(4, 5)), Err(RouterError::Proto(_))));
    // From a seat that is not at the table: refused.
    assert!(r.receive(PlayerId(7), report(3, 5)).is_err());
    // A flood of distinct ticks keeps the newest DIGESTS_KEPT only.
    for k in 0..1000u64 {
        r.receive(PlayerId(1), report(3 * k, MAX_SECTIONS)).unwrap();
    }
    let kept = (0..1000u64).filter(|&k| !r.sections_at(3 * k).is_empty()).count();
    assert_eq!(kept, sta_net::lockstep::DIGESTS_KEPT);
    assert!(!r.sections_at(3 * 999).is_empty() && r.sections_at(0).is_empty());
}

#[test]
fn a_peer_records_how_its_seats_got_their_starting_forces() {
    use sta_sim::replay::StartRec;
    let mut r = room(2, cfg(3, 2));
    assert_eq!(r.peers[0].replay().starts, StartRec::Bare);
    r.peers[0].set_starts(StartRec::Races { loadouts: 0xfeed });
    assert_eq!(r.peers[0].replay().starts, StartRec::Races { loadouts: 0xfeed });
    // It survives the replay's own encoding, which is what a desync report is.
    let back = Replay::decode(&r.peers[0].replay().encode()).expect("a replay");
    assert_eq!(back.starts, StartRec::Races { loadouts: 0xfeed });
}
