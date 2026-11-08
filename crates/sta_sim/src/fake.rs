//! World sources to build against before the sim has the systems
//! (docs/WORKPLAN.md, "Fakes and fixtures"). No game data.
//!
//! - [`ScriptedSource`] plays a JSON5 timeline: units on rails and events at
//!   fixed ticks. Render FX, audio and HUD work start here instead of
//!   waiting for combat and economy.
//! - [`ViewLog`] records a real run's [`TickOutput`]s, stores them as
//!   `postcard`, and replays them: render performance and audio work on a
//!   real run without running the sim.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sta_content::ContentDb;
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, SEAM_REV, TICK_HZ, Tick, Vec3};

use crate::command::Command;
use crate::event::SimEvent;
use crate::source::WorldSource;
use crate::view::{TickOutput, UnitFlags, UnitView, WorldView};

/// A timeline file; see [`ScriptedSource`].
#[derive(Debug, Clone, Deserialize)]
struct Timeline {
    #[serde(default)]
    units: Vec<RailUnit>,
    #[serde(default)]
    events: Vec<TimedEvent>,
}

#[derive(Debug, Clone, Deserialize)]
struct RailUnit {
    id: EntityId,
    #[serde(default)]
    def: Option<String>,
    #[serde(default)]
    owner: PlayerId,
    /// Full hull; the view shows `[hull, hull]`.
    #[serde(default)]
    hull: Fixed,
    /// Ascending `at`. The unit appears at the first point (a `Spawned`
    /// event unless that is tick 0), moves in straight lines between points
    /// and stays at the last one.
    rails: Vec<RailPoint>,
    /// The tick it disappears with a `Destroyed` event.
    #[serde(default)]
    until: Option<Tick>,
}

#[derive(Debug, Clone, Deserialize)]
struct RailPoint {
    at: Tick,
    pos: Vec3,
    /// Heading from here to the next point; default: the direction of travel,
    /// or +Z when standing.
    #[serde(default)]
    heading: Option<(Fixed, Fixed)>,
}

#[derive(Debug, Clone, Deserialize)]
struct TimedEvent {
    at: Tick,
    event: SimEvent,
}

struct Rail {
    id: EntityId,
    def: ContentKey,
    owner: PlayerId,
    hull: Fixed,
    points: Vec<RailPoint>,
    until: Option<Tick>,
}

/// A [`WorldSource`] that plays a timeline and ignores commands. Each step
/// produces the next tick: its view, then the events timed at that tick
/// (spawns and removals from the rails first, then the file's events in
/// file order). Views carry digest 0: there is no sim state to digest.
///
/// In a timeline, numbers where the sim has `Fixed` are plain numbers;
/// `ContentKey`s in events are numbers (`db.key(...)`; for `fake::db()` they
/// are fixed by its pinned hash: `fx_beam` is 0), unit definitions are names.
///
/// ```json5
/// {
///   units: [
///     { id: 1, def: "fx_cruiser", owner: 0, hull: 800,
///       rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }, { at: 60, pos: { x: 0, y: 0, z: 300 } }] },
///     { id: 2, def: "fx_frigate", owner: 1, until: 50,
///       rails: [{ at: 0, pos: { x: 0, y: 0, z: 600 }, heading: [0, -1] }] },
///   ],
///   events: [
///     { at: 30, event: { Fired: { shooter: 1, weapon: 0, hardpoint: 0, target: { Entity: 2 } } } },
///   ],
/// }
/// ```
pub struct ScriptedSource {
    tick: Tick,
    rails: Vec<Rail>,
    events: Vec<TimedEvent>,
}

