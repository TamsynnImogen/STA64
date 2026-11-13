//! Mission-side intents that can wait for a spawned entity's receipt.
//! Parsing retains the existing `Entity:giveOrder` call shapes and diagnostic
//! strings; dispatch and dependency scheduling belong to the runtime.

use mlua::{MultiValue, Value};
use sta_core::{PlayerId, Vec3};

use super::entity::Entity;
use super::entity_ref::EntityRef;
use super::{Shared, point_of, to_vec3};

#[derive(Debug, Clone)]
pub(crate) enum TargetRef {
    None,
    Entity(EntityRef),
    Point(Vec3),
}

#[derive(Debug, Clone)]
pub(crate) enum OrderIntent {
    Move { to: Vec3, queue: bool },
    Stop,
    Attack { target: EntityRef },
    Mine { node: EntityRef },
    Special { slot: u8, target: TargetRef },
}

#[derive(Debug, Clone)]
pub(crate) enum EntityAction {
    Order(OrderIntent),
    Remove,
    Immortal(bool),
    Cloak(bool),
    Hull(f64),
    ForceOntoMap(bool),
    PerceivedTeam(PlayerId),
    /// A numeric or text attribute edit (SIM-54/55/57), held for a pending spawn like the others.
    Attr(AttrEdit),
}

/// An attribute assignment of the Roots `Entity` binding (and the A2/A1 setters of the same
/// rules) as a value that waits for a pending spawn's id and then becomes one sim edit.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AttrEdit {
    MaxHull(f64),
    HullValue(f64),
    MaxCrew(f64),
    Crew(f64),
    MaxShields(f64),
    /// A fraction of the maximum (`Entity.shields =`).
    Shields(f64),
    Name(String),
    Race(String),
    /// A horizontal direction (x, z), already scaled into range.
    Heading(f64, f64),
    Omega(f64),
}

impl AttrEdit {
    /// The call's name in counted feedback.
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::MaxHull(_) => "Entity.maxHull",
            Self::HullValue(_) => "Entity.hullValue",
            Self::MaxCrew(_) => "Entity.maxCrew",
            Self::Crew(_) => "Entity.crew",
            Self::MaxShields(_) => "Entity.maxShields",
            Self::Shields(_) => "Entity.shields",
            Self::Name(_) => "Entity.name",
            Self::Race(_) => "Entity.race",
            Self::Heading(..) => "Entity.forward",
            Self::Omega(_) => "Entity.omega",
        }
    }

    /// What a read of the attribute shows until the sim's view does.
    pub(super) fn overlay(&self) -> super::spawn::Overlay {
        use super::spawn::Overlay::{Num, Text};
        match self {
            Self::MaxHull(v) | Self::HullValue(v) | Self::MaxCrew(v) | Self::Crew(v) | Self::MaxShields(v) | Self::Shields(v) | Self::Omega(v) => Num(*v),
            Self::Heading(x, _) => Num(*x),
            Self::Name(s) | Self::Race(s) => Text(s.clone()),
        }
    }

    /// The sim edit for the resolved unit.
    pub(crate) fn edit(&self, id: sta_core::EntityId) -> sta_sim::command::Edit {
        use sta_core::Fixed;
        use sta_sim::command::Edit;
        match self {
            Self::MaxHull(v) => Edit::MaxHull(id, Fixed::from_f64(*v)),
            Self::HullValue(v) => Edit::HullValue(id, Fixed::from_f64(*v)),
            Self::MaxCrew(v) => Edit::MaxCrew(id, Fixed::from_f64(*v)),
            Self::Crew(v) => Edit::UnitCrew(id, Fixed::from_f64(*v)),
            Self::MaxShields(v) => Edit::MaxShieldsKeep(id, Fixed::from_f64(*v)),
            Self::Shields(v) => Edit::ShieldPercent(id, Fixed::from_f64(*v)),
            Self::Name(s) => Edit::Name(id, s.clone()),
            Self::Race(s) => Edit::UnitRace(id, s.clone()),
            Self::Heading(x, z) => Edit::Heading(id, (Fixed::from_f64(*x), Fixed::from_f64(*z))),
            Self::Omega(v) => Edit::AngularVelocity(id, Fixed::from_f64(*v)),
        }
    }
}

impl EntityAction {
    /// References needed in addition to the action's source entity.
    pub(crate) fn dependencies(&self) -> Vec<&EntityRef> {
        match self {
            Self::Order(OrderIntent::Attack { target }) => vec![target],
            Self::Order(OrderIntent::Mine { node }) => vec![node],
            Self::Order(OrderIntent::Special {
                target: TargetRef::Entity(target),
                ..
            }) => vec![target],
            Self::Order(OrderIntent::Move { .. } | OrderIntent::Stop | OrderIntent::Special { .. })
            | Self::Remove
            | Self::Immortal(_)
            | Self::Cloak(_)
            | Self::Hull(_)
            | Self::ForceOntoMap(_)
            | Self::PerceivedTeam(_)
            | Self::Attr(_) => Vec::new(),
        }
    }
}

/// Parse the same supported names, targets and queue flag as the existing
/// `Entity:giveOrder` bridge. No Lua value is retained after this call.
pub(crate) fn parse_order(ctx: &Shared, actor: &EntityRef, args: &MultiValue) -> Result<OrderIntent, String> {
    let Some(Value::String(name)) = args.front() else {
        return Err("giveOrder: no order".into());
    };
    let name = name.to_string_lossy();
    let target = args.get(1).cloned().unwrap_or(Value::Nil);
    let queue = matches!(args.get(2), Some(Value::Boolean(true)));
    let target_entity = |v: &Value| match v {
        Value::UserData(u) => u.borrow::<Entity>().ok().map(|e| e.key.clone()),
        _ => None,
    };

    match name.as_str() {
        "Go" | "GoSingle" | "MoveHigh" | "MoveMedium" | "MoveLow" | "GoWarp" | "GoWarpSingle" | "Scout" => {
            match point_of(ctx, &target) {
                Some((x, y, z)) => Ok(OrderIntent::Move {
                    to: to_vec3(x, y, z),
                    queue,
                }),
                None => Err("giveOrder Go: no point".into()),
            }
        }
        "Stop" | "None" | "Default" => Ok(OrderIntent::Stop),
        "Attack" | "SearchAndDestroy" | "AnchoredAttack" => target_entity(&target)
            .map(|target| OrderIntent::Attack { target })
            .ok_or_else(|| "giveOrder Attack: no target entity".into()),
        "Mine" => {
            let node = if matches!(target, Value::Nil) {
                super::spawn::default_mine_node(&ctx.borrow(), actor)
                    .ok_or_else(|| "giveOrder Mine: no resource node".to_owned())?
            } else {
                target_entity(&target).ok_or_else(|| "giveOrder Mine: no target entity".to_owned())?
            };
            Ok(OrderIntent::Mine { node })
        }
        "SpecialWeaponHigh" | "SpecialWeaponMedium" | "SpecialAttack" => {
            let slot = if name == "SpecialWeaponMedium" { 1 } else { 0 };
            let target = match (target_entity(&target), point_of(ctx, &target)) {
                (Some(entity), _) => TargetRef::Entity(entity),
                (None, Some((x, y, z))) => TargetRef::Point(to_vec3(x, y, z)),
                _ => TargetRef::None,
            };
            Ok(OrderIntent::Special { slot, target })
        }
        other => Err(format!("giveOrder {other}")),
    }
}
