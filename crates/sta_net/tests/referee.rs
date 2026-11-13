//! NET-05: the referee (NET_DECISIONS N5). A headless seatless peer re-simulates
//! the match from `Start` and the seats' turn records, checks every seat's
//! digests against its own, names the first diverging tick, section and seat,
//! serves the resync of a diverged seat, and writes the artefacts
//! `sta-bench desync` reads.
//!
//! In-process over `LoopbackNet`, and over the real relay and TCP on
//! 127.0.0.1 (ports chosen by the OS). No assertion depends on a sleep or on
//! two independent connections being ordered: every wait is a loop on a real
//! condition with a deadline.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sta_content::ContentDb;
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Tick, Vec3};
use sta_net::diagnose::{self, diagnose, Options, Side};
use sta_net::lockstep::{
    Destination, Fault, Incoming, LoopbackNet, Peer, StepOutcome, Transport,
    TransportError, MAX_EVENTS,
};
use sta_net::proto::{
    DesyncNotice, EngineId, Hello, MapRef, Message, Occupant, PackInfo, Refusal, ResyncRequest,
    RuleEntry, RulesDigest, RulesIdentity, SectionReport, Seat, SenderKind, SnapshotRefusal, Start,
    TickDigest, TurnConfig,
};
use sta_net::referee::{
    admit_referee, await_start, judge, EndReason, FindingKind, Referee, RefereeLimits,
    RefereePolicy, RefereeStep, Verdict,
};
use sta_net::resync::{ResyncEvent, ResyncLimits};
use sta_net::transport::{ClientOptions, NetTransport, Relay, RelayConfig};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::{Controller, GameOptions};
use sta_sim::{Sim, SimEvent, WorldSource, WorldView};

const WAIT: Duration = Duration::from_secs(120);

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn engine() -> EngineId {
    EngineId {
        name: "net05".into(),
        version: "1".into(),
        build: "test".into(),
    }
}

fn tmp_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The computer player of the tests: every 15 ticks it orders one of its
/// units somewhere. It runs on the host only, like the real controllers.
struct Scripted {
    seat: PlayerId,
    seq: u32,
    sent: usize,
}

impl Controller for Scripted {
    fn think(&mut self, view: &WorldView, _events: &[SimEvent], out: &mut Vec<Command>) {
        if !view.tick.is_multiple_of(15) {
            return;
        }
        let mine: Vec<EntityId> = view.units.iter().filter(|u| u.owner == self.seat).map(|u| u.id).collect();
        if mine.is_empty() {
            return;
        }
        let k = (view.tick / 15) as usize;
        out.push(Command {
            player: self.seat,
            seq: self.seq,
            kind: CommandKind::Order(Order::Move {
                units: vec![mine[k % mine.len()]],
                to: Vec3::from_f32([((k * 53) % 400) as f32 - 200.0, 0.0, ((k * 29) % 400) as f32 - 200.0]),
                queue: false,
            }),
        });
        self.seq += 1;
        self.sent += 1;
    }
}

fn start_of(players: u8, ai: bool, db: &ContentDb) -> Start {
    let seat = |slot: u8, occupant: Occupant| Seat {
        slot: PlayerId(slot),
        team: TeamId(slot),
        race: "fx".into(),
        occupant,
        name: format!("p{slot}"),
        lineage: db.lineage().to_owned(),
        packs: Vec::<PackInfo>::new(),
    };
    let mut seats: Vec<Seat> = (0..players).map(|s| seat(s, Occupant::Peer)).collect();
    if ai {
        seats.push(seat(players, Occupant::Ai("scripted".into())));
    }
    Start {
        seed: 0x5eed,
        map: MapRef { name: "fx_two_bases".into(), digest: 0 },
        options: GameOptions::default(),
        seats,
        host: PlayerId(0),
        turn: TurnConfig { ticks_per_turn: 3, delay_turns: 2, speed_percent: 100 },
        rules: RulesIdentity::digest(db.digest()),
    }
}

fn fast_limits() -> ResyncLimits {
    ResyncLimits { min_attempt_interval: Duration::from_millis(1), ..ResyncLimits::default() }
}

fn fast_referee_limits() -> RefereeLimits {
    RefereeLimits {
        release_interval: Duration::from_millis(5),
        report_delay: Duration::from_millis(1),
        ..RefereeLimits::default()
    }
}

struct Room {
    db: Arc<ContentDb>,
    peers: Vec<Peer>,
    referee: Referee,
    net: Option<LoopbackNet>,
    ai: Option<Scripted>,
    last_order: Vec<Option<Tick>>,
    last_think: Option<Tick>,
    events: Vec<SimEvent>,
    // Keeps a relay alive for the room's life.
    _relay: Option<Relay>,
}

type Wrap<'a> = &'a mut dyn FnMut(PlayerId, Box<dyn Transport>) -> Box<dyn Transport>;

impl Room {
    /// `players` seats (0 is the host), an AI seat after them when `ai`, and
    /// the referee in the next slot, over the in-process loopback.
    fn new(players: u8, ai: bool) -> Room {
        Room::build(players, ai, RefereePolicy::default(), &mut |_, t| t)
    }

    fn build(players: u8, ai: bool, policy: RefereePolicy, wrap: Wrap<'_>) -> Room {
        let db = Arc::new(sta_content::fake::db());
        let rslot = PlayerId(players + u8::from(ai));
        let mut ids: Vec<PlayerId> = (0..players).map(PlayerId).collect();
        ids.push(rslot);
        let (net, links) = LoopbackNet::new(&ids);
        let transports: Vec<(PlayerId, Box<dyn Transport>)> =
            links.into_iter().map(|l| (l.seat(), Box::new(l) as Box<dyn Transport>)).collect();
        Room::assemble(players, ai, policy, db, transports, Some(net), None, wrap)
    }

    #[allow(clippy::too_many_arguments)]
    fn assemble(
        players: u8,
        ai: bool,
        policy: RefereePolicy,
        db: Arc<ContentDb>,
        transports: Vec<(PlayerId, Box<dyn Transport>)>,
        net: Option<LoopbackNet>,
        relay: Option<Relay>,
        wrap: Wrap<'_>,
    ) -> Room {
        let map = Arc::new(sta_content::fake::two_bases());
        let start = start_of(players, ai, &db);
        let rslot = PlayerId(players + u8::from(ai));
        let mut peers = Vec::new();
        let mut referee = None;
        for (seat, t) in transports {
            let t = wrap(seat, t);
            if seat == rslot {
                referee = Some(
                    Referee::new(&start, seat, db.clone(), map.clone(), t, policy, fast_referee_limits())
                        .expect("a referee"),
                );
            } else {
                let mut p = Peer::start(&start, seat, db.clone(), map.clone(), t).expect("a peer");
                p.set_resync_limits(fast_limits());
                peers.push(p);
            }
        }
        let mut referee = referee.expect("the referee's transport");
        referee.peer_mut().set_resync_limits(fast_limits());
        peers[0].router_mut().seat_referee(Some(rslot)).expect("seat the referee");
        let ai = ai.then_some(Scripted { seat: PlayerId(players), seq: 0, sent: 0 });
        let mut room = Room {
            db,
            last_order: vec![None; peers.len()],
            peers,
            referee,
            net,
            ai,
            last_think: None,
            events: Vec::new(),
            _relay: relay,
        };
        room.spawn_forces(players);
        room
    }

