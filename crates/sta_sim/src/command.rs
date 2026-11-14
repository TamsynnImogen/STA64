//! Commands: the only input to the simulation (S2).
//!
//! - A tick applies its batch sorted by `(player, seq)`. Invalid commands
//!   (dead unit, wrong owner, not yet implemented) are ignored
//!   deterministically and produce [`SimEvent::Rejected`](crate::SimEvent).
//! - [`Edit`] is accepted only from [`PlayerId::BENCH`] (bench builds) and
//!   [`PlayerId::SCRIPT`] (missions). Edits are in the command log and the
//!   digest, so edited runs replay.
//! - Evolution: new variants and new fields with defaults within a
//!   milestone. The wire encoding (`postcard`) is versioned by `SEAM_REV`.

use serde::{Deserialize, Serialize};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, Tick, Vec3};

/// One player's (or script's, or the bench's) instruction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Command {
    pub player: PlayerId,
    /// Per player, increasing; orders a player's commands within a tick.
    pub seq: u32,
    pub kind: CommandKind,
}

impl Command {
    /// The order commands apply in within a tick.
    pub fn sort_key(&self) -> (PlayerId, u32) {
        (self.player, self.seq)
    }
}

/// Player orders and mission text; edits only scripts and the bench.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CommandKind {
    Order(Order),
    Edit(Edit),
    /// Text submitted by a configured, noneliminated player seat for mission
    /// hooks. Raises `SimEvent::MissionText` without changing physical state.
    /// UTF-8, at most [`MAX_MISSION_TEXT_BYTES`] bytes; empty text is allowed.
    /// Appended to preserve existing postcard command tags.
    MissionText { text: String },
}

/// Maximum UTF-8 byte length of a single mission-text command.
pub const MAX_MISSION_TEXT_BYTES: usize = 4096;

/// What a weapon, special or event aims at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Target {
    None,
    Entity(EntityId),
    Point(Vec3),
}

/// What a player's input makes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Order {
    /// `queue`: append a waypoint instead of replacing the course (Shift).
    Move { units: Vec<EntityId>, to: Vec3, queue: bool },
    Stop { units: Vec<EntityId> },
    Attack { units: Vec<EntityId>, target: EntityId },
    /// `at`: where to place a station; `None` for a ship from a yard.
    Build { builder: EntityId, what: ContentKey, at: Option<Vec3> },
    Formation { units: Vec<EntityId>, kind: u8 },
    Special { units: Vec<EntityId>, slot: u8, target: Target },
    Cloak { units: Vec<EntityId>, on: bool },
    Mine { units: Vec<EntityId>, node: EntityId },
    /// Set a builder's destination for newly produced mobile units. `None`
    /// clears it; entity positions are resolved when a unit is produced.
    /// Appended: existing postcard Order tags must keep their values.
    Rally { builders: Vec<EntityId>, at: Target },
    /// Set the movement or special-weapon autonomy of craft (SIM-38).
    /// Appended: existing postcard Order tags keep their values (`Rally` is
    /// 8, this is 9).
    Autonomy { units: Vec<EntityId>, kind: AutonomyKind, level: AutonomyLevel },
    /// Mission `Patrol(unit, path)` (SIM-56): fly the points in order, from the
    /// first, and from the last back to the first, until another order. Fewer
    /// than two points is a plain move. Appended tag 10.
    Patrol { units: Vec<EntityId>, points: Vec<Vec3> },
    /// Mission `SearchAndDestroy` (SIM-56): fight the best enemy the craft's
    /// sensors show, roam the map when there is none; never ends. Tag 11.
    SearchAndDestroy { units: Vec<EntityId> },
    /// Mission `Scout` (SIM-56): wander near the starting point to unexplored
    /// ground without firing; never ends. Tag 12.
    Scout { units: Vec<EntityId> },
    /// Mission `AI_Attack(unit)` (SIM-56): attack what the sensors show and
    /// stop when nothing is left. Tag 13.
    AttackAny { units: Vec<EntityId> },
    /// Mission `AI_Attack(unit, point)` (SIM-56): fly to weapon range of the
    /// point firing nothing on the way, then stop. Tag 14.
    AttackPoint { units: Vec<EntityId>, at: Vec3 },
    /// Mission `Goto(.., warp = true)` (SIM-56): a move that asks for warp when
    /// the goal is far. `queue` as [`Order::Move`]. Tag 15.
    MoveWarp { units: Vec<EntityId>, to: Vec3, queue: bool },
    /// Request yard repair (SIM-40). `Some(yard)` names a facility selected
    /// by the caller; `None` asks the repair runtime to choose a facility.
    /// The runtime owns eligibility, selection and repeat-order policy.
    /// Rejected without changing orders until that runtime is integrated.
    /// Appended tag 16 preserves every preceding Order tag and payload.
    Repair { units: Vec<EntityId>, yard: Option<EntityId> },
    /// Colonize a planet with pod colony ships (SIM-30; mission `Colonize`
    /// needs the ship's special-forces flag): see [`crate::colony`]. Tag 17.
    Colonize { units: Vec<EntityId>, planet: EntityId },
}

