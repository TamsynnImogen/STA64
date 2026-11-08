//! Replay files (S7): a game's setup, its command log and digests.
//!
//! - A [`Replay`] holds what a game started from ([`GameSetup`] minus the
//!   content database, which is too big to embed: the file carries its hash
//!   and the loader supplies the database), every command applied, and the
//!   sim's digest at intervals. Controllers are not recorded or rerun; their
//!   commands are in the log like any player's.
//! - [`Recorder`] steps a [`Sim`] and logs as it goes; [`Replay::play`] runs
//!   a replay from tick 0 and checks every digest, so a desync (or a sim
//!   change that breaks old replays) names the first tick that differs.
//! - Wire format: magic `STAR`, a `u16` format version, then `postcard`.
//!   The body also carries [`SEAM_REV`] and the content hash; [`Replay::decode`]
//!   rejects a file whose magic, format or seam revision it does not know.
//!   Content is checked when a game is built from the replay.
//! - Not in the file yet: [`GameOptions`] is empty today. When it gains
//!   fields, add them here with a format bump.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sta_content::{ContentDb, MapDef};
use sta_core::{PlayerId, TeamId, Tick};

use crate::command::Command;
use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
use crate::source::WorldSource;
use crate::{SEAM_REV, Sim};

/// First four bytes of every replay file.
pub const MAGIC: [u8; 4] = *b"STAR";
/// Bumped when the layout of [`Replay`] changes.
pub const FORMAT_VERSION: u16 = 1;
/// Digest every this many ticks unless the recorder is told otherwise.
pub const DEFAULT_DIGEST_EVERY: Tick = 30;

/// Why a replay could not be read or played.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayError {
    /// Not a replay file (wrong or missing magic).
    BadMagic,
    /// Written by a different layout of this file.
    FormatVersion { found: u16 },
    /// Written against another seam revision: commands may not decode alike.
    SeamRev { found: u32 },
    /// Truncated or corrupt body.
    Corrupt(String),
    /// The command log is not in tick order.
    Unordered,
    /// The supplied content is not the content the game was recorded with.
    ContentMismatch { expected: u64, found: u64 },
    /// The re-run's digest differs from the recorded one.
    Desync {
        tick: Tick,
        expected: u64,
        found: u64,
    },
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayError::BadMagic => write!(f, "not a replay file"),
            ReplayError::FormatVersion { found } => {
                write!(f, "replay format {found}, expected {FORMAT_VERSION}")
            }
            ReplayError::SeamRev { found } => {
                write!(f, "replay seam revision {found}, expected {SEAM_REV}")
            }
            ReplayError::Corrupt(e) => write!(f, "corrupt replay: {e}"),
            ReplayError::Unordered => write!(f, "replay commands are not in tick order"),
            ReplayError::ContentMismatch { expected, found } => {
                write!(
                    f,
                    "content hash {found:#018x}, replay was recorded with {expected:#018x}"
                )
            }
            ReplayError::Desync {
                tick,
                expected,
                found,
            } => {
                write!(
                    f,
                    "desync at tick {tick}: digest {found:#018x}, recorded {expected:#018x}"
                )
            }
        }
    }
}

impl std::error::Error for ReplayError {}

/// A seat, as stored (mirrors [`PlayerSetup`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatRec {
    pub id: PlayerId,
    pub team: TeamId,
    pub race: String,
    pub control: ControlRec,
}

/// [`Control`], as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlRec {
    Human,
    Remote,
    Ai(String),
    Idle,
}

/// A command and the tick it was applied on (the tick the sim was at when
/// it was passed to `step`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Logged {
    pub tick: Tick,
    pub command: Command,
}

/// A recorded game.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Replay {
    pub seam_rev: u32,
    /// `ContentDb::hash` of the content the game ran on.
    pub content_hash: u64,
    pub seed: u64,
    pub map: MapDef,
    pub players: Vec<SeatRec>,
    /// Ascending tick, then `(player, seq)`.
    pub commands: Vec<Logged>,
    /// `(tick, digest)`, ascending; starts with tick 0.
    pub digests: Vec<(Tick, u64)>,
    /// The tick the recording stopped at.
    pub end_tick: Tick,
}