impl ScriptedSource {
    /// Read a timeline; unit `def` names are looked up in `db`.
    pub fn from_json5(text: &str, db: &ContentDb) -> Result<ScriptedSource, String> {
        let t: Timeline = json5::from_str(text).map_err(|e| e.to_string())?;
        let mut rails = Vec::new();
        for u in t.units {
            let def = match &u.def {
                Some(name) => db.key(name).ok_or_else(|| format!("unit {}: unknown def {name:?}", u.id.0))?,
                None => ContentKey::NONE,
            };
            if u.rails.is_empty() {
                return Err(format!("unit {}: no rail points", u.id.0));
            }
            if u.rails.windows(2).any(|w| w[0].at >= w[1].at) {
                return Err(format!("unit {}: rail points must have ascending `at`", u.id.0));
            }
            rails.push(Rail { id: u.id, def, owner: u.owner, hull: u.hull, points: u.rails, until: u.until });
        }
        rails.sort_by_key(|r| r.id);
        if rails.windows(2).any(|w| w[0].id == w[1].id) {
            return Err("unit ids must be unique".into());
        }
        let mut events = t.events;
        events.sort_by_key(|e| e.at);
        Ok(ScriptedSource { tick: 0, rails, events })
    }

    fn view_at(&self, t: Tick) -> WorldView {
        let units = self.rails.iter().filter_map(|r| r.at(t)).collect();
        WorldView { tick: t, digest: 0, units, players: Vec::new(), visibility: Vec::new() }
    }

    fn events_at(&self, t: Tick) -> Vec<SimEvent> {
        let mut out = Vec::new();
        for r in &self.rails {
            if t > 0 && r.points[0].at == t {
                out.push(SimEvent::Spawned { id: r.id });
            }
            if r.until == Some(t) {
                out.push(SimEvent::Destroyed { id: r.id, by: None });
            }
        }
        out.extend(self.events.iter().filter(|e| e.at == t).map(|e| e.event.clone()));
        out
    }
}

impl Rail {
    fn at(&self, t: Tick) -> Option<UnitView> {
        let first = &self.points[0];
        if t < first.at || self.until.is_some_and(|u| t >= u) {
            return None;
        }
        // The segment `t` is on; past the last point, standing at it.
        let i = self.points.iter().rposition(|p| p.at <= t).unwrap_or(0);
        let p0 = &self.points[i];
        let (pos, travel, speed) = match self.points.get(i + 1) {
            Some(p1) => {
                let span = Fixed::from_int((p1.at - p0.at) as i32);
                let f = Fixed::from_int((t - p0.at) as i32) / span;
                let d = Vec3 { x: p1.pos.x - p0.pos.x, y: p1.pos.y - p0.pos.y, z: p1.pos.z - p0.pos.z };
                let pos = Vec3 { x: p0.pos.x + d.x * f, y: p0.pos.y + d.y * f, z: p0.pos.z + d.z * f };
                let len = (d.x * d.x + d.y * d.y + d.z * d.z).sqrt();
                let flat = (d.x * d.x + d.z * d.z).sqrt();
                let travel = (flat > Fixed::ZERO).then(|| (d.x / flat, d.z / flat));
                (pos, travel, len * Fixed::from_int(TICK_HZ as i32) / span)
            }
            None => (p0.pos, None, Fixed::ZERO),
        };
        let heading = p0.heading.or(travel).unwrap_or((Fixed::ZERO, Fixed::ONE));
        let hull = [self.hull, self.hull];
        Some(UnitView {
            id: self.id,
            def: self.def,
            owner: self.owner,
            pos,
            heading,
            speed,
            hull,
            shields: [[Fixed::ZERO; 2]; 4],
            flags: UnitFlags::SELECTABLE,
            progress: None,
        })
    }
}

impl WorldSource for ScriptedSource {
    fn step(&mut self, _commands: &[Command]) -> TickOutput {
        self.tick += 1;
        TickOutput { view: Arc::new(self.view_at(self.tick)), events: self.events_at(self.tick) }
    }

    fn view(&self) -> Arc<WorldView> {
        Arc::new(self.view_at(self.tick))
    }
}

/// What a [`ViewLog`] stores.
#[derive(Serialize, Deserialize)]
struct LogFile {
    rev: u32,
    initial: Arc<WorldView>,
    ticks: Vec<TickOutput>,
}

