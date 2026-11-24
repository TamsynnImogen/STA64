//! Seat-scoped game observations and player-order decoding for MCP.
//! Keep this boundary on the simulation view: presentation fog lifting is not authority.

use std::sync::Arc;

use serde_json::{Value, json};
use sta_content::ContentDb;
use sta_sim::command::{Order, Target};
use sta_sim::{EntityId, Fixed, PlayerId, Vec3};

use crate::sim_link::SimLink;
use crate::ui::hud::model::{self, HudFeed, Look};

const MAX_UNITS: usize = 64;
const MAX_PAGE: usize = 100;
// A defensive input ceiling, not a map boundary. The sim checks actual placement.
const MAX_COORD: f64 = 100_000.0;

fn seat_view(link: &SimLink, seat: PlayerId) -> Result<&sta_sim::WorldView, String> {
    let view = link.real_curr();
    if !seat.is_player() || view.player(seat).is_none() {
        return Err("seat is not an active player in this game".into());
    }
    // An empty visibility table means an omniscient legacy source. Never expose
    // it to a seat-scoped client.
    if view.visibility.is_empty() || !view.visibility.iter().any(|(id, _)| *id == seat) {
        return Err("seat visibility is unavailable".into());
    }
    Ok(view)
}

fn visible(view: &sta_sim::WorldView, seat: PlayerId, id: EntityId) -> bool {
    view.visibility
        .iter()
        .find(|(player, _)| *player == seat)
        .is_some_and(|(_, ids)| ids.binary_search(&id).is_ok())
}

fn entity(args: &Value, key: &str) -> Result<EntityId, String> {
    let id = args
        .get(key)
        .and_then(Value::as_u64)
        .filter(|id| *id > 0 && *id <= u32::MAX as u64)
        .ok_or_else(|| format!("{key} must be a nonzero u32 entity id"))?;
    Ok(EntityId(id as u32))
}

fn owned(view: &sta_sim::WorldView, seat: PlayerId, id: EntityId) -> Result<(), String> {
    if !visible(view, seat, id) {
        return Err(format!("unit {} is not visible", id.0));
    }
    let unit = view
        .unit(id)
        .ok_or_else(|| format!("unit {} does not exist", id.0))?;
    if unit.owner != seat {
        return Err(format!("unit {} is not owned by seat {}", id.0, seat.0));
    }
    Ok(())
}

fn target(view: &sta_sim::WorldView, seat: PlayerId, id: EntityId) -> Result<EntityId, String> {
    if !visible(view, seat, id) {
        return Err(format!("target {} is not visible", id.0));
    }
    Ok(id)
}

fn units(view: &sta_sim::WorldView, seat: PlayerId, args: &Value) -> Result<Vec<EntityId>, String> {
    let values = args
        .get("units")
        .and_then(Value::as_array)
        .ok_or("units must be an array of entity ids")?;
    if values.is_empty() || values.len() > MAX_UNITS {
        return Err(format!("units must contain 1 to {MAX_UNITS} ids"));
    }
    let mut ids = Vec::with_capacity(values.len());
    for value in values {
        let id = value
            .as_u64()
            .filter(|id| *id > 0 && *id <= u32::MAX as u64)
            .map(|id| EntityId(id as u32))
            .ok_or("units contains an invalid entity id")?;
        owned(view, seat, id)?;
        if ids.contains(&id) {
            return Err(format!("duplicate unit {}", id.0));
        }
        ids.push(id);
    }
    Ok(ids)
}

fn position(args: &Value) -> Result<Vec3, String> {
    let values = args
        .get("position")
        .and_then(Value::as_array)
        .filter(|v| v.len() == 3)
        .ok_or("position must be [x,y,z] in map space")?;
    let mut out = [Fixed::ZERO; 3];
    for (i, value) in values.iter().enumerate() {
        let n = value
            .as_f64()
            .filter(|n| n.is_finite() && n.abs() <= MAX_COORD)
            .ok_or_else(|| format!("position[{i}] must be finite and within +/-{MAX_COORD}"))?;
        out[i] = Fixed::from_f64(n);
    }
    Ok(Vec3 {
        x: out[0],
        y: out[1],
        z: out[2],
    })
}

fn optional_target(
    view: &sta_sim::WorldView,
    seat: PlayerId,
    args: &Value,
) -> Result<Target, String> {
    if args.get("target").is_some() && args.get("position").is_some() {
        return Err("provide target or position, not both".into());
    }
    if args.get("target").is_some() {
        return Ok(Target::Entity(target(view, seat, entity(args, "target")?)?));
    }
    if args.get("position").is_some() {
        return Ok(Target::Point(position(args)?));
    }
    Ok(Target::None)
}