impl Replay {
    /// The file's bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.extend(postcard::to_allocvec(self).expect("replay serialises"));
        out
    }

    /// Reads a file, checking magic, format and seam revision.
    pub fn decode(bytes: &[u8]) -> Result<Replay, ReplayError> {
        if bytes.len() < 6 || bytes[..4] != MAGIC {
            return Err(ReplayError::BadMagic);
        }
        let found = u16::from_le_bytes([bytes[4], bytes[5]]);
        if found != FORMAT_VERSION {
            return Err(ReplayError::FormatVersion { found });
        }
        let r: Replay =
            postcard::from_bytes(&bytes[6..]).map_err(|e| ReplayError::Corrupt(e.to_string()))?;
        if r.seam_rev != SEAM_REV {
            return Err(ReplayError::SeamRev { found: r.seam_rev });
        }
        if r.commands.windows(2).any(|w| w[0].tick > w[1].tick) {
            return Err(ReplayError::Unordered);
        }
        Ok(r)
    }

    /// The setup the game started from, on `content`, which must be the
    /// content it was recorded with.
    pub fn setup(&self, content: Arc<ContentDb>) -> Result<GameSetup, ReplayError> {
        if content.hash() != self.content_hash {
            return Err(ReplayError::ContentMismatch {
                expected: self.content_hash,
                found: content.hash(),
            });
        }
        Ok(GameSetup {
            seed: self.seed,
            map: Arc::new(self.map.clone()),
            content,
            players: self
                .players
                .iter()
                .map(|p| PlayerSetup {
                    id: p.id,
                    team: p.team,
                    race: p.race.clone(),
                    control: match &p.control {
                        ControlRec::Human => Control::Human,
                        ControlRec::Remote => Control::Remote,
                        ControlRec::Ai(n) => Control::Ai(n.clone()),
                        ControlRec::Idle => Control::Idle,
                    },
                })
                .collect(),
            options: GameOptions::default(),
        })
    }

    /// Re-runs the game from tick 0 to [`Replay::end_tick`], checking each
    /// recorded digest. Returns the final sim.
    pub fn play(&self, content: Arc<ContentDb>) -> Result<Sim, ReplayError> {
        let mut sim = Sim::from_setup(&self.setup(content)?);
        let mut next_digest = 0;
        let mut next_cmd = 0;
        loop {
            let tick = sim.view().tick;
            while let Some(&(t, expected)) = self.digests.get(next_digest) {
                if t > tick {
                    break;
                }
                next_digest += 1;
                let found = sim.digest();
                if t == tick && found != expected {
                    return Err(ReplayError::Desync {
                        tick,
                        expected,
                        found,
                    });
                }
            }
            if tick >= self.end_tick {
                return Ok(sim);
            }
            let mut batch = Vec::new();
            while let Some(l) = self.commands.get(next_cmd).filter(|l| l.tick <= tick) {
                batch.push(l.command.clone());
                next_cmd += 1;
            }
            sim.step(&batch);
        }
    }
}

/// Steps a [`Sim`] and logs what it was given and what it digested.
pub struct Recorder {
    sim: Sim,
    replay: Replay,
    every: Tick,
}

impl Recorder {
    /// Starts from `setup`, digesting every [`DEFAULT_DIGEST_EVERY`] ticks.
    pub fn new(setup: &GameSetup) -> Recorder {
        Recorder::with_interval(setup, DEFAULT_DIGEST_EVERY)
    }

    /// As [`Recorder::new`], digesting every `every` ticks (at least 1).
    pub fn with_interval(setup: &GameSetup, every: Tick) -> Recorder {
        let sim = Sim::from_setup(setup);
        let replay = Replay {
            seam_rev: SEAM_REV,
            content_hash: setup.content.hash(),
            seed: setup.seed,
            map: (*setup.map).clone(),
            players: setup
                .players
                .iter()
                .map(|p| SeatRec {
                    id: p.id,
                    team: p.team,
                    race: p.race.clone(),
                    control: match &p.control {
                        Control::Human => ControlRec::Human,
                        Control::Remote => ControlRec::Remote,
                        Control::Ai(n) => ControlRec::Ai(n.clone()),
                        // Idle, and any seat kind added later: nothing sends for them.
                        _ => ControlRec::Idle,
                    },
                })
                .collect(),
            commands: Vec::new(),
            digests: vec![(0, sim.digest())],
            end_tick: 0,
        };
        Recorder {
            sim,
            replay,
            every: every.max(1),
        }
    }

    /// The sim being recorded.
    pub fn sim(&self) -> &Sim {
        &self.sim
    }

    /// Logs `commands` against the current tick and steps the sim.
    pub fn step(&mut self, commands: &[Command]) -> crate::TickOutput {
        let tick = self.sim.view().tick;
        let mut sorted: Vec<&Command> = commands.iter().collect();
        sorted.sort_by_key(|c| c.sort_key());
        self.replay
            .commands
            .extend(sorted.into_iter().map(|c| Logged {
                tick,
                command: c.clone(),
            }));
        let out = self.sim.step(commands);
        let now = out.view.tick;
        self.replay.end_tick = now;
        if now.is_multiple_of(self.every) {
            self.replay.digests.push((now, out.view.digest));
        }
        out
    }

