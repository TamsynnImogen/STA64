//! A headless two-process (or N-process) match over a relay, with the fake
//! test content: the "two processes play 5 minutes over a local relay" check
//! of NET-03, and a worked example of what the lobby (UI-11) does with the
//! transport: join, `Hello`/`Welcome`, `Start`, then lockstep.
//!
//! ```text
//! relay --listen 127.0.0.1:7864
//! netplay host --relay 127.0.0.1:7864 --peers 2 --ticks 9000 --hz 30
//! netplay join --relay 127.0.0.1:7864 --code K7M-Q2X --ticks 9000 --hz 30
//! ```
//!
//! Every process prints `DONE tick=<n> digest=<hex>` at the end; they must
//! agree.

use std::sync::Arc;
use std::time::{Duration, Instant};

use sta_core::{Fixed, PlayerId, TeamId, Vec3};
use sta_net::lockstep::{admit, Destination, Peer, StepOutcome, Transport};
use sta_net::proto::{
    ContentHashRules, EngineId, Hello, MapRef, Message, Occupant, PackInfo, Refusal, RulesIdentity,
    Seat, Start, TurnConfig, Welcome,
};
use sta_net::transport::{ClientOptions, NetTransport, RoomCode};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::setup::GameOptions;
use sta_sim::WorldSource;

fn die(msg: &str) -> ! {
    eprintln!("netplay: {msg}");
    std::process::exit(1)
}

fn engine() -> EngineId {
    EngineId {
        name: "netplay".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        build: "dev".into(),
    }
}