    fn spawn_forces(&mut self, players: u8) {
        let cruiser = self.db.key("fx_cruiser").unwrap();
        let frigate = self.db.key("fx_frigate").unwrap();
        let seats = players + u8::from(self.ai.is_some());
        let mut seq = 0;
        for p in 0..i32::from(seats) {
            for k in 0..3 {
                self.peers[0].submit(Command {
                    player: PlayerId::SCRIPT,
                    seq,
                    kind: CommandKind::Edit(Edit::Spawn {
                        odf: if k % 2 == 0 { cruiser } else { frigate },
                        player: PlayerId(p as u8),
                        pos: Vec3::from_f32([
                            if p % 2 == 0 { -150.0 + 40.0 * p as f32 } else { 150.0 - 40.0 * p as f32 },
                            0.0,
                            (k * 70 - 100) as f32,
                        ]),
                        heading: (Fixed::ZERO, Fixed::ONE),
                    }),
                });
                seq += 1;
            }
        }
    }

    /// One step of peer `i`, with the orders a player (or, on the host, the
    /// computer player) would give.
    fn step_peer(&mut self, i: usize) -> StepOutcome {
        let me = PlayerId(i as u8);
        let p = &mut self.peers[i];
        if i == 0
            && let Some(ai) = self.ai.as_mut()
            && self.last_think != Some(p.tick())
        {
            self.last_think = Some(p.tick());
            let view = p.sim().view();
            let mut out = Vec::new();
            ai.think(&view, &self.events, &mut out);
            for c in out {
                p.submit(c);
            }
        }
        let t = p.tick();
        if t.is_multiple_of(9) && self.last_order[i] != Some(t) {
            self.last_order[i] = Some(t);
            let mine: Vec<EntityId> = p.sim().view().units.iter().filter(|u| u.owner == me).map(|u| u.id).collect();
            if !mine.is_empty() {
                let k = (t / 9) as usize;
                let seq = p.router().last_seq(me).map_or(0, |s| s + 1);
                p.submit(Command {
                    player: me,
                    seq,
                    kind: CommandKind::Order(Order::Move {
                        units: vec![mine[k % mine.len()]],
                        to: Vec3::from_f32([((k * 37) % 500) as f32 - 250.0, 0.0, ((k * 91) % 500) as f32 - 250.0]),
                        queue: k.is_multiple_of(3),
                    }),
                });
            }
        }
        let out = p.step();
        if i == 0
            && let StepOutcome::Ran { events, .. } = &out
        {
            self.events.clone_from(events);
        }
        out
    }

