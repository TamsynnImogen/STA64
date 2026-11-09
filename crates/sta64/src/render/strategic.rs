//! Strategic icons: zoomed out, each team's ships and stations show as solid
//! team-coloured icons at a fixed screen size (as in Sins of a Solar Empire and
//! Supreme Commander), fading in as the model gets too small to make out.
//!
//! Stock Armada II only grows small objects a little in its strategic view
//! (`cfgMIN_STRATEGIC_VIEW_OBJECT_RADIUS` 25, at most x1.75); on large maps that
//! still leaves ships as specks, hence the icons.
//!
//! - Each object's icon is its model's silhouette from above, in its team's colour:
//!   the mod's own `<model>_sv` strategic sprite when its sprite table has one
//!   (the FO4 / STA Classic strategic sprite sets), else baked from the model at
//!   load (`sta_assets::silhouette`, the same look). Ships turn with their heading.
//! - Without a model: an arrow for ships, a square for stations, a circle for
//!   support craft.
//! - Each icon fades in as its object's on-screen radius falls from
//!   [`FADE_START_PX`] to [`FADE_FULL_PX`]. Tab cycles auto / always / off.
//! - Zoomed out past [`STRATEGIC_HEIGHT`] (or Tab on "always") the view is
//!   strategic: models hide, every unit shows as an icon, a grid is drawn on the
//!   map plane, and a height bar drops from each unit to it. Icons within
//!   [`STACK_RADIUS_PX`] on screen merge into one fleet icon with a count badge,
//!   grouped by id order (see [`stack`]).
//! - Selected units' icons get a green outline; clicks and drag boxes select by
//!   icon when it shows (see `input::select`).

use crate::render::units::{FromView, NeedsModel, SelectRadius, TeamColour};
use crate::sim_link::{Selection, SimEntity};
use sta_sim::EntityId;
use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

/// On-screen radius (px) where an icon starts to show, and where it is solid.
pub const FADE_START_PX: f32 = 14.0;
pub const FADE_FULL_PX: f32 = 7.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Arrow,
    Square,
    Circle,
}

/// An object shown as an icon when zoomed out.
#[derive(Component, Clone)]
pub struct Strategic {
    pub shape: Shape,
    pub colour: Color,
    /// World radius (for the fade and the icon's size tier).
    pub radius: f32,
    /// How much the icon shows this frame (0 hidden, 1 solid), and where.
    pub shown: f32,
    pub screen: Vec2,
    pub size_px: f32,
    /// The silhouette (white, tinted by `colour`), and its rectangle (0..1) when it
    /// is a sprite in an atlas; `None`: the generic `shape`.
    pub image: Option<Handle<Image>>,
    pub uv: Option<[f32; 4]>,
}

impl Strategic {
    pub fn new(shape: Shape, colour: Color, radius: f32) -> Self {
        Strategic { shape, colour, radius, shown: 0.0, screen: Vec2::ZERO, size_px: icon_size(shape, radius), image: None, uv: None }
    }

    /// With a silhouette image (`uv`: its rectangle in an atlas).
    pub fn with_image(mut self, image: Handle<Image>, uv: Option<[f32; 4]>) -> Self {
        self.image = Some(image);
        self.uv = uv;
        self.size_px = silhouette_size(self.radius);
        self
    }
}

/// A silhouette's edge in pixels: small craft 18, growing with size to 40.
pub fn silhouette_size(radius: f32) -> f32 {
    (14.0 + radius * 0.06).clamp(18.0, 40.0)
}

/// Silhouettes baked from models, by model path (`None`: nothing solid).
#[derive(Resource, Default)]
pub struct Silhouettes(std::collections::HashMap<String, Option<Handle<Image>>>);

impl Silhouettes {
    pub fn get(&mut self, key: &str, model: &sta_assets::model::Model, images: &mut Assets<Image>) -> Option<Handle<Image>> {
        self.0
            .entry(key.to_owned())
            .or_insert_with(|| sta_assets::silhouette::bake(model, 64, 4).map(|s| images.add(silhouette_image(&s))))
            .clone()
    }
}