/// Which autonomy an [`Order::Autonomy`] sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AutonomyKind {
    Movement,
    SpecialWeapon,
}

/// An autonomy level as the originals have it: 0, 1, 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AutonomyLevel {
    Low,
    Medium,
    High,
}

/// Direct changes to the world: the bench's edits (docs/TESTBENCH.md,
/// "Commands") and missions' world changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Edit {
    Spawn { odf: ContentKey, player: PlayerId, pos: Vec3, heading: (Fixed, Fixed) },
    Kill(EntityId),
    Damage(EntityId, Fixed),
    Heal(EntityId),
    Immortal(EntityId, bool),
    /// Set absolute special energy on a live unit with a defined pool,
    /// clamped to 0..max. Normal regeneration/cloak drain follows that tick.
    /// Requires SCRIPT/BENCH privileges; missing/dead/no-pool targets reject.
    SetEnergy(EntityId, Fixed),
    /// Grant a technology to a player.
    Tech(PlayerId, ContentKey),
    /// Set a player's stock of a resource (by resource id).
    Cargo(PlayerId, u8, Fixed),
    /// Engines on or off.
    Engines(EntityId, bool),
    /// Weapons on or off.
    Weapons(EntityId, bool),
    /// Change an entity's owner.
    Team(EntityId, PlayerId),
    /// Reseed the sim's random sequence.
    Seed(u64),
    /// Something to show or play that changes nothing in the world: the sim
    /// raises [`SimEvent::Present`](crate::event::SimEvent::Present) with it
    /// that tick (see [`crate::present`]).
    Present(crate::present::Presentation),
    /// A subsystem on or off (mission `Disable*` calls). `Engines` and
    /// `Weapons` act like [`Edit::Engines`] / [`Edit::Weapons`]; `Shields`,
    /// `LifeSupport` and `Sensors` are recorded as switched off or on
    /// (SIM-55; the combat consequences are not modelled, see `combat/damage.rs`).
    Subsystem(EntityId, Subsystem, bool),
    /// Alert state (mission `SetRedAlert` and friends). Rejected until a
    /// package models alert states.
    Alert(EntityId, AlertState),
    /// Cap a unit's speed (`SetImpulseSpeed`), `None` to lift the cap.
    /// Rejected until movement applies it.
    MaxSpeed(EntityId, Option<Fixed>),
    /// Whether other units may target it (`AllowAsTarget`). Rejected until
    /// targeting applies it.
    Targetable(EntityId, bool),
    /// Whether its owner may decommission it (`SetCanBeDecommissioned`).
    /// Rejected until decommissioning exists.
    Decommissionable(EntityId, bool),
    /// A mission spawn with an opaque caller-owned correlation token. Only
    /// SCRIPT may issue this edit. Success emits ordinary `Spawned` and
    /// `ScriptSpawned { request, id }`; failure uses `Rejected { player, seq }`.
    /// Consumers allocate unique tokens and correlate their command sequence;
    /// the sim does not reserve IDs or deduplicate request tokens.
    /// Appended to preserve every existing postcard Edit tag.
    SpawnRequest { request: u64, odf: ContentKey, player: PlayerId, pos: Vec3, heading: (Fixed, Fixed) },
    /// Set absolute hull at application time without changing shields.
    /// Clamped to 0..max hull, with the existing immortal floor (one hull,
    /// bounded by max). Zero destroys a mortal target normally. Requires a
    /// live target and SCRIPT/BENCH privileges; ordinary players are rejected.
    /// Appended to preserve every existing postcard Edit tag.
    Hull(EntityId, Fixed),
    /// Only changes the saved flag; projection occurs on position updates.
    /// Requires a live represented actor with usable bounds and SCRIPT/BENCH.
    /// Appended tag 20 preserves every previous payload and tag.
    ForceOntoMap { id: EntityId, enabled: bool },
    /// Global target identity, independent of ownership. SCRIPT/BENCH only.
    /// Appended tag 21 preserves all older edit tags.
    PerceivedTeam(EntityId, PlayerId),
    /// Mission `SetShieldPercent` / `Entity.shields` (SIM-55): every shield
    /// pool of a live craft or station to this fraction of its maximum,
    /// clamped to 0..1. SCRIPT/BENCH only. Appended tag 22.
    ShieldPercent(EntityId, Fixed),
    /// Mission `SetMaxShields` (SIM-55): the maximum, and every pool with it,
    /// to this value (negative reads as 0). SCRIPT/BENCH only. Appended tag 23.
    MaxShields(EntityId, Fixed),
    /// Mission `Damage<Subsystem>` (SIM-55): add this many points of damage to
    /// one subsystem, clamped to 0..its hit points (negative repairs). Rejected
    /// for a unit whose ODF has no hit points for it. SCRIPT/BENCH only.
    /// Appended tag 24.
    SubsystemDamage(EntityId, Subsystem, Fixed),
    /// Mission `Player.maxOfficers` (SIM-54): the player's officer limit,
    /// floored at 0 and capped at 1e9. Rejected for a seat with no initialized
    /// officer limit. SCRIPT/BENCH only. Appended tag 25.
    MaxOfficers(PlayerId, Fixed),
    /// Mission `SetCrew` / `Entity.crew` (SIM-54): a craft's or station's
    /// current crew, negative read as 0 and clamped to the unit's own current
    /// maximum crew (`Craft::SetCrew`, read from the executable, static; it is
    /// not clamped to the global `MAX_CREW`). Where the casualty model runs
    /// (the unit's features have `crew_loss_on_hit`) the derelict check of
    /// SIM-39 follows: crew under 1 latches the unit derelict and a later edit
    /// to 1 or more clears it by the next tick. Without that feature the edit
    /// never latches. Rejected for a unit with no crew pool.
    /// SCRIPT/BENCH only. Appended tag 26.
    UnitCrew(EntityId, Fixed),
    /// Mission `resources.crew` (SIM-54): the player's crew stock, floored at 0
    /// and capped at 1e9 (not clamped to the team capacity, which a stock may
    /// exceed). Rejected for a seat with no initialized stock. SCRIPT/BENCH
    /// only. Appended tag 27.
    TeamCrew(PlayerId, Fixed),
    /// Mission `SetPosition` (SIM-57): move a unit to this point at once with
    /// its speed zeroed; heading, turn rate, target and queued waypoints are
    /// kept. A unit whose ForceOntoMap flag is set is then confined by the map
    /// rule (the position update of `boundary`); there is no solid-contact
    /// recovery, as in the original. Rejected for a dying unit and for one under
    /// construction (STA64 policy, not measured). SCRIPT/BENCH only. Appended
    /// tag 28.
    Position(EntityId, Vec3),
    /// Mission `PointAt` / Roots `Entity.forward =` (SIM-57): face this
    /// direction at once (x, z of a horizontal direction; the sim normalises
    /// it; a zero direction or a dying unit is rejected). Speed, turn rate and
    /// orders are kept. SCRIPT/BENCH only. Appended tag 29.
    Heading(EntityId, (Fixed, Fixed)),
    /// Mission `SetAngularVelocity` / Roots `Entity.omega =` (SIM-57): the
    /// yaw turn rate in radians per second, positive from +X toward +Z,
    /// clamped to +-[`MAX_SCRIPT_OMEGA`]. Stored only for a craft with its
    /// engines off, which then turns without decay; for any other live unit
    /// the edit is accepted and changes nothing (a craft under its own
    /// steering has the value overwritten within a tick in the original).
    /// A dying unit is rejected. SCRIPT/BENCH only. Appended tag 30.
    AngularVelocity(EntityId, Fixed),
    /// Mission `AllowAsTarget(object, team, allowed)` (SIM-58): seat `player`'s
    /// craft may (not) pick the unit on their own; explicit attack orders are
    /// unaffected. Beside [`Edit::Targetable`], which stays. SCRIPT/BENCH only.
    /// Appended tag 31.
    AllowAsTarget { id: EntityId, player: PlayerId, allowed: bool },
    /// Mission `CraftCanRepair` (SIM-58): false stops shield regeneration.
    /// Craft only. Appended tag 32.
    CraftCanRepair(EntityId, bool),
    /// Mission `ButItCanBeDerelicted` (SIM-58): stored, no derelict state yet.
    /// Craft only. Appended tag 33.
    DerelictAllowed(EntityId, bool),
    /// Mission `CraftCanRecrew` (SIM-58): stored, no per-craft recrew yet.
    /// Craft only. Appended tag 34.
    CanRecrew(EntityId, bool),
    /// Mission `HideObject` (SIM-58): the hidden flag (drawing only in the
    /// original). Appended tag 35.
    Hidden(EntityId, bool),
    /// Mission `SetName` (SIM-58): a craft's display name, at most
    /// [`crate::switches::MAX_NAME_BYTES`] bytes; empty clears. Appended tag 36.
    Name(EntityId, String),
    /// Mission `SetSpecialForcesFlag` (SIM-58): the flag the original's script
    /// order gate, assist and AI planner read. Craft only. Appended tag 37.
    SpecialForces(EntityId, bool),
    /// Roots `Entity.maxHull =` (SIM-54): this unit's maximum hull. A negative
    /// value is accepted and ignored (the original keeps the old maximum);
    /// zero (kept as the least positive maximum, so the unit stays hittable) and large values are accepted, capped at 1e9. The
    /// current hull is not rescaled or clamped, so it may exceed the maximum
    /// (read from the executable, static). Dying units and non-units are
    /// rejected. SCRIPT/BENCH only. Appended tag 38.
    MaxHull(EntityId, Fixed),
    /// Roots `Entity.maxCrew =` (SIM-54): this unit's maximum crew, floored at
    /// 0 and capped at 1e9; the current crew is clamped down to it and never
    /// raised. Rejected for a unit with no crew pool. SCRIPT/BENCH only.
    /// Appended tag 39.
    MaxCrew(EntityId, Fixed),
    /// Roots `Entity.hullValue =` (SIM-54; A2 `SetCurrentHealth` and Roots `Entity.hull =` still send
    /// [`Edit::Hull`], which clamps): the unit's
    /// current hull in points, with no upper clamp (it may exceed the maximum).
    /// An immortal unit ignores a lower value; zero or less leaves the least
    /// positive hull (the original does not destroy at 0, the next hit does).
    /// Rejected for a dying unit. SCRIPT/BENCH only. Appended tag 40.
    HullValue(EntityId, Fixed),
    /// Roots `Entity.maxShields =` (SIM-54): only the maximum, floored at 0 and
    /// capped at 1e9; the current shield pools are not touched (Roots writes
    /// the field raw; A2 `SetMaxShields` is [`Edit::MaxShields`], which sets
    /// both). SCRIPT/BENCH only. Appended tag 41.
    MaxShieldsKeep(EntityId, Fixed),
    /// A2 `SetRace` / Roots `Entity.race =` (SIM-54): this unit's race (the
    /// unit only; team, tech tree and AI are unchanged). The name must be a
    /// race some unit definition carries, matched case-insensitively and stored as the content spells it; anything
    /// else is rejected. SCRIPT/BENCH only. Appended tag 42.
    UnitRace(EntityId, String),
    /// Roots `Player.race =` (SIM-54): the seat's race, with the same name
    /// rule as [`Edit::UnitRace`]; rejected for a seat the sim does not have.
    /// The sim keeps and shows it; no rule reads it yet. SCRIPT/BENCH only.
    /// Appended tag 43.
    PlayerRace(PlayerId, String),
    /// A scripted edit of a team's pool (SIM-54), with the rule of the call that
    /// sent it: crew stock add or set (`AddCrew` / `SetTeamCrew` of A2 and A1,
    /// clamped to the team's crew capacity, no floor), the raw capacity write
    /// of Roots `Player.maxCrew`, and A1 `AddOfficers` / `SetOfficers`
    /// (clamped to the officer cap). Rejected for a seat the sim does not
    /// have, for a capacity-clamped edit when the content has no crew rules,
    /// and for an officer edit on a seat with no officer limit. SCRIPT/BENCH
    /// only. Appended tag 44.
    TeamPool(PlayerId, TeamOp, Fixed),
    /// Mission `SetRelation(a, b, r)` / Roots `Player:setRelation` (SIM-63):
    /// seat `from`'s own view of seat `to` becomes `relation` (directional:
    /// the reverse view is untouched). Both must be configured, distinct
    /// seats; anything else is rejected. Takes effect on the next tick: units
    /// stop or start engaging, shared vision follows mutual Ally. Setting the
    /// default (same team Ally, other team Enemy) removes the override. Wire
    /// bounds: seats are bytes and `Relation` has three variants (Ally,
    /// Neutral, Enemy); a variant index above 2 fails to decode. SCRIPT/BENCH
    /// only. Appended tag 45.
    SetRelation { from: PlayerId, to: PlayerId, relation: crate::view::Relation },
    /// Mission `SetPlanetCivPop` (SIM-30): a Producer planet's population in
    /// people, kept as given within +-1e9 (growth cuts a value above the
    /// planet's cap on the next tick; a value of 0 or less neutralizes an
    /// owned planet). Creates the planet's population row. Rejected for any
    /// other target (STA64 policy; the original has no type check).
    /// SCRIPT/BENCH only. Appended tag 46.
    PlanetPopulation(EntityId, Fixed),
}

