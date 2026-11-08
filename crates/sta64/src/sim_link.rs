//! The client's link to the simulation (docs/WORKPLAN.md S8).
//!
//! - [`SimLinkPlugin`] decides from [`AnimClock`] how many ticks run this
//!   frame, steps the [`WorldSource`], shifts `prev`/`curr`, and sends a
//!   [`SimMsg`] per event. Nothing else calls `step`.
//! - Interpolation: a thing is drawn at `lerp(prev, curr, alpha)`; heading
//!   lerps then renormalises. An id only in `curr` appears at `curr`; an id
//!   only in `prev` is gone (its `Destroyed` event drives the effect).
//! - Render-side entities link to the sim only by [`SimEntity`]; nothing
//!   keys on Bevy `Entity` across frames.
//! - [`PlaySound`] is how UI, render and input ask for audio without
//!   depending on the audio module.
//!
//! Positions here are the sim's map space (left-handed); the renderer
//! mirrors Z when it places things.

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy::platform::cell::SyncCell;
use bevy::prelude::*;
use bevy::time::Real;
use sta_content::ContentDb;
use sta_sim::command::{Command, CommandKind, CommandRouter, Order};
use sta_sim::{EntityId, PlayerId, SimEvent, Vec3 as SimVec3, WorldSource, WorldView};

use crate::app::GameSet;
use crate::clock::AnimClock;

/// The running world source, its router and the last two views.
#[derive(Resource)]
pub struct SimLink {
    source: SyncCell<Box<dyn WorldSource>>,
    router: SyncCell<Box<dyn CommandRouter>>,
    prev: Arc<WorldView>,
    curr: Arc<WorldView>,
    /// Who this client's [`CommandQueue`] speaks for.
    player: PlayerId,
    /// Next sequence number for this client's commands.
    seq: u32,
}

impl SimLink {
    pub fn new(source: Box<dyn WorldSource>, router: Box<dyn CommandRouter>, player: PlayerId) -> SimLink {
        let view = source.view();
        SimLink { source: SyncCell::new(source), router: SyncCell::new(router), prev: view.clone(), curr: view, player, seq: 0 }
    }

    /// The view before the last tick.
    pub fn prev(&self) -> &Arc<WorldView> {
        &self.prev
    }

    /// The view after the last tick.
    pub fn curr(&self) -> &Arc<WorldView> {
        &self.curr
    }

    pub fn player(&self) -> PlayerId {
        self.player
    }

    /// Stamp a command as this client's next and hand it to the router.
    pub fn submit(&mut self, kind: CommandKind) {
        let cmd = Command { player: self.player, seq: self.seq, kind };
        self.seq += 1;
        self.router.get().submit(cmd);
    }

    /// Where to draw `id` at `alpha` between the last two ticks: map-space
    /// position and XZ heading, as floats for rendering. `None` once gone.
    pub fn interpolate(&self, id: EntityId, alpha: f32) -> Option<([f32; 3], [f32; 2])> {
        let now = self.curr.unit(id)?;
        let then = self.prev.unit(id).unwrap_or(now);
        let pos = Vec3::from(then.pos.to_f32()).lerp(Vec3::from(now.pos.to_f32()), alpha);
        let h = |u: &sta_sim::UnitView| Vec2::new(u.heading.0.to_f32(), u.heading.1.to_f32());
        let heading = h(then).lerp(h(now), alpha).normalize_or(h(now));
        Some((pos.to_array(), heading.to_array()))
    }

    /// Run one tick if the router has its commands; `None` on a stall.
    fn tick(&mut self) -> Option<Vec<SimEvent>> {
        let batch = self.router.get().take_ready(self.curr.tick + 1)?;
        let out = self.source.get().step(&batch);
        self.prev = std::mem::replace(&mut self.curr, out.view);
        Some(out.events)
    }
}

/// Links a render-side entity to the sim entity it shows.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimEntity(pub EntityId);

/// Render-side entity per sim entity, for whoever spawns them (RND-02).
#[derive(Resource, Debug, Default)]
pub struct SimEntities(pub BTreeMap<EntityId, Entity>);

/// A sim event, sent after the tick that produced it.
#[derive(Message, Debug, Clone)]
pub struct SimMsg(pub SimEvent);

/// What kind of order input just gave; for voice acks, which play on input,
/// not on the tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OrderKind {
    Move,
    Stop,
    Attack,
    Build,
    Formation,
    Special,
    Cloak,
    Mine,
}

impl OrderKind {
    /// `None` for orders added to the seam after this enum.
    pub fn of(order: &Order) -> Option<OrderKind> {
        Some(match order {
            Order::Move { .. } => OrderKind::Move,
            Order::Stop { .. } => OrderKind::Stop,
            Order::Attack { .. } => OrderKind::Attack,
            Order::Build { .. } => OrderKind::Build,
            Order::Formation { .. } => OrderKind::Formation,
            Order::Special { .. } => OrderKind::Special,
            Order::Cloak { .. } => OrderKind::Cloak,
            Order::Mine { .. } => OrderKind::Mine,
            _ => return None,
        })
    }
}

