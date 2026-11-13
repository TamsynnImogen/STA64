//! Model animation by event (RND-18).
//!
//! A SOD animation channel is a step flipbook of evenly spaced keys over a period,
//! and the file says nothing about when it should run: node names are the modeller's
//! (`Box01`, `Door03`) and the ODFs name no tracks. What the file does say is whether
//! the track is a *cycle*. A rotating dish, a pulsing light or a drifting field ends
//! one step short of where it began, so looping it is seamless; a bay door or a build
//! arm ends far from its first key (open, extended) and looping it snaps it shut and
//! open again nonstop. So the rule is data-driven and the same in Armada, Armada II,
//! Fleet Operations and every mod: a channel whose last key is far from its first
//! (against its own key-to-key step) is an *event* track; the rest are *ambient*.
//!
//! Ambient tracks keep looping in [`model_scene::animate`]. An event track rests on
//! its first key (shut) and plays once forward and back (open, then close) on the sim
//! clock when its model's unit does what the unit's own data says it can do (RND-21):
//!
//! - a unit that **builds** (`build.list`, the ODF's build items) plays on
//!   [`SimEvent::BuildStarted`] and [`SimEvent::Built`]: bay doors, build arms;
//! - a unit that is **armed** and builds nothing (`hardpoints`, the ODF's weapon
//!   slots) plays on [`SimEvent::Fired`] of its own: turrets, platform weapons, special
//!   weapons such as hyper artillery;
//! - a model **without a sim unit** (the model viewer, a map-view ship, an effect
//!   model) has nothing to wait for and plays on its own, then rests a period
//!   ([`Trigger::Idle`]; the rest is ours, nothing in the model says it);
//! - a unit that neither builds nor fires and takes deposits (class `mining`,
//!   `p_mining`, or `is_starbase`) plays while a freighter is docked at it
//!   ([`Trigger::Dock`], the view's `UnitFlags::DOCKED`): looped while docked, at rest
//!   otherwise;
//! - any other unit that neither builds nor fires has no event that says when
//!   ([`Trigger::None`]); the track rests.
//!
//! What the original engine does, per trigger (read from the executable, static; the
//! owning functions are `ConstructionRig::StartBuild` / `WorkerBeeHasReturned`,
//! `ResourceTransferInterface::SetDock`, `Shipyard::SetRepairDock` and
//! `Weapon::Simulate`): every call sets the owner's animation flags and restarts its
//! clock. Flag 1 loops, flag 2 plays forward once and holds the last key, flag 4 plays
//! backwards once and holds the first key. A rig sets 2 at `StartBuild` and 4 once the
//! last worker bee is back: **workers at work** is the construction hold already done
//! by `ConstructionBays` (open from the start of the build until every worker is
//! home). A freighter docking at a transfer station sets 1 on the station (loop) and
//! its leaving disables the station's animation (rest). A craft docking at a shipyard
//! sets 2 and its leaving 4 when the ODF's `repairAnimation` is not 0; the sim has no
//! repair dock yet, so that one has no signal. A weapon with `animateOwner = 1` sets 2
//! on its owner at every shot (what [`Trigger::Fire`] approximates by "armed", since
//! the weapon key is not in the content database yet). **Deploy** has no trigger of its
//! own in the engine: a unit that deploys either builds (the rig above) or is armed
//! (`bor_dodeca_relay*` plays on its own weapon); wormhole objects (`wtranwrp`) are
//! animated by the generator that opens and closes them, not by a unit's own state.
//!
//! The choice is read from the unit's content, never from a model or track name.
use crate::clock::AnimClock;
use crate::render::model_scene::{AnimKeys, NodeAnim};
use crate::sim_link::{SimEntities, SimMsg};
use bevy::prelude::*;
use sta_assets::model::Keys;
use sta_sim::SimEvent;

/// What starts an event track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Trigger {
    /// The unit builds: [`SimEvent::BuildStarted`] / [`SimEvent::Built`].
    #[default]
    Build,
    /// The unit is armed and builds nothing: its own [`SimEvent::Fired`].
    Fire,
    /// The unit takes deposits and builds nothing and is not armed: loops while a
    /// freighter is docked at it ([`sta_sim::view::UnitFlags::DOCKED`]).
    Dock,
    /// ODF `animation = 2`: loops all the time.
    Loop,
    /// ODF `animation = 1`: plays forward once when construction finishes and holds.
    Once,
    /// No sim unit: plays by itself, once each period of rest.
    Idle,
    /// A unit that neither builds nor fires: nothing in the sim says when.
    None,
}

/// The ODF `animation` key (`GameObjectClass`, read from the executable, static): 0 or
/// absent and 3 are off (the model rests on its first key; only events animate it), 1
/// plays every track forward once when the object is made and holds the last key, 2 loops
/// every track. Other values do nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnimMode {
    #[default]
    Off,
    Once,
    Loop,
}

impl AnimMode {
    pub fn of(def: &sta_content::UnitDef) -> AnimMode {
        match def.raw.i64("animation") {
            Some(1) => AnimMode::Once,
            Some(2) => AnimMode::Loop,
            _ => AnimMode::Off,
        }
    }
}

/// What a unit's content says it can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Caps {
    /// `build.list` is not empty.
    pub builds: bool,
    /// Has a weapon whose shots animate its owner (see [`owner_animates`]).
    pub armed: bool,
    /// Takes deposits from freighters: class `mining` / `p_mining`, or `is_starbase`.
    pub drop_off: bool,
    pub mode: AnimMode,
}

/// Whether a shot of this weapon animates its owner: the weapon's ODF `animateOwner` (read
/// from the executable, static: `Weapon::Simulate` plays the owner forward once per shot
/// when it is set). A weapon with no ODF text behind it (native content, test content) has
/// no such key to read, so its shots count.
pub fn owner_animates(w: &sta_content::WeaponDef) -> bool {
    w.raw.class_label().is_none() || w.raw.i64("animateOwner").is_some_and(|v| v != 0)
}