/// Which team-pool rule an [`Edit::TeamPool`] applies (appended variants only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TeamOp {
    /// A2 `AddCrew(team, x)`: over capacity a positive add is dropped and a
    /// negative one applied, otherwise the sum clamped to capacity.
    CrewAddA2,
    /// A2 `SetTeamCrew(team, v)`: `AddCrew(v - pool)`.
    CrewSetA2,
    /// A1 `AddCrew`: `min(pool + x, capacity)`.
    CrewAddA1,
    /// A1 `SetTeamCrew`: `min(v, capacity)`.
    CrewSetA1,
    /// Roots `Player.maxCrew = v`: the capacity becomes `v` (raw, unclamped);
    /// later capacity changes shift it by their delta.
    CapacityRaw,
    /// A1 `AddOfficers`: the officer count becomes `min(count + trunc(x), cap)`.
    OfficersAddA1,
    /// A1 `SetOfficers`: the officer count becomes `min(trunc(v), cap)`.
    OfficersSetA1,
}

/// The largest turn rate (radians per second) a script may set.
pub const MAX_SCRIPT_OMEGA: i32 = 100;

/// A unit system a mission can switch on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Subsystem {
    Engines,
    Weapons,
    Shields,
    LifeSupport,
    Sensors,
}

/// Armada's alert states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AlertState {
    Green,
    Yellow,
    Red,
}

