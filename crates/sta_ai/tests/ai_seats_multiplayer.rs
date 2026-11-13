//! Which seats are computer players is session state: every peer holds the
//! same set (it gates builds and passive income inside `step`), so a joiner
//! and a snapshot taken from the host agree with the host byte for byte,
//! while only the host runs the controller and sends its commands through the
//! router like a player.

use std::sync::Arc;

use sta_ai::{Registry, SKIRMISH};
use sta_content::ContentDb;
use sta_core::{Fixed, PlayerId, TeamId, Vec3};
use sta_net::lockstep::{LoopbackNet, Peer, StepOutcome};
use sta_net::proto::{MapRef, Occupant, PackInfo, RulesDigest, Seat, Start, TurnConfig};
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::digest::SectionDigests;
use sta_sim::setup::{Control, Controller, GameOptions, GameSetup};
use sta_sim::{Sim, SimEvent, WorldSource};

const HOST: PlayerId = PlayerId(0);
const AI: PlayerId = PlayerId(1);
const JOINER: PlayerId = PlayerId(2);

fn seat(slot: u8, occupant: Occupant) -> Seat {
    Seat {
        slot: PlayerId(slot),
        team: TeamId(slot),
        race: "fx".into(),
        occupant,
        name: format!("p{slot}"),
        lineage: "Test lineage".into(),
        packs: vec![PackInfo { name: "Test pack".into(), version: "1".into() }],
    }
}

fn start() -> Start {
    Start {
        seed: 0x5eed,
        map: MapRef { name: "fx_two_bases".into(), digest: 0 },
        options: GameOptions::default(),
        seats: vec![seat(0, Occupant::Peer), seat(1, Occupant::Ai(SKIRMISH.into())), seat(2, Occupant::Peer)],
        host: HOST,
        turn: TurnConfig { ticks_per_turn: 3, delay_turns: 1, speed_percent: 100 },
        rules: RulesDigest([0; 32]),
    }
}

struct Game {
    peers: Vec<Peer>,
    net: LoopbackNet,
    db: Arc<ContentDb>,
    ai: Box<dyn Controller>,
    events: Vec<SimEvent>,
    seq: u32,
    ai_commands: usize,
}

fn game() -> Game {
    let start = start();
    let db = Arc::new(sta_content::fake::db());
    let map = Arc::new(sta_content::fake::two_bases());
    let ids: Vec<PlayerId> = start.seats.iter().filter(|s| s.occupant == Occupant::Peer).map(|s| s.slot).collect();
    let (net, links) = LoopbackNet::new(&ids);
    let peers: Vec<Peer> = links
        .into_iter()
        .map(|l| {
            let me = l.seat();
            Peer::start(&start, me, db.clone(), map.clone(), Box::new(l)).unwrap()
        })
        .collect();
    // The host's own setup is what builds the controller of the AI seat.
    let setup = GameSetup { seed: start.seed, map, content: db.clone(), players: start.player_setups(HOST), options: start.options.clone() };
    let ai = Registry::builtin().create_for_seat(SKIRMISH, &setup, AI).unwrap();
    Game { peers, net, db, ai, events: Vec::new(), seq: 0, ai_commands: 0 }
}

impl Game {
    /// The host, as a mission script would: a station and yard for the two
    /// map starts, and stock for them.
    fn spawn_forces(&mut self) {
        let mut edits = Vec::new();
        for (i, s) in sta_content::fake::two_bases().starts.iter().enumerate() {
            let p = PlayerId(i as u8);
            for (odf, dz) in [("fx_station", 0), ("fx_yard", 300)] {
                let pos = Vec3 { x: s.pos.x, y: s.pos.y, z: s.pos.z + Fixed::from_int(dz) };
                edits.push(Edit::Spawn { odf: self.db.key(odf).unwrap(), player: p, pos, heading: s.heading });
            }
            edits.push(Edit::Cargo(p, 0, Fixed::from_int(1000)));
        }
        for e in edits {
            self.seq += 1;
            self.peers[0].submit(Command { player: PlayerId::SCRIPT, seq: self.seq, kind: CommandKind::Edit(e) });
        }
    }

