//! NET-05 (NET_DECISIONS N5): "Resyncing the host is allowed only if the host's
//! controllers (AI, mission Lua) can carry on across a reload of the sim."
//! This is the proof for the skirmish AI: a host whose sim differs from the
//! referee's is resynced from the referee while its controller, the same
//! object throughout, goes on sending commands that every peer applies.
//!
//! Mission Lua is not covered here (it lives in `sta_mission`), which is why
//! `RefereePolicy::default().resync_host` stays `false`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use sta_ai::{Registry, SKIRMISH};
use sta_content::ContentDb;
use sta_core::{Fixed, PlayerId, TeamId, Tick, Vec3};
use sta_net::lockstep::{Fault, LoopbackNet, Peer, StepOutcome};
use sta_net::proto::{MapRef, Occupant, PackInfo, RulesDigest, Seat, Start, TurnConfig};
use sta_net::referee::{Referee, RefereeLimits, RefereePolicy, RefereeStep};
use sta_net::resync::ResyncLimits;
use sta_sim::command::{Command, CommandKind, Edit};
use sta_sim::setup::{Controller, GameOptions, GameSetup};
use sta_sim::{SimEvent, WorldSource};

const HOST: PlayerId = PlayerId(0);
const AI: PlayerId = PlayerId(1);
const JOINER: PlayerId = PlayerId(2);
const REFEREE: PlayerId = PlayerId(3);
const WAIT: Duration = Duration::from_secs(120);

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
    referee: Referee,
    net: LoopbackNet,
    ai: Box<dyn Controller>,
    events: Vec<SimEvent>,
    ai_commands: usize,
}

fn game(policy: RefereePolicy) -> Game {
    let start = start();
    let db = Arc::new(sta_content::fake::db());
    let map = Arc::new(sta_content::fake::two_bases());
    let (net, mut links) = LoopbackNet::new(&[HOST, JOINER, REFEREE]);
    let referee_link = links.pop().unwrap();
    let limits = ResyncLimits { min_attempt_interval: Duration::from_millis(1), ..ResyncLimits::default() };
    let peers: Vec<Peer> = links
        .into_iter()
        .map(|l| {
            let me = l.seat();
            let mut p = Peer::start(&start, me, db.clone(), map.clone(), Box::new(l)).unwrap();
            p.set_resync_limits(limits.clone());
            p
        })
        .collect();
    let mut referee = Referee::new(
        &start,
        REFEREE,
        db.clone(),
        map.clone(),
        Box::new(referee_link),
        policy,
        RefereeLimits { release_interval: Duration::from_millis(5), ..RefereeLimits::default() },
    )
    .unwrap();
    referee.peer_mut().set_resync_limits(limits);
    let mut peers = peers;
    peers[0].router_mut().seat_referee(Some(REFEREE)).unwrap();
    let setup = GameSetup { seed: start.seed, map, content: db.clone(), players: start.player_setups(HOST), options: start.options.clone() };
    let ai = Registry::builtin().create_for_seat(SKIRMISH, &setup, AI).unwrap();
    let mut g = Game { peers, referee, net, ai, events: Vec::new(), ai_commands: 0 };
    g.spawn_forces(&db);
    g
}

impl Game {
    fn spawn_forces(&mut self, db: &ContentDb) {
        let mut seq = 0;
        for (i, s) in sta_content::fake::two_bases().starts.iter().enumerate() {
            let p = PlayerId(i as u8);
            for (odf, dz) in [("fx_station", 0), ("fx_yard", 300)] {
                let pos = Vec3 { x: s.pos.x, y: s.pos.y, z: s.pos.z + Fixed::from_int(dz) };
                seq += 1;
                self.peers[0].submit(Command {
                    player: PlayerId::SCRIPT,
                    seq,
                    kind: CommandKind::Edit(Edit::Spawn { odf: db.key(odf).unwrap(), player: p, pos, heading: s.heading }),
                });
            }
            seq += 1;
            self.peers[0].submit(Command {
                player: PlayerId::SCRIPT,
                seq,
                kind: CommandKind::Edit(Edit::Cargo(p, 0, Fixed::from_int(1000))),
            });
        }
    }