    /// Steps every peer and the referee once; whether any of them ran.
    fn round(&mut self) -> bool {
        let mut progressed = false;
        for i in 0..self.peers.len() {
            progressed |= matches!(self.step_peer(i), StepOutcome::Ran { .. });
        }
        progressed |= matches!(self.referee.step(), RefereeStep::Ran(_));
        if !progressed {
            if let Some(n) = &self.net {
                n.advance(1);
            } else {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        progressed
    }

    fn run_until(&mut self, what: &str, done: impl Fn(&Room) -> bool) {
        let end = Instant::now() + WAIT;
        while !done(self) {
            self.round();
            if Instant::now() >= end {
                let mut extra = String::new();
                for p in self.peers.iter_mut() {
                    let ev: Vec<_> = p.router_mut().take_events().into_iter().take(12).collect();
                    extra.push_str(&format!("\n seat {} router events {ev:?}; resync {:?}", p.router().me().0, p.take_resync_events()));
                }
                panic!("timed out waiting for {what}: {}{extra}", self.status());
            }
        }
    }

    fn status(&self) -> String {
        let mut s: Vec<String> = self
            .peers
            .iter()
            .map(|p| {
                format!(
                    "seat {}: tick {} next turn {} waiting {:?} halted {}",
                    p.router().me().0,
                    p.tick(),
                    p.router().next_turn(),
                    p.router().waiting_on(),
                    p.router().halted()
                )
            })
            .collect();
        s.push(format!(
            "referee: tick {} seated {} findings {:?}",
            self.referee.peer().tick(),
            self.referee.peer().router().seated(),
            self.referee.findings()
        ));
        s.join("; ")
    }

    fn all_at(&self, tick: Tick) -> bool {
        self.peers.iter().all(|p| p.tick() >= tick) && self.referee.peer().tick() >= tick
    }

    fn any_halted(&self) -> bool {
        self.peers.iter().any(|p| p.router().halted())
    }

    /// The referee's digests agree with every seat's replay at every tick
    /// both recorded.
    fn assert_referee_agrees_with_all_seats(&self) {
        let theirs: BTreeMap<Tick, u64> = self.referee.peer().replay().digests.iter().copied().collect();
        assert!(theirs.len() > 20, "the referee recorded too few turns: {}", theirs.len());
        for p in &self.peers {
            let mut common = 0;
            for (t, d) in &p.replay().digests {
                // A resynced peer's replay may differ before its snapshot.
                if p.resyncs().iter().any(|n| *t < n.tick) {
                    continue;
                }
                if let Some(r) = theirs.get(t) {
                    assert_eq!(r, d, "seat {} at tick {t}", p.router().me().0);
                    common += 1;
                }
            }
            assert!(common > 20, "seat {} shares {common} ticks with the referee", p.router().me().0);
        }
    }

    fn damage(&mut self, victim: usize) -> Tick {
        let unit = self.peers[victim]
            .sim()
            .view()
            .units
            .iter()
            .find(|u| u.owner == PlayerId(victim as u8))
            .unwrap()
            .id;
        let at = self.peers[victim].tick();
        self.peers[victim].inject(Fault::Inject(Command {
            player: PlayerId::SCRIPT,
            seq: 9_999,
            kind: CommandKind::Edit(Edit::Damage(unit, Fixed::from_int(7))),
        }));
        at
    }
}

/// The first turn boundary whose digest shows a change applied at `tick`.
fn first_visible(tick: Tick) -> Tick {
    (tick + 1).div_ceil(3) * 3
}

// ---------------------------------------------------------------------------
// A clean match is verified
// ---------------------------------------------------------------------------

fn clean_match(players: u8) {
    let mut r = Room::new(players, true);
    r.run_until("tick 900", |r| r.all_at(900));
    assert!(!r.any_halted(), "{}", r.status());
    assert!(r.referee.findings().is_empty(), "{:?}", r.referee.findings());
    assert_eq!(r.referee.protocol_errors(), 0);
    assert!(r.referee.peer().router().seated());
    // The computer player played, and its commands reached the referee.
    assert!(r.ai.as_ref().unwrap().sent > 5);
    let ai = PlayerId(players);
    assert!(r.referee.peer().replay().commands.iter().any(|c| c.command.player == ai), "the AI's commands never reached the referee");
    r.assert_referee_agrees_with_all_seats();
    // The referee holds every seat's digest for a recent tick, all equal.
    let t = r.referee.peer().tick() - 30;
    let t = t - t % 3;
    let known = r.referee.peer().router().digests_at(t);
    assert_eq!(known.len(), usize::from(players) + 1, "{known:?}");
    assert!(known.iter().all(|(_, d)| *d == known[0].1), "{known:?}");
}

#[test]
fn a_clean_two_peer_match_with_an_ai_seat_is_verified() {
    clean_match(2);
}

#[test]
fn a_clean_three_peer_match_with_an_ai_seat_is_verified() {
    clean_match(3);
}

// ---------------------------------------------------------------------------
// A divergence is attributed, and the seat is resynced from the referee
// ---------------------------------------------------------------------------

/// Runs until the referee names a seat with its sections.
fn until_named(r: &mut Room, seat: u8) {
    r.run_until("the referee to name the seat and its sections", |r| {
        r.referee
            .findings()
            .iter()
            .any(|f| f.seat == PlayerId(seat) && matches!(f.kind, FindingKind::Diverged { .. }))
    });
}

fn until_repaired(r: &mut Room, resynced: &[usize]) {
    r.run_until("the resync", |r| {
        !r.any_halted() && resynced.iter().all(|&i| !r.peers[i].resyncs().is_empty())
    });
}

#[test]
fn an_injected_command_is_attributed_and_the_seat_is_resynced_from_the_referee() {
    let mut r = Room::new(3, true);
    r.run_until("tick 90", |r| r.all_at(90));
    let at = r.damage(2);
    until_named(&mut r, 2);
    let f = r.referee.first_finding().unwrap();
    assert_eq!(f.seat, PlayerId(2));
    assert!(!f.is_host);
    assert_eq!(f.tick, first_visible(at), "applied at {at}");
    assert_eq!(f.kind, FindingKind::Diverged { sections: vec!["combat"] });
    // Only that seat differs.
    assert_eq!(r.referee.findings().len(), 1, "{:?}", r.referee.findings());

    // The artefacts: written, read back, and consumed the way sta-bench does.
    let dir = tmp_dir("referee-injected");
    let files = r.referee.write_report(&dir, "referee").unwrap();
    let victim = r.peers[2].write_desync_report(&dir, "seat2").unwrap();
    let side = |f: &sta_net::lockstep::DesyncFiles| {
        let replay = diagnose::read_replay("replay", &std::fs::read(&f.replay).unwrap()).unwrap();
        let sim = Sim::from_setup(&replay.setup(r.db.clone()).unwrap());
        Side { label: f.replay.display().to_string(), replay, sim, sidecar: Some(std::fs::read_to_string(&f.sidecar).unwrap()) }
    };
    let report = diagnose(side(&files), side(&victim), &Options::default()).unwrap();
    let d = report.divergence.as_ref().expect("the replays diverge");
    assert_eq!(d.tick, at + 1);
    assert_eq!(d.detected_at, Some(f.tick));
    assert_eq!(d.sections_at_detection, ["combat"]);
    let sidecar = std::fs::read_to_string(&files.sidecar).unwrap();
    let reported = diagnose::parse_sidecar(&sidecar);
    assert_eq!(reported.tick, Some(f.tick), "{sidecar}");
    assert_eq!(reported.sections, ["combat"], "{sidecar}");
    assert_eq!(diagnose::sidecar_agrees(&reported, d), Some(true));
    assert!(sidecar.contains("verdict: seat 2 diverged at tick"), "{sidecar}");
    assert!(sidecar.contains("referee: yes"), "{sidecar}");

    // The diverged seat is resynced from the referee, not the host, and the
    // match goes on in step with it.
    until_repaired(&mut r, &[2]);
    assert!(r.peers[0].resyncs().is_empty() && r.peers[1].resyncs().is_empty());
    let host_events = r.peers[0].take_resync_events();
    assert!(!host_events.iter().any(|e| matches!(e, ResyncEvent::Sent { .. })), "the host served: {host_events:?}");
    assert!(
        host_events.iter().any(|e| matches!(e, ResyncEvent::Resynced { slot, last: true, .. } if *slot == PlayerId(2))),
        "every player is told: {host_events:?}"
    );
    let t = r.peers.iter().map(Peer::tick).max().unwrap();
    r.run_until("1500 more ticks", |r| r.all_at(t + 1500));
    for p in &r.peers[1..] {
        assert!(!p.router().halted());
    }
    r.assert_referee_agrees_with_all_seats();
    assert!(!r.any_halted());
}

#[test]
fn a_dropped_command_is_attributed_to_the_seat_that_dropped_it() {
    let mut r = Room::new(2, true);
    r.run_until("tick 90", |r| r.all_at(90));
    // Seat 1 never applies the host's next-but-one order for the AI seat.
    let host = &r.peers[0];
    let ai = PlayerId(2);
    let next = host.router().last_seq(ai).map_or(0, |s| s + 2);
    r.peers[1].inject(Fault::DropCommand { player: ai, seq: next });
    until_named(&mut r, 1);
    let f = r.referee.first_finding().unwrap();
    assert_eq!(f.seat, PlayerId(1));
    match &f.kind {
        FindingKind::Diverged { sections } => assert!(sections.contains(&"movement"), "{sections:?}"),
        k => panic!("{k:?}"),
    }
    let dir = tmp_dir("referee-dropped");
    let files = r.referee.write_report(&dir, "referee").unwrap();
    let victim = r.peers[1].write_desync_report(&dir, "seat1").unwrap();
    let side = |f: &sta_net::lockstep::DesyncFiles| {
        let replay = diagnose::read_replay("replay", &std::fs::read(&f.replay).unwrap()).unwrap();
        let sim = Sim::from_setup(&replay.setup(r.db.clone()).unwrap());
        Side { label: f.replay.display().to_string(), replay, sim, sidecar: Some(std::fs::read_to_string(&f.sidecar).unwrap()) }
    };
    let report = diagnose(side(&files), side(&victim), &Options::default()).unwrap();
    assert_eq!(report.divergence.as_ref().unwrap().detected_at, Some(f.tick));
    until_repaired(&mut r, &[1]);
    let t = r.peers.iter().map(Peer::tick).max().unwrap();
    r.run_until("1200 more ticks", |r| r.all_at(t + 1200));
    r.assert_referee_agrees_with_all_seats();
}

// ---------------------------------------------------------------------------
// The host is the one that diverged
// ---------------------------------------------------------------------------

#[test]
fn a_diverged_host_ends_the_match_with_reports_by_default() {
    let mut r = Room::new(3, true);
    r.run_until("tick 90", |r| r.all_at(90));
    let at = r.damage(0);
    r.run_until("the host's request to be refused", |r| r.peers[0].resync_failed());
    until_named(&mut r, 0);
    let f = r.referee.first_finding().unwrap();
    assert_eq!(f.seat, PlayerId(0));
    assert!(f.is_host);
    assert_eq!(f.tick, first_visible(at));
    assert_eq!(f.kind, FindingKind::Diverged { sections: vec!["combat"] });
    // Nobody else differs, and nobody was resynced: the match is stopped.
    assert_eq!(r.referee.findings().len(), 1);
    assert!(r.peers.iter().all(|p| p.resyncs().is_empty()));
    let events = r.peers[0].take_resync_events();
    assert!(
        events.iter().any(|e| matches!(e, ResyncEvent::Failed(sta_net::resync::CatchUpError::Refused(SnapshotRefusal::HostEndsMatch)))),
        "{events:?}"
    );
    assert!(r.peers.iter().all(|p| p.router().halted()), "{}", r.status());
    let ticks: Vec<Tick> = r.peers.iter().map(Peer::tick).collect();
    for _ in 0..200 {
        r.round();
    }
    assert_eq!(r.peers.iter().map(Peer::tick).collect::<Vec<_>>(), ticks, "the stopped match ran on");
    // Reports are written for everyone to read.
    let dir = tmp_dir("referee-host-ends");
    let files = r.referee.write_report(&dir, "referee").unwrap();
    let side = std::fs::read_to_string(&files.sidecar).unwrap();
    assert!(side.contains("seat 0 (the host) diverged"), "{side}");
}

#[test]
fn a_diverged_host_is_resynced_with_its_ai_still_running_when_the_policy_allows() {
    let policy = RefereePolicy { resync_host: true };
    let mut r = Room::build(3, true, policy, &mut |_, t| t);
    r.run_until("tick 90", |r| r.all_at(90));
    r.damage(0);
    until_repaired(&mut r, &[0]);
    assert!(r.peers[1].resyncs().is_empty() && r.peers[2].resyncs().is_empty());
    let sent_before = r.ai.as_ref().unwrap().sent;
    let t = r.peers.iter().map(Peer::tick).max().unwrap();
    r.run_until("1500 more ticks", |r| r.all_at(t + 1500));
    // The host's controller carried on: its orders went out and everyone,
    // the referee included, applied them.
    let ai = PlayerId(3);
    assert!(r.ai.as_ref().unwrap().sent > sent_before + 20);
    let after = r
        .referee
        .peer()
        .replay()
        .commands
        .iter()
        .filter(|c| c.command.player == ai && c.tick > t)
        .count();
    assert!(after > 20, "the AI's orders after the resync reached the referee: {after}");
    r.assert_referee_agrees_with_all_seats();
    assert!(r.referee.findings().iter().all(|f| f.is_host));
    assert!(!r.any_halted());
}

// ---------------------------------------------------------------------------
// Hostile peers
// ---------------------------------------------------------------------------

/// Rewrites the digest of the records its seat sends from `from` on.
struct LyingDigest {
    inner: Box<dyn Transport>,
    from: Tick,
}

impl Transport for LyingDigest {
    fn send(&mut self, to: Destination, msg: &Message) -> Result<(), TransportError> {
        if let Message::Turn(t) = msg
            && let Some(d) = t.digest
            && d.tick >= self.from
        {
            let mut lie = t.clone();
            lie.digest = Some(TickDigest { tick: d.tick, digest: d.digest ^ 0xdead_beef });
            return self.inner.send(to, &Message::Turn(lie));
        }
        self.inner.send(to, msg)
    }
    fn poll(&mut self) -> Result<Option<Incoming>, TransportError> {
        self.inner.poll()
    }
}

#[test]
fn a_peer_that_lies_about_its_digest_is_named_and_nobody_else_is() {
    let mut r = Room::build(3, true, RefereePolicy::default(), &mut |seat, t| {
        if seat == PlayerId(1) {
            Box::new(LyingDigest { inner: t, from: 60 })
        } else {
            t
        }
    });
    r.run_until("the referee to name the liar", |r| r.referee.findings().iter().any(|f| f.seat == PlayerId(1)));
    let f = r.referee.first_finding().unwrap();
    assert_eq!(f.seat, PlayerId(1));
    assert_eq!(f.tick, 60, "{f:?}");
    assert_eq!(f.reported, f.reference ^ 0xdead_beef);
    // The liar's sim is fine: it never asks for a state, and nothing is
    // handed out. The honest seats are not named.
    assert!(r.referee.findings().iter().all(|x| x.seat == PlayerId(1)), "{:?}", r.referee.findings());
    for _ in 0..400 {
        r.round();
    }
    assert!(r.peers.iter().all(|p| p.resyncs().is_empty()));
    assert!(r.referee.findings().iter().all(|x| x.seat == PlayerId(1)));
    // It sent no section digests, which a seat that really differs would
    // have: the verdict says so instead of naming sections.
    assert!(
        matches!(r.referee.first_finding().unwrap().kind, FindingKind::SectionsUnknown | FindingKind::MisreportedDigest),
        "{:?}",
        r.referee.first_finding()
    );
}

/// A seat that floods the room with well-formed messages that are no good:
/// section reports, claims of a desync that did not happen, resync requests.
struct Flooder {
    inner: Box<dyn Transport>,
    per_record: u32,
    last_tick: Tick,
    sent: u64,
}

impl Transport for Flooder {
    fn send(&mut self, to: Destination, msg: &Message) -> Result<(), TransportError> {
        if let Message::Turn(t) = msg {
            if let Some(d) = t.digest {
                self.last_tick = d.tick;
            }
            for k in 0..self.per_record {
                let tick = self.last_tick;
                let junk = match k % 4 {
                    0 => Message::Sections(SectionReport { tick: (u64::from(k) * 3) % 3000, digests: vec![u64::from(k); 5] }),
                    1 => Message::Desync(DesyncNotice {
                        slot: PlayerId(0),
                        ours: TickDigest { tick, digest: 1 },
                        theirs: TickDigest { tick, digest: 2 },
                    }),
                    2 => Message::ResyncRequest(ResyncRequest { tick, digest: u64::from(k), position: k }),
                    _ => Message::Reference(TickDigest { tick, digest: 7 }),
                };
                self.sent += 1;
                let _ = self.inner.send(Destination::All, &junk);
            }
        }
        self.inner.send(to, msg)
    }
    fn poll(&mut self) -> Result<Option<Incoming>, TransportError> {
        self.inner.poll()
    }
}

#[test]
fn a_seat_flooding_junk_does_not_stop_the_match_or_the_referee() {
    let mut r = Room::build(3, true, RefereePolicy::default(), &mut |seat, t| {
        if seat == PlayerId(2) {
            Box::new(Flooder { inner: t, per_record: 40, last_tick: 0, sent: 0 })
        } else {
            t
        }
    });
    // The honest seats and the referee play on in step, though every claim
    // of a desync stops them for a moment.
    r.run_until("tick 600", |r| r.all_at(600));
    assert!(r.referee.false_claims(PlayerId(2)) > 0, "the false claims were counted");
    assert_eq!(r.referee.false_claims(PlayerId(0)), 0);
    assert_eq!(r.referee.false_claims(PlayerId(1)), 0);
    // Only the flooder is ever named; nobody was resynced to a bogus state.
    assert!(r.referee.findings().iter().all(|f| f.seat == PlayerId(2)), "{:?}", r.referee.findings());
    assert!(r.peers.iter().all(|p| p.resyncs().is_empty()));
    assert!(r.referee.protocol_errors() > 0, "the junk was refused, not applied");
    // What the referee keeps is bounded whatever arrives.
    let router = r.referee.peer_mut().router_mut();
    assert!(router.divergences().len() <= sta_net::lockstep::MAX_REFEREE_FINDINGS);
    assert!(router.take_claims().len() <= sta_net::lockstep::MAX_REFEREE_FINDINGS);
    assert!(router.take_events().len() <= MAX_EVENTS + 64);
}

#[test]
fn a_flood_of_requests_is_held_within_the_inbound_bound_and_answered_sparingly() {
    let mut r = Room::new(2, false);
    r.run_until("tick 60", |r| r.all_at(60));
    // 50,000 resync requests and section reports from a seat, delivered at once.
    let router = r.referee.peer_mut().router_mut();
    for k in 0..50_000u64 {
        let _ = router.receive(
            PlayerId(1),
            Message::ResyncRequest(ResyncRequest { tick: 3 * (k % 20), digest: k, position: k as u32 }),
        );
        let _ = router.receive(
            PlayerId(1),
            Message::Sections(SectionReport { tick: 3 * (k % 1000), digests: vec![k; 16] }),
        );
    }
    let inbound = router.take_inbound();
    assert!(inbound.len() <= sta_net::lockstep::MAX_INBOUND, "{}", inbound.len());
    assert!(inbound.len() < 50_000);
    // Still alive and verifying.
    r.run_until("tick 300", |r| r.all_at(300));
    assert!(r.referee.findings().is_empty());
}

#[test]
fn a_seat_that_breaks_the_stream_is_faulted_and_the_referee_stays_up() {
    let mut r = Room::new(2, false);
    r.run_until("tick 60", |r| r.all_at(60));
    let router = r.referee.peer_mut().router_mut();
    let bad = |turn: u32, slot: u8| {
        Message::Turn(sta_net::proto::TurnCommands {
            turn,
            slot: PlayerId(slot),
            commands: vec![],
            schedule: vec![],
            digest: None,
        })
    };
    // Out of order, from another seat's slot, a record far ahead: refused.
    assert!(router.receive(PlayerId(1), bad(100_000, 1)).is_err());
    assert!(router.receive(PlayerId(1), bad(1, 0)).is_err());
    assert!(router.receive(PlayerId(0), bad(7, 1)).is_err());
    // Messages a seat may not send, or that name no seat.
    assert!(router.receive(PlayerId(1), Message::Reference(TickDigest { tick: 3, digest: 1 })).is_err());
    assert!(router.receive(PlayerId(1), Message::RefereeSeated { slot: Some(PlayerId(1)) }).is_err());
    assert!(router.receive(PlayerId(1), Message::SnapshotRefused(SnapshotRefusal::Busy)).is_err());
    assert!(router.receive(PlayerId(6), bad(0, 6)).is_err());
    // It goes on without panic.
    for _ in 0..50 {
        r.round();
    }
    assert!(r.referee.peer().tick() > 0);
}

// ---------------------------------------------------------------------------
// The referee itself is not believed blindly
// ---------------------------------------------------------------------------

/// Rewrites the digests the referee reports.
struct LyingReferee {
    inner: Box<dyn Transport>,
    from: Tick,
}

impl Transport for LyingReferee {
    fn send(&mut self, to: Destination, msg: &Message) -> Result<(), TransportError> {
        if let Message::Reference(d) = msg
            && d.tick >= self.from
        {
            return self.inner.send(to, &Message::Reference(TickDigest { tick: d.tick, digest: d.digest ^ 1 }));
        }
        self.inner.send(to, msg)
    }
    fn poll(&mut self) -> Result<Option<Incoming>, TransportError> {
        self.inner.poll()
    }
}

#[test]
fn a_referee_that_every_seat_disagrees_with_is_ignored() {
    let mut r = Room::build(3, false, RefereePolicy::default(), &mut |seat, t| {
        if seat == PlayerId(3) {
            Box::new(LyingReferee { inner: t, from: 60 })
        } else {
            t
        }
    });
    r.run_until("every peer to stop believing the referee", |r| {
        r.all_at(300) && r.peers.iter().all(|p| p.router().authority() == PlayerId(0))
    });
    assert!(!r.any_halted(), "{}", r.status());
    assert!(r.peers.iter().all(|p| p.resyncs().is_empty()));
    let ev = r.peers[1].router_mut().take_events();
    assert!(ev.iter().any(|e| matches!(e, sta_net::lockstep::RouterEvent::RefereeOutvoted { .. })), "{ev:?}");
}

#[test]
fn judge_believes_the_referee_unless_the_seats_agree_against_it() {
    assert_eq!(judge(&[5, 6, 5], 5), Verdict::ReferenceStands);
    assert_eq!(judge(&[5, 6], 6), Verdict::ReferenceStands);
    // Two seats differ from each other and from the referee: it is believed.
    assert_eq!(judge(&[5, 6], 7), Verdict::ReferenceStands);
    // Every seat agrees and the referee alone differs.
    assert_eq!(judge(&[5, 5, 5], 7), Verdict::RefereeOutvoted);
    // One seat left: nobody outvotes the referee.
    assert_eq!(judge(&[5], 7), Verdict::ReferenceStands);
    assert_eq!(judge(&[5, 5], 7), Verdict::RefereeOutvoted);
    assert_eq!(judge(&[], 7), Verdict::ReferenceStands);
}

// ---------------------------------------------------------------------------
// Roles, admission, limits
// ---------------------------------------------------------------------------

#[test]
fn only_the_referee_sends_reference_and_it_sends_no_turns_or_lobby() {
    let reference = Message::Reference(TickDigest { tick: 3, digest: 1 });
    assert!(reference.check_sender_kind(SenderKind::Referee).is_ok());
    assert!(reference.check_sender_kind(SenderKind::Peer).is_err());
    assert!(reference.check_sender_kind(SenderKind::Host).is_err());
    assert!(reference.check_sender(true).is_err() && reference.check_sender(false).is_err());
    let seated = Message::RefereeSeated { slot: None };
    assert!(seated.check_sender_kind(SenderKind::Host).is_ok());
    assert!(seated.check_sender_kind(SenderKind::Referee).is_err());
    assert!(seated.check_sender_kind(SenderKind::Peer).is_err());
    let turn = Message::Turn(sta_net::proto::TurnCommands { turn: 0, slot: PlayerId(0), commands: vec![], schedule: vec![], digest: None });
    assert!(turn.check_sender_kind(SenderKind::Referee).is_err());
    for m in [Message::CatchUpDone, Message::SnapshotRefused(SnapshotRefusal::Busy)] {
        assert!(m.check_sender_kind(SenderKind::Referee).is_ok(), "{m:?}");
    }
    assert!(Message::RulesRequest.check_sender_kind(SenderKind::Referee).is_err());
    assert!(Message::ResyncRequest(ResyncRequest { tick: 0, digest: 0, position: 0 })
        .check_sender_kind(SenderKind::Referee)
        .is_err());
}

struct Altered {
    digest: RulesDigest,
    entries: Vec<RuleEntry>,
}

impl RulesIdentity for Altered {
    fn digest(&self) -> RulesDigest {
        self.digest
    }
    fn entries(&self) -> Vec<RuleEntry> {
        self.entries.clone()
    }
}

#[test]
fn a_referee_with_another_rules_digest_is_refused_naming_the_key() {
    let db = sta_content::fake::db();
    let host_rules: &dyn RulesIdentity = db.digest();
    let hello = |rules: RulesDigest, e: EngineId| Hello { engine: e, rules, lineage: "x".into(), packs: vec![], name: "referee".into() };
    // The same rules: admitted.
    assert!(admit_referee(&engine(), host_rules, &hello(host_rules.digest(), engine()), host_rules).is_ok());
    // Another build: refused.
    let other = EngineId { build: "other".into(), ..engine() };
    let err = admit_referee(&engine(), host_rules, &hello(host_rules.digest(), other), host_rules).unwrap_err();
    assert!(matches!(err.refusal(), Refusal::Engine { .. }));
    // One value differs: refused with the key.
    let mut entries = host_rules.entries();
    assert!(!entries.is_empty());
    let key = entries[0].key.clone();
    entries[0].value.push('!');
    let altered = Altered { digest: RulesDigest([9; 32]), entries };
    let err = admit_referee(&engine(), host_rules, &hello(altered.digest, engine()), &altered).unwrap_err();
    match err.refusal() {
        Refusal::RulesDiffer(d) => assert_eq!(d.key, key),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_referee_bound_by_its_limits_ends_and_says_why() {
    // No seating: a referee the host never seats runs nothing and ends idle.
    let db = Arc::new(sta_content::fake::db());
    let map = Arc::new(sta_content::fake::two_bases());
    let start = start_of(2, false, &db);
    let (_net, mut links) = LoopbackNet::new(&[PlayerId(0), PlayerId(1), PlayerId(2)]);
    let link = links.pop().unwrap();
    let limits = RefereeLimits { idle: Duration::from_millis(30), ..RefereeLimits::default() };
    let mut referee = Referee::new(&start, PlayerId(2), db.clone(), map.clone(), Box::new(link), RefereePolicy::default(), limits).unwrap();
    let end = Instant::now() + WAIT;
    let reason = loop {
        if let RefereeStep::Ended(why) = referee.step() {
            break why;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(reason, EndReason::Idle);
    assert_eq!(referee.peer().tick(), 0);

    // A tick bound, and a bound on what its replay holds.
    let mut r = Room::new(2, true);
    r.run_until("tick 90", |r| r.all_at(90));
    r.referee.set_limits(RefereeLimits { max_logged_commands: 3, ..RefereeLimits::default() });
    assert_eq!(r.referee.step(), RefereeStep::Ended(EndReason::LogFull));
    r.referee.set_limits(RefereeLimits { max_ticks: 10, ..RefereeLimits::default() });
    assert_eq!(r.referee.step(), RefereeStep::Ended(EndReason::TooLong));
}

#[test]
fn a_referee_whose_slot_is_a_seat_is_refused() {
    let db = Arc::new(sta_content::fake::db());
    let map = Arc::new(sta_content::fake::two_bases());
    let start = start_of(2, true, &db);
    for slot in [0u8, 1, 2, 9] {
        let (_net, mut links) = LoopbackNet::new(&[PlayerId(0), PlayerId(1), PlayerId(2), PlayerId(3), PlayerId(4)]);
        let link = links.pop().unwrap();
        assert!(
            Referee::new(&start, PlayerId(slot), db.clone(), map.clone(), Box::new(link), RefereePolicy::default(), RefereeLimits::default()).is_err(),
            "slot {slot}"
        );
    }
}

#[test]
fn the_host_cannot_seat_a_player_as_the_referee() {
    let mut r = Room::new(2, false);
    r.run_until("tick 30", |r| r.all_at(30));
    assert!(r.peers[0].router_mut().seat_referee(Some(PlayerId(1))).is_err());
    assert!(r.peers[0].router_mut().seat_referee(Some(PlayerId(0))).is_err());
    // A peer that is not the host cannot name one at all.
    assert!(r.peers[1].router_mut().seat_referee(Some(PlayerId(5))).is_err());
    // And a peer refuses a `RefereeSeated` that names a seat.
    let err = r.peers[1].router_mut().receive(PlayerId(0), Message::RefereeSeated { slot: Some(PlayerId(0)) });
    assert!(err.is_err());
    assert_eq!(r.peers[1].router().referee(), Some(PlayerId(2)));
    assert_eq!(r.peers[1].router().authority(), PlayerId(2));
}

#[test]
fn a_room_without_a_referee_behaves_as_before() {
    // Peers with no referee seated: the host is the authority (N3) and a
    // diverged peer is resynced from it.
    let db = Arc::new(sta_content::fake::db());
    let map = Arc::new(sta_content::fake::two_bases());
    let start = start_of(2, false, &db);
    let (net, links) = LoopbackNet::new(&[PlayerId(0), PlayerId(1)]);
    let mut peers: Vec<Peer> = links
        .into_iter()
        .map(|l| {
            let me = l.seat();
            let mut p = Peer::start(&start, me, db.clone(), map.clone(), Box::new(l)).unwrap();
            p.set_resync_limits(fast_limits());
            p
        })
        .collect();
    assert_eq!(peers[1].router().authority(), PlayerId(0));
    let cruiser = db.key("fx_cruiser").unwrap();
    for p in 0..2u8 {
        peers[0].submit(Command {
            player: PlayerId::SCRIPT,
            seq: u32::from(p),
            kind: CommandKind::Edit(Edit::Spawn {
                odf: cruiser,
                player: PlayerId(p),
                pos: Vec3::from_f32([f32::from(p) * 100.0, 0.0, 0.0]),
                heading: (Fixed::ZERO, Fixed::ONE),
            }),
        });
    }
    let end = Instant::now() + WAIT;
    let mut injected = false;
    loop {
        let mut ran = false;
        for p in peers.iter_mut() {
            ran |= matches!(p.step(), StepOutcome::Ran { .. });
        }
        if !ran {
            net.advance(1);
        }
        if !injected && peers[1].tick() >= 60 {
            injected = true;
            let unit = peers[1].sim().view().units.iter().find(|u| u.owner == PlayerId(1)).unwrap().id;
            peers[1].inject(Fault::Inject(Command {
                player: PlayerId::SCRIPT,
                seq: 9_999,
                kind: CommandKind::Edit(Edit::Damage(unit, Fixed::from_int(3))),
            }));
        }
        if injected && !peers[1].resyncs().is_empty() && !peers.iter().any(|p| p.router().halted()) {
            break;
        }
        assert!(Instant::now() < end, "never resynced");
    }
    assert!(peers[0].resyncs().is_empty());
}

// ---------------------------------------------------------------------------
// Over the real relay and TCP
// ---------------------------------------------------------------------------

fn over_relay(players: u8, policy: RefereePolicy) -> Room {
    let relay = Relay::bind("127.0.0.1:0", RelayConfig::default()).expect("bind");
    let addr = relay.local_addr();
    let opts = ClientOptions::default();
    let host = NetTransport::create(addr, "", &opts).expect("create a room");
    let code = host.control().room_code();
    let mut ts = vec![host];
    // The players first, then the referee: it takes the next relay seat.
    for _ in 1..=players {
        ts.push(NetTransport::join(addr, code, "", &opts).expect("join"));
    }
    ts[0].control().set_open(false).unwrap();
    let transports: Vec<(PlayerId, Box<dyn Transport>)> =
        ts.into_iter().map(|t| (t.seat(), Box::new(t) as Box<dyn Transport>)).collect();
    let db = Arc::new(sta_content::fake::db());
    Room::assemble(players, false, policy, db, transports, None, Some(relay), &mut |_, t| t)
}

#[test]
fn over_the_relay_a_clean_match_is_verified() {
    let mut r = over_relay(2, RefereePolicy::default());
    r.run_until("tick 600", |r| r.all_at(600));
    assert!(r.referee.findings().is_empty() && !r.any_halted());
    r.assert_referee_agrees_with_all_seats();
}

#[test]
fn over_the_relay_a_diverged_seat_is_named_and_resynced_from_the_referee() {
    let mut r = over_relay(2, RefereePolicy::default());
    r.run_until("tick 90", |r| r.all_at(90));
    let at = r.damage(1);
    until_named(&mut r, 1);
    let f = r.referee.first_finding().unwrap();
    assert_eq!((f.seat, f.tick), (PlayerId(1), first_visible(at)));
    assert_eq!(f.kind, FindingKind::Diverged { sections: vec!["combat"] });
    until_repaired(&mut r, &[1]);
    let t = r.peers.iter().map(Peer::tick).max().unwrap();
    r.run_until("600 more ticks", |r| r.all_at(t + 600));
    r.assert_referee_agrees_with_all_seats();
}

#[test]
fn a_referee_joins_through_the_handshake_over_the_relay() {
    // The host side as the lobby does it: admit, welcome, start, seat.
    let relay = Relay::bind("127.0.0.1:0", RelayConfig::default()).expect("bind");
    let addr = relay.local_addr();
    let opts = ClientOptions::default();
    let mut host = NetTransport::create(addr, "", &opts).unwrap();
    let code = host.control().room_code();
    let mut player = NetTransport::join(addr, code, "", &opts).unwrap();
    let referee_t = NetTransport::join(addr, code, "", &opts).unwrap();
    let rslot = referee_t.seat();
    assert_eq!(rslot, PlayerId(2));
    let db = Arc::new(sta_content::fake::db());
    let start = start_of(2, false, &db);
    let hello = Hello { engine: engine(), rules: start.rules, lineage: db.lineage().to_owned(), packs: vec![], name: "referee".into() };
    let joiner = std::thread::spawn(move || {
        let mut t = referee_t;
        let s = await_start(&mut t, PlayerId(0), hello, Duration::from_secs(60));
        (s, t)
    });
    // The host answers the hello and starts the game.
    let end = Instant::now() + WAIT;
    loop {
        match host.poll().unwrap() {
            Some(Incoming { from, msg: Message::RefereeHello(h) }) => {
                assert_eq!(from, rslot);
                admit_referee(&engine(), db.digest(), &h, db.digest()).expect("admitted");
                host.send(Destination::Seat(from), &sta_net::referee::welcome(from, PlayerId(0))).unwrap();
                break;
            }
            _ => assert!(Instant::now() < end),
        }
    }
    host.send(Destination::All, &Message::Start(start.clone())).unwrap();
    let (got, _t) = joiner.join().unwrap();
    assert_eq!(got.unwrap(), start);
    // The player saw the start too, and no referee message before the host
    // names one.
    let mut saw_start = false;
    let end = Instant::now() + WAIT;
    while !saw_start {
        if let Some(Incoming { msg: Message::Start(_), .. }) = player.poll().unwrap() {
            saw_start = true;
        }
        assert!(Instant::now() < end);
    }
}

// ---------------------------------------------------------------------------
// The processes: relay, netplay with a referee, the referee binary
// ---------------------------------------------------------------------------

mod processes {
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc::{channel, Receiver};
    use std::time::{Duration, Instant};

    use sta_net::transport::{Relay, RelayConfig};

    struct Proc {
        child: Child,
        lines: Receiver<String>,
        seen: Vec<String>,
    }

    impl Proc {
        fn spawn(exe: &str, args: &[&str]) -> Proc {
            let mut child = Command::new(exe)
                .args(args)
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap_or_else(|e| panic!("{exe}: {e}"));
            let out = child.stdout.take().unwrap();
            let (tx, rx) = channel();
            std::thread::spawn(move || {
                for l in BufReader::new(out).lines().map_while(Result::ok) {
                    if tx.send(l).is_err() {
                        break;
                    }
                }
            });
            Proc { child, lines: rx, seen: Vec::new() }
        }

        /// The first line starting with `prefix`, within `limit`.
        fn wait_for(&mut self, prefix: &str, limit: Duration) -> String {
            let end = Instant::now() + limit;
            loop {
                if let Some(l) = self.seen.iter().find(|l| l.starts_with(prefix)) {
                    return l.clone();
                }
                let left = end.saturating_duration_since(Instant::now());
                match self.lines.recv_timeout(left.max(Duration::from_millis(1))) {
                    Ok(l) => self.seen.push(l),
                    Err(_) => panic!("no `{prefix}` line in time; saw {:?}", self.seen),
                }
            }
        }

        fn finish(mut self, limit: Duration) -> (i32, Vec<String>) {
            let end = Instant::now() + limit;
            loop {
                while let Ok(l) = self.lines.try_recv() {
                    self.seen.push(l);
                }
                if let Some(status) = self.child.try_wait().unwrap() {
                    while let Ok(l) = self.lines.try_recv() {
                        self.seen.push(l);
                    }
                    return (status.code().unwrap_or(-1), std::mem::take(&mut self.seen));
                }
                if Instant::now() > end {
                    let _ = self.child.kill();
                    panic!("process did not end in time; saw {:?}", self.seen);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    impl Drop for Proc {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn field(line: &str, name: &str) -> String {
        line.split_whitespace()
            .find_map(|w| w.strip_prefix(&format!("{name}=")))
            .unwrap_or_default()
            .to_owned()
    }

    #[test]
    fn a_refereed_match_runs_as_three_processes_and_is_verified() {
        let relay = Relay::bind("127.0.0.1:0", RelayConfig::default()).unwrap();
        let addr = relay.local_addr().to_string();
        let netplay = env!("CARGO_BIN_EXE_netplay");
        let referee = env!("CARGO_BIN_EXE_referee");
        let wait = Duration::from_secs(90);
        let mut host = Proc::spawn(
            netplay,
            &["host", "--relay", &addr, "--peers", "2", "--referee", "--ticks", "1500", "--hz", "0"],
        );
        let room = host.wait_for("ROOM ", wait);
        let code = room.trim_start_matches("ROOM ").trim().to_owned();
        let join = Proc::spawn(netplay, &["join", "--relay", &addr, "--code", &code, "--ticks", "1500", "--hz", "0"]);
        host.wait_for("JOINED seat 1", wait);
        let out = std::env::temp_dir().join(format!("sta-referee-{}", std::process::id()));
        let mut referee = Proc::spawn(
            referee,
            &["--relay", &addr, "--code", &code, "--out", out.to_str().unwrap(), "--engine-name", "netplay", "--engine-build", "dev", "--idle-seconds", "5"],
        );
        referee.wait_for("SEATED", wait);
        host.wait_for("REFEREE seat 2", wait);
        let (hcode, hout) = host.finish(wait);
        let (jcode, jout) = join.finish(wait);
        let (rcode, rout) = referee.finish(wait);
        let done = |o: &[String]| o.iter().find(|l| l.starts_with("DONE")).cloned().unwrap_or_default();
        assert_eq!((hcode, jcode), (0, 0), "{hout:?} {jout:?}");
        assert_eq!(field(&done(&hout), "digest"), field(&done(&jout), "digest"));
        let rdone = done(&rout);
        assert_eq!(rcode, 0, "{rout:?}");
        assert!(rdone.contains("diverged=[]"), "{rdone}");
        assert!(field(&rdone, "tick").parse::<u64>().unwrap() > 100, "{rdone}");
        let _ = std::fs::remove_dir_all(&out);
        drop(relay);
    }
}

// ---------------------------------------------------------------------------
// Review findings: a hostile authority, a departing referee
// ---------------------------------------------------------------------------

#[test]
fn a_resynced_for_a_far_future_turn_does_not_blind_the_comparison() {
    let mut r = Room::new(2, false);
    r.run_until("tick 60", |r| r.all_at(60));
    let rslot = PlayerId(2);
    let huge = Message::Resynced(sta_net::proto::Resynced { slot: PlayerId(1), turn: u32::MAX, count: 1, last: true });
    r.peers[1].router_mut().receive(rslot, huge).unwrap();
    // Desync detection still works afterwards.
    r.run_until("tick 90", |r| r.all_at(90));
    let at = r.damage(1);
    r.run_until("the divergence to be seen", |r| r.referee.findings().iter().any(|f| f.seat == PlayerId(1)));
    assert_eq!(r.referee.first_finding().unwrap().tick, first_visible(at));
}

#[test]
fn the_host_unseats_a_referee_whose_connection_ended() {
    let mut r = Room::new(2, false);
    r.run_until("tick 30", |r| r.all_at(30));
    assert_eq!(r.peers[1].router().referee(), Some(PlayerId(2)));
    r.peers[0].note_departed(PlayerId(2));
    r.run_until("peers to drop the referee", |r| r.peers[1].router().referee().is_none());
    assert_eq!(r.peers[1].router().authority(), PlayerId(0));
    // A reclaimed slot's digests are not believed.
    let lie = Message::Reference(TickDigest { tick: 3, digest: 1 });
    assert!(r.peers[1].router_mut().receive(PlayerId(2), lie).is_err());
}
