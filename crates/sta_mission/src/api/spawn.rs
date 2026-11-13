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
use super::entity_action::{AttrEdit, EntityAction, OrderIntent, TargetRef};
use super::entity_ref::{EntityRef, SpawnHandle, SpawnStatus};

#[derive(Default)]
pub(crate) struct SpawnStore {
    next: u64,
    outstanding: BTreeMap<u64, Rc<SpawnHandle>>,
    actions: Vec<QueuedAction>,
    /// Entities removed by the script since the last tick's receipts: (def, owner, place, label).
    /// A spawn of the same def for the same owner at the same place takes the label over.
    removed: Vec<(EntityId, ContentKey, PlayerId, Vec3, String)>,
    /// Entities the script removed since the last tick's receipts: gone to `EntityFinder` at once.
    killed: BTreeSet<EntityId>,
    /// What the script has just assigned, read back at once: the sim applies an edit on the next
    /// tick, the original's setters are synchronous (read from the executable, static). Keyed by
    /// the unit (or the pending request) and the attribute; a row of a resolved unit is valid in the
    /// view tick it was set in only.
    overlay: BTreeMap<(OverlayKey, &'static str), (u64, Overlay)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum OverlayKey {
    Id(EntityId),
    Request(u64),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Overlay {
    Num(f64),
    Text(String),
}

fn overlay_key(actor: &EntityRef) -> Option<OverlayKey> {
    match (actor.id(), actor) {
        (Some(id), _) => Some(OverlayKey::Id(id)),
        (None, EntityRef::Spawn(h)) if actor.pending() => Some(OverlayKey::Request(h.request)),
        _ => None,
    }
}

/// Remember an assignment for read-back until the sim's view shows it.
pub(super) fn overlay_set(c: &mut Ctx, actor: &EntityRef, attr: &'static str, value: Overlay) {
    if let Some(key) = overlay_key(actor) {
        let tick = c.view.tick;
        c.spawns.overlay.insert((key, attr), (tick, value));
    }
}

pub(super) fn overlay_get(c: &Ctx, actor: &EntityRef, attr: &'static str) -> Option<Overlay> {
    let key = overlay_key(actor)?;
    let (tick, value) = c.spawns.overlay.get(&(key, attr))?;
    (matches!(key, OverlayKey::Request(_)) || *tick == c.view.tick).then(|| value.clone())
}

/// The numeric overlay of an attribute, if any.
pub(super) fn overlay_num(c: &Ctx, actor: &EntityRef, attr: &'static str) -> Option<f64> {
    match overlay_get(c, actor, attr)? {
        Overlay::Num(v) => Some(v),
        Overlay::Text(_) => None,
    }
}

/// Assign an attribute: held for a pending spawn, else queued as its sim edit now.
pub(super) fn assign(c: &mut Ctx, actor: &EntityRef, edit: AttrEdit) -> bool {
    let attr = edit.label();
    let value = edit.overlay();
    if !queue_action(c, actor.clone(), EntityAction::Attr(edit)) {
        return false;
    }
    if let Some(value) = value {
        overlay_set(c, actor, attr, value);
    }
    true
}

/// Spawns the script has asked for that the sim has not answered yet. `EntityFinder` lists them
/// after the live units: the original adds an entity at once, and scripts look it up right after
/// `Entity.add` (a Roots mission does, then sets its crew and shields).
pub(super) fn pending_spawns(c: &Ctx) -> Vec<Rc<SpawnHandle>> {
    c.spawns
        .outstanding
        .values()
        .filter(|h| matches!(h.state.get(), SpawnStatus::Queued | SpawnStatus::Submitted(_)))
        .cloned()
        .collect()
}

/// Whether the script removed `id` since the last tick's receipts.
pub(super) fn is_killed(c: &Ctx, id: EntityId) -> bool {
    c.spawns.killed.contains(&id)
}

/// How near (squared, sim units) a spawn must be to a removed entity's place to replace it. The
/// script's position went through floating point on its way back.
const REPLACE_RADIUS_SQ: f64 = 1.0;

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
        c.def(unit).is_some_and(EntityKind::mineable_def)
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
        return Err("Entity.add requires a player slot 0..7 or the neutral owner");
    }
    let content = c.content.as_ref().ok_or("Entity.add requires content")?;
    let def = content.unit(odf).ok_or("Entity.add requires a unit definition")?;
    // Neutral is the owner of what no team plays: resources and the map's
    // fixtures (props: gates, wormholes, background objects), as the core
    // classifies them (explicit resource metadata counts as a resource). Craft
    // and stations need a player: a neutral one would have no one to order it.
    // (Armada I missions build team-0 gates, LUA-17; read from data.)
    if owner == NEUTRAL
        && matches!(EntityKind::of_def(def), EntityKind::Craft | EntityKind::Station | EntityKind::Projectile)
        && def.resource.is_none()
    {
        return Err("Entity.add neutral ownership requires a resource or a prop definition");
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
    // MMM scripts replace a unit with `Entity.remove(u) ; Entity.add(u.odf, owner, u.position)` and
    // go on to find it by its map label. The label belongs to the place the unit held, so the
    // replacement takes it (rule inferred from a shipped Roots mission, not measured on the original).
    let near = |a: Vec3, b: Vec3| {
        let (dx, dz) = (f64::from(a.x.to_f32()) - f64::from(b.x.to_f32()), f64::from(a.z.to_f32()) - f64::from(b.z.to_f32()));
        dx * dx + dz * dz < REPLACE_RADIUS_SQ
    };
    let inherit_label = c
        .spawns
        .removed
        .iter()
        .position(|(_, def, who, at, _)| *def == odf && *who == owner && near(*at, pos))
        .map(|i| {
            let (old, _, _, _, label) = c.spawns.removed.remove(i);
            (old, label)
        });
    let handle = Rc::new(SpawnHandle {
        request,
        seq,
        hint,
        state: Cell::new(SpawnStatus::Queued),
        inherit_label,
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
    c.spawns.removed.clear();
    c.spawns.killed.clear();
    let tick = c.view.tick;
    c.spawns.overlay.retain(|(key, _), (set, _)| match key {
        OverlayKey::Id(_) => *set >= tick,
        OverlayKey::Request(_) => true,
    });
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
                if let Some((old, label)) = &handle.inherit_label {
                    c.labels.remove(old);
                    c.labels.insert(id, label.clone());
                }
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
        c.spawns.overlay.retain(|(k, _), _| *k != OverlayKey::Request(handle.request));
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
        EntityAction::Remove => {
            let row = c.alive(id).and_then(|u| c.labels.get(&id).map(|l| (id, u.def, u.owner, u.pos, l.clone())));
            let queued = edit(c, Edit::Kill(id));
            if queued {
                c.spawns.killed.insert(id);
                c.spawns.removed.extend(row);
            }
            queued
        }
        EntityAction::Immortal(on) => edit(c, Edit::Immortal(id, on)),
        EntityAction::ForceOntoMap(enabled) => {
            if c.alive(id).is_none() { return false; }
            match super::edit_feedback::queue(c, "Entity.forceOntoMap", Edit::ForceOntoMap { id, enabled }) {
                Ok(()) => true,
                Err(_) => {
                    c.unsupported("Entity.forceOntoMap: command could not be queued");
                    false
                }
            }
        }
        EntityAction::PerceivedTeam(to) => {
            if c.alive(id).is_none() { return false; }
            match super::edit_feedback::queue(c, "Entity.perceivedTeam", Edit::PerceivedTeam(id, to)) {
                Ok(()) => true,
                Err(_) => {
                    c.unsupported("Entity.perceivedTeam: command could not be queued");
                    false
                }
            }
        }
        EntityAction::Cloak(on) => c.order(id, |units| Order::Cloak { units, on }),
        EntityAction::Hull(to) => {
            if !to.is_finite() {
                c.unsupported("entity.hull: non-finite value");
                return false;
            }
            if c.alive(id).is_none() {
                return false;
            }
            // Absolute input is applied in command order. A cached snapshot
            // cannot supply a correct damage delta for multiple assignments.
            edit(c, Edit::Hull(id, Fixed::from_f64(to)))
        }
        EntityAction::Attr(attr) => {
            if c.alive(id).is_none() {
                c.unsupported(&format!("{}: entity not alive", attr.label()));
                return false;
            }
            let queued = super::edit_feedback::queue(c, attr.label(), attr.edit(id)).is_ok();
            if !queued {
                c.unsupported(&format!("{}: command could not be queued", attr.label()));
            } else {
                if let Some(value) = attr.overlay() {
                    overlay_set(c, &EntityRef::Existing(id), attr.label(), value);
                }
            }
            queued
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