impl Caps {
    pub fn of(def: &sta_content::UnitDef, db: &sta_content::ContentDb) -> Caps {
        Caps {
            builds: !def.build.list.is_empty(),
            armed: def.hardpoints.iter().any(|h| db.weapon(h.weapon.key).is_some_and(owner_animates)),
            drop_off: def.is_starbase || matches!(def.class.0.as_str(), "mining" | "p_mining" | "starbase"),
            mode: AnimMode::of(def),
        }
    }
}

/// The trigger of a model's event tracks from what its unit can do; `None` for the
/// unit: no sim unit. A builder's tracks are its bay; then the unit's own `animation`
/// key (loop, or once at completion); then its owner-animating weapons; a drop-off that
/// is none of these plays while docked.
pub fn trigger_for(unit: Option<Caps>) -> Trigger {
    match unit {
        None => Trigger::Idle,
        Some(c) if c.builds => Trigger::Build,
        Some(c) if c.mode == AnimMode::Loop => Trigger::Loop,
        Some(c) if c.mode == AnimMode::Once => Trigger::Once,
        Some(c) if c.armed => Trigger::Fire,
        Some(c) if c.drop_off => Trigger::Dock,
        Some(_) => Trigger::None,
    }
}

/// How a track plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    /// Cycles seamlessly: loops all the time.
    Ambient,
    /// Ends away from where it starts: plays once, on an event.
    Event,
}

/// A track whose last key is further from its first than this share of the whole
/// path its keys travel is open-ended: it goes somewhere and stops. A cycle returns,
/// so its end-to-start distance is a few steps against a path many times longer.
const OPEN_ENDED: f32 = 0.5;

/// The length of the path a track's keys travel, and the distance from first to last.
fn path_span(keys: &Keys) -> (f32, f32) {
    fn go(n: usize, dist: impl Fn(usize, usize) -> f32) -> (f32, f32) {
        if n < 3 {
            return (0.0, 0.0);
        }
        ((1..n).map(|i| dist(i - 1, i)).sum(), dist(0, n - 1))
    }
    match keys {
        Keys::Matrix(k) => go(k.len(), |a, b| {
            k[a].m.iter().zip(&k[b].m).map(|(x, y)| (x - y).abs()).sum()
        }),
        Keys::Scale(k) => go(k.len(), |a, b| (k[a] - k[b]).abs()),
    }
}

/// Sorts a channel's keys. Fewer than three keys, or keys that never move, are
/// ambient (nothing to play).
pub fn classify(keys: &Keys) -> TrackKind {
    let (path, span) = path_span(keys);
    if path > 1e-4 && span > OPEN_ENDED * path {
        TrackKind::Event
    } else {
        TrackKind::Ambient
    }
}

#[cfg(test)]
fn key_len(k: &Keys) -> usize {
    match k {
        Keys::Matrix(k) => k.len(),
        Keys::Scale(k) => k.len(),
    }
}

#[cfg(test)]
fn ratio(keys: &Keys) -> f32 {
    let (path, span) = path_span(keys);
    if path > 0.0 { span / path } else { 0.0 }
}

/// An event track on a model: rests on its first key until its owner's event.
#[derive(Component)]
pub struct EventAnim {
    pub track: NodeAnim,
    /// The unit entity whose model this is (the one with `SimEntity`).
    pub root: Entity,
    /// Anim-clock time the current play began; `None` at rest.
    pub started: Option<f64>,
    /// Construction keeps the bay open until every worker has returned.
    held: bool,
    /// Fraction open when a held bay begins closing, including early cancellation.
    closing_from: Option<f32>,
    /// What starts it.
    pub trigger: Trigger,
    /// The trigger has been read from the unit's content.
    assigned: bool,
    /// Wall-clock time an idle track may play again.
    idle_from: f64,
}

impl EventAnim {
    pub fn new(track: NodeAnim, root: Entity) -> Self {
        EventAnim {
            track,
            root,
            started: None,
            held: false,
            closing_from: None,
            trigger: Trigger::Build,
            assigned: false,
            idle_from: 0.0,
        }
    }
}

fn key_pose(track: &NodeAnim, k: usize) -> Transform {
    match &track.keys {
        AnimKeys::Matrix(keys) => keys[k],
        AnimKeys::Scale(keys) => track.base.with_scale(Vec3::splat(keys[k])),
    }
}

fn key_count(track: &NodeAnim) -> usize {
    match &track.keys {
        AnimKeys::Matrix(k) => k.len(),
        AnimKeys::Scale(k) => k.len(),
    }
}

/// The pose `elapsed` seconds into a play (`None`: at rest). A play runs the track
/// forward over its period, then back over the same, and is over after both.
/// Returns the pose and whether the play is still going.
pub fn pose(track: &NodeAnim, elapsed: Option<f32>) -> (Transform, bool) {
    let n = key_count(track);
    let Some(t) = elapsed else {
        return (key_pose(track, 0), false);
    };
    if t >= 2.0 * track.period || t < 0.0 {
        return (key_pose(track, 0), false);
    }
    let x = if t < track.period {
        t
    } else {
        2.0 * track.period - t
    };
    let k = ((x / track.period * n as f32) as usize).min(n - 1);
    (key_pose(track, k), true)
}

/// The pose of a looped play `elapsed` seconds in (`None`: at rest on the first key):
/// forward over the period, then again, as the engine's flag 1 does.
pub fn loop_pose(track: &NodeAnim, elapsed: Option<f32>) -> Transform {
    let Some(t) = elapsed else {
        return key_pose(track, 0);
    };
    let n = key_count(track);
    let x = t.rem_euclid(track.period);
    key_pose(track, ((x / track.period * n as f32) as usize).min(n - 1))
}