fn wait_msg(t: &mut NetTransport, limit: Duration) -> (PlayerId, Message) {
    let end = Instant::now() + limit;
    loop {
        match t.poll() {
            Ok(Some(m)) => return (m.from, m.msg),
            Ok(None) => {}
            Err(e) => die(&format!("transport: {e}")),
        }
        if Instant::now() > end {
            die("timed out waiting for the other side");
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let role = args.next().unwrap_or_else(|| die("host or join"));
    let (mut relay, mut code, mut peers, mut ticks, mut hz) =
        (String::new(), String::new(), 2u8, 3_000u64, 30u64);
    while let Some(a) = args.next() {
        let mut v = || args.next().unwrap_or_else(|| die(&format!("{a} needs a value")));
        match a.as_str() {
            "--relay" => relay = v(),
            "--code" => code = v(),
            "--peers" => peers = v().parse().unwrap_or_else(|_| die("--peers")),
            "--ticks" => ticks = v().parse().unwrap_or_else(|_| die("--ticks")),
            "--hz" => hz = v().parse().unwrap_or_else(|_| die("--hz")),
            _ => die(&format!("unknown argument {a}")),
        }
    }
    if relay.is_empty() || !(2..=8).contains(&peers) {
        die("--relay ADDR is required; --peers is 2 to 8");
    }
    let db = Arc::new(sta_content::fake::db());
    let map = Arc::new(sta_content::fake::two_bases());
    let rules = ContentHashRules(db.hash());
    let opts = ClientOptions::default();
    let wait = Duration::from_secs(120);

    let (mut t, start) = if role == "host" {
        let mut t = NetTransport::create(relay.as_str(), "", &opts)
            .unwrap_or_else(|e| die(&e.to_string()));
        println!("ROOM {}", t.control().room_code());
        let mut joined = 1u8;
        while joined < peers {
            let (from, msg) = wait_msg(&mut t, wait);
            let Message::Hello(hello) = msg else { continue };
            let reply = match admit(&engine(), &rules, &hello, &rules) {
                Ok(()) => Message::Welcome(Welcome {
                    slot: from,
                    host: PlayerId(0),
                    rejoin_token: 0,
                }),
                Err(e) => Message::Refuse(e.refusal()),
            };
            let ok = matches!(reply, Message::Welcome(_));
            let _ = t.send(Destination::Seat(from), &reply);
            if ok {
                joined += 1;
                println!("JOINED seat {}", from.0);
            }
        }
        let start = Start {
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
                    lineage: "netplay".into(),
                    packs: Vec::<PackInfo>::new(),
                })
                .collect(),
            host: PlayerId(0),
            turn: TurnConfig {
                ticks_per_turn: 3,
                delay_turns: 2,
                speed_percent: 100,
            },
            rules: rules.digest(),
        };
        // The game has started: close the room to newcomers; a vacated seat
        // stays reserved for a rejoin.
        let _ = t.control().set_open(false);
        t.send(Destination::All, &Message::Start(start.clone()))
            .unwrap_or_else(|e| die(&e.to_string()));
        (t, start)
    } else {
        let code = RoomCode::parse(&code).unwrap_or_else(|| die("--code is not a room code"));
        let mut t = NetTransport::join(relay.as_str(), code, "", &opts)
            .unwrap_or_else(|e| die(&e.to_string()));
        let hello = Hello {
            engine: engine(),
            rules: rules.digest(),
            lineage: "netplay".into(),
            packs: vec![],
            name: format!("p{}", t.seat().0),
        };
        t.send(Destination::Seat(PlayerId(0)), &Message::Hello(hello))
            .unwrap_or_else(|e| die(&e.to_string()));
        println!("JOINED seat {}", t.seat().0);
        let start = loop {
            match wait_msg(&mut t, wait).1 {
                Message::Start(s) => break s,
                Message::Refuse(Refusal::Engine { .. }) => die("refused: another build"),
                Message::Refuse(r) => die(&format!("refused: {r:?}")),
                _ => {}
            }
        };
        (t, start)
    };
    let me = t.seat();
    let control = t.control();
    // The peer takes the transport; keep a handle for the end.
    let _ = &mut t;
    let mut peer = Peer::start(&start, me, db.clone(), map, Box::new(t))
        .unwrap_or_else(|e| die(&e.to_string()));

    let mut seq = 0u32;
    if me == start.host {
        let cruiser = db.key("fx_cruiser").unwrap_or_else(|| die("fake content"));
        let frigate = db.key("fx_frigate").unwrap_or_else(|| die("fake content"));
        for p in 0..i32::from(peers) {
            for k in 0..3 {
                peer.submit(Command {
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
    let mut my_seq = 0u32;
    let begun = Instant::now();
    let mut stalls = 0u64;
    let mut last_order = None;
    while peer.tick() < ticks {
        if hz > 0 {
            let due = begun + Duration::from_secs_f64(peer.tick() as f64 / hz as f64);
            let now = Instant::now();
            if due > now {
                std::thread::sleep((due - now).min(Duration::from_millis(5)));
                continue;
            }
        }
        let t = peer.tick();
        if t.is_multiple_of(9) && last_order != Some(t) {
            last_order = Some(t);
            let mine: Vec<_> = peer
                .sim()
                .view()
                .units
                .iter()
                .filter(|u| u.owner == me)
                .map(|u| u.id)
                .collect();
            if !mine.is_empty() {
                let k = (t / 9) as usize;
                peer.submit(Command {
                    player: me,
                    seq: my_seq,
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
                my_seq += 1;
            }
        }
        match peer.step() {
            StepOutcome::Ran { .. } => {}
            StepOutcome::Stalled { .. } => {
                stalls += 1;
                std::thread::sleep(Duration::from_millis(1));
            }
            StepOutcome::Halted => die("desync"),
        }
        if begun.elapsed() > Duration::from_secs(3600) {
            die("gave up");
        }
    }
    println!(
        "DONE seat={} tick={} digest={:016x} commands={} stalls={}",
        me.0,
        peer.tick(),
        peer.sim().digest(),
        peer.replay().commands.len(),
        stalls
    );
    control.leave();
    std::thread::sleep(Duration::from_millis(300));
}
