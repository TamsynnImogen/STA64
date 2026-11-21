//! NET-03: NET-02's lockstep scenarios over the real transport (TCP through
//! the relay), and the relay and client under hostile input. Everything runs
//! on 127.0.0.1 with ports chosen by the OS; every wait is bounded.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use sta_content::ContentDb;
use sta_core::{Fixed, PlayerId, TeamId, Tick, Vec3};
use sta_net::lockstep::{
    Destination, LockstepRouter, Peer, RouterError, RouterEvent, StepOutcome, Transport,
    TransportError, MAX_EVENTS, MAX_QUEUED_TURNS,
};
use sta_net::proto::{
    encode, Action, DropChoice, MapRef, Message, Occupant, PackInfo, RulesDigest, Seat, Start,
    TickDigest, TurnCommands, TurnConfig, HEADER_LEN, MAX_FRAME_BYTES,
};
use sta_net::transport::wire::{ClientMsg, RoomCode, ServerMsg, MAX_ENVELOPE_BYTES};
use sta_net::transport::{
    ClientOptions, CloseCause, CloseLogHandle, CloseReason, CloseRecord, ConnectError, LogSink,
    Misuse, NetControl, NetTransport, RefuseReason, Relay, RelayConfig, CLOSE_LOG,
};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::GameOptions;
use sta_sim::WorldSource;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Generous: CI runners (macOS, Windows) are slow and shared.
const WAIT: Duration = Duration::from_secs(60);
const SHORT: Duration = Duration::from_secs(10);

fn wait_until(limit: Duration, what: &str, mut cond: impl FnMut() -> bool) {
    let end = Instant::now() + limit;
    while !cond() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(2));
    }
}

fn relay_with(cfg: RelayConfig) -> Relay {
    Relay::bind_with_log("127.0.0.1:0", cfg, Arc::new(|_| {})).expect("bind an ephemeral port")
}

/// A relay whose log lines are collected.
fn relay_logging(cfg: RelayConfig) -> (Relay, Arc<Mutex<Vec<String>>>) {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let l = lines.clone();
    let sink: LogSink = Arc::new(move |line| l.lock().unwrap().push(line.to_owned()));
    let r = Relay::bind_with_log("127.0.0.1:0", cfg, sink).expect("bind an ephemeral port");
    (r, lines)
}

fn relay() -> Relay {
    relay_with(RelayConfig::default())
}

fn opts() -> ClientOptions {
    ClientOptions::default()
}

fn open_room(addr: SocketAddr) -> (NetTransport, RoomCode) {
    open_room_with(addr, &opts())
}

fn open_room_with(addr: SocketAddr, o: &ClientOptions) -> (NetTransport, RoomCode) {
    let t = NetTransport::create(addr, "", o).expect("create a room");
    let code = t.control().room_code();
    (t, code)
}

fn ping(nonce: u64) -> Message {
    Message::Ping { nonce }
}

/// Polls until a message arrives.
fn recv(t: &mut NetTransport) -> sta_net::lockstep::Incoming {
    let end = Instant::now() + SHORT;
    loop {
        match t.poll() {
            Ok(Some(m)) => return m,
            Ok(None) => {}
            Err(e) => panic!("transport failed: {e} ({:?})", t.control().close_reason()),
        }
        assert!(Instant::now() < end, "no message arrived");
        thread::sleep(Duration::from_millis(1));
    }
}

fn expect_quiet(t: &mut NetTransport, for_: Duration) {
    let end = Instant::now() + for_;
    while Instant::now() < end {
        assert!(matches!(t.poll(), Ok(None)), "an unexpected message arrived");
        thread::sleep(Duration::from_millis(2));
    }
}

// ---------------------------------------------------------------------------
// A delaying, interruptible TCP proxy: the injected latency and the held
// connection of the lockstep scenarios.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Proxy {
    addr: SocketAddr,
    hold: Arc<AtomicBool>,
    /// Bytes read from a side / written to the other side, over all
    /// connections and both directions (stall diagnostics only).
    read: Arc<AtomicU64>,
    written: Arc<AtomicU64>,
    /// How each pumped direction ended, with the time since the proxy
    /// started (stall diagnostics only): who closed first.
    ends: Arc<Mutex<Vec<String>>>,
}

/// What one pumped direction shares with the proxy.
struct Tap {
    dir: &'static str,
    latency: Arc<AtomicU64>,
    hold: Arc<AtomicBool>,
    read: Arc<AtomicU64>,
    written: Arc<AtomicU64>,
    ends: Arc<Mutex<Vec<String>>>,
    started: Instant,
}

impl Tap {
    fn ended(&self, how: String) {
        let at = self.started.elapsed().as_millis();
        if let Ok(mut ends) = self.ends.lock() {
            ends.push(format!("{} ended at {at} ms: {how}", self.dir));
        }
    }
}

impl Proxy {
    fn new(target: SocketAddr, latency_ms: u64) -> Proxy {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let latency = Arc::new(AtomicU64::new(latency_ms));
        let hold = Arc::new(AtomicBool::new(false));
        let (l, h) = (latency.clone(), hold.clone());
        let read = Arc::new(AtomicU64::new(0));
        let written = Arc::new(AtomicU64::new(0));
        let ends = Arc::new(Mutex::new(Vec::new()));
        let started = Instant::now();
        let (r2, w2, e2) = (read.clone(), written.clone(), ends.clone());
        thread::spawn(move || {
            while let Ok((client, _)) = listener.accept() {
                let Ok(server) = TcpStream::connect(target) else {
                    continue;
                };
                for (dir, from, to) in [
                    ("client to relay", client.try_clone().unwrap(), server.try_clone().unwrap()),
                    ("relay to client", server, client),
                ] {
                    let tap = Tap {
                        dir,
                        latency: l.clone(),
                        hold: h.clone(),
                        read: r2.clone(),
                        written: w2.clone(),
                        ends: e2.clone(),
                        started,
                    };
                    pump(from, to, tap);
                }
            }
        });
        Proxy {
            addr,
            hold,
            read,
            written,
            ends,
        }
    }
}

/// One direction: bytes are written no earlier than `latency` after they were
/// read, in order, and not at all while `hold` is set.
fn pump(mut from: TcpStream, mut to: TcpStream, tap: Tap) {
    let (tx, rx) = mpsc::channel::<(Instant, Vec<u8>)>();
    let tap = Arc::new(tap);
    let reader_tap = tap.clone();
    thread::spawn(move || {
        let tap = reader_tap;
        let mut buf = [0u8; 8192];
        loop {
            match from.read(&mut buf) {
                Ok(0) => {
                    tap.ended("the sender closed (end of stream)".into());
                    break;
                }
                Ok(n) => {
                    tap.read.fetch_add(n as u64, Ordering::SeqCst);
                    let due =
                        Instant::now() + Duration::from_millis(tap.latency.load(Ordering::SeqCst));
                    if tx.send((due, buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    tap.ended(format!("read failed: {:?} (os {:?}): {e}", e.kind(), e.raw_os_error()));
                    break;
                }
            }
        }
    });
    thread::spawn(move || {
        while let Ok((due, bytes)) = rx.recv() {
            while Instant::now() < due || tap.hold.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(1));
            }
            if let Err(e) = to.write_all(&bytes) {
                tap.ended(format!("write failed: {:?} (os {:?}): {e}", e.kind(), e.raw_os_error()));
                break;
            }
            tap.written.fetch_add(bytes.len() as u64, Ordering::SeqCst);
        }
        let _ = to.shutdown(std::net::Shutdown::Both);
    });
}

// ---------------------------------------------------------------------------
// Lockstep harness
// ---------------------------------------------------------------------------

fn seat(slot: u8) -> Seat {
    Seat {
        slot: PlayerId(slot),
        team: TeamId(slot),
        race: "fx".into(),
        occupant: Occupant::Peer,
        name: format!("p{slot}"),
        lineage: "Test lineage".into(),
        packs: vec![PackInfo {
            name: "Test pack".into(),
            version: "1".into(),
        }],
    }
}

fn start(peers: u8, ticks_per_turn: u8, delay_turns: u8) -> Start {
    Start {
        seed: 0x5eed,
        map: MapRef {
            name: "fx_two_bases".into(),
            digest: 0,
        },
        options: GameOptions::default(),
        seats: (0..peers).map(seat).collect(),
        host: PlayerId(0),
        turn: TurnConfig {
            ticks_per_turn,
            delay_turns,
            speed_percent: 100,
        },
        rules: RulesDigest([0; 32]),
    }
}

