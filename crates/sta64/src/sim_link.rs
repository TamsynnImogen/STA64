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
use sta_sim::replay::{DEFAULT_DIGEST_EVERY, Replay};
use sta_sim::setup::Controller;
use sta_sim::{EntityId, PlayerId, SimEvent, Vec3 as SimVec3, WorldSource, WorldView};

use crate::app::GameSet;
use crate::clock::AnimClock;

#[cfg(test)]
#[path = "sim_link_production_tests.rs"]
mod production_tests;

/// The running world source, its router and the last two views.
#[derive(Resource)]
pub struct SimLink {
    source: SyncCell<Box<dyn WorldSource>>,
    router: SyncCell<Box<dyn CommandRouter>>,
    prev: Arc<WorldView>,
    curr: Arc<WorldView>,
    production: Vec<sta_sim::production::view::ProductionView>,
    cargo: Vec<sta_sim::economy::view::CargoView>,
    /// Resource-node amounts at the same tick as `curr` (UI-48).
    nodes: Vec<sta_sim::economy::view::NodeView>,
    systems: Vec<sta_sim::combat::damage::SystemView>,
    /// Who this client's [`CommandQueue`] speaks for.
    player: PlayerId,
    /// Next sequence number for this client's commands.
    seq: u32,
    /// Computer players of this game (`sta_ai`), thinking after every tick.
    controllers: SyncCell<Vec<Box<dyn Controller>>>,
    /// Commands the controllers have sent, for the log.
    controller_commands: u64,
    /// The game so far as a replay, when something asked for one (the playtest
    /// capture, TOOL-14): what each tick was given, never read by the sim.
    log: Option<Replay>,
    /// UI-46: the views the client draws and reads, when the player lifted the fog (see
    /// [`SimLink::set_fog_lifted`]); `None` otherwise, when they are `prev` and `curr`.
    lifted: Option<(Arc<WorldView>, Arc<WorldView>)>,
    fog_lifted: bool,
}

impl SimLink {
    pub fn new(source: Box<dyn WorldSource>, router: Box<dyn CommandRouter>, player: PlayerId) -> SimLink {
        let view = source.view();
        let production = source.production_view();
        let cargo = source.cargo_view();
        let nodes = source.node_view();
        let systems = source.system_view();
        SimLink { source: SyncCell::new(source), router: SyncCell::new(router), prev: view.clone(), curr: view, production, cargo, nodes, systems, player, seq: 0, controllers: SyncCell::new(Vec::new()), controller_commands: 0, log: None, lifted: None, fog_lifted: false }
    }

    /// Log every tick into `replay` (a [`Replay::new`] header of the game's
    /// setup; the link adds tick 0's digest and the rest as it steps).
    pub fn with_replay(mut self, mut replay: Replay) -> SimLink {
        replay.digests.push((self.curr.tick, self.curr.digest));
        replay.end_tick = self.curr.tick;
        self.log = Some(replay);
        self
    }

    /// The replay of the game so far, if the link is logging one.
    pub fn replay(&self) -> Option<&Replay> {
        self.log.as_ref()
    }

    /// The source's snapshot at the current tick, if it can make one.
    pub fn snapshot(&mut self) -> Option<Vec<u8>> {
        self.source.get().snapshot()
    }

    /// Seat computer players: each reads the new view and the tick's events
    /// after every tick and its commands go to the router like a player's
    /// (D3: controllers run on the host and are never rerun by replays).
    pub fn with_controllers(mut self, controllers: Vec<Box<dyn Controller>>) -> SimLink {
        self.controllers = SyncCell::new(controllers);
        self
    }

    /// How many controllers run.
    pub fn controllers(&mut self) -> usize {
        self.controllers.get().len()
    }

    /// The view before the last tick, as the client draws it (every unit shown while the fog is lifted).
    pub fn prev(&self) -> &Arc<WorldView> {
        self.lifted.as_ref().map_or(&self.prev, |l| &l.0)
    }

    /// The view after the last tick, as the client draws it (every unit shown while the fog is lifted).
    pub fn curr(&self) -> &Arc<WorldView> {
        self.lifted.as_ref().map_or(&self.curr, |l| &l.1)
    }

    /// The sim's own view after the last tick, whatever the client shows: what captures, replays and
    /// anything that must not depend on a cheat read.
    pub fn real_curr(&self) -> &Arc<WorldView> {
        &self.curr
    }