/// The pose of a forward-once play `elapsed` seconds in: it holds the last key at the end
/// (the engine's flag 2); `None`: not started, the first key.
pub fn once_pose(track: &NodeAnim, elapsed: Option<f32>) -> Transform {
    let Some(t) = elapsed else {
        return key_pose(track, 0);
    };
    let n = key_count(track);
    key_pose(track, ((t.max(0.0) / track.period * n as f32) as usize).min(n - 1))
}

/// Whether the unit has finished construction (or never was under construction).
fn complete(link: &crate::sim_link::SimLink, id: sta_sim::EntityId) -> bool {
    link.curr().unit(id).is_some_and(|u| u.progress.is_none_or(|p| p >= sta_sim::Fixed::ONE))
}

/// The unit whose event tracks `event` plays, and the trigger kind it is for.
pub fn triggers(event: &SimEvent) -> Option<(sta_sim::EntityId, Trigger)> {
    match event {
        // Build arms, and the bay opening as the ship starts to come together.
        SimEvent::BuildStarted { builder, .. } => Some((*builder, Trigger::Build)),
        // The finished ship leaves: bay doors.
        SimEvent::Built { builder, .. } => Some((*builder, Trigger::Build)),
        // A weapon fires: turrets and the like.
        SimEvent::Fired { shooter, .. } => Some((*shooter, Trigger::Fire)),
        _ => None,
    }
}

/// Gives each new event track its trigger from its unit's content. A track whose root has
/// no sim entity is idle; one whose unit's definition is not known (no content loaded)
/// keeps the build trigger it was made with.
pub fn assign_triggers(
    link: Res<crate::sim_link::SimLink>,
    content: Option<Res<crate::sim_link::ContentRes>>,
    roots: Query<&crate::sim_link::SimEntity>,
    mut fresh: Query<&mut EventAnim>,
) {
    for mut a in &mut fresh {
        if a.assigned {
            continue;
        }
        let unit = roots.get(a.root).ok().map(|id| id.0);
        let caps = match (unit, content.as_deref()) {
            (None, _) => None,
            (Some(id), Some(c)) => {
                let def = link.curr().unit(id).and_then(|u| c.0.unit(u.def));
                match def {
                    Some(d) => Some(Caps::of(d, &c.0)),
                    None => continue,
                }
            }
            (Some(_), None) => continue,
        };
        a.trigger = trigger_for(caps);
        a.assigned = true;
    }
}

/// Tracks of a model with no sim unit (the model viewer has no sim at all) play by
/// themselves: once forward and back, then a period of rest.
pub fn play_idle(clock: Res<AnimClock>, mut q: Query<&mut EventAnim>, owners: Query<(), With<crate::sim_link::SimEntity>>) {
    let wall = clock.wall();
    for mut a in &mut q {
        // No `SimEntity` on the root: nothing in the sim to wait for, whatever the
        // trigger said before content loaded.
        if owners.get(a.root).is_err() && a.trigger != Trigger::Idle {
            a.trigger = Trigger::Idle;
            a.assigned = true;
        }
        if a.trigger == Trigger::Idle && a.started.is_none() && wall >= a.idle_from {
            a.started = Some(wall);
            a.idle_from = wall + f64::from(3.0 * a.track.period); // two periods playing, one resting
        }
    }
}

/// Starts the event tracks of every builder an event names.
pub fn start_events(
    mut msgs: MessageReader<SimMsg>,
    content: Option<Res<crate::sim_link::ContentRes>>,
    ents: Res<SimEntities>,
    clock: Res<AnimClock>,
    mut q: Query<&mut EventAnim>,
) {
    let mut roots = std::collections::HashSet::new();
    for SimMsg(ev) in msgs.read() {
        // A shot animates its owner only if the weapon's ODF says so.
        if let SimEvent::Fired { weapon, .. } = ev
            && let Some(w) = content.as_deref().and_then(|c| c.0.weapon(*weapon))
            && !owner_animates(w)
        {
            continue;
        }
        if let Some((id, kind)) = triggers(ev)
            && let Some(root) = ents.0.get(&id)
        {
            roots.insert((*root, kind));
        }
    }
    if roots.is_empty() {
        return;
    }
    let now = clock.now();
    for mut a in &mut q {
        // A play in progress is not restarted.
        if a.started.is_none() && roots.contains(&(a.root, a.trigger)) {
            a.started = Some(now);
        }
    }
}

/// Drives the state-based triggers from the view: a drop-off's tracks start when a
/// freighter docks at it (the view's [`sta_sim::view::UnitFlags::DOCKED`]) and end when the
/// last one leaves; a `Once` unit's start when it is complete. Both read the view, so a unit
/// that is already docked or complete when the client joins or loads plays from the first frame.
pub fn track_docks(
    link: Res<crate::sim_link::SimLink>,
    clock: Res<AnimClock>,
    roots: Query<&crate::sim_link::SimEntity>,
    mut q: Query<&mut EventAnim>,
) {
    let now = clock.now();
    for mut a in &mut q {
        if !matches!(a.trigger, Trigger::Dock | Trigger::Once) {
            continue;
        }
        let Some(id) = roots.get(a.root).ok().map(|id| id.0) else { continue };
        if a.trigger == Trigger::Once {
            if a.started.is_none() && complete(&link, id) {
                a.started = Some(now);
            }
            continue;
        }
        let docked = link
            .curr()
            .unit(id)
            .is_some_and(|u| u.flags.contains(sta_sim::view::UnitFlags::DOCKED));
        match (docked, a.started.is_some()) {
            (true, false) => a.started = Some(now),
            (false, true) => a.started = None,
            _ => {}
        }
    }
}

/// How an ambient (cycle) track of a unit with a sim entity is held back.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub enum AmbientGate {
    /// Rests until a freighter docks, then loops while one is docked.
    Docked(Option<f64>),
    /// Plays forward once from the moment the unit is complete, then holds the last key.
    Once(Option<f64>),
}

/// Marks an ambient track as looked at, so it is gated once.
#[derive(Component)]
pub struct GateChecked;

