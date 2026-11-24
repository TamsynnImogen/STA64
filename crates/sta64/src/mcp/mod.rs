//! Opt-in local MCP control. The transport runs off-thread. Only the game
//! schedule can grant a lease, inspect a seat or enqueue its orders.

mod bridge;
mod game;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::prelude::*;
use serde_json::{Value, json};
use sta_sim::command::{Command, CommandKind, Order};
use sta_sim::{PlayerId, SimEvent, Tick};

pub(crate) use self::bridge::Bridge;
use self::bridge::Request;
use crate::app::{GameSet, LaunchConfig, Mode};
use crate::sim_link::{ContentRes, SimLink};
use crate::ui::notices::{Notice, Severity};

const LEASE_TIME: Duration = Duration::from_secs(90);
const QUEUE_LIMIT: usize = 32;

#[derive(Resource)]
pub(crate) struct AuthorizedSeat(pub(crate) PlayerId);

#[derive(Resource)]
pub(crate) struct MenuEndpoint(PathBuf);

struct Lease {
    id: String,
    expires: Instant,
}

/// Owned by an eligible live SimLink, so a new map never inherits a lease.
pub(crate) struct SeatControl {
    pub(crate) player: PlayerId,
    bounds: (sta_sim::Vec3, sta_sim::Vec3),
    lease: Option<Lease>,
    next_seq: u32,
    pending: Vec<Command>,
    results: VecDeque<Value>,
}

impl SeatControl {
    pub(crate) fn new(player: PlayerId, bounds: (sta_sim::Vec3, sta_sim::Vec3)) -> Self {
        Self {
            player,
            bounds,
            lease: None,
            next_seq: 0,
            pending: Vec::new(),
            results: VecDeque::new(),
        }
    }

    pub(crate) fn claimed(&self) -> bool {
        self.lease.is_some()
    }

    fn claim(&mut self, now: Instant) -> Result<String, String> {
        if self.claimed() {
            return Err("This seat is already controlled by a connected MCP client.".into());
        }
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(|_| "Could not generate a seat lease.")?;
        let id = format!("{:032x}", u128::from_le_bytes(bytes));
        self.lease = Some(Lease {
            id: id.clone(),
            expires: now + LEASE_TIME,
        });
        self.results.clear();
        Ok(id)
    }

    fn authorize(&mut self, id: Option<&str>, now: Instant) -> Result<(), String> {
        let Some(lease) = &mut self.lease else {
            return Err("No active seat lease. Claim the seat first.".into());
        };
        if id != Some(lease.id.as_str()) || lease.expires <= now {
            return Err("This seat lease is no longer valid. Claim the seat again.".into());
        }
        lease.expires = now + LEASE_TIME;
        Ok(())
    }

    fn release(&mut self) -> bool {
        let was_claimed = self.lease.take().is_some();
        self.pending.clear();
        self.results.clear();
        was_claimed
    }

    pub(crate) fn expire(&mut self, now: Instant) -> bool {
        if self
            .lease
            .as_ref()
            .is_some_and(|lease| lease.expires <= now)
        {
            self.release()
        } else {
            false
        }
    }

    fn enqueue(&mut self, order: Order) -> Result<u32, String> {
        if self.pending.len() >= QUEUE_LIMIT {
            return Err(
                "The seat's order queue is full. Let the match advance before retrying.".into(),
            );
        }
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.pending.push(Command {
            player: self.player,
            seq,
            kind: CommandKind::Order(order),
        });
        Ok(seq)
    }

    /// Drop commands which were already in the local router at handoff and
    /// apply the MCP player's orders in the ordinary tick/replay batch.
    pub(crate) fn merge_orders(&mut self, batch: &mut Vec<Command>) -> Vec<u32> {
        if !self.claimed() {
            return Vec::new();
        }
        batch.retain(|command| command.player != self.player);
        let submitted = self.pending.iter().map(|command| command.seq).collect();
        batch.append(&mut self.pending);
        batch.sort_by_key(Command::sort_key);
        submitted
    }

    pub(crate) fn record_results(&mut self, tick: Tick, submitted: Vec<u32>, events: &[SimEvent]) {
        for seq in submitted {
            let rejected = events.iter().any(|event| matches!(event,
                SimEvent::Rejected { player, seq: rejected } if *player == self.player && *rejected == seq));
            if self.results.len() == QUEUE_LIMIT {
                self.results.pop_front();
            }
            self.results
                .push_back(json!({ "sequence": seq, "tick": tick, "accepted": !rejected }));
        }
    }
}

pub(crate) fn enable(app: &mut App, cfg: &LaunchConfig) {
    if !matches!(cfg.mode(), Mode::FrontEnd | Mode::Game) {
        if cfg.mcp_seat.is_some() || cfg.mcp_endpoint.is_some() {
            warn!("MCP requires a local Instant Action or skirmish match");
        }
        return;
    }
    app.add_systems(Update, serve.in_set(GameSet::Bench));
    if cfg.mode() == Mode::FrontEnd
        && let Some(path) = endpoint_path(cfg, "instant-action.json")
    {
        app.insert_resource(MenuEndpoint(path));
    }
    let Some(seat) = cfg.mcp_seat else {
        return;
    };
    let path = endpoint_path(cfg, &format!("seat-{seat}.json"));
    match open_bridge(path.as_deref(), PlayerId(seat)) {
        Ok(bridge) => {
            app.insert_resource(AuthorizedSeat(PlayerId(seat)))
                .insert_resource(bridge);
        }
        Err(error) => warn!("Could not start the local MCP bridge: {error}"),
    }
}

fn endpoint_path(cfg: &LaunchConfig, default_name: &str) -> Option<PathBuf> {
    cfg.mcp_endpoint
        .clone()
        .or_else(|| crate::platform::data_dir().map(|dir| dir.join("mcp").join(default_name)))
}