    /// Ends the recording. The last tick's digest is always included.
    pub fn finish(mut self) -> Replay {
        let end = self.replay.end_tick;
        if self.replay.digests.last().map(|d| d.0) != Some(end) {
            self.replay.digests.push((end, self.sim.digest()));
        }
        self.replay
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandKind, Edit, Order};
    use sta_core::{EntityId, Fixed, Vec3};

    fn setup() -> GameSetup {
        GameSetup {
            seed: 7,
            map: Arc::new(sta_content::fake::two_bases()),
            content: Arc::new(sta_content::fake::db()),
            players: vec![
                PlayerSetup {
                    id: PlayerId(0),
                    team: TeamId(0),
                    race: "fx".into(),
                    control: Control::Human,
                },
                PlayerSetup {
                    id: PlayerId(1),
                    team: TeamId(1),
                    race: "fx".into(),
                    control: Control::Ai("idle".into()),
                },
            ],
            options: GameOptions::default(),
        }
    }

    fn record(n: Tick) -> (Replay, u64) {
        let mut rec = Recorder::with_interval(&setup(), 10);
        let cruiser = sta_content::fake::db().key("fx_cruiser").unwrap();
        for t in 0..n {
            let cmds = if t == 0 {
                vec![Command {
                    player: PlayerId::BENCH,
                    seq: 0,
                    kind: CommandKind::Edit(Edit::Spawn {
                        odf: cruiser,
                        player: PlayerId(0),
                        pos: Vec3::from_f32([0.0, 0.0, 0.0]),
                        heading: (Fixed::ZERO, Fixed::ONE),
                    }),
                }]
            } else if t % 25 == 0 {
                let units: Vec<EntityId> = rec
                    .sim()
                    .view()
                    .units
                    .iter()
                    .filter(|u| u.owner == PlayerId(0))
                    .map(|u| u.id)
                    .collect();
                assert!(!units.is_empty());
                let to = Vec3::from_f32([t as f32, 0.0, 40.0]);
                vec![Command {
                    player: PlayerId(0),
                    seq: t as u32,
                    kind: CommandKind::Order(Order::Move {
                        units,
                        to,
                        queue: false,
                    }),
                }]
            } else {
                vec![]
            };
            rec.step(&cmds);
        }
        let digest = rec.sim().digest();
        (rec.finish(), digest)
    }

    #[test]
    fn record_replay_equal_digests() {
        let (replay, digest) = record(200);
        assert_eq!(replay.end_tick, 200);
        assert_eq!(replay.commands.len(), 8);
        let decoded = Replay::decode(&replay.encode()).unwrap();
        assert_eq!(decoded, replay);
        let sim = decoded.play(Arc::new(sta_content::fake::db())).unwrap();
        assert_eq!(sim.digest(), digest);
        assert_eq!(sim.view().tick, 200);
    }

    #[test]
    fn tampered_log_is_a_desync_at_the_first_differing_digest() {
        let (mut replay, _) = record(100);
        replay.commands.remove(1); // the order at tick 25
        match replay.play(Arc::new(sta_content::fake::db())) {
            Ok(_) => panic!("tampered log replayed clean"),
            Err(ReplayError::Desync { tick, .. }) => {
                assert!((30..=40).contains(&tick), "tick {tick}")
            }
            Err(e) => panic!("{e}"),
        }
    }

    #[test]
    fn rejects_foreign_files_and_content() {
        let (replay, _) = record(10);
        let mut bytes = replay.encode();
        assert_eq!(
            Replay::decode(b"nope!!").unwrap_err(),
            ReplayError::BadMagic
        );
        assert!(matches!(
            Replay::decode(&bytes[..bytes.len() - 3]),
            Err(ReplayError::Corrupt(_))
        ));
        bytes[4] = 9;
        assert_eq!(
            Replay::decode(&bytes).unwrap_err(),
            ReplayError::FormatVersion { found: 9 }
        );
        let mut other = replay.clone();
        other.seam_rev = SEAM_REV + 1;
        assert_eq!(
            Replay::decode(&other.encode()).unwrap_err(),
            ReplayError::SeamRev {
                found: SEAM_REV + 1
            }
        );
        let empty = Arc::new(ContentDb::builder().build().unwrap());
        assert!(matches!(
            replay.play(empty),
            Err(ReplayError::ContentMismatch { .. })
        ));
    }
}
