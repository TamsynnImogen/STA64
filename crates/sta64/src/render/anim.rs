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
//! its first key (shut) and, when its model's builder starts a build or launches a
//! ship ([`SimEvent::BuildStarted`], [`SimEvent::Built`]), plays once forward and
//! back (open, then close) on the sim clock. Docking has no sim event yet; when it
//! gains one it only needs another arm in [`triggers`].
use crate::clock::AnimClock;
use crate::render::model_scene::{AnimKeys, NodeAnim};
use crate::sim_link::{SimEntities, SimMsg};
use bevy::prelude::*;
use sta_assets::model::Keys;
use sta_sim::SimEvent;

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
}

impl EventAnim {
    pub fn new(track: NodeAnim, root: Entity) -> Self {
        EventAnim {
            track,
            root,
            started: None,
            held: false,
            closing_from: None,
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

/// Whether `event` plays the event tracks of the builder it names.
pub fn triggers(event: &SimEvent) -> Option<sta_sim::EntityId> {
    match event {
        // Build arms, and the bay opening as the ship starts to come together.
        SimEvent::BuildStarted { builder, .. } => Some(*builder),
        // The finished ship leaves: bay doors.
        SimEvent::Built { builder, .. } => Some(*builder),
        _ => None,
    }
}

/// Starts the event tracks of every builder an event names.
pub fn start_events(
    mut msgs: MessageReader<SimMsg>,
    ents: Res<SimEntities>,
    clock: Res<AnimClock>,
    mut q: Query<&mut EventAnim>,
) {
    let mut roots = Vec::new();
    for SimMsg(ev) in msgs.read() {
        if let Some(root) = triggers(ev).and_then(|id| ents.0.get(&id)) {
            roots.push(*root);
        }
    }
    if roots.is_empty() {
        return;
    }
    let now = clock.now();
    for mut a in &mut q {
        // A play in progress is not restarted.
        if a.started.is_none() && roots.contains(&a.root) {
            a.started = Some(now);
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
        let held = roots
            .get(a.root)
            .ok()
            .is_some_and(|id| bays.as_ref().is_some_and(|b| b.open.contains(&id.0)));
        let (p, going) = construction_pose(&mut a, now, held);
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
        (start_events, play_events)
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
        for name in sta_assets::list_models(&vfs) {
            let Some((path, fmt)) = sta_assets::find_model(&vfs, &name) else {
                continue;
            };
            let Ok(model) = sta_assets::load_model(&vfs, &path, fmt) else {
                continue;
            };
            let mut line = Vec::new();
            for c in model.channels.iter().filter(|c| c.period > 0.0) {
                let kind = classify(&c.keys);
                match kind {
                    TrackKind::Ambient => ambient += 1,
                    TrackKind::Event => {
                        event += 1;
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
        }
        println!("tracks: {ambient} ambient, {event} event");
        assert!(bad.is_empty(), "scenery tracks classed as events: {bad:?}");
    }
}