/// Reuse the HUD's live build policy with a seat-scoped world. Its tech-state
/// calculation must not inspect hidden units in the original world view.
fn build_feed(
    link: &SimLink,
    db: &Arc<ContentDb>,
    seat: PlayerId,
    view: &sta_sim::WorldView,
) -> HudFeed {
    let mut scoped = view.clone();
    scoped.units.retain(|u| visible(view, seat, u.id));
    HudFeed {
        view: Arc::new(scoped),
        player: seat,
        selected: Vec::new(),
        content: Some(db.clone()),
        production: link
            .production()
            .iter()
            .filter(|p| {
                view.unit(p.builder)
                    .is_some_and(|u| u.owner == seat && visible(view, seat, u.id))
            })
            .cloned()
            .collect(),
        names: Default::default(),
    }
}

fn ready_builder(unit: &sta_sim::UnitView) -> bool {
    unit.hull[0] > Fixed::ZERO
        && !unit.flags.contains(sta_sim::view::UnitFlags::DYING)
        && unit.progress.is_none_or(|progress| progress >= Fixed::ONE)
}

/// Read only the selected seat's own economy and current sensor-visible units.
pub(crate) fn observe(
    link: &SimLink,
    db: &Arc<ContentDb>,
    seat: PlayerId,
    args: &Value,
) -> Result<Value, String> {
    let view = seat_view(link, seat)?;
    let after = args.get("after").map_or(Ok(None), |v| {
        v.as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .map(EntityId)
            .map(Some)
            .ok_or("after must be a nonnegative u32 entity id")
    })?;
    let limit = args.get("limit").map_or(Ok(50), |v| {
        v.as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=MAX_PAGE).contains(n))
            .ok_or(format!("limit must be 1 to {MAX_PAGE}"))
    })?;
    let me = view.player(seat).ok_or("seat is unavailable")?;
    let shown = view.visible_to(seat);
    let count = shown.len();
    let start = shown.partition_point(|u| after.is_some_and(|cursor| u.id <= cursor));
    let end = (start + limit).min(count);
    let page = &shown[start..end];
    let feed = page
        .iter()
        .any(|u| {
            u.owner == seat
                && ready_builder(u)
                && (db.unit(u.def).is_some_and(|def| !def.build.list.is_empty())
                    || link
                        .production()
                        .binary_search_by_key(&u.id, |row| row.builder)
                        .is_ok())
        })
        .then(|| build_feed(link, db, seat, view));
    let rows: Vec<_> = page.iter()
        .map(|u| {
            let owned = u.owner == seat;
            let offerings: Vec<_> = feed.as_ref().filter(|_| owned && ready_builder(u))
                .into_iter().flat_map(|feed| model::build_items(feed, u))
                .filter(|item| item.look == Look::Ready).map(|item| item.odf).collect();
            let specials: Vec<_> = db.unit(u.def).filter(|_| owned).into_iter()
                .flat_map(|def| sta_sim::specials::specials_of(db, def)).enumerate()
                .map(|(slot, (kind, weapon))| json!({"slot": slot, "kind": format!("{kind:?}"), "odf": weapon.odf}))
                .collect();
            let owner = if owned { seat } else { view.perceived_owner(u.id).unwrap_or(u.owner) };
            let jobs: Vec<_> = owned.then(|| link.production().binary_search_by_key(&u.id, |row| row.builder).ok())
                .flatten().into_iter().flat_map(|i| &link.production()[i].jobs)
                .map(|job| json!({"odf": db.name(job.def), "progress": job.progress.to_f32(), "started": job.started}))
                .collect();
            let cargo = owned.then(|| link.cargo().binary_search_by_key(&u.id, |row| row.entity).ok()).flatten()
                .map(|i| {
                    let row = &link.cargo()[i];
                    json!({"amount": row.amount.to_f32(), "capacity": row.capacity.to_f32(),
                        "resource": row.resource.map(|res| db.resources().name(res as usize))})
                });
            let node = link.nodes().binary_search_by_key(&u.id, |row| row.entity).ok().map(|i| {
                let row = &link.nodes()[i];
                json!({"amount": row.amount.to_f32(), "resource": db.resources().name(row.resource as usize),
                    "infinite": row.infinite})
            });
            json!({"id": u.id.0, "odf": db.name(u.def), "owner": owner.0, "owned": owned,
            "position": [u.pos.x.to_f32(), u.pos.y.to_f32(), u.pos.z.to_f32()],
            "speed": u.speed.to_f32(), "resource_node": node, "cargo": cargo,
            "hull": [u.hull[0].to_f32(), u.hull[1].to_f32()],
            "shields": u.shields.iter().map(|s| [s[0].to_f32(), s[1].to_f32()]).collect::<Vec<_>>(),
            "crew": u.crew.map(|s| [s[0].to_f32(), s[1].to_f32()]),
            "energy": u.energy.map(|s| [s[0].to_f32(), s[1].to_f32()]),
            "progress": u.progress.map(Fixed::to_f32), "flags": u.flags.0,
            "build_offerings": offerings, "production": jobs, "specials": specials})
        })
        .collect();
    Ok(json!({"tick": view.tick, "seat": seat.0, "team": me.team.0,
        "resources": me.resources.iter().enumerate().map(|(i, n)| json!({"name": db.resources().name(i), "amount": n.to_f32()})).collect::<Vec<_>>(),
        "officers": [me.officers[0].to_f32(), me.officers[1].to_f32()],
        "crew": me.crew.map(Fixed::to_f32), "eliminated": me.eliminated,
        "relations": me.relations.iter().map(|(other, relation)| json!({"seat": other.0, "relation": format!("{relation:?}").to_lowercase()})).collect::<Vec<_>>(),
        "units": rows, "total_visible": count,
        "next_after": page.last().filter(|_| end < count).map(|u| u.id.0)}))
}