    /// The host's controller thinks about the host's own view each tick; its
    /// commands go through the router as the AI seat's.
    fn round(&mut self, last_thought: &mut Option<Tick>) -> bool {
        let mut progress = false;
        for i in 0..self.peers.len() {
            if i == 0 && *last_thought != Some(self.peers[0].tick()) {
                *last_thought = Some(self.peers[0].tick());
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
        progress |= matches!(self.referee.step(), RefereeStep::Ran(_));
        if !progress {
            self.net.advance(1);
        }
        progress
    }

    fn run_until(&mut self, what: &str, done: impl Fn(&Game) -> bool) {
        let end = Instant::now() + WAIT;
        let mut last = None;
        while !done(self) {
            self.round(&mut last);
            assert!(Instant::now() < end, "timed out: {what}");
        }
    }

    fn all_at(&self, t: Tick) -> bool {
        self.peers.iter().all(|p| p.tick() >= t) && self.referee.peer().tick() >= t
    }
}

#[test]
fn the_skirmish_ai_plays_under_a_referee_and_the_match_is_verified() {
    let mut g = game(RefereePolicy::default());
    g.run_until("two minutes", |g| g.all_at(2 * 60 * 30));
    assert!(g.ai_commands > 0, "the skirmish AI sent no command");
    assert!(g.referee.findings().is_empty(), "{:?}", g.referee.findings());
    assert!(g.referee.peer().replay().commands.iter().any(|c| c.command.player == AI), "the referee never saw the AI's commands");
    assert!(g.peers.iter().all(|p| !p.router().halted()));
}

#[test]
fn a_diverged_host_is_resynced_from_the_referee_and_its_skirmish_ai_carries_on() {
    let mut g = game(RefereePolicy { resync_host: true });
    g.run_until("tick 600", |g| g.all_at(600));
    let before = g.ai_commands;
    assert!(before > 0);
    // The host's own sim differs from everyone else's from here on.
    let unit = g.peers[0].sim().view().units.iter().find(|u| u.owner == HOST).expect("a host unit").id;
    g.peers[0].inject(Fault::Inject(Command {
        player: PlayerId::SCRIPT,
        seq: 99_999,
        kind: CommandKind::Edit(Edit::Damage(unit, Fixed::from_int(7))),
    }));
    g.run_until("the host to be resynced", |g| !g.peers[0].resyncs().is_empty() && g.peers.iter().all(|p| !p.router().halted()));
    assert!(g.peers[1].resyncs().is_empty(), "only the host differed");
    let resumed = g.peers[0].tick();
    let ai_then = g.ai_commands;
    g.run_until("two more minutes", |g| g.all_at(resumed + 2 * 60 * 30));
    // The same controller object kept thinking about the reloaded sim and its
    // commands reached everyone.
    assert!(g.ai_commands > ai_then, "the AI sent {} commands after the resync", g.ai_commands - ai_then);
    let after = g
        .referee
        .peer()
        .replay()
        .commands
        .iter()
        .filter(|c| c.command.player == AI && c.tick > resumed)
        .count();
    assert!(after > 0, "{after} AI commands after the resync reached the referee");
    let joiner_after = g.peers[1].replay().commands.iter().filter(|c| c.command.player == AI && c.tick > resumed).count();
    assert!(joiner_after > 0, "{joiner_after}");
    // Everyone is in step with the referee.
    let r = g.referee.peer().sim().view().tick;
    assert!(r >= resumed);
    assert!(g.referee.findings().iter().all(|f| f.seat == HOST));
    assert!(g.peers.iter().all(|p| !p.router().halted()));
}