    /// Queue/site metadata at the same simulation tick as `curr`.
    pub fn production(&self) -> &[sta_sim::production::view::ProductionView] {
        &self.production
    }

    /// Economy cargo metadata at the same simulation tick as `curr`.
    pub fn cargo(&self) -> &[sta_sim::economy::view::CargoView] {
        &self.cargo
    }

    /// Resource-node amounts at the same simulation tick as `curr` (UI-48).
    pub fn nodes(&self) -> &[sta_sim::economy::view::NodeView] {
        &self.nodes
    }

    /// Damaged or switched-off systems (UI-48 system icons), refreshed each tick.
    pub fn systems(&self) -> &[sta_sim::combat::damage::SystemView] {
        &self.systems
    }

    /// UI-46 (presentation only): show every unit to the local player while `lifted`. The shown views
    /// are copies of the sim's with every player's visibility list made complete; the sim's views, the
    /// controllers, the replay log, the snapshot and the digest are never given them.
    pub fn set_fog_lifted(&mut self, lifted: bool) {
        self.fog_lifted = lifted;
        self.refresh_lifted();
    }

    pub fn fog_lifted(&self) -> bool {
        self.fog_lifted
    }

    fn refresh_lifted(&mut self) {
        self.lifted = self.fog_lifted.then(|| (all_seen(&self.prev), all_seen(&self.curr)));
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

    /// UI-47: already-authorized local sandbox input. Only the single-player cheat dispatcher
    /// calls this; a normal player's `submit` keeps that player's identity and cannot edit.
    /// BENCH has its own reserved identity, separate from the mission's SCRIPT sequence.
    /// The router and replay log see these requests exactly like any existing bench command.
    pub(crate) fn submit_sandbox(&mut self, kind: CommandKind) {
        let cmd = Command { player: PlayerId::BENCH, seq: self.seq, kind };
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

    /// The bench's frame path steps its own sim: it hands each new view in here,
    /// so effects and units read it as they read a link's. A caller that builds the
    /// new view from the old one starts from [`SimLink::real_curr`], never `curr()`
    /// (which shows every unit while the fog is lifted, UI-46).
    pub fn feed(&mut self, view: Arc<WorldView>) {
        self.feed_with_production(view, Vec::new());
    }

    /// A bench owning the source can hand in both immutable views together.
    pub fn feed_with_production(&mut self, view: Arc<WorldView>, production: Vec<sta_sim::production::view::ProductionView>) {
        self.feed_with_production_and_cargo(view, production, Vec::new());
    }

    /// A bench owning the source can supply its actual production and economy
    /// metadata alongside the view from the same tick.
    pub fn feed_with_production_and_cargo(&mut self, view: Arc<WorldView>, production: Vec<sta_sim::production::view::ProductionView>, cargo: Vec<sta_sim::economy::view::CargoView>) {
        self.prev = std::mem::replace(&mut self.curr, view);
        self.production = production;
        self.cargo = cargo;
        self.refresh_lifted();
    }

    /// Run one tick if the router has its commands; `None` on a stall.
    pub(crate) fn tick(&mut self) -> Option<Vec<SimEvent>> {
        let batch = self.router.get().take_ready(self.curr.tick + 1)?;
        let before = self.curr.tick;
        let out = self.source.get().step(&batch);
        self.production = self.source.get().production_view();
        self.cargo = self.source.get().cargo_view();
        self.nodes = self.source.get().node_view();
        self.systems = self.source.get().system_view();
        if let Some(log) = &mut self.log {
            log.log_step(before, &batch, out.view.tick, out.view.digest, DEFAULT_DIGEST_EVERY);
        }
        // While lifted, the new shown `prev` is the old shown `curr`: no second copy.
        let old_shown = self.lifted.take().map(|l| l.1);
        self.prev = std::mem::replace(&mut self.curr, out.view);
        self.lifted = self.fog_lifted.then(|| (old_shown.unwrap_or_else(|| all_seen(&self.prev)), all_seen(&self.curr)));
        let mut cmds = Vec::new();
        for c in self.controllers.get().iter_mut() {
            c.think(&self.curr, &out.events, &mut cmds);
        }
        for cmd in cmds {
            self.controller_commands += 1;
            if self.controller_commands == 1 || self.controller_commands.is_multiple_of(50) {
                info!("ai: command {} at tick {}: seat {} {:?}", self.controller_commands, self.curr.tick, cmd.player.0, cmd.kind);
            }
            self.router.get().submit(cmd);
        }
        Some(out.events)
    }
}

/// A copy of `view` in which every player sees every unit. A view without visibility data already
/// shows everything and is returned as it is.
pub fn all_seen(view: &Arc<WorldView>) -> Arc<WorldView> {
    if view.visibility.is_empty() {
        return view.clone();
    }
    let ids: Vec<EntityId> = view.units.iter().map(|u| u.id).collect();
    let mut v = (**view).clone();
    for (_, seen) in &mut v.visibility {
        seen.clone_from(&ids);
    }
    Arc::new(v)
}

/// The ids of units a mission script removed with a removal call (`Entity.remove`) whose `Destroyed`
/// the client has not met yet. Rule: a scripted removal shows no death effect (inferred from the stock
/// openings showing none; not measured). Fed by the mission's controller after each think, read by
/// [`step_sim`], which drops the matching `Destroyed` events. Calls that explode a unit on purpose,
/// damage and hull edits are not listed. Armada I and II removal calls map to the same sim edit but
/// are left out until their documentation says the original removes silently.
#[derive(Resource, Clone, Default)]
pub struct ScriptRemovals(std::sync::Arc<std::sync::Mutex<std::collections::BTreeSet<EntityId>>>);

impl ScriptRemovals {
    /// Ids the script has just removed.
    pub fn add(&self, ids: impl IntoIterator<Item = EntityId>) {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).extend(ids);
    }

    /// Whether `id` was a scripted removal; the entry is spent (ids are not reused within a sim).
    pub fn take(&self, id: EntityId) -> bool {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&id)
    }
}

