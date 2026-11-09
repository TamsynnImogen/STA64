//! The MMM Lua API (LUA-03), most used names first.
//!
//! Everything a script can reach lives in a [`Ctx`] shared by the Lua
//! bindings and the runtime: the latest [`WorldView`] and events, the
//! content database, the commands the script has queued, objectives, relation
//! overrides and monitors. Bindings read the view, never the `Sim`; whatever
//! changes the world becomes a [`Command`]:
//!
//! - orders (`unit:giveOrder`, `unit:cloak`) are sent as the unit's owner,
//!   like a player's input (the sim rejects them for other owners);
//! - world changes (`unit:remove`, `unit.hull = x`, `player.resources.metal =
//!   x`, `unit.player = p`, `unit.invincible = b`) are `CommandKind::Edit`
//!   from [`PlayerId::SCRIPT`] (D3/D4).
//!
//! # Bound names
//!
//! - `Vector(x, y, z)` (`+ - *`, `length`, `distance`);
//! - `Entity.Type.*`, `Order.*`, `Relation.*`, `SlotType.*` enum values;
//! - entity handles (from `EntityFinder`, `Player` and monitor callbacks):
//!   `id valid destroyed position forward velocity hull maxHull shields
//!   maxShields crew maxCrew player owner race odf class label cloaked
//!   invincible`, `isType giveOrder cloak decloak remove canPlayerSee`;
//! - `EntityFinder():find(filter)` / `:findOne(filter)`, a filter being a
//!   label or ODF name or a table `{label, odf, class, type, player, center,
//!   radius}`; results ascend by id;
//! - `Player(n)` / `Player.get(n)`, `Game.localPlayer`: `number name race
//!   color isAI slotType maxOfficers maxCrew resources`, `getRelation`,
//!   `setRelation`, `loadAIP` (recorded, no effect);
//! - `Objectives.objectives:add(title | {title, description})` returning an
//!   objective (`state title description id`, `:complete()`, `:fail()`),
//!   `Objectives.visible`, `Objectives.count`;
//! - `Monitor()` with `:hook(kind, ...)`, see [`monitor`];
//! - `Game.time localSlot localPlayer seed isHost gameSpeed allowSave`,
//!   `Mission.finish`, `print`.
//!
//! Signatures are inferred from the name inventory (`docs/MMM_API.md` lists
//! names, not arguments); each is a place to revisit when a script shows
//! otherwise. Unbound names fall back to the runtime's permissive stubs, also
//! as the fallback for unknown fields of bound objects.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use mlua::{Lua, Value};
use sta_content::{ContentDb, UnitDef};
use sta_core::{EntityId, Fixed, PlayerId, Vec3};
use sta_sim::command::{Command, CommandKind, Edit, Order};
use sta_sim::view::{Relation, UnitFlags, UnitView, WorldView};
use sta_sim::SimEvent;

mod entity;
mod monitor;
mod player;
#[cfg(test)]
mod tests;

pub use monitor::MonitorKind;
pub(crate) use monitor::{fire_monitors, MonitorState};

/// What a mission needs to know about its game that `Controller::think`
/// does not pass it (see the seam gaps in `crate::runtime`).
#[derive(Clone, Default)]
pub struct MissionConfig {
    /// The seat the mission runs as (`Game.localSlot`); `PlayerId(0)` if unset.
    pub seat: Option<PlayerId>,
    /// ODF names, classes and races for `odf`, `class`, `isType`.
    pub content: Option<Arc<ContentDb>>,
    /// Map labels by entity (`sim.labels`), for `EntityFinder` and `label`.
    pub labels: BTreeMap<EntityId, String>,
    /// The world before the first tick, so a script's load-time finds work.
    pub view: Option<WorldView>,
}

/// An objective's progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectiveState {
    Active,
    Complete,
    Failed,
}

/// One objective a script added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Objective {
    pub id: u32,
    pub title: String,
    pub description: String,
    pub state: ObjectiveState,
}

/// Script-visible state; see the module docs.
pub(crate) struct Ctx {
    pub seat: PlayerId,
    pub seed: u64,
    pub content: Option<Arc<ContentDb>>,
    pub labels: BTreeMap<EntityId, String>,
    pub view: WorldView,
    pub events: Vec<SimEvent>,
    pub commands: Vec<Command>,
    seq: BTreeMap<PlayerId, u32>,
    pub objectives: Vec<Objective>,
    pub objectives_visible: bool,
    pub relations: BTreeMap<(PlayerId, PlayerId), Relation>,
    pub monitors: Vec<MonitorState>,
    pub next_monitor: u64,
    pub finished: bool,
    pub log: Vec<String>,
    /// Orders and names the API accepted but could not carry out, by name.
    pub unsupported: BTreeMap<String, u32>,
    pub loaded_aips: Vec<(PlayerId, String)>,
}

