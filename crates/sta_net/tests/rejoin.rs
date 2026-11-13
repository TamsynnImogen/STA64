//! NET-04: a peer that dropped rejoins its seat from a snapshot, and a peer
//! that desynced is resynced from the host's state; over the in-process
//! loopback and over the real transport and relay (127.0.0.1, ports chosen by
//! the OS). Hostile snapshots are refused without panic.
//!
//! No assertion here depends on a sleep or on two independent connections
//! being ordered: every wait is a loop on a real condition with a deadline.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use sta_content::{ContentDb, MapDef};
use sta_core::{EntityId, Fixed, PlayerId, TeamId, Tick, Vec3};
use sta_net::lockstep::{Fault, LoopbackLink, LoopbackNet, Peer, StepOutcome, Transport};
use sta_net::proto::{
    EngineId, Hello, MapRef, Message, Occupant, Refusal, Rejoined, Seat, SnapshotKind, SnapshotOffer,
    SnapshotPart, SnapshotRefusal, Start, TurnConfig, MAX_SNAPSHOT_PART_BYTES,
};
use sta_net::resync::{
    snapshot_hash, CatchUpError, RejoinError, RejoinProgress, Rejoiner, ResyncEvent, ResyncLimits,
};
use sta_net::transport::{ClientOptions, ConnectError, NetControl, NetTransport, Relay, RelayConfig};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::GameOptions;
use sta_sim::snapshot::FORMAT_VERSION;
use sta_sim::WorldSource;

const WAIT: Duration = Duration::from_secs(90);

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn engine() -> EngineId {
    EngineId {
        name: "net04".into(),
        version: "1".into(),
        build: "test".into(),
    }
}

fn start_of(peers: u8, db: &ContentDb) -> Start {
    Start {
        seed: 0x5eed,
        map: MapRef {
            name: "fx_two_bases".into(),
            digest: 0,
        },
        options: GameOptions::default(),
        seats: (0..peers)
            .map(|s| Seat {
                slot: PlayerId(s),
                team: TeamId(s),
                race: "fx".into(),
                occupant: Occupant::Peer,
                name: format!("p{s}"),
                // The host's own content: a snapshot's provenance must match
                // the host's seat.
                lineage: db.lineage().to_owned(),
                packs: Vec::new(),
            })
            .collect(),
        host: PlayerId(0),
        turn: TurnConfig {
            ticks_per_turn: 3,
            delay_turns: 2,
            speed_percent: 100,
        },
        rules: sta_net::proto::RulesIdentity::digest(db.digest()),
    }
}

fn hello(start: &Start, seat: u8) -> Hello {
    Hello {
        engine: engine(),
        rules: start.rules,
        lineage: "native".into(),
        packs: Vec::new(),
        name: format!("p{seat}"),
    }
}

fn token_of(seat: u8) -> u64 {
    0x7000 + u64::from(seat)
}

fn fast_limits() -> ResyncLimits {
    ResyncLimits {
        min_attempt_interval: Duration::from_millis(1),
        ..ResyncLimits::default()
    }
}

struct Game {
    start: Start,
    db: Arc<ContentDb>,
    map: Arc<MapDef>,
    peers: Vec<Option<Peer>>,
    last_order: Vec<Option<Tick>>,
    net: Option<LoopbackNet>,
    controls: Vec<Option<NetControl>>,
    relay: Option<(Relay, std::net::SocketAddr, sta_net::transport::RoomCode)>,
}

impl Game {
    fn base(peers: u8) -> Game {
        let db = Arc::new(sta_content::fake::db());
        let map = Arc::new(sta_content::fake::two_bases());
        let start = start_of(peers, &db);
        Game {
            start,
            db,
            map,
            peers: Vec::new(),
            last_order: vec![None; usize::from(peers)],
            net: None,
            controls: vec![None; usize::from(peers)],
            relay: None,
        }
    }

    fn serve(&mut self) {
        let tokens: Vec<(PlayerId, u64)> =
            (1..self.peers.len() as u8).map(|s| (PlayerId(s), token_of(s))).collect();
        let host = self.peers[0].as_mut().unwrap();
        host.serve_rejoins(engine(), tokens);
        host.set_resync_limits(fast_limits());
        if let Some(c) = self.controls[0].clone() {
            host.set_departure_source(move || c.take_departures());
        }
    }

    fn loopback(peers: u8) -> Game {
        let mut g = Game::base(peers);
        let ids: Vec<PlayerId> = (0..peers).map(PlayerId).collect();
        let (net, links) = LoopbackNet::new(&ids);
        for l in links {
            let me = l.seat();
            g.peers.push(Some(
                Peer::start(&g.start, me, g.db.clone(), g.map.clone(), Box::new(l)).unwrap(),
            ));
        }
        g.net = Some(net);
        g.serve();
        g.spawn_forces();
        g
    }

    fn over_relay(peers: u8) -> Game {
        let mut g = Game::base(peers);
        let relay = Relay::bind("127.0.0.1:0", RelayConfig::default()).expect("bind");
        let addr = relay.local_addr();
        let opts = ClientOptions::default();
        let host = NetTransport::create(addr, "", &opts).expect("create a room");
        let code = host.control().room_code();
        let mut ts = vec![host];
        for _ in 1..peers {
            ts.push(NetTransport::join(addr, code, "", &opts).expect("join"));
        }
        // The game has started: a vacated seat is reserved for its rejoin.
        ts[0].control().set_open(false).unwrap();
        for t in ts {
            let me = t.seat();
            g.controls[usize::from(me.0)] = Some(t.control());
            g.peers.push(Some(
                Peer::start(&g.start, me, g.db.clone(), g.map.clone(), Box::new(t)).unwrap(),
            ));
        }
        g.relay = Some((relay, addr, code));
        g.serve();
        g.spawn_forces();
        g
    }