/// What the unit's `animation` key and class make of its ambient tracks (the engine animates every
/// track of the model alike; this is the part that differs from looping all the time):
/// a drop-off whose key is off rests until docked, any unit with key 1 plays once. Key 2
/// keeps looping; the same for units with key off that are not drop-offs (the engine rests
/// them too, not applied here).
pub fn ambient_gate(mode: AnimMode, drop_off: bool) -> Option<AmbientGate> {
    match (mode, drop_off) {
        (AnimMode::Once, _) => Some(AmbientGate::Once(None)),
        (AnimMode::Off, true) => Some(AmbientGate::Docked(None)),
        _ => None,
    }
}

/// Ambient tracks not yet looked at.
type UngatedAmbient = (With<NodeAnim>, Without<EventAnim>, Without<GateChecked>);

/// Gives each ambient track its gate once its unit's definition is known.
pub fn assign_gates(
    mut commands: Commands,
    link: Res<crate::sim_link::SimLink>,
    content: Option<Res<crate::sim_link::ContentRes>>,
    roots: Query<&crate::sim_link::SimEntity>,
    fresh: Query<(Entity, &crate::render::appearance::ModelNode), UngatedAmbient>,
) {
    for (e, node) in &fresh {
        let Ok(id) = roots.get(node.root).map(|id| id.0) else {
            // No sim unit (model viewer, map objects): loops, as before.
            commands.entity(e).insert(GateChecked);
            continue;
        };
        let Some(c) = content.as_deref() else { continue };
        let Some(def) = link.curr().unit(id).and_then(|u| c.0.unit(u.def)) else { continue };
        let caps = Caps::of(def, &c.0);
        let mut ec = commands.entity(e);
        ec.insert(GateChecked);
        if let Some(g) = ambient_gate(caps.mode, caps.drop_off) {
            ec.insert(g);
        }
    }
}

/// Poses gated ambient tracks.
pub fn play_gated(
    link: Res<crate::sim_link::SimLink>,
    clock: Res<AnimClock>,
    roots: Query<&crate::sim_link::SimEntity>,
    mut q: Query<(&NodeAnim, &crate::render::appearance::ModelNode, &mut AmbientGate, &mut Transform)>,
) {
    let now = clock.now();
    for (track, node, mut gate, mut tf) in &mut q {
        let Ok(id) = roots.get(node.root).map(|id| id.0) else { continue };
        let (next, pose) = match *gate {
            AmbientGate::Docked(start) => {
                let docked = link.curr().unit(id).is_some_and(|u| u.flags.contains(sta_sim::view::UnitFlags::DOCKED));
                let start = match (docked, start) {
                    (true, None) => Some(now),
                    (false, Some(_)) => None,
                    (_, s) => s,
                };
                (AmbientGate::Docked(start), loop_pose(track, start.map(|s| (now - s) as f32)))
            }
            AmbientGate::Once(start) => {
                let start = start.or_else(|| complete(&link, id).then_some(now));
                (AmbientGate::Once(start), once_pose(track, start.map(|s| (now - s) as f32)))
            }
        };
        if *gate != next {
            *gate = next;
        }
        if *tf != pose {
            *tf = pose;
        }
    }
}

/// Poses event tracks: at rest on the first key, or mid-play.
pub fn play_events(clock: Res<AnimClock>,
    bays: Option<Res<crate::render::fx::specials::ConstructionBays>>,
    roots: Query<&crate::sim_link::SimEntity>,
    mut q: Query<(&mut EventAnim, &mut Transform)>,
) {
    let now = clock.now();
    for (mut a, mut tf) in &mut q {
        if matches!(a.trigger, Trigger::Dock | Trigger::Loop | Trigger::Once) {
            let elapsed = a.started.map(|s| (now - s) as f32);
            let p = match a.trigger {
                Trigger::Once => once_pose(&a.track, elapsed),
                Trigger::Loop => loop_pose(&a.track, Some(now as f32)),
                _ => loop_pose(&a.track, elapsed),
            };
            if *tf != p {
                *tf = p;
            }
            continue;
        }
        let held = roots
            .get(a.root)
            .ok()
            .is_some_and(|id| bays.as_ref().is_some_and(|b| b.open.contains(&id.0)));
        let at = if a.trigger == Trigger::Idle { clock.wall() } else { now };
        let (p, going) = construction_pose(&mut a, at, held);
        if !going && a.started.is_some() {
            a.started = None;
        }
        if *tf != p {
            *tf = p;
        }
    }
}

/// Construction uses the existing event track in two halves: open and hold,
/// then close after the last worker is home. Other launches retain their
/// forward/back event behavior. Reopening a closing bay preserves its pose.
fn construction_pose(a: &mut EventAnim, now: f64, held: bool) -> (Transform, bool) {
    let fraction = |a: &EventAnim| {
        let elapsed = a.started.map_or(0.0, |s| (now - s).max(0.0) as f32);
        match a.closing_from {
            Some(from) => (from - elapsed / a.track.period).clamp(0.0, 1.0),
            None => (elapsed / a.track.period).clamp(0.0, 1.0),
        }
    };
    if held && !a.held {
        let from = if a.closing_from.is_some() {
            fraction(a)
        } else {
            0.0
        };
        a.started = Some(now - f64::from(from * a.track.period));
        a.closing_from = None;
    } else if !held && a.held {
        a.closing_from = Some(fraction(a));
        a.started = Some(now);
    }
    a.held = held;
    if held || a.closing_from.is_some() {
        let open = fraction(a);
        let k = ((open * key_count(&a.track) as f32) as usize).min(key_count(&a.track) - 1);
        let going = held || open > 0.0;
        if !going {
            a.closing_from = None;
        }
        (key_pose(&a.track, k), going)
    } else {
        pose(&a.track, a.started.map(|s| (now - s) as f32))
    }
}