pub(crate) type Shared = Rc<RefCell<Ctx>>;

impl Ctx {
    pub fn new(cfg: MissionConfig, seed: u64) -> Ctx {
        Ctx {
            seat: cfg.seat.unwrap_or(PlayerId(0)),
            seed,
            content: cfg.content,
            labels: cfg.labels,
            view: cfg.view.unwrap_or_default(),
            events: Vec::new(),
            commands: Vec::new(),
            seq: BTreeMap::new(),
            objectives: Vec::new(),
            objectives_visible: true,
            relations: BTreeMap::new(),
            monitors: Vec::new(),
            next_monitor: 0,
            finished: false,
            log: Vec::new(),
            unsupported: BTreeMap::new(),
            loaded_aips: Vec::new(),
        }
    }

    pub fn unit(&self, id: EntityId) -> Option<&UnitView> {
        self.view.unit(id)
    }

    /// Alive: in the view and not dying.
    pub fn alive(&self, id: EntityId) -> Option<&UnitView> {
        self.unit(id).filter(|u| !u.flags.contains(UnitFlags::DYING))
    }

    pub fn def(&self, u: &UnitView) -> Option<&UnitDef> {
        self.content.as_ref()?.unit(u.def)
    }

    pub fn odf_name(&self, u: &UnitView) -> String {
        self.content.as_ref().and_then(|c| c.name(u.def)).unwrap_or_default().to_owned()
    }

    pub fn push(&mut self, player: PlayerId, kind: CommandKind) {
        let s = self.seq.entry(player).or_insert(0);
        *s += 1;
        self.commands.push(Command { player, seq: *s, kind });
    }

    pub fn edit(&mut self, e: Edit) {
        self.push(PlayerId::SCRIPT, CommandKind::Edit(e));
    }

    /// An order from the unit's owner; false (and counted) if it has none.
    pub fn order(&mut self, id: EntityId, make: impl FnOnce(Vec<EntityId>) -> Order) -> bool {
        let Some(owner) = self.unit(id).map(|u| u.owner).filter(|o| o.is_player()) else {
            self.unsupported("order: no owner");
            return false;
        };
        self.push(owner, CommandKind::Order(make(vec![id])));
        true
    }

    pub fn unsupported(&mut self, what: &str) {
        *self.unsupported.entry(what.to_owned()).or_default() += 1;
    }

    /// How `from` stands toward `to`: script override, else the view.
    pub fn relation(&self, from: PlayerId, to: PlayerId) -> Relation {
        if from == to {
            return Relation::Ally;
        }
        if let Some(r) = self.relations.get(&(from, to)) {
            return *r;
        }
        self.view
            .player(from)
            .and_then(|p| p.relations.iter().find(|(o, _)| *o == to).map(|(_, r)| *r))
            .unwrap_or(Relation::Neutral)
    }
}

pub(crate) fn pos_f64(v: Vec3) -> (f64, f64, f64) {
    (v.x.to_f32() as f64, v.y.to_f32() as f64, v.z.to_f32() as f64)
}

pub(crate) fn to_vec3(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x: Fixed::from_f64(x), y: Fixed::from_f64(y), z: Fixed::from_f64(z) }
}

pub(crate) fn dist(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2) + (a.2 - b.2).powi(2)).sqrt()
}

/// A point from a `Vector`-like table or an entity handle.
pub(crate) fn point_of(ctx: &Ctx, v: &Value) -> Option<(f64, f64, f64)> {
    match v {
        Value::Table(t) => Some((t.get("x").ok()?, t.get("y").unwrap_or(0.0), t.get("z").ok()?)),
        Value::UserData(u) => {
            let e = u.borrow::<entity::Entity>().ok()?;
            ctx.unit(e.id).map(|u| pos_f64(u.pos))
        }
        _ => None,
    }
}

const ENTITY_TYPES: &[&str] = &[
    "AntimatterMine", "AreaEffectObject", "Artillery", "AsteroidField", "BackgroundObject", "BackgroundPlanet", "Beam",
    "BeamToggle", "BlackHole", "Bullet", "CargoShip", "Colony", "ConstructionObject", "ConstructionRig", "Craft", "Entity",
    "Evolver", "Explosion", "Freighter", "GameObject", "GravityMine", "IonStorm", "LatinumNebula", "Mine", "MineArray",
    "MiningStation", "Missile", "Nebula", "Ordnance", "Phaser", "PhotonTorpedo", "Planet", "PlanetMiningBase", "Producer",
    "PulsePhaser", "RepairShip", "ResearchPod", "ResearchStation", "ResourceInterface", "ResourceTransferInterface",
    "Scavenger", "Scrap", "SensorArray", "Shipyard", "Starbase", "TerrainObject", "TradingStation", "UtritiumBall", "Wormhole",
];