/// Input gave an order (sent by input when it queues the command).
#[derive(Message, Debug, Clone)]
pub struct OrderIssued {
    pub units: Vec<EntityId>,
    pub kind: OrderKind,
}

/// The selection changed (sent by input).
#[derive(Message, Debug, Clone, Default)]
pub struct SelectionChanged;

/// Commands from this client's input, waiting for the link to stamp them
/// with the player and a sequence number.
#[derive(Resource, Debug, Default)]
pub struct CommandQueue(pub Vec<CommandKind>);

/// The selected sim entities, in selection order.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct Selection(pub Vec<EntityId>);

/// The content database of the running game.
#[derive(Resource, Clone)]
pub struct ContentRes(pub Arc<ContentDb>);

/// Mixer bus a sound plays on (AUD-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Bus {
    Music,
    #[default]
    Sfx,
    Voice,
    Ui,
}

/// A request to play a sound by bare name, positioned in map space or not.
#[derive(Message, Debug, Clone)]
pub struct PlaySound {
    pub name: String,
    pub at: Option<SimVec3>,
    pub bus: Bus,
}

/// Steps the sim from the animation clock. Does nothing until a [`SimLink`]
/// resource exists.
pub struct SimLinkPlugin;

impl Plugin for SimLinkPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SimMsg>()
            .add_message::<OrderIssued>()
            .add_message::<SelectionChanged>()
            .add_message::<PlaySound>()
            .init_resource::<AnimClock>()
            .init_resource::<CommandQueue>()
            .init_resource::<Selection>()
            .init_resource::<SimEntities>()
            .add_systems(Update, step_sim.in_set(GameSet::SimLink).run_if(resource_exists::<SimLink>));
    }
}

fn step_sim(
    time: Res<Time<Real>>,
    mut clock: ResMut<AnimClock>,
    mut link: ResMut<SimLink>,
    mut queue: ResMut<CommandQueue>,
    mut out: MessageWriter<SimMsg>,
) {
    for kind in queue.0.drain(..) {
        link.submit(kind);
    }
    let due = clock.begin_frame(time.delta_secs_f64());
    for _ in 0..due {
        // A stall (lockstep waiting for a peer) holds anim time at alpha 1.
        let Some(events) = link.tick() else { break };
        clock.ticked();
        out.write_batch(events.into_iter().map(SimMsg));
    }
    clock.end_frame();
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use sta_sim::command::LocalRouter;
    use sta_sim::{Fixed, MoveStats, Sim};
    use std::time::Duration;

    #[derive(Resource, Default)]
    struct Seen(Vec<SimEvent>);

    fn collect(mut seen: ResMut<Seen>, mut msgs: MessageReader<SimMsg>) {
        seen.0.extend(msgs.read().map(|m| m.0.clone()));
    }

    #[test]
    fn link_steps_the_sim_from_the_clock() {
        let mut sim = Sim::new(1);
        let id = sim.spawn(0, SimVec3::ZERO, (Fixed::ZERO, Fixed::ONE), MoveStats::from_f32(140.0, 100.0, 1.0, 1.0, 1.0));
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0)))
            .configure_sets(Update, GameSet::SimLink)
            .add_plugins(SimLinkPlugin)
            .insert_resource(SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0)))
            .init_resource::<Seen>()
            .add_systems(Update, collect.after(GameSet::SimLink));
        let to = SimVec3::from_f32([0.0, 0.0, 1000.0]);
        app.world_mut().resource_mut::<CommandQueue>().0.push(CommandKind::Order(Order::Move { units: vec![id], to, queue: false }));
        for _ in 0..121 {
            app.update();
        }
        let link = app.world().resource::<SimLink>();
        let clock = app.world().resource::<AnimClock>();
        // Two seconds of 60 Hz frames: about 60 ticks (the first frame has no
        // time yet), the clock and the views agreeing.
        assert!((59..=60).contains(&link.curr().tick), "{}", link.curr().tick);
        assert_eq!(clock.tick(), link.curr().tick);
        assert_eq!(link.prev().tick + 1, link.curr().tick);
        let z = link.curr().unit(id).unwrap().pos.z.to_f32();
        assert!(z > 100.0, "the move order reached the sim: z = {z}");
        let (p, _) = link.interpolate(id, 0.5).unwrap();
        let (p0, p1) = (link.prev().unit(id).unwrap().pos.z.to_f32(), z);
        assert!((p[2] - (p0 + p1) / 2.0).abs() < 1e-3);
        assert_eq!(app.world().resource::<Seen>().0, [SimEvent::Spawned { id }]);
    }
}