/// White where the silhouette is, with a dark one-pixel rim (it stays dark under
/// the team tint) so icons read against bright nebulae.
pub fn silhouette_image(s: &sta_assets::silhouette::Silhouette) -> Image {
    let n = s.size as i32;
    let at = |x: i32, y: i32| if x < 0 || y < 0 || x >= n || y >= n { 0 } else { s.alpha[(y * n + x) as usize] };
    let mut data = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let a = at(x, y);
            let mut rim = 0u8;
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1)] {
                rim = rim.max(at(x + dx, y + dy));
            }
            let px = if a >= 128 { [255, 255, 255, a] } else { [30, 30, 30, a.max((rim as f32 * 0.85) as u8)] };
            data.extend_from_slice(&px);
        }
    }
    Image::new(
        Extent3d { width: s.size, height: s.size, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Icon edge in pixels: ships in three tiers by radius, stations larger.
pub fn icon_size(shape: Shape, radius: f32) -> f32 {
    match shape {
        Shape::Square => 20.0,
        Shape::Circle => 14.0,
        Shape::Arrow if radius < 60.0 => 13.0,
        Shape::Arrow if radius < 150.0 => 17.0,
        Shape::Arrow => 22.0,
    }
}

/// The shape for an object from its ODF class label; `mobile` when it moves.
pub fn shape_for(class: &str, mobile: bool) -> Shape {
    match class.to_ascii_lowercase().as_str() {
        "freighter" | "constructionrig" | "borgbore" | "miner" => Shape::Circle,
        _ if mobile => Shape::Arrow,
        _ => Shape::Square,
    }
}

/// How much an icon shows for an object `px` pixels in radius on screen.
pub fn fade(px: f32) -> f32 {
    ((FADE_START_PX - px) / (FADE_START_PX - FADE_FULL_PX)).clamp(0.0, 1.0)
}

#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    #[default]
    Auto,
    Always,
    Off,
}

/// Camera height above the map plane from which the view is strategic (Auto).
pub const STRATEGIC_HEIGHT: f32 = 2500.0;
/// Icons closer than this on screen merge into one fleet icon.
pub const STACK_RADIUS_PX: f32 = 28.0;
/// Grid cell edge in world units, and cells drawn each way from the view centre.
pub const GRID_STEP: f32 = 500.0;
pub const GRID_HALF_CELLS: i32 = 12;

/// A fleet icon: one unit, or several merged. `leader` is the lowest id.
#[derive(Clone, Debug, PartialEq)]
pub struct StackIcon {
    pub leader: EntityId,
    pub members: Vec<EntityId>,
    /// Leader's world position.
    pub pos: Vec3,
}

impl StackIcon {
    pub fn count(&self) -> usize {
        self.members.len()
    }
}

/// A vertical line from a unit down to the map plane (y = 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeightBar {
    pub unit: EntityId,
    pub top: Vec3,
    pub bottom: Vec3,
}

impl HeightBar {
    pub fn length(&self) -> f32 {
        self.top.y - self.bottom.y
    }
}

/// What the strategic view shows this frame.
#[derive(Resource, Default, Debug)]
pub struct StrategicView {
    pub active: bool,
    pub icons: Vec<StackIcon>,
    pub bars: Vec<HeightBar>,
    pub grid_lines: usize,
    /// Units whose own icon is folded into another's.
    pub folded: std::collections::BTreeSet<EntityId>,
    models_hidden: bool,
}