/// A mission or match starting or ending: the list belonged to the last sim, whose ids the next one reuses.
fn forget_removals(mut commands: Commands) {
    commands.remove_resource::<ScriptRemovals>();
}

/// Marks a link the bench feeds itself: `step_sim` leaves it alone.
#[derive(Resource)]
pub struct BenchDriven;

/// A launched mission's sim waits for the game to be shown: its tick 0 and the script's opening
/// ticks ([`crate::clock::OPENING_TICKS`]) are the first in-game frame, whatever the load took. Inserted when the mission is
/// started, removed on entering `InGame` (or back at the front end). Menu scene, skirmish and
/// multiplayer links never get it.
#[derive(Resource)]
pub struct SimHeld;

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

/// A mission script's sound (`AudioMessage`, `PlayMusic`): its own stream, mixed with whatever else
/// plays and stopped by its `id` alone ([`StopScriptSound`]), on the UI bus. The ids come from one
/// counter for voice lines and music tracks.
#[derive(Message, Debug, Clone)]
pub struct PlayScriptSound {
    pub id: u32,
    pub name: String,
}

/// Stops the script stream `id` and nothing else (`StopAudioMessage`); a finished or unknown id is
/// ignored.
#[derive(Message, Debug, Clone, Copy)]
pub struct StopScriptSound {
    pub id: u32,
}

/// A request to cut what plays on one bus (a mission's `Media:stopSounds()`).
#[derive(Message, Debug, Clone, Copy)]
pub struct StopSounds {
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
            .add_message::<StopSounds>()
            .add_message::<PlayScriptSound>()
            .add_message::<StopScriptSound>()
            .init_resource::<AnimClock>()
            .init_resource::<CommandQueue>()
            .init_resource::<Selection>()
            .init_resource::<SimEntities>()
            .add_systems(OnEnter(crate::app::AppState::Loading), forget_removals)
            .add_systems(OnExit(crate::app::AppState::InGame), forget_removals)
            .add_systems(Update, step_sim.in_set(GameSet::SimLink).run_if(resource_exists::<SimLink>).run_if(not(resource_exists::<BenchDriven>)).run_if(not(resource_exists::<SimHeld>)));
    }
}