pub(crate) const ORDERS: &[&str] = &[
    "AnchoredAttack", "Attack", "Build", "BuyResource", "CancelBuild", "CancelBuildQueueItem", "Colonize", "Decommision",
    "Default", "Defend", "EndTrade", "Follow", "FormFormation", "Formation", "GetSuperRepair", "GiveResource", "Go", "GoRally",
    "GoSingle", "GoWarp", "GoWarpSingle", "GreenAlert", "Guard", "Mine", "MoveHigh", "MoveLow", "MoveMedium", "None", "Patrol",
    "Queue", "Recrew", "RecycleDilithium", "RecycleMetal", "RedAlert", "Repair", "RepairOther", "Scout", "SearchAndDestroy",
    "Select", "SellResource", "SetAttackMode", "SetFormation", "SetFormationOrientation", "SetRallyPoint", "SpecialAttack",
    "SpecialWeaponHigh", "SpecialWeaponMedium", "SpecialWeaponNone", "StartTrade", "Stop", "Trading", "Transport",
    "TransportAttack", "TransportSpecial", "WeaponQueue", "Wormhole", "YellowAlert",
];

const SLOT_TYPES: &[&str] = &["AI_Easy", "AI_Hard", "AI_Neural", "AI_Normal", "Human", "None", "Observer"];

const VECTOR: &str = r#"
local V = {}
V.__index = V
local function new(x, y, z) return setmetatable({ x = x or 0, y = y or 0, z = z or 0 }, V) end
V.__add = function(a, b) return new(a.x + b.x, a.y + b.y, a.z + b.z) end
V.__sub = function(a, b) return new(a.x - b.x, a.y - b.y, a.z - b.z) end
V.__mul = function(a, b)
  if type(a) == "number" then a, b = b, a end
  if type(b) == "number" then return new(a.x * b, a.y * b, a.z * b) end
  return a.x * b.x + a.y * b.y + a.z * b.z
end
V.__unm = function(a) return new(-a.x, -a.y, -a.z) end
V.__eq = function(a, b) return a.x == b.x and a.y == b.y and a.z == b.z end
V.__tostring = function(a) return "(" .. a.x .. ", " .. a.y .. ", " .. a.z .. ")" end
function V:length() return math.sqrt(self.x * self.x + self.y * self.y + self.z * self.z) end
function V:distance(o) return (self - o):length() end
Vector = setmetatable({}, { __call = function(_, x, y, z) return new(x, y, z) end, __index = V })
return new
"#;

fn enum_table(lua: &Lua, names: &[&str]) -> mlua::Result<mlua::Table> {
    let t = lua.create_table()?;
    for n in names {
        t.set(*n, *n)?;
    }
    Ok(t)
}

/// A fresh `Vector` table (from the Lua prelude's constructor).
pub(crate) fn vector(lua: &Lua, p: (f64, f64, f64)) -> mlua::Result<mlua::Table> {
    let new: mlua::Function = lua.globals().raw_get("__vector")?;
    new.call(p)
}

/// What an unknown field of a bound object returns: a permissive stub.
pub(crate) fn stub(lua: &Lua) -> mlua::Result<Value> {
    match lua.globals().raw_get::<Value>("__stub")? {
        Value::Function(f) => f.call(()),
        _ => Ok(Value::Nil),
    }
}

/// Bind the API into `lua`. Called after the base environment and before
/// the permissive stubs are installed.
pub(crate) fn install(lua: &Lua, ctx: &Shared) -> mlua::Result<()> {
    let g = lua.globals();
    let new_vec: mlua::Function = lua.load(VECTOR).set_name("=vector").eval()?;
    g.raw_set("__vector", new_vec)?;

    let entity_t = lua.create_table()?;
    entity_t.set("Type", enum_table(lua, ENTITY_TYPES)?)?;
    g.raw_set("Entity", entity_t)?;
    g.raw_set("Order", enum_table(lua, ORDERS)?)?;
    g.raw_set("SlotType", enum_table(lua, SLOT_TYPES)?)?;
    g.raw_set("Relation", enum_table(lua, &["Ally", "Enemy", "Neutral"])?)?;

    entity::install(lua, ctx)?;
    player::install(lua, ctx)?;
    monitor::install(lua, ctx)?;

    let mission = lua.create_table()?;
    let c = ctx.clone();
    mission.set(
        "finish",
        lua.create_function(move |_, _: mlua::MultiValue| {
            c.borrow_mut().finished = true;
            Ok(())
        })?,
    )?;
    g.raw_set("Mission", mission)?;
    let c = ctx.clone();
    g.raw_set(
        "print",
        lua.create_function(move |_, args: mlua::Variadic<Value>| {
            let line = args.iter().map(|v| v.to_string().unwrap_or_default()).collect::<Vec<_>>().join("\t");
            let mut c = c.borrow_mut();
            if c.log.len() < 1000 {
                c.log.push(line);
            }
            Ok(())
        })?,
    )?;
    Ok(())
}