/// Merge units whose screen distance is under `radius_px` into stacks. Units go
/// in id order; each joins the first stack whose leader (its lowest id) is in
/// range, else leads a new one. Screen distance is the offset across the view
/// direction scaled by `focal / depth` (pixels per world unit at distance 1).
pub fn stack(units: &[(EntityId, Vec3)], eye: Vec3, fwd: Vec3, focal: f32, radius_px: f32) -> Vec<StackIcon> {
    let mut sorted: Vec<(EntityId, Vec3)> = units.to_vec();
    sorted.sort_by_key(|u| u.0);
    let mut out: Vec<StackIcon> = Vec::new();
    for (id, pos) in sorted {
        let depth = (pos - eye).dot(fwd).max(1.0);
        let near = out.iter_mut().find(|s| {
            let d = pos - s.pos;
            (d - fwd * d.dot(fwd)).length() * focal / depth <= radius_px
        });
        match near {
            Some(s) => s.members.push(id),
            None => out.push(StackIcon { leader: id, members: vec![id], pos }),
        }
    }
    out
}

#[derive(Resource)]
struct Masks {
    fill: [Handle<Image>; 3],
    /// The selection ring, drawn around any icon.
    ring: Handle<Image>,
}

/// The UI node drawing an object's icon (and its selection outline child).
#[derive(Component)]
struct Icon {
    target: Entity,
    outline: Entity,
    /// The count badge of a fleet icon.
    badge: Entity,
}

pub struct StrategicPlugin;

impl Plugin for StrategicPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Mode>()
            .init_resource::<StrategicView>()
            .add_systems(Startup, make_masks)
            .add_systems(Update, (toggle, ensure_icons, attach).chain())
            .add_systems(PostUpdate, (compute, draw.run_if(resource_exists::<bevy::gizmos::config::GizmoConfigStore>)).chain().before(update))
            // After this frame's transforms, before the UI lays the icons out.
            .add_systems(
                PostUpdate,
                update.after(TransformSystems::Propagate).before(bevy::ui::UiSystems::Prepare),
            );
    }
}

const MASK: u32 = 64;

/// Coverage of each shape at (x, y) in -1..1 (y down), as signed distance in
/// mask pixels: positive inside.
fn inside(shape: Shape, x: f32, y: f32) -> f32 {
    let px = MASK as f32 / 2.0;
    match shape {
        Shape::Square => (0.78 - x.abs()).min(0.78 - y.abs()) * px,
        Shape::Circle => (0.8 - (x * x + y * y).sqrt()) * px,
        Shape::Arrow => {
            // A dart pointing up: tip (0,-0.95), wings (+-0.75, 0.85), notch (0, 0.45).
            let tri = [Vec2::new(0.0, -0.95), Vec2::new(0.75, 0.85), Vec2::new(0.0, 0.45), Vec2::new(-0.75, 0.85)];
            let p = Vec2::new(x, y);
            // Signed distance to the polygon (inside positive).
            let mut d = f32::MAX;
            let mut sign = -1.0;
            for i in 0..4 {
                let (a, b) = (tri[i], tri[(i + 1) % 4]);
                let e = b - a;
                let t = ((p - a).dot(e) / e.length_squared()).clamp(0.0, 1.0);
                d = d.min((p - a - e * t).length());
                if (a.y > y) != (b.y > y) && x < (b.x - a.x) * (y - a.y) / (b.y - a.y) + a.x {
                    sign = -sign;
                }
            }
            sign * d * px
        }
    }
}

