//! Ships in the map view, driven by the deterministic simulation.
//!
//! - Objects whose ODF names a `physicsFile` become `sta_sim` units with the
//!   physics ODF's movement limits; their position and heading come from the map.
//! - The simulation steps at `TICK_HZ` in `FixedUpdate`; each frame the renderer
//!   places the ships between the last two ticks (interpolation).
//! - Left click selects (Shift adds), left drag selects by box, right click
//!   orders the selection to move, keeping its formation. Space stops it.
//!
//! The sim works in the map's left-handed space; the renderer mirrors Z.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use sta_sim::{Command, EntityId, Fixed, MoveStats, Sim, Vec3 as SimVec3};
use std::collections::HashMap;

/// The running simulation and its link to entities.
#[derive(Resource)]
pub struct Game {
    pub sim: Sim,
    pending: Vec<Command>,
    /// Positions and headings before the last tick, for interpolation.
    prev: HashMap<EntityId, ([f32; 3], [f32; 2])>,
}

impl Default for Game {
    fn default() -> Self {
        Game {
            sim: Sim::new(1),
            pending: Vec::new(),
            prev: HashMap::new(),
        }
    }
}

impl Game {
    pub fn reset(&mut self) {
        *self = Game::default();
    }

    /// Add a unit at a map transform (right, up, front, position).
    pub fn spawn(&mut self, team: i32, transform: &[f32; 12], stats: MoveStats) -> EntityId {
        let pos = SimVec3::from_f32([transform[9], transform[10], transform[11]]);
        let heading = (Fixed::from_f64(transform[6] as f64), Fixed::from_f64(transform[8] as f64));
        self.sim.spawn(team.clamp(0, 255) as u8, pos, heading, stats)
    }
}

/// An entity drawn at a sim unit's interpolated place.
#[derive(Component)]
pub struct Unit {
    pub id: EntityId,
    /// Selection radius in world units.
    pub radius: f32,
}

#[derive(Component)]
pub struct Selected;

/// The drag rectangle shown while box-selecting.
#[derive(Component)]
struct SelectBox;

#[derive(Resource, Default)]
struct Drag {
    start: Option<Vec2>,
}

pub struct UnitsPlugin;

impl Plugin for UnitsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Game>()
            .init_resource::<Drag>()
            .insert_resource(Time::<Fixed64>::from_hz(sta_sim::TICK_HZ as f64))
            .add_systems(Startup, spawn_select_box)
            .add_systems(FixedUpdate, tick)
            .add_systems(Update, (select, order, place, rings).chain());
    }
}

type Fixed64 = bevy::time::Fixed;

fn to_bevy(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[0], p[1], -p[2])
}

fn tick(mut game: ResMut<Game>) {
    let game = &mut *game;
    game.prev.clear();
    for u in &game.sim.units {
        game.prev.insert(u.id, (u.pos.to_f32(), [u.heading.0.to_f32(), u.heading.1.to_f32()]));
    }
    let cmds = std::mem::take(&mut game.pending);
    game.sim.step(&cmds);
}

/// Place each unit between its last two ticks.
fn place(game: Res<Game>, time: Res<Time<Fixed64>>, mut q: Query<(&Unit, &mut Transform)>) {
    let a = time.overstep_fraction().clamp(0.0, 1.0);
    for (u, mut tf) in &mut q {
        let Some(s) = game.sim.unit(u.id) else { continue };
        let now = (s.pos.to_f32(), [s.heading.0.to_f32(), s.heading.1.to_f32()]);
        let (p0, h0) = game.prev.get(&u.id).copied().unwrap_or(now);
        let pos = Vec3::from(p0).lerp(Vec3::from(now.0), a);
        let h = Vec2::from(h0).lerp(Vec2::from(now.1), a).normalize_or(Vec2::Y);
        tf.translation = to_bevy(pos.to_array());
        // The model's front (+Z in the files) is Bevy's -Z after the mirror, which
        // is what `looking_to` points along the heading.
        tf.rotation = Transform::IDENTITY.looking_to(Vec3::new(h.x, 0.0, -h.y), Vec3::Y).rotation;
    }
}

fn spawn_select_box(mut commands: Commands) {
    commands.spawn((
        SelectBox,
        Node {
            position_type: PositionType::Absolute,
            ..default()
        },
        BackgroundColor(Color::srgba(0.3, 1.0, 0.4, 0.15)),
        Visibility::Hidden,
    ));
}