fn step_sim(
    time: Res<Time<Real>>,
    mut clock: ResMut<AnimClock>,
    mut link: ResMut<SimLink>,
    mut queue: ResMut<CommandQueue>,
    mut out: MessageWriter<SimMsg>,
    mut selection: ResMut<Selection>,
    mut selection_changed: MessageWriter<SelectionChanged>,
    removals: Option<Res<ScriptRemovals>>,
) {
    for kind in queue.0.drain(..) {
        link.submit(kind);
    }
    let due = clock.begin_frame(time.delta_secs_f64());
    for _ in 0..due {
        // A stall (lockstep waiting for a peer) holds anim time at alpha 1.
        let Some(mut events) = link.tick() else { break };
        clock.ticked();
        // A unit the script removed with a removal call raises no death effect, sound, notice or
        // music weight, at any tick of the mission (the script itself still sees the event). A
        // destruction the script causes on purpose, or any other kill, is delivered as it is.
        if let Some(r) = &removals {
            events.retain(|e| !matches!(e, SimEvent::Destroyed { id, .. } if r.take(*id)));
        }
        if remap_replaced_selection(&mut selection, &events, link.real_curr(), link.player()) {
            selection_changed.write(SelectionChanged);
        }
        out.write_batch(events.into_iter().map(SimMsg));
    }
    clock.end_frame();
}