/// White fill with a dark rim (tinted by the team colour, the rim stays dark), and
/// a white ring around the shape for the selection outline.
fn mask(shape: Shape, outline: bool) -> Image {
    let mut data = Vec::with_capacity((MASK * MASK * 4) as usize);
    for j in 0..MASK {
        for i in 0..MASK {
            let x = (i as f32 + 0.5) / MASK as f32 * 2.0 - 1.0;
            let y = (j as f32 + 0.5) / MASK as f32 * 2.0 - 1.0;
            // Shrink the shape so the outline ring fits inside the mask.
            let d = inside(shape, x / 0.82, y / 0.82) * 0.82;
            let px = if outline {
                let ring = (d + 6.0).clamp(0.0, 1.0) * (1.0 - (d + 1.0).clamp(0.0, 1.0));
                [255, 255, 255, (ring * 255.0) as u8]
            } else {
                let a = d.clamp(0.0, 1.0);
                let v = if d > 3.5 { 255 } else { 40 };
                [v, v, v, (a * 255.0) as u8]
            };
            data.extend_from_slice(&px);
        }
    }
    Image::new(
        Extent3d { width: MASK, height: MASK, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn shape_index(s: Shape) -> usize {
    match s {
        Shape::Arrow => 0,
        Shape::Square => 1,
        Shape::Circle => 2,
    }
}

fn make_masks(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let shapes = [Shape::Arrow, Shape::Square, Shape::Circle];
    commands.insert_resource(Masks {
        fill: shapes.map(|s| images.add(mask(s, false))),
        ring: images.add(mask(Shape::Circle, true)),
    });
}

fn toggle(keys: Res<ButtonInput<KeyCode>>, mut mode: ResMut<Mode>) {
    if keys.just_pressed(KeyCode::Tab) {
        *mode = match *mode {
            Mode::Auto => Mode::Always,
            Mode::Always => Mode::Off,
            Mode::Off => Mode::Auto,
        };
    }
}

/// Units drawn from the view get an icon too (an arrow in the owner's colour).
#[allow(clippy::type_complexity)]
fn ensure_icons(
    mut commands: Commands,
    units: Query<(Entity, Option<&TeamColour>, Option<&SelectRadius>), (With<FromView>, With<SimEntity>, Without<Strategic>, Without<NeedsModel>)>,
) {
    for (e, colour, radius) in &units {
        let colour = colour.map_or(Color::WHITE, |c| c.0);
        commands.entity(e).insert(Strategic::new(Shape::Arrow, colour, radius.map_or(20.0, |r| r.0)));
    }
}

fn view_params(cam: Option<&Camera>, proj: Option<&Projection>) -> f32 {
    let view = cam.and_then(|c| c.logical_viewport_size()).unwrap_or(Vec2::new(1280.0, 720.0));
    let fov = match proj {
        Some(Projection::Perspective(p)) => p.fov,
        _ => 0.8,
    };
    view.y * 0.5 / (fov * 0.5).tan()
}

/// Decide the mode, stack icons, list height bars, count grid lines, and show
/// models or hide them.
#[allow(clippy::type_complexity)]
fn compute(
    mode: Res<Mode>,
    camera: Query<(&Camera, &Transform, Option<&Projection>), crate::map::MainCamera>,
    mut units: Query<(&SimEntity, &Transform, &mut Visibility), With<Strategic>>,
    mut view: ResMut<StrategicView>,
) {
    let Ok((cam, tf, proj)) = camera.single() else { return };
    let eye = tf.translation;
    let fwd = tf.forward().as_vec3();
    let active = match *mode {
        Mode::Always => true,
        Mode::Off => false,
        Mode::Auto => eye.y.abs() > STRATEGIC_HEIGHT,
    };
    view.active = active;
    view.icons.clear();
    view.bars.clear();
    view.folded.clear();
    view.grid_lines = 0;
    if active {
        let list: Vec<(EntityId, Vec3)> = units.iter().map(|(s, t, _)| (s.0, t.translation)).collect();
        view.icons = stack(&list, eye, fwd, view_params(Some(cam), proj), STACK_RADIUS_PX);
        let folded: Vec<EntityId> = view.icons.iter().flat_map(|i| i.members.iter().skip(1).copied()).collect();
        view.folded.extend(folded);
        view.bars = list
            .iter()
            .map(|&(unit, p)| HeightBar { unit, top: p, bottom: Vec3::new(p.x, 0.0, p.z) })
            .collect();
        view.bars.sort_by_key(|b| b.unit);
        view.grid_lines = (GRID_HALF_CELLS as usize * 2 + 1) * 2;
    }
    // Units that appear while strategic hide too; coming back restores them once.
    if active || view.models_hidden {
        for (_, _, mut v) in &mut units {
            let want = if active { Visibility::Hidden } else { Visibility::Inherited };
            if *v != want {
                *v = want;
            }
        }
    }
    view.models_hidden = active;
}

fn draw(view: Res<StrategicView>, camera: Query<&Transform, crate::map::MainCamera>, mut gizmos: Gizmos) {
    if !view.active {
        return;
    }
    let Ok(tf) = camera.single() else { return };
    // The grid is centred on where the camera looks at the plane.
    let fwd = tf.forward().as_vec3();
    let hit = if fwd.y < -1e-3 { tf.translation + fwd * (-tf.translation.y / fwd.y) } else { Vec3::new(tf.translation.x, 0.0, tf.translation.z) };
    let (cx, cz) = ((hit.x / GRID_STEP).round() * GRID_STEP, (hit.z / GRID_STEP).round() * GRID_STEP);
    let half = GRID_HALF_CELLS as f32 * GRID_STEP;
    let col = Color::srgba(0.4, 0.6, 0.8, 0.25);
    for i in -GRID_HALF_CELLS..=GRID_HALF_CELLS {
        let o = i as f32 * GRID_STEP;
        gizmos.line(Vec3::new(cx + o, 0.0, cz - half), Vec3::new(cx + o, 0.0, cz + half), col);
        gizmos.line(Vec3::new(cx - half, 0.0, cz + o), Vec3::new(cx + half, 0.0, cz + o), col);
    }
    for b in view.bars.iter().filter(|b| b.length() > 0.0) {
        gizmos.line(b.top, b.bottom, Color::srgba(0.8, 0.9, 1.0, 0.5));
    }
}

/// Give each new strategic object its UI icon.
fn attach(mut commands: Commands, masks: Res<Masks>, added: Query<(Entity, &Strategic), Added<Strategic>>) {
    for (e, s) in &added {
        let i = shape_index(s.shape);
        let size = Val::Px(s.size_px);
        let node = Node {
            position_type: PositionType::Absolute,
            width: size,
            height: size,
            display: Display::None,
            ..default()
        };
        // The ring, a little larger than the icon and centred on it.
        let outline = commands
            .spawn((
                ImageNode { image: masks.ring.clone(), color: Color::srgb(0.3, 1.0, 0.4), ..default() },
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Percent(-25.0),
                    top: Val::Percent(-25.0),
                    width: Val::Percent(150.0),
                    height: Val::Percent(150.0),
                    ..default()
                },
                Visibility::Hidden,
            ))
            .id();
        let badge = commands
            .spawn((
                Text::new(""),
                TextFont { font_size: 12.0.into(), ..default() },
                TextColor(Color::WHITE),
                Node { position_type: PositionType::Absolute, left: Val::Percent(85.0), top: Val::Percent(-45.0), ..default() },
                Visibility::Hidden,
            ))
            .id();
        let image = s.image.clone().unwrap_or_else(|| masks.fill[i].clone());
        commands
            .spawn((
                Icon { target: e, outline, badge },
                ImageNode { image, color: s.colour, ..default() },
                node,
                UiTransform::IDENTITY,
                // Under the HUD text, over the scene.
                GlobalZIndex(-1),
            ))
            .add_children(&[outline, badge]);
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn update(
    mut commands: Commands,
    mode: Res<Mode>,
    camera: Query<(&Camera, &GlobalTransform, &Projection), crate::map::MainCamera>,
    mut objects: Query<(&GlobalTransform, &mut Strategic, Option<&SimEntity>)>,
    selection: Res<Selection>,
    mut icons: Query<(Entity, &Icon, &mut Node, &mut ImageNode, &mut UiTransform)>,
    mut outlines: Query<&mut Visibility, Without<Icon>>,
    mut texts: Query<&mut Text>,
    sv: Res<StrategicView>,
    images: Res<Assets<Image>>,
) {
    let Ok((cam, cam_tf, proj)) = camera.single() else { return };
    let Some(view) = cam.logical_viewport_size() else { return };
    let fov = match proj {
        Projection::Perspective(p) => p.fov,
        _ => 0.8,
    };
    // Pixels per world unit at distance 1.
    let focal = view.y * 0.5 / (fov * 0.5).tan();
    let eye = cam_tf.translation();
    let fwd = cam_tf.forward().as_vec3();
    for (icon_e, icon, mut node, mut img, mut ui_tf) in &mut icons {
        let Ok((gt, mut s, sim)) = objects.get_mut(icon.target) else {
            commands.entity(icon_e).despawn();
            continue;
        };
        let selected = sim.is_some_and(|e| selection.0.contains(&e.0));
        let pos = gt.translation();
        let depth = (pos - eye).dot(fwd);
        let screen = if depth > 1.0 { cam.world_to_viewport(cam_tf, pos).ok() } else { None };
        let shown = match (*mode, screen) {
            (_, None) | (Mode::Off, _) => 0.0,
            (Mode::Always, _) => 1.0,
            (Mode::Auto, _) if sv.active => 1.0,
            (Mode::Auto, _) => fade(s.radius * focal / depth),
        };
        s.shown = shown;
        let Some(p) = screen.filter(|_| shown > 0.0) else {
            node.display = Display::None;
            continue;
        };
        s.screen = p;
        let id = sim.map(|e| e.0);
        // Folded into a fleet icon: only the leader draws.
        let folded = sv.active && id.is_some_and(|i| sv.folded.contains(&i));
        let count = id.and_then(|i| sv.icons.iter().find(|s| s.leader == i)).map_or(1, StackIcon::count);
        if let Ok(mut v) = outlines.get_mut(icon.badge) {
            *v = if sv.active && count > 1 { Visibility::Inherited } else { Visibility::Hidden };
        }
        if count > 1
            && let Ok(mut t) = texts.get_mut(icon.badge)
            && t.0 != count.to_string()
        {
            t.0 = count.to_string();
        }
        node.display = if folded { Display::None } else { Display::Flex };
        node.left = Val::Px(p.x - s.size_px * 0.5);
        node.top = Val::Px(p.y - s.size_px * 0.5);
        img.color = s.colour.with_alpha(shown);
        // An atlas sprite: its rectangle in pixels once the atlas has loaded.
        if img.rect.is_none()
            && let (Some(uv), Some(h)) = (s.uv, s.image.as_ref())
            && let Some(atlas) = images.get(h)
        {
            let size = atlas.size_f32();
            img.rect = Some(Rect::new(uv[0] * size.x, uv[1] * size.y, (uv[0] + uv[2]) * size.x, (uv[1] + uv[3]) * size.y));
        }
        // Silhouettes keep the object's facing as seen on screen; of the generic
        // shapes only the ships' arrow turns.
        if s.image.is_some() || s.shape == Shape::Arrow {
            // Point along the heading as it falls on the screen.
            let ahead = cam.world_to_viewport(cam_tf, pos + gt.forward().as_vec3() * s.radius.max(1.0)).ok();
            if let Some(a) = ahead.map(|a| a - p).filter(|d| d.length_squared() > 1e-6) {
                // The mask points up (-y on screen); UI rotation is clockwise.
                ui_tf.rotation = Rot2::radians(a.x.atan2(-a.y));
            }
        }
        if let Ok(mut v) = outlines.get_mut(icon.outline) {
            *v = if selected { Visibility::Inherited } else { Visibility::Hidden };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_fade_in_as_ships_shrink() {
        assert_eq!(fade(40.0), 0.0);
        assert_eq!(fade(FADE_START_PX), 0.0);
        assert_eq!(fade(FADE_FULL_PX), 1.0);
        assert_eq!(fade(1.0), 1.0);
        let mid = fade((FADE_START_PX + FADE_FULL_PX) * 0.5);
        assert!((mid - 0.5).abs() < 1e-5);
    }

    #[test]
    fn shapes_by_role() {
        assert_eq!(shape_for("Craft", true), Shape::Arrow);
        assert_eq!(shape_for("shipyard", false), Shape::Square);
        assert_eq!(shape_for("Craft", false), Shape::Square);
        assert_eq!(shape_for("freighter", true), Shape::Circle);
        assert!(icon_size(Shape::Arrow, 30.0) < icon_size(Shape::Arrow, 300.0));
    }

    #[test]
    fn masks_have_a_solid_centre_and_clear_corners() {
        for s in [Shape::Arrow, Shape::Square, Shape::Circle] {
            assert!(inside(s, 0.0, 0.3) > 3.0, "{s:?} centre");
            assert!(inside(s, 0.98, 0.98) < 0.0, "{s:?} corner");
        }
        // The arrow points up: its tip region is inside, the bottom middle (notch) not.
        assert!(inside(Shape::Arrow, 0.0, -0.8) > 0.0);
        assert!(inside(Shape::Arrow, 0.0, 0.7) < 0.0);
    }

    #[test]
    fn stacking_is_by_id_order_and_screen_distance() {
        let eye = Vec3::new(0.0, 5000.0, 0.0);
        let fwd = Vec3::NEG_Y;
        let focal = 700.0;
        // 5000 up: 1 px is ~7 world units, so the radius is ~200 units.
        let near = |x: f32| Vec3::new(x, 0.0, 0.0);
        let out = stack(&[(EntityId(3), near(150.0)), (EntityId(1), near(0.0)), (EntityId(2), near(5000.0))], eye, fwd, focal, STACK_RADIUS_PX);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].leader, EntityId(1));
        assert_eq!(out[0].members, vec![EntityId(1), EntityId(3)]);
        assert_eq!(out[1].members, vec![EntityId(2)]);
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;
    use crate::app::configure_sets;
    use crate::render::units::{LocalPlayer, add_view_systems};
    use crate::sim_link::{SimLink, SimLinkPlugin};
    use crate::viewer::TeamColours;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;
    use sta_sim::PlayerId;
    use sta_sim::command::LocalRouter;
    use sta_sim::fake::ScriptedSource;

    fn rig(text: &str, local: Option<PlayerId>, cam_y: f32) -> App {
        let mut app = App::new();
        let db = sta_content::ContentDb::builder().build().unwrap();
        let source = ScriptedSource::from_json5(text, &db).unwrap();
        app.add_plugins(MinimalPlugins)
            .add_plugins(AssetPlugin::default())
            .init_asset::<Image>()
            .init_resource::<Selection>()
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 30.0)))
            .add_plugins(SimLinkPlugin)
            .insert_resource(LocalPlayer(local))
            .insert_resource(TeamColours(vec![("e".into(), Color::WHITE), ("b".into(), Color::srgb(0.0, 0.0, 1.0)), ("r".into(), Color::srgb(1.0, 0.0, 0.0))]))
            .insert_resource(SimLink::new(Box::new(source), Box::new(LocalRouter::new()), PlayerId(1)));
        configure_sets(&mut app);
        add_view_systems(&mut app);
        app.add_plugins(StrategicPlugin);
        app.world_mut().spawn((Camera3d::default(), Transform::from_xyz(0.0, cam_y, 0.0).looking_to(Vec3::NEG_Y, Vec3::NEG_Z)));
        app
    }

    fn unit(id: u32, owner: u32, x: f32, y: f32, z: f32) -> String {
        format!("{{ id: {id}, owner: {owner}, hull: 100, rails: [{{ at: 0, pos: {{ x: {x}, y: {y}, z: {z} }} }}] }}")
    }

    fn world(units: &[String]) -> String {
        format!("{{ units: [{}] }}", units.join(","))
    }

    fn view(app: &App) -> &StrategicView {
        app.world().resource::<StrategicView>()
    }

    fn visible(app: &mut App, id: u32) -> Visibility {
        let mut q = app.world_mut().query::<(&SimEntity, &Visibility)>();
        *q.iter(app.world()).find(|(s, _)| s.0 == EntityId(id)).unwrap().1
    }

    #[test]
    fn spread_units_each_get_an_icon_and_a_height_bar() {
        let us = [unit(1, 1, -2000.0, 0.0, 0.0), unit(2, 1, 0.0, 120.0, 0.0), unit(3, 2, 2000.0, 40.0, 500.0)];
        let mut app = rig(&world(&us), None, 6000.0);
        app.update();
        app.update();
        let v = view(&app);
        assert!(v.active);
        assert_eq!(v.icons.len(), 3);
        assert!(v.icons.iter().all(|i| i.count() == 1));
        assert_eq!(v.bars.len(), 3);
        let lens: Vec<f32> = v.bars.iter().map(|b| b.length()).collect();
        assert_eq!(lens, vec![0.0, 120.0, 40.0]);
        assert!(v.bars.iter().all(|b| b.bottom.y == 0.0 && b.bottom.x == b.top.x));
        assert_eq!(v.grid_lines, 50);
    }

    #[test]
    fn close_units_merge_and_split_again() {
        // Three units within ~120 units of each other merge; one moved away splits off.
        let us = [
            unit(1, 1, 0.0, 0.0, 0.0),
            unit(2, 1, 100.0, 0.0, 0.0),
            unit(3, 1, 120.0, 0.0, 0.0),
        ];
        let mut app = rig(&world(&us), None, 6000.0);
        app.update();
        app.update();
        let v = view(&app);
        assert_eq!(v.icons.len(), 1);
        assert_eq!(v.icons[0].count(), 3);
        assert_eq!(v.icons[0].leader, EntityId(1));
        assert_eq!(v.folded.len(), 2);
        // `place` rewrites transforms each frame, so check the split on the pure function.
        let eye = Vec3::new(0.0, 6000.0, 0.0);
        let near = stack(&[(EntityId(1), Vec3::ZERO), (EntityId(3), Vec3::new(4000.0, 0.0, 0.0))], eye, Vec3::NEG_Y, 700.0, STACK_RADIUS_PX);
        assert_eq!(near.len(), 2);
    }

    #[test]
    fn strategic_mode_toggles_icons_and_models() {
        let us = [unit(1, 1, 0.0, 0.0, 0.0)];
        let mut low = rig(&world(&us), None, 300.0);
        low.update();
        low.update();
        assert!(!view(&low).active);
        assert!(view(&low).icons.is_empty() && view(&low).bars.is_empty());
        assert_eq!(visible(&mut low, 1), Visibility::Inherited);
        *low.world_mut().resource_mut::<Mode>() = Mode::Always;
        low.update();
        assert!(view(&low).active);
        assert_eq!(view(&low).icons.len(), 1);
        assert_eq!(visible(&mut low, 1), Visibility::Hidden);
        *low.world_mut().resource_mut::<Mode>() = Mode::Off;
        low.update();
        assert!(!view(&low).active);
        assert_eq!(visible(&mut low, 1), Visibility::Inherited);
        let mut high = rig(&world(&us), None, 6000.0);
        high.update();
        high.update();
        assert!(view(&high).active);
        assert_eq!(visible(&mut high, 1), Visibility::Hidden);
    }

    #[test]
    fn only_units_visible_to_the_local_player_get_icons() {
        // Unit 2 (player 2) is far outside player 1's sight; 1 and 3 are mine.
        let us = [unit(1, 1, 0.0, 0.0, 0.0), unit(2, 2, 90000.0, 0.0, 90000.0), unit(3, 1, 2000.0, 0.0, 0.0)];
        let mut all = rig(&world(&us), None, 6000.0);
        all.update();
        all.update();
        assert_eq!(view(&all).icons.len(), 3);
        let mut mine = rig(&world(&us), Some(PlayerId(1)), 6000.0);
        mine.update();
        mine.update();
        let n = view(&mine).icons.len();
        let seen = mine.world().resource::<SimLink>().curr().visible_to(PlayerId(1)).len();
        assert_eq!(n, seen);
    }
}