/// Where commands wait between being issued and being applied. Local play
/// uses [`LocalRouter`]; lockstep (`sta_net`) holds a turn until every peer's
/// commands for it have arrived.
pub trait CommandRouter: Send {
    fn submit(&mut self, cmd: Command);
    /// The commands to apply on `tick`, sorted by `(player, seq)`; `None`
    /// when they are not all here yet and the sim must stall.
    fn take_ready(&mut self, tick: Tick) -> Option<Vec<Command>>;
}

/// Single player: everything submitted lands on the next tick; never stalls.
#[derive(Debug, Clone, Default)]
pub struct LocalRouter {
    pending: Vec<Command>,
}

impl LocalRouter {
    pub fn new() -> LocalRouter {
        LocalRouter::default()
    }
}

impl CommandRouter for LocalRouter {
    fn submit(&mut self, cmd: Command) {
        self.pending.push(cmd);
    }

    fn take_ready(&mut self, _tick: Tick) -> Option<Vec<Command>> {
        let mut batch = std::mem::take(&mut self.pending);
        // Stable: equal keys (a bug in the sender) keep submission order.
        batch.sort_by_key(Command::sort_key);
        Some(batch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop(player: u8, seq: u32) -> Command {
        Command { player: PlayerId(player), seq, kind: CommandKind::Order(Order::Stop { units: vec![EntityId(1)] }) }
    }

    #[test]
    fn local_router_delivers_next_tick_in_player_seq_order() {
        let mut r = LocalRouter::new();
        assert_eq!(r.take_ready(0), Some(vec![]));
        r.submit(stop(1, 0));
        r.submit(stop(0, 2));
        r.submit(stop(255, 0));
        r.submit(stop(0, 1));
        r.submit(stop(254, 7));
        let got: Vec<_> = r.take_ready(1).unwrap().iter().map(Command::sort_key).collect();
        let want = [(0, 1), (0, 2), (1, 0), (254, 7), (255, 0)].map(|(p, s)| (PlayerId(p), s));
        assert_eq!(got, want);
        // Delivered once: the next tick is empty.
        assert_eq!(r.take_ready(2), Some(vec![]));
    }

    #[test]
    fn commands_round_trip_through_postcard() {
        let c = Command {
            player: PlayerId(3),
            seq: 9,
            kind: CommandKind::Order(Order::Move { units: vec![EntityId(4), EntityId(5)], to: Vec3::from_f32([1.5, 0.0, -2.0]), queue: true }),
        };
        let bytes = postcard::to_allocvec(&c).unwrap();
        assert_eq!(postcard::from_bytes::<Command>(&bytes).unwrap(), c);
    }

    #[test]
    fn hull_appends_a_tag_without_renumbering_existing_edits() {
        let id = EntityId(1);
        let odf = ContentKey(7);
        let player = PlayerId(0);
        let heading = (Fixed::ZERO, Fixed::ONE);
        let edits = [
            Edit::Spawn { odf, player, pos: Vec3::ZERO, heading },
            Edit::Kill(id),
            Edit::Damage(id, Fixed::ONE),
            Edit::Heal(id),
            Edit::Immortal(id, true),
            Edit::SetEnergy(id, Fixed::ONE),
            Edit::Tech(player, odf),
            Edit::Cargo(player, 0, Fixed::ONE),
            Edit::Engines(id, true),
            Edit::Weapons(id, true),
            Edit::Team(id, player),
            Edit::Seed(1),
            Edit::Present(crate::present::Presentation::Cineractive { on: false, skippable: false }),
            Edit::Subsystem(id, Subsystem::Engines, true),
            Edit::Alert(id, AlertState::Green),
            Edit::MaxSpeed(id, None),
            Edit::Targetable(id, true),
            Edit::Decommissionable(id, true),
            Edit::SpawnRequest { request: 1, odf, player, pos: Vec3::ZERO, heading },
        ];
        for (tag, edit) in edits.iter().enumerate() {
            let bytes = postcard::to_allocvec(edit).unwrap();
            assert_eq!(bytes[0], tag as u8, "legacy edit {edit:?}");
            assert_eq!(postcard::from_bytes::<Edit>(&bytes).unwrap(), *edit);
        }
        assert_eq!(postcard::to_allocvec(&Edit::Hull(id, Fixed::ZERO)).unwrap(), [19, 1, 0]);
        for hull in [Fixed::from_bits(i64::MIN), Fixed::ZERO, Fixed::ONE, Fixed::from_bits(i64::MAX)] {
            let c = Command { player: PlayerId::SCRIPT, seq: 9, kind: CommandKind::Edit(Edit::Hull(id, hull)) };
            let bytes = postcard::to_allocvec(&c).unwrap();
            assert_eq!(postcard::from_bytes::<Command>(&bytes).unwrap(), c);
        }
    }

    #[test]
    fn shield_subsystem_and_personnel_edits_append_tags_22_to_27() {
        let id = EntityId(1);
        let edits = [
            (22, Edit::ShieldPercent(id, Fixed::ONE)),
            (23, Edit::MaxShields(id, Fixed::ONE)),
            (24, Edit::SubsystemDamage(id, Subsystem::Sensors, Fixed::ONE)),
            (25, Edit::MaxOfficers(PlayerId(0), Fixed::ONE)),
            (26, Edit::UnitCrew(id, Fixed::ONE)),
            (27, Edit::TeamCrew(PlayerId(0), Fixed::ONE)),
        ];
        for (tag, edit) in edits {
            let bytes = postcard::to_allocvec(&edit).unwrap();
            assert_eq!(bytes[0], tag, "{edit:?}");
            assert_eq!(postcard::from_bytes::<Edit>(&bytes).unwrap(), edit);
        }
    }

    #[test]
    fn placement_and_facing_edits_append_tags_28_to_30() {
        let id = EntityId(1);
        let edits = [
            (28, Edit::Position(id, Vec3::from_f32([1.0, 2.0, -3.0]))),
            (29, Edit::Heading(id, (Fixed::ONE, Fixed::ZERO))),
            (30, Edit::AngularVelocity(id, Fixed::ONE)),
        ];
        for (tag, edit) in edits {
            let bytes = postcard::to_allocvec(&edit).unwrap();
            assert_eq!(bytes[0], tag, "{edit:?}");
            assert_eq!(postcard::from_bytes::<Edit>(&bytes).unwrap(), edit);
        }
    }

    #[test]
    fn team_pool_edit_appends_tag_44() {
        for op in [TeamOp::CrewAddA2, TeamOp::CrewSetA2, TeamOp::CrewAddA1, TeamOp::CrewSetA1, TeamOp::CapacityRaw, TeamOp::OfficersAddA1, TeamOp::OfficersSetA1] {
            let edit = Edit::TeamPool(PlayerId(1), op, Fixed::ONE);
            let bytes = postcard::to_allocvec(&edit).unwrap();
            assert_eq!(bytes[0], 44, "{edit:?}");
            assert_eq!(postcard::from_bytes::<Edit>(&bytes).unwrap(), edit);
        }
    }

    #[test]
    fn set_relation_appends_tag_45_and_bounds_the_relation() {
        let e = Edit::SetRelation { from: PlayerId(1), to: PlayerId(2), relation: crate::view::Relation::Ally };
        let bytes = postcard::to_allocvec(&e).unwrap();
        assert_eq!(bytes, [45, 1, 2, 0]);
        assert_eq!(postcard::from_bytes::<Edit>(&bytes).unwrap(), e);
        // Neutral and Enemy are 1 and 2; nothing above decodes.
        assert!(postcard::from_bytes::<Edit>(&[45, 1, 2, 3]).is_err());
        assert!(postcard::from_bytes::<Edit>(&[45, 1, 2]).is_err());
    }

    #[test]
    fn attribute_edits_append_tags_38_to_43() {
        let id = EntityId(1);
        let edits = [
            (38, Edit::MaxHull(id, Fixed::ONE)),
            (39, Edit::MaxCrew(id, Fixed::ONE)),
            (40, Edit::HullValue(id, Fixed::ONE)),
            (41, Edit::MaxShieldsKeep(id, Fixed::ONE)),
            (42, Edit::UnitRace(id, "x".into())),
            (43, Edit::PlayerRace(PlayerId(0), "x".into())),
        ];
        for (tag, edit) in edits {
            let bytes = postcard::to_allocvec(&edit).unwrap();
            assert_eq!(bytes[0], tag, "{edit:?}");
            assert_eq!(postcard::from_bytes::<Edit>(&bytes).unwrap(), edit);
        }
    }

    #[test]
    fn planet_population_edit_appends_tag_46() {
        let edit = Edit::PlanetPopulation(EntityId(1), Fixed::ONE);
        let bytes = postcard::to_allocvec(&edit).unwrap();
        assert_eq!(bytes[0], 46);
        assert_eq!(postcard::from_bytes::<Edit>(&bytes).unwrap(), edit);
    }

    #[test]
    fn switch_edits_and_scripted_orders_append_tags() {
        let id = EntityId(1);
        let edits = [
            (31, Edit::AllowAsTarget { id, player: PlayerId(1), allowed: false }),
            (32, Edit::CraftCanRepair(id, false)),
            (33, Edit::DerelictAllowed(id, true)),
            (34, Edit::CanRecrew(id, true)),
            (35, Edit::Hidden(id, true)),
            (36, Edit::Name(id, "Voyager".into())),
            (37, Edit::SpecialForces(id, true)),
        ];
        for (tag, edit) in edits {
            let bytes = postcard::to_allocvec(&edit).unwrap();
            assert_eq!(bytes[0], tag, "{edit:?}");
            assert_eq!(postcard::from_bytes::<Edit>(&bytes).unwrap(), edit);
        }
        let u = vec![id];
        let at = Vec3::from_f32([1.0, 0.0, 2.0]);
        let orders = [
            (10, Order::Patrol { units: u.clone(), points: vec![at] }),
            (11, Order::SearchAndDestroy { units: u.clone() }),
            (12, Order::Scout { units: u.clone() }),
            (13, Order::AttackAny { units: u.clone() }),
            (14, Order::AttackPoint { units: u.clone(), at }),
            (15, Order::MoveWarp { units: u.clone(), to: at, queue: true }),
            (17, Order::Colonize { units: u, planet: id }),
        ];
        for (tag, o) in orders {
            let bytes = postcard::to_allocvec(&o).unwrap();
            assert_eq!(bytes[0], tag, "{o:?}");
            assert_eq!(postcard::from_bytes::<Order>(&bytes).unwrap(), o);
        }
    }

    #[test]
    fn rally_appends_a_wire_tag_and_round_trips_all_target_kinds() {
        let mine = Order::Mine { units: vec![], node: EntityId(1) };
        assert_eq!(postcard::to_allocvec(&mine).unwrap(), [7, 0, 1]);
        let clear = Order::Rally { builders: vec![], at: Target::None };
        assert_eq!(postcard::to_allocvec(&clear).unwrap(), [8, 0, 0]);
        for at in [Target::None, Target::Entity(EntityId(42)), Target::Point(Vec3::from_f32([1.5, -2.0, 3.0]))] {
            let c = Command { player: PlayerId(0), seq: 9, kind: CommandKind::Order(Order::Rally { builders: vec![EntityId(3)], at }) };
            let bytes = postcard::to_allocvec(&c).unwrap();
            assert_eq!(postcard::from_bytes::<Command>(&bytes).unwrap(), c);
        }
    }
}