fn cursor_ray(window: &Window, cam: (&Camera, &GlobalTransform)) -> Option<(Vec2, Ray3d)> {
    let c = window.cursor_position()?;
    cam.0.viewport_to_world(cam.1, c).ok().map(|r| (c, r))
}

#[allow(clippy::too_many_arguments)]
fn select(
    mut commands: Commands,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), crate::map::MainCamera>,
    units: Query<(Entity, &Unit, &GlobalTransform, Option<&crate::render::strategic::Strategic>)>,
    selected: Query<Entity, With<Selected>>,
    mut drag: ResMut<Drag>,
    mut boxes: Query<(&mut Node, &mut Visibility), With<SelectBox>>,
) {
    let (Ok(window), Ok(cam)) = (window.single(), camera.single()) else { return };
    let Some((cursor, ray)) = cursor_ray(window, cam) else { return };
    if buttons.just_pressed(MouseButton::Left) {
        drag.start = Some(cursor);
    }
    let Some(start) = drag.start else { return };
    let dragging = start.distance(cursor) > 6.0;
    if let Ok((mut node, mut vis)) = boxes.single_mut() {
        if dragging && buttons.pressed(MouseButton::Left) {
            let (lo, hi) = (start.min(cursor), start.max(cursor));
            node.left = Val::Px(lo.x);
            node.top = Val::Px(lo.y);
            node.width = Val::Px(hi.x - lo.x);
            node.height = Val::Px(hi.y - lo.y);
            *vis = Visibility::Visible;
        } else {
            *vis = Visibility::Hidden;
        }
    }
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    drag.start = None;
    let add = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    if !add {
        for e in &selected {
            commands.entity(e).remove::<Selected>();
        }
    }
    if dragging {
        let (lo, hi) = (start.min(cursor), start.max(cursor));
        for (e, _, gt, _) in &units {
            if let Ok(p) = cam.0.world_to_viewport(cam.1, gt.translation())
                && p.cmpge(lo).all()
                && p.cmple(hi).all()
            {
                commands.entity(e).insert(Selected);
            }
        }
    } else {
        // A unit whose strategic icon is under the cursor (nearest icon centre)...
        let icon = units
            .iter()
            .filter_map(|(e, _, _, s)| {
                let s = s.filter(|s| s.shown > 0.5)?;
                let d = s.screen.distance(cursor);
                (d <= s.size_px * 0.6).then_some((d, e))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, e)) = icon {
            commands.entity(e).insert(Selected);
            return;
        }
        // ...else the nearest unit whose sphere the ray passes through.
        let mut best: Option<(f32, Entity)> = None;
        for (e, u, gt, _) in &units {
            let to = gt.translation() - ray.origin;
            let t = to.dot(*ray.direction);
            if t < 0.0 {
                continue;
            }
            let miss = (to - *ray.direction * t).length();
            if miss <= u.radius && best.is_none_or(|(bt, _)| t < bt) {
                best = Some((t, e));
            }
        }
        if let Some((_, e)) = best {
            commands.entity(e).insert(Selected);
        }
    }
}

fn order(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), crate::map::MainCamera>,
    selected: Query<&Unit, With<Selected>>,
    mut game: ResMut<Game>,
) {
    if keys.just_pressed(KeyCode::Space) {
        for u in &selected {
            game.pending.push(Command::Stop { unit: u.id });
        }
    }
    if !buttons.just_pressed(MouseButton::Right) {
        return;
    }
    let (Ok(window), Ok(cam)) = (window.single(), camera.single()) else { return };
    let Some((_, ray)) = cursor_ray(window, cam) else { return };
    // Positions in the sim's space; the order point is on the plane at the
    // selection's mean height.
    let picked: Vec<(EntityId, [f32; 3])> = selected
        .iter()
        .filter_map(|u| game.sim.unit(u.id).map(|s| (u.id, s.pos.to_f32())))
        .collect();
    if picked.is_empty() {
        return;
    }
    let n = picked.len() as f32;
    let centre = picked.iter().fold(Vec3::ZERO, |a, (_, p)| a + Vec3::from(*p)) / n;
    let dir = *ray.direction;
    if dir.y.abs() < 1e-4 {
        return;
    }
    let t = (centre.y - ray.origin.y) / dir.y;
    if t <= 0.0 {
        return;
    }
    let hit = ray.origin + dir * t;
    let target = Vec3::new(hit.x, centre.y, -hit.z);
    // Keep the formation: each unit keeps its offset from the group's centre.
    for (id, p) in picked {
        let to = target + (Vec3::from(p) - centre);
        game.pending.push(Command::Move {
            unit: id,
            to: SimVec3::from_f32(to.to_array()),
        });
    }
}