/// A recorded run, replayed as a [`WorldSource`] that ignores commands.
/// Past the end it repeats the last view with no events.
#[derive(Debug, Clone)]
pub struct ViewLog {
    initial: Arc<WorldView>,
    ticks: Vec<TickOutput>,
    /// Index of the next output to replay.
    next: usize,
}

impl ViewLog {
    /// Step `source` once per batch, keeping every output.
    pub fn record(source: &mut dyn WorldSource, batches: impl IntoIterator<Item = Vec<Command>>) -> ViewLog {
        let initial = source.view();
        let ticks = batches.into_iter().map(|b| source.step(&b)).collect();
        ViewLog { initial, ticks, next: 0 }
    }

    /// The recorded outputs, in tick order.
    pub fn ticks(&self) -> &[TickOutput] {
        &self.ticks
    }

    /// Start the replay over.
    pub fn rewind(&mut self) {
        self.next = 0;
    }

    /// `postcard`, tagged with [`SEAM_REV`].
    pub fn to_bytes(&self) -> Vec<u8> {
        let file = LogFile { rev: SEAM_REV, initial: self.initial.clone(), ticks: self.ticks.clone() };
        postcard::to_allocvec(&file).expect("views serialize")
    }

    /// Fails on a log from another seam revision or a damaged file.
    pub fn from_bytes(bytes: &[u8]) -> Result<ViewLog, String> {
        let file: LogFile = postcard::from_bytes(bytes).map_err(|e| e.to_string())?;
        if file.rev != SEAM_REV {
            return Err(format!("view log is seam revision {}, this build reads {SEAM_REV}", file.rev));
        }
        Ok(ViewLog { initial: file.initial, ticks: file.ticks, next: 0 })
    }
}

impl WorldSource for ViewLog {
    fn step(&mut self, _commands: &[Command]) -> TickOutput {
        match self.ticks.get(self.next) {
            Some(out) => {
                self.next += 1;
                out.clone()
            }
            None => TickOutput { view: self.view(), events: Vec::new() },
        }
    }

