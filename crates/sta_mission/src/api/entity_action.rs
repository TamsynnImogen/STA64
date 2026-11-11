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
            | Self::PerceivedTeam(_) => Vec::new(),
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