fn rings(mut gizmos: Gizmos, selected: Query<(&Unit, &GlobalTransform), With<Selected>>, game: Res<Game>) {
    let flat = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    for (u, gt) in &selected {
        let colour = Color::srgb(0.3, 1.0, 0.4);
        gizmos.circle(Isometry3d::new(gt.translation(), flat), u.radius, colour);
        if let Some(t) = game.sim.unit(u.id).and_then(|s| s.target) {
            let to = to_bevy(t.to_f32());
            gizmos.line(gt.translation(), to, Color::srgba(0.3, 1.0, 0.4, 0.4));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    /// A ship cruising in a straight line must move the same distance every
    /// rendered frame (sim at 30 Hz, frames at 60 Hz, interpolated between ticks).
    #[test]
    fn straight_line_motion_is_even_per_frame() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0)))
            .insert_resource(Time::<Fixed64>::from_hz(sta_sim::TICK_HZ as f64))
            .init_resource::<Game>()
            .add_systems(FixedUpdate, tick)
            .add_systems(Update, place);
        let id = {
            let mut game = app.world_mut().resource_mut::<Game>();
            let t = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
            let id = game.spawn(1, &t, MoveStats::from_f32(140.0, 100.0, 1.0, 1.0, 1.0));
            game.pending.push(Command::Move { unit: id, to: SimVec3::from_f32([0.0, 0.0, 100_000.0]) });
            id
        };
        let e = app.world_mut().spawn((Unit { id, radius: 10.0 }, Transform::IDENTITY)).id();
        let mut zs = Vec::new();
        for _ in 0..180 {
            app.update();
            zs.push(app.world().get::<Transform>(e).unwrap().translation.z);
        }
        // Skip the first second (acceleration), then compare frame steps.
        let steps: Vec<f32> = zs.windows(2).skip(60).map(|w| (w[1] - w[0]).abs()).collect();
        let (lo, hi) = steps.iter().fold((f32::MAX, 0f32), |(l, h), &s| (l.min(s), h.max(s)));
        let expect = 140.0 / 60.0;
        assert!(lo > expect * 0.8 && hi < expect * 1.2, "uneven frame steps: min {lo}, max {hi}, expected ~{expect}");
    }

    /// With uneven frame times the on-screen speed (step / frame time) must stay
    /// steady too.
    #[test]
    fn motion_follows_uneven_frames() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Time::<Fixed64>::from_hz(sta_sim::TICK_HZ as f64))
            .init_resource::<Game>()
            .add_systems(FixedUpdate, tick)
            .add_systems(Update, place);
        let id = {
            let mut game = app.world_mut().resource_mut::<Game>();
            let t = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
            let id = game.spawn(1, &t, MoveStats::from_f32(140.0, 100.0, 1.0, 1.0, 1.0));
            game.pending.push(Command::Move { unit: id, to: SimVec3::from_f32([0.0, 0.0, 100_000.0]) });
            id
        };
        let e = app.world_mut().spawn((Unit { id, radius: 10.0 }, Transform::IDENTITY)).id();
        let frames = [0.007, 0.023, 0.016, 0.031, 0.011, 0.019, 0.042, 0.009];
        let mut speeds = Vec::new();
        let mut last = 0.0f32;
        for i in 0..400 {
            let dt = frames[i % frames.len()];
            app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(dt)));
            app.update();
            let z = app.world().get::<Transform>(e).unwrap().translation.z;
            if i > 120 {
                speeds.push((z - last).abs() / dt as f32);
            }
            last = z;
        }
        let (lo, hi) = speeds.iter().fold((f32::MAX, 0f32), |(l, h), &s| (l.min(s), h.max(s)));
        assert!(lo > 140.0 * 0.85 && hi < 140.0 * 1.15, "on-screen speed varies: {lo}..{hi}");
    }
}