struct Game {
    peers: Vec<Peer>,
    controls: Vec<NetControl>,
    db: Arc<ContentDb>,
    seq: [u32; 256],
    stalls: u64,
    issued: Vec<Option<Tick>>,
    /// Per peer: what its last `step` returned, and how many ticks it ran.
    last_step: Vec<String>,
    ticks_run: Vec<u64>,
    /// Per peer: when it last ran a tick (stall diagnostics only).
    last_ran: Vec<Instant>,
    /// Proxies whose byte counters the stall message reports: (label, read, written).
    watched: Vec<(String, Proxy)>,
    /// The relay's close records, printed by the stall report.
    relay_closes: Option<CloseLogHandle>,
}

/// `proxies[i]` (if any) is the address peer `i` uses instead of the relay's.
fn game(relay: SocketAddr, st: &Start, proxies: &[Option<SocketAddr>]) -> Game {
    let db = Arc::new(sta_content::fake::db());
    let map = Arc::new(sta_content::fake::two_bases());
    let mut peers = Vec::new();
    let mut controls = Vec::new();
    let mut code = None;
    for (i, _) in st.seats.iter().enumerate() {
        let via = proxies.get(i).copied().flatten().unwrap_or(relay);
        let t = match code {
            None => {
                let t = NetTransport::create(via, "", &opts()).unwrap();
                code = Some(t.control().room_code());
                t
            }
            Some(c) => NetTransport::join(via, c, "", &opts()).unwrap(),
        };
        // The relay's seat is the game's seat.
        assert_eq!(t.seat(), PlayerId(i as u8));
        controls.push(t.control());
        peers.push(
            Peer::start(st, PlayerId(i as u8), db.clone(), map.clone(), Box::new(t)).unwrap(),
        );
    }
    let n = peers.len();
    Game {
        peers,
        controls,
        db,
        seq: [0; 256],
        stalls: 0,
        issued: vec![None; n],
        last_step: vec!["never stepped".to_owned(); n],
        ticks_run: vec![0; n],
        last_ran: vec![Instant::now(); n],
        watched: Vec::new(),
        relay_closes: None,
    }
}

impl Game {
    fn next_seq(&mut self, p: PlayerId) -> u32 {
        let s = &mut self.seq[usize::from(p.0)];
        *s += 1;
        *s - 1
    }

    fn spawn_forces(&mut self, per_seat: u32) {
        let cruiser = self.db.key("fx_cruiser").unwrap();
        let frigate = self.db.key("fx_frigate").unwrap();
        for p in 0..self.peers.len() as i32 {
            for k in 0..per_seat as i32 {
                let seq = self.next_seq(PlayerId::SCRIPT);
                let x = if p % 2 == 0 { -150 + 40 * p } else { 150 - 40 * p };
                self.peers[0].submit(Command {
                    player: PlayerId::SCRIPT,
                    seq,
                    kind: CommandKind::Edit(Edit::Spawn {
                        odf: if k % 2 == 0 { cruiser } else { frigate },
                        player: PlayerId(p as u8),
                        pos: Vec3::from_f32([x as f32, 0.0, (k * 70 - 100) as f32]),
                        heading: (Fixed::ZERO, Fixed::ONE),
                    }),
                });
            }
        }
    }

    /// Every ninth tick, each live peer sends one of its units somewhere,
    /// chosen from its own view and a per-peer sequence: deterministic given
    /// the lockstep stream, whatever the wall clock does.
    fn order(&mut self, i: usize) {
        let t = self.peers[i].tick();
        if !t.is_multiple_of(9) || self.issued[i] == Some(t) {
            return;
        }
        self.issued[i] = Some(t);
        let me = PlayerId(i as u8);
        let units: Vec<_> = self.peers[i]
            .sim()
            .view()
            .units
            .iter()
            .filter(|u| u.owner == me)
            .map(|u| u.id)
            .collect();
        if units.is_empty() {
            return;
        }
        let k = (t / 9) as usize;
        let seq = self.next_seq(me);
        self.peers[i].submit(Command {
            player: me,
            seq,
            kind: CommandKind::Order(Order::Move {
                units: vec![units[k % units.len()]],
                to: Vec3::from_f32([((k * 37) % 500) as f32 - 250.0, 0.0, ((k * 91) % 500) as f32 - 250.0]),
                queue: k.is_multiple_of(3),
            }),
        });
    }

    /// Has the stall message report why `relay` closed connections.
    fn watch_relay(&mut self, relay: &Relay) {
        self.relay_closes = Some(relay.close_log());
    }

    /// Has the stall message report `proxy`'s forwarded byte counters.
    fn watch(&mut self, label: &str, proxy: &Proxy) {
        self.watched.push((label.to_owned(), proxy.clone()));
    }

    /// Per peer: tick, the last step outcome, router state and connection
    /// state; per watched proxy: bytes read and written. For the panic message
    /// of a stalled run, so a transport stall can be told from a router one.
    fn stall_report(&self, target: Tick, live: &[usize]) -> String {
        let mut out = format!("target tick {target}\n");
        for &i in live {
            let peer = &self.peers[i];
            let router = peer.router();
            let seats: Vec<String> = (0..self.peers.len() as u8)
                .map(|s| format!("{s}:{:?}", router.last_record(PlayerId(s))))
                .collect();
            out.push_str(&format!(
                "peer {i}: tick {} ({} run here, the last {} ms ago), last step: {}, next turn {}, waiting on {:?}, last record per seat [{}], connection closed: {:?}\n",
                peer.tick(),
                self.ticks_run[i],
                self.last_ran[i].elapsed().as_millis(),
                self.last_step[i],
                router.next_turn(),
                router.waiting_on(),
                seats.join(" "),
                self.controls[i].close_reason(),
            ));
        }
        for (label, proxy) in &self.watched {
            out.push_str(&format!(
                "proxy {label}: {} bytes read, {} bytes written\n",
                proxy.read.load(Ordering::SeqCst),
                proxy.written.load(Ordering::SeqCst)
            ));
            for end in proxy.ends.lock().map(|e| e.clone()).unwrap_or_default() {
                out.push_str(&format!("  {end}\n"));
            }
        }
        if let Some(log) = &self.relay_closes {
            let closes = log.snapshot();
            out.push_str(&format!("relay: {} connections ended\n", closes.len()));
            for c in closes {
                out.push_str(&format!("  {c}\n"));
            }
        }
        out
    }

    /// Steps `live` peers (all when empty) to `target`, sleeping a little when
    /// none can move so the sockets get a turn. The `WAIT` deadline covers the
    /// whole run to `target`, not the time since the last step.
    fn run_to(&mut self, target: Tick, live: &[usize], orders: bool) {
        let live: Vec<usize> = if live.is_empty() {
            (0..self.peers.len()).collect()
        } else {
            live.to_vec()
        };
        let end = Instant::now() + WAIT;
        while live.iter().any(|&i| self.peers[i].tick() < target) {
            let mut progress = false;
            for &i in &live {
                for _ in 0..4 {
                    if self.peers[i].tick() >= target {
                        break;
                    }
                    if orders {
                        self.order(i);
                    }
                    match self.peers[i].step() {
                        StepOutcome::Ran { tick, .. } => {
                            progress = true;
                            self.ticks_run[i] += 1;
                            self.last_ran[i] = Instant::now();
                            self.last_step[i] = format!("Ran to tick {tick}");
                        }
                        StepOutcome::Stalled { waiting } => {
                            self.stalls += 1;
                            self.last_step[i] = format!("Stalled, waiting {waiting:?}");
                            break;
                        }
                        StepOutcome::Halted => panic!("peer {i} halted: a desync"),
                    }
                }
            }
            if !progress {
                assert!(
                    Instant::now() < end,
                    "tick {target} not reached within {WAIT:?}\n{}",
                    self.stall_report(target, &live)
                );
                thread::sleep(Duration::from_millis(1));
            }
        }
    }

    fn assert_in_sync(&self, live: &[usize]) {
        let first = &self.peers[live[0]];
        for &i in live {
            let p = &self.peers[i];
            assert_eq!(p.tick(), first.tick());
            assert_eq!(p.sim().digest(), first.sim().digest(), "peer {i} diverged");
            assert_eq!(p.replay().digests, first.replay().digests);
            assert_eq!(p.replay().commands, first.replay().commands);
            assert!(p.router().desync().is_none() && p.router().peer_desync().is_none());
        }
        // The log replays to the same state on one machine.
        let sim = first.replay().play(self.db.clone()).expect("replay verifies");
        assert_eq!(sim.digest(), first.sim().digest());
    }