/// Follow replacement events in their emitted order, then validate the final
/// identity. An intermediate replacement can already be gone by this tick's view.
fn remap_replaced_selection(selection: &mut Selection, events: &[SimEvent], view: &WorldView, player: PlayerId) -> bool {
    let mut changed = false;
    for selected in &mut selection.0 {
        let original = *selected;
        let mut next = original;
        for event in events {
            if let SimEvent::Replaced { old, new, select: true } = event
                && *old == next
            {
                next = *new;
            }
        }
        if next != original && view.unit(next).is_some_and(|u| u.owner == player
            && !u.flags.contains(sta_sim::view::UnitFlags::DYING)
            && u.hull[0] > sta_sim::Fixed::ZERO)
        {
            *selected = next;
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_selection_follows_chain_without_selecting_unselected_units() {
        let mut sim = Sim::new(1);
        let spawn = |sim: &mut Sim, owner| sim.spawn(owner, SimVec3::ZERO,
            (Fixed::ZERO, Fixed::ONE), MoveStats::from_f32(140.0, 100.0, 1.0, 1.0, 1.0));
        let old = spawn(&mut sim, 0);
        let middle = spawn(&mut sim, 0);
        let newest = spawn(&mut sim, 0);
        let other_player = spawn(&mut sim, 1);
        let mut view = sim.view();
        for unit in &mut view.units {
            unit.hull = [Fixed::ONE; 2];
        }
        let mut selection = Selection(vec![old]);
        let chain = [
            SimEvent::Replaced { old, new: middle, select: true },
            SimEvent::Replaced { old: middle, new: newest, select: true },
        ];
        assert!(remap_replaced_selection(&mut selection, &chain, &view, PlayerId(0)));
        assert_eq!(selection.0, [newest]);
        assert!(!remap_replaced_selection(&mut selection, &chain, &view, PlayerId(0)));
        assert_eq!(selection.0, [newest]);
        let mut selection = Selection(vec![old]);
        assert!(!remap_replaced_selection(&mut selection,
            &[SimEvent::Replaced { old, new: other_player, select: true }], &view, PlayerId(0)));
        assert_eq!(selection.0, [old]);
        assert!(!remap_replaced_selection(&mut selection,
            &[SimEvent::Replaced { old, new: middle, select: false }], &view, PlayerId(0)));
        assert_eq!(selection.0, [old]);
    }
    use bevy::ecs::system::RunSystemOnce;
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

    /// A stand-in script: after the tick a unit is due, it kills it (`listed`: through a removal call).
    struct Killer {
        due: Vec<(u64, EntityId, bool)>,
        removals: ScriptRemovals,
        seq: u32,
    }

    impl Controller for Killer {
        fn think(&mut self, view: &WorldView, _: &[SimEvent], out: &mut Vec<sta_sim::command::Command>) {
            for &(tick, id, listed) in &self.due {
                if view.tick == tick {
                    if listed {
                        self.removals.add([id]);
                    }
                    self.seq += 1;
                    out.push(sta_sim::command::Command { player: PlayerId::SCRIPT, seq: self.seq, kind: CommandKind::Edit(sta_sim::command::Edit::Kill(id)) });
                }
            }
        }
    }

    #[derive(Resource, Default)]
    struct Died(Vec<(u64, EntityId)>);

    fn record_deaths(mut died: ResMut<Died>, mut msgs: MessageReader<SimMsg>, link: Res<SimLink>) {
        for m in msgs.read() {
            if let SimEvent::Destroyed { id, .. } = m.0 {
                died.0.push((link.curr().tick, id));
            }
        }
    }

    /// A sim with `n` units, the stand-in script as its controller and `removals` as the world's list.
    fn killer_app(n: usize, due: impl Fn(&[EntityId]) -> Vec<(u64, EntityId, bool)>, opening: bool) -> (App, Vec<EntityId>, ScriptRemovals) {
        let mut sim = Sim::new(1);
        let ids: Vec<EntityId> = (0..n).map(|i| sim.spawn(0, SimVec3::from_f32([i as f32 * 100.0, 0.0, 0.0]), (Fixed::ZERO, Fixed::ONE), MoveStats::from_f32(140.0, 100.0, 1.0, 1.0, 1.0))).collect();
        let removals = ScriptRemovals::default();
        let killer = Killer { due: due(&ids), removals: removals.clone(), seq: 0 };
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0)))
            .configure_sets(Update, GameSet::SimLink)
            .add_plugins(SimLinkPlugin)
            .insert_resource(SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0)).with_controllers(vec![Box::new(killer)]))
            .insert_resource(removals.clone())
            .init_resource::<Died>()
            .add_systems(Update, record_deaths.after(GameSet::SimLink));
        if opening {
            app.world_mut().resource_mut::<AnimClock>().start_fresh();
        }
        (app, ids, removals)
    }

    fn run(app: &mut App, ticks: u64) {
        for _ in 0..(ticks * 3) {
            app.update();
        }
    }

    /// The latency `clock::OPENING_TICKS` adds up: a controller thinks after a tick and its command
    /// applies on the next one. A change in how `SimLink::tick` steps and thinks fails here.
    #[test]
    fn a_controller_command_applies_on_the_next_tick() {
        let (mut app, ids, _) = killer_app(1, |ids| vec![(1, ids[0], false)], false);
        run(&mut app, 6);
        assert_eq!(app.world().resource::<Died>().0, [(2, ids[0])], "thought after tick 1, applied on tick 2");
        assert_eq!(crate::clock::OPENING_TICKS, 4, "the number's parts are derived in `clock`; a changed latency means a new sum");
    }

    #[test]
    fn a_script_removed_unit_raises_no_death_event_at_tick_2_nor_at_tick_500() {
        let (mut app, ids, removals) = killer_app(2, |ids| vec![(1, ids[0], true), (499, ids[1], true)], false);
        run(&mut app, 520);
        assert!(app.world().resource::<SimLink>().curr().unit(ids[0]).is_none() && app.world().resource::<SimLink>().curr().unit(ids[1]).is_none(), "both are gone");
        assert!(app.world().resource::<Died>().0.is_empty(), "{:?}", app.world().resource::<Died>().0);
        assert!(!removals.take(ids[0]) && !removals.take(ids[1]), "the entries are spent");
    }

    #[test]
    fn a_unit_the_script_destroys_on_purpose_keeps_its_effect_also_in_the_opening_ticks() {
        let (mut app, ids, _) = killer_app(1, |ids| vec![(1, ids[0], false)], true);
        run(&mut app, 6);
        assert_eq!(app.world().resource::<Died>().0, [(4, ids[0])], "delivered in the first frame, which ran the opening ticks");
    }

    #[test]
    fn a_new_sim_does_not_inherit_the_last_ones_removals() {
        let (mut app, ids, removals) = killer_app(1, |ids| vec![(3, ids[0], false)], false);
        removals.add([ids[0]]); // left over from a mission that ended
        app.world_mut().run_system_once(forget_removals).unwrap();
        app.update();
        run(&mut app, 8);
        assert_eq!(app.world().resource::<Died>().0, [(4, ids[0])], "the same id in a new sim shows its death");
    }

    #[test]
    fn an_ordinary_kill_keeps_its_effect_late_in_a_mission() {
        let (mut app, ids, _) = killer_app(2, |ids| vec![(1, ids[0], true), (499, ids[1], false)], false);
        run(&mut app, 520);
        assert_eq!(app.world().resource::<Died>().0, [(500, ids[1])], "only the removed one is silent");
    }
}