    fn view(&self) -> Arc<WorldView> {
        match self.next {
            0 => self.initial.clone(),
            n => self.ticks[n - 1].view.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandKind, Order, Target};
    use crate::Sim;

    const TIMELINE: &str = r#"{
        // Two ships pass; the cruiser fires at 30 and hits at 45; the frigate
        // leaves at 60 and a third ship appears at 10.
        units: [
            { id: 1, def: "fx_cruiser", owner: 0, hull: 800,
              rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }, { at: 60, pos: { x: 0, y: 0, z: 300 } }] },
            { id: 2, def: "fx_frigate", owner: 1, until: 60,
              rails: [{ at: 0, pos: { x: 0, y: 0, z: 600 }, heading: [0, -1] }] },
            { id: 3, rails: [{ at: 10, pos: { x: 100, y: 0, z: 0 } }] },
        ],
        events: [
            { at: 45, event: { Hit: { target: 2, by: 1, weapon: 0, facing: 0, shield: 20, hull: 0,
                                      at: { x: 0, y: 0, z: 600 } } } },
            { at: 30, event: { Fired: { shooter: 1, weapon: 0, hardpoint: 0, target: { Entity: 2 } } } },
        ],
    }"#;

    #[test]
    fn scripted_source_plays_a_timeline() {
        let db = sta_content::fake::db();
        let mut s = ScriptedSource::from_json5(TIMELINE, &db).unwrap();
        let v0 = s.view();
        assert_eq!(v0.units.len(), 2);
        assert_eq!(v0.unit(EntityId(1)).unwrap().def, db.key("fx_cruiser").unwrap());
        let mut log = Vec::new();
        for _ in 0..70 {
            let out = s.step(&[]);
            for e in &out.events {
                log.push((out.view.tick, e.clone()));
            }
            if out.view.tick == 30 {
                // Halfway along the rail, at 150 units per second.
                let u = out.view.unit(EntityId(1)).unwrap();
                assert_eq!(u.pos.z, Fixed::from_int(150));
                assert_eq!(u.speed, Fixed::from_int(150));
                assert_eq!(u.heading, (Fixed::ZERO, Fixed::ONE));
                assert_eq!(out.view.unit(EntityId(2)).unwrap().heading, (Fixed::ZERO, -Fixed::ONE));
            }
        }
        // Weapon 0 in the timeline is `fx_beam` under the fake db's keys.
        let beam = db.key("fx_beam").unwrap();
        assert_eq!(beam, ContentKey(0));
        assert_eq!(log.len(), 4, "{log:?}");
        assert_eq!(log[0], (10, SimEvent::Spawned { id: EntityId(3) }));
        let fired = SimEvent::Fired { shooter: EntityId(1), weapon: beam, hardpoint: 0, target: Target::Entity(EntityId(2)) };
        assert_eq!(log[1], (30, fired));
        assert!(matches!(log[2], (45, SimEvent::Hit { target: EntityId(2), by: EntityId(1), .. })));
        assert_eq!(log[3], (60, SimEvent::Destroyed { id: EntityId(2), by: None }));
        // After its `until` the frigate is gone; the cruiser stands at the end.
        let end = s.view();
        assert!(end.unit(EntityId(2)).is_none());
        assert_eq!(end.unit(EntityId(1)).unwrap().pos.z, Fixed::from_int(300));
        assert_eq!(end.unit(EntityId(1)).unwrap().speed, Fixed::ZERO);
    }

    #[test]
    fn scripted_source_rejects_bad_timelines() {
        let db = sta_content::fake::db();
        assert!(ScriptedSource::from_json5("{ units: [{ id: 1, def: \"nope\", rails: [{ at: 0, pos: {x:0,y:0,z:0} }] }] }", &db).is_err());
        assert!(ScriptedSource::from_json5("{ units: [{ id: 1, rails: [] }] }", &db).is_err());
        assert!(ScriptedSource::from_json5("{ units: [", &db).is_err());
    }

    fn real_run() -> (Sim, Vec<Vec<Command>>) {
        let mut sim = Sim::new(3);
        let stats = sta_content::MoveStats::from_f32(140.0, 1.0, 1.0, 1.0, 0.2);
        let a = sim.spawn(0, Vec3::ZERO, (Fixed::ZERO, Fixed::ONE), stats);
        let b = sim.spawn(1, Vec3::from_f32([500.0, 0.0, 0.0]), (Fixed::ONE, Fixed::ZERO), stats);
        let mv = |player, unit, to| Command {
            player: PlayerId(player),
            seq: 0,
            kind: CommandKind::Order(Order::Move { units: vec![unit], to: Vec3::from_f32(to), queue: false }),
        };
        let batches = (0..300)
            .map(|t| match t {
                5 => vec![mv(0, a, [300.0, 0.0, -400.0]), mv(1, b, [0.0, 50.0, 800.0])],
                150 => vec![mv(0, a, [0.0, 0.0, 0.0])],
                _ => vec![],
            })
            .collect();
        (sim, batches)
    }

    #[test]
    fn view_log_round_trips_a_real_run() {
        let (mut sim, batches) = real_run();
        let log = ViewLog::record(&mut sim, batches.clone());
        let bytes = log.to_bytes();
        let mut replay = ViewLog::from_bytes(&bytes).unwrap();

        // A second, independent run of the same commands.
        let (mut again, _) = real_run();
        assert_eq!(replay.view(), again.view());
        for b in &batches {
            let want = WorldSource::step(&mut again, b);
            let got = replay.step(&[]);
            assert_eq!(got.view.digest, want.view.digest);
            assert_eq!(got, want);
        }
        assert_eq!(replay.view().digest, again.digest());
        assert_eq!(replay.view().tick, 300);
        // Past the end: the last view again, no events.
        let past = replay.step(&[]);
        assert_eq!(past.view.tick, 300);
        assert!(past.events.is_empty());
    }

    #[test]
    fn view_log_refuses_other_revisions() {
        let file = LogFile { rev: SEAM_REV + 1, initial: Arc::default(), ticks: Vec::new() };
        let bytes = postcard::to_allocvec(&file).unwrap();
        assert!(ViewLog::from_bytes(&bytes).is_err());
    }
}