/// Decode the documented action set into a regular player order. The sim
/// remains authoritative for capability, costs, targeting and map admission.
pub(crate) fn order(
    link: &SimLink,
    db: &Arc<ContentDb>,
    seat: PlayerId,
    args: &Value,
) -> Result<Order, String> {
    let view = seat_view(link, seat)?;
    if view.player(seat).is_some_and(|p| p.eliminated) {
        return Err("seat is eliminated".into());
    }
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .ok_or("action is required")?;
    let queue = args
        .get("queue")
        .map_or(Ok(false), |v| v.as_bool().ok_or("queue must be boolean"))?;
    match action {
        "build" => {
            let builder = entity(args, "builder")?;
            owned(view, seat, builder)?;
            let odf = args
                .get("odf")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= 128)
                .ok_or("odf must be a nonempty string of at most 128 bytes")?;
            let what = db
                .key(odf)
                .filter(|key| db.unit(*key).is_some())
                .ok_or("unknown unit ODF")?;
            let feed = build_feed(link, db, seat, view);
            let builder_view = view.unit(builder).ok_or("builder no longer exists")?;
            if !ready_builder(builder_view) {
                return Err(format!("builder {} is not ready", builder.0));
            }
            if !model::build_items(&feed, builder_view)
                .iter()
                .any(|item| item.key == what && item.look == Look::Ready)
            {
                return Err(format!(
                    "{odf} is not currently buildable by builder {}",
                    builder.0
                ));
            }
            let at = args.get("position").map(|_| position(args)).transpose()?;
            Ok(Order::Build { builder, what, at })
        }
        "move" | "move_warp" => {
            let ids = units(view, seat, args)?;
            let to = position(args)?;
            Ok(if action == "move" {
                Order::Move {
                    units: ids,
                    to,
                    queue,
                }
            } else {
                Order::MoveWarp {
                    units: ids,
                    to,
                    queue,
                }
            })
        }
        "attack" | "mine" => {
            let ids = units(view, seat, args)?;
            let target = target(view, seat, entity(args, "target")?)?;
            Ok(if action == "attack" {
                Order::Attack { units: ids, target }
            } else {
                Order::Mine {
                    units: ids,
                    node: target,
                }
            })
        }
        "stop" => Ok(Order::Stop {
            units: units(view, seat, args)?,
        }),
        "scout" => Ok(Order::Scout {
            units: units(view, seat, args)?,
        }),
        "search_and_destroy" => Ok(Order::SearchAndDestroy {
            units: units(view, seat, args)?,
        }),
        "rally" => {
            let builders = units(view, seat, args)?;
            Ok(Order::Rally {
                builders,
                at: optional_target(view, seat, args)?,
            })
        }
        "special" => {
            let ids = units(view, seat, args)?;
            let slot = args
                .get("slot")
                .and_then(Value::as_u64)
                .filter(|n| *n <= u8::MAX as u64)
                .ok_or("slot must be a u8")? as u8;
            Ok(Order::Special {
                units: ids,
                slot,
                target: optional_target(view, seat, args)?,
            })
        }
        "repair" => {
            let ids = units(view, seat, args)?;
            let yard = args
                .get("target")
                .map(|_| target(view, seat, entity(args, "target")?))
                .transpose()?;
            Ok(Order::Repair { units: ids, yard })
        }
        _ => Err(format!("unsupported action: {action}")),
    }
}
