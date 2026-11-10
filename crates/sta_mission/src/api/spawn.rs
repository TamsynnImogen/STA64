//! Queued Entity.add handles. One mission runtime owns the SCRIPT stream.
//! No Lua values are stored here: pending actions contain only typed data and
//! references whose state is resolved from the simulation's actual receipts.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use sta_core::{ContentKey, EntityId, Fixed, PlayerId, Vec3};
use sta_sim::SimEvent;
use sta_sim::world::{EntityKind, NEUTRAL};
use sta_sim::command::{Command, CommandKind, Edit, Order, Target};

use super::Ctx;
use super::entity_action::{EntityAction, OrderIntent, TargetRef};
use super::entity_ref::{EntityRef, SpawnHandle, SpawnStatus};

#[derive(Default)]
pub(crate) struct SpawnStore {
    next: u64,
    outstanding: BTreeMap<u64, Rc<SpawnHandle>>,
    actions: Vec<QueuedAction>,
}

struct QueuedAction {
    actor: EntityRef,
    action: EntityAction,
}

/// Exact squared XZ distance can require 129 bits across the Fixed range.
/// The carry is the high bit, so tuple ordering remains distance ordering.
fn flat_distance(a: Vec3, b: Vec3) -> (bool, u128) {
    let dx = (i128::from(a.x.to_bits()) - i128::from(b.x.to_bits())).unsigned_abs();
    let dz = (i128::from(a.z.to_bits()) - i128::from(b.z.to_bits())).unsigned_abs();
    let (low, carry) = (dx * dx).overflowing_add(dz * dz);
    (carry, low)
}

/// STA64's omitted-target Mine policy: choose once, using content-backed
/// resource kinds and current/request positions. Existing nodes win ties,
/// then actual IDs or request tokens. Receipts still determine actual IDs.
pub(super) fn default_mine_node(c: &Ctx, actor: &EntityRef) -> Option<EntityRef> {
    let actor = actor.id().and_then(|id| c.alive(id)).or_else(|| actor.hint())?;
    let resource = |unit: &sta_sim::view::UnitView| {
        c.def(unit).is_some_and(|def| EntityKind::of(&def.class, &def.movement) == EntityKind::Resource)
    };
    let current = c.view.units.iter()
        .filter(|unit| unit.id != EntityId::NONE
            && !unit.flags.contains(sta_sim::view::UnitFlags::DYING)
            && resource(unit))
        .map(|unit| (
            (flat_distance(actor.pos, unit.pos), 0u8, u64::from(unit.id.0)),
            EntityRef::Existing(unit.id),
        ));
    let pending = c.spawns.outstanding.values()
        .filter(|handle| matches!(handle.state.get(), SpawnStatus::Queued | SpawnStatus::Submitted(_))
            && resource(&handle.hint))
        .map(|handle| (
            (flat_distance(actor.pos, handle.hint.pos), 1u8, handle.request),
            EntityRef::Spawn(handle.clone()),
        ));
    current.chain(pending).min_by_key(|(key, _)| *key).map(|(_, node)| node)
}

pub(super) fn queue_spawn(c: &mut Ctx, odf: ContentKey, owner: PlayerId, pos: Vec3) -> Result<EntityRef, &'static str> {
    if !owner.is_player() && owner != NEUTRAL {
        return Err("Entity.add requires a player slot 0..7 or a neutral resource owner");
    }
    let content = c.content.as_ref().ok_or("Entity.add requires content")?;
    let def = content.unit(odf).ok_or("Entity.add requires a unit definition")?;
    // Match the core's resource-neutrality classification, including explicit
    // resource metadata on definitions whose class has a different kind.
    if owner == NEUTRAL
        && EntityKind::of(&def.class, &def.movement) != EntityKind::Resource
        && def.resource.is_none()
    {
        return Err("Entity.add neutral ownership requires a resource definition");
    }
    let heading = (Fixed::ZERO, Fixed::ONE);
    // This is request data for getters, not a unit inserted in the world.
    // Its NONE id must never be emitted in a command or exposed as a real ID.
    let hint = sta_sim::world::Entity::from_def(def, content.features(), owner, pos, heading).view();
    let request = c.spawns.next.checked_add(1).ok_or("spawn request ids exhausted")?;
    let seq = c.try_push(
        PlayerId::SCRIPT,
        CommandKind::Edit(Edit::SpawnRequest {
            request,
            odf,
            player: owner,
            pos,
            heading,
        }),
    )?;
    let handle = Rc::new(SpawnHandle {
        request,
        seq,
        hint,
        state: Cell::new(SpawnStatus::Queued),
    });
    c.spawns.next = request;
    c.spawns.outstanding.insert(request, handle.clone());
    Ok(EntityRef::Spawn(handle))
}

/// Mark only requests actually drained to the host as submitted. A queued
/// handle cannot accidentally consume a receipt that predates its command.
pub(super) fn drain(c: &mut Ctx) -> Vec<Command> {
    let commands = std::mem::take(&mut c.commands);
    for command in &commands {
        if command.player != PlayerId::SCRIPT {
            continue;
        }
        let CommandKind::Edit(Edit::SpawnRequest { request, .. }) = &command.kind else {
            continue;
        };
        if let Some(handle) = c.spawns.outstanding.get(request)
            && handle.seq == command.seq
            && handle.state.get() == SpawnStatus::Queued
        {
            handle.state.set(SpawnStatus::Submitted(c.view.tick));
        }
    }
    commands
}