    /// The host's controller thinks about the host's own view; its commands go
    /// through the router as the AI seat's. Joiners run nothing.
    fn run_to(&mut self, target: u64) {
        let mut idle = 0;
        let mut last_thought = None;
        while self.peers.iter().any(|p| p.tick() < target) {
            self.net.advance(1);
            let mut progress = false;
            for i in 0..self.peers.len() {
                if self.peers[i].tick() >= target {
                    continue;
                }
                if i == 0 && last_thought != Some(self.peers[0].tick()) {
                    last_thought = Some(self.peers[0].tick());
                    let view = self.peers[0].sim().view();
                    let mut out = Vec::new();
                    self.ai.think(&view, &self.events, &mut out);
                    for c in out {
                        assert_eq!(c.player, AI);
                        self.ai_commands += 1;
                        self.peers[0].submit(c);
                    }
                }
                if let StepOutcome::Ran { events, .. } = self.peers[i].step() {
                    progress = true;
                    if i == 0 {
                        self.events = events;
                    }
                }
            }
            idle = if progress { 0 } else { idle + 1 };
            assert!(idle < 5_000, "deadlock");
        }
    }

    fn assert_identical(&self, what: &str) {
        let host = self.peers[0].sim();
        for p in &self.peers[1..] {
            let (s, seat) = (p.sim(), p.router().me().0);
            assert_eq!(s.tick, host.tick, "{what}");
            assert_eq!(s.digest(), host.digest(), "{what}: full digest, seat {seat}");
            assert_eq!(s.section_digests(), host.section_digests(), "{what}: section digests, seat {seat}");
            for name in SectionDigests::NAMES {
                assert_eq!(s.section_rows(name), host.section_rows(name), "{what}: {name} rows, seat {seat}");
            }
            assert!(s.snapshot() == host.snapshot(), "{what}: snapshot bytes differ between the host and seat {seat}");
            assert!(p.router().desync().is_none() && p.router().peer_desync().is_none(), "{what}: desync reported");
        }
    }
}

#[test]
fn joiner_and_host_agree_on_ai_seats_at_start_and_after_play() {
    let mut g = game();
    g.assert_identical("at tick 0");
    g.spawn_forces();
    g.run_to(2 * 60 * 30);
    // The AI actually played: its commands reached every peer through the router.
    assert!(g.ai_commands > 0, "the skirmish AI sent no command");
    assert!(g.peers[1].replay().commands.iter().any(|c| c.command.player == AI), "the joiner never received an AI command");
    g.assert_identical("after two minutes");
    assert_eq!(g.peers[0].replay().digests, g.peers[1].replay().digests);
}

/// NET-04's rejoin: the host's snapshot, loaded by another peer, steps on to
/// the digests that peer's own sim reaches.
#[test]
fn a_snapshot_from_the_host_continues_to_the_joiners_digests() {
    let mut g = game();
    g.spawn_forces();
    g.run_to(600);
    let snapshot = g.peers[0].sim().snapshot();
    let mut loaded = Sim::from_snapshot(&snapshot, Some(g.db.clone())).unwrap();
    let mut joiner = g.peers[1].sim().clone();
    assert_eq!(loaded.digest(), joiner.digest());
    assert!(loaded.snapshot() == joiner.snapshot());
    // The same input to both from here on: whatever the host's AI says.
    let mut events = g.events.clone();
    for _ in 0..1800 {
        let mut out = Vec::new();
        g.ai.think(&loaded.view(), &events, &mut out);
        let a = loaded.step(&out);
        let b = joiner.step(&out);
        assert_eq!(a.view.digest, b.view.digest, "tick {}", a.view.tick);
        events = a.events;
    }
    assert!(loaded.snapshot() == joiner.snapshot());
    assert_eq!(loaded.section_digests(), joiner.section_digests());
}

/// The regression: the lobby handed non-hosts the AI seat as plain remote
/// input, so their `Sim::ai_seats` stayed empty.
#[test]
fn the_joiner_sets_up_the_ai_seat_as_an_ai_seat() {
    let s = start();
    let controls = |me| s.player_setups(me).into_iter().map(|p| p.control).collect::<Vec<_>>();
    assert_eq!(controls(HOST)[1], Control::Ai(SKIRMISH.into()));
    assert_eq!(controls(JOINER)[1], Control::RemoteAi(SKIRMISH.into()));
}