pub(crate) fn open_menu_bridge(
    endpoint: Option<&MenuEndpoint>,
    seat: PlayerId,
) -> Result<Bridge, String> {
    open_bridge(endpoint.map(|endpoint| endpoint.0.as_path()), seat)
}

fn open_bridge(path: Option<&std::path::Path>, seat: PlayerId) -> Result<Bridge, String> {
    let path = path.ok_or(
        "No STA64 data directory. Use --mcp-endpoint to choose a private descriptor path.",
    )?;
    let bridge = Bridge::start(path.to_owned())?;
    info!(
        "MCP available for seat {}. Descriptor: {}",
        seat.0,
        path.display()
    );
    Ok(bridge)
}

fn serve(
    bridge: Option<Res<Bridge>>,
    cli: Option<Res<AuthorizedSeat>>,
    mut link: Option<ResMut<SimLink>>,
    content: Option<Res<ContentRes>>,
    editing: Option<Res<crate::editor::Editing>>,
    mut notices: Option<ResMut<Messages<Notice>>>,
) {
    let Some(bridge) = bridge else { return };
    let blocked = editing.is_some();
    let now = Instant::now();
    if let Some(control) = link.as_deref_mut().and_then(|link| link.mcp.as_mut()) {
        let released = if blocked {
            control.release()
        } else {
            control.expire(now)
        };
        if released {
            notice(
                &mut notices,
                format!("MCP released seat {}", control.player.0),
            );
        }
    }
    for _ in 0..8 {
        let Some(pending) = bridge.next() else { break };
        if pending.deadline <= Instant::now() {
            continue;
        }
        let link = link.as_deref_mut().filter(|_| !blocked);
        let configured = link
            .as_ref()
            .and_then(|link| link.mcp.as_ref())
            .map(|control| control.player)
            .or_else(|| cli.as_ref().map(|cli| cli.0));
        let result = if let Some(seat) = configured {
            let result = respond(
                &pending.request,
                seat,
                link,
                content.as_deref().map(|db| &db.0),
                now,
            );
            if result.is_ok() && matches!(pending.request.op.as_str(), "claim" | "release") {
                let verb = if pending.request.op == "claim" {
                    "took control of"
                } else {
                    "released"
                };
                notice(&mut notices, format!("MCP {verb} seat {}", seat.0));
            }
            result
        } else if pending.request.op == "status" {
            Ok(
                json!({ "seat": null, "tick": null, "available": false, "controlled": false,
                "message": "Start an Instant Action match with an MCP slot." }),
            )
        } else {
            Err("Start an eligible local match with the authorized seat present.".into())
        };
        let _ = pending.reply.try_send(result);
    }
}

fn notice(notices: &mut Option<ResMut<Messages<Notice>>>, message: String) {
    info!("{message}");
    if let Some(notices) = notices {
        notices.write(Notice::new(Severity::Info, message));
    }
}

fn respond(
    request: &Request,
    configured: PlayerId,
    link: Option<&mut SimLink>,
    content: Option<&Arc<sta_content::ContentDb>>,
    now: Instant,
) -> Result<Value, String> {
    let link = link.filter(|link| {
        link.mcp
            .as_ref()
            .is_some_and(|control| control.player == configured)
    });
    if request.op == "status" {
        let available = link
            .as_ref()
            .and_then(|link| link.real_curr().player(configured))
            .is_some_and(|player| !player.eliminated);
        return Ok(json!({
            "seat": configured.0,
            "available": available,
            "controlled": link.as_ref().and_then(|link| link.mcp.as_ref()).is_some_and(SeatControl::claimed),
            "tick": link.as_ref().map(|link| link.real_curr().tick),
            "message": if available { "Authorized local seat is available." } else { "No eligible active local seat. Start an Instant Action or skirmish match with this seat present." },
        }));
    }
    let link = link.ok_or("Start an eligible local match with the authorized seat present.")?;
    let live = link
        .real_curr()
        .player(configured)
        .is_some_and(|player| !player.eliminated);
    let control = link
        .mcp
        .as_mut()
        .ok_or("This match does not have an authorized MCP seat.")?;
    if request.op == "release" {
        control.authorize(request.lease.as_deref(), now)?;
        control.release();
        return Ok(json!({ "released": true, "seat": configured.0 }));
    }
    if !live {
        return Err("The authorized seat has been eliminated.".into());
    }
    if request.op == "claim" {
        let lease = control.claim(now)?;
        return Ok(json!({ "lease": lease, "seat": configured.0, "tick": link.real_curr().tick }));
    }
    control.authorize(request.lease.as_deref(), now)?;
    if request.op == "keepalive" {
        return Ok(json!({ "seat": configured.0 }));
    }
    let db = content.ok_or("Match content is still loading.")?;
    match request.op.as_str() {
        "observe" => {
            let mut view = game::observe(link, db, configured, &request.arguments)?;
            if let (Some(object), Some(control)) = (view.as_object_mut(), &link.mcp) {
                object.insert("recent_orders".into(), json!(control.results));
                object.insert(
                    "map_bounds".into(),
                    json!([control.bounds.0.to_f32(), control.bounds.1.to_f32()]),
                );
            }
            Ok(view)
        }
        "order" => {
            let order = game::order(link, db, configured, &request.arguments)?;
            let seq = link
                .mcp
                .as_mut()
                .ok_or("No active seat control.")?
                .enqueue(order)?;
            Ok(
                json!({ "queued": true, "sequence": seq, "earliest_tick": link.real_curr().tick + 1 }),
            )
        }
        _ => Err(format!("Unknown STA64 operation: {}", request.op)),
    }
}