    fn spawn_forces(&mut self) {
        let cruiser = self.db.key("fx_cruiser").unwrap();
        let frigate = self.db.key("fx_frigate").unwrap();
        let n = self.peers.len() as i32;
        let host = self.peers[0].as_mut().unwrap();
        let mut seq = 0;
        for p in 0..n {
            for k in 0..3 {
                host.submit(Command {
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

    /// One step of peer `i`, with the orders a player would give.
    fn step(&mut self, i: usize) -> StepOutcome {
        let p = self.peers[i].as_mut().expect("peer is present");
        let me = PlayerId(i as u8);
        let t = p.tick();
        if t.is_multiple_of(9) && self.last_order[i] != Some(t) {
            self.last_order[i] = Some(t);
            let mine: Vec<EntityId> = p
                .sim()
                .view()
                .units
                .iter()
                .filter(|u| u.owner == me)
                .map(|u| u.id)
                .collect();
            if !mine.is_empty() {
                let k = (t / 9) as usize;
                let seq = p.router().last_seq(me).map_or(0, |s| s + 1);
                p.submit(Command {
                    player: me,
                    seq,
                    kind: CommandKind::Order(Order::Move {
                        units: vec![mine[k % mine.len()]],
                        to: Vec3::from_f32([
                            ((k * 37) % 500) as f32 - 250.0,
                            0.0,
                            ((k * 91) % 500) as f32 - 250.0,
                        ]),
                        queue: k.is_multiple_of(3),
                    }),
                });
            }
        }
        p.step()
    }

    fn present(&self) -> Vec<usize> {
        (0..self.peers.len()).filter(|&i| self.peers[i].is_some()).collect()
    }

    /// Steps the present peers round-robin until each is at `target`.
    fn run_to(&mut self, target: Tick) {
        let end = Instant::now() + WAIT;
        loop {
            let mut ran = false;
            let mut behind = false;
            for i in self.present() {
                if self.peers[i].as_ref().unwrap().tick() < target {
                    behind = true;
                    if matches!(self.step(i), StepOutcome::Ran { .. }) {
                        ran = true;
                    }
                }
            }
            if !behind {
                return;
            }
            if !ran {
                if let Some(n) = &self.net {
                    n.advance(1);
                } else {
                    thread::sleep(Duration::from_millis(1));
                }
            }
            assert!(Instant::now() < end, "timed out before tick {target}: {}", self.status());
        }
    }

    fn status(&self) -> String {
        self.peers
            .iter()
            .enumerate()
            .map(|(i, p)| {
                p.as_ref().map_or(format!("{i}: absent"), |p| {
                    format!(
                        "{i}: tick {} next turn {} waiting {:?} halted {}",
                        p.tick(),
                        p.router().next_turn(),
                        p.router().waiting_on(),
                        p.router().halted()
                    )
                })
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Steps the present peers until nobody can run (a full round with no
    /// progress, twice, so queued frames are delivered).
    fn settle(&mut self) {
        let mut quiet = 0;
        let end = Instant::now() + WAIT;
        while quiet < 3 {
            let mut ran = false;
            for i in self.present() {
                ran |= matches!(self.step(i), StepOutcome::Ran { .. });
            }
            if let Some(n) = &self.net {
                n.advance(1);
            } else {
                thread::sleep(Duration::from_millis(2));
            }
            quiet = if ran { 0 } else { quiet + 1 };
            assert!(Instant::now() < end, "never settled: {}", self.status());
        }
    }

    fn assert_in_sync(&self) {
        let ps: Vec<&Peer> = self.peers.iter().flatten().collect();
        for p in &ps[1..] {
            assert_eq!(p.tick(), ps[0].tick());
            assert_eq!(p.sim().digest(), ps[0].sim().digest(), "{}", self.status());
        }
    }

    /// A peer is lost: its connection ends (nothing it sent is recalled) and
    /// its sim and router are gone. The host reads the departure marker.
    fn drop_peer(&mut self, i: usize, how: Drop) {
        match (&self.net, how) {
            (Some(net), _) => {
                net.vacate(PlayerId(i as u8));
                self.peers[i] = None;
                self.peers[0].as_mut().unwrap().note_departed(PlayerId(i as u8));
            }
            (None, Drop::Abrupt) => {
                self.controls[i].as_ref().unwrap().abort();
                self.peers[i] = None;
            }
            (None, Drop::Clean) => {
                self.controls[i].as_ref().unwrap().leave();
                self.peers[i] = None;
            }
        }
    }

    /// The lost peer comes back; the others keep stepping meanwhile. Returns
    /// the new token.
    fn rejoin(&mut self, i: usize, token: u64, limits: ResyncLimits) -> Result<u64, RejoinError> {
        let me = PlayerId(i as u8);
        let transport: Box<dyn Transport> = if let Some(net) = &self.net {
            Box::new(net.reconnect(me).expect("seat is vacated"))
        } else {
            let (_, addr, code) = self.relay.as_ref().unwrap();
            let end = Instant::now() + WAIT;
            let t = loop {
                // The relay frees the seat when it notices the old
                // connection closed: ask until it has.
                match NetTransport::rejoin(*addr, *code, "", me, &ClientOptions::default()) {
                    Ok(t) => break t,
                    Err(ConnectError::Refused(_)) if Instant::now() < end => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(e) => panic!("cannot rejoin: {e}"),
                }
            };
            self.controls[i] = Some(t.control());
            Box::new(t)
        };
        let mut rj = Rejoiner::begin(
            transport,
            me,
            PlayerId(0),
            hello(&self.start, i as u8),
            token,
            self.db.clone(),
            self.map.clone(),
            limits,
            Instant::now(),
        )
        .expect("send Rejoin");
        let end = Instant::now() + WAIT;
        loop {
            for k in self.present() {
                self.step(k);
            }
            let polled = rj.poll();
            if polled.is_err()
                && let Some(n) = &self.net
            {
                // A failed rejoin ends that connection: the seat is vacated
                // again, as the relay does.
                n.vacate(me);
            }
            match polled? {
                RejoinProgress::Done(d) => {
                    self.peers[i] = Some(d.peer);
                    self.last_order[i] = None;
                    return Ok(d.token);
                }
                RejoinProgress::Waiting => {}
            }
            if let Some(n) = &self.net {
                n.advance(1);
            } else {
                thread::sleep(Duration::from_millis(1));
            }
            assert!(Instant::now() < end, "rejoin never finished: {}", self.status());
        }
    }
}

#[derive(Clone, Copy)]
enum Drop {
    Clean,
    Abrupt,
}

fn events(g: &mut Game, i: usize) -> Vec<ResyncEvent> {
    g.peers[i].as_mut().unwrap().take_resync_events()
}

// ---------------------------------------------------------------------------
// Rejoin over the loopback
// ---------------------------------------------------------------------------

#[test]
fn rejoin_after_a_clean_drop_with_the_others_stalled() {
    let mut g = Game::loopback(2);
    g.run_to(300);
    g.drop_peer(1, Drop::Clean);
    // The host plays on until it needs the seat's next record, then stalls.
    g.settle();
    let stalled_at = g.peers[0].as_ref().unwrap().tick();
    assert!((300..=300 + 3 * 8).contains(&stalled_at), "{stalled_at}");
    assert_eq!(g.peers[0].as_ref().unwrap().router().waiting_on(), vec![PlayerId(1)]);
    g.rejoin(1, token_of(1), fast_limits()).unwrap();
    let back = g.peers[1].as_ref().unwrap().tick();
    assert!(back > 0, "the rejoiner did not start from tick 0");
    assert!(back >= 3 * 90, "{back}: it starts at the host's boundary, not at tick 0");
    g.run_to(1500);
    g.assert_in_sync();
    // Everyone was told.
    assert!(events(&mut g, 0).iter().any(|e| matches!(e, ResyncEvent::Arrived { slot, .. } if *slot == PlayerId(1))));
}

#[test]
fn rejoin_while_the_others_are_still_playing_three_peers() {
    let mut g = Game::loopback(3);
    g.run_to(240);
    g.drop_peer(2, Drop::Abrupt);
    // No settle: the host and seat 1 still hold records of seat 2 and run on
    // while the transfer happens.
    g.rejoin(2, token_of(2), fast_limits()).unwrap();
    g.run_to(2400);
    g.assert_in_sync();
    let t = g.peers[2].as_ref().unwrap().tick();
    assert!(t >= 2400);
}

#[test]
fn rejoin_twice_uses_the_rotated_token_and_the_old_one_is_spent() {
    let mut g = Game::loopback(3);
    g.run_to(150);
    g.drop_peer(1, Drop::Clean);
    g.settle();
    let t1 = g.rejoin(1, token_of(1), fast_limits()).unwrap();
    assert_ne!(t1, token_of(1));
    g.run_to(600);
    g.drop_peer(1, Drop::Abrupt);
    g.settle();
    // The spent token is refused (BadRejoin), the new one works.
    match g.rejoin(1, token_of(1), fast_limits()) {
        Err(RejoinError::CatchUp(CatchUpError::RefusedRejoin(Refusal::BadRejoin))) => {}
        other => panic!("{:?}", other.map(|_| ()).err()),
    }
    // The seat is still vacated (the refusal did not take it): back with the
    // current token, after the rate limit.
    thread::sleep(Duration::from_millis(5));
    g.rejoin(1, t1, fast_limits()).unwrap();
    g.run_to(1800);
    g.assert_in_sync();
}

#[test]
fn digests_stay_equal_for_thousands_of_ticks_after_a_rejoin() {
    for peers in [2u8, 3] {
        let mut g = Game::loopback(peers);
        g.run_to(120);
        g.drop_peer(1, Drop::Clean);
        g.settle();
        g.rejoin(1, token_of(1), fast_limits()).unwrap();
        g.run_to(6000);
        g.assert_in_sync();
        // The orders flowed: a real game, not an idle one.
        assert!(g.peers[0].as_ref().unwrap().replay().commands.len() > 100);
    }
}

// ---------------------------------------------------------------------------
// Refusals on the host
// ---------------------------------------------------------------------------

/// A host with a raw link for seat 1, to send it what a hostile peer would.
fn host_and_raw(peers: u8) -> (Game, Vec<LoopbackLink>) {
    let mut g = Game::base(peers);
    let ids: Vec<PlayerId> = (0..peers).map(PlayerId).collect();
    let (net, mut links) = LoopbackNet::new(&ids);
    let first = links.remove(0);
    g.peers.push(Some(
        Peer::start(&g.start, PlayerId(0), g.db.clone(), g.map.clone(), Box::new(first)).unwrap(),
    ));
    for i in 1..peers {
        g.peers.push(None);
        let _ = i;
    }
    g.net = Some(net);
    g.serve();
    (g, links)
}

fn drain(link: &mut LoopbackLink) -> Vec<Message> {
    let mut v = Vec::new();
    while let Ok(Some(m)) = link.poll() {
        // The host's lockstep records are not what these tests look at.
        if !matches!(m.msg, Message::Turn(_)) {
            v.push(m.msg);
        }
    }
    v
}

fn rejoin_msg(g: &Game, seat: u8, token: u64) -> Message {
    Message::Rejoin(sta_net::proto::Rejoin {
        hello: hello(&g.start, seat),
        slot: PlayerId(seat),
        token,
        next_turn: None,
    })
}

#[test]
fn a_rejoin_for_a_seat_that_is_not_vacated_is_refused() {
    let (mut g, mut links) = host_and_raw(2);
    let mut l1 = links.remove(0);
    let msg = rejoin_msg(&g, 1, token_of(1));
    l1.send(sta_net::lockstep::Destination::Seat(PlayerId(0)), &msg).unwrap();
    g.peers[0].as_mut().unwrap().step();
    assert_eq!(drain(&mut l1), vec![Message::Refuse(Refusal::BadRejoin)]);
    let ev = events(&mut g, 0);
    assert!(ev.iter().any(|e| matches!(e, ResyncEvent::RejoinRefused { why: "the seat is not vacated", .. })), "{ev:?}");
}

#[test]
fn bad_tokens_are_counted_per_seat_then_ignored_and_reported_once() {
    let (mut g, mut links) = host_and_raw(2);
    let mut l1 = links.remove(0);
    let limits = ResyncLimits {
        bad_attempts: 3,
        min_attempt_interval: Duration::ZERO,
        ..ResyncLimits::default()
    };
    g.peers[0].as_mut().unwrap().set_resync_limits(limits);
    g.peers[0].as_mut().unwrap().note_departed(PlayerId(1));
    let mut replies = 0;
    for n in 0..40u64 {
        l1.send(
            sta_net::lockstep::Destination::Seat(PlayerId(0)),
            &rejoin_msg(&g, 1, 0xbad0 + n),
        )
        .unwrap();
        g.peers[0].as_mut().unwrap().step();
        replies += drain(&mut l1).len();
    }
    // Three answered, then silence: the host's per-seat state does not grow
    // with attempts and it stops spending frames on the seat.
    assert_eq!(replies, 3);
    let abuse = events(&mut g, 0)
        .into_iter()
        .filter(|e| matches!(e, ResyncEvent::SeatAbuse { .. }))
        .count();
    assert_eq!(abuse, 1);
}

#[test]
fn attempts_inside_the_interval_are_ignored_without_a_reply() {
    let (mut g, mut links) = host_and_raw(2);
    let mut l1 = links.remove(0);
    g.peers[0].as_mut().unwrap().note_departed(PlayerId(1));
    let t0 = Instant::now();
    let mut answered = 0;
    for n in 0..10u32 {
        l1.send(
            sta_net::lockstep::Destination::Seat(PlayerId(0)),
            &rejoin_msg(&g, 1, 1),
        )
        .unwrap();
        // The clock is ours: all ten arrive within a millisecond of it.
        g.peers[0].as_mut().unwrap().step_at(t0 + Duration::from_micros(u64::from(n)));
        answered += drain(&mut l1).len();
    }
    assert_eq!(answered, 1);
}

#[test]
fn a_rejoin_with_another_build_or_other_rules_is_refused() {
    let (mut g, mut links) = host_and_raw(2);
    let mut l1 = links.remove(0);
    let host = g.peers[0].as_mut().unwrap();
    host.set_resync_limits(ResyncLimits {
        min_attempt_interval: Duration::ZERO,
        ..ResyncLimits::default()
    });
    host.note_departed(PlayerId(1));
    let mut other_build = hello(&g.start, 1);
    other_build.engine.build = "other".into();
    let mut other_rules = hello(&g.start, 1);
    other_rules.rules.0[0] ^= 1;
    for (h, want) in [
        (other_build, Message::Refuse(Refusal::Engine { host: engine() })),
        (other_rules, Message::Refuse(Refusal::BadRejoin)),
    ] {
        let msg = Message::Rejoin(sta_net::proto::Rejoin {
            hello: h,
            slot: PlayerId(1),
            token: token_of(1),
            next_turn: None,
        });
        l1.send(sta_net::lockstep::Destination::Seat(PlayerId(0)), &msg).unwrap();
        g.peers[0].as_mut().unwrap().step();
        assert_eq!(drain(&mut l1), vec![want]);
    }
}

#[test]
fn a_host_without_a_rejoin_policy_refuses_every_rejoin() {
    let mut g = Game::base(2);
    let (_net, mut links) = LoopbackNet::new(&[PlayerId(0), PlayerId(1)]);
    let mut l1 = links.remove(1);
    let l0 = links.remove(0);
    let mut host = Peer::start(&g.start, PlayerId(0), g.db.clone(), g.map.clone(), Box::new(l0)).unwrap();
    host.note_departed(PlayerId(1));
    let msg = rejoin_msg(&g, 1, 0);
    l1.send(sta_net::lockstep::Destination::Seat(PlayerId(0)), &msg).unwrap();
    host.step();
    assert_eq!(drain(&mut l1), vec![Message::Refuse(Refusal::BadRejoin)]);
    g.peers.push(Some(host));
}

#[test]
fn a_seat_over_its_rejoin_limit_is_a_stalled_seat() {
    let mut g = Game::loopback(2);
    let limits = ResyncLimits {
        rejoins_per_seat: 2,
        min_attempt_interval: Duration::ZERO,
        ..ResyncLimits::default()
    };
    g.peers[0].as_mut().unwrap().set_resync_limits(limits.clone());
    g.run_to(60);
    let mut token = token_of(1);
    for _ in 0..2 {
        g.drop_peer(1, Drop::Clean);
        g.settle();
        token = g.rejoin(1, token, limits.clone()).unwrap();
        g.run_to(g.peers[1].as_ref().unwrap().tick() + 60);
    }
    g.drop_peer(1, Drop::Clean);
    g.settle();
    match g.rejoin(1, token, limits) {
        Err(RejoinError::CatchUp(CatchUpError::Refused(SnapshotRefusal::LimitReached))) => {}
        other => panic!("{:?}", other.map(|_| ()).err()),
    }
    assert!(events(&mut g, 0).iter().any(|e| matches!(e, ResyncEvent::LimitReached { kind: SnapshotKind::Rejoin, .. })));
}

// ---------------------------------------------------------------------------
// Hostile snapshots, refused by the rejoiner
// ---------------------------------------------------------------------------

/// What an honest host would send a rejoining seat 1 of a two-peer game, as
/// messages, taken from a real game.
struct Material {
    start: Start,
    frames: Vec<Message>,
    db: Arc<ContentDb>,
    map: Arc<MapDef>,
}

fn material() -> Material {
    let mut g = Game::loopback(2);
    g.run_to(180);
    let host = g.peers[0].as_ref().unwrap();
    assert!(host.tick().is_multiple_of(3));
    let (seed, records) = host.router().seed().expect("seed fits");
    let bytes = host.sim().snapshot();
    let offer = SnapshotOffer {
        kind: SnapshotKind::Rejoin,
        tick: host.tick(),
        format: FORMAT_VERSION,
        content_hash: g.db.hash(),
        digest: host.sim().digest(),
        sections: host.sim().section_digests().values().to_vec(),
        len: bytes.len() as u32,
        chunk: MAX_SNAPSHOT_PART_BYTES as u32,
        parts: bytes.len().div_ceil(MAX_SNAPSHOT_PART_BYTES) as u16,
        hash: snapshot_hash(&bytes),
        seed,
    };
    let mut frames = vec![
        Message::Rejoined(Rejoined {
            last_turn: Some(offer.seed.seats[1].next_turn - 1),
            rejoin_token: 99,
        }),
        Message::Start(g.start.clone()),
        Message::SnapshotOffer(offer),
    ];
    for (i, c) in bytes.chunks(MAX_SNAPSHOT_PART_BYTES).enumerate() {
        frames.push(Message::SnapshotPart(SnapshotPart {
            index: i as u16,
            data: c.to_vec(),
        }));
    }
    frames.extend(records.into_iter().map(Message::Replayed));
    frames.push(Message::CatchUpDone);
    Material {
        start: g.start.clone(),
        frames,
        db: g.db.clone(),
        map: g.map.clone(),
    }
}

fn run_fake_host(
    m: &Material,
    mutate: impl FnOnce(&mut Vec<Message>),
) -> Result<Box<sta_net::resync::RejoinDone>, RejoinError> {
    let (_net, mut links) = LoopbackNet::new(&[PlayerId(0), PlayerId(1)]);
    let mut host = links.remove(0);
    let joiner = links.remove(0);
    let mut rj = Rejoiner::begin(
        Box::new(joiner),
        PlayerId(1),
        PlayerId(0),
        hello(&m.start, 1),
        5,
        m.db.clone(),
        m.map.clone(),
        ResyncLimits::default(),
        Instant::now(),
    )
    .unwrap();
    let mut frames = m.frames.clone();
    mutate(&mut frames);
    for f in &frames {
        // A frame the encoder refuses never reaches the wire.
        if host.send(sta_net::lockstep::Destination::Seat(PlayerId(1)), f).is_err() {
            return Err(RejoinError::Overflow);
        }
    }
    match rj.poll()? {
        RejoinProgress::Done(d) => Ok(d),
        RejoinProgress::Waiting => Err(RejoinError::Start("still waiting")),
    }
}

fn offer_of(frames: &mut [Message]) -> &mut SnapshotOffer {
    frames
        .iter_mut()
        .find_map(|m| if let Message::SnapshotOffer(o) = m { Some(o) } else { None })
        .unwrap()
}

fn parts_of(frames: &mut [Message]) -> Vec<&mut SnapshotPart> {
    frames
        .iter_mut()
        .filter_map(|m| if let Message::SnapshotPart(p) = m { Some(p) } else { None })
        .collect()
}

fn refused(r: Result<Box<sta_net::resync::RejoinDone>, RejoinError>) -> CatchUpError {
    match r {
        Err(RejoinError::CatchUp(e)) => e,
        Err(other) => panic!("not a catch-up error: {other}"),
        Ok(_) => panic!("a hostile snapshot was applied"),
    }
}

#[test]
fn the_honest_material_rejoins() {
    let m = material();
    let d = run_fake_host(&m, |_| {}).expect("honest transfer");
    assert_eq!(d.peer.tick(), 180);
    assert_eq!(d.token, 99);
}

#[test]
fn a_damaged_part_is_refused_by_its_hash() {
    let m = material();
    let e = refused(run_fake_host(&m, |f| parts_of(f)[0].data[40] ^= 0x55));
    assert_eq!(e, CatchUpError::Damaged("hash"));
}

#[test]
fn a_mutated_snapshot_with_a_valid_hash_is_refused_without_a_panic() {
    // The hash only catches damage: here the host (or a middlebox) fixes it
    // up. The load or the digests must still refuse every mutation.
    let m = material();
    let total: usize = m.frames.iter().filter_map(|f| if let Message::SnapshotPart(p) = f { Some(p.data.len()) } else { None }).sum();
    let mut refused_count = 0;
    let mut pos = 0;
    while pos < total {
        let r = run_fake_host(&m, |f| {
            let mut bytes: Vec<u8> = parts_of(f).iter().flat_map(|p| p.data.clone()).collect();
            bytes[pos] ^= 0x01;
            for (p, c) in parts_of(f).into_iter().zip(bytes.chunks(MAX_SNAPSHOT_PART_BYTES)) {
                p.data = c.to_vec();
            }
            let h = snapshot_hash(&bytes);
            offer_of(f).hash = h;
        });
        match r {
            Err(RejoinError::CatchUp(
                CatchUpError::Snapshot(_)
                | CatchUpError::Digest { .. }
                | CatchUpError::Damaged(_)
                | CatchUpError::Content { .. }
                | CatchUpError::Format { .. }
                | CatchUpError::Provenance
                | CatchUpError::Protocol(_),
            )) => refused_count += 1,
            // A flipped bit in padding or an unused field can load and still
            // be the same state (then the digests agree and it is harmless):
            // it must at least not have changed the state.
            Ok(d) => assert_eq!(d.peer.sim().digest(), {
                let Message::SnapshotOffer(o) = &m.frames[2] else { unreachable!() };
                o.digest
            }),
            Err(e) => panic!("byte {pos}: unexpected {e}"),
        }
        // Sample the snapshot densely at the start (header, provenance) and
        // sparsely after, to bound the test.
        pos += if pos < 256 { 3 } else { total / 40 + 1 };
    }
    assert!(refused_count > 10, "{refused_count} refusals");
}

#[test]
fn a_digest_mismatch_names_the_differing_section() {
    let m = material();
    // Lying about one section's digest.
    let e = refused(run_fake_host(&m, |f| offer_of(f).sections[1] ^= 1));
    match e {
        CatchUpError::Digest { sections, .. } => assert_eq!(sections, vec!["movement"]),
        other => panic!("{other}"),
    }
    let e = refused(run_fake_host(&m, |f| offer_of(f).digest ^= 1));
    assert!(matches!(e, CatchUpError::Digest { .. }), "{e}");
    let text = e.to_string();
    assert!(text.contains("differing sections"), "{text}");
}

#[test]
fn truncated_and_short_transfers_are_refused() {
    let m = material();
    // CatchUpDone before the last part.
    let e = refused(run_fake_host(&m, |f| {
        let at = f.iter().rposition(|m| matches!(m, Message::SnapshotPart(_))).unwrap();
        f.truncate(at);
        f.push(Message::CatchUpDone);
    }));
    assert_eq!(e, CatchUpError::Protocol("catch-up incomplete"));
    // The last part cut short.
    let e = refused(run_fake_host(&m, |f| {
        let mut p = parts_of(f);
        let last = p.len() - 1;
        let n = p[last].data.len();
        p[last].data.truncate(n - 1);
    }));
    assert_eq!(e, CatchUpError::Damaged("part size"));
    // A record missing.
    let e = refused(run_fake_host(&m, |f| {
        let at = f.iter().position(|m| matches!(m, Message::Replayed(_))).unwrap();
        f.remove(at);
    }));
    assert!(matches!(e, CatchUpError::Protocol(_)), "{e}");
    // Parts out of order and repeated.
    let e = refused(run_fake_host(&m, |f| {
        let at = f.iter().position(|m| matches!(m, Message::SnapshotPart(_))).unwrap();
        let dup = f[at].clone();
        f.insert(at, dup);
    }));
    assert!(matches!(e, CatchUpError::Protocol("part out of order") | CatchUpError::Damaged(_)), "{e}");
}

#[test]
fn an_oversized_or_inconsistent_offer_is_refused_before_buffering() {
    let m = material();
    // Over the bound: the encoder (and so the decoder) refuses the frame, so
    // it can never reach a rejoiner (proto's tests cover the decoder side).
    let r = run_fake_host(&m, |f| offer_of(f).len = 64 * 1024 * 1024);
    assert!(matches!(r, Err(RejoinError::Overflow)), "{:?}", r.err().map(|e| e.to_string()));
    // Within the bound but claiming more parts than ever come: it waits and
    // the idle deadline ends it, having held only what arrived.
    let (_net, mut links) = LoopbackNet::new(&[PlayerId(0), PlayerId(1)]);
    let mut host = links.remove(0);
    let joiner = links.remove(0);
    let t0 = Instant::now();
    let mut rj = Rejoiner::begin(
        Box::new(joiner),
        PlayerId(1),
        PlayerId(0),
        hello(&m.start, 1),
        5,
        m.db.clone(),
        m.map.clone(),
        ResyncLimits::default(),
        t0,
    )
    .unwrap();
    let mut frames = m.frames.clone();
    {
        let o = offer_of(&mut frames);
        o.len = 8 * 1024 * 1024;
        o.parts = 256;
    }
    // The offer arrives and nothing after it: the rejoiner has buffered
    // nothing, and the idle deadline ends it.
    frames.truncate(3);
    for f in &frames {
        host.send(sta_net::lockstep::Destination::Seat(PlayerId(1)), f).unwrap();
    }
    assert!(matches!(rj.poll_at(t0 + Duration::from_secs(1)), Ok(RejoinProgress::Waiting)));
    match rj.poll_at(t0 + Duration::from_secs(3600)) {
        Err(RejoinError::CatchUp(CatchUpError::Timeout(_))) => {}
        other => panic!("{:?}", other.map(|_| ()).err().map(|e| e.to_string())),
    }
}

#[test]
fn a_wrong_session_snapshot_is_refused() {
    let m = material();
    let e = refused(run_fake_host(&m, |f| offer_of(f).content_hash ^= 1));
    assert!(matches!(e, CatchUpError::Content { .. }), "{e}");
    let e = refused(run_fake_host(&m, |f| offer_of(f).format = FORMAT_VERSION + 1));
    assert_eq!(e, CatchUpError::Format { theirs: FORMAT_VERSION + 1, ours: FORMAT_VERSION });
    // The snapshot's own header names other content than the offer says.
    let e = refused(run_fake_host(&m, |f| {
        let mut bytes: Vec<u8> = parts_of(f).iter().flat_map(|p| p.data.clone()).collect();
        bytes[10] ^= 1;
        for (p, c) in parts_of(f).into_iter().zip(bytes.chunks(MAX_SNAPSHOT_PART_BYTES)) {
            p.data = c.to_vec();
        }
        let h = snapshot_hash(&bytes);
        offer_of(f).hash = h;
    }));
    assert_eq!(e, CatchUpError::Damaged("content hash differs from the offer's"));
    // Provenance: the host's seat in Start says another lineage.
    let e = refused(run_fake_host(&m, |f| {
        for msg in f.iter_mut() {
            if let Message::Start(s) = msg {
                s.seats[0].lineage = "fo4".into();
            }
        }
    }));
    assert_eq!(e, CatchUpError::Provenance);
    // Another game: a different seat set.
    let e = refused(run_fake_host(&m, |f| {
        offer_of(f).seed.seats.pop();
    }));
    assert!(matches!(e, CatchUpError::Protocol(_)), "{e}");
    // A Start for other rules than this peer's.
    let r = run_fake_host(&m, |f| {
        for msg in f.iter_mut() {
            if let Message::Start(s) = msg {
                s.rules.0[3] ^= 1;
            }
        }
    });
    assert!(matches!(r, Err(RejoinError::Start(_))), "{:?}", r.err().map(|e| e.to_string()));
}

#[test]
fn forged_catch_up_records_are_refused_like_live_ones() {
    let m = material();
    let e = refused(run_fake_host(&m, |f| {
        for msg in f.iter_mut() {
            if let Message::Replayed(r) = msg
                && r.slot == PlayerId(0)
            {
                // The host's record carrying a command for seat 1's player
                // is fine; a *client's* record must not carry the host's.
            }
            if let Message::Replayed(r) = msg
                && r.slot == PlayerId(1)
            {
                r.commands.push(Command {
                    player: PlayerId(0),
                    seq: 1_000_000,
                    kind: CommandKind::Order(Order::Stop { units: vec![] }),
                });
                r.commands.sort_by_key(Command::sort_key);
                break;
            }
        }
    }));
    assert!(matches!(e, CatchUpError::Authority(_)), "{e}");
}

#[test]
fn a_host_that_stays_silent_ends_the_rejoin_by_deadline() {
    let m = material();
    let (_net, mut links) = LoopbackNet::new(&[PlayerId(0), PlayerId(1)]);
    let _host = links.remove(0);
    let joiner = links.remove(0);
    let t0 = Instant::now();
    let mut rj = Rejoiner::begin(
        Box::new(joiner),
        PlayerId(1),
        PlayerId(0),
        hello(&m.start, 1),
        5,
        m.db.clone(),
        m.map.clone(),
        ResyncLimits::default(),
        t0,
    )
    .unwrap();
    assert!(matches!(rj.poll_at(t0 + Duration::from_secs(1)), Ok(RejoinProgress::Waiting)));
    match rj.poll_at(t0 + Duration::from_secs(3600)) {
        Err(RejoinError::CatchUp(CatchUpError::Timeout(_))) => {}
        other => panic!("{:?}", other.map(|_| ()).err().map(|e| e.to_string())),
    }
}

#[test]
fn unsolicited_snapshot_traffic_is_dropped_not_buffered() {
    // A peer that did not ask for a snapshot gets an offer, parts and
    // records from the host seat: nothing is kept and nothing breaks.
    let m = material();
    let mut g = Game::loopback(2);
    g.run_to(60);
    let (_net, mut links) = LoopbackNet::new(&[PlayerId(0), PlayerId(1)]);
    let mut host = links.remove(0);
    let l1 = links.remove(0);
    let mut peer = Peer::start(&g.start, PlayerId(1), g.db.clone(), g.map.clone(), Box::new(l1)).unwrap();
    for f in &m.frames[2..] {
        host.send(sta_net::lockstep::Destination::Seat(PlayerId(1)), f).unwrap();
    }
    // Look at the router's stash before the peer's own step drains it.
    peer.router_mut().pump();
    assert!(peer.router_mut().take_inbound().is_empty(), "unsolicited traffic was buffered");
    assert!(!peer.resyncing());
}

// ---------------------------------------------------------------------------
// Resync after a desync (not a rejoin: the connection never left)
// ---------------------------------------------------------------------------

fn desync_by_local_damage(g: &mut Game, victim: usize) {
    let unit = g.peers[victim]
        .as_ref()
        .unwrap()
        .sim()
        .view()
        .units
        .iter()
        .find(|u| u.owner == PlayerId(victim as u8))
        .unwrap()
        .id;
    g.peers[victim].as_mut().unwrap().inject(Fault::Inject(Command {
        player: PlayerId::SCRIPT,
        seq: 9_999,
        kind: CommandKind::Edit(Edit::Damage(unit, Fixed::from_int(7))),
    }));
}

/// Steps everyone until nobody is stopped by the desync any more and the
/// victims have resynced, or fails with the state.
fn until_repaired(g: &mut Game, resynced: &[usize]) {
    let end = Instant::now() + WAIT;
    let mut seen_halt = false;
    loop {
        for i in g.present() {
            g.step(i);
        }
        if let Some(n) = &g.net {
            n.advance(1);
        }
        let halted = g.peers.iter().flatten().any(|p| p.router().halted());
        seen_halt |= halted;
        if seen_halt
            && !halted
            && resynced.iter().all(|&i| !g.peers[i].as_ref().unwrap().resyncs().is_empty())
        {
            return;
        }
        assert!(Instant::now() < end, "never repaired: {} halted-seen {seen_halt}", g.status());
    }
}

#[test]
fn a_diverged_peer_is_resynced_and_digests_stay_equal_two_peers() {
    let mut g = Game::loopback(2);
    g.run_to(90);
    desync_by_local_damage(&mut g, 1);
    until_repaired(&mut g, &[1]);
    let note = g.peers[1].as_ref().unwrap().resyncs()[0].clone();
    assert!(note.tick >= note.first_differing);
    g.run_to(g.peers[0].as_ref().unwrap().tick().max(g.peers[1].as_ref().unwrap().tick()) + 1500);
    g.assert_in_sync();
    // Everyone is told: the host and the peer saw the resync.
    let ev0 = events(&mut g, 0);
    assert!(ev0.iter().any(|e| matches!(e, ResyncEvent::Resynced { slot, count: 1, last: true, .. } if *slot == PlayerId(1))), "{ev0:?}");
    let ev1 = events(&mut g, 1);
    assert!(ev1.iter().any(|e| matches!(e, ResyncEvent::Repaired { .. })), "{ev1:?}");
    // The sidecar of the resynced peer marks it.
    let side = g.peers[1].as_ref().unwrap().desync_sidecar();
    assert!(side.contains("resynced from the host"), "{side}");
    // The authority's replay of record has no such mark and reruns.
    assert!(!g.peers[0].as_ref().unwrap().desync_sidecar().contains("resynced from the host"));
}

#[test]
fn only_the_diverged_peer_is_resynced_three_peers_and_the_host_wins_when_it_differs() {
    // A non-host peer diverges: the others do not reload anything.
    let mut g = Game::loopback(3);
    g.run_to(90);
    desync_by_local_damage(&mut g, 2);
    until_repaired(&mut g, &[2]);
    assert!(g.peers[0].as_ref().unwrap().resyncs().is_empty());
    assert!(g.peers[1].as_ref().unwrap().resyncs().is_empty());
    let t = g.peers[0].as_ref().unwrap().tick().max(g.peers[2].as_ref().unwrap().tick());
    g.run_to(t + 1200);
    g.assert_in_sync();

    // The host diverges: it is the authority anyway (N3), so both others
    // are resynced to its state.
    let mut g = Game::loopback(3);
    g.run_to(90);
    desync_by_local_damage(&mut g, 0);
    until_repaired(&mut g, &[1, 2]);
    assert!(g.peers[0].as_ref().unwrap().resyncs().is_empty());
    let t = g.peers.iter().flatten().map(Peer::tick).max().unwrap();
    g.run_to(t + 1200);
    g.assert_in_sync();
}

/// Steps the peers in `who` round-robin, `rounds` times at most, until `done`;
/// whether it got there. A count of rounds, not a clock: the loopback has no
/// latency, so the outcome does not depend on how fast the machine is.
fn rounds_until(g: &mut Game, who: &[usize], rounds: usize, done: impl Fn(&Game) -> bool) -> bool {
    for _ in 0..rounds {
        if done(g) {
            return true;
        }
        for &i in who {
            g.step(i);
        }
        g.net.as_ref().unwrap().advance(1);
    }
    done(g)
}

fn halted(g: &Game, i: usize) -> bool {
    g.peers[i].as_ref().unwrap().router().halted()
}

fn tick_of(g: &Game, i: usize) -> Tick {
    g.peers[i].as_ref().unwrap().tick()
}

/// Repaired, nobody stopped, and the match plays on in step for `ticks` more.
fn repaired_and_plays_on(g: &mut Game, resynced: usize, ticks: Tick) {
    let all = g.present();
    let fixed = rounds_until(g, &all, 3_000, |g| {
        !g.peers[resynced].as_ref().unwrap().resyncs().is_empty() && all.iter().all(|&i| !halted(g, i))
    });
    assert!(fixed, "never repaired: {}", g.status());
    // Each peer to exactly `to`, then they compare equal.
    let to = all.iter().map(|&i| tick_of(g, i)).max().unwrap() + ticks;
    for _ in 0..20_000 {
        let behind: Vec<usize> = all.iter().copied().filter(|&i| tick_of(g, i) < to).collect();
        if behind.is_empty() {
            break;
        }
        for i in behind {
            g.step(i);
        }
        g.net.as_ref().unwrap().advance(1);
    }
    assert!(all.iter().all(|&i| tick_of(g, i) == to && !halted(g, i)), "stopped again after the repair: {}", g.status());
    g.assert_in_sync();
}

#[test]
fn a_host_behind_the_differing_tick_runs_on_to_it_and_the_diverged_seat_is_repaired() {
    // Three peers. The two guests play ahead of the host (as far as the host's
    // records let them); seat 1 diverges, and seat 2 finds it and stops the
    // match before the host has reached the tick that differs. The host must
    // still get there, or nobody ever has the authority's digest for it, the
    // diverged seat cannot ask to be repaired, and the match never resumes.
    let mut g = Game::loopback(3);
    g.run_to(90);
    desync_by_local_damage(&mut g, 1);
    let stopped = rounds_until(&mut g, &[1, 2], 200, |g| halted(g, 1) && halted(g, 2));
    assert!(stopped, "the guests did not stop: {}", g.status());
    let first = g.peers[2].as_ref().unwrap().router().desync().map(|n| n.theirs.tick);
    assert!(first.is_some_and(|t| t > tick_of(&g, 0)), "the host is not behind the differing tick: {first:?}, {}", g.status());
    repaired_and_plays_on(&mut g, 1, 600);
    assert!(g.peers[0].as_ref().unwrap().resyncs().is_empty());
    assert!(g.peers[2].as_ref().unwrap().resyncs().is_empty());
}

#[test]
fn what_a_seat_reported_before_its_repair_does_not_stop_the_match_again() {
    // Two peers; the guest runs ahead of the host and diverges, and reports
    // the digests of its damaged state for the turns it ran ahead. The host
    // repairs it from an earlier turn: the guest's reports for the turns after
    // that one described the state that was replaced and must not be compared
    // with the host's when it gets there.
    let mut g = Game::loopback(2);
    g.run_to(90);
    desync_by_local_damage(&mut g, 1);
    // As far as the host's records let it: two turns of delay.
    rounds_until(&mut g, &[1], 100, |_| false);
    assert!(tick_of(&g, 1) >= tick_of(&g, 0) + 6, "the guest did not run ahead: {}", g.status());
    repaired_and_plays_on(&mut g, 1, 600);
    assert_eq!(g.peers[1].as_ref().unwrap().resyncs().len(), 1, "one repair, not a second one");
}

#[test]
fn a_seat_over_its_resync_limit_goes_to_the_stall_dialog() {
    let mut g = Game::loopback(2);
    let limits = ResyncLimits {
        resyncs_per_seat: 1,
        min_attempt_interval: Duration::ZERO,
        ..ResyncLimits::default()
    };
    g.peers[0].as_mut().unwrap().set_resync_limits(limits.clone());
    g.peers[1].as_mut().unwrap().set_resync_limits(limits);
    g.run_to(90);
    desync_by_local_damage(&mut g, 1);
    until_repaired(&mut g, &[1]);
    let t = g.peers[0].as_ref().unwrap().tick().max(g.peers[1].as_ref().unwrap().tick());
    g.run_to(t + 60);
    desync_by_local_damage(&mut g, 1);
    // The second desync is refused: the peer stays stopped and is told why.
    let end = Instant::now() + WAIT;
    while !g.peers[1].as_ref().unwrap().resync_failed() {
        for i in 0..2 {
            g.step(i);
        }
        g.net.as_ref().unwrap().advance(1);
        assert!(Instant::now() < end, "{}", g.status());
    }
    let ev1 = events(&mut g, 1);
    assert!(ev1.iter().any(|e| matches!(e, ResyncEvent::Failed(CatchUpError::Refused(SnapshotRefusal::LimitReached)))), "{ev1:?}");
    let ev0 = events(&mut g, 0);
    assert!(ev0.iter().any(|e| matches!(e, ResyncEvent::LimitReached { kind: SnapshotKind::Resync, .. })), "{ev0:?}");
    assert!(g.peers[0].as_ref().unwrap().router().halted());
}

#[test]
fn a_resync_request_that_matches_no_divergence_is_refused() {
    let (mut g, mut links) = host_and_raw(2);
    let mut l1 = links.remove(0);
    l1.send(
        sta_net::lockstep::Destination::Seat(PlayerId(0)),
        &Message::ResyncRequest(sta_net::proto::ResyncRequest { tick: 30, digest: 1, position: 10 }),
    )
    .unwrap();
    g.peers[0].as_mut().unwrap().step();
    assert_eq!(drain(&mut l1), vec![Message::SnapshotRefused(SnapshotRefusal::NotDiverged)]);
}

// ---------------------------------------------------------------------------
// The real transport and relay
// ---------------------------------------------------------------------------

#[test]
fn rejoin_over_the_relay_after_an_abrupt_loss_two_peers() {
    let mut g = Game::over_relay(2);
    g.run_to(300);
    g.drop_peer(1, Drop::Abrupt);
    g.settle_quiet_host();
    g.rejoin(1, token_of(1), ResyncLimits::default()).unwrap();
    g.run_to(1800);
    g.assert_in_sync();
}

#[test]
fn rejoin_over_the_relay_after_a_clean_leave_three_peers_others_playing() {
    let mut g = Game::over_relay(3);
    g.run_to(240);
    g.drop_peer(2, Drop::Clean);
    g.rejoin(2, token_of(2), ResyncLimits::default()).unwrap();
    g.run_to(2400);
    g.assert_in_sync();
}

#[test]
fn a_second_rejoin_for_an_occupied_seat_is_refused_by_the_relay() {
    let g = Game::over_relay(2);
    let (_, addr, code) = g.relay.as_ref().unwrap();
    // Seat 1 is connected: its seat is not vacated.
    match NetTransport::rejoin(*addr, *code, "", PlayerId(1), &ClientOptions::default()) {
        Err(ConnectError::Refused(_)) => {}
        Ok(_) => panic!("an occupied seat was taken"),
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn a_second_rejoiner_for_a_seat_being_rejoined_is_refused() {
    let mut g = Game::over_relay(2);
    g.run_to(150);
    g.drop_peer(1, Drop::Abrupt);
    let (_, addr, code) = g.relay.as_ref().unwrap();
    let (addr, code) = (*addr, *code);
    let end = Instant::now() + WAIT;
    let first = loop {
        match NetTransport::rejoin(addr, code, "", PlayerId(1), &ClientOptions::default()) {
            Ok(t) => break t,
            Err(ConnectError::Refused(_)) if Instant::now() < end => thread::sleep(Duration::from_millis(2)),
            Err(e) => panic!("{e}"),
        }
    };
    // The seat is taken again at the relay: a second connection is refused.
    assert!(matches!(
        NetTransport::rejoin(addr, code, "", PlayerId(1), &ClientOptions::default()),
        Err(ConnectError::Refused(_))
    ));
    drop(first);
}

impl Game {
    /// Over the relay: lets the host read the departure and stall on the
    /// vacated seat (the loop ends when the host cannot run any more).
    fn settle_quiet_host(&mut self) {
        let end = Instant::now() + WAIT;
        let mut quiet = 0;
        while quiet < 20 {
            let ran = matches!(self.step(0), StepOutcome::Ran { .. });
            thread::sleep(Duration::from_millis(1));
            quiet = if ran { 0 } else { quiet + 1 };
            assert!(Instant::now() < end, "{}", self.status());
        }
    }
}