    fn protocol_events(&mut self, i: usize) -> Vec<RouterEvent> {
        self.peers[i]
            .router_mut()
            .take_events()
            .into_iter()
            .filter(|e| matches!(e, RouterEvent::Protocol { .. } | RouterEvent::Transport(_)))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Lockstep over the relay
// ---------------------------------------------------------------------------

#[test]
fn two_peers_over_the_relay_stay_identical() {
    let r = relay();
    let mut g = game(r.local_addr(), &start(2, 3, 2), &[]);
    g.spawn_forces(3);
    g.run_to(2_500, &[], true);
    g.assert_in_sync(&[0, 1]);
    // Non-vacuous: many orders were released and they mattered.
    assert!(g.peers[0].replay().commands.len() > 100);
    for i in 0..2 {
        assert!(g.protocol_events(i).is_empty());
    }
}

#[test]
fn three_peers_over_the_relay_stay_identical() {
    let r = relay();
    let mut g = game(r.local_addr(), &start(3, 2, 3), &[]);
    g.spawn_forces(2);
    g.run_to(2_000, &[], true);
    g.assert_in_sync(&[0, 1, 2]);
    assert!(g.peers[0].replay().commands.len() > 100);
    for i in 0..3 {
        assert!(g.protocol_events(i).is_empty());
    }
}

#[test]
fn injected_latency_stalls_but_never_diverges() {
    let r = relay();
    // Peer 1 and 2 sit behind 60 ms and 25 ms one-way delays, both ways.
    let p1 = Proxy::new(r.local_addr(), 60);
    let p2 = Proxy::new(r.local_addr(), 25);
    let mut g = game(
        r.local_addr(),
        &start(3, 3, 2),
        &[None, Some(p1.addr), Some(p2.addr)],
    );
    g.watch_relay(&r);
    g.watch("peer 1 (60 ms)", &p1);
    g.watch("peer 2 (25 ms)", &p2);
    g.spawn_forces(2);
    g.run_to(1_200, &[], true);
    g.assert_in_sync(&[0, 1, 2]);
    // The delay is longer than the input delay covers (2 turns of 100 ms), so
    // the peers really waited.
    assert!(g.stalls > 0, "latency never stalled anyone: the test proves nothing");
    for i in 0..3 {
        assert!(g.protocol_events(i).is_empty());
    }
}

#[test]
fn a_held_connection_stalls_the_others_and_play_resumes_in_sync() {
    let r = relay();
    let proxy = Proxy::new(r.local_addr(), 0);
    let mut g = game(r.local_addr(), &start(3, 3, 2), &[None, None, Some(proxy.addr)]);
    g.spawn_forces(2);
    g.run_to(300, &[], true);
    // Settle before holding: seat 2 sends nothing while it is not stepped, so
    // once peers 0 and 1 hold every record it sent, no byte of seat 2 is in
    // flight and the hold below is exact (not a race with the proxy's pump).
    let sent = g.peers[2].router().last_record(PlayerId(2));
    let end = Instant::now() + WAIT;
    while (0..2).any(|i| g.peers[i].router().last_record(PlayerId(2)) != sent) {
        for i in 0..2 {
            g.peers[i].step();
        }
        assert!(Instant::now() < end, "seat 2's records never all arrived");
        thread::sleep(Duration::from_millis(1));
    }
    proxy.hold.store(true, Ordering::SeqCst);
    // Everyone keeps playing: seat 2 sends more, all of it held; peers 0 and
    // 1 run until they wait on seat 2, and the digest is untouched while they
    // wait.
    let end = Instant::now() + WAIT;
    loop {
        for i in 0..3 {
            g.peers[i].step();
        }
        if g.peers[0].router().waiting_on() == vec![PlayerId(2)]
            && g.peers[1].router().waiting_on() == vec![PlayerId(2)]
        {
            break;
        }
        assert!(Instant::now() < end, "never stalled on the held seat");
        thread::sleep(Duration::from_millis(1));
    }
    // Both finish the turn they are in and stop at the boundary.
    for i in 0..2 {
        let end = Instant::now() + WAIT;
        while !matches!(g.peers[i].step(), StepOutcome::Stalled { .. }) {
            assert!(Instant::now() < end, "peer {i} never reached the turn boundary");
        }
    }
    let frozen = (g.peers[0].tick(), g.peers[0].sim().digest());
    // Nothing of seat 2 can arrive now, however long the others look.
    for _ in 0..75 {
        assert!(matches!(g.peers[0].step(), StepOutcome::Stalled { .. }));
        assert!(matches!(g.peers[1].step(), StepOutcome::Stalled { .. }));
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!((g.peers[0].tick(), g.peers[0].sim().digest()), frozen);
    assert!(g.peers[0].router().desync().is_none());
    // Release: everyone plays on and agrees.
    proxy.hold.store(false, Ordering::SeqCst);
    g.run_to(700, &[], true);
    g.assert_in_sync(&[0, 1, 2]);
}

#[test]
fn a_disconnected_peer_is_dropped_by_the_host_and_the_rest_play_on() {
    let r = relay();
    let mut g = game(r.local_addr(), &start(3, 3, 2), &[]);
    g.spawn_forces(2);
    g.run_to(150, &[], true);
    // Seat 2 vanishes without a word (a crash or a pulled cable).
    g.controls[2].abort();
    let end = Instant::now() + WAIT;
    // The host and seat 1 run until they stall on it, and the host reads the
    // departure marker (after every frame the seat sent).
    // Seat 1 must have read it too: until then a record of seat 2 may still
    // be on its way to it, and "waiting on seat 2" could be a passing state.
    let mut departed = [false; 2];
    while !(departed == [true; 2]
        && g.peers[0].router().waiting_on() == vec![PlayerId(2)]
        && g.peers[1].router().waiting_on() == vec![PlayerId(2)])
    {
        for (i, seen) in departed.iter_mut().enumerate() {
            g.peers[i].step();
            *seen |= g.controls[i].take_departures().contains(&PlayerId(2));
        }
        assert!(Instant::now() < end, "the departure was never seen");
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(g.peers[1].router().waiting_on(), vec![PlayerId(2)]);
    let last = g.peers[0].router().last_record(PlayerId(2)).unwrap();
    // The host drops the seat; the change goes out at once.
    g.peers[0].router_mut().schedule(Action::Drop {
        slot: PlayerId(2),
        last_turn: last,
        then: DropChoice::Eliminate,
    });
    g.peers[0].router_mut().extend_lead();
    g.run_to(g.peers[0].tick() + 600, &[0, 1], true);
    g.assert_in_sync(&[0, 1]);
    for i in 0..2 {
        let dropped = g.peers[i].router_mut().take_events().into_iter().any(|e| {
            matches!(e, RouterEvent::SeatDropped { slot, last_turn, .. }
                if slot == PlayerId(2) && last_turn == last)
        });
        assert!(dropped, "peer {i} did not see the drop");
    }
}

#[test]
fn a_departure_marker_follows_every_frame_the_seat_sent() {
    let r = relay();
    let (mut host, code) = open_room(r.local_addr());
    let mut a = NetTransport::join(r.local_addr(), code, "", &opts()).unwrap();
    let mut b = NetTransport::join(r.local_addr(), code, "", &opts()).unwrap();
    let (ch, cb) = (host.control(), b.control());
    for n in 0..50 {
        a.send(Destination::All, &ping(n)).unwrap();
    }
    a.control().leave();
    // Both others read all fifty, and only then see the marker: whatever the
    // seat sent is before its departure (docs/NET.md, section 8).
    for (t, c) in [(&mut host, &ch), (&mut b, &cb)] {
        for n in 0..50 {
            let m = recv(t);
            assert_eq!((m.from, m.msg), (PlayerId(1), ping(n)));
            assert!(c.take_departures().is_empty(), "marker before frame {n}");
        }
        wait_until(SHORT, "the departure marker", || {
            let _ = t.poll();
            c.take_departures() == vec![PlayerId(1)]
        });
    }
}

#[test]
fn the_host_can_kick_a_seat() {
    let r = relay();
    let (mut host, code) = open_room(r.local_addr());
    let mut a = NetTransport::join(r.local_addr(), code, "", &opts()).unwrap();
    let mut b = NetTransport::join(r.local_addr(), code, "", &opts()).unwrap();
    let (ca, cb, ch) = (a.control(), b.control(), host.control());
    assert_eq!((ca.seat(), cb.seat()), (PlayerId(1), PlayerId(2)));
    ch.kick(PlayerId(1)).unwrap();
    wait_until(SHORT, "the host's departure marker", || {
        let _ = host.poll();
        ch.take_departures() == vec![PlayerId(1)]
    });
    wait_until(SHORT, "seat 2's departure marker", || {
        let _ = b.poll();
        cb.take_departures() == vec![PlayerId(1)]
    });
    wait_until(SHORT, "the kicked client to see the close", || ca.close_reason().is_some());
    assert!(matches!(a.poll(), Err(TransportError::Closed)));
    assert!(a.send(Destination::All, &ping(0)).is_err());
    // Kicking the host or an empty seat does nothing.
    ch.kick(PlayerId(0)).unwrap();
    ch.kick(PlayerId(6)).unwrap();
    b.send(Destination::All, &ping(1)).unwrap();
    assert_eq!(recv(&mut host).msg, ping(1));
}

// ---------------------------------------------------------------------------
// Rooms, stamps, direction
// ---------------------------------------------------------------------------

#[test]
fn the_relay_stamps_the_sender_and_delivers_to_the_destination_only() {
    let r = relay();
    let (mut host, code) = open_room(r.local_addr());
    let mut a = NetTransport::join(r.local_addr(), code, "", &opts()).unwrap();
    let mut b = NetTransport::join(r.local_addr(), code, "", &opts()).unwrap();
    assert!(host.control().is_host() && !a.control().is_host());
    assert_eq!(a.control().host_seat(), PlayerId(0));
    // Broadcast: everyone but the sender.
    a.send(Destination::All, &ping(1)).unwrap();
    let got = recv(&mut host);
    assert_eq!((got.from, got.msg), (PlayerId(1), ping(1)));
    let got = recv(&mut b);
    assert_eq!((got.from, got.msg), (PlayerId(1), ping(1)));
    expect_quiet(&mut a, Duration::from_millis(100));
    // Directed: only the named seat (a rejoin token never reaches others).
    a.send(Destination::Seat(PlayerId(2)), &ping(2)).unwrap();
    let got = recv(&mut b);
    assert_eq!((got.from, got.msg), (PlayerId(1), ping(2)));
    expect_quiet(&mut host, Duration::from_millis(100));
    // Sending to oneself or an absent or impossible seat is refused locally
    // or goes nowhere; never to a third party.
    assert!(a.send(Destination::Seat(PlayerId(1)), &ping(3)).is_err());
    assert!(a.send(Destination::Seat(PlayerId(9)), &ping(3)).is_err());
    a.send(Destination::Seat(PlayerId(5)), &ping(3)).unwrap();
    expect_quiet(&mut host, Duration::from_millis(100));
    expect_quiet(&mut b, Duration::from_millis(100));
    // Per-sender order is kept.
    for n in 0..200 {
        host.send(Destination::All, &ping(n)).unwrap();
    }
    for n in 0..200 {
        assert_eq!(recv(&mut a).msg, ping(n));
        assert_eq!(recv(&mut b).msg, ping(n));
    }
}

#[test]
fn rooms_check_codes_passwords_capacity_and_the_open_flag() {
    let r = relay();
    let host = NetTransport::create(r.local_addr(), "hunter2", &opts()).unwrap();
    let hc = host.control();
    let code = hc.room_code();
    let refused = |res: Result<NetTransport, ConnectError>| match res {
        Err(ConnectError::Refused(why)) => why,
        other => panic!("expected a refusal, got {:?}", other.map(|_| ())),
    };
    assert_eq!(
        refused(NetTransport::join(r.local_addr(), code, "wrong", &opts())),
        RefuseReason::BadPassword
    );
    assert_eq!(
        refused(NetTransport::join(r.local_addr(), code, "", &opts())),
        RefuseReason::BadPassword
    );
    let other = RoomCode::parse("AAAAAA").unwrap();
    assert_eq!(
        refused(NetTransport::join(r.local_addr(), other, "hunter2", &opts())),
        RefuseReason::NoRoom
    );
    // A room holds eight seats.
    let mut members = Vec::new();
    for want in 1..8u8 {
        let t = NetTransport::join(r.local_addr(), code, "hunter2", &opts()).unwrap();
        assert_eq!(t.seat(), PlayerId(want));
        members.push(t);
    }
    assert_eq!(
        refused(NetTransport::join(r.local_addr(), code, "hunter2", &opts())),
        RefuseReason::Full
    );
    // The host closes the room: nobody new, even with a free seat.
    members.pop().unwrap().control().leave();
    hc.set_open(false).unwrap();
    wait_until(SHORT, "the room to close", || {
        matches!(
            NetTransport::join(r.local_addr(), code, "hunter2", &opts()),
            Err(ConnectError::Refused(RefuseReason::Closed))
        )
    });
    // Over-long passwords are not even sent.
    let long = "x".repeat(40);
    assert!(NetTransport::create(r.local_addr(), &long, &opts()).is_err());
}

#[test]
fn a_lost_seat_can_be_taken_back_only_while_it_is_vacated() {
    let r = relay();
    let (mut host, code) = open_room(r.local_addr());
    let mut a = NetTransport::join(r.local_addr(), code, "", &opts()).unwrap();
    let hc = host.control();
    // Not vacated yet, and not closed: refused.
    let busy = NetTransport::rejoin(r.local_addr(), code, "", PlayerId(1), &opts());
    assert!(matches!(busy, Err(ConnectError::Refused(RefuseReason::BadReclaim))));
    hc.set_open(false).unwrap();
    // The close and the abort travel on different connections, which the
    // relay serves independently: if seat 1 vanished before the relay had
    // read the close, the seat would be freed (the room was open), not held
    // for a rejoin. The relay reads the host's messages in order, so a frame
    // from the host that has reached seat 1 proves the close was applied.
    host.send(Destination::Seat(PlayerId(1)), &ping(8)).unwrap();
    assert_eq!(recv(&mut a).msg, ping(8));
    a.control().abort();
    drop(a);
    wait_until(SHORT, "the departure", || {
        let _ = host.poll();
        hc.take_departures() == vec![PlayerId(1)]
    });
    // A newcomer may not take it; the rejoin may.
    assert!(matches!(
        NetTransport::join(r.local_addr(), code, "", &opts()),
        Err(ConnectError::Refused(RefuseReason::Closed))
    ));
    let mut back = NetTransport::rejoin(r.local_addr(), code, "", PlayerId(1), &opts()).unwrap();
    assert_eq!(back.seat(), PlayerId(1));
    // The host hears from the seat again, stamped with the same seat.
    back.send(Destination::Seat(PlayerId(0)), &ping(9)).unwrap();
    let got = recv(&mut host);
    assert_eq!((got.from, got.msg), (PlayerId(1), ping(9)));
    // And a second rejoin of the same seat is refused while it is held.
    assert!(matches!(
        NetTransport::rejoin(r.local_addr(), code, "", PlayerId(1), &opts()),
        Err(ConnectError::Refused(RefuseReason::BadReclaim))
    ));
}

#[test]
fn only_the_host_may_kick_or_close_the_room() {
    let r = relay();
    let (mut host, code) = open_room(r.local_addr());
    let a = NetTransport::join(r.local_addr(), code, "", &opts()).unwrap();
    let mut b = NetTransport::join(r.local_addr(), code, "", &opts()).unwrap();
    // A non-host's kick is a protocol violation: the relay closes it.
    a.control().kick(PlayerId(2)).unwrap();
    wait_until(SHORT, "the offender to be closed", || a.control().close_reason().is_some());
    // Seat 2 is untouched, the host sees the offender leave.
    let hc = host.control();
    wait_until(SHORT, "the departure", || {
        let _ = host.poll();
        hc.take_departures().contains(&PlayerId(1))
    });
    assert!(b.control().close_reason().is_none());
    b.send(Destination::All, &ping(1)).unwrap();
    assert_eq!(recv(&mut host).msg, ping(1));
}

#[test]
fn the_host_leaving_ends_the_room_for_newcomers_and_tells_everyone() {
    let r = relay();
    let (host, code) = open_room(r.local_addr());
    let mut a = NetTransport::join(r.local_addr(), code, "", &opts()).unwrap();
    let ac = a.control();
    host.control().leave();
    drop(host);
    wait_until(SHORT, "the host's departure", || {
        let _ = a.poll();
        ac.take_departures() == vec![PlayerId(0)]
    });
    assert!(matches!(
        NetTransport::join(r.local_addr(), code, "", &opts()),
        Err(ConnectError::Refused(RefuseReason::Closed))
    ));
    drop(a);
    wait_until(SHORT, "the room to be removed", || r.rooms() == 0);
    wait_until(SHORT, "the connections to be released", || r.connections() == 0);
}

#[test]
fn shutting_the_relay_down_closes_every_client() {
    let mut r = relay();
    let (mut host, _) = open_room(r.local_addr());
    r.shutdown();
    wait_until(SHORT, "the client to notice", || host.control().close_reason().is_some());
    assert!(matches!(host.poll(), Err(TransportError::Closed)));
}

// ---------------------------------------------------------------------------
// Hostile clients against the relay
// ---------------------------------------------------------------------------

fn raw(addr: SocketAddr) -> TcpStream {
    let s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    s
}

/// True once the relay has closed the stream (EOF or reset), within `limit`.
fn closed_by_peer(s: &mut TcpStream, limit: Duration) -> bool {
    let end = Instant::now() + limit;
    let mut buf = [0u8; 4096];
    while Instant::now() < end {
        match s.read(&mut buf) {
            Ok(0) => return true,
            Ok(_) => {}
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => return true,
        }
    }
    false
}

/// Reads envelopes until `want` matches or the stream ends.
fn read_server(s: &mut TcpStream, limit: Duration) -> Option<ServerMsg> {
    let end = Instant::now() + limit;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    while Instant::now() < end {
        if buf.len() >= 4 {
            let n = u32::from_le_bytes(buf[..4].try_into().unwrap()) as usize;
            if buf.len() >= 4 + n {
                return ServerMsg::decode(&buf[4..4 + n]).ok();
            }
        }
        match s.read(&mut chunk) {
            Ok(0) => return None,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => return None,
        }
    }
    None
}

fn raw_join(addr: SocketAddr, code: RoomCode) -> TcpStream {
    let mut s = raw(addr);
    let m = ClientMsg::Join {
        code,
        reclaim: None,
        password: vec![],
    };
    s.write_all(&m.encode().unwrap()).unwrap();
    let answer = read_server(&mut s, SHORT);
    assert!(matches!(answer, Some(ServerMsg::Joined { .. })), "join answered {answer:?}");
    s
}

fn quick_opts() -> ClientOptions {
    ClientOptions {
        idle_timeout: Duration::from_millis(2000),
        ..opts()
    }
}

fn fast_cfg() -> RelayConfig {
    RelayConfig {
        throttle_limit: Duration::from_millis(500),
        handshake_timeout: Duration::from_millis(600),
        idle_timeout: Duration::from_millis(2000),
        frame_timeout: Duration::from_millis(600),
        ..RelayConfig::default()
    }
}

/// After any abuse the relay still serves a healthy room.
///
/// Its clients must keepalive faster than the fast relay's idle limit (the
/// default client sends one every 5 s, the fast relay cuts at 2 s), or a
/// slow runner that stalls the test for two seconds loses the host.
fn assert_relay_healthy(addr: SocketAddr) {
    let (mut host, code) = open_room_with(addr, &quick_opts());
    let mut a = NetTransport::join(addr, code, "", &quick_opts()).unwrap();
    a.send(Destination::All, &ping(5)).unwrap();
    assert_eq!(recv(&mut host).msg, ping(5));
}

fn junk_frame_header(magic: &[u8; 4], version: u16, len: u32) -> Vec<u8> {
    let mut f = magic.to_vec();
    f.extend_from_slice(&version.to_le_bytes());
    f.extend_from_slice(&len.to_le_bytes());
    f
}

fn envelope(payload: &[u8]) -> Vec<u8> {
    let mut v = (payload.len() as u32).to_le_bytes().to_vec();
    v.extend_from_slice(payload);
    v
}

#[test]
fn malformed_first_messages_are_refused_without_panic_or_allocation() {
    let r = relay_with(fast_cfg());
    let big = (MAX_ENVELOPE_BYTES as u32 + 1).to_le_bytes();
    let table: Vec<(&str, Vec<u8>)> = vec![
        ("empty envelope", envelope(&[])),
        ("length 4 GiB", u32::MAX.to_le_bytes().to_vec()),
        ("length just over the limit", big.to_vec()),
        ("limit then junk", {
            let mut v = big.to_vec();
            v.extend(vec![0xAB; 70_000]);
            v
        }),
        ("unknown kind", envelope(&[0xEE, 1, 2, 3])),
        ("data before a room", envelope(&[3, 0xFF])),
        ("keepalive before a room", envelope(&[4])),
        ("create with the wrong version", envelope(&[1, 99, 0])),
        ("create with a short password", envelope(&[1, 1, 5, b'a'])),
        ("create with a long password", {
            let mut p = vec![1, 1, 40];
            p.extend(vec![b'a'; 40]);
            envelope(&p)
        }),
        ("join with a lowercase code", envelope(&[2, 1, b'a', b'b', b'c', b'd', b'e', b'f', 0xFF, 0])),
        ("join with a bad seat", envelope(&[2, 1, b'A', b'A', b'A', b'A', b'A', b'A', 9, 0])),
        ("trailing bytes", envelope(&[1, 1, 0, 0, 0])),
        ("http", b"GET / HTTP/1.1\r\nHost: x\r\n\r\n".to_vec()),
        ("a proto frame, not an envelope", encode(&ping(1)).unwrap()),
        ("zeros", vec![0u8; 5000]),
    ];
    for (name, bytes) in table {
        let mut s = raw(r.local_addr());
        let _ = s.write_all(&bytes);
        assert!(closed_by_peer(&mut s, SHORT), "not closed: {name}");
    }
    assert_relay_healthy(r.local_addr());
    wait_until(SHORT, "the connections to be released", || r.connections() <= 2);
}

#[test]
fn bad_frames_inside_data_envelopes_close_the_sender() {
    let r = relay_with(fast_cfg());
    let (mut host, code) = open_room_with(r.local_addr(), &quick_opts());
    let ok = encode(&ping(1)).unwrap();
    let mut long = ok.clone();
    long.push(0);
    let table: Vec<(&str, Vec<u8>)> = vec![
        ("no frame", vec![3, 0xFF]),
        ("short header", [vec![3, 0xFF], ok[..6].to_vec()].concat()),
        ("bad magic", [vec![3, 0xFF], junk_frame_header(b"NOPE", 1, 0)].concat()),
        ("declared length too big", [vec![3, 0xFF], junk_frame_header(b"S64N", 1, 70_000)].concat()),
        ("declared length 4 GiB", [vec![3, 0xFF], junk_frame_header(b"S64N", 1, u32::MAX)].concat()),
        ("declared longer than sent", [vec![3, 0xFF], junk_frame_header(b"S64N", 1, 50)].concat()),
        ("longer than declared", [vec![3, 0xFF], long].concat()),
    ];
    for (name, payload) in table {
        let mut s = raw_join(r.local_addr(), code);
        s.write_all(&envelope(&payload)).unwrap();
        assert!(closed_by_peer(&mut s, SHORT), "not closed: {name}");
        // Nothing of it reached the host.
        expect_quiet(&mut host, Duration::from_millis(20));
    }
    assert_relay_healthy(r.local_addr());
}

#[test]
fn a_frame_with_a_good_header_and_a_bad_body_is_the_receivers_to_refuse() {
    // The relay reads headers only, so it forwards this; the receiving
    // transport refuses it as a frame error and goes on.
    let r = relay();
    let (mut host, code) = open_room(r.local_addr());
    let mut s = raw_join(r.local_addr(), code);
    let mut frame = junk_frame_header(b"S64N", 1, 6);
    frame.extend_from_slice(&[0xFF; 6]);
    s.write_all(&ClientMsg::Data { mask: 1, frame }.encode().unwrap()).unwrap();
    let good = encode(&Message::Ping { nonce: 77 }).unwrap();
    s.write_all(&ClientMsg::Data { mask: 1, frame: good }.encode().unwrap()).unwrap();
    let end = Instant::now() + SHORT;
    let mut saw_error = false;
    loop {
        match host.poll() {
            Err(TransportError::Frame(_)) => saw_error = true,
            Ok(Some(m)) => {
                assert_eq!(m.msg, ping(77));
                break;
            }
            Ok(None) => {}
            Err(e) => panic!("{e}"),
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(saw_error, "the bad body was accepted");
}

#[test]
fn slow_silent_and_flooding_clients_are_cut_off() {
    let r = relay_with(RelayConfig {
        bytes_per_sec: 20_000,
        burst_bytes: 80_000,
        ..fast_cfg()
    });
    // The host's own keepalives must beat the relay's short idle limit.
    let quick = ClientOptions {
        idle_timeout: Duration::from_millis(2000),
        ..opts()
    };
    let mut host = NetTransport::create(r.local_addr(), "", &quick).unwrap();
    let code = host.control().room_code();

    // Never says anything: the handshake deadline.
    let mut s = raw(r.local_addr());
    assert!(closed_by_peer(&mut s, SHORT), "silent connection kept open");

    // Half a header, then nothing: the frame deadline (also the handshake's).
    let mut s = raw(r.local_addr());
    s.write_all(&[9, 0]).unwrap();
    assert!(closed_by_peer(&mut s, SHORT), "half an envelope kept open");

    // Joined, then half an envelope: the frame deadline.
    let mut s = raw_join(r.local_addr(), code);
    s.write_all(&[200, 0, 0]).unwrap();
    assert!(closed_by_peer(&mut s, SHORT), "stalled envelope kept open");

    // Joined, then silent: the idle deadline (keepalives would keep it).
    let mut s = raw_join(r.local_addr(), code);
    let end = Instant::now() + SHORT;
    let mut got_keepalive_free_close = false;
    let mut buf = [0u8; 256];
    while Instant::now() < end {
        match s.read(&mut buf) {
            Ok(0) => {
                got_keepalive_free_close = true;
                break;
            }
            Ok(_) => {} // the relay's own keepalives
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => {
                got_keepalive_free_close = true;
                break;
            }
        }
    }
    assert!(got_keepalive_free_close, "idle connection kept open");

    // A flood beyond the byte rate is throttled (not read faster than the
    // budget refills), so the host sees a fraction of it.
    let mut s = raw_join(r.local_addr(), code);
    let frame = encode(&Message::Ping { nonce: 1 }).unwrap();
    let data = ClientMsg::Data { mask: 1, frame }.encode().unwrap();
    let began = Instant::now();
    for _ in 0..20_000 {
        if s.write_all(&data).is_err() {
            break;
        }
    }
    thread::sleep(Duration::from_millis(1500));
    let mut got = 0u64;
    while matches!(host.poll(), Ok(Some(_))) {
        got += 1;
    }
    // Budget: the byte burst (80 kB) plus 20 kB/s at about 30 bytes a frame,
    // or the frame budget (400 plus 200 a second), whichever is tighter.
    let secs = began.elapsed().as_secs_f64();
    let allowed = (((80_000.0 + 20_000.0 * secs) / 30.0) as u64).min((400.0 + 200.0 * secs) as u64);
    assert!(got > 0 && got <= allowed + 200, "{got} frames got through, budget {allowed}");
    assert!(got < 20_000);
    drop(s);

    // A keepalive-sending client is not idle.
    let t = NetTransport::join(r.local_addr(), code, "", &ClientOptions {
        idle_timeout: Duration::from_millis(2000),
        ..opts()
    })
    .unwrap();
    thread::sleep(Duration::from_millis(2_500));
    assert!(t.control().close_reason().is_none(), "{:?}", t.control().close_reason());
    assert_relay_healthy(r.local_addr());
}

#[test]
fn limits_per_address_hold() {
    let r = relay_with(RelayConfig {
        max_connections_per_ip: 3,
        max_rooms_per_ip: 2,
        failed_joins: 3,
        ..fast_cfg()
    });
    // Rooms per address.
    let a = NetTransport::create(r.local_addr(), "", &quick_opts()).unwrap();
    let b = NetTransport::create(r.local_addr(), "", &quick_opts()).unwrap();
    let c = NetTransport::create(r.local_addr(), "", &quick_opts());
    assert!(matches!(c, Err(ConnectError::Refused(RefuseReason::Busy))), "a third room");
    // Connections per address: two are open; a third works, a fourth is
    // dropped before it can say anything.
    let code = a.control().room_code();
    // The refused third room's connection is released a moment after its
    // refusal is written; admission of the next one counts it until then.
    wait_until(SHORT, "the refused connection to be released", || r.connections() <= 2);
    let third = NetTransport::join(r.local_addr(), code, "", &quick_opts());
    assert!(third.is_ok());
    let mut s = TcpStream::connect(r.local_addr()).unwrap();
    s.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    assert!(closed_by_peer(&mut s, SHORT), "a fourth connection from one address");
    drop((third, b));
    wait_until(SHORT, "connections to drain", || r.connections() <= 1);
    // Guessing codes is slowed: three failures, then refused outright, even
    // for a correct code.
    let bad = RoomCode::parse("ZZZZZZ").unwrap();
    for _ in 0..3 {
        wait_until(SHORT, "the refused connection to be released", || r.connections() <= 1);
        assert!(matches!(
            NetTransport::join(r.local_addr(), bad, "", &quick_opts()),
            Err(ConnectError::Refused(RefuseReason::NoRoom))
        ));
    }
    wait_until(SHORT, "the refused connection to be released", || r.connections() <= 1);
    assert!(matches!(
        NetTransport::join(r.local_addr(), code, "", &quick_opts()),
        Err(ConnectError::Refused(RefuseReason::RateLimited))
    ));
}

// ---------------------------------------------------------------------------
// The client against a hostile or dead relay
// ---------------------------------------------------------------------------

/// A fake relay that accepts one connection, reads the request and runs
/// `script` on the stream.
fn fake_relay(script: impl FnOnce(&mut TcpStream) + Send + 'static) -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    thread::spawn(move || {
        if let Ok((mut s, _)) = l.accept() {
            let mut buf = [0u8; 256];
            let _ = s.read(&mut buf);
            script(&mut s);
        }
    });
    addr
}

fn created() -> Vec<u8> {
    ServerMsg::Created {
        code: RoomCode::parse("K7MQ2X").unwrap(),
        seat: 0,
    }
    .encode()
    .unwrap()
}

/// Gives up on a silent relay after 600 ms; the handshake deadline is long so
/// a slow runner cannot trip it before the fake relay has answered.
fn short_client() -> ClientOptions {
    ClientOptions {
        handshake_timeout: Duration::from_secs(5),
        idle_timeout: Duration::from_millis(600),
        frame_timeout: Duration::from_millis(600),
        ..opts()
    }
}

#[test]
fn the_client_gives_up_on_a_relay_that_never_answers() {
    let addr = fake_relay(|_| thread::sleep(Duration::from_secs(3)));
    let started = Instant::now();
    let res = NetTransport::create(addr, "", &ClientOptions {
        handshake_timeout: Duration::from_millis(600),
        ..short_client()
    });
    assert!(matches!(res, Err(ConnectError::Timeout)));
    assert!(started.elapsed() < Duration::from_secs(5));
    // Nothing listening at all.
    let dead = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    assert!(matches!(
        NetTransport::create(dead, "", &short_client()),
        Err(ConnectError::Io(_))
    ));
}

#[test]
fn the_client_loses_a_connection_that_goes_silent() {
    let addr = fake_relay(|s| {
        let _ = s.write_all(&created());
        thread::sleep(Duration::from_secs(5));
    });
    let t = NetTransport::create(addr, "", &short_client()).unwrap();
    wait_until(SHORT, "the idle timeout", || t.control().close_reason().is_some());
    assert_eq!(t.control().close_reason(), Some(CloseReason::TimedOut));
}

#[test]
fn the_client_refuses_garbage_from_the_relay() {
    let hostile: Vec<(&str, Vec<u8>)> = vec![
        ("oversized length", u32::MAX.to_le_bytes().to_vec()),
        ("zero length", envelope(&[])),
        ("unknown kind", envelope(&[0xEE])),
        ("data with a bad frame", envelope(&[4, 1, 0, 1, 2, 3])),
        ("data with a bad sender", envelope(&[4, 200, 0])),
        ("a second Created", created()),
        ("trailing bytes", envelope(&[6, 0])),
    ];
    for (name, bytes) in hostile {
        let addr = fake_relay({
            let bytes = bytes.clone();
            move |s| {
                let _ = s.write_all(&created());
                let _ = s.write_all(&bytes);
                thread::sleep(Duration::from_secs(3));
            }
        });
        // A long idle limit: the garbage must be what ends the connection.
        let patient = ClientOptions {
            idle_timeout: Duration::from_secs(30),
            ..short_client()
        };
        let mut t = NetTransport::create(addr, "", &patient).unwrap();
        wait_until(SHORT, name, || t.control().close_reason().is_some());
        assert!(
            matches!(t.control().close_reason(), Some(CloseReason::Protocol(_))),
            "{name}: {:?}",
            t.control().close_reason()
        );
        assert!(matches!(t.poll(), Err(TransportError::Closed)));
    }
    // Garbage in place of the answer.
    let addr = fake_relay(|s| {
        let _ = s.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n");
    });
    assert!(matches!(
        NetTransport::create(addr, "", &short_client()),
        Err(ConnectError::Protocol(_))
    ));
}

#[test]
fn a_client_that_does_not_poll_is_closed_instead_of_buffering_without_bound() {
    let r = relay();
    let small = ClientOptions {
        max_inbox_bytes: 4 * 1024,
        ..opts()
    };
    let host = NetTransport::create(r.local_addr(), "", &opts()).unwrap();
    let code = host.control().room_code();
    let slow = NetTransport::join(r.local_addr(), code, "", &small).unwrap();
    let mut host = host;
    for n in 0..400 {
        host.send(Destination::All, &ping(n)).unwrap();
    }
    wait_until(SHORT, "the inbox limit", || slow.control().close_reason().is_some());
    assert_eq!(slow.control().close_reason(), Some(CloseReason::InboxFull));
}

#[test]
fn a_sender_that_the_relay_cannot_drain_is_closed() {
    // A relay that never reads: the send queue fills and the transport closes
    // instead of growing it.
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    thread::spawn(move || {
        if let Ok((mut s, _)) = l.accept() {
            let mut buf = [0u8; 256];
            let _ = s.read(&mut buf);
            let _ = s.write_all(&created());
            while !s2.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(20));
            }
        }
    });
    let mut t = NetTransport::create(addr, "", &ClientOptions {
        outgoing_queue: 16,
        ..opts()
    })
    .unwrap();
    let big = Message::Chat(sta_net::proto::Chat {
        from: PlayerId(0),
        scope: sta_net::proto::ChatScope::All,
        recipients: 0b10,
        text: "x".repeat(200),
    });
    let end = Instant::now() + SHORT;
    let mut failed = false;
    while Instant::now() < end && !failed {
        failed = t.send(Destination::All, &big).is_err();
    }
    stop.store(true, Ordering::SeqCst);
    assert!(failed, "the send queue grew without bound");
    assert!(t.control().close_reason().is_some());
}

// ---------------------------------------------------------------------------
// The router under a flooding peer (state bounds that NET-02 deferred)
// ---------------------------------------------------------------------------

struct Mute;
impl Transport for Mute {
    fn send(&mut self, _: Destination, _: &Message) -> Result<(), TransportError> {
        Ok(())
    }
    fn poll(&mut self) -> Result<Option<sta_net::lockstep::Incoming>, TransportError> {
        Ok(None)
    }
}

fn record(turn: u32, slot: u8, digest: Option<TickDigest>) -> Message {
    Message::Turn(TurnCommands {
        turn,
        slot: PlayerId(slot),
        commands: vec![],
        schedule: vec![],
        digest,
    })
}

#[test]
fn a_flooding_peer_cannot_grow_the_routers_state_without_bound() {
    let st = start(2, 3, 2);
    let mut router = LockstepRouter::new(&st, PlayerId(0), Box::new(Mute)).unwrap();
    // Records far ahead of anything consumed: refused after the window.
    let mut refused = 0;
    for turn in 2..2 + 100_000u32 {
        match router.receive(PlayerId(1), record(turn, 1, None)) {
            Ok(()) => {}
            Err(RouterError::TooFarAhead(_)) => {
                refused += 1;
                break;
            }
            Err(e) => panic!("{e}"),
        }
    }
    assert_eq!(refused, 1);
    // The seat is faulted from then on.
    assert!(router.receive(PlayerId(1), record(3, 1, None)).is_err());
    const { assert!(MAX_QUEUED_TURNS < 1000) };

    // Digests for ticks nobody has reached: nothing is kept or reported.
    let mut router = LockstepRouter::new(&st, PlayerId(0), Box::new(Mute)).unwrap();
    for i in 0..5_000u32 {
        let d = TickDigest {
            tick: u64::from(i) * 3 + 100_000,
            digest: u64::from(i),
        };
        // Each in its own record would be gap-free only once; the digest
        // path is the same for a Desync notice, which has no order rule.
        let _ = router.receive(
            PlayerId(1),
            Message::Desync(sta_net::proto::DesyncNotice {
                slot: PlayerId(1),
                ours: d,
                theirs: d,
            }),
        );
        assert!(router.digests_at(d.tick).is_empty());
    }

    // Errors from a hostile peer do not pile up as unread events.
    for _ in 0..5_000 {
        let _ = router.receive(PlayerId(1), Message::Ping { nonce: 0 });
        let _ = router.receive(PlayerId(1), Message::Welcome(sta_net::proto::Welcome {
            slot: PlayerId(1),
            host: PlayerId(0),
            rejoin_token: 0,
        }));
    }
    assert!(router.take_events().len() <= MAX_EVENTS);
}

#[test]
fn frame_size_constants_agree_with_the_envelope() {
    assert_eq!(MAX_ENVELOPE_BYTES, MAX_FRAME_BYTES + 8);
    const { assert!(HEADER_LEN < MAX_FRAME_BYTES) };
    let _ = Action::Pause;
    let _ = Fixed::ONE;
    let _ = Duration::ZERO;
}

// ---------------------------------------------------------------------------
// Why the relay closed a connection
// ---------------------------------------------------------------------------

/// The newest close record that matches, once the relay has made one (the
/// connection's thread records after the peer already saw the close).
fn close_where(r: &Relay, what: &str, pred: impl Fn(&CloseRecord) -> bool) -> CloseRecord {
    let mut found = None;
    wait_until(SHORT, what, || {
        found = r.closes().into_iter().rev().find(|c| pred(c));
        found.is_some()
    });
    found.unwrap()
}

#[test]
fn a_zero_or_oversized_length_prefix_is_recorded_with_its_value() {
    let r = relay();
    let mut s = raw(r.local_addr());
    s.write_all(&0u32.to_le_bytes()).unwrap();
    let c = close_where(&r, "the zero length", |c| c.cause == CloseCause::BadLength(0));
    assert_eq!(c.seat, None);
    assert_eq!(c.unparsed, 4);
    let big = MAX_ENVELOPE_BYTES + 1;
    let mut s = raw(r.local_addr());
    s.write_all(&(big as u32).to_le_bytes()).unwrap();
    close_where(&r, "the oversized length", |c| c.cause == CloseCause::BadLength(big));
}

#[test]
fn an_envelope_that_does_not_decode_is_recorded_with_the_decoders_text() {
    let r = relay();
    let mut s = raw(r.local_addr());
    s.write_all(&envelope(&[0xEE, 1, 2, 3])).unwrap();
    let c = close_where(&r, "the decode failure", |c| {
        matches!(c.cause, CloseCause::DecodeFailed(_))
    });
    let CloseCause::DecodeFailed(text) = &c.cause else { unreachable!() };
    assert!(text.contains("kind"), "{text}");
    // A bad frame inside a data envelope, from a seated client.
    let (_host, code) = open_room(r.local_addr());
    let mut s = raw_join(r.local_addr(), code);
    s.write_all(&envelope(&[3, 0xFF])).unwrap();
    let c = close_where(&r, "the bad frame", |c| {
        c.seat == Some(1) && matches!(c.cause, CloseCause::DecodeFailed(_))
    });
    assert_eq!(c.unparsed, 0);
}

#[test]
fn a_client_that_closes_is_recorded_clean_or_with_its_half_envelope() {
    let r = relay();
    let (_host, code) = open_room(r.local_addr());
    drop(raw_join(r.local_addr(), code));
    let c = close_where(&r, "the clean close", |c| c.seat == Some(1));
    assert_eq!(c.cause, CloseCause::ClientClosed);
    assert_eq!(c.unparsed, 0);
    // Two bytes of a length prefix, then gone.
    let mut s = raw_join(r.local_addr(), code);
    s.write_all(&[9, 0]).unwrap();
    // Let a read see them before the close.
    thread::sleep(Duration::from_millis(150));
    drop(s);
    let c = close_where(&r, "the close with unparsed bytes", |c| c.unparsed == 2);
    assert_eq!(c.cause, CloseCause::ClientClosed);
    assert!(c.reads.timeouts >= 1, "a connection idle for 150 ms saw no read timeout");
    // The two bytes were held across them: the hazard window is counted.
    assert!(c.reads.partial_timeouts >= 1, "{c}");
    assert!(c.reads.partial_timeouts <= c.reads.timeouts);
    assert_eq!(c.reads.spurious, 0);
    assert_eq!(c.reads.partial_spurious, 0);
}

#[test]
fn protocol_misuse_is_recorded_by_kind() {
    let r = relay();
    // Not a create or join first.
    let mut s = raw(r.local_addr());
    s.write_all(&envelope(&[4])).unwrap();
    close_where(&r, "a keepalive first", |c| {
        c.cause == CloseCause::Misuse(Misuse::NotOpenOrJoin)
    });
    // Create twice.
    let mut s = raw(r.local_addr());
    let create = ClientMsg::Create { password: vec![] }.encode().unwrap();
    s.write_all(&create).unwrap();
    assert!(matches!(read_server(&mut s, SHORT), Some(ServerMsg::Created { .. })));
    s.write_all(&create).unwrap();
    close_where(&r, "a second create", |c| {
        c.cause == CloseCause::Misuse(Misuse::RepeatedOpenOrJoin) && c.seat == Some(0)
    });
    // A non-host's kick.
    let (_host, code) = open_room(r.local_addr());
    let mut s = raw_join(r.local_addr(), code);
    s.write_all(&ClientMsg::Kick(2).encode().unwrap()).unwrap();
    let c = close_where(&r, "a non-host kick", |c| c.cause == CloseCause::Misuse(Misuse::HostOnly));
    assert_eq!(c.seat, Some(1));
}

#[test]
fn leaving_a_kick_and_a_refusal_are_recorded() {
    let r = relay();
    let (host, code) = open_room(r.local_addr());
    let mut a = raw_join(r.local_addr(), code);
    a.write_all(&ClientMsg::Leave.encode().unwrap()).unwrap();
    close_where(&r, "a leave", |c| c.cause == CloseCause::ClientLeft && c.seat == Some(1));
    let b = raw_join(r.local_addr(), code);
    host.control().kick(PlayerId(1)).unwrap();
    let c = close_where(&r, "a kick", |c| c.cause == CloseCause::Kicked);
    assert_eq!(c.seat, Some(1));
    drop(b);
}

#[test]
fn refused_and_dropped_before_a_seat_are_counted_and_summarised_not_logged_each() {
    let (mut r, lines) = relay_logging(RelayConfig {
        max_connections_per_ip: 1,
        // No periodic summary mid-test, however slow the runner: the one
        // line comes with the shutdown.
        log_summary_every: Duration::from_secs(3600),
        ..RelayConfig::default()
    });
    // A join to a room that does not exist is refused.
    let mut s = raw(r.local_addr());
    let m = ClientMsg::Join {
        code: RoomCode::random().unwrap(),
        reclaim: None,
        password: vec![],
    };
    s.write_all(&m.encode().unwrap()).unwrap();
    wait_until(SHORT, "the refusal", || r.unseated_pending() == 1);
    wait_until(SHORT, "the slot to be free", || r.connections() == 0);
    // One admitted connection now holds the only slot.
    let _keep = raw(r.local_addr());
    wait_until(SHORT, "the admission", || r.connections() == 1);
    // A thousand attempts over the limit, in batches the accept backlog can
    // take (a full backlog only delays a connection, but by whole seconds).
    for batch in 1..=20u64 {
        for _ in 0..50 {
            drop(TcpStream::connect(r.local_addr()).unwrap());
        }
        wait_until(SHORT, "the batch to be counted", || r.unseated_pending() >= 1 + batch * 50);
    }
    assert_eq!(r.unseated_pending(), 1001);
    assert!(lines.lock().unwrap().is_empty(), "{:?}", lines.lock().unwrap());
    // Not kept in the bounded list either: they would push out real records.
    assert!(r.closes().is_empty());
    r.shutdown();
    let lines = lines.lock().unwrap();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains("1001 connections")
            && lines[0].contains("limit: 1000")
            && lines[0].contains("join refused: 1"),
        "{}",
        lines[0]
    );
}

#[test]
fn a_blocked_log_does_not_hold_a_departure_or_a_connection_count() {
    let release = Arc::new(AtomicBool::new(false));
    let entered = Arc::new(AtomicBool::new(false));
    let (rel, ent) = (release.clone(), entered.clone());
    let sink: LogSink = Arc::new(move |_| {
        ent.store(true, Ordering::SeqCst);
        let end = Instant::now() + SHORT;
        while !rel.load(Ordering::SeqCst) && Instant::now() < end {
            thread::sleep(Duration::from_millis(2));
        }
    });
    let r = Relay::bind_with_log("127.0.0.1:0", RelayConfig::default(), sink).unwrap();
    let (mut host, code) = open_room(r.local_addr());
    let hc = host.control();
    let mut s = raw_join(r.local_addr(), code);
    // A seated connection that fails to decode is logged (the sink blocks).
    s.write_all(&envelope(&[3, 0xFF])).unwrap();
    wait_until(SHORT, "the log write to start", || entered.load(Ordering::SeqCst));
    // While it is blocked the seat is already gone and released.
    wait_until(SHORT, "the departure", || {
        let _ = host.poll();
        hc.take_departures().contains(&PlayerId(1))
    });
    wait_until(SHORT, "the connection count", || r.connections() == 1);
    assert!(r.closes().is_empty(), "recorded before the log write returned");
    release.store(true, Ordering::SeqCst);
    close_where(&r, "the record", |c| matches!(c.cause, CloseCause::DecodeFailed(_)));
}

#[test]
fn the_deadlines_are_recorded() {
    // The idle deadline is long: it is checked before the half-sent
    // envelope's, and must not be able to win on a starved runner.
    let r = relay_with(RelayConfig {
        idle_timeout: Duration::from_secs(30),
        ..fast_cfg()
    });
    let _silent = raw(r.local_addr());
    close_where(&r, "the handshake deadline", |c| c.cause == CloseCause::HandshakeTimeout);
    let (_host, code) = open_room(r.local_addr());
    let mut s = raw_join(r.local_addr(), code);
    s.write_all(&[9, 0]).unwrap();
    let c = close_where(&r, "the half-sent envelope deadline", |c| {
        c.cause == CloseCause::FrameTimeout
    });
    assert_eq!(c.unparsed, 2);
    assert_eq!(c.seat, Some(1));
}

#[test]
fn shutting_down_is_recorded() {
    let mut r = relay();
    let log = r.close_log();
    let (_host, _) = open_room(r.local_addr());
    r.shutdown();
    wait_until(SHORT, "the shutdown record", || {
        log.snapshot().iter().any(|c| c.cause == CloseCause::RelayShutdown)
    });
}

#[test]
fn the_close_records_can_be_read_while_connections_come_and_go() {
    let r = relay();
    let log = r.close_log();
    let stop = Arc::new(AtomicBool::new(false));
    let reader = {
        let stop = stop.clone();
        thread::spawn(move || {
            let mut most = 0;
            while !stop.load(Ordering::SeqCst) {
                most = most.max(log.snapshot().len());
            }
            most
        })
    };
    let addr = r.local_addr();
    for _ in 0..(CLOSE_LOG / 2) {
        drop(raw(addr));
    }
    wait_until(SHORT, "the records", || r.closes().len() == CLOSE_LOG / 2);
    stop.store(true, Ordering::SeqCst);
    assert!(reader.join().unwrap() <= CLOSE_LOG);
    // Never more than the bound, however many connections ended.
    for _ in 0..CLOSE_LOG {
        drop(raw(addr));
    }
    wait_until(SHORT, "the log to fill", || r.closes().len() == CLOSE_LOG);
    wait_until(SHORT, "the connections to be released", || r.connections() == 0);
    assert_eq!(r.closes().len(), CLOSE_LOG);
}

#[test]
fn the_refusal_summary_is_logged_periodically_while_the_count_is_nonzero() {
    let (r, lines) = relay_logging(RelayConfig {
        max_connections: 0,
        log_summary_every: Duration::from_millis(200),
        ..RelayConfig::default()
    });
    for _ in 0..5 {
        drop(TcpStream::connect(r.local_addr()).unwrap());
    }
    wait_until(SHORT, "a summary line", || !lines.lock().unwrap().is_empty());
    let first = lines.lock().unwrap()[0].clone();
    assert!(first.contains("connections refused or dropped before a seat"), "{first}");
    assert!(first.contains("limit:"), "{first}");
    // Nothing more while nothing new is refused.
    thread::sleep(Duration::from_millis(500));
    let n = lines.lock().unwrap().len();
    assert!(n <= 5, "{n} lines for 5 attempts");
}