pub fn add_systems(app: &mut App) {
    use crate::app::GameSet;
    app.add_message::<SimMsg>().add_systems(
        Update,
        (assign_triggers, assign_gates, play_idle, start_events, track_docks, play_gated, play_events)
            .chain()
            .after(crate::render::fx::specials::ConstructionSet)
            .in_set(GameSet::Interpolate)
            .run_if(resource_exists::<crate::sim_link::SimLink>),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_assets::a2fmt::Matrix34;

    fn mat(x: f32) -> Matrix34 {
        let mut m = Matrix34::IDENTITY;
        m.m[9] = x;
        m
    }

    #[test]
    fn open_ended_tracks_are_events_and_cycles_ambient() {
        // A door: slides out over 20 keys and stops.
        let door = Keys::Matrix((0..20).map(|i| mat(i as f32)).collect());
        assert_eq!(classify(&door), TrackKind::Event);
        // A spinning thing: sweeps a circle, last key one step before the first.
        let spin = Keys::Scale(
            (0..20)
                .map(|i| (i as f32 / 20.0 * std::f32::consts::TAU).sin())
                .collect(),
        );
        assert_eq!(classify(&spin), TrackKind::Ambient);
        // Out and back within the period: a cycle.
        let pulse = Keys::Matrix((0..20).map(|i| mat((i as f32 - 10.0).abs())).collect());
        assert_eq!(classify(&pulse), TrackKind::Ambient);
        assert_eq!(
            classify(&Keys::Matrix(vec![mat(0.0); 10])),
            TrackKind::Ambient
        );
    }

    fn door() -> NodeAnim {
        NodeAnim {
            period: 2.0,
            base: Transform::IDENTITY,
            keys: AnimKeys::Matrix(
                (0..20)
                    .map(|i| Transform::from_xyz(i as f32, 0.0, 0.0))
                    .collect(),
            ),
        }
    }

    #[test]
    fn pose_rests_plays_forward_and_back_once() {
        let d = door();
        assert_eq!(pose(&d, None), (Transform::IDENTITY, false));
        let (mid, going) = pose(&d, Some(1.0));
        assert!(going && mid.translation.x > 8.0);
        let (back, going) = pose(&d, Some(3.9));
        assert!(going && back.translation.x < 2.0);
        assert_eq!(pose(&d, Some(4.1)), (Transform::IDENTITY, false));
    }

    #[test]
    fn construction_holds_the_bay_until_workers_return_then_closes() {
        let mut a = EventAnim::new(door(), Entity::PLACEHOLDER);
        assert_eq!(construction_pose(&mut a, 10.0, true).0.translation.x, 0.0);
        assert_eq!(construction_pose(&mut a, 11.0, true).0.translation.x, 10.0);
        assert_eq!(construction_pose(&mut a, 50.0, true).0.translation.x, 19.0);
        // A paused clock produces exactly the same pose.
        assert_eq!(construction_pose(&mut a, 50.0, true).0.translation.x, 19.0);
        assert_eq!(construction_pose(&mut a, 51.0, false).0.translation.x, 19.0);
        assert_eq!(construction_pose(&mut a, 52.0, false).0.translation.x, 10.0);
        let (shut, going) = construction_pose(&mut a, 53.0, false);
        assert_eq!(shut.translation.x, 0.0);
        assert!(!going);
    }

    #[test]
    fn cancelled_build_closes_from_current_pose_and_new_build_reopens() {
        let mut a = EventAnim::new(door(), Entity::PLACEHOLDER);
        construction_pose(&mut a, 0.0, true);
        let before_cancel = construction_pose(&mut a, 0.8, true).0;
        assert_eq!(construction_pose(&mut a, 0.8, false).0, before_cancel);
        let closing = construction_pose(&mut a, 1.0, false).0;
        assert!(closing.translation.x < before_cancel.translation.x);
        assert_eq!(construction_pose(&mut a, 1.0, true).0, closing);
        assert!(construction_pose(&mut a, 1.5, true).0.translation.x > closing.translation.x);
    }

    /// An idle ship's bay stays shut; a launch event plays it once.
    #[test]
    fn idle_bay_is_shut_and_a_launch_plays_it_once() {
        use sta_sim::command::LocalRouter;
        use sta_sim::fake::ScriptedSource;
        let db = sta_content::fake::db();
        let text = r#"{ units: [{ id: 1, def: "fx_cruiser", owner: 0, hull: 800, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] }],
            events: [{ at: 30, event: { Built: { builder: 1, id: 2 } } }] }"#;
        let source = ScriptedSource::from_json5(text, &db).unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs_f64(1.0 / 60.0),
            ))
            .configure_sets(
                Update,
                (
                    crate::app::GameSet::SimLink,
                    crate::app::GameSet::Interpolate,
                )
                    .chain(),
            )
            .add_plugins(crate::sim_link::SimLinkPlugin)
            .init_resource::<SimEntities>()
            .insert_resource(crate::sim_link::SimLink::new(
                Box::new(source),
                Box::new(LocalRouter::new()),
                sta_sim::PlayerId(0),
            ));
        add_systems(&mut app);
        app.update();
        let root = app
            .world_mut()
            .spawn(crate::sim_link::SimEntity(sta_sim::EntityId(1)))
            .id();
        app.world_mut()
            .resource_mut::<SimEntities>()
            .0
            .insert(sta_sim::EntityId(1), root);
        let node = app
            .world_mut()
            .spawn((EventAnim::new(door(), root), Transform::IDENTITY))
            .id();
        let x = |app: &App| app.world().get::<Transform>(node).unwrap().translation.x;
        let mut peak = 0.0f32;
        let mut peak_before = 0.0f32;
        for frame in 0..600 {
            app.update();
            if frame < 20 {
                peak_before = peak_before.max(x(&app));
            }
            peak = peak.max(x(&app));
        }
        assert_eq!(peak_before, 0.0, "shut while idle");
        assert!(peak > 10.0, "launch opened the bay, peak {peak}");
        assert_eq!(x(&app), 0.0, "and it shut again: played once, not looping");
        assert!(
            app.world()
                .get::<EventAnim>(node)
                .unwrap()
                .started
                .is_none()
        );
    }

    /// The highest the door gets when unit 1 of `def` (with the fake content loaded) has
    /// `event` (a JSON5 `SimEvent`) happen at tick 30.
    fn peak_after(def: &str, event: &str) -> f32 {
        use sta_sim::command::LocalRouter;
        use sta_sim::fake::ScriptedSource;
        let db = sta_content::fake::db();
        let text = format!(
            "{{ units: [{{ id: 1, def: \"{def}\", owner: 0, hull: 800, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 0 }} }}] }}],
               events: [{{ at: 30, event: {event} }}] }}"
        );
        let source = ScriptedSource::from_json5(&text, &db).unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f64(1.0 / 60.0)))
            .configure_sets(Update, (crate::app::GameSet::SimLink, crate::app::GameSet::Interpolate).chain())
            .add_plugins(crate::sim_link::SimLinkPlugin)
            .init_resource::<SimEntities>()
            .insert_resource(crate::sim_link::ContentRes(std::sync::Arc::new(db)))
            .insert_resource(crate::sim_link::SimLink::new(Box::new(source), Box::new(LocalRouter::new()), sta_sim::PlayerId(0)));
        add_systems(&mut app);
        app.update();
        let root = app.world_mut().spawn(crate::sim_link::SimEntity(sta_sim::EntityId(1))).id();
        app.world_mut().resource_mut::<SimEntities>().0.insert(sta_sim::EntityId(1), root);
        let node = app.world_mut().spawn((EventAnim::new(door(), root), Transform::IDENTITY)).id();
        let mut peak = 0.0f32;
        for _ in 0..600 {
            app.update();
            peak = peak.max(app.world().get::<Transform>(node).unwrap().translation.x);
        }
        peak
    }

    #[test]
    fn what_a_unit_can_do_picks_its_trigger() {
        let c = |builds, armed, drop_off| Some(Caps { builds, armed, drop_off, mode: AnimMode::Off });
        assert_eq!(trigger_for(c(true, true, false)), Trigger::Build);
        assert_eq!(trigger_for(c(true, false, true)), Trigger::Build);
        assert_eq!(trigger_for(c(false, true, false)), Trigger::Fire);
        // An armed drop-off keeps its weapons' tracks on its shots.
        assert_eq!(trigger_for(c(false, true, true)), Trigger::Fire);
        assert_eq!(trigger_for(c(false, false, true)), Trigger::Dock);
        assert_eq!(trigger_for(c(false, false, false)), Trigger::None);
        assert_eq!(trigger_for(None), Trigger::Idle);
        // The unit's `animation` key outranks everything but a bay.
        let m = |mode, armed| Some(Caps { builds: false, armed, drop_off: true, mode });
        assert_eq!(trigger_for(m(AnimMode::Loop, true)), Trigger::Loop);
        assert_eq!(trigger_for(m(AnimMode::Once, false)), Trigger::Once);
        let yard = Some(Caps { builds: true, armed: false, drop_off: false, mode: AnimMode::Loop });
        assert_eq!(trigger_for(yard), Trigger::Build);
    }

    fn odf(text: &str) -> std::sync::Arc<sta_odf::Odf> {
        let text = text.to_owned();
        std::sync::Arc::new(sta_odf::resolve("u", &mut |_| Some(text.clone())).unwrap())
    }

    fn unit(class: &str, starbase: bool, odf_text: &str) -> sta_content::UnitDef {
        let mut d = sta_content::UnitDef::new("x", class);
        d.is_starbase = starbase;
        d.raw = odf(&format!("[GameObjectClass]\nclassLabel = \"{class}\"\n{odf_text}\n"));
        d
    }

    #[test]
    fn caps_are_read_from_the_class_and_the_starbase_flag() {
        let db = sta_content::fake::db();
        let of = |class: &str, starbase: bool| Caps::of(&unit(class, starbase, ""), &db).drop_off;
        assert!(of("mining", false) && of("p_mining", false) && of("starbase", false) && of("shipyard", true));
        assert!(!of("shipyard", false) && !of("pod", false) && !of("sensor", false) && !of("freighter", false));
    }

    /// `animation`: absent and 0 and 3 off, 1 once, 2 loop (executable, static; the ODF
    /// comments say the same: "play forward when construction is finished", "looped").
    #[test]
    fn the_animation_key_gives_the_mode() {
        let mode = |t: &str| AnimMode::of(&unit("sensor", false, t));
        assert_eq!(mode(""), AnimMode::Off);
        assert_eq!(mode("animation = 0"), AnimMode::Off);
        assert_eq!(mode("animation = 3"), AnimMode::Off);
        assert_eq!(mode("animation = 1"), AnimMode::Once);
        assert_eq!(mode("animation = 2"), AnimMode::Loop);
    }

    #[test]
    fn ambient_tracks_are_gated_only_where_the_engine_rests_them() {
        assert_eq!(ambient_gate(AnimMode::Off, true), Some(AmbientGate::Docked(None)));
        assert_eq!(ambient_gate(AnimMode::Once, false), Some(AmbientGate::Once(None)));
        assert_eq!(ambient_gate(AnimMode::Loop, true), None);
        assert_eq!(ambient_gate(AnimMode::Off, false), None);
    }

    /// A weapon's shots animate its owner only if its ODF says `animateOwner`; a weapon with no
    /// ODF text behind it keeps the old "every shot" behaviour.
    #[test]
    fn a_weapon_animates_its_owner_by_its_odf() {
        let mut w = sta_content::WeaponDef::new("w", "cannon", sta_content::WeaponKind::Beam);
        assert!(owner_animates(&w), "no ODF text");
        w.raw = odf("[GameObjectClass]\nclassLabel = \"cannon\"\n");
        assert!(!owner_animates(&w));
        w.raw = odf("[GameObjectClass]\nclassLabel = \"cannon\"\nanimateOwner = 1\n");
        assert!(owner_animates(&w));
    }

    const FIRED: &str = "{ Fired: { shooter: 1, weapon: 0, hardpoint: 0, target: { Entity: 1 } } }";
    const BUILT: &str = "{ Built: { builder: 1, id: 2 } }";

    /// An armed unit's event tracks play when it fires, and not when a ship leaves it.
    #[test]
    fn an_armed_units_track_plays_on_its_own_shot() {
        assert!(peak_after("fx_cruiser", FIRED) > 10.0);
        assert_eq!(peak_after("fx_cruiser", BUILT), 0.0);
    }

    /// A builder's tracks play on a build or launch, not on its guns.
    #[test]
    fn a_builders_track_plays_on_a_launch_and_not_on_a_shot() {
        assert!(peak_after("fx_yard", BUILT) > 10.0);
        assert_eq!(peak_after("fx_yard", FIRED), 0.0);
    }

    /// A unit that neither builds nor fires has no sim event for its track: it rests.
    #[test]
    fn a_unit_with_no_event_to_wait_for_rests() {
        assert_eq!(peak_after("fx_miner", FIRED), 0.0);
        assert_eq!(peak_after("fx_miner", BUILT), 0.0);
    }

    /// A source that raises `DOCKED` on unit 1 for ticks `from..to`, as the sim does while a
    /// freighter unloads there.
    struct DockSource {
        inner: sta_sim::fake::ScriptedSource,
        from: u64,
        to: u64,
        tick: u64,
    }

    impl DockSource {
        fn flagged(&self, v: std::sync::Arc<sta_sim::WorldView>) -> std::sync::Arc<sta_sim::WorldView> {
            let mut v = (*v).clone();
            let on = (self.from..self.to).contains(&self.tick);
            for u in &mut v.units {
                u.flags = u.flags.with(sta_sim::view::UnitFlags::DOCKED, on);
            }
            std::sync::Arc::new(v)
        }
    }

    impl sta_sim::WorldSource for DockSource {
        fn step(&mut self, c: &[sta_sim::command::Command]) -> sta_sim::TickOutput {
            self.tick += 1;
            let mut out = self.inner.step(c);
            out.view = self.flagged(out.view);
            out
        }
        fn view(&self) -> std::sync::Arc<sta_sim::WorldView> {
            self.flagged(self.inner.view())
        }
    }

    /// The door's x at each frame while unit 1 (a drop-off with no build list and no weapons)
    /// is docked at for ticks 30..`to`; `joined`: the client starts with the unit already docked.
    fn dock_app(to: u64, joined: bool) -> (App, Entity) {
        use sta_sim::command::LocalRouter;
        let mut db = sta_content::fake::db();
        let mut builder = sta_content::ContentDb::builder().profile(db.profile()).features(db.features().clone());
        for u in db.units() {
            builder = builder.unit(u.clone());
        }
        for w in db.weapons() {
            builder = builder.weapon(w.clone());
        }
        db = builder.unit(sta_content::UnitDef::new("fx_dock", "mining")).build().unwrap();
        let text = "{ units: [{ id: 1, def: \"fx_dock\", owner: 0, hull: 800, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] }], events: [] }";
        let inner = sta_sim::fake::ScriptedSource::from_json5(text, &db).unwrap();
        let src = DockSource { inner, from: if joined { 0 } else { 30 }, to, tick: 0 };
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f64(1.0 / 60.0)))
            .configure_sets(Update, (crate::app::GameSet::SimLink, crate::app::GameSet::Interpolate).chain())
            .add_plugins(crate::sim_link::SimLinkPlugin)
            .init_resource::<SimEntities>()
            .insert_resource(crate::sim_link::ContentRes(std::sync::Arc::new(db)))
            .insert_resource(crate::sim_link::SimLink::new(Box::new(src), Box::new(LocalRouter::new()), sta_sim::PlayerId(0)));
        add_systems(&mut app);
        app.update();
        let root = app.world_mut().spawn(crate::sim_link::SimEntity(sta_sim::EntityId(1))).id();
        app.world_mut().resource_mut::<SimEntities>().0.insert(sta_sim::EntityId(1), root);
        (app, root)
    }

    fn dock_run(to: u64, frames: usize, joined: bool) -> Vec<f32> {
        let (mut app, root) = dock_app(to, joined);
        let node = app.world_mut().spawn((EventAnim::new(door(), root), Transform::IDENTITY)).id();
        (0..frames)
            .map(|_| {
                app.update();
                app.world().get::<Transform>(node).unwrap().translation.x
            })
            .collect()
    }

    /// An ambient (cycle) track of a drop-off whose ODF has no `animation` key rests until a
    /// freighter docks, loops while it is docked and rests again after: the stock mining
    /// stations (A1 and A2 `fmining`, `kmining`, `rmining`, `bmining`).
    #[test]
    fn an_ambient_track_of_a_drop_off_loops_only_while_docked() {
        let (mut app, root) = dock_app(300, false);
        let node = app
            .world_mut()
            .spawn((
                door(),
                crate::render::appearance::ModelNode { root, index: 0 },
                Transform::IDENTITY,
            ))
            .id();
        let xs: Vec<f32> = (0..900)
            .map(|_| {
                app.update();
                app.world().get::<Transform>(node).unwrap().translation.x
            })
            .collect();
        assert!(xs[..40].iter().all(|&x| x == 0.0), "rests before the dock");
        assert!(xs[100..560].iter().cloned().fold(0.0, f32::max) > 10.0, "loops while docked");
        assert!(xs[700..].iter().all(|&x| x == 0.0), "rests after");
        assert!(app.world().get::<AmbientGate>(node).is_some());
    }

    /// A drop-off's track loops while a freighter is docked and rests on the first key before
    /// and after (the engine's flag 1 at `SetDock`, animation off when the freighter leaves).
    #[test]
    fn a_drop_offs_track_loops_while_docked_and_rests_otherwise() {
        // 60 fps frames; the sim ticks at 30 Hz, so tick 30..300 is about frames 60..600.
        let xs = dock_run(300, 900, false);
        assert!(xs[..40].iter().all(|&x| x == 0.0), "rests before the dock");
        let docked = &xs[100..560];
        assert!(docked.iter().cloned().fold(0.0, f32::max) > 10.0, "plays while docked");
        // Looped: with a 2 s period it passes the first key again during the dock.
        let wraps = docked.windows(2).filter(|w| w[1] < w[0]).count();
        assert!(wraps >= 2, "wraps {wraps}");
        assert!(xs[700..].iter().all(|&x| x == 0.0), "rests after the dock");
    }

    /// A client that joins (or loads a snapshot) while a freighter is docked plays at once:
    /// the state is in the view, not in an event it missed.
    #[test]
    fn a_client_joining_mid_dock_shows_the_dock_animation() {
        let xs = dock_run(10_000, 200, true);
        assert!(xs[..200].iter().cloned().fold(0.0, f32::max) > 10.0);
    }

    /// A model with no sim unit plays by itself, on the wall clock, and rests between.
    #[test]
    fn a_model_with_no_sim_unit_plays_on_its_own_and_rests_between() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f64(1.0 / 30.0)))
            .insert_resource(AnimClock::default())
            .add_systems(Update, (crate::clock::advance_idle_clock, play_idle, play_events).chain());
        let root = app.world_mut().spawn_empty().id();
        let node = app.world_mut().spawn((EventAnim::new(door(), root), Transform::IDENTITY)).id();
        let (mut peaks, mut shut_after) = (0, false);
        let mut was_open = false;
        for _ in 0..600 {
            app.update();
            let open = app.world().get::<Transform>(node).unwrap().translation.x > 10.0;
            if open && !was_open {
                peaks += 1;
            }
            if was_open && !open {
                shut_after = true;
            }
            was_open = open;
        }
        // The door is 2 s each way (4 s a play) and rests a period (2 s): 20 s is three plays.
        assert!((3..=4).contains(&peaks) && shut_after, "plays: {peaks}");
        assert!(app.world().get::<EventAnim>(node).unwrap().trigger == Trigger::Idle);
    }

    /// With `STA64_TEST_INSTALL` (+ `STA64_TEST_MOD`): every model's tracks, ambient
    /// or event. Stock ships' open-ended tracks are bay doors and arms, so the
    /// listing is printed (run with `--nocapture`) and the check is that such tracks
    /// exist and that no event track is on a planet, nebula or field.
    #[test]
    fn install_lists_tracks_as_ambient_or_event() {
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
            return;
        };
        let m = std::env::var("STA64_TEST_MOD")
            .ok()
            .filter(|m| !m.is_empty());
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), m.as_deref()).unwrap();
        let (mut ambient, mut event) = (0, 0);
        let mut bad = Vec::new();
        // Each model's unit, from the ODFs: what it builds and whether it is armed.
        let (index, _) = sta_content::OdfIndex::build(&vfs);
        let caps: std::collections::BTreeMap<String, Caps> = match sta_content::ContentDb::load(&vfs, &index) {
            Ok((db, _)) => {
                let mut m = std::collections::BTreeMap::new();
                for u in db.units() {
                    let c = Caps::of(u, &db);
                    let e = m.entry(u.model.to_ascii_lowercase()).or_insert(Caps::default());
                    e.builds |= c.builds;
                    e.armed |= c.armed;
                    e.drop_off |= c.drop_off;
                    if e.mode == AnimMode::Off {
                        e.mode = c.mode;
                    }
                }
                m
            }
            Err(e) => panic!("content did not load: {e}"),
        };
        let mut by_trigger: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for name in sta_assets::list_models(&vfs) {
            let Some((path, fmt)) = sta_assets::find_model(&vfs, &name) else {
                continue;
            };
            let Ok(model) = sta_assets::load_model(&vfs, &path, fmt) else {
                continue;
            };
            let mut line = Vec::new();
            let mut events_here = 0;
            for c in model.channels.iter().filter(|c| c.period > 0.0) {
                let kind = classify(&c.keys);
                match kind {
                    TrackKind::Ambient => ambient += 1,
                    TrackKind::Event => {
                        event += 1;
                        events_here += 1;
                        if name.starts_with("map_planet")
                            || name.starts_with("map_nebula")
                            || name.contains("asteroid")
                            || name.contains("field")
                        {
                            bad.push(format!("{name}:{}", c.node));
                        }
                    }
                }
                line.push(format!(
                    "{}={kind:?}({:.2},p{},k{})",
                    c.node,
                    ratio(&c.keys),
                    c.period,
                    key_len(&c.keys)
                ));
            }
            if !line.is_empty() {
                println!("{name}: {}", line.join(" "));
            }
            if events_here > 0 {
                // The model's unit decides; a model no unit uses is an effect or a map
                // object, with no sim unit to wait for.
                let t = trigger_for(caps.get(&name.to_ascii_lowercase()).copied());
                by_trigger.entry(format!("{t:?}")).or_default().push(format!("{name}({events_here})"));
            }
        }
        println!("tracks: {ambient} ambient, {event} event");
        for (t, models) in &by_trigger {
            println!("event tracks, trigger {t}: {}", models.join(" "));
        }
        assert!(!by_trigger.is_empty(), "no model has an event track: the check is vacuous");
        // The models resolve to their units: some event tracks belong to builders.
        assert!(by_trigger.contains_key("Build"), "no model resolved to a builder: {:?}", by_trigger.keys());
        assert!(bad.is_empty(), "scenery tracks classed as events: {bad:?}");
    }
}