pub(super) fn queue_action(c: &mut Ctx, actor: EntityRef, action: EntityAction) -> bool {
    let mut references = action.dependencies();
    references.push(&actor);
    if references.iter().any(|r| r.id().is_none() && !r.pending()) {
        c.unsupported("pending entity action: failed spawn");
        return false;
    }
    // Once an actor waits for a reference, later intents stay behind it.
    // Otherwise an older deferred Attack could override a newer Stop.
    let blocked = c.spawns.actions.iter().any(|q| q.actor.same_entity(&actor));
    if references.iter().any(|r| r.pending()) || blocked {
        c.spawns.actions.push(QueuedAction { actor, action });
        return true;
    }
    dispatch(c, actor, action)
}

/// Called after replacing the view/events, before any Lua callbacks. WorldSource
/// must deliver the complete events of a submitted batch on the next view.
pub(super) fn process(c: &mut Ctx) {
    let mut receipts: BTreeMap<u64, Vec<EntityId>> = BTreeMap::new();
    let mut ids: BTreeMap<EntityId, usize> = BTreeMap::new();
    let mut rejected = BTreeSet::new();
    for event in &c.events {
        match *event {
            SimEvent::ScriptSpawned { request, id } => {
                receipts.entry(request).or_default().push(id);
                *ids.entry(id).or_default() += 1;
            }
            SimEvent::Rejected {
                player: PlayerId::SCRIPT,
                seq,
            } => {
                rejected.insert(seq);
            }
            _ => {}
        }
    }
    for token in receipts.keys() {
        let expected = c
            .spawns
            .outstanding
            .get(token)
            .is_some_and(|h| matches!(h.state.get(), SpawnStatus::Submitted(t) if c.view.tick > t));
        if !expected {
            c.unsupported("Entity.add: unexpected spawn receipt");
        }
    }
    let outstanding: Vec<_> = c.spawns.outstanding.values().cloned().collect();
    for handle in outstanding {
        let SpawnStatus::Submitted(tick) = handle.state.get() else {
            continue;
        };
        if c.view.tick <= tick {
            continue;
        }
        let receipt = receipts.get(&handle.request);
        let failure = rejected.contains(&handle.seq);
        let resolved = match receipt.map(Vec::as_slice) {
            Some([id]) if !failure && *id != EntityId::NONE && ids.get(id) == Some(&1) => Some(*id),
            _ => None,
        };
        match resolved {
            Some(id) => {
                // Keep the actual ID even if a later command killed it in
                // this tick. Liveness comes from the view; actions below drop.
                handle.state.set(SpawnStatus::Resolved(id));
            }
            None => {
                handle.state.set(SpawnStatus::Failed);
                c.unsupported(if failure {
                    "Entity.add: spawn rejected"
                } else {
                    "Entity.add: missing or conflicting spawn receipt"
                });
            }
        }
        c.spawns.outstanding.remove(&handle.request);
    }
    for queued in std::mem::take(&mut c.spawns.actions) {
        let mut references = queued.action.dependencies();
        references.push(&queued.actor);
        if references.iter().any(|r| r.id().is_none() && !r.pending()) {
            c.unsupported("pending entity action: failed spawn");
        } else if references.iter().any(|r| r.pending())
            || c.spawns.actions.iter().any(|q| q.actor.same_entity(&queued.actor))
        {
            c.spawns.actions.push(queued);
        } else if references.iter().any(|r| r.id().is_none_or(|id| c.alive(id).is_none())) {
            c.unsupported("pending entity action: entity no longer alive");
        } else {
            dispatch(c, queued.actor, queued.action);
        }
    }
}

fn edit(c: &mut Ctx, value: Edit) -> bool {
    match c.try_push(PlayerId::SCRIPT, CommandKind::Edit(value)) {
        Ok(_) => true,
        Err(reason) => {
            c.unsupported(reason);
            false
        }
    }
}

fn dispatch(c: &mut Ctx, actor: EntityRef, action: EntityAction) -> bool {
    let Some(id) = actor.id() else { return false };
    match action {
        EntityAction::Remove => edit(c, Edit::Kill(id)),
        EntityAction::Immortal(on) => edit(c, Edit::Immortal(id, on)),
        EntityAction::Cloak(on) => c.order(id, |units| Order::Cloak { units, on }),
        EntityAction::Hull(to) => {
            if !to.is_finite() {
                c.unsupported("entity.hull: non-finite value");
                return false;
            }
            let Some(now) = c.alive(id).map(|u| u.hull[0].to_f32() as f64) else {
                return false;
            };
            if to < now {
                edit(c, Edit::Damage(id, Fixed::from_f64(now - to)))
            } else if to > now {
                edit(c, Edit::Heal(id))
            } else {
                true
            }
        }
        EntityAction::Order(intent) => match intent {
            OrderIntent::Move { to, queue } => c.order(id, |units| Order::Move { units, to, queue }),
            OrderIntent::Stop => c.order(id, |units| Order::Stop { units }),
            OrderIntent::Attack { target } => match target.id() {
                Some(target) => c.order(id, |units| Order::Attack { units, target }),
                None => false,
            },
            OrderIntent::Mine { node } => match node.id() {
                Some(node) => c.order(id, |units| Order::Mine { units, node }),
                None => false,
            },
            OrderIntent::Special { slot, target } => {
                let target = match target {
                    TargetRef::None => Target::None,
                    TargetRef::Point(p) => Target::Point(p),
                    TargetRef::Entity(e) => match e.id() {
                        Some(id) => Target::Entity(id),
                        None => return false,
                    },
                };
                c.order(id, |units| Order::Special { units, slot, target })
            }
        },
    }
}

#[cfg(test)]
mod tests;
